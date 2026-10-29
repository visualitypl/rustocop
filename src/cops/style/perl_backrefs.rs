//! `Style/PerlBackrefs` — prefer explicit regexp match access.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct PerlBackrefs;
impl Cop for PerlBackrefs {
    fn name(&self) -> &'static str {
        "Style/PerlBackrefs"
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
        let loc = node.location();
        let (expression, safe) = if let Some(n) = node.as_numbered_reference_read_node() {
            // Zero is Prism's oversized-reference sentinel, not the whole match.
            (
                format!(
                    "Regexp.last_match({})",
                    String::from_utf8_lossy(&loc.as_slice()[1..])
                ),
                n.number() != 0 && n.number() <= i32::MAX as u32,
            )
        } else if node.as_back_reference_read_node().is_some()
            || node.as_global_variable_read_node().is_some()
        {
            match loc.as_slice() {
                b"$&" => ("Regexp.last_match(0)".into(), true),
                b"$MATCH" => ("Regexp.last_match(0)".into(), false),
                b"$`" | b"$PREMATCH" => ("Regexp.last_match.pre_match".into(), false),
                b"$'" | b"$POSTMATCH" => ("Regexp.last_match.post_match".into(), false),
                _ => return,
            }
        } else {
            return;
        };
        let expression = if cx
            .ancestors()
            .any(|n| matches!(n, Node::ClassNode { .. } | Node::ModuleNode { .. }))
        {
            format!("::{expression}")
        } else {
            expression
        };
        // Pre/post match calls raise when no match exists. English aliases need
        // a dominating English load and can be independently rebound. Preserve
        // those reads; detection must not depend on correction eligibility.
        let correction = (cx.corrections_requested()
            && safe
            && !cx.ancestors().any(|n| n.as_defined_node().is_some())
            && !file
                .perl_backref_bindings(cx.ancestors().last().unwrap_or(node))
                .unsafe_binding)
            .then(|| {
                let replacement = if cx
                    .parent()
                    .is_some_and(|n| n.as_embedded_variable_node().is_some())
                {
                    format!("{{{expression}}}")
                } else {
                    expression.clone()
                };
                Correction {
                    range: loc.start_offset()..loc.end_offset(),
                    replacement,
                }
            });
        let original = String::from_utf8_lossy(loc.as_slice());
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Prefer `{expression}` over `{original}`."),
            line,
            col,
            end_line: line,
            correction,
        });
    }
}

/// File-wide visible hazards, indexed once only when fixes are requested.
/// Namespace-local Regexp shadows (except Object's) are bypassed with ::.
/// No code is evaluated and no control-flow/dominance assumptions are made.
#[derive(Default)]
pub(crate) struct Bindings {
    unsafe_binding: bool,
    // Each class/module records whether its unqualified writes reach Object.
    namespaces: Vec<bool>,
    stack: Vec<bool>,
    #[cfg(test)]
    pub(crate) visited: usize,
}
impl Bindings {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut bindings = Self::default();
        crate::cops::walk::walk(&mut bindings, root);
        bindings
    }
    fn protect_name(&mut self, name: &[u8]) {
        self.unsafe_binding |= self.namespaces.last().copied().unwrap_or(true) && name == b"Regexp";
    }
    fn protect_path(&mut self, path: &Node<'_>) {
        use crate::cops::ast;
        if path.as_constant_read_node().is_some() {
            self.protect_name(path.location().as_slice());
        } else {
            // Qualified writes can reach Object::Regexp or ::Regexp. Unknown
            // namespace aliases are not value-flow analyzed.
            self.unsafe_binding |= ast::short_const_name(path).as_deref() == Some("Regexp");
        }
    }
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Bindings {
    fn enter(&mut self, node: Node<'pr>) {
        use super::file_api::{binds_constant, binds_method, may_name_method};
        #[cfg(test)]
        {
            self.visited += 1;
        }
        macro_rules! constants {
            ($($access:ident),*) => { $(if let Some(n) = node.$access() { self.protect_name(n.name().as_slice()); })* };
        }
        constants!(
            as_constant_write_node,
            as_constant_or_write_node,
            as_constant_and_write_node,
            as_constant_operator_write_node,
            as_constant_target_node
        );
        if let Some(n) = node.as_constant_path_target_node() {
            self.protect_path(&n.as_node());
        }
        macro_rules! paths {
            ($($access:ident),*) => { $(if let Some(n) = node.$access() { self.protect_path(&n.target().as_node()); })* };
        }
        paths!(
            as_constant_path_write_node,
            as_constant_path_or_write_node,
            as_constant_path_and_write_node,
            as_constant_path_operator_write_node
        );
        if let Some(n) = node.as_class_node() {
            self.protect_path(&n.constant_path());
        }
        if let Some(n) = node.as_module_node() {
            self.protect_path(&n.constant_path());
        }
        if let Some(n) = node.as_def_node() {
            self.unsafe_binding |= n.name().as_slice() == b"last_match";
        }
        if let Some(n) = node.as_alias_method_node() {
            self.unsafe_binding |= may_name_method(&n.new_name(), &[b"last_match"]);
        }
        if let Some(n) = node.as_undef_node() {
            self.unsafe_binding |= n
                .names()
                .iter()
                .any(|n| may_name_method(&n, &[b"last_match"]));
        }
        self.unsafe_binding |= node.as_alias_global_variable_node().is_some();
        if let Some(call) = node.as_call_node() {
            self.unsafe_binding |= binds_constant(&call, b"Regexp")
                || binds_method(&call, &[b"last_match"])
                || matches!(
                    call.name().as_slice(),
                    b"extend" | b"include" | b"prepend" | b"refine" | b"using" | b"eval" | b"module_eval" | b"class_eval" | b"instance_eval" | b"module_exec" | b"class_exec" | b"instance_exec"
                    // Reflective dispatch can hide eval, visibility or binders.
                    | b"send" | b"__send__" | b"public_send"
                )
                // A visible Method/UnboundMethod lookup can hide the binder
                // behind call/[]/bind_call, or store it for later invocation.
                // Protect at acquisition; do not attempt value-flow analysis.
                || (matches!(
                    call.name().as_slice(),
                    b"method" | b"public_method" | b"singleton_method"
                        | b"instance_method" | b"public_instance_method"
                ) && call.arguments().is_some_and(|a| {
                    a.arguments().iter().any(|n| may_name_method(&n, &[
                        b"define_method", b"define_singleton_method", b"alias_method",
                        b"attr", b"attr_reader", b"attr_accessor", b"undef_method", b"remove_method",
                        b"const_set", b"remove_const", b"autoload", b"private_constant",
                        b"private", b"protected", b"private_class_method",
                        b"extend", b"include", b"prepend", b"refine", b"using",
                        b"eval", b"module_eval", b"class_eval", b"instance_eval",
                        b"module_exec", b"class_exec", b"instance_exec",
                        b"send", b"__send__", b"public_send",
                    ]))
                }))
                || (matches!(
                    call.name().as_slice(),
                    b"private" | b"protected" | b"private_class_method"
                ) && call.arguments().is_some_and(|a| {
                    a.arguments()
                        .iter()
                        .any(|n| may_name_method(&n, &[b"last_match"]))
                }));
        }
        let namespace = node
            .as_class_node()
            .map(|n| n.constant_path())
            .or_else(|| node.as_module_node().map(|n| n.constant_path()));
        self.stack.push(namespace.is_some());
        if let Some(path) = namespace {
            // Object owns the top-level constants, including ::Regexp. A
            // nested namespace within Object still owns its own constants.
            self.namespaces
                .push(crate::cops::ast::is_global_const(&path, &["Object"]));
        }
    }
    fn leave(&mut self) {
        if self.stack.pop().unwrap() {
            self.namespaces.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/perl_backrefs_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/PerlBackrefs")
            .collect()
    }
    #[test]
    fn pinned_detection_and_safe_numbered_and_whole_match_corrections() {
        for (name, expression, safe) in [
            ("$1", "Regexp.last_match(1)", true),
            ("$9", "Regexp.last_match(9)", true),
            ("$&", "Regexp.last_match(0)", true),
            ("$`", "Regexp.last_match.pre_match", false),
            ("$'", "Regexp.last_match.post_match", false),
            ("$MATCH", "Regexp.last_match(0)", false),
            ("$PREMATCH", "Regexp.last_match.pre_match", false),
            ("$POSTMATCH", "Regexp.last_match.post_match", false),
        ] {
            let source = format!("# header\nputs {name}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 6, 2));
            assert_eq!(
                found[0].message,
                format!("Prefer `{expression}` over `{name}`.")
            );
            assert!(found[0].correction.is_none());
            if safe {
                assert_correction(
                    &PerlBackrefs,
                    &source,
                    &format!("# header\nputs {expression}\n"),
                );
            } else {
                assert!(
                    correction_offenses(&PerlBackrefs, &source)[0]
                        .correction
                        .is_none()
                );
            }
        }
    }
    #[test]
    fn braceless_and_explicit_interpolation_and_namespaces() {
        let found = offenses("module Foo\n  class Regexp\n  end\n\n  puts $1\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (5, 8));
        assert_eq!(
            found[0].message,
            "Prefer `::Regexp.last_match(1)` over `$1`."
        );
        for (before, after) in [
            ("\"#$1\"", "\"#{Regexp.last_match(1)}\""),
            ("`#$1`", "`#{Regexp.last_match(1)}`"),
            ("/#$1/", "/#{Regexp.last_match(1)}/"),
            ("\"#{$1}\"", "\"#{Regexp.last_match(1)}\""),
            ("%W[#$1]", "%W[#{Regexp.last_match(1)}]"),
            ("%I[#$1]", "%I[#{Regexp.last_match(1)}]"),
            (
                "module Foo\n  class Regexp\n  end\n  puts $1\nend\n",
                "module Foo\n  class Regexp\n  end\n  puts ::Regexp.last_match(1)\nend\n",
            ),
            (
                "class Foo; def f; $1; end; end",
                "class Foo; def f; ::Regexp.last_match(1); end; end",
            ),
        ] {
            assert_correction(&PerlBackrefs, before, after);
        }
    }
    #[test]
    fn visible_bindings_refinement_visibility_eval_and_defined_withhold_not_hide() {
        for binding in [
            "Regexp = Custom",
            "::Regexp = Custom",
            "Regexp ||= Custom",
            "Regexp, x = values",
            "Object::Regexp = Custom",
            "class Regexp; end",
            "class << Regexp; def last_match(*); :bad; end; end",
            "def Regexp.last_match(*); :bad; end",
            "alias last_match object_id",
            "undef last_match",
            "Regexp.extend(External)",
            "Regexp.singleton_class.prepend(External)",
            "class << Regexp; include External; end",
            "Regexp.define_singleton_method(:last_match) { :bad }",
            "Regexp.send(:define_singleton_method, :last_match) { :bad }",
            "Regexp.singleton_class.class_eval { alias_method :last_match, :object_id }",
            "Regexp.private_class_method(:last_match)",
            "Regexp.singleton_class.send(:private, :last_match)",
            "Object.const_set(:Regexp, Custom)",
            "Object.send(:remove_const, :Regexp)",
            "Object.private_constant(:Regexp)",
            "module M; refine Regexp.singleton_class do; def last_match(*); :bad; end; end; end; using M",
            "Regexp.instance_eval('def last_match(*); :bad; end')",
            "obj.instance_exec { $x = 1 }",
            "alias $capture $&",
        ] {
            let source = format!("{binding}\n$1\n");
            let found = correction_offenses(&PerlBackrefs, &source);
            assert_eq!(
                found.len(),
                if binding.starts_with("alias $") { 2 } else { 1 },
                "{source}"
            );
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        }
        for source in ["defined?($1)", "defined?($&)", "defined?(\"#$1\")"] {
            let found = correction_offenses(&PerlBackrefs, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        assert_correction(
            &PerlBackrefs,
            "module M; Regexp = Custom; $1; end",
            "module M; Regexp = Custom; ::Regexp.last_match(1); end",
        );
        assert_correction(
            &PerlBackrefs,
            "obj.last_match; $1",
            "obj.last_match; Regexp.last_match(1)",
        );
    }
    #[test]
    fn object_namespace_writes_mutate_the_root_regexp() {
        for namespace in ["Object", "::Object", ":: Object"] {
            for binding in [
                "Regexp = Custom",
                "Regexp ||= Custom",
                "Regexp &&= Custom",
                "Regexp += Custom",
                "Regexp, x = values",
                "class Regexp; end",
            ] {
                let source = format!("class {namespace}; {binding}; end; $1");
                let found = correction_offenses(&PerlBackrefs, &source);
                assert_eq!(found.len(), 1, "{source}");
                assert!(found[0].correction.is_none(), "{source}");
            }
        }
        assert_correction(
            &PerlBackrefs,
            "class Object; module M; Regexp = Custom; $1; end; end",
            "class Object; module M; Regexp = Custom; ::Regexp.last_match(1); end; end",
        );
    }
    #[test]
    fn method_object_binders_withhold_corrections() {
        for binding in [
            "Regexp.method(:define_singleton_method).call(:last_match) { :bad }",
            "Regexp.public_method('define_singleton_method')[:last_match] { :bad }",
            "binder = Regexp.method(:define_singleton_method); binder.call(:last_match) { :bad }",
            "Module.instance_method(:define_method).bind_call(Regexp.singleton_class, :last_match) { :bad }",
            "Object.method(:const_set).call(:Regexp, Custom)",
            "Regexp.singleton_class.method(:private).call(:last_match)",
            "Regexp.method(selector).call(:last_match) { :bad }",
        ] {
            let source = format!("{binding}; $1");
            let found = correction_offenses(&PerlBackrefs, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        assert_correction(
            &PerlBackrefs,
            "Regexp.method(:last_match); $1",
            "Regexp.method(:last_match); Regexp.last_match(1)",
        );
    }
    #[test]
    fn oversized_number_variables_are_always_nil_not_whole_matches() {
        // Prism stores zero for out-of-range references; that is not $&.
        for name in [
            "$1073741824",
            "$2147483647",
            "$2147483648",
            "$4294967295",
            "$999999999999999999999999999999",
        ] {
            let source = format!("/(a)/ =~ 'a'; p {name}\n");
            let found = correction_offenses(&PerlBackrefs, &source);
            assert_eq!(found.len(), 1);
            assert_eq!(
                found[0].message,
                format!("Prefer `Regexp.last_match({})` over `{name}`.", &name[1..])
            );
            assert!(found[0].correction.is_none(), "{source}");
        }
        assert_correction(
            &PerlBackrefs,
            "$1073741823",
            "Regexp.last_match(1073741823)",
        );
    }
    #[test]
    fn no_equivalent_or_non_read_lookalikes() {
        let found = offenses("alias $x $MATCH");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 10));
        for source in [
            "$+",
            "$LAST_PAREN_MATCH",
            "$~",
            "$0",
            "$MATCH = 1",
            "$MATCH ||= 1",
            "'$1' # $&\n",
            "Regexp.last_match(1)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
