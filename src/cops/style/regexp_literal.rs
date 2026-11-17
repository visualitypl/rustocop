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
        } else if has_slash || allowed_argument(node, cx, &regexp) {
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
    // Main-parse facts for the pinned regexp source-tree consumer's emitted
    // multiline str ranges. Style spelling/corrections still use full fragments.
    pub(crate) fragment_line_counts: Vec<usize>,
}
impl<'pr> Regexp<'pr> {
    pub(crate) fn new(node: &Node<'pr>) -> Option<Self> {
        let lines = |bytes: &[u8]| bytes.split_inclusive(|b| *b == b'\n').count();
        let part_lines = |parts: ruby_prism::NodeList<'pr>| {
            parts
                .iter()
                .filter_map(|n| n.as_string_node().map(|s| lines(s.unescaped())))
                .collect()
        };
        let (opening, closing, fragments, fragment_line_counts) = match node {
            Node::RegularExpressionNode { .. } => {
                let n = node.as_regular_expression_node()?;
                (
                    n.opening_loc(),
                    n.closing_loc(),
                    vec![n.content_loc()],
                    vec![lines(n.unescaped())],
                )
            }
            Node::MatchLastLineNode { .. } => {
                let n = node.as_match_last_line_node()?;
                (
                    n.opening_loc(),
                    n.closing_loc(),
                    vec![n.content_loc()],
                    vec![lines(n.unescaped())],
                )
            }
            Node::InterpolatedRegularExpressionNode { .. } => {
                let n = node.as_interpolated_regular_expression_node()?;
                (
                    n.opening_loc(),
                    n.closing_loc(),
                    static_fragments(n.parts()),
                    part_lines(n.parts()),
                )
            }
            Node::InterpolatedMatchLastLineNode { .. } => {
                let n = node.as_interpolated_match_last_line_node()?;
                (
                    n.opening_loc(),
                    n.closing_loc(),
                    static_fragments(n.parts()),
                    part_lines(n.parts()),
                )
            }
            _ => return None,
        };
        Some(Self {
            opening,
            closing,
            fragments,
            fragment_line_counts,
        })
    }
}
// Corrections touch only direct static text, never interpolation expressions.
// The atomic envelope costs at most the literal's size; nesting is bounded by
// the pipeline's fixed 64-node AST-depth limit. No per-node parse or file scan.
/// RuboCop's corrector: delimiters, inner slashes and escaped delimiters.
fn correction(
    regexp: &Regexp<'_>,
    slashes: bool,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let _ = cx;
    let opening = &regexp.opening;
    let closing = &regexp.closing;
    let edit =
        |range: std::ops::Range<usize>, replacement: String| Correction { range, replacement };
    let (new_open, new_close) = if slashes { ("%r{", "}") } else { ("/", "/") };
    let mut edits = vec![
        edit(
            opening.start_offset()..opening.end_offset(),
            new_open.into(),
        ),
        edit(
            closing.start_offset()..closing.start_offset() + 1,
            new_close.into(),
        ),
    ];
    // `correct_inner_slashes`
    let inner = |open: &[u8]| {
        if matches!(open, b"/" | b"%r/") {
            &b"\\/"[..]
        } else {
            &b"/"[..]
        }
    };
    let pattern = inner(opening.as_slice());
    let replacement = std::str::from_utf8(inner(new_open.as_bytes())).ok()?;
    // `correct_escaped_delimiters`
    let delimiter = (!slashes)
        .then(|| opening.as_slice().last().copied())
        .flatten()
        .filter(|d: &u8| *d != b'/' && !b"$*+.?^|()[]{}<>".contains(d));
    let extended = closing.as_slice()[1..].contains(&b'x');
    for loc in &regexp.fragments {
        let text = loc.as_slice();
        let base = loc.start_offset();
        let mut at = 0;
        while let Some(found) = text
            .get(at..)
            .and_then(|t| t.windows(pattern.len()).position(|w| w == pattern))
        {
            let index = at + found;
            edits.push(edit(
                base + index..base + index + pattern.len(),
                replacement.into(),
            ));
            at = index + 1;
        }
        if let Some(delimiter) = delimiter {
            let mut index = 0;
            while let Some(found) = text
                .get(index..)
                .and_then(|t| t.iter().position(|b| *b == b'\\'))
            {
                index += found;
                if text.get(index + 1) == Some(&delimiter) {
                    let next = text.get(index + 2);
                    let replacement =
                        if delimiter == b'#' && next.is_some_and(|b| b"{@$".contains(b)) {
                            if extended { "#\\" } else { "\\#" }.to_owned()
                        } else {
                            char::from(delimiter).to_string()
                        };
                    edits.push(edit(base + index..base + index + 2, replacement));
                }
                index += 2;
            }
        }
    }
    super::correction::combine(file, edits)
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
fn allowed_argument(node: &Node<'_>, cx: &NodeContext<'_, '_>, regexp: &Regexp<'_>) -> bool {
    let mut ancestors = cx.ancestors();
    let Some(parent) = ancestors.next() else {
        return false;
    };
    let first = regexp
        .fragments
        .first()
        .and_then(|fragment| fragment.as_slice().first().copied());
    if let Some(call) = parent.as_call_node() {
        let start = node.location().start_offset();
        return call
            .receiver()
            .is_some_and(|receiver| receiver.location().start_offset() == start)
            && !(call.name().as_slice() == b"=~"
                && call.call_operator_loc().is_none()
                && node.as_regular_expression_node().is_some())
            && matches!(first, Some(b' ' | b'='));
    }
    let Some(arguments) = parent.as_arguments_node() else {
        return false;
    };
    let Some(call) = ancestors.next().and_then(|n| n.as_call_node()) else {
        return false;
    };
    if matches!(first, Some(b' ' | b'=')) {
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
        && first.is_some_and(|byte| byte.is_ascii_whitespace())
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
    fn percent_r_receivers_follow_the_static_content_and_the_call_shape() {
        for source in [
            "%r/#{name} (?<expression>.+?)/ =~ info\n",
            "%r/#{name}=value/ =~ info\n",
            "%r/ value/.match(info)\n",
            "%r/=value/.match?(info)\n",
            "%r/ value/ !~ info\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for (source, col) in [
            ("%r/ value/ =~ info\n", 1),
            ("%r/#{name}value/ =~ info\n", 1),
            ("%r/value/.match(info)\n", 1),
            ("%r/#{name}/ =~ info\n", 1),
            ("x = %r/ value/\n", 5),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
        }
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
