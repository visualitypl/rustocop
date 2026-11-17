//! `Style/RedundantArrayConstructor` — literal constructors with conservative core-only fixes.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantArrayConstructor;
impl Cop for RedundantArrayConstructor {
    fn name(&self) -> &'static str {
        "Style/RedundantArrayConstructor"
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
            if let Some(getter) = ast::compound_index_getter(node)
                && !getter.safe_navigation
                && ast::is_global_const(&getter.receiver, &["Array"])
            {
                let loc = getter.receiver.location();
                let (line, col) = file.line_col(loc.start_offset());
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    message: "Remove the redundant `Array` constructor.".into(),
                    line,
                    col,
                    end_line: file.last_line_of(loc),
                    // The getter belongs to an assignment target, not an
                    // expression: replacing it with a literal makes invalid Ruby.
                    correction: None,
                });
            }
            return;
        };
        let Some(form) = candidate(&call) else {
            return;
        };
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (start, end) = match form {
            Form::New => (
                call.receiver().unwrap().location().start_offset(),
                selector.end_offset(),
            ),
            Form::Kernel => (selector.start_offset(), selector.end_offset()),
            Form::Brackets => {
                let receiver = call.receiver().unwrap();
                (
                    receiver.location().start_offset(),
                    receiver.location().end_offset(),
                )
            }
        };
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Remove the redundant `Array` constructor.".into(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: cx
                .corrections_requested()
                .then(|| correction(&call, form, file))
                .flatten(),
        });
    }
}
#[derive(Clone, Copy)]
enum Form {
    New,
    Kernel,
    Brackets,
}
fn candidate(call: &ruby_prism::CallNode<'_>) -> Option<Form> {
    if call.is_safe_navigation() {
        return None;
    }
    let args = ast::call_arguments(call);
    match (call.name().as_slice(), call.receiver()) {
        (b"[]", Some(receiver)) if ast::is_global_const(&receiver, &["Array"]) => {
            Some(Form::Brackets)
        }
        (b"new", Some(receiver))
            if ast::is_global_const(&receiver, &["Array"])
                && matches!(args.as_slice(),[a] if a.as_array_node().is_some()) =>
        {
            Some(Form::New)
        }
        (b"Array", None) if matches!(args.as_slice(),[a] if a.as_array_node().is_some()) => {
            Some(Form::Kernel)
        }
        _ => None,
    }
}
/// RuboCop's corrector: the call becomes its array literal, or `[args]`.
fn correction(
    call: &ruby_prism::CallNode<'_>,
    form: Form,
    file: &SourceFile,
) -> Option<Correction> {
    let loc = call.location();
    let args = ast::call_arguments(call);
    let replacement = match form {
        Form::New | Form::Kernel => std::str::from_utf8(args[0].location().as_slice())
            .ok()?
            .to_owned(),
        Form::Brackets => {
            // From the selector to the end of the call.
            let start = match call.call_operator_loc() {
                Some(_) => call.message_loc()?.start_offset(),
                None => call.opening_loc()?.start_offset(),
            };
            std::str::from_utf8(&file.bytes[start..loc.end_offset()])
                .ok()?
                .to_owned()
        }
    };
    Some(Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement,
    })
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop redundant_array_constructor_spec.rb (MIT
    //! License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::correction_offenses;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == RedundantArrayConstructor.name())
            .collect()
    }
    #[test]
    fn compound_assignment_getters_report_without_literal_lhs_corrections() {
        for op in ["+=", "||=", "&&="] {
            for getter in ["Array[0]", "::Array[0]", "Array[]", "Array[0, 1]"] {
                let s = format!("# header\n  {getter} {op} rhs\n");
                let o = correction_offenses(&RedundantArrayConstructor, &s);
                assert_eq!(o.len(), 1, "{s}");
                assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, 3, 2));
                assert_eq!(o[0].message, "Remove the redundant `Array` constructor.");
                assert!(o[0].correction.is_none(), "{s}");
                assert!(offenses(&format!("{getter} = rhs")).is_empty());
            }
            for getter in ["Other::Array[0]", "obj[0]"] {
                assert!(offenses(&format!("{getter} {op} rhs")).is_empty());
            }
        }
    }
    #[test]
    fn pinned_positions_and_messages() {
        for s in [
            "Array.new([])",
            "::Array.new([])",
            "Array[]",
            "::Array[]",
            "Array([])",
            "Array.new(['foo', 'bar', 'baz'])",
            "Array['foo', 'bar', 'baz']",
            "Array(['foo', 'bar', 'baz'])",
        ] {
            let o = offenses(&format!("# header\n  {s}\n"));
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, 3, 2));
            assert_eq!(o[0].message, "Remove the redundant `Array` constructor.");
        }
    }
    #[test]
    fn pinned_no_offenses_and_exact_receiver_arity_shapes() {
        for s in [
            "[]",
            "Array.new(array)",
            "Array(array)",
            "Array.new(3, 'foo')",
            "Array.new(3) { 'foo' }",
            "Array.new",
            "Array()",
            "Array.new([], [])",
            "Kernel.Array([])",
            "Other::Array.new([])",
            "Other.Array([])",
            "Array.new(([]))",
            "Array.new([], &b)",
            "Array([], &b)",
            "Array&.new([])",
            "Array&.[]()",
            "Array.new(*[[]])",
            "Array(**{})",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
