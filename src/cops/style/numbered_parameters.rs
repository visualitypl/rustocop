//! `Style/NumberedParameters` — numbered parameters only in single-line blocks.
use super::implicit_parameters;
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct NumberedParameters;
impl Cop for NumberedParameters {
    fn name(&self) -> &'static str {
        "Style/NumberedParameters"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(b) = implicit_parameters::block(node) else {
            return;
        };
        if b.parameters
            .is_some_and(|p| p.as_numbered_parameters_node().is_some())
            && file.line_of(b.opening.start_offset()) != file.line_of(b.closing.start_offset())
        {
            implicit_parameters::report(
                self,
                node,
                cx,
                file,
                out,
                "Avoid using numbered parameters for multi-line blocks.",
            );
        }
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/numbered_parameters_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == NumberedParameters.name())
            .collect()
    }
    #[test]
    fn pinned_multiline_position_message_and_no_fix() {
        let f = offenses("collection.each do\n  puts _1\nend\n");
        assert_eq!(f.len(), 1);
        assert_eq!((f[0].line, f[0].col, f[0].end_line), (1, 1, 3));
        assert_eq!(
            f[0].message,
            "Avoid using numbered parameters for multi-line blocks."
        );
        assert!(!NumberedParameters.safe_fixes());
        assert!(f[0].correction.is_none());
    }
    #[test]
    fn fixed_version_bounds_are_explicit() {
        use crate::cops::{
            policy::{DEFAULT_RUBY_VERSION, RubyVersion},
            style::{ItAssignment, ItBlockParameter},
        };
        assert_eq!(
            NumberedParameters.ruby_versions().minimum,
            Some(RubyVersion(2, 7))
        );
        assert!(
            !NumberedParameters
                .ruby_versions()
                .supports(RubyVersion(2, 6))
        );
        assert!(
            NumberedParameters
                .ruby_versions()
                .supports(DEFAULT_RUBY_VERSION)
        );
        assert_eq!(
            ItBlockParameter.ruby_versions().minimum,
            Some(RubyVersion(3, 4))
        );
        assert!(!ItBlockParameter.ruby_versions().supports(RubyVersion(3, 3)));
        assert!(
            ItBlockParameter
                .ruby_versions()
                .supports(DEFAULT_RUBY_VERSION)
        );
        assert_eq!(ItAssignment.ruby_versions().minimum, None);
    }
    #[test]
    fn lambda_proc_nested_and_receiver_association() {
        for (source, positions) in [
            ("-> do\n puts _2\nend", vec![(1, 1)]),
            ("lambda do\n puts _1\nend", vec![(1, 1)]),
            ("proc {\n use(_9)\n}", vec![(1, 1)]),
            ("foo { inner do\n puts _1\nend }", vec![(1, 7)]),
            ("foo do\n inner { puts _1 }\nend", vec![]),
            ("x = foo(42) do\n puts _1\nend.bar", vec![(1, 5)]),
            ("obj&.foo do\n puts _1\nend", vec![(1, 1)]),
        ] {
            let f = offenses(source);
            assert_eq!(
                f.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                positions,
                "{source}"
            );
            assert!(
                crate::cops::testing::correction_offenses(&NumberedParameters, source)
                    .iter()
                    .all(|o| o.correction.is_none())
            );
        }
    }
    #[test]
    fn pinned_negatives_and_explicit_it_lookalikes() {
        for s in [
            "collection.each\n          .foo { puts _1 }",
            "collection.each { puts _1 }",
            "collection.each do |x|\n puts x\nend",
            "collection.each do\n puts it\nend",
            "_1()",
            "# _1\n\"_1\"",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
