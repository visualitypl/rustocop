//! `Naming/AccessorMethodName`: accessor prefixes at the pinned arities.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct AccessorMethodName;
impl Cop for AccessorMethodName {
    fn name(&self) -> &'static str {
        "Naming/AccessorMethodName"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_def_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(def) = node.as_def_node() else {
            return;
        };
        let name = def.name().as_slice();
        if name.last().is_some_and(|b| matches!(b, b'!' | b'?' | b'=')) {
            return;
        }
        let parameters = def.parameters();
        let ordinary_only = parameters.as_ref().is_none_or(|p| {
            p.optionals().is_empty()
                && p.rest().is_none()
                && p.posts().is_empty()
                && p.keywords().is_empty()
                && p.keyword_rest().is_none()
                && p.block().is_none()
        });
        let kind = if name.starts_with(b"get_")
            && ordinary_only
            && parameters.as_ref().is_none_or(|p| p.requireds().is_empty())
        {
            "reader"
        } else if name.starts_with(b"set_")
            && ordinary_only
            && parameters.as_ref().is_some_and(|p| {
                p.requireds().len() == 1
                    && p.requireds()
                        .iter()
                        .next()
                        .unwrap()
                        .as_required_parameter_node()
                        .is_some()
            })
        {
            "writer"
        } else {
            return;
        };
        let prefix = if kind == "reader" { "get_" } else { "set_" };
        let (line, col) = file.line_col(def.name_loc().start_offset());
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: format!("Do not prefix {kind} method names with `{prefix}`."),
            line,
            col,
            end_line: line,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop naming/accessor_method_name_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Naming/AccessorMethodName")
            .collect()
    }
    #[test]
    fn flags_only_expected_accessor_arities_at_the_method_name() {
        for (name, args, kind) in [
            ("get_something", "", "reader"),
            ("get_something", "()", "reader"),
            ("set_something", "(arg)", "writer"),
        ] {
            for (receiver, col) in [("", 5), ("self.", 10), ("object.", 12)] {
                let source = format!("# accessor\ndef {receiver}{name}{args}\nend\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (2, col));
                let prefix = if kind == "reader" { "get_" } else { "set_" };
                assert_eq!(
                    found[0].message,
                    format!("Do not prefix {kind} method names with `{prefix}`.")
                );
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn accepts_other_parameter_forms_suffixes_and_lookalikes() {
        for args in [
            "",
            "()",
            "(a, b)",
            "(a = nil)",
            "(*a)",
            "(k:)",
            "(k: nil)",
            "(**k)",
            "(&b)",
            "(...)",
            "((a, b))",
            "(a, &b)",
            "(a, **nil)",
            "(*)",
            "(**)",
            "(&)",
        ] {
            assert!(
                offenses(&format!("def set_value{args}; end")).is_empty(),
                "{args}"
            );
        }
        for args in ["(arg)", "(arg = nil)", "(*)", "(**nil)", "(&)", "(...)"] {
            assert!(
                offenses(&format!("def get_value{args}; end")).is_empty(),
                "{args}"
            );
        }
        for source in [
            "def get_value!; end",
            "def get_value?; end",
            "def get_value=; end",
            "def set_value!(x); end",
            "def set_value?(x); end",
            "def set_value=(x); end",
            "def forget_value; end",
            "define_method(:get_value) {}",
            "get_value",
            "alias get_value value",
            "# def get_value\n'def set_value(x)'",
            "def get; end",
            "def set(x); end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
