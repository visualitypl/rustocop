//! `Style/HashExcept` — Ruby >= 3.0, unsafe detection only.
//! Matcher semantics ported from RuboCop's HashSubset (MIT License,
//! Copyright (c) 2012-26 Bozhidar Batsov).
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{BlockNode, CallNode, Node};
pub struct HashExcept;
impl Cop for HashExcept {
    fn name(&self) -> &'static str {
        "Style/HashExcept"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        check_subset(self, Subset::Except, node, file, out);
    }
}

// Shared only with HashSlice: both use the pinned HashSubset extraction;
// their accepted semantic polarities are exact complements.
pub(super) enum Subset {
    Except,
    Slice,
}

pub(super) fn check_subset(
    cop: &dyn Cop,
    subset: Subset,
    node: &Node<'_>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    let Some(call) = node.as_call_node() else {
        return;
    };
    if !matches!(call.name().as_slice(), b"reject" | b"select" | b"filter")
        || call.arguments().is_some_and(|a| !a.arguments().is_empty())
    {
        return;
    }
    let Some(block) = call.block().and_then(|b| b.as_block_node()) else {
        return;
    };
    let Some((key, value)) = parameters(&block) else {
        return;
    };
    let Some(statements) = block.body().and_then(|b| b.as_statements_node()) else {
        return;
    };
    if statements.body().len() != 1 {
        return;
    }
    let Some(body) = statements.body().first().and_then(|n| plain_send(&n)) else {
        return;
    };
    let (comparison, negated) =
        if body.name().as_slice() == b"!" && ast::call_arguments(&body).is_empty() {
            let Some(inner) = body.receiver().and_then(|n| plain_send(&n)) else {
                return;
            };
            (inner, true)
        } else {
            (body, false)
        };
    let args = ast::call_arguments(&comparison);
    let [rhs] = args.as_slice() else { return };
    let Some(lhs) = comparison.receiver() else {
        return;
    };
    if !local_named(&lhs, key.name().as_slice()) && !local_named(rhs, key.name().as_slice()) {
        return;
    }
    let method = comparison.name().as_slice();
    if !matches!(method, b"==" | b"!=" | b"eql?" | b"include?") {
        return;
    }
    if method == b"include?"
        && (rhs.location().as_slice() != key.location().as_slice()
            || local_named(&lhs, value.name().as_slice())
            || local_named(rhs, value.name().as_slice()))
    {
        return;
    }
    if range_receiver(&lhs) || rhs.as_range_node().is_some() {
        return;
    }
    let except_key = if lhs.location().as_slice() == key.location().as_slice() {
        rhs
    } else {
        &lhs
    };
    if !negated
        && matches!(method, b"==" | b"!=")
        && !matches!(
            except_key,
            Node::SymbolNode { .. } | Node::StringNode { .. } | Node::SourceFileNode { .. }
        )
    {
        return;
    }
    // Deliberately follow the pinned unsafe detector: negation affects
    // membership only, not comparator polarity.
    let semantic = if call.name().as_slice() == b"reject" {
        matches!(method, b"==" | b"eql?") || (method == b"include?" && !negated)
    } else {
        method == b"!=" || (method == b"include?" && negated)
    };
    let preferred = match subset {
        Subset::Except if semantic => "except",
        Subset::Slice if !semantic => "slice",
        _ => return,
    };
    let start = call.message_loc().unwrap().start_offset();
    let (line, col) = file.line_col(start);
    out.push(Offense {
        cop: cop.name(),
        severity: cop.severity(),
        message: format!("Use `{preferred}({})` instead.", key_source(except_key)),
        line,
        col,
        end_line: file.last_line_of(block.location()),
        correction: None,
    });
}

// No implicit numbered/it parameters, destructuring, defaults, keywords,
// splats, block passes or shadowargs: precisely two ordinary `arg` nodes.
fn parameters<'pr>(
    block: &BlockNode<'pr>,
) -> Option<(
    ruby_prism::RequiredParameterNode<'pr>,
    ruby_prism::RequiredParameterNode<'pr>,
)> {
    let bp = block.parameters()?.as_block_parameters_node()?;
    if !bp.locals().is_empty() {
        return None;
    }
    let p = bp.parameters()?;
    if p.requireds().len() != 2
        || !p.optionals().is_empty()
        || !p.posts().is_empty()
        || p.rest()
            .is_some_and(|r| r.as_implicit_rest_node().is_none())
        || !p.keywords().is_empty()
        || p.keyword_rest().is_some()
        || p.block().is_some()
    {
        return None;
    }
    let mut required = p.requireds().iter();
    Some((
        required.next()?.as_required_parameter_node()?,
        required.next()?.as_required_parameter_node()?,
    ))
}
fn plain_send<'pr>(node: &Node<'pr>) -> Option<CallNode<'pr>> {
    node.as_call_node().filter(|c| {
        !c.is_safe_navigation() && !c.block().is_some_and(|b| b.as_block_node().is_some())
    })
}
fn local_named(node: &Node<'_>, name: &[u8]) -> bool {
    node.as_local_variable_read_node()
        .is_some_and(|n| n.name().as_slice() == name)
}
fn range_receiver(node: &Node<'_>) -> bool {
    node.as_range_node().is_some()
        || node.as_parentheses_node().is_some_and(|p| {
            p.body()
                .and_then(|b| b.as_statements_node())
                .and_then(|s| s.body().first())
                .is_some_and(|n| range_receiver(&n))
        })
}
fn key_source(node: &Node<'_>) -> String {
    if let Some(array) = node.as_array_node() {
        let percent = array
            .opening_loc()
            .is_some_and(|l| l.as_slice().starts_with(b"%"));
        return array
            .elements()
            .iter()
            .map(|element| {
                let source = String::from_utf8_lossy(element.location().as_slice());
                if !percent {
                    return source.into_owned();
                }
                if element.as_interpolated_symbol_node().is_some() {
                    format!(":\"{source}\"")
                } else if element.as_interpolated_string_node().is_some() {
                    format!("\"{source}\"")
                } else if element.as_symbol_node().is_some() {
                    format!(":{source}")
                } else if let Some(s) = element.as_string_node() {
                    format!(
                        "'{}'",
                        String::from_utf8_lossy(s.unescaped())
                            .replace('\\', "\\\\")
                            .replace('\'', "\\'")
                    )
                } else {
                    format!("'{source}'")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
    }
    let source = String::from_utf8_lossy(node.location().as_slice());
    if ast::is_literal(node) {
        source.into_owned()
    } else {
        format!("*{source}")
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop `spec/rubocop/cop/style/hash_except_spec.rb` (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Fixed ActiveSupport=false.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        let parsed = crate::parser::parse(s.as_bytes());
        assert!(parsed.errors().next().is_none(), "invalid fixture {s}");
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/HashExcept")
            .collect()
    }
    fn check(s: &str, key: &str, line: u32, col: u32, end_line: u32) {
        let o = offenses(s);
        assert_eq!(o.len(), 1, "{s}");
        assert_eq!(
            (o[0].line, o[0].col, o[0].end_line),
            (line, col, end_line),
            "{s}"
        );
        assert_eq!(o[0].message, format!("Use `except({key})` instead."), "{s}");
        assert!(o[0].correction.is_none());
    }
    #[test]
    fn minimum_ruby_gate_precedes_force_selection_and_config() {
        use crate::{
            config::ResolvedConfig,
            cops::{Registry, policy::RubyVersion, select_active_for_version},
        };
        let registry = Registry::new();
        let raw = toml::from_str("[cops.\"Style/HashExcept\"]\nenabled = true\n").unwrap();
        let config = ResolvedConfig::resolve(raw, None, &registry).unwrap();
        let only = ["Style/HashExcept".to_string()];
        for (version, expected) in [
            (RubyVersion(2, 7), 0),
            (RubyVersion(3, 0), 1),
            (RubyVersion(3, 4), 1),
        ] {
            let active = select_active_for_version(&registry, &config, &only, &[], version);
            let found = crate::pipeline::lint_str_with(&active, "hash.reject { |k,v| k == :bar }");
            assert_eq!(found.len(), expected);
        }
    }

    #[test]
    fn pinned_membership_and_percent_key_messages() {
        for (method, bang) in [("reject", ""), ("select", "!"), ("filter", "!")] {
            for (array, keys) in [
                ("[:foo, :bar]", ":foo, :bar"),
                ("%i[foo bar]", ":foo, :bar"),
                ("%I[#{foo} bar]", ":\"#{foo}\", :bar"),
                ("%W[#{foo} bar]", "\"#{foo}\", 'bar'"),
                ("%w[isn't back\\\\slash]", "'isn\\'t', 'back\\\\slash'"),
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
    }
    #[test]
    fn pinned_comparators_and_exact_locations() {
        for (method, op) in [("reject", "=="), ("select", "!="), ("filter", "!=")] {
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
            "{foo: 1, bar: 2, baz: 3}&.reject { |k,v| k == :bar }",
            ":bar",
            1,
            27,
            1,
        );
        check("hash.reject { |k,v| k == 'str' }", "'str'", 1, 6, 1);
        check("hash.reject { |k,v| k.eql?(0.0) }", "0.0", 1, 6, 1);
        check("hash.reject do |k,v|\n  k == :bar\nend", ":bar", 1, 6, 3);
    }

    #[test]
    fn pinned_no_offense_comparators_and_mutating_methods() {
        for s in [
            "hash.reject { |k,v| k == 0.0 }",
            "hash.select { |k,v| k != 0.0 }",
            "hash.select { |k,v| !(k == 0.0) }",
            "hash.reject { |k,v| !(k != 0.0) }",
            "hash.delete_if { |k,v| k == :bar }",
            "hash.keep_if { |k,v| k != :bar }",
            "hash.reject { |k,v| v.eql?(:bar) }",
            "hash.reject { |k,v| k != :bar }",
            "hash.reject { |k,v| :bar != key }",
            "hash.select { |k,v| k == :bar }",
            "hash.select { |k,v| :bar == key }",
            "hash.reject { |k,v| do_something != :bar }",
            "hash.reject",
            "hash.except(:bar)",
            "hash.reject { |k,v| k == :\"#{effect}\" }",
            "hash.select { |k,v| k != \"#{effect}\" }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }

    #[test]
    fn pinned_no_offense_membership_and_fixed_active_support_false() {
        for s in [
            "hash.reject { |k,v| !%i[foo bar].include?(k) }",
            "array = %i[foo bar]; hash.reject { |k,v| !array.include?(k) }",
            "hash.reject { |k,v| !array.include?(k) }",
            "hash.reject { |k,v| ![1,2].include?(v) }",
            "hash.reject { |k,v| k.include?('oo') }",
            "hash.reject { |k,v| !k.include?('oo') }",
            "hash.reject { |k,v| (:baa..:bbb).include?(k) }",
            "hash.reject { |k,v| (:baa...:bbb).include?(k) }",
            "hash.reject { |k,v| !v.include?(k) }",
            "hash.reject { |k,v| v.include?(k) }",
            "hash.reject { |k,v| %i[foo bar].include? }",
            "hash.reject { |k,v| k.in?(%i[foo bar]) }",
            "hash&.reject { |k,v| k.in?(%i[foo bar]) }",
            "hash.reject { |k,v| %i[foo bar].in?(k) }",
            "hash.reject { |k,v| !%i[foo bar].exclude?(k) }",
            "hash&.reject { |k,v| !%i[foo bar].exclude?(k) }",
            "hash.reject { |k,v| k.exclude?('oo') }",
            "hash.reject { |k,v| !k.exclude?('oo') }",
            "hash.select { |k,v| %i[foo bar].exclude?(k) }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }

    #[test]
    fn exactly_two_plain_parameters_not_numbered_it_keywords_or_destructuring() {
        for p in [
            "k", "k,v,z", "k,*v", "*k,v", "(k,z),v", "k,(v,z)", "k,v=1", "k,v:", "k,**v", "k,&v",
            "k,v;z", ";k,v",
        ] {
            let s = format!("hash.reject {{ |{p}| k == :bar }}");
            assert!(offenses(&s).is_empty(), "{s}");
        }
        for s in [
            "hash.reject { _1 == :bar }",
            "hash.reject { _1.eql?(_2) }",
            "hash.reject { it == :bar }",
            "hash.reject { |it| it == :bar }",
            "hash.reject { || k == :bar }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        check("hash.reject { |k,v,| k == :bar }", ":bar", 1, 6, 1);
        check("hash.reject { |it,v| it == :bar }", ":bar", 1, 6, 1);
    }

    #[test]
    fn literal_blocks_arity_and_receiver_lookalikes_follow_pinned_unsafe_policy() {
        for prefix in [
            "hash.reject",
            "hash.reject()",
            "[].reject",
            "self.reject",
            "nil.reject",
            "reject",
            "hash(foo: 1).reject",
        ] {
            let s = format!("{prefix} {{ |k,v| k == :bar }}");
            let col = s.find("reject").unwrap() as u32 + 1;
            check(&s, ":bar", 1, col, 1);
        }
        for s in [
            "hash.reject(1) { |k,v| k == :bar }",
            "hash.reject(kw: 1) { |k,v| k == :bar }",
            "hash.reject({kw: 1}) { |k,v| k == :bar }",
            "hash.reject(*args) { |k,v| k == :bar }",
            "hash.reject(&predicate)",
            "hash.reject(&:predicate)",
            "hash.reject(&nil)",
            "hash.reject { |k,v| k.eql?(:bar, &predicate) }",
            "hash.reject { |k,v| k.eql?(:bar) {} }",
            "hash.reject { |k,v| k&.eql?(:bar) }",
            "hash.reject { |k,v| [:bar]&.include?(k) }",
            "hash.reject { |k,v| k.eql? }",
            "hash.reject { |k,v| k.eql?(:bar, :baz) }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        check(
            "hash.reject { |k,v| k.eql?(&predicate) }",
            "*&predicate",
            1,
            6,
            1,
        );
    }

    #[test]
    fn boolean_and_wrapper_shapes_do_not_broaden_the_matcher() {
        for body in [
            "k == :bar || k == :baz",
            "k != :bar && k != :baz",
            "(k == :bar)",
            "!(k == :bar)",
            "!!k.eql?(:bar)",
            "begin; k == :bar; end",
            "log; k == :bar",
            "return k == :bar",
            "defined?(k == :bar)",
            "k == :bar if condition",
        ] {
            let s = format!("hash.reject {{ |k,v| {body} }}");
            assert!(offenses(&s).is_empty(), "{s}");
        }
        // Upstream unwraps a single `!` for comparator semantics but does
        // not invert ==/!= polarity or run its direct-comparator literal gate.
        check("hash.reject { |k,v| !k.==(0.0) }", "0.0", 1, 6, 1);
        check("hash.select { |k,v| !k.!=(effect) }", "*effect", 1, 6, 1);
        check("hash.reject { |k,v| !k.eql?(:bar) }", ":bar", 1, 6, 1);
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
            check(&format!("hash.reject {{ |k,v| {body} }}"), keys, 1, 6, 1);
        }
        for s in [
            "hash.reject { |k,v| k.eql?(1..3) }",
            "hash.reject { |k,v| (1..3).eql?(k) }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }

    #[test]
    fn source_unicode_crlf_bom_and_heredoc_ast_ownership() {
        check(
            "é = 1; hash.reject { |clé,v| clé == :bar }",
            ":bar",
            1,
            13,
            1,
        );
        check(
            "\u{feff}# encoding: UTF-8\r\nhash.reject do |k,v|\r\n k == :bar\r\nend\r\n__END__\r\nraw\0\r\n",
            ":bar",
            2,
            6,
            4,
        );
        check(
            "consume(<<~TEXT)\n  #{hash.reject { |k,v| k == :bar }}\nTEXT\n",
            ":bar",
            2,
            10,
            2,
        );
        check(
            "hash.reject { |k,v| k.eql?(<<~KEY) }\nfoo\nKEY\n",
            "<<~KEY",
            1,
            6,
            1,
        );
        check(
            "hash.reject { |k,v| <<~KEY.include?(k) }\nfoo\nKEY\n",
            "<<~KEY",
            1,
            6,
            1,
        );
    }
}
