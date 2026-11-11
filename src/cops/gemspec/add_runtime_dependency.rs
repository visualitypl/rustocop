//! `Gemspec/AddRuntimeDependency`: `add_runtime_dependency` called with an
//! explicit receiver and at least one argument in a supplied gemspec. The
//! pinned matcher does not ask what the receiver is; only the path policy
//! (`**/*.gemspec`) narrows it.
use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, bundler::ordered_gem_node::inactive_program},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct AddRuntimeDependency;

impl Cop for AddRuntimeDependency {
    fn name(&self) -> &'static str {
        "Gemspec/AddRuntimeDependency"
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
        // `on_send` (never `csend`) restricted to :add_runtime_dependency;
        // `return if !node.receiver || node.arguments.empty?`. A block pass is
        // a pinned argument, a literal block is not.
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.is_safe_navigation()
            || call.name().as_slice() != b"add_runtime_dependency"
            || call.receiver().is_none()
            || (call.arguments().is_none()
                && call
                    .block()
                    .is_none_or(|b| b.as_block_argument_node().is_none()))
        {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (line, col) = crate::cops::lint::reference_line_col(selector.start_offset(), file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Use `add_dependency` instead of `add_runtime_dependency`.".into(),
            correction: cx
                .corrections_requested()
                .then(|| inactive_rename(&selector, cx, file))
                .flatten(),
        });
    }
}

// The upstream edit renames the selector. On a live receiver that is a
// dispatch change: the receiver is unknown (any object, `method_missing`,
// `method_added` hooks or a redefined alias may tell the two names apart), so
// a live file keeps its finding and bytes. The canonical rename is applied
// only where nothing runs: the program's sole statement is a closed top-level
// `if false ... end` without a data section or a listed source observer.
// Shared main-parse position/heredoc/atomic guards still apply.
fn inactive_rename(
    selector: &ruby_prism::Location<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let root = cx.ancestors().last()?;
    if selector.as_slice() != b"add_runtime_dependency" || !inactive_program(root, file) {
        return None;
    }
    Some(Correction {
        range: selector.start_offset()..selector.end_offset(),
        replacement: "add_dependency".into(),
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's gemspec/add_runtime_dependency_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov), plus lookalikes and the
    //! guarded correction.
    use super::*;
    use crate::cops::bundler::tests::assert_offenses;

    const MSG: &str = "Use `add_dependency` instead of `add_runtime_dependency`.";

    fn expect_at(path: &str, source: &str, expected: &[(u32, u32)]) {
        let expected: Vec<_> = expected
            .iter()
            .map(|&(line, col)| (line, col, MSG.to_owned()))
            .collect();
        assert_offenses(&AddRuntimeDependency, path, source, &expected);
    }

    fn expect(source: &str, expected: &[(u32, u32)]) {
        expect_at("example.gemspec", source, expected);
    }

    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_path_with_corrections(
            &[&AddRuntimeDependency],
            "example.gemspec",
            source,
        )
    }

    fn fixed(source: &str) -> Option<String> {
        let found = offenses(source);
        found.iter().any(|o| o.correction.is_some()).then(|| {
            let applied = crate::corrections::apply(source.as_bytes(), &found).unwrap();
            String::from_utf8(applied.bytes).unwrap()
        })
    }

    #[test]
    fn registers_an_offense_when_using_add_runtime_dependency() {
        expect(
            "Gem::Specification.new do |spec|\n  spec.add_runtime_dependency('rubocop')\nend\n",
            &[(2, 8)],
        );
    }

    #[test]
    fn no_offense_for_add_dependency_or_add_development_dependency() {
        expect(
            "Gem::Specification.new do |spec|\n  spec.add_dependency('rubocop')\nend\n",
            &[],
        );
        expect(
            "Gem::Specification.new do |spec|\n  spec.add_development_dependency('rubocop')\nend\n",
            &[],
        );
    }

    #[test]
    fn no_offense_without_receiver_or_arguments() {
        expect("add_runtime_dependency('rubocop')\n", &[]);
        expect("spec.add_runtime_dependency\n", &[]);
    }

    #[test]
    fn any_receiver_and_argument_shape_is_reported_at_the_selector() {
        // Not restricted to a Gem::Specification block variable.
        expect(
            "Foo.add_runtime_dependency 'a'\nself.add_runtime_dependency 'a'\n@s.add_runtime_dependency(*x)\nspec::add_runtime_dependency 'a'\nfoo.bar.add_runtime_dependency(&b)\nx.add_runtime_dependency 'a' do\nend\n",
            &[(1, 5), (2, 6), (3, 4), (4, 7), (5, 9), (6, 3)],
        );
        // Multi-line call: the selector line.
        expect(
            "spec\n  .add_runtime_dependency(\n    'a'\n  )\n",
            &[(2, 4)],
        );
        // Safe navigation (csend), a literal block only, other names.
        expect(
            "spec&.add_runtime_dependency 'a'\nspec.add_runtime_dependency { }\nspec.add_runtime_dependency do\nend\nspec.add_runtime_dependencies 'a'\nspec.add_runtime_dependency = 'a'\nadd_runtime_dependency 'a'\n",
            &[],
        );
    }

    #[test]
    fn only_supplied_gemspecs() {
        let source = "spec.add_runtime_dependency 'a'\n";
        for path in ["Gemfile", "x.rb", "gemspec", "gems.rb"] {
            expect_at(path, source, &[]);
        }
        expect_at("nested/.x.gemspec", source, &[(1, 6)]);
        expect_at(
            "x.gemspec",
            &format!(
                "\u{feff}{}",
                "x\nspec.add_runtime_dependency 'a'\n".replace('\n', "\r\n")
            ),
            &[(2, 6)],
        );
    }

    #[test]
    fn live_findings_keep_their_bytes() {
        for source in [
            "Gem::Specification.new do |spec|\n  spec.add_runtime_dependency('rubocop')\nend\n",
            "spec.add_runtime_dependency 'a' if false\n",
            "x\nif false\n  spec.add_runtime_dependency 'a'\nend\n",
            "if false\n  spec.add_runtime_dependency 'a'\nelse\nend\n",
            "unless true\n  spec.add_runtime_dependency 'a'\nend\n",
            "if false\n  spec.add_runtime_dependency 'a'\nend\n__END__\n",
            "if false\n  spec.add_runtime_dependency __FILE__\nend\n",
            "if false\n  spec.add_runtime_dependency 'a', caller\nend\n",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn canonical_rename_in_a_closed_inactive_envelope() {
        let before = "if false\n  Gem::Specification.new do |spec|\n    spec.add_runtime_dependency('rubocop')\n    spec\n      .add_runtime_dependency 'a', '~> 1' # c\n  end\nend\n";
        let after = "if false\n  Gem::Specification.new do |spec|\n    spec.add_dependency('rubocop')\n    spec\n      .add_dependency 'a', '~> 1' # c\n  end\nend\n";
        assert_eq!(fixed(before).as_deref(), Some(after));
        assert_eq!(crate::parser::parse(after.as_bytes()).errors().count(), 0);
        assert!(offenses(after).is_empty());
    }
}
