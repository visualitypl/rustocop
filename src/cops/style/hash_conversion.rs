//! `Style/HashConversion` — fixed AllowSplatArgument=true; unsafe corrections omitted.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct HashConversion;
impl Cop for HashConversion {
    fn name(&self) -> &'static str {
        "Style/HashConversion"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some((args, end)) = candidate(node) else {
            return;
        };
        let start = node.location().start_offset();
        // Upstream ignores all nested sends inside a matched constructor, even
        // when the outer splat has no offense. Literal block bodies are outside
        // the send range; compound-assignment values are outside the getter.
        // Check only bounded ancestors, never root/body/sibling subtrees.
        if cx.ancestors().any(|parent| {
            candidate_end(parent).is_some_and(|parent_end| {
                parent.location().start_offset() <= start && end <= parent_end
            })
        }) {
            return;
        }
        let message = match args.as_slice() {
            [arg] if arg.as_hash_node().is_some() || arg.as_keyword_hash_node().is_some() => {
                "Prefer literal hash to `Hash[key: value, ...]`."
            }
            [arg] if arg.as_splat_node().is_some() => return,
            [_] => "Prefer `ary.to_h` to `Hash[ary]`.",
            _ if args.iter().any(|arg| arg.as_splat_node().is_some()) => return,
            _ => "Prefer literal hash to `Hash[arg1, arg2, ...]`.",
        };
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: message.into(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: None,
        });
    }
}
fn candidate<'pr>(node: &Node<'pr>) -> Option<(Vec<Node<'pr>>, usize)> {
    let end = candidate_end(node)?;
    let args = if let Some(call) = node.as_call_node() {
        ast::call_arguments(&call)
    } else {
        ast::compound_index_getter(node)?.arguments
    };
    Some((args, end))
}
// This part does not allocate an argument vector: it is also used for the
// ignored-ancestor test, whose cost must not depend on ancestor arity.
fn candidate_end(node: &Node<'_>) -> Option<usize> {
    if let Some(call) = node.as_call_node() {
        return (!call.is_safe_navigation()
            && call.name().as_slice() == b"[]"
            && call
                .receiver()
                .is_some_and(|r| ast::is_global_const(&r, &["Hash"])))
        .then(|| {
            // Use aggregate locations, not `ast::source_end`: that helper
            // collects and scans every argument, making the ancestor checks
            // quadratic for a wide constructor containing nested candidates.
            // The full call location includes a literal block, but a block
            // pass is part of the send and must remain inside this boundary.
            let mut end = call.message_loc().unwrap().end_offset();
            if let Some(closing) = call.closing_loc() {
                end = end.max(closing.end_offset());
            }
            if let Some(arguments) = call.arguments() {
                end = end.max(arguments.location().end_offset());
            }
            if let Some(block) = call
                .block()
                .filter(|block| block.as_block_argument_node().is_some())
            {
                end = end.max(block.location().end_offset());
            }
            end
        });
    }
    macro_rules! getter_end {
        ($access:ident) => {{
            let n = node.$access().unwrap();
            (!n.is_safe_navigation()
                && n.receiver()
                    .is_some_and(|r| ast::is_global_const(&r, &["Hash"])))
            .then(|| n.closing_loc().end_offset())
        }};
    }
    match node {
        Node::IndexOperatorWriteNode { .. } => getter_end!(as_index_operator_write_node),
        Node::IndexOrWriteNode { .. } => getter_end!(as_index_or_write_node),
        Node::IndexAndWriteNode { .. } => getter_end!(as_index_and_write_node),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop hash_conversion_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == HashConversion.name())
            .collect()
    }
    const TO_H: &str = "Prefer `ary.to_h` to `Hash[ary]`.";
    const MULTI: &str = "Prefer literal hash to `Hash[arg1, arg2, ...]`.";
    const HASH: &str = "Prefer literal hash to `Hash[key: value, ...]`.";
    #[test]
    fn wide_constructors_do_not_rescan_ancestor_arguments() {
        for n in [100, 1000] {
            for source in [
                format!("Hash[{}]", vec!["Hash[ary]"; n].join(",")),
                format!(
                    "Hash.[]({}) {{ Hash[other] }}",
                    vec!["Hash[ary]"; n].join(",")
                ),
                format!("Hash.[] {}", vec!["Hash[ary]"; n].join(",")),
            ] {
                ast::CALL_ARGUMENT_VISITS.set(0);
                let found = crate::cops::testing::correction_offenses(&HashConversion, &source);
                let visits = ast::CALL_ARGUMENT_VISITS.get();
                assert!(
                    visits <= 3 * n + 3,
                    "{visits} argument visits for {n} nested candidates"
                );
                assert_eq!(found.len(), if source.contains('{') { 2 } else { 1 });
                assert!(found.iter().all(|o| o.correction.is_none()));
            }
        }
    }
    #[test]
    fn aggregate_constructor_boundaries_match_send_ranges() {
        for source in [
            "Hash[]",
            "::Hash[Hash[ary], Hash[other]]",
            "Hash.[](ary)",
            "Hash::[] ary, Hash[other]",
            "Hash.[] ary, &Hash[other]",
            "Hash[ary, &Hash[other]]",
            "Hash[ary] { Hash[other] }",
            "Hash.[] ary do\n Hash[other]\nend",
            "Hash[\n ary\n] do\n Hash[other]\nend",
            "Hash[<<~TEXT, Hash[ary]]\n  body\nTEXT\n",
            "Hash.[] Hash[ary], <<~TEXT\n  body\nTEXT\n",
        ] {
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{source}");
            let call = parsed
                .node()
                .as_program_node()
                .unwrap()
                .statements()
                .body()
                .iter()
                .next()
                .unwrap();
            assert_eq!(
                candidate_end(&call),
                Some(ast::source_end(&call)),
                "{source}"
            );
        }
    }
    #[test]
    fn pinned_single_argument_and_nested_examples() {
        for arg in [
            "ary",
            "a.foo :bar",
            "array.zip",
            "array.zip()",
            "a || b",
            "(a || b)",
            "a && b",
            "(a && b)",
            "array.zip([1, 2, 3])",
            "Hash[]",
            "Hash[k, v]",
        ] {
            let s = format!("Hash[{arg}]\n");
            let o = offenses(&s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, 1, 1));
            assert_eq!(o[0].message, TO_H);
            let fixes = crate::cops::testing::correction_offenses(&HashConversion, &s);
            assert_eq!(fixes.len(), 1);
            assert!(fixes[0].correction.is_none());
        }
    }
    #[test]
    fn pinned_multi_argument_and_hash_examples_with_all_parent_shapes() {
        for (s, line, col, msg) in [
            ("Hash[a, b, c, d]", 1, 1, MULTI),
            ("Hash[a: b, c: d]", 1, 1, HASH),
            ("do_something(Hash[a: b, c: d], 42)", 1, 14, HASH),
            ("do_something Hash[a: b, c: d], 42", 1, 14, HASH),
            ("foo&.bar Hash[a => b]", 1, 10, HASH),
            ("foo&.bar = Hash[a, b]", 1, 12, MULTI),
            ("def foo\n  super Hash[a => b]\nend", 2, 9, HASH),
            ("def foo\n  super x, Hash[a => b]\nend", 2, 12, HASH),
            ("def foo\n  yield Hash[a, b]\nend", 2, 9, MULTI),
            ("do_something Hash[a, b].merge(c)", 1, 14, MULTI),
            ("do_something Hash[a => b].size", 1, 14, HASH),
            ("do_something Hash[a, b]&.merge(c)", 1, 14, MULTI),
            ("do_something Hash[a, b].map { |k, v| k }", 1, 14, MULTI),
            ("Hash[]", 1, 1, MULTI),
            ("do_something Hash[a, b, c, d], arg", 1, 14, MULTI),
            ("Hash[a, b, c]", 1, 1, MULTI),
            ("Hash[1, Hash[k, v]]", 1, 1, MULTI),
            ("Hash[].to_h", 1, 1, MULTI),
            ("Hash[a, b].size", 1, 1, MULTI),
            ("!Hash[a => b]", 1, 2, HASH),
            ("to_h Hash[a, b]", 1, 6, MULTI),
            ("foo[Hash[a, b]]", 1, 5, MULTI),
            ("foo.bar = Hash[a => b]", 1, 11, HASH),
            ("foo == Hash[a, b]", 1, 8, MULTI),
            ("foo.[] Hash[a, b]", 1, 8, MULTI),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (line, col), "{s}");
            assert_eq!(o[0].message, msg, "{s}");
            let fixes = crate::cops::testing::correction_offenses(&HashConversion, s);
            assert_eq!(fixes.len(), 1, "{s}");
            assert!(fixes[0].correction.is_none(), "{s}");
        }
        assert!(!HashConversion.safe_fixes());
    }
    #[test]
    fn pinned_splats_and_receiver_setter_lookalikes() {
        for s in [
            "Hash[*ary, x]",
            "Hash[*ary]",
            "def foo(*); Hash[*]; end",
            "Other::Hash[ary]",
            "object[ary]",
            "Hash.new(ary)",
            "Hash&.[](ary)",
            "Hash[ary] = value",
            "Hash[ary], x = values",
            "Hash = {}; Hash[:x] = 1",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn qualified_calls_blocks_keywords_wrappers_and_arity() {
        for (s, msg) in [
            ("::Hash[ary]", TO_H),
            ("Hash.[](ary)", TO_H),
            ("Hash::[](ary)", TO_H),
            ("Hash[&block]", TO_H),
            ("Hash[ary, &block]", MULTI),
            ("Hash[] { effect }", MULTI),
            ("Hash[ary] { effect }", TO_H),
            ("Hash[**options]", HASH),
            ("Hash[{a: b}]", HASH),
            ("Hash[({a: b})]", TO_H),
            ("Hash[(ary)]", TO_H),
            ("Hash[a, b: c]", MULTI),
            ("Hash[\"a\" => value]", HASH),
            ("Hash[effect() => value]", HASH),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!(o[0].message, msg, "{s}");
        }
        for s in [
            "Hash[*ary, &block]",
            "Hash[*Hash[ary]]",
            "Hash[Hash[*Hash[ary]]]",
        ] {
            let expected = usize::from(s.starts_with("Hash[Hash"));
            assert_eq!(offenses(s).len(), expected, "{s}");
        }
        assert_eq!(offenses("Hash[ary] { Hash[other] }").len(), 2);
    }
    #[test]
    fn multiline_ranges_stop_before_literal_blocks_but_include_block_pass() {
        for (s, line, col, end_line) in [
            (
                "# header\n  Hash[\n    ary\n  ] do\n effect\n end\n",
                2,
                3,
                4,
            ),
            ("Hash.[](\n ary,\n &block\n)\n", 1, 1, 4),
            ("\u{feff}\"é\"; ::Hash[ary]", 1, 6, 1),
            ("Hash[\n ary\n] ||= rhs\n", 1, 1, 3),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!(
                (o[0].line, o[0].col, o[0].end_line),
                (line, col, end_line),
                "{s}"
            );
            assert!(o[0].correction.is_none());
        }
    }
    #[test]
    fn compound_getters_are_not_setters_and_do_not_ignore_the_rhs() {
        for op in ["+=", "||=", "&&="] {
            for (args, msg) in [
                ("", MULTI),
                ("ary", TO_H),
                ("{a: b}", HASH),
                ("a,b,c", MULTI),
            ] {
                let s = format!("# header\n  ::Hash[{args}] {op} Hash[other]\n");
                let o = offenses(&s);
                assert_eq!(o.len(), 2, "{s}");
                assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, 3, 2));
                assert_eq!(o[0].message, msg, "{s}");
                assert_eq!(o[1].message, TO_H, "{s}");
            }
        }
        assert_eq!(offenses("Hash[Hash[ary]] ||= Hash[other]").len(), 2);
        assert_eq!(offenses("Hash[Hash[ary]] = Hash[other]").len(), 2);
    }
}
