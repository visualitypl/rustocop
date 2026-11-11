//! `Gemspec/RequiredRubyVersion` at the fixed `TargetRubyVersion` 3.4:
//!
//! * a location-free finding when the supplied gemspec has no
//!   `(send _ :required_ruby_version= _)` anywhere, empty and comment-only
//!   files included (`on_new_investigation` with a nil AST);
//! * each `required_ruby_version=` send whose static value (a `str`, an
//!   array of exactly two `str`, or `Gem::Requirement.new(str+)`) does not
//!   yield `3.4` from the first two digits of its first `>`/`=` requirement.
//!
//! Values that are receiverless sends, variables, or contain a send or a
//! variable are dynamic and skipped. No `.rubocop.yml` is read; detection
//! only, nothing is evaluated.
use ruby_prism::{CallNode, Node, ParseResult};

use crate::{
    cops::{
        Cop, NodeContext,
        bundler::declarations::str_value,
        walk::{NodeVisitor, walk},
    },
    offense::{GlobalOffense, Offense},
    source::SourceFile,
};

pub struct RequiredRubyVersion;

/// The fixed `TargetRubyVersion`.
const TARGET: &str = "3.4";
const MISSING: &str = "`required_ruby_version` should be specified.";

impl Cop for RequiredRubyVersion {
    fn name(&self) -> &'static str {
        "Gemspec/RequiredRubyVersion"
    }

    fn check_global(
        &self,
        result: &ParseResult<'_>,
        _file: &SourceFile,
        out: &mut Vec<GlobalOffense>,
    ) {
        // `required_ruby_version?(processed_source.ast)`: one complete walk.
        struct Finder(bool);
        impl<'pr> NodeVisitor<'pr> for Finder {
            fn enter(&mut self, node: Node<'pr>) {
                self.0 |= node
                    .as_call_node()
                    .is_some_and(|call| setter(&call) && argument_count(&call) == 1);
            }
        }
        let mut finder = Finder(false);
        walk(&mut finder, &result.node());
        if !finder.0 {
            out.push(GlobalOffense {
                cop: self.name(),
                severity: self.severity(),
                message: MISSING.into(),
            });
        }
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // `on_send` (never `csend`). A multiple-assignment, `for` or
        // `rescue` target has no first argument and raises upstream.
        let Some(call) = node.as_call_node() else {
            return;
        };
        if !setter(&call) {
            return;
        }
        let Some(value) = first_argument(&call) else {
            return;
        };
        if dynamic(&value) {
            return;
        }
        if extract(&value, file).as_deref() == Some(TARGET) {
            return;
        }
        let location = value.location();
        let (line, col) = crate::cops::lint::reference_line_col(location.start_offset(), file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(location),
            message: format!(
                "`required_ruby_version` and `TargetRubyVersion` ({TARGET}, which may be specified in .rubocop.yml) should be equal."
            ),
            correction: None,
        });
    }
}

/// A `send` (never `csend`) of `required_ruby_version=`.
fn setter(call: &CallNode<'_>) -> bool {
    !call.is_safe_navigation() && call.name().as_slice() == b"required_ruby_version="
}

/// The pinned send's arguments: a block pass is one, a literal block is not.
fn argument_count(call: &CallNode<'_>) -> usize {
    call.arguments().map_or(0, |a| a.arguments().len())
        + usize::from(
            call.block()
                .is_some_and(|b| b.as_block_argument_node().is_some()),
        )
}

fn first_argument<'pr>(call: &CallNode<'pr>) -> Option<Node<'pr>> {
    call.arguments()
        .and_then(|a| a.arguments().iter().next())
        .or_else(|| {
            call.block()
                .filter(|b| b.as_block_argument_node().is_some())
        })
}

/// `dynamic_version?`: a receiverless `send`, a variable, or any `send` or
/// variable descendant (`ivar`, `gvar`, `cvar`, `lvar`).
fn dynamic(value: &Node<'_>) -> bool {
    if let Some(call) = value.as_call_node()
        && !call.is_safe_navigation()
        && call.receiver().is_none()
    {
        return true;
    }
    struct Finder<'pr> {
        root: (usize, usize),
        // The regexp `=~` of a named-capture match is `match_with_lvasgn`.
        excluded: Vec<(usize, usize)>,
        found: bool,
        _marker: std::marker::PhantomData<&'pr ()>,
    }
    impl<'pr> NodeVisitor<'pr> for Finder<'pr> {
        fn enter(&mut self, node: Node<'pr>) {
            let location = node.location();
            let span = (location.start_offset(), location.end_offset());
            if let Some(write) = node.as_match_write_node() {
                let call = write.call().location();
                self.excluded.push((call.start_offset(), call.end_offset()));
            }
            let variable = matches!(
                node,
                Node::LocalVariableReadNode { .. }
                    | Node::ItLocalVariableReadNode { .. }
                    | Node::InstanceVariableReadNode { .. }
                    | Node::ClassVariableReadNode { .. }
                    | Node::GlobalVariableReadNode { .. }
            );
            let send = match &node {
                Node::CallNode { .. } => {
                    let call = node.as_call_node().unwrap();
                    // A literal block wraps the send: the call is then a
                    // descendant of the pinned `block` node.
                    !call.is_safe_navigation()
                        && (span != self.root
                            || call.block().is_some_and(|b| b.as_block_node().is_some()))
                        && !self.excluded.contains(&span)
                }
                // `op_asgn`/`or_asgn`/`and_asgn` and multiple-assignment
                // targets wrap or are `send` nodes.
                Node::CallOperatorWriteNode { .. } => !node
                    .as_call_operator_write_node()
                    .unwrap()
                    .is_safe_navigation(),
                Node::CallOrWriteNode { .. } => {
                    !node.as_call_or_write_node().unwrap().is_safe_navigation()
                }
                Node::CallAndWriteNode { .. } => {
                    !node.as_call_and_write_node().unwrap().is_safe_navigation()
                }
                Node::IndexOperatorWriteNode { .. } => !node
                    .as_index_operator_write_node()
                    .unwrap()
                    .is_safe_navigation(),
                Node::IndexOrWriteNode { .. } => {
                    !node.as_index_or_write_node().unwrap().is_safe_navigation()
                }
                Node::IndexAndWriteNode { .. } => {
                    !node.as_index_and_write_node().unwrap().is_safe_navigation()
                }
                Node::CallTargetNode { .. } => {
                    !node.as_call_target_node().unwrap().is_safe_navigation()
                }
                Node::IndexTargetNode { .. } => {
                    !node.as_index_target_node().unwrap().is_safe_navigation()
                }
                _ => false,
            };
            // The root itself counts as a variable (`node.variable?`); a
            // root send only through the receiverless check above.
            self.found |= variable || send;
        }
    }
    let location = value.location();
    let mut finder = Finder {
        root: (location.start_offset(), location.end_offset()),
        excluded: Vec::new(),
        found: false,
        _marker: std::marker::PhantomData,
    };
    walk(&mut finder, value);
    finder.found
}

/// `extract_ruby_version(defined_ruby_version(node))`: the first two digits
/// of the selected requirement, joined by a dot.
fn extract(value: &Node<'_>, file: &SourceFile) -> Option<String> {
    let requirement = if let Some(value) = str_value(value, file) {
        value
    } else {
        let candidates = if let Some(array) = value.as_array_node() {
            // `(array (str _) (str _))`
            let elements: Vec<_> = array.elements().iter().collect();
            if elements.len() != 2 {
                return None;
            }
            elements
                .iter()
                .map(|e| str_value(e, file))
                .collect::<Option<Vec<_>>>()?
        } else {
            requirement_arguments(value, file)?
        };
        candidates
            .into_iter()
            .find(|v| v.iter().any(|b| matches!(b, b'>' | b'=')))?
    };
    let digits: Vec<String> = requirement
        .iter()
        .filter(|b| b.is_ascii_digit())
        .take(2)
        .map(|b| char::from(*b).to_string())
        .collect();
    Some(digits.join("."))
}

/// `(send (const (const nil? :Gem) :Requirement) :new $str+)`.
fn requirement_arguments(value: &Node<'_>, file: &SourceFile) -> Option<Vec<Vec<u8>>> {
    let call = value.as_call_node()?;
    if call.is_safe_navigation() || call.name().as_slice() != b"new" || call.block().is_some() {
        return None;
    }
    let path = call.receiver()?.as_constant_path_node()?;
    if path.name()?.as_slice() != b"Requirement"
        || path
            .parent()?
            .as_constant_read_node()
            .is_none_or(|gem| gem.name().as_slice() != b"Gem")
    {
        return None;
    }
    let values = call
        .arguments()?
        .arguments()
        .iter()
        .map(|argument| str_value(&argument, file))
        .collect::<Option<Vec<_>>>()?;
    (!values.is_empty()).then_some(values)
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's gemspec/required_ruby_version_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov), at the fixed
    //! TargetRubyVersion 3.4, plus lookalikes.
    use super::*;
    use crate::{config::ResolvedConfig, pipeline::lint_one};
    use std::path::Path;

    const NOT_EQUAL: &str = "`required_ruby_version` and `TargetRubyVersion` (3.4, which may be specified in .rubocop.yml) should be equal.";

    /// (line, col) of each located finding, and the global messages.
    fn lint(path: &str, source: &str) -> (Vec<(u32, u32)>, Vec<String>) {
        let report = lint_one(
            Path::new(path),
            Path::new(path),
            Some(source.as_bytes()),
            &[&RequiredRubyVersion],
            &ResolvedConfig::default(),
        );
        assert!(report.errors.is_empty(), "{report:?}");
        assert!(report.internal_errors.is_empty(), "{report:?}");
        let mut located: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| {
                assert_eq!(o.message, NOT_EQUAL);
                assert!(o.correction.is_none());
                (o.line, o.col)
            })
            .collect();
        located.sort();
        let globals = report
            .result
            .global_offenses
            .iter()
            .map(|o| o.message.clone())
            .collect();
        (located, globals)
    }

    /// A specification block assigning `value`: (line, col) findings, the
    /// missing finding never applies.
    fn value(value: &str) -> Vec<(u32, u32)> {
        let source = format!(
            "Gem::Specification.new do |spec|\n  spec.required_ruby_version = {value}\nend\n"
        );
        let (located, globals) = lint("/path/to/foo.gemspec", &source);
        assert!(globals.is_empty(), "{source}");
        located
    }

    #[test]
    fn lower_than_target_with_ge_tilde_array_and_requirement() {
        for v in [
            "'>= 3.3.0'",
            "'~> 3.3.0'",
            "['>= 3.3.0', '< 4.0.0']",
            "Gem::Requirement.new(\">= 3.3.0\")",
            "Gem::Requirement.new('< 3.4')",
        ] {
            assert_eq!(value(v), [(2, 32)], "{v}");
        }
    }

    #[test]
    fn requirement_with_multiple_requirements_is_accepted() {
        assert_eq!(value("Gem::Requirement.new(\">= 3.4.0\", \"<= 3.8\")"), []);
    }

    #[test]
    fn false_negatives_for_variables() {
        let source = "Gem::Specification.new do |spec|\n  version = '>= 3.3.0'\n  spec.required_ruby_version = version\nend\n";
        assert_eq!(lint("a.gemspec", source), (vec![], vec![]));
        let source = "Gem::Specification.new do |spec|\n  lowest_version = '>= 3.3.0'\n  highest_version = '< 3.8.0'\n  spec.required_ruby_version = [lowest_version, highest_version]\nend\n";
        assert_eq!(lint("a.gemspec", source), (vec![], vec![]));
    }

    #[test]
    fn ruby33_examples_replayed_at_the_fixed_target() {
        // The `:ruby33` examples expect a 3.3 message; at 3.4 they are equal.
        assert_eq!(value("'>= 3.4.0'"), []);
        assert_eq!(value("'~> 3.4.0'"), []);
    }

    #[test]
    fn equal_to_target() {
        for v in [
            "'>= 3.4.0'",
            "'~> 3.4.0'",
            "'>= 3.4'",
            "'~> 3.4'",
            "['>= 3.4.0', '< 4.1.0']",
        ] {
            assert_eq!(value(v), [], "{v}");
        }
    }

    #[test]
    fn without_a_minor_version_blank_and_empty_array() {
        for v in ["'>= 3'", "'~> 3'", "''", "[]"] {
            assert_eq!(value(v), [(2, 32)], "{v}");
        }
    }

    #[test]
    fn missing_is_a_global_finding_including_empty_files() {
        let missing = vec![MISSING.to_owned()];
        for source in [
            "Gem::Specification.new do |spec|\nend\n",
            "",
            "# comment only\n",
            "\u{feff}# c\r\n",
            "x = 1\n__END__\nspec.required_ruby_version = '>= 3.4'\n",
            // Not `(send _ :required_ruby_version= _)`.
            "spec&.required_ruby_version = '>= 3.4'\n",
            "spec.required_ruby_version\n",
            "spec.required_ruby_version ||= '>= 3.4'\n",
            "spec.required_ruby_version, x = '>= 3.4', 1\n",
            "spec.send(:required_ruby_version=, '>= 3.4')\n",
            "spec.required_rubygems_version = '>= 3.4'\n",
            "required_ruby_version = '>= 3.4'\n",
        ] {
            assert_eq!(
                lint("bar.gemspec", source),
                (vec![], missing.clone()),
                "{source:?}"
            );
        }
        // Any receiver, anywhere in the tree, even if never reached.
        for source in [
            "spec.required_ruby_version = '>= 3.4'\n",
            "if false\n  x.y.required_ruby_version = '>= 3.4'\nend\n",
            "def f\n  self.required_ruby_version = '>= 3.4'\nend\n",
        ] {
            assert_eq!(lint("bar.gemspec", source), (vec![], vec![]), "{source:?}");
        }
    }

    #[test]
    fn digit_extraction_and_requirement_selection_follow_the_pinned_code() {
        // The first two digits, not version segments.
        assert_eq!(value("'>= 34'"), []);
        assert_eq!(value("'>= 3.10'"), [(2, 32)]);
        // The first `>`/`=` requirement of an array or a requirement list.
        assert_eq!(value("['< 4', '>= 3.4']"), []);
        assert_eq!(value("['>= 3.4', '>= 2.7']"), []);
        assert_eq!(value("['<= 3.4', '>= 2.7']"), []);
        assert_eq!(value("['< 3.4', '< 2.7']"), [(2, 32)]);
        assert_eq!(value("Gem::Requirement.new('< 4', '>= 3.4')"), []);
        // Implicit and %w arrays are arrays; three elements are not matched.
        assert_eq!(value("'>= 3.4', '< 4'"), []);
        assert_eq!(value("%w[>=3.4 <4]"), []);
        assert_eq!(value("['>= 3.4', '< 4', '!= 3.5']"), [(2, 32)]);
        // Not `(const (const nil? :Gem) :Requirement)` with str+.
        assert_eq!(value("::Gem::Requirement.new('>= 3.4')"), [(2, 32)]);
        assert_eq!(value("Gem::Requirement.new"), [(2, 32)]);
        assert_eq!(value("Gem::Requirement.create('>= 3.4')"), [(2, 32)]);
        assert_eq!(value("Gem::Requirement.new(:'>= 3.4')"), [(2, 32)]);
        // Static non-matching values are findings.
        for v in ["Foo::VERSION", "'>= ' '3.4'", "nil", "3.4", ":'>= 3.4'"] {
            assert_eq!(value(v), [(2, 32)], "{v}");
        }
        // A multi-line value is reported at its start.
        let source = "Gem::Specification.new do |spec|\n  spec.required_ruby_version = [\n    '>= 3.3', '< 4'\n  ]\nend\n";
        assert_eq!(lint("a.gemspec", source), (vec![(2, 32)], vec![]));
    }

    #[test]
    fn dynamic_values_are_skipped() {
        for v in [
            "version",
            "version(1)",
            "@version",
            "$version",
            "\"#{x}\"",
            "\"#@x\"",
            "Gem::Requirement.new(x)",
            "[x, '< 4']",
            "Foo.bar { }",
            "Foo.bar(&b)",
            "!x",
            "[1].map { it }",
            "->(a) { a }",
            "Gem::Requirement.new('>= 3.3').tap { }",
        ] {
            assert_eq!(value(v), [], "{v}");
        }
        // csend, nth_ref and back_ref are neither a send nor a variable.
        // A const receiver is not a send: this is static, and a finding.
        for v in [
            "Foo&.bar",
            "$1",
            "$&",
            "Foo::X&.y",
            "-> { }",
            "File.read('.ruby-version')",
            "('>= 3.4')",
        ] {
            assert_eq!(value(v), [(2, 32)], "{v}");
        }
    }

    #[test]
    fn every_assignment_anywhere_is_checked() {
        let source = "x.required_ruby_version = '>= 2.7'\nGem::Specification.new do |s|\n  s.required_ruby_version = '>= 3.4'\n  s.required_ruby_version=('>= 3.0')\nend\n";
        assert_eq!(lint("a.gemspec", source), (vec![(1, 27), (4, 27)], vec![]));
        // A target without a value raises upstream: no located finding, and
        // it is not `(send _ :required_ruby_version= _)` either.
        let source = "s.required_ruby_version, x = 1, 2\nfor s.required_ruby_version in [1]; end\nbegin; rescue => s.required_ruby_version; end\n";
        assert_eq!(
            lint("a.gemspec", source),
            (vec![], vec![MISSING.to_owned()])
        );
    }

    #[test]
    fn only_supplied_gemspecs() {
        for path in ["Gemfile", "x.rb", "gemspec", "gems.rb"] {
            assert_eq!(lint(path, ""), (vec![], vec![]), "{path}");
            assert_eq!(
                lint(path, "s.required_ruby_version = '>= 2'\n"),
                (vec![], vec![]),
                "{path}"
            );
        }
        assert_eq!(
            lint("nested/.x.gemspec", "s.required_ruby_version = '>= 2'\n"),
            (vec![(1, 27)], vec![])
        );
        assert_eq!(
            lint("x.gemspec", "\u{feff}s.required_ruby_version = '>= 2'\r\n"),
            (vec![(1, 28)], vec![])
        );
    }
}
