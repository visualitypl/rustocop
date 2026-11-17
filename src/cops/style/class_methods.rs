//! `Style/ClassMethods` — use lexical self for matching direct singleton defs.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ClassMethods;
impl Cop for ClassMethods {
    fn name(&self) -> &'static str {
        "Style/ClassMethods"
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
        let Some((name, body)) = declaration(node) else {
            return;
        };
        // A dynamic/qualified header can be large. Fingerprint it once, not
        // once per direct method (including mismatching short receivers).
        let name_key = ast::fingerprint(&name);
        for child in elements(body) {
            let Some(def) = child.as_def_node() else {
                continue;
            };
            let Some(original_receiver) = def.receiver() else {
                continue;
            };
            let receiver = ungroup(def.receiver().unwrap());
            if std::mem::discriminant(&name) != std::mem::discriminant(&receiver)
                || name_key != ast::fingerprint(&receiver)
            {
                continue;
            }
            let loc = receiver
                .as_constant_path_node()
                .map(|n| n.name_loc())
                .unwrap_or(receiver.location());
            let (line, col) = file.line_col(loc.start_offset());
            let method = String::from_utf8_lossy(def.name().as_slice()).into_owned();
            let class = String::from_utf8_lossy(name.location().as_slice());
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                col,
                end_line: file.last_line_of(loc),
                message: format!("Use `self.{method}` instead of `{class}.{method}`."),
                correction: cx
                    .corrections_requested()
                    .then(|| file.class_method_bindings(cx.ancestors().last().unwrap_or(node)))
                    .filter(|p| p.contains(node) && !p.observes_program_name)
                    .map(|_| Correction {
                        range: original_receiver.location().start_offset()
                            ..original_receiver.location().end_offset(),
                        replacement: "self".into(),
                    }),
            });
        }
    }
}

// The pinned AST drops the receiver's syntactic parentheses, but retains
// additional grouping as begin nodes. Strip exactly one layer, not all groups.
fn ungroup(node: Node<'_>) -> Node<'_> {
    let Some(body) = node.as_parentheses_node().and_then(|group| group.body()) else {
        return node;
    };
    if let Some(statements) = body.as_statements_node() {
        if statements.body().len() != 1 {
            return node;
        }
        statements.body().first().unwrap()
    } else {
        body
    }
}

pub(super) fn declaration<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Option<Node<'pr>>)> {
    node.as_class_node()
        .map(|n| (n.constant_path(), n.body()))
        .or_else(|| node.as_module_node().map(|n| (n.constant_path(), n.body())))
}

// Parser collapses an implicit singleton statement list; parentheses produce
// a begin node. Unwrap just that one group, never rescue/explicit kwbegin.
pub(super) fn elements<'pr>(body: Option<Node<'pr>>) -> Vec<Node<'pr>> {
    let Some(body) = body else {
        return Vec::new();
    };
    let nodes: Vec<_> = body
        .as_statements_node()
        .map(|n| n.body().iter().collect())
        .unwrap_or_else(|| vec![body]);
    if let [only] = nodes.as_slice()
        && let Some(group) = only.as_parentheses_node()
    {
        return group
            .body()
            .map(|body| {
                body.as_statements_node()
                    .map(|n| n.body().iter().collect())
                    .unwrap_or_else(|| vec![body])
            })
            .unwrap_or_default();
    }
    nodes
}

/// Lazy correction-only evidence: one complete observer walk, at most one
/// shared scope analysis for local block passes, and one pass over disjoint
/// top-level bodies. No reparsing or repeated subtree/sibling scans. Unknown
/// execution ends the proof prefix; detection is never gated by this.
#[derive(Default)]
pub(crate) struct Bindings {
    declarations: std::collections::HashSet<usize>,
    pub(super) observes_sources: bool,
    /// Acquiring the main-script path can expose every source byte, not just
    /// method/VM metadata. Also gates edits that preserve metadata and DATA size.
    pub(super) observes_program_name: bool,
    // Raw byte reads are distinct from method/line metadata. A reader plus a
    // possible current-source acquisition ends every text-changing proof.
    reads_source: bool,
    local_constructors: LocalConstructors,
    acquires_source_path: bool,
    accesses_path: bool,
    repositions_data: bool,
    consumes_stream: bool,
    // The closed list, independent of older accepted-family proofs.
    listed_source_observer: bool,
    // `SCRIPT_LINES__` alone, the listed observer that collects source lines.
    script_lines_observer: bool,
    listed_data_observer: bool,
    observes_argf: bool,
    metadata_constructor_acquisition: bool,
    metadata_constructor_overridden: bool,
    observes_tracepoint: bool,
    observes_snippets: bool,
    literal_regexp_source: bool,
    source_getter_overridden: bool,
    snippet_selector: bool,
    indirect_snippet_dispatch: bool,
    // Unknown block values can be selectors from any origin. Defer only local
    // reads, so the shared binding analysis can prove unmodified &parameters.
    local_block_dispatches: std::collections::HashSet<usize>,
    // A literal conversion proves its selector only while main-AST evidence
    // leaves core conversion dispatch unchanged. Combine after the whole walk:
    // deferred calls can execute after overrides located later in the source.
    selector_conversion_dispatch: bool,
    selector_conversion_overridden: bool,
    #[cfg(test)]
    pub(crate) forwarding_analyses: usize,
    /// DATA's initial byte/line position depends on all preceding source edits.
    /// Kept separate so each newline consumer can opt into this proof.
    pub(super) observes_data: bool,
    #[cfg(test)]
    pub(crate) visits: usize,
    #[cfg(test)]
    pub(crate) observer_visits: usize,
}
impl Bindings {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut result = Self::default();
        crate::cops::walk::walk(&mut result, root);
        result.observes_snippets |= result.literal_regexp_source && result.source_getter_overridden;
        result.observes_program_name |=
            result.selector_conversion_dispatch && result.selector_conversion_overridden;
        // Enumerable implicitly calls #each; these selectors need no explicit
        // IO read in the AST. ARGF can start at the code prefix through ARGV,
        // whereas DATA needs repositioning. Ordinary filename enumeration and
        // payload-only DATA consumption remain metadata/payload observations.
        result.reads_source |= result.consumes_stream
            && (result.observes_argf || (result.observes_data && result.repositions_data));
        result.acquires_source_path |= result.observes_argf;
        result.reads_source |=
            result.metadata_constructor_acquisition && result.metadata_constructor_overridden;
        let mut unknown_locals = std::mem::take(&mut result.local_block_dispatches);
        // The original source-acquisition proof can short-circuit on a known
        // program path. The global barrier still needs a forwarding proof when
        // that path is only metadata (no reader/snippet is otherwise present).
        if (!result.observes_program_name || (!result.reads_source && !result.observes_snippets))
            && !unknown_locals.is_empty()
            && let Some(program) = root.as_program_node()
        {
            use crate::cops::variables::{Declaration, ParameterKind, ScopeAnalysis};
            #[cfg(test)]
            {
                result.forwarding_analyses += 1;
            }
            // One lazy binding pass, never a scan per dispatch/candidate. A
            // block parameter is already Proc-or-nil at entry, but only if no
            // write (including deferred captures and patterns) can replace it.
            // Required/default parameters and arbitrary local values are not
            // forwarding proofs. Shared exposure/mutation evidence includes
            // ignored Binding blocks, default-value captures and uncertainty
            // from reflective/aliased mutation without a syntactic assignment.
            // Definite exposure matches lexical identities, not spellings.
            let analysis = ScopeAnalysis::of(&program);
            for scope in analysis.scopes_in_exit_order() {
                for id in analysis.scope(scope).variables() {
                    let variable = analysis.variable(id);
                    if variable.declaration == Declaration::Parameter(ParameterKind::Block)
                        && variable.assignments.is_empty()
                        && !variable.has_pattern_bindings
                        && !analysis.may_be_dynamically_mutated(id)
                    {
                        for reference in &variable.references {
                            unknown_locals.remove(&analysis.reference(*reference).range.start);
                        }
                    }
                }
            }
        }
        result.acquires_source_path |=
            (result.observes_data || result.observes_tracepoint) && result.accesses_path;
        // DATA starts at the payload, but repositioning (including pread's
        // explicit offset) exposes the code prefix without acquiring a path.
        result.acquires_source_path |= result.observes_data && result.repositions_data;
        result.observes_snippets |= result.snippet_selector && result.indirect_snippet_dispatch;
        result.observes_snippets |= !unknown_locals.is_empty()
            || (result.selector_conversion_dispatch && result.selector_conversion_overridden);
        result.acquires_source_path |= !unknown_locals.is_empty()
            || (result.selector_conversion_dispatch && result.selector_conversion_overridden);
        result.observes_program_name |= !unknown_locals.is_empty();
        result.observes_sources |= result.observes_program_name;
        let Some(program) = root.as_program_node() else {
            result.reads_source |= !result.local_constructors.acquisitions.is_empty();
            return result;
        };
        // BEGIN executes before *all* ordinary statements even when written
        // after a candidate. It cannot be treated as a source-order barrier.
        if program
            .statements()
            .body()
            .iter()
            .any(|n| n.as_pre_execution_node().is_some())
        {
            result.reads_source |= !result.local_constructors.acquisitions.is_empty();
            return result;
        }
        let mut seen = std::collections::HashSet::new();
        for node in program.statements().body().iter() {
            let Some((name, body)) = declaration(&node) else {
                break;
            };
            let Some(name) = name.as_constant_read_node() else {
                break;
            };
            if node
                .as_class_node()
                .is_some_and(|c| c.superclass().is_some())
                || !seen.insert(name.name().as_slice().to_vec())
            {
                break;
            }
            let mut valid = true;
            for element in elements(body) {
                #[cfg(test)]
                {
                    result.visits += 1;
                }
                if let Some(singleton) = element.as_singleton_class_node() {
                    valid &= singleton.expression().as_self_node().is_some();
                    for method in elements(singleton.body()) {
                        #[cfg(test)]
                        {
                            result.visits += 1;
                        }
                        valid &= method.as_def_node().is_some_and(|d| {
                            d.receiver().is_none() && !callback(d.name().as_slice())
                        });
                    }
                } else {
                    valid &= element.as_def_node().is_some_and(|d| {
                        !callback(d.name().as_slice())
                            && d.receiver().is_none_or(|r| {
                                r.as_self_node().is_some() || ast::same_node(&r, &name.as_node())
                            })
                    });
                }
            }
            if !valid {
                break;
            }
            result.declarations.insert(node.location().start_offset());
            result.local_constructors.allow_declaration(&node);
        }
        result.reads_source |= !result.local_constructors.proven();
        result
    }
    /// Unconditional listed-observer hold: no reader or value-flow proof needed.
    /// DATA is an observer only when the parser found an actual data section.
    pub(crate) fn has_listed_source_observer(&self, has_data: bool) -> bool {
        self.listed_source_observer || (self.listed_data_observer && has_data)
    }

    pub(super) fn contains(&self, node: &Node<'_>) -> bool {
        self.declarations.contains(&node.location().start_offset())
    }
}
// Source metadata can escape through acquired methods, eval, tracing or
// source-file reads even when the extracted methods themselves are inert.
// One complete main-AST pass protects all singleton extractions together.
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Bindings {
    fn enter(&mut self, node: Node<'pr>) {
        self.local_constructors.observe(&node);
        #[cfg(test)]
        {
            self.observer_visits += 1;
        }
        // Exact readers include opening a handle (which can escape), copying
        // streams and Pathname/Digest-style APIs. Inspect indirect acquisitions
        // too, without executing code, resolving receivers or following aliases.
        self.acquires_source_path |= matches!(node, Node::SourceFileNode { .. })
            || node
                .as_global_variable_read_node()
                .is_some_and(|g| program_name(g.name().as_slice()))
            || node
                .as_global_variable_or_write_node()
                .is_some_and(|g| program_name(g.name().as_slice()))
            || node
                .as_global_variable_and_write_node()
                .is_some_and(|g| program_name(g.name().as_slice()))
            || node
                .as_global_variable_operator_write_node()
                .is_some_and(|g| program_name(g.name().as_slice()))
            || node.as_call_node().is_some_and(|c| {
                // Block eval executes visible children, not hidden Ruby text.
                !(matches!(
                    c.name().as_slice(),
                    b"class_eval" | b"module_eval" | b"instance_eval"
                ) && c.block().is_some_and(|b| b.as_block_node().is_some()))
                    && dispatch_call_observes(&c, source_path)
            })
            || node
                .as_symbol_node()
                .is_some_and(|s| source_path(s.unescaped()))
            || node
                .as_alias_method_node()
                .is_some_and(|a| symbol_dispatch_observes(a.old_name(), source_path))
            || node
                .as_call_or_write_node()
                .is_some_and(|c| source_path(c.read_name().as_slice()))
            || node
                .as_call_and_write_node()
                .is_some_and(|c| source_path(c.read_name().as_slice()))
            || node
                .as_call_operator_write_node()
                .is_some_and(|c| source_path(c.read_name().as_slice()));
        // DATA exposes an open main-script handle; TracePoint callbacks expose
        // the current event's source path. Keep path/reposition evidence apart
        // so payload-only DATA reads and metadata-only tracing remain harmless.
        self.accesses_path |= selector_acquisition_observes(&node, path_getter);
        self.repositions_data |= selector_acquisition_observes(&node, data_reposition);
        self.consumes_stream |= selector_acquisition_observes(&node, stream_consumer);
        self.listed_source_observer |= listed_source_observer(&node);
        self.script_lines_observer |= named_constant(&node, b"SCRIPT_LINES__")
            || reflected_constant(&node, b"SCRIPT_LINES__");
        self.listed_data_observer |=
            named_constant(&node, b"DATA") || reflected_constant(&node, b"DATA");
        self.observes_argf |= argf_acquisition(&node);
        self.metadata_constructor_acquisition |= node.as_call_node().is_some_and(|c| {
            metadata_constructor(&c)
                && dispatch_call_observes(&c, |name| matches!(name, b"new" | b"initialize"))
        });
        self.metadata_constructor_overridden |= overrides_metadata_constructor(&node);
        self.observes_tracepoint |= node
            .as_constant_read_node()
            .is_some_and(|c| c.name().as_slice() == b"TracePoint")
            || node.as_constant_path_node().is_some_and(|c| {
                c.name()
                    .is_some_and(|name| name.as_slice() == b"TracePoint")
            })
            || node
                .as_constant_or_write_node()
                .is_some_and(|c| c.name().as_slice() == b"TracePoint")
            || node
                .as_constant_and_write_node()
                .is_some_and(|c| c.name().as_slice() == b"TracePoint")
            || node
                .as_constant_operator_write_node()
                .is_some_and(|c| c.name().as_slice() == b"TracePoint");
        self.reads_source |= node
            .as_call_node()
            .is_some_and(|c| local_constructor_call(&c).is_none() && reads_source_call(&c))
            || node
                .as_symbol_node()
                .is_some_and(|s| source_reader(s.unescaped()))
            || node
                .as_alias_method_node()
                .is_some_and(|a| symbol_dispatch_observes(a.old_name(), source_reader_acquisition))
            || node
                .as_call_or_write_node()
                .is_some_and(|c| source_reader_acquisition(c.read_name().as_slice()))
            || node
                .as_call_and_write_node()
                .is_some_and(|c| source_reader_acquisition(c.read_name().as_slice()))
            || node
                .as_call_operator_write_node()
                .is_some_and(|c| source_reader_acquisition(c.read_name().as_slice()));
        self.literal_regexp_source |= node
            .as_call_node()
            .is_some_and(|c| literal_regexp_source(&c));
        self.source_getter_overridden |= overrides_source_getter(&node);
        self.snippet_selector |= node
            .as_symbol_node()
            .is_some_and(|s| source_snippet(s.unescaped()))
            || node
                .as_string_node()
                .is_some_and(|s| source_snippet(s.unescaped()));
        // Exception snippets and retained VM/AST source getters expose raw
        // source even without a file reader or path acquisition in the AST.
        // Do not require keep_script_lines= here: retention can be enabled by
        // the launcher before entry, and acquired objects/getters can escape.
        self.observes_snippets |= node.as_call_node().is_some_and(|c| {
            !literal_regexp_source(&c) && dispatch_call_observes(&c, source_snippet)
        }) || node
            .as_alias_method_node()
            .is_some_and(|a| symbol_dispatch_observes(a.old_name(), source_snippet))
            || node
                .as_call_or_write_node()
                .is_some_and(|c| source_snippet(c.read_name().as_slice()))
            || node
                .as_call_and_write_node()
                .is_some_and(|c| source_snippet(c.read_name().as_slice()))
            || node
                .as_call_operator_write_node()
                .is_some_and(|c| source_snippet(c.read_name().as_slice()))
            || node
                .as_block_argument_node()
                .and_then(|b| b.expression())
                .is_some_and(|n| {
                    dispatch_expression(n)
                        .as_symbol_node()
                        .is_some_and(|s| source_snippet(s.unescaped()))
                });
        // Any namespace can resolve DATA, including Object and its aliases.
        // Inspect only the final name: resolving receivers would require value
        // flow, while the complete walk already visits each path component.
        self.observes_data |= node.as_constant_read_node().is_some_and(|c| c.name().as_slice() == b"DATA")
            || node.as_constant_path_node().is_some_and(|c| c.name().is_some_and(|name| name.as_slice() == b"DATA"))
            // Bare compound assignments read their target without a separate
            // ConstantReadNode, even when short-circuiting skips the write.
            || node.as_constant_or_write_node().is_some_and(|c| c.name().as_slice() == b"DATA")
            || node.as_constant_and_write_node().is_some_and(|c| c.name().as_slice() == b"DATA")
            || node.as_constant_operator_write_node().is_some_and(|c| c.name().as_slice() == b"DATA")
            // Constant lookup (or an aliased/proc-converted selector) can
            // acquire DATA without spelling a ConstantReadNode. No value-flow
            // proof: even an inert const_get symbol conservatively ends it.
            || node.as_call_node().is_some_and(|c| c.name().as_slice() == b"const_get")
            || node.as_symbol_node().is_some_and(|s| s.unescaped() == b"const_get")
            || node.as_call_or_write_node().is_some_and(|c| c.read_name().as_slice() == b"const_get")
            || node.as_call_and_write_node().is_some_and(|c| c.read_name().as_slice() == b"const_get")
            || node.as_call_operator_write_node().is_some_and(|c| c.read_name().as_slice() == b"const_get");
        // Reuse the shared method-mutation matcher, with no receiver or
        // execution-order inference. Mixins/refinements can import conversions
        // whose implementations are not visible in the main AST.
        self.selector_conversion_overridden |= node
            .as_def_node()
            .is_some_and(|d| selector_conversion(d.name().as_slice()))
            || node
                .as_alias_method_node()
                .is_some_and(|a| symbol_selector_observes(a.new_name(), selector_conversion))
            || matches!(node, Node::UndefNode { .. })
            || node.as_call_node().is_some_and(|c| {
                super::file_api::binds_method(&c, &[b"to_sym", b"intern"])
                    || matches!(
                        c.name().as_slice(),
                        b"include" | b"prepend" | b"extend" | b"refine" | b"using"
                    )
            });
        if let Some(expression) = node.as_block_argument_node().and_then(|b| b.expression()) {
            let expression = dispatch_expression(expression);
            if let Some(local) = expression.as_local_variable_read_node() {
                self.local_block_dispatches
                    .insert(local.location().start_offset());
            } else {
                self.selector_conversion_dispatch |= expression
                    .as_call_node()
                    .is_some_and(|c| selector_conversion(c.name().as_slice()));
                // Opaque callback calls are not by themselves raw readers.
                // A known snippet selector elsewhere can feed such a call;
                // literal conversions and to_proc are dispatch acquisitions.
                if expression.as_call_node().is_some_and(|c| {
                    !matches!(c.name().as_slice(), b"to_sym" | b"intern" | b"to_proc")
                }) {
                    self.indirect_snippet_dispatch = true;
                } else {
                    self.reads_source |= selector_block_dispatch_observes(
                        node.as_block_argument_node().unwrap().expression().unwrap(),
                        source_reader_acquisition,
                    );
                    self.observes_snippets |= selector_block_dispatch_observes(
                        node.as_block_argument_node().unwrap().expression().unwrap(),
                        source_snippet,
                    );
                }
                let observes = program_name_block_dispatch_observes(expression);
                self.acquires_source_path |= observes;
                self.observes_program_name |= observes;
            }
        }
        // Anonymous & and ... carry an already-converted VM block, not an
        // unknown selector expression. Named forwarding needs the proof above.
        // Acquisition is enough: aliases, locals, deferred bodies and grouped
        // expressions can pass the path to a reader later. Do not infer value
        // flow or execution order. Compound globals have no separate read node;
        // aliases expose read children. argv0 selectors also cover proc/alias
        // acquisition, including escaped symbols, without resolving Process.
        self.observes_program_name |= node
            .as_global_variable_read_node()
            .is_some_and(|g| program_name(g.name().as_slice()))
            || node
                .as_global_variable_or_write_node()
                .is_some_and(|g| program_name(g.name().as_slice()))
            || node
                .as_global_variable_and_write_node()
                .is_some_and(|g| program_name(g.name().as_slice()))
            || node
                .as_global_variable_operator_write_node()
                .is_some_and(|g| program_name(g.name().as_slice()))
            || node
                .as_call_node()
                .is_some_and(|c| program_name_call_observes(&c))
            || node
                .as_call_or_write_node()
                .is_some_and(|c| matches!(c.read_name().as_slice(), b"argv0" | b"to_proc"))
            || node
                .as_call_and_write_node()
                .is_some_and(|c| matches!(c.read_name().as_slice(), b"argv0" | b"to_proc"))
            || node
                .as_call_operator_write_node()
                .is_some_and(|c| matches!(c.read_name().as_slice(), b"argv0" | b"to_proc"))
            || node
                .as_symbol_node()
                .is_some_and(|s| s.unescaped() == b"argv0")
            || node
                .as_alias_method_node()
                .is_some_and(|a| symbol_dispatch_observes(a.old_name(), program_name_dispatch));
        self.observes_sources |= self.observes_program_name
            || matches!(node, Node::SourceFileNode { .. })
            // Qualified lookup (e.g. Object::RubyVM) exposes the same metadata.
            // As for DATA above, unknown namespaces conservatively end the proof.
            || node.as_constant_read_node().is_some_and(|c| matches!(c.name().as_slice(), b"TracePoint" | b"RubyVM"))
            || node.as_constant_path_node().is_some_and(|c| c.name().is_some_and(|name| matches!(name.as_slice(), b"TracePoint" | b"RubyVM")))
            || node.as_call_node().is_some_and(|c| {
                if c.name().as_slice() == b"to_proc" {
                    proc_conversion_observes(&c, source_observer)
                } else {
                    source_observer(c.name().as_slice())
                }
            })
            // Compound assignments invoke a getter without a CallNode. Treat
            // observer getters (including to_proc) as barriers too, even when
            // short-circuiting may skip the setter. No value flow is inferred.
            || node
                .as_call_or_write_node()
                .is_some_and(|c| source_observer(c.read_name().as_slice()))
            || node
                .as_call_and_write_node()
                .is_some_and(|c| source_observer(c.read_name().as_slice()))
            || node
                .as_call_operator_write_node()
                .is_some_and(|c| source_observer(c.read_name().as_slice()))
            // A selector alias can hide later calls (and symbol dispatch) under
            // another name. No alias graph is needed: its source already ends
            // the proof, irrespective of execution order or alias chains.
            || node
                .as_alias_method_node()
                .is_some_and(|a| symbol_dispatch_observes(a.old_name(), source_observer))
            // Unknown block values may be selectors/procs supplied through a
            // local, parameter, constant, interpolation or anonymous forwarding.
            // Do not try to follow bindings: only static harmless dispatch is
            // proved here. Inert symbols outside dispatch remain harmless data.
            || node
                .as_block_argument_node()
                .is_some_and(|b| b.expression().is_none_or(|n| block_dispatch_observes(n, source_observer)))
            || matches!(node, Node::ForwardingArgumentsNode { .. });
    }
}

// Each wrapper chain belongs to one expression. Block passes, to_proc receivers,
// selector arguments and alias sources inspect it a constant number of times; no candidate
// scans, recursion, reparsing or general dataflow/taint tracking are involved.
pub(crate) fn dispatch_expression(mut node: Node<'_>) -> Node<'_> {
    loop {
        if let Some(group) = node.as_parentheses_node()
            && let Some(body) = group.body()
        {
            node = body;
        } else if let Some(statements) = node.as_statements_node()
            && statements.body().len() == 1
        {
            node = statements.body().first().unwrap();
        } else {
            return node;
        }
    }
}

fn symbol_dispatch_observes(node: Node<'_>, observer: fn(&[u8]) -> bool) -> bool {
    dispatch_expression(node)
        .as_symbol_node()
        .is_none_or(|s| observer(s.unescaped()) || reflective_dispatch(s.unescaped()))
}

fn symbol_selector_observes(node: Node<'_>, observer: fn(&[u8]) -> bool) -> bool {
    dispatch_expression(node)
        .as_symbol_node()
        .is_none_or(|s| observer(s.unescaped()))
}

fn proc_conversion_observes(call: &ruby_prism::CallNode<'_>, observer: fn(&[u8]) -> bool) -> bool {
    call.arguments().is_some()
        || call.block().is_some()
        || call
            .receiver()
            .is_none_or(|n| symbol_dispatch_observes(n, observer))
}

fn block_dispatch_observes(node: Node<'_>, observer: fn(&[u8]) -> bool) -> bool {
    let node = dispatch_expression(node);
    if node.as_nil_node().is_some() {
        return false; // &nil does not dispatch or forward a block.
    }
    if let Some(call) = node.as_call_node()
        && call.name().as_slice() == b"to_proc"
    {
        return proc_conversion_observes(&call, observer);
    }
    symbol_dispatch_observes(node, observer)
}

fn program_name_block_dispatch_observes(node: Node<'_>) -> bool {
    selector_block_dispatch_observes(node, program_name_dispatch)
}

pub(crate) fn selector_block_dispatch_observes(
    node: Node<'_>,
    observer: fn(&[u8]) -> bool,
) -> bool {
    let node = dispatch_expression(node);
    if node.as_lambda_node().is_some() {
        // Literal arrows create a Proc without method lookup or selector
        // conversion. The complete observer walk still checks their bodies.
        return false;
    }
    if let Some(call) = node.as_call_node()
        && selector_conversion(call.name().as_slice())
    {
        return selector_conversion_observes(&call, observer);
    }
    block_dispatch_observes(node, observer)
}

fn program_name(name: &[u8]) -> bool {
    matches!(name, b"$0" | b"$PROGRAM_NAME")
}

fn selector_conversion(name: &[u8]) -> bool {
    matches!(name, b"to_sym" | b"intern")
}

fn selector_conversion_observes(
    call: &ruby_prism::CallNode<'_>,
    observer: fn(&[u8]) -> bool,
) -> bool {
    call.arguments().is_some()
        || call.block().is_some()
        || call.receiver().is_none_or(|n| {
            let n = dispatch_expression(n);
            if let Some(s) = n.as_string_node() {
                observer(s.unescaped()) || reflective_dispatch(s.unescaped())
            } else {
                symbol_dispatch_observes(n, observer)
            }
        })
}

fn program_name_call_observes(call: &ruby_prism::CallNode<'_>) -> bool {
    dispatch_call_observes(call, program_name_dispatch)
}

fn selector_acquisition_observes(node: &Node<'_>, observer: fn(&[u8]) -> bool) -> bool {
    node.as_call_node()
        .is_some_and(|c| dispatch_call_observes(&c, observer))
        || node
            .as_symbol_node()
            .is_some_and(|s| observer(s.unescaped()))
        || node
            .as_alias_method_node()
            .is_some_and(|a| symbol_dispatch_observes(a.old_name(), observer))
        || node
            .as_call_or_write_node()
            .is_some_and(|c| observer(c.read_name().as_slice()))
        || node
            .as_call_and_write_node()
            .is_some_and(|c| observer(c.read_name().as_slice()))
        || node
            .as_call_operator_write_node()
            .is_some_and(|c| observer(c.read_name().as_slice()))
}

/// A `const_get` of the literal constant `name`. The reflected constant is
/// the same listed object, regardless of its namespace or receiver alias.
fn reflected_constant(node: &Node<'_>, name: &[u8]) -> bool {
    node.as_call_node().is_some_and(|call| {
        call.name().as_slice() == b"const_get"
            && call
                .arguments()
                .and_then(|args| args.arguments().iter().next())
                .is_some_and(|arg| {
                    let arg = dispatch_expression(arg);
                    let symbol = arg.as_symbol_node();
                    let string = arg.as_string_node();
                    symbol
                        .as_ref()
                        .map(|s| s.unescaped())
                        .or_else(|| string.as_ref().map(|s| s.unescaped()))
                        .is_some_and(|literal| {
                            literal == name
                                || literal
                                    .strip_suffix(name)
                                    .is_some_and(|prefix| prefix.ends_with(b"::"))
                        })
                })
    })
}

fn named_constant(node: &Node<'_>, name: &[u8]) -> bool {
    node.as_constant_read_node()
        .is_some_and(|n| n.name().as_slice() == name)
        || node
            .as_constant_path_node()
            .is_some_and(|n| n.name().is_some_and(|n| n.as_slice() == name))
        || node
            .as_constant_path_target_node()
            .is_some_and(|n| n.name().is_some_and(|n| n.as_slice() == name))
        || node
            .as_constant_write_node()
            .is_some_and(|n| n.name().as_slice() == name)
        || node
            .as_constant_target_node()
            .is_some_and(|n| n.name().as_slice() == name)
        || node
            .as_constant_or_write_node()
            .is_some_and(|n| n.name().as_slice() == name)
        || node
            .as_constant_and_write_node()
            .is_some_and(|n| n.name().as_slice() == name)
        || node
            .as_constant_operator_write_node()
            .is_some_and(|n| n.name().as_slice() == name)
}

fn listed_source_observer(node: &Node<'_>) -> bool {
    matches!(node, Node::SourceFileNode { .. })
        || named_constant(node, b"SCRIPT_LINES__")
        || node
            .as_global_variable_read_node()
            .is_some_and(|n| program_name(n.name().as_slice()))
        || node
            .as_global_variable_write_node()
            .is_some_and(|n| program_name(n.name().as_slice()))
        || node
            .as_global_variable_target_node()
            .is_some_and(|n| program_name(n.name().as_slice()))
        || node
            .as_global_variable_or_write_node()
            .is_some_and(|n| program_name(n.name().as_slice()))
        || node
            .as_global_variable_and_write_node()
            .is_some_and(|n| program_name(n.name().as_slice()))
        || node
            .as_global_variable_operator_write_node()
            .is_some_and(|n| program_name(n.name().as_slice()))
        || selector_acquisition_observes(node, |name| {
            matches!(
                name,
                b"__dir__" | b"caller" | b"caller_locations" | b"source_location"
            )
        })
}

fn reflective_dispatch(name: &[u8]) -> bool {
    matches!(
        name,
        b"send"
            | b"__send__"
            | b"public_send"
            | b"method"
            | b"public_method"
            | b"singleton_method"
            | b"instance_method"
            | b"public_instance_method"
            | b"alias_method"
            | b"enum_for"
            | b"to_enum"
            | b"to_proc"
    )
}

fn dispatch_call_observes(call: &ruby_prism::CallNode<'_>, observer: fn(&[u8]) -> bool) -> bool {
    let name = call.name().as_slice();
    if name == b"to_proc" {
        return proc_conversion_observes(call, observer);
    }
    if !reflective_dispatch(name) {
        return observer(name);
    }
    let Some(arguments) = call.arguments() else {
        return true;
    };
    let list = arguments.arguments();
    let mut arguments = list.iter();
    let mut alias = name == b"alias_method";
    let mut forwards = matches!(name, b"send" | b"__send__" | b"public_send");
    loop {
        if alias {
            arguments.next(); // alias_method's first argument is the new name.
        }
        let Some(node) = arguments.next() else {
            return true;
        };
        let node = dispatch_expression(node);
        let symbol = node.as_symbol_node();
        let string = node.as_string_node();
        let selector = if let Some(s) = &symbol {
            s.unescaped()
        } else if let Some(s) = &string {
            s.unescaped()
        } else {
            // Nonselectors cannot acquire a core method; unknown expressions
            // may evaluate to an observer and remain correction-only barriers.
            return !matches!(
                node,
                Node::IntegerNode { .. }
                    | Node::FloatNode { .. }
                    | Node::NilNode { .. }
                    | Node::TrueNode { .. }
                    | Node::FalseNode { .. }
            );
        };
        if observer(selector) {
            return true;
        }
        if !reflective_dispatch(selector) {
            return false;
        }
        // send forwards the remaining arguments, so inspect each nested
        // selector once. A Method/alias/proc acquisition can hide selectors
        // supplied later: conservatively stop instead of following value flow.
        if !forwards {
            return true;
        }
        alias = selector == b"alias_method";
        forwards = matches!(selector, b"send" | b"__send__" | b"public_send");
    }
}

/// A deliberately closed grammar, not a value-flow analysis. It only recovers
/// eager, zero-argument `C.new` after a fresh prefix declaration with default
/// allocation/initialization. All other new receivers still count as readers.
/// Collected during the existing observer walk and disjoint prefix pass; no
/// candidate scans, execution, reparsing, or follow-up tree walks.
#[derive(Default)]
struct LocalConstructors {
    uncertain: bool,
    classes: std::collections::HashSet<usize>,
    definitions: std::collections::HashSet<usize>,
    constants: std::collections::HashSet<usize>,
    allowed_constants: std::collections::HashSet<usize>,
    allowed_classes: std::collections::HashSet<usize>,
    allowed_definitions: std::collections::HashSet<usize>,
    declarations: std::collections::HashMap<Vec<u8>, LocalClass>,
    acquisitions: Vec<(Vec<u8>, usize)>,
    invocations: Vec<(Vec<u8>, Vec<u8>)>,
}

struct LocalClass {
    end: usize,
    methods: std::collections::HashSet<Vec<u8>>,
}

impl LocalConstructors {
    fn observe(&mut self, node: &Node<'_>) {
        let offset = node.location().start_offset();
        match node {
            Node::ProgramNode { .. }
            | Node::StatementsNode { .. }
            | Node::ArgumentsNode { .. }
            | Node::SourceFileNode { .. }
            | Node::StringNode { .. }
            | Node::IntegerNode { .. }
            | Node::NilNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. } => {}
            Node::ClassNode { .. } => {
                self.classes.insert(offset);
            }
            Node::DefNode { .. } => {
                self.definitions.insert(offset);
            }
            Node::ConstantReadNode { .. } => {
                self.constants.insert(offset);
            }
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                let name = call.name().as_slice();
                if let Some(class) = local_constructor_call(&call) {
                    self.allowed_constants
                        .insert(call.receiver().unwrap().location().start_offset());
                    self.acquisitions.push((class, offset));
                } else if let Some(receiver) = call.receiver() {
                    if receiver
                        .as_constant_read_node()
                        .is_some_and(|c| c.name().as_slice() == b"File")
                        && matches!(
                            name,
                            b"dirname" | b"realpath" | b"expand_path" | b"basename" | b"join"
                        )
                        && call.block().is_none()
                    {
                        self.allowed_constants
                            .insert(receiver.location().start_offset());
                    } else if let Some(class) = receiver
                        .as_call_node()
                        .and_then(|c| local_constructor_call(&c))
                        && ast::call_arguments(&call).is_empty()
                        && call.block().is_none()
                    {
                        self.invocations.push((class, name.to_vec()));
                    } else {
                        self.uncertain = true;
                    }
                } else if !(matches!(name, b"p" | b"puts") && call.block().is_none()
                    || name == b"__dir__"
                        && ast::call_arguments(&call).is_empty()
                        && call.block().is_none())
                {
                    self.uncertain = true;
                }
            }
            // Assignments, namespaces, callbacks, blocks/lambdas, superclass
            // expressions, bindings and all reflective/deferred mutation are
            // outside this grammar. In particular a class may not escape as a
            // value, even to an otherwise harmless-looking helper.
            _ => self.uncertain = true,
        }
    }

    fn allow_declaration(&mut self, node: &Node<'_>) {
        let Some(class) = node.as_class_node() else {
            return;
        };
        let Some(name) = class.constant_path().as_constant_read_node() else {
            return;
        };
        let name_bytes = name.name().as_slice();
        // Core reopenings are not fresh local classes, even without a visible
        // superclass. Restrict names to the same finite non-core boundary used
        // by the closed declaration helpers.
        if super::mixin_grouping::core_namespace(name_bytes)
            || METADATA_CONSTRUCTORS
                .iter()
                .any(|name| name.as_bytes() == name_bytes)
        {
            return;
        }
        let mut methods = std::collections::HashSet::new();
        for element in elements(class.body()) {
            let Some(def) = element.as_def_node() else {
                return;
            };
            if def.receiver().is_some()
                || def.parameters().is_some()
                || constructor_hook(def.name().as_slice())
                || callback(def.name().as_slice())
                || matches!(def.name().as_slice(), b"p" | b"puts" | b"__dir__")
            {
                return;
            }
            methods.insert(def.name().as_slice().to_vec());
            self.allowed_definitions
                .insert(element.location().start_offset());
        }
        self.allowed_constants
            .insert(name.location().start_offset());
        self.allowed_classes.insert(node.location().start_offset());
        self.declarations.insert(
            name_bytes.to_vec(),
            LocalClass {
                end: node.location().end_offset(),
                methods,
            },
        );
    }

    fn proven(&self) -> bool {
        self.acquisitions.is_empty()
            || (!self.uncertain
                && self.classes == self.allowed_classes
                && self.definitions == self.allowed_definitions
                && self.constants == self.allowed_constants
                && self.acquisitions.iter().all(|(name, offset)| {
                    self.declarations
                        .get(name)
                        .is_some_and(|class| *offset >= class.end)
                })
                && self.invocations.iter().all(|(name, method)| {
                    self.declarations
                        .get(name)
                        .is_some_and(|class| class.methods.contains(method))
                }))
    }
}

fn local_constructor_call(call: &ruby_prism::CallNode<'_>) -> Option<Vec<u8>> {
    if call.name().as_slice() != b"new"
        || call.block().is_some()
        || !ast::call_arguments(call).is_empty()
        || metadata_constructor(call)
    {
        return None;
    }
    call.receiver()?
        .as_constant_read_node()
        .map(|name| name.name().as_slice().to_vec())
}

fn source_path(name: &[u8]) -> bool {
    matches!(
        name,
        b"argv0"
            | b"source_location"
            | b"const_source_location"
            | b"caller"
            | b"caller_locations"
            | b"backtrace"
            | b"backtrace_locations"
            | b"eval"
            | b"class_eval"
            | b"module_eval"
            | b"instance_eval"
    )
}

fn reads_source_call(call: &ruby_prism::CallNode<'_>) -> bool {
    if call.name().as_slice() == b"open"
        && let Some(mode) = call
            .arguments()
            .and_then(|a| a.arguments().iter().nth(1))
            .and_then(|n| n.as_string_node())
        && matches!(
            mode.unescaped(),
            b"w" | b"wb" | b"wt" | b"a" | b"ab" | b"at"
        )
    {
        // A known write-only open cannot expose the old source bytes. Explicit
        // read calls elsewhere still count; unknown/read-write modes do too.
        return false;
    }
    // An unknown receiver is not proof of a non-IO constructor: File/IO can
    // arrive through constants, locals, factories, Method or proc dispatch.
    // Opening an escaping handle is itself enough; no visible read is needed.
    // Exempt a small set of known non-IO core constructors, subject to
    // whole-main-AST mutation evidence (including later/deferred overrides).
    // Exact local zero-argument acquisitions are deferred by the caller to
    // LocalConstructors' closed-file proof; failed proofs restore this barrier.
    dispatch_call_observes(
        call,
        if metadata_constructor(call) {
            source_reader
        } else {
            source_reader_acquisition
        },
    )
}

const METADATA_CONSTRUCTORS: &[&str] = &[
    "Object",
    "Class",
    "Module",
    "Struct",
    "Data",
    "Proc",
    "TracePoint",
    "Pathname",
];

fn metadata_constructor(call: &ruby_prism::CallNode<'_>) -> bool {
    call.receiver().is_some_and(|r| {
        METADATA_CONSTRUCTORS
            .iter()
            .any(|name| ast::is_global_const(&r, &[name]))
    })
}

fn overrides_metadata_constructor(node: &Node<'_>) -> bool {
    let name = if let Some(n) = node.as_constant_write_node() {
        Some(n.name().as_slice())
    } else if let Some(n) = node.as_constant_or_write_node() {
        Some(n.name().as_slice())
    } else if let Some(n) = node.as_constant_and_write_node() {
        Some(n.name().as_slice())
    } else if let Some(n) = node.as_constant_operator_write_node() {
        Some(n.name().as_slice())
    } else {
        node.as_constant_target_node().map(|n| n.name().as_slice())
    };
    let target = match node {
        Node::ConstantPathWriteNode { .. } => Some(
            node.as_constant_path_write_node()
                .unwrap()
                .target()
                .as_node(),
        ),
        Node::ConstantPathOrWriteNode { .. } => Some(
            node.as_constant_path_or_write_node()
                .unwrap()
                .target()
                .as_node(),
        ),
        Node::ConstantPathAndWriteNode { .. } => Some(
            node.as_constant_path_and_write_node()
                .unwrap()
                .target()
                .as_node(),
        ),
        Node::ConstantPathOperatorWriteNode { .. } => Some(
            node.as_constant_path_operator_write_node()
                .unwrap()
                .target()
                .as_node(),
        ),
        _ => None,
    };
    name.is_some_and(|name| METADATA_CONSTRUCTORS.iter().any(|n| n.as_bytes() == name))
        || target
            .as_ref()
            .and_then(ast::short_const_name)
            .is_some_and(|name| METADATA_CONSTRUCTORS.contains(&name.as_str()))
        || node.as_constant_path_target_node().is_some_and(|n| {
            METADATA_CONSTRUCTORS
                .iter()
                .any(|name| name.as_bytes() == n.name_loc().as_slice())
        })
        || declaration(node).is_some_and(|(name, _)| {
            // A bare metadata-class spelling can resolve in a nested lexical
            // namespace. Qualified declarations invalidate that spelling too.
            ast::short_const_name(&name)
                .is_some_and(|name| METADATA_CONSTRUCTORS.contains(&name.as_str()))
        })
        || node
            .as_def_node()
            .is_some_and(|d| constructor_hook(d.name().as_slice()))
        || node.as_alias_method_node().is_some_and(|a| {
            symbol_selector_observes(a.new_name(), constructor_hook)
                || symbol_dispatch_observes(a.old_name(), constructor_mutator)
        })
        || matches!(node, Node::UndefNode { .. })
        || node
            .as_call_node()
            .is_some_and(|c| constructor_mutation_call(&c))
        || node
            .as_symbol_node()
            .is_some_and(|s| constructor_mutator(s.unescaped()))
}

fn constructor_hook(name: &[u8]) -> bool {
    matches!(name, b"new" | b"initialize" | b"allocate")
}

fn constructor_mutator(name: &[u8]) -> bool {
    matches!(
        name,
        b"const_set"
            | b"remove_const"
            | b"autoload"
            | b"define_method"
            | b"define_singleton_method"
            | b"alias_method"
            | b"attr"
            | b"attr_reader"
            | b"attr_accessor"
            | b"remove_method"
            | b"undef_method"
            | b"include"
            | b"prepend"
            | b"extend"
            | b"refine"
            | b"using"
    )
}

fn constructor_mutation_call(call: &ruby_prism::CallNode<'_>) -> bool {
    let name = call.name().as_slice();
    let mut args = call
        .arguments()
        .map(|a| a.arguments().iter())
        .into_iter()
        .flatten();
    // Inspect only selector arguments and mutation targets, once each. This
    // is a finite constructor-proof check, not receiver or alias value flow.
    if matches!(name, b"send" | b"__send__" | b"public_send") {
        loop {
            let Some(arg) = args.next() else {
                return true;
            };
            let arg = dispatch_expression(arg);
            let symbol = arg.as_symbol_node();
            let string = arg.as_string_node();
            let name = if let Some(s) = &symbol {
                s.unescaped()
            } else if let Some(s) = &string {
                s.unescaped()
            } else {
                return true;
            };
            if matches!(name, b"send" | b"__send__" | b"public_send") {
                continue;
            }
            if reflective_dispatch(name) && !constructor_mutator(name) {
                return dispatch_call_observes(call, constructor_mutator);
            }
            return constructor_mutation_arguments(name, args);
        }
    }
    if reflective_dispatch(name) && !constructor_mutator(name) {
        // Acquired mutators may be called with targets supplied later.
        return dispatch_call_observes(call, constructor_mutator);
    }
    constructor_mutation_arguments(name, args)
}

fn constructor_mutation_arguments<'pr>(
    name: &[u8],
    mut args: impl Iterator<Item = Node<'pr>>,
) -> bool {
    if !constructor_mutator(name) {
        return false;
    }
    if matches!(
        name,
        b"include" | b"prepend" | b"extend" | b"refine" | b"using"
    ) {
        return true;
    }
    let constants = matches!(name, b"const_set" | b"remove_const" | b"autoload");
    let multiple = matches!(
        name,
        b"attr" | b"attr_reader" | b"attr_accessor" | b"remove_method" | b"undef_method"
    );
    let matches_target = |node| {
        let node = dispatch_expression(node);
        let symbol = node.as_symbol_node();
        let string = node.as_string_node();
        let target = if let Some(s) = &symbol {
            s.unescaped()
        } else if let Some(s) = &string {
            s.unescaped()
        } else {
            return !matches!(
                node,
                Node::NilNode { .. } | Node::TrueNode { .. } | Node::FalseNode { .. }
            );
        };
        if constants {
            METADATA_CONSTRUCTORS
                .iter()
                .any(|name| name.as_bytes() == target)
        } else {
            constructor_hook(target)
        }
    };
    if multiple {
        args.any(matches_target)
    } else {
        args.next().is_none_or(matches_target)
    }
}

// ARGF is a main-script-capable stream even without DATA or a repositioning
// selector: ARGV (including values supplied before script entry) can name it.
// Track acquisition, not receiver flow, so aliases and deferred bodies remain
// covered in the same walk. Metadata-only handle access is not consumption.
fn argf_acquisition(node: &Node<'_>) -> bool {
    node.as_constant_read_node()
        .is_some_and(|c| c.name().as_slice() == b"ARGF")
        || node
            .as_constant_path_node()
            .is_some_and(|c| c.name().is_some_and(|name| name.as_slice() == b"ARGF"))
        || node
            .as_constant_or_write_node()
            .is_some_and(|c| c.name().as_slice() == b"ARGF")
        || node
            .as_constant_and_write_node()
            .is_some_and(|c| c.name().as_slice() == b"ARGF")
        || node
            .as_constant_operator_write_node()
            .is_some_and(|c| c.name().as_slice() == b"ARGF")
        || node
            .as_global_variable_read_node()
            .is_some_and(|g| g.name().as_slice() == b"$<")
        || node
            .as_global_variable_or_write_node()
            .is_some_and(|g| g.name().as_slice() == b"$<")
        || node
            .as_global_variable_and_write_node()
            .is_some_and(|g| g.name().as_slice() == b"$<")
        || node
            .as_global_variable_operator_write_node()
            .is_some_and(|g| g.name().as_slice() == b"$<")
        || if let Some(call) = node.as_call_node()
            && call.name().as_slice() == b"const_get"
        {
            // A fully spelled literal lookup for DATA (or another constant)
            // is not an ARGF acquisition. Opaque/escaped lookups still may be.
            call.arguments()
                .and_then(|a| a.arguments().iter().next())
                .is_none_or(|arg| {
                    let arg = dispatch_expression(arg);
                    let symbol = arg.as_symbol_node();
                    let string = arg.as_string_node();
                    let name = symbol
                        .as_ref()
                        .map(|s| s.unescaped())
                        .or_else(|| string.as_ref().map(|s| s.unescaped()));
                    name.is_none_or(|name| name == b"ARGF" || name.ends_with(b"::ARGF"))
                })
        } else {
            selector_acquisition_observes(node, |name| name == b"const_get")
        }
}

fn source_reader_acquisition(name: &[u8]) -> bool {
    // File.allocate followed by private/reflective #initialize acquires the
    // same escaping readable handle as #new. Allocation alone reads no bytes.
    source_reader(name) || matches!(name, b"new" | b"initialize")
}

fn path_getter(name: &[u8]) -> bool {
    matches!(name, b"path" | b"to_path")
}

fn data_reposition(name: &[u8]) -> bool {
    matches!(name, b"rewind" | b"seek" | b"sysseek" | b"pos=" | b"pread")
}

fn source_reader(name: &[u8]) -> bool {
    matches!(
        name,
        b"read"
            | b"pread"
            | b"binread"
            | b"readlines"
            | b"readpartial"
            | b"read_nonblock"
            | b"sysread"
            | b"gets"
            | b"readline"
            | b"readchar"
            | b"readbyte"
            | b"each_line"
            | b"each_byte"
            | b"each_char"
            | b"foreach"
            | b"open"
            | b"sysopen"
            | b"reopen"
            | b"popen"
            | b"for_fd"
            | b"try_convert"
            | b"copy_stream"
            | b"file"
            // VM file compilers/parsers acquire escaping source-bearing code
            // objects, like open acquires a readable handle. Keep this raw-byte
            // evidence separate from the code-object/coordinate proof.
            | b"compile_file"
            | b"compile_file_prism"
            | b"parse_file"
    )
}

// IO's instance-reader aliases plus Enumerable's complete implicit #each
// surface (Ruby 3.4). Include enum/lazy acquisition and Enumerator consumption:
// deferred/aliased handles can connect these after the correction candidate.
// Keep this separate from path readers: `[__FILE__].map(&:itself)` is metadata,
// and DATA's original payload-only reads do not expose the code prefix. ARGF
// consumption can expose that prefix without any repositioning selector.
fn stream_consumer(name: &[u8]) -> bool {
    source_reader(name)
        || matches!(
            name,
            b"getbyte"
                | b"getc"
                | b"each"
                | b"each_codepoint"
                | b"set_encoding_by_bom"
                | b"all?"
                | b"any?"
                | b"chain"
                | b"chunk"
                | b"chunk_while"
                | b"collect"
                | b"collect_concat"
                | b"compact"
                | b"count"
                | b"cycle"
                | b"detect"
                | b"drop"
                | b"drop_while"
                | b"each_cons"
                | b"each_entry"
                | b"each_slice"
                | b"each_with_index"
                | b"each_with_object"
                | b"entries"
                | b"filter"
                | b"filter_map"
                | b"find"
                | b"find_all"
                | b"find_index"
                | b"first"
                | b"flat_map"
                | b"grep"
                | b"grep_v"
                | b"group_by"
                | b"include?"
                | b"inject"
                | b"lazy"
                | b"map"
                | b"max"
                | b"max_by"
                | b"member?"
                | b"min"
                | b"min_by"
                | b"minmax"
                | b"minmax_by"
                | b"none?"
                | b"one?"
                | b"partition"
                | b"reduce"
                | b"reject"
                | b"reverse_each"
                | b"select"
                | b"slice_after"
                | b"slice_before"
                | b"slice_when"
                | b"sort"
                | b"sort_by"
                | b"sum"
                | b"take"
                | b"take_while"
                | b"tally"
                | b"to_a"
                | b"to_h"
                | b"to_set"
                | b"uniq"
                | b"zip"
                | b"next"
                | b"next_values"
                | b"peek"
                | b"peek_values"
        )
}

// A closed Regexp literal's core #source exposes its pattern, not script text.
// Preserve the existing literal/Layout controls without guessing arbitrary
// receiver identities. Whole-AST mutation evidence invalidates this exception.
fn literal_regexp_source(call: &ruby_prism::CallNode<'_>) -> bool {
    call.name().as_slice() == b"source"
        && call.arguments().is_none()
        && call.block().is_none()
        && call.receiver().is_some_and(|receiver| {
            matches!(
                dispatch_expression(receiver),
                Node::RegularExpressionNode { .. } | Node::InterpolatedRegularExpressionNode { .. }
            )
        })
}

fn overrides_source_getter(node: &Node<'_>) -> bool {
    node.as_def_node()
        .is_some_and(|d| d.name().as_slice() == b"source")
        || node.as_alias_method_node().is_some_and(|a| {
            symbol_selector_observes(a.new_name(), |name| name == b"source")
                || symbol_dispatch_observes(a.old_name(), source_getter_mutator)
        })
        || matches!(node, Node::UndefNode { .. })
        || node
            .as_call_node()
            .is_some_and(|c| dispatch_call_observes(&c, source_getter_mutator))
        || node
            .as_symbol_node()
            .is_some_and(|s| source_getter_mutator(s.unescaped()))
}

fn source_getter_mutator(name: &[u8]) -> bool {
    // Acquired/aliased mutators can receive their target later. No receiver,
    // execution-order or imported-method value-flow proof is attempted.
    constructor_mutator(name)
        || matches!(
            name,
            b"eval" | b"class_eval" | b"module_eval" | b"instance_eval"
        )
}

fn source_snippet(name: &[u8]) -> bool {
    matches!(
        name,
        b"full_message" | b"detailed_message" | b"script_lines" | b"source"
    )
}

// Dynamic lookup/evaluation can acquire argv0 or a global read without spelling
// either in the main AST. No value flow is inferred. Keep ordinary metadata
// observers separate: byte-only edits can preserve those observations.
fn program_name_dispatch(name: &[u8]) -> bool {
    matches!(
        name,
        b"argv0"
            | b"method"
            | b"public_method"
            | b"singleton_method"
            | b"instance_method"
            | b"public_instance_method"
            | b"send"
            | b"__send__"
            | b"public_send"
            | b"eval"
            | b"class_eval"
            | b"module_eval"
            | b"instance_eval"
            | b"alias_method"
            | b"to_proc"
            | b"enum_for"
            | b"to_enum"
    )
}

fn source_observer(name: &[u8]) -> bool {
    matches!(
        name,
        b"argv0"
            | b"method"
            | b"public_method"
            | b"singleton_method"
            | b"instance_method"
            | b"public_instance_method"
            | b"source_location"
            | b"const_source_location"
            | b"binding"
            | b"caller"
            | b"caller_locations"
            | b"backtrace"
            | b"backtrace_locations"
            | b"full_message"
            | b"eval"
            | b"class_eval"
            | b"module_eval"
            | b"instance_eval"
            | b"set_trace_func"
            | b"send"
            | b"__send__"
            | b"public_send"
            | b"alias_method"
            | b"to_proc"
            | b"enum_for"
            | b"to_enum"
            | b"load"
            | b"require"
            | b"require_relative"
    )
}

fn callback(name: &[u8]) -> bool {
    matches!(
        name,
        b"singleton_method_added"
            | b"method_added"
            | b"inherited"
            | b"const_added"
            | b"const_missing"
            | b"method_missing"
    )
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/class_methods_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    #[test]
    fn reflected_data_is_listed_only_with_a_data_section() {
        for (source, listed) in [
            ("Object.const_get(:DATA)", true),
            ("Object.const_get(\"DATA\")", true),
            ("::Object.const_get('::Object::DATA')", true),
            ("Root = Object; Root.const_get((:DATA))", true),
            ("Object.const_get(:String)", false),
            ("Object.const_get('NOT_DATA')", false),
            ("'Object.const_get(:DATA)'", false),
            ("# Object.const_get(:DATA)", false),
        ] {
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{source}");
            let facts = Bindings::build(&parsed.node());
            assert!(!facts.has_listed_source_observer(false), "{source}");
            assert_eq!(facts.has_listed_source_observer(true), listed, "{source}");
        }
    }
    #[test]
    fn out_of_scope_implicit_and_process_dispatch_is_not_a_listed_observer() {
        // Parsed only: selector/command subjects are never executed.
        for source in [
            "for line in input; print line; end",
            "for line in [1, 2]; p line; end",
            "f(1, *input)",
            "p [*(input)]",
            "f(**input)",
            "Array(input)",
            "Kernel.Array(input)",
            "Kernel.public_send(:Array, input)",
            "Kernel.method('Array')",
            "alias convert Array",
            "Hash(input)",
            "Array.try_convert(input)",
            "system(command)",
            "Process.spawn(command)",
            "exec(command)",
            "fork { body }",
            "Open3.capture3(command)",
            "Kernel.public_send(:system, command)",
            "Kernel.method('system')",
            "alias launch system",
            "alias_method :launch, :spawn",
            ":\"sys\\x74em\"",
            "p `opaque`",
            "p %x(opaque #{value})",
            "object.system ||= value",
        ] {
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{source}");
            let facts = Bindings::build(&parsed.node());
            assert!(!facts.has_listed_source_observer(false), "{source}");
        }
    }
    #[test]
    fn out_of_scope_stream_positions_are_not_listed_observers() {
        for source in [
            "input.seek(0, 2)",
            "input.sysseek(0, 2)",
            "input.rewind",
            "input.pos = 0",
            "input.pos",
            "input.tell",
            "input.eof",
            "input.eof?",
            "input.public_send(:tell)",
            "input.method('pos')",
            "input.public_send(:public_send, :eof?)",
            "alias end_position sysseek",
            "alias_method :cursor, :tell",
            ":\"te\\x6cl\"",
            "input.tell ||= value",
            "input.pos &&= value",
            "input.tell += value",
        ] {
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{source}");
            let facts = Bindings::build(&parsed.node());
            assert!(!facts.has_listed_source_observer(false), "{source}");
        }
        for source in [
            "p 'seek sysseek rewind pos tell eof eof?'",
            "# input.seek(0, 2)\nf(1, 2)",
            "input.position",
            "input.tell_time",
        ] {
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{source}");
            assert!(
                !Bindings::build(&parsed.node()).has_listed_source_observer(false),
                "{source}"
            );
        }
    }
    #[test]
    fn literal_and_unknown_splats_are_outside_the_closed_observer_list() {
        for source in [
            "f(1, *[2, 3])",
            "f(1, *([2, 3]))",
            "p [*[1, 2]]",
            "f(**{a: 1})",
            "a, b = [1, 2]",
            "p 'system Array for *input `opaque`'",
            "# system(command)\nf(1, 2)",
        ] {
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{source}");
            let facts = Bindings::build(&parsed.node());
            assert!(!facts.has_listed_source_observer(false), "{source}");
        }
        // Nested opaque conversions are equally outside the closed list.
        for source in ["f(*[*input])", "f(**{**input})", "a, b = [*input]"] {
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{source}");
            assert!(
                !Bindings::build(&parsed.node()).has_listed_source_observer(false),
                "{source}"
            );
        }
    }
    #[test]
    fn pinned_class_and_module_receiver_locations() {
        for kind in ["class", "module"] {
            let source =
                format!("{kind} Test\n  def Test.some_method\n    do_something\n  end\nend\n");
            let found: Vec<_> = lint_str(&source)
                .into_iter()
                .filter(|o| o.cop == ClassMethods.name())
                .collect();
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 7, 2));
            assert_eq!(
                found[0].message,
                "Use `self.some_method` instead of `Test.some_method`."
            );
        }
    }
    #[test]
    fn pinned_lookalikes_and_non_direct_defs() {
        for source in [
            "module Test; def self.some_method; end; end",
            "class Test; X = Something.new; def X.some_method; end; end",
            "def Test.some_method; end",
            "class Test; if condition; def Test.foo; end; end; end",
            "class Test; class << self; def Test.foo; end; end; end",
            "class Test; private_class_method def Test.foo; end; end",
            "module Outer; class Test; def Outer.foo; end; end; end",
            "class A::Test; def Test.foo; end; end",
            "class Test; def (::Test).foo; end; end",
        ] {
            assert!(
                correction_offenses(&ClassMethods, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn receiver_parentheses_match_pinned_ast_semantics() {
        for kind in ["class", "module"] {
            for receiver in ["A", "A::B"] {
                let source = format!("{kind} {receiver}; def ({receiver}).foo; end; end");
                let found = correction_offenses(&ClassMethods, &source);
                assert_eq!(found.len(), 1, "{source}");
                let col = source.find("def (").unwrap()
                    + 6
                    + receiver.rfind("::").map_or(0, |offset| offset + 2);
                assert_eq!(
                    (found[0].line, found[0].col),
                    (1, u32::try_from(col).unwrap()),
                    "{source}"
                );
                for receiver in [format!("(({receiver}))"), format!("((({receiver})))")] {
                    let name = if receiver.contains("::") { "A::B" } else { "A" };
                    let source = format!("{kind} {name}; def {receiver}.foo; end; end");
                    assert!(
                        correction_offenses(&ClassMethods, &source).is_empty(),
                        "{source}"
                    );
                }
            }
        }
    }
    #[test]
    fn qualified_nested_reopened_singleton_and_shadowed_receivers_still_report() {
        for (source, col, receiver) in [
            ("class A::Test; def (A::Test).foo; end; end", 24, "A::Test"),
            (
                "module M; class Test; def Test.foo; end; end; end",
                27,
                "Test",
            ),
            (
                "class Test; end; class Test; def Test.foo; end; end",
                34,
                "Test",
            ),
            (
                "class << obj; class Test; def Test.foo; end; end; end",
                31,
                "Test",
            ),
            (
                "class Test; Test = other; def Test.foo; end; end",
                31,
                "Test",
            ),
            ("class Test; (def Test.foo; end); end", 18, "Test"),
        ] {
            let found = correction_offenses(&ClassMethods, source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
            assert_eq!(
                found[0].message,
                format!("Use `self.foo` instead of `{receiver}.foo`.")
            );
        }
    }
    #[test]
    fn uncertain_binding_execution_and_callbacks_withhold_edits_not_offenses() {
        for source in [
            "class C; C = other; def C.foo; end; end",
            "C = Class.new; class C; def C.foo; end; end",
            "class C < Base; def C.foo; end; end",
            "class C; end; class C; def C.foo; end; end",
            "module M; class C; def C.foo; end; end; end",
            "class N::C; def (N::C).foo; end; end",
            "class ::C; def (::C).foo; end; end",
            "class << obj; class C; def C.foo; end; end; end",
            "class C; include M; def C.foo; end; end",
            "class C; def C.foo; end; extend M; end",
            "class C; private; def C.foo; end; end",
            "class C; def self.singleton_method_added(name); C = other; end; def C.foo; end; end",
            "class C; def C.foo; end; def self.const_added(name); end; end",
            "setup; class C; def C.foo; end; end",
            "class C; def C.foo; end; end\nBEGIN { C = Class.new; C::C = Class.new }",
            "class C; def C.foo; end; end\nBEGIN { class C; def self.singleton_method_added(n); end; end }",
        ] {
            // Constant assignment in a method is invalid Ruby; use a dynamic
            // callback to exercise the same hazard without a parser rejection.
            let source = source.replace(
                "C = other; end; def C.foo",
                "const_set(:C, other); end; def C.foo",
            );
            assert_eq!(
                ruby_prism::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            let found = correction_offenses(&ClassMethods, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn declaration_proof_is_lazy_cached_and_linear_in_disjoint_bodies() {
        for count in [10, 100, 1000] {
            let source = (0..count).map(|i| format!("class C{i}\ndef C{i}.foo; end\nclass << self\ndef bar; :bar; end\nend\nend\n")).collect::<String>();
            let parsed = ruby_prism::parse(source.as_bytes());
            let root = parsed.node();
            let file = SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
            let proof = file.class_method_bindings(&root);
            assert_eq!(proof.declarations.len(), count);
            assert_eq!(proof.visits, 3 * count);
            for _ in 0..100 {
                assert!(std::ptr::eq(proof, file.class_method_bindings(&root)));
            }
        }
    }

    #[test]
    fn main_script_unknown_dispatch_requires_binding_proof_not_constructor_correlation() {
        for suffix in [
            "items.map(&unknown_selector)",
            "items.map(&proc { value })",
            "items.map(&-> { $0 })",
            "selector = factory; items.map(&selector)",
            "items.map(&@selector)",
            "items.map(&SELECTOR)",
            "items.map(&$selector)",
            "items.map(&(condition ? left : right))",
            "def f(selector); items.map(&selector); end",
            "def f(&block); block ||= factory; items.map(&block); end",
            "def f(&block); later { block = factory }; items.map(&block); end",
            "def f(&block); block, other = factory; items.map(&block); end",
            "def f(&block); factory => block; items.map(&block); end",
            "def f(&block); /(?<block>.)/ =~ text; items.map(&block); end",
            "def f(&block); def g(block); items.map(&block); end; end",
            "def f(&block); items.each { |block| items.map(&block) }; end",
            "items.map(&(\"arg\" + \"v0\").to_sym)",
            "items.map(&:to_sym).each { |selector| items.map(&selector) }",
        ] {
            let parsed = ruby_prism::parse(suffix.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{suffix}");
            assert!(
                Bindings::build(&parsed.node()).observes_program_name,
                "{suffix}"
            );
        }
        // These former top-level "forwarding" examples have no binding proof:
        // `block`/`callback` are arbitrary method calls, not &parameters.
        for (cop, source, col) in [
            (
                &crate::cops::style::MethodCallWithArgsParentheses as &dyn Cop,
                "object.test *args, **kwargs, &block\n",
                1,
            ),
            (
                &crate::cops::style::NestedParenthesizedCalls,
                "outer(inner &block)",
                7,
            ),
            (
                &crate::cops::style::SuperWithArgsParentheses,
                "super &block",
                1,
            ),
            (
                &crate::cops::style::SuperWithArgsParentheses,
                "super *items, **settings, &callback",
                1,
            ),
        ] {
            let found = correction_offenses(cop, source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col));
            assert!(found[0].correction.is_none(), "{source}");
        }
        for suffix in [
            "items.each(&nil)",
            "items.map(&((:to_s)))",
            "items.map(&(\"to_s\".to_sym))",
            "items.map(&(\"to_s\".intern))",
            "items.map(&(:to_s.to_sym))",
            "items.map(&->(n) { n.to_s })",
            "def f(&block); g(&block); end",
            "def f(&block); items.each { g(&((block))) }; end",
            "def f(&block); items.each { |block| g(&nil) }; g(&block); end",
            "def f(&block); items.each { |; block| block = factory }; g(&block); end",
            "def f(&block); def g; block = factory; end; h(&block); end",
            "def f(&); g(&); end",
            "def f(...); g(...); end",
            "items.each { |&block| g(&block) }",
            "f = ->(&block) { g(&block) }",
            "p :\"inert#{value}\"",
        ] {
            let parsed = ruby_prism::parse(suffix.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{suffix}");
            assert!(
                !Bindings::build(&parsed.node()).observes_program_name,
                "{suffix}"
            );
            for (cop, prefix) in [
                (&ClassMethods as &dyn Cop, "class A\n  def A.f; end\nend\n"),
                (&crate::cops::style::TrailingCommaInArrayLiteral, "p [1,]\n"),
            ] {
                let source = format!("{prefix}{suffix}\n");
                let found = correction_offenses(cop, &source);
                assert_eq!(found.len(), 1, "{}: {source}", cop.name());
                assert!(found[0].correction.is_some(), "{}: {source}", cop.name());
            }
        }
    }

    #[test]
    fn literal_selector_conversion_requires_unchanged_core_dispatch() {
        for mutation in [
            "class String; def to_sym; factory; end; end",
            "class Symbol; def intern; factory; end; end",
            "class String; alias to_sym other; end",
            "class String; alias intern other; end",
            "String.define_method(:to_sym) { factory }",
            "String.define_singleton_method(:intern) { factory }",
            "String.attr_reader :to_sym",
            "String.attr_accessor :intern",
            "String.prepend(extension)",
            "String.include(extension)",
            "using refinement",
            "class String; undef to_sym; end",
        ] {
            for dispatch in [
                "items.map(&(\"to_s\".to_sym))",
                "items.map(&(:to_s.intern))",
            ] {
                // No execution-order assumption: a deferred dispatcher can run
                // after an override written later in the source.
                for suffix in [
                    format!("{mutation}; {dispatch}"),
                    format!("{dispatch}; {mutation}"),
                ] {
                    let parsed = ruby_prism::parse(suffix.as_bytes());
                    assert_eq!(parsed.errors().count(), 0, "{suffix}");
                    assert!(
                        Bindings::build(&parsed.node()).observes_program_name,
                        "{suffix}"
                    );
                }
            }
            // Mutations alone do not acquire a main-script path.
            let parsed = ruby_prism::parse(mutation.as_bytes());
            assert!(
                !Bindings::build(&parsed.node()).observes_program_name,
                "{mutation}"
            );
        }
        for suffix in [
            "class String; def ordinary; factory; end; end; items.map(&(\"to_s\".to_sym))",
            "String.attr_reader :ordinary; items.map(&(\"to_s\".intern))",
        ] {
            let parsed = ruby_prism::parse(suffix.as_bytes());
            assert!(
                !Bindings::build(&parsed.node()).observes_program_name,
                "{suffix}"
            );
        }
    }

    #[test]
    fn forwarding_requires_shared_block_bearing_and_reflective_mutation_evidence() {
        for body in [
            "binding(&nil)",
            "binding(&-> {})",
            "binding { }",
            "self.binding(&nil)",
            "Kernel.binding(&nil)",
            "K.binding(&nil)",
            "other.binding",
            "Object.new.binding",
            "(self.binding ||= nil)",
            "(Kernel.binding &&= nil)",
            "(K.binding += value)",
            "binding(&block)",
            "saved = binding(&nil); saved",
            "later { binding(&nil) }",
            "callback = -> { binding(&nil) }",
            "send(:binding)",
            "__send__('binding', &nil)",
            "Kernel.public_send(:binding, &nil)",
            "method(:binding)",
            "Kernel.method('binding')",
            "Kernel.instance_method(:binding)",
            "send(selector)",
            "send(:send, selector)",
            "method(selector)",
            "stash.local_variable_set(:block, selector)",
            "stash.local_variable_set ||= nil",
            "stash.local_variable_set &&= nil",
            "stash.local_variable_set += value",
            "stash.eval(code)",
            "stash.method(:local_variable_set)",
            "Kernel.alias_method(:capture, :binding); capture",
        ] {
            let suffix = format!("def f(&block); {body}; g(&block); end");
            let parsed = ruby_prism::parse(suffix.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{suffix}");
            assert!(
                Bindings::build(&parsed.node()).observes_program_name,
                "{suffix}"
            );
        }
        for suffix in [
            "def f(&); binding(&); end; def g(&block); h(&block); end",
            "def f(...); binding(...); end; def g(&block); h(&block); end",
            "def f(&block); items.each { |block| binding(&nil) }; g(&block); end",
            "def f(&block); def g; binding(&nil); end; h(&block); end",
            "def f(&block); other.to_s; h(&block); end",
            "def f(&block); binding(nil); h(&block); end",
            "def f(&block); send(:to_s); h(&block); end",
            "def f(&block); h(&block); end; items.map(&nil); items.map(&:to_s)",
        ] {
            let parsed = ruby_prism::parse(suffix.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{suffix}");
            assert!(
                !Bindings::build(&parsed.node()).observes_program_name,
                "{suffix}"
            );
        }
        for suffix in [
            "def f(value = binding(&nil).local_variable_set(:block, selector), &block); g(&block); end",
            "def f(value = -> { binding(&nil) }, &block); g(&block); end",
        ] {
            let parsed = ruby_prism::parse(suffix.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{suffix}");
            assert!(
                Bindings::build(&parsed.node()).observes_program_name,
                "{suffix}"
            );
        }
        // Aliases and saved method getters can cross closed lexical scopes;
        // they are uncertainty, not evidence of a read of every local.
        for prefix in [
            "module Kernel; alias capture binding; end;",
            "Kernel.alias_method(:capture, :binding);",
            "GETTER = Kernel.instance_method(:binding);",
            "module Kernel; alias_method :capture, unknown; end;",
        ] {
            let source = format!("{prefix} def f(&block); capture; g(&block); end");
            let parsed = ruby_prism::parse(source.as_bytes());
            assert!(
                Bindings::build(&parsed.node()).observes_program_name,
                "{source}"
            );
        }
    }

    #[test]
    fn forwarding_parameters_require_no_binding_exposure() {
        for suffix in [
            "def f(&block); binding; g(&block); end",
            "def f(&block); g(&block); binding; end",
            "def f(&block); items.each { binding }; g(&block); end",
            "def f(&block); callback = -> { binding }; g(&block); end",
        ] {
            let parsed = ruby_prism::parse(suffix.as_bytes());
            assert!(
                Bindings::build(&parsed.node()).observes_program_name,
                "{suffix}"
            );
        }
        for suffix in [
            "def f(&block); def g; binding; end; h(&block); end",
            "def f(&block); h(&block); end; def g(&block); binding; end",
        ] {
            let parsed = ruby_prism::parse(suffix.as_bytes());
            assert!(
                !Bindings::build(&parsed.node()).observes_program_name,
                "{suffix}"
            );
        }
    }

    #[test]
    fn pinned_canonical_corrections() {
        for kind in ["class", "module"] {
            let source =
                format!("{kind} Test\n  def Test.some_method\n    do_something\n  end\nend\n");
            assert_correction(
                &ClassMethods,
                &source,
                &source.replace("def Test.", "def self."),
            );
        }
    }
}
