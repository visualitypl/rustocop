//! `Style/InverseMethods` — fixed inverse predicates/blocks, detection only.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};

pub struct InverseMethods;
impl Cop for InverseMethods {
    fn name(&self) -> &'static str {
        "Style/InverseMethods"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        let pair = if let Some((method, inverse, _)) = inverse_block(&call) {
            let mut parents = cx.ancestors();
            if double_negated(&mut parents) || has_next(node, cx, file) {
                return;
            }
            Some((method, inverse))
        } else if call.name().as_slice() == b"!"
            && !call.is_safe_navigation()
            && ast::call_arguments(&call).is_empty()
            && call.block().is_none()
        {
            if cx.parent().is_some_and(is_not) {
                return;
            }
            // RuboCop ignores the inverted block-tail subtree, not earlier
            // siblings or every predicate in the surrounding block.
            for (i, parent) in cx.ancestors().enumerate() {
                if let Some(parent_call) = parent.as_call_node()
                    && let Some((_, _, tail)) = inverse_block(&parent_call)
                {
                    let loc = tail.location();
                    // Lexical containment is not AST ownership: heredoc
                    // interpolations can be beyond the call/block's source end.
                    if std::iter::once(node)
                        .chain(cx.ancestors().take(i))
                        .any(|ancestor| {
                            ancestor.as_call_node().is_some()
                                && ancestor.location().start_offset() == loc.start_offset()
                                && ancestor.location().end_offset() == loc.end_offset()
                        })
                        && !double_negated(&mut cx.ancestors().skip(i + 1))
                        && !has_next(parent, cx, file)
                    {
                        return;
                    }
                }
            }
            let Some(receiver) = call.receiver().and_then(single_group) else {
                return;
            };
            let Some(candidate) = receiver.as_call_node() else {
                return;
            };
            if candidate.receiver().is_none()
                || (candidate
                    .block()
                    .is_some_and(|b| b.as_block_node().is_some())
                    && !ast::call_arguments(&candidate).is_empty())
            {
                return;
            }
            let method = candidate.name().as_slice();
            let Some(inverse) = inverse_method(method) else {
                return;
            };
            if candidate.is_safe_navigation()
                && matches!(method, b"any?" | b"none?" | b"<" | b">" | b"<=" | b">=")
            {
                return;
            }
            if matches!(method, b"<" | b">" | b"<=" | b">=")
                && (candidate.receiver().is_some_and(|n| camel_constant(&n))
                    || matches!(ast::call_arguments(&candidate).as_slice(), [rhs] if camel_constant(rhs)))
            {
                return;
            }
            Some((method, inverse))
        } else {
            None
        };
        let Some((method, inverse)) = pair else {
            return;
        };
        let loc = node.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(loc),
            message: format!(
                "Use `{}` instead of inverting `{}`.",
                String::from_utf8_lossy(inverse),
                String::from_utf8_lossy(method)
            ),
            correction: None,
        });
    }
}
fn inverse_method(method: &[u8]) -> Option<&'static [u8]> {
    Some(match method {
        b"any?" => b"none?",
        b"none?" => b"any?",
        b"even?" => b"odd?",
        b"odd?" => b"even?",
        b"==" => b"!=",
        b"!=" => b"==",
        b"=~" => b"!~",
        b"!~" => b"=~",
        b"<" => b">=",
        b">=" => b"<",
        b">" => b"<=",
        b"<=" => b">",
        _ => return None,
    })
}
fn is_not(node: &Node<'_>) -> bool {
    node.as_call_node()
        .is_some_and(|c| c.name().as_slice() == b"!")
}
fn double_negated<'a, 'pr: 'a>(parents: &mut impl Iterator<Item = &'a Node<'pr>>) -> bool {
    parents.next().is_some_and(is_not) && parents.next().is_some_and(is_not)
}
fn single_group(node: Node<'_>) -> Option<Node<'_>> {
    if let Some(p) = node.as_parentheses_node() {
        let s = p.body()?.as_statements_node()?;
        if s.body().len() != 1 {
            return None;
        }
        s.body().iter().next()
    } else {
        Some(node)
    }
}
fn camel_constant(node: &Node<'_>) -> bool {
    ast::is_const(node)
        && node
            .location()
            .as_slice()
            .windows(2)
            .any(|w| w[0].is_ascii_uppercase() && w[1].is_ascii_lowercase())
}
fn inverse_block<'pr>(call: &CallNode<'pr>) -> Option<(&'pr [u8], &'static [u8], Node<'pr>)> {
    call.receiver()?;
    if !ast::call_arguments(call).is_empty() {
        return None;
    }
    let method = call.name().as_slice();
    let inverse: &[u8] = match method {
        b"select" => b"reject",
        b"reject" => b"select",
        b"select!" => b"reject!",
        b"reject!" => b"select!",
        _ => return None,
    };
    let block = call.block()?.as_block_node()?;
    let body = block.body()?.as_statements_node()?;
    let mut tail = body.body().last()?;
    if let Some(p) = tail.as_parentheses_node() {
        tail = p.body()?.as_statements_node()?.body().last()?;
    }
    let inverted = tail.as_call_node()?;
    if inverted
        .block()
        .is_some_and(|b| b.as_block_node().is_some())
        || (inverted.name().as_slice() == b"!" && !ast::call_arguments(&inverted).is_empty())
        || !matches!(inverted.name().as_slice(), b"!" | b"!=" | b"!~")
        || (inverted.is_safe_navigation() && inverted.name().as_slice() != b"!")
    {
        return None;
    }
    Some((method, inverse, tail))
}
fn has_next(node: &Node<'_>, cx: &NodeContext<'_, '_>, file: &SourceFile) -> bool {
    let root = cx.ancestors().next_back().unwrap_or(node);
    file.inverse_method_nexts(root)
        .owners
        .contains(&(node.location().start_offset(), node.location().end_offset()))
}
/// One lazy postorder main-AST pass. A block exempts even syntactically nested
/// `next` nodes, including those in its call's arguments (RuboCop block shape).
#[derive(Default)]
pub(crate) struct NextIndex {
    owners: std::collections::HashSet<(usize, usize)>,
    stack: Vec<(Option<(usize, usize)>, bool)>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl NextIndex {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut index = Self::default();
        crate::cops::walk::walk(&mut index, root);
        index
    }
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for NextIndex {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        {
            self.visits += 1;
        }
        let owner = node
            .as_call_node()
            .filter(|c| c.block().is_some())
            .map(|_| (node.location().start_offset(), node.location().end_offset()));
        self.stack.push((owner, node.as_next_node().is_some()));
    }
    fn leave(&mut self) {
        let (owner, next) = self.stack.pop().unwrap();
        if next {
            self.owners.extend(owner);
            if let Some(parent) = self.stack.last_mut() {
                parent.1 = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/inverse_methods_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::correction_offenses;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == InverseMethods.name())
            .collect()
    }
    #[test]
    fn fixed_inverse_pairs_and_positions() {
        for (method, inverse) in [
            ("any?", "none?"),
            ("none?", "any?"),
            ("even?", "odd?"),
            ("odd?", "even?"),
        ] {
            for receiver in [
                "foo",
                "$foo",
                "@foo",
                "@@foo",
                "FOO",
                "FOO::BAR",
                "foo[\"bar\"]",
                "foo.bar",
            ] {
                for negation in ["!", "not "] {
                    let s = format!("\n  {negation}{receiver}.{method}");
                    let o = offenses(&s);
                    assert_eq!(o.len(), 1, "{s}");
                    assert_eq!((o[0].line, o[0].col), (2, 3));
                    assert_eq!(
                        o[0].message,
                        format!("Use `{inverse}` instead of inverting `{method}`.")
                    );
                    assert!(
                        correction_offenses(&InverseMethods, &s)[0]
                            .correction
                            .is_none()
                    );
                }
            }
        }
    }
    #[test]
    fn operator_pairs_blocks_and_parent_binding() {
        for (op, inverse) in [
            ("==", "!="),
            ("!=", "=="),
            ("=~", "!~"),
            ("!~", "=~"),
            ("<", ">="),
            (">=", "<"),
            (">", "<="),
            ("<=", ">"),
        ] {
            let s = format!("foo != !(bar {op} baz)");
            let o = offenses(&s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (1, 8));
            assert_eq!(
                o[0].message,
                format!("Use `{inverse}` instead of inverting `{op}`.")
            );
        }
        for s in [
            "!foo.none?(&:even?)",
            "!foo.none? { |f| f.even? }",
            "!foo.none? { _1.even? }",
            "!foo.none? { it.even? }",
            "!(foo.any? &:working?)",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
    }
    #[test]
    fn inverse_blocks_ignore_only_the_inverted_tail() {
        for (method, inverse) in [
            ("select", "reject"),
            ("reject", "select"),
            ("select!", "reject!"),
            ("reject!", "select!"),
        ] {
            for receiver in ["foo.", "foo&."] {
                for body in [
                    "!e",
                    "e != 2",
                    "e !~ /a/",
                    "something; !e.even?",
                    "e.bar?.!",
                    "e&.bar?&.!",
                    "!(e =~ /a/)",
                    "e.!=(&block)",
                    "e.!~(pattern, &block)",
                ] {
                    let s = format!("{receiver}{method} {{ |e| {body} }}");
                    let o = offenses(&s);
                    assert_eq!(o.len(), 1, "{s}");
                    assert_eq!((o[0].line, o[0].col), (1, 1));
                    assert_eq!(
                        o[0].message,
                        format!("Use `{inverse}` instead of inverting `{method}`.")
                    );
                }
            }
        }
        let o = offenses("foo.select { |e| !e.even?; !e }");
        assert_eq!(o.len(), 2);
    }
    #[test]
    fn implicit_receivers_negation_arity_and_literal_block_arguments_are_not_candidates() {
        for s in [
            "!none?",
            "not any?",
            "!(even?)",
            "odd?.!",
            "x.none?.!(y)",
            "x.none?.!(&b)",
            "x.select { |e| e.none?.!(x) }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        for (s, col) in [
            ("!x.==(y) { !item.even? }", 12),
            ("!x.<(y) { !item.even? }", 11),
            ("!none? { !item.even? }", 10),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (1, col));
        }
    }

    #[test]
    fn argument_bearing_blocks_do_not_match_or_suppress_tail_offenses() {
        for method in ["select", "reject", "select!", "reject!"] {
            assert!(offenses(&format!("foo.{method}(1) {{ !e }}")).is_empty());
        }
        let o = offenses("foo.select(foo) { |e| !e.even? }");
        assert_eq!(o.len(), 1);
        assert_eq!((o[0].line, o[0].col), (1, 23));
        assert_eq!(o[0].message, "Use `odd?` instead of inverting `even?`.");
        assert!(o[0].correction.is_none());
    }

    #[test]
    fn ignored_tail_ownership_includes_deferred_heredoc_children() {
        let s = "foo.select { |e| !e.any?(<<~TEXT) }\n#{!bar.even?}\nTEXT\n";
        let o = offenses(s);
        assert_eq!(o.len(), 1, "{o:?}");
        assert_eq!((o[0].line, o[0].col), (1, 1));
        assert_eq!(o[0].message, "Use `reject` instead of inverting `select`.");
        assert_eq!(offenses("!(foo.<(Foo, other))").len(), 1);
    }

    #[test]
    fn wide_block_checks_tail_in_constant_time_and_keeps_earlier_siblings() {
        let s = format!("foo.select do |e|\n{}!e\nend", "!e.even?\n".repeat(6000));
        let o = offenses(&s);
        assert_eq!(o.len(), 6001);
        assert_eq!(o[0].message, "Use `reject` instead of inverting `select`.");
    }

    #[test]
    fn exemptions_and_non_default_maps() {
        for s in [
            "foo.none?",
            "!!(string =~ /a/)",
            "!!foo.reject { |e| !e }",
            "!foo&.none?(&:even?)",
            "!(foo&.any? &:working?)",
            "!nullable&.<(0)",
            "!(klass < BaseClass)",
            "!(BaseClass < klass)",
            "!foo.blank?",
            "!foo.present?",
            "!foo.include?(x)",
            "!foo.exclude?(x)",
            "foo.select { |e| next if e.zero?; e != 2 }",
            "select { |e| !e }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        assert_eq!(offenses("!(klass < FOO_BAR)").len(), 1);
        assert_eq!(offenses("!foo&.even?").len(), 1);
    }
}
