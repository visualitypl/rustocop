//! `Style/FormatStringToken` — fixed annotated, aggressive, maximum one unnamed token.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use regex::bytes::Regex;
use ruby_prism::{CallNode, Node};
use std::{collections::HashSet, sync::LazyLock};

// Sequence grammar ported from RuboCop's Utils::FormatString (MIT License,
// Copyright (c) 2012-26 Bozhidar Batsov). ASCII word names/digits deliberately
// match Ruby's \\w/\\d, not Unicode regex classes.
pub(super) static TOKENS: LazyLock<Regex> = LazyLock::new(|| {
    let flag = r"(?:[ #0+\-]|[0-9]+\$)";
    let number = r"(?:[0-9]+|\*(?:[0-9]+\$)?|\#\{[^\n]*?\})";
    let width = format!("{number}?");
    let precision = format!(r"(?:\.{number}?)?");
    // Ruby excludes a template name immediately preceded by '#'. Only flags
    // can end in '#', so constrain their final byte when neither width nor
    // precision separates them from the name. Filtering after a match would
    // lose alternatives such as %#{w}{x}, where '#{w}' is the width.
    let template_flags = format!(r"(?:{flag}*(?:[ 0+\-]|[0-9]+\$))?");
    let template_prefix =
        format!(r"(?:{flag}*(?:{number}{precision}|\.{number}?)|{template_flags})");
    Regex::new(&format!(r"(?-u)%%|%(?:{flag}*(?:{width}{precision}(?P<name_after><[A-Za-z0-9_]+>)?|{width}(?P<name_middle><[A-Za-z0-9_]+>){precision}|(?P<name_before><[A-Za-z0-9_]+>){flag}*{width}{precision})[bBdiouxXeEfgGaAcps]|{template_prefix}(?P<template>\{{[A-Za-z0-9_]+\}}))")).unwrap()
});

fn typical(call: &CallNode<'_>, string: &Node<'_>) -> bool {
    if call.is_safe_navigation() {
        return false;
    }
    let Some(arguments) = call.arguments() else {
        return false;
    };
    let arguments = arguments.arguments();
    let same = |n: &Node<'_>| {
        n.location().start_offset() == string.location().start_offset()
            && n.location().end_offset() == string.location().end_offset()
    };
    if call.name().as_slice() == b"%" {
        return call.receiver().is_some_and(|r| same(&r)) && arguments.len() == 1;
    }
    matches!(call.name().as_slice(), b"format" | b"sprintf" | b"printf")
        && arguments.iter().next().as_ref().is_some_and(same)
}

fn direct_context(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    cx.ancestors()
        .find(|p| p.as_arguments_node().is_none())
        .and_then(|p| p.as_call_node())
        .is_some_and(|c| typical(&c, node))
}
pub struct FormatStringToken;
impl Cop for FormatStringToken {
    fn name(&self) -> &'static str {
        "Style/FormatStringToken"
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
        let Some(string) = node.as_string_node() else {
            return;
        };
        if !string.unescaped().contains(&b'%')
            || cx.ancestors().any(|n| {
                matches!(
                    n,
                    Node::XStringNode { .. }
                        | Node::InterpolatedXStringNode { .. }
                        | Node::RegularExpressionNode { .. }
                        | Node::InterpolatedRegularExpressionNode { .. }
                )
            })
        {
            return;
        }
        let content = string.content_loc();
        let bytes = content.as_slice();
        let context = direct_context(node, cx);
        // Detection's permissive upstream grammar is not a proof of Ruby's
        // argument mode. Prove a conservative named-only subset during this
        // same scan: every percent must belong to %% or a complete named token.
        // In particular, '*' and '$' can consume positional arguments even in
        // an annotated token. Unknown/malformed directives withhold ALL edits,
        // in either order, without changing detection or reparsing candidates.
        let mut named_only = cx.corrections_requested();
        let mut covered = 0;
        let detections: Vec<_> = TOKENS
            .captures_iter(bytes)
            .filter_map(|caps| {
                let token = caps.get(0).unwrap();
                named_only = named_only && !bytes[covered..token.start()].contains(&b'%');
                covered = token.end();
                if token.as_bytes() == b"%%" {
                    return None;
                }
                let template = caps.name("template");
                named_only = named_only
                    && if let Some(template) = template {
                        // Only plain %{name}, not flags/widths before the braces.
                        template.start() == token.start() + 1
                    } else {
                        // %<name> followed by static flags/width/precision/type.
                        // The captured grammar bounds these bytes; exclude dynamic
                        // widths/precision and all numbered-argument syntax.
                        caps.name("name_before").is_some_and(|name| {
                            name.start() == token.start() + 1
                                && !token
                                    .as_bytes()
                                    .iter()
                                    .any(|b| matches!(b, b'*' | b'$' | b'{' | b'}'))
                        })
                    };
                let named = template.is_some()
                    || ["name_after", "name_middle", "name_before"]
                        .iter()
                        .any(|name| caps.name(name).is_some());
                let annotated = named && token.as_bytes().contains(&b'<');
                (template.is_some() || annotated || context).then_some((
                    token,
                    template.is_some(),
                    annotated,
                ))
            })
            .collect();
        if detections.len() <= 1 && detections.iter().all(|(_, t, a)| !t && !a) {
            return;
        }
        named_only = named_only && !bytes[covered..].contains(&b'%');
        let keys = (named_only && detections.iter().any(|(_, t, _)| *t))
            .then(|| correction_context(node, cx, file))
            .flatten();
        for (token, template, annotated) in detections {
            if annotated {
                continue;
            }
            let start = content.start_offset() + token.start();
            let end = content.start_offset() + token.end();
            let (line, col) = file.line_col(start);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: format!(
                    "Prefer annotated tokens (like `%<foo>s`) over {}.",
                    if template {
                        "template tokens (like `%{foo}`)"
                    } else {
                        "unannotated tokens (like `%s`)"
                    }
                ),
                line,
                col,
                end_line: file.line_of(end.saturating_sub(1)),
                correction: keys
                    .filter(|_| template)
                    .and_then(|keys| correction(keys, cx, file, start..end)),
            });
        }
    }
}
/// A fragment is not a whole format: interpolation/implicit concatenation can
/// supply a preceding '%' that escapes the apparent directive. Never edit such
/// fragments, decoded escapes, modifiers or data consumed by unknown dispatch.
fn correction_context<'f>(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &'f SourceFile,
) -> Option<&'f HashSet<Vec<u8>>> {
    let string = node.as_string_node()?;
    if string.content_loc().as_slice() != string.unescaped() {
        return None;
    }
    // New ASCII bytes must not become percent-literal closing delimiters.
    // '<...>' is balanced; an unpaired '>' would terminate early.
    if let (Some(open), Some(close)) = (string.opening_loc(), string.closing_loc())
        && close.as_slice() == b">"
        && open.as_slice().last() != Some(&b'<')
    {
        return None;
    }
    let call = cx
        .ancestors()
        .find(|p| p.as_arguments_node().is_none())?
        .as_call_node()?;
    if !typical(&call, node) || call.is_safe_navigation() || call.block().is_some() {
        return None;
    }
    if call.name().as_slice() != b"%"
        && (!matches!(call.name().as_slice(), b"format" | b"sprintf")
            || !call
                .receiver()
                .is_none_or(|r| ast::is_global_const(&r, &["Kernel"])))
    {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    let arguments = call.arguments()?.arguments();
    let hash = if call.name().as_slice() == b"%" {
        if arguments.len() != 1 {
            return None;
        }
        arguments.iter().next()?
    } else {
        if arguments.len() != 2 {
            return None;
        }
        arguments.iter().nth(1)?
    };
    let facts = file.format_string_facts(root);
    // Missing-key errors spell the directive as key{x} versus key<x>.
    // Only a literal symbol-keyed Hash proves this edit cannot change that
    // observable exception. Key sets are indexed once, not scanned per token.
    if !facts.core_before(call.location().start_offset())
        || !facts.closed(&call.as_node())
        || file.heredoc_pending_at(root, call.location().start_offset())
    {
        return None;
    }
    facts.named_keys(&hash)
}
fn correction(
    keys: &HashSet<Vec<u8>>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    range: std::ops::Range<usize>,
) -> Option<Correction> {
    let token = &file.bytes[range.clone()];
    if !token.starts_with(b"%{") || !token.ends_with(b"}") {
        return None;
    }
    let name = std::str::from_utf8(&token[2..token.len() - 1]).ok()?;
    if !keys.contains(name.as_bytes()) {
        return None;
    }
    super::literal_spelling::safe_edit(
        file,
        cx,
        Correction {
            range,
            replacement: format!("%<{name}>s"),
        },
    )
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop format_string_token_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Fixed annotated/aggressive/Max=1.
    use super::*;
    use crate::pipeline::lint_str;
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == FormatStringToken.name())
            .collect()
    }
    #[test]
    fn pinned_template_diagnostics() {
        for (s, line, col) in [
            ("\"%{foo}\"", 1, 2),
            ("redirect(\"%{foo}\")", 1, 11),
            ("\"c#{b}%{template}\"", 1, 7),
            ("format(\"c#{b}%{template}\")", 1, 14),
            ("<<-MSG\n  %{colour} is not valid\nMSG\n", 2, 3),
            ("{ bar: format('%{foo}', foo: 'foo') }", 1, 16),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (line, col), "{s}");
            assert_eq!(
                o[0].message,
                "Prefer annotated tokens (like `%<foo>s`) over template tokens (like `%{foo}`)."
            );
        }
    }
    #[test]
    fn pinned_dual_unannotated_and_flags() {
        for token in [
            "A", "B", "E", "G", "X", "a", "b", "c", "d", "e", "f", "g", "i", "o", "p", "s", "u",
            "x", "-20s", "2$s", "*.*f",
        ] {
            let s = format!("format('%{token} %s', foo, bar)");
            let o = offenses(&s);
            assert_eq!(o.len(), 2, "{s}");
            assert_eq!((o[0].line, o[0].col), (1, 9));
            assert_eq!(o[1].col, 11 + token.len() as u32);
            assert_eq!(
                o[0].message,
                "Prefer annotated tokens (like `%<foo>s`) over unannotated tokens (like `%s`)."
            );
            assert!(offenses(&format!("format('%{token}', foo)")).is_empty());
        }
    }
    #[test]
    fn template_exclusion_allows_alternative_width_matches() {
        for s in ["'%#{w}{x}'", "'%##{w}{x}'", "'%#{w}.#{p}{x}'"] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (1, 2), "{s}");
            assert_eq!(
                o[0].message,
                "Prefer annotated tokens (like `%<foo>s`) over template tokens (like `%{foo}`)."
            );
        }
        for s in ["'%#{x}'", "'%0#{x}'", "'% #{x}'", "'%##{x}'", "'%%#{w}{x}'"] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn comparison_in_interpolated_width_or_precision_is_not_a_name() {
        for s in [
            "format('%10.#{a<b}s %s', 1, 2)",
            "format('%#{a<b}.20s %s', 1, 2)",
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 2, "{s}");
            assert_eq!((o[0].line, o[0].col), (1, 9), "{s}");
            assert_eq!((o[1].line, o[1].col), (1, 21), "{s}");
            for offense in o {
                assert_eq!(
                    offense.message,
                    "Prefer annotated tokens (like `%<foo>s`) over unannotated tokens (like `%s`)."
                );
            }
        }
        for s in [
            "format('%10.#{a<b}s', 1)",
            "format('%10.#{a<b}<x>s', x: 1)",
            "format('%10<x>.#{a<b}s', x: 1)",
            "format('%<x>10.#{a<b}s', x: 1)",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn direct_closed_consumers_get_canonical_edits() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            ("format('%{a}', a: 1)", "format('%<a>s', a: 1)"),
            (
                "sprintf('%{foo}', foo: 'bar')",
                "sprintf('%<foo>s', foo: 'bar')",
            ),
            ("'%{foo}' % { foo: 'bar' }", "'%<foo>s' % { foo: 'bar' }"),
            (
                "Kernel.format('%{foo}', foo: 'bar')",
                "Kernel.format('%<foo>s', foo: 'bar')",
            ),
            (
                "::Kernel.sprintf('%{foo}', foo: 'bar')",
                "::Kernel.sprintf('%<foo>s', foo: 'bar')",
            ),
            (
                "format('%{a}/%%/%{b}', a: 1, b: :two)",
                "format('%<a>s/%%/%<b>s', a: 1, b: :two)",
            ),
            ("format(%q[%{a}], a: 1)", "format(%q[%<a>s], a: 1)"),
        ] {
            assert_correction(&FormatStringToken, before, after);
        }
    }
    #[test]
    fn every_remaining_pinned_correction_call_is_individually_withheld() {
        use crate::cops::testing::correction_offenses;
        // 18 annotated types x 3 correction calls in the upstream shared group.
        for ty in [
            "A", "B", "E", "G", "X", "a", "b", "c", "d", "e", "f", "g", "i", "o", "p", "s", "u",
            "x",
        ] {
            for s in [
                format!(
                    "format(<<-HEREDOC, foo: bar)\nfoo %<named>{ty} + bar %{{template}}\nHEREDOC\n"
                ),
                format!("format(\"a#{{b}}%<named>{ty} c#{{d}}%{{template}} e#{{f}}\")\n"),
                "format(\"%{a}\", a: foo)\n".into(),
            ] {
                let o = correction_offenses(&FormatStringToken, &s);
                assert_eq!(o.len(), 1, "{s}");
                assert!(
                    o[0].correction.is_none(),
                    "unknown argument/interpolation: {s}"
                );
            }
        }
        // The three other withheld pinned calls: unknown b (two), and printf.
        for s in [
            "format(\"c#{b}%{template}\")\n",
            "printf('%{foo}', foo: 'bar')\n",
            "\"c#{b}%{foo}\" % { foo: 'bar' }\n",
        ] {
            let o = correction_offenses(&FormatStringToken, s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(o[0].correction.is_none(), "{s}");
        }
    }
    #[test]
    fn five_applicable_pinned_corrections_apply_exactly() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            (
                "sprintf('%{foo}', foo: 'bar')\n",
                "sprintf('%<foo>s', foo: 'bar')\n",
            ),
            (
                "'%{foo}' % { foo: 'bar' }\n",
                "'%<foo>s' % { foo: 'bar' }\n",
            ),
            (
                "Kernel.format('%{foo}', foo: 'bar')\n",
                "Kernel.format('%<foo>s', foo: 'bar')\n",
            ),
            (
                "Kernel.sprintf('%{foo}', foo: 'bar')\n",
                "Kernel.sprintf('%<foo>s', foo: 'bar')\n",
            ),
            (
                "{ bar: format('%{foo}', foo: 'foo') }\n",
                "{ bar: format('%<foo>s', foo: 'foo') }\n",
            ),
        ] {
            assert_correction(&FormatStringToken, before, after);
        }
    }
    #[test]
    fn delimiter_changes_and_fragment_directives_are_never_blindly_edited() {
        use crate::cops::testing::{assert_correction, correction_offenses};
        for s in [
            "format(%q>%{x}>, x: 1)",
            "format(\"%\" \"%{x}\", x: 1)",
            "format(\"a%{x}\" \"%{y}\", x: 1, y: 2)",
            "format(\"#{'%'}%{x}\", x: 1)",
            "format(\"\\x25%{x}\", x: 1)",
            "format(\"%{x}\\n\", x: 1)",
            "format('%10{x}', x: 1)",
            "format('%{x>y}', :'x>y' => 1)",
        ] {
            let o = correction_offenses(&FormatStringToken, s);
            assert!(o.iter().all(|o| o.correction.is_none()), "{s}");
        }
        assert_correction(
            &FormatStringToken,
            "format(%q<%{x}>, x: 1)",
            "format(%q<%<x>s>, x: 1)",
        );
    }
    #[test]
    fn aggressive_detection_keeps_fragment_ownership_and_physical_positions() {
        for (s, positions) in [
            ("format(\"%\" \"%{x}\", x:1)", vec![(1, 13)]),
            ("format(\"#{'%'}%{x}\", x:1)", vec![(1, 15)]),
            ("'é%{x}'\n'%{y}'", vec![(1, 3), (2, 2)]),
            ("%q[%{x}]", vec![(1, 4)]),
            ("%w[%{x} %{y}]", vec![(1, 4), (1, 9)]),
            ("\"%{a}\" \"%{b}\"", vec![(1, 2), (1, 9)]),
        ] {
            let o = offenses(s);
            assert_eq!(
                o.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                positions,
                "{s}"
            );
        }
    }
    #[test]
    fn pure_data_never_gets_safe_edits_and_mixed_styles_keep_all_offenses() {
        use crate::cops::testing::correction_offenses;
        for s in [
            "'https://gitlab.com/%{project_path}/-/blob/%{default_branch}/file'",
            "validates :colour, format: { message: '%{colour} is not a valid hex colour' }",
            "<<-MSG\n  %{colour} is not valid\nMSG\n",
            "foo('%{bar}')",
            "'%{foo}'",
        ] {
            let o = correction_offenses(&FormatStringToken, s);
            assert!(!o.is_empty(), "{s}");
            assert!(o.iter().all(|o| o.correction.is_none()), "{s}");
        }
        assert_eq!(offenses("format('%s/%{x}', x: 1)").len(), 2);
        assert_eq!(offenses("format('%s/%<x>s', x: 1)").len(), 1);
        assert!(offenses("format('%s/%%', 1)").is_empty());
        assert!(offenses("format('%s', '%s %s')").is_empty());
    }
    #[test]
    fn mixed_or_uncertain_whole_formats_keep_offenses_without_edits() {
        use crate::cops::testing::correction_offenses;
        for other in [
            "%s",
            "%1$s",
            "%10.2s",
            "%*.*f",
            "%1$*2$.*3$f",
            "%<x>*s",
            "%*<x>s",
            "%10{x}",
            "%#{w}{x}",
            "%<x>#{w}s",
            "%<x>.#{p}s",
            "%q",
            "%",
            "%<other",
            "%{not-a-word}",
        ] {
            for format in [
                format!("{other}/%{{x}}/%%/%{{y}}"),
                format!("%{{x}}/%%/%{{y}}/{other}"),
            ] {
                for s in [
                    format!("format('{format}', x: 1, y: 2)"),
                    format!("sprintf('{format}', x: 1, y: 2)"),
                    format!("'{format}' % {{x: 1, y: 2}}"),
                ] {
                    let o = correction_offenses(&FormatStringToken, &s);
                    assert!(o.len() >= 2, "offenses retained: {s}");
                    assert!(o.iter().all(|o| o.correction.is_none()), "{s}");
                }
            }
        }
    }
    #[test]
    fn named_only_whole_formats_still_get_canonical_edits() {
        use crate::cops::testing::assert_correction;
        for format in [
            "%{x}/%%/%{y}",
            "%<x>10.2s/%{y}/%%/%{x}",
            "%{x}/%%/%{y}/%<x>-10.2s",
            "%%%%%{x}/%<y>06.2f",
        ] {
            let before = format!("format('{format}', x: 1, y: 2)");
            let after = before.replace("%{x}", "%<x>s").replace("%{y}", "%<y>s");
            assert_correction(&FormatStringToken, &before, &after);
        }
    }
    #[test]
    fn pinned_no_offense_lookalikes() {
        for s in [
            "format('%<hit_rate>6.2f%%', hit_rate: 12.34)",
            "`echo \"%s %<annotated>s %{template}\"`",
            "/foo %{x} %u/",
            "%r{foo %{x} %u}",
            "Time.strptime('2017-12-13', '%Y-%m-%d')",
            "time_format = '%Y-%m-%d'",
            "'%s %s'",
            "foo('%s %s')",
            "'https://ru.wikipedia.org/wiki/%D0%90_' '(%D0%BA)'",
            "format('%<day>s %<start>s-%<end>s', day: time.strftime('%a'))",
            "__FILE__",
            "object&.format('%s %s', 1, 2)",
            "'%s %s'&.%([1, 2])",
            "'%<x>s'",
            "'%%{x}'",
            "'%{not-a-name}'",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
