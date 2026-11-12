//! `Gemspec/DeprecatedAttributeAssignment`: in each `Gem::Specification.new`
//! block, the first descendant (parser pre-order) that assigns `test_files`,
//! `date`, `specification_version` or `rubygems_version` on a receiver whose
//! source is the block's first argument, or that compound-assigns
//! `test_files`.
use std::collections::HashSet;

use ruby_prism::{BlockNode, Node, ParseResult};

use super::help::specification_block;
use crate::{
    cops::{
        Cop,
        bundler::{
            declarations::parser_order_nodes,
            ordered_gem_node::{inactive_program, pinned},
        },
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct DeprecatedAttributeAssignment;

const ATTRIBUTES: [&str; 4] = [
    "test_files",
    "date",
    "specification_version",
    "rubygems_version",
];

struct Candidate<'pr> {
    start: usize,
    end: usize,
    last_line: u32,
    /// Whether the subtree has a heredoc or binds a local variable.
    pinned: bool,
    attribute: &'static str,
    receiver: &'pr [u8],
    /// Enclosing specification blocks: (start offset, first argument source).
    blocks: Vec<(usize, &'pr [u8])>,
    /// Whether the node is a statement of a statement list.
    statement: bool,
}

impl Cop for DeprecatedAttributeAssignment {
    fn name(&self) -> &'static str {
        "Gemspec/DeprecatedAttributeAssignment"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn check_parse(&self, result: &ParseResult<'_>, file: &SourceFile, out: &mut Vec<Offense>) {
        let root = result.node();
        let candidates = parser_order_nodes(&root, |node, ancestors| {
            let (attribute, receiver) = assignment(node)?;
            let blocks: Vec<_> = ancestors
                .iter()
                .filter_map(|ancestor| {
                    let block = specification_block(&ancestor.as_call_node()?)?;
                    Some((ancestor.location().start_offset(), first_argument(&block)?))
                })
                .collect();
            (!blocks.is_empty()).then(|| Candidate {
                start: node.location().start_offset(),
                end: node.location().end_offset(),
                last_line: file.last_line_of(node.location()),
                pinned: pinned(node),
                attribute,
                receiver,
                blocks,
                statement: ancestors
                    .last()
                    .is_some_and(|parent| parent.as_statements_node().is_some()),
            })
        });
        // `detect` stops at each block's first match; an offense at a range
        // already reported (by an enclosing block) is not added again.
        let mut done = HashSet::new();
        let mut reported = HashSet::new();
        let mut editable = None;
        let first = out.len();
        for candidate in candidates {
            let mut matched = false;
            for &(block, argument) in &candidate.blocks {
                if argument == candidate.receiver && done.insert(block) {
                    matched = true;
                }
            }
            if !matched || !reported.insert((candidate.start, candidate.end)) {
                continue;
            }
            let (line, col) = crate::cops::lint::reference_line_col(candidate.start, file);
            let correction = (candidate.statement
                && *editable.get_or_insert_with(|| inactive_program(&root, file)))
            .then(|| removal(result, file, &candidate))
            .flatten();
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                col,
                end_line: candidate.last_line,
                message: format!("Do not set `{}` in gemspec.", candidate.attribute),
                correction,
            });
        }
        compose(result, file, &mut out[first..]);
    }
}

/// Each removal was checked alone; together they can still lose something
/// (two removed reads of one local leave it unused and warned about). Keep,
/// in the order the corrector selects them, only removals that do not
/// overlap an earlier kept one and keep the program free of errors and its
/// warnings unchanged together with every removal kept before them.
fn compose(result: &ParseResult<'_>, file: &SourceFile, offenses: &mut [Offense]) {
    let mut order: Vec<usize> = (0..offenses.len())
        .filter(|&i| offenses[i].correction.is_some())
        .collect();
    if order.len() < 2 {
        return;
    }
    order.sort_by_key(|&i| {
        let range = &offenses[i].correction.as_ref().unwrap().range;
        (range.start, range.end)
    });
    let before: Vec<_> = result
        .warnings()
        .map(|w| w.message().into_owned())
        .collect();
    let mut kept: Vec<std::ops::Range<usize>> = Vec::new();
    for i in order {
        let range = offenses[i].correction.as_ref().unwrap().range.clone();
        let fits = kept.last().is_none_or(|last| range.start >= last.end) && {
            let mut edited = Vec::with_capacity(file.bytes.len());
            let mut offset = 0;
            for removed in kept.iter().chain(std::iter::once(&range)) {
                edited.extend_from_slice(&file.bytes[offset..removed.start]);
                offset = removed.end;
            }
            edited.extend_from_slice(&file.bytes[offset..]);
            let reparsed = crate::parser::parse_as(&edited, file.ruby_version());
            let after: Vec<_> = reparsed
                .warnings()
                .map(|w| w.message().into_owned())
                .collect();
            reparsed.errors().next().is_none() && before == after
        };
        if fits {
            kept.push(range);
        } else {
            offenses[i].correction = None;
        }
    }
}

/// `use_deprecated_attributes?`: a `send` (never `csend`) of `attr=`
/// (a multiple-assignment, `for` or `rescue` target is such a send), or an
/// `op_asgn` whose lhs sends `test_files`. The pinned loop replaces the node
/// with its lhs on the first attribute, so a compound assignment only ever
/// matches `test_files`; `||=`/`&&=` never match.
fn assignment<'pr>(node: &Node<'pr>) -> Option<(&'static str, &'pr [u8])> {
    let setter = |name: &[u8]| {
        ATTRIBUTES
            .into_iter()
            .find(|attribute| name.strip_suffix(b"=") == Some(attribute.as_bytes()))
    };
    if let Some(call) = node.as_call_node() {
        if call.is_safe_navigation() {
            return None;
        }
        return Some((
            setter(call.name().as_slice())?,
            call.receiver()?.location().as_slice(),
        ));
    }
    if let Some(target) = node.as_call_target_node() {
        if target.is_safe_navigation() {
            return None;
        }
        return Some((
            setter(target.name().as_slice())?,
            target.receiver().location().as_slice(),
        ));
    }
    let write = node.as_call_operator_write_node()?;
    if write.is_safe_navigation() || write.read_name().as_slice() != b"test_files" {
        return None;
    }
    Some(("test_files", write.receiver()?.location().as_slice()))
}

/// `block_node.first_argument.source`: the first parameter (or block-local)
/// in source order. A block without parameters raises upstream and a
/// numbered or `it` block is not an `on_block` node: neither is checked.
fn first_argument<'pr>(block: &BlockNode<'pr>) -> Option<&'pr [u8]> {
    let params = block.parameters()?.as_block_parameters_node()?;
    let mut nodes: Vec<Node<'pr>> = params.locals().iter().collect();
    if let Some(list) = params.parameters() {
        nodes.extend(list.requireds().iter());
        nodes.extend(list.optionals().iter());
        nodes.extend(list.rest());
        nodes.extend(list.posts().iter());
        nodes.extend(list.keywords().iter());
        nodes.extend(list.keyword_rest());
        nodes.extend(list.block().map(|b| b.as_node()));
    }
    nodes
        .into_iter()
        .map(|node| node.location())
        .min_by_key(|location| location.start_offset())
        .map(|location| location.as_slice())
}

// The upstream edit deletes the assignment's whole lines. In a live gemspec
// that drops a call to an unknown receiver (and its value's effects), so a
// live finding keeps its bytes. The canonical removal is applied only in a
// closed top-level `if false ... end` program (no data section, no listed
// source observer), and only when nothing else is lost: the assignment is a
// complete statement alone on its lines, with no comment, heredoc or local
// binding there (removing a binding would turn a later read into a call),
// and the edited program parses with exactly the same warnings.
fn removal(
    result: &ParseResult<'_>,
    file: &SourceFile,
    candidate: &Candidate<'_>,
) -> Option<Correction> {
    let (start, end) = (candidate.start, candidate.end);
    let first = file.line_start(file.line_of(start));
    let last = file.line_start(candidate.last_line + 1);
    let after = &file.bytes[end..last];
    let after = after.strip_suffix(b"\n").unwrap_or(after);
    let after = after.strip_suffix(b"\r").unwrap_or(after);
    let blank = |bytes: &[u8]| bytes.iter().all(|b| matches!(b, b' ' | b'\t'));
    if !blank(&file.bytes[first..start])
        || !blank(after)
        || !file.bytes[..last].ends_with(b"\n")
        || candidate.pinned
    {
        return None;
    }
    let range = first..last;
    let inside = |offset: usize| range.contains(&offset);
    if result
        .comments()
        .any(|comment| inside(comment.location().start_offset()))
        || result
            .warnings()
            .any(|warning| inside(warning.location().start_offset()))
    {
        return None;
    }
    let mut edited = file.bytes[..first].to_vec();
    edited.extend_from_slice(&file.bytes[last..]);
    let reparsed = crate::parser::parse_as(&edited, file.ruby_version());
    let before: Vec<_> = result
        .warnings()
        .map(|w| w.message().into_owned())
        .collect();
    let after: Vec<_> = reparsed
        .warnings()
        .map(|w| w.message().into_owned())
        .collect();
    (reparsed.errors().next().is_none() && before == after).then(|| Correction {
        range,
        replacement: String::new(),
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's gemspec/deprecated_attribute_assignment_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov), plus lookalikes and the
    //! guarded correction.
    use super::*;
    use crate::pipeline::lint_path_with_corrections;

    fn offenses(path: &str, source: &str) -> Vec<Offense> {
        lint_path_with_corrections(&[&DeprecatedAttributeAssignment], path, source)
    }

    fn found(path: &str, source: &str) -> Vec<(u32, u32, u32, String)> {
        let mut found: Vec<_> = offenses(path, source)
            .into_iter()
            .map(|o| (o.line, o.col, o.end_line, o.message))
            .collect();
        found.sort();
        found
    }

    fn expect(source: &str, expected: &[(u32, u32, &str)]) {
        let found: Vec<_> = found("example.gemspec", source)
            .into_iter()
            .map(|(line, col, _, message)| (line, col, message))
            .collect();
        let expected: Vec<_> = expected
            .iter()
            .map(|&(line, col, attribute)| {
                (line, col, format!("Do not set `{attribute}` in gemspec."))
            })
            .collect();
        assert_eq!(found, expected, "{source}");
    }

    fn fixed(source: &str) -> Option<String> {
        let found = offenses("example.gemspec", source);
        found.iter().any(|o| o.correction.is_some()).then(|| {
            let applied = crate::corrections::apply(source.as_bytes(), &found).unwrap();
            String::from_utf8(applied.bytes).unwrap()
        })
    }

    const SHARED: [(&str, &str); 4] = [
        ("date", "Time.now.strftime('%Y-%m-%d')"),
        ("rubygems_version", "'2.5'"),
        ("specification_version", "'2.5'"),
        ("test_files", "Dir.glob('test/**/*')"),
    ];

    #[test]
    fn deprecated_attributes_shared_examples() {
        for (attribute, value) in SHARED {
            for variable in ["s", "spec"] {
                let source = format!(
                    "Gem::Specification.new do |{variable}|\n  {variable}.name = 'your_cool_gem_name'\n  {variable}.{attribute} = {value}\n  {variable}.bindir = 'exe'\nend\n"
                );
                expect(&source, &[(3, 3, attribute)]);
                // The upstream canonical edit, applied in the inactive envelope.
                let envelope = format!(
                    "if false\n{}end\n",
                    source
                        .lines()
                        .map(|l| format!("  {l}\n"))
                        .collect::<String>()
                );
                let after = format!(
                    "if false\n  Gem::Specification.new do |{variable}|\n    {variable}.name = 'your_cool_gem_name'\n    {variable}.bindir = 'exe'\n  end\nend\n"
                );
                assert_eq!(fixed(&envelope).as_deref(), Some(after.as_str()));
            }
            expect(&format!("s.{attribute} = {value}\n"), &[]);
            expect(
                &format!("Gem::Specification.new do |spec|\n  s.{attribute} = {value}\nend\n"),
                &[],
            );
        }
    }

    #[test]
    fn deprecated_attributes_with_addition_shared_examples() {
        for variable in ["s", "spec"] {
            let source = format!(
                "Gem::Specification.new do |{variable}|\n  {variable}.name = 'your_cool_gem_name'\n  {variable}.test_files += Dir.glob('test/**/*')\n  {variable}.bindir = 'exe'\nend\n"
            );
            expect(&source, &[(3, 3, "test_files")]);
            let envelope = format!("if false\n{source}end\n");
            let after = format!(
                "if false\nGem::Specification.new do |{variable}|\n  {variable}.name = 'your_cool_gem_name'\n  {variable}.bindir = 'exe'\nend\nend\n"
            );
            assert_eq!(fixed(&envelope).as_deref(), Some(after.as_str()));
        }
    }

    #[test]
    fn only_the_first_matching_descendant_of_each_block() {
        expect(
            "Gem::Specification.new do |s|\n  s.date = 1\n  s.test_files = 2\nend\n",
            &[(2, 3, "date")],
        );
        // Pre-order: the outer assignment precedes its nested value.
        expect(
            "Gem::Specification.new do |s|\n  s.date = (s.test_files = 2)\nend\n",
            &[(2, 3, "date")],
        );
        // Non-matching receivers do not stop the search.
        expect(
            "Gem::Specification.new do |s|\n  t.date = 1\n  s.rubygems_version = 2\nend\n",
            &[(3, 3, "rubygems_version")],
        );
        // Parser pre-order visits an `unless` else body first.
        expect(
            "Gem::Specification.new do |s|\n  unless x\n    s.date = 1\n  else\n    s.test_files = 2\n  end\nend\n",
            &[(5, 5, "test_files")],
        );
        // Each block reports its own first match; a shared node once.
        expect(
            "Gem::Specification.new do |s|\n  Gem::Specification.new do |t|\n    t.date = 1\n    s.date = 2\n  end\nend\n",
            &[(3, 5, "date"), (4, 5, "date")],
        );
        expect(
            "Gem::Specification.new do |s|\n  Gem::Specification.new do |s|\n    s.date = 1\n  end\nend\n",
            &[(3, 5, "date")],
        );
    }

    #[test]
    fn compound_assignment_only_for_test_files() {
        expect(
            "Gem::Specification.new do |s|\n  s.date += 1\n  s.rubygems_version -= 1\n  s.test_files ||= 1\n  s.test_files &&= 1\n  s&.test_files += 1\n  s.test_files[0] += 1\nend\n",
            &[],
        );
        expect(
            "Gem::Specification.new do |s|\n  s.test_files <<= 1\nend\n",
            &[(2, 3, "test_files")],
        );
    }

    #[test]
    fn targets_and_receiver_source_matching() {
        // Multiple-assignment, `for` and `rescue` targets are setter sends.
        expect(
            "Gem::Specification.new do |s|\n  a, s.date = 1, 2\nend\n",
            &[(2, 6, "date")],
        );
        expect(
            "Gem::Specification.new do |s|\n  for s.test_files in [] do end\nend\n",
            &[(2, 7, "test_files")],
        );
        expect(
            "Gem::Specification.new do |s|\n  begin\n  rescue => s.date\n  end\nend\n",
            &[(3, 13, "date")],
        );
        // Receiver source, not binding: a method call spelled like it counts.
        expect(
            "Gem::Specification.new do |s|\n  def x\n    s.date = 1\n  end\nend\n",
            &[(3, 5, "date")],
        );
        expect(
            "Gem::Specification.new do |s|\n  s.date=(1)\n  s.date= 1\nend\n",
            &[(2, 3, "date")],
        );
        // First argument source in source order.
        expect(
            "Gem::Specification.new do |s, t|\n  t.date = 1\n  s.date = 2\nend\n",
            &[(3, 3, "date")],
        );
        expect(
            "Gem::Specification.new { |*s| s.date = 1 }\nGem::Specification.new { |s = 1| s.date = 1 }\nGem::Specification.new { |;s| s.date = 1 }\n",
            &[(3, 31, "date")],
        );
        // Lookalikes.
        expect(
            "Gem::Specification.new do |s|\n  s&.date = 1\n  (s).date = 1\n  self.date = 1\n  s.dates = 1\n  s.date\n  s.send(:date=, 1)\n  s[:date] = 1\n  date = 1\nend\n",
            &[],
        );
        for shape in [
            "Gem::Specification.new do\n  s.date = 1\nend\n",
            "Gem::Specification.new { _1.date = 1 }\n",
            "Gem::Specification.new { it.date = 1 }\n",
            "Gem::Specification.new('x') { |s| s.date = 1 }\n",
            "Gem::Specification&.new { |s| s.date = 1 }\n",
            "Specification.new { |s| s.date = 1 }\n",
            "X::Gem::Specification.new { |s| s.date = 1 }\n",
            "Gem::Specification.new(&b)\n",
        ] {
            expect(shape, &[]);
        }
        expect(
            "::Gem::Specification.new { |s| s.date = 1 }\n",
            &[(1, 32, "date")],
        );
    }

    #[test]
    fn whole_node_location_and_supplied_gemspecs_only() {
        let source = "Gem::Specification.new do |s|\n  s.test_files =\n    Dir['x']\nend\n";
        assert_eq!(
            found("a.gemspec", source),
            [(2, 3, 3, "Do not set `test_files` in gemspec.".to_owned())]
        );
        for path in ["Gemfile", "gems.rb", "x.rb", "gemspec"] {
            assert!(found(path, source).is_empty());
        }
    }

    #[test]
    fn live_findings_keep_their_bytes() {
        let spec = |body: &str| format!("Gem::Specification.new do |s|\n{body}end\n");
        let envelope =
            |body: &str| format!("if false\n  Gem::Specification.new do |s|\n{body}  end\nend\n");
        for source in [
            spec("  s.date = 1\n"),
            format!("x\n{}", envelope("    s.date = 1\n")),
            format!("{}__END__\n", envelope("    s.date = 1\n")),
            envelope("    s.date = __FILE__\n"),
            envelope("    s.date = 1 # keep\n"),
            envelope("    s.date = [\n      # keep\n      1\n    ]\n"),
            envelope("    s.date = 1; s.name = 2\n"),
            envelope("    x; s.date = 1\n"),
            envelope("    a, s.date = 1, 2\n"),
            envelope("    foo(s.date = 1)\n"),
            envelope("    s.date = <<~D\n      1\n    D\n"),
            envelope("    s.date = (x = 1)\n    s.name = x\n"),
            envelope("    s.date = /(?<x>a)/ =~ y\n"),
            // Losing or adding a parse warning holds the edit.
            envelope("    s.date = { a: 1, a: 2 }\n"),
            envelope("    x = 1\n    s.date = x\n"),
        ] {
            let found = offenses("example.gemspec", &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn canonical_removal_in_a_closed_inactive_envelope() {
        let before = "if false\r\n  Gem::Specification.new do |s|\r\n    s.name = 'x'\r\n    s.test_files =\r\n      Dir['test/**/*']   \r\n  end\r\nend\r\n";
        let after =
            "if false\r\n  Gem::Specification.new do |s|\r\n    s.name = 'x'\r\n  end\r\nend\r\n";
        assert_eq!(fixed(before).as_deref(), Some(after));
        // The only statement; the next run finds nothing more to remove.
        let before = "if false\n  Gem::Specification.new { |s|\n    s.date = Time.now\n  }\nend\n";
        let after = "if false\n  Gem::Specification.new { |s|\n  }\nend\n";
        assert_eq!(fixed(before).as_deref(), Some(after));
        assert!(offenses("example.gemspec", after).is_empty());
        // Single pass: one finding per block; the next run finds the next one.
        let before = "if false\n  Gem::Specification.new do |s|\n    s.date = 1\n    s.test_files = 2\n  end\nend\n";
        let once = fixed(before).unwrap();
        assert_eq!(
            fixed(&once).as_deref(),
            Some("if false\n  Gem::Specification.new do |s|\n  end\nend\n")
        );
    }

    #[test]
    fn removals_are_checked_together() {
        // Each removal alone keeps `x` read; both together leave it unused
        // and warned about, so only the first is applied.
        let before = "if false\n  x = 1\n  Gem::Specification.new do |s|\n    s.date = x\n  end\n  Gem::Specification.new do |t|\n    t.date = x\n  end\nend\n";
        let after = "if false\n  x = 1\n  Gem::Specification.new do |s|\n  end\n  Gem::Specification.new do |t|\n    t.date = x\n  end\nend\n";
        assert_eq!(fixed(before).as_deref(), Some(after));
        let held = offenses("example.gemspec", after);
        assert_eq!(held.len(), 1);
        assert!(held[0].correction.is_none());
        // Independent removals still apply together.
        let before = "if false\n  Gem::Specification.new do |s|\n    s.date = 1\n  end\n  Gem::Specification.new do |t|\n    t.date = 2\n  end\nend\n";
        let after = "if false\n  Gem::Specification.new do |s|\n  end\n  Gem::Specification.new do |t|\n  end\nend\n";
        assert_eq!(fixed(before).as_deref(), Some(after));
    }
}
