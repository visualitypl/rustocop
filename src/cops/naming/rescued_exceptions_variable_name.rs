//! `Naming/RescuedExceptionsVariableName` — prefer `e` (or unused `_e`).
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RescuedExceptionsVariableName;
impl Cop for RescuedExceptionsVariableName {
    fn name(&self) -> &'static str {
        "Naming/RescuedExceptionsVariableName"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_rescue_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(rescue) = node.as_rescue_node() else {
            return;
        };
        let Some(target) = rescue.reference() else {
            return;
        };
        let name = if let Some(n) = target.as_local_variable_target_node() {
            n.name().as_slice()
        } else if let Some(n) = target.as_instance_variable_target_node() {
            n.name().as_slice()
        } else if let Some(n) = target.as_class_variable_target_node() {
            n.name().as_slice()
        } else if let Some(n) = target.as_global_variable_target_node() {
            n.name().as_slice()
        } else if let Some(n) = target.as_constant_target_node() {
            n.name().as_slice()
        } else if let Some(n) = target.as_constant_path_target_node() {
            let Some(name) = n.name() else { return };
            name.as_slice()
        } else {
            return;
        };
        let preferred = if name.starts_with(b"_") { "_e" } else { "e" };
        if name == preferred.as_bytes() {
            return;
        }
        // Prism links sibling clauses through `subsequent`; only other child
        // edges make a handler nested. Heredocs invalidate source-order tests.
        let mut child = node;
        for ancestor in cx.ancestors() {
            if ancestor.as_rescue_node().is_some() && !is_subsequent(ancestor, child) {
                return;
            }
            child = ancestor;
        }
        let Some(root) = cx.ancestors().next_back() else {
            return;
        };
        if file.rescue_preferred_reads(root).contains(node) {
            return;
        }
        let (line, col) = file.line_col(target.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Use `{preferred}` instead of `{}`.",
                String::from_utf8_lossy(name)
            ),
            line,
            col,
            end_line: line,
            correction: cx
                .corrections_requested()
                .then(|| rename(&rescue, &target, name, preferred, cx, file))
                .flatten(),
        });
    }
}

/// RuboCop's `autocorrect`: the variable, then its reads in the body up to a
/// reassignment, or else in the statements after the `begin`.
fn rename(
    rescue: &ruby_prism::RescueNode<'_>,
    target: &Node<'_>,
    name: &[u8],
    preferred: &str,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    let loc = target.location();
    let mut edits = vec![crate::offense::Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement: preferred.to_owned(),
    }];
    let reassigned = rescue
        .statements()
        .is_some_and(|body| correct_node(&body.as_node(), name, preferred, &mut edits));
    if !reassigned
        && let Some(kwbegin) = cx.ancestors().find(|n| {
            n.as_begin_node()
                .is_some_and(|b| b.begin_keyword_loc().is_some())
        })
    {
        let mut ancestors = cx.ancestors().skip_while(|n| !std::ptr::eq(*n, kwbegin));
        ancestors.next();
        if let Some(statements) = ancestors.next().and_then(|n| n.as_statements_node()) {
            let start = kwbegin.location().start_offset();
            for sibling in statements
                .body()
                .iter()
                .filter(|n| n.location().start_offset() > start)
            {
                if correct_node(&sibling, name, preferred, &mut edits) {
                    break;
                }
            }
        }
    }
    crate::cops::style::correction::combine(file, edits)
}

/// RuboCop's `correct_node`: whether it stopped at a reassignment.
fn correct_node(
    node: &Node<'_>,
    name: &[u8],
    preferred: &str,
    edits: &mut Vec<crate::offense::Correction>,
) -> bool {
    struct Walk<'a> {
        name: &'a [u8],
        preferred: &'a str,
        edits: &'a mut Vec<crate::offense::Correction>,
        // Each node: where an omitted hash value is inserted, when it is one.
        stack: Vec<Option<usize>>,
        label_end: Option<usize>,
        done: bool,
    }
    impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Walk<'_> {
        fn enter(&mut self, n: Node<'pr>) {
            let loc = n.location();
            if let Some(pair) = n.as_assoc_node()
                && pair.value().as_implicit_node().is_some()
            {
                self.label_end = Some(pair.key().location().end_offset());
            }
            let parent = self.stack.last().copied().flatten();
            self.stack.push(if n.as_implicit_node().is_some() {
                self.label_end
            } else {
                None
            });
            if self.done {
                return;
            }
            if let Some(read) = n.as_local_variable_read_node() {
                if read.name().as_slice() == self.name {
                    if let Some(at) = parent {
                        // `{error:}`: the value is added after the label.
                        self.edits.push(crate::offense::Correction {
                            range: at..at,
                            replacement: format!(" {}", self.preferred),
                        });
                    } else {
                        self.edits.push(crate::offense::Correction {
                            range: loc.start_offset()..loc.end_offset(),
                            replacement: self.preferred.to_owned(),
                        });
                    }
                }
                return;
            }
            let value = if let Some(w) = n.as_local_variable_write_node() {
                (w.name().as_slice() == self.name).then(|| Some(w.value()))
            } else if let Some(w) = n.as_multi_write_node() {
                targets_name(&n, self.name).then(|| Some(w.value()))
            } else if let Some(w) = n.as_local_variable_operator_write_node() {
                (w.name().as_slice() == self.name).then_some(None)
            } else if let Some(w) = n.as_local_variable_or_write_node() {
                (w.name().as_slice() == self.name).then_some(None)
            } else if let Some(w) = n.as_local_variable_and_write_node() {
                (w.name().as_slice() == self.name).then_some(None)
            } else if let Some(t) = n.as_local_variable_target_node() {
                (t.name().as_slice() == self.name).then_some(None)
            } else {
                None
            };
            if let Some(value) = value {
                if let Some(value) = value {
                    correct_node(&value, self.name, self.preferred, self.edits);
                }
                self.done = true;
            }
        }
        fn leave(&mut self) {
            self.stack.pop();
        }
    }
    let mut walk = Walk {
        name,
        preferred,
        edits,
        stack: Vec::new(),
        label_end: None,
        done: false,
    };
    crate::cops::walk::walk(&mut walk, node);
    walk.done
}

fn targets_name(node: &Node<'_>, name: &[u8]) -> bool {
    struct Find<'a>(&'a [u8], bool);
    impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Find<'_> {
        fn enter(&mut self, n: Node<'pr>) {
            self.1 |= n
                .as_local_variable_target_node()
                .is_some_and(|t| t.name().as_slice() == self.0);
        }
    }
    let Some(write) = node.as_multi_write_node() else {
        return false;
    };
    let mut find = Find(name, false);
    for target in write
        .lefts()
        .iter()
        .chain(write.rest())
        .chain(write.rights().iter())
    {
        crate::cops::walk::walk(&mut find, &target);
    }
    find.1
}

/// Whether this actual parent-child edge links sibling rescue clauses.
fn is_subsequent(parent: &Node<'_>, child: &Node<'_>) -> bool {
    child.as_rescue_node().is_some()
        && parent
            .as_rescue_node()
            .and_then(|n| n.subsequent())
            .is_some_and(|next| next.location().start_offset() == child.location().start_offset())
}

/// Clause identities with preferred-name reads, built by one complete
/// typed-child walk. Queries are logarithmic, without physical containment
/// assumptions or repeated scans of nested bodies.
#[derive(Default)]
pub(crate) struct PreferredReads(Vec<usize>);
impl PreferredReads {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut builder = ReadIndex::default();
        crate::cops::walk::walk(&mut builder, root);
        builder.result.0.sort_unstable();
        builder.result
    }
    fn contains(&self, rescue: &Node<'_>) -> bool {
        self.0
            .binary_search(&rescue.location().start_offset())
            .is_ok()
    }
}

struct ReadFrame<'pr> {
    node: Node<'pr>,
    subtree_read: bool,
    clause_read: bool,
}
#[derive(Default)]
struct ReadIndex<'pr> {
    frames: Vec<ReadFrame<'pr>>,
    result: PreferredReads,
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for ReadIndex<'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        // Upstream passes the lvar *node* to preferred_name, whose string
        // form does not start with `_`; its shadow guard only matches e.
        let read = node
            .as_local_variable_read_node()
            .is_some_and(|n| n.name().as_slice() == b"e");
        self.frames.push(ReadFrame {
            node,
            subtree_read: read,
            clause_read: read,
        });
    }
    fn leave(&mut self) {
        let frame = self.frames.pop().unwrap();
        if frame.node.as_rescue_node().is_some() && frame.clause_read {
            self.result.0.push(frame.node.location().start_offset());
        }
        if let Some(parent) = self.frames.last_mut() {
            parent.subtree_read |= frame.subtree_read;
            // A sibling's reads do not belong to this clause. Still propagate
            // the entire subtree so enclosing clauses see all nested handlers.
            if !is_subsequent(&parent.node, &frame.node) {
                parent.clause_read |= frame.subtree_read;
            }
        }
    }
}

pub(crate) static RESCUE_PREFERRED_READS: crate::source::Fact<PreferredReads> =
    crate::source::Fact::new();

impl crate::source::SourceFile {
    pub(crate) fn rescue_preferred_reads(&self, root: &ruby_prism::Node<'_>) -> &PreferredReads {
        RESCUE_PREFERRED_READS.get(self, || PreferredReads::build(root))
    }
}

#[cfg(test)]
mod tests {
    //! Ported RuboCop specs (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::pipeline::lint_str;
    fn found(s: &str) -> Vec<crate::offense::Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Naming/RescuedExceptionsVariableName")
            .collect()
    }
    #[test]
    fn exact_locations_names_and_exception_forms() {
        for exception in [
            "",
            "StandardError ",
            "ActiveSupport::JSON.my_method ",
            "*handled ",
        ] {
            for name in ["exc", "_exc"] {
                let s = format!("begin\nrescue {exception}=> {name}\n  {name}\nend\n");
                let o = found(&s);
                assert_eq!(o.len(), 1, "{s}");
                assert_eq!((o[0].line, o[0].col), (2, 11 + exception.len() as u32));
                let preferred = if name.starts_with('_') { "_e" } else { "e" };
                assert_eq!(
                    o[0].message,
                    format!("Use `{preferred}` instead of `{name}`.")
                );
            }
        }
    }
    #[test]
    fn preferred_reads_follow_heredoc_clause_ownership() {
        // A descendant interpolation can be physically beyond its rescue's end.
        let inside = "e=1\nbegin\n  raise\nrescue => error\n  puts <<~TXT\n    #{e}\n  TXT\nend\n";
        assert!(found(inside).is_empty());
    }
    #[test]
    fn preferred_reads_in_earlier_heredoc_do_not_belong_to_rescue() {
        // Conversely, an earlier statement's interpolation can be physically
        // inside the rescue's range without belonging to that clause.
        let outside = "e=1\nbegin\n  puts <<~TXT; rescue => error\n    #{e}\n  TXT\n  error\nend\n";
        let o = found(outside);
        assert_eq!(o.len(), 1);
        assert_eq!((o[0].line, o[0].col), (3, 26));
        assert_eq!(o[0].message, "Use `e` instead of `error`.");
    }
    #[test]
    fn nested_rescue_in_heredoc_stays_nested_past_sibling_header() {
        let s = "begin\nrescue => error\n  puts <<~TXT; rescue Other => second\n    #{begin; raise; rescue => inner; inner; end}\n  TXT\n  second\nend\n";
        let o = found(s);
        assert_eq!(o.len(), 2);
        assert_eq!((o[0].line, o[0].col), (2, 11));
        assert_eq!(o[0].message, "Use `e` instead of `error`.");
        assert_eq!((o[1].line, o[1].col), (3, 32));
        assert_eq!(o[1].message, "Use `e` instead of `second`.");
    }
    #[test]
    fn preferred_reads_exclude_siblings_but_include_all_nested_handlers() {
        let o = found("e=1; begin; rescue => first; rescue => second; e; end");
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].message, "Use `e` instead of `first`.");
        assert!(
            found(
                "e=1; begin; rescue => outer; begin; rescue => inner; rescue => next_inner; e; end; end"
            )
            .is_empty()
        );
    }
}
