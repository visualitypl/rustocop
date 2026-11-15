//! `Style/RedundantRegexpArgument` — deterministic regexp arguments.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
#[derive(Default)]
pub struct RedundantRegexpArgument {
    /// The style of Style/StringLiterals is `single_quotes`.
    single_quotes: bool,
}
impl Cop for RedundantRegexpArgument {
    fn name(&self) -> &'static str {
        "Style/RedundantRegexpArgument"
    }
    fn with_single_quoted_strings(&self, single: bool) -> Option<Box<dyn Cop>> {
        Some(Box::new(Self {
            single_quotes: single,
        }))
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if !METHODS.contains(&call.name().as_slice()) {
            return;
        }
        let Some(args) = call.arguments() else { return };
        let Some(first) = args.arguments().iter().next() else {
            return;
        };
        let Some(regexp) = first.as_regular_expression_node() else {
            return;
        };
        if regexp.closing_loc().as_slice().len() != 1
            || regexp.content_loc().as_slice() == b" "
            || !deterministic(regexp.location().as_slice())
        {
            return;
        }
        let Some(prefer) = preferred(regexp.content_loc().as_slice(), self.single_quotes) else {
            return;
        };
        let loc = regexp.location();
        let (line, col) = file.line_col(loc.start_offset());
        let correction = if cx.corrections_requested()
            && trusted_call(&call)
            && cx
                .ancestors()
                .next_back()
                .is_some_and(|root| !file.redundant_regexp_safety(root).changed)
        {
            super::literal_spelling::safe_edit(
                file,
                cx,
                Correction {
                    range: loc.start_offset()..loc.end_offset(),
                    replacement: prefer.clone(),
                },
            )
        } else {
            None
        };
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Use string `{prefer}` as argument instead of regexp `{}`.",
                String::from_utf8_lossy(loc.as_slice())
            ),
            line,
            col,
            end_line: file.last_line_of(loc),
            correction,
        });
    }
}
const METHODS: &[&[u8]] = &[
    b"byteindex",
    b"byterindex",
    b"gsub",
    b"gsub!",
    b"partition",
    b"rpartition",
    b"scan",
    b"split",
    b"start_with?",
    b"sub",
    b"sub!",
];
// Port of the pinned LITERAL_REGEX token policy, including the literal's delimiters.
fn deterministic(bytes: &[u8]) -> bool {
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' {
            i += 1;
            if bytes
                .get(i)
                .is_none_or(|b| b"AbBdDgGhHkpPRwWXsSzZ0123456789".contains(b))
            {
                return false;
            }
        } else if !b.is_ascii_alphanumeric()
            && !(b.is_ascii_whitespace() || b == 0x0b)
            && !b"_-,\"'!#%&<>=;:`~/".contains(&b)
        {
            return false;
        }
        i += 1;
    }
    true
}
/// The string for the regexp content `raw`. As in RuboCop, a string with a
/// quote in it is single-quoted and one with a backslash is double-quoted;
/// any other has the quotes of Style/StringLiterals.
fn preferred(raw: &[u8], single_quotes: bool) -> Option<String> {
    let text = std::str::from_utf8(raw).ok()?;
    let mut chars = text.chars();
    let mut value = String::new();
    while let Some(c) = chars.next() {
        if c != '\\' {
            value.push(c);
            continue;
        }
        let next = chars.next()?;
        if "acCefMn\"'\\tbruvx01234567".contains(next) {
            value.push('\\')
        }
        value.push(next);
    }
    let quote = if value.contains(['\"', '\'']) || (single_quotes && !value.contains('\\')) {
        '\''
    } else {
        '\"'
    };
    if quote == '\'' {
        value = value.replace("\\\"", "\"");
        let mut result = String::new();
        let mut slashes = 0;
        for c in value.chars() {
            if c == '\'' && slashes % 2 == 0 {
                result.push('\\');
            }
            result.push(c);
            slashes = if c == '\\' { slashes + 1 } else { 0 };
        }
        value = result;
    } else {
        let mut result = String::new();
        let mut chars = value.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '#' && chars.peek().is_some_and(|c| "{@$".contains(*c)) {
                result.push('\\');
            }
            result.push(c);
        }
        value = result;
    }
    Some(format!("{quote}{value}{quote}"))
}
// String-pattern scan/sub/gsub retain match state for this narrow literal subset.
// Other methods, backslash patterns, blocks and unknown dispatch are detection-only.
pub(super) fn trusted_call(call: &ruby_prism::CallNode<'_>) -> bool {
    let method = call.name().as_slice();
    if !matches!(method, b"scan" | b"sub" | b"sub!" | b"gsub" | b"gsub!")
        || call.block().is_some()
        || !call
            .receiver()
            .and_then(|n| n.as_string_node())
            .is_some_and(|n| n.unescaped().is_ascii())
    {
        return false;
    }
    let args = ast::call_arguments(call);
    if args.len() != if method == b"scan" { 1 } else { 2 } {
        return false;
    }
    let string = args[0].as_string_node();
    let pattern = if let Some(r) = args[0].as_regular_expression_node() {
        if r.opening_loc().as_slice() != b"/" || r.closing_loc().as_slice() != b"/" {
            return false;
        }
        r.content_loc().as_slice()
    } else if let Some(s) = &string {
        s.unescaped()
    } else {
        return false;
    };
    !pattern.is_empty()
        && pattern.iter().all(u8::is_ascii_alphanumeric)
        && (args.len() == 1
            || args[1]
                .as_string_node()
                .is_some_and(|s| s.unescaped().is_ascii() && !s.unescaped().contains(&b'\\')))
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop redundant_regexp_argument_spec.rb (MIT, Copyright
    //! (c) 2012-26 Bozhidar Batsov), with fixed double-quote policy.
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/RedundantRegexpArgument")
            .collect()
    }
    #[test]
    fn pinned_methods_and_exact_diagnostics() {
        assert_eq!(offenses("'foo'.split(/\u{000b}/)").len(), 1);
        for method in [
            "byteindex",
            "byterindex",
            "gsub",
            "gsub!",
            "partition",
            "rpartition",
            "scan",
            "split",
            "start_with?",
            "sub",
            "sub!",
        ] {
            for op in [".", "&."] {
                let s = format!("\n  'foo'{op}{method}(/f/)\n");
                let found = offenses(&s);
                assert_eq!(found.len(), 1, "{s}");
                assert_eq!(
                    (found[0].line, found[0].col),
                    (2, (9 + op.len() + method.len()) as u32)
                );
                assert_eq!(
                    found[0].message,
                    "Use string `\"f\"` as argument instead of regexp `/f/`."
                );
            }
        }
    }
    #[test]
    fn pinned_no_offense_lookalikes() {
        for s in [
            "hash[/regexp/]",
            "hash[/regexp/] = value",
            "'x'.split('x')",
            "'x'.split(/x+/)",
            "'x'.split(/x/i)",
            "'x'.split(/x/x)",
            "'x'.split(/ /)",
            "'x'.split(/#{x}/)",
            "'x'.split(/(x)/)",
            "'x'.split(/\\1/)",
            "'x'.split(/x/n)",
            "'x'.split(/x/u)",
            "'x'.split(%r{x})",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn canonical_match_state_preserving_literal_corrections() {
        for method in ["scan", "sub", "gsub", "sub!", "gsub!"] {
            let tail = if method == "scan" { "" } else { ", \"x\"" };
            let s = format!("\"foo\".{method}(/f/{tail})");
            let after = format!("\"foo\".{method}(\"f\"{tail})");
            assert_correction(&RedundantRegexpArgument::default(), &s, &after);
        }
    }
    #[test]
    fn single_quotes_of_string_literals_are_followed_where_the_string_allows_both() {
        // RuboCop's examples for `Style/StringLiterals` with `single_quotes`.
        let cop = RedundantRegexpArgument {
            single_quotes: true,
        };
        for (source, prefer) in [
            ("'foo'.split(/f/)", "'f'"),
            // A backslash escape needs double quotes, and a quote single ones.
            ("'foo'.split(/\\n/)", "\"\\n\""),
            ("'foo'.split(/\"/)", "'\"'"),
        ] {
            let found = correction_offenses(&cop, source);
            let message = format!(
                "Use string `{prefer}` as argument instead of regexp `{}`.",
                &source[12..source.len() - 1]
            );
            assert_eq!(found[0].message, message, "{source}");
        }
        assert_correction(&cop, "\"foo\".scan(/f/)", "\"foo\".scan('f')");
    }
    #[test]
    fn unknown_dispatch_match_state_and_blocks_stay_reported_without_edits() {
        for s in [
            "str.gsub(/f/, 'x')",
            "Hook; 'foo'.scan(/f/)",
            "'foo'.split(/f/)",
            "'foo'.partition(/f/)",
            "'foo'.start_with?(/f/)",
            "'foo'.scan(/f/) { $~ }",
            "'foo'.gsub(/f/, replacement)",
            "class String; def sub(*); end; end; 'foo'.sub(/f/, 'x')",
            "'foo'.scan(/\\n/)",
        ] {
            let found = correction_offenses(&RedundantRegexpArgument::default(), s);
            assert_eq!(found.len(), 1, "{s}");
            assert!(found[0].correction.is_none(), "{s}");
        }
    }
}
