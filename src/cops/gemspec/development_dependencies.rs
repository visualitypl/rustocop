//! `Gemspec/DevelopmentDependencies`, at the fixed `EnforcedStyle: Gemfile`
//! and `AllowedGems: []`: `add_development_dependency` with a `str` gem name
//! and at most two further arguments, on any receiver, in a supplied
//! gemspec, `Gemfile` or `gems.rb`. Companion files are never read.
//! Detection only.
use ruby_prism::Node;

use crate::{
    cops::{
        Cop, NodeContext,
        bundler::{declarations::str_value, ordered_gem_node::send_end},
    },
    offense::Offense,
    source::SourceFile,
};

pub struct DevelopmentDependencies;

impl Cop for DevelopmentDependencies {
    fn name(&self) -> &'static str {
        "Gemspec/DevelopmentDependencies"
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
        // (send _ :add_development_dependency (str #forbidden_gem? ...) _? _?)
        // on `on_send` only (a `csend` is not dispatched). A block pass is a
        // pinned argument; a literal block is not.
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.is_safe_navigation() || call.name().as_slice() != b"add_development_dependency" {
            return;
        }
        let mut arguments: Vec<_> = call
            .arguments()
            .map(|a| a.arguments().iter().collect())
            .unwrap_or_default();
        if let Some(block) = call.block()
            && block.as_block_argument_node().is_some()
        {
            arguments.push(block);
        }
        if !(1..=3).contains(&arguments.len()) || str_value(&arguments[0], file).is_none() {
            return;
        }
        let (line, col) =
            crate::cops::lint::reference_line_col(call.location().start_offset(), file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(send_end(&call)),
            message: "Specify development dependencies in Gemfile.".into(),
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's gemspec/development_dependencies_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov) at the fixed `EnforcedStyle:
    //! Gemfile` and `AllowedGems: []`, plus lookalikes.
    use super::*;
    use crate::pipeline::lint_path_with_corrections;

    const MSG: &str = "Specify development dependencies in Gemfile.";

    fn found_at(path: &str, source: &str) -> Vec<(u32, u32, u32)> {
        let mut found: Vec<_> =
            lint_path_with_corrections(&[&DevelopmentDependencies], path, source)
                .into_iter()
                .map(|o| {
                    assert_eq!(o.message, MSG);
                    assert!(o.correction.is_none());
                    (o.line, o.col, o.end_line)
                })
                .collect();
        found.sort();
        found
    }

    fn expect_at(path: &str, source: &str, expected: &[(u32, u32)]) {
        let found: Vec<_> = found_at(path, source)
            .into_iter()
            .map(|(line, col, _)| (line, col))
            .collect();
        assert_eq!(found, expected, "{path}: {source}");
    }

    fn expect(source: &str, expected: &[(u32, u32)]) {
        expect_at("example.gemspec", source, expected);
    }

    fn spec(body: &str) -> String {
        format!("Gem::Specification.new do |spec|\n  spec.name = 'example'\n{body}end\n")
    }

    #[test]
    fn prefer_gem_file_examples_at_the_fixed_empty_allowlist() {
        // The spec's `allowed` gem is reported too at the fixed `[]`.
        for args in ["", ", '>= 1.0'", ", '>= 1.0', '< 2.0'"] {
            expect(
                &spec(&format!(
                    "  spec.add_development_dependency 'foo'{args}\n  spec.add_development_dependency 'allowed'{args}\n"
                )),
                &[(3, 3), (4, 3)],
            );
        }
        // `AllowedGems: nil` behaves like the empty list.
        expect(
            "Gem::Specification.new do |spec|\n  spec.add_development_dependency 'foo'\nend\n",
            &[(2, 3)],
        );
        // Gemfile/gems.rb declarations are the preferred place.
        expect_at("Gemfile", "gem 'example'\n", &[]);
        expect_at("gems.rb", "gem 'example'\n", &[]);
    }

    #[test]
    fn gemspec_style_examples_are_not_reported_at_the_fixed_gemfile_style() {
        expect_at("Gemfile", "gem 'example'\ngem 'allowed'\n", &[]);
        expect_at("gems.rb", "gem 'example'\ngem 'allowed'\n", &[]);
    }

    #[test]
    fn any_receiver_str_name_and_at_most_two_more_arguments() {
        expect(
            "add_development_dependency 'a'\nfoo.add_development_dependency \"a\", *v\nFoo::add_development_dependency('a', x, &b)\nx.add_development_dependency ?a, 1 do\nend\n@s.add_development_dependency <<~A\n  a\nA\n",
            &[(1, 1), (2, 1), (3, 1), (4, 1), (6, 1)],
        );
        expect(
            "s.add_development_dependency\ns.add_development_dependency()\ns.add_development_dependency 'a', 1, 2, 3\ns.add_development_dependency 'a', 1, 2, &b\ns.add_development_dependency :a\ns.add_development_dependency \"#{a}\"\ns.add_development_dependency 'a' 'b'\ns.add_development_dependency A\ns&.add_development_dependency 'a'\ns.add_dependency 'a'\ns.add_runtime_dependency 'a'\ngem 'a'\n",
            &[],
        );
        // The whole send, without its literal block.
        assert_eq!(
            found_at(
                "x.gemspec",
                "s.add_development_dependency(\n  'a'\n) do\nend\n"
            ),
            [(1, 1, 3)]
        );
    }

    #[test]
    fn supplied_gemspecs_gemfiles_and_gems_rb_only() {
        let source = "spec.add_development_dependency 'a'\n";
        for path in [
            "a.gemspec",
            "Gemfile",
            "gems.rb",
            "nested/Gemfile",
            ".x/gems.rb",
        ] {
            expect_at(path, source, &[(1, 1)]);
        }
        for path in ["x.gemfile", "Gemfile.lock", "x.rb", "gemspec", "Gemfile.rb"] {
            expect_at(path, source, &[]);
        }
    }
}
