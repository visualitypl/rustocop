//! `Gemspec/RubyVersionGlobalsUsage`: every `RUBY_VERSION`, `::RUBY_VERSION`,
//! `Ruby::VERSION` or `::Ruby::VERSION` constant read in a supplied gemspec
//! whose tree contains a `Gem::Specification.new do |x| ... end` block
//! anywhere (the pinned `gem_specification(processed_source.ast)` gate is a
//! whole-root search, so the constant need not be inside the block).
//! Detection only; nothing is evaluated.
use ruby_prism::{Node, ParseResult};

use super::help::specification;
use crate::{
    cops::{
        Cop,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};

pub struct RubyVersionGlobalsUsage;

impl Cop for RubyVersionGlobalsUsage {
    fn name(&self) -> &'static str {
        "Gemspec/RubyVersionGlobalsUsage"
    }

    fn check_parse(&self, result: &ParseResult<'_>, file: &SourceFile, out: &mut Vec<Offense>) {
        // One complete walk: the constants and the whole-root gate together.
        let mut collector = Collector {
            ancestors: Vec::new(),
            constants: Vec::new(),
            specification: false,
        };
        walk(&mut collector, &result.node());
        if !collector.specification {
            return;
        }
        for (start, source) in collector.constants {
            let (line, col) = crate::cops::lint::reference_line_col(start, file);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                col,
                end_line: line,
                message: format!(
                    "Do not use `{}` in gemspec file.",
                    String::from_utf8_lossy(source)
                ),
                correction: None,
            });
        }
    }
}

struct Collector<'pr> {
    ancestors: Vec<Node<'pr>>,
    constants: Vec<(usize, &'pr [u8])>,
    specification: bool,
}

impl<'pr> NodeVisitor<'pr> for Collector<'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        if !self.specification
            && let Some(call) = node.as_call_node()
            && specification(&call).is_some()
        {
            self.specification = true;
        }
        if ruby_version(&node) && !assignment_target(&node, self.ancestors.last()) {
            let location = node.location();
            self.constants
                .push((location.start_offset(), location.as_slice()));
        }
        self.ancestors.push(node);
    }

    fn leave(&mut self) {
        self.ancestors.pop();
    }
}

/// `{(const {cbase nil?} :RUBY_VERSION) (const (const {cbase nil?} :Ruby) :VERSION)}`.
fn ruby_version(node: &Node<'_>) -> bool {
    if let Some(constant) = node.as_constant_read_node() {
        return constant.name().as_slice() == b"RUBY_VERSION";
    }
    let Some(path) = node.as_constant_path_node() else {
        return false;
    };
    let Some(name) = path.name() else {
        return false;
    };
    match (name.as_slice(), path.parent()) {
        (b"RUBY_VERSION", None) => true,
        (b"VERSION", Some(parent)) => {
            parent
                .as_constant_read_node()
                .is_some_and(|c| c.name().as_slice() == b"Ruby")
                || parent.as_constant_path_node().is_some_and(|p| {
                    p.parent().is_none() && p.name().is_some_and(|n| n.as_slice() == b"Ruby")
                })
        }
        _ => false,
    }
}

/// A constant path that is the target of a constant write: the pinned node
/// is a `casgn`, not a `const` (its scope is still a `const`, and visited).
fn assignment_target(node: &Node<'_>, parent: Option<&Node<'_>>) -> bool {
    let Some(parent) = parent else {
        return false;
    };
    let target = match parent {
        Node::ConstantPathWriteNode { .. } => {
            parent.as_constant_path_write_node().unwrap().target()
        }
        Node::ConstantPathOrWriteNode { .. } => {
            parent.as_constant_path_or_write_node().unwrap().target()
        }
        Node::ConstantPathAndWriteNode { .. } => {
            parent.as_constant_path_and_write_node().unwrap().target()
        }
        Node::ConstantPathOperatorWriteNode { .. } => parent
            .as_constant_path_operator_write_node()
            .unwrap()
            .target(),
        _ => return false,
    };
    target.location().start_offset() == node.location().start_offset()
        && target.location().end_offset() == node.location().end_offset()
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's gemspec/ruby_version_globals_usage_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov), plus lookalikes.
    use super::*;
    use crate::cops::bundler::tests::assert_offenses;

    fn expect_at(path: &str, source: &str, expected: &[(u32, u32, &str)]) {
        let expected: Vec<_> = expected
            .iter()
            .map(|&(line, col, constant)| {
                (
                    line,
                    col,
                    format!("Do not use `{constant}` in gemspec file."),
                )
            })
            .collect();
        assert_offenses(&RubyVersionGlobalsUsage, path, source, &expected);
    }

    fn expect(source: &str, expected: &[(u32, u32, &str)]) {
        expect_at("example.gemspec", source, expected);
    }

    fn in_spec(body: &str) -> String {
        format!("Gem::Specification.new do |spec|\n  {body}\nend\n")
    }

    #[test]
    fn registers_an_offense_when_using_ruby_version() {
        expect(&in_spec("RUBY_VERSION"), &[(2, 3, "RUBY_VERSION")]);
    }

    #[test]
    fn registers_an_offense_when_using_cbase_ruby_version() {
        expect(&in_spec("::RUBY_VERSION"), &[(2, 3, "::RUBY_VERSION")]);
    }

    #[test]
    fn registers_an_offense_when_using_ruby_colon_version() {
        expect(&in_spec("Ruby::VERSION"), &[(2, 3, "Ruby::VERSION")]);
    }

    #[test]
    fn registers_an_offense_when_using_cbase_ruby_colon_version() {
        expect(&in_spec("::Ruby::VERSION"), &[(2, 3, "::Ruby::VERSION")]);
    }

    #[test]
    fn does_not_register_an_offense_when_no_ruby_version() {
        expect("Gem::Specification.new do |spec|\nend\n", &[]);
    }

    #[test]
    fn whole_root_gate_not_block_scope() {
        // Anywhere in a file that has a specification block, before or after.
        expect(
            "x = RUBY_VERSION\nGem::Specification.new do |spec|\nend\nif Ruby::VERSION >= '3'\nend\n",
            &[(1, 5, "RUBY_VERSION"), (4, 4, "Ruby::VERSION")],
        );
        // Every use, in conditions, interpolation, arguments, nested blocks.
        expect(
            "Gem::Specification.new do |spec|\n  if RUBY_VERSION >= '3.0'\n    spec.add_dependency 'a'\n  end\n  [1].each { \"#{::RUBY_VERSION}\" }\n  foo(Ruby::VERSION, RUBY_VERSION::X)\nend\n",
            &[
                (2, 6, "RUBY_VERSION"),
                (5, 17, "::RUBY_VERSION"),
                (6, 7, "Ruby::VERSION"),
                (6, 22, "RUBY_VERSION"),
            ],
        );
    }

    #[test]
    fn no_specification_block_means_no_offense() {
        for source in [
            "RUBY_VERSION\n",
            "Gem::Specification.new\nRUBY_VERSION\n",
            // Not `(args (arg $_))`.
            "Gem::Specification.new do\nend\nRUBY_VERSION\n",
            "Gem::Specification.new do |a, b|\nend\nRUBY_VERSION\n",
            "Gem::Specification.new { _1 }\nRUBY_VERSION\n",
            "Gem::Specification.new { it }\nRUBY_VERSION\n",
            "Gem::Specification.new do |s; t|\nend\nRUBY_VERSION\n",
            "Gem::Specification.new(1) do |s|\nend\nRUBY_VERSION\n",
            "Foo::Gem::Specification.new do |s|\nend\nRUBY_VERSION\n",
            "Gem::Specification&.new do |s|\nend\nRUBY_VERSION\n",
        ] {
            expect(source, &[]);
        }
        expect(
            "::Gem::Specification.new do |s|\nend\nRUBY_VERSION\n",
            &[(3, 1, "RUBY_VERSION")],
        );
    }

    #[test]
    fn lookalike_constants_and_assignment_targets() {
        expect(
            &in_spec(
                "Foo::RUBY_VERSION\n  Ruby::Foo\n  Foo::Ruby::VERSION\n  RUBY_ENGINE\n  VERSION\n  ::Ruby\n  ruby_version\n  :RUBY_VERSION\n  'RUBY_VERSION'\n  RUBY_VERSION = 1\n  ::RUBY_VERSION = 1\n  Ruby::VERSION ||= 1\n  ::Ruby::VERSION += 1\n  RUBY_VERSION, ::Ruby::VERSION = 1, 2",
            ),
            &[],
        );
        // A casgn scope is a const.
        expect(&in_spec("RUBY_VERSION::X = 1"), &[(2, 3, "RUBY_VERSION")]);
        // Class names and patterns are `const` nodes.
        expect(
            &in_spec("class Ruby::VERSION; end\n  case 1\n  in RUBY_VERSION then 1\n  end"),
            &[(2, 9, "Ruby::VERSION"), (4, 6, "RUBY_VERSION")],
        );
    }

    #[test]
    fn only_supplied_gemspecs() {
        let source = in_spec("RUBY_VERSION");
        for path in ["Gemfile", "x.rb", "gemspec", "gems.rb"] {
            expect_at(path, &source, &[]);
        }
        expect_at("nested/.x.gemspec", &source, &[(2, 3, "RUBY_VERSION")]);
        expect_at(
            "x.gemspec",
            &format!("\u{feff}{}", source.replace('\n', "\r\n")),
            &[(2, 3, "RUBY_VERSION")],
        );
    }
}
