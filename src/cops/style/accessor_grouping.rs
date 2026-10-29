//! `Style/AccessorGrouping` — fixed grouped declarations.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct AccessorGrouping;
impl Cop for AccessorGrouping {
    fn name(&self) -> &'static str {
        "Style/AccessorGrouping"
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
        let Some(children) = super::accessors::children(node) else {
            return;
        };
        let mut groups = std::collections::HashMap::<(Vec<u8>, Vec<u8>), Vec<usize>>::new();
        let mut visibility = b"public".to_vec();
        for (i, child) in children.iter().enumerate() {
            let Some(call) = child.as_call_node() else {
                continue;
            };
            if let Some(v) = super::accessors::visibility(&call) {
                visibility = v.to_vec();
            }
            if super::accessors::family(&call)
                && super::accessors::groupable(child, i.checked_sub(1).map(|p| &children[p]), file)
            {
                groups
                    .entry((call.name().as_slice().to_vec(), visibility.clone()))
                    .or_default()
                    .push(i);
            }
        }
        for ((family, _), indexes) in groups {
            if indexes.len() < 2 {
                continue;
            }
            let correction = if cx.corrections_requested()
                && indexes.windows(2).all(|w| w[1] == w[0] + 1)
                && cx.ancestors().last().is_some_and(|root| {
                    file.accessor_bindings(root).contains(&children[indexes[0]])
                }) {
                let nodes: Vec<_> = indexes.iter().map(|i| &children[*i]).collect();
                let mut names = Vec::new();
                for child in &nodes {
                    for arg in crate::cops::ast::call_arguments(&child.as_call_node().unwrap()) {
                        let text = String::from_utf8_lossy(arg.location().as_slice()).into_owned();
                        names.push(text);
                    }
                }
                super::accessors::rewrite(
                    file,
                    &nodes,
                    &format!("{} {}", String::from_utf8_lossy(&family), names.join(", ")),
                )
            } else {
                None
            };
            for (n, i) in indexes.into_iter().enumerate() {
                let child = &children[i];
                let (line, col) = file.line_col(child.location().start_offset());
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    message: format!(
                        "Group together all `{}` attributes.",
                        String::from_utf8_lossy(&family)
                    ),
                    line,
                    col,
                    end_line: file.last_line_of(child.location()),
                    correction: if n == 0 { correction.clone() } else { None },
                });
            }
        }
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/accessor_grouping_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/AccessorGrouping")
            .collect()
    }
    #[test]
    fn groups_same_family_and_visibility_across_other_statements() {
        let s = "class Foo\n  attr_reader :bar1\n  attr_reader :bar2\n  attr_accessor :quux\n  private\n  attr_reader :baz\n  public\n  attr_reader :bar3\nend\n";
        let found = offenses(s);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(2, 3), (3, 3), (8, 3)]
        );
        assert!(
            found
                .iter()
                .all(|o| o.message == "Group together all `attr_reader` attributes.")
        );
    }
    #[test]
    fn docs_annotations_and_adjacent_calls_are_not_groupable() {
        for body in [
            "# docs\nattr_reader :one\nattr_reader :two",
            "sig { returns(Integer) }\nattr_reader :one\n\nattr_reader :two, :three",
            "attr_reader :one #: String\n\nattr_reader :two, :three",
            "annotation :one\nattr_reader :one\nannotation :two\nattr_reader :two",
        ] {
            assert!(
                offenses(&format!("class Foo\n{body}\nend")).is_empty(),
                "{body}"
            );
        }
    }
    #[test]
    fn multiline_annotation_blocks_use_the_unwrapped_send_location() {
        for previous in [
            "sig do\nfoo\nend",
            "sig do\nfoo\nend #: String",
            "attr_reader(\n:a\n) #: String",
        ] {
            let found = offenses(&format!(
                "class Foo\n{previous}\nattr_reader :b\nattr_reader :c\nend\n"
            ));
            assert_eq!(
                found.len(),
                if previous.starts_with("attr_reader") {
                    3
                } else {
                    2
                },
                "{previous}"
            );
        }
    }
    #[test]
    fn generated_singleton_overrides_do_not_prove_core_dispatch_or_callbacks() {
        for name in [
            "method_added",
            "singleton_method_added",
            "attr",
            "attr_reader",
            "attr_writer",
            "attr_accessor",
            "private",
            "protected",
            "public",
        ] {
            let source = format!(
                "class Foo\n  class << self\n    attr_reader :{name}\n  end\n  attr_reader :one\n  attr_reader :two\nend\n"
            );
            let found =
                crate::cops::testing::correction_offenses(&super::AccessorGrouping, &source);
            assert_eq!(found.len(), 2, "{source}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        }
    }
    #[test]
    fn all_attribute_families_class_module_eigenclass_and_arguments() {
        for scope in ["class Foo", "module Foo", "class << self"] {
            for family in ["attr", "attr_reader", "attr_writer", "attr_accessor"] {
                let source = format!("{scope}\n  {family} :one\n  {family}(:two, :three)\nend\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 2, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                assert_eq!(
                    found[1].message,
                    format!("Group together all `{family}` attributes.")
                );
            }
        }
        for source in [
            "attr_reader :one\nattr_reader :two",
            "def f; attr_reader :one; attr_reader :two; end",
            "Class.new { attr_reader :one; attr_reader :two }",
            "class Foo; self.attr_reader :one; self.attr_reader :two; end",
            "class Foo; attr_reader; attr_reader; end",
            "class Foo; attr_reader(:one) {}; attr_reader(:two) {}; end",
            "class Foo; if x; attr_reader :one; attr_reader :two; end; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn pinned_documentation_and_eigenclass_corrections() {
        for (before, after) in [
            (
                "class Foo\n  # docs for one\n  attr_reader :one, :two\n\n  # docs for three\n  attr_reader :three\n\n  attr_reader :four\n  attr_reader :five\nend\n",
                "class Foo\n  # docs for one\n  attr_reader :one, :two\n\n  # docs for three\n  attr_reader :three\n\n  attr_reader :four, :five\nend\n",
            ),
            (
                "class Foo\n  attr_reader :bar\n  class << self\n    attr_reader :baz1, :baz2\n    attr_reader :baz3\n    private\n    attr_reader :quux1\n    attr_reader :quux2\n  end\nend\n",
                "class Foo\n  attr_reader :bar\n  class << self\n    attr_reader :baz1, :baz2, :baz3\n    private\n    attr_reader :quux1, :quux2\n  end\nend\n",
            ),
        ] {
            crate::cops::testing::assert_correction(&super::AccessorGrouping, before, after);
        }
    }
    #[test]
    fn individually_withheld_pinned_grouping_rewrites_keep_offenses() {
        for body in [
            // Mixed macro, visibility/order changes, annotations, definitions,
            // duplicate warnings, trailing docs, splats/constants, and constants.
            "attr_reader :bar1\nattr_reader :bar2\nattr_accessor :quux\nattr_reader :bar3, :bar4\nother_macro :zoo",
            "attr_reader :bar1\nattr_reader :bar2\nprotected\nattr_accessor :quux\nprivate\nattr_reader :baz1, :baz2\nattr_writer :baz3\nattr_reader :baz4\npublic\nattr_reader :bar3\nother_macro :zoo",
            "extend T::Sig\nsig { returns(Integer) }\nattr_reader :one\n\nattr_reader :two, :three\n\nattr_reader :four",
            "def foo; end\nattr_reader :one\ndef bar; end\nattr_reader :two",
            "attr_reader :one\nattr_reader :two\nattr_reader :one",
            "attr_reader :one # comment #: String\nattr_reader :two, :three",
            "attr_reader :one\nOTHER_ATTRS = %i[two three].freeze\nattr_reader(*OTHER_ATTRS)",
            "attr_reader :one\nattr_reader :bar\nOTHER_ATTRS = %i[two three].freeze",
        ] {
            let source = format!("class Foo\n{body}\nend\n");
            let found =
                crate::cops::testing::correction_offenses(&super::AccessorGrouping, &source);
            assert!(!found.is_empty(), "{source}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        }
    }
    #[test]
    fn one_explicit_begin_layer_is_a_direct_body_but_not_rescue_or_nested_begins() {
        let found = offenses("class Foo\nbegin\nattr_reader :a\nattr_reader :b\nend\nend");
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(3, 1), (4, 1)]
        );
        for body in [
            "begin\nattr_reader :a\nattr_reader :b\nrescue\nend",
            "((attr_reader :a; attr_reader :b))",
        ] {
            assert!(offenses(&format!("class Foo\n{body}\nend")).is_empty());
        }
    }
    #[test]
    fn safe_navigation_lambda_and_module_function_previous_siblings() {
        for (previous, count) in [
            ("self&.other", 2),
            ("self&.attr_reader :a", 2),
            ("-> {}", 0),
            ("-> do\nx\nend", 2),
            ("module_function", 2),
            ("module_function :x", 2),
        ] {
            let source = format!("class Foo\n{previous}\nattr_reader :b\nattr_reader :c\nend\n");
            assert_eq!(offenses(&source).len(), count, "{source}");
        }
    }
    #[test]
    fn redefinition_warnings_including_reopened_eigenclasses_prevent_grouping() {
        for body in [
            "attr_reader :one\nattr_reader :two\nattr_reader :one",
            "attr_reader :one\nattr_reader :\"one\"",
            "attr_accessor :existing\nattr_reader :existing\nattr_reader :one\nattr_reader :two",
            "class << self\nattr_reader :existing\nend\nclass << self\nattr_reader :existing\nattr_reader :one\nattr_reader :two\nend",
        ] {
            let source = format!("class Foo\n{body}\nend\n");
            let found =
                crate::cops::testing::correction_offenses(&super::AccessorGrouping, &source);
            assert!(!found.is_empty(), "{source}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        }
    }
    #[test]
    fn sole_logical_expression_matches_only_its_immediate_children_without_rewriting() {
        for scope in ["class Foo", "module Foo", "class << self"] {
            for (body, second_col) in [
                ("attr_reader :a and attr_reader :b", 22),
                ("attr_reader :a or attr_reader :b", 21),
                ("attr_reader(:a) && attr_reader(:b)", 22),
                ("attr_reader(:a) || attr_reader(:b)", 22),
            ] {
                let source = format!("{scope}\n  {body}\nend\n");
                let found = offenses(&source);
                assert_eq!(
                    found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                    [(2, 3), (2, second_col)],
                    "{source}"
                );
                assert!(
                    found
                        .iter()
                        .all(|o| { o.message == "Group together all `attr_reader` attributes." })
                );
                let found =
                    crate::cops::testing::correction_offenses(&super::AccessorGrouping, &source);
                assert_eq!(found.len(), 2, "{source}");
                assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
            }
        }
        for body in [
            "attr_reader(:a) && (attr_reader(:b) && attr_reader(:c))",
            "attr_reader(:a) && attr_reader(:b) && attr_reader(:c)",
            "attr_reader(:a) && attr_reader(:b)\nattr_reader(:c)",
            "(attr_reader(:a) && attr_reader(:b))",
            "begin\nattr_reader(:a) && attr_reader(:b)\nend",
            "attr_reader(:a) && (class Bar; attr_reader :b; end)",
            "factory { attr_reader :a; attr_reader :b }",
        ] {
            let source = format!("class Foo\n  {body}\nend\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
    #[test]
    fn canonical_grouped_correction_is_atomic() {
        crate::cops::testing::assert_correction(
            &super::AccessorGrouping,
            "class Foo\n  attr_reader :one\n  attr_reader :two\nend\n",
            "class Foo\n  attr_reader :one, :two\nend\n",
        );
    }
}
