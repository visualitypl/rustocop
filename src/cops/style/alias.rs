//! `Style/Alias` — fixed `prefer_alias`, with source-proven safe edits.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense, single_line},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct Alias;
impl Cop for Alias {
    fn name(&self) -> &'static str {
        "Style/Alias"
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
        if let Some(call) = node.as_call_node() {
            if call.receiver().is_some()
                || call.name().as_slice() != b"alias_method"
                || super::accessors::literal_block(&call)
                || scope(cx) == Scope::Dynamic
                || value_used(cx)
            {
                return;
            }
            let args = ast::call_arguments(&call);
            let [new, old] = args.as_slice() else { return };
            if new.as_symbol_node().is_none() || old.as_symbol_node().is_none() {
                return;
            }
            let context = cx
                .ancestors()
                .find(|n| n.as_class_node().is_some() || n.as_module_node().is_some());
            let context = match context {
                Some(n) if n.as_class_node().is_some() => "in a class body",
                Some(_) => "in a module body",
                None => "at the top level",
            };
            let Some(selector) = call.message_loc() else {
                return;
            };
            // Unknown dispatch, inherited targets, hooks and consumed values are
            // not equivalent to lexical alias. The lazy closed-source proof is
            // intentionally much narrower than detection.
            let correction = cx.corrections_requested().then(|| {
                replace(
                    node,
                    format!("alias {} {}", identifier(new)?, identifier(old)?),
                )
            });
            let correction = correction.flatten();
            report(
                self,
                file,
                out,
                selector.start_offset()..selector.end_offset(),
                format!("Use `alias` instead of `alias_method` {context}."),
                correction,
            );
        } else if let Some(alias) = node.as_alias_method_node() {
            // Upstream exempts every enclosing *instance* def, even when a
            // nearer class/block would otherwise select a different scope.
            if scope(cx) == Scope::InstanceEval
                || cx
                    .ancestors()
                    .any(|n| n.as_def_node().is_some_and(|d| d.receiver().is_none()))
            {
                return;
            }
            if scope(cx) == Scope::Dynamic {
                let keyword = alias.keyword_loc();
                report(
                    self,
                    file,
                    out,
                    keyword.start_offset()..keyword.end_offset(),
                    "Use `alias_method` instead of `alias`.".into(),
                    cx.corrections_requested()
                        .then(|| {
                            replace(
                                node,
                                format!(
                                    "alias_method {}, {}",
                                    identifier(&alias.new_name())?,
                                    identifier(&alias.old_name())?
                                ),
                            )
                        })
                        .flatten(),
                );
                return;
            }
            let new = alias.new_name();
            let old = alias.old_name();
            if ![&new, &old]
                .iter()
                .all(|n| n.location().as_slice().starts_with(b":") && bareword_symbol(n))
            {
                return;
            }
            let preferred = format!("{} {}", value(&new), value(&old));
            let current = format!(
                "{} {}",
                single_line(new.location().as_slice()),
                single_line(old.location().as_slice())
            );
            let correction = cx
                .corrections_requested()
                .then(|| {
                    super::correction::combine(
                        file,
                        vec![
                            Correction {
                                range: new.location().start_offset()..new.location().end_offset(),
                                replacement: value(&new),
                            },
                            Correction {
                                range: old.location().start_offset()..old.location().end_offset(),
                                replacement: value(&old),
                            },
                        ],
                    )
                })
                .flatten();
            report(
                self,
                file,
                out,
                new.location().start_offset()..old.location().end_offset(),
                format!("Use `alias {preferred}` instead of `alias {current}`."),
                correction,
            );
        }
    }
}
#[derive(PartialEq, Eq)]
enum Scope {
    Lexical,
    Dynamic,
    InstanceEval,
}
fn scope(cx: &NodeContext<'_, '_>) -> Scope {
    let mut ancestors = cx.ancestors();
    while let Some(parent) = ancestors.next() {
        if parent.as_class_node().is_some() || parent.as_module_node().is_some() {
            return Scope::Lexical;
        }
        if parent.as_def_node().is_some() || parent.as_lambda_node().is_some() {
            return Scope::Dynamic;
        }
        if parent.as_block_node().is_some() {
            let call = ancestors.next();
            return if call
                .and_then(Node::as_call_node)
                .is_some_and(|c| c.name().as_slice() == b"instance_eval")
            {
                Scope::InstanceEval
            } else {
                Scope::Dynamic
            };
        }
    }
    Scope::Lexical
}
fn value(node: &Node<'_>) -> String {
    String::from_utf8_lossy(node.as_symbol_node().unwrap().unescaped()).into_owned()
}
// Port the reference's bareword classification, including its deliberately
// permissive unquoted operators. Correction syntax is checked more strictly.
fn bareword_symbol(node: &Node<'_>) -> bool {
    let Some(symbol) = node.as_symbol_node() else {
        return false;
    };
    let name = symbol.unescaped();
    let identifier = !name.is_empty()
        && (name[0].is_ascii_alphabetic() || name[0] == b'_')
        && name[1..].iter().enumerate().all(|(i, b)| {
            b.is_ascii_alphanumeric()
                || *b == b'_'
                || (i + 2 == name.len() && matches!(b, b'?' | b'!' | b'='))
        });
    identifier
        || (!node.location().as_slice().starts_with(b":\"")
            && !node.location().as_slice().starts_with(b":'")
            && !name.starts_with(b"@")
            && !name.starts_with(b"$"))
}
fn value_used(cx: &NodeContext<'_, '_>) -> bool {
    let Some(parent) = cx.parent() else {
        return false;
    };
    if parent.as_arguments_node().is_some() {
        return cx.ancestors().nth(1).is_some_and(|owner| {
            owner.as_call_node().is_some_and(|call| !call.is_safe_navigation())
                // Prism folds the send on an assignment LHS into its target /
                // compound-write node; its index arguments are still send args.
                || matches!(owner, Node::IndexTargetNode { .. } | Node::IndexAndWriteNode { .. }
                    | Node::IndexOrWriteNode { .. } | Node::IndexOperatorWriteNode { .. })
        });
    }
    matches!(
        parent,
        Node::LocalVariableWriteNode { .. }
            | Node::InstanceVariableWriteNode { .. }
            | Node::ClassVariableWriteNode { .. }
            | Node::GlobalVariableWriteNode { .. }
            | Node::ConstantWriteNode { .. }
            | Node::ConstantPathWriteNode { .. }
            | Node::MultiWriteNode { .. }
            | Node::LocalVariableAndWriteNode { .. }
            | Node::LocalVariableOrWriteNode { .. }
            | Node::LocalVariableOperatorWriteNode { .. }
            | Node::InstanceVariableAndWriteNode { .. }
            | Node::InstanceVariableOrWriteNode { .. }
            | Node::InstanceVariableOperatorWriteNode { .. }
            | Node::ClassVariableAndWriteNode { .. }
            | Node::ClassVariableOrWriteNode { .. }
            | Node::ClassVariableOperatorWriteNode { .. }
            | Node::GlobalVariableAndWriteNode { .. }
            | Node::GlobalVariableOrWriteNode { .. }
            | Node::GlobalVariableOperatorWriteNode { .. }
            | Node::ConstantAndWriteNode { .. }
            | Node::ConstantOrWriteNode { .. }
            | Node::ConstantOperatorWriteNode { .. }
            | Node::ConstantPathAndWriteNode { .. }
            | Node::ConstantPathOrWriteNode { .. }
            | Node::ConstantPathOperatorWriteNode { .. }
            | Node::CallAndWriteNode { .. }
            | Node::CallOrWriteNode { .. }
            | Node::CallOperatorWriteNode { .. }
            | Node::IndexAndWriteNode { .. }
            | Node::IndexOrWriteNode { .. }
            | Node::IndexOperatorWriteNode { .. }
    )
}
fn report(
    cop: &Alias,
    file: &SourceFile,
    out: &mut Vec<Offense>,
    range: std::ops::Range<usize>,
    message: String,
    correction: Option<Correction>,
) {
    let (line, col) = file.line_col(range.start);
    out.push(Offense {
        cop: cop.name(),
        severity: cop.severity(),
        message,
        line,
        col,
        end_line: file.line_of(range.end - 1),
        correction,
    });
}
fn replace(node: &Node<'_>, replacement: String) -> Option<Correction> {
    let loc = node.location();
    Some(Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement,
    })
}

/// RuboCop's `identifier`.
fn identifier(node: &Node<'_>) -> Option<String> {
    if bareword_symbol(node) {
        Some(format!(":{}", value(node)))
    } else {
        std::str::from_utf8(node.location().as_slice())
            .ok()
            .map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/alias_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            crate::parser::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/Alias")
            .collect()
    }
    #[test]
    fn pinned_lexical_symbol_arguments() {
        for (before, after) in [
            ("alias :ala :bala\n", "alias ala bala\n"),
            ("alias :\"ala\" :bala\n", "alias ala bala\n"),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 7, 1));
            assert_eq!(
                found[0].message,
                format!("Use `alias ala bala` instead of `{}`.", before.trim())
            );
            assert!(found[0].correction.is_none());
            assert_correction(&Alias, before, after);
        }
    }
    #[test]
    fn scope_boundaries_and_value_contexts_match_reference() {
        for source in [
            "alias_method :a, :b do; end",
            "alias_method(:a, :b) { effect }",
            "-> { alias_method :a, :b }",
            "module M; instance_eval { alias :a :b }; end",
            "def outer; class << self; alias :a :b; end; end",
            "public(alias_method :a, :b)",
            "x ||= alias_method :a, :b",
            "x, y = alias_method :a, :b",
            "object[alias_method(:a, :b)], other = values",
            "object[alias_method(:a, :b)] ||= value",
            "object[alias_method(:a, :b)] &&= value",
            "object[alias_method(:a, :b)] += value",
        ] {
            assert_eq!(
                crate::parser::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "class C; included { class D; alias_method :a, :b; end }; end",
            "class << self; alias_method :a, :b; end",
            "module M; instance_eval { alias_method :a, :b }; end",
            "-> { alias :a :b }",
            "def self.outer; alias :a :b; end",
            "object&.take(alias_method :a, :b)",
        ] {
            assert_eq!(
                crate::parser::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn pinned_negative_lookalikes() {
        for source in [
            "alias ala bala",
            "alias $ala $bala",
            "alias :ala bala",
            "alias ala :bala",
            "alias :@type :type",
            "alias :\"foo bar\" :baz",
            "alias :\"string#{interpolation}\" :symbol",
            "def foo\n alias :ala :bala\n alias :baz :qux\nend",
            "module M; def foo; instance_eval { alias bar baz }; end; end",
            "class C; receiver.alias_method :ala, :bala; end",
            "def self.method; alias_method :ala, :bala; end",
            "dsl_method { alias_method :ala, :bala }",
            "dsl_method { do_something(_1); alias_method :ala, :bala }",
            "dsl_method { do_something(it); alias_method :ala, :bala }",
            "alias_method :bar, FOO",
            "alias_method :baz, foo.bar",
            "alias_method",
            "alias_method :foo",
            "class C; public alias_method :ala, :bala; end",
            "class C; private alias_method :ala, :bala; end",
            "class C; protected alias_method :ala, :bala; end",
            "module M; module_function alias_method :ala, :bala; end",
            "class C; NAME = alias_method :ala, :bala; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
