//! `Style/ClassCheck` — fixed preference for `is_a?`.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ClassCheck;
impl Cop for ClassCheck {
    fn name(&self) -> &'static str {
        "Style/ClassCheck"
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
        if call.name().as_slice() != b"kind_of?" {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (line, col) = file.line_col(selector.start_offset());
        let correction = (cx.corrections_requested()
            && !file
                .class_check_bindings(cx.ancestors().last().unwrap_or(node))
                .custom)
            .then(|| Correction {
                range: selector.start_offset()..selector.end_offset(),
                replacement: "is_a?".into(),
            });
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Prefer `Object#is_a?` over `Object#kind_of?`.".into(),
            correction,
        });
    }
}

// Corrections only: one cached walk of the main AST, never one scan per call.
// Visible custom dispatch/visibility/refinements protect the whole file. We
// cannot infer arbitrary external monkey patches or project-wide bindings.
#[derive(Default)]
pub(crate) struct Bindings {
    custom: bool,
    #[cfg(test)]
    pub(crate) visited: usize,
}
const METHODS: &[&[u8]] = &[b"kind_of?", b"is_a?", b"method_missing"];
const METHOD_LOOKUPS: &[&[u8]] = &[
    b"method",
    b"public_method",
    b"singleton_method",
    b"instance_method",
    b"public_instance_method",
];
// Acquiring these methods can conceal a later binding/visibility change behind
// call/[]/bind_call (possibly after storage). Protect at acquisition rather than
// infer receivers, invocation arguments, value flow or execution order.
const ACQUIRED_HAZARDS: &[&[u8]] = &[
    // Struct.new/Data.define generate member readers. As with other acquired
    // binders, do not infer receiver identity (including aliases/shadowing).
    b"new",
    b"define",
    b"define_method",
    b"define_singleton_method",
    b"alias_method",
    b"attr",
    b"attr_reader",
    b"attr_accessor",
    b"undef_method",
    b"remove_method",
    b"private",
    b"public",
    b"protected",
    b"module_function",
    b"private_class_method",
    b"public_class_method",
    b"extend",
    b"include",
    b"prepend",
    b"refine",
    b"using",
    b"eval",
    b"class_eval",
    b"module_eval",
    b"instance_eval",
    b"class_exec",
    b"module_exec",
    b"instance_exec",
    b"send",
    b"__send__",
    b"public_send",
];
impl Bindings {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut result = Self::default();
        walk(&mut result, root);
        result
    }
}
impl<'pr> NodeVisitor<'pr> for Bindings {
    fn enter(&mut self, node: Node<'pr>) {
        use super::file_api::{binds_method, may_name_method};
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
            self.custom |= binds_method(&call, METHODS);
            let args = ast::call_arguments(&call);
            let mut name = call.name().as_slice().to_vec();
            let mut args = args.as_slice();
            if matches!(name.as_slice(), b"send" | b"__send__" | b"public_send")
                && let Some((first, rest)) = args.split_first()
            {
                name = first
                    .as_symbol_node()
                    .map(|n| n.unescaped().to_vec())
                    .or_else(|| first.as_string_node().map(|n| n.unescaped().to_vec()))
                    .unwrap_or_default();
                args = rest;
            }
            // Unknown lookup selectors may acquire any hazard. Known benign
            // reflection (e.g. method(:is_a?)) must still permit canonical fixes.
            // Lookups themselves can also be acquired for an indirect lookup.
            self.custom |= (METHOD_LOOKUPS.contains(&name.as_slice())
                && args.iter().any(|n| {
                    may_name_method(n, ACQUIRED_HAZARDS) || may_name_method(n, METHOD_LOOKUPS)
                }))
                || matches!(name.as_slice(), b"using" | b"refine")
                || (matches!(
                    name.as_slice(),
                    b"private"
                        | b"public"
                        | b"protected"
                        | b"module_function"
                        | b"private_class_method"
                        | b"public_class_method"
                ) && args.iter().any(|n| may_name_method(n, METHODS)))
                || (matches!(
                    name.as_slice(),
                    b"eval" | b"class_eval" | b"module_eval" | b"instance_eval"
                ) && !args.is_empty());
        }
    }
}

/// Correction-only binding evidence for class-check selector replacements.
pub(crate) static CLASS_CHECK_BINDINGS: crate::source::Fact<Bindings> = crate::source::Fact::new();

impl crate::source::SourceFile {
    pub(crate) fn class_check_bindings(&self, root: &ruby_prism::Node<'_>) -> &Bindings {
        CLASS_CHECK_BINDINGS.get(self, || Bindings::build(root))
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/class_check_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    #[test]
    fn pinned_selectors_and_canonical_corrections() {
        for (before, after, col) in [
            ("x.kind_of? y", "x.is_a? y", 3),
            ("x&.kind_of? y", "x&.is_a? y", 4),
            ("kind_of?(String)", "is_a?(String)", 1),
            ("x::kind_of? (String)", "x::is_a? (String)", 4),
        ] {
            let found: Vec<_> = lint_str(before)
                .into_iter()
                .filter(|o| o.cop == ClassCheck.name())
                .collect();
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, col, 1)
            );
            assert_eq!(
                found[0].message,
                "Prefer `Object#is_a?` over `Object#kind_of?`."
            );
            assert_correction(&ClassCheck, before, after);
        }
    }
    #[test]
    fn ignores_lookalikes_and_local_bindings() {
        for source in [
            "x.is_a?(String)",
            "kind_of = 1; kind_of",
            "x.kind_of",
            "x.kind_of!",
            "def kind_of?(x); end",
            "'x.kind_of?(String)' # kind_of?\n",
        ] {
            assert!(
                correction_offenses(&ClassCheck, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn keeps_offenses_but_withholds_visible_custom_dispatch() {
        for prefix in [
            "def is_a?(x); :custom; end",
            "def x.kind_of?(x); :custom; end",
            "class Object; alias kind_of? custom; end",
            "class Object; alias is_a? kind_of?; end",
            "class Object; undef is_a?; end",
            "C.define_method(:is_a?) { false }",
            "C.define_method(:method_missing) { |name, *args| :custom }",
            "class C < BasicObject; alias method_missing handler; end",
            "def method_missing(*args); :custom; end",
            "C.define_singleton_method(:kind_of?) { false }",
            "C.alias_method(:kind_of?, :custom)",
            "C.send(:define_method, :is_a?) { false }",
            "C.remove_method(:is_a?)",
            "C.private(:is_a?)",
            "C.protected(:kind_of?)",
            "class C; private; def kind_of?(x); false; end; end",
            "module R; refine String do; def is_a?(x); false; end; end; end; using R",
            "C.class_eval('def is_a?(x); false; end')",
            "C.instance_eval(code)",
            "using R",
        ] {
            let source = format!("{prefix}\nx.kind_of?(String)\n");
            let found = correction_offenses(&ClassCheck, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn module_function_visibility_keeps_offenses_but_withholds_corrections() {
        for visibility in [
            "module_function :is_a?",
            "module_function :kind_of?",
            "module_function name",
            "send(:module_function, :is_a?)",
            "__send__('module_function', 'is_a?')",
            "public_send(:module_function, name)",
        ] {
            let source = format!("module M; {visibility}; end\nx.kind_of?(String)");
            let found = correction_offenses(&ClassCheck, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        for visibility in ["module_function", "module_function :other"] {
            let before = format!("module M; {visibility}; end\nx.kind_of?(String)");
            assert_correction(&ClassCheck, &before, &before.replace("kind_of?", "is_a?"));
        }
    }
    #[test]
    fn method_object_binder_and_visibility_acquisitions_withhold_not_hide() {
        // Guard acquisition, not just immediate invocation: Method objects can
        // escape through locals/containers/blocks without visible value flow.
        for lookup in [
            "method",
            "public_method",
            "singleton_method",
            "instance_method",
            "public_instance_method",
        ] {
            for selector in [
                ":define_singleton_method",
                "'define_method'",
                ":new",
                "'define'",
                ":alias_method",
                ":attr",
                ":attr_reader",
                ":attr_accessor",
                ":remove_method",
                ":undef_method",
                ":private",
                ":public",
                ":protected",
                ":private_class_method",
                ":public_class_method",
                ":module_function",
                ":extend",
                ":include",
                ":prepend",
                ":refine",
                ":using",
                ":eval",
                ":class_eval",
                ":module_eval",
                ":instance_eval",
                ":class_exec",
                ":module_exec",
                ":instance_exec",
                ":send",
                ":__send__",
                ":public_send",
                ":method",
                ":public_method",
                ":singleton_method",
                ":instance_method",
                ":public_instance_method",
                "selector",
                "*names",
                "\"define_#{suffix}\"",
            ] {
                for acquisition in [
                    format!("obj.{lookup}({selector})"),
                    format!("obj.send(:{lookup}, {selector})"),
                    format!("obj.__send__('{lookup}', {selector})"),
                    format!("obj.public_send(:{lookup}, {selector})"),
                ] {
                    let source = format!("binder = {acquisition}; x.kind_of?(String)");
                    let found = correction_offenses(&ClassCheck, &source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert!(found[0].correction.is_none(), "{source}");
                    assert_eq!(
                        found[0].message,
                        "Prefer `Object#is_a?` over `Object#kind_of?`."
                    );
                }
            }
        }
    }
    #[test]
    fn method_object_invocations_and_stored_binders_withhold_corrections() {
        for binding in [
            "Struct.method(:new).call(:is_a?)",
            "binder = Struct.method(:new); binder[:is_a?]",
            "Data.public_method('define').call(:is_a?)",
            "binder = Data.method(:define); binder.call(:is_a?)",
            "obj.method(:define_singleton_method).call(:is_a?) { |_| false }",
            "obj.public_method('define_singleton_method')[:is_a?] { |_| false }",
            "binder = obj.method(:define_singleton_method); binder.call(:is_a?) { |_| false }",
            "Module.instance_method(:define_method).bind_call(obj.singleton_class, :is_a?) { |_| false }",
            "Module.public_instance_method(:define_method).bind(obj.singleton_class).call(:is_a?) { |_| false }",
            "module M; method(:module_function).call(:is_a?); end",
            "module M; binder = method(:module_function); binder[:is_a?]; end",
            "obj.singleton_class.method(:private).call(:is_a?)",
            "obj.method(selector).call(:is_a?) { |_| false }",
        ] {
            let source = format!("{binding}; obj.kind_of?(Object)");
            let found = correction_offenses(&ClassCheck, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn benign_method_reflection_keeps_canonical_fixes() {
        for lookup in [
            "method",
            "public_method",
            "singleton_method",
            "instance_method",
            "public_instance_method",
        ] {
            for selector in [
                ":is_a?",
                "'kind_of?'",
                ":object_id",
                ":other",
                ":attr_writer",
            ] {
                for acquisition in [
                    format!("obj.{lookup}({selector})"),
                    format!("obj.send(:{lookup}, {selector})"),
                    format!("obj.__send__('{lookup}', {selector})"),
                    format!("obj.public_send(:{lookup}, {selector})"),
                ] {
                    let before = format!("m = {acquisition}; x.kind_of?(String)");
                    let after = format!("m = {acquisition}; x.is_a?(String)");
                    assert_correction(&ClassCheck, &before, &after);
                }
            }
        }
        assert_correction(
            &ClassCheck,
            "obj.method(:is_a?).call(Object); obj.kind_of?(Object)",
            "obj.method(:is_a?).call(Object); obj.is_a?(Object)",
        );
    }
}
