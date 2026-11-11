//! `Gemspec/OrderedDependencies`: dependency declarations of the same kind
//! on adjacent lines of a supplied gemspec that are not in case-insensitive
//! alphabetical order. Reuses the `OrderedGemNode` projection of
//! `Bundler/OrderedGems` at the same fixed defaults.
use crate::{
    cops::{
        Cop,
        bundler::ordered_gem_node::{declarations, investigate, same_method},
    },
    offense::Offense,
    source::SourceFile,
};

pub struct OrderedDependencies;

const METHODS: [&[u8]; 3] = [
    b"add_dependency",
    b"add_runtime_dependency",
    b"add_development_dependency",
];

impl Cop for OrderedDependencies {
    fn name(&self) -> &'static str {
        "Gemspec/OrderedDependencies"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn check_parse(
        &self,
        result: &ruby_prism::ParseResult<'_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // (send (lvar _) {:add_dependency :add_runtime_dependency
        //   :add_development_dependency} (str _) ...); `it` and `_1` are
        // `lvar` reads in the pinned AST, a safe-navigation call is `csend`.
        let root = result.node();
        let found = declarations(&root, file, |call| {
            !call.is_safe_navigation()
                && call.receiver().is_some_and(|receiver| {
                    receiver.as_local_variable_read_node().is_some()
                        || receiver.as_it_local_variable_read_node().is_some()
                })
                && METHODS.contains(&call.name().as_slice())
        });
        investigate(
            self.name(),
            self.severity(),
            &root,
            file,
            &found,
            same_method,
            |current, previous| {
                format!(
                    "Dependencies should be sorted in an alphabetical order within their \
                     section of the gemspec. Dependency `{current}` should appear before \
                     `{previous}`."
                )
            },
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's gemspec/ordered_dependencies_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov) at the fixed defaults, plus
    //! lookalikes and the guarded correction.
    use super::*;
    use crate::{cops::bundler::tests::assert_offenses, offense::Correction};

    fn message(current: &str, previous: &str) -> String {
        format!(
            "Dependencies should be sorted in an alphabetical order within their section of \
             the gemspec. Dependency `{current}` should appear before `{previous}`."
        )
    }

    fn expect_at(path: &str, source: &str, expected: &[(u32, u32, &str, &str)]) {
        let expected: Vec<_> = expected
            .iter()
            .map(|&(line, col, current, previous)| (line, col, message(current, previous)))
            .collect();
        assert_offenses(&OrderedDependencies, path, source, &expected);
    }

    fn expect(source: &str, expected: &[(u32, u32, &str, &str)]) {
        expect_at("example.gemspec", source, expected);
    }

    fn spec(body: &str) -> String {
        format!("Gem::Specification.new do |spec|\n{body}end\n")
    }

    #[test]
    fn ordered_dependency_shared_examples() {
        for method in METHODS {
            let m = std::str::from_utf8(method).unwrap();
            // Sorted.
            expect(
                &spec(&format!("  spec.{m} 'rspec'\n  spec.{m} 'rubocop'\n")),
                &[],
            );
            // Not sorted.
            expect(
                &spec(&format!("  spec.{m} 'rubocop'\n  spec.{m} 'rspec'\n")),
                &[(3, 3, "rspec", "rubocop")],
            );
            // Each group sorted.
            expect(
                &spec(&format!(
                    "  spec.{m} 'rspec'\n  spec.{m} 'rubocop'\n\n  spec.{m} 'hello'\n  spec.{m} 'world'\n"
                )),
                &[],
            );
            // Separated by a multi-line comment: TreatCommentsAsGroupSeparators
            // is fixed true, so the spec's `false` example has no offense.
            expect(
                &spec(&format!(
                    "  # For code quality\n  spec.{m} 'rubocop'\n  # For\n  # test\n  spec.{m} 'rspec'\n"
                )),
                &[],
            );
        }
    }

    #[test]
    fn different_dependency_kinds_are_separate() {
        expect(
            &spec(
                "  spec.add_dependency         'rubocop'\n  spec.add_runtime_dependency 'rspec'\n",
            ),
            &[],
        );
    }

    #[test]
    fn method_calls_on_names_and_variables_are_not_declarations() {
        expect(
            &spec(
                "  spec.add_dependency         'rubocop'.freeze\n  spec.add_runtime_dependency 'rspec'.freeze\n",
            ),
            &[],
        );
        expect(
            &spec(
                "  %w(rubocop-performance rubocop-rails).each { |dep| spec.add_dependency dep }\n  spec.add_dependency 'parser'\n",
            ),
            &[],
        );
    }

    #[test]
    fn both_branches_of_a_conditional_are_sorted_independently() {
        expect(
            &spec(
                "  if spec.respond_to?(:specification_version)\n    spec.add_dependency('rubocop', '>= 1.0')\n    spec.add_dependency('rspec', '>= 3.0')\n  else\n    spec.add_dependency('rubocop', '>= 1.0')\n    spec.add_dependency('rspec', '>= 3.0')\n  end\n",
            ),
            &[(4, 5, "rspec", "rubocop"), (7, 5, "rspec", "rubocop")],
        );
    }

    #[test]
    fn receivers_are_exact_local_variable_reads() {
        // Any local variable, `it` and `_1` included, regardless of its name.
        expect(
            "s = x\ns.add_dependency 'b'\nt = s\nt.add_dependency 'c'\nt.add_dependency 'a'\n",
            &[(5, 1, "a", "c")],
        );
        expect(
            "Gem::Specification.new do |a|\n  a.add_dependency 'b'\nend\nGem::Specification.new do |b|\n  b.add_dependency 'a'\nend\n",
            &[],
        );
        expect(
            "x = 1\nx.add_dependency 'b'\nx.add_dependency 'a'\n",
            &[(3, 1, "a", "b")],
        );
        expect(
            "Gem::Specification.new {\n  it.add_dependency 'b'\n  it.add_dependency 'a'\n}\n",
            &[(3, 3, "a", "b")],
        );
        expect(
            "Gem::Specification.new {\n  _1.add_dependency 'b'\n  _1.add_dependency 'a'\n}\n",
            &[(3, 3, "a", "b")],
        );
        // Not `(send (lvar _) ...)`: method/ivar/constant receivers, safe
        // navigation, receiverless calls, other methods and non-str names.
        expect(
            &spec(
                "  spec.add_dependency 'z'\n  spec&.add_dependency 'b'\n  @spec.add_dependency 'b'\n  Spec.add_dependency 'b'\n  add_dependency 'b'\n  spec.add_dependencies 'b'\n  spec.add_dependency :b\n  spec.add_dependency \"#{b}\"\n  self.add_dependency 'b'\n",
            ),
            &[],
        );
        expect(
            "def spec = x\nspec.add_dependency 'b'\nspec.add_dependency 'a'\n",
            &[],
        );
    }

    #[test]
    fn paths_and_positions() {
        let source = spec("  spec.add_dependency 'b'\n  spec.add_dependency 'a'\n");
        for path in ["Gemfile", "x.rb", "gemspec"] {
            expect_at(path, &source, &[]);
        }
        expect_at("nested/x.gemspec", &source, &[(3, 3, "a", "b")]);
        expect(
            &format!("\u{feff}{}", source.replace('\n', "\r\n")),
            &[(3, 3, "a", "b")],
        );
        expect(
            &spec("  spec.add_dependency 'b', <<~V\n    1\n  V\n  spec.add_dependency 'a'\n"),
            &[],
        );
        expect(
            &spec("  spec.add_dependency 'b' do\n  end\n  spec.add_dependency 'a'\n"),
            &[],
        );
    }

    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_path_with_corrections(
            &[&OrderedDependencies],
            "example.gemspec",
            source,
        )
    }

    fn corrections(source: &str) -> Vec<Option<Correction>> {
        offenses(source).into_iter().map(|o| o.correction).collect()
    }

    #[test]
    fn live_gemspecs_keep_findings_without_edits() {
        for source in [
            spec("  spec.add_dependency 'rubocop'\n  spec.add_dependency 'rspec'\n"),
            "spec = x\nif y\n  spec.add_dependency 'b'\n  spec.add_dependency 'a'\nend\n".to_owned(),
            "if false\n  spec = 1\n  spec.add_dependency 'b'\n  spec.add_dependency 'a'\nend\n__END__\n"
                .to_owned(),
            "if false\n  spec = 1\n  spec.add_dependency 'b'\n  spec.add_dependency 'a'\n  $0\nend\n"
                .to_owned(),
            "if false\n  s = 1\n  s.add_dependency 'b', x = 1\n  s.add_dependency 'a', x\nend\n"
                .to_owned(),
            "if false\n  s = 1\n  s.add_dependency 'b', x\n  s.add_dependency 'a', x = 1\nend\n"
                .to_owned(),
        ] {
            assert_eq!(corrections(&source), vec![None], "{source}");
        }
    }

    #[test]
    fn canonical_swap_in_a_closed_inactive_envelope() {
        let before = "if false\n  Gem::Specification.new do |spec|\n    spec.add_dependency('rubocop', '>= 1.0') # x\n    spec.add_dependency('rspec', '>= 3.0')\n  end\nend\n";
        let after = "if false\n  Gem::Specification.new do |spec|\n    spec.add_dependency('rspec', '>= 3.0')\n    spec.add_dependency('rubocop', '>= 1.0') # x\n  end\nend\n";
        assert_eq!(crate::parser::parse(before.as_bytes()).errors().count(), 0);
        let found = offenses(before);
        assert_eq!(found.len(), 1);
        let applied = crate::corrections::apply(before.as_bytes(), &found).unwrap();
        assert_eq!(String::from_utf8(applied.bytes).unwrap(), after);
    }
}
