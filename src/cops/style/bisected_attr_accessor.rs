//! `Style/BisectedAttrAccessor` — combine matching readers and writers.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct BisectedAttrAccessor;
impl Cop for BisectedAttrAccessor {
    fn name(&self) -> &'static str {
        "Style/BisectedAttrAccessor"
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
        let mut macros = Vec::new();
        let mut names = std::collections::HashMap::new();
        let mut visibility = b"public".to_vec();
        for (i, child) in children.iter().enumerate() {
            let Some(call) = child.as_call_node() else {
                continue;
            };
            if let Some(v) = super::accessors::visibility(&call) {
                visibility = v.to_vec();
            }
            if call.is_safe_navigation() || super::accessors::literal_block(&call) {
                continue;
            }
            let bit = match call.name().as_slice() {
                b"attr_reader" | b"attr" => 1u8,
                b"attr_writer" => 2,
                _ => continue,
            };
            let args = crate::cops::ast::call_arguments(&call);
            // Upstream keys names by source, not decoded symbol value. Repeated
            // source keys retain only their last argument's offense position.
            let mut last = std::collections::HashMap::new();
            for (n, arg) in args.iter().enumerate() {
                let text = arg.location().as_slice().to_vec();
                *names
                    .entry((visibility.clone(), text.clone()))
                    .or_insert(0u8) |= bit;
                last.insert(text, n);
            }
            macros.push((i, visibility.clone(), bit, args, last));
        }
        for (i, visibility, bit, args, last) in macros {
            let correction = if bit == 1
                && cx.corrections_requested()
                && cx
                    .ancestors()
                    .last()
                    .is_some_and(|root| file.accessor_bindings(root).contains(&children[i]))
            {
                children.get(i + 1).and_then(|next| {
                    let call = next.as_call_node()?;
                    let writer_args = crate::cops::ast::call_arguments(&call);
                    if call.name().as_slice() != b"attr_writer"
                        || args.len() != writer_args.len()
                        || !args
                            .iter()
                            .zip(&writer_args)
                            .all(|(a, b)| a.location().as_slice() == b.location().as_slice())
                    {
                        return None;
                    }
                    let text = args
                        .iter()
                        .map(|a| String::from_utf8_lossy(a.location().as_slice()).into_owned())
                        .collect::<Vec<_>>()
                        .join(", ");
                    super::accessors::rewrite(
                        file,
                        &[&children[i], next],
                        &format!("attr_accessor {text}"),
                    )
                })
            } else {
                None
            };
            let mut correction = correction;
            for (n, arg) in args.iter().enumerate() {
                let text = arg.location().as_slice();
                if last.get(text) != Some(&n)
                    || names.get(&(visibility.clone(), text.to_vec())) != Some(&3)
                {
                    continue;
                }
                let (line, col) = file.line_col(arg.location().start_offset());
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    message: format!(
                        "Combine both accessors into `attr_accessor {}`.",
                        String::from_utf8_lossy(text)
                    ),
                    line,
                    col,
                    end_line: file.last_line_of(arg.location()),
                    correction: correction.take(),
                });
            }
        }
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/bisected_attr_accessor_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/BisectedAttrAccessor")
            .collect()
    }
    #[test]
    fn exact_argument_positions_and_message() {
        for reader in ["attr_reader", "attr"] {
            let s = format!("class Foo\n  {reader} :bar\n  attr_writer :bar\nend\n");
            let found = offenses(&s);
            assert_eq!(found.len(), 2, "{s}");
            assert_eq!((found[0].line, found[0].col), (2, reader.len() as u32 + 4));
            assert_eq!((found[1].line, found[1].col), (3, 15));
            assert!(
                found
                    .iter()
                    .all(|o| o.message == "Combine both accessors into `attr_accessor :bar`.")
            );
        }
    }
    #[test]
    fn splats_partial_families_and_reentered_visibility() {
        for (body, count) in [
            ("attr_reader *ATTRIBUTES\nattr_writer *ATTRIBUTES", 2),
            ("attr_reader :baz, :bar, :quux\nattr_writer :bar, :zoo", 2),
            ("attr_reader :foo, :bar, :baz\nattr_writer :foo, :baz", 4),
            ("attr_writer :foo\nattr_reader :foo", 2),
            (
                "attr_reader :foo\nprivate\nattr_writer :bar\npublic\nattr_writer :foo",
                2,
            ),
        ] {
            assert_eq!(
                offenses(&format!("module Foo\n{body}\nend")).len(),
                count,
                "{body}"
            );
        }
    }
    #[test]
    fn different_visibility_scopes_and_indirect_bodies_are_not_combined() {
        for body in [
            "attr_reader :bar\nprivate\nattr_writer :bar",
            "attr_reader :bar\nattr_writer :baz",
            "attr_accessor :bar",
            "attr_reader :bar\ndef f; attr_writer :bar; end",
            "attr_reader :bar\nclass << self; attr_writer :bar; end",
        ] {
            assert!(
                offenses(&format!("class Foo\n{body}\nend")).is_empty(),
                "{body}"
            );
        }
    }
    #[test]
    fn receivers_blocks_dynamic_sources_and_duplicate_argument_locations() {
        let s = "class Foo\n  Other.attr_reader :a\n  self.attr_writer :a\n  attr_reader :b, :b\n  attr_writer :b\n  attr_reader :\"a\"\n  attr_writer :a\nend\n";
        let found = offenses(s);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(2, 21), (3, 20), (4, 19), (5, 15), (7, 15)]
        );
        for s in [
            "class Foo; attr_reader(:a) {}; attr_writer :a; end",
            "class Foo; self&.attr_reader :a; attr_writer :a; end",
            "class Foo; attr_reader :a; attr_writer :\"a\"; end",
            "class Foo; private attr_reader :a; attr_writer :a; end",
            "attr_reader :a\nattr_writer :a",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn pinned_module_visibility_and_eigenclass_canonical_pairs() {
        for body in [
            "  attr_reader :a\n  attr_writer :a\n\n  private\n  attr_reader :b\n  attr_writer :b",
            "  class << self\n    attr_reader :baz\n    attr_writer :baz\n    private\n    attr_reader :quux\n  end",
        ] {
            let after = body
                .replace("attr_reader :a\n  attr_writer :a", "attr_accessor :a")
                .replace("attr_reader :b\n  attr_writer :b", "attr_accessor :b")
                .replace(
                    "attr_reader :baz\n    attr_writer :baz",
                    "attr_accessor :baz",
                );
            for kind in ["class", "module"] {
                crate::cops::testing::assert_correction(
                    &super::BisectedAttrAccessor,
                    &format!("{kind} Foo\n{body}\nend\n"),
                    &format!("{kind} Foo\n{after}\nend\n"),
                );
            }
        }
    }
    #[test]
    fn individually_withheld_pinned_corrections() {
        for body in [
            "attr_reader :bar\nattr_writer :bar\nother_macro :something",
            "attr :bar\nattr_writer :bar\nother_macro :something",
            "ATTRIBUTES = %i[foo bar]\nattr_reader *ATTRIBUTES\nattr_writer *ATTRIBUTES\nother_macro :something",
            "attr_reader :baz, :bar, :quux\nattr_writer :bar, :zoo\nother_macro :something",
            "attr_writer :foo\nattr_reader :foo\nother_macro :something",
            "attr_reader :foo, :bar, :baz\nattr_writer :foo, :baz",
            "attr_reader :foo\nprivate\nattr_writer :bar\npublic\nattr_writer :foo",
            "attr_reader :foo\nattr_writer :foo, :bar\nprivate\nattr_reader :bar, :baz\nattr_writer :baz",
        ] {
            let source = format!("class Foo\n{body}\nend\n");
            let found =
                crate::cops::testing::correction_offenses(&super::BisectedAttrAccessor, &source);
            assert!(!found.is_empty(), "{source}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        }
        // The pinned mixed-direction visibility case corrects the public pair
        // only; private writer-first ordering is intentionally preserved.
        crate::cops::testing::assert_correction(
            &super::BisectedAttrAccessor,
            "class Foo\n  attr_reader :bar\n  attr_writer :bar\n  private\n  attr_writer :baz\n  attr_reader :baz\nend\n",
            "class Foo\n  attr_accessor :bar\n  private\n  attr_writer :baz\n  attr_reader :baz\nend\n",
        );
    }
    #[test]
    fn explicit_begin_body_keeps_argument_locations() {
        let found = offenses("class Foo\nbegin\nattr_reader :a\nattr_writer :a\nend\nend");
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(3, 13), (4, 13)]
        );
    }
    #[test]
    fn sole_logical_expression_matches_only_its_immediate_children_without_rewriting() {
        for scope in ["class Foo", "module Foo", "class << self"] {
            for (body, first_col, second_col) in [
                ("attr_reader(:a) || attr_writer(:a)", 15, 34),
                ("attr_reader(:a) && attr_writer(:a)", 15, 34),
                ("attr_reader :a and attr_writer :a", 15, 34),
                ("attr_reader :a or attr_writer :a", 15, 33),
            ] {
                let source = format!("{scope}\n  {body}\nend\n");
                let found = offenses(&source);
                assert_eq!(
                    found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                    [(2, first_col), (2, second_col)],
                    "{source}"
                );
                assert!(
                    found.iter().all(|o| {
                        o.message == "Combine both accessors into `attr_accessor :a`."
                    })
                );
                let found = crate::cops::testing::correction_offenses(
                    &super::BisectedAttrAccessor,
                    &source,
                );
                assert_eq!(found.len(), 2, "{source}");
                assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
            }
        }
        for body in [
            "attr_reader(:a) || (attr_writer(:a) || attr_writer(:b))",
            "attr_reader(:a) || attr_writer(:b) || attr_writer(:a)",
            "attr_reader(:a) || attr_writer(:a)\nattr_writer(:b)",
            "(attr_reader(:a) || attr_writer(:a))",
            "begin\nattr_reader(:a) || attr_writer(:a)\nend",
            "attr_reader(:a) || (class Bar; attr_writer :a; end)",
            "factory { attr_reader :a; attr_writer :a }",
        ] {
            let source = format!("class Foo\n  {body}\nend\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
    #[test]
    fn canonical_adjacent_complete_pairs_correct_atomically() {
        for reader in ["attr_reader", "attr"] {
            for names in [":bar", ":foo, :bar, :baz"] {
                crate::cops::testing::assert_correction(
                    &super::BisectedAttrAccessor,
                    &format!("class Foo\n  {reader} {names}\n  attr_writer {names}\nend\n"),
                    &format!("class Foo\n  attr_accessor {names}\nend\n"),
                );
            }
        }
    }
}
