//! `Style/FetchEnvVar` — explicit nil defaults, empty AllowedVariables.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::collections::{HashMap, HashSet};
#[cfg(test)]
thread_local! {
    static CONDITION_CHILD_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
pub struct FetchEnvVar;
impl Cop for FetchEnvVar {
    fn name(&self) -> &'static str {
        "Style/FetchEnvVar"
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
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.is_safe_navigation()
            || call.name().as_slice() != b"[]"
            || !call.receiver().is_some_and(|r| {
                r.as_constant_read_node()
                    .is_some_and(|c| c.name().as_slice() == b"ENV")
            })
        {
            return;
        }
        let args = ast::call_arguments(&call);
        let [key] = args.as_slice() else { return };
        if allowable(node, key, cx, file) {
            return;
        }
        let spelling = String::from_utf8_lossy(key.location().as_slice());
        let text = format!("ENV.fetch({spelling}, nil)");
        let correction = (cx.corrections_requested()
            && correction_key(key)
            && std::str::from_utf8(node.location().as_slice()).is_ok())
        .then(|| super::file_api::replacement(node, cx, file, text.clone(), "Env"))
        .flatten();
        let start = node.location().start_offset();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `{text}` instead of `ENV[{spelling}]`."),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1).max(start)),
            correction,
        });
    }
}

// A bracket argument has a different grammar from a positional method argument:
// command calls, bare rescue, splats and keyword hashes cannot simply be moved
// before `, nil`. Keep these findings, but do not guess at their runtime binding.
fn correction_key(key: &Node<'_>) -> bool {
    if ast::is_basic_literal(key)
        || ast::is_variable(key)
        || ast::is_const(key)
        || matches!(
            key,
            Node::InterpolatedStringNode { .. } | Node::ParenthesesNode { .. }
        )
    {
        return true;
    }
    let Some(call) = key.as_call_node() else {
        return false;
    };
    if call.block().is_some() {
        return false;
    }
    let args = ast::call_arguments(&call);
    args.is_empty()
        || call.opening_loc().is_some()
        || (call.name().as_slice() == b"+"
            && args.len() == 1
            && args[0].as_string_node().is_some()
            && call
                .receiver()
                .is_some_and(|r| r.as_string_node().is_some()))
}
fn comparison(name: &[u8]) -> bool {
    matches!(name, b"==" | b"===" | b"!=" | b"<" | b">" | b"<=" | b">=")
}
fn same_location(a: &Node<'_>, b: &Node<'_>) -> bool {
    a.location().start_offset() == b.location().start_offset()
        && a.location().end_offset() == b.location().end_offset()
}
fn allowable(node: &Node<'_>, key: &Node<'_>, cx: &NodeContext<'_, '_>, file: &SourceFile) -> bool {
    // Argument/statement lists are Prism-only wrappers. Explicit parentheses
    // are reference `begin` nodes and must remain in the ancestor chain.
    let mut ancestors = cx
        .ancestors()
        .filter(|p| p.as_arguments_node().is_none() && p.as_statements_node().is_none());
    if let Some(parent) = ancestors.next() {
        if let Some(p) = parent.as_call_node() {
            if comparison(p.name().as_slice())
                || (p.name().as_slice() == b"!" && p.call_operator_loc().is_none())
            {
                return true;
            }
            if p.receiver().is_some_and(|r| same_location(&r, node))
                && p.call_operator_loc().is_some()
            {
                return true;
            }
        }
        if let Some(path) = parent.as_constant_path_node()
            && path
                .parent()
                .is_some_and(|scope| same_location(&scope, node))
            && let Some(assignment) = ancestors.next()
        {
            // Prism splits the namespace-bearing casgn LHS into a target
            // path and a write node. In the reference, its namespace is an
            // assigned direct child, including under ||=, &&= and op=.
            macro_rules! path_assignment {
                ($($access:ident),*) => { $(
                    if let Some(write) = assignment.$access()
                        && same_location(&write.target().as_node(), parent)
                    { return true; }
                )* };
            }
            path_assignment!(
                as_constant_path_write_node,
                as_constant_path_or_write_node,
                as_constant_path_and_write_node,
                as_constant_path_operator_write_node
            );
        }
        if let Some(or) = parent.as_or_node()
            && (same_location(&or.left(), node)
                || ancestors.next().is_some_and(|p| p.as_or_node().is_some()))
        {
            return true;
        }
    }
    let condition = cx.ancestors().find_map(|p| {
        p.as_if_node()
            .map(|p| p.predicate())
            .or_else(|| p.as_unless_node().map(|p| p.predicate()))
    });
    let Some(condition) = condition else {
        return false;
    };
    let location = condition.location();
    let mut cache = file.fetch_env_conditions().lock().unwrap();
    let guards = cache
        .entry((location.start_offset(), location.end_offset()))
        .or_insert_with(|| ConditionGuards::build(&condition));
    guards.keys.contains(&ast::fingerprint(key))
}

pub(crate) type Conditions = HashMap<(usize, usize), ConditionGuards>;

/// Only direct reference-AST children can guard a repeated read. Index their
/// structural keys once, not once per ENV read (wide arrays/calls are common).
#[derive(Default)]
pub(crate) struct ConditionGuards {
    keys: HashSet<ast::Fingerprint<'static>>,
}
impl ConditionGuards {
    fn key(&mut self, key: &Node<'_>) {
        self.keys.insert(ast::fingerprint(key).into_owned());
    }
    fn child(&mut self, node: Node<'_>) {
        #[cfg(test)]
        CONDITION_CHILD_VISITS.with(|count| count.set(count.get() + 1));
        let Some(call) = node.as_call_node() else {
            return;
        };
        if !call.is_safe_navigation()
            && call.block().is_none()
            && call.name().as_slice() == b"[]"
            && env_receiver(call.receiver())
        {
            let args = ast::call_arguments(&call);
            if let [key] = args.as_slice() {
                self.key(key);
            }
        }
    }
    fn statements(&mut self, node: Node<'_>, single: bool) {
        if let Some(body) = node.as_statements_node() {
            if !single || body.body().len() == 1 {
                for child in body.body().iter() {
                    self.child(child);
                }
            }
        } else {
            self.child(node);
        }
    }
    fn build(condition: &Node<'_>) -> Self {
        let mut guards = Self::default();
        if let Some(c) = condition.as_call_node() {
            let args = ast::call_arguments(&c);
            if let Some(block) = c.block().and_then(|b| b.as_block_node()) {
                // Reference block children: send, parameters, body. The send's
                // arguments are not direct children of the block.
                if !c.is_safe_navigation()
                    && c.name().as_slice() == b"[]"
                    && env_receiver(c.receiver())
                    && let [key] = args.as_slice()
                {
                    guards.key(key);
                }
                if let Some(body) = block.body() {
                    guards.statements(body, true);
                }
                return guards;
            }
            // Reference compares send children, not the method name (key?, []=).
            if !c.is_safe_navigation()
                && env_receiver(c.receiver())
                && (args.len() == 1 || c.name().as_slice() == b"[]=")
                && let Some(key) = args.first()
            {
                guards.key(key);
            }
            if c.is_safe_navigation()
                || comparison(c.name().as_slice())
                || c.name().as_slice().ends_with(b"?")
            {
                for child in c.receiver().into_iter().chain(args) {
                    guards.child(child);
                }
            }
            return guards;
        }
        if let Some(c) = condition.as_and_node() {
            guards.child(c.left());
            guards.child(c.right());
            return guards;
        }
        if let Some(c) = condition.as_or_node() {
            guards.child(c.left());
            guards.child(c.right());
            return guards;
        }
        macro_rules! values {
            ($($access:ident),*) => { $(if let Some(n) = condition.$access() {
                guards.child(n.value()); return guards;
            })* };
        }
        values!(
            as_local_variable_write_node,
            as_local_variable_or_write_node,
            as_local_variable_and_write_node,
            as_local_variable_operator_write_node,
            as_instance_variable_write_node,
            as_instance_variable_or_write_node,
            as_instance_variable_and_write_node,
            as_instance_variable_operator_write_node,
            as_class_variable_write_node,
            as_class_variable_or_write_node,
            as_class_variable_and_write_node,
            as_class_variable_operator_write_node,
            as_global_variable_write_node,
            as_global_variable_or_write_node,
            as_global_variable_and_write_node,
            as_global_variable_operator_write_node,
            as_constant_write_node,
            as_constant_or_write_node,
            as_constant_and_write_node,
            as_constant_operator_write_node,
            as_call_or_write_node,
            as_call_and_write_node,
            as_call_operator_write_node,
            as_constant_path_or_write_node,
            as_constant_path_and_write_node,
            as_constant_path_operator_write_node
        );
        if let Some(n) = condition.as_constant_path_write_node() {
            // A plain casgn exposes its namespace directly; compound writes
            // instead expose the synthetic casgn LHS, not that LHS's children.
            if let Some(parent) = n.target().parent() {
                guards.child(parent);
            }
            guards.child(n.value());
            return guards;
        }
        macro_rules! indices {
            ($($access:ident),*) => { $(if let Some(n) = condition.$access() {
                guards.child(n.value());
                let args: Vec<_> = n.arguments().map(|a| a.arguments().iter().collect()).unwrap_or_default();
                if env_receiver(n.receiver()) && let [key] = args.as_slice() { guards.key(key); }
                return guards;
            })* };
        }
        indices!(
            as_index_or_write_node,
            as_index_and_write_node,
            as_index_operator_write_node
        );
        if let Some(c) = condition.as_array_node() {
            for child in c.elements().iter() {
                guards.child(child);
            }
        } else if let Some(c) = condition.as_range_node() {
            for child in [c.left(), c.right()].into_iter().flatten() {
                guards.child(child);
            }
        } else if let Some(c) = condition.as_flip_flop_node() {
            for child in [c.left(), c.right()].into_iter().flatten() {
                guards.child(child);
            }
        } else if let Some(c) = condition.as_defined_node() {
            guards.child(c.value());
        } else if let Some(c) = condition.as_parentheses_node() {
            if let Some(body) = c.body() {
                guards.statements(body, false);
            }
        } else if let Some(c) = condition.as_begin_node()
            && let Some(body) = c.statements()
        {
            guards.statements(body.as_node(), false);
        }
        guards
    }
}
fn env_receiver(receiver: Option<Node<'_>>) -> bool {
    receiver.is_some_and(|r| {
        r.as_constant_read_node()
            .is_some_and(|c| c.name().as_slice() == b"ENV")
    })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/fetch_env_var_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    #[test]
    fn pinned_value_positions_and_canonical_corrections() {
        for (source, col) in [
            ("ENV['X']", 1),
            ("ENV['X' + 'Y']", 1),
            ("y ||= ENV['X']", 7),
            ("y &&= ENV['X']", 7),
            ("some_method(ENV['X'])", 13),
            ("x.some_method(ENV['X'])", 15),
            ("x = ENV['X']", 5),
            ("x, y = ENV['X'], 1", 8),
            ("[ENV['X']]", 2),
            ("{ ENV['X'] => :x }", 3),
            ("{ x: ENV['X'] }", 6),
            ("\"#{ENV['X']}\"", 4),
            ("y || ENV['X']", 6),
            ("y || z || ENV['X']", 11),
            ("ENV['X'] && y", 1),
        ] {
            let key = if source.contains("'X' + 'Y'") {
                "'X' + 'Y'"
            } else {
                "'X'"
            };
            let found: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/FetchEnvVar")
                .collect();
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
            assert_eq!(
                found[0].message,
                format!("Use `ENV.fetch({key}, nil)` instead of `ENV[{key}]`.")
            );
            assert_correction(
                &FetchEnvVar,
                source,
                &source.replace(&format!("ENV[{key}]"), &format!("ENV.fetch({key}, nil)")),
            );
        }
    }
    #[test]
    fn pinned_multiline_and_guarded_before_after_examples() {
        for (source, keys) in [
            (
                "some_method(\n  ENV['A'].some_method,\n  ENV['B'] || ENV['C'],\n  ENV['X'],\n  ENV['Y']\n)\n",
                vec!["C", "X", "Y"],
            ),
            ("x, y = ENV['X'],\n       ENV['Y']\n", vec!["X", "Y"]),
            ("[\n  ENV['X'],\n  ENV['Y']\n]\n", vec!["X", "Y"]),
            (
                "{\n  ENV['X'] => :x,\n  ENV['Y'] => :y\n}\n",
                vec!["X", "Y"],
            ),
            ("{\n  x: ENV['X'],\n  y: ENV['Y']\n}\n", vec!["X", "Y"]),
            ("if ENV['X']\n  puts ENV['Y']\nend\n", vec!["Y"]),
            ("if ENV['X'] = x\n  puts ENV['Y']\nend\n", vec!["Y"]),
            ("if ENV && x\n  ENV[x]\nend\n", vec![]),
            ("if ENV || x\n  ENV[x]\nend\n", vec![]),
            ("if a == b\n  ENV['X']\nend\n", vec!["X"]),
            ("if a || b && c\n  puts ENV['X']\nend\n", vec!["X"]),
        ] {
            let mut expected = source.to_owned();
            for key in keys {
                expected = expected.replace(
                    &format!("ENV['{key}']"),
                    &format!("ENV.fetch('{key}', nil)"),
                );
            }
            expected = expected.replace("ENV[x]", "ENV.fetch(x, nil)");
            assert_correction(&FetchEnvVar, source, &expected);
        }
    }
    #[test]
    fn flags_chains_assignments_and_nonmatching_receivers_are_allowed() {
        for source in [
            "!ENV['X']",
            "not ENV['X']",
            "ENV['X'].some_method",
            "ENV['X']&.some_method",
            "ENV['X'] == 1",
            "1 != ENV['X']",
            "ENV['X'] ||= y",
            "ENV['X'] &&= y",
            "x ||= ENV['X'] ||= y",
            "ENV['X'] += y",
            "ENV['X'] = y",
            "ENV['X'] || y",
            "z || ENV['X'] || y",
            "ENV.fetch('X')",
            "ENV.fetch('X', default_value)",
            "ENV.fetch('X') { fallback }",
            "::ENV['X']",
            "Other::ENV['X']",
            "env['X']",
            "ENV&.[]('X')",
            "ENV['X', 'Y']",
            "ENV['X', &b]",
        ] {
            assert!(
                correction_offenses(&FetchEnvVar, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn binding_mutations_and_uncertain_key_syntax_keep_the_offense_without_edits() {
        for binding in [
            "ENV = Custom",
            "module M; ENV = Custom; end",
            "Object.const_set(:ENV, custom)",
            "def ENV.fetch(*args); :changed; end",
            "def fetch(*args); :changed; end",
            "ENV.define_singleton_method(:[]) { |k| 'changed' }",
            "ENV.singleton_class.alias_method(:fetch, :[])",
            "ENV.instance_eval('def fetch(*args); 123; end')",
            "ENV.instance_eval { def fetch(*args); 123; end }",
            "ENV.singleton_class.class_exec(:fetch) { |n| define_method(n) { 123 } }",
            "module M; refine ENV.singleton_class do; def fetch(*args); 123; end; end; end",
            "alias fetch object_id",
            "undef fetch",
            "define_method(name) { 123 }",
        ] {
            let source = format!("{binding}\nENV['X']\n");
            let found = correction_offenses(&FetchEnvVar, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        for source in [
            "ENV[key value]",
            "ENV[*keys]",
            "ENV[x: 1]",
            "ENV['X'] { side_effect }",
            "ENV[ # keep\n 'X']",
            "ENV[<<~KEY]\nX\nKEY\n",
            "ENV[helper(<<~KEY)]\nX\nKEY\n",
            "ENV[\n 'X'\n]\np __LINE__\n",
            "ENV[\n 'X'\n] # rustocop:disable Lint/Debugger\n",
        ] {
            let found = correction_offenses(&FetchEnvVar, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        for binding in [
            "def other; 123; end",
            "ENV['X'] = 'value'",
            "ENV.delete('X')",
            "# ENV = Custom",
            "'def fetch; end'",
        ] {
            let source = format!("{binding}\nENV['X']\n");
            assert_correction(
                &FetchEnvVar,
                &source,
                &source.replace("ENV['X']\n", "ENV.fetch('X', nil)\n"),
            );
        }
    }
    #[test]
    fn literal_spelling_dynamic_keys_and_source_bytes_are_preserved() {
        for key in [
            "'X'",
            "\"\\x58\"",
            "%q(X)",
            "\"#{next_key()}\"",
            "key",
            "key()",
            "obj.key(1)",
            "('X')",
            "__LINE__",
            "'X' + 'Y'",
            "\"X\nY\"",
        ] {
            assert_correction(
                &FetchEnvVar,
                &format!("ENV[{key}]"),
                &format!("ENV.fetch({key}, nil)"),
            );
        }
        assert_correction(
            &FetchEnvVar,
            "\u{feff}# encoding: UTF-8\r\nx = ENV[\"\\x58\"] # keep\r\np __LINE__\r\n__END__\r\nraw\0\r\n",
            "\u{feff}# encoding: UTF-8\r\nx = ENV.fetch(\"\\x58\", nil) # keep\r\np __LINE__\r\n__END__\r\nraw\0\r\n",
        );
    }
    #[test]
    fn assignment_array_and_literal_block_conditions_use_direct_ruby_children() {
        for condition in [
            "x = ENV['X']",
            "@x = ENV['X']",
            "x ||= ENV['X']",
            "ENV['X'] ||= x",
            "[ENV['X']]",
            "ENV['X'] { side_effect }",
            "foo { ENV['X'] }",
        ] {
            let source = format!("if {condition}; puts ENV['X']; end");
            assert!(
                correction_offenses(&FetchEnvVar, &source).is_empty(),
                "{source}"
            );
        }
        let source = "if { x: ENV['X'] }; puts ENV['X']; end";
        assert_eq!(
            correction_offenses(&FetchEnvVar, source).len(),
            2,
            "{source}"
        );
    }
    #[test]
    fn synthetic_blocks_safe_calls_and_non_send_conditions_match_reference_flags() {
        for condition in [
            "ENV['X']&.foo",
            "ENV['X']..x",
            "x..ENV['X']",
            "defined?(ENV['X'])",
            "(ENV['X']; foo)",
        ] {
            let source = format!("if {condition}; puts ENV['X']; end");
            assert!(
                correction_offenses(&FetchEnvVar, &source).is_empty(),
                "{source}"
            );
        }
        for condition in ["ENV&.key?('X')", "foo?(ENV['X']) { 1 }"] {
            let source = format!("if {condition}; puts ENV['X']; end");
            let expected = if condition.starts_with("foo") { 2 } else { 1 };
            assert_eq!(
                correction_offenses(&FetchEnvVar, &source).len(),
                expected,
                "{source}"
            );
        }
    }
    #[test]
    fn distinct_conditional_keys_have_bounded_child_matching_work() {
        for count in [256, 512, 1024] {
            let keys: Vec<_> = (0..count).map(|i| format!("ENV['KEY{i}']")).collect();
            for condition in [
                format!("[{}]", keys.join(",")),
                format!("foo?({})", keys.join(",")),
                format!("({})", keys.join(";")),
                format!("begin {}; end", keys.join(";")),
            ] {
                let source = format!("if {condition}; p {}; end", keys.join(","));
                CONDITION_CHILD_VISITS.with(|visits| visits.set(0));
                assert!(correction_offenses(&FetchEnvVar, &source).is_empty());
                CONDITION_CHILD_VISITS.with(|visits| assert_eq!(visits.get(), count));
            }
        }
    }
    #[test]
    fn compound_attribute_and_constant_path_guards_use_only_direct_children() {
        for condition in [
            "obj.a ||= ENV['X']",
            "obj.a &&= ENV['X']",
            "obj.a += ENV['X']",
            "obj&.a ||= ENV['X']",
            "Other::A = ENV['X']",
            "::A = ENV['X']",
            "Other::A ||= ENV['X']",
            "Other::A &&= ENV['X']",
            "Other::A += ENV['X']",
            "ENV['X']::A = y",
        ] {
            let source = format!("if {condition}; p ENV['X']; end");
            assert!(
                correction_offenses(&FetchEnvVar, &source).is_empty(),
                "{source}"
            );
            let source = format!("if {condition}; p ENV['Y']; end");
            let found = correction_offenses(&FetchEnvVar, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col),
                (1, source.find("ENV['Y']").unwrap() as u32 + 1)
            );
            assert_eq!(
                found[0].message,
                "Use `ENV.fetch('Y', nil)` instead of `ENV['Y']`."
            );
        }
        for source in [
            "if obj.a ||= wrap(ENV['X']); p ENV['X']; end",
            "if [ENV['X'] { x }]; p ENV['X']; end",
        ] {
            assert_eq!(
                correction_offenses(&FetchEnvVar, source).len(),
                2,
                "{source}"
            );
        }
        // The namespace is the direct first child of a synthetic casgn LHS,
        // not of the outer compound assignment. Only that read is assigned.
        for operator in ["=", "||=", "&&=", "+="] {
            let source = format!("ENV['X']::A {operator} y");
            assert!(
                correction_offenses(&FetchEnvVar, &source).is_empty(),
                "{source}"
            );
            let source = format!("if (ENV['X']::A {operator} y); p ENV['X']; end");
            let found = correction_offenses(&FetchEnvVar, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(found[0].col, source.rfind("ENV['X']").unwrap() as u32 + 1);
        }
    }
    #[test]
    fn explicit_grouping_preserves_reference_parent_and_condition_relationships() {
        for source in [
            "!(ENV['X'])",
            "(ENV['X']).foo",
            "(ENV['X']) || y",
            "if ((ENV['X'])); p ENV['X']; end",
            "if [(ENV['X'])]; p ENV['X']; end",
        ] {
            let expected: Vec<_> = source
                .match_indices("ENV['X']")
                .map(|(i, _)| (1, i as u32 + 1))
                .collect();
            let found = correction_offenses(&FetchEnvVar, source);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                expected,
                "{source}"
            );
            assert_correction(
                &FetchEnvVar,
                source,
                &source.replace("ENV['X']", "ENV.fetch('X', nil)"),
            );
        }
        for source in [
            "if (ENV['X']); p ENV['X']; end",
            "!ENV['X']",
            "ENV['X'].foo",
            "ENV['X'] || y",
        ] {
            assert!(
                correction_offenses(&FetchEnvVar, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn conditional_flags_and_repeated_guarded_keys() {
        for condition in [
            "ENV['X']",
            "ENV['X'] == foo",
            "ENV['X'] != foo",
            "ENV['X'].present?",
            "ENV['X'].in?(%w[A B])",
            "%w[A B].include?(ENV['X'])",
            "ENV.key?('X')",
            "ENV['X'] = x",
        ] {
            for source in [
                format!("if {condition}\n puts ENV['X']\nend"),
                format!("unless {condition}\n puts ENV['X']\nend"),
            ] {
                assert!(
                    correction_offenses(&FetchEnvVar, &source).is_empty(),
                    "{source}"
                );
            }
        }
        for source in [
            "ENV['X'] ? x : y",
            "if foo? || ENV['X']; puts ENV['X']; end",
            "if ENV['X'] && bar?; puts ENV['X']; end",
        ] {
            assert!(
                correction_offenses(&FetchEnvVar, source).is_empty(),
                "{source}"
            );
        }
        for source in [
            "if ENV['X']; puts ENV['Y']; end",
            "if ENV['X'] = x; puts ENV['Y']; end",
            "if ENV && x; ENV[x]; end",
            "if ENV || x; ENV[x]; end",
            "if a == b; ENV['X']; end",
            "if a || b && c; puts ENV['X']; end",
        ] {
            assert_eq!(
                correction_offenses(&FetchEnvVar, source).len(),
                1,
                "{source}"
            );
        }
        assert_correction(
            &FetchEnvVar,
            "case ENV['X']\nwhen ENV['Y']\n puts x\nend",
            "case ENV.fetch('X', nil)\nwhen ENV.fetch('Y', nil)\n puts x\nend",
        );
    }
}
