//! Prism's own lexer tokens, for the parser-gem token projection in
//! `source::tokens` (Layout/ExtraSpacing and its PrecedingFollowingAlignment
//! consumers). RuboCop's tokens are Prism 1.9's `Prism.lex` stream translated
//! by `Prism::Translation::Parser::Lexer`; this records the same stream with
//! Prism's native lex callback, exactly as `Prism.lex` does.
//!
//! The pipeline's main parse records them ([`super::parse_with_tokens`]).
//! Elsewhere [`lex`] makes a token-only parse of the same bytes with the same
//! options, on the same dedicated parser stack. Only token types, byte
//! offsets and the lex state at each token are kept. Nothing is executed.
use std::ffi::c_void;

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
    // SAFETY: `recording` keeps the collector alive and unaliased for the
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

/// Run `parse` with a lexer callback that records every token Prism lexes
/// from `source`, in Prism's emission order (a heredoc body directly after
/// its opener, as in `Prism.lex`).
pub(super) fn recording<T>(
    source: &[u8],
    parse: impl FnOnce(&mut sys::pm_lex_callback_t) -> T,
) -> (T, Vec<RawToken>) {
    let mut collector = Collector {
        base: source.as_ptr() as usize,
        len: source.len(),
        tokens: Vec::new(),
    };
    let mut callback = sys::pm_lex_callback_t {
        data: (&raw mut collector).cast(),
        callback: Some(record),
    };
    let result = parse(&mut callback);
    (result, collector.tokens)
}

/// Every token Prism lexes from `source`: a token-only parse with the main
/// parse's options, for a file whose main parse did not record its tokens.
pub(crate) fn lex(source: &[u8], version: crate::cops::policy::RubyVersion) -> Vec<RawToken> {
    let grammar = super::grammar(version);
    stacker::grow(super::PARSER_STACK_SIZE, || {
        recording(source, |callback| {
            ruby_prism::parse_with_version_lexing(source, grammar, callback)
        })
        .1
    })
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
