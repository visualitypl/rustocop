//! `Style/HashSlice` — Ruby >= 2.5, unsafe detection only.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct HashSlice;
impl Cop for HashSlice {
    fn name(&self) -> &'static str {
        "Style/HashSlice"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        super::hash_except::check_subset(self, super::hash_except::Subset::Slice, node, file, out);
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop `spec/rubocop/cop/style/hash_slice_spec.rb` and
    //! HashSubset (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    //! Fixed ActiveSupport=false; upstream unsafe edits are never applied.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        let parsed = crate::parser::parse(s.as_bytes());
        assert!(parsed.errors().next().is_none(), "invalid fixture {s}");
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/HashSlice")
            .collect()
    }
    fn check(s: &str, keys: &str, line: u32, col: u32, end_line: u32) {
        let o = offenses(s);
        assert_eq!(o.len(), 1, "{s}");
        assert_eq!(
            (o[0].line, o[0].col, o[0].end_line),
            (line, col, end_line),
            "{s}"
        );
        assert_eq!(o[0].message, format!("Use `slice({keys})` instead."), "{s}");
        assert!(o[0].correction.is_none());
    }
    #[test]
    fn pinned_comparators_locations_and_navigation() {
        for (method, op) in [("select", "=="), ("reject", "!="), ("filter", "==")] {
            for body in [format!("k {op} :bar"), format!(":bar {op} k")] {
                check(
                    &format!("{{foo: 1, bar: 2, baz: 3}}.{method} {{ |k,v| {body} }}"),
                    ":bar",
                    1,
                    26,
                    1,
                );
            }
        }
        check(
            "{foo: 1, bar: 2, baz: 3}&.select { |k,v| k == :bar }",
            ":bar",
            1,
            27,
            1,
        );
        check("hash.select { |k,v| k == 'str' }", "'str'", 1, 6, 1);
        check("hash.select { |k,v| k.eql?(0.0) }", "0.0", 1, 6, 1);
        check("hash.select do |k,v|\n k == :bar\nend", ":bar", 1, 6, 3);
    }
    #[test]
    fn pinned_membership_percent_arrays_and_dynamic_keys() {
        for (method, bang) in [("select", ""), ("reject", "!"), ("filter", "")] {
            for (array, keys) in [
                ("[:foo, :bar]", ":foo, :bar"),
                ("[:bar]", ":bar"),
                ("%i[foo bar]", ":foo, :bar"),
                ("%I[#{foo} bar]", ":\"#{foo}\", :bar"),
                ("%W[#{foo} bar]", "\"#{foo}\", 'bar'"),
                ("%w[a'b cd]", "'a\\'b', 'cd'"),
                ("%w[back\\\\slash]", "'back\\\\slash'"),
                ("array", "*array"),
                ("[]", ""),
            ] {
                check(
                    &format!("hash.{method} {{ |k,v| {bang}{array}.include?(k) }}"),
                    keys,
                    1,
                    6,
                    1,
                );
            }
        }
        check(
            "array = [:foo]; hash.select { |k,v| array.include?(k) }",
            "*array",
            1,
            22,
            1,
        );
    }
    #[test]
    fn pinned_negative_comparators_mutation_and_key_lookalikes() {
        for s in [
            "hash.select { |k,v| k == 0.0 }",
            "hash.reject { |k,v| k != 0.0 }",
            "hash.reject { |k,v| !(k == 0.0) }",
            "hash.select { |k,v| !(k != 0.0) }",
            "hash.delete_if { |k,v| k == :bar }",
            "hash.keep_if { |k,v| k != :bar }",
            "hash.select { |k,v| v.eql?(:bar) }",
            "hash.select { |k,v| k != :bar }",
            "hash.select { |k,v| :bar != key }",
            "hash.reject { |k,v| k == :bar }",
            "hash.reject { |k,v| :bar == key }",
            "hash.select { |k,v| do_something != :bar }",
            "hash.select",
            "hash.slice(:bar)",
            "hash.select { |k,v| k == :\"#{effect}\" }",
            "hash.reject { |k,v| k != \"#{effect}\" }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn pinned_negative_membership_ranges_value_variable_and_active_support() {
        for s in [
            "hash.select { |k,v| !%i[foo bar].include?(k) }",
            "array = %i[foo bar]; hash.select { |k,v| !array.include?(k) }",
            "hash.select { |k,v| !array.include?(k) }",
            "hash.select { |k,v| ![1,2].include?(v) }",
            "hash.select { |k,v| k.include?('oo') }",
            "hash.select { |k,v| !k.include?('oo') }",
            "hash.select { |k,v| (:baa..:bbb).include?(k) }",
            "hash.select { |k,v| (:baa...:bbb).include?(k) }",
            "hash.select { |k,v| v.include?(k) }",
            "hash.select { |k,v| %i[foo bar].include? }",
            "hash.select { |k,v| k.in?(%i[foo bar]) }",
            "hash&.select { |k,v| k.in?(%i[foo bar]) }",
            "hash.select { |k,v| %i[foo bar].in?(k) }",
            "hash.select { |k,v| k.in?(v) }",
            "hash.select { |k,v| !%i[foo bar].exclude?(k) }",
            "hash&.select { |k,v| !%i[foo bar].exclude?(k) }",
            "hash.select { |k,v| k.exclude?('oo') }",
            "hash.select { |k,v| !k.exclude?('oo') }",
            "hash.select { |k,v| !v.exclude?(k) }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn exactly_two_plain_parameters_not_numbered_it_or_wrappers() {
        for p in [
            "k", "k,v,z", "k,*v", "*k,v", "(k,z),v", "k,(v,z)", "k,v=1", "k,v:", "k,**v", "k,&v",
            "k,v;z", ";k,v",
        ] {
            let s = format!("hash.select {{ |{p}| k == :bar }}");
            assert!(offenses(&s).is_empty(), "{s}");
        }
        for s in [
            "hash.select { _1 == :bar }",
            "hash.select { _1.eql?(_2) }",
            "hash.select { it == :bar }",
            "hash.select { |it| it == :bar }",
            "hash.select { || k == :bar }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        check("hash.select { |k,v,| k == :bar }", ":bar", 1, 6, 1);
        check("hash.select { |it,v| it == :bar }", ":bar", 1, 6, 1);
    }
    #[test]
    fn literal_blocks_argument_modes_and_unknown_receivers() {
        for prefix in [
            "hash.select",
            "hash.select()",
            "[].select",
            "self.select",
            "nil.select",
            "select",
            "hash(foo: 1).select",
        ] {
            let s = format!("{prefix} {{ |k,v| k == :bar }}");
            check(&s, ":bar", 1, s.find("select").unwrap() as u32 + 1, 1);
        }
        for s in [
            "hash.select(1) { |k,v| k == :bar }",
            "hash.select(kw: 1) { |k,v| k == :bar }",
            "hash.select({kw: 1}) { |k,v| k == :bar }",
            "hash.select(*args) { |k,v| k == :bar }",
            "hash.select(&predicate)",
            "hash.select(&:predicate)",
            "hash.select(&nil)",
            "hash.select { |k,v| k.eql?(:bar, &predicate) }",
            "hash.select { |k,v| k.eql?(:bar) {} }",
            "hash.select { |k,v| k&.eql?(:bar) }",
            "hash.select { |k,v| [:bar]&.include?(k) }",
            "hash.select { |k,v| k.eql? }",
            "hash.select { |k,v| k.eql?(:bar, :baz) }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        check(
            "hash.select { |k,v| k.eql?(&predicate) }",
            "*&predicate",
            1,
            6,
            1,
        );
    }
    #[test]
    fn negation_polarity_and_boolean_statement_wrapper_quirks() {
        for body in [
            "k == :bar || k == :baz",
            "k == :bar && k == :baz",
            "(k == :bar)",
            "!(k == :bar)",
            "!!k.eql?(:bar)",
            "begin; k == :bar; end",
            "log; k == :bar",
            "return k == :bar",
            "defined?(k == :bar)",
            "k == :bar if condition",
        ] {
            let s = format!("hash.select {{ |k,v| {body} }}");
            assert!(offenses(&s).is_empty(), "{s}");
        }
        // Pinned unwrapping does not invert comparator polarity or reapply
        // the direct ==/!= static String/Symbol gate.
        check("hash.select { |k,v| !k.==(0.0) }", "0.0", 1, 6, 1);
        check("hash.reject { |k,v| !k.!=(effect) }", "*effect", 1, 6, 1);
        check("hash.select { |k,v| !k.eql?(:bar) }", ":bar", 1, 6, 1);
    }
    #[test]
    fn dynamic_keys_evaluation_order_and_valueflow_are_not_inferred() {
        for (body, keys) in [
            ("k.eql?(effect())", "*effect()"),
            ("effect().eql?(k)", "*effect()"),
            ("k.eql?(v)", "*v"),
            ("v.eql?(k)", "*v"),
            ("k.eql?(k)", "*k"),
            ("k.eql?([:a, effect(), *keys])", ":a, effect(), *keys"),
            ("k.eql?({a: effect()})", "{a: effect()}"),
            ("k.eql?(a: effect())", "a: effect()"),
            ("k.eql?(\"#{effect()}\")", "\"#{effect()}\""),
            ("k.eql?(:\"#{effect()}\")", ":\"#{effect()}\""),
            ("k.eql?(nil)", "nil"),
            ("k.eql?(__LINE__)", "__LINE__"),
            ("keys.include?(k)", "*keys"),
            ("k.include?(k)", "*k"),
        ] {
            check(&format!("hash.select {{ |k,v| {body} }}"), keys, 1, 6, 1);
        }
        for s in [
            "hash.select { |k,v| k.eql?(1..3) }",
            "hash.select { |k,v| (1..3).eql?(k) }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn source_unicode_crlf_bom_and_heredoc_ownership() {
        check(
            "é = 1; hash.select { |clé,v| clé == :bar }",
            ":bar",
            1,
            13,
            1,
        );
        check(
            "\u{feff}# encoding: UTF-8\r\nhash.select do |k,v|\r\n k == :bar\r\nend\r\n__END__\r\nraw\0\r\n",
            ":bar",
            2,
            6,
            4,
        );
        check(
            "consume(<<~TEXT)\n  #{hash.select { |k,v| k == :bar }}\nTEXT\n",
            ":bar",
            2,
            10,
            2,
        );
        check(
            "hash.select { |k,v| k.eql?(<<~KEY) }\nfoo\nKEY\n",
            "<<~KEY",
            1,
            6,
            1,
        );
        check(
            "hash.select { |k,v| <<~KEY.include?(k) }\nfoo\nKEY\n",
            "<<~KEY",
            1,
            6,
            1,
        );
    }
    #[test]
    fn minimum_ruby_gate_precedes_force_selection_and_config() {
        use crate::{
            config::ResolvedConfig,
            cops::{Registry, policy::RubyVersion, select_active_for_version},
        };
        let registry = Registry::new();
        let raw = toml::from_str("[cops.\"Style/HashSlice\"]\nenabled = true\n").unwrap();
        let config = ResolvedConfig::resolve(raw, None, &registry).unwrap();
        let only = ["Style/HashSlice".to_string()];
        for (version, expected) in [
            (RubyVersion(2, 4), 0),
            (RubyVersion(2, 5), 1),
            (RubyVersion(3, 4), 1),
        ] {
            let active = select_active_for_version(&registry, &config, &only, &[], version);
            assert_eq!(
                crate::pipeline::lint_str_with(&active, "hash.select { |k,v| k == :bar }").len(),
                expected
            );
        }
    }
}
