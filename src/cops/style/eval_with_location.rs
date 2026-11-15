//! `Style/EvalWithLocation` — source metadata for literal eval strings.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
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
        let safe = cx.corrections_requested() && safe_context(&call, &args, cx);
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
                safe.then(|| Correction {
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

// Adding metadata is observable through __FILE__/__LINE__, caller, exceptions,
// definitions and arbitrary methods in the evaluated string. Do not execute or
// reparse it: a deliberately tiny grammar proves inert integer/bool/nil values.
// Whole-program isolation proves there are no visible custom constants, aliases,
// refinements, BEGIN hooks or eval/binding overrides. Unknown external monkey
// patches are outside this local proof, as for the other built-in API fixes.
// This is O(1) ancestor/root work plus O(local argument bytes), correction-only;
// no repeated source scans or per-node parses, even in a file with many evals.
fn safe_context(call: &CallNode<'_>, args: &[Node<'_>], cx: &NodeContext<'_, '_>) -> bool {
    let Some(root) = cx.ancestors().last().and_then(|n| n.as_program_node()) else {
        return false;
    };
    let body = root.statements().body();
    if body.len() != 1
        || body.iter().next().unwrap().location().start_offset() != call.location().start_offset()
        || body.iter().next().unwrap().location().end_offset() != call.location().end_offset()
        || call.block().is_some()
    {
        return false;
    }
    let name = call.name().as_slice();
    let base = if name == b"eval" { 2 } else { 1 };
    if args.len() < base || args.len() > base + 2 {
        return false;
    }
    if let Some(receiver) = call.receiver() {
        if !ast::is_global_const(
            &receiver,
            if name == b"eval" {
                &["Kernel"]
            } else {
                &["Object", "Module", "Class", "Kernel"]
            },
        ) {
            return false;
        }
    } else if !matches!(name, b"eval" | b"instance_eval") {
        return false;
    }
    if name == b"eval"
        && !args[1].as_call_node().is_some_and(|b| {
            b.name().as_slice() == b"binding"
                && b.receiver().is_none()
                && b.arguments().is_none()
                && b.block().is_none()
        })
    {
        return false;
    }
    let Some(code) = args[0].as_string_node() else {
        return false;
    };
    let code = code.unescaped();
    // A decoded CR can emit a parser warning whose location changes when we
    // supply metadata. Do not trim that observable behavior out of the proof.
    if code.contains(&b'\r') {
        return false;
    }
    let code = code.trim_ascii();
    let inert = matches!(code, b"nil" | b"true" | b"false" | b"0")
        || code.first().is_some_and(|b| matches!(b, b'1'..=b'9'))
            && code.iter().all(u8::is_ascii_digit);
    if !inert {
        return false;
    }
    // Replacement must not remove a heredoc opener (releasing its body as
    // executable Ruby), comments, or physical lines. Insertion alone does not
    // remove them, but use one atomic context proof for all this call's edits.
    if args[base..].iter().any(|arg| {
        arg.location()
            .as_slice()
            .iter()
            .any(|b| matches!(b, b'\n' | b'\r' | b'#'))
    }) {
        return false;
    }
    if let Some(filename) = args.get(base)
        && !matches!(filename, Node::SourceFileNode { .. })
        && !filename.as_string_node().is_some_and(|s| {
            !s.opening_loc()
                .is_some_and(|loc| loc.as_slice().starts_with(b"<<"))
                && s.unescaped().is_ascii()
                && !s.unescaped().contains(&0)
        })
    {
        return false;
    }
    args.get(base + 1).is_none_or(effect_free_line)
}
fn effect_free_line(node: &Node<'_>) -> bool {
    if matches!(node, Node::SourceLineNode { .. }) {
        return true;
    }
    if let Some(n) = node.as_integer_node() {
        let value = n.value();
        let (_, digits) = value.to_u32_digits();
        return digits.len() <= 1 && digits.first().copied().unwrap_or(0) <= 1_000_000;
    }
    if let Some(c) = node.as_call_node() {
        let args = ast::call_arguments(&c);
        return !c.is_safe_navigation()
            && c.block().is_none()
            && matches!(c.name().as_slice(), b"+" | b"-")
            && matches!(c.receiver(), Some(Node::SourceLineNode { .. }))
            && matches!(args.as_slice(), [arg] if arg.as_integer_node().is_some() && effect_free_line(arg));
    }
    false
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/eval_with_location_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Source-observable edits are withheld.
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == EvalWithLocation.name())
            .collect()
    }
    #[test]
    fn pinned_missing_location_receiver_and_heredoc_shapes() {
        for prefix in ["", "Kernel.", "::Kernel."] {
            for suffix in ["", ", binding", ", binding, __FILE__"] {
                let source = format!("{prefix}eval <<-CODE{suffix}\n  do_something\nCODE\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (1, 1));
                assert_eq!(
                    found[0].message,
                    "Pass a binding, `__FILE__`, and `__LINE__` to `eval`."
                );
                assert!(
                    correction_offenses(&EvalWithLocation, &source)[0]
                        .correction
                        .is_none()
                );
            }
        }
        for method in ["class_eval", "module_eval", "instance_eval"] {
            for receiver in ["C.", "M.", "foo.", ""] {
                let source = format!("{receiver}{method} <<-CODE\n  do_something\nCODE\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (1, 1));
                assert_eq!(
                    found[0].message,
                    format!("Pass `__FILE__` and `__LINE__` to `{method}`.")
                );
                assert!(
                    correction_offenses(&EvalWithLocation, &source)[0]
                        .correction
                        .is_none()
                );
            }
        }
    }
    #[test]
    fn pinned_incorrect_line_and_file_positions_messages_and_withheld_edits() {
        for (source, line, col, expected, actual) in [
            (
                "eval 'do_something', binding, __FILE__, __LINE__ + 1",
                1,
                41,
                "__LINE__",
                "__LINE__ + 1",
            ),
            (
                "eval <<-CODE, binding, __FILE__, __LINE__ + 2\n  do_something\nCODE\n",
                1,
                34,
                "__LINE__ + 1",
                "__LINE__ + 2",
            ),
            (
                "eval('puts 42',\n     binding,\n     __FILE__,\n     __LINE__)\n",
                4,
                6,
                "__LINE__ - 3",
                "__LINE__",
            ),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (line, col));
            assert_eq!(
                found[0].message,
                format!(
                    "Incorrect line number for `eval`; use `{expected}` instead of `{actual}`."
                )
            );
            assert!(
                correction_offenses(&EvalWithLocation, source)[0]
                    .correction
                    .is_none()
            );
        }
        for (method, binding) in [
            ("eval", ", binding"),
            ("instance_eval", ""),
            ("class_eval", ""),
            ("module_eval", ""),
        ] {
            let source = format!("{method} <<-CODE{binding}, 'foo', 'bar'\n  do_something\nCODE\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 2, "{source}");
            assert_eq!(
                (found[0].line, found[0].col),
                (1, source.find("'foo'").unwrap() as u32 + 1)
            );
            assert_eq!(
                (found[1].line, found[1].col),
                (1, source.find("'bar'").unwrap() as u32 + 1)
            );
            assert_eq!(
                found[0].message,
                format!("Incorrect file for `{method}`; use `__FILE__` instead of `'foo'`.")
            );
            assert_eq!(
                found[1].message,
                format!(
                    "Incorrect line number for `{method}`; use `__LINE__ + 1` instead of `'bar'`."
                )
            );
            assert!(
                correction_offenses(&EvalWithLocation, &source)
                    .iter()
                    .all(|o| o.correction.is_none())
            );
        }
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
    fn safe_navigation_line_calls_report_but_ordinary_calls_are_exempt() {
        for (method, binding) in [("eval", ", binding"), ("module_eval", "")] {
            let source = format!("{method}('42'{binding}, __FILE__, obj&.lineno)");
            let found = correction_offenses(&EvalWithLocation, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col),
                (1, source.find("obj&.").unwrap() as u32 + 1)
            );
            assert_eq!(
                found[0].message,
                format!(
                    "Incorrect line number for `{method}`; use `__LINE__` instead of `obj&.lineno`."
                )
            );
            assert!(found[0].correction.is_none());
            assert_correction(&EvalWithLocation, &source, &source);
            assert!(offenses(&source.replace("obj&.", "obj.")).is_empty());
        }
    }
    #[test]
    fn warning_producing_decoded_carriage_returns_withhold_corrections() {
        for source in [
            "eval(\"42\\r\", binding)",
            "eval(\"\\r42\", binding, 'old.rb', 99)",
            "Object.class_eval(\"42\\x0d\")",
            "Module.module_eval(\"42\\r\\n\")",
            "instance_eval(\"false\\r\")",
        ] {
            let found = correction_offenses(&EvalWithLocation, source);
            assert!(!found.is_empty(), "{source}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
            assert_correction(&EvalWithLocation, source, source);
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
    #[test]
    fn all_sixteen_pinned_correction_examples_are_individually_withheld() {
        // Unlike upstream, do not change observable locations of unknown code,
        // dispatch through unknown receivers, or remove invalid/effectful args.
        for source in [
            "eval <<-CODE, binding\n  do_something\nCODE\n",
            "eval <<-CODE, binding, __FILE__\n  do_something\nCODE\n",
            "eval(<<-CODE, binding, __FILE__)\n  do_something\nCODE\n",
            "eval 'do_something', binding, __FILE__, __LINE__ + 1\n",
            "eval <<-CODE, binding, __FILE__, __LINE__ + 2\n  do_something\nCODE\n",
            "eval('puts 42',\n     binding,\n     __FILE__,\n     __LINE__)\n",
            "C.class_eval <<-CODE\n  do_something\nCODE\n",
            "M.module_eval <<-CODE\n  do_something\nCODE\n",
            "foo.instance_eval <<-CODE\n  do_something\nCODE\n",
            "instance_eval('@foo = foo')\n",
            "C.class_eval <<-CODE, __FILE__, __LINE__\n  do_something\nCODE\n",
            "eval <<-CODE, binding, 'foo', 'bar'\n  do_something\nCODE\n",
            "instance_eval <<-CODE, 'foo', 'bar'\n  do_something\nCODE\n",
            "class_eval <<-CODE, 'foo', 'bar'\n  do_something\nCODE\n",
            "module_eval <<-CODE, 'foo', 'bar'\n  do_something\nCODE\n",
            "module_eval <<-CODE, __FILE__, 'bar'\n  do_something\nCODE\n",
        ] {
            assert_correction(&EvalWithLocation, source, source);
            assert!(
                correction_offenses(&EvalWithLocation, source)
                    .iter()
                    .all(|o| o.correction.is_none())
            );
        }
    }
    #[test]
    fn multiline_and_heredoc_metadata_never_release_body_text_or_shift_lines() {
        for source in [
            "eval('42', binding, <<~NAME, 99)\nside_effect()\nNAME\n",
            "eval('42', binding, \"file\nname\", 99)",
            "eval('42', binding, __FILE__, __LINE__ +\n 1)",
            "eval('42', binding, __FILE__, __LINE__ + <<~NAME)\n1\nNAME\n",
        ] {
            assert_correction(&EvalWithLocation, source, source);
            assert!(
                correction_offenses(&EvalWithLocation, source)
                    .iter()
                    .all(|o| o.correction.is_none()),
                "{source}"
            );
        }
    }
    #[test]
    fn many_eval_calls_have_no_repeated_source_proof_or_code_parses() {
        // safe_context rejects this root in O(1) per call (body.len != 1),
        // before inspecting code bytes. Detection still visits every call.
        let source = "Kernel.eval('42', binding)\n".repeat(4000);
        let found = correction_offenses(&EvalWithLocation, &source);
        assert_eq!(found.len(), 4000);
        assert!(found.iter().all(|o| o.correction.is_none()));
    }
    #[test]
    fn source_file_keyword_is_a_literal_code_string_but_never_corrected() {
        for source in [
            "eval(__FILE__)",
            "Kernel.eval(__FILE__, binding)",
            "Object.class_eval(__FILE__)",
            "module_eval(__FILE__, __FILE__, __LINE__ + 1)",
        ] {
            let found = correction_offenses(&EvalWithLocation, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn source_observation_context_custom_binding_and_effects_withhold_not_hide() {
        for source in [
            "eval '42'",
            "eval('42', custom_binding)",
            "eval('42', binding, filename())",
            "eval('42', binding, nil, 1)",
            "eval('42', binding, 'x', 'bar')",
            "eval('42', binding, 'wrong', 9999999999999999999999999)",
            "eval('42', binding, 'wrong', 1 << 100)",
            "eval('42', binding, 'wrong', false)",
            "eval('42', binding, 'wrong', __LINE__ + 9999999999999999999999)",
            "eval('42', binding, 'x', calculate + 1)",
            "eval('42', binding, 'x', 1 + side_effect)",
            "obj.instance_eval('42')",
            "C.class_eval('42')",
            "M.module_eval('42')",
            "Kernel = Custom; Kernel.eval('42', binding)",
            "def eval(*args); :custom; end; eval('42', binding)",
            "module M; def self.instance_eval(*args); end; instance_eval('42'); end",
            "Object.class_eval('42') { side_effect }",
            "eval('42', binding, 'x', 99, extra)",
            "eval(\"#{observe}\", binding)",
            "eval('__FILE__', binding)",
            "eval('__LINE__', binding)",
            "eval('caller', binding)",
            "eval('proc {}.source_location', binding)",
            "eval('raise', binding)",
            "eval('/x/ =~ text', binding)",
            "eval('def method; end', binding)",
            "eval('42', (binding))",
            "eval('42', binding, 'wrong', __LINE__ + side_effect())",
            "BEGIN { def eval(*args); :custom; end }; eval('42', binding)",
        ] {
            let found = correction_offenses(&EvalWithLocation, source);
            assert!(!found.is_empty(), "{source}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        }
    }
}
