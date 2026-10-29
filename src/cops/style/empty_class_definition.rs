//! `Style/EmptyClassDefinition` — fixed `class_keyword`, no allowed parents.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
use std::collections::HashSet;
pub struct EmptyClassDefinition;
impl Cop for EmptyClassDefinition {
    fn name(&self) -> &'static str {
        "Style/EmptyClassDefinition"
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
        let Some(_) = candidate(node) else { return };
        let loc = node.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use the `class` keyword instead of `Class.new` to define an empty class."
                .into(),
            line,
            col,
            end_line: file.last_line_of(loc),
            correction: cx
                .corrections_requested()
                .then(|| correction(node, cx, file))
                .flatten(),
        });
    }
}
fn candidate<'pr>(node: &Node<'pr>) -> Option<(String, CallNode<'pr>, Node<'pr>)> {
    let (name, value) = if let Some(w) = node.as_constant_write_node() {
        (
            String::from_utf8_lossy(w.name().as_slice()).into_owned(),
            w.value(),
        )
    } else {
        let w = node.as_constant_path_write_node()?;
        (
            String::from_utf8_lossy(w.target().location().as_slice()).into_owned(),
            w.value(),
        )
    };
    let call = value.as_call_node()?;
    let args = ast::call_arguments(&call);
    if call.is_safe_navigation()
        || call.block().is_some()
        || call.name().as_slice() != b"new"
        || ast::short_const_name(&call.receiver()?).as_deref() != Some("Class")
        || !matches!(args.as_slice(), [parent] if ast::is_const(parent))
    {
        return None;
    }
    Some((name, call, args.into_iter().next()?))
}
/// A deliberately closed declaration grammar: fresh modules and fresh named
/// subclasses of the untouched core StandardError. No reopening, inherited /
/// const hooks, class-factory dispatch, deferred bodies, value consumers or
/// unknown parent lookup is assumed safe. Built once, correction-only.
#[derive(Default)]
pub(crate) struct Facts {
    eligible: HashSet<usize>,
    names: HashSet<String>,
    modules: HashSet<String>,
    safe: bool,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut facts = Self {
            safe: true,
            ..Self::default()
        };
        facts.declarations(root, "");
        facts
    }
    fn declarations(&mut self, node: &Node<'_>, namespace: &str) {
        #[cfg(test)]
        {
            self.visits += 1;
        }
        if let Some(p) = node.as_program_node() {
            self.declarations(&p.statements().as_node(), namespace);
        } else if let Some(s) = node.as_statements_node() {
            for child in s.body().iter() {
                self.declarations(&child, namespace);
            }
        } else if let Some(m) = node.as_module_node() {
            let path = m.constant_path();
            let Some(name) = path
                .as_constant_read_node()
                .and_then(|n| std::str::from_utf8(n.name().as_slice()).ok())
            else {
                self.safe = false;
                return;
            };
            let full = qualify(namespace, name);
            self.safe &= fresh_name(name) && self.names.insert(full.clone());
            self.modules.insert(full.clone());
            if let Some(body) = m.body() {
                self.declarations(&body, &full);
            }
        } else if let Some((name, call, parent)) = candidate(node) {
            let short = name.rsplit("::").next().unwrap_or("");
            let full = if name.starts_with("::") {
                name.trim_start_matches("::").to_owned()
            } else {
                qualify(namespace, &name)
            };
            let known_namespace = full
                .rsplit_once("::")
                .is_none_or(|(prefix, _)| self.modules.contains(prefix));
            self.safe &= fresh_name(short)
                && known_namespace
                && self.names.insert(full)
                && ast::is_global_const(&call.receiver().unwrap(), &["Class"])
                && ast::is_global_const(&parent, &["StandardError"]);
            self.eligible.insert(node.location().start_offset());
        } else {
            self.safe = false;
        }
    }
}
fn qualify(namespace: &str, name: &str) -> String {
    if namespace.is_empty() {
        name.into()
    } else {
        format!("{namespace}::{name}")
    }
}
fn fresh_name(name: &str) -> bool {
    !name.is_empty() && !super::mixin_grouping::core_namespace(name.as_bytes())
        // Ruby's preexisting metadata constants are not fresh class names.
        && name.bytes().any(|b| b.is_ascii_lowercase())
}
fn correction(node: &Node<'_>, cx: &NodeContext<'_, '_>, file: &SourceFile) -> Option<Correction> {
    let root = cx.ancestors().next_back()?;
    let facts = file.empty_class_facts(root);
    let loc = node.location();
    if !facts.safe
        || !facts.eligible.contains(&loc.start_offset())
        || !loc.as_slice().is_ascii()
        || loc.as_slice().contains(&b'\n')
        || super::correction::contains_comment(file, loc.start_offset()..loc.end_offset())
        || file.heredoc_pending_at(root, loc.start_offset())
    {
        return None;
    }
    let (name, _, parent) = candidate(node)?;
    let line = file.line_of(loc.start_offset());
    let prefix = &file.bytes[file.line_start(line)..loc.start_offset()];
    let prefix = if line == 1 {
        prefix.strip_prefix(b"\xef\xbb\xbf").unwrap_or(prefix)
    } else {
        prefix
    };
    let tail = &file.bytes[loc.end_offset()..file.line_start(line + 1)];
    if !prefix.iter().all(|b| matches!(b, b' ' | b'\t'))
        || !tail.iter().all(u8::is_ascii_whitespace)
    {
        // Moving an attached comment/directive to `end`, or splitting inline
        // siblings, changes source ownership even in an otherwise closed body.
        return None;
    }
    let indent = std::str::from_utf8(prefix).ok()?;
    let parent = std::str::from_utf8(parent.location().as_slice()).ok()?;
    let newline = if file.bytes[file.line_start(line)..file.line_start(line + 1)].ends_with(b"\r\n")
    {
        "\r\n"
    } else {
        "\n"
    };
    super::literal_spelling::safe_edit(
        file,
        cx,
        Correction {
            range: loc.start_offset()..loc.end_offset(),
            replacement: format!("class {name} < {parent}{newline}{indent}end"),
        },
    )
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop empty_class_definition_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Fixed class_keyword.
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/EmptyClassDefinition")
            .collect()
    }
    #[test]
    fn pinned_positions_and_canonical_fresh_error_definitions() {
        for (s, line, col, after) in [
            (
                "FooError = Class.new(StandardError)\n",
                1,
                1,
                "class FooError < StandardError\nend\n",
            ),
            (
                "module Foo\n  BarError = Class.new(StandardError)\nend\n",
                2,
                3,
                "module Foo\n  class BarError < StandardError\n  end\nend\n",
            ),
            (
                "module Foo; end\nFoo::BarError = Class.new(::StandardError)\n",
                2,
                1,
                "module Foo; end\nclass Foo::BarError < ::StandardError\nend\n",
            ),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (line, col, line));
            assert_eq!(
                o[0].message,
                "Use the `class` keyword instead of `Class.new` to define an empty class."
            );
            assert_correction(&EmptyClassDefinition, s, after);
        }
    }
    #[test]
    fn pinned_calls_and_all_six_corrections_remain_accounted_for() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/empty_class_definition_pinned.json"
        ))
        .unwrap();
        let mut fixes = 0;
        let mut holds = 0;
        assert_eq!(cases.as_array().unwrap().len(), 33);
        for e in cases.as_array().unwrap() {
            let s = e["source"].as_str().unwrap();
            let found = offenses(s);
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            let expected: Vec<Vec<u32>> = serde_json::from_value(e["positions"].clone()).unwrap();
            assert_eq!(positions, expected, "spec {}: {s}", e["spec_line"]);
            for o in found {
                assert_eq!(o.message, e["messages"][0].as_str().unwrap());
            }
            if let Some(after) = e["correction"].as_str() {
                match e["spec_line"].as_u64().unwrap() {
                    8 | 38 | 493 => {
                        assert_correction(&EmptyClassDefinition, s, after);
                        fixes += 1;
                    }
                    // Unknown namespace lookup / unknown inherited callbacks.
                    20 | 54 | 66 => {
                        assert_correction(&EmptyClassDefinition, s, s);
                        holds += 1;
                    }
                    other => panic!("unclassified pinned correction {other}"),
                }
            }
        }
        assert_eq!((fixes, holds), (3, 3));
    }
    #[test]
    fn source_envelopes_preserve_bom_crlf_encoding_and_data_payload() {
        for (s, after) in [
            (
                "\u{feff}FooError = Class.new(StandardError)\r\n__END__\r\npayload\0\r\n",
                "\u{feff}class FooError < StandardError\r\nend\r\n__END__\r\npayload\0\r\n",
            ),
            (
                "# encoding: windows-31j\r\nmodule Foo\r\n\tBarError = Class.new(StandardError)\r\nend\r\n",
                "# encoding: windows-31j\r\nmodule Foo\r\n\tclass BarError < StandardError\r\n\tend\r\nend\r\n",
            ),
        ] {
            assert_correction(&EmptyClassDefinition, s, after);
        }
    }
    #[test]
    fn exact_assignment_receiver_parent_and_block_shapes() {
        for s in [
            "ÉError = Class.new(StandardError)",
            "Foo::BarError = Class.new(StandardError)",
            "MyClass = Class.new(Alchemy::Admin::PreviewUrl)",
            "MyClass = Class.new(::Safemode::Jail)",
            "Foo = ::Class.new(StandardError)",
            "Foo = Other::Class.new(StandardError)",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
        for s in [
            "MyClass = Class.new",
            "MyClass = Class.new()",
            "class FooError < StandardError; end",
            "class MyClass; end",
            "class FooError < StandardError; def initialize; end; end",
            "MyClass = Class.new(StandardError) { }",
            "MyClass = Class.new(StandardError) do end",
            "MyClass = Class.new(StandardError) { _1 }",
            "MyClass = Class.new(StandardError) { it }",
            "local_var = Class.new(Base)",
            "@instance_var = Class.new(Base)",
            "@@class_var = Class.new(Base)",
            "$global_var = Class.new(Base)",
            "local_var ||= Class.new(Base)",
            "CONST ||= Class.new(Base)",
            "raise Class.new(StandardError)",
            "foo(Class.new(Base))",
            "return Class.new(Error)",
            "MyClass = Class.new(self)",
            "MyClass = Class.new(nil)",
            "MyClass = Class.new(false)",
            "MyClass = Class.new(local_var)",
            "MyClass = Class.new(StandardError).any_method",
            "CONST = condition",
            "BOOLEAN = ActiveRecord::Type::Boolean.new",
            "Foo = Class&.new(StandardError)",
            "Foo = Class.new(StandardError, Object)",
            "Foo = Class.new(*parents)",
            "Foo = Class.new(StandardError, &block)",
            "let(:foo) { Class.new(StandardError) }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn uncertain_class_bindings_hooks_reopening_and_value_contexts_keep_offenses() {
        for s in [
            "Foo::BarError = Class.new(StandardError)",
            "MyClass = Class.new(Alchemy::Admin::PreviewUrl)",
            "MyClass = Class.new(::Safemode::Jail)",
            "Foo = Other::Class.new(StandardError)",
            "Class = Factory; Foo = Class.new(StandardError)",
            "StandardError = Other; Foo = Class.new(StandardError)",
            "class Base; def self.inherited(child); p child.name; end; end; Foo = Class.new(Base)",
            "SyntaxSuggest = Class.new(StandardError)",
            "Monitor = Class.new(StandardError)",
            "module SyntaxSuggest; Foo = Class.new(StandardError); end",
            "class Foo; end; Foo = Class.new(StandardError)",
            "Foo = Class.new(StandardError); Foo = Class.new(StandardError)",
            "Foo = Class.new(StandardError); class Foo; end",
            "module M; Class = Factory; Foo = Class.new(StandardError); end",
            "module M; StandardError = Other; Foo = Class.new(StandardError); end",
            "class << obj; Foo = Class.new(StandardError); end",
            "require 'other'; Foo = Class.new(StandardError)",
            "Foo = Class.new(StandardError); unknown",
            "consume(Foo = Class.new(StandardError))",
            "Foo = (Bar = Class.new(StandardError))",
            "if true; Foo = Class.new(StandardError); end",
            "loop { Foo = Class.new(StandardError) }",
            "Foo = Class.new( # keep\n StandardError\n)",
            "Foo = Class.new(StandardError) # keep attached\n",
            "Foo = Class.new(StandardError) # rubocop:disable Style/Documentation\n",
            "Foo = Class.new(StandardError); OtherError = Class.new(StandardError)\n",
            "Foo = Class.new(StandardError)\np __LINE__\n",
            "Foo = Class.new(StandardError)\np DATA.pos\n__END__\npayload\n",
        ] {
            let o = correction_offenses(&EmptyClassDefinition, s);
            assert!(!o.is_empty(), "{s}");
            assert!(o.iter().all(|o| o.correction.is_none()), "{s}");
        }
    }
}
