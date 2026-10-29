//! `Style/RedundantLineContinuation` — remove only syntax-noise backslashes.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    parser::Continuation,
    source::SourceFile,
};
use ruby_prism::Node;
use std::collections::HashMap;

/// Only the main lexer's code-mode escapes enter this index: never comments,
/// string/regexp/command text, percent-array separators, or heredoc payloads.
/// A regular newline is proved redundant at a *completed* AST boundary, not
/// merely because the next token happens to parse as some valid Ruby.
pub(crate) struct Facts {
    records: Vec<Continuation>,
    by_previous_end: HashMap<usize, Vec<usize>>,
    by_next_start: HashMap<usize, Vec<usize>>,
    redundant: Vec<bool>,
    stable_tokens: Vec<bool>,
    header_separators: Vec<bool>,
    #[cfg(test)]
    gap_bytes: usize,
    #[cfg(test)]
    boundary_probes: usize,
    #[cfg(test)]
    chain_steps: usize,
}
impl Facts {
    pub(crate) fn new(file: &SourceFile, records: &[Continuation]) -> Self {
        let bytes = &file.bytes;
        let mut by_previous_end = HashMap::<_, Vec<_>>::new();
        let mut by_next_start = HashMap::<_, Vec<_>>::new();
        let mut redundant = Vec::with_capacity(records.len());
        // Prism has already diagnosed the precise command-argument prefixes
        // whose classification depends on the slash being non-whitespace.
        let ambiguous: std::collections::HashSet<_> = file
            .warnings()
            .iter()
            .filter(|w| {
                w.message.starts_with("ambiguous")
                    && (w.message.contains("argument") || w.message.contains("prefix"))
            })
            .map(|w| w.range.end)
            .collect();
        let mut stable_tokens = Vec::with_capacity(records.len());
        let mut header_separators = Vec::with_capacity(records.len());
        // Trust the main parser's embedded-comment ranges, not `=begin`
        // lookalikes in literal text. Production records these before dispatch.
        let embedded_comments: HashMap<_, _> = file
            .comments()
            .iter()
            .filter(|r| bytes.get(r.start) == Some(&b'='))
            .map(|r| (r.start, r.end))
            .collect();
        let offsets: HashMap<_, _> = records
            .iter()
            .enumerate()
            .map(|(i, r)| (r.offset, i))
            .collect();
        #[cfg(test)]
        let mut gap_bytes = 0;
        let mut gaps = Vec::with_capacity(records.len());
        for record in records {
            let gap = next_content(bytes, record.following_line, &embedded_comments);
            #[cfg(test)]
            {
                gap_bytes += gap.0 - record.following_line;
            }
            gaps.push(gap);
        }
        // Connect gaps at lexer-observed escapes only. Memoize suffix facts
        // once, even for a very long chain; never rescan a common trivia tail.
        // Following offsets strictly increase, independently of callback order
        // (heredoc interpolation can make that order differ from source order).
        let links: Vec<_> = records
            .iter()
            .zip(&gaps)
            .map(|(r, gap)| offsets.get(&gap.0).copied().filter(|_| gap.0 > r.offset))
            .collect();
        #[cfg(test)]
        let mut chain_steps = 0;
        let mut resolved = vec![false; records.len()];
        let mut path = Vec::new();
        for start in 0..records.len() {
            let mut i = start;
            while !resolved[i] {
                #[cfg(test)]
                {
                    chain_steps += 1;
                }
                resolved[i] = true;
                path.push(i);
                let Some(next) = links[i] else { break };
                i = next;
            }
            while let Some(i) = path.pop() {
                #[cfg(test)]
                {
                    chain_steps += 1;
                }
                if let Some(next) = links[i] {
                    gaps[i].0 = gaps[next].0;
                    // Deleting intervening escapes leaves blank physical
                    // lines: these facts must NOT prove a leading-dot chain.
                    gaps[i].1 = false;
                    gaps[i].2 |= gaps[next].2;
                }
            }
        }
        for (i, record) in records.iter().enumerate() {
            by_previous_end
                .entry(record.previous_end)
                .or_default()
                .push(i);
            let (next, no_blank_line, header_separator) = gaps[i];
            header_separators.push(header_separator);
            by_next_start.entry(next).or_default().push(i);
            let chain = no_blank_line
                && (bytes.get(next..next + 2) == Some(b"&.")
                    || bytes.get(next) == Some(&b'.') && bytes.get(next + 1) != Some(&b'.'));
            stable_tokens.push(!ambiguous.contains(&record.offset));
            redundant.push(record.ignored || chain);
        }
        Self {
            records: records.to_vec(),
            by_previous_end,
            by_next_start,
            redundant,
            stable_tokens,
            header_separators,
            #[cfg(test)]
            gap_bytes,
            #[cfg(test)]
            boundary_probes: 0,
            #[cfg(test)]
            chain_steps,
        }
    }
    fn boundary(&mut self, end: usize) {
        #[cfg(test)]
        {
            self.boundary_probes += 1;
        }
        if let Some(indices) = self.by_previous_end.get(&end) {
            for &i in indices {
                self.redundant[i] = true;
            }
        }
    }
    // The main AST proves a completed header; the gap must independently
    // contain a separator that survives deletion. In particular, `do` alone
    // is NOT proof: a newline before a loop's `do` would change its binding.
    fn header_boundary(&mut self, end: usize) {
        #[cfg(test)]
        {
            self.boundary_probes += 1;
        }
        if let Some(indices) = self.by_previous_end.get(&end) {
            for &i in indices {
                self.redundant[i] |= self.header_separators[i];
            }
        }
    }
    fn before(&mut self, start: usize) {
        #[cfg(test)]
        {
            self.boundary_probes += 1;
        }
        if let Some(indices) = self.by_next_start.get(&start) {
            for &i in indices {
                self.redundant[i] = true;
            }
        }
    }
}

/// Ruby permits leading-dot chains through comment lines, but not a blank
/// physical line. Each inspected trivia gap ends at the next escape or token;
/// adjacent candidates cannot repeatedly scan a common suffix.
fn next_content(
    bytes: &[u8],
    mut next: usize,
    embedded_comments: &HashMap<usize, usize>,
) -> (usize, bool, bool) {
    let mut no_blank_line = true;
    let mut header_separator = false;
    loop {
        while matches!(
            bytes.get(next),
            Some(b' ' | b'\t' | b'\r' | b'\x0b' | b'\x0c')
        ) {
            next += 1;
        }
        if bytes.get(next) == Some(&b'\n') {
            no_blank_line = false;
            header_separator = true;
            next += 1;
            continue;
        }
        if let Some(&end) = embedded_comments.get(&next) {
            next = end;
            no_blank_line = false;
            // Prism skips the comment's internal/trailing newlines. They
            // cannot independently separate a header from a following `do`.
            continue;
        }
        if bytes.get(next) != Some(&b'#') {
            break;
        }
        let Some(end) = memchr::memchr(b'\n', &bytes[next..]) else {
            return (bytes.len(), no_blank_line, header_separator);
        };
        header_separator = true;
        next += end + 1;
    }
    (
        next,
        no_blank_line,
        header_separator || bytes.get(next) == Some(&b';'),
    )
}

fn statement_boundary(cx: &NodeContext<'_, '_>) -> bool {
    if cx.parent().is_none_or(|p| p.as_statements_node().is_none()) {
        return false;
    }
    let Some(owner) = cx.ancestors().nth(1) else {
        return false;
    };
    if let Some(n) = owner.as_if_node() {
        return n
            .if_keyword_loc()
            .is_none_or(|loc| loc.start_offset() == owner.location().start_offset());
    }
    if let Some(n) = owner.as_unless_node() {
        return n.keyword_loc().start_offset() == owner.location().start_offset();
    }
    if let Some(n) = owner.as_while_node() {
        return n.keyword_loc().start_offset() == owner.location().start_offset();
    }
    if let Some(n) = owner.as_until_node() {
        return n.keyword_loc().start_offset() == owner.location().start_offset();
    }
    true
}

pub struct RedundantLineContinuation;
impl Cop for RedundantLineContinuation {
    fn name(&self) -> &'static str {
        "Style/RedundantLineContinuation"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_leave_node(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        _out: &mut Vec<Offense>,
    ) {
        let Some(facts) = file.line_continuations() else {
            return;
        };
        let mut facts = facts.lock().unwrap();
        // A child of StatementsNode is already complete before this escape.
        // If an operator, modifier, argument, block, or implicit concatenation
        // used the escape, the statement would extend beyond it instead.
        if statement_boundary(cx) {
            facts.boundary(node.location().end_offset());
        }
        // Class/module headers end independently of their bodies. Do not
        // mistake the gap before a superclass for a completed header.
        if let Some(class) = node.as_class_node() {
            facts.boundary(
                class
                    .superclass()
                    .unwrap_or_else(|| class.constant_path())
                    .location()
                    .end_offset(),
            );
        } else if let Some(module) = node.as_module_node() {
            facts.boundary(module.constant_path().location().end_offset());
        } else if let Some(n) = node.as_singleton_class_node() {
            facts.header_boundary(n.expression().location().end_offset());
        } else if let Some(n) = node.as_case_node() {
            if let Some(predicate) = n.predicate() {
                facts.header_boundary(predicate.location().end_offset());
            }
        } else if let Some(n) = node.as_case_match_node() {
            if let Some(predicate) = n.predicate() {
                facts.header_boundary(predicate.location().end_offset());
            }
        } else if let Some(n) = node.as_rescue_node() {
            if let Some(then) = n.then_keyword_loc() {
                facts.before(then.start_offset());
            }
            // Only the *complete* exception/target header owns separators.
            // Before `=> target`, the last exception alone is not complete.
            if let Some(last) = n.reference().or_else(|| n.exceptions().last()) {
                facts.header_boundary(last.location().end_offset());
            }
        }
        // Closing delimiters and explicit header terminators consume a
        // newline without changing the completed expression before them.
        let closing = match node {
            Node::CallNode { .. } => node.as_call_node().unwrap().closing_loc(),
            Node::ArrayNode { .. } => node.as_array_node().unwrap().closing_loc(),
            Node::HashNode { .. } => Some(node.as_hash_node().unwrap().closing_loc()),
            Node::ParenthesesNode { .. } => Some(node.as_parentheses_node().unwrap().closing_loc()),
            Node::DefNode { .. } => node.as_def_node().unwrap().rparen_loc(),
            Node::DefinedNode { .. } => node.as_defined_node().unwrap().rparen_loc(),
            Node::SuperNode { .. } => node.as_super_node().unwrap().rparen_loc(),
            Node::BlockParametersNode { .. } => {
                node.as_block_parameters_node().unwrap().closing_loc()
            }
            Node::ArrayPatternNode { .. } => node.as_array_pattern_node().unwrap().closing_loc(),
            Node::FindPatternNode { .. } => node.as_find_pattern_node().unwrap().closing_loc(),
            Node::HashPatternNode { .. } => node.as_hash_pattern_node().unwrap().closing_loc(),
            Node::IndexAndWriteNode { .. } => {
                Some(node.as_index_and_write_node().unwrap().closing_loc())
            }
            Node::IndexOrWriteNode { .. } => {
                Some(node.as_index_or_write_node().unwrap().closing_loc())
            }
            Node::IndexOperatorWriteNode { .. } => {
                Some(node.as_index_operator_write_node().unwrap().closing_loc())
            }
            Node::MultiTargetNode { .. } => node.as_multi_target_node().unwrap().rparen_loc(),
            Node::MultiWriteNode { .. } => node.as_multi_write_node().unwrap().rparen_loc(),
            Node::BlockNode { .. } => Some(node.as_block_node().unwrap().closing_loc()),
            Node::EmbeddedStatementsNode { .. } => {
                Some(node.as_embedded_statements_node().unwrap().closing_loc())
            }
            _ => None,
        };
        if let Some(close) = closing.filter(|c| matches!(c.as_slice(), b")" | b"]" | b"}" | b"|")) {
            facts.before(close.start_offset());
        }
        if let Some(n) = node.as_when_node() {
            if cx.parent().is_some_and(|p| p.as_case_node().is_some()) {
                facts.before(n.keyword_loc().start_offset());
            }
            if let Some(then) = n.then_keyword_loc() {
                facts.before(then.start_offset());
            }
            if let Some(last) = n.conditions().last() {
                facts.header_boundary(last.location().end_offset());
            }
        } else if let Some(n) = node.as_in_node() {
            if cx
                .parent()
                .is_some_and(|p| p.as_case_match_node().is_some())
            {
                facts.before(n.in_loc().start_offset());
            }
            if let Some(then) = n.then_loc() {
                facts.before(then.start_offset());
            }
            facts.header_boundary(n.pattern().location().end_offset());
        } else if let Some(n) = node.as_multi_write_node() {
            // A comma-bearing, unparenthesized MLHS continues before `=`.
            // A closed outer MLHS or sole splat does not: `(x, y)\n= values`
            // and `*x\n= values` are invalid. Nested target parentheses do
            // not close the *outer* MLHS; lefts/rights prove its comma.
            if n.rparen_loc().is_none() && (!n.lefts().is_empty() || !n.rights().is_empty()) {
                facts.before(n.operator_loc().start_offset());
            }
        } else if let Some(n) = node.as_defined_node() {
            facts.boundary(n.keyword_loc().end_offset());
        }
        // Prefix headers only: neither a modifier nor ternary punctuation
        // proves that a newline before the condition's end is redundant.
        if let Some(n) = node.as_if_node().filter(|n| {
            n.if_keyword_loc()
                .is_some_and(|loc| loc.start_offset() == node.location().start_offset())
        }) {
            let end = n.predicate().location().end_offset();
            if n.then_keyword_loc()
                .is_some_and(|loc| loc.as_slice() == b"then")
            {
                facts.boundary(end);
            } else {
                facts.header_boundary(end);
            }
        } else if let Some(n) = node
            .as_unless_node()
            .filter(|n| n.keyword_loc().start_offset() == node.location().start_offset())
        {
            let end = n.predicate().location().end_offset();
            if n.then_keyword_loc()
                .is_some_and(|loc| loc.as_slice() == b"then")
            {
                facts.boundary(end);
            } else {
                facts.header_boundary(end);
            }
        } else if let Some(n) = node
            .as_while_node()
            .filter(|n| n.keyword_loc().start_offset() == node.location().start_offset())
        {
            facts.header_boundary(n.predicate().location().end_offset());
        } else if let Some(n) = node
            .as_until_node()
            .filter(|n| n.keyword_loc().start_offset() == node.location().start_offset())
        {
            facts.header_boundary(n.predicate().location().end_offset());
        } else if let Some(n) = node.as_for_node() {
            facts.header_boundary(n.collection().location().end_offset());
        }
    }
    fn leave_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node
            .as_program_node()
            .is_none_or(|program| program.statements().body().is_empty())
        {
            return;
        }
        let Some(facts) = file.line_continuations() else {
            return;
        };
        let facts = facts.lock().unwrap();
        for (i, record) in facts.records.iter().enumerate() {
            if !facts.redundant[i] || !facts.stable_tokens[i] {
                continue;
            }
            let offset = record.offset;
            let (line, col) = file.line_col(offset);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "Redundant line continuation.".into(),
                line,
                col,
                end_line: line,
                // The physical newline, indentation, literal bytes, __LINE__,
                // and directive line spans survive. Shared source/code/heap,
                // DATA-position and heredoc guards still vet this deletion.
                correction: cx.corrections_requested().then(|| Correction {
                    range: offset..offset + 1,
                    replacement: String::new(),
                }),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's
    //! `spec/rubocop/cop/style/redundant_line_continuation_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<crate::offense::Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/RedundantLineContinuation")
            .collect()
    }
    #[test]
    fn bounded_main_dispatch_evidence_handles_many_required_and_redundant_escapes() {
        let source = "foo('a' \\\n 'b', \\\n 2)\nfoo +\\\n bar\n".repeat(2000);
        let file = SourceFile::new("many.rb".into(), source.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        assert_eq!(parsed.errors().count(), 0);
        file.record_continuations(&parsed.continuations);
        file.record_comments(&parsed);
        file.record_warnings(&parsed);
        file.record_encoding(&parsed);
        let mut found = Vec::new();
        crate::cops::run_cops_with_corrections(
            &[&RedundantLineContinuation],
            &parsed,
            &file,
            &mut found,
            false,
        );
        assert_eq!(found.len(), 2000);
        assert!(found.iter().all(|o| o.correction.is_none()));
        let index = file.line_continuations().unwrap();
        let facts = index.lock().unwrap();
        assert_eq!(facts.records.len(), 6000);
        assert!(facts.gap_bytes <= file.bytes.len());
        assert!(facts.boundary_probes <= 12 * 2000);
        assert_eq!(
            facts.by_previous_end.values().map(Vec::len).sum::<usize>(),
            6000
        );
        assert_eq!(
            facts.by_next_start.values().map(Vec::len).sum::<usize>(),
            6000
        );
        drop(facts);
        assert!(std::ptr::eq(index, file.line_continuations().unwrap()));
    }
    #[test]
    fn review_header_and_delimiter_evidence_stays_bounded() {
        let source =
            "if foo \\\n# comment\n\n bar; end\nwhile foo \\\ndo bar; end\nfoo { |a,b \\\n| a }\n"
                .repeat(2000);
        let file = SourceFile::new("headers.rb".into(), source.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        assert_eq!(parsed.errors().count(), 0);
        file.record_continuations(&parsed.continuations);
        file.record_comments(&parsed);
        file.record_warnings(&parsed);
        file.record_encoding(&parsed);
        let mut found = Vec::new();
        crate::cops::run_cops_with_corrections(
            &[&RedundantLineContinuation],
            &parsed,
            &file,
            &mut found,
            false,
        );
        assert_eq!(found.len(), 4000);
        let facts = file.line_continuations().unwrap().lock().unwrap();
        assert_eq!(facts.records.len(), 6000);
        assert!(facts.gap_bytes <= file.bytes.len());
        assert!(facts.boundary_probes <= 12 * 2000);
    }
    #[test]
    fn completed_headers_and_parenthesized_mlhs_keep_index_work_linear() {
        let source = "class << obj \\\n# comment\nfoo;end\ncase x \\\nwhen 1;foo;end\nbegin;foo;rescue E => e \\\n\nbar;end\n(x,(y,z)) \\\n= a,b\n".repeat(1500);
        let file = SourceFile::new("headers.rb".into(), source.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        assert_eq!(parsed.errors().count(), 0);
        file.record_continuations(&parsed.continuations);
        file.record_comments(&parsed);
        file.record_warnings(&parsed);
        file.record_encoding(&parsed);
        let mut found = Vec::new();
        crate::cops::run_cops_with_corrections(
            &[&RedundantLineContinuation],
            &parsed,
            &file,
            &mut found,
            false,
        );
        assert_eq!(found.len(), 4500);
        assert!(found.iter().all(|o| o.correction.is_none()));
        let facts = file.line_continuations().unwrap().lock().unwrap();
        assert_eq!(facts.records.len(), 6000);
        assert!(facts.gap_bytes <= file.bytes.len());
        assert!(facts.boundary_probes <= 24 * 1500);
    }
    #[test]
    fn long_continuation_chain_shares_embedded_comment_suffix_once() {
        let source = format!(
            "case x \\\n{}=begin\n{}\n=end\nwhen 1;foo;end\n",
            "\\\n".repeat(6000),
            "comment \\\n".repeat(6000)
        );
        let file = SourceFile::new("chain.rb".into(), source.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        assert_eq!(parsed.errors().count(), 0);
        file.record_continuations(&parsed.continuations);
        file.record_comments(&parsed);
        file.record_warnings(&parsed);
        let mut found = Vec::new();
        crate::cops::run_cops_with_corrections(
            &[&RedundantLineContinuation],
            &parsed,
            &file,
            &mut found,
            false,
        );
        assert_eq!(found.len(), 6001);
        let facts = file.line_continuations().unwrap().lock().unwrap();
        assert_eq!(facts.records.len(), 6001);
        assert!(facts.gap_bytes <= file.bytes.len());
        assert_eq!(facts.chain_steps, 2 * facts.records.len());
        assert!(facts.boundary_probes < 20);
        assert_eq!(facts.by_next_start.len(), 1);
    }
    #[test]
    fn continuation_chains_do_not_prove_required_expressions_or_leading_dots() {
        for source in [
            "class << obj \\\n\\\n (argument);foo;end\n",
            "case x \\\n\\\n + y;when 1;foo;end\n",
            "begin;foo;rescue E \\\n\\\n => e;bar;end\n",
            "while foo \\\n\\\ndo bar;end\n",
            "until foo \\\n\\\ndo bar;end\n",
            "for x in values \\\n\\\ndo bar;end\n",
            "while foo \\\n=begin\ncomment\n=end\ndo bar;end\n",
            "until foo \\\n=begin\ncomment\n=end\ndo bar;end\n",
            "for x in values \\\n=begin\ncomment\n=end\ndo bar;end\n",
            "class << obj \\\n=begin\ncomment\n=end\nfoo;end\n",
            "(x, y) \\\n\\\n= values\n",
        ] {
            assert_eq!(
                crate::parser::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            assert!(offenses(source).is_empty(), "{source}");
        }
        // Only the second slash can go: removing both leaves a blank line
        // that prevents the dot from chaining to foo.
        assert_correction(
            &RedundantLineContinuation,
            "foo \\\n\\\n .bar\n",
            "foo \\\n\n .bar\n",
        );
        assert!(offenses("foo \\\n\n .bar\n").is_empty());
    }
    #[test]
    fn all_31_pinned_correction_pairs_apply_and_stabilize() {
        let pairs: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_line_continuation_corrections.json"
        ))
        .unwrap();
        assert_eq!(pairs.as_array().unwrap().len(), 31);
        for pair in pairs.as_array().unwrap() {
            let before = pair["before"].as_str().unwrap();
            let after = pair["after"].as_str().unwrap();
            assert_correction(&RedundantLineContinuation, before, after);
            assert!(offenses(after).is_empty(), "{after}");
        }
    }
    #[test]
    fn canonical_ignored_newlines_and_exact_diagnostic() {
        for source in [
            "foo. \\\n  bar\n",
            "foo&. \\\n  bar\n",
            "[foo, \\\n bar]\n",
            "{foo: \\\n bar}\n",
            "foo(bar, \\\n baz)\n",
            "return(\\\n foo\n)\n",
            "foo = do_something( \\\n argument)\n",
            "def foo(bar, \\\n baz)\nend\n",
            "def self.foo(bar, \\\n baz)\nend\n",
            "foo do \\\n bar\nend\n",
            "a, b, \\\n c = [1, 2, 3]\n",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            let slash = source.find('\\').unwrap();
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, slash as u32 + 1, 1)
            );
            assert_eq!(found[0].message, "Redundant line continuation.");
            assert_correction(
                &RedundantLineContinuation,
                source,
                &source.replacen('\\', "", 1),
            );
        }
    }
    #[test]
    fn canonical_statement_header_chain_and_eof_boundaries() {
        for source in [
            "class Foo \\\nend\n",
            "module Foo \\\n x\nend\n",
            "if foo \\\nthen bar\nend\n",
            "foo \\\n .bar \\\n .baz\n",
            "obj\n .foo(42) \\\n .bar\n",
            "obj.\n foo(42). \\\n\n bar\n",
            "foo 'bar' \\\n\n'baz'\n",
            "def foo\n bar \\\nend\n",
            "def foo\n super \\\nend\n",
            "foo \\\n",
            "def foo\n bar\nend\nsome_code \\\n",
            "foo \\\n\n__END__\ndata \\\n",
            "def foo\n 'first' \\\n 'second' \\\nend\n",
        ] {
            let expected = if source.contains("'first'") {
                source.replacen("'second' \\", "'second' ", 1)
            } else if source.contains("__END__") {
                source.replacen('\\', "", 1)
            } else {
                source.replace('\\', "")
            };
            assert_correction(&RedundantLineContinuation, source, &expected);
        }
    }
    #[test]
    fn ignored_newlines_must_not_retokenize_ambiguous_argument_prefixes() {
        for token in ["+", "-", "*", "**", "&"] {
            let source = format!("foo {token}\\\n bar\n");
            assert!(offenses(&source).is_empty(), "{source}");
            let source = format!("x = \"#{{foo {token}\\\n bar}}\"\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        for source in ["foo + \\\n bar\n", "(+\\\n 1)\n", "foo(* \\\n bar)\n"] {
            assert_correction(
                &RedundantLineContinuation,
                source,
                &source.replace('\\', ""),
            );
        }
    }
    #[test]
    fn grammar_terminators_forwarding_patterns_and_defined() {
        for source in [
            "{ a: 1, b: 2\\\n }\n",
            "foo(1\\\n\n # comment\n)\n",
            "case x; when 1\\\n\n then 2; end\n",
            "[foo\\\n]\n",
            "foo(bar\\\n)\n",
            "def foo(...\\\n); bar(...); end\n",
            "case foo; when 1, 2\\\n then bar; end\n",
            "case foo; in [a, b]\\\n then a + b; end\n",
            "foo ? bar\\\n : baz\n",
            "foo ? bar ? baz : quux\\\n : other\n",
            "a, b, c\\\n = 1, 2, 3\n",
            "a, (b, c)\\\n = values\n",
            "defined?\\\n foo\n",
        ] {
            assert_correction(
                &RedundantLineContinuation,
                source,
                &source.replace('\\', ""),
            );
        }
    }
    #[test]
    fn review_missing_typed_closing_delimiters() {
        for source in [
            "foo { |a, b \\\n| [a,b] }\n",
            "foo { |a; b \\\n| a }\n",
            "->(a, b \\\n) { [a,b] }\n",
            "defined?(foo \\\n)\n",
            "def foo; super(1, 2 \\\n); end\n",
            "obj[1, 2 \\\n] ||= value\n",
            "obj[1, 2 \\\n] &&= value\n",
            "obj[1, 2 \\\n] += value\n",
            "case x; in [a, b \\\n]; a; end\n",
            "case x; in [*, a, * \\\n]; a; end\n",
            "case x; in {a:, b: \\\n}; a; end\n",
            "case x; in Point(a, b \\\n); a; end\n",
            "a, (b, c \\\n) = values\n",
        ] {
            assert_eq!(
                crate::parser::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, source.find('\\').unwrap() as u32 + 1, 1),
                "{source}"
            );
            assert_eq!(found[0].message, "Redundant line continuation.");
            assert_correction(
                &RedundantLineContinuation,
                source,
                &source.replace('\\', ""),
            );
        }
    }
    #[test]
    fn review_header_separators() {
        for source in [
            "if foo \\\n; bar; end\n",
            "if foo \\\n\n bar\nend\n",
            "unless foo \\\n; bar; end\n",
            "unless foo \\\n\n bar\nend\n",
            "while foo \\\n; bar; end\n",
            "while foo \\\n\n bar\nend\n",
            "until foo \\\n; bar; end\n",
            "until foo \\\n\n bar\nend\n",
            "if foo \\\n # comment\n bar\nend\n",
            "if foo \\\r\n \r\n bar\r\nend\r\n",
            "for x in values \\\n; bar; end\n",
            "case x; when 1, 2 \\\n; bar; end\n",
            "case x; in [a, b] \\\n\n a; end\n",
        ] {
            assert_eq!(
                crate::parser::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, source.find('\\').unwrap() as u32 + 1, 1),
                "{source}"
            );
            assert_eq!(found[0].message, "Redundant line continuation.");
            assert_correction(
                &RedundantLineContinuation,
                source,
                &source.replace('\\', ""),
            );
        }
    }
    #[test]
    fn review_delimiter_and_header_necessary_continuations() {
        for source in [
            "foo { |a = (foo \\\n + bar)| a }\n",
            "defined?(foo \\\n + bar)\n",
            "def foo; super(1 \\\n + 2); end\n",
            "obj[1 \\\n + 2] ||= value\n",
            "obj[1 \\\n + 2] &&= value\n",
            "obj[1 \\\n + 2] += value\n",
            "case x; in [\"a\" \\\n \"b\"]; end\n",
            "if foo \\\n + bar\n baz\nend\n",
            "unless foo \\\n + bar\n baz\nend\n",
            "while foo \\\ndo bar; end\n",
            "until foo \\\ndo bar; end\n",
            "for x in values \\\ndo bar; end\n",
            "bar \\\n if foo\n",
            "bar \\\n while foo\n",
        ] {
            assert_eq!(
                crate::parser::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            assert!(offenses(source).is_empty(), "{source}");
            assert!(
                crate::cops::testing::correction_offenses(&RedundantLineContinuation, source)
                    .is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn parenthesized_mlhs_requires_escape_before_assignment_operator() {
        for nl in ["\n", "\r\n"] {
            for target in [
                "(x, y)",
                "(x, (y, z))",
                "((x, y))",
                "(x,)",
                "(*x)",
                "(obj.x, ary[0])",
                "*x",
                "x",
                "obj.x",
                "ary[0]",
            ] {
                let source = format!("{target} \\{nl}= a, b{nl}");
                assert_eq!(
                    crate::parser::parse(source.as_bytes()).errors().count(),
                    0,
                    "{source}"
                );
                assert!(offenses(&source).is_empty(), "{source}");
                assert!(
                    crate::cops::testing::correction_offenses(&RedundantLineContinuation, &source)
                        .is_empty(),
                    "{source}"
                );
            }
            for target in [
                "x, y",
                "x, (y, z)",
                "(x, y), z",
                "x,",
                "x, *y",
                "*x, y",
                "obj.x, ary[0]",
            ] {
                let source = format!("{target} \\{nl}= a, b{nl}");
                assert_eq!(
                    crate::parser::parse(source.as_bytes()).errors().count(),
                    0,
                    "{source}"
                );
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    (found[0].line, found[0].col),
                    (1, target.len() as u32 + 2),
                    "{source}"
                );
                assert_eq!(found[0].message, "Redundant line continuation.");
                assert_correction(
                    &RedundantLineContinuation,
                    &source,
                    &source.replace('\\', ""),
                );
            }
        }
    }
    #[test]
    fn completed_singleton_case_and_rescue_headers() {
        for nl in ["\n", "\r\n"] {
            for (header, tails) in [
                (
                    "class << obj",
                    vec!["; foo; end", "\nfoo; end", "# comment\nfoo; end"],
                ),
                (
                    "case x",
                    vec![
                        "when 1 then foo end",
                        ";when 1; foo; end",
                        "\nwhen 1; foo; end",
                        "# comment\nwhen 1; foo; end",
                        ";in [a]; a; end",
                        "\nin [a]; a; end",
                        "# comment\nin [a]; a; end",
                    ],
                ),
                (
                    "begin;foo;rescue E",
                    vec![
                        "then bar;end",
                        ";bar;end",
                        "\nbar;end",
                        "# comment\nbar;end",
                    ],
                ),
                (
                    "begin;foo;rescue E => e",
                    vec![
                        "then bar;end",
                        ";bar;end",
                        "\nbar;end",
                        "# comment\nbar;end",
                    ],
                ),
                (
                    "begin;foo;rescue E1, E2 => e",
                    vec![
                        "then bar;end",
                        ";bar;end",
                        "\nbar;end",
                        "# comment\nbar;end",
                    ],
                ),
            ] {
                for tail in tails {
                    let source = format!("{header} \\\n{tail}\n").replace('\n', nl);
                    assert_eq!(
                        crate::parser::parse(source.as_bytes()).errors().count(),
                        0,
                        "{source}"
                    );
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!(
                        (found[0].line, found[0].col, found[0].end_line),
                        (1, header.len() as u32 + 2, 1),
                        "{source}"
                    );
                    assert_eq!(found[0].message, "Redundant line continuation.");
                    assert_correction(
                        &RedundantLineContinuation,
                        &source,
                        &source.replace('\\', ""),
                    );
                }
            }
        }
    }
    #[test]
    fn completed_headers_through_continuation_and_embedded_comment_trivia() {
        for nl in ["\n", "\r\n"] {
            for (header, tail) in [
                ("class << obj", ";foo;end"),
                ("case x", "when 1;foo;end"),
                ("case x", ";in [a];a;end"),
                ("begin;foo;rescue E", "then bar;end"),
                ("begin;foo;rescue E => e", ";bar;end"),
                ("if foo", ";bar;end"),
                ("while foo", ";bar;end"),
            ] {
                for trivia in [
                    "\\\n",
                    "\\\n  \\\n",
                    "=begin\ncomment \\\n=end\n",
                    "\\\n=begin\ncomment\n=end\n\\\n",
                    "# comment\n\\\n\n",
                ] {
                    let source = format!("{header} \\\n{trivia}{tail}\n").replace('\n', nl);
                    let parsed = crate::parser::parse(source.as_bytes());
                    assert_eq!(parsed.errors().count(), 0, "{source}");
                    let expected: Vec<_> = parsed
                        .continuations
                        .iter()
                        .map(|r| {
                            let file =
                                SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
                            file.line_col(r.offset)
                        })
                        .collect();
                    let found = offenses(&source);
                    assert_eq!(
                        found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                        expected,
                        "{source}"
                    );
                    assert!(found.iter().all(
                        |o| o.message == "Redundant line continuation." && o.end_line == o.line
                    ));
                    // Embedded-comment backslashes are payload, never edits.
                    let mut fixed = source.as_bytes().to_vec();
                    let mut offsets: Vec<_> =
                        parsed.continuations.iter().map(|r| r.offset).collect();
                    offsets.sort_unstable();
                    for offset in offsets.into_iter().rev() {
                        fixed.remove(offset);
                    }
                    let fixed = String::from_utf8(fixed).unwrap();
                    assert_correction(&RedundantLineContinuation, &source, &fixed);
                    assert!(offenses(&fixed).is_empty(), "{fixed}");
                }
            }
        }
    }
    #[test]
    fn completed_header_proof_does_not_split_required_expressions() {
        for nl in ["\n", "\r\n"] {
            for source in [
                "class << obj \\\n (argument); foo; end\n",
                "class << obj \\\n + other; foo; end\n",
                "case x \\\n + y; when 1; foo; end\n",
                "case x \\\n (argument); in [a]; a; end\n",
                "begin;foo;rescue E \\\n => e;bar;end\n",
                "begin;foo;rescue E \\\n , Other;bar;end\n",
                "begin;foo;rescue \\\n E;bar;end\n",
                "begin;foo;rescue E \\\n + other;bar;end\n",
                "while foo \\\ndo bar; end\n",
            ] {
                let source = source.replace('\n', nl);
                assert_eq!(
                    crate::parser::parse(source.as_bytes()).errors().count(),
                    0,
                    "{source}"
                );
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
    }
    #[test]
    fn closing_outer_mlhs_delimiter_accepts_newline_before_it_not_after_it() {
        for nl in ["\n", "\r\n"] {
            for source in [
                "(x, y \\\n) = a, b\n",
                "(x, (y, z) \\\n) = a, b\n",
                "(x, \\\n) = a, b\n",
                "(obj.x, ary[0] \\\n) = a, b\n",
            ] {
                let source = source.replace('\n', nl);
                assert_eq!(
                    crate::parser::parse(source.as_bytes()).errors().count(),
                    0,
                    "{source}"
                );
                assert_correction(
                    &RedundantLineContinuation,
                    &source,
                    &source.replace('\\', ""),
                );
            }
        }
    }
    #[test]
    fn empty_ruby_has_no_ast_continuation_investigation() {
        for source in ["", "\\\n", " \\\r\n# only a comment\n"] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn heredoc_payload_is_not_leading_dot_code_evidence() {
        for body in [".foo", "# body\n.foo", "  &.foo"] {
            for spacing in ["", " "] {
                let source = format!("x = <<~TXT{spacing}\\\n{body}\nTXT\n + 2\n");
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        // Leading-dot evidence must instead come from resumed *code*.
        let source = "x = <<~TXT \\\npayload\nTXT\n .strip\n";
        assert_correction(
            &RedundantLineContinuation,
            source,
            &source.replace('\\', ""),
        );
    }
    #[test]
    fn queued_heredoc_opening_uses_physical_header_boundary_not_body_token_end() {
        for source in [
            "foo <<~TXT\\\n\nbody #{1 + 2}\nTXT\n",
            "x = <<~TXT\\\n\n#{foo(1, 2)}\nTXT\n",
            "foo(<<~A, \\\nfirst\nA\n  <<~B)\nsecond\nB\n",
        ] {
            assert_correction(
                &RedundantLineContinuation,
                source,
                &source.replacen('\\', "", 1),
            );
        }
    }
    #[test]
    fn necessary_operators_arguments_modifiers_and_literal_contents() {
        for source in [
            "1 \\\n + 2 \\\n - 3 \\\n * 4 \\\n / 5 \\\n % 6 \\\n ** 7\n",
            "foo \\\n && bar\n",
            "foo \\\n || bar\n",
            "foo \\\n & bar \\\n | baz\n",
            "foo \\\n ? bar : baz\n",
            "bar \\\n if foo\n",
            "bar \\\n unless foo\n",
            "bar \\\n while foo\n",
            "bar \\\n until foo\n",
            "bar \\\n rescue foo\n",
            "return \\\n foo\n",
            "def foo\n yield \\\n bar\nend\n",
            "foo do\n break \\\n bar\nend\n",
            "foo do\n next \\\n bar\nend\n",
            "some_method \\\n (argument)\n",
            "foo = do_something \\\n argument\n",
            "foo('bar' \\\n 'baz')\n",
            "foo 'bar' \\\n \\\n 'baz'\n",
            "obj\n .foo(42) \\\n\n .bar\n",
            "obj\n &.foo(42) \\\n \t\n .bar\n",
            "'foo \\\n bar'\n",
            "\"foo \\\n #{bar}\"\n",
            "`never \\\n #{executed}`\n",
            "/pattern \\\n #{bar}/\n",
            ":\"symbol \\\n #{bar}\"\n",
            "<<~SQL\nSELECT \\\n x\nSQL\n",
            "# comment \\\nfoo # \\\n",
            "def foo = 1 \\\n + 2\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
