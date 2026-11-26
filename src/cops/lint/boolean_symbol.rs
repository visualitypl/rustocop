//! `Lint/BooleanSymbol` — boolean-named symbols, except direct %i/%I words.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct BooleanSymbol;
impl Cop for BooleanSymbol {
    fn name(&self) -> &'static str {
        "Lint/BooleanSymbol"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_symbol_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(symbol) = node.as_symbol_node() else {
            return;
        };
        let value = match symbol.unescaped() {
            b"true" => "true",
            b"false" => "false",
            _ => return,
        };
        if cx
            .parent()
            .and_then(|p| p.as_array_node())
            .and_then(|a| a.opening_loc())
            .is_some_and(|l| l.as_slice().starts_with(b"%i") || l.as_slice().starts_with(b"%I"))
        {
            return;
        }
        // The reference translator can compose a physical multiline static
        // symbol into dsym rather than sym. Do not match those children as
        // standalone symbols. No global AST fingerprint behavior is changed.
        let raw = symbol.value_loc();
        let single = symbol
            .opening_loc()
            .is_some_and(|l| l.as_slice() == b":'" || l.as_slice().starts_with(b"%s"));
        if crate::cops::ast::require_literal::parts(
            raw.as_ref().map_or(symbol.unescaped(), |l| l.as_slice()),
            symbol.unescaped(),
            single,
            file,
        )
        .len()
            != 1
        {
            return;
        }
        // The symbol's colons go; a `true:` key becomes `true =>`.
        let loc = node.location();
        let source = String::from_utf8_lossy(loc.as_slice());
        let key = symbol.closing_loc().is_some_and(|c| c.as_slice() == b":")
            && cx
                .parent()
                .and_then(|p| p.as_assoc_node())
                .is_some_and(|pair| pair.key().location().start_offset() == loc.start_offset());
        let replacement = if key {
            format!("{} =>", source.trim_end_matches(':'))
        } else {
            source.replace(':', "")
        };
        let correction = crate::offense::Correction {
            range: loc.start_offset()..loc.end_offset(),
            replacement,
        };
        let (line, col) = file.reference_line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Symbol with a boolean name - you probably meant to use `{value}`."),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction: cx.corrections_requested().then_some(correction),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/boolean_symbol_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/BooleanSymbol")
            .collect()
    }
    #[test]
    fn pinned_boolean_names_and_hash_keys() {
        for (source, cols, value) in [
            (":true", vec![1], "true"),
            (":false", vec![1], "false"),
            ("{ :false => 42 }", vec![3], "false"),
            ("{ true: 'Foo' }", vec![3], "true"),
            ("{ false: :bar }", vec![3], "false"),
            ("{ false: :false }", vec![3, 10], "false"),
        ] {
            let found = offenses(source);
            assert_eq!(
                found
                    .iter()
                    .map(|o| (o.line, o.col, o.end_line))
                    .collect::<Vec<_>>(),
                cols.into_iter().map(|c| (1, c, 1)).collect::<Vec<_>>(),
                "{source}"
            );
            for o in found {
                assert_eq!(
                    o.message,
                    format!("Symbol with a boolean name - you probably meant to use `{value}`.")
                );
                assert!(o.correction.is_none());
            }
        }
    }
    #[test]
    fn pinned_and_percent_literal_exemptions() {
        for source in [
            ":something",
            "true",
            "false",
            "%i[foo false]",
            "%I[foo true false]",
            ":TRUE",
            ":falsey",
            "'false'",
            ":\"#{false}\"",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "[:true]",
            "%I[#{:true}]",
            "%W[#{:false}]",
            "[:\"true\"]",
            ":'false'",
            "%s(true)",
            ":\"tr\\u0075e\"",
            "{\"true\": 1}",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
}
