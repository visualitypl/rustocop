//! `Style/RedundantDoubleSplatHashBraces` — use keyword pairs directly.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{HashNode, Node};
pub struct RedundantDoubleSplatHashBraces;
const MESSAGE: &str =
    "Remove the redundant double splat and braces, use keyword arguments directly.";
impl Cop for RedundantDoubleSplatHashBraces {
    fn name(&self) -> &'static str {
        "Style/RedundantDoubleSplatHashBraces"
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
        let Some(splat) = node.as_assoc_splat_node() else {
            return;
        };
        let Some(value) = splat.value() else {
            return;
        };
        // The index coalesces all Hash callbacks sharing one upstream kwsplat
        // range. One lazy walk of the main parse avoids per-candidate sibling
        // or subtree scans, including hashes buried in merge arguments.
        let Some(root) = cx.ancestors().next_back() else {
            return;
        };
        if !file
            .redundant_double_splats(root)
            .contains(&splat.location().start_offset())
        {
            return;
        }
        let hash = value.as_hash_node();
        let loc = splat.location();
        let (line, col) = file.line_col(loc.start_offset());
        let correction = if cx.corrections_requested()
            && hash.as_ref().is_some_and(label_hash)
            && !cx
                .ancestors()
                .any(|n| matches!(n, Node::ShareableConstantNode { .. }))
        {
            let mut edits = Vec::new();
            if file.contains_heredoc(root, node)
                || file.contains_keyword_block(root, node)
                || !collect_removals(
                    &hash.unwrap(),
                    splat.operator_loc().start_offset(),
                    file,
                    enclosing_continuation(cx),
                    &mut edits,
                )
            {
                None
            } else {
                super::correction::combine(file, edits)
                    .and_then(|edit| super::literal_spelling::safe_edit(file, cx, edit))
            }
        } else {
            None
        };
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: MESSAGE.into(),
            line,
            col,
            end_line: file.last_line_of(loc),
            correction,
        });
    }
}

pub(crate) fn index(root: &Node<'_>) -> std::collections::HashSet<usize> {
    use crate::cops::walk::{NodeVisitor, walk};
    struct Builder<'pr> {
        ancestors: Vec<Node<'pr>>,
        found: std::collections::HashSet<usize>,
        eligible: std::collections::HashMap<usize, bool>,
    }
    impl<'pr> NodeVisitor<'pr> for Builder<'pr> {
        fn enter(&mut self, node: Node<'pr>) {
            if node.as_hash_node().is_some_and(|h| label_hash(&h)) {
                let mut parents = self.ancestors.iter().rev().filter(|n| {
                    n.as_arguments_node().is_none() && n.as_statements_node().is_none()
                });
                if let Some(parent) = parents.next()
                    && (parent.as_call_node().is_some() || parent.as_assoc_splat_node().is_some())
                    && std::iter::once(parent)
                        .chain(parents)
                        .map_while(|n| n.as_call_node())
                        .all(|c| matches!(c.name().as_slice(), b"merge" | b"merge!"))
                    && let Some(splat) = self
                        .ancestors
                        .iter()
                        .rev()
                        .find_map(|n| n.as_assoc_splat_node())
                    && *self
                        .eligible
                        .entry(splat.location().start_offset())
                        .or_insert_with(|| splat.value().is_some_and(|v| !allowed_receiver(&v)))
                {
                    self.found.insert(splat.location().start_offset());
                }
            }
            self.ancestors.push(node);
        }
        fn leave(&mut self) {
            self.ancestors.pop();
        }
    }
    let mut builder = Builder {
        ancestors: Vec::new(),
        found: Default::default(),
        eligible: Default::default(),
    };
    walk(&mut builder, root);
    builder.found
}

fn allowed_receiver(value: &Node<'_>) -> bool {
    let Some(call) = value.as_call_node() else {
        return false;
    };
    if call.block().is_some_and(|n| n.as_block_node().is_some()) {
        return true;
    }
    let mut recv = call.receiver();
    while let Some(next) = recv
        .as_ref()
        .and_then(|n| n.as_call_node())
        .and_then(|n| n.receiver())
    {
        recv = Some(next);
    }
    recv.is_none_or(|n| n.as_hash_node().is_none())
}

fn label_hash(hash: &HashNode<'_>) -> bool {
    let mut has_pair = false;
    for n in hash.elements().iter() {
        if let Some(pair) = n.as_assoc_node() {
            if pair.operator_loc().is_some() {
                return false;
            }
            has_pair = true;
        }
    }
    has_pair
}

// A retained hash brace or argument parenthesis continues an opening newline.
// Do not borrow continuation from a more distant call across a bare command.
fn enclosing_continuation(cx: &NodeContext<'_, '_>) -> bool {
    cx.ancestors()
        .find(|n| n.as_keyword_hash_node().is_none() && n.as_arguments_node().is_none())
        .is_some_and(|n| {
            n.as_hash_node().is_some()
                || n.as_call_node().is_some_and(|c| c.opening_loc().is_some())
        })
}

fn collect_removals(
    hash: &HashNode<'_>,
    start: usize,
    file: &SourceFile,
    opening_continues: bool,
    edits: &mut Vec<Correction>,
) -> bool {
    let elements = hash.elements();
    let Some(first) = elements.iter().next() else {
        return false;
    };
    let Some(last) = elements.last() else {
        return false;
    };
    // Removing ** and { must not expose a statement-ending newline (including
    // one after an opener comment). Carry the retained enclosing continuation
    // through nested removals: their own braces may disappear in this group.
    if !opening_continues && file.bytes[start + 2..first.location().start_offset()].contains(&b'\n')
    {
        return false;
    }
    // A following enclosing comma would move to a new logical line when the
    // brace disappears. Don't silently change binding or relocate comments;
    // retain this individual offense until its layout is unambiguous.
    if file.bytes[last.location().end_offset()..hash.closing_loc().start_offset()].contains(&b'\n')
    {
        return false;
    }
    let mut open_end = hash.opening_loc().end_offset();
    while file
        .bytes
        .get(open_end)
        .is_some_and(|b| matches!(b, b' ' | b'\t'))
    {
        open_end += 1;
    }
    edits.push(Correction {
        range: start..start + 2,
        replacement: String::new(),
    });
    edits.push(Correction {
        range: hash.opening_loc().start_offset()..open_end,
        replacement: String::new(),
    });
    let close = hash.closing_loc();
    let mut close_start = close.start_offset();
    while close_start > open_end && matches!(file.bytes[close_start - 1], b' ' | b'\t') {
        close_start -= 1;
    }
    // Here the closing margin is one physical line and heredocs have already
    // been excluded. This byte can only be the final literal separator.
    if close_start > open_end && file.bytes[close_start - 1] == b',' {
        close_start -= 1;
    }
    edits.push(Correction {
        range: close_start..close.end_offset(),
        replacement: String::new(),
    });
    for n in hash.elements().iter() {
        if let Some(splat) = n.as_assoc_splat_node() {
            // An unknown splat may contain custom keys. Eliminating a hash
            // merge can change their hash/eql? effects or exceptions, even if
            // to_hash is still called exactly once. Only closed label literals
            // prove that every key in the flattened hash is a Symbol.
            let Some(inner) = splat.value().and_then(|n| n.as_hash_node()) else {
                return false;
            };
            if !label_hash(&inner)
                || !collect_removals(
                    &inner,
                    splat.operator_loc().start_offset(),
                    file,
                    opening_continues,
                    edits,
                )
            {
                return false;
            }
        }
    }
    true
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop redundant_double_splat_hash_braces_spec.rb
    //! (MIT License, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == RedundantDoubleSplatHashBraces.name())
            .collect()
    }
    #[test]
    fn pinned_direct_diagnostics_and_corrections() {
        for (before, after, count) in [
            (
                "do_something(**{foo: bar, baz: qux})",
                "do_something(foo: bar, baz: qux)",
                1,
            ),
            (
                "do_something(**{foo: bar, **{baz: qux}})",
                "do_something(foo: bar, baz: qux)",
                2,
            ),
            (
                "block do\n  do_something(**{foo: bar, baz: qux})\nend",
                "block do\n  do_something(foo: bar, baz: qux)\nend",
                1,
            ),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), count, "{before}");
            let (line, col) = if before.starts_with("block") {
                (2, 16)
            } else {
                (1, 14)
            };
            assert_eq!((found[0].line, found[0].col), (line, col));
            assert!(found.iter().all(|o| o.message == MESSAGE));
            if count == 2 {
                assert_eq!((found[1].line, found[1].col), (1, 27));
            }
            assert_correction(&RedundantDoubleSplatHashBraces, before, after);
        }
    }
    #[test]
    fn pinned_unknown_splat_correction_is_withheld() {
        let source = "do_something(**{foo: bar, **options})";
        let found = correction_offenses(&RedundantDoubleSplatHashBraces, source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 14));
        assert_eq!(found[0].message, MESSAGE);
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn all_nine_pinned_merge_correction_calls_are_individually_withheld() {
        // Removing merge dispatch is not safe for unknown to_hash conversions,
        // keyword-vs-positional binding, blocks, or Hash overrides/refinements.
        for s in [
            "do_something(**{foo: bar, baz: qux}.merge(options))",
            "do_something(**{foo: bar, baz: qux}.merge!(options))",
            "do_something(**{foo: bar, baz: qux}.merge(x: y))",
            "do_something(**{foo: bar}.merge({baz: qux}))",
            "do_something(**{foo: bar, baz: qux}&.merge(options))",
            "do_something(**{ foo: bar }.merge(options))\ndo_something(**{ baz: qux }.merge(options))",
            "do_something(**{foo: bar, baz: qux}.merge(options1, options2))",
            "do_something(**{foo: bar, baz: qux}.merge(options1, options2).merge(options3))",
            "do_something(**{foo: bar, baz: qux}.merge(options1, options2)&.merge!(options3))",
        ] {
            let found = correction_offenses(&RedundantDoubleSplatHashBraces, s);
            assert_eq!(found.len(), if s.contains('\n') { 2 } else { 1 }, "{s}");
            assert_eq!((found[0].line, found[0].col), (1, 14));
            assert!(found.iter().all(|o| o.correction.is_none()), "{s}");
        }
    }
    #[test]
    fn parenthesized_calls_and_buried_hash_candidates_coalesce_at_kwsplat() {
        for s in [
            "f(**({a: b}.merge(opts)))",
            "f(**({a:b}.merge(opts) { 1 }))",
            "f(**{}.merge(x: {a:b}.merge(y)))",
            "f(**{x=>y}.merge({x=>{a:b}.merge(y)}))",
            "f(**{}.merge({a:b},{c:d}))",
            "f(**{x=>{a:b}.merge(y)})",
        ] {
            let found = correction_offenses(&RedundantDoubleSplatHashBraces, s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (1, 3));
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn significant_trivia_trailing_commas_and_nested_atomic_edits() {
        for (before, after) in [
            ("f(** \\\n { a: 1 })", "f( \\\n a: 1)"),
            ("f(**{ # keep\n a: 1})", "f(# keep\n a: 1)"),
            ("f(**{a: 1, **{b: 2,},})", "f(a: 1, b: 2)"),
            (
                "{x: 0, **{a: effect(), b:}, z: 2}",
                "{x: 0, a: effect(), b:, z: 2}",
            ),
            ("f **{\"#{key}\": value}", "f \"#{key}\": value"),
        ] {
            assert_correction(&RedundantDoubleSplatHashBraces, before, after);
        }
        for source in [
            "f(**{a: 1,\n}, b: 2)",
            "f(**{a: 1, # keep\n}, b: 2)",
            "f(**{ # keep\n a: 1\n})",
            "f(**{a: <<~X})\n#{g(1,2)}\nX\n",
        ] {
            let found = correction_offenses(&RedundantDoubleSplatHashBraces, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn keyword_blocks_in_values_cannot_rebind_to_outer_command() {
        let s =
            "def make; yield; 1; end\ndef f(**kw); kw; end\nf **{a: make do 2 end}\nf(**{b: 2})\n";
        let found = correction_offenses(&RedundantDoubleSplatHashBraces, s);
        assert_eq!(found.len(), 2);
        assert!(found[0].correction.is_none());
        assert!(found[1].correction.is_some());
        assert_correction(
            &RedundantDoubleSplatHashBraces,
            s,
            &s.replace("f(**{b: 2})", "f(b: 2)"),
        );
    }
    #[test]
    fn unknown_splat_keys_withhold_only_the_uncertain_literal() {
        for source in [
            "f(**{a: 1, **opts}, b: 2)",
            "f(**{a: 1, **{b: 2, **opts}}, c: 3)",
            "f(**{a: 1, **{key => 2}}, b: 3)",
        ] {
            let found = correction_offenses(&RedundantDoubleSplatHashBraces, source);
            assert!(!found.is_empty());
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
            let with_sibling = format!("{source}; f(**{{safe: 1}})");
            assert_correction(
                &RedundantDoubleSplatHashBraces,
                &with_sibling,
                &format!("{source}; f(safe: 1)"),
            );
        }
    }
    #[test]
    fn opening_newlines_require_enclosing_continuation() {
        for source in [
            "f **{\n a: 1}",
            "f **{ # keep\n a: 1}",
            "obj.f **{\r\n a: 1}",
            "f **{a: 1, **{\n b: 2}}",
        ] {
            let found = correction_offenses(&RedundantDoubleSplatHashBraces, source);
            assert!(found[0].correction.is_none(), "{source}");
            let with_sibling = format!("{source}; f(**{{safe: 1}})");
            // The nested offense may safely use the retained outer braces.
            let expected = if source.contains("b: 2") {
                "f **{a: 1, \n b: 2}; f(safe: 1)".to_string()
            } else {
                format!("{source}; f(safe: 1)")
            };
            assert_correction(&RedundantDoubleSplatHashBraces, &with_sibling, &expected);
        }
        for (before, after) in [
            ("f(**{\n a: 1})", "f(\n a: 1)"),
            ("{**{ # keep\n a: 1}}", "{# keep\n a: 1}"),
        ] {
            assert_correction(&RedundantDoubleSplatHashBraces, before, after);
        }
    }
    #[test]
    fn wide_atomic_corrections_and_parameter_block_lookalikes() {
        let before = "f(**{a: 1, **{b: 2}})\n".repeat(6000);
        let after = "f(a: 1, b: 2)\n".repeat(6000);
        assert_correction(&RedundantDoubleSplatHashBraces, &before, &after);
        for s in [
            "def f(**kw); end",
            "f { |**kw| kw }",
            "->(**kw) { kw }",
            "def f(**); g(**); end",
            "case x; in {a:, **rest}; end",
            "f({a: 1})",
            "x = {a: 1}",
            "f(**{a: 1}.merge(x) { |k,l,r| l })",
            "f(**unknown.merge({a: 1}))",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn all_pinned_no_offenses() {
        for s in [
            "do_something(foo: bar, baz: qux)",
            "do_something(**options.merge(foo: bar, baz: qux))",
            "do_something(**options.merge({foo: bar}))",
            "do_something(**options&.merge({foo: bar}))",
            "do_something(**(options.merge(foo: bar)))",
            "do_something(**{})",
            "do_something({})",
            "do_something(**{foo => bar})",
            "do_something(**{foo: bar}.invert)",
            "do_something(**{foo: bar, baz: qux}.merge(options).compact_blank)",
            "do_something({foo: bar, baz: qux})",
            "do_something(**h)",
            "{ a: a }",
            "do_something(**x.do_something { [foo: bar] })",
            "do_something(**x.do_something { {foo: {bar: baz}} })",
            "do_something(**x.do_something { {foo: {bar: _1}} })",
            "do_something(**(foo ? {bar: bar} : baz))",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
