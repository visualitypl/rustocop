//! `Lint/IneffectiveAccessModifier` — lexical modifiers on singleton methods.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{Node, StatementsNode};
use std::collections::HashMap;

pub struct IneffectiveAccessModifier;
impl Cop for IneffectiveAccessModifier {
    fn name(&self) -> &'static str {
        "Lint/IneffectiveAccessModifier"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let body = if let Some(class) = node.as_class_node() {
            class.body()
        } else if let Some(module) = node.as_module_node() {
            module.body()
        } else {
            return;
        };
        let Some(statements) = body.and_then(|b| b.as_statements_node()) else {
            return;
        };
        // RuboCop starts ONLY at a begin (implicit sequence or parentheses),
        // not at a single kwbegin, rescue, def, block or singleton class.
        let body = statements.body();
        let sequence = if body.len() > 1 {
            statements
        } else if let Some(paren) = body.iter().next().and_then(|n| n.as_parentheses_node()) {
            let Some(statements) = paren.body().and_then(|b| b.as_statements_node()) else {
                return;
            };
            statements
        } else {
            return;
        };
        let root = cx.ancestors().last().unwrap_or(node);
        check_sequence(&sequence, &sequence, None, root, file, out);
    }
}

/// Each explicit begin inherits, but does not return, the current modifier.
/// Other constructs (including ordinary parentheses below the root) are opaque.
fn check_sequence<'pr>(
    statements: &StatementsNode<'pr>,
    ignored_sequence: &StatementsNode<'pr>,
    mut modifier: Option<(&'static str, u32)>,
    root: &Node<'pr>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    for child in statements.body().iter() {
        if let Some(call) = child.as_call_node() {
            if call.receiver().is_none() && call.arguments().is_none() && call.block().is_none() {
                let visibility = match call.name().as_slice() {
                    b"private" => "private",
                    b"protected" => "protected",
                    b"public" => "public",
                    _ => continue,
                };
                modifier = Some((visibility, file.line_of(call.location().start_offset())));
            }
        } else if let Some(def) = child.as_def_node() {
            let Some((visibility, modifier_line)) = modifier else {
                continue;
            };
            if def.receiver().is_none() || visibility == "public" {
                continue;
            }
            if file.ineffective_private_names(root).contains(
                &file.match_pattern_source(def.name().as_slice()),
                ignored_sequence,
            ) {
                continue;
            }
            let alternative = if visibility == "private" {
                "`private_class_method` or `private` inside a `class << self` block"
            } else {
                "`protected` inside a `class << self` block"
            };
            let (line, col) = super::array_literal_in_regexp::reference_line_col(
                def.def_keyword_loc().start_offset(),
                file,
            );
            out.push(Offense {
                cop: "Lint/IneffectiveAccessModifier",
                severity: crate::offense::Severity::Error,
                message: format!("`{visibility}` (on line {modifier_line}) does not make singleton methods {visibility}. Use {alternative} instead."),
                line,
                col,
                end_line: line,
                correction: None,
            });
        } else if let Some(begin) = child.as_begin_node()
            && begin.begin_keyword_loc().is_some()
            && begin.rescue_clause().is_none()
            && begin.ensure_clause().is_none()
            && let Some(statements) = begin.statements()
        {
            check_sequence(&statements, ignored_sequence, modifier, root, file, out);
        }
    }
}

/// A private_class_method symbol anywhere below the root begin exempts that
/// name, regardless of receiver of the def, ordering, nested scope or branch.
/// Index the main AST once and query preorder subtree intervals, rather than
/// rescanning overlapping class subtrees or sibling lists for every singleton
/// definition. Source spans identify sequences only; they cannot bound subtrees
/// because heredoc interpolations can be physically outside their ancestors.
#[derive(Default)]
pub(crate) struct PrivateNames {
    calls: HashMap<Vec<u8>, Vec<usize>>,
    sequences: HashMap<(usize, usize), std::ops::Range<usize>>,
    #[cfg(test)]
    nodes_visited: usize,
}
fn sequence_key(sequence: &StatementsNode<'_>) -> (usize, usize) {
    let loc = sequence.location();
    (loc.start_offset(), loc.end_offset())
}
struct NameCollector<'a> {
    names: &'a mut PrivateNames,
    file: &'a SourceFile,
    next_ordinal: usize,
    stack: Vec<Option<((usize, usize), usize)>>,
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for NameCollector<'_> {
    fn enter(&mut self, node: Node<'pr>) {
        let ordinal = self.next_ordinal;
        self.next_ordinal += 1;
        self.stack.push(
            node.as_statements_node()
                .map(|sequence| (sequence_key(&sequence), ordinal)),
        );
        #[cfg(test)]
        {
            self.names.nodes_visited += 1;
        }
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.receiver().is_some()
            || call.is_safe_navigation()
            || call.name().as_slice() != b"private_class_method"
        {
            return;
        }
        if let Some(args) = call.arguments() {
            for arg in args.arguments().iter() {
                if let Some(symbol) = arg.as_symbol_node() {
                    let raw = symbol.value_loc();
                    let single = symbol
                        .opening_loc()
                        .is_some_and(|o| o.as_slice() == b":'" || o.as_slice().starts_with(b"%s"));
                    // Project source glyphs BEFORE escape parsing, as the pinned
                    // buffer does. Escape result bytes must not be normalized.
                    // Physical multiline symbols may be dsym, not basic sym.
                    let parts = crate::cops::ast::require_literal::parts(
                        raw.as_ref().map_or(symbol.unescaped(), |l| l.as_slice()),
                        symbol.unescaped(),
                        single,
                        self.file,
                    );
                    if let [value] = parts.as_slice() {
                        self.names
                            .calls
                            .entry(value.clone())
                            .or_default()
                            .push(ordinal);
                    }
                }
            }
        }
    }
    fn leave(&mut self) {
        if let Some((key, start)) = self.stack.pop().flatten() {
            self.names.sequences.insert(key, start..self.next_ordinal);
        }
    }
}
impl PrivateNames {
    pub(crate) fn build(root: &Node<'_>, file: &SourceFile) -> Self {
        let mut names = Self::default();
        crate::cops::walk::walk(
            &mut NameCollector {
                names: &mut names,
                file,
                next_ordinal: 0,
                stack: Vec::new(),
            },
            root,
        );
        // Calls are already sorted by traversal entry ordinal.
        names
    }
    fn contains(&self, name: &[u8], sequence: &StatementsNode<'_>) -> bool {
        let range = &self.sequences[&sequence_key(sequence)];
        self.calls.get(name).is_some_and(|ordinals| {
            ordinals
                .get(ordinals.partition_point(|ordinal| *ordinal < range.start))
                .is_some_and(|ordinal| *ordinal < range.end)
        })
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/ineffective_access_modifier_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/IneffectiveAccessModifier")
            .collect()
    }
    #[test]
    fn pinned_private_protected_and_wrong_private_class_method() {
        for (modifier, name, ignored, alternative) in [
            (
                "private",
                "method",
                "",
                "`private_class_method` or `private` inside a `class << self` block",
            ),
            (
                "protected",
                "method",
                "",
                "`protected` inside a `class << self` block",
            ),
            (
                "private",
                "method2",
                "  private_class_method :method\n",
                "`private_class_method` or `private` inside a `class << self` block",
            ),
        ] {
            let source = format!(
                "class C\n  {modifier}\n\n  def self.{name}\n    puts \"hi\"\n  end\n{ignored}end\n"
            );
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (4, 3, 4));
            assert_eq!(
                found[0].message,
                format!(
                    "`{modifier}` (on line 2) does not make singleton methods {modifier}. Use {alternative} instead."
                )
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn pinned_intervening_instance_method() {
        let found = offenses(
            "class C\n\n  private\n\n  def instance_method\n  end\n\n  def self.method\n    puts \"hi\"\n  end\nend\n",
        );
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (8, 3));
        assert!(found[0].message.contains("(on line 3)"));
    }
    #[test]
    fn explicit_begin_inherits_but_does_not_return_modifier_state() {
        let found = offenses(
            "module C\nprivate\nbegin\nprotected\ndef self.f;end\nend\ndef other.g;end\npublic\ndef self.h;end\nend\n",
        );
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(5, 1), (7, 1)]
        );
        assert!(found[0].message.starts_with("`protected` (on line 4)"));
        assert!(found[1].message.starts_with("`private` (on line 2)"));
        assert!(offenses("class C;begin;private;def self.f;end;end;end").is_empty());
        assert_eq!(offenses("class C;(private;def self.f;end);end").len(), 1);
        assert!(offenses("class C;((private;def self.f;end));end").is_empty());
    }
    #[test]
    fn only_symbol_values_exempt_and_upstream_literal_errors_do_not_crash() {
        for literal in [
            "\"f\"",
            "[ :f ]",
            "*[:f]",
            "1",
            "nil",
            "true",
            "false",
            ":\"f#{x}\"",
        ] {
            let source =
                format!("class C; private;def self.f;end;private_class_method {literal};end");
            assert_eq!(offenses(&source).len(), 1, "{source}");
        }
    }
    #[test]
    fn heredoc_descendant_outside_sequence_source_span_exempts_method() {
        let source = "class C\n  private\n  def self.f; end\n  text = <<~TEXT\n    #{private_class_method :f}\n  TEXT\nend\n";
        assert!(offenses(source).is_empty());
    }
    #[test]
    fn unrelated_interleaved_heredoc_does_not_exempt_method() {
        let source = "text = <<~TEXT; class C; private\n  #{private_class_method :f}\nTEXT\n  def self.f; end\nend\n";
        let found = offenses(source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (4, 3, 4));
        assert_eq!(
            found[0].message,
            "`private` (on line 1) does not make singleton methods private. Use `private_class_method` or `private` inside a `class << self` block instead."
        );
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn name_index_is_cached_once_linear_and_file_local() {
        let mut source = String::new();
        for i in 0..20 {
            source.push_str(&format!("class C{i};private;def self.f{i};end;"));
        }
        for i in 0..1000 {
            source.push_str(&format!("def self.s{i};end;private_class_method :s{i};"));
        }
        for i in (0..20).rev() {
            source.push_str(&format!("private_class_method :f{i};end;"));
        }
        let file = SourceFile::new("example.rb".into(), source.into_bytes());
        let parsed = ruby_prism::parse(&file.bytes);
        assert_eq!(parsed.errors().count(), 0);
        let root = parsed.node();
        let names = file.ineffective_private_names(&root);
        assert_eq!(names.calls.len(), 1020);
        assert!(names.nodes_visited < 15_000, "{}", names.nodes_visited);
        assert!(names.sequences.len() <= names.nodes_visited);
        for range in names.sequences.values() {
            assert!(range.start < range.end && range.end <= names.nodes_visited);
        }
        for ordinals in names.calls.values() {
            assert!(ordinals.is_sorted());
            assert!(
                ordinals
                    .iter()
                    .all(|ordinal| *ordinal < names.nodes_visited)
            );
        }
        for i in 0..1000 {
            assert!(std::ptr::eq(names, file.ineffective_private_names(&root)));
            assert!(names.contains(
                format!("s{i}").as_bytes(),
                &root.as_program_node().unwrap().statements(),
            ));
        }
        let other = SourceFile::new(
            "example.rb".into(),
            b"class C; private;def self.s0;end;end".to_vec(),
        );
        let parsed = ruby_prism::parse(&other.bytes);
        assert!(!other.ineffective_private_names(&parsed.node()).contains(
            b"s0",
            &parsed.node().as_program_node().unwrap().statements()
        ));
    }
    #[test]
    fn pinned_no_offense_examples() {
        for source in [
            "class C\n  private\n  def self.method\n    puts \"hi\"\n  end\n  private_class_method :method\nend\n",
            "class C\n  def self.method\n    puts \"hi\"\n  end\n  def self.method2\n    puts \"hi\"\n  end\nend\n",
            "class C\n  private\n  class << self\n    def self.method\n      puts \"hi\"\n    end\n  end\nend\n",
            "class C\n  begin\n  end\n  def do_something\n  end\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
