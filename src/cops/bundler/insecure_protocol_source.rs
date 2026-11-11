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
        let (line, col) = crate::cops::lint::reference_line_col(start, file);
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
            correction: cx
                .corrections_requested()
                .then(|| inactive_edit(node, &argument, cx, file))
                .flatten(),
        });
    }
}

// Replacing a deprecated symbol source changes what a live Gemfile resolves
// (Bundler maps the symbols to an HTTP source with a deprecation warning; the
// edit switches the protocol). Prove ONLY the upstream canonical edit of an
// exact `source :name` that is the sole statement of a closed top-level
// `if false ... end`, which never runs. Every live or uncertain finding stays
// unedited. Shared main-parse encoding/source/runtime/atomic guards still
// decide whether the proposed individual edit may be applied.
fn inactive_edit(
    node: &Node<'_>,
    argument: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let spelled = SOURCES.iter().any(|name| {
        node.location().as_slice() == [b"source :".as_slice(), name].concat().as_slice()
    });
    if file.data_start().is_some() || !spelled {
        return None;
    }
    let mut ancestors = cx.ancestors();
    let body = ancestors.next()?.as_statements_node()?;
    let conditional = ancestors.next()?.as_if_node()?;
    let top = ancestors.next()?.as_statements_node()?;
    ancestors.next()?.as_program_node()?;
    if ancestors.next().is_some()
        || body.body().len() != 1
        || top.body().len() != 1
        || conditional.predicate().as_false_node().is_none()
        || conditional.subsequent().is_some()
        || conditional.end_keyword_loc().is_none()
        || conditional.if_keyword_loc()?.as_slice() != b"if"
    {
        return None;
    }
    let location = argument.location();
    Some(Correction {
        range: location.start_offset()..location.end_offset(),
        replacement: "'https://rubygems.org'".into(),
    })
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

    fn corrections(source: &str) -> Vec<Option<Correction>> {
        offenses(source).into_iter().map(|o| o.correction).collect()
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
    fn live_sources_keep_findings_without_edits() {
        for source in [
            "source :gemcutter\n",
            "source :rubygems\ngem 'a'\n",
            "if x\n  source :rubygems\nend\n",
            "if false\n  source :rubygems\n  gem 'a'\nend\n",
            "if false\n  source(:rubygems)\nend\n",
            "if false\n  source :rubygems\nelse\n  x\nend\n",
            "unless true\n  source :rubygems\nend\n",
            "source :rubygems if false\n",
            "x\nif false\n  source :rubygems\nend\n",
            "if false\n  source :rubygems do\n  end\nend\n",
            "if false\n  source :rubygems\nend\n__END__\n",
        ] {
            assert_eq!(corrections(source), vec![None], "{source}");
        }
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
