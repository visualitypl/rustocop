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
            correction: None,
        });
        if target.as_local_variable_target_node().is_some() {
            super::local_rename::attach(out, cx, file, target.location().start_offset(), preferred);
        }
    }
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
    fn preferred_no_variable_writers_nested_and_collision_lookalikes() {
        for s in [
            "begin; rescue => e; end",
            "begin; rescue => _e; end",
            "begin; rescue StandardError; end",
            "begin; rescue my_exception; end",
            "begin; rescue => storage.exception; end",
            "e=1; begin; rescue => exc; log(e, exc); end",
        ] {
            assert!(found(s).is_empty(), "{s}");
        }
        for target in ["exc", "_exc"] {
            let s = format!("_e=1; begin; rescue => {target}; log(_e, {target}); end");
            assert_eq!(
                found(&s).len(),
                1,
                "the pinned descendant guard recognizes e, not _e: {s}"
            );
            let o = crate::cops::testing::correction_offenses(
                &super::RescuedExceptionsVariableName,
                &s,
            );
            assert_eq!(o[0].correction.is_some(), target == "exc", "{s}");
        }
        let o = found(
            "begin; rescue => exc; begin; rescue => inner; log(exc, inner); end; rescue => second; end",
        );
        assert_eq!(o.len(), 2);
        assert_eq!(o[0].message, "Use `e` instead of `exc`.");
        assert_eq!(o[1].message, "Use `e` instead of `second`.");
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
    #[test]
    fn canonical_rescue_renames_follow_binding_through_ensure_and_after_end() {
        crate::cops::testing::assert_correction(
            &super::RescuedExceptionsVariableName,
            "begin\n raise\nrescue => error\n f { |value=error| value }; {error:}\nensure\n log(error)\nend\nlog(error)\n",
            "begin\n raise\nrescue => e\n f { |value=e| value }; {error: e}\nensure\n log(e)\nend\nlog(e)\n",
        );
    }
    #[test]
    fn rescue_uncertainty_is_not_diagnostic_suppression() {
        for s in [
            "error=1; begin; rescue => error; log(error); end",
            "begin; rescue => error; error=build(error); end; log(error)",
            "begin; rescue => error; f { |e| error }; end",
            "begin; rescue => error; binding; end",
            "begin; rescue => error; end; e=1",
            "begin; rescue => @error; end",
        ] {
            let found =
                crate::cops::testing::correction_offenses(&super::RescuedExceptionsVariableName, s);
            assert_eq!(found.len(), 1, "{s}");
            assert!(found[0].correction.is_none(), "{s}");
        }
    }
    #[test]
    fn implicit_method_rescue_and_nonlocal_targets() {
        assert_eq!(found("def f; raise; rescue => error; error; end").len(), 1);
        // Upstream considers named nonlocal targets too; only locals can be safely renamed.
        for target in [
            "@error",
            "@@error",
            "$error",
            "Error",
            "::Error",
            "Some::Error",
        ] {
            let s = format!("begin; rescue => {target}; end");
            let o = crate::cops::testing::correction_offenses(
                &super::RescuedExceptionsVariableName,
                &s,
            );
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (1, 18));
            assert!(o[0].correction.is_none(), "{s}");
            assert_eq!(
                o[0].message,
                format!(
                    "Use `e` instead of `{}`.",
                    target.rsplit("::").next().unwrap()
                )
            );
        }
    }
}
