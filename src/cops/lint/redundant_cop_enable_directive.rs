//! `Lint/RedundantCopEnableDirective`: unnecessary enable and orphan pop comments.
//! Pairing diagnostics port CommentConfig (MIT, Copyright (c) 2012-26
//! Bozhidar Batsov). Effective suppression is a separate correction proof:
//! upstream's counters are not evidence that removing an enable is harmless.
use crate::{
    cops::{Cop, NodeContext},
    directives::{
        self,
        comments::{DirectiveComment, hits, registered_names, suppression_records},
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
        let mut proof = HashMap::new();
        let attachments = cx
            .corrections_requested()
            .then(|| directives::comment_attachments(file));
        let mut temporary = HashSet::new();
        for d in file.directive_records() {
            if file.bytes[d.range.start] != b'#' {
                continue;
            }
            if let Some(a) = &attachments
                && d.standalone
                && matches!(
                    d.mode,
                    "disable-next" | "todo-next" | "enable-next" | "next"
                )
                && d.has_cops()
                && let Some(&code) = a.get(&d.line)
            {
                temporary.insert(code);
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
                    cx.corrections_requested()
                        .then(|| removal(file, d, &[], true, attachments.as_ref(), &temporary))
                        .flatten(),
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
            let all = d.names.iter().all(|n| redundant.contains(n));
            let safe =
                cx.corrections_requested()
                    && d.names
                        .iter()
                        .filter(|n| redundant.contains(*n))
                        .all(|name| {
                            // A first explicit source opt-in is never expendable merely
                            // because this run selected no offenses (or forced --only).
                            !file.directive_disabled().iter().any(|cop| {
                                name == cop || Some(name.as_str()) == cop.rsplit('/').next()
                            }) && registered_names()
                                .iter()
                                .filter(|cop| hits(name, cop))
                                .filter(|cop| {
                                    !file.directive_disabled().iter().any(|n| n == **cop)
                                        || mobilized.contains(*cop)
                                })
                                .all(|cop| {
                                    proof
                                        .entry(*cop)
                                        .or_insert_with(|| directives::redundant_enables(file, cop))
                                        .contains(&d.line)
                                })
                        });
            let redundant_ranges: Vec<_> = tokens
                .iter()
                .filter(|(n, _)| redundant.contains(n.as_str()))
                .map(|(n, start)| (*start, *start + n.len()))
                .collect();
            let edit = safe
                .then(|| {
                    removal(
                        file,
                        d,
                        &redundant_ranges,
                        all,
                        attachments.as_ref(),
                        &temporary,
                    )
                })
                .flatten();
            // One atomic list edit belongs to the first diagnostic. Other
            // findings remain; no overlapping/partial selector removals.
            let mut edit = edit;
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
                    edit.take(),
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
fn removal(
    file: &SourceFile,
    d: &DirectiveComment,
    redundant: &[(usize, usize)],
    all: bool,
    attachments: Option<&HashMap<u32, u32>>,
    temporary: &HashSet<u32>,
) -> Option<Correction> {
    let text = &file.bytes[d.range.clone()];
    let matches = syntax::matches(text);
    let m = matches.first()?;
    let tail = text[m.end..].trim_ascii();
    // Preserve prose, magic keys, malformed/stacked markers and block comments.
    if m.start != 0
        || matches.len() != 1
        || text.contains(&b'\n')
        || m.signed
        || (d.mode == "pop" && !m.names.is_empty())
        || (!tail.is_empty() && !tail.starts_with(b"--"))
        || crate::cops::style::correction::contains_magic_comment_key(text)
    {
        return None;
    }
    if all {
        if attachments
            .and_then(|a| a.get(&d.line))
            .is_some_and(|code| temporary.contains(code))
        {
            return None;
        }
        let mut range = d.range.clone();
        if text.last() == Some(&b'\r') && file.bytes.get(range.end) == Some(&b'\n') {
            range.end -= 1;
        }
        // Only delete physical lines at EOF: subsequent executable line and
        // backtrace positions must not move. Else preserve an empty line.
        if file.bytes[range.end..]
            .iter()
            .all(|b| matches!(b, b'\r' | b'\n'))
            && d.standalone
        {
            range.end = file.bytes.len();
            let start = file.line_start(d.line);
            if file.bytes[start..range.start]
                .iter()
                .all(u8::is_ascii_whitespace)
            {
                range.start = start;
            }
        }
        return Some(Correction {
            range,
            replacement: String::new(),
        });
    }
    // Remove contiguous runs, not independent names sharing a comma. Keep
    // precisely one separator between surviving tokens, with its original
    // spacing; never reconstruct a reason or arbitrary legacy-encoded bytes.
    let removed: HashSet<_> = redundant.iter().map(|&(start, _)| start).collect();
    let tokens: Vec<_> = m
        .names
        .iter()
        .map(|name| {
            let start = name.as_ptr() as usize - text.as_ptr() as usize;
            (start, start + name.len())
        })
        .collect();
    let mut edits = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if !removed.contains(&tokens[i].0) {
            i += 1;
            continue;
        }
        let first = i;
        while i < tokens.len() && removed.contains(&tokens[i].0) {
            i += 1;
        }
        let (start, end) = if first == 0 {
            (tokens[first].0, tokens.get(i)?.0)
        } else {
            let prev = tokens[first - 1].1;
            let comma = text[prev..tokens[first].0]
                .iter()
                .position(|b| *b == b',')?;
            let mut end = tokens[i - 1].1;
            while end < m.end && text[end].is_ascii_whitespace() {
                end += 1;
            }
            (prev + comma, end)
        };
        edits.push(Correction {
            range: d.range.start + start..d.range.start + end,
            replacement: String::new(),
        });
    }
    crate::cops::style::correction::combine(file, edits)
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
                    "# P:disable Layout/LineLength\n# P:enable Metrics/AbcSize, Layout/LineLength\n",
                    vec!["Metrics/AbcSize"],
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
    fn explicit_cop_overrides_department_for_both_selector_orders() {
        for prefix in ["rubocop", "rustocop"] {
            for (selectors, col) in [
                ("Layout/LineLength, Layout", 18),
                ("Layout, Layout/LineLength", 26),
            ] {
                let source =
                    format!("# {prefix}:disable Layout\n# {prefix}:enable {selectors}\nfoo\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}: {found:?}");
                assert_eq!(
                    (found[0].line, found[0].col),
                    (2, col + prefix.len() as u32 - 7)
                );
                assert_eq!(
                    found[0].message,
                    "Unnecessary enabling of Layout/LineLength."
                );
                // Pairing redundancy alone does not prove that removing the
                // explicit selector preserves the effective suppression state.
                let found = crate::cops::testing::correction_offenses(
                    &RedundantCopEnableDirective,
                    &source,
                );
                assert_eq!(found.len(), 1);
                assert!(found[0].correction.is_none());
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
                "# rubocop:disable Layout/LineLength\n# rubocop:enable Metrics/AbcSize,Layout/LineLength\n",
                "# rubocop:disable Layout/LineLength\n# rubocop:enable Layout/LineLength\n",
            ),
            (
                "# rubocop:disable Layout/LineLength, Lint/Debugger\n# rubocop:enable Layout/LineLength,  Metrics/AbcSize,  Lint/Debugger\n",
                "# rubocop:disable Layout/LineLength, Lint/Debugger\n# rubocop:enable Layout/LineLength,  Lint/Debugger\n",
            ),
            (
                "# rubocop:disable Layout/LineLength\n# rubocop:enable Layout/LineLength, Metrics/AbcSize\n",
                "# rubocop:disable Layout/LineLength\n# rubocop:enable Layout/LineLength\n",
            ),
        ] {
            assert_correction(&RedundantCopEnableDirective, before, after);
        }
    }
    #[test]
    fn findings_survive_uncertain_effective_state_removals() {
        use crate::cops::testing::correction_offenses;
        for source in [
            "# rubocop:disable Layout/LineLength\n# rubocop:enable Layout\n",
            "# rubocop:push -Layout/LineLength\n# rubocop:enable Layout/LineLength\n",
            "# rubocop:disable-next Style/For\n# rubocop:enable Layout/LineLength\nfoo\n",
            "# rubocop:enable Layout/LineLength -- coding: ascii\n",
            "# rubocop:enable Layout/LineLength # rubocop:enable Style/For\n",
        ] {
            let found = correction_offenses(&RedundantCopEnableDirective, source);
            assert!(!found.is_empty(), "{source}");
            assert!(
                found.iter().all(|o| o.correction.is_none()),
                "{source}: {found:?}"
            );
        }
    }
    #[test]
    fn adjacent_list_removals_keep_one_valid_separator() {
        use crate::cops::testing::assert_correction;
        for (list, expected) in [
            (
                "Metrics/AbcSize, Metrics/ClassLength, Layout/LineLength",
                "Layout/LineLength",
            ),
            (
                "Layout/LineLength, Metrics/AbcSize, Metrics/ClassLength",
                "Layout/LineLength",
            ),
            (
                "Metrics/AbcSize, Layout/LineLength, Metrics/ClassLength",
                "Layout/LineLength",
            ),
            (
                "Metrics/AbcSize,Metrics/ClassLength,Layout/LineLength",
                "Layout/LineLength",
            ),
            (
                "Layout/LineLength,Metrics/AbcSize,Metrics/ClassLength",
                "Layout/LineLength",
            ),
        ] {
            let before = format!(
                "# rubocop:disable Layout/LineLength\n# rubocop:enable {list} -- retained\n"
            );
            let after = format!(
                "# rubocop:disable Layout/LineLength\n# rubocop:enable {expected} -- retained\n"
            );
            assert_correction(&RedundantCopEnableDirective, &before, &after);
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
