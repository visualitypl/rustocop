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
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_assoc_splat_node().is_some()
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
        let loc = splat.location();
        let (line, col) = file.line_col(loc.start_offset());
        let correction = cx
            .corrections_requested()
            .then(|| correction(&splat, &value, file))
            .flatten();
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

/// RuboCop's corrector for the hash that reports the double splat first:
/// the splat's value, or the receiver its merge chain starts from.
fn correction(
    splat: &ruby_prism::AssocSplatNode<'_>,
    value: &Node<'_>,
    file: &SourceFile,
) -> Option<Correction> {
    let edit =
        |range: std::ops::Range<usize>, replacement: String| Correction { range, replacement };
    let mut hash = value.as_hash_node();
    let mut call = value.as_call_node();
    while let Some(c) = call {
        let receiver = c.receiver()?;
        hash = receiver.as_hash_node();
        call = receiver.as_call_node();
    }
    let hash = hash.filter(label_hash)?;
    let elements = hash.elements();
    let (first, last) = (elements.iter().next()?, elements.last()?);
    let operator = splat.operator_loc();
    let mut edits = vec![
        edit(
            operator.start_offset()..operator.end_offset(),
            String::new(),
        ),
        edit(
            hash.opening_loc().start_offset()..first.location().start_offset(),
            String::new(),
        ),
        edit(
            last.location().end_offset()..hash.closing_loc().end_offset(),
            String::new(),
        ),
    ];
    // `select_merge_method_nodes`: mergeable calls, in pre-order.
    struct Merges<'pr> {
        stack: Vec<Node<'pr>>,
        found: Vec<(Option<usize>, usize, Vec<String>)>,
    }
    impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Merges<'pr> {
        fn enter(&mut self, n: Node<'pr>) {
            if let Some(call) = n.as_call_node() {
                let mergeable = std::iter::once(&n)
                    .chain(self.stack.iter().rev())
                    .filter(|a| a.as_arguments_node().is_none())
                    .map_while(|a| a.as_call_node())
                    .all(|c| matches!(c.name().as_slice(), b"merge" | b"merge!"));
                if mergeable {
                    let arguments = crate::cops::ast::call_arguments(&call)
                        .iter()
                        .map(|arg| {
                            let text = std::str::from_utf8(arg.location().as_slice()).unwrap_or("");
                            match arg.as_hash_node() {
                                Some(h) => h
                                    .elements()
                                    .iter()
                                    .filter(|e| e.as_assoc_node().is_some())
                                    .map(|e| {
                                        std::str::from_utf8(e.location().as_slice())
                                            .unwrap_or("")
                                            .to_owned()
                                    })
                                    .collect::<Vec<_>>()
                                    .join(", "),
                                None if arg.as_keyword_hash_node().is_some() => text.to_owned(),
                                None => format!("**{text}"),
                            }
                        })
                        .collect();
                    self.found.push((
                        call.call_operator_loc().map(|l| l.start_offset()),
                        crate::cops::ast::source_end(&n),
                        arguments,
                    ));
                }
            }
            self.stack.push(n);
        }
        fn leave(&mut self) {
            self.stack.pop();
        }
    }
    let mut merges = Merges {
        stack: vec![splat.as_node()],
        found: Vec::new(),
    };
    crate::cops::walk::walk(&mut merges, value);
    if let (Some(outer), Some(inner)) = (merges.found.first(), merges.found.last()) {
        let range = inner.0?..outer.1;
        let mut arguments = vec![String::new()];
        for (_, _, args) in merges.found.iter().rev() {
            arguments.extend(args.iter().cloned());
        }
        edits.push(edit(range, arguments.join(", ")));
    }
    super::correction::combine(file, edits)
}

pub(crate) static REDUNDANT_DOUBLE_SPLATS: crate::source::Fact<std::collections::HashSet<usize>> =
    crate::source::Fact::new();

impl crate::source::SourceFile {
    pub(crate) fn redundant_double_splats(
        &self,
        root: &ruby_prism::Node<'_>,
    ) -> &std::collections::HashSet<usize> {
        REDUNDANT_DOUBLE_SPLATS.get(self, || index(root))
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop redundant_double_splat_hash_braces_spec.rb
    //! (MIT License, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::correction_offenses;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == RedundantDoubleSplatHashBraces.name())
            .collect()
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
