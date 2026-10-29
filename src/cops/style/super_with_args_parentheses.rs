//! Parentheses for explicit `super` arguments.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct SuperWithArgsParentheses;
impl Cop for SuperWithArgsParentheses {
    fn name(&self) -> &'static str {
        "Style/SuperWithArgsParentheses"
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
        let Some(call) = node.as_super_node() else {
            return;
        };
        if call.lparen_loc().is_some() {
            return;
        }
        let start = call.keyword_loc().start_offset();
        let end = super::super_arguments::source_end(&call);
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            message: "Use parentheses for `super` with arguments.".into(),
            correction: cx
                .corrections_requested()
                .then(|| super::method_call_with_args_parentheses::correction(node, end, file))
                .flatten()
                .filter(|edit| super::correction::preserves_data_positions(file, cx, edit)),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop super_with_args_parentheses_spec.rb (MIT, Copyright
    //! (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    #[test]
    fn command_group_heredoc_and_block_ownership() {
        for (before, after) in [
            (
                "def f(&block); super &block; end",
                "def f(&block); super(&block); end",
            ),
            (
                "def f(*items, **settings, &callback); super *items, **settings, &callback; end",
                "def f(*items, **settings, &callback); super(*items, **settings, &callback); end",
            ),
            ("super x do\n body\nend", "super(x) do\n body\nend"),
            ("super x { body }", "super(x { body })"),
            ("super foo 1, 2", "super(foo 1, 2)"),
            ("super (value)", "super(value)"),
            ("super (a; b)", "super((a; b))"),
            ("super (value if condition)", "super((value if condition))"),
            (
                "super (alias copied original)",
                "super((alias copied original))",
            ),
            ("super (undef original)", "super((undef original))"),
            ("super (END { body })", "super((END { body }))"),
            ("super (1;)", "super((1;))"),
            (
                "super <<~TEXT\n  café #{__LINE__}\nTEXT\n",
                "super(<<~TEXT)\n  café #{__LINE__}\nTEXT\n",
            ),
            (
                "super \\\n value,\n __LINE__",
                "super(\\\n value,\n __LINE__)",
            ),
        ] {
            assert_correction(&SuperWithArgsParentheses, before, after);
        }
    }
    #[test]
    fn reports_invocation_not_literal_block_body() {
        let source = "super value do\n body\nend";
        let found = correction_offenses(&SuperWithArgsParentheses, source);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].end_line, 1);
        let source = "def f(...); super ...; end";
        // With a space this is an endless RangeNode, not `...` forwarding.
        assert!(correction_offenses(&SuperWithArgsParentheses, source).is_empty());
    }
    #[test]
    fn data_byte_position_observations_withhold_length_changing_edits() {
        for observer in [
            "p DATA.pos",
            "p Object.const_get(:DATA).pos",
            "p DATA.method(:pos).call",
        ] {
            let source = format!("def f(a); super a; end\n{observer}\n__END__\npayload\n");
            let found = correction_offenses(&SuperWithArgsParentheses, &source);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn pinned_examples() {
        let found = correction_offenses(&SuperWithArgsParentheses, "super name, age\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 1));
        assert_eq!(
            found[0].message,
            "Use parentheses for `super` with arguments."
        );
        assert_correction(
            &SuperWithArgsParentheses,
            "super name, age\n",
            "super(name, age)\n",
        );
        for source in ["super(name, age)", "super()", "super"] {
            assert!(correction_offenses(&SuperWithArgsParentheses, source).is_empty());
        }
    }
}
