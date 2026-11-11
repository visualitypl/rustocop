//! `Bundler/OrderedGems`: `gem` declarations on adjacent lines of a supplied
//! Gemfile that are not in case-insensitive alphabetical order. Fixed
//! `TreatCommentsAsGroupSeparators: true` and `ConsiderPunctuation: false`.
use super::ordered_gem_node::{declarations, investigate};
use crate::{cops::Cop, offense::Offense, source::SourceFile};

pub struct OrderedGems;

impl Cop for OrderedGems {
    fn name(&self) -> &'static str {
        "Bundler/OrderedGems"
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
        // (send nil? :gem str ...)
        let root = result.node();
        let found = declarations(&root, file, |call| {
            call.receiver().is_none() && call.name().as_slice() == b"gem"
        });
        investigate(
            self.name(),
            self.severity(),
            &root,
            file,
            &found,
            |_, _| true,
            |current, previous| {
                format!(
                    "Gems should be sorted in an alphabetical order within their section of \
                     the Gemfile. Gem `{current}` should appear before `{previous}`."
                )
            },
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's bundler/ordered_gems_spec.rb (MIT, Copyright
    //! (c) 2012-26 Bozhidar Batsov) at the fixed defaults, plus lookalikes
    //! and the guarded correction.
    use super::*;
    use crate::{
        cops::bundler::tests::{assert_offenses, lint},
        offense::Correction,
    };

    fn message(current: &str, previous: &str) -> String {
        format!(
            "Gems should be sorted in an alphabetical order within their section of the \
             Gemfile. Gem `{current}` should appear before `{previous}`."
        )
    }

    fn expect(source: &str, expected: &[(u32, u32, &str, &str)]) {
        let expected: Vec<_> = expected
            .iter()
            .map(|&(line, col, current, previous)| (line, col, message(current, previous)))
            .collect();
        assert_offenses(&OrderedGems, "Gemfile", source, &expected);
    }

    #[test]
    fn sorted_gems() {
        expect("gem 'rspec'\ngem 'rubocop'\n", &[]);
    }

    #[test]
    fn a_gem_referenced_from_a_variable_breaks_the_group() {
        expect(
            "gem 'rspec'\ngem ENV['env_key_undefined'] if ENV.key?('env_key_undefined')\ngem 'rubocop'\n",
            &[],
        );
        expect(
            "gem 'rubocop'\ngem ENV['env_key_undefined'] if ENV.key?('env_key_undefined')\ngem 'ast'\n",
            &[],
        );
    }

    #[test]
    fn unsorted_gems() {
        expect(
            "gem 'rubocop'\ngem 'rspec'\n",
            &[(2, 1, "rspec", "rubocop")],
        );
        // Without a final newline.
        expect("gem 'rubocop'\ngem 'rspec'", &[(2, 1, "rspec", "rubocop")]);
    }

    #[test]
    fn blank_lines_separate_sorted_groups() {
        expect(
            "gem 'rspec'\ngem 'rubocop'\n\ngem 'hello'\ngem 'world'\n",
            &[],
        );
    }

    #[test]
    fn a_multiline_declaration_ends_on_its_last_line() {
        expect(
            "gem 'rubocop',\n    '0.1.1'\ngem 'rspec'\n",
            &[(3, 1, "rspec", "rubocop")],
        );
    }

    #[test]
    fn a_comment_only_gemfile() {
        expect("# Gemfile", &[]);
        expect("", &[]);
    }

    #[test]
    fn each_unsorted_group_reports_each_adjacent_inversion() {
        expect(
            "gem \"d\"\ngem \"b\"\ngem \"e\"\ngem \"a\"\ngem \"c\"\n\ngem \"h\"\ngem \"g\"\ngem \"j\"\ngem \"f\"\ngem \"i\"\n",
            &[
                (2, 1, "b", "d"),
                (4, 1, "a", "e"),
                (8, 1, "g", "h"),
                (10, 1, "f", "j"),
            ],
        );
    }

    #[test]
    fn comments_separate_groups_at_the_fixed_default() {
        // The spec's TreatCommentsAsGroupSeparators: true example.
        expect(
            "# For code quality\ngem 'rubocop'\n# For\n# test\ngem 'rspec'\n",
            &[],
        );
        // Its `false` example runs at the fixed `true`: no offense.
        expect(
            "# For code quality\ngem 'rubocop'\n# For tests\ngem 'rspec'\n",
            &[],
        );
    }

    #[test]
    fn inline_comments_do_not_separate() {
        expect(
            "gem 'rubocop' # For code quality\ngem 'pry'\ngem 'rspec'   # For test\n",
            &[(2, 1, "pry", "rubocop")],
        );
    }

    #[test]
    fn punctuation_and_case_are_ignored() {
        expect("gem 'paperclip'\ngem 'paper_trail'\n", &[]);
        expect("gem 'a'\ngem 'Z'\n", &[]);
        expect("gem 'Z'\ngem 'a'\n", &[(2, 1, "a", "Z")]);
        // Fixed ConsiderPunctuation: false.
        expect(
            "gem 'active-admin-some_plugin'\ngem 'active_admin_other_plugin'\ngem 'activeadmin'\n",
            &[
                (
                    2,
                    1,
                    "active_admin_other_plugin",
                    "active-admin-some_plugin",
                ),
                (3, 1, "activeadmin", "active_admin_other_plugin"),
            ],
        );
    }

    #[test]
    fn duplicated_gems_in_a_group() {
        expect(
            "gem 'a'\n\ngroup :development do\n  gem 'b'\n  gem 'c'\n  gem 'b'\nend\n",
            &[(6, 3, "b", "c")],
        );
    }

    #[test]
    fn pinned_matcher_lookalikes() {
        // Not `(send nil? :gem str ...)`: no declaration, and no group break.
        expect(
            "gem 'b'\ngem :a\nfoo.gem 'a'\ngem \"#{x}\"\ngem\ngems 'a'\nself.gem 'a'\ngem 'a' 'b'\n",
            &[],
        );
        // Plain `str` spellings are declarations.
        expect(
            "gem 'd'\ngem(\"c\")\ngem %q(b)\ngem ?a\n",
            &[(2, 1, "c", "d"), (3, 1, "b", "c"), (4, 1, "a", "b")],
        );
        // Escapes compare by value.
        expect("gem \"\\x62\"\ngem 'a'\n", &[(2, 1, "a", "b")]);
        // Only supplied Gemfiles.
        assert!(lint(&OrderedGems, "foo.rb", "gem 'b'\ngem 'a'\n").is_empty());
        assert!(lint(&OrderedGems, "a.gemspec", "gem 'b'\ngem 'a'\n").is_empty());
    }

    #[test]
    fn adjacency_uses_the_send_not_its_block() {
        // A literal block is not part of the pinned send.
        expect("gem 'b' do\nend\ngem 'a'\n", &[]);
        expect(
            "gem 'c'\ngem 'b' do\n  gem 'a'\nend\n",
            &[(2, 1, "b", "c"), (3, 3, "a", "b")],
        );
        // Modifiers and same-line declarations are still declarations.
        expect("gem 'b' if x\ngem 'a'\n", &[(2, 1, "a", "b")]);
        expect("gem 'b'; gem 'a'\n", &[]);
        expect("gem 'c'\ngem 'b'; gem 'a'\n", &[(2, 1, "b", "c")]);
        // Parser pre-order: an unless stores its else body first.
        expect("unless x\n  gem 'b'\nelse\n  gem 'a'\nend\n", &[]);
        // Nested declarations follow the outer one.
        expect("gem 'b', gem('a')\n", &[]);
        expect("gem 'c'\ngem 'b', gem('a')\n", &[(2, 1, "b", "c")]);
    }

    #[test]
    fn encodings_and_positions() {
        expect("\u{feff}gem 'b'\ngem 'a'\n", &[(2, 1, "a", "b")]);
        expect("gem 'b'\r\ngem 'a'\r\n", &[(2, 1, "a", "b")]);
        expect("gem 'é'\ngem 'e'\n", &[(2, 1, "e", "é")]);
        expect("gem 'b'\n__END__\ngem 'a'\n", &[]);
        expect("gem 'b'\nx = 'é'; gem 'a'\n", &[(2, 10, "a", "b")]);
    }

    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_path_with_corrections(&[&OrderedGems], "Gemfile", source)
    }

    fn corrections(source: &str) -> Vec<Option<Correction>> {
        offenses(source).into_iter().map(|o| o.correction).collect()
    }

    /// Apply every proposed edit once (single pass) and check both sides
    /// with the fixed Ruby 3.4 grammar.
    fn assert_correction(before: &str, after: &str) {
        assert_eq!(crate::parser::parse(before.as_bytes()).errors().count(), 0);
        let found = offenses(before);
        assert!(found.iter().all(|o| o.correction.is_some()), "{before}");
        let applied = crate::corrections::apply(before.as_bytes(), &found).unwrap();
        assert_eq!(String::from_utf8(applied.bytes.clone()).unwrap(), after);
        assert_eq!(crate::parser::parse(&applied.bytes).errors().count(), 0);
    }

    #[test]
    fn live_gemfiles_keep_findings_without_edits() {
        for source in [
            "gem 'rubocop'\ngem 'rspec'\n",
            "gem 'rubocop'\ngem 'rspec'",
            "source 'https://rubygems.org'\ngem 'b'\ngem 'a'\n",
            "if x\n  gem 'b'\n  gem 'a'\nend\n",
            "if false\n  gem 'b'\n  gem 'a'\nend\nx\n",
            "if false\n  gem 'b'\n  gem 'a'\nelse\nend\n",
            "unless true\n  gem 'b'\n  gem 'a'\nend\n",
            "if false\n  gem 'b'\n  gem 'a'\nend\n__END__\n",
            "group :x do\n  gem 'b'\n  gem 'a'\nend\n",
        ] {
            assert_eq!(corrections(source), vec![None], "{source}");
        }
    }

    #[test]
    fn unmovable_declarations_keep_their_bytes_in_an_inactive_program() {
        for source in [
            "if false\n  gem 'b' if x\n  gem 'a'\nend\n",
            "if false\n  gem 'b'\n  gem 'a'; x\nend\n",
            "if false\n  x; gem 'b'\n  gem 'a'\nend\n",
            "if false\n  gem 'c'\n  gem 'b' do\n  end\nend\n",
            "if false\n  gem 'b'\n  gem 'a', <<~X\n    x\n  X\nend\n",
            "if false\n  gem 'b', <<~X\n  X\n  gem 'a'\nend\n",
            "if false\n  (gem 'b')\n  gem 'a'\nend\n",
        ] {
            let found = corrections(source);
            assert!(found.iter().all(Option::is_none), "{source}: {found:?}");
        }
        // A listed source observer holds the edit.
        assert_eq!(
            corrections("if false\n  gem 'b'\n  gem 'a'\n  __dir__\nend\n"),
            vec![None]
        );
    }

    #[test]
    fn declarations_that_bind_locals_keep_their_bytes() {
        // Swapping would turn a local read into a method call (or back).
        for source in [
            "if false\n  gem 'b', x = 1\n  gem 'a', x\nend\n",
            "if false\n  gem 'b', x\n  gem 'a', x = 1\nend\n",
            "if false\n  gem 'b', (x ||= 1)\n  gem 'a', x\nend\n",
            "if false\n  gem 'b', (x, y = 1, 2)\n  gem 'a', y\nend\n",
            "if false\n  gem 'b', (/(?<x>.)/ =~ 'c')\n  gem 'a', x\nend\n",
            "if false\n  gem 'b', (1 in x)\n  gem 'a', x\nend\n",
        ] {
            assert_eq!(crate::parser::parse(source.as_bytes()).errors().count(), 0);
            assert_eq!(corrections(source), vec![None], "{source}");
        }
        // Reads alone do not change binding when swapped.
        assert_correction(
            "if false\n  x = 1\n  gem 'b', x\n  gem 'a', x\nend\n",
            "if false\n  x = 1\n  gem 'a', x\n  gem 'b', x\nend\n",
        );
    }

    #[test]
    fn canonical_swap_in_a_closed_inactive_envelope() {
        assert_correction(
            "if false\n  gem 'rubocop'\n  gem 'rspec'\nend\n",
            "if false\n  gem 'rspec'\n  gem 'rubocop'\nend\n",
        );
        assert_correction(
            "if false\r\n  gem 'rubocop'\r\n  gem 'rspec'\r\nend\r\n",
            "if false\r\n  gem 'rspec'\r\n  gem 'rubocop'\r\nend\r\n",
        );
        // Multi-line declarations and trailing comments travel whole.
        assert_correction(
            "if false\n  gem 'rubocop',\n      '0.1.1'\n  gem 'rspec'\nend\n",
            "if false\n  gem 'rspec'\n  gem 'rubocop',\n      '0.1.1'\nend\n",
        );
        assert_correction(
            "if false\n  gem 'rubocop' # For code quality\n  gem 'pry'\n  gem 'rspec'   # For test\nend\n",
            "if false\n  gem 'pry'\n  gem 'rubocop' # For code quality\n  gem 'rspec'   # For test\nend\n",
        );
        assert_correction(
            "if false\n  group :development do\n    gem 'c'\n    gem 'b'\n  end\nend\n",
            "if false\n  group :development do\n    gem 'b'\n    gem 'c'\n  end\nend\n",
        );
        // Single pass: overlapping swaps wait for the next run.
        let source = "if false\n  gem 'c'\n  gem 'b'\n  gem 'a'\nend\n";
        let found = offenses(source);
        assert_eq!(found.len(), 2);
        let applied = crate::corrections::apply(source.as_bytes(), &found).unwrap();
        assert_eq!(
            applied.bytes,
            b"if false\n  gem 'b'\n  gem 'c'\n  gem 'a'\nend\n"
        );
    }
}
