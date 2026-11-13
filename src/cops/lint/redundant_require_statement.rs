//! `Lint/RedundantRequireStatement` — flags `require` of a feature Ruby
//! already loads at boot.
//!
//! Matches RuboCop's semantics for the Ruby version of the run: `enumerator`,
//! `thread`, `rational`, `complex`, `ruby2_keywords`, `fiber` and `set` are
//! redundant. `pathname` is preloaded from Ruby 4.0, so it is redundant only
//! when the run targets Ruby 4.0 or newer. Only a receiverless `require` with one
//! plain string argument counts, as in RuboCop. The offense is reported at
//! the call.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, policy::RubyVersion},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/RedundantRequireStatement` cop.
pub struct RedundantRequireStatement;

/// Features loaded before any user code runs, on Ruby 3.4.
const REDUNDANT_FEATURES: [&[u8]; 7] = [
    b"enumerator",
    b"thread",
    b"rational",
    b"complex",
    b"ruby2_keywords",
    b"fiber",
    b"set",
];

impl Cop for RedundantRequireStatement {
    fn name(&self) -> &'static str {
        "Lint/RedundantRequireStatement"
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.name().as_slice() != b"require"
            || call.receiver().is_some()
            || call
                .block()
                .is_some_and(|block| block.as_block_argument_node().is_some())
        {
            return;
        }
        let Some(arguments) = call.arguments() else {
            return;
        };
        let mut arguments = arguments.arguments().iter();
        let (Some(feature), None) = (arguments.next(), arguments.next()) else {
            return;
        };
        let Some(feature) = feature.as_string_node() else {
            return;
        };
        let feature = feature.unescaped();
        let preloaded = REDUNDANT_FEATURES.contains(&feature)
            || (feature == b"pathname" && file.ruby_version() >= RubyVersion(4, 0));
        if !preloaded {
            return;
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            call.location().start_offset(),
            file,
        );
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: "Remove unnecessary `require` statement.".to_string(),
            line,
            end_line: file.line_of(crate::cops::ast::source_end(node).saturating_sub(1)),
            col,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/redundant_require_statement_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for target
    //! Ruby 3.4.

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/RedundantRequireStatement")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn flags_require_enumerator_with_message() {
        let found = offenses("require 'enumerator'\nrequire 'uri'\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 1));
        assert_eq!(found[0].message, "Remove unnecessary `require` statement.");
    }

    #[test]
    fn flags_modifier_and_nested_requires() {
        assert_eq!(
            positions("require 'enumerator' if condition\nrequire 'uri'\n"),
            [(1, 1)]
        );
        assert_eq!(
            positions("if condition\n  require 'enumerator'\nend\nrequire 'uri'\n"),
            [(2, 3)]
        );
    }

    #[test]
    fn flags_every_feature_redundant_on_ruby_3_4() {
        let source = "\
require 'enumerator'
require 'rational'
require 'complex'
require 'thread'
require 'ruby2_keywords'
require 'fiber'
require 'set'
require 'uri'
";
        assert_eq!(
            positions(source),
            [(1, 1), (2, 1), (3, 1), (4, 1), (5, 1), (6, 1), (7, 1)]
        );
    }

    #[test]
    fn flags_other_string_spellings() {
        for source in [
            "require \"set\"\n",
            "require('set')\n",
            "require %q(set)\n",
            "require 'set' do\nend\n",
        ] {
            assert_eq!(positions(source), [(1, 1)], "should flag {source:?}");
        }
    }

    #[test]
    fn accepts_rubocop_spec_no_offenses() {
        // `pp` is not preloaded; `pathname` is only from Ruby 4.0.
        for source in [
            "require 'pp'\n\n# Imagine this code to be in a different file than the require.\nfoo.pretty_inspect\n",
            "require 'pathname'\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn pathname_is_redundant_from_ruby_4_0() {
        let source = "require 'pathname'\nrequire 'set'\nrequire 'uri'\n";
        let at = |version| {
            crate::pipeline::lint_str_at(version, &[&RedundantRequireStatement], source)
                .iter()
                .map(|o| (o.line, o.col))
                .collect::<Vec<_>>()
        };
        assert_eq!(at(RubyVersion(3, 4)), [(2, 1)]);
        assert_eq!(at(RubyVersion(4, 0)), [(1, 1), (2, 1)]);
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "require 'uri'\n",
            "require 'set/sorted_set'\n",
            "require 'sets'\n",
            "require_relative 'set'\n",
            "Kernel.require 'set'\n",
            "foo.require 'set'\n",
            "require \"#{name}set\"\n",
            "require :set\n",
            "require 'set', 'thread'\n",
            "require('set', &blk)\n",
            "require\n",
            "x = 'set'\nrequire x\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn honors_directives() {
        assert!(
            offenses("require 'set' # rubocop:disable Lint/RedundantRequireStatement\n").is_empty()
        );
    }
}
