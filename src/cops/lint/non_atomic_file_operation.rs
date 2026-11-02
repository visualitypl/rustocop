//! `Lint/NonAtomicFileOperation` — syntactic filesystem TOCTOU checks.
//! Detection only: upstream's atomic-operation rewrites are unsafe.

use crate::{
    cops::{
        Cop, NodeContext, ast, variables,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};

pub struct NonAtomicFileOperation;

impl Cop for NonAtomicFileOperation {
    fn name(&self) -> &'static str {
        "Lint/NonAtomicFileOperation"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let (predicate, body, keyword, has_else) = if let Some(n) = node.as_if_node() {
            (
                n.predicate(),
                n.statements(),
                n.if_keyword_loc(),
                n.subsequent()
                    .is_some_and(|s| s.as_else_node().is_none_or(|e| e.statements().is_some())),
            )
        } else if let Some(n) = node.as_unless_node() {
            (
                n.predicate(),
                n.statements(),
                Some(n.keyword_loc()),
                n.else_clause().is_some_and(|e| e.statements().is_some()),
            )
        } else {
            return;
        };
        if has_else || matches!(predicate, Node::AndNode { .. } | Node::OrNode { .. }) {
            return;
        }
        let Some(keyword) = keyword else { return };
        // Own the conditional once: both its predicate and sole body statement
        // can be eligible sends, but Base deduplicates the existence range.
        // Only Statements is transparent; grouping/kwbegin/block stay parents.
        let body = body
            .filter(|s| s.body().len() == 1)
            .and_then(|s| s.body().iter().next());
        let candidates = [Some(predicate), body];
        if !candidates.iter().flatten().any(|n| operation(n).is_some()) {
            return;
        }
        let mut search = ExistenceSearch::default();
        walk(&mut search, node);
        let Some(exist) = search.first else { return };
        let exist_arg = ast::call_arguments(&exist).into_iter().next();
        let mut matched = false;
        let mut projected = None;
        for candidate in candidates.iter().flatten() {
            let Some((call, replacement)) = operation(candidate) else {
                continue;
            };
            let mut options = ForceOptions::default();
            walk(&mut options, candidate);
            if options.explicit_false {
                continue;
            }
            let arguments = ast::call_arguments(&call);
            let same = match (arguments.first(), exist_arg.as_ref()) {
                (None, None) => true,
                (Some(a), Some(b)) => {
                    let locals = projected.get_or_insert_with(|| projected_locals(node, cx, file));
                    // Project BEFORE equality: two raw vcalls can differ in the
                    // reference buffer when a declaration lies between them.
                    let spans = locals.projected.then_some(&locals.spans);
                    ast::fingerprint_for_file_operation(a, file, spans).source_buffer_scope_eq(
                        &ast::fingerprint_for_file_operation(b, file, spans),
                        file,
                    )
                }
                _ => false,
            };
            if !same {
                continue;
            }
            matched = true;
            // Upstream's explicit-false guard searches the whole send, but
            // force_option? searches only arguments, never the receiver.
            let mut argument_options = ForceOptions::default();
            for argument in &arguments {
                walk(&mut argument_options, argument);
            }
            let force = matches!(
                call.name().as_slice(),
                b"makedirs" | b"mkdir_p" | b"mkpath" | b"rm_f" | b"rm_rf"
            ) || argument_options.explicit_true;
            if !force {
                report(
                    self,
                    file,
                    candidate.location().start_offset(),
                    ast::source_end(candidate),
                    format!("Use atomic file operation method `FileUtils.{replacement}`."),
                    out,
                );
            }
        }
        if matched {
            let receiver = ast::short_const_name(&exist.receiver().unwrap()).unwrap();
            let method = String::from_utf8_lossy(exist.name().as_slice()).into_owned();
            let predicate = candidates[0].as_ref().unwrap();
            report(
                self,
                file,
                keyword.start_offset(),
                ast::source_end(predicate),
                format!("Remove unnecessary existence check `{receiver}.{method}`."),
                out,
            );
        }
    }
}
/// Detection identity only. Prism parses original bytes; the pinned source
/// buffer can compose UTF8-MAC names BEFORE classifying bare calls as locals.
/// Query original VariableForce declarations through the actual complete stack,
/// never normalize shared bindings or reparse a candidate.
#[derive(Default)]
struct ProjectedLocals {
    spans: std::collections::HashSet<(usize, usize)>,
    projected: bool,
}
fn projected_locals<'pr>(
    node: &Node<'pr>,
    cx: &NodeContext<'_, 'pr>,
    file: &SourceFile,
) -> ProjectedLocals {
    let Some(root) = cx.ancestors().last() else {
        return ProjectedLocals::default();
    };
    struct Reads<'a, 'pr> {
        ancestors: Vec<Node<'pr>>,
        cx: NodeContext<'a, 'pr>,
        file: &'a SourceFile,
        root: &'a Node<'pr>,
        conditional: &'a Node<'pr>,
        result: ProjectedLocals,
    }
    impl<'pr> NodeVisitor<'pr> for Reads<'_, 'pr> {
        fn enter(&mut self, node: Node<'pr>) {
            let mut local = node.as_local_variable_read_node().is_some();
            if let Some(call) = node.as_call_node()
                && call.is_variable_call()
                && call.receiver().is_none()
                && call.arguments().is_none()
                && call.block().is_none()
                && !call.name().as_slice().is_ascii()
            {
                let context = NodeContext {
                    ancestors: &self.ancestors,
                    ..self.cx
                };
                let locals = self.file.visible_locals(self.root);
                let name = call.name().as_slice();
                let projected = self.file.match_pattern_source(name);
                let closed = self
                    .ancestors
                    .iter()
                    .rev()
                    .zip(std::iter::once(&node).chain(self.ancestors.iter().rev()))
                    .any(|(parent, child)| {
                        matches!(
                            parent,
                            Node::DefNode { .. }
                                | Node::ClassNode { .. }
                                | Node::ModuleNode { .. }
                                | Node::SingletonClassNode { .. }
                        ) && variables::scope_owns_child(parent, child)
                    });
                let before = |start| {
                    heredoc_order(self.file.heredocs(self.root), start)
                        <= heredoc_order(
                            self.file.heredocs(self.root),
                            node.location().start_offset(),
                        )
                };
                local = locals.declaration_names().any(|alias| {
                    alias != name
                        && self.file.match_pattern_source(alias) == projected
                        && (locals.lexically_declared_before_with(alias, &node, &context, before)
                            || (!closed
                                && locals.lexically_declared_before_with(
                                    alias,
                                    self.conditional,
                                    &self.cx,
                                    before,
                                )))
                });
                self.result.projected |= local;
            }
            if local {
                // Retain dynamic literal-container children in the projected
                // fingerprint, rather than recursively using old raw identities.
                for n in std::iter::once(&node).chain(self.ancestors.iter()) {
                    let loc = n.location();
                    self.result
                        .spans
                        .insert((loc.start_offset(), loc.end_offset()));
                }
            }
            self.ancestors.push(node);
        }
        fn leave(&mut self) {
            self.ancestors.pop();
        }
    }
    let mut reads = Reads {
        ancestors: Vec::new(),
        cx: *cx,
        file,
        root,
        conditional: node,
        result: ProjectedLocals::default(),
    };
    walk(&mut reads, node);
    reads.result
}
/// Heredoc bodies lex at their opener, before the rest of that physical line.
/// Same ordering as the existing projected-local consumer; offsets stay intact.
pub(super) fn heredoc_order(
    heredocs: &[crate::source::heredoc_boundaries::Heredoc],
    mut offset: usize,
) -> Vec<usize> {
    let mut key = vec![usize::MAX];
    while let Some(h) = heredocs
        .iter()
        .filter(|h| (h.body.start..h.closing.end).contains(&offset))
        .min_by_key(|h| h.closing.end - h.body.start)
    {
        key.push(offset);
        offset = h.opening.end;
    }
    key.push(offset);
    key.reverse();
    key
}
fn operation<'pr>(node: &Node<'pr>) -> Option<(CallNode<'pr>, &'static str)> {
    let call = node.as_call_node()?;
    if call.is_safe_navigation()
        || !call.receiver().is_some_and(|r| ast::is_const(&r))
        || call.block().is_some_and(|b| b.as_block_node().is_some())
    {
        return None;
    }
    let replacement = match call.name().as_slice() {
        b"mkdir" => "mkdir_p",
        b"remove" | b"delete" | b"unlink" | b"remove_file" | b"rm" | b"rmdir" | b"safe_unlink" => {
            "rm_f"
        }
        b"remove_dir" | b"remove_entry" | b"remove_entry_secure" => "rm_rf",
        b"makedirs" => "makedirs",
        b"mkdir_p" => "mkdir_p",
        b"mkpath" => "mkpath",
        b"rm_f" => "rm_f",
        b"rm_rf" => "rm_rf",
        _ => return None,
    };
    Some((call, replacement))
}
fn report(
    cop: &NonAtomicFileOperation,
    file: &SourceFile,
    start: usize,
    end: usize,
    message: String,
    out: &mut Vec<Offense>,
) {
    let (line, col) = super::array_literal_in_regexp::reference_line_col(start, file);
    out.push(Offense {
        cop: cop.name(),
        severity: cop.severity(),
        message,
        line,
        col,
        end_line: file.line_of(end.saturating_sub(1).max(start)),
        correction: None,
    });
}
#[derive(Default)]
struct ForceOptions {
    explicit_true: bool,
    explicit_false: bool,
}
impl<'pr> NodeVisitor<'pr> for ForceOptions {
    fn enter(&mut self, node: Node<'pr>) {
        if let Some(pair) = node.as_assoc_node()
            && pair
                .key()
                .as_symbol_node()
                .is_some_and(|s| s.unescaped() == b"force")
        {
            self.explicit_true |= pair.value().as_true_node().is_some();
            self.explicit_false |= pair.value().as_false_node().is_some();
        }
    }
}
#[derive(Default)]
struct ExistenceSearch<'pr> {
    first: Option<CallNode<'pr>>,
}
impl<'pr> NodeVisitor<'pr> for ExistenceSearch<'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        if self.first.is_none()
            && let Some(call) = node.as_call_node()
            && !call.is_safe_navigation()
            && matches!(call.name().as_slice(), b"exist?" | b"exists?")
            && call.receiver().is_some_and(|r| {
                ["FileTest", "File", "Dir", "Shell"]
                    .iter()
                    .any(|name| ast::is_global_const(&r, &[*name]))
            })
        {
            self.first = Some(call);
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/non_atomic_file_operation_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/NonAtomicFileOperation")
            .collect()
    }
    #[test]
    fn source_buffer_equality_is_opt_in_and_never_relaxes_old_keys() {
        use crate::{cops::ast, source::SourceFile};
        let source = "# coding: UTF8-MAC\nname(e\u{301}); name(é)\n";
        let file = SourceFile::new("example.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(source.as_bytes());
        let nodes: Vec<_> = parsed
            .node()
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .iter()
            .collect();
        let a = ast::fingerprint_for_conditional_scope(&nodes[0], &file);
        let b = ast::fingerprint_for_conditional_scope(&nodes[1], &file);
        assert_ne!(a, b);
        assert!(!a.conditional_scope_eq(&b));
        assert!(a.source_buffer_scope_eq(&b, &file));
        assert!(!ast::same_node(&nodes[0], &nodes[1]));
        assert_ne!(
            ast::fingerprint_for_branch(&nodes[0], &file),
            ast::fingerprint_for_branch(&nodes[1], &file)
        );
    }
    #[test]
    fn local_projection_never_changes_raw_parse_old_keys_or_explicit_calls() {
        use crate::{
            cops::{
                ast,
                walk::{NodeVisitor, walk},
            },
            source::SourceFile,
        };
        let source = "# coding: UTF8-MAC\né = 1; e\u{301}; é; e\u{301}()\n";
        let file = SourceFile::new("example.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(source.as_bytes());
        let nodes: Vec<_> = parsed
            .node()
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .iter()
            .collect();
        let (call, read, explicit) = (&nodes[1], &nodes[2], &nodes[3]);
        assert!(call.as_call_node().unwrap().is_variable_call());
        assert!(read.as_local_variable_read_node().is_some());
        assert!(!explicit.as_call_node().unwrap().is_variable_call());
        let spans = nodes
            .iter()
            .map(|n| {
                let loc = n.location();
                (loc.start_offset(), loc.end_offset())
            })
            .collect();
        let projected = ast::fingerprint_for_file_operation(call, &file, Some(&spans));
        assert!(projected.source_buffer_scope_eq(
            &ast::fingerprint_for_file_operation(read, &file, Some(&spans)),
            &file
        ));
        assert!(!projected.source_buffer_scope_eq(
            &ast::fingerprint_for_file_operation(explicit, &file, Some(&spans)),
            &file
        ));
        assert!(!ast::same_node(call, read));
        assert_ne!(
            ast::fingerprint_for_branch(call, &file),
            ast::fingerprint_for_branch(read, &file)
        );
        assert!(
            !ast::fingerprint_for_conditional_scope(call, &file).source_buffer_scope_eq(
                &ast::fingerprint_for_conditional_scope(read, &file),
                &file
            )
        );
        // Exercise the complete main-parse walk as well: raw kinds remain intact.
        #[derive(Default)]
        struct Kinds {
            calls: usize,
            reads: usize,
        }
        impl<'pr> NodeVisitor<'pr> for Kinds {
            fn enter(&mut self, n: ruby_prism::Node<'pr>) {
                self.calls += usize::from(n.as_call_node().is_some());
                self.reads += usize::from(n.as_local_variable_read_node().is_some());
            }
        }
        let mut kinds = Kinds::default();
        walk(&mut kinds, &parsed.node());
        assert_eq!((kinds.calls, kinds.reads), (2, 1));
    }
    #[test]
    fn mkdir_reports_both_exact_locations_and_messages_without_edits() {
        let found = offenses("unless FileTest.exist?(path)\n  FileUtils.mkdir(path)\nend\n");
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 1));
        assert_eq!(
            found[0].message,
            "Remove unnecessary existence check `FileTest.exist?`."
        );
        assert_eq!((found[1].line, found[1].col, found[1].end_line), (2, 3, 2));
        assert_eq!(
            found[1].message,
            "Use atomic file operation method `FileUtils.mkdir_p`."
        );
        assert!(found.iter().all(|o| o.correction.is_none()));
    }
    #[test]
    fn projected_local_reads_match_bare_calls_only_after_lexical_declaration() {
        for (decl, alias) in [("é", "e\u{301}"), ("e\u{301}", "é")] {
            for (body, columns) in [
                (
                    format!("{decl} = 1; FileUtils.mkdir({alias}) unless File.exist?({decl})"),
                    [8, 27],
                ),
                (
                    format!(
                        "foo {{ |{decl}| FileUtils.mkdir({alias}) unless File.exist?({decl}) }}"
                    ),
                    [11, 30],
                ),
            ] {
                let source = format!("# coding: UTF8-MAC\n{body}\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 2, "{source}");
                assert_eq!((found[0].line, found[0].col), (2, columns[0]));
                assert_eq!((found[1].line, found[1].col), (2, columns[1]));
                assert_eq!(
                    found[0].message,
                    "Use atomic file operation method `FileUtils.mkdir_p`."
                );
                assert_eq!(
                    found[1].message,
                    "Remove unnecessary existence check `File.exist?`."
                );
                assert!(found.iter().all(|o| o.correction.is_none()));
                assert!(offenses(&source.replace("UTF8-MAC", "UTF-8")).is_empty());
            }
            for body in [
                format!("{decl} = 1; FileUtils.mkdir({alias}()) unless File.exist?({decl})"),
                format!("{decl} = 1; FileUtils.mkdir(self.{alias}) unless File.exist?({decl})"),
                format!("FileUtils.mkdir({alias}) unless File.exist?({decl} = 1)"),
                format!(
                    "{decl} = 1; def f; FileUtils.mkdir({alias}) unless File.exist?({decl} = 1); end"
                ),
            ] {
                let source = format!("# coding: UTF8-MAC\n{body}\n");
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
    }
    #[test]
    fn repeated_modifier_assignments_use_lexical_not_evaluation_order() {
        for (decl, alias) in [("é", "e\u{301}"), ("e\u{301}", "é")] {
            let source = format!(
                "# coding: UTF8-MAC\nFileUtils.mkdir(({decl} = 1; {alias})) unless File.exist?(({decl} = 1; {decl}))\n"
            );
            let found = offenses(&source);
            assert_eq!(found.len(), 2, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 1));
            assert_eq!((found[1].line, found[1].col), (2, 29));
            assert_eq!(
                found[0].message,
                "Use atomic file operation method `FileUtils.mkdir_p`."
            );
            assert_eq!(
                found[1].message,
                "Remove unnecessary existence check `File.exist?`."
            );
            assert!(found.iter().all(|o| o.correction.is_none()));
            let source = format!(
                "# coding: UTF8-MAC\nFileUtils.mkdir(({alias}; {decl} = 1)) unless File.exist?(({alias}; {decl} = 1))\n"
            );
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
    #[test]
    fn force_and_remove_method_families() {
        for (methods, replacement) in [
            (&["mkdir"][..], Some("mkdir_p")),
            (
                &[
                    "remove",
                    "delete",
                    "unlink",
                    "remove_file",
                    "rm",
                    "rmdir",
                    "safe_unlink",
                ][..],
                Some("rm_f"),
            ),
            (
                &["remove_dir", "remove_entry", "remove_entry_secure"][..],
                Some("rm_rf"),
            ),
            (
                &["makedirs", "mkdir_p", "mkpath", "rm_f", "rm_rf"][..],
                None,
            ),
        ] {
            for method in methods {
                let found = offenses(&format!(
                    "if ::FileTest.exists?(path)\n  ::FileUtils.{method}(path)\nend\n"
                ));
                assert_eq!(
                    found.len(),
                    if replacement.is_some() { 2 } else { 1 },
                    "{method}"
                );
                assert_eq!(
                    found[0].message,
                    "Remove unnecessary existence check `FileTest.exists?`."
                );
                if let Some(replacement) = replacement {
                    assert_eq!(
                        found[1].message,
                        format!("Use atomic file operation method `FileUtils.{replacement}`.")
                    );
                }
            }
        }
    }
    #[test]
    fn modifiers_negation_options_and_elsif() {
        for source in [
            "if !FileTest.exist?(path); FileUtils.makedirs(path); end",
            "FileUtils.makedirs(path, force: true) unless FileTest.exists?(path)",
            "FileUtils.makedirs(path, verbose: true) unless FileTest.exists?(path)",
            "if condition; do_something; elsif FileTest.exist?(path); FileUtils.rm_f path; end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        let found = offenses("Dir.mkdir(path, 0o0755) unless Dir.exist?(path)");
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].line, found[0].col), (1, 1));
        assert_eq!((found[1].line, found[1].col), (1, 25));
        let found = offenses(
            "FileUtils.mkdir(path) unless (\n                      FileTest.exist?(path))\n",
        );
        assert_eq!(found.len(), 2);
        assert_eq!((found[1].line, found[1].col, found[1].end_line), (1, 23, 2));
    }
    #[test]
    fn receiver_force_options_do_not_count_as_operation_arguments() {
        let found = offenses(
            "(build_namespace(force: true))::FileUtils.mkdir(path) unless File.exist?(path)",
        );
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].line, found[0].col), (1, 1));
        assert_eq!(
            found[0].message,
            "Use atomic file operation method `FileUtils.mkdir_p`."
        );
        assert_eq!((found[1].line, found[1].col), (1, 55));
        assert_eq!(
            found[1].message,
            "Remove unnecessary existence check `File.exist?`."
        );
        assert!(found.iter().all(|o| o.correction.is_none()));
        for (source, expected) in [
            (
                "(build_namespace(force: true))::FileUtils.mkdir(path, force: true) unless File.exist?(path)",
                1,
            ),
            (
                "(build_namespace(force: true))::FileUtils.mkdir(path, force: false) unless File.exist?(path)",
                0,
            ),
            (
                "(build_namespace(force: false))::FileUtils.mkdir(path) unless File.exist?(path)",
                0,
            ),
            (
                "(build_namespace(force: false))::FileUtils.mkdir(path, force: true) unless File.exist?(path)",
                0,
            ),
            (
                "(build_namespace(force: false))::FileUtils.mkdir(path, force: false) unless File.exist?(path)",
                0,
            ),
        ] {
            assert_eq!(offenses(source).len(), expected, "{source}");
        }
    }
    #[test]
    fn two_direct_operation_children_share_one_condition_finding() {
        let found = offenses(
            "if FileUtils.mkdir(path, FileTest.exist?(path)); FileUtils.remove(path); end\n",
        );
        assert_eq!(found.len(), 3);
        assert_eq!(found.iter().map(|o| o.col).collect::<Vec<_>>(), [1, 4, 50]);
        assert!(found.iter().all(|o| o.correction.is_none()));
    }
    #[test]
    fn pinned_no_offense_lookalikes() {
        for source in [
            "FileUtils.mkdir_p(path)",
            "FileUtils.mkdir_p(y) unless FileTest.exist?(path)",
            "NotFile.remove(path) unless FileUtils.exist?(path)",
            "FileUtils.options_of(:rm) unless FileUtils.exist?(path)",
            "FileUtils.mkdir_p(path) unless FileUtils.options_of(:rm)",
            "FileUtils.remove(path) if FileTest.executable?(path)",
            "if FileTest.exist?(path); FileUtils.mkdir(path); do_something; end",
            "if FileTest.exist?(path); do_something; FileUtils.mkdir(path); end",
            "if FileTest.exist?(path); FileUtils.mkdir(path); else; do_something; end",
            "FileUtils.mkdir(path) if FileTest.exist?(path) && condition",
            "FileUtils.mkdir(path) if FileTest.exist?(path) || condition",
            "mkdir(path) unless FileTest.exist?(path)",
            "storage[:files].delete(file) unless File.exists?(file)",
            "FileUtils.makedirs(path, force: false) unless FileTest.exists?(path)",
            "FileUtils.rm_r(path) if FileTest.exist?(path)",
            "FileUtils.rmtree(path) if FileTest.exist?(path)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
