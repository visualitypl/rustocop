//! `Style/ItAssignment` — reserve `it` for implicit block parameters.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ItAssignment;
impl Cop for ItAssignment {
    fn name(&self) -> &'static str {
        "Style/ItAssignment"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let mut location = None;
        macro_rules! named {
            ($($access:ident),* $(,)?) => {$(
                if let Some(n) = node.$access() && n.name().as_slice() == b"it" {
                    location = Some(n.name_loc());
                }
            )*};
        }
        named!(
            as_local_variable_write_node,
            as_local_variable_and_write_node,
            as_local_variable_or_write_node,
            as_local_variable_operator_write_node,
            as_optional_parameter_node,
            as_optional_keyword_parameter_node,
            as_required_keyword_parameter_node
        );
        if let Some(n) = node.as_required_parameter_node()
            && n.name().as_slice() == b"it"
        {
            location = Some(n.location());
        }
        if let Some(n) = node.as_local_variable_target_node()
            && n.name().as_slice() == b"it"
        {
            // Pattern variables are match_var, not lvasgn; explicit assignment
            // targets nested in a pattern's value still count.
            let explicit = cx
                .ancestors()
                .find_map(|a| match a {
                    Node::MultiWriteNode { .. }
                    | Node::ForNode { .. }
                    | Node::RescueNode { .. } => Some(true),
                    Node::MatchWriteNode { .. }
                    | Node::MatchRequiredNode { .. }
                    | Node::MatchPredicateNode { .. }
                    | Node::InNode { .. } => Some(false),
                    _ => None,
                })
                .unwrap_or(true);
            if explicit {
                location = Some(n.location());
            }
        }
        macro_rules! optional {
            ($($access:ident),* $(,)?) => {$(
                if let Some(n) = node.$access() && n.name().is_some_and(|name| name.as_slice() == b"it") {
                    location = n.name_loc();
                }
            )*};
        }
        optional!(
            as_rest_parameter_node,
            as_keyword_rest_parameter_node,
            as_block_parameter_node
        );
        let Some(location) = location else {
            return;
        };
        let (line, col) = file.line_col(location.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "`it` is the default block parameter; consider another name.".into(),
            line,
            col,
            end_line: line,
            correction: None,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/it_assignment_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str;
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == ItAssignment.name())
            .collect()
    }
    #[test]
    fn pinned_assignments_and_every_parameter_kind() {
        for (source, line, col) in [
            ("it = 5", 1, 1),
            ("def foo(it)\nend", 1, 9),
            ("def foo(it:)\nend", 1, 9),
            ("def foo(it = 5)\nend", 1, 9),
            ("def foo(it: 5)\nend", 1, 9),
            ("def foo(*it)\nend", 1, 10),
            ("def foo(**it)\nend", 1, 11),
            ("def foo(&it)\nend", 1, 10),
            ("foo { it = 5 }", 1, 7),
            ("foo do\n  it = 5\n  bar(it)\nend", 2, 3),
            ("foo { |x| it = x }", 1, 11),
            ("foo { it = _2 }", 1, 7),
            ("-> { it = 5 }", 1, 6),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (line, col), "{source}");
            assert_eq!(
                found[0].message,
                "`it` is the default block parameter; consider another name."
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn compound_multiple_rescue_for_and_destructured_parameters() {
        for source in [
            "it ||= 1",
            "it &&= 1",
            "it += 1",
            "it, x = values",
            "x, (it,y) = values",
            "for it in xs; end",
            "begin; foo; rescue => it; end",
            "foo { |it| it }",
            "->(it) { it }",
            "def f((it,x));end",
        ] {
            let f = offenses(source);
            assert_eq!(f.len(), 1, "{source}: {f:?}");
            assert_eq!(f[0].col, source.find("it").unwrap() as u32 + 1);
        }
    }
    #[test]
    fn pattern_bindings_named_captures_and_block_locals_are_not_lvasgn() {
        for source in [
            "/(?<it>.)/ =~ value",
            "value => [it]",
            "value in {it:}",
            "case value; in [it]; end",
            "foo { |;it| use(it) }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn pinned_lookalikes_and_implicit_parameter() {
        for source in [
            "self.it = 5",
            "foo { self.it = 5 }",
            "@it = 5",
            "foo { @it = 5 }",
            "$it = 5",
            "foo { $it = 5 }",
            "IT = 5",
            "def it\nend",
            "foo { puts it }",
            "foo { _1 }",
            "item = 5",
            "# it = 5\n\"it = 5\"",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
