//! `Naming/ClassAndModuleCamelCase` — flags class and module names with an
//! underscore in them.
//!
//! Matches RuboCop's semantics at its default `AllowedNames`
//! (`module_parent`): the written name, namespace included (`Top::My_Class`,
//! `module_parent::MyClass`), is checked after removing every
//! `module_parent`, and the offense covers the whole name. Singleton classes
//! (`class << obj`) are not checked.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The `Naming/ClassAndModuleCamelCase` cop.
pub struct ClassAndModuleCamelCase;

/// RuboCop's default `AllowedNames`: removed from the name before checking.
const ALLOWED_NAME: &[u8] = b"module_parent";

impl Cop for ClassAndModuleCamelCase {
    fn name(&self) -> &'static str {
        "Naming/ClassAndModuleCamelCase"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let constant_path = if let Some(class) = node.as_class_node() {
            class.constant_path()
        } else if let Some(module) = node.as_module_node() {
            module.constant_path()
        } else {
            return;
        };
        let location = constant_path.location();
        if !has_disallowed_underscore(location.as_slice()) {
            return;
        }
        let (line, col) = file.line_col(location.start_offset());
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: "Use CamelCase for classes and modules.".to_string(),
            line,
            end_line: file.last_line_of(location),
            col,
        });
    }
}

/// Whether `name` still holds an underscore once every `module_parent` is
/// removed (RuboCop's `gsub` then `include?('_')`).
fn has_disallowed_underscore(name: &[u8]) -> bool {
    let mut rest = name;
    while let Some((&first, tail)) = rest.split_first() {
        if rest.starts_with(ALLOWED_NAME) {
            rest = &rest[ALLOWED_NAME.len()..];
        } else if first == b'_' {
            return true;
        } else {
            rest = tail;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/naming/class_and_module_camel_case_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for the default
    //! `AllowedNames`.

    use crate::pipeline::lint_str;

    const MESSAGE: &str = "Use CamelCase for classes and modules.";

    fn found(source: &str) -> Vec<(u32, u32, String)> {
        let parsed = ruby_prism::parse(source.as_bytes());
        assert!(
            parsed.errors().next().is_none(),
            "fixture must parse: {source:?}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Naming/ClassAndModuleCamelCase")
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    fn at(line: u32, col: u32) -> (u32, u32, String) {
        (line, col, MESSAGE.to_string())
    }

    #[test]
    fn flags_underscore_in_class_and_module_names() {
        assert_eq!(
            found("class My_Class\nend\n\nmodule My_Module\nend\n"),
            [at(1, 7), at(4, 8)]
        );
    }

    #[test]
    fn is_not_fooled_by_qualified_names() {
        assert_eq!(
            found("class Top::My_Class\nend\n\nmodule My_Module::Ala\nend\n"),
            [at(1, 7), at(4, 8)]
        );
        assert_eq!(found("class ::My_Class; end\n"), [at(1, 7)]);
        assert_eq!(found("module Foo::Bar::Baz_Qux; end\n"), [at(1, 8)]);
    }

    #[test]
    fn checks_nested_definitions() {
        assert_eq!(
            found("module Outer\n  class In_Ner < Base\n  end\nend\n"),
            [at(2, 9)]
        );
    }

    #[test]
    fn module_parent_is_allowed_anywhere_in_the_name() {
        assert!(found("class module_parent::MyClass\nend\n").is_empty());
        assert!(found("class Foo::module_parent::MyClass\nend\n").is_empty());
        // Only the allowed part is removed; another underscore still counts.
        assert_eq!(found("class module_parent::My_Class\nend\n"), [at(1, 7)]);
    }

    #[test]
    fn accepts_camel_case_names() {
        assert!(found("class MyClass\nend\n\nmodule Mine\nend\n").is_empty());
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "class Foo < My_Base\nend\n",
            "class Foo\n  MY_CONST = 1\n  def my_method; end\nend\n",
            "class << my_object\nend\n",
            "My_Class = Class.new\n",
            "Struct.new(:my_field)\n",
            "foo::My_Thing\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }
}
