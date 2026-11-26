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
        let (line, col) = file.reference_line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Avoid leaving a trailing comma in attribute declarations.".into(),
            correction: cx
                .corrections_requested()
                .then(|| crate::offense::Correction {
                    range: start..start + 1,
                    replacement: String::new(),
                }),
        });
    }
}
#[cfg(test)]
mod tests {
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
            let bytes: Vec<u8> = crate::fixture::value(&row["bytes"]);
            let Ok(source) = std::str::from_utf8(&bytes) else {
                continue;
            };
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Lint/TrailingCommaInAttributeDeclaration")
                .map(|o| (o.line, o.col, o.end_line, o.message))
                .collect();
            let expected: Vec<(u32, u32, u32, String)> = crate::fixture::value(&row["findings"]);
            assert_eq!(found, expected, "{row}");
        }
    }
}
