//! `Style/ArrayCoercion`: unsafe, opt-in array coercion suggestions only.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ArrayCoercion;
impl Cop for ArrayCoercion {
    fn name(&self) -> &'static str {
        "Style/ArrayCoercion"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let message = if let Some(array) = node.as_array_node() {
            if array.opening_loc().is_none_or(|l| l.as_slice() != b"[")
                || array.elements().len() != 1
            {
                return;
            }
            let Some(arg) = array
                .elements()
                .first()
                .and_then(|n| n.as_splat_node())
                .and_then(|n| n.expression())
            else {
                return;
            };
            let arg = String::from_utf8_lossy(arg.location().as_slice());
            format!("Use `Array({arg})` instead of `[*{arg}]`.")
        } else {
            let Some(name) = explicit_check(node) else {
                return;
            };
            format!(
                "Use `Array({})` instead of explicit `Array` check.",
                String::from_utf8_lossy(&name)
            )
        };
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction: None,
        });
    }
}
// Prism has separate Unless/Else/Statements nodes; only Statements is
// transparent in the pinned `(if condition nil assignment)` pattern.
fn explicit_check(node: &Node<'_>) -> Option<Vec<u8>> {
    let (predicate, body) = if let Some(n) = node.as_unless_node() {
        if n.else_clause().and_then(|e| e.statements()).is_some() {
            return None;
        }
        (n.predicate(), n.statements()?)
    } else {
        let n = node.as_if_node()?;
        if n.statements().is_some() {
            return None;
        }
        (n.predicate(), n.subsequent()?.as_else_node()?.statements()?)
    };
    let call = predicate.as_call_node()?;
    if call.is_safe_navigation() || call.block().is_some() || call.name().as_slice() != b"is_a?" {
        return None;
    }
    let receiver = call.receiver()?.as_local_variable_read_node()?;
    let args = ast::call_arguments(&call);
    let [arg] = args.as_slice() else {
        return None;
    };
    if !arg
        .as_constant_read_node()
        .is_some_and(|c| c.name().as_slice() == b"Array")
        || body.body().len() != 1
    {
        return None;
    }
    let assignment = body.body().first()?.as_local_variable_write_node()?;
    let array = assignment.value().as_array_node()?;
    if array.elements().len() != 1 {
        return None;
    }
    let value = array.elements().first()?.as_local_variable_read_node()?;
    (receiver.name().as_slice() == assignment.name().as_slice()
        && value.name().as_slice() == assignment.name().as_slice())
    .then(|| receiver.name().as_slice().to_vec())
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop array_coercion_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(&format!("# rustocop:enable Style/ArrayCoercion\n{s}"))
            .into_iter()
            .filter(|o| o.cop == ArrayCoercion.name())
            .collect()
    }
    #[test]
    fn pinned_splat_diagnostic() {
        let o = offenses("  [*paths].each { |path| do_something(path) }\n");
        assert_eq!(o.len(), 1);
        assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, 3, 2));
        assert_eq!(o[0].message, "Use `Array(paths)` instead of `[*paths]`.");
    }
    #[test]
    fn pinned_explicit_check_diagnostic() {
        let o = offenses("paths = [paths] unless paths.is_a?(Array)\n");
        assert_eq!(o.len(), 1);
        assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, 1, 2));
        assert_eq!(
            o[0].message,
            "Use `Array(paths)` instead of explicit `Array` check."
        );
    }
    #[test]
    fn pinned_no_offenses_and_structural_lookalikes() {
        for s in [
            "first_path, rest = *paths",
            "[*paths, '/root'].each { |path| do_something(path) }",
            "other_paths = [paths] unless paths.is_a?(Array)",
            "[]",
            "[paths]",
            "[*]",
            "paths = [paths] if paths.is_a?(Array)",
            "paths = [other] unless paths.is_a?(Array)",
            "paths = [paths] unless paths.kind_of?(Array)",
            "paths = [paths] unless paths.is_a?(::Array)",
            "paths = [paths] unless paths.is_a?(Other::Array)",
            "@paths = [@paths] unless @paths.is_a?(Array)",
            "paths = [paths] unless paths&.is_a?(Array)",
            "paths = [paths] unless (paths.is_a?(Array))",
            "paths = []; paths.is_a?(Array) ? nil : paths = [paths]",
            "unless paths.is_a?(Array); paths = [paths]; effect; end",
            "unless paths.is_a?(Array); paths = [paths]; else; effect; end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn check_forms_preserve_local_identity_and_exact_array_constant_shape() {
        for s in [
            "paths = []; unless paths.is_a?(Array)\n paths = [paths]\nend",
            "paths = []; if paths.is_a?(Array)\nelse\n paths = [paths]\nend",
            "paths = []; paths = [paths] unless paths.is_a?(Array)",
            "Array = Other; paths = [paths] unless paths.is_a?(Array)",
            "def f(paths); paths = [paths] unless paths.is_a?(Array); end",
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!(
                o[0].message,
                "Use `Array(paths)` instead of explicit `Array` check."
            );
        }
    }
    #[test]
    fn empty_else_is_absent_but_explicit_nil_and_call_receivers_are_not_locals() {
        let o = offenses("paths = []; unless paths.is_a?(Array); paths = [paths]; else; end");
        assert_eq!(o.len(), 1);
        for s in [
            "paths = []; unless paths.is_a?(Array); paths = [paths]; else; nil; end",
            "unless paths.is_a?(Array); paths = [paths]; end",
            "paths = []; paths.is_a?(Array) ? nil : paths = [paths]",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn splat_values_nested_arrays_and_evaluation_are_detection_only() {
        for s in [
            "[*nil]",
            "[*[]]",
            "[*[1, [2]]]",
            "[*effect()]",
            "[*{a: 1}]",
            "[*%w[a\\ b c]]",
            "[*%i[a b]]",
            "[*paths,]",
            "[*[ *paths ]]",
        ] {
            let o = crate::cops::testing::correction_offenses(&ArrayCoercion, s);
            assert!(!o.is_empty(), "{s}");
            assert!(o.iter().all(|o| o.correction.is_none()), "{s}");
        }
        assert!(!ArrayCoercion.safe_fixes());
        assert!(!ArrayCoercion.default_enabled());
    }
}
