//! Potential heap allocation/lifetime observation, shared by every correction.
//!
//! No escape/value-flow proof: acquiring an observer namespace or reader is
//! enough, even in deferred code or AFTER an edit. Literal core File/DATA.stat
//! and immediate namespace metadata have non-heap proofs, revoked by main-AST
//! lookup mutations.
//! Fed by the existing lazy code-object walk; no candidate parsing/scans.
use ruby_prism::Node;

use super::{
    ast, code_object_safety,
    style::class_methods::{dispatch_expression, selector_block_dispatch_observes},
};

#[derive(Default)]
pub(crate) struct Facts {
    observed: bool,
    metadata_exemption: bool,
    metadata_changed: bool,
}

fn scalar_selector(node: Node<'_>) -> Option<Vec<u8>> {
    let node = dispatch_expression(node);
    node.as_symbol_node()
        .map(|s| s.unescaped().to_vec())
        .or_else(|| node.as_string_node().map(|s| s.unescaped().to_vec()))
}

// Only a shallow written core path, never an arbitrary suffix named File/GC.
fn core_constant(node: &Node<'_>, names: &[&str]) -> bool {
    ast::is_global_const(node, names)
        || node.as_constant_path_node().is_some_and(|p| {
            p.name()
                .is_some_and(|n| names.iter().any(|s| s.as_bytes() == n.as_slice()))
                && p.parent()
                    .is_some_and(|p| ast::is_global_const(&p, &["Object"]))
        })
}

fn reader(name: &[u8]) -> bool {
    matches!(
        name,
        b"count_objects"
            | b"count_objects_size"
            | b"count_objects_size_by_type"
            | b"count_nodes"
            | b"count_tdata_objects"
            | b"count_imemo_objects"
            | b"count_symbols"
            | b"memsize_of"
            | b"memsize_of_all"
            | b"each_object"
            | b"reachable_objects_from"
            | b"reachable_objects_from_root"
            | b"dump"
            | b"dump_all"
            | b"dump_shapes"
            | b"stat"
            | b"stat_heap"
            | b"latest_gc_info"
            | b"latest_compact_info"
            | b"total_time"
            | b"total_allocated_objects"
            | b"malloc_allocated_size"
            | b"malloc_allocations"
            | b"define_finalizer"
            | b"undefine_finalizer"
            | b"garbage_collect"
            | b"weakref_alive?"
            | b"_id2ref"
            | b"trace_object_allocations"
            | b"trace_object_allocations_start"
            | b"trace_object_allocations_debug_start"
            | b"trace_object_allocations_stop"
            | b"trace_object_allocations_clear"
            | b"allocation_sourcefile"
            | b"allocation_sourceline"
            | b"allocation_class_path"
            | b"allocation_method_id"
            | b"allocation_generation"
    )
}

fn forwarding(name: &[u8]) -> bool {
    matches!(name, b"send" | b"__send__" | b"public_send")
}
fn getter(name: &[u8]) -> bool {
    matches!(
        name,
        b"method"
            | b"public_method"
            | b"singleton_method"
            | b"instance_method"
            | b"public_instance_method"
            | b"enum_for"
            | b"to_enum"
    )
}

impl Facts {
    fn observe_proc_selector(&mut self, name: &[u8]) {
        self.observed |= reader(name) || forwarding(name) || getter(name) || name == b"to_proc";
        self.metadata_changed |= code_object_safety::mutation(name);
    }

    pub(crate) fn observed(&self) -> bool {
        self.observed || self.metadata_exemption && self.metadata_changed
    }

    pub(crate) fn observe(&mut self, node: &Node<'_>, parent: Option<&Node<'_>>) {
        let namespace = node
            .as_constant_read_node()
            .is_some_and(|c| matches!(c.name().as_slice(), b"ObjectSpace" | b"GC"))
            || node.as_constant_path_node().is_some_and(|c| {
                c.name()
                    .is_some_and(|n| matches!(n.as_slice(), b"ObjectSpace" | b"GC"))
            });
        if namespace {
            let core = core_constant(node, &["GC", "ObjectSpace"]);
            let metadata = parent.is_some_and(|p| {
                p.as_defined_node().is_some()
                    || p.as_call_node().is_some_and(|c| {
                        c.arguments().is_none()
                            && c.block().is_none()
                            && matches!(c.name().as_slice(), b"name" | b"class" | b"object_id")
                            && c.receiver().is_some_and(|r| {
                                r.location().start_offset() == node.location().start_offset()
                                    && r.location().end_offset() == node.location().end_offset()
                            })
                    })
            });
            self.metadata_exemption |= core && metadata;
            self.observed |= !core || !metadata;
        }
        self.metadata_changed |= matches!(
            node,
            Node::ClassNode { .. }
                | Node::ModuleNode { .. }
                | Node::SingletonClassNode { .. }
                | Node::AliasMethodNode { .. }
                | Node::UndefNode { .. }
                | Node::PreExecutionNode { .. }
                | Node::ConstantWriteNode { .. }
                | Node::ConstantPathWriteNode { .. }
                | Node::ConstantOrWriteNode { .. }
                | Node::ConstantPathOrWriteNode { .. }
                | Node::ConstantAndWriteNode { .. }
                | Node::ConstantPathAndWriteNode { .. }
                | Node::ConstantOperatorWriteNode { .. }
                | Node::ConstantPathOperatorWriteNode { .. }
                | Node::ConstantTargetNode { .. }
                | Node::ConstantPathTargetNode { .. }
        );
        self.metadata_changed |= node.as_def_node().is_some_and(|d| {
            let name = d.name().as_slice();
            name == b"stat"
                || forwarding(name)
                || getter(name)
                || code_object_safety::mutation(name)
                || matches!(
                    name,
                    b"method_missing"
                        | b"respond_to_missing?"
                        | b"const_missing"
                        | b"method_added"
                        | b"singleton_method_added"
                        | b"const_added"
                        | b"inherited"
                        | b"to_proc"
                        | b"name"
                        | b"class"
                        | b"object_id"
                )
        });
        if let Some(alias) = node.as_alias_method_node() {
            self.observed |= scalar_selector(alias.old_name()).is_none_or(|n| reader(&n));
        }
        if let Some(block) = node.as_block_argument_node()
            && let Some(expression) = block.expression()
        {
            let expression = dispatch_expression(expression);
            if let Some(symbol) = expression.as_symbol_node() {
                self.observe_proc_selector(symbol.unescaped());
            } else if !matches!(
                expression,
                Node::NilNode { .. } | Node::LambdaNode { .. } | Node::LocalVariableReadNode { .. }
            ) && !expression
                .as_call_node()
                .is_some_and(|c| c.name().as_slice() == b"to_proc")
            {
                self.observed |= selector_block_dispatch_observes(expression, reader);
                self.metadata_changed |= selector_block_dispatch_observes(
                    block.expression().unwrap(),
                    code_object_safety::mutation,
                );
            }
            // Static to_sym/intern reuse the shared conversion/override proof;
            // unresolved conversions cannot establish a harmless selector.
            // Explicit to_proc is checked below, including unresolved receivers.
            // Local block values use the mandatory shared raw-source guard's
            // existing binding proof: only unmodified Proc-or-nil &parameters
            // survive it. Anonymous & already carries a VM block, not a selector.
            // Literal arrows' bodies are checked by the same complete walk.
        }
        // Prism folds compound-assignment getter sends into write nodes rather
        // than CallNode. Analyze that implicit call with the same rules (it has
        // no arguments/block); the RHS is visited separately by the shared walk.
        macro_rules! compound_getter {
            ($access:ident) => {{
                let call = node.$access().unwrap();
                (
                    call.read_name().as_slice().to_vec(),
                    call.receiver(),
                    None,
                    false,
                )
            }};
        }
        let (mut name, receiver, arguments, has_block) = match node {
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                (
                    call.name().as_slice().to_vec(),
                    call.receiver(),
                    call.arguments(),
                    call.block().is_some(),
                )
            }
            Node::CallOrWriteNode { .. } => compound_getter!(as_call_or_write_node),
            Node::CallAndWriteNode { .. } => compound_getter!(as_call_and_write_node),
            Node::CallOperatorWriteNode { .. } => compound_getter!(as_call_operator_write_node),
            _ => return,
        };
        self.metadata_changed |= code_object_safety::mutation(&name);
        let list = arguments.map(|a| a.arguments());
        let mut args = list.as_ref().into_iter().flat_map(|a| a.iter());
        loop {
            if name == b"to_proc" {
                // Only a written Symbol with core-shaped conversion proves a
                // selector. Unknown/composed receivers may acquire a reader or
                // mutation; absence of a literal is not harmless dispatch.
                let symbol = receiver
                    .map(dispatch_expression)
                    .and_then(|n| n.as_symbol_node());
                if let Some(symbol) = symbol
                    && args.next().is_none()
                    && !has_block
                {
                    self.observe_proc_selector(symbol.unescaped());
                } else {
                    self.observed = true;
                }
                return;
            }
            if reader(&name) {
                let file_stat = name == b"stat"
                    && !has_block
                    && receiver
                        .as_ref()
                        .is_some_and(|r| core_constant(r, &["File", "DATA"]));
                self.metadata_exemption |= file_stat;
                self.observed |= !file_stat;
                return;
            }
            if name == b"alias_method" {
                args.next();
                self.observed |= args
                    .next()
                    .and_then(scalar_selector)
                    .is_none_or(|n| reader(&n));
                return;
            }
            if !forwarding(&name) && !getter(&name) {
                return;
            }
            let forwards = forwarding(&name);
            let Some(arg) = args.next() else {
                return;
            };
            let arg = dispatch_expression(arg);
            // Inspect its literal value without moving the expression needed
            // by the fixed non-selector primitive proof below.
            let next = arg
                .as_symbol_node()
                .map(|s| s.unescaped().to_vec())
                .or_else(|| arg.as_string_node().map(|s| s.unescaped().to_vec()));
            let Some(next) = next else {
                // Core reflection rejects fixed non-selector primitives. Unknown
                // selectors can acquire a heap reader; don't infer their type.
                self.observed |= !matches!(
                    arg,
                    Node::IntegerNode { .. }
                        | Node::FloatNode { .. }
                        | Node::NilNode { .. }
                        | Node::TrueNode { .. }
                        | Node::FalseNode { .. }
                );
                return;
            };
            self.metadata_changed |= code_object_safety::mutation(&next);
            if !forwards {
                self.observed |=
                    reader(&next) || forwarding(&next) || getter(&next) || next == b"to_proc";
                return;
            }
            name = next;
        }
    }
}
