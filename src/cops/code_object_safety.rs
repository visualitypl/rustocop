//! Correction-only VM code-object acquisition/uncertainty, independent of IO
//! readers and physical-line/byte deltas. Coordinates, AST flags/shape, bytecode
//! and retained source can change even under equal-length corrections.
//!
//! A narrow closed-use proof, not Ruby value flow: a code namespace escaping
//! into an alias, argument, factory or deferred body is uncertain. Only direct
//! non-code metadata uses are exempt, and only with unchanged main-AST core
//! dispatch. Dynamic evaluation can return factories without a typed VM node;
//! neither evaluated strings nor escaping reflective getters are followed.
//! Marshal loaders can also return VM modules/classes from opaque serialized
//! names. Treat the serializer namespace as a factory acquisition boundary;
//! never decode payloads or connect returned values to their consumers.
use ruby_prism::Node;

use super::{ast, walk};

#[derive(Default)]
pub(crate) struct Facts {
    pub(crate) acquired_or_uncertain: bool,
    /// Independent heap-observation axis, collected in this same lazy walk.
    pub(crate) heap: super::heap_safety::Facts,
    #[cfg(test)]
    pub(crate) visits: usize,
    #[cfg(test)]
    pub(crate) constant_segments: usize,
}

impl Facts {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut collector = Collector {
            facts: Self::default(),
            ancestors: Vec::new(),
            core_exemption: false,
            reflection_exemption: false,
            block_evaluation_exemption: false,
            evaluation_dispatch_changed: false,
            dispatch_changed: false,
            reflection_dispatch_changed: false,
            namespace_changed: false,
        };
        walk::walk(&mut collector, root);
        // Order-independent: deferred/later mutations invalidate earlier
        // metadata/getter proofs too. No per-candidate or per-use subtree walk.
        collector.facts.acquired_or_uncertain |= collector.core_exemption
            && (collector.dispatch_changed || collector.namespace_changed)
            || collector.reflection_exemption && collector.reflection_dispatch_changed
            || collector.block_evaluation_exemption && collector.evaluation_dispatch_changed;
        collector.facts
    }
}

// This is an immediate parent/child edge, not structural equality. Comparing
// identity spans avoids fingerprint walks of a receiver's constant-path tree.
fn is_receiver(call: &ruby_prism::CallNode<'_>, node: &Node<'_>) -> bool {
    call.receiver().is_some_and(|r| {
        r.location().start_offset() == node.location().start_offset()
            && r.location().end_offset() == node.location().end_offset()
    })
}

fn selector(node: &Node<'_>) -> Option<Vec<u8>> {
    node.as_symbol_node()
        .map(|s| s.unescaped().to_vec())
        .or_else(|| node.as_string_node().map(|s| s.unescaped().to_vec()))
}

fn unrelated_constant(name: Option<Node<'_>>) -> bool {
    name.as_ref().and_then(selector).is_some_and(|value| {
        // An unfamiliar literal name can itself be an external VM alias.
        // Prove only core non-code constants, not arbitrary name inequality.
        let value = value.strip_prefix(b"::").unwrap_or(&value);
        let value = value.strip_prefix(b"Object::").unwrap_or(value);
        matches!(
            value,
            b"String"
                | b"Symbol"
                | b"Array"
                | b"Hash"
                | b"Integer"
                | b"Float"
                | b"Rational"
                | b"Complex"
                | b"Object"
                | b"BasicObject"
                | b"Class"
                | b"Module"
                | b"Kernel"
                | b"Enumerable"
                | b"Enumerator"
                | b"TracePoint"
                | b"File"
                | b"IO"
                | b"Dir"
                | b"ARGF"
                | b"DATA"
        )
    })
}

fn reflective(name: &[u8]) -> bool {
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
    )
}

fn evaluation(name: &[u8]) -> bool {
    matches!(
        name,
        b"eval" | b"class_eval" | b"module_eval" | b"instance_eval"
    )
}

// Deliberately global rather than a receiver/alias analysis. These operations
// can change constants, lookup, visibility or code reached by an exempt getter.
// Acquiring the operation is already uncertainty, even if invoked later.
pub(super) fn mutation(name: &[u8]) -> bool {
    matches!(
        name,
        b"define_method"
            | b"define_singleton_method"
            | b"alias_method"
            | b"attr"
            | b"attr_reader"
            | b"attr_writer"
            | b"attr_accessor"
            | b"public"
            | b"private"
            | b"protected"
            | b"module_function"
            | b"public_class_method"
            | b"private_class_method"
            | b"public_constant"
            | b"private_constant"
            | b"remove_method"
            | b"undef_method"
            | b"include"
            | b"prepend"
            | b"extend"
            | b"append_features"
            | b"prepend_features"
            | b"extend_object"
            | b"refine"
            | b"using"
            | b"const_set"
            | b"remove_const"
            | b"autoload"
            | b"load"
            | b"require"
            | b"require_relative"
            | b"class_exec"
            | b"module_exec"
            | b"instance_exec"
    )
}

fn reflective_entry(name: &[u8]) -> bool {
    reflective(name)
        || matches!(
            name,
            b"to_proc" | b"method_missing" | b"respond_to_missing?"
        )
}

// Visibility/removal of an unrelated method cannot replace a reflective getter.
// Core constant/getter exemptions still reject ALL such mutations. Reflection's
// unrelated-selector proof only needs the reflective entry points themselves.
fn changes_reflection<'a, 'pr: 'a>(
    name: &[u8],
    mut args: impl Iterator<Item = &'a Node<'pr>>,
) -> bool {
    if matches!(
        name,
        b"const_set" | b"remove_const" | b"private_constant" | b"public_constant"
    ) {
        return false;
    }
    if matches!(
        name,
        b"public"
            | b"private"
            | b"protected"
            | b"public_class_method"
            | b"private_class_method"
            | b"remove_method"
            | b"undef_method"
    ) {
        return args.any(|arg| {
            let name = arg
                .as_def_node()
                .map(|d| d.name().as_slice().to_vec())
                .or_else(|| selector(arg));
            name.is_none_or(|s| reflective_entry(&s))
        });
    }
    true
}

fn evaluation_entry(name: &[u8]) -> bool {
    evaluation(name)
        || matches!(name, b"class_exec" | b"module_exec" | b"instance_exec")
        || reflective_entry(name)
}

// A core literal-block evaluator can only run its already walked body. Changing
// ordinary methods in that body invalidates metadata proofs, but not evaluator
// dispatch itself. Definitions/aliases/macros affecting evaluator or forwarding
// entries, and opaque mutation operations, end this separate proof.
fn changes_evaluation<'a, 'pr: 'a>(
    name: &[u8],
    mut args: impl Iterator<Item = &'a Node<'pr>>,
) -> bool {
    if matches!(
        name,
        b"define_method"
            | b"define_singleton_method"
            | b"alias_method"
            | b"attr"
            | b"attr_reader"
            | b"attr_writer"
            | b"attr_accessor"
            | b"public"
            | b"private"
            | b"protected"
            | b"public_class_method"
            | b"private_class_method"
            | b"remove_method"
            | b"undef_method"
    ) {
        return args.any(|arg| {
            let name = arg
                .as_def_node()
                .map(|d| d.name().as_slice().to_vec())
                .or_else(|| selector(arg));
            name.is_none_or(|s| evaluation_entry(&s))
        });
    }
    true
}

fn untyped_factory(name: &[u8]) -> bool {
    matches!(name, b"const_missing" | b"_id2ref" | b"each_object")
}

fn acquisition(name: &[u8]) -> bool {
    name == b"const_get" || evaluation(name) || untyped_factory(name) || reflective(name)
}

// No parser or Ruby execution: only a tiny decoded scalar grammar has a known
// non-code result. This preserves canonical EvalWithLocation fixes too. Even
// this exemption is conditional on the whole-main-AST dispatch evidence.
fn inert_evaluation(call: &ruby_prism::CallNode<'_>, code: Option<Node<'_>>) -> bool {
    if call.block().is_some()
        || !call.receiver().is_none_or(|r| {
            ast::is_global_const(
                &r,
                if call.name().as_slice() == b"eval" {
                    &["Kernel"]
                } else {
                    &["Object", "Module", "Class", "Kernel"]
                },
            )
        })
    {
        return false;
    }
    code.and_then(|c| c.as_string_node()).is_some_and(|s| {
        let code = s.unescaped().trim_ascii();
        matches!(code, b"nil" | b"true" | b"false" | b"0")
            || code.first().is_some_and(|b| matches!(b, b'1'..=b'9'))
                && code.iter().all(u8::is_ascii_digit)
    })
}

enum Lookup<'pr> {
    None,
    CoreDispatch,
    Direct(Option<Node<'pr>>),
    Acquired,
    Uncertain,
    Evaluation(Option<Node<'pr>>),
    BlockEvaluation,
    // Mutates (reflective dispatch, evaluator dispatch), separately from the
    // broader core metadata/constant dispatch proof.
    Mutation(bool, bool),
}

// Resolve only written selectors, consuming nested forwarding arguments once.
// Unknown reflective selectors can acquire const_get even through an aliased
// Object/VM receiver; they are not a non-code proof.
fn lookup<'pr>(call: &ruby_prism::CallNode<'pr>) -> Lookup<'pr> {
    let name = call.name().as_slice();
    if name == b"to_proc" {
        return match call.receiver().as_ref().and_then(selector) {
            Some(name) if acquisition(&name) => Lookup::Uncertain,
            Some(name) if mutation(&name) => Lookup::Mutation(true, true),
            _ => Lookup::CoreDispatch,
        };
    }
    let list = call.arguments();
    let args = list.as_ref().map(|l| l.arguments());
    let mut args = args.as_ref().into_iter().flat_map(|a| a.iter());
    let mut name = name.to_vec();
    let mut forwarded = false;
    loop {
        if name == b"const_get" {
            return Lookup::Direct(args.next());
        }
        if evaluation(&name) {
            // Only unchanged core dispatch limits evaluation to the walked
            // block. An override may ignore it and evaluate a string instead.
            // Kernel eval evaluates its string even when given a block.
            if name != b"eval" && call.block().is_some_and(|b| b.as_block_node().is_some()) {
                return Lookup::BlockEvaluation;
            }
            return Lookup::Evaluation(args.next());
        }
        if untyped_factory(&name) {
            return Lookup::Uncertain;
        }
        if name == b"alias_method" {
            let new = args.next();
            let old = args.next();
            return if old
                .as_ref()
                .and_then(selector)
                .is_none_or(|s| acquisition(&s))
            {
                Lookup::Uncertain
            } else {
                Lookup::Mutation(
                    true,
                    changes_evaluation(&name, new.as_ref().into_iter().chain(old.as_ref())),
                )
            };
        }
        if matches!(
            name.as_slice(),
            b"class_exec" | b"module_exec" | b"instance_exec"
        ) && call.block().is_some_and(|b| b.as_block_node().is_some())
        {
            return Lookup::BlockEvaluation;
        }
        if mutation(&name) {
            let args: Vec<_> = args.collect();
            return Lookup::Mutation(
                changes_reflection(&name, args.iter()),
                changes_evaluation(&name, args.iter()),
            );
        }
        let forwards = matches!(name.as_slice(), b"send" | b"__send__" | b"public_send");
        if !reflective(&name) {
            return if forwarded {
                Lookup::CoreDispatch
            } else {
                Lookup::None
            };
        }
        forwarded = true;
        let Some(arg) = args.next() else {
            return Lookup::CoreDispatch;
        };
        let Some(next) = selector(&arg) else {
            return if matches!(
                arg,
                Node::IntegerNode { .. }
                    | Node::FloatNode { .. }
                    | Node::NilNode { .. }
                    | Node::TrueNode { .. }
                    | Node::FalseNode { .. }
            ) {
                Lookup::CoreDispatch
            } else {
                Lookup::Uncertain
            };
        };
        if !forwards {
            return if next == b"const_get" {
                Lookup::Acquired
            } else if acquisition(&next) {
                Lookup::Uncertain
            } else if mutation(&next) {
                Lookup::Mutation(true, true)
            } else {
                Lookup::CoreDispatch
            };
        }
        name = next;
    }
}

fn factory_namespace(name: &[u8]) -> bool {
    matches!(
        name,
        b"RubyVM" | b"AbstractSyntaxTree" | b"InstructionSequence" | b"Marshal"
    )
}

struct Collector<'pr> {
    facts: Facts,
    ancestors: Vec<Node<'pr>>,
    core_exemption: bool,
    reflection_exemption: bool,
    block_evaluation_exemption: bool,
    evaluation_dispatch_changed: bool,
    // Actual metadata/constant exemptions also need unchanged namespaces.
    dispatch_changed: bool,
    // An unrelated selector need not resolve a core constant, but still must
    // invoke the original reflective entry point (not an override ignoring it).
    reflection_dispatch_changed: bool,
    namespace_changed: bool,
}

impl<'pr> walk::NodeVisitor<'pr> for Collector<'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        {
            self.facts.visits += 1;
        }
        let parent = self.ancestors.last();
        self.facts.heap.observe(&node, parent);
        // Bare compound writes read the old constant without a ConstantRead
        // child. Merely invalidating metadata exemptions misses that returned
        // factory. Qualified compound targets retain a ConstantPath child and
        // are handled by the maximal-path logic below. Plain assignments and
        // parallel targets do not implicitly acquire the old constant value.
        let implicit_read = match &node {
            Node::ConstantOrWriteNode { .. } => node.as_constant_or_write_node().map(|n| n.name()),
            Node::ConstantAndWriteNode { .. } => {
                node.as_constant_and_write_node().map(|n| n.name())
            }
            Node::ConstantOperatorWriteNode { .. } => {
                node.as_constant_operator_write_node().map(|n| n.name())
            }
            _ => None,
        };
        if let Some(name) = implicit_read {
            #[cfg(test)]
            {
                self.facts.constant_segments += 1;
            }
            self.facts.acquired_or_uncertain |= factory_namespace(name.as_slice());
        }
        // Skip namespace children BEFORE doing any path work. Inspect each
        // maximal written path iteratively, visiting its segments exactly once;
        // const_name recursively allocates all prefixes even for a single use.
        if matches!(
            node,
            Node::ConstantReadNode { .. } | Node::ConstantPathNode { .. }
        ) && !parent.is_some_and(|p| p.as_constant_path_node().is_some())
        {
            let mut parts = Vec::new();
            let mut current = node
                .as_constant_path_node()
                .map(|p| p.as_node())
                .or_else(|| node.as_constant_read_node().map(|r| r.as_node()));
            let mut rooted = true;
            while let Some(segment) = current {
                #[cfg(test)]
                {
                    self.facts.constant_segments += 1;
                }
                if let Some(read) = segment.as_constant_read_node() {
                    parts.push(read.name().as_slice());
                    current = None;
                } else if let Some(path) = segment.as_constant_path_node() {
                    if let Some(name) = path.name() {
                        parts.push(name.as_slice());
                    }
                    current = path.parent();
                } else {
                    rooted = false;
                    break;
                }
            }
            parts.reverse();
            let vm_or_serializer = parts.iter().any(|p| factory_namespace(p));
            if vm_or_serializer {
                let parts = parts
                    .strip_prefix(&[b"Object".as_slice()])
                    .unwrap_or(&parts);
                let core = rooted
                    && matches!(
                        parts,
                        [b"RubyVM"]
                            | [b"Marshal"]
                            | [b"Marshal", b"MAJOR_VERSION" | b"MINOR_VERSION"]
                            | [
                                b"RubyVM",
                                b"AbstractSyntaxTree"
                                    | b"InstructionSequence"
                                    | b"YJIT"
                                    | b"OPTS"
                                    | b"DEFAULT_PARAMS"
                            ]
                    );
                let defined = parent.is_some_and(|p| p.as_defined_node().is_some());
                let plain_data = matches!(
                    parts,
                    [b"RubyVM", b"OPTS" | b"DEFAULT_PARAMS"]
                        | [b"Marshal", b"MAJOR_VERSION" | b"MINOR_VERSION"]
                );
                // RubyVM.stat is not invariant metadata: even a regexp-read
                // rewrite can change its constant-cache counters.
                let metadata = parent.and_then(|p| p.as_call_node()).is_some_and(|call| {
                    is_receiver(&call, &node)
                        && call.block().is_none()
                        && call.arguments().is_none()
                        && (matches!(call.name().as_slice(), b"name" | b"class" | b"object_id")
                            || (parts == [b"RubyVM".as_slice(), b"YJIT".as_slice()]
                                && call.name().as_slice() == b"enabled?"))
                });
                // load/restore (including unknown, reflective and escaping
                // uses) may return a VM factory with no typed VM reference.
                // Only direct core dump is non-acquiring. This also retains
                // ordinary YAML/File corrections: a bare `load` spelling is
                // not itself a Marshal acquisition or a payload proof.
                let serialization_only = parts == [b"Marshal".as_slice()]
                    && parent.and_then(|p| p.as_call_node()).is_some_and(|call| {
                        is_receiver(&call, &node)
                            && call.name().as_slice() == b"dump"
                            && call.block().is_none()
                    });
                let exempt = core && (defined || plain_data || metadata || serialization_only);
                self.core_exemption |= exempt;
                self.facts.acquired_or_uncertain |= !exempt;
            }
        }
        self.evaluation_dispatch_changed |= node
            .as_def_node()
            .is_some_and(|def| evaluation_entry(def.name().as_slice()));
        self.evaluation_dispatch_changed |= node.as_undef_node().is_some_and(|undef| {
            undef
                .names()
                .iter()
                .any(|n| selector(&n).is_none_or(|name| evaluation_entry(&name)))
        }) || node.as_pre_execution_node().is_some();
        self.reflection_dispatch_changed |= node
            .as_def_node()
            .is_some_and(|def| reflective_entry(def.name().as_slice()));
        self.dispatch_changed |= node.as_def_node().is_some_and(|def| {
            let name = def.name().as_slice();
            acquisition(name)
                || mutation(name)
                || matches!(
                    name,
                    b"name"
                        | b"class"
                        | b"object_id"
                        | b"enabled?"
                        | b"call"
                        | b"to_proc"
                        | b"binding"
                        | b"method_missing"
                        | b"respond_to_missing?"
                        | b"method_added"
                        | b"singleton_method_added"
                        | b"const_added"
                        | b"inherited"
                )
        });
        self.dispatch_changed |= matches!(
            node,
            Node::AliasMethodNode { .. } | Node::UndefNode { .. } | Node::PreExecutionNode { .. }
        );
        self.reflection_dispatch_changed |= matches!(
            node,
            Node::AliasMethodNode { .. } | Node::UndefNode { .. } | Node::PreExecutionNode { .. }
        );
        self.namespace_changed |= matches!(
            node,
            Node::ClassNode { .. }
                | Node::ModuleNode { .. }
                | Node::SingletonClassNode { .. }
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
        if let Some(call) = node.as_call_node() {
            let uncertain = match lookup(&call) {
                Lookup::None => false,
                // Even an unrelated/invalid literal selector is only non-code
                // under core dispatch. A main-AST method/send/to_proc override
                // may ignore that selector and return/acquire a VM factory.
                Lookup::CoreDispatch => {
                    self.reflection_exemption = true;
                    false
                }
                Lookup::Uncertain => true,
                Lookup::Mutation(reflection, evaluation) => {
                    self.dispatch_changed = true;
                    self.reflection_dispatch_changed |= reflection;
                    self.evaluation_dispatch_changed |= evaluation;
                    false
                }
                Lookup::BlockEvaluation => {
                    self.block_evaluation_exemption = true;
                    self.dispatch_changed = true;
                    self.reflection_dispatch_changed = true;
                    false
                }
                Lookup::Evaluation(code) => {
                    // Forwarded/aliased evaluation has no core receiver proof.
                    let exempt =
                        evaluation(call.name().as_slice()) && inert_evaluation(&call, code);
                    self.core_exemption |= exempt;
                    !exempt
                }
                Lookup::Direct(name) => {
                    let exempt = call
                        .receiver()
                        .is_some_and(|r| ast::is_global_const(&r, &["Object"]))
                        && call.block().is_none()
                        && unrelated_constant(name);
                    self.core_exemption |= exempt;
                    !exempt
                }
                Lookup::Acquired => {
                    // A Method invoked immediately with a literal unrelated
                    // name is closed; saved/escaping getters are not followed.
                    let exempt = call
                        .receiver()
                        .is_some_and(|r| ast::is_global_const(&r, &["Object"]))
                        && call.block().is_none()
                        && parent.and_then(|p| p.as_call_node()).is_some_and(|p| {
                            p.name().as_slice() == b"call"
                                && p.block().is_none()
                                && is_receiver(&p, &node)
                                && unrelated_constant(ast::call_arguments(&p).into_iter().next())
                        });
                    self.core_exemption |= exempt;
                    !exempt
                }
            };
            self.facts.acquired_or_uncertain |= uncertain;
        }
        if let Some(alias) = node.as_alias_method_node() {
            self.evaluation_dispatch_changed |= [alias.new_name(), alias.old_name()]
                .iter()
                .any(|n| selector(n).is_none_or(|name| evaluation_entry(&name)));
            self.facts.acquired_or_uncertain |=
                selector(&alias.old_name()).is_none_or(|s| acquisition(&s));
        }
        if let Some(block) = node.as_block_argument_node()
            && let Some(name) = block.expression().as_ref().and_then(selector)
        {
            self.facts.acquired_or_uncertain |= acquisition(&name);
            self.dispatch_changed |= mutation(&name);
            self.reflection_dispatch_changed |= mutation(&name);
            self.evaluation_dispatch_changed |= mutation(&name);
            self.reflection_exemption = true; // Symbol#to_proc must also be core.
        }
        self.ancestors.push(node);
    }

    fn leave(&mut self) {
        self.ancestors.pop();
    }
}
