//! `Lint/RedundantCopEnableDirective`: unnecessary enable and orphan pop comments.
//! Pairing diagnostics port CommentConfig (MIT, Copyright (c) 2012-26
//! Bozhidar Batsov). Effective suppression is a separate correction proof:
//! upstream's counters are not evidence that removing an enable is harmless.
use crate::{
    cops::{Cop, NodeContext},
    directives::{
        self,
        comments::{hits, registered_names, suppression_records},
        syntax,
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
use std::collections::{BinaryHeap, HashMap, HashSet};
pub struct RedundantCopEnableDirective;
impl Cop for RedundantCopEnableDirective {
    fn name(&self) -> &'static str {
        "Lint/RedundantCopEnableDirective"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_program_node().is_none() {
            return;
        }
        let mobilized: HashSet<_> = file
            .directive_disabled()
            .iter()
            .filter(|cop| directives::source_opts_in(suppression_records(file), cop))
            .map(String::as_str)
            .collect();
        let mut counts = Pairing::default();
        for cop in &mobilized {
            counts.disable(cop);
        }
        let mut depth = 0usize;
        for d in file.directive_records() {
            if file.bytes[d.range.start] != b'#' {
                continue;
            }
            if d.mode == "push" {
                depth += 1;
                continue;
            }
            if d.mode == "pop" {
                if depth > 0 {
                    depth -= 1;
                    continue;
                }
                let text = &file.bytes[d.range.clone()];
                let Some(m) = syntax::matches(text).into_iter().next() else {
                    continue;
                };
                self.report(
                    file,
                    d.range.start + m.start,
                    d.range.start + m.end,
                    "Unnecessary `rubocop:pop` without a matching `rubocop:push`.".into(),
                    cx.corrections_requested().then(|| {
                        whole_removal(file, d.range.start + m.start, d.range.start + m.end, None)
                    }),
                    out,
                );
                continue;
            }
            // Self-closing scopes and trailing enables do not participate in
            // ordinary disable/enable pairing, exactly as in CommentConfig.
            if !d.standalone
                || !d.signed.is_empty()
                || !matches!(d.mode, "disable" | "todo" | "enable")
            {
                continue;
            }
            let mut redundant = HashSet::new();
            if d.mode == "enable" && d.names == ["all"] {
                if counts.enable_all() {
                    redundant.insert("all".to_owned());
                }
            } else {
                for raw in &d.names {
                    let expanded = expand(raw);
                    for name in expanded {
                        if d.disabled() {
                            counts.disable(name);
                        } else if counts.enable(name) {
                            redundant.insert(name.to_owned());
                        }
                    }
                }
            }
            if redundant.is_empty() {
                continue;
            }
            // Attribute expanded identities after pairing the whole list:
            // explicitly named cops override their department regardless of
            // which selector produced the redundant occurrence.
            // RuboCop's `directive.match?`: every cop the directive names,
            // departments expanded, is redundant.
            let all = redundant.contains("all")
                || d.names
                    .iter()
                    .flat_map(|raw| expand(raw))
                    .all(|name| redundant.contains(name));
            let selectors: HashSet<_> = d.names.iter().map(String::as_str).collect();
            let redundant: HashSet<_> = redundant
                .into_iter()
                .map(|name| {
                    if selectors.contains(name.as_str()) {
                        return name;
                    }
                    let department = name.split('/').next().expect("cop name");
                    if selectors.contains(department) {
                        department.to_owned()
                    } else {
                        name
                    }
                })
                .collect();
            let text = &file.bytes[d.range.clone()];
            let Some(m) = syntax::matches(text).into_iter().next() else {
                continue;
            };
            // Syntax names borrow their exact token, avoiding substring/prefix
            // collisions and quadratic searches in very long selector lists.
            let tokens: Vec<_> = d
                .names
                .iter()
                .zip(&m.names)
                .map(|(name, parsed)| {
                    let start = if *parsed == "all" {
                        m.end - 3
                    } else {
                        parsed.as_ptr() as usize - text.as_ptr() as usize
                    };
                    (name, start)
                })
                .collect();
            let mut seen = HashSet::new();
            for (name, start) in tokens {
                if !redundant.contains(name) || !seen.insert(name) {
                    continue;
                }
                self.report(
                    file,
                    d.range.start + start,
                    d.range.start + start + name.len(),
                    format!(
                        "Unnecessary enabling of {}.",
                        if name == "all" { "all cops" } else { name }
                    ),
                    cx.corrections_requested().then(|| {
                        if all {
                            whole_removal(
                                file,
                                d.range.start + m.start,
                                d.range.start + m.end,
                                Some(d.range.end),
                            )
                        } else {
                            comma_removal(file, d.range.clone(), name)
                        }
                    }),
                    out,
                );
            }
        }
    }
}
impl RedundantCopEnableDirective {
    fn report(
        &self,
        file: &SourceFile,
        start: usize,
        end: usize,
        message: String,
        correction: Option<Correction>,
        out: &mut Vec<Offense>,
    ) {
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            correction,
        });
    }
}
/// Lazy global decrement makes `enable all` independent of the number of
/// source-named unknown cops. Heap entries are discarded at most once, avoiding
/// a source-vocabulary x directive-count scan on adversarial comments.
#[derive(Default)]
struct Pairing {
    epoch: usize,
    counts: HashMap<String, usize>,
    max: BinaryHeap<(usize, String)>,
}
impl Pairing {
    fn disable(&mut self, name: &str) {
        let n = self.counts.entry(name.to_owned()).or_default();
        *n = (*n).max(self.epoch) + 1;
        self.max.push((*n, name.to_owned()));
    }
    fn enable(&mut self, name: &str) -> bool {
        let n = self.counts.entry(name.to_owned()).or_default();
        if *n <= self.epoch {
            return true;
        }
        *n -= 1;
        self.max.push((*n, name.to_owned()));
        false
    }
    fn enable_all(&mut self) -> bool {
        while self
            .max
            .peek()
            .is_some_and(|(n, name)| self.counts.get(name) != Some(n))
        {
            self.max.pop();
        }
        let redundant = self.max.peek().is_none_or(|(n, _)| *n <= self.epoch);
        self.epoch += 1;
        redundant
    }
}

fn expand(name: &str) -> Vec<&str> {
    // DirectiveComment#parsed_cop_names excludes mandatory parser diagnostics.
    // This is diagnostic pairing only, not registration or suppression policy.
    if name == "Lint/Syntax" {
        return vec![];
    }
    if name == "all"
        || registered_names()
            .iter()
            .any(|cop| cop.split('/').next() == Some(name))
    {
        registered_names()
            .iter()
            .filter(|cop| hits(name, cop))
            .copied()
            .collect()
    } else {
        vec![name]
    }
}
/// `range_with_surrounding_space(directive.range_with_reason, side: :right)`.
fn whole_removal(
    file: &SourceFile,
    start: usize,
    end: usize,
    comment_end: Option<usize>,
) -> Correction {
    let mut end = end;
    if let Some(comment_end) = comment_end
        && file.bytes[end..comment_end]
            .trim_ascii_start()
            .starts_with(b"--")
    {
        end = comment_end;
    }
    while matches!(file.bytes.get(end), Some(b' ' | b'\t')) {
        end += 1;
    }
    while file.bytes.get(end) == Some(&b'\n') {
        end += 1;
    }
    Correction {
        range: start..end,
        replacement: String::new(),
    }
}

/// RuboCop's `range_with_comma`.
fn comma_removal(file: &SourceFile, comment: std::ops::Range<usize>, name: &str) -> Correction {
    let text = &file.bytes[comment.clone()];
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let found = (0..text.len()).find(|&i| {
        text[i..].starts_with(name.as_bytes())
            && !text.get(i + name.len()).is_some_and(|&b| word(b))
    });
    let remove = |range| Correction {
        range,
        replacement: String::new(),
    };
    let Some(mut begin) = found else {
        return remove(comment);
    };
    let mut end = begin + name.len();
    while begin > 0 && matches!(text[begin - 1], b' ' | b'\t') {
        begin -= 1;
    }
    while matches!(text.get(end), Some(b' ' | b'\t')) {
        end += 1;
    }
    let start = comment.start;
    if begin > 0 && text[begin - 1] == b',' {
        remove(start + begin - 1..start + end)
    } else if text.get(end) == Some(&b',') {
        if text.get(end + 1) != Some(&b' ') {
            begin += 1;
        }
        remove(start + begin..start + end + 1)
    } else {
        remove(comment)
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/redundant_cop_enable_directive_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/RedundantCopEnableDirective")
            .collect()
    }
    #[test]
    fn pairing_departments_all_lists_and_prefixes() {
        for prefix in ["rubocop", "rustocop"] {
            for (source, names) in [
                ("# P:enable all\n", vec!["all cops"]),
                (
                    "# P:disable Layout/LineLength\nfoo\n# P:enable all\n",
                    vec![],
                ),
                (
                    "# P:disable Layout/LineLength\n# P:enable Lint/EmptyBlock, Layout/LineLength\n",
                    vec!["Lint/EmptyBlock"],
                ),
                (
                    "# P:disable Layout\n# P:enable Layout, Layout/LineLength\n",
                    vec!["Layout/LineLength"],
                ),
                (
                    "# P:disable Layout/LineLength\n# P:enable Layout\n",
                    vec!["Layout"],
                ),
                (
                    "# P:disable Layout/LineLength\n# P:enable Layout/LineLength\n# P:enable Layout/LineLength\n",
                    vec!["Layout/LineLength"],
                ),
                (
                    "# P:disable Layout/LineLength\n# P:disable Layout/LineLength\n# P:enable Layout/LineLength\n# P:enable Layout/LineLength\n",
                    vec![],
                ),
            ] {
                let found = offenses(&source.replace("P:", &format!("{prefix}:")));
                assert_eq!(
                    found.iter().map(|o| o.message.clone()).collect::<Vec<_>>(),
                    names
                        .iter()
                        .map(|n| format!("Unnecessary enabling of {n}."))
                        .collect::<Vec<_>>(),
                    "{source}"
                );
            }
        }
    }
    #[test]
    fn block_comment_pseudo_opt_ins_do_not_seed_disabled_counters() {
        for prefix in ["rubocop", "rustocop"] {
            for directive in [
                "enable Style/IpAddresses",
                "enable-next Style/IpAddresses",
                "push +Style/IpAddresses",
                "next +Style/IpAddresses",
            ] {
                let source =
                    format!("=begin\n# {prefix}:{directive}\n=end\n# {prefix}:enable all\n0\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}: {found:?}");
                assert_eq!((found[0].line, found[0].col), (4, 11 + prefix.len() as u32));
                assert_eq!(found[0].message, "Unnecessary enabling of all cops.");
            }
        }
    }
    #[test]
    fn comments_literals_trailing_and_self_closing_scopes() {
        for s in [
            "foo # rubocop:enable Layout/LineLength\n",
            "# # rubocop:enable Layout/LineLength\n",
            "x = '# rubocop:enable Layout/LineLength'\n",
            "x = <<~H\n# rubocop:enable Layout/LineLength\nH\n",
            "__END__\n# rubocop:enable Layout/LineLength\n",
            "# rubocop:enable-next Layout/LineLength\nfoo(\nbar\n)\n",
            "# rustocop:next +Layout/LineLength\nfoo\n",
            "# rubocop:push -Layout/LineLength\nfoo\n# rubocop:pop\n",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn orphan_pops_include_trailing_comments_and_nested_depth() {
        for prefix in ["rubocop", "rustocop"] {
            let s = format!(
                "# {prefix}:push\n# {prefix}:push -Style/For\n# {prefix}:pop\n# {prefix}:pop\nx # {prefix}:pop\n"
            );
            let found = offenses(&s);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), (5, 3));
            assert_eq!(
                found[0].message,
                "Unnecessary `rubocop:pop` without a matching `rubocop:push`."
            );
        }
    }
    #[test]
    fn real_canonical_edits_and_partial_comma_spacing() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            ("foo\n# rubocop:enable Layout/LineLength\n", "foo\n"),
            (
                "foo\n# rubocop:enable Layout/LineLength -- no longer needed\n",
                "foo\n",
            ),
            ("foo = 1\n# rubocop:pop\n", "foo = 1\n"),
            (
                "# rubocop:disable Layout/LineLength\n# rubocop:enable Lint/EmptyBlock,Layout/LineLength\n",
                "# rubocop:disable Layout/LineLength\n# rubocop:enable Layout/LineLength\n",
            ),
            (
                "# rubocop:disable Layout/LineLength, Lint/Debugger\n# rubocop:enable Layout/LineLength,  Lint/EmptyBlock,  Lint/Debugger\n",
                "# rubocop:disable Layout/LineLength, Lint/Debugger\n# rubocop:enable Layout/LineLength,  Lint/Debugger\n",
            ),
            (
                "# rubocop:disable Layout/LineLength\n# rubocop:enable Layout/LineLength, Lint/EmptyBlock\n",
                "# rubocop:disable Layout/LineLength\n# rubocop:enable Layout/LineLength\n",
            ),
        ] {
            assert_correction(&RedundantCopEnableDirective, before, after);
        }
    }
    #[test]
    fn lazy_pairing_counters_match_reference_operations_without_all_cross_products() {
        let mut counts = Pairing::default();
        let mut reference = HashMap::<String, usize>::new();
        for i in 0..10000 {
            let name = format!("Unknown/Cop{}", i % 317);
            match i % 7 {
                0 | 1 | 4 => {
                    counts.disable(&name);
                    *reference.entry(name).or_default() += 1;
                }
                2 | 5 => {
                    let n = reference.entry(name.clone()).or_default();
                    assert_eq!(counts.enable(&name), *n == 0);
                    *n = n.saturating_sub(1);
                }
                _ => {
                    assert_eq!(counts.enable_all(), reference.values().all(|n| *n == 0));
                    for n in reference.values_mut() {
                        *n = n.saturating_sub(1);
                    }
                }
            }
        }
        assert!(counts.max.len() <= 10000);
    }
    #[test]
    fn dense_lists_and_deep_pushes_use_bounded_shared_facts() {
        let n = 3000;
        let list = (0..n)
            .map(|i| format!("Unknown/Cop{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!("# rustocop:enable {list}\n");
        assert_eq!(offenses(&source).len(), n);
        let source = format!(
            "{}# rubocop:enable Layout/LineLength\n{}# rubocop:pop\n",
            "# rubocop:push\n".repeat(n),
            "# rubocop:pop\n".repeat(n)
        );
        let found = offenses(&source);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].line, n as u32 + 1);
        assert_eq!(found[1].line, 2 * n as u32 + 2);
    }
    #[test]
    fn version_inactive_defaults_do_not_seed_configured_off_pairing() {
        for target in ["Style/ReverseFind", "Security/YAMLLoad"] {
            for prefix in ["rubocop", "rustocop"] {
                let source = format!("# {prefix}:enable all\n# {prefix}:enable {target}\n0\n");
                let found = offenses(&source);
                assert_eq!(
                    found
                        .iter()
                        .map(|o| (o.line, o.col, o.message.as_str()))
                        .collect::<Vec<_>>(),
                    vec![
                        (
                            1,
                            11 + prefix.len() as u32,
                            "Unnecessary enabling of all cops."
                        ),
                        (
                            2,
                            11 + prefix.len() as u32,
                            format!("Unnecessary enabling of {target}.").as_str()
                        ),
                    ],
                    "{source}"
                );
            }
        }
    }
    #[test]
    fn reserved_syntax_is_excluded_from_diagnostics_not_unknown_selectors() {
        for prefix in ["rubocop", "rustocop"] {
            for suffix in ["", "0\n"] {
                let source = format!("# {prefix}:enable Lint/Syntax\n{suffix}");
                assert!(offenses(&source).is_empty(), "{source}");
            }
            for selector in ["Lint/UnknownCop", "Lint/SyntaxError", "Unknown/Syntax"] {
                let source = format!("# {prefix}:enable Lint/Syntax, {selector}\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}: {found:?}");
                assert_eq!((found[0].line, found[0].col), (1, 24 + prefix.len() as u32));
                assert_eq!(
                    found[0].message,
                    format!("Unnecessary enabling of {selector}.")
                );
            }
        }
    }
    #[test]
    fn unnecessary_enable_has_exact_location_and_message() {
        let found = offenses("foo\n# rubocop:enable Layout/LineLength\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 18));
        assert_eq!(
            found[0].message,
            "Unnecessary enabling of Layout/LineLength."
        );
    }
}
