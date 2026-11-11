//! `Bundler/DuplicatedGem`: a gem's requirements listed more than once in a
//! supplied Gemfile, except when every declaration sits in the branches of
//! the first declaration's conditional (pinned structural equality).
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use super::declarations::{
    SendKey, branch_keys, conditional_root, gem_name, receiverless_calls, send_key, text,
};
use crate::{cops::Cop, offense::Offense, source::SourceFile};

pub struct DuplicatedGem;

struct Declaration {
    start: usize,
    name: Vec<u8>,
    key: SendKey,
    /// The send keys of the conditional this declaration is a branch of.
    branches: Option<Rc<HashSet<SendKey>>>,
}

impl Cop for DuplicatedGem {
    fn name(&self) -> &'static str {
        "Bundler/DuplicatedGem"
    }

    fn check_parse(
        &self,
        result: &ruby_prism::ParseResult<'_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // One investigation-local cache per conditional, shared by its gems.
        let mut conditionals: HashMap<(usize, usize), Rc<HashSet<SendKey>>> = HashMap::new();
        let declarations = receiverless_calls(&result.node(), b"gem", |call, ancestors| {
            let name = gem_name(call, file)?;
            let branches = conditional_root(ancestors).map(|root| {
                let span = (root.location().start_offset(), root.location().end_offset());
                conditionals
                    .entry(span)
                    .or_insert_with(|| Rc::new(branch_keys(root)))
                    .clone()
            });
            Some(Declaration {
                start: call.location().start_offset(),
                name,
                key: send_key(call),
                branches,
            })
        });
        let mut groups: Vec<Vec<&Declaration>> = Vec::new();
        let mut index: HashMap<&[u8], usize> = HashMap::new();
        for declaration in &declarations {
            let next = groups.len();
            let group = *index.entry(&declaration.name).or_insert(next);
            if group == next {
                groups.push(Vec::new());
            }
            groups[group].push(declaration);
        }
        for group in groups.iter().filter(|group| group.len() > 1) {
            let first = group[0];
            if let Some(branches) = &first.branches
                && group.iter().all(|d| branches.contains(&d.key))
            {
                continue; // conditional_declaration?
            }
            let (first_line, _) = crate::cops::lint::reference_line_col(first.start, file);
            for declaration in &group[1..] {
                let (line, col) = crate::cops::lint::reference_line_col(declaration.start, file);
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    line,
                    col,
                    end_line: line,
                    message: format!(
                        "Gem `{}` requirements already given on line {first_line} of the Gemfile.",
                        text(&declaration.name)
                    ),
                    correction: None,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's bundler/duplicated_gem_spec.rb (MIT, Copyright
    //! (c) 2012-26 Bozhidar Batsov), plus lookalikes.
    use super::*;
    use crate::cops::bundler::tests::{assert_offenses, lint};

    const MSG: &str = "requirements already given on line";

    fn expect(source: &str, expected: &[(u32, u32, &str, u32)]) {
        let expected: Vec<_> = expected
            .iter()
            .map(|&(line, col, gem, first)| {
                (
                    line,
                    col,
                    format!("Gem `{gem}` {MSG} {first} of the Gemfile."),
                )
            })
            .collect();
        assert_offenses(&DuplicatedGem, "Gemfile", source, &expected);
    }

    #[test]
    fn ruby_files_and_empty_gemfiles_are_ignored() {
        assert!(lint(&DuplicatedGem, "foo.rb", "gem('rubocop')\ngem('rubocop')\n").is_empty());
        expect("", &[]);
    }

    #[test]
    fn no_duplicates() {
        expect("gem 'rubocop'\ngem 'flog'\n", &[]);
    }

    #[test]
    fn duplicated_in_default_group() {
        expect(
            "source 'https://rubygems.org'\ngem 'rubocop'\ngem 'rubocop'\n",
            &[(3, 1, "rubocop", 2)],
        );
    }

    #[test]
    fn duplicated_in_a_block() {
        expect(
            "gem 'rubocop'\ngroup :development do\n  gem 'rubocop', path: '/path/to/gem'\nend\n",
            &[(3, 3, "rubocop", 1)],
        );
    }

    #[test]
    fn default_group_conditionally_duplicated() {
        expect(
            "gem 'rubocop'\nif Dir.exist? local\n  gem 'rubocop', path: local\nelse\n  gem 'rubocop', '~> 0.90.0'\nend\n",
            &[(3, 3, "rubocop", 1), (5, 3, "rubocop", 1)],
        );
    }

    #[test]
    fn conditional_branches_are_not_duplicates() {
        expect(
            "if Dir.exist?(local)\n  gem 'rubocop', path: local\n  gem 'flog', path: local\nelse\n  gem 'rubocop', '~> 0.90.0'\nend\n",
            &[],
        );
        expect(
            "if Dir.exist?(local)\n  gem 'rubocop', path: local\nelsif ENV['RUBOCOP_VERSION'] == 'master'\n  gem 'rubocop', git: 'https://github.com/rubocop/rubocop.git'\nelsif (version = ENV['RUBOCOP_VERSION'])\n  gem 'rubocop', version\nelse\n  gem 'rubocop', '~> 0.90.0'\nend\n",
            &[],
        );
        expect(
            "case\nwhen Dir.exist?(local)\n  gem 'rubocop', path: local\nwhen ENV['RUBOCOP_VERSION'] == 'master'\n  gem 'rubocop', git: 'https://github.com/rubocop/rubocop.git'\nelse\n  gem 'rubocop', '~> 0.90.0'\nend\n",
            &[],
        );
        expect(
            "case\nwhen Dir.exist?(local)\n  gem 'rubocop', path: local\nwhen ENV['RUBOCOP_VERSION'] == 'master'\n  # no-op, do nothing club\nelse\n  gem 'rubocop', '~> 0.90.0'\nend\n",
            &[],
        );
    }

    #[test]
    fn documented_conditional_example() {
        expect(
            "if Dir.exist?(local)\n  gem 'rubocop', path: local\nelsif ENV['RUBOCOP_VERSION'] == 'master'\n  gem 'rubocop', git: 'https://github.com/rubocop/rubocop.git'\nelse\n  gem 'rubocop', '~> 0.90.0'\nend\n",
            &[],
        );
        expect(
            "group :development do\n  gem 'rubocop'\nend\n\ngroup :test do\n  gem 'rubocop'\nend\n",
            &[(6, 3, "rubocop", 2)],
        );
    }

    #[test]
    fn only_receiverless_gem_calls_with_a_plain_string_name() {
        expect(
            "gem :rubocop\ngem :rubocop\nfoo.gem 'a'\nfoo.gem 'a'\ngem \"#{x}\"\ngem \"#{x}\"\ngem\ngem\ngems 'a'\ngems 'a'\n",
            &[],
        );
        // Spelling is irrelevant: the pinned key is the str value.
        expect(
            "gem 'a'\ngem(\"a\")\ngem %q(a), '1'\ngem ?a\n",
            &[(2, 1, "a", 1), (3, 1, "a", 1), (4, 1, "a", 1)],
        );
        // An adjacent concatenation and a multi-line literal are dstr.
        expect("gem 'ab'\ngem 'a' 'b'\ngem 'a\nb'\ngem 'a\nb'\n", &[]);
    }

    #[test]
    fn block_and_nested_declarations_are_found_in_parser_order() {
        expect("gem 'a' do\n  gem 'a'\nend\n", &[(2, 3, "a", 1)]);
        // `unless` stores its else body before its body in the pinned AST.
        expect(
            "unless c\n  gem 'a'\nelse\n  foo do\n    gem 'a', '1'\n  end\nend\n",
            &[(2, 3, "a", 5)],
        );
    }

    #[test]
    fn first_declaration_outside_a_conditional_reports_all_others() {
        expect("gem 'a'\nif x\n  gem 'a'\nend\n", &[(3, 3, "a", 1)]);
        // Nested in a block inside a branch: the nearest parent is the block.
        expect(
            "if x\n  group :a do\n    gem 'a'\n  end\nelse\n  gem 'a'\nend\n",
            &[(6, 3, "a", 3)],
        );
    }

    #[test]
    fn branch_membership_is_structural_equality_not_identity() {
        // A later top-level declaration equal to a branch counts as "within".
        expect("if x\n  gem 'a'\nelse\n  gem 'a', '1'\nend\ngem 'a'\n", &[]);
        expect(
            "if x\n  gem 'a'\nelse\n  gem 'a', '1'\nend\ngem 'a', '2'\n",
            &[(4, 3, "a", 2), (6, 1, "a", 2)],
        );
        // Direct children of a multi-statement branch, a block branch and a
        // nested non-elsif conditional are members too.
        expect(
            "if x\n  gem 'a'\n  y\nelse\n  group :g do\n    gem 'a', '1'\n  end\nend\n",
            &[],
        );
        expect(
            "if x\n  gem 'a'\nelse\n  if y\n    gem 'a', '1'\n  else\n    gem 'a', '2'\n  end\nend\n",
            &[],
        );
        expect(
            "if x\n  gem 'a'\nelse\n  if y\n    z\n    gem 'a', '1'\n  end\nend\n",
            &[(6, 5, "a", 2)],
        );
        // Ternaries and modifiers are `if` nodes as well.
        expect("x ? gem('a') : gem('a', '1')\n", &[]);
        expect("gem 'a' if x\ngem 'a' unless x\n", &[]);
        expect("unless x\n  gem 'a'\nelse\n  gem 'a', '1'\nend\n", &[]);
    }

    #[test]
    fn first_declaration_in_case_else_is_not_conditional() {
        expect(
            "case x\nwhen 1\n  y\nelse\n  gem 'a'\nend\ngem 'a'\n",
            &[(7, 1, "a", 5)],
        );
    }

    #[test]
    fn crlf_bom_and_message_text() {
        expect("gem 'a'\r\ngem 'a'\r\n", &[(2, 1, "a", 1)]);
        expect("\u{feff}gem 'a'\ngem 'a'\n", &[(2, 1, "a", 1)]);
        expect("gem 'café'\ngem \"caf\\u00e9\"\n", &[(2, 1, "café", 1)]);
    }
}
