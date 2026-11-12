//! Prism's own lexer tokens, for the parser-gem token projection in
//! `source::tokens` (Layout/ExtraSpacing and its PrecedingFollowingAlignment
//! consumers). RuboCop's tokens are Prism 1.9's `Prism.lex` stream translated
//! by `Prism::Translation::Parser::Lexer`; this records the same stream with
//! Prism's native lex callback, exactly as `Prism.lex` does.
//!
//! It is a lazy, token-only parse of the same bytes with the same fixed Ruby
//! 3.4 options as [`super::parse`], on the same dedicated parser stack. Only
//! token types, byte offsets and the lex state at each token are kept; the
//! tree is destroyed on that stack before returning. Nothing is executed and
//! no grammar or option differs from the main parse.
use std::{ffi::c_void, mem::MaybeUninit};

use ruby_prism_sys as sys;

/// One token as Prism's lexer reported it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RawToken {
    pub kind: sys::pm_token_type,
    pub start: usize,
    pub end: usize,
    /// `parser->lex_state` when the token was reported (what `Prism.lex`
    /// pairs with each token, and the translator reads for `tLBRACE`).
    pub state: sys::pm_lex_state_t,
}

struct Collector {
    base: usize,
    len: usize,
    tokens: Vec<RawToken>,
}

unsafe extern "C" fn record(
    data: *mut c_void,
    parser: *mut sys::pm_parser_t,
    token: *mut sys::pm_token_t,
) {
    // SAFETY: `lex` keeps the collector alive and exclusively borrowed for the
    // synchronous parse that owns this callback; Prism passes its live parser
    // and current token. Offsets are clamped, never dereferenced.
    let (collector, token, state) =
        unsafe { (&mut *data.cast::<Collector>(), &*token, (*parser).lex_state) };
    let offset = |pointer: *const u8| {
        (pointer as usize)
            .wrapping_sub(collector.base)
            .min(collector.len)
    };
    let start = offset(token.start);
    collector.tokens.push(RawToken {
        kind: token.type_,
        start,
        end: offset(token.end).max(start),
        state,
    });
}

/// Every token Prism lexes from `source`, in Prism's emission order (a
/// heredoc body directly after its opener, as in `Prism.lex`).
pub(crate) fn lex(source: &[u8], version: crate::cops::policy::RubyVersion) -> Vec<RawToken> {
    stacker::grow(super::PARSER_STACK_SIZE, || {
        let mut collector = Collector {
            base: source.as_ptr() as usize,
            len: source.len(),
            tokens: Vec::new(),
        };
        // SAFETY: the same initialization as ruby_prism::parse_ruby_3_4: a
        // zeroed pm_options_t is Prism's documented default; the options and
        // callback outlive pm_parse, and the parser and tree are freed here.
        unsafe {
            let mut options = std::mem::zeroed::<sys::pm_options_t>();
            options.line = 1;
            options.version = super::grammar(version);
            options.main_script = main_script(source);
            let mut callback = sys::pm_lex_callback_t {
                data: (&raw mut collector).cast(),
                callback: Some(record),
            };
            let mut parser = Box::new(MaybeUninit::<sys::pm_parser_t>::uninit());
            sys::pm_parser_init(parser.as_mut_ptr(), source.as_ptr(), source.len(), &options);
            let parser = parser.as_mut_ptr();
            (*parser).lex_callback = &raw mut callback;
            let node = sys::pm_parse(parser);
            (*parser).lex_callback = std::ptr::null_mut();
            sys::pm_node_destroy(parser, node);
            sys::pm_parser_free(parser);
        }
        collector.tokens
    })
}

/// ruby_prism::parse_ruby_3_4's shebang predicate (Prism 1.9's own search:
/// the length excludes a BOM, the search starts at the original source).
fn main_script(source: &[u8]) -> bool {
    let after_bom = source.strip_prefix(b"\xef\xbb\xbf").unwrap_or(source);
    if !after_bom.starts_with(b"#!") {
        return false;
    }
    let length = after_bom
        .iter()
        .position(|&byte| byte == b'\n')
        .unwrap_or(after_bom.len());
    source[..length].windows(4).any(|word| word == b"ruby")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cops::policy::DEFAULT_RUBY_VERSION;

    fn kinds(source: &str) -> Vec<(sys::pm_token_type, &str)> {
        lex(source.as_bytes(), DEFAULT_RUBY_VERSION)
            .into_iter()
            .filter(|t| t.kind != sys::PM_TOKEN_EOF)
            .map(|t| (t.kind, &source[t.start..t.end]))
            .collect()
    }

    #[test]
    fn the_stream_is_prisms_emission_order() {
        assert_eq!(
            kinds("foo(<<~EOS, 1)\n  body\nEOS\n"),
            [
                (sys::PM_TOKEN_IDENTIFIER, "foo"),
                (sys::PM_TOKEN_PARENTHESIS_LEFT, "("),
                (sys::PM_TOKEN_HEREDOC_START, "<<~EOS"),
                (sys::PM_TOKEN_STRING_CONTENT, "  body\n"),
                (sys::PM_TOKEN_HEREDOC_END, "EOS\n"),
                (sys::PM_TOKEN_COMMA, ","),
                (sys::PM_TOKEN_INTEGER, "1"),
                (sys::PM_TOKEN_PARENTHESIS_RIGHT, ")"),
                (sys::PM_TOKEN_NEWLINE, "\n"),
            ]
        );
        assert_eq!(
            kinds("a # c\n"),
            [
                (sys::PM_TOKEN_IDENTIFIER, "a"),
                (sys::PM_TOKEN_COMMENT, "# c\n"),
            ]
        );
    }

    #[test]
    fn shebang_predicate_matches_the_main_parse() {
        assert!(main_script(b"#!/usr/bin/env ruby\nx\n"));
        // Prism's quirk: the BOM-less length is searched from the BOM.
        assert!(!main_script(b"\xef\xbb\xbf#!ruby\nx\n"));
        assert!(main_script(b"\xef\xbb\xbf#!/bin/ruby -w\nx\n"));
        assert!(!main_script(b"#!/bin/sh\nx\n"));
        assert!(!main_script(b"x\n"));
    }

    #[test]
    fn deep_and_invalid_sources_lex_without_panicking() {
        let deep = format!("{}1{}", "[".repeat(20_000), "]".repeat(20_000));
        assert!(!lex(deep.as_bytes(), DEFAULT_RUBY_VERSION).is_empty());
        assert!(!lex(b"def (", DEFAULT_RUBY_VERSION).is_empty());
        assert!(
            lex(b"", DEFAULT_RUBY_VERSION)
                .iter()
                .all(|t| t.kind == sys::PM_TOKEN_EOF)
        );
    }
}
