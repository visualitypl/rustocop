//! `Lint/TrailingCommaInAttributeDeclaration` — definitions swallowed as attributes.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct TrailingCommaInAttributeDeclaration;
impl Cop for TrailingCommaInAttributeDeclaration {
    fn name(&self) -> &'static str {
        "Lint/TrailingCommaInAttributeDeclaration"
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
        if call.receiver().is_some()
            || !matches!(
                call.name().as_slice(),
                b"attr" | b"attr_reader" | b"attr_writer" | b"attr_accessor"
            )
        {
            return;
        }
        let args = ast::call_arguments(&call);
        if args.len() < 2 || args.last().is_none_or(|n| n.as_def_node().is_none()) {
            return;
        }
        let mut start = args[args.len() - 2].location().end_offset();
        // RangeHelp's defaults consume spaces/tabs, then newlines (not all
        // Ruby whitespace or continuations). Preserve even its form-feed range.
        while file
            .bytes
            .get(start)
            .is_some_and(|b| matches!(b, b' ' | b'\t'))
        {
            start += 1;
        }
        loop {
            if file.bytes.get(start..start + 2) == Some(b"\r\n") {
                start += 2;
            } else if file.bytes.get(start) == Some(&b'\n') {
                start += 1;
            } else {
                break;
            }
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(start, file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Avoid leaving a trailing comma in attribute declarations.".into(),
            correction: cx
                .corrections_requested()
                .then(|| inert_comma(cx, file, &call, &args, start))
                .flatten(),
        });
    }
}
// Ordinary comma deletion changes argument evaluation, return values, method
// lookup and even which getter overwrites a definition. Only a closed, never
// executed top-level `if false` supports this source edit, not intention repair.
fn inert_comma(
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    call: &ruby_prism::CallNode<'_>,
    args: &[Node<'_>],
    start: usize,
) -> Option<crate::offense::Correction> {
    if file.data_start().is_some()
        || call.opening_loc().is_some()
        || call.block().is_some()
        || args.len() != 2
        || args[0].as_symbol_node().is_none()
        || file.bytes.get(start) != Some(&b',')
    {
        return None;
    }
    let definition = args[1].as_def_node()?;
    if definition.receiver().is_some()
        || definition.parameters().is_some()
        || definition.body().is_some()
    {
        return None;
    }
    let gap = file
        .bytes
        .get(start + 1..definition.location().start_offset())?;
    if !gap.contains(&b'\n') || !gap.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    let mut ancestors = cx.ancestors();
    let body = ancestors.next()?.as_statements_node()?;
    let conditional = ancestors.next()?.as_if_node()?;
    if body.body().len() != 1
        || conditional.predicate().as_false_node().is_none()
        || conditional.subsequent().is_some()
        || conditional.if_keyword_loc()?.as_slice() != b"if"
    {
        return None;
    }
    let top = ancestors.next()?.as_statements_node()?;
    ancestors.next()?.as_program_node()?;
    if ancestors.next().is_some() || top.body().len() != 1 {
        return None;
    }
    Some(crate::offense::Correction {
        range: start..start + 1,
        replacement: String::new(),
    })
}
#[cfg(test)]
mod tests {
    //! Ported RuboCop lint/trailing_comma_in_attribute_declaration_spec.rb examples
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    #[test]
    fn canonical_unexecuted_comma_edit_and_ordinary_behavior_repair_hold() {
        for (before, after) in [
            (
                "if false\n  attr_reader :bar,\n  def baz; end\nend\n",
                "if false\n  attr_reader :bar\n  def baz; end\nend\n",
            ),
            (
                "class Foo\n  attr_reader :bar,\n  def baz; end\nend\n",
                "class Foo\n  attr_reader :bar,\n  def baz; end\nend\n",
            ),
            (
                "if false\n  attr_reader(:bar, def baz; end)\nend\n",
                "if false\n  attr_reader(:bar, def baz; end)\nend\n",
            ),
        ] {
            let offenses = crate::cops::testing::correction_offenses(
                &super::TrailingCommaInAttributeDeclaration,
                before,
            );
            assert_eq!(offenses.len(), 1);
            assert_eq!(
                crate::corrections::apply(before.as_bytes(), &offenses)
                    .unwrap()
                    .bytes,
                after.as_bytes()
            );
        }
    }
    #[test]
    fn pinned_examples_and_source_controls() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/else_attribute_layout.json"
        ))
        .unwrap();
        for row in rows
            .into_iter()
            .filter(|r| r["cop"] == "Lint/TrailingCommaInAttributeDeclaration")
        {
            let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
            let Ok(source) = std::str::from_utf8(&bytes) else {
                continue;
            };
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Lint/TrailingCommaInAttributeDeclaration")
                .map(|o| (o.line, o.col, o.end_line, o.message))
                .collect();
            let expected: Vec<(u32, u32, u32, String)> =
                serde_json::from_value(row["findings"].clone()).unwrap();
            assert_eq!(found, expected, "{row}");
        }
    }
}
