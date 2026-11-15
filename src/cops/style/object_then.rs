//! `Style/ObjectThen` — prefer `then` for block-bearing `yield_self` calls.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ObjectThen;
impl Cop for ObjectThen {
    fn name(&self) -> &'static str {
        "Style/ObjectThen"
    }
    fn safe_fixes(&self) -> bool {
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
        if call.name().as_slice() != b"yield_self" {
            return;
        }
        let Some(block) = call.block() else { return };
        // Prism stores &expr separately from positional arguments. Literal
        // blocks are checked even with arguments, matching upstream on_block.
        if block.as_block_node().is_none() && call.arguments().is_some() {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Prefer `then` over `yield_self`.".into(),
            correction: (cx.corrections_requested()
                && !file
                    .object_then_bindings(cx.ancestors().next_back().unwrap_or(node))
                    .custom)
                .then(|| Correction {
                    range: selector.start_offset()..selector.end_offset(),
                    replacement: if call.receiver().is_some() {
                        "then"
                    } else {
                        "self.then"
                    }
                    .into(),
                }),
        });
    }
}

// One lazy, bounded main-AST walk, correction-only. We guard visible custom
// dispatch/visibility rather than claiming to infer external monkey patches.
#[derive(Default)]
pub(crate) struct Bindings {
    custom: bool,
    #[cfg(test)]
    pub(crate) visited: usize,
}
impl Bindings {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut facts = Self::default();
        crate::cops::walk::walk(&mut facts, root);
        facts
    }
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Bindings {
    fn enter(&mut self, node: Node<'pr>) {
        use super::file_api::{binds_method, may_name_method};
        const METHODS: &[&[u8]] = &[b"then", b"yield_self", b"method_missing"];
        #[cfg(test)]
        {
            self.visited += 1;
        }
        if let Some(def) = node.as_def_node() {
            self.custom |= METHODS.contains(&def.name().as_slice());
        }
        if let Some(alias) = node.as_alias_method_node() {
            self.custom |= may_name_method(&alias.new_name(), METHODS);
        }
        if let Some(undef) = node.as_undef_node() {
            self.custom |= undef.names().iter().any(|n| may_name_method(&n, METHODS));
        }
        if let Some(call) = node.as_call_node() {
            self.custom |= binds_method(&call, METHODS)
                || matches!(
                    call.name().as_slice(),
                    b"using"
                        | b"refine"
                        | b"eval"
                        | b"class_eval"
                        | b"module_eval"
                        | b"instance_eval"
                        | b"send"
                        | b"__send__"
                        | b"public_send"
                        | b"private"
                        | b"public"
                        | b"protected"
                        | b"module_function"
                        | b"private_class_method"
                        | b"public_class_method"
                        | b"method"
                        | b"public_method"
                        | b"singleton_method"
                        | b"instance_method"
                        | b"public_instance_method"
                );
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/object_then_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    #[test]
    fn minimum_ruby_and_multiline_selector_locations() {
        use crate::{
            config::ResolvedConfig,
            cops::{Registry, policy::RubyVersion, select_active_for_version},
            pipeline::lint_str_with,
        };
        let registry = Registry::new();
        let config = ResolvedConfig::default();
        for (version, count) in [
            (RubyVersion(2, 5), 0),
            (RubyVersion(2, 6), 1),
            (RubyVersion(3, 4), 1),
        ] {
            let active = select_active_for_version(
                &registry,
                &config,
                &["Style/ObjectThen".into()],
                &[],
                version,
            );
            assert_eq!(
                lint_str_with(&active, "obj.yield_self { |e| e }").len(),
                count
            );
        }
        let source = "obj.\n  yield_self do |x|\n    x\n  end\n";
        let found = correction_offenses(&ObjectThen, source);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
        assert_correction(&ObjectThen, source, "obj.\n  then do |x|\n    x\n  end\n");
    }

    #[test]
    fn literal_block_arguments_and_captures_remain_verbatim() {
        // Literal blocks are checked irrespective of arity upstream; no argument
        // or block-signature inference, deletion, or enumerator rewrite.
        for (before, after) in [
            (
                "obj.yield_self(1) { |x, y| [x, y] }",
                "obj.then(1) { |x, y| [x, y] }",
            ),
            (
                "n = 0; obj.yield_self { |x; local| n += 1; local = x; local }",
                "n = 0; obj.then { |x; local| n += 1; local = x; local }",
            ),
            ("obj.yield_self { next :done }", "obj.then { next :done }"),
            ("obj.yield_self { break :done }", "obj.then { break :done }"),
        ] {
            assert_correction(&ObjectThen, before, after);
        }
    }

    #[test]
    fn visible_custom_bindings_keep_offenses_but_withhold_edits() {
        for binding in [
            "def then; :different; end",
            "def yield_self; :different; end",
            "class X; alias then foo; end",
            "class X; undef then; end",
            "X.define_method(:then) { :different }",
            "X.send(:private, :then)",
            "X.private(:yield_self)",
            "using R",
            "eval(code)",
            "binder = X.method(:define_method)",
        ] {
            let source = format!("{binding}\nobj.yield_self {{ it }}");
            let found = correction_offenses(&ObjectThen, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn no_block_and_lookalikes_are_unchanged() {
        for source in [
            "obj.map { |x| x.foo }",
            "obj.then { it }",
            "obj.yield_self",
            "obj&.yield_self",
            "obj.yield_self(other, &:test)",
            "obj&.yield_self(other, &:test)",
            "yield_self = 1; yield_self",
            "def yield_self; end",
            "obj.yield_self(1)",
        ] {
            assert!(
                correction_offenses(&ObjectThen, source).is_empty(),
                "{source}"
            );
        }
    }
}
