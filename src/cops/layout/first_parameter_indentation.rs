//! `Layout/FirstParameterIndentation` — fixed `consistent` style: the first
//! parameter after a line break is indented two spaces from the start of the
//! line holding the opening parenthesis.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct FirstParameterIndentation;
impl Cop for FirstParameterIndentation {
    fn name(&self) -> &'static str {
        "Layout/FirstParameterIndentation"
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
        let Some(paren) = node.as_def_node().and_then(|def| def.lparen_loc()) else {
            return;
        };
        let Some(first) =
            super::parameter_alignment::parameters(node).and_then(|items| items.into_iter().next())
        else {
            return;
        };
        let loc = first.location();
        let paren_line = file.line_of(paren.start_offset());
        if file.line_of(loc.start_offset()) == paren_line {
            return;
        }
        // Upstream compares character columns: `source_line =~ /\S/` on the
        // parenthesis line plus the fixed width, against the parameter column.
        let line_start = file.line_start(paren_line);
        let indentation = file.bytes[line_start..paren.start_offset()]
            .iter()
            .position(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0b' | b'\x0c'))
            .map_or(0, |i| {
                super::alignment::effective_column(file, line_start + i)
            });
        let delta = (indentation + super::alignment::INDENT) as i64
            - super::alignment::effective_column(file, loc.start_offset()) as i64;
        if delta == 0 {
            return;
        }
        let alignment = super::alignment::Alignment {
            expression: loc.start_offset()..loc.end_offset(),
            delta,
        };
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            correction: cx
                .corrections_requested()
                .then(|| alignment.correction(file, cx.ancestors().next_back().unwrap_or(node)))
                .flatten(),
            cop: self.name(),
            severity: self.severity(),
            message: "Use 2 spaces for indentation in method args, relative to the start of the \
                      line where the left parenthesis is."
                .into(),
            line,
            col,
            end_line: file.last_line_of(loc),
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop first_parameter_indentation_spec.rb at 97f30c4
    //! (MIT, Copyright (c) 2012–26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    const MSG: &str = "Use 2 spaces for indentation in method args, relative to the start of the line where the left parenthesis is.";
    fn positions(source: &str) -> Vec<(u32, u32, u32)> {
        let found: Vec<_> = lint_str(source)
            .into_iter()
            .filter(|o| o.cop == FirstParameterIndentation.name())
            .collect();
        assert!(found.iter().all(|o| o.message == MSG), "{source}");
        found.iter().map(|o| (o.line, o.col, o.end_line)).collect()
    }
    fn check(before: &str, after: &str, expected: &[(u32, u32, u32)]) {
        assert_eq!(positions(before), expected, "{before}");
        assert_correction(&FirstParameterIndentation, before, after);
        assert!(positions(after).is_empty(), "{after}");
    }
    #[test]
    fn pinned_consistent_style_offenses_and_corrections() {
        for (def, params) in [
            ("def abc(", ["foo,", "bar,", "baz"]),
            ("def abc(", ["foo: 1,", "bar: 3,", "baz: 3"]),
            ("def self.abc(", ["foo: 1,", "bar: 3,", "baz: 3"]),
        ] {
            let indent = if params[0] == "foo," { 12 } else { 10 };
            let pad = " ".repeat(indent);
            let tail = format!("{pad}{}\n{pad}{}\n)\n  foo\nend\n", params[1], params[2]);
            check(
                &format!("{def}\n{pad}{}\n{tail}", params[0]),
                &format!("{def}\n  {}\n{tail}", params[0]),
                &[(2, indent as u32 + 1, 2)],
            );
        }
    }
    #[test]
    fn pinned_accepted_shapes() {
        for source in [
            "def abc foo, bar, baz\n  foo\nend\n",
            "def abc foo: 1, bar: 3, baz: 3\n  foo\nend\n",
            "def abc(foo, bar, baz)\n  foo\nend\n",
            "def abc(foo: 1, bar: 3, baz: 3)\n  foo\nend\n",
            "def abc(\n  foo,\n  bar,\n  baz\n)\n  foo\nend\n",
            "def abc(\n  foo: 1,\n  bar: 3,\n  baz: 3\n)\n  foo\nend\n",
            "def self.abc(\n  foo,\n  bar,\n  baz\n)\n  foo\nend\n",
            "def self.abc(\n  foo: 1,\n  bar: 3,\n  baz: 3\n)\n  foo\nend\n",
        ] {
            assert!(positions(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn single_parameters_indented_definitions_and_multiline_first_parameters() {
        check(
            "def foo(\na\n)\nend\n",
            "def foo(\n  a\n)\nend\n",
            &[(2, 1, 2)],
        );
        check(
            "class C\n  def foo(\n  a, b)\n  end\nend\n",
            "class C\n  def foo(\n    a, b)\n  end\nend\n",
            &[(3, 3, 3)],
        );
        check(
            "x = def foo(\n      a = [1,\n           2])\nend\n",
            "x = def foo(\n  a = [1,\n       2])\nend\n",
            &[(2, 7, 3)],
        );
        check(
            "def foo(\n   ...)\n  bar(...)\nend\n",
            "def foo(\n  ...)\n  bar(...)\nend\n",
            &[(2, 4, 2)],
        );
        for source in [
            "def foo()\nend\n",
            "def foo(\n)\nend\n",
            "def foo(a,\n        b)\nend\n",
            "def foo a,\n  b\nend\n",
            "def foo(\n  a) = a\n",
            "foo(\n      a)\n",
            "->(\n      a) {}\n",
        ] {
            assert!(positions(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn closed_source_observers_hold_corrections() {
        let source = "def foo(\na)\nend\np caller\n";
        let found = correction_offenses(&FirstParameterIndentation, source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }
}
