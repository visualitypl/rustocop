//! `Naming/MethodName` — flags method names not written in snake_case.
//!
//! Matches RuboCop's semantics at its defaults (`EnforcedStyle:
//! snake_case`, no allowed or forbidden patterns, `ForbiddenIdentifiers`
//! `__id__` and `__send__`). Checked are `def` and `def obj.` names
//! (operators aside), the new name of `alias`, the first argument of
//! `define_method`, `define_singleton_method` and two-argument
//! `alias_method`, the members of `Struct.new` (after a leading class name
//! string) and `Data.define`, and the names given to receiverless `attr`,
//! `attr_reader`, `attr_writer` and `attr_accessor`; names are plain symbols
//! or strings, never interpolated ones. Calls with `&.` are not checked.
//!
//! A singleton method named like a class among its siblings is a class
//! emitter (`def self.Model` beside `class Model`) and is accepted. As in
//! RuboCop, the siblings are those of the definition, or of the outermost
//! singleton method it is the only statement of.
//!
//! Locations follow RuboCop: the name of a `def`, the argument naming the
//! method, but for a non-forbidden name in `define_method` and `attr*`, one
//! column past the method name (the argument, after a single space or
//! parenthesis). RuboCop keeps one offense per location, so an `attr` call
//! reports at most one non-snake_case name. A forbidden `attr*` name is
//! reported at that name; RuboCop reports it at the call's last argument.

use memchr::memmem;
use ruby_prism::{CallNode, DefNode, Node};

use crate::{
    cops::{
        Cop, NodeContext,
        ast::{call_arguments, is_global_const, is_operator_method},
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};

use super::format::is_snake_case;

/// The `Naming/MethodName` cop.
pub struct MethodName;

/// RuboCop's default `ForbiddenIdentifiers`.
const FORBIDDEN: [&[u8]; 2] = [b"__id__", b"__send__"];

/// The `attr*` methods whose arguments name methods.
const ATTR_METHODS: [&[u8]; 4] = [b"attr", b"attr_reader", b"attr_writer", b"attr_accessor"];

impl Cop for MethodName {
    fn name(&self) -> &'static str {
        "Naming/MethodName"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let mut report = |offset: usize, end: usize, message: String| {
            let (line, col) = file.line_col(offset);
            out.push(Offense {
                correction: None,
                cop: self.name(),
                severity: self.severity(),
                message,
                line,
                end_line: file.line_of(end.saturating_sub(1).max(offset)),
                col,
            });
        };
        match node {
            Node::DefNode { .. } => {
                let def = node.as_def_node().unwrap();
                let name = def.name().as_slice();
                let offset = def.name_loc().start_offset();
                if is_operator_method(name) {
                    return;
                }
                if FORBIDDEN.contains(&name) {
                    report(offset, def.name_loc().end_offset(), forbidden_message(name));
                } else if !is_snake_case(&String::from_utf8_lossy(name))
                    && !may_be_class_emitter(&def, file)
                {
                    report(
                        offset,
                        def.name_loc().end_offset(),
                        SNAKE_CASE_MESSAGE.to_string(),
                    );
                }
            }
            Node::ProgramNode { .. } if mentions_class(file) => {
                // The singleton methods that may be class emitters are judged
                // here, in one walk with ancestors (cops get no parents).
                let mut checker = EmitterChecker {
                    ancestors: Vec::new(),
                    offenders: Vec::new(),
                };
                walk(&mut checker, node);
                for (offset, end) in checker.offenders {
                    report(offset, end, SNAKE_CASE_MESSAGE.to_string());
                }
            }
            Node::AliasMethodNode { .. } => {
                let new_name = node.as_alias_method_node().unwrap().new_name();
                if let Some(name) = literal_name(&new_name) {
                    let offset = new_name.location().start_offset();
                    if let Some(message) = method_name_problem(&name) {
                        report(offset, new_name.location().end_offset(), message);
                    }
                }
            }
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                if !call.is_safe_navigation() {
                    check_call(&call, &mut report);
                }
            }
            _ => {}
        }
    }
}

const SNAKE_CASE_MESSAGE: &str = "Use snake_case for method names.";

fn forbidden_message(name: &[u8]) -> String {
    format!(
        "`{}` is forbidden, use another method name instead.",
        String::from_utf8_lossy(name)
    )
}

/// RuboCop's `handle_method_name` judgement of a name given as a symbol or
/// string: forbidden, or (unless an operator) not snake_case.
fn method_name_problem(name: &[u8]) -> Option<String> {
    if FORBIDDEN.contains(&name) {
        Some(forbidden_message(name))
    } else if !is_operator_method(name) && !is_snake_case(&String::from_utf8_lossy(name)) {
        Some(SNAKE_CASE_MESSAGE.to_string())
    } else {
        None
    }
}

/// Checks the method-defining calls (RuboCop's `on_send`).
fn check_call(call: &CallNode<'_>, report: &mut impl FnMut(usize, usize, String)) {
    let method = call.name().as_slice();
    if !matches!(
        method,
        b"define_method" | b"define_singleton_method" | b"new" | b"define" | b"alias_method"
    ) && !ATTR_METHODS.contains(&method)
    {
        return;
    }
    let arguments = call_arguments(call);
    // RuboCop's range for a non-forbidden name passed to `define_method` or
    // `attr*`: from one past the end of the method name.
    let past_selector = || {
        call.message_loc()
            .map_or(call.location().start_offset(), |selector| {
                selector.end_offset() + 1
            })
    };
    match method {
        b"define_method" | b"define_singleton_method" => {
            let Some(first) = arguments.first() else {
                return;
            };
            let Some(name) = literal_name(first) else {
                return;
            };
            if FORBIDDEN.contains(&name.as_slice()) {
                report(
                    first.location().start_offset(),
                    first.location().end_offset(),
                    forbidden_message(&name),
                );
            } else if let Some(message) = method_name_problem(&name) {
                report(
                    past_selector(),
                    crate::cops::ast::source_end(&call.as_node()),
                    message,
                );
            }
        }
        b"new" | b"define" => {
            let constant = if method == b"new" { "Struct" } else { "Data" };
            if !call
                .receiver()
                .is_some_and(|receiver| is_global_const(&receiver, &[constant]))
            {
                return;
            }
            // A leading string names the struct class.
            let skip = usize::from(
                method == b"new"
                    && arguments
                        .first()
                        .is_some_and(|first| first.as_string_node().is_some()),
            );
            for argument in &arguments[skip..] {
                if let Some(message) =
                    literal_name(argument).and_then(|name| method_name_problem(&name))
                {
                    report(
                        argument.location().start_offset(),
                        argument.location().end_offset(),
                        message,
                    );
                }
            }
        }
        b"alias_method" => {
            let [first, _] = arguments.as_slice() else {
                return;
            };
            if let Some(message) = literal_name(first).and_then(|name| method_name_problem(&name)) {
                report(
                    first.location().start_offset(),
                    first.location().end_offset(),
                    message,
                );
            }
        }
        _ if ATTR_METHODS.contains(&method) && call.receiver().is_none() => {
            let mut reported_case = false;
            for argument in &arguments {
                let Some(name) = literal_name(argument) else {
                    continue;
                };
                if FORBIDDEN.contains(&name.as_slice()) {
                    report(
                        argument.location().start_offset(),
                        argument.location().end_offset(),
                        forbidden_message(&name),
                    );
                } else if !reported_case && !is_snake_case(&String::from_utf8_lossy(&name)) {
                    // Every name shares this range; RuboCop keeps the first.
                    reported_case = true;
                    report(
                        past_selector(),
                        crate::cops::ast::source_end(&call.as_node()),
                        SNAKE_CASE_MESSAGE.to_string(),
                    );
                }
            }
        }
        _ => {}
    }
}

/// The name a plain symbol or string literal holds (RuboCop's `sym` and
/// `str`; interpolated ones are `dsym` and `dstr`).
fn literal_name(node: &Node<'_>) -> Option<Vec<u8>> {
    match node {
        Node::SymbolNode { .. } => Some(node.as_symbol_node().unwrap().unescaped().to_vec()),
        Node::StringNode { .. } => Some(node.as_string_node().unwrap().unescaped().to_vec()),
        _ => None,
    }
}

/// Whether a file may hold a class emitter: one that mentions `class`, as
/// the emitted class's definition would.
fn mentions_class(file: &SourceFile) -> bool {
    memmem::find(&file.bytes, b"class").is_some()
}

/// Whether `def` (a non-snake_case, non-forbidden, non-operator method) may
/// be a class emitter (RuboCop's `class_emitter_method?`): a singleton
/// method in a file that mentions `class`. Those are left to the
/// [`EmitterChecker`] walk from the program node, which needs ancestors.
fn may_be_class_emitter(def: &DefNode<'_>, file: &SourceFile) -> bool {
    def.receiver().is_some() && mentions_class(file)
}

/// Reports the singleton methods [`may_be_class_emitter`] defers that are
/// not class emitters: a class emitter is named after a class defined among
/// its siblings (in the parser gem's tree), after climbing out of enclosing
/// singleton methods it is the only statement of.
struct EmitterChecker<'pr> {
    ancestors: Vec<Node<'pr>>,
    /// Name spans of the non-snake_case singleton methods to report.
    offenders: Vec<(usize, usize)>,
}

impl<'pr> NodeVisitor<'pr> for EmitterChecker<'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        if let Some(def) = node.as_def_node() {
            let name = def.name().as_slice();
            if def.receiver().is_some()
                && !is_operator_method(name)
                && !FORBIDDEN.contains(&name)
                && !is_snake_case(&String::from_utf8_lossy(name))
                && !self.has_sibling_class(name)
            {
                self.offenders
                    .push((def.name_loc().start_offset(), def.name_loc().end_offset()));
            }
        }
        self.ancestors.push(node);
    }

    fn leave(&mut self) {
        self.ancestors.pop();
    }
}

impl EmitterChecker<'_> {
    /// Whether the definition being entered (just past the ancestors) has a
    /// sibling class named `name`, as RuboCop sees siblings.
    fn has_sibling_class(&self, name: &[u8]) -> bool {
        // The ancestors of the node in question are `ancestors[..depth]`.
        let mut depth = self.ancestors.len();
        loop {
            let Some(statements) = depth
                .checked_sub(1)
                .and_then(|parent| self.ancestors[parent].as_statements_node())
            else {
                return false;
            };
            let body = statements.body();
            if body.iter().nth(1).is_some() {
                // Several statements: the parser gem's `begin`, whose children
                // are the definition's siblings.
                return body.iter().any(|statement| {
                    statement
                        .as_class_node()
                        .is_some_and(|class| class.constant_path().location().as_slice() == name)
                });
            }
            // The only statement: its parser-gem parent is the construct
            // around it. RuboCop climbs out of a singleton method there.
            let encloses = depth.checked_sub(2).and_then(|grandparent| {
                self.ancestors[grandparent]
                    .as_def_node()
                    .filter(|outer| outer.receiver().is_some())
            });
            if encloses.is_none() {
                return false;
            }
            depth -= 2;
        }
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/naming/method_name_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for the default
    //! configuration: `snake_case`, no `AllowedPatterns` or
    //! `ForbiddenPatterns`, and `ForbiddenIdentifiers` `__id__` and
    //! `__send__` (the spec's forbidden-identifier examples use `super`).

    use crate::pipeline::lint_str;

    const SNAKE: &str = "Use snake_case for method names.";

    fn forbidden(name: &str) -> String {
        format!("`{name}` is forbidden, use another method name instead.")
    }

    fn found(source: &str) -> Vec<(u32, u32, String)> {
        let parsed = ruby_prism::parse(source.as_bytes());
        assert!(
            parsed.errors().next().is_none(),
            "fixture must parse: {source:?}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Naming/MethodName")
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    fn snake(line: u32, col: u32) -> (u32, u32, String) {
        (line, col, SNAKE.to_string())
    }

    /// The 1-based column of `needle` in `line`.
    fn col(line: &str, needle: &str) -> u32 {
        line[..line.find(needle).unwrap()].chars().count() as u32 + 1
    }

    fn assert_accepts(sources: &[&str]) {
        for source in sources {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn flags_mixed_case_in_attr_methods() {
        let name = "visit_Arel_Nodes_SelectStatement";
        assert_eq!(
            found(&format!(
                "attr :{name}\n\nattr_reader :{name}\n\nattr_accessor :{name}\n\n\
                 attr_writer :{name}\n\nattr '{name}'\n"
            )),
            [
                snake(1, 6),
                snake(3, 13),
                snake(5, 15),
                snake(7, 13),
                snake(9, 6)
            ]
        );
        assert_eq!(
            found(&format!("attr_reader :{name}\n\nattr_reader '{name}'\n")),
            [snake(1, 13), snake(3, 13)]
        );
        assert_eq!(
            found(
                "attr :MyMethod\n\nattr_reader :MyMethod\n\nattr_accessor :MyMethod\n\nattr_writer :MyMethod\n"
            ),
            [snake(1, 6), snake(3, 13), snake(5, 15), snake(7, 13)]
        );
    }

    #[test]
    fn flags_mixed_and_capitalized_case_in_definitions() {
        assert_eq!(
            found("def visit_Arel_Nodes_SelectStatement\nend\n"),
            [snake(1, 5)]
        );
        assert_eq!(
            found("class MyClass\n  def MyMethod\n  end\nend\n"),
            [snake(2, 7)]
        );
        assert_eq!(
            found("module Sequel\n  def self.Model(source)\n  end\nend\n"),
            [snake(2, 12)]
        );
    }

    #[test]
    fn flags_camel_case_definitions() {
        assert_eq!(found("def myMethod\n  # ...\nend\n"), [snake(1, 5)]);
        assert_eq!(
            found("def my_method\nend\ndef myMethod\nend\n"),
            [snake(3, 5)]
        );
        assert_eq!(found("def self.myMethod\n  # ...\nend\n"), [snake(1, 10)]);
        assert_eq!(
            found("class Sequel\n  def self.fooBar\n  end\nend\n"),
            [snake(2, 12)]
        );
    }

    #[test]
    fn flags_camel_case_in_attr_methods() {
        assert_eq!(
            found("attr_reader :myMethod\n\nattr_accessor :myMethod\n\nattr_writer :myMethod\n"),
            [snake(1, 13), snake(3, 15), snake(5, 13)]
        );
    }

    #[test]
    fn flags_attr_calls_with_several_names_once() {
        // RuboCop reports every name of an `attr` call at the same range (all
        // its arguments) and keeps one offense per range.
        assert_eq!(
            found(
                "attr :my_method, :myMethod\n\nattr_reader :my_method, :myMethod\n\n\
                 attr_accessor :myMethod, :my_method\n\nattr_accessor 'myMethod', 'my_method'\n"
            ),
            [snake(1, 6), snake(3, 13), snake(5, 15), snake(7, 15)]
        );
        assert_eq!(found("attr_reader :fooBar, :bazQux\n"), [snake(1, 13)]);
        assert_eq!(found("attr :fooBar, true\n"), [snake(1, 6)]);
    }

    #[test]
    fn flags_camel_case_struct_and_data_members() {
        for prefix in ["", "::"] {
            let line = format!(
                "{prefix}Struct.new(\"camelCase\", :snake_case, var, *args, :camelCase, \
                 :snake_case_2, \"camelCase2\")"
            );
            assert_eq!(
                found(&format!("{line}\n")),
                [
                    snake(1, col(&line, ":camelCase")),
                    snake(1, col(&line, "\"camelCase2\""))
                ]
            );
            let line = format!(
                "{prefix}Data.define(:snake_case, var, *args, :camelCase, :snake_case_2, \
                 \"camelCase2\")"
            );
            assert_eq!(
                found(&format!("{line}\n")),
                [
                    snake(1, col(&line, ":camelCase")),
                    snake(1, col(&line, "\"camelCase2\""))
                ]
            );
        }
        assert_eq!(found("Struct.new(\"Name\", \"fooBar\")\n"), [snake(1, 20)]);
        assert_eq!(
            found("Struct.new(:fooBar, keyword_init: true) do\nend\n"),
            [snake(1, 12)]
        );
    }

    #[test]
    fn flags_camel_case_aliases() {
        assert_eq!(found("alias fooBar foo\n"), [snake(1, 7)]);
        assert_eq!(found("alias :fooBar :foo\n"), [snake(1, 7)]);
        assert_eq!(found("alias_method :fooBar, :foo\n"), [snake(1, 14)]);
        assert_eq!(found("alias_method \"fooBar\", \"foo\"\n"), [snake(1, 14)]);
        assert_eq!(found("alias_method(:fooBar, :foo)\n"), [snake(1, 14)]);
    }

    #[test]
    fn flags_camel_case_define_method() {
        for name in ["fooBar", "fooBar?", "fooBar!", "fooBar="] {
            assert_eq!(
                found(&format!("define_method :{name} do\nend\n")),
                [snake(1, 15)]
            );
            assert_eq!(
                found(&format!("define_method '{name}' do\nend\n")),
                [snake(1, 15)]
            );
            assert_eq!(
                found(&format!("define_singleton_method :{name} do\nend\n")),
                [snake(1, 25)]
            );
            assert_eq!(
                found(&format!("define_singleton_method '{name}' do\nend\n")),
                [snake(1, 25)]
            );
        }
    }

    #[test]
    fn reports_define_method_one_column_past_the_method_name() {
        // RuboCop's range starts one character after `define_method`.
        assert_eq!(found("define_method(:fooBar) {}\n"), [snake(1, 15)]);
        assert_eq!(found("define_method  :fooBar do end\n"), [snake(1, 15)]);
        assert_eq!(found("obj.define_method :fooBar\n"), [snake(1, 19)]);
        assert_eq!(
            found("define_method :fooBar, instance_method(:x)\n"),
            [snake(1, 15)]
        );
    }

    #[test]
    fn flags_default_forbidden_identifiers() {
        for name in ["__id__", "__send__"] {
            let message = forbidden(name);
            let at = |line, col| (line, col, message.clone());
            assert_eq!(found(&format!("def {name}\n  true\nend\n")), [at(1, 5)]);
            assert_eq!(found(&format!("def {name}; true; end\n")), [at(1, 5)]);
            assert_eq!(found(&format!("def self.{name}\nend\n")), [at(1, 10)]);
            assert_eq!(found(&format!("def foo.{name}\nend\n")), [at(1, 9)]);
            assert_eq!(
                found(&format!(
                    "attr_reader :{name}\nattr_writer :{name}\nattr_accessor :{name}\n"
                )),
                [at(1, 13), at(2, 13), at(3, 15)]
            );
            assert_eq!(found(&format!("define_method :{name}\n")), [at(1, 15)]);
            assert_eq!(
                found(&format!("define_singleton_method :{name}\n")),
                [at(1, 25)]
            );
            assert_eq!(found(&format!("Struct.new(:{name})\n")), [at(1, 12)]);
            assert_eq!(found(&format!("Data.define(:{name})\n")), [at(1, 13)]);
            assert_eq!(found(&format!("alias {name} foo\n")), [at(1, 7)]);
            assert_eq!(found(&format!("alias_method :{name}, :foo\n")), [at(1, 14)]);
        }
    }

    #[test]
    fn reports_a_forbidden_attr_name_at_itself() {
        // RuboCop reports it at the call's last argument, naming that one.
        assert_eq!(
            found("attr_reader :__id__, :foo\n"),
            [(1, 13, forbidden("__id__"))]
        );
        assert_eq!(
            found("attr_reader :fooBar, :__send__\n"),
            [snake(1, 13), (1, 22, forbidden("__send__"))]
        );
    }

    #[test]
    fn accepts_rubocop_spec_no_offenses() {
        assert_accepts(&[
            "def body; '' end\n",
            "def +(other)\n  # ...\nend\n",
            "def ~@; end\n",
            "def !@; end\n",
            "alias_method foo, :bar\nalias_method fooBar, :bar\n",
            "alias_method *ary\n",
            "alias_method :foo, :bar, :baz\nalias_method :fooBar, :bar, :baz\n",
            "Struct.new()\n",
            "::Struct.new()\n",
            "attr_reader :my_method\nattr_accessor :my_method\nattr_writer :my_method\n",
            "def my_method\nend\n",
            "alias :\"foo#{bar}\" :baz\n",
            "alias_method \"foo#{bar}\", \"baz\"\n",
            "def última_vista; end\n",
            "def super; end\n",
        ]);
        for method in ["define_method", "define_singleton_method"] {
            for name in ["==", ">=", "<=", ">", "<", "=~", "!", "[]", "[]=", "gärten"] {
                assert_accepts(&[&format!("{method} :{name} do\nend\n")]);
            }
            assert_accepts(&[
                &format!("{method} do\nend\n"),
                &format!("{method} foo do\nend\n"),
                &format!("{method} '`' do\nend\n"),
            ]);
        }
    }

    #[test]
    fn accepts_class_emitter_methods() {
        for kind in ["class", "module"] {
            assert_accepts(&[&format!(
                "{kind} Sequel\n  def self.Model(source)\n  end\n\n  class Model\n  end\nend\n"
            )]);
        }
        assert_accepts(&[
            "module DPN\n  module Flow\n    module BaseFlow\n      class Start\n      end\n      \
             def self.included(base)\n        def base.Start(aws_env, *args)\n        end\n      \
             end\n    end\n  end\nend\n",
            "class Model; end\ndef self.Model; end\n",
            "begin\n  def self.Model; end\n  class Model; end\nend\n",
            "def self.a\n  def self.b\n    def obj.Model; end\n  end\nend\nclass Model; end\n",
        ]);
    }

    #[test]
    fn flags_singleton_methods_without_a_sibling_class() {
        // RuboCop looks for the class among the definition's siblings only,
        // climbing out of enclosing singleton methods with a single
        // statement.
        assert_eq!(
            found("module D\n  def self.Model; end\nend\nclass Model; end\n"),
            [snake(2, 12)]
        );
        assert_eq!(
            found(
                "module Sequel\n  def self.included(base)\n    def base.Start; end\n    \
                 def base.Other; end\n  end\n  class Start; end\nend\n"
            ),
            [snake(3, 14), snake(4, 14)]
        );
        assert_eq!(
            found(
                "module Sequel\n  def included(base)\n    def base.Start; end\n  end\n  class Start; end\nend\n"
            ),
            [snake(3, 14)]
        );
        assert_eq!(
            found("module A\n  def self.Model; end\n  class Foo::Model; end\nend\n"),
            [snake(2, 12)]
        );
        assert_eq!(
            found("module B\n  def self.Model; end\n  module Model; end\nend\n"),
            [snake(2, 12)]
        );
        // Only singleton methods emit classes.
        assert_eq!(
            found("module C\n  def Model; end\n  class Model; end\nend\n"),
            [snake(2, 7)]
        );
        assert_eq!(
            found("def self.Model; end\nModel = 1\nclass Other; end\n"),
            [snake(1, 10)]
        );
        // Several candidates in one file, judged in one walk.
        assert_eq!(
            found(
                "class Foo\n  def self.Bar; end\n  def self.Baz; end\n  def self.Qux; end\n  \
                 def self.__id__; end\n  def self.+(o); end\n  class Baz; end\nend\n\
                 def obj.Bar; end\n"
            ),
            [
                snake(2, 12),
                snake(4, 12),
                (5, 12, forbidden("__id__")),
                snake(9, 9)
            ]
        );
    }

    #[test]
    fn flags_other_non_snake_case_definitions() {
        assert_eq!(found("def fooBar=(v); end\n"), [snake(1, 5)]);
        assert_eq!(found("def Foo; end\n"), [snake(1, 5)]);
        // Like RuboCop, a name without lowercase letters is not snake_case.
        assert_eq!(found("def 日本語; end\n"), [snake(1, 5)]);
        assert_eq!(found("define_method(:\"foo bar\") {}\n"), [snake(1, 15)]);
    }

    #[test]
    fn ignores_lookalikes() {
        assert_accepts(&[
            "def foo_bar?; end\n",
            "def foo_bar!; end\n",
            "def foo_bar=(v); end\n",
            "def _private; end\n",
            "def `(cmd); end\n",
            "def []=(k, v); end\n",
            "def <=>(other); end\n",
            "def -@; end\n",
            "alias $new $old\n",
            "obj&.define_method :fooBar\n",
            "self.attr_reader :fooBar\n",
            "foo.attr :fooBar\n",
            "Foo::Struct.new(:fooBar)\n",
            "Struct&.new(:fooBar)\n",
            "Struct.build(:fooBar)\n",
            "Data.new(:fooBar)\n",
            "Struct.new(:\"foo#{x}\", fooBar)\n",
            "alias_method :fooBar, :foo, &blk\n",
            "attr_reader\n",
            "attr_reader *names\n",
            "fooBar(1)\n",
            "obj.fooBar = 1\n",
            "send(:fooBar)\n",
            "fooBar = 1\n",
            "FooBar = 1\n",
        ]);
    }
}
