//! `Style/ArrayJoin` — prefer explicit literal Array#join over Array#*.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ArrayJoin;
impl Cop for ArrayJoin {
    fn name(&self) -> &'static str {
        "Style/ArrayJoin"
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
        if !candidate(&call) {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Favor `Array#join` over `Array#*`.".into(),
            line,
            col,
            end_line: file.last_line_of(selector),
            correction: cx
                .corrections_requested()
                .then(|| correction(&call, cx, file))
                .flatten(),
        });
    }
}
fn candidate(call: &ruby_prism::CallNode<'_>) -> bool {
    !call.is_safe_navigation()
        && call.name().as_slice() == b"*"
        && call.receiver().is_some_and(|r| r.as_array_node().is_some())
        && matches!(ast::call_arguments(call).as_slice(), [a] if a.as_string_node().is_some())
}
/// A narrow core API identity for the shared dispatch index. Child closedness
/// is computed by that index, not by rescanning this candidate's descendants.
/// Trust the canonical spelling too, so a second pass does not end the prefix.
pub(super) fn primitive_api(call: &ruby_prism::CallNode<'_>) -> bool {
    !call.is_safe_navigation()
        && call.block().is_none()
        && matches!(call.name().as_slice(), b"*" | b"join")
        && call.receiver().is_some_and(|r| r.as_array_node().is_some())
        && matches!(ast::call_arguments(call).as_slice(), [a] if a.as_string_node().is_some())
}
fn correction(
    call: &ruby_prism::CallNode<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    if !primitive_api(call) {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    let facts = file.format_string_facts(root);
    let receiver = call.receiver()?;
    let args = ast::call_arguments(call);
    let loc = call.location();
    if !facts.core_before(loc.start_offset())
        || !facts.closed(&receiver)
        || !facts.closed(&args[0])
        || file.contains_heredoc(root, &call.as_node())
        || file.heredoc_pending_at(root, loc.start_offset())
        || loc.as_slice().contains(&b'\n')
        || super::correction::contains_comment(file, loc.start_offset()..loc.end_offset())
    {
        return None;
    }
    // No trimming or decoding: percent delimiters and escaped whitespace can
    // carry data, even at the last byte of an argument (e.g. ?\\<space>).
    let receiver = std::str::from_utf8(receiver.location().as_slice()).ok()?;
    let separator = std::str::from_utf8(args[0].location().as_slice()).ok()?;
    super::literal_spelling::safe_edit(
        file,
        cx,
        Correction {
            range: loc.start_offset()..loc.end_offset(),
            replacement: format!("{receiver}.join({separator})"),
        },
    )
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop array_join_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == ArrayJoin.name())
            .collect()
    }
    #[test]
    fn pinned_operator_positions_and_messages() {
        for (s, col) in [
            ("%w(one two three) * \", \"", 19),
            ("%w(one two three)*\", \"", 18),
            ("foo = %w(one two three)*\", \"", 24),
        ] {
            let o = offenses(&format!("# header\n{s}\n"));
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, col, 2));
            assert_eq!(o[0].message, "Favor `Array#join` over `Array#*`.");
        }
    }
    #[test]
    fn all_three_pinned_corrections_apply_and_stabilize() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            (
                "%w(one two three) * \", \"",
                "%w(one two three).join(\", \")",
            ),
            ("%w(one two three)*\", \"", "%w(one two three).join(\", \")"),
            (
                "foo = %w(one two three)*\", \"",
                "foo = %w(one two three).join(\", \")",
            ),
        ] {
            assert_correction(&ArrayJoin, before, after);
            assert!(offenses(after).is_empty());
        }
    }
    #[test]
    fn canonical_empty_nested_percent_and_significant_token_bytes() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            ("[] * ','", "[].join(',')"),
            (
                "[nil, [1, false], :x] * ','",
                "[nil, [1, false], :x].join(',')",
            ),
            ("%w[a\\ b c\\]d] * %q|,|", "%w[a\\ b c\\]d].join(%q|,|)"),
            ("%i[a b] * ','", "%i[a b].join(',')"),
            ("%W[a b] * ','", "%W[a b].join(',')"),
            ("['x'].*(%q , )", "['x'].join(%q , )"),
            ("['x'] * ?\\ ", "['x'].join(?\\ )"),
            (
                "x = ['a'] * ','; y = [] * ','",
                "x = ['a'].join(','); y = [].join(',')",
            ),
            ("['x']::*(%q|,|)", "['x'].join(%q|,|)"),
            ("['\\xFF'] * \"\\xFF\"", "['\\xFF'].join(\"\\xFF\")"),
        ] {
            assert_correction(&ArrayJoin, before, after);
            assert!(offenses(after).is_empty());
        }
    }
    #[test]
    fn open_elements_dispatch_blocks_and_physical_boundaries_withhold_individually() {
        use crate::cops::testing::correction_offenses;
        for s in [
            "[x] * ','",
            "[factory()] * ','",
            "[*x] * ','",
            "[Object.new] * ','",
            "[\"#{effect}\"] * ','",
            "[x = 1] * ','",
            "[1.to_s] * ','",
            "[1].*(',') { effect }",
            "[1].*(',') do effect end",
            "require 'other'\n[1] * ','",
            "def x; [1] * ','; end",
            "class Array; alias join first; end\n[1] * ','",
            "class String; def to_s; 'bad'; end; end\n['x'] * ','",
            "module R; refine Array do; def join(*); 'bad'; end; end; end; using R; [1] * ','",
            "[1, # comment\n2] * ','",
            "[1] * # comment\n','",
            "[1].*(<<~SEP)\n,\nSEP\n",
            "[<<~TEXT] * ','\nx\nTEXT\n",
            "consume(<<~TEXT, [] * ',')\nx\nTEXT\n",
        ] {
            let o = correction_offenses(&ArrayJoin, s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(o[0].correction.is_none(), "{s}");
        }
        let s = "[] * ','; unknown; [] * ','";
        let o = correction_offenses(&ArrayJoin, s);
        assert!(o[0].correction.is_some());
        assert!(o[1].correction.is_none());
    }
    #[test]
    fn source_heap_code_data_and_line_guards_keep_offenses() {
        use crate::cops::testing::{assert_correction, correction_offenses};
        for s in [
            "[] * ','\np File.binread(__FILE__)",
            "[] * ','\np ObjectSpace.each_object(String).to_a",
            "[] * ','\np RubyVM::InstructionSequence.of(method(:f))",
            "[] * ','\np DATA.pos\n__END__\npayload\n",
        ] {
            let o = correction_offenses(&ArrayJoin, s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(o[0].correction.is_none(), "{s}");
        }
        for (before, after) in [
            (
                "\u{feff}[] * ','\r\n__LINE__\r\n__END__\r\npayload\0\r\n",
                "\u{feff}[].join(',')\r\n__LINE__\r\n__END__\r\npayload\0\r\n",
            ),
            (
                "# frozen_string_literal: true\n[] * ','\n",
                "# frozen_string_literal: true\n[].join(',')\n",
            ),
        ] {
            assert_correction(&ArrayJoin, before, after);
        }
    }
    #[test]
    fn pinned_and_structural_no_offense_lookalikes() {
        for s in [
            "%w(one two three) * 4",
            "%w(one two three) * test",
            "x * ','",
            "'x' * ','",
            "[a] * \"#{effect}\"",
            "[a]&.*(',')",
            "[a].*(',', &block)",
            "[a].*(&block)",
            "[a].*(',', ',')",
            "([a]) * ','",
            "[a] * (',')",
            "[a].join(',')",
            "# [a] * ','\n'*'",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
