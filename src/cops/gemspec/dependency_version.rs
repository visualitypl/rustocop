//! `Gemspec/DependencyVersion` (opt-in), at the fixed `EnforcedStyle:
//! required` and `AllowedGems: []`: a dependency declared on the first
//! `Gem::Specification.new` block variable with neither a version `str` nor
//! a `branch:`/`ref:`/`tag:` commit reference. Detection only.
use ruby_prism::{CallNode, Node};

use super::help::specification;
use crate::{
    cops::{
        Cop,
        bundler::{
            declarations::{parser_order_calls, str_value},
            ordered_gem_node::send_end,
        },
        lint::reference_basic_literal::is_basic_literal,
    },
    offense::Offense,
    source::SourceFile,
};

pub struct DependencyVersion;

const METHODS: [&[u8]; 3] = [
    b"add_dependency",
    b"add_runtime_dependency",
    b"add_development_dependency",
];

impl Cop for DependencyVersion {
    fn name(&self) -> &'static str {
        "Gemspec/DependencyVersion"
    }

    fn check_parse(
        &self,
        result: &ruby_prism::ParseResult<'_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // `match_block_variable_name?` compares with the block variable of
        // the FIRST `gem_specification` in the parser's pre-order; one walk
        // collects both that name and the candidate sends.
        enum Found<'pr> {
            Specification(Vec<u8>),
            Candidate(Node<'pr>, Vec<u8>),
        }
        let root = result.node();
        let found = parser_order_calls(&root, |call, _| {
            if let Some(name) = specification(call) {
                return Some(Found::Specification(name));
            }
            let receiver = receiver_name(call)?;
            (!call.is_safe_navigation() && METHODS.contains(&call.name().as_slice()))
                .then(|| Found::Candidate(call.as_node(), receiver))
        });
        let Some(first) = found.iter().find_map(|found| match found {
            Found::Specification(name) => Some(name.clone()),
            Found::Candidate(..) => None,
        }) else {
            return;
        };
        for found in found {
            let Found::Candidate(node, receiver) = found else {
                continue;
            };
            let Some(call) = node.as_call_node() else {
                continue;
            };
            if receiver != first {
                continue;
            }
            let arguments = arguments(&call);
            // `allowed_gem?` reads `node.first_argument.str_content`: a
            // declaration without arguments raises upstream (no finding).
            if arguments.is_empty()
                || arguments
                    .iter()
                    .any(|a| version_specification(a, file) || commit_reference(a, file))
            {
                continue;
            }
            let start = call.location().start_offset();
            let (line, col) = file.reference_line_col(start);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                col,
                end_line: file.line_of(send_end(&call)),
                message: "Dependency version specification is required.".into(),
                correction: None,
            });
        }
    }
}

/// `(lvar name)`: the pinned AST reads `it` as `(lvar :it)`.
fn receiver_name(call: &CallNode<'_>) -> Option<Vec<u8>> {
    let receiver = call.receiver()?;
    if let Some(local) = receiver.as_local_variable_read_node() {
        return Some(local.name().as_slice().to_vec());
    }
    receiver
        .as_it_local_variable_read_node()
        .map(|_| b"it".to_vec())
}

/// The pinned `send` arguments: a block pass is one, a literal block is not.
fn arguments<'pr>(call: &CallNode<'pr>) -> Vec<Node<'pr>> {
    let mut arguments: Vec<_> = call
        .arguments()
        .map(|a| a.arguments().iter().collect())
        .unwrap_or_default();
    if let Some(block) = call.block()
        && block.as_block_argument_node().is_some()
    {
        arguments.push(block);
    }
    arguments
}

/// `(str #version_specification?)`: `/^\s*[~<>=]*\s*[0-9.]+/` matches at
/// the start of any line of the value. The greedy scan is exact: neither
/// optional run can give back characters the next one would accept.
fn version_specification(node: &Node<'_>, file: &SourceFile) -> bool {
    let Some(value) = str_value(node, file) else {
        return false;
    };
    let space = |b: &u8| matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r');
    let matches_at = |start: usize| {
        let mut rest = &value[start..];
        for skip in [
            &space as &dyn Fn(&u8) -> bool,
            &|b: &u8| matches!(b, b'~' | b'<' | b'>' | b'='),
            &space,
        ] {
            let n = rest.iter().take_while(|b| skip(b)).count();
            rest = &rest[n..];
        }
        rest.first()
            .is_some_and(|b| b.is_ascii_digit() || *b == b'.')
    };
    std::iter::once(0)
        .chain(
            value
                .iter()
                .enumerate()
                .filter(|(_, b)| **b == b'\n')
                .map(|(i, _)| i + 1),
        )
        .any(matches_at)
}

/// `(hash <(pair (sym {:branch :ref :tag}) (str _)) ...>)`, braces or not.
fn commit_reference(node: &Node<'_>, file: &SourceFile) -> bool {
    let elements = if let Some(hash) = node.as_hash_node() {
        hash.elements()
    } else if let Some(hash) = node.as_keyword_hash_node() {
        hash.elements()
    } else {
        return false;
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
    //! Ported from RuboCop's gemspec/dependency_version_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov) at the fixed `EnforcedStyle:
    //! required` and `AllowedGems: []`, plus lookalikes.
    use super::*;
    use crate::pipeline::lint_path_with_corrections;

    const MSG: &str = "Dependency version specification is required.";

    fn found_at(path: &str, source: &str) -> Vec<(u32, u32, u32)> {
        let mut found: Vec<_> = lint_path_with_corrections(&[&DependencyVersion], path, source)
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

    fn expect(source: &str, expected: &[(u32, u32)]) {
        let found: Vec<_> = found_at("example.gemspec", source)
            .into_iter()
            .map(|(line, col, _)| (line, col))
            .collect();
        assert_eq!(found, expected, "{source}");
    }

    fn spec(body: &str) -> String {
        format!("Gem::Specification.new do |spec|\n{body}end\n")
    }

    #[test]
    fn required_style_for_every_dependency_method() {
        for method in METHODS {
            let m = std::str::from_utf8(method).unwrap();
            for offense in [
                format!("  spec.{m} 'parser'\n"),
                format!("  spec.{m}('parser')\n"),
                format!("  spec.{m} 'rubocop', git: 'https://github.com/rubocop/rubocop'\n"),
                format!("  spec.{m}('rubocop', git: 'https://github.com/rubocop/rubocop')\n"),
            ] {
                expect(&spec(&offense), &[(2, 3)]);
            }
            for good in [
                format!("  spec.{m} 'parser', '~> 3.1', '>= 3.1.1.0'\n"),
                format!("  spec.{m}('parser', '>= 3.1.1.0')\n"),
                format!(
                    "  spec.{m} 'rubocop', git: 'https://github.com/rubocop/rubocop', ref: '54f4c8228'\n"
                ),
                format!(
                    "  spec.{m}('rubocop', git: 'https://github.com/rubocop/rubocop', ref: '54f4c8228')\n"
                ),
                format!(
                    "  spec.{m} 'rubocop', git: 'https://github.com/rubocop/rubocop', tag: 'v1.28.0'\n"
                ),
                format!(
                    "  spec.{m}('rubocop', git: 'https://github.com/rubocop/rubocop', tag: 'v1.28.0')\n"
                ),
                format!(
                    "  spec.{m} 'rubocop', git: 'https://github.com/rubocop/rubocop', branch: 'main'\n"
                ),
                format!(
                    "  spec.{m}('rubocop', git: 'https://github.com/rubocop/rubocop', branch: 'main')\n"
                ),
            ] {
                expect(&spec(&good), &[]);
            }
        }
        // A method called on the gem name argument.
        expect(&spec("  spec.add_dependency('parser'.freeze)\n"), &[(2, 3)]);
    }

    #[test]
    fn allowed_gems_examples_at_the_fixed_empty_list() {
        // The spec allows `rubocop`; at the fixed `[]` it is reported too.
        expect(
            &spec(
                "  spec.add_dependency 'rubocop'\n  spec.add_development_dependency 'rubocop-ast', '~> 0.1'\n  spec.add_dependency 'parser'\n",
            ),
            &[(2, 3), (4, 3)],
        );
    }

    #[test]
    fn version_specification_matches_any_line_of_any_str_argument() {
        for good in [
            "'a', '1'",
            "'a', '.1'",
            "'a', ' ~> 1'",
            "'a', '~>  1.0'",
            "'a', '<=1'",
            "'a', \">= 1\\n\"",
            "'a', \"x\\n1\"",
            "'a', \"\\t\\v\\f\\r\\n= 1\"",
            "'1password'",
            "'a', 'x', '2'",
            "'a', %q(1)",
            "'a', ?1",
            "'a', <<~V\n    1\n  V",
            "'a', {ref: 'x'}",
            "'a', :ref => 'x'",
            "'a', 'ref': 'x'",
            "'a', \"tag\": 'x'",
            "'a', **x, branch: 'main'",
            "'a', {}, ref: \"x\"",
        ] {
            expect(&spec(&format!("  spec.add_dependency {good}\n")), &[]);
        }
        for bad in [
            "'a', 'x1'",
            "'a', \"1\" \"x\"",
            "'a', '~> x'",
            "'a', '>'",
            "'a', \"#{v}\"",
            "'a', \"1#{v}\"",
            "'a', 1",
            "'a', :'1'",
            "'a', VERSION",
            "'a', ['1']",
            "'a', ref: x",
            "'a', ref: \"#{x}\"",
            "'a', ref: :x",
            "'a', commit: 'x'",
            "'a', \"#{'ref'}\": 'x'",
            "'a', 'ref' => 'x'",
            "'a', x: { ref: 'x' }",
            "'a', *v",
            "'a', &b",
            "&b",
            ":a",
        ] {
            expect(&spec(&format!("  spec.add_dependency {bad}\n")), &[(2, 3)]);
        }
    }

    #[test]
    fn receiver_is_the_first_specification_block_variable() {
        // No arguments: `str_content` on nil raises upstream, no finding.
        expect(
            &spec("  spec.add_dependency\n  spec.add_dependency()\n"),
            &[],
        );
        // Outside any specification block the name still matches.
        expect(
            "Gem::Specification.new do |spec|\nend\nspec = x\nspec.add_dependency 'a'\n",
            &[(4, 1)],
        );
        expect(
            "spec = x\nspec.add_dependency 'a'\nGem::Specification.new do |spec|\nend\n",
            &[(2, 1)],
        );
        // Only the first specification's name; a second one is ignored.
        expect(
            "Gem::Specification.new do |s|\n  s.add_dependency 'a'\nend\nGem::Specification.new do |t|\n  t.add_dependency 'a'\nend\n",
            &[(2, 3)],
        );
        // Parser pre-order: `unless` visits its else body first.
        expect(
            "unless x\n  Gem::Specification.new { |s| s.add_dependency 'a' }\nelse\n  Gem::Specification.new { |t| t.add_dependency 'a' }\nend\n",
            &[(4, 32)],
        );
        // No (or no matching) specification block: nothing.
        expect("spec.add_dependency 'a'\n", &[]);
        for shape in [
            "Gem::Specification.new('x') do |spec|\n",
            "Gem::Specification.new do |spec, x|\n",
            "Gem::Specification.new do |*spec|\n",
            "Gem::Specification.new do |spec; x|\n",
            "Gem::Spec.new do |spec|\n",
            "Specification.new do |spec|\n",
            "X::Gem::Specification.new do |spec|\n",
        ] {
            expect(&format!("{shape}  spec.add_dependency 'a'\nend\n"), &[]);
        }
        expect(
            "::Gem::Specification.new { |spec| spec.add_dependency 'a' }\n",
            &[(1, 35)],
        );
        // Receivers that are not `(lvar :spec)`.
        expect(
            &spec(
                "  spec&.add_dependency 'a'\n  @spec.add_dependency 'a'\n  self.add_dependency 'a'\n  add_dependency 'a'\n  s.add_dependency 'a'\n  spec.add_dependencies 'a'\n  spec.dependency 'a'\n",
            ),
            &[],
        );
        expect(
            "def spec = x\nspec.add_dependency 'a'\nGem::Specification.new { |spec| }\n",
            &[],
        );
        expect("Gem::Specification.new { it.add_dependency 'a' }\n", &[]);
        expect("Gem::Specification.new { _1.add_dependency 'a' }\n", &[]);
    }

    #[test]
    fn whole_send_location_without_its_block_and_supplied_gemspecs_only() {
        assert_eq!(
            found_at(
                "a.gemspec",
                &spec("  spec.add_dependency(\n    'a'\n  ) do\n  end\n")
            ),
            [(2, 3, 4)]
        );
        for path in ["Gemfile", "gems.rb", "x.rb", "gemspec"] {
            assert!(found_at(path, &spec("  spec.add_dependency 'a'\n")).is_empty());
        }
    }
}
