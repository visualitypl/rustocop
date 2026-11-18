//! `Lint/DuplicateSetElement` — flags a repeated element in a set literal:
//! `Set[...]`, `Set.new([...])`, `[...].to_set` (and `SortedSet`).
//!
//! Matches RuboCop's semantics: only literal, constant and variable elements
//! are compared (structurally); method calls may return different values on
//! each call and are skipped. `::Set` counts; `Foo::Set` does not.

use std::collections::HashSet;

use ruby_prism::{CallNode, Node};

use crate::{
    cops::{
        Cop, NodeContext,
        ast::{
            fingerprint_for_duplicate, is_const, is_global_const, is_literal, is_variable,
            short_const_name,
        },
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

/// The `Lint/DuplicateSetElement` cop.
pub struct DuplicateSetElement;

const SET_CLASSES: [&str; 2] = ["Set", "SortedSet"];

/// The elements of a set built by `call`, and the class name for messages.
fn set_elements<'pr>(call: &CallNode<'pr>) -> Option<(Vec<Node<'pr>>, String)> {
    // Filter on the name first: this runs for every call in the file.
    let name = call.name().as_slice();
    if !matches!(name, b"[]" | b"new" | b"to_set") {
        return None;
    }
    let receiver = call.receiver()?;
    let arguments: Vec<Node<'pr>> = call
        .arguments()
        .map(|arguments| arguments.arguments().iter().collect())
        .unwrap_or_default();
    let block_pass = call
        .block()
        .is_some_and(|block| block.as_block_argument_node().is_some());
    let safe_navigation = call.is_safe_navigation();
    match name {
        b"[]" if !safe_navigation && is_global_const(&receiver, &SET_CLASSES) => {
            Some((arguments, short_const_name(&receiver)?))
        }
        b"new"
            if !safe_navigation
                && !block_pass
                && arguments.len() == 1
                && is_global_const(&receiver, &SET_CLASSES) =>
        {
            let array = arguments[0].as_array_node()?;
            Some((
                array.elements().iter().collect(),
                short_const_name(&receiver)?,
            ))
        }
        b"to_set" if arguments.is_empty() && !block_pass => {
            let array = receiver.as_array_node()?;
            Some((array.elements().iter().collect(), "Set".to_string()))
        }
        _ => None,
    }
}

impl Cop for DuplicateSetElement {
    fn name(&self) -> &'static str {
        "Lint/DuplicateSetElement"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
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
        let Some((elements, class_name)) = set_elements(&call) else {
            return;
        };
        let mut seen = HashSet::new();
        for (index, element) in elements.iter().enumerate() {
            if !is_literal(element) && !is_const(element) && !is_variable(element) {
                continue;
            }
            if seen.insert(fingerprint_for_duplicate(element, file)) {
                continue;
            }
            let (line, col) = super::array_literal_in_regexp::reference_line_col(
                element.location().start_offset(),
                file,
            );
            let correction = cx.corrections_requested().then(|| Correction {
                range: elements[index - 1].location().end_offset()..element.location().end_offset(),
                replacement: String::new(),
            });
            out.push(Offense {
                correction,
                cop: self.name(),
                severity: self.severity(),
                message: format!("Remove the duplicate element in {class_name}."),
                line,
                end_line: file.last_line_of(element.location()),
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/duplicate_set_element_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/DuplicateSetElement")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    /// Shift for the class name length relative to `Set`.
    fn shift(class: &str) -> u32 {
        class.len() as u32 - 3
    }

    #[test]
    fn flags_duplicate_symbol_in_brackets_with_message() {
        for class in SET_CLASSES {
            let found = offenses(&format!("{class}[:foo, :bar, :foo]\n"));
            assert_eq!(found.len(), 1, "{class}");
            assert_eq!((found[0].line, found[0].col), (1, 17 + shift(class)));
            assert_eq!(
                found[0].message,
                format!("Remove the duplicate element in {class}.")
            );
        }
    }

    #[test]
    fn flags_duplicate_symbol_in_top_level_constant_brackets() {
        for class in SET_CLASSES {
            let found = offenses(&format!("::{class}[:foo, :bar, :foo]\n"));
            assert_eq!(found.len(), 1, "{class}");
            assert_eq!((found[0].line, found[0].col), (1, 19 + shift(class)));
            assert_eq!(
                found[0].message,
                format!("Remove the duplicate element in {class}.")
            );
        }
    }

    #[test]
    fn flags_multiple_duplicate_symbols() {
        for class in SET_CLASSES {
            assert_eq!(
                positions(&format!("{class}[:foo, :bar, :foo, :baz, :baz]\n")),
                vec![(1, 17 + shift(class)), (1, 29 + shift(class))]
            );
        }
    }

    #[test]
    fn flags_duplicate_variables_and_constants() {
        for class in SET_CLASSES {
            let s = shift(class);
            assert_eq!(
                positions(&format!(
                    "foo = do_foo\nbar = do_bar\n\n{class}[foo, bar, foo]\n"
                )),
                vec![(4, 15 + s)],
                "lvar in {class}"
            );
            assert_eq!(
                positions(&format!("{class}[@foo, @bar, @foo]\n")),
                vec![(1, 17 + s)],
                "ivar in {class}"
            );
            assert_eq!(
                positions(&format!("{class}[Foo, Bar, Foo]\n")),
                vec![(1, 15 + s)],
                "const in {class}"
            );
        }
    }

    #[test]
    fn accepts_duplicate_method_calls() {
        for class in SET_CLASSES {
            for source in [
                format!("{class}[foo, bar, foo]\n"),
                format!("{class}[obj&.foo, obj&.bar, obj&.foo]\n"),
                format!("{class}[rand > 0.5 ? 1 : 2, rand > 0.5 ? 1 : 2]\n"),
            ] {
                assert!(offenses(&source).is_empty(), "should not flag {source}");
            }
        }
    }

    #[test]
    fn flags_duplicates_in_new_with_array() {
        for class in SET_CLASSES {
            let s = shift(class);
            assert_eq!(
                positions(&format!("{class}.new([:foo, :bar, :foo])\n")),
                vec![(1, 22 + s)]
            );
            assert_eq!(
                positions(&format!("{class}.new(%i[foo bar foo])\n")),
                vec![(1, 20 + s)]
            );
            let found = offenses(&format!("::{class}.new([:foo, :bar, :foo])\n"));
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                vec![(1, 24 + s)]
            );
            assert_eq!(
                found[0].message,
                format!("Remove the duplicate element in {class}.")
            );
        }
    }

    #[test]
    fn accepts_sets_without_duplicates() {
        for class in SET_CLASSES {
            for source in [
                format!("{class}[foo, bar]\n"),
                format!("{class}.new([:foo, :bar])\n"),
                format!("{class}[]\n"),
                format!("{class}[:foo]\n"),
            ] {
                assert!(offenses(&source).is_empty(), "should not flag {source}");
            }
        }
    }

    #[test]
    fn flags_duplicates_in_to_set() {
        let found = offenses("[:foo, :bar, :foo].to_set\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 14));
        assert_eq!(found[0].message, "Remove the duplicate element in Set.");
        assert_eq!(positions("[:foo, :bar, :foo]&.to_set\n"), vec![(1, 14)]);
    }

    #[test]
    fn accepts_array_constructors() {
        assert!(offenses("Array[:foo, :bar, :foo]\n").is_empty());
        assert!(offenses("Array.new([:foo, :bar, :foo])\n").is_empty());
    }

    #[test]
    fn flags_literal_spellings_and_literal_kinds() {
        assert_eq!(positions("Set['a', \"a\"]\n"), vec![(1, 10)]);
        assert_eq!(positions("Set[1, 2, 1]\n"), vec![(1, 11)]);
        assert_eq!(positions("Set[nil, nil]\n"), vec![(1, 10)]);
        assert_eq!(positions("Set[[a], [a]]\n"), vec![(1, 10)]);
    }

    #[test]
    fn complex_hash_keys_preserve_component_types_and_normalize_zero() {
        for (a, b, duplicate) in [
            ("1i", "1ri", false),
            ("1i", "1.0i", false),
            ("0.0i", "-0.0i", true),
            ("1ri", "1.0ri", true),
            ("[1i]", "[1ri]", false),
            ("[0.0i]", "[-0.0i]", true),
        ] {
            let source = format!("Set[\n{a},\n{b}\n]");
            assert_eq!(
                positions(&source),
                if duplicate { vec![(3, 1)] } else { vec![] },
                "{source}"
            );
        }
    }

    #[test]
    fn normalizes_source_buffer_recursively_in_compound_keys() {
        for (a, b, duplicate) in [
            ("[/e\u{301}/im]", "[/é/mi]", true),
            ("[[/e\u{301}/]]", "[[/é/]]", true),
            ("\"#{/e\u{301}/}\"", "\"#{/é/}\"", true),
            ("[e\u{301}.f]", "[é.f]", true),
            ("[/e\u{301}/i]", "[/é/m]", false),
            ("[/e\u{301}/]", "[/è/]", false),
        ] {
            let body = format!("Set[\n{a},\n{b}\n]");
            assert_eq!(
                positions(&format!("# coding: UTF8-MAC\n{body}")),
                if duplicate { vec![(4, 1)] } else { vec![] },
                "{body}"
            );
            assert!(
                offenses(&body).is_empty(),
                "UTF-8 must retain distinct spellings: {body}"
            );
        }
    }

    #[test]
    fn compares_parser_literal_kinds_and_evaluated_source_lines() {
        for source in ["Set[:\"\", :\"\\u{}\"]", "Set[:\"\\u{}\", :\"\"]"] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_eq!(positions("Set[__LINE__, __LINE__]"), vec![(1, 15)]);
        assert!(offenses("Set[__LINE__,\n__LINE__]").is_empty());
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "Foo::Set[:a, :a]\n",
            "Set[:a, 'a']\n",
            "Set[1, 1.0]\n",
            "Set.new([:a, :b], [:a])\n",
            "Set.new(:a, :a)\n",
            "[:a, :a].to_a\n",
            "[:a, :a].to_set(SortedSet)\n",
            "Set[*a, *a]\n",
            "x = [:a, :a]\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
