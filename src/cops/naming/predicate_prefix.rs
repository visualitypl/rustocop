//! `Naming/PredicatePrefix`: fixed forbidden prefixes, not return-type inference.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct PredicatePrefix;
impl Cop for PredicatePrefix {
    fn name(&self) -> &'static str {
        "Naming/PredicatePrefix"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if let Some(def) = node.as_def_node() {
            report(self, def.name().as_slice(), def.name_loc(), file, out);
        } else if let Some(call) = node.as_call_node() {
            if call.receiver().is_some()
                || !matches!(
                    call.name().as_slice(),
                    b"define_method" | b"define_singleton_method"
                )
            {
                return;
            }
            let Some(argument) = call.arguments().and_then(|a| a.arguments().iter().next()) else {
                return;
            };
            let Some(symbol) = argument.as_symbol_node() else {
                return;
            };
            // A physically split quoted symbol is a dsym in the reference AST.
            if crate::cops::ast::is_reference_symbol(&argument, file) {
                report(self, symbol.unescaped(), symbol.location(), file, out);
            }
        }
    }
}

fn report(
    cop: &PredicatePrefix,
    name: &[u8],
    location: ruby_prism::Location<'_>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    if name == b"is_a?" || name.ends_with(b"=") {
        return;
    }
    let Some(rest) = [b"is_".as_slice(), b"has_", b"have_", b"does_"]
        .into_iter()
        .find_map(|prefix| name.strip_prefix(prefix))
    else {
        return;
    };
    if rest.first().is_none_or(u8::is_ascii_digit) {
        return;
    }
    let name = String::from_utf8_lossy(name);
    let rest = String::from_utf8_lossy(rest);
    let suffix = if name.ends_with('?') { "" } else { "?" };
    let (line, col) = file.line_col(location.start_offset());
    out.push(Offense {
        correction: None,
        cop: cop.name(),
        severity: cop.severity(),
        message: format!("Rename `{name}` to `{rest}{suffix}`."),
        line,
        col,
        end_line: file.line_of(
            location
                .end_offset()
                .saturating_sub(1)
                .max(location.start_offset()),
        ),
    });
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop naming/predicate_prefix_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov), adapted to fixed defaults.
    use super::*;
    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Naming/PredicatePrefix")
            .collect()
    }
    #[test]
    fn flags_all_four_prefixes_with_or_without_question_marks() {
        for prefix in ["is_", "has_", "have_", "does_"] {
            for suffix in ["", "?", "!"] {
                for (receiver, col) in [("", 5), ("self.", 10), ("object.", 12)] {
                    let name = format!("{prefix}ready{suffix}");
                    let found = offenses(&format!("def {receiver}{name}(arg); 'not boolean'; end"));
                    assert_eq!(found.len(), 1, "{name}");
                    assert_eq!((found[0].line, found[0].col), (1, col));
                    let expected = if suffix == "?" {
                        "ready?".to_owned()
                    } else {
                        format!("ready{suffix}?")
                    };
                    assert_eq!(
                        found[0].message,
                        format!("Rename `{name}` to `{expected}`.")
                    );
                    assert!(found[0].correction.is_none());
                }
            }
        }
    }
    #[test]
    fn macros_require_bare_call_and_first_static_symbol_not_a_block() {
        for macro_name in ["define_method", "define_singleton_method"] {
            for tail in [
                "",
                " { |value| value }",
                " do |value| value end",
                ", implementation",
            ] {
                let source = if tail.starts_with(',') {
                    format!("{macro_name}(:is_hello{tail})")
                } else {
                    format!("{macro_name}(:is_hello){tail}")
                };
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    (found[0].line, found[0].col),
                    (1, macro_name.len() as u32 + 2)
                );
                assert_eq!(found[0].message, "Rename `is_hello` to `hello?`.");
            }
        }
        for source in [
            "self.define_method(:is_hello) {}",
            "object.define_singleton_method(:has_value) {}",
            "object&.define_method(:is_hello) {}",
            "define_method('is_hello') {}",
            "define_method(name) {}",
            "define_method(:\"is_#{name}\") {}",
            "define_method",
            "define_method(:fine, :is_hello)",
            "def_node_matcher :is_hello, pattern",
            "attr_reader :is_hello",
            "alias is_hello hello",
            "is_hello",
            "def even; true; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn static_symbols_use_decoded_names_but_physically_composed_symbols_are_dynamic() {
        for (symbol, name, expected) in [
            (r#":is_ready"#, "is_ready", "ready?"),
            (r#":'has_value'"#, "has_value", "value?"),
            (r#"%s(does_work)"#, "does_work", "work?"),
            (r#":"\u0069s_ready""#, "is_ready", "ready?"),
            (r#":is_٢d"#, "is_٢d", "٢d?"),
            (r#":is_?"#, "is_?", "?"),
        ] {
            let found = offenses(&format!("define_method({symbol}) {{}}"));
            assert_eq!(found.len(), 1, "{symbol}");
            assert_eq!((found[0].line, found[0].col), (1, 15));
            assert_eq!(
                found[0].message,
                format!("Rename `{name}` to `{expected}`.")
            );
        }
        for symbol in [
            r#":"is_\u0032d""#,
            ":\"is_ready\nthere\"",
            ":'is_ready\nthere'",
            "%s(is_ready\nthere)",
        ] {
            assert!(
                offenses(&format!("define_method({symbol}) {{}}")).is_empty(),
                "{symbol}"
            );
        }
    }

    #[test]
    fn numeric_empty_writer_and_allowed_names_are_not_predicates() {
        for name in [
            "is_a?",
            "is_2d?",
            "has_0",
            "have_9lives",
            "does_1",
            "is_",
            "has_",
            "have_",
            "does_",
            "is_ready=",
            "has_value=",
            "island",
            "haste",
            "have",
            "does",
        ] {
            assert!(offenses(&format!("def {name}; end")).is_empty(), "{name}");
            assert!(
                offenses(&format!("define_method(:{name}) {{}}")).is_empty(),
                "{name}"
            );
        }
    }
    #[test]
    fn sigs_and_return_values_do_not_change_the_fixed_policy() {
        for sig in [
            "",
            "sig { returns(String) }\n",
            "sig { returns(T::Boolean) }\n",
        ] {
            let found = offenses(&format!("{sig}def is_this_thing_on?; 'yes'; end"));
            assert_eq!(found.len(), 1);
            assert_eq!(
                found[0].message,
                "Rename `is_this_thing_on?` to `this_thing_on?`."
            );
        }
    }
}
