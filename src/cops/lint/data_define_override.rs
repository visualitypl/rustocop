//! `Lint/DataDefineOverride` — literal members overriding built-in Data methods.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/DataDefineOverride` cop (detection only).
pub struct DataDefineOverride;

// Pinned RuboCop 1.92's fixed table, based on Ruby 4.0, even at target Ruby 3.4.
// Do not derive this from the host Ruby or the broader Struct method table.
const DATA_METHOD_NAMES: &[&[u8]] = &[
    b"!",
    b"!=",
    b"!~",
    b"<=>",
    b"==",
    b"===",
    b"__id__",
    b"__send__",
    b"class",
    b"clone",
    b"deconstruct",
    b"deconstruct_keys",
    b"define_singleton_method",
    b"display",
    b"dup",
    b"enum_for",
    b"eql?",
    b"equal?",
    b"extend",
    b"freeze",
    b"frozen?",
    b"hash",
    b"inspect",
    b"instance_eval",
    b"instance_exec",
    b"instance_of?",
    b"instance_variable_defined?",
    b"instance_variable_get",
    b"instance_variable_set",
    b"instance_variables",
    b"is_a?",
    b"itself",
    b"kind_of?",
    b"members",
    b"method",
    b"methods",
    b"nil?",
    b"object_id",
    b"private_methods",
    b"protected_methods",
    b"public_method",
    b"public_methods",
    b"public_send",
    b"remove_instance_variable",
    b"respond_to?",
    b"send",
    b"singleton_class",
    b"singleton_method",
    b"singleton_methods",
    b"tap",
    b"then",
    b"to_enum",
    b"to_h",
    b"to_s",
    b"with",
    b"yield_self",
];

impl Cop for DataDefineOverride {
    fn name(&self) -> &'static str {
        "Lint/DataDefineOverride"
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.is_safe_navigation()
            || call.name().as_slice() != b"define"
            || !call
                .receiver()
                .is_some_and(|r| ast::is_global_const(&r, &["Data"]))
        {
            return;
        }
        let Some(args) = call.arguments() else {
            return;
        };
        for argument in args.arguments().iter() {
            let sym = argument.as_symbol_node();
            let string = argument.as_string_node();
            let (value, symbol) = if let Some(sym) = &sym {
                (sym.unescaped(), true)
            } else if let Some(string) = &string {
                (string.unescaped(), false)
            } else {
                continue;
            };
            if !DATA_METHOD_NAMES.contains(&value) {
                continue;
            }
            // Every table entry is an ASCII, unquoted symbol and needs no escaping.
            let name = String::from_utf8_lossy(value);
            let inspected = if symbol {
                format!(":{name}")
            } else {
                format!("\"{name}\"")
            };
            let (line, col) = file.line_col(argument.location().start_offset());
            out.push(Offense {
                correction: None,
                cop: self.name(),
                severity: self.severity(),
                message: format!(
                    "`{inspected}` member overrides `Data#{name}` and it may be unexpected."
                ),
                line,
                col,
                end_line: file.last_line_of(argument.location()),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/lint/data_define_override_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/DataDefineOverride")
            .collect()
    }

    #[test]
    fn flags_symbol_and_string_members_at_the_argument() {
        for (source, col, member) in [
            ("Bad = Data.define(:members)", 19, ":members"),
            ("Bad = ::Data.define(:members)", 21, ":members"),
            (
                "Bad = Data.define(:name, :members, :address)",
                26,
                ":members",
            ),
            ("Bad = Data.define(:name, \"members\")", 26, "\"members\""),
            ("Data.define(\"members\")", 13, "\"members\""),
            (
                "Data.define(:members) do\n  def members?\n    !members.empty?\n  end\nend",
                13,
                ":members",
            ),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, col, 1)
            );
            assert_eq!(
                found[0].message,
                format!("`{member}` member overrides `Data#members` and it may be unexpected.")
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn flags_multiple_overrides_in_source_order() {
        let found = offenses("Data.define(:members, :clone, :to_s)");
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 13), (1, 23), (1, 31)]
        );
        for (o, name) in found.iter().zip(["members", "clone", "to_s"]) {
            assert_eq!(
                o.message,
                format!("`:{name}` member overrides `Data#{name}` and it may be unexpected.")
            );
        }
    }

    #[test]
    fn ignores_no_override_members_and_receiver_or_value_lookalikes() {
        for source in [
            "Good = Data.define(:id, :name)",
            "Good = Data.define(:id, :name) do\n  def members\n    super.tap { |ret| pp \"members: \" + ret.to_s }\n  end\nend",
            "Other::Data.define(:members)",
            "Data&.define(:members)",
            "Data.new(:members)",
            "Struct.define(:members)",
            "define(:members)",
            "Data.define(members)",
            "Data.define(*%i[members])",
            "Data.define(&:members)",
            "Data.define(:\"#{name}\", \"#{name}\")",
            "Data.define(:count, :to_a, :[], :[]=)",
            "Data.define",
            "# Data.define(:members)\n'Data.define(:members)'",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
