//! `Bundler/InsecureProtocolSource`: `source :gemcutter`, `:rubygems` or
//! `:rubyforge` in a supplied Gemfile. Fixed `AllowHttpProtocol: true`, so an
//! existing `'http://rubygems.org'` source is allowed.
use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, lint::reference_basic_literal::is_basic_literal},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct InsecureProtocolSource;

const SOURCES: [&[u8]; 3] = [b"gemcutter", b"rubygems", b"rubyforge"];

impl Cop for InsecureProtocolSource {
    fn name(&self) -> &'static str {
        "Bundler/InsecureProtocolSource"
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
        // (send nil? :source (sym {:gemcutter :rubygems :rubyforge}))
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.receiver().is_some()
            || call.name().as_slice() != b"source"
            || call
                .block()
                .is_some_and(|b| b.as_block_argument_node().is_some())
        {
            return;
        }
        let Some(arguments) = call.arguments() else {
            return;
        };
        let mut list = arguments.arguments().iter();
        let (Some(argument), None) = (list.next(), list.next()) else {
            return;
        };
        let Some(symbol) = argument.as_symbol_node() else {
            return;
        };
        let value = symbol.unescaped();
        if !is_basic_literal(&argument, file) || !SOURCES.contains(&value) {
            return;
        }
        let start = argument.location().start_offset();
        let (line, col) = file.reference_line_col(start);
        let name = std::str::from_utf8(value).unwrap_or_default();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: format!(
                "The source `:{name}` is deprecated because HTTP requests are insecure. \
                 Please change your source to 'https://rubygems.org' if possible, or \
                 'http://rubygems.org' if not."
            ),
            correction: cx.corrections_requested().then(|| Correction {
                range: start..argument.location().end_offset(),
                replacement: "'https://rubygems.org'".into(),
            }),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's bundler/insecure_protocol_source_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov), at the fixed
    //! `AllowHttpProtocol: true`, plus lookalikes and the correction envelope.
    use super::*;
    use crate::cops::bundler::tests::{assert_offenses, lint};

    fn message(name: &str) -> String {
        format!(
            "The source `:{name}` is deprecated because HTTP requests are insecure. Please change your source to 'https://rubygems.org' if possible, or 'http://rubygems.org' if not."
        )
    }

    fn expect(source: &str, expected: &[(u32, u32, &str)]) {
        let expected: Vec<_> = expected
            .iter()
            .map(|&(line, col, name)| (line, col, message(name)))
            .collect();
        assert_offenses(&InsecureProtocolSource, "Gemfile", source, &expected);
    }

    #[test]
    fn deprecated_symbol_sources() {
        for name in ["gemcutter", "rubygems", "rubyforge"] {
            expect(&format!("source :{name}\n"), &[(1, 8, name)]);
        }
    }

    #[test]
    fn allowed_sources() {
        expect("source 'https://rubygems.org'\n", &[]);
        // AllowHttpProtocol is fixed true.
        expect("source 'http://rubygems.org'\n", &[]);
    }

    #[test]
    fn pinned_matcher_lookalikes() {
        expect(
            "source(:rubygems)\nsource :\"rubygems\"\n",
            &[(1, 8, "rubygems"), (2, 8, "rubygems")],
        );
        expect(
            "source :rubygems do\n  gem 'a'\nend\n",
            &[(1, 8, "rubygems")],
        );
        expect(
            "source :rubygems, type: :x\nsource :other\nfoo.source :rubygems\nsource 'rubygems'\nsource :\"#{x}\"\nsource\nsource :rubygems, &b\n",
            &[],
        );
        assert!(lint(&InsecureProtocolSource, "a.rb", "source :rubygems\n").is_empty());
        expect("\u{feff}source :rubygems\n", &[(1, 9, "rubygems")]);
    }

    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_path_with_corrections(&[&InsecureProtocolSource], "Gemfile", source)
    }

    /// The single canonical edit, validated on both sides with the fixed
    /// Ruby 3.4 grammar.
    fn assert_correction(before: &str, after: &str) {
        assert_eq!(crate::parser::parse(before.as_bytes()).errors().count(), 0);
        let found = offenses(before);
        assert_eq!(found.len(), 1, "{before}");
        let applied = crate::corrections::apply(before.as_bytes(), &found).unwrap();
        assert_eq!(applied.bytes, after.as_bytes(), "{before}");
        assert_eq!(crate::parser::parse(&applied.bytes).errors().count(), 0);
    }

    #[test]
    fn canonical_edit_only_in_a_closed_inactive_envelope() {
        for name in ["gemcutter", "rubygems", "rubyforge"] {
            assert_correction(
                &format!("if false\n  source :{name}\nend\n"),
                "if false\n  source 'https://rubygems.org'\nend\n",
            );
        }
    }
}
