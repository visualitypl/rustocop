//! `Bundler/GemVersion` (opt-in): fixed `EnforcedStyle: required` and
//! `AllowedGems: []`. A gem declaration needs a version specification (any
//! `str` argument matching the pinned pattern, the name included) or a
//! `branch:`/`ref:`/`tag:` string commit reference.
use ruby_prism::Node;

use super::declarations::{gem_name, str_value};
use crate::{
    cops::{Cop, lint::reference_basic_literal::is_basic_literal},
    offense::Offense,
    source::SourceFile,
};

pub struct GemVersion;

impl Cop for GemVersion {
    fn name(&self) -> &'static str {
        "Bundler/GemVersion"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &crate::cops::NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if gem_name(&call, file).is_none() {
            return;
        }
        let arguments: Vec<_> = call
            .arguments()
            .iter()
            .flat_map(|a| a.arguments().iter())
            .collect();
        if arguments.iter().any(|a| {
            str_value(a, file).is_some_and(|v| version_specification(&v))
                || commit_reference(a, file)
        }) {
            return;
        }
        let (line, col) =
            crate::cops::lint::reference_line_col(call.location().start_offset(), file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Gem version specification is required.".into(),
            correction: None,
        });
    }
}

/// `/^\s*[~<>=]*\s*[0-9.]+/`, where `^` is the start of any line.
fn version_specification(value: &[u8]) -> bool {
    std::iter::once(0)
        .chain(
            value
                .iter()
                .enumerate()
                .filter(|&(_, &b)| b == b'\n')
                .map(|(i, _)| i + 1),
        )
        .any(|start| {
            let mut rest = &value[start..];
            // Ruby's `\s` here is ASCII whitespace, newlines included.
            let skip = |rest: &mut &[u8], pred: fn(u8) -> bool| {
                while rest.first().is_some_and(|&b| pred(b)) {
                    *rest = &rest[1..];
                }
            };
            skip(&mut rest, |b| {
                matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
            });
            skip(&mut rest, |b| matches!(b, b'~' | b'<' | b'>' | b'='));
            skip(&mut rest, |b| {
                matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
            });
            rest.first()
                .is_some_and(|&b| b.is_ascii_digit() || b == b'.')
        })
}

/// `(hash <(pair (sym {:branch :ref :tag}) (str _)) ...>)`.
fn commit_reference(node: &Node<'_>, file: &SourceFile) -> bool {
    let elements = match node {
        Node::KeywordHashNode { .. } => node.as_keyword_hash_node().unwrap().elements(),
        Node::HashNode { .. } => node.as_hash_node().unwrap().elements(),
        _ => return false,
    };
    elements.iter().any(|element| {
        let Some(pair) = element.as_assoc_node() else {
            return false;
        };
        let key = pair.key();
        key.as_symbol_node().is_some_and(|symbol| {
            is_basic_literal(&key, file)
                && matches!(symbol.unescaped(), b"branch" | b"ref" | b"tag")
        }) && str_value(&pair.value(), file).is_some()
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's bundler/gem_version_spec.rb (MIT, Copyright (c)
    //! 2012-26 Bozhidar Batsov), every example at the fixed defaults
    //! (`EnforcedStyle: required`, `AllowedGems: []`), plus lookalikes.
    use super::*;
    use crate::cops::bundler::tests::{assert_offenses, lint};

    const MSG: &str = "Gem version specification is required.";

    fn expect(source: &str, expected: &[(u32, u32)]) {
        let expected: Vec<_> = expected
            .iter()
            .map(|&(line, col)| (line, col, MSG.to_owned()))
            .collect();
        assert_offenses(&GemVersion, "Gemfile", source, &expected);
    }

    #[test]
    fn flags_gems_that_do_not_specify_a_version() {
        expect(
            "gem 'rubocop'\ngem 'rubocop', require: false\n",
            &[(1, 1), (2, 1)],
        );
        // AllowedGems is fixed empty.
        expect("gem 'rspec'\n", &[(1, 1)]);
    }

    #[test]
    fn versions_and_commit_references() {
        expect(
            "gem 'rubocop', '>=1.10.0'\ngem 'rubocop', '~> 1'\ngem 'rubocop', '~> 1.12', require: false\ngem 'rubocop', '>= 1.5.0', '< 1.10.0', git: 'https://github.com/rubocop/rubocop'\ngem 'rubocop', branch: 'feature-branch'\ngem 'rubocop', ref: 'b3f37bc7f'\ngem 'rubocop', tag: 'v1'\n",
            &[],
        );
        // The forbidden-style examples, at the fixed required style.
        expect("gem 'rspec', '~> 3.10'\n", &[]);
    }

    #[test]
    fn pinned_pattern_lookalikes() {
        // Any str argument counts, the gem name included.
        expect("gem '1password'\n", &[]);
        expect("gem 'a', 'x', '1'\n", &[]);
        expect("gem 'a', \"x\\n2\"\n", &[]);
        expect("gem 'a', '>= x'\n", &[(1, 1)]);
        expect("gem 'a', :'1'\n", &[(1, 1)]);
        expect("gem 'a', \"#{v}\"\n", &[(1, 1)]);
        expect("gem 'a', version\n", &[(1, 1)]);
        expect("gem 'a', branch: :main\n", &[(1, 1)]);
        expect("gem 'a', 'branch' => 'main'\n", &[(1, 1)]);
        expect("gem 'a', \"branch\": 'main'\n", &[]);
        expect("gem 'a', { tag: 'v1' }\n", &[]);
        expect("gem 'a', github: 'x/y'\n", &[(1, 1)]);
        assert!(lint(&GemVersion, "a.rb", "gem 'a'\n").is_empty());
        expect("gem :a\nfoo.gem 'a'\ngem \"#{a}\"\n", &[]);
    }
}
