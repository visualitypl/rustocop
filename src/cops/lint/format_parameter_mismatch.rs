//! `Lint/FormatParameterMismatch` — flags `format`, `sprintf` and
//! `String#%` calls whose argument count does not match the format string's
//! fields, and format strings that mix numbered (`%1$s`), named (`%<a>s`,
//! `%{a}`) and unnumbered (`%s`) sequences.
//!
//! Matches RuboCop's semantics: only literal format strings are checked
//! (`format`/`sprintf` without a receiver or on `Kernel`, with at least one
//! argument after the string; `%` on a string literal, counted only when the
//! right-hand side is an array literal). Sequences are found in the string's
//! source text, as RuboCop's `FormatString` does: `%%` takes no argument,
//! each `*` one more, named sequences mean a single hash argument, and
//! `%N$` references need N arguments. Heredoc format strings, splats among
//! `format` arguments, and interpolated strings or arrays with no fields are
//! skipped. The offense is reported at the method name (or `%` operator).
//!
//! Two deliberate deviations from RuboCop:
//!
//! - `"%s %s %s" % [a, *rest]`: RuboCop counts the splat as one argument and
//!   reports a mismatch; here an array with a splat is only reported when its
//!   other elements alone outnumber the fields.
//! - A string mixing `%N$` references with `%%` (`format('%1$s %%', a)`)
//!   makes RuboCop crash; here `%%` is ignored as usual.

use ruby_prism::{CallNode, Node};

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/FormatParameterMismatch` cop.
pub struct FormatParameterMismatch;

impl Cop for FormatParameterMismatch {
    fn name(&self) -> &'static str {
        "Lint/FormatParameterMismatch"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        let Some(message) = check_call(&call) else {
            return;
        };
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            end_line: line,
            col,
        });
    }
}

/// Which formatting method a call is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    Format,
    Sprintf,
    Percent,
}

/// The offense message for a format call, if it offends.
fn check_call(call: &CallNode<'_>) -> Option<String> {
    let method = match call.name().as_slice() {
        b"format" => Method::Format,
        b"sprintf" => Method::Sprintf,
        b"%" => Method::Percent,
        _ => return None,
    };
    if call.is_safe_navigation() {
        return None;
    }
    let receiver = call.receiver();
    // The arguments as RuboCop sees them: a block argument is the last one.
    let mut arguments: Vec<Node<'_>> = call
        .arguments()
        .map(|arguments| arguments.arguments().iter().collect())
        .unwrap_or_default();
    if let Some(block) = call
        .block()
        .filter(|block| block.as_block_argument_node().is_some())
    {
        arguments.push(block);
    }
    let first = arguments.first();

    // RuboCop's `called_on_string?`.
    let called_on_string = match &receiver {
        None => first.is_some_and(is_string),
        Some(receiver) if is_const(receiver) => first.is_some_and(is_string),
        Some(receiver) => is_string(receiver),
    };
    if !called_on_string {
        return None;
    }

    let format_method =
        method != Method::Percent && is_format_method(receiver.as_ref(), &arguments);
    let percent = method == Method::Percent && {
        let receiver_is_string = receiver.as_ref().is_some_and(is_string);
        let percent = receiver_is_string || first.is_some_and(is_array);
        percent && !(receiver_is_string && first.is_some_and(is_heredoc))
    };
    if !format_method && !percent {
        return None;
    }

    // The format string: the first argument of `format`, the receiver of `%`.
    let format_string = if format_method {
        first?
    } else {
        receiver.as_ref()?
    };
    let format = FormatString::parse(format_string.location().as_slice());
    if mixes_sequence_kinds(&format) {
        return Some(
            "Format string is invalid because formatting sequence types (numbered, named or \
             unnumbered) are mixed."
                .to_string(),
        );
    }

    let first = first?;
    // The arguments passed, and how many of them are not splats.
    let (passed, fixed) = if format_method {
        let splat = arguments[1..]
            .iter()
            .any(|argument| matches!(argument, Node::SplatNode { .. }));
        if splat || is_heredoc(first) {
            return None;
        }
        (arguments.len() - 1, arguments.len() - 1)
    } else {
        let elements = first.as_array_node()?.elements();
        let fixed = elements
            .iter()
            .filter(|element| !matches!(element, Node::SplatNode { .. }))
            .count();
        (elements.iter().count(), fixed)
    };
    let expected = expected_fields(&format);
    if expected == 0
        && matches!(
            first,
            Node::InterpolatedStringNode { .. } | Node::ArrayNode { .. }
        )
    {
        return None;
    }
    let mismatch = if fixed < passed {
        // The splats may expand to the missing arguments.
        fixed > expected
    } else {
        passed != expected
    };
    if !mismatch {
        return None;
    }
    let method_name = match method {
        Method::Format => "format",
        Method::Sprintf => "sprintf",
        Method::Percent => "String#%",
    };
    Some(format!(
        "Number of arguments ({passed}) to `{method_name}` doesn't match the number of fields \
         ({expected})."
    ))
}

/// RuboCop's `format_method?`: `format`/`sprintf` with no receiver (or on
/// `Kernel`, or a string), more than one argument, and a string first.
fn is_format_method(receiver: Option<&Node<'_>>, arguments: &[Node<'_>]) -> bool {
    if let Some(receiver) = receiver
        && is_const(receiver)
        && short_const_name(receiver) != Some(b"Kernel".as_slice())
    {
        return false;
    }
    arguments.len() > 1 && is_string(&arguments[0])
}

/// The number of arguments a format string expects.
fn expected_fields(format: &FormatString) -> usize {
    if format.sequences.iter().any(|sequence| sequence.named) {
        return 1;
    }
    let max_reference = format
        .sequences
        .iter()
        .filter_map(|sequence| sequence.max_reference)
        .max();
    if let Some(max_reference) = max_reference.filter(|&max| max != 0) {
        return usize::try_from(max_reference).unwrap_or(usize::MAX);
    }
    format
        .sequences
        .iter()
        .filter(|sequence| !sequence.percent)
        .map(|sequence| sequence.arity)
        .sum()
}

/// RuboCop's `FormatString#valid?`, negated: whether named, numbered and
/// unnumbered sequences are mixed (`%%` aside).
fn mixes_sequence_kinds(format: &FormatString) -> bool {
    let mut kinds = format
        .sequences
        .iter()
        .filter(|sequence| !sequence.percent)
        .map(|sequence| {
            if sequence.named {
                0
            } else if sequence.max_reference.is_some() {
                1
            } else {
                2
            }
        });
    let Some(first) = kinds.next() else {
        return false;
    };
    kinds.any(|kind| kind != first)
}

/// RuboCop's `str`/`dstr`: a string literal, interpolated or not
/// (`__FILE__` is a string in its AST).
fn is_string(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::StringNode { .. } | Node::InterpolatedStringNode { .. } | Node::SourceFileNode { .. }
    )
}

fn is_array(node: &Node<'_>) -> bool {
    matches!(node, Node::ArrayNode { .. })
}

fn is_const(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::ConstantReadNode { .. } | Node::ConstantPathNode { .. }
    )
}

/// The last segment of a constant reference.
fn short_const_name<'pr>(node: &Node<'pr>) -> Option<&'pr [u8]> {
    match node {
        Node::ConstantReadNode { .. } => {
            Some(node.as_constant_read_node().unwrap().name().as_slice())
        }
        Node::ConstantPathNode { .. } => node
            .as_constant_path_node()
            .unwrap()
            .name()
            .map(|name| name.as_slice()),
        _ => None,
    }
}

/// RuboCop's `heredoc?`: the node's source starts with `<<` (a heredoc's
/// node spans only its opening).
fn is_heredoc(node: &Node<'_>) -> bool {
    node.location().as_slice().starts_with(b"<<")
}

/// The format sequences of a format string's source text: a port of
/// RuboCop's `Utils::FormatString`.
pub struct FormatString {
    sequences: Vec<Sequence>,
}

/// One format sequence, e.g. `%-10s`, `%<name>d`, `%{name}`, `%%`.
#[derive(Debug, PartialEq, Eq)]
struct Sequence {
    /// `%%`.
    percent: bool,
    /// Named, `%<name>…` or `%{name}`.
    named: bool,
    /// Arguments consumed: one, plus one per `*`.
    arity: usize,
    /// The largest `N$` reference in the sequence.
    max_reference: Option<u64>,
}

impl FormatString {
    /// Scan `source` for sequences, left to right, like Ruby's `String#scan`
    /// with RuboCop's `SEQUENCE` regexp.
    pub fn parse(source: &[u8]) -> Self {
        let chars: Vec<char> = String::from_utf8_lossy(source).chars().collect();
        let mut sequences = Vec::new();
        let mut start = 0;
        while start < chars.len() {
            if chars[start] == '%'
                && let Some(matched) = match_sequence(&chars, start)
            {
                let text = &chars[start..matched.end];
                sequences.push(Sequence {
                    percent: matched.percent,
                    named: matched.named,
                    arity: text.iter().filter(|&&c| c == '*').count() + 1,
                    max_reference: max_reference(text),
                });
                start = matched.end;
                continue;
            }
            start += 1;
        }
        FormatString { sequences }
    }
}

/// A sequence match: where it ends and what it captured.
struct Match {
    end: usize,
    percent: bool,
    named: bool,
}

/// RuboCop's `SEQUENCE` regexp, anchored at a `%`:
///
/// ```text
///   % (?<type>%)
/// | % FLAG*
///   (?: (?: WIDTH? PRECISION? NAME? | WIDTH? NAME PRECISION?
///         | NAME FLAG* WIDTH? PRECISION? ) TYPE
///     | WIDTH? PRECISION? TEMPLATE_NAME )
/// ```
///
/// Alternatives and greedy repetitions are tried in the regexp engine's
/// order, so the first match found is the one Ruby finds.
fn match_sequence(chars: &[char], start: usize) -> Option<Match> {
    let at = start + 1;
    if chars.get(at) == Some(&'%') {
        return Some(Match {
            end: at + 1,
            percent: true,
            named: false,
        });
    }
    let typed = |end: usize, named: bool| {
        chars
            .get(end)
            .is_some_and(|&c| "bBdiouxXeEfgGaAcps".contains(c))
            .then_some(Match {
                end: end + 1,
                percent: false,
                named,
            })
    };
    for flags in flag_ends(chars, at) {
        // WIDTH? PRECISION? NAME? TYPE
        for width in optional(number(chars, flags), flags) {
            for precision in optional(precision(chars, width), width) {
                for name in optional(name(chars, precision), precision) {
                    if let Some(matched) = typed(name, name != precision) {
                        return Some(matched);
                    }
                }
            }
        }
        // WIDTH? NAME PRECISION? TYPE
        for width in optional(number(chars, flags), flags) {
            for name in name(chars, width) {
                for precision in optional(precision(chars, name), name) {
                    if let Some(matched) = typed(precision, true) {
                        return Some(matched);
                    }
                }
            }
        }
        // NAME FLAG* WIDTH? PRECISION? TYPE
        for name in name(chars, flags) {
            for more_flags in flag_ends(chars, name) {
                for width in optional(number(chars, more_flags), more_flags) {
                    for precision in optional(precision(chars, width), width) {
                        if let Some(matched) = typed(precision, true) {
                            return Some(matched);
                        }
                    }
                }
            }
        }
        // WIDTH? PRECISION? TEMPLATE_NAME
        for width in optional(number(chars, flags), flags) {
            for precision in optional(precision(chars, width), width) {
                if let Some(end) = template_name(chars, precision) {
                    return Some(Match {
                        end,
                        percent: false,
                        named: true,
                    });
                }
            }
        }
    }
    None
}

/// The ends of a greedy `X?`: X's ends, then not matching it.
fn optional(ends: Vec<usize>, at: usize) -> Vec<usize> {
    let mut ends = ends;
    ends.push(at);
    ends
}

/// The ends of `FLAG*` (`FLAG = [ #0+-] | \d+\$`) from `at`, in the order a
/// backtracking engine tries them (longest first), without duplicates.
fn flag_ends(chars: &[char], at: usize) -> Vec<usize> {
    // Post-order depth-first search over flag matches: a position's
    // continuations come before the position itself.
    let mut ends = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![(at, false)];
    while let Some((position, expanded)) = stack.pop() {
        if expanded {
            ends.push(position);
            continue;
        }
        if !seen.insert(position) {
            continue;
        }
        stack.push((position, true));
        // Pushed in reverse so the first alternative is explored first.
        if let Some(end) = digit_dollar(chars, position) {
            stack.push((end, false));
        }
        if chars.get(position).is_some_and(|&c| " #0+-".contains(c)) {
            stack.push((position + 1, false));
        }
    }
    ends
}

/// `\d+\$` at `at`: the end, if it matches.
fn digit_dollar(chars: &[char], at: usize) -> Option<usize> {
    let digits = digit_run(chars, at);
    (digits > 0 && chars.get(at + digits) == Some(&'$')).then_some(at + digits + 1)
}

/// `NUMBER = \d+ | \*(\d+\$)? | \#\{.*?\}` at `at`: its ends, in order.
fn number(chars: &[char], at: usize) -> Vec<usize> {
    let digits = digit_run(chars, at);
    if digits > 0 {
        return (1..=digits).rev().map(|length| at + length).collect();
    }
    match chars.get(at) {
        Some('*') => {
            let mut ends = Vec::new();
            if let Some(end) = digit_dollar(chars, at + 1) {
                ends.push(end);
            }
            ends.push(at + 1);
            ends
        }
        Some('#') if chars.get(at + 1) == Some(&'{') => chars[at + 2..]
            .iter()
            .take_while(|&&c| c != '\n')
            .enumerate()
            .filter(|&(_, &c)| c == '}')
            .map(|(offset, _)| at + 2 + offset + 1)
            .collect(),
        _ => Vec::new(),
    }
}

/// `PRECISION = \.NUMBER?` at `at`.
fn precision(chars: &[char], at: usize) -> Vec<usize> {
    if chars.get(at) != Some(&'.') {
        return Vec::new();
    }
    optional(number(chars, at + 1), at + 1)
}

/// `NAME = <\w+>` at `at`.
fn name(chars: &[char], at: usize) -> Vec<usize> {
    if chars.get(at) != Some(&'<') {
        return Vec::new();
    }
    let word = word_run(chars, at + 1);
    if word > 0 && chars.get(at + 1 + word) == Some(&'>') {
        vec![at + 1 + word + 1]
    } else {
        Vec::new()
    }
}

/// `TEMPLATE_NAME = (?<!\#)\{\w+\}` at `at`.
fn template_name(chars: &[char], at: usize) -> Option<usize> {
    if chars.get(at) != Some(&'{') || (at > 0 && chars[at - 1] == '#') {
        return None;
    }
    let word = word_run(chars, at + 1);
    (word > 0 && chars.get(at + 1 + word) == Some(&'}')).then_some(at + 1 + word + 1)
}

/// The length of the run of ASCII digits (Ruby's `\d`) at `at`.
fn digit_run(chars: &[char], at: usize) -> usize {
    chars.get(at..).map_or(0, |rest| {
        rest.iter().take_while(|c| c.is_ascii_digit()).count()
    })
}

/// The length of the run of word characters (Ruby's `\w`) at `at`.
fn word_run(chars: &[char], at: usize) -> usize {
    chars.get(at..).map_or(0, |rest| {
        rest.iter()
            .take_while(|&&c| c.is_alphanumeric() || c == '_')
            .count()
    })
}

/// The largest `N` of the `N$` references in a sequence's text.
fn max_reference(text: &[char]) -> Option<u64> {
    let mut max = None;
    let mut i = 0;
    while i < text.len() {
        let digits = digit_run(text, i);
        if digits == 0 {
            i += 1;
            continue;
        }
        if text.get(i + digits) == Some(&'$') {
            let value = text[i..i + digits].iter().fold(0u64, |acc, c| {
                acc.saturating_mul(10)
                    .saturating_add(u64::from(c.to_digit(10).unwrap_or(0)))
            });
            max = max.max(Some(value));
        }
        i += digits;
    }
    max
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/format_parameter_mismatch_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/FormatParameterMismatch")
            .collect()
    }

    /// (line, col, message) of each offense.
    fn found(source: &str) -> Vec<(u32, u32, String)> {
        offenses(source)
            .into_iter()
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    fn msg(passed: usize, method: &str, expected: usize) -> String {
        format!(
            "Number of arguments ({passed}) to `{method}` doesn't match the number of fields \
             ({expected})."
        )
    }

    const INVALID: &str = "Format string is invalid because formatting sequence types \
                           (numbered, named or unnumbered) are mixed.";

    #[test]
    fn flags_rubocop_spec_offenses() {
        for (source, expected) in [
            ("Kernel.format(\"%s %s\", 1)", (1, 8, msg(1, "format", 2))),
            ("Kernel.sprintf(\"%s %s\", 1)", (1, 8, msg(1, "sprintf", 2))),
            ("format(\"%s %s\", 1)", (1, 1, msg(1, "format", 2))),
            ("format(\"something\", 1)", (1, 1, msg(1, "format", 0))),
            ("format(\"%s %s\", 1, 2, 3)", (1, 1, msg(3, "format", 2))),
            ("sprintf(\"%s %s\", 1, 2, 3)", (1, 1, msg(3, "sprintf", 2))),
            ("\"%s %s\" % [1, 2, 3]", (1, 9, msg(3, "String#%", 2))),
            (
                "puts \"%s, %s, %s\" % [1, 2, 3, 4, *arr]",
                (1, 19, msg(5, "String#%", 3)),
            ),
            (
                "format('%1$s %2$s', 'foo', 'bar', 'baz')",
                (1, 1, msg(3, "format", 2)),
            ),
            (
                "format('%s %2$s', 'foo', 'bar')",
                (1, 1, INVALID.to_string()),
            ),
            ("format(\"%*d\", id)", (1, 1, msg(1, "format", 2))),
            ("format(\"<< %s bleh\", 1, 2)", (1, 1, msg(2, "format", 1))),
            (
                "params = { y: '2015', m: '01', d: '01' }\nputs format('%{y}-%{m}-%{d}', 2015, 1, 1)",
                (2, 6, msg(3, "format", 1)),
            ),
            (
                "params = { y: '2015', m: '01', d: '01' }\nputs format('%<y>d-%<m>d-%<d>d', 2015, 1, 1)",
                (2, 6, msg(3, "format", 1)),
            ),
            (
                "format(\"#{foo} %s %s\", \"bar\")",
                (1, 1, msg(1, "format", 2)),
            ),
            (
                "\"%s %s\" % [\"#{foo}\", 1, 2]",
                (1, 9, msg(3, "String#%", 2)),
            ),
            ("format(\"%s %s\", \"#{foo}\")", (1, 1, msg(1, "format", 2))),
            (
                "\"#{foo} %s %s\" % [1, 2, 3]",
                (1, 16, msg(3, "String#%", 2)),
            ),
        ] {
            assert_eq!(found(source), vec![expected], "for {source:?}");
        }
    }

    #[test]
    fn accepts_rubocop_spec_no_offenses() {
        let mut sources = Vec::new();
        for variable in ["CONST", "var", "@var", "@@var", "$var"] {
            sources.push(format!("{variable} = '%s'\n{variable} % [foo]\n"));
            sources.push(format!("{variable} = '%s'\nformat({variable}, foo)\n"));
            sources.push(format!("{variable} = '%s'\nsprintf({variable}, foo)\n"));
        }
        for source in [
            "format(\"%s %d %i\", 1, 2, 3)",
            "format('%s %s %% %s %%%% %%%%%% %%5B', 1, 2, 3)",
            "format(A_CONST, 1, 2, 3)",
            "sprintf(\"%020x%+g:% g %%%#20.8x %#.0e\", 1, 2, 3, 4, 5)",
            "\"%s %s\" % [1, 2]",
            "puts \"%s\" % {\"a\" => 1}",
            "puts \"%s\" % CONST",
            "sprintf(\"%s, %s, %s\", 1, *arr)",
            "puts format(\"%s, %s, %s\", 1, 2, 3, 4, *arr)",
            "puts sprintf(\"%s, %s, %s\", 1, 2, 3, 4, *arr)",
            "puts \"%s %s\" % var",
            "puts \"%s %s\" % (\"ab\".chars)",
            "format('%1$s %2$s', 'foo', 'bar')",
            "format('%1$s %1$s', 'foo')",
            "puts str % [1, 2]",
            "'%' % []",
            "format('%<t>s', t: '%d')",
            "format(\"%0.1f%% percent\", 22.5)",
            "format(\"%*d\", max_width, id)",
            "format(\"%0*x\", max_width, id)",
            "format(\"%s\", \"a b c\".gsub(\" \", \"_\"))",
            "\"foo %{bar} baz\" % { bar: 42 }",
            "format(\"%%%<hex>02X\", hex: 10)",
            "\"duration: %10.fms\" % 42",
            "sprintf(\"%d%d\", *test)",
            "format(\"%d%d\", *test)",
            "format(\"%.d\", 0)",
            "params = { y: '2015', m: '01', d: '01' }\nputs format('%{y}-%{m}-%{d}', params)",
            "params = { y: '2015', m: '01', d: '01' }\nputs format('%<y>d-%<m>d-%<d>d', params)",
            "format(\"%*d\", 10, 3)",
            "format(\"%.*f\", 2, 20.19)",
            "format(\"%*.*f\", 10, 3, 20.19)",
            "format(\"%*.*f %*.*f\", 10, 2, 20.19, 5, 1, 11.22)",
            "format(\"#{foo} %s\", \"bar\")",
            "format(\"#{foo}\", \"bar\", \"baz\")",
            "Kernel.format(\"%.#{number_of_decimal_places}f\", num)",
            "\"%s %s\" % [\"#{foo}\", 1]",
            "\"#{foo}\" % [1, 2]",
            "format(\"%#{padding}s: %s\", prefix, message)",
            "sprintf(\"| %-#{key_offset}s | %-#{val_offset}s |\", key, value)",
            "format(\"%s\", \"#{foo}\")",
            "\"#{foo} %s %s\" % [1, 2]",
        ] {
            sources.push(source.to_string());
        }
        for source in &sources {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn heredocs_are_not_counted() {
        for source in [
            "format(<<~EOS, 1, 2)\n  %s\nEOS\n",
            "puts \"%s\" % <<~EOS\n  hi\nEOS\n",
            "<<~EOS % [1, 2]\n  %s\nEOS\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn follows_rubocop_on_receivers_and_arguments() {
        // Any receiver but a constant other than `Kernel` (even a string).
        assert_eq!(
            found("::Kernel.format('%s', 1, 2)"),
            vec![(1, 10, msg(2, "format", 1))]
        );
        assert_eq!(
            found("\"x\".format('%s', 1, 2)"),
            vec![(1, 5, msg(2, "format", 1))]
        );
        assert!(found("Foo.format('%s', 1, 2)").is_empty());
        assert!(found("foo.format('%s', 1, 2)").is_empty());
        // A block argument is an argument; a double splat is one hash.
        assert_eq!(
            found("format('%s', a, &b)"),
            vec![(1, 1, msg(2, "format", 1))]
        );
        assert!(found("format('%s', **opts)").is_empty());
        // Format sequences are found in the source text, interpolation too.
        assert_eq!(
            found("format('%-#{key_offset}s', 1, 2)"),
            vec![(1, 1, msg(2, "format", 1))]
        );
        assert_eq!(
            found("format(\"%s\" \"%s\", 1)"),
            vec![(1, 1, msg(1, "format", 2))]
        );
        assert_eq!(
            found("format('%0$s', 1, 2)"),
            vec![(1, 1, msg(2, "format", 1))]
        );
        assert_eq!(
            found("format('%<a>s %s', 1, 2)"),
            vec![(1, 1, INVALID.to_string())]
        );
        assert_eq!(
            found("format('%{a}', 1, 2)"),
            vec![(1, 1, msg(2, "format", 1))]
        );
        assert_eq!(
            found("x = [\n  format('%s %s', a),\n]\n"),
            vec![(2, 3, msg(1, "format", 2))]
        );
    }

    #[test]
    fn splat_in_percent_array_only_flags_too_many_arguments() {
        // Deviation: RuboCop also reports this one.
        assert!(found("\"%s %s %s\" % [1, *arr]").is_empty());
        assert!(found("\"%s %s %s\" % [1, 2, 3, *arr]").is_empty());
        assert_eq!(
            found("\"%s\" % [1, 2, *arr]"),
            vec![(1, 6, msg(3, "String#%", 1))]
        );
    }

    #[test]
    fn numbered_references_ignore_literal_percents() {
        // RuboCop crashes on these.
        assert!(found("format('%1$s %%', a)").is_empty());
        assert_eq!(
            found("format('%2$s %%', a)"),
            vec![(1, 1, msg(1, "format", 2))]
        );
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "format('%s', 1)",
            "format('%s')",
            "format",
            "sprintf(*args)",
            "\"%s\" % [1]",
            "\"%s\"&.%([1, 2])",
            "x % [1, 2]",
            "5 % 3",
            "foo.sprintf('%s %s', 1)",
            "printf('%s %s', 1)",
            "format('%s %s', 1, 2) { }",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    /// (percent, named, arity, max reference) of each sequence.
    fn sequences(source: &str) -> Vec<(bool, bool, usize, Option<u64>)> {
        FormatString::parse(source.as_bytes())
            .sequences
            .into_iter()
            .map(|s| (s.percent, s.named, s.arity, s.max_reference))
            .collect()
    }

    #[test]
    fn parses_format_sequences() {
        let plain = (false, false, 1, None);
        assert_eq!(sequences("'%s %d'"), vec![plain, plain]);
        assert_eq!(sequences("%%"), vec![(true, false, 1, None)]);
        assert_eq!(sequences("%*.*f"), vec![(false, false, 3, None)]);
        assert_eq!(sequences("%1$*2$d"), vec![(false, false, 2, Some(2))]);
        assert_eq!(sequences("%01$s"), vec![(false, false, 1, Some(1))]);
        assert_eq!(sequences("%<a>s %{b}"), vec![(false, true, 1, None); 2]);
        assert_eq!(sequences("%-10<a>s"), vec![(false, true, 1, None)]);
        assert_eq!(sequences("%<a>-10.2f"), vec![(false, true, 1, None)]);
        assert_eq!(sequences("%-#{w}s"), vec![plain]);
        assert_eq!(sequences("%#{w}s"), vec![plain]);
        // `#{` is not a template; a bare `%` or unknown type is no sequence.
        assert_eq!(sequences("%#{a}"), vec![]);
        assert_eq!(sequences("100%"), vec![]);
        assert_eq!(sequences("%y %"), vec![]);
        // `% d` is a sequence (space flag).
        assert_eq!(sequences("100% done"), vec![plain]);
        // A long run of flags does not blow up.
        let long = format!("%{}s", "0".repeat(5000));
        assert_eq!(sequences(&long), vec![plain]);
        let long = format!("%{}", "0$".repeat(5000));
        assert_eq!(sequences(&long), vec![]);
    }
}
