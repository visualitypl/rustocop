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
                .then(|| correction(&call, form, cx, file))
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
/// Exact trusted API shape only. The shared index separately proves all child
/// values closed and rejects source-visible overriding/loading/refinements.
pub(super) fn primitive_api(call: &ruby_prism::CallNode<'_>) -> bool {
    candidate(call).is_some()
        && call.block().is_none()
        && !ast::call_arguments(call)
            .iter()
            .any(|n| n.as_keyword_hash_node().is_some())
}
fn correction(
    call: &ruby_prism::CallNode<'_>,
    form: Form,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    if !primitive_api(call) {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    let facts = file.format_string_facts(root);
    let loc = call.location();
    if !facts.core_before(loc.start_offset())
        || !facts.closed(&call.as_node())
        || loc.as_slice().contains(&b'\n')
        || super::correction::contains_comment(file, loc.start_offset()..loc.end_offset())
        || file.contains_heredoc(root, &call.as_node())
        || file.heredoc_pending_at(root, loc.start_offset())
    {
        return None;
    }
    let args = ast::call_arguments(call);
    // Ruby 3.4 compiles a direct string call argument differently from an
    // array-literal element when frozen_string_literal is unspecified: moving
    // the token introduces a chilled-string mutation warning. Neither closed
    // core dispatch nor frozen? proves this compiler-context property.
    // Adjacent string tokens are an InterpolatedStringNode even without
    // interpolation. In a literal element Ruby 3.4 can freeze their result even
    // under explicit false. Only explicit true proves that move; keep other
    // direct interpolated/concatenated argument shapes conservative as well.
    if matches!(form, Form::Brackets)
        && args.iter().any(|arg| {
            (arg.as_string_node().is_some() && file.frozen_string_setting().is_none())
                || (arg.as_interpolated_string_node().is_some()
                    && file.frozen_string_setting() != Some(true))
        })
    {
        return None;
    }
    let replacement = match form {
        Form::New | Form::Kernel => std::str::from_utf8(args[0].location().as_slice())
            .ok()?
            .to_owned(),
        Form::Brackets => {
            // Keep exact argument token bytes, including significant escaped
            // whitespace. Never interpret percent arrays or synthesize elements.
            let interior =
                if let (Some(open), Some(close)) = (call.opening_loc(), call.closing_loc()) {
                    &file.bytes[open.end_offset()..close.start_offset()]
                } else if let (Some(first), Some(last)) = (args.first(), args.last()) {
                    &file.bytes[first.location().start_offset()..last.location().end_offset()]
                } else {
                    b""
                };
            format!("[{}]", std::str::from_utf8(interior).ok()?)
        }
    };
    super::literal_spelling::safe_edit(
        file,
        cx,
        Correction {
            range: loc.start_offset()..loc.end_offset(),
            replacement,
        },
    )
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop redundant_array_constructor_spec.rb (MIT
    //! License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
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
    fn unspecified_frozen_policy_keeps_direct_string_arguments_reported_but_unedited() {
        for literal in ["\"x\"", "'x'", "?x", "%q(x)", "%Q(x)", "\"\\xFF\""] {
            for constructor in ["Array[EXPR]", "::Array[EXPR]", "Array.[](EXPR)"] {
                let s = format!("# header\n  {}\n", constructor.replace("EXPR", literal));
                let o = correction_offenses(&RedundantArrayConstructor, &s);
                assert_eq!(o.len(), 1, "{s}");
                assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, 3, 2));
                assert_eq!(o[0].message, "Remove the redundant `Array` constructor.");
                assert!(o[0].correction.is_none(), "{s}");
            }
        }
    }
    #[test]
    fn adjacent_string_arguments_need_frozen_true_even_with_explicit_mutable_policy() {
        // Ruby 3.4 also treats an unparenthesized adjacent-string expression
        // differently in array elements (frozen) and call arguments (mutable).
        for magic in ["", "# frozen_string_literal: false\n"] {
            for expr in ["\"x\" \"y\"", "'x' 'y'", "\"x\" 'y'", "\"x\" \\\n\"y\""] {
                let s = format!("{magic}Array[{expr}]\n");
                let o = correction_offenses(&RedundantArrayConstructor, &s);
                assert_eq!(o.len(), 1, "{s}");
                assert!(o[0].correction.is_none(), "{s}");
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
    fn all_eight_pinned_before_after_pairs() {
        for (before, after) in [
            ("Array.new([])", "[]"),
            ("::Array.new([])", "[]"),
            ("Array[]", "[]"),
            ("::Array[]", "[]"),
            ("Array([])", "[]"),
            ("Array.new(['foo', 'bar', 'baz'])", "['foo', 'bar', 'baz']"),
            ("Array(['foo', 'bar', 'baz'])", "['foo', 'bar', 'baz']"),
        ] {
            assert_correction(&RedundantArrayConstructor, before, after);
            assert!(offenses(after).is_empty());
        }
        // The eighth pinned pair changes chilled warnings without a policy.
        // Keep its offense, but require explicit false/true for the canonical.
        let before = "Array['foo', 'bar', 'baz']";
        assert!(
            correction_offenses(&RedundantArrayConstructor, before)[0]
                .correction
                .is_none()
        );
        for value in ["false", "true"] {
            assert_correction(
                &RedundantArrayConstructor,
                &format!("# frozen_string_literal: {value}\n{before}"),
                &format!("# frozen_string_literal: {value}\n['foo', 'bar', 'baz']"),
            );
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
    #[test]
    fn canonical_nested_literals_percent_arrays_and_exact_token_bytes() {
        for (before, after) in [
            (
                "x = Array.new([nil, false, [1, :a], {a: 1}])",
                "x = [nil, false, [1, :a], {a: 1}]",
            ),
            ("Array(%w[a\\ b c\\]d])", "%w[a\\ b c\\]d]"),
            ("Array.new(%i[a b])", "%i[a b]"),
            ("Array.new(%W[a b])", "%W[a b]"),
            ("Array[[], [1], :x]", "[[], [1], :x]"),
            ("Array.[]()", "[]"),
            ("Array.[](1, [2])", "[1, [2]]"),
            ("Array []", "[]"),
            ("Array.new [1]", "[1]"),
            ("Array.new([?\\ ])", "[?\\ ]"),
            ("Array.new([]); Array([]); Array[]", "[]; []; []"),
        ] {
            assert_correction(&RedundantArrayConstructor, before, after);
        }
    }
    #[test]
    fn frozen_controls_and_unchanged_child_contexts_retain_canonical_corrections() {
        for literal in ["\"x\"", "'x'", "?x", "%q(x)", "%Q(x)"] {
            for value in ["false", "true"] {
                for constructor in ["Array[EXPR]", "::Array[EXPR]", "Array.[](EXPR)"] {
                    assert_correction(
                        &RedundantArrayConstructor,
                        &format!(
                            "# frozen_string_literal: {value}\n{}",
                            constructor.replace("EXPR", literal)
                        ),
                        &format!("# frozen_string_literal: {value}\n[{literal}]"),
                    );
                }
            }
        }
        for (before, after) in [
            ("Array[(\"x\")]", "[(\"x\")]"),
            ("Array[(?x)]", "[(?x)]"),
            ("Array[(\"x\" \"y\")]", "[(\"x\" \"y\")]"),
            ("Array[[\"x\"]]", "[[\"x\"]]"),
            ("Array[%w[x]]", "[%w[x]]"),
            ("Array[{a: \"x\"}]", "[{a: \"x\"}]"),
            ("Array[\"x\".strip]", "[\"x\".strip]"),
            ("Array.new([?x])", "[?x]"),
            ("Array([\"x\"])", "[\"x\"]"),
            ("Array.new([\"x\" \"y\"])", "[\"x\" \"y\"]"),
            (
                "# frozen_string_literal: true\nArray[\"x\" \"y\"]",
                "# frozen_string_literal: true\n[\"x\" \"y\"]",
            ),
        ] {
            assert_correction(&RedundantArrayConstructor, before, after);
        }
    }
    #[test]
    fn uncertain_dispatch_effects_blocks_binding_and_source_boundaries_withhold_not_hide() {
        for s in [
            "Array = Other; Array.new([])",
            "class X; Array = Other; Array.new([]); end",
            "class Array; def self.new(x); :changed; end; end; Array.new([])",
            "def Array(x); :changed; end; Array([])",
            "def f; Array.new([]); end",
            "module R; refine Array.singleton_class do; def new(x); nil; end; end; end; using R; Array.new([])",
            "require 'other'; Array.new([])",
            "load 'other'; Array[]",
            "Array.new([effect()])",
            "Array([x])",
            "Array[*x]",
            "Array.new([*[]])",
            "Array.new([x = []])",
            "Array.new([\"#{effect}\"])",
            "Array.new([]) { effect }",
            "Array([]) { effect }",
            "Array[] { effect }",
            "Array[&b]",
            "Array[k: 1]",
            "Array.new([<<~TEXT])\nx\nTEXT\n",
            "Array.new( # keep\n []\n)",
            "Array.new([\n1\n])",
            "Array( # keep\n[])",
            "consume(<<~TEXT, Array.new([]))\nx\nTEXT\n",
        ] {
            let o = correction_offenses(&RedundantArrayConstructor, s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(o[0].correction.is_none(), "{s}");
        }
        let o = correction_offenses(&RedundantArrayConstructor, "Array[]; unknown; Array[]");
        assert!(o[0].correction.is_some());
        assert!(o[1].correction.is_none());
    }
    #[test]
    fn source_heap_code_data_guards_and_line_preserving_envelopes() {
        for suffix in [
            "p File.binread(__FILE__)",
            "p ObjectSpace.each_object(Array).to_a",
            "p RubyVM::InstructionSequence.of(method(:f))",
            "p DATA.pos\n__END__\npayload",
        ] {
            let s = format!("Array.new([])\n{suffix}\n");
            let o = correction_offenses(&RedundantArrayConstructor, &s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(o[0].correction.is_none(), "{s}");
        }
        for (before, after) in [
            (
                "\u{feff}Array.new([])\r\n__LINE__\r\n__END__\r\npayload\0\r\n",
                "\u{feff}[]\r\n__LINE__\r\n__END__\r\npayload\0\r\n",
            ),
            (
                "# frozen_string_literal: true\nArray.new(['x'])\n",
                "# frozen_string_literal: true\n['x']\n",
            ),
        ] {
            assert_correction(&RedundantArrayConstructor, before, after);
        }
    }
}
