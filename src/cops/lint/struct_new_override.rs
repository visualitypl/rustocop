//! `Lint/StructNewOverride` — flags `Struct.new` members whose accessors
//! would override a built-in `Struct` method (`members`, `count`, `clone`,
//! ...).
//!
//! Matches RuboCop's semantics: a `Struct.new` or `::Struct.new` call (not
//! `&.`) is checked argument by argument; plain symbols and strings are
//! member names, except a leading string, which names the class. The method
//! list is `Struct.instance_methods` of Ruby 4.0, as in RuboCop 1.92. The
//! offense is reported at the member argument.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, ast::is_global_const},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/StructNewOverride` cop.
pub struct StructNewOverride;

/// `Struct.instance_methods.sort` on Ruby 4.0.0 (RuboCop's list).
const STRUCT_METHOD_NAMES: [&str; 124] = [
    "!",
    "!=",
    "!~",
    "<=>",
    "==",
    "===",
    "[]",
    "[]=",
    "__id__",
    "__send__",
    "all?",
    "any?",
    "chain",
    "chunk",
    "chunk_while",
    "class",
    "clone",
    "collect",
    "collect_concat",
    "compact",
    "count",
    "cycle",
    "deconstruct",
    "deconstruct_keys",
    "define_singleton_method",
    "detect",
    "dig",
    "display",
    "drop",
    "drop_while",
    "dup",
    "each",
    "each_cons",
    "each_entry",
    "each_pair",
    "each_slice",
    "each_with_index",
    "each_with_object",
    "entries",
    "enum_for",
    "eql?",
    "equal?",
    "extend",
    "filter",
    "filter_map",
    "find",
    "find_all",
    "find_index",
    "first",
    "flat_map",
    "freeze",
    "frozen?",
    "grep",
    "grep_v",
    "group_by",
    "hash",
    "include?",
    "inject",
    "inspect",
    "instance_eval",
    "instance_exec",
    "instance_of?",
    "instance_variable_defined?",
    "instance_variable_get",
    "instance_variable_set",
    "instance_variables",
    "is_a?",
    "itself",
    "kind_of?",
    "lazy",
    "length",
    "map",
    "max",
    "max_by",
    "member?",
    "members",
    "method",
    "methods",
    "min",
    "min_by",
    "minmax",
    "minmax_by",
    "nil?",
    "none?",
    "object_id",
    "one?",
    "partition",
    "private_methods",
    "protected_methods",
    "public_method",
    "public_methods",
    "public_send",
    "reduce",
    "reject",
    "remove_instance_variable",
    "respond_to?",
    "reverse_each",
    "select",
    "send",
    "singleton_class",
    "singleton_method",
    "singleton_methods",
    "size",
    "slice_after",
    "slice_before",
    "slice_when",
    "sort",
    "sort_by",
    "sum",
    "take",
    "take_while",
    "tally",
    "tap",
    "then",
    "to_a",
    "to_enum",
    "to_h",
    "to_s",
    "to_set",
    "uniq",
    "values",
    "values_at",
    "yield_self",
    "zip",
];

impl Cop for StructNewOverride {
    fn name(&self) -> &'static str {
        "Lint/StructNewOverride"
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
        if call.name().as_slice() != b"new"
            || call.is_safe_navigation()
            || !call
                .receiver()
                .is_some_and(|receiver| is_global_const(&receiver, &["Struct"]))
        {
            return;
        }
        let Some(arguments) = call.arguments() else {
            return;
        };
        for (index, argument) in arguments.arguments().iter().enumerate() {
            let (member, inspected) = if let Some(symbol) = argument.as_symbol_node() {
                let member = String::from_utf8_lossy(symbol.unescaped()).into_owned();
                let inspected = format!(":{member}");
                (member, inspected)
            } else if let Some(string) = argument.as_string_node() {
                // A leading string names the struct class.
                if index == 0 {
                    continue;
                }
                let member = String::from_utf8_lossy(string.unescaped()).into_owned();
                let inspected = format!("\"{member}\"");
                (member, inspected)
            } else {
                continue;
            };
            // Every listed name is a plain symbol literal and needs no
            // escaping inside double quotes, so `inspect` is a prefix or
            // quotes around it.
            if !STRUCT_METHOD_NAMES.contains(&member.as_str()) {
                continue;
            }
            let (line, col) = super::array_literal_in_regexp::reference_line_col(
                argument.location().start_offset(),
                file,
            );
            out.push(Offense {
                correction: None,
                cop: self.name(),
                severity: self.severity(),
                message: format!(
                    "`{inspected}` member overrides `Struct#{member}` and it may be unexpected."
                ),
                line,
                end_line: file.last_line_of(argument.location()),
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/struct_new_override_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/StructNewOverride")
            .collect()
    }

    fn found(source: &str) -> Vec<(u32, u32, String)> {
        offenses(source)
            .into_iter()
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    fn at(line: u32, col: u32, member: &str, method: &str) -> (u32, u32, String) {
        (
            line,
            col,
            format!("`{member}` member overrides `Struct#{method}` and it may be unexpected."),
        )
    }

    #[test]
    fn flags_symbol_member() {
        assert_eq!(
            found("Bad = Struct.new(:members)\n"),
            [at(1, 18, ":members", "members")]
        );
        assert_eq!(
            found("Bad = ::Struct.new(:members)\n"),
            [at(1, 20, ":members", "members")]
        );
    }

    #[test]
    fn skips_a_leading_class_name_string() {
        assert_eq!(
            found("Struct.new('Bad', :members, :name)\n"),
            [at(1, 19, ":members", "members")]
        );
        // Only the first argument can name the class.
        assert_eq!(
            found("Bad = Struct.new(:name, \"members\")\n"),
            [at(1, 25, "\"members\"", "members")]
        );
        assert!(found("Struct.new('members')\n").is_empty());
    }

    #[test]
    fn flags_rubocop_spec_offenses() {
        assert_eq!(
            found("Bad = Struct.new(:name, :members, :address)\n"),
            [at(1, 25, ":members", "members")]
        );
        assert_eq!(
            found("Struct.new(:members) do\n  def members?\n    !members.empty?\n  end\nend\n"),
            [at(1, 12, ":members", "members")]
        );
        assert_eq!(
            found("Struct.new(:members, :clone, :zip)\n"),
            [
                at(1, 12, ":members", "members"),
                at(1, 22, ":clone", "clone"),
                at(1, 30, ":zip", "zip"),
            ]
        );
        assert_eq!(
            found("Struct.new(:members, keyword_init: true)\n"),
            [at(1, 12, ":members", "members")]
        );
    }

    #[test]
    fn flags_operator_and_predicate_members() {
        assert_eq!(
            found("Struct.new(:==, :[]=, :nil?, '!')\n"),
            [
                at(1, 12, ":==", "=="),
                at(1, 17, ":[]=", "[]="),
                at(1, 23, ":nil?", "nil?"),
                at(1, 30, "\"!\"", "!"),
            ]
        );
        assert_eq!(
            found("Struct.new(:\"count\", %s(size))\n"),
            [at(1, 12, ":count", "count"), at(1, 22, ":size", "size")]
        );
    }

    #[test]
    fn accepts_rubocop_spec_no_offenses() {
        for source in [
            "Good = Struct.new(:id, :name)\n",
            "Good = Struct.new(:id, :name) do\n  def members\n    super.tap { |ret| pp \"members: \" + ret.to_s }\n  end\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "Foo::Struct.new(:members)\n",
            "Struct&.new(:members)\n",
            "Struct.build(:members)\n",
            "Data.define(:members)\n",
            "Struct.new(:\"#{name}\", :\"members#{x}\")\n",
            "Struct.new(members)\n",
            "Struct.new(*%i[members])\n",
            "Struct.new(:member, :memberz, :to_str)\n",
            "Struct.new(&:members)\n",
            "Struct.new\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
