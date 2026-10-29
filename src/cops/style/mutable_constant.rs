//! `Style/MutableConstant`: fixed literals policy, Recursive=false; detection only.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct MutableConstant;
impl Cop for MutableConstant {
    fn name(&self) -> &'static str {
        "Style/MutableConstant"
    }
    fn check_node(
        &self,
        node: &Node<'_>,
        _cx: &NodeContext<'_, '_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let value = match node {
            Node::ConstantWriteNode { .. } => node.as_constant_write_node().unwrap().value(),
            Node::ConstantPathWriteNode { .. } => {
                node.as_constant_path_write_node().unwrap().value()
            }
            Node::ConstantOrWriteNode { .. } => node.as_constant_or_write_node().unwrap().value(),
            Node::ConstantPathOrWriteNode { .. } => {
                node.as_constant_path_or_write_node().unwrap().value()
            }
            _ => return,
        };
        if !matches!(
            value,
            Node::ArrayNode { .. }
                | Node::HashNode { .. }
                | Node::StringNode { .. }
                | Node::InterpolatedStringNode { .. }
                | Node::XStringNode { .. }
                | Node::InterpolatedXStringNode { .. }
        ) {
            return;
        }
        let loc = value.location();
        let magic = file.mutable_constant_magic();
        let last_line = file.line_of(loc.end_offset().saturating_sub(1));
        if magic.shareable_at(last_line) || (magic.frozen_strings && uninterpolated_string(&value))
        {
            return;
        }
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Freeze mutable objects assigned to constants.".into(),
            line,
            col,
            end_line: file.line_of(loc.end_offset().saturating_sub(1)),
            correction: None,
        });
    }
}

// Ruby 3.4 freezes ranges/regexps but not interpolated strings. Prism also
// represents adjacent static strings as InterpolatedStringNode containers.
fn uninterpolated_string(node: &Node<'_>) -> bool {
    node.as_string_node().is_some()
        || node
            .as_interpolated_string_node()
            .is_some_and(|s| s.parts().iter().all(|part| uninterpolated_string(&part)))
}

/// Cache the pinned magic-comment policy in one forward physical-line scan.
/// Upstream uses the last valid shareable directive through the value's last
/// line, globally (not lexical Ruby scope). Index transitions, never rescan all
/// preceding lines per constant as upstream does.
#[derive(Default)]
pub(crate) struct MagicState {
    frozen_strings: bool,
    shareable: Vec<(u32, bool)>,
}
impl MagicState {
    pub(crate) fn build(file: &SourceFile) -> Self {
        use super::magic_comment::{MagicComment, leading_end};
        let first_code_line = file.line_of(leading_end(file).saturating_sub(1));
        let mut result = Self::default();
        let mut frozen = None;
        for line in 1..=file.line_count() as u32 {
            let text = file.line_text(line);
            let text = if line == 1 {
                text.strip_prefix(b"\xef\xbb\xbf").unwrap_or(text)
            } else {
                text
            };
            let start = text
                .iter()
                .position(|b| !super::is_ruby_whitespace(*b))
                .unwrap_or(text.len());
            let comment = MagicComment::parse(&text[start..]);
            if line < first_code_line && frozen.is_none() {
                frozen = comment.frozen_strings();
            }
            if let Some(enabled) = comment.shareable_constants() {
                result.shareable.push((line, enabled));
            }
        }
        result.frozen_strings = frozen.unwrap_or(false);
        result
    }
    fn shareable_at(&self, line: u32) -> bool {
        let index = self.shareable.partition_point(|(at, _)| *at <= line);
        index > 0 && self.shareable[index - 1].1
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/mutable_constant_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str;
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/MutableConstant")
            .collect()
    }
    fn assert_at(source: &str, positions: &[(u32, u32)]) {
        let found = offenses(source);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            positions,
            "{source}"
        );
        for o in found {
            assert_eq!(o.message, "Freeze mutable objects assigned to constants.");
            assert!(o.correction.is_none());
        }
    }
    #[test]
    fn mutable_literals_with_plain_and_or_assignment() {
        for literal in [
            "[1, 2, 3]",
            "%w(a b c)",
            "%W(a #{b})",
            "%i(a b)",
            "{ a: 1, b: 2 }",
            "'str'",
            "\"top#{1 + 2}\"",
            "YYY, ZZZ",
            "*1..10",
            "*(1..10)",
            "`echo value`",
            "`echo #{value}`",
        ] {
            assert_at(&format!("CONST = {literal}\n"), &[(1, 9)]);
            if literal != "YYY, ZZZ" && !literal.starts_with('*') {
                assert_at(&format!("CONST ||= {literal}\n"), &[(1, 11)]);
            }
        }
    }
    #[test]
    fn immutable_and_nonliteral_values() {
        for value in [
            "1",
            "1.5",
            "1r",
            "1i",
            ":sym",
            ":\"#{x}\"",
            "nil",
            "true",
            "false",
            "FOO + BAR",
            "FOO - BAR",
            "'foo' + 'bar'",
            "ENV['foo']",
            "::ENV['foo']",
            "Something.new",
            "OTHER_CONST",
            "Namespace::OTHER_CONST",
            "Struct.new(:a)",
            "Data.define(:a)",
            "[1].freeze",
            "[{ a: [] }].freeze",
            "/regexp/",
            "/#{x}/",
            "1..99",
            "(1...99)",
            "([])",
            "(bar = [])",
            "bar = []",
            "if x; []; else; {}; end",
        ] {
            assert_at(&format!("CONST = {value}\n"), &[]);
            assert_at(&format!("CONST ||= {value}\n"), &[]);
        }
    }
    #[test]
    fn frozen_header_only_exempts_uninterpolated_strings() {
        for header in [
            "# frozen_string_literal: true",
            "# frozen-string-literal: TRUE",
            "# -*- encoding: utf-8; frozen_string_literal: true -*-",
        ] {
            assert_at(&format!("{header}\nCONST = 'str'\n"), &[]);
            assert_at(&format!("{header}\nCONST = \"#{{x}}\"\n"), &[(2, 9)]);
            assert_at(&format!("{header}\nCONST = [1]\n"), &[(2, 9)]);
            assert_at(&format!("{header}\nCONST = 'foo' \\\n 'bar'\n"), &[]);
        }
        assert_at("# frozen_string_literal: false\nCONST = 'str'\n", &[(2, 9)]);
        assert_at(
            "# frozen_string_literal: true\nCONST = <<~HERE\n  foo #{x}\nHERE\n",
            &[(2, 9)],
        );
        assert_at(
            "# frozen_string_literal: true\nCONST = <<~HERE\n  foo\nHERE\n",
            &[],
        );
    }
    #[test]
    fn shareable_magic_values_and_resets() {
        for mode in ["literal", "experimental_everything", "experimental_copy"] {
            assert_at(
                &format!("# shareable_constant_value: {mode}\nCONST = [1]\n"),
                &[],
            );
        }
        assert_at(
            "# shareable_constant_value: literal\nX = []\n# shareable_constant_value: none\nY = []\n# shareable_constant_value: experimental_everything\nZ = []\n",
            &[(4, 5)],
        );
        assert_at(
            "X = [1, 2, 3] # shareable_constant_value: literal\nY = [4, 5, 6]\n",
            &[(1, 5), (2, 5)],
        );
    }
    #[test]
    fn inline_emacs_shareable_enabling_and_resets() {
        for mode in [
            "literal",
            "experimental_everything",
            "experimental_copy",
            "LITERAL",
        ] {
            assert_at(
                &format!("A = [] # -*- shareable_constant_value: {mode} -*-\nB = {{}}\n"),
                &[],
            );
        }
        assert_at(
            "A = [] # -*- shareable_constant_value: literal -*-\nB = {} # -*- shareable_constant_value: none -*-\nC = []\n",
            &[(2, 5), (3, 5)],
        );
        assert_at(
            "A = [] # -*- shareable_constant_value: literal -*-\nB = {} # shareable_constant_value: none\nC = []\n",
            &[],
        );
        // The pinned Emacs search is unanchored even outside Ruby comments.
        assert_at(
            "A = '-*- shareable_constant_value: literal -*-'\nB = {}\n",
            &[],
        );
    }
    #[test]
    fn mixed_magic_keys_and_header_precedence() {
        assert_at("# frozen-string_literal: true\nCONST = 'str'\n", &[]);
        assert_at("# shareable_constant-value: literal\nCONST = []\n", &[]);
        for header in [
            "# frozen_string_literal: false\n# frozen_string_literal: true",
            "# frozen_string_literal: invalid\n# frozen_string_literal: true",
            "# vim: frozen_string_literal=true",
        ] {
            assert_at(
                &format!("{header}\nCONST = 'str'\n"),
                &[(header.lines().count() as u32 + 1, 9)],
            );
        }
        assert_at(
            "# preliminary\n\n# frozen_string_literal: true\nCONST = 'str'\n",
            &[],
        );
        assert_at(
            "x = 1\n# frozen_string_literal: true\nCONST = 'str'\n",
            &[(3, 9)],
        );
        assert_at(
            "# frozen_string_literal: true\nCONST = `echo value`\n",
            &[(2, 9)],
        );
        assert_at(
            "# shareable_constant_value: literal\n# shareable_constant_value: invalid\nCONST = []\n",
            &[],
        );
        // Simple shareable values are case-sensitive; Emacs values are not.
        assert_at(
            "# shareable_constant_value: LITERAL\nCONST = []\n",
            &[(2, 9)],
        );
        assert_at(
            "# -*- shareable_constant_value: LITERAL -*-\nCONST = []\n",
            &[],
        );
    }
    #[test]
    fn constant_paths_nesting_reopens_singletons_and_other_writes() {
        assert_at(
            "module Outer\n  A = []\n  module Inner\n    B ||= {}\n  end\nend\nmodule Outer::Inner\n  C = 'x'\nend\nclass << self\n  D = []\nend\n::Outer::E ||= []\nobject::F = {}\n",
            &[(2, 7), (4, 11), (8, 7), (11, 7), (13, 16), (14, 13)],
        );
        assert_at("A, B = [], {}\n::A += []\nN::B &&= {}\n", &[]);
        assert_at(
            "class C\n  private\n  X = []\n  private_constant :X\nend\nclass C\n  X ||= []\nend\n",
            &[(3, 7), (7, 9)],
        );
        assert_at(
            "module M; include X; A = []; extend Y; B = {}; end\n",
            &[(1, 26), (1, 44)],
        );
    }
    #[test]
    fn shareable_policy_uses_source_order_not_lexical_scope() {
        assert_at(
            "module M\n# shareable_constant_value: literal\nX = []\nend\nY = []\n# shareable_constant_value: none\nZ = []\n",
            &[(7, 5)],
        );
        // The pinned cop searches through the RHS's last physical line.
        assert_at(
            "X = [\n# shareable_constant_value: literal\n1]\nY = []\n",
            &[],
        );
    }
    #[test]
    fn heredoc_interpolation_concat_bom_crlf_and_data_positions() {
        assert_at(
            "\u{feff}# coding: utf-8\r\nCONST = <<~HERE\r\n  café\r\nHERE\r\n__END__\r\nCONST = []\r\n",
            &[(2, 9)],
        );
        assert_at(
            "# frozen_string_literal: true\nCONST = \"#{foo}\" \\\n 'bar'\n",
            &[(2, 9)],
        );
        assert_at(
            "# frozen_string_literal: true\nCONST = \"#@foo\"\nCONST2 = \"#$foo\"\nCONST3 = \"#{'x'}\"\n",
            &[(2, 9), (3, 10), (4, 10)],
        );
    }
    #[test]
    fn magic_index_is_cached_and_scales_with_lines_not_constants_times_lines() {
        let mut source = String::new();
        for i in 0..2000 {
            source.push_str(&format!(
                "# shareable_constant_value: {}\nC{i} = []\n",
                if i % 2 == 0 { "literal" } else { "none" }
            ));
        }
        let file = SourceFile::new("x.rb".into(), source.into_bytes());
        let index = file.mutable_constant_magic();
        assert!(std::ptr::eq(index, file.mutable_constant_magic()));
        assert_eq!(index.shareable.len(), 2000);
        for i in 0..2000 {
            assert_eq!(index.shareable_at(2 * i + 2), i % 2 == 0);
        }
    }
    #[test]
    fn only_outer_literal_and_inner_chained_constant() {
        assert!(!MutableConstant.safe_fixes());
        assert_at("CONST = [{ a: [], b: 'foo' }]\n", &[(1, 9)]);
        assert_at("FOO = BAR = 'baz'\n", &[(1, 13)]);
        assert_at("CONST += []\nCONST &&= {}\nx = []\n@x = []\n", &[]);
    }
}
