//! `Style/RegexpLiteral` — slashes unless static regexp text contains a slash.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{Location, Node};

pub struct RegexpLiteral;
impl Cop for RegexpLiteral {
    fn name(&self) -> &'static str {
        "Style/RegexpLiteral"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(regexp) = Regexp::new(node) else {
            return;
        };
        let slashes = regexp.opening.as_slice() == b"/";
        let body = || {
            regexp.fragments.iter().flat_map(|loc| {
                super::literal_spelling::character_heads(file, loc.start_offset()..loc.end_offset())
            })
        };
        let has_slash = body().any(|b| b == b'/');
        if slashes {
            if !has_slash || !balanced_braces(body()) {
                return;
            }
        } else if has_slash || allowed_argument(node, cx, &regexp, file) {
            return;
        }
        let (line, col) = file.line_col(regexp.opening.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: if slashes {
                "Use `%r` around regular expression."
            } else {
                "Use `//` around regular expression."
            }
            .into(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction: cx
                .corrections_requested()
                .then(|| correction(&regexp, slashes, cx, file))
                .flatten(),
        });
    }
}

pub(crate) struct Regexp<'pr> {
    pub(crate) opening: Location<'pr>,
    pub(crate) closing: Location<'pr>,
    // Only direct static fragments: interpolation expressions are separate Ruby
    // code, never text in the outer regexp. Total work is linear in source bytes.
    pub(crate) fragments: Vec<Location<'pr>>,
}
impl<'pr> Regexp<'pr> {
    pub(crate) fn new(node: &Node<'pr>) -> Option<Self> {
        let (opening, closing, fragments) = match node {
            Node::RegularExpressionNode { .. } => {
                let n = node.as_regular_expression_node()?;
                (n.opening_loc(), n.closing_loc(), vec![n.content_loc()])
            }
            Node::MatchLastLineNode { .. } => {
                let n = node.as_match_last_line_node()?;
                (n.opening_loc(), n.closing_loc(), vec![n.content_loc()])
            }
            Node::InterpolatedRegularExpressionNode { .. } => {
                let n = node.as_interpolated_regular_expression_node()?;
                (
                    n.opening_loc(),
                    n.closing_loc(),
                    static_fragments(n.parts()),
                )
            }
            Node::InterpolatedMatchLastLineNode { .. } => {
                let n = node.as_interpolated_match_last_line_node()?;
                (
                    n.opening_loc(),
                    n.closing_loc(),
                    static_fragments(n.parts()),
                )
            }
            _ => return None,
        };
        Some(Self {
            opening,
            closing,
            fragments,
        })
    }
}
// Corrections touch only direct static text, never interpolation expressions.
// The atomic envelope costs at most the literal's size; nesting is bounded by
// the pipeline's fixed 64-node AST-depth limit. No per-node parse or file scan.
fn correction(
    regexp: &Regexp<'_>,
    slashes: bool,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let opening = &regexp.opening;
    let closing = &regexp.closing;
    let used = *opening.as_slice().last()?;
    if !slashes && (opening.as_slice().len() != 3 || !used.is_ascii_punctuation() || used == b'\\')
    {
        return None; // physical-newline/whitespace/backslash delimiters lack a lexical proof
    }
    let mut edits = vec![
        Correction {
            range: opening.start_offset()..opening.end_offset(),
            replacement: if slashes { "%r{" } else { "/" }.into(),
        },
        Correction {
            range: closing.start_offset()..closing.start_offset() + 1,
            replacement: if slashes { "}" } else { "/" }.into(),
        },
    ];
    let unescape = !slashes && !b"/$*+.?^|()[]{}<>".contains(&used);
    for loc in &regexp.fragments {
        // Whole envelopes containing non-UTF-8 bytes cannot be represented by
        // Correction. Do not reinterpret legacy multibyte trail bytes as syntax.
        let text = std::str::from_utf8(loc.as_slice()).ok()?;
        let bytes = text.as_bytes();
        let mut i = 0;
        while i + 1 < bytes.len() {
            if bytes[i] != b'\\' {
                i += 1;
                continue;
            }
            let next = bytes[i + 1];
            if (slashes && next == b'/') || (unescape && next == used) {
                // Hash-delimiter rewrites can change Regexp#source; removing
                // an escaped @ after # can introduce #@ / #@@ interpolation,
                // even in extended-mode comments. Keep the offense, not the edit.
                if (used == b'#' && bytes.get(i + 2).is_some_and(|b| b"{@$".contains(b)))
                    || (used == b'@' && i > 0 && bytes[i - 1] == b'#')
                {
                    return None;
                }
                edits.push(Correction {
                    range: loc.start_offset() + i..loc.start_offset() + i + 2,
                    replacement: char::from(next).to_string(),
                });
            }
            i += 2;
        }
    }
    let edit = super::correction::combine(file, edits)?;
    super::literal_spelling::safe_edit(file, cx, edit)
}
fn static_fragments(parts: ruby_prism::NodeList<'_>) -> Vec<Location<'_>> {
    parts
        .iter()
        .filter_map(|n| n.as_string_node().map(|n| n.content_loc()))
        .collect()
}
fn balanced_braces(bytes: impl Iterator<Item = u8>) -> bool {
    let mut depth = 0usize;
    let mut bytes = bytes;
    while let Some(b) = bytes.next() {
        match b {
            b'\\' => {
                bytes.next();
            }
            b'{' => depth += 1,
            b'}' => {
                let Some(next) = depth.checked_sub(1) else {
                    return false;
                };
                depth = next;
            }
            _ => {}
        }
    }
    depth == 0
}
fn allowed_argument(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    regexp: &Regexp<'_>,
    file: &SourceFile,
) -> bool {
    let mut ancestors = cx.ancestors();
    let Some(parent) = ancestors.next() else {
        return false;
    };
    let Some(arguments) = parent.as_arguments_node() else {
        return false;
    };
    let Some(call) = ancestors.next().and_then(|n| n.as_call_node()) else {
        return false;
    };
    let content = &file.bytes[regexp.opening.end_offset()..regexp.closing.start_offset()];
    if matches!(content.first(), Some(b' ' | b'=')) {
        return true;
    }
    // Only the first argument matters. Collecting all siblings here would be
    // quadratic for a command call containing many regexp arguments.
    arguments
        .arguments()
        .iter()
        .next()
        .is_some_and(|first| first.location().start_offset() == node.location().start_offset())
        && call.opening_loc().is_none()
        && !ast::is_operator_method(call.name().as_slice())
        && !call.name().as_slice().ends_with(b"=")
        && content.first().is_some_and(u8::is_ascii_whitespace)
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's `spec/rubocop/cop/style/regexp_literal_spec.rb`.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/RegexpLiteral")
            .collect()
    }
    #[test]
    fn wide_command_argument_detection_does_not_collect_siblings_per_regexp() {
        let count = 2_000;
        let source = format!("consume {}\n", vec!["%r{a}"; count].join(", "));
        ast::CALL_ARGUMENT_VISITS.set(0);
        let found = crate::cops::testing::correction_offenses(&RegexpLiteral, &source);
        assert_eq!(found.len(), count);
        assert!(
            ast::CALL_ARGUMENT_VISITS.get() < count * 10,
            "argument visits: {}",
            ast::CALL_ARGUMENT_VISITS.get()
        );
    }
    #[test]
    fn unicode_scalar_columns_and_regexp_options() {
        let found = offenses("p \"é\"; %r{café}imxou\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 8));
        assert_eq!(found[0].message, "Use `//` around regular expression.");
        assert_correction(&RegexpLiteral, "%r{café}imxou", "/café/imxou");
        for option in ["n", "e", "s", "u", "imx", "o"] {
            assert_correction(
                &RegexpLiteral,
                &format!("/home\\//{option}"),
                &format!("%r{{home/}}{option}"),
            );
        }
    }
    #[test]
    fn pinned_diagnostics() {
        for (source, message, end_line) in [
            (r"foo = /home\//", "Use `%r` around regular expression.", 2),
            ("foo = %r{a}", "Use `//` around regular expression.", 2),
            (
                "foo = %r{\n  foo\n  bar\n}x",
                "Use `//` around regular expression.",
                5,
            ),
            (
                "foo = /\n  https?:\\/\\/\n  example\\.com\n/x",
                "Use `%r` around regular expression.",
                5,
            ),
            (
                r"foo = %r{^#{line[/^\s+/]}\s}",
                "Use `//` around regular expression.",
                2,
            ),
        ] {
            let found = offenses(&format!("# heading\n  {source}\n"));
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (2, 9, end_line)
            );
            assert_eq!(found[0].message, message);
        }
    }
    #[test]
    fn pinned_canonical_corrections() {
        for (before, after) in [
            (r"foo = /home\//", "foo = %r{home/}"),
            (
                r"foo = /users\/#{user.id}\/forms/",
                "foo = %r{users/#{user.id}/forms}",
            ),
            (r"x = /{foo}\//", "x = %r{{foo}/}"),
            (r"x = /\{foo\//", r"x = %r{\{foo/}"),
            (r#"x = /\/\A#{"{"}\z/"#, r#"x = %r{/\A#{"{"}\z}"#),
            (r"x =~ %r{^#{line[/^\s+/]}\s}", r"x =~ /^#{line[/^\s+/]}\s/"),
            ("%r_ls_", "/ls/"),
            ("foo = %r{a}", "foo = /a/"),
            (r"foo = %r!(?\!x)!", "foo = /(?!x)/"),
            (r"foo = %r!a\\\!b!", r"foo = /a\\!b/"),
            (r"foo = %r-[a\-z]-", "foo = /[a-z]/"),
            (r"foo = %r#a\#b#x", "foo = /a#b/x"),
            (r"foo = %r@a\@b@", "foo = /a@b/"),
            (r"foo = %r@a\@b@x", "foo = /a@b/x"),
            (r"foo = %r{a\{b}", r"foo = /a\{b/"),
            (
                "foo = /\n  https?:\\/\\/\n  example\\.com\n/x",
                "foo = %r{\n  https?://\n  example\\.com\n}x",
            ),
            ("foo = %r{\n  foo\n  bar\n}x", "foo = /\n  foo\n  bar\n/x"),
            ("do_something %r/regexp/", "do_something /regexp/"),
            ("foo&.do_something %r/regexp/", "foo&.do_something /regexp/"),
            ("%r/regexp/", "/regexp/"),
            ("%r/ regexp/", "/ regexp/"),
            (
                "do_something(%r{\n  regexp\n}x)",
                "do_something(/\n  regexp\n/x)",
            ),
            (
                "do_something foo, %r{\n  regexp\n}x",
                "do_something foo, /\n  regexp\n/x",
            ),
            ("foo =~ %r{\n  regexp\n}x", "foo =~ /\n  regexp\n/x"),
        ] {
            assert_correction(&RegexpLiteral, before, after);
        }
    }
    #[test]
    fn pinned_no_offense_lookalikes() {
        for source in [
            "foo = /a/",
            "foo = /\n  foo\n  bar\n/x",
            "foo = %r{home/}",
            "foo = %r{\n  https?://\n  example\\.com\n}x",
            r"x =~ /\s{#{x[/\s+/].length}}/",
            r"x = /{\//",
            r"x = /}\//",
            r"x = /\\{\//",
            "do_something %r/ regexp/",
            "foo&.do_something %r/ regexp/",
            "do_something %r/=regexp/",
            "do_something %r{\n  regexp\n}x",
            "'/home\\//'",
            "# %r{a}\n",
            "Regexp.new('a')",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
