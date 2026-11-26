//! `Gemspec/RequireMFA`: `on_block` for every parser `block` node (a call,
//! lambda or `super` with a literal block, not a numbered or `it` block)
//! whose subtree contains a `Gem::Specification.new do |x| ... end` block.
//! The first `metadata = $_` or `metadata['rubygems_mfa_required'] = $_` send
//! below it (parser pre-order, any receiver) gives the metadata value; a
//! `str` value, or else the first `rubygems_mfa_required` pair inside it,
//! gives the MFA value. A value other than `'true'` is reported at the
//! value; no value at all is reported at the block. Offenses at a range
//! already reported are not added again.
use std::collections::HashSet;

use ruby_prism::{CallNode, Node, ParseResult};

use super::help::specification;
use crate::{
    cops::{
        Cop,
        bundler::{
            declarations::{parser_order_nodes, str_value},
            ordered_gem_node::pinned,
        },
        lint::reference_basic_literal::is_basic_literal,
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct RequireMFA;

const MSG: &str = "`metadata['rubygems_mfa_required']` must be set to `'true'`.";
const KEY: &[u8] = b"rubygems_mfa_required";

type Span = (usize, usize);

/// Where a block's closing `end`/`}` starts, and whether its body is empty.
#[derive(Clone, Copy)]
struct Closing {
    start: usize,
    empty: bool,
}

fn span(node: &Node<'_>) -> Span {
    let location = node.location();
    (location.start_offset(), location.end_offset())
}

/// One node of interest, in parser pre-order.
struct Item<'pr> {
    span: Span,
    /// Enclosing parser blocks (their spans), outermost first.
    blocks: Vec<Span>,
    /// The parser block whose body has this node as a direct statement.
    statement_of: Option<Span>,
    /// A parser `block` node: whether it is a specification block (its
    /// variable), and its closing.
    block: Option<(Option<Vec<u8>>, Option<Closing>)>,
    /// `metadata(node)`'s send: its captured value.
    metadata: Option<Node<'pr>>,
    /// `metadata_assignment(node)`.
    assignment: bool,
}

impl Cop for RequireMFA {
    fn name(&self) -> &'static str {
        "Gemspec/RequireMFA"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn check_parse(&self, result: &ParseResult<'_>, file: &SourceFile, out: &mut Vec<Offense>) {
        let root = result.node();
        let items = parser_order_nodes(&root, |node, ancestors| item(node, ancestors, file));
        let mut reported = HashSet::new();
        let first = out.len();
        for (index, block) in items.iter().enumerate() {
            let Some((own, closing)) = &block.block else {
                continue;
            };
            let inside = |item: &Item<'_>| item.blocks.contains(&block.span);
            // `gem_specification(node)` includes the block itself; only its
            // first yield can add an offense (the rest repeat its range).
            let Some(variable) = items[index..]
                .iter()
                .filter(|item| item.span == block.span || inside(item))
                .find_map(|item| item.block.as_ref()?.0.clone())
            else {
                continue;
            };
            let metadata = items[index + 1..]
                .iter()
                .filter(|item| inside(item))
                .find_map(|item| item.metadata.as_ref());
            let value = metadata.and_then(|metadata| mfa_value(metadata, file));
            let (range, correction) = if let Some(value) = &value {
                if str_value(value, file).as_deref() == Some(b"true") {
                    continue;
                }
                (span(value), Edit::Replace(value))
            } else {
                let edit = match metadata {
                    Some(metadata) => match metadata.as_hash_node() {
                        Some(hash) => Edit::Hash(hash),
                        None => Edit::None,
                    },
                    None => {
                        let last = items[index + 1..]
                            .iter()
                            .rev()
                            .find(|item| inside(item) && item.assignment);
                        // The directive is written against the first
                        // yielded variable; only the specification block's
                        // own variable is in scope at the insertion point.
                        if own.as_ref() != Some(&variable) {
                            Edit::None
                        } else if let Some(last) = last {
                            if last.statement_of == Some(block.span) {
                                Edit::After(last.span.1, variable)
                            } else {
                                Edit::None
                            }
                        } else if let Some(closing) = closing {
                            Edit::Before(*closing, variable)
                        } else {
                            Edit::None
                        }
                    }
                };
                (block.span, edit)
            };
            if !reported.insert(range) {
                continue;
            }
            let (line, col) = file.reference_line_col(range.0);
            let correction = (!matches!(correction, Edit::None))
                .then(|| correction.apply(result, file))
                .flatten();
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                col,
                end_line: file.line_of(range.1.saturating_sub(1).max(range.0)),
                message: MSG.into(),
                correction,
            });
        }
        compose(result, file, &mut out[first..]);
    }
}

fn item<'pr>(node: &Node<'pr>, ancestors: &[Node<'pr>], file: &SourceFile) -> Option<Item<'pr>> {
    let block = parser_block(node);
    let (metadata, assignment) = node
        .as_call_node()
        .map_or((None, false), |call| metadata(&call, file));
    if block.is_none() && metadata.is_none() && !assignment {
        return None;
    }
    let blocks = ancestors
        .iter()
        .filter(|ancestor| parser_block(ancestor).is_some())
        .map(span)
        .collect();
    let statement_of = match ancestors {
        [.., call, body, statements]
            if statements.as_statements_node().is_some() && body.as_block_node().is_some() =>
        {
            call.as_call_node().map(|call| span(&call.as_node()))
        }
        _ => None,
    };
    Some(Item {
        span: span(node),
        blocks,
        statement_of,
        block,
        metadata,
        assignment,
    })
}

/// A parser `block` node (not `numblock`/`itblock`): whether it is a
/// specification block, with its variable, and where its closing starts.
fn parser_block(node: &Node<'_>) -> Option<(Option<Vec<u8>>, Option<Closing>)> {
    let ordinary = |parameters: Option<Node<'_>>| {
        parameters.is_none_or(|p| {
            p.as_numbered_parameters_node().is_none() && p.as_it_parameters_node().is_none()
        })
    };
    let block = match node {
        Node::CallNode { .. } => {
            let call = node.as_call_node().unwrap();
            let block = call.block()?.as_block_node()?;
            if !ordinary(block.parameters()) {
                return None;
            }
            return Some((
                specification(&call),
                Some(Closing {
                    start: block.closing_loc().start_offset(),
                    empty: block.body().is_none(),
                }),
            ));
        }
        Node::LambdaNode { .. } => {
            return ordinary(node.as_lambda_node().unwrap().parameters()).then_some((None, None));
        }
        Node::SuperNode { .. } => node.as_super_node().unwrap().block(),
        Node::ForwardingSuperNode { .. } => node
            .as_forwarding_super_node()
            .unwrap()
            .block()
            .map(|b| b.as_node()),
        _ => return None,
    };
    let block = block?.as_block_node()?;
    ordinary(block.parameters()).then_some((None, None))
}

/// The pinned send's children after the method name: its arguments and a
/// block pass (a literal block is a separate `block` node).
fn send_arguments<'pr>(call: &CallNode<'pr>) -> Option<Vec<Node<'pr>>> {
    if call.block().is_some_and(|b| b.as_block_node().is_some()) {
        return None;
    }
    let mut arguments: Vec<_> = call
        .arguments()
        .iter()
        .flat_map(|a| a.arguments().iter())
        .collect();
    arguments.extend(call.block());
    Some(arguments)
}

/// `(str "rubygems_mfa_required")` or `(sym :rubygems_mfa_required)`.
fn mfa_key(node: &Node<'_>, file: &SourceFile) -> bool {
    str_value(node, file).as_deref() == Some(KEY)
        || node
            .as_symbol_node()
            .is_some_and(|s| is_basic_literal(node, file) && s.unescaped() == KEY)
}

/// `{str sym}`.
fn literal_key(node: &Node<'_>, file: &SourceFile) -> bool {
    str_value(node, file).is_some()
        || (node.as_symbol_node().is_some() && is_basic_literal(node, file))
}

/// The `metadata` capture and whether `metadata_assignment` matches:
/// `(send _ :metadata= $_)`, or
/// `(send (send _ :metadata) :[]= {(str KEY) (sym KEY)} $_)` (and, for the
/// assignment search, any `{str sym}` key).
fn metadata<'pr>(call: &CallNode<'pr>, file: &SourceFile) -> (Option<Node<'pr>>, bool) {
    if call.is_safe_navigation() {
        return (None, false);
    }
    let Some(mut arguments) = send_arguments(call) else {
        return (None, false);
    };
    match call.name().as_slice() {
        b"metadata=" if arguments.len() == 1 => (arguments.pop(), true),
        b"[]=" if arguments.len() == 2 => {
            let receiver = call.receiver().and_then(|r| r.as_call_node());
            if !receiver.is_some_and(|r| {
                !r.is_safe_navigation()
                    && r.name().as_slice() == b"metadata"
                    && send_arguments(&r).is_some_and(|a| a.is_empty())
            }) {
                return (None, false);
            }
            let assignment = literal_key(&arguments[0], file);
            let value = arguments.pop().unwrap();
            (mfa_key(&arguments[0], file).then_some(value), assignment)
        }
        _ => (None, false),
    }
}

/// `mfa_value`: a `str` metadata value itself, or else the value of the
/// first `(pair {(str KEY) (sym KEY)} $_)` inside it.
fn mfa_value<'pr>(metadata: &Node<'pr>, file: &SourceFile) -> Option<Node<'pr>> {
    if str_value(metadata, file).is_some() {
        return Some(match metadata {
            Node::StringNode { .. } => metadata.as_string_node().unwrap().as_node(),
            _ => metadata.as_interpolated_string_node()?.as_node(),
        });
    }
    parser_order_nodes(metadata, |node, _| {
        let pair = node.as_assoc_node()?;
        mfa_key(&pair.key(), file).then(|| pair.value())
    })
    .into_iter()
    .next()
}

/// The upstream canonical edits.
enum Edit<'a, 'pr> {
    None,
    /// `corrector.replace(value, "'true'")`.
    Replace(&'a Node<'pr>),
    /// `correct_metadata`: after the last pair, or before the closing brace.
    Hash(ruby_prism::HashNode<'pr>),
    /// `insert_after(last_assignment, "\n#{directive}")`.
    After(usize, Vec<u8>),
    /// `insert_before(node.loc.end, "#{directive}\n")`.
    Before(Closing, Vec<u8>),
}

// Setting the metadata changes what a live gemspec publishes (RubyGems then
// demands MFA for every privileged operation on the gem), so a live file
// keeps its finding and bytes. The canonical edit is applied only where
// nothing runs: a closed top-level `if false ... end` program without a data
// section or a listed source observer. It must also lose nothing: a replaced
// value has no heredoc, local binding or comment; an insertion is not inside
// a heredoc (opener to terminator) and moves no trailing comment onto new
// code; a directive goes into the specification block's own
// body, after one of its statements, or before a closing that starts its
// line or closes an empty body. The whole set is then checked together by `compose`.
impl Edit<'_, '_> {
    fn apply(&self, result: &ParseResult<'_>, file: &SourceFile) -> Option<Correction> {
        // Text inserted with a line break in its middle moves the rest of
        // the line, a trailing comment (a directive) included, onto the new
        // code's line: such an insertion needs no comment after it.
        let insertion = |offset: usize, replacement: String| {
            let line_end = file.line_start(file.line_of(offset) + 1).max(offset);
            let moves = replacement.trim_end_matches('\n').contains('\n');
            let comment = result.comments().any(|c| {
                let start = c.location().start_offset();
                start >= offset && start < line_end
            });
            (!(moves && comment) && !in_heredoc(&result.node(), offset)).then_some(Correction {
                range: offset..offset,
                replacement,
            })
        };
        let directive = |variable: &[u8]| {
            std::str::from_utf8(variable)
                .ok()
                .map(|v| format!("{v}.metadata['rubygems_mfa_required'] = 'true'"))
        };
        match self {
            Edit::None => None,
            Edit::Replace(value) => {
                let (start, end) = span(value);
                let commented = result.comments().any(|c| {
                    let offset = c.location().start_offset();
                    offset >= start && offset < end
                });
                (!pinned(value) && !commented).then(|| Correction {
                    range: start..end,
                    replacement: "'true'".into(),
                })
            }
            Edit::Hash(hash) => {
                let pairs: Vec<_> = hash
                    .elements()
                    .iter()
                    .filter_map(|e| e.as_assoc_node())
                    .collect();
                // Ruby's compiler (not the parser) warns about a duplicated
                // static key, such as adjacent literals the pinned `str`
                // pattern does not match.
                if pairs
                    .iter()
                    .any(|pair| static_string(&pair.key()).as_deref() == Some(KEY))
                {
                    return None;
                }
                if let Some(last) = pairs.last() {
                    insertion(
                        last.location().end_offset(),
                        ",\n'rubygems_mfa_required' => 'true'".into(),
                    )
                } else {
                    insertion(
                        hash.closing_loc().start_offset(),
                        "'rubygems_mfa_required' => 'true'".into(),
                    )
                }
            }
            Edit::After(offset, variable) => {
                insertion(*offset, format!("\n{}", directive(variable)?))
            }
            Edit::Before(closing, variable) => {
                // After code on the closing's line the directive would
                // become that code's argument or operand (`foo end`).
                let start = file.line_start(file.line_of(closing.start));
                if !closing.empty
                    && !file.bytes[start..closing.start]
                        .iter()
                        .all(|b| matches!(b, b' ' | b'\t'))
                {
                    return None;
                }
                insertion(closing.start, format!("{}\n", directive(variable)?))
            }
        }
    }
}

/// The value of a string literal without interpolation, adjacent literals
/// included.
fn static_string(node: &Node<'_>) -> Option<Vec<u8>> {
    if let Some(string) = node.as_string_node() {
        return Some(string.unescaped().to_vec());
    }
    let mut value = Vec::new();
    for part in node.as_interpolated_string_node()?.parts().iter() {
        value.extend(static_string(&part)?);
    }
    Some(value)
}

/// Whether `offset` lies after a heredoc opener and before the end of its
/// terminator line: text there lands on the opener's line or in the body.
fn in_heredoc(root: &Node<'_>, offset: usize) -> bool {
    parser_order_nodes(root, |node, _| {
        let (opening, closing) = match node {
            Node::StringNode { .. } => {
                let s = node.as_string_node().unwrap();
                (s.opening_loc()?, s.closing_loc()?)
            }
            Node::InterpolatedStringNode { .. } => {
                let s = node.as_interpolated_string_node().unwrap();
                (s.opening_loc()?, s.closing_loc()?)
            }
            Node::XStringNode { .. } => {
                let s = node.as_x_string_node().unwrap();
                (s.opening_loc(), s.closing_loc())
            }
            Node::InterpolatedXStringNode { .. } => {
                let s = node.as_interpolated_x_string_node().unwrap();
                (s.opening_loc(), s.closing_loc())
            }
            _ => return None,
        };
        (opening.as_slice().starts_with(b"<<")
            && opening.start_offset() < offset
            && offset < closing.end_offset())
        .then_some(())
    })
    .into_iter()
    .next()
    .is_some()
}

/// Each edit was checked alone; together they can still collide (two blocks
/// inserting the same pair) or interact. Keep, in the order the corrector
/// selects them, only edits it would apply (no overlap with, and no shared
/// boundary of an insertion with, an earlier kept edit) that keep the
/// program free of errors and its warnings unchanged together with every
/// edit kept before them.
fn compose(result: &ParseResult<'_>, file: &SourceFile, offenses: &mut [Offense]) {
    let mut order: Vec<usize> = (0..offenses.len())
        .filter(|&i| offenses[i].correction.is_some())
        .collect();
    order.sort_by(|&a, &b| {
        let a = offenses[a].correction.as_ref().unwrap();
        let b = offenses[b].correction.as_ref().unwrap();
        (a.range.start, a.range.end, &a.replacement).cmp(&(
            b.range.start,
            b.range.end,
            &b.replacement,
        ))
    });
    let before: Vec<_> = result
        .warnings()
        .map(|w| w.message().into_owned())
        .collect();
    let mut kept: Vec<Correction> = Vec::new();
    for i in order {
        let correction = offenses[i].correction.clone().unwrap();
        let fits = kept.last().is_none_or(|last| {
            let (a, b) = (&last.range, &correction.range);
            !(b.start < a.end || (b.start == a.end && (a.is_empty() || b.is_empty())))
        }) && {
            let mut edited = Vec::with_capacity(file.bytes.len() + 64);
            let mut offset = 0;
            for edit in kept.iter().chain(std::iter::once(&correction)) {
                edited.extend_from_slice(&file.bytes[offset..edit.range.start]);
                edited.extend_from_slice(edit.replacement.as_bytes());
                offset = edit.range.end;
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
            kept.push(correction);
        } else {
            offenses[i].correction = None;
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's gemspec/require_mfa_spec.rb (MIT, Copyright (c)
    //! 2012-26 Bozhidar Batsov), plus lookalikes and the guarded corrections.
    //! Spec corrections are replayed inside the closed inactive envelope.
    use super::*;
    use crate::pipeline::lint_path_with_corrections;

    fn offenses(path: &str, source: &str) -> Vec<Offense> {
        lint_path_with_corrections(&[&RequireMFA], path, source)
    }

    /// Sorted (line, col) findings; every message is the pinned one.
    fn found(source: &str) -> Vec<(u32, u32)> {
        let mut found: Vec<_> = offenses("my.gemspec", source)
            .into_iter()
            .map(|o| {
                assert_eq!(o.message, MSG);
                (o.line, o.col)
            })
            .collect();
        found.sort();
        found
    }

    fn fixed(source: &str) -> Option<String> {
        let found = offenses("my.gemspec", source);
        found.iter().any(|o| o.correction.is_some()).then(|| {
            let applied = crate::corrections::apply(source.as_bytes(), &found).unwrap();
            String::from_utf8(applied.bytes).unwrap()
        })
    }

    /// `if false` around every line, two spaces deeper.
    fn envelope(source: &str) -> String {
        let body: String = source
            .lines()
            .map(|l| {
                if l.is_empty() {
                    "\n".to_owned()
                } else {
                    format!("  {l}\n")
                }
            })
            .collect();
        format!("if false\n{body}end\n")
    }

    #[test]
    fn when_the_gemspec_is_blank() {
        assert!(found("").is_empty());
    }

    #[test]
    fn non_hash_metadata_is_reported_but_not_corrected() {
        let source = "Gem::Specification.new do |spec|\n  spec.metadata = Metadata.new\nend\n";
        assert_eq!(found(source), [(1, 1)]);
        assert_eq!(fixed(&envelope(source)), None);
    }

    #[test]
    fn other_metadata_keys_with_the_key_included() {
        for source in [
            "Gem::Specification.new do |spec|\n  spec.metadata = {\n    'foo' => 'bar',\n    'rubygems_mfa_required' => 'true',\n    'baz' => 'quux'\n  }\nend\n",
            "Gem::Specification.new do |spec|\n  spec.metadata = {\n    foo: 'bar',\n    rubygems_mfa_required: 'true',\n    baz: 'quux'\n  }\nend\n",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn key_assignment_with_the_key_included() {
        for source in [
            "Gem::Specification.new do |spec|\n  spec.metadata['foo'] = 'bar'\n  spec.metadata['rubygems_mfa_required'] = 'true'\nend\n",
            "Gem::Specification.new do |spec|\n  spec.metadata[:foo] = 'bar'\n  spec.metadata[:rubygems_mfa_required] = 'true'\nend\n",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn with_rubygems_mfa_required_true() {
        for source in [
            "Gem::Specification.new do |spec|\n  spec.metadata = {\n    'rubygems_mfa_required' => 'true'\n  }\nend\n",
            "Gem::Specification.new do |spec|\n  spec.metadata = {\n    rubygems_mfa_required: 'true'\n  }\nend\n",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
    }

    const SPEC: &str = "Gem::Specification.new do |spec|\n";

    #[test]
    fn metadata_shapes_follow_the_pinned_patterns() {
        // Any receiver, receiverless `metadata`, `:sym`, `"str"`, quoted
        // pair keys and nested pairs all count.
        for body in [
            "  foo.metadata['rubygems_mfa_required'] = 'true'\n",
            "  metadata['rubygems_mfa_required'] = 'true'\n",
            "  self.metadata = { 'rubygems_mfa_required' => 'true' }\n",
            "  spec.metadata[:\"rubygems_mfa_required\"] = 'true'\n",
            "  spec.metadata[%s(rubygems_mfa_required)] = 'true'\n",
            "  spec.metadata[\"rubygems_mfa_required\"] = \"true\"\n",
            "  spec.metadata = { 'rubygems_mfa_required': 'true' }\n",
            "  spec.metadata = { 'x' => { rubygems_mfa_required: 'true' } }\n",
            "  spec.metadata = 'true'\n",
            "  spec.metadata = Foo.new(rubygems_mfa_required: 'true')\n",
            "  spec.metadata.[]=('rubygems_mfa_required', 'true')\n",
            "  if x\n    spec.metadata['rubygems_mfa_required'] = 'true'\n  end\n",
        ] {
            assert!(found(&format!("{SPEC}{body}end\n")).is_empty(), "{body}");
        }
        // A non-'true' str value is reported at the value.
        for (body, at) in [
            ("  spec.metadata = 'false'\n", (2, 19)),
            (
                "  spec.metadata['rubygems_mfa_required'] = \"yes\"\n",
                (2, 44),
            ),
            ("  spec.metadata[:rubygems_mfa_required] = ''\n", (2, 43)),
            (
                "  spec.metadata = { 'x' => { rubygems_mfa_required: 'false' } }\n",
                (2, 53),
            ),
            (
                "  spec.metadata = { rubygems_mfa_required: true }\n",
                (2, 44),
            ),
            ("  spec.metadata = { rubygems_mfa_required: }\n", (2, 21)),
            (
                "  spec.metadata = { 'rubygems_mfa_required' => <<~E }\n    true\n  E\n",
                (2, 48),
            ),
        ] {
            assert_eq!(found(&format!("{SPEC}{body}end\n")), [at], "{body}");
        }
        // Not the pinned shapes: reported at the block.
        for body in [
            "",
            "  spec&.metadata = { 'rubygems_mfa_required' => 'true' }\n",
            "  spec&.metadata['rubygems_mfa_required'] = 'true'\n",
            "  spec.metadata(1)['rubygems_mfa_required'] = 'true'\n",
            "  spec.metadata['rubygems_mfa_required', 1] = 'true'\n",
            "  spec.metadata[\"rubygems_#{x}\"] = 'true'\n",
            "  spec.metadata['rubygems_mfa_required'] ||= 'true'\n",
            "  spec.metadata['rubygems_mfa_required'], x = 'true', 1\n",
            "  spec.metadata.store('rubygems_mfa_required', 'true')\n",
            "  spec.meta_data['rubygems_mfa_required'] = 'true'\n",
            "  spec.metadata['rubygems_mfa_required'] = true\n",
            "  spec.metadata = { 'x' => 'y' }\n  spec.metadata['rubygems_mfa_required'] = 'true'\n",
            "  spec.metadata = { \"rubygems_mfa_required\" \"\" => 'true' }\n",
        ] {
            assert_eq!(found(&format!("{SPEC}{body}end\n")), [(1, 1)], "{body}");
        }
    }

    #[test]
    fn every_parser_block_containing_a_specification_is_checked() {
        // Not `(args (arg $_))`, or not a `block` node: nothing.
        for source in [
            "Gem::Specification.new do\nend\n",
            "Gem::Specification.new do |a, b|\nend\n",
            "Gem::Specification.new { _1 }\n",
            "Gem::Specification.new { it }\n",
            "Gem::Specification.new(1) do |s|\nend\n",
            "Gem::Specification&.new do |s|\nend\n",
            "Foo::Gem::Specification.new do |s|\nend\n",
            "Gem::Specification.new(&b)\n",
            "foo do\nend\n",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
        assert_eq!(found("::Gem::Specification.new do |s|\nend\n"), [(1, 1)]);
        // An enclosing call, lambda or super block is reported too.
        for (source, at) in [
            (
                "foo do\n  Gem::Specification.new do |spec|\n  end\nend\n",
                [(1, 1), (2, 3)],
            ),
            (
                "-> do\n  Gem::Specification.new do |spec|\n  end\nend\n",
                [(1, 1), (2, 3)],
            ),
            (
                "def f\n  super do\n    Gem::Specification.new { |s| }\n  end\nend\n",
                [(2, 3), (3, 5)],
            ),
            (
                "x = Gem::Specification.new do |s|\nend.tap { |y| }\n",
                [(1, 5), (1, 5)],
            ),
        ] {
            assert_eq!(found(source), at, "{source}");
        }
        // Numbered and `it` blocks are not `block` nodes.
        assert_eq!(
            found(
                "foo { _1\n  Gem::Specification.new { |spec| }\n}\nbar { it; Gem::Specification.new { |s| } }\n"
            ),
            [(2, 3), (4, 11)]
        );
        // Each block searches its whole subtree, first match in pre-order.
        let source = "Gem::Specification.new do |a|\n  Gem::Specification.new do |b|\n    b.metadata['rubygems_mfa_required'] = 'true'\n  end\nend\n";
        assert!(found(source).is_empty());
        let source = "foo do\n  Gem::Specification.new do |b|\n    b.metadata['rubygems_mfa_required'] = 'false'\n  end\nend\n";
        assert_eq!(found(source), [(3, 43)]);
        let source = "unless x\n  y.metadata['rubygems_mfa_required'] = 'no'\nelse\n  y.metadata['rubygems_mfa_required'] = 'true'\nend\n";
        let source = format!("foo do\n{source}  Gem::Specification.new {{ |s| }}\nend\n");
        // parser's `unless` keeps its else branch first: the enclosing block
        // finds 'true'; the specification block has no metadata of its own.
        assert_eq!(found(&source), [(7, 3)], "{source}");
    }

    #[test]
    fn multi_line_block_findings_span_the_block() {
        let found = offenses(
            "my.gemspec",
            "Gem::Specification.new do |spec|\n  spec.name = 'x'\nend\n",
        );
        assert_eq!((found[0].line, found[0].end_line), (1, 3));
    }

    #[test]
    fn only_supplied_gemspecs() {
        let source = "Gem::Specification.new do |spec|\nend\n";
        for path in ["Gemfile", "x.rb", "gemspec", "gems.rb"] {
            assert!(offenses(path, source).is_empty(), "{path}");
        }
        assert_eq!(offenses("nested/.x.gemspec", source).len(), 1);
        let bom = format!("\u{feff}{}", source.replace('\n', "\r\n"));
        let found = offenses("x.gemspec", &bom);
        assert_eq!((found[0].line, found[0].col), (1, 2));
    }

    /// Wrapped in the envelope, the single finding keeps its bytes.
    fn held(body: &str) {
        let source = envelope(&format!("{SPEC}{body}end\n"));
        let found = offenses("my.gemspec", &source);
        assert_eq!(found.len(), 1, "{source}");
        assert!(found[0].correction.is_none(), "{source}");
    }

    #[test]
    fn envelope_edits_that_could_lose_something_are_held() {
        // A replaced value with a comment, a binding or a heredoc.
        held("  spec.metadata = { 'rubygems_mfa_required' => foo(1, # c\n    2) }\n");
        held("  spec.metadata = { 'rubygems_mfa_required' => (x = 'no') }\n");
        held("  spec.metadata = { 'rubygems_mfa_required' => <<~E }\n    no\n  E\n");
        // An insertion that would move a trailing comment or land in a heredoc.
        held("  spec.metadata['foo'] = 'bar' # rubocop:disable Foo\n");
        held("  spec.metadata = {\n    'foo' => 'bar' # c\n  }\n");
        held("  spec.metadata['foo'] = <<~E\n    x\n  E\n");
        held("  spec.metadata = {\n    'foo' => <<~E\n      x\n    E\n  }\n");
        // A directive outside the block's own statements.
        held("  spec.metadata['foo'] = 'bar' if x\n");
        held("  [1].each { |spec| spec.metadata['foo'] = 'bar' }\n");
        held("  def f\n    spec.metadata['foo'] = 'bar'\n  end\n");
        // Before a closing that follows code on its line.
        let source = envelope("Gem::Specification.new do |spec| foo end\n");
        assert!(offenses("my.gemspec", &source)[0].correction.is_none());
        // A kwsplat-only hash: the insertion does not parse.
        held("  spec.metadata = { **x }\n");
        // A static adjacent-literal key the pinned str pattern misses: Ruby
        // would warn that the inserted key duplicates it.
        held("  spec.metadata = { \"rubygems_mfa_required\" \"\" => 'false' }\n");
        held("  spec.metadata = { 'a' => 1, \"rubygems_\" 'mfa_' \"required\" => 2 }\n");
        // An enclosing block's directive would name an out-of-scope variable.
        let source = envelope("foo do\n  Gem::Specification.new do |spec|\n  end\nend\n");
        let found = offenses("my.gemspec", &source);
        assert_eq!(found.len(), 2);
        assert_eq!(found.iter().filter(|o| o.correction.is_some()).count(), 1);
    }

    #[test]
    fn envelope_edits_in_other_safe_shapes() {
        // One-line blocks, an empty body, trailing comma, `;` statements.
        for (source, want) in [
            (
                "Gem::Specification.new { |spec| }\n",
                "Gem::Specification.new { |spec| spec.metadata['rubygems_mfa_required'] = 'true'\n}\n",
            ),
            (
                "Gem::Specification.new do |s|\n  s.metadata[:a] = 1; s.name = 'x'\nend\n",
                "Gem::Specification.new do |s|\n  s.metadata[:a] = 1\ns.metadata['rubygems_mfa_required'] = 'true'; s.name = 'x'\nend\n",
            ),
            (
                "Gem::Specification.new do |s|\n  s.metadata = { 'a' => 'b', }\nend\n",
                "Gem::Specification.new do |s|\n  s.metadata = { 'a' => 'b',\n'rubygems_mfa_required' => 'true', }\nend\n",
            ),
            (
                "Gem::Specification.new do |s|\n  s.metadata = { 'rubygems_mfa_required' => :no }\nend\n",
                "Gem::Specification.new do |s|\n  s.metadata = { 'rubygems_mfa_required' => 'true' }\nend\n",
            ),
            (
                "Gem::Specification.new do |s|\n  s.metadata = 'false'\nend\n",
                "Gem::Specification.new do |s|\n  s.metadata = 'true'\nend\n",
            ),
        ] {
            let source = format!("if false\n{source}end\n");
            let want = format!("if false\n{want}end\n");
            assert_eq!(fixed(&source).as_deref(), Some(want.as_str()), "{source}");
            assert_eq!(crate::parser::parse(want.as_bytes()).errors().count(), 0);
        }
    }

    #[test]
    fn edits_are_composed_in_the_correctors_order() {
        // An enclosing block and the specification block insert the same
        // pair at the same point: only one is applied.
        let source = "if false\nfoo do\n  Gem::Specification.new do |spec|\n    spec.metadata = {\n      'a' => 'b'\n    }\n  end\nend\nend\n";
        let found = offenses("my.gemspec", source);
        assert_eq!(found.len(), 2);
        assert_eq!(found.iter().filter(|o| o.correction.is_some()).count(), 1);
        assert_eq!(
            fixed(source).unwrap(),
            source.replace(
                "'a' => 'b'\n",
                "'a' => 'b',\n'rubygems_mfa_required' => 'true'\n"
            )
        );
        // Two specification blocks each get their own directive.
        let source = "if false\nGem::Specification.new do |a|\nend\nGem::Specification.new do |b|\n  b.metadata['x'] = 'y'\nend\nend\n";
        assert_eq!(
            fixed(source).unwrap(),
            "if false\nGem::Specification.new do |a|\na.metadata['rubygems_mfa_required'] = 'true'\nend\nGem::Specification.new do |b|\n  b.metadata['x'] = 'y'\nb.metadata['rubygems_mfa_required'] = 'true'\nend\nend\n"
        );
    }
}
