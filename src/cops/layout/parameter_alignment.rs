//! `Layout/ParameterAlignment` — fixed alignment with the first parameter.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ParameterAlignment;
impl Cop for ParameterAlignment {
    fn name(&self) -> &'static str {
        "Layout/ParameterAlignment"
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
        let Some(items) = parameters(node).filter(|items| items.len() >= 2) else {
            return;
        };
        let base = super::alignment::display_column(file, items[0].location().start_offset());
        let reserved = cx.corrections_requested() && enclosing_shift(node, cx, file);
        for item in misaligned(file, &items, base) {
            let loc = item.location();
            let Some(alignment) =
                super::alignment::Alignment::new(file, loc.start_offset()..loc.end_offset(), base)
            else {
                continue;
            };
            let (line, col) = file.line_col(loc.start_offset());
            out.push(Offense {
                correction: (cx.corrections_requested() && !reserved)
                    .then(|| alignment.correction(file, cx.ancestors().next_back().unwrap_or(node)))
                    .flatten(),
                cop: self.name(),
                severity: self.severity(),
                message:
                    "Align the parameters of a method definition if they span more than one line."
                        .into(),
                line,
                col,
                end_line: file.last_line_of(loc),
            });
        }
    }
}
/// A `def`/`defs` node's parameters in source order: upstream `args` children,
/// including destructuring, anonymous, `**nil` and `...` forwarding parameters.
pub(super) fn parameters<'pr>(node: &Node<'pr>) -> Option<Vec<Node<'pr>>> {
    let parameters = node.as_def_node()?.parameters()?;
    let mut items: Vec<_> = parameters.requireds().iter().collect();
    items.extend(parameters.optionals().iter());
    items.extend(parameters.rest());
    items.extend(parameters.posts().iter());
    items.extend(parameters.keywords().iter());
    items.extend(parameters.keyword_rest());
    items.extend(parameters.block().map(|b| b.as_node()));
    items.sort_by_key(|item| item.location().start_offset());
    Some(items)
}
/// Upstream `each_bad_alignment`: a parameter that begins its line at another
/// display column than the first parameter.
fn misaligned<'a, 'pr>(
    file: &'a SourceFile,
    items: &'a [Node<'pr>],
    base: usize,
) -> impl Iterator<Item = &'a Node<'pr>> + 'a {
    items.iter().filter(move |item| {
        let start = item.location().start_offset();
        super::alignment::starts_line(file, start)
            && super::alignment::display_column(file, start) != base
    })
}
/// Upstream defers a parameter inside an earlier offense of this cop
/// (`within?`): a misaligned parameter of an enclosing definition, reached
/// through `def -> ParametersNode -> parameter` ancestor edges.
fn enclosing_shift(node: &Node<'_>, cx: &NodeContext<'_, '_>, file: &SourceFile) -> bool {
    let mut child = node.location();
    let mut parameter = None;
    for parent in cx.ancestors() {
        if parent.as_parameters_node().is_some() {
            parameter = Some(child);
        } else if let (Some(items), Some(param)) = (parameters(parent), parameter.take())
            && items.len() >= 2
        {
            let base = super::alignment::display_column(file, items[0].location().start_offset());
            if misaligned(file, &items, base).any(|item| {
                let loc = item.location();
                (loc.start_offset(), loc.end_offset()) == (param.start_offset(), param.end_offset())
            }) {
                return true;
            }
        } else {
            parameter = None;
        }
        child = parent.location();
    }
    false
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop parameter_alignment_spec.rb at 97f30c4
    //! (MIT, Copyright (c) 2012–26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    const MSG: &str =
        "Align the parameters of a method definition if they span more than one line.";
    fn positions(source: &str) -> Vec<(u32, u32, u32)> {
        let found: Vec<_> = lint_str(source)
            .into_iter()
            .filter(|o| o.cop == ParameterAlignment.name())
            .collect();
        assert!(found.iter().all(|o| o.message == MSG), "{source}");
        found.iter().map(|o| (o.line, o.col, o.end_line)).collect()
    }
    fn check(before: &str, after: &str, expected: &[(u32, u32, u32)]) {
        assert_eq!(positions(before), expected, "{before}");
        assert_correction(&ParameterAlignment, before, after);
        assert!(positions(after).is_empty(), "{after}");
    }
    #[test]
    fn pinned_with_first_parameter_offenses_and_corrections() {
        check(
            "def method(a,\n  b)\nend\n",
            "def method(a,\n           b)\nend\n",
            &[(2, 3, 2)],
        );
        check(
            "def method(a,\n    b)\nend\n",
            "def method(a,\n           b)\nend\n",
            &[(2, 5, 2)],
        );
        check(
            "def func2(a,\n         *b,\n          c)\nend\n",
            "def func2(a,\n          *b,\n          c)\nend\n",
            &[(2, 10, 2)],
        );
        check(
            "def self.method(a,\n  b)\nend\n",
            "def self.method(a,\n                b)\nend\n",
            &[(2, 3, 2)],
        );
        check(
            "def func(a,\n       b,\nc)\n  123\nend\n",
            "def func(a,\n         b,\n         c)\n  123\nend\n",
            &[(2, 8, 2), (3, 1, 3)],
        );
    }
    #[test]
    fn pinned_accepted_shapes() {
        for source in [
            "def method(a, b)\nend\n",
            "def method(a,\n           b)\nend\n",
            "def method(\n  a,\n  b)\nend\n",
            "def method\nend\n",
            "def self.method(a,\n                b)\nend\n",
        ] {
            assert!(positions(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn every_parameter_kind_endless_and_parenless_definitions() {
        check(
            "def f(a, b = 1,\n  *c, d,\n  e:, f: 2,\n  **g, &h)\nend\n",
            "def f(a, b = 1,\n      *c, d,\n      e:, f: 2,\n      **g, &h)\nend\n",
            &[(2, 3, 2), (3, 3, 3), (4, 3, 4)],
        );
        check(
            "def f((a, b),\n   c) = a\n",
            "def f((a, b),\n      c) = a\n",
            &[(2, 4, 2)],
        );
        check(
            "def f a,\n  b\nend\n",
            "def f a,\n      b\nend\n",
            &[(2, 3, 2)],
        );
        check(
            "def f(a,\n  ...)\n  g(...)\nend\n",
            "def f(a,\n      ...)\n  g(...)\nend\n",
            &[(2, 3, 2)],
        );
        check(
            "def f(*,\n  **, &)\nend\n",
            "def f(*,\n      **, &)\nend\n",
            &[(2, 3, 2)],
        );
        check(
            "def f(a,\n  **nil)\nend\n",
            "def f(a,\n      **nil)\nend\n",
            &[(2, 3, 2)],
        );
        for source in [
            "def f(a)\nend\n",
            "def f(\n  a\n)\nend\n",
            "f = ->(a,\n  b) {}\n",
            "f { |a,\n  b| }\n",
            "def f(a, b = <<~X,\n    body\n  X\n      c)\nend\n",
        ] {
            assert!(positions(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn nested_definitions_in_defaults_reserve_the_outer_shift() {
        let source = "def f(a,\n  b = def g(x,\n  y); end)\nend\n";
        let found = correction_offenses(&ParameterAlignment, source);
        assert_eq!(
            found
                .iter()
                .map(|o| (o.line, o.col, o.correction.is_some()))
                .collect::<Vec<_>>(),
            [(2, 3, true), (3, 3, false)]
        );
        assert_correction(
            &ParameterAlignment,
            source,
            "def f(a,\n      b = def g(x,\n      y); end)\nend\n",
        );
    }
}
