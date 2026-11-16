//! `Style/EvalWithLocation` — source metadata for literal eval strings.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct EvalWithLocation;
impl Cop for EvalWithLocation {
    fn name(&self) -> &'static str {
        "Style/EvalWithLocation"
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
        let name = call.name().as_slice();
        if call.is_safe_navigation()
            || !matches!(
                name,
                b"eval" | b"class_eval" | b"module_eval" | b"instance_eval"
            )
        {
            return;
        }
        if name == b"eval"
            && call
                .receiver()
                .is_some_and(|r| !ast::is_global_const(&r, &["Kernel"]))
        {
            return;
        }
        let args = ast::call_arguments(&call);
        let Some(code) = args.first() else { return };
        if code.as_string_node().is_none()
            && code.as_interpolated_string_node().is_none()
            && !matches!(code, Node::SourceFileNode { .. })
        {
            return;
        }
        let method = std::str::from_utf8(name).expect("known selector");
        let base = if name == b"eval" { 2 } else { 1 };
        let filename = args.get(base);
        let line_arg = args.get(base + 1);
        let safe = cx.corrections_requested();
        let mut emit =
            |target: &Node<'_>, end: usize, message: String, correction: Option<Correction>| {
                let (line, col) = file.line_col(target.location().start_offset());
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    line,
                    col,
                    end_line: file.line_of(end.saturating_sub(1)),
                    message,
                    correction,
                });
            };
        if let Some(filename) = filename
            && !matches!(filename, Node::SourceFileNode { .. })
        {
            emit(
                filename,
                ast::source_end(filename),
                format!(
                    "Incorrect file for `{method}`; use `__FILE__` instead of `{}`.",
                    String::from_utf8_lossy(filename.location().as_slice())
                ),
                safe.then(|| replacement(filename, "__FILE__".into())),
            );
        }
        if line_arg.is_some() {
            // Upstream deliberately checks the last argument, even for invalid arity.
            let line = args.last().unwrap();
            if ast::is_variable(line)
                || line
                    .as_call_node()
                    .is_some_and(|c| !c.is_safe_navigation() && c.name().as_slice() != b"+")
            {
                return;
            }
            let diff = i64::from(string_first_line(code, file))
                - i64::from(file.line_of(line.location().start_offset()));
            if correct_line(line, diff) {
                return;
            }
            let expected = expected_line(diff);
            emit(
                line,
                ast::source_end(line),
                format!(
                    "Incorrect line number for `{method}`; use `{expected}` instead of `{}`.",
                    String::from_utf8_lossy(line.location().as_slice())
                ),
                safe.then(|| replacement(line, expected)),
            );
        } else {
            let last = args.last().unwrap();
            let expected = expected_line(
                i64::from(string_first_line(code, file))
                    - i64::from(file.line_of(last.location().start_offset())),
            );
            let message = if name == b"eval" {
                "Pass a binding, `__FILE__`, and `__LINE__` to `eval`.".into()
            } else {
                format!("Pass `__FILE__` and `__LINE__` to `{method}`.")
            };
            let offset = ast::source_end(last);
            emit(
                node,
                ast::source_end(node),
                message,
                // An `eval` without a binding gets no correction.
                (safe && (name != b"eval" || args.len() >= 2)).then(|| Correction {
                    range: offset..offset,
                    replacement: if filename.is_some() {
                        format!(", {expected}")
                    } else {
                        format!(", __FILE__, {expected}")
                    },
                }),
            );
        }
    }
}
fn replacement(node: &Node<'_>, replacement: String) -> Correction {
    Correction {
        range: node.location().start_offset()..ast::source_end(node),
        replacement,
    }
}
fn expected_line(diff: i64) -> String {
    match diff.cmp(&0) {
        std::cmp::Ordering::Equal => "__LINE__".into(),
        std::cmp::Ordering::Greater => format!("__LINE__ + {diff}"),
        std::cmp::Ordering::Less => format!("__LINE__ - {}", -diff),
    }
}
fn string_first_line(code: &Node<'_>, file: &SourceFile) -> u32 {
    if let Some(s) = code.as_string_node()
        && s.opening_loc()
            .is_some_and(|l| l.as_slice().starts_with(b"<<"))
    {
        return file.line_of(s.content_loc().start_offset());
    }
    if let Some(s) = code.as_interpolated_string_node()
        && s.opening_loc()
            .is_some_and(|l| l.as_slice().starts_with(b"<<"))
    {
        return file.line_of(s.parts().iter().next().map_or_else(
            || s.closing_loc().unwrap().start_offset(),
            |p| p.location().start_offset(),
        ));
    }
    file.line_of(code.location().start_offset())
}
fn integer_equals(node: &Node<'_>, number: u32) -> bool {
    node.as_integer_node().is_some_and(|n| {
        let value = n.value();
        let (negative, digits) = value.to_u32_digits();
        !negative && digits == [number]
    })
}
fn correct_line(line: &Node<'_>, diff: i64) -> bool {
    if diff == 0 {
        return matches!(line, Node::SourceLineNode { .. });
    }
    let Some(call) = line.as_call_node() else {
        return false;
    };
    let args = ast::call_arguments(&call);
    let (Some(receiver), [arg]) = (call.receiver(), args.as_slice()) else {
        return false;
    };
    !call.is_safe_navigation()
        && call.block().is_none()
        && call.name().as_slice() == if diff > 0 { b"+" } else { b"-" }
        && ((matches!(receiver, Node::SourceLineNode { .. })
            && integer_equals(arg, diff.unsigned_abs() as u32))
            || (matches!(arg, Node::SourceLineNode { .. })
                && integer_equals(&receiver, diff.unsigned_abs() as u32)))
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/eval_with_location_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Source-observable edits are withheld.
    use super::*;
    use crate::pipeline::lint_str;
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == EvalWithLocation.name())
            .collect()
    }
    #[test]
    fn pinned_correct_locations_dynamic_code_blocks_and_line_sources() {
        for source in [
            "foo.eval 'CODE'",
            "self.eval 'CODE'",
            "Other::Kernel.eval 'CODE'",
            "code = something; eval code",
            "eval code",
            "eval",
            "eval <<-CODE, binding, __FILE__, __LINE__ + 1\n  do_something\nCODE\n",
            "eval 'do_something', binding, __FILE__, __LINE__",
            "eval 'do_something', binding, __FILE__,\n     __LINE__ - 1",
            "module_eval(<<~CODE, __FILE__, lineno)\n  do_something\nCODE\n",
            "lineno = calc\nmodule_eval(<<~CODE, __FILE__, lineno)\n  do_something\nCODE\n",
            "def self.included(base); base.class_eval do; include OtherModule; end; end",
            "module_eval 'code', __FILE__, @line",
            "module_eval 'code', __FILE__, $line",
            "module_eval 'code', __FILE__, @@line",
            "module_eval <<~CODE, __FILE__, 1 + __LINE__\n  do_something\nCODE\n",
            "module_eval 'code', __FILE__, calculate()",
            "Kernel&.eval '42'",
            "C&.class_eval '42'",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn interpolation_heredoc_and_multiple_missing_findings() {
        for (source, count) in [
            ("eval(\"#{code}\", binding)", 1),
            ("instance_eval '@foo = foo'", 1),
            (
                "eval(<<-CODE, binding, __FILE__)\n  do_something\nCODE\n",
                1,
            ),
            (
                "module_eval <<-CODE, __FILE__, 'bar'\n  do_something\nCODE\n",
                1,
            ),
            ("module_eval '42', 'wrong'", 2),
        ] {
            assert_eq!(offenses(source).len(), count, "{source}");
        }
    }
}
