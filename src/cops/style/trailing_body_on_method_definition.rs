//! `Style/TrailingBodyOnMethodDefinition` — start multiline method bodies below the header.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct TrailingBodyOnMethodDefinition;
impl Cop for TrailingBodyOnMethodDefinition {
    fn name(&self) -> &'static str {
        "Style/TrailingBodyOnMethodDefinition"
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
        let Some(def) = node.as_def_node() else {
            return;
        };
        if def.equal_loc().is_some() {
            return;
        }
        let header_end = def
            .rparen_loc()
            .map(|l| l.end_offset())
            .or_else(|| def.parameters().map(|p| p.location().end_offset()))
            .unwrap_or(def.name_loc().end_offset());
        super::trailing_body_on_class::check_body(
            self,
            node,
            def.body(),
            header_end,
            cx,
            file,
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/trailing_body_on_method_definition_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/TrailingBodyOnMethodDefinition")
            .collect()
    }
    #[test]
    fn pinned_positions_and_messages() {
        for (source, line, col) in [
            ("def some_method; body\nend\n", 1, 18),
            ("def extra_large; { size: 15 };\nend\n", 1, 18),
            (
                "def seven_times(stuff) 7.times { do_this(stuff) }\nend\n",
                1,
                24,
            ),
            ("def some_method; foo = {}\n  more_body(foo)\nend\n", 1, 18),
            (
                "def do_stuff(thing) process(thing)\n  8.times { thing + 9 }\n  even_more(thing)\nend\n",
                1,
                21,
            ),
            ("foo;def some_method; body\nend\n", 1, 22),
            ("  def some_method; body # stuff\n  end\n", 1, 20),
            ("  def some_method arg1, arg2; body\n  end\n", 1, 31),
            ("  def some_method; body; more_body;\n  end\n", 1, 20),
            ("\n  def some_method; body\n  end\n", 2, 20),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, line),
                "{source}"
            );
            assert_eq!(
                found[0].message,
                "Place the first line of a multi-line method definition's body on its own line."
            );
        }
    }
    #[test]
    fn all_pinned_correction_pairs() {
        for (before, after) in [
            (
                "def some_method; body\nend\ndef extra_large; { size: 15 };\nend\ndef seven_times(stuff) 7.times { do_this(stuff) }\nend\n",
                "def some_method \n  body\nend\ndef extra_large \n  { size: 15 };\nend\ndef seven_times(stuff) \n  7.times { do_this(stuff) }\nend\n",
            ),
            (
                "def some_method; foo = {}\n  more_body(foo)\nend\n",
                "def some_method \n  foo = {}\n  more_body(foo)\nend\n",
            ),
            (
                "def do_stuff(thing) process(thing)\n  8.times { thing + 9 }\n  even_more(thing)\nend\n",
                "def do_stuff(thing) \n  process(thing)\n  8.times { thing + 9 }\n  even_more(thing)\nend\n",
            ),
            (
                "foo;def some_method; body\nend\n",
                "foo;def some_method \n      body\nend\n",
            ),
            (
                "  def some_method; body # stuff\n  end\n",
                "  # stuff\n  def some_method \n    body \n  end\n",
            ),
            (
                "  def some_method arg1, arg2; body\n  end\n",
                "  def some_method arg1, arg2 \n    body\n  end\n",
            ),
            (
                "  def some_method; body; more_body;\n  end\n",
                "  def some_method \n    body; more_body;\n  end\n",
            ),
            (
                "\n  def some_method; body\n  end\n",
                "\n  def some_method \n    body\n  end\n",
            ),
        ] {
            assert_correction(&TrailingBodyOnMethodDefinition, before, after);
            assert!(offenses(after).is_empty(), "{after}");
        }
    }
    #[test]
    fn visibility_comments_stay_before_the_complete_wrapper() {
        for visibility in ["private", "protected", "public"] {
            let before = format!("  {visibility} def f; 1 # docs\n  end\n  def g; end\n");
            let after = format!(
                "  # docs\n  {visibility} def f \n{}1 \n  end\n  def g; end\n",
                " ".repeat(visibility.len() + 5)
            );
            assert_correction(&TrailingBodyOnMethodDefinition, &before, &after);
        }
    }
    #[test]
    fn uncertain_newlines_retain_each_offense_without_an_edit() {
        for source in [
            "def f; <<~TXT\npayload\nTXT\nend\n",
            "def f; 1\nend\np __LINE__\n",
            "def f; __LINE__\nend\n",
            "def f; 1 # rubocop:disable Layout\nend\n",
            "def f; 1 # rustocop:disable Layout\nend\n",
            "def f; 1 # frozen_string_literal: true\nend\n",
            "def outer\n def inner; 1\n end\nend\n",
            "def f; 1\nend\np errors.map(&:full_message)\n",
            "def f; 1\nend\np Object.const_get(:DATA).pos\n__END__\nraw\n",
            "x = def f; 1\nend\n",
            "foo;def f; 1 # docs\nend\n",
        ] {
            let found = correction_offenses(&TrailingBodyOnMethodDefinition, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn sibling_deferred_heredocs_withhold_newlines_but_keep_the_offense() {
        for opener in [
            "<<TXT",
            "<<-TXT",
            "<<~TXT",
            "<<'TXT'",
            "<<\"TXT\"",
            "<<`TXT`",
        ] {
            let source = format!("x = {opener};def f; 1\npayload\nTXT\nend\n");
            let found = correction_offenses(&TrailingBodyOnMethodDefinition, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col),
                (1, source.find("1\n").unwrap() as u32 + 1)
            );
            assert!(found[0].correction.is_none(), "{source}");
        }
        // A lookalike or an already completed heredoc is not pending here.
        for prefix in ["x = '<<TXT';", "x = 1 << 2;", "x = <<TXT\npayload\nTXT\n"] {
            assert_correction(
                &TrailingBodyOnMethodDefinition,
                &format!("{prefix}def f; 1\nend\n"),
                &format!(
                    "{prefix}def f \n{}1\nend\n",
                    " ".repeat(prefix.rsplit('\n').next().unwrap().len() + 2)
                ),
            );
        }
    }
    #[test]
    fn singleton_operator_setter_parameters_and_visibility_headers_are_preserved() {
        for header in [
            "def self.f",
            "def (Object.new).f",
            "def self.[]=(key, value)",
            "def f=(value)",
            "def [](key)",
            "def +(other)",
            "def !",
            "def end",
            "def f()",
            "def f(x = (side_effect; 1), *rest, k: 2, **kw, &block)",
            "def f(a, b = 1, *rest, k:, **kw, &block)",
            "def f(a, ...)",
            "def f arg1, arg2",
            "private def f",
            "protected def f",
            "public def f",
        ] {
            let before = format!("{header}; value\nend\n");
            let parsed = ruby_prism::parse(before.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{before}");
            let found = offenses(&before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!((found[0].line, found[0].col), (1, header.len() as u32 + 3));
            let indent = header.find("def").unwrap() + 2;
            assert_correction(
                &TrailingBodyOnMethodDefinition,
                &before,
                &format!("{header} \n{}value\nend\n", " ".repeat(indent)),
            );
        }
    }
    #[test]
    fn parentheses_highlight_inner_expression_but_move_the_whole_group() {
        for (body, line, col, last) in [
            ("(1)", 1, 9, 1),
            ("(1\n)", 1, 9, 1),
            ("(\n1\n)", 2, 1, 2),
            ("((1))", 1, 9, 1),
            ("(1; 2)", 1, 9, 1),
            ("(begin\n1\nend)", 1, 9, 3),
            ("(1\n); 2", 1, 8, 2),
        ] {
            let before = format!("def f; {body}\nend\n");
            let found = offenses(&before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, last),
                "{before}"
            );
            let after = format!("def f \n  {body}\nend\n");
            assert_correction(&TrailingBodyOnMethodDefinition, &before, &after);
            assert!(offenses(&after).is_empty());
        }
    }
    #[test]
    fn implicit_rescue_ensure_and_explicit_begin_ranges() {
        for (body, last) in [
            ("foo\nrescue\nbar", 3),
            ("foo\nensure\nbar", 3),
            ("foo\nrescue\nbar\nelse\nbaz\nensure\nqux", 7),
            ("rescue\nbar", 2),
            ("ensure\nbar", 2),
            ("begin\nfoo\nend", 3),
        ] {
            let before = format!("def f; {body}\nend\n");
            let found = offenses(&before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, 8, last)
            );
            assert_correction(
                &TrailingBodyOnMethodDefinition,
                &before,
                &format!("def f \n  {body}\nend\n"),
            );
            assert!(offenses(&format!("def f\n{body}\nend\n")).is_empty());
        }
    }
    #[test]
    fn literal_contents_and_parameter_semicolons_are_not_reformatted() {
        for body in ["\"a\nb\"", "/a\nb/", "%w[a\nb]", "(1; 2)"] {
            assert_correction(
                &TrailingBodyOnMethodDefinition,
                &format!("def f(x = (side_effect; 1)); {body}\nend\n"),
                &format!("def f(x = (side_effect; 1)) \n  {body}\nend\n"),
            );
        }
        assert_correction(
            &TrailingBodyOnMethodDefinition,
            "p __LINE__\ndef f; 1\nend\n",
            "p __LINE__\ndef f \n  1\nend\n",
        );
    }
    #[test]
    fn accepts_regular_empty_single_line_endless_and_multiline_headers() {
        for source in [
            "def some_method\n  body\nend\n",
            "def stuff_method\n  stuff\n  9.times { process(stuff) }\n  more_stuff\nend\n",
            "def some_method\n  body\nfoo; end\n",
            "def some_method = (\n  body\n)\n",
            "def f = body\ndef self.f =\n  body\n",
            "def f; body; end\ndef self.f; body; end\n",
            "def f; end\ndef self.f\nend\n",
            "def f(\n  x\n); body\nend\n",
            "def f(x = (\n  1\n)); body\nend\n",
            "def f;\n body\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
