//! `Style/DirectiveScope`: prefer statement-scoped directives; unsafe correction.
use crate::{
    cops::{Cop, NodeContext},
    directives::{
        comment_attachments,
        comments::{DirectiveComment, registered_names},
        statement_ends,
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::collections::HashMap;
pub struct DirectiveScope;
impl Cop for DirectiveScope {
    fn name(&self) -> &'static str {
        "Style/DirectiveScope"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_program_node().is_none() {
            return;
        }
        let records = file.directive_records();
        // Ordinary files do no secondary AST/range analysis at all.
        if !records.iter().any(|d| {
            d.standalone
                && (d.disabled()
                    || d.mode == "enable"
                    || (d.mode == "push" && !d.signed.is_empty()))
        }) {
            return;
        }
        let ends: HashMap<_, _> = statement_ends(
            node,
            file,
            records
                .iter()
                .filter(|d| d.standalone)
                .map(|d| d.line + 1)
                .collect(),
        )
        .into_iter()
        .collect();
        let analysis = Analysis::build(records, &ends, &comment_attachments(file));
        let by_line: HashMap<_, _> = records
            .iter()
            .enumerate()
            .map(|(i, d)| (d.line, i))
            .collect();
        for (i, d) in records.iter().enumerate() {
            if !d.standalone {
                continue;
            }
            let Some(&end) = ends.get(&(d.line + 1)) else {
                continue;
            };
            let Some(&closing) = by_line.get(&(end + 1)) else {
                continue;
            };
            let close = &records[closing];
            // The opening directive becomes its `-next` form; the closing
            // directive's line goes.
            let text = String::from_utf8_lossy(&file.bytes[d.range.clone()]).into_owned();
            let mut replacement = None;
            let message = if d.disabled()
                && !d.embedded
                && !d.names.is_empty()
                && close.mode == "enable"
                && d.same_names(close)
                && analysis.uninterrupted[i]
            {
                replacement = sub_word(&text, d.mode, &format!("{}-next", d.mode));
                Some(format!(
                    "Use `{}-next` instead of a `{}`/`enable` pair around a single statement.",
                    d.mode, d.mode
                ))
            } else if d.mode == "push" && !d.signed.is_empty() && d.pop == Some(closing) {
                replacement = Some(push_replacement(&text, d));
                Some(format!(
                    "Use `{}` instead of `push`/`pop` around a single statement.",
                    d.replacement_mode()
                ))
            } else if d.mode == "enable"
                && !d.embedded
                && d.names != ["all"]
                && !d.names.is_empty()
                && close.disabled()
                && !close.embedded
                && d.same_names(close)
                && analysis.closed_all[i]
            {
                replacement = sub_word(&text, "enable", "enable-next");
                Some("Use `enable-next` instead of an `enable`/`disable` pair around a single statement.".into())
            } else {
                None
            };
            if !close.standalone {
                continue;
            }
            if let Some(message) = message {
                let correction = replacement.and_then(|replacement| {
                    crate::offense::Correction::from_edits(
                        &file.bytes,
                        vec![
                            (d.range.clone(), replacement),
                            (
                                crate::cops::style::correction::whole_lines(
                                    file,
                                    close.range.clone(),
                                ),
                                String::new(),
                            ),
                        ],
                    )
                });
                let (line, col) = file.line_col(d.range.start);
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    message,
                    line,
                    col,
                    end_line: file.line_of(d.range.end.saturating_sub(1).max(d.range.start)),
                    correction,
                });
            }
        }
    }
}

/// `text.sub(/\b#{word}\b/, replacement)`.
fn sub_word(text: &str, word: &str, replacement: &str) -> Option<String> {
    let is_word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut from = 0;
    while let Some(i) = text[from..].find(word).map(|i| i + from) {
        let end = i + word.len();
        if !is_word(text[..i].chars().last()) && !is_word(text[end..].chars().next()) {
            return Some(format!("{}{replacement}{}", &text[..i], &text[end..]));
        }
        from = end;
    }
    None
}

/// Upstream's `push_replacement`: a `-next` directive for the signed
/// arguments, keeping the reason after `--`.
fn push_replacement(text: &str, d: &DirectiveComment) -> String {
    let prefix = if text.contains("rustocop") {
        "# rustocop:"
    } else {
        "# rubocop:"
    };
    let after = sub_word(text, "push", "\0")
        .and_then(|marked| marked.split_once('\0').map(|(_, rest)| rest.to_owned()))
        .unwrap_or_default();
    let (cops, reason) = match after.split_once("--") {
        Some((cops, reason)) => (cops.trim().to_owned(), Some(reason.trim().to_owned())),
        None => (after.trim().to_owned(), None),
    };
    let names = |enable: bool| {
        d.signed
            .iter()
            .filter(|(e, _)| *e == enable)
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let directive = match d.replacement_mode() {
        "disable-next" => format!("{prefix}disable-next {}", names(false)),
        "enable-next" => format!("{prefix}enable-next {}", names(true)),
        _ => format!("{prefix}next {cops}"),
    };
    match reason.filter(|r| !r.is_empty()) {
        Some(reason) => format!("{directive} -- {reason}"),
        None => directive,
    }
}

/// Query continuity at candidate closing boundaries instead of retaining every
/// target/range ever closed. Wildcards are intervals, not target cross-products.
struct Analysis {
    uninterrupted: Vec<bool>,
    closed_all: Vec<bool>,
    #[cfg(test)]
    allocated_nodes: usize,
}
impl Analysis {
    fn build(
        records: &[DirectiveComment],
        ends: &HashMap<u32, u32>,
        attachments: &HashMap<u32, u32>,
    ) -> Self {
        let selectors = Selectors::new(records);
        let mut tree = RangeTree::new(selectors.len);
        // State stores 0 for disabled, 1 for enabled; touches store the most
        // recent potential interruption's source index. Both use persistent
        // range assignment; an epoch tracks pops that split every open range.
        let mut state = tree.leaf(1);
        let mut touched = tree.leaf(0);
        let mut stack = Vec::new();
        let mut restored_at = 0;
        let mut a = Self {
            uninterrupted: vec![false; records.len()],
            closed_all: vec![false; records.len()],
            #[cfg(test)]
            allocated_nodes: 0,
        };
        let by_line: HashMap<_, _> = records
            .iter()
            .enumerate()
            .map(|(i, d)| (d.line, i))
            .collect();
        let mut closing = vec![Vec::new(); records.len()];
        for (i, d) in records.iter().enumerate() {
            if d.disabled()
                && d.standalone
                && let Some(end) = ends.get(&(d.line + 1))
                && let Some(&close) = by_line.get(&(end + 1))
            {
                closing[close].push(i);
            }
        }
        for (i, d) in records.iter().enumerate() {
            for &open in &closing[i] {
                let names = &records[open].names;
                a.uninterrupted[open] = !names.is_empty()
                    && names.iter().all(|n| {
                        selectors
                            .get(n)
                            .iter()
                            .all(|&range| tree.max(touched, range).max(restored_at) == open + 1)
                    });
            }
            if d.mode == "pop" {
                if let Some(saved) = stack.pop() {
                    state = saved;
                    // Pop closes and reopens every saved disable, even for
                    // targets untouched by the push. One global epoch marks
                    // that split without replaying ranges or copying state.
                    restored_at = i + 1;
                }
                continue;
            }
            if d.mode == "push" {
                stack.push(state);
            }
            let plain =
                d.standalone && !d.embedded && matches!(d.mode, "disable" | "todo" | "enable");
            if plain {
                a.closed_all[i] = !d.names.is_empty()
                    && d.names.iter().all(|n| {
                        selectors
                            .get(n)
                            .iter()
                            .all(|&range| tree.max(state, range) == 0)
                    });
            }
            let temporary = d.standalone
                && attachments.contains_key(&d.line)
                && matches!(d.mode, "enable-next" | "next");
            let ops: Vec<_> = if d.mode == "push" || (temporary && d.mode == "next") {
                d.signed
                    .iter()
                    .map(|(enable, name)| (*enable, name))
                    .collect()
            } else if plain || temporary {
                d.names.iter().map(|name| (!d.disabled(), name)).collect()
            } else {
                Vec::new()
            };
            for (enable, name) in ops {
                // A temporary enable punches a hole and restores the same owner
                // below its statement (RuboCop's suspend_disable). Mark the
                // interruption without replacing that persistent owner/state.
                if temporary && !enable {
                    continue;
                }
                let ranges = if matches!(d.mode, "push" | "next") {
                    selectors.get_signed(name)
                } else {
                    selectors.get(name)
                };
                for &range in ranges {
                    // A negative push preserves already-disabled owners. Newly
                    // disabled targets cannot break an outer disable either:
                    // any intervening enable/pop already marked its split.
                    // Keep the whole interval assignment (including mixed
                    // departments) rather than expanding it into target leaves.
                    if d.mode != "push" || enable {
                        touched = tree.set(touched, range, i + 1);
                    }
                    if !temporary {
                        state = tree.set(state, range, usize::from(enable));
                    }
                }
            }
        }
        #[cfg(test)]
        {
            a.allocated_nodes = tree.nodes.len();
        }
        a
    }
}

type Interval = (usize, usize);

/// Lexically sorted qualified names make every department contiguous. Short
/// aliases resolve against the bounded registry, not arbitrary source names.
/// Each vocabulary entry contributes only a constant number of index entries.
struct Selectors {
    ranges: HashMap<String, Vec<Interval>>,
    len: usize,
}
impl Selectors {
    fn new(records: &[DirectiveComment]) -> Self {
        let mut vocabulary: std::collections::BTreeSet<String> =
            registered_names().iter().map(|n| (*n).to_owned()).collect();
        for d in records {
            for n in d.names.iter().chain(d.signed.iter().map(|(_, n)| n)) {
                if n != "all" {
                    vocabulary.insert(n.clone());
                }
            }
        }
        let len = vocabulary.len().max(1);
        let mut indices: HashMap<String, Vec<usize>> = HashMap::new();
        for (id, name) in vocabulary.into_iter().enumerate() {
            if let Some((department, short)) = name.split_once('/') {
                indices.entry(department.into()).or_default().push(id);
                if registered_names().contains(&name.as_str()) {
                    indices.entry(short.into()).or_default().push(id);
                }
            }
            indices.entry(name).or_default().push(id);
        }
        let mut ranges = HashMap::new();
        for (name, mut ids) in indices {
            ids.sort_unstable();
            ids.dedup();
            let mut spans: Vec<Interval> = Vec::new();
            for id in ids {
                if let Some(last) = spans.last_mut()
                    && last.1 == id
                {
                    last.1 += 1;
                } else {
                    spans.push((id, id + 1));
                }
            }
            ranges.insert(name, spans);
        }
        ranges.insert("all".into(), vec![(0, len)]);
        Self { ranges, len }
    }
    fn get(&self, name: &str) -> &[Interval] {
        self.ranges.get(name).map(Vec::as_slice).unwrap_or(&[])
    }
    fn get_signed(&self, name: &str) -> &[Interval] {
        if name == "all" {
            // The pinned reference treats signed `all` as an unknown literal,
            // not a wildcard. No ordinary selector addresses that literal, so
            // it cannot interrupt a pair or change its disabled state.
            &[]
        } else {
            self.get(name)
        }
    }
}

/// Persistent, lazily uniform segment tree. Assignment and maximum queries
/// cost O(log vocabulary); a wildcard assignment costs O(1). Push saves a root
/// index, so restoring even a large mixed state never copies its vocabulary.
struct RangeTree {
    nodes: Vec<RangeNode>,
    len: usize,
}
struct RangeNode {
    max: usize,
    children: Option<(usize, usize)>,
}
impl RangeTree {
    fn new(len: usize) -> Self {
        Self {
            nodes: Vec::new(),
            len,
        }
    }
    fn leaf(&mut self, value: usize) -> usize {
        let id = self.nodes.len();
        self.nodes.push(RangeNode {
            max: value,
            children: None,
        });
        id
    }
    fn set(&mut self, root: usize, range: Interval, value: usize) -> usize {
        self.assign(root, (0, self.len), range, value)
    }
    fn assign(&mut self, root: usize, bounds: Interval, range: Interval, value: usize) -> usize {
        if range.0 <= bounds.0 && bounds.1 <= range.1 {
            return self.leaf(value);
        }
        let (mut left, mut right) = self.nodes[root].children.unwrap_or((root, root));
        let mid = (bounds.0 + bounds.1) / 2;
        if range.0 < mid {
            left = self.assign(left, (bounds.0, mid), range, value);
        }
        if range.1 > mid {
            right = self.assign(right, (mid, bounds.1), range, value);
        }
        let id = self.nodes.len();
        self.nodes.push(RangeNode {
            max: self.nodes[left].max.max(self.nodes[right].max),
            children: Some((left, right)),
        });
        id
    }
    fn max(&self, root: usize, range: Interval) -> usize {
        self.query(root, (0, self.len), range)
    }
    fn query(&self, root: usize, bounds: Interval, range: Interval) -> usize {
        let node = &self.nodes[root];
        let Some((left, right)) = node.children else {
            return node.max;
        };
        if range.0 <= bounds.0 && bounds.1 <= range.1 {
            return node.max;
        }
        let mid = (bounds.0 + bounds.1) / 2;
        let mut max = 0;
        if range.0 < mid {
            max = self.query(left, (bounds.0, mid), range);
        }
        if range.1 > mid {
            max = max.max(self.query(right, (mid, bounds.1), range));
        }
        max
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/directive_scope_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == DirectiveScope.name())
            .collect()
    }
    #[test]
    fn signed_pushes_and_enable_pairs_have_exact_messages() {
        for prefix in ["rubocop", "rustocop"] {
            for (args, replacement) in [
                ("-Metrics/AbcSize", "disable-next"),
                ("+Metrics/AbcSize", "enable-next"),
                ("-Metrics/AbcSize +Style/For", "next"),
            ] {
                let source = format!(
                    "# {prefix}:disable Metrics/AbcSize\n# {prefix}:push {args} -- reviewed\ndef foo\nend\n# {prefix}:pop\n# {prefix}:enable Metrics/AbcSize\n"
                );
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (2, 1));
                assert_eq!(
                    found[0].message,
                    format!(
                        "Use `{replacement}` instead of `push`/`pop` around a single statement."
                    )
                );
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn pinned_no_offense_lookalikes() {
        for body in [
            "# rubocop:disable Metrics/AbcSize\ndef foo; end\n\ndef bar; end\n# rubocop:enable Metrics/AbcSize\n",
            "# rubocop:disable Metrics/AbcSize, Metrics/MethodLength\ndef foo; end\n# rubocop:enable Metrics/AbcSize\n# rubocop:enable Metrics/MethodLength\n",
            "foo # rubocop:disable Metrics/AbcSize\n",
            "# rubocop:disable Metrics/AbcSize\ndef foo; end\n",
            "# rubocop:enable Metrics/AbcSize\ndef foo; end\n# rubocop:disable Metrics/AbcSize\ndef bar; end\ndef baz; end\n# rubocop:enable Metrics/AbcSize\n",
            "# rubocop:push -Metrics/AbcSize\n# rubocop:push -Style/For\ndef foo; end\n\ndef quux; end\n# rubocop:pop\ndef bar; end\n# rubocop:pop\n",
            "# rubocop:push\n# rubocop:disable Metrics/AbcSize\ndef foo; end\n# rubocop:pop\n",
            "# rubocop:disable-next Metrics/AbcSize\ndef foo; end\n",
            "# rubocop:disable Metrics/AbcSize\ndef foo; end\n\n# rubocop:enable Metrics/AbcSize\n",
            "# rubocop:disable Metrics/AbcSize\n# protected comment\ndef foo; end\n# rubocop:enable Metrics/AbcSize\n",
            "# rubocop:disable Metrics/AbcSize\n\ndef foo; end\n# rubocop:enable Metrics/AbcSize\n",
            "# rubocop:push -Metrics/AbcSize\ndef foo; end\n",
            "# rubocop:disable Metrics/AbcSize\ndef foo; end # rubocop:enable Metrics/AbcSize\n",
            "# # rubocop:disable Metrics/AbcSize\ndef foo; end\n# rubocop:enable Metrics/AbcSize\n",
        ] {
            assert!(offenses(body).is_empty(), "{body}");
        }
    }
    #[test]
    fn multiline_scopes_heredocs_and_nested_statements() {
        for (body, expected) in [
            ("foo(\n  bar\n)", 1),
            ("foo(<<~H)\n  text\nH", 1),
            ("foo(<<~A, <<~B)\n  a\nA\n  b\nB", 1),
            ("if a\n  b\nelse\n  c\nend", 1),
            ("def foo\n  bar\nrescue\n  baz\nensure\n  quux\nend", 1),
            ("foo\nbar", 0),
        ] {
            let source = format!(
                "# rustocop:todo Metrics/AbcSize\n{body}\n# rustocop:enable Metrics/AbcSize\n"
            );
            assert_eq!(offenses(&source).len(), expected, "{source}");
        }
        let nested = "def foo\n# rubocop:disable Metrics/AbcSize\nbar\n# rubocop:enable Metrics/AbcSize\nend\n";
        assert_eq!(offenses(nested)[0].line, 2);
    }
    #[test]
    fn unicode_mode_boundaries_and_vertical_tabs() {
        for word in ["é", "中", "\u{301}", "_", "2"] {
            let source = format!(
                "# rubocop:disable{word} Metrics/AbcSize\nfoo\n# rubocop:enable Metrics/AbcSize\n"
            );
            assert!(offenses(&source).is_empty(), "{source}");
        }
        let source = "\u{feff}\u{b}#\u{b}rustocop\u{b}:\u{b}todo\u{b}Metrics/AbcSize\r\nfoo\r\n# rustocop:enable Metrics/AbcSize\r\n";
        let found = offenses(source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 2));
    }
    #[test]
    fn self_suppression_and_comments_inside_literals_or_data() {
        for prefix in ["rubocop", "rustocop"] {
            for target in ["Style/DirectiveScope", "DirectiveScope", "Style", "all"] {
                let source = format!(
                    "# {prefix}:disable {target}\n# {prefix}:disable Metrics/AbcSize\nfoo\n# {prefix}:enable Metrics/AbcSize\n"
                );
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        for source in [
            "x = '# rubocop:disable Metrics/AbcSize'\nfoo\n# rubocop:enable Metrics/AbcSize\n",
            "x = <<~H\n# rubocop:disable Metrics/AbcSize\nfoo\n# rubocop:enable Metrics/AbcSize\nH\n",
            "__END__\n# rubocop:disable Metrics/AbcSize\nfoo\n# rubocop:enable Metrics/AbcSize\n",
            "# mention rubocop:disable Metrics/AbcSize\nfoo\n# rubocop:enable Metrics/AbcSize\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn nested_push_restoration_does_not_replay_inner_changes_at_outer_pop() {
        let source = format!(
            "{}# rubocop:disable Metrics/AbcSize\nwork\n{}# rubocop:enable Metrics/AbcSize\n",
            "# rubocop:push\n".repeat(3000),
            "# rubocop:pop\n".repeat(3000)
        );
        let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
        let parse = ruby_prism::parse(&file.bytes);
        file.record_comments(&parse);
        let a = Analysis::build(file.directive_records(), &HashMap::new(), &HashMap::new());
        assert!(!a.closed_all.last().unwrap());
        assert!(a.allocated_nodes < 1000);
    }
    #[test]
    fn wildcard_work_does_not_multiply_source_named_vocabulary() {
        for tail in [
            "# rubocop:disable all\n",
            "# rubocop:push -all\n# rubocop:pop\n",
            "# rubocop:push -Metrics\n# rubocop:pop\n",
        ] {
            let mut previous = None;
            // Keep the vocabulary's tree shape stable as cops are registered:
            // otherwise midpoint rounding alone changes allocation ratios.
            // The department tail introduces one extra literal selector.
            let fixed = registered_names().len() + usize::from(tail.contains("Metrics"));
            // Keep the generated source vocabulary dominant as registration
            // grows: otherwise doubling capacity more than doubles n, and a
            // fixed 2.5x n-log-n bound can fail on a new cop alone.
            let base = (fixed * 8).next_power_of_two();
            for capacity in [base, 2 * base, 4 * base] {
                let n = capacity - fixed;
                let mut source = String::new();
                for i in 0..n {
                    let mode = if i % 2 == 0 { "disable" } else { "enable" };
                    source.push_str(&format!("# rubocop:{mode} Metrics/Cop{i}\n"));
                }
                source.push_str(&tail.repeat(n));
                source.push_str("work\n");
                let file = SourceFile::new("test.rb".into(), source.into_bytes());
                let parse = ruby_prism::parse(&file.bytes);
                file.record_comments(&parse);
                let a = Analysis::build(file.directive_records(), &HashMap::new(), &HashMap::new());
                // Structural work bound, not a flaky wall-clock assertion:
                // logarithmic point updates, constant-size wildcard transitions
                // and root snapshots (even when repeatedly restoring mixed state).
                // The registry reaching 257 names doubles the base capacity;
                // bound n*log(capacity), not the old fixed tree height.
                assert!(
                    a.allocated_nodes < 6 * capacity.ilog2() as usize * n,
                    "{}",
                    a.allocated_nodes
                );
                if let Some(previous) = previous {
                    assert!(
                        a.allocated_nodes * 2 < previous * 5,
                        "tail={tail:?}, n={n}, previous={previous}, current={}",
                        a.allocated_nodes
                    );
                }
                previous = Some(a.allocated_nodes);
            }
        }
    }
    #[test]
    fn temporary_enables_interrupt_only_their_own_targets() {
        for prefix in ["rubocop", "rustocop"] {
            for mode in [
                "enable-next Metrics/AbcSize",
                "next +Metrics/AbcSize",
                "next +Metrics/AbcSize -Style/For",
                "enable-next Metrics",
                "enable-next all",
                "next +all +Metrics/AbcSize",
                "next -all +Metrics/AbcSize",
            ] {
                let s = format!(
                    "# {prefix}:disable Metrics/AbcSize\ndef foo\n# {prefix}:{mode}\n  bar\nend\n# {prefix}:enable Metrics/AbcSize\n"
                );
                assert!(offenses(&s).is_empty(), "{s}");
            }
            for mode in [
                "enable-next Style/For",
                "next +Style/For",
                "next +all",
                "next -all",
                "next +all -Style/For",
                "next -all +Style/For",
            ] {
                let s = format!(
                    "# {prefix}:disable Metrics/AbcSize\ndef foo\n# {prefix}:{mode}\n  bar\nend\n# {prefix}:enable Metrics/AbcSize\n"
                );
                let found = offenses(&s);
                assert_eq!(found.len(), 1, "{s}");
                assert_eq!((found[0].line, found[0].col), (1, 1));
                assert_eq!(
                    found[0].message,
                    "Use `disable-next` instead of a `disable`/`enable` pair around a single statement."
                );
            }
        }
    }
    #[test]
    fn repeated_negative_pushes_preserve_disabled_owners() {
        for prefix in ["rubocop", "rustocop"] {
            for (target, args) in [
                ("Metrics/AbcSize", "-Metrics/AbcSize"),
                ("Metrics/AbcSize", "-Metrics/AbcSize -Metrics/AbcSize"),
                ("Metrics/AbcSize", "-Metrics"),
                ("Metrics/AbcSize", "-Metrics +Style/For"),
                ("Metrics/AbcSize", "-Metrics/MethodLength -Metrics/AbcSize"),
                ("Lint/Debugger", "-Debugger"),
                ("Lint/Debugger", "-Lint"),
            ] {
                for mode in ["disable", "todo"] {
                    let source = format!(
                        "# {prefix}:{mode} {target}\ndef foo\n# {prefix}:push {args}\n  bar\n  baz\nend\n# {prefix}:enable {target}\n"
                    );
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col), (1, 1), "{source}");
                    assert_eq!(
                        found[0].message,
                        format!(
                            "Use `{mode}-next` instead of a `{mode}`/`enable` pair around a single statement."
                        )
                    );
                    assert!(found[0].correction.is_none());
                }
            }
        }
    }
    #[test]
    fn effective_push_changes_and_balancing_pops_still_interrupt() {
        for prefix in ["rubocop", "rustocop"] {
            for inner in [
                format!("# {prefix}:push +Metrics/AbcSize\n"),
                format!("# {prefix}:push +Metrics/AbcSize\n# {prefix}:push -Metrics/AbcSize\n"),
                format!("# {prefix}:push -Metrics/AbcSize +Metrics/AbcSize\n"),
                format!("# {prefix}:push -Metrics\n# {prefix}:pop\n"),
                format!("# {prefix}:push -Metrics/AbcSize\n# {prefix}:pop\n"),
            ] {
                let source = format!(
                    "# {prefix}:disable Metrics/AbcSize\ndef foo\n{inner}  bar\n  baz\nend\n# {prefix}:enable Metrics/AbcSize\n"
                );
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
    }
    #[test]
    fn negative_pushes_still_establish_disabled_state_for_enable_pairs() {
        for prefix in ["rubocop", "rustocop"] {
            for args in ["-Metrics/AbcSize", "-Metrics", "-Metrics/AbcSize -Metrics"] {
                let source = format!(
                    "# {prefix}:push {args}\n# {prefix}:enable Metrics/AbcSize\nwork\n# {prefix}:disable Metrics/AbcSize\n"
                );
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (2, 1));
                assert_eq!(
                    found[0].message,
                    "Use `enable-next` instead of an `enable`/`disable` pair around a single statement."
                );
            }
            let source = format!(
                "# {prefix}:push +Metrics/AbcSize\n# {prefix}:enable Metrics/AbcSize\nwork\n# {prefix}:disable Metrics/AbcSize\n"
            );
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
    #[test]
    fn signed_all_pushes_do_not_change_named_disable_state() {
        for prefix in ["rubocop", "rustocop"] {
            for sign in ["+", "-"] {
                let source = format!(
                    "# {prefix}:push {sign}all\n# {prefix}:enable Metrics/AbcSize\nwork\n# {prefix}:disable Metrics/AbcSize\n# {prefix}:pop\n"
                );
                assert!(offenses(&source).is_empty(), "{source}");
                let source = format!(
                    "# {prefix}:disable Metrics/AbcSize\n# {prefix}:push {sign}all\n# {prefix}:enable Metrics/AbcSize\nwork\n# {prefix}:disable Metrics/AbcSize\n# {prefix}:pop\n# {prefix}:enable Metrics/AbcSize\nwork\n# {prefix}:disable Metrics/AbcSize\n"
                );
                let found = offenses(&source);
                assert_eq!(found.len(), 2, "{source}");
                for (offense, line) in found.iter().zip([3, 7]) {
                    assert_eq!((offense.line, offense.col), (line, 1), "{source}");
                    assert_eq!(
                        offense.message,
                        "Use `enable-next` instead of an `enable`/`disable` pair around a single statement."
                    );
                }
            }
        }
    }
    #[test]
    fn signed_all_push_pop_splits_enclosing_ranges_but_unclosed_push_does_not() {
        for prefix in ["rubocop", "rustocop"] {
            for sign in ["+", "-"] {
                for (closed, expected) in [(false, 1), (true, 0)] {
                    let pop = if closed {
                        format!("# {prefix}:pop\n")
                    } else {
                        String::new()
                    };
                    let source = format!(
                        "# {prefix}:disable Metrics/AbcSize\ndef foo\n# {prefix}:push {sign}all\n bar\n baz\n{pop}end\n# {prefix}:enable Metrics/AbcSize\n"
                    );
                    let found = offenses(&source);
                    assert_eq!(found.len(), expected, "{source}");
                    if let Some(offense) = found.first() {
                        assert_eq!((offense.line, offense.col), (1, 1));
                        assert_eq!(
                            offense.message,
                            "Use `disable-next` instead of a `disable`/`enable` pair around a single statement."
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn signed_all_push_pairs_still_report_their_replacement() {
        for prefix in ["rubocop", "rustocop"] {
            for (args, mode) in [
                ("+all", "enable-next"),
                ("-all", "disable-next"),
                ("+all -all", "next"),
            ] {
                let source = format!("# {prefix}:push {args}\nwork\n# {prefix}:pop\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (1, 1));
                assert_eq!(
                    found[0].message,
                    format!("Use `{mode}` instead of `push`/`pop` around a single statement.")
                );
            }
        }
    }
    #[test]
    fn detached_and_stacked_temporary_enables_and_restored_owners() {
        for prefix in ["rubocop", "rustocop"] {
            let detached = format!(
                "# {prefix}:disable Metrics/AbcSize\ndef foo\n# {prefix}:enable-next Metrics/AbcSize\n\n  bar\nend\n# {prefix}:enable Metrics/AbcSize\n"
            );
            assert_eq!(offenses(&detached).len(), 1);
            let stacked = format!(
                "# {prefix}:disable Metrics/AbcSize\ndef foo\n# {prefix}:enable-next Metrics/AbcSize\n# comment\n# {prefix}:next +Style/For\n  bar\nend\n# {prefix}:enable Metrics/AbcSize\n"
            );
            assert!(offenses(&stacked).is_empty());
            let restored = format!(
                "# {prefix}:disable Metrics/AbcSize\n# {prefix}:enable-next Metrics/AbcSize\nfoo\n# {prefix}:enable Metrics/AbcSize\nbar\n# {prefix}:disable Metrics/AbcSize\n"
            );
            let found = offenses(&restored);
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].line, 4);
        }
    }
    #[test]
    fn many_pairs_share_one_statement_scan_and_cached_comment_records() {
        let source = "# rubocop:disable Metrics/AbcSize\nfoo(<<~H)\ntext\nH\n# rubocop:enable Metrics/AbcSize\n".repeat(3000);
        let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
        let parse = ruby_prism::parse(&file.bytes);
        file.record_comments(&parse);
        let records = file.directive_records().as_ptr();
        assert_eq!(file.directive_records().len(), 6000);
        assert_eq!(records, file.directive_records().as_ptr());
        assert_eq!(offenses(&source).len(), 3000);
    }
    #[test]
    fn enable_pairs_resolve_department_and_all_regions() {
        for outer in ["Metrics", "all", "Metrics/AbcSize"] {
            let activate = if outer == "all" {
                "# rubocop:enable Style/DirectiveScope\n"
            } else {
                ""
            };
            let s = format!(
                "# rubocop:disable {outer}\n{activate}# rubocop:enable Metrics/AbcSize\ndef foo; end\n# rubocop:disable Metrics/AbcSize\n"
            );
            let found = offenses(&s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!(
                (found[0].line, found[0].col),
                (if outer == "all" { 3 } else { 2 }, 1)
            );
            assert_eq!(
                found[0].message,
                "Use `enable-next` instead of an `enable`/`disable` pair around a single statement."
            );
        }
    }
    #[test]
    fn plain_pairs_report_exactly_and_correct() {
        for prefix in ["rubocop", "rustocop"] {
            for mode in ["disable", "todo"] {
                let s = format!(
                    "  # {prefix}:{mode} Metrics/AbcSize -- reason\ndef foo\n  bar\nend\n# {prefix}:enable Metrics/AbcSize\n"
                );
                let found = offenses(&s);
                assert_eq!(found.len(), 1, "{s}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 3, 1));
                assert_eq!(
                    found[0].message,
                    format!(
                        "Use `{mode}-next` instead of a `{mode}`/`enable` pair around a single statement."
                    )
                );
                assert!(
                    crate::cops::testing::correction_offenses(&DirectiveScope, &s)[0]
                        .correction
                        .is_some()
                );
            }
        }
    }
}
