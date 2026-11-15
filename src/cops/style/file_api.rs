//! Fixed-depth AST helpers and cached binding evidence for standard-library API cops.
use super::correction::{preserves_line_values, touches_directive};
use crate::cops::walk::{NodeVisitor, walk};
use crate::{
    cops::{NodeContext, ast},
    offense::Correction,
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
use std::collections::HashSet;

/// Conservative evidence of nonstandard bindings. Cache one main-AST walk,
/// shared by all fixes, rather than rescanning every enclosing scope per match.
#[derive(Default)]
pub(crate) struct Bindings {
    file: bool,
    dir: bool,
    custom_dir_method: bool,
    pathname: bool,
    custom_pathname_method: bool,
    custom_require_relative: bool,
    custom_require: bool,
    custom_read_method: bool,
    custom_write_method: bool,
    yaml: bool,
    custom_yaml_method: bool,
    env: bool,
    custom_env_method: bool,
    time: bool,
    custom_time_method: bool,
    random: bool,
    custom_random_method: bool,
    dir_locals: HashSet<(usize, usize)>,
    // Temporary walk stack, empty once built. Only literal eval/exec blocks
    // propagate API self evidence; no strings are parsed or evaluated.
    api_scopes: Vec<ApiFrame>,
    #[cfg(test)]
    visited: usize,
}
struct ApiFrame {
    active: Option<ApiScope>,
    eval_block: Option<ApiScope>,
}
#[derive(Clone, Copy)]
struct ApiScope {
    start: usize,
    end: usize,
    api: &'static [u8],
}

impl Bindings {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut bindings = Self::default();
        walk(&mut bindings, root);
        bindings
    }
    fn dir_local(&self, cx: &NodeContext<'_, '_>) -> bool {
        cx.ancestors().any(|n| {
            self.dir_locals
                .contains(&(n.location().start_offset(), n.location().end_offset()))
        })
    }
    fn protect(&mut self, name: &[u8]) {
        self.env |= name == b"ENV";
        self.time |= matches!(name, b"Time" | b"Class");
        self.random |= matches!(name, b"Random" | b"Kernel" | b"Integer" | b"Range");
        self.file |= name == b"File";
        self.dir |= name == b"Dir";
        self.pathname |= name == b"Pathname";
        self.custom_pathname_method |= name == b"Kernel";
        self.custom_dir_method |= name == b"Kernel";
        self.custom_require_relative |= name == b"Kernel";
        self.custom_require |= name == b"Kernel";
        self.custom_read_method |= matches!(name, b"IO" | b"Symbol");
        self.custom_write_method |= matches!(name, b"IO" | b"Symbol");
        self.yaml |= matches!(name, b"YAML" | b"Psych" | b"NULL" | b"Encoding");
    }
}
impl<'pr> NodeVisitor<'pr> for Bindings {
    fn enter(&mut self, n: Node<'pr>) {
        #[cfg(test)]
        {
            self.visited += 1;
        }
        let parent = self.api_scopes.last();
        // Eval receivers and arguments run with the enclosing self. Defer the
        // new API self until entering this call's block, not the call itself.
        let scope = parent
            .and_then(|p| p.eval_block)
            .filter(|s| {
                n.as_block_node().is_some()
                    && n.location().start_offset() == s.start
                    && n.location().end_offset() == s.end
            })
            .or_else(|| parent.and_then(|p| p.active));
        self.api_scopes.push(ApiFrame {
            active: scope,
            eval_block: n.as_call_node().and_then(|c| api_eval_scope(&c)),
        });
        if let Some(scope) = scope {
            let methods = api_methods(scope.api);
            let changes_api = if let Some(call) = n.as_call_node() {
                api_self(call.receiver(), scope.api) && call_binds_api(&call, scope.api)
            } else if let Some(method) = n.as_def_node() {
                api_self(method.receiver(), scope.api)
                    && methods.contains(&method.name().as_slice())
            } else if let Some(alias) = n.as_alias_method_node() {
                may_name_method(&alias.new_name(), methods)
            } else if let Some(undef) = n.as_undef_node() {
                undef
                    .names()
                    .iter()
                    .any(|name| may_name_method(&name, methods))
            } else {
                false
            };
            if changes_api {
                self.protect(scope.api);
            }
        }
        // Locals are parser-owned binding lists, not a second variable-flow
        // analysis: each list is inspected exactly once, including parameters.
        let locals = match n {
            Node::ProgramNode { .. } => Some(n.as_program_node().unwrap().locals()),
            Node::DefNode { .. } => Some(n.as_def_node().unwrap().locals()),
            Node::BlockNode { .. } => Some(n.as_block_node().unwrap().locals()),
            Node::LambdaNode { .. } => Some(n.as_lambda_node().unwrap().locals()),
            Node::ClassNode { .. } => Some(n.as_class_node().unwrap().locals()),
            Node::ModuleNode { .. } => Some(n.as_module_node().unwrap().locals()),
            Node::SingletonClassNode { .. } => Some(n.as_singleton_class_node().unwrap().locals()),
            _ => None,
        };
        if locals.is_some_and(|list| list.iter().any(|name| name.as_slice() == b"__dir__")) {
            self.dir_locals
                .insert((n.location().start_offset(), n.location().end_offset()));
        }
        macro_rules! writes {
            ($($access:ident),*) => { $(if let Some(write) = n.$access() { self.protect(write.name().as_slice()); })* };
        }
        writes!(
            as_constant_write_node,
            as_constant_or_write_node,
            as_constant_and_write_node,
            as_constant_operator_write_node,
            as_constant_target_node
        );
        if let Some(target) = n.as_constant_path_target_node() {
            self.protect(target.name_loc().as_slice());
        }
        if let Some(method) = n.as_def_node() {
            self.custom_dir_method |= method.name().as_slice() == b"__dir__";
            self.custom_pathname_method |= method.name().as_slice() == b"Pathname";
            self.custom_require_relative |= method.name().as_slice() == b"require_relative";
            self.custom_require |= method.name().as_slice() == b"require";
            self.custom_read_method |= READ_METHODS.contains(&method.name().as_slice());
            self.custom_write_method |= WRITE_METHODS.contains(&method.name().as_slice());
            self.custom_yaml_method |= YAML_METHODS.contains(&method.name().as_slice());
            self.custom_env_method |= ENV_METHODS.contains(&method.name().as_slice());
            self.custom_time_method |= TIME_METHODS.contains(&method.name().as_slice());
            self.custom_random_method |= RANDOM_METHODS.contains(&method.name().as_slice());
        }
        if let Some(call) = n.as_call_node() {
            // Like a class reopening, a visible refinement of an API (or its
            // singleton class) can replace its methods. Do not infer whether
            // `using` activates it at a particular call site.
            if let Some(api) = api_refinement_target(&call) {
                self.protect(api);
            }
            self.custom_dir_method |= binds_method(&call, &[b"__dir__"]);
            self.custom_pathname_method |= binds_method(&call, &[b"Pathname"]);
            self.custom_require_relative |= binds_method(&call, &[b"require_relative"]);
            self.custom_require |= binds_method(&call, &[b"require"]);
            self.custom_read_method |= binds_method(&call, READ_METHODS);
            self.custom_write_method |= binds_method(&call, WRITE_METHODS);
            self.custom_yaml_method |= binds_method(&call, YAML_METHODS);
            self.custom_env_method |= binds_method(&call, ENV_METHODS);
            self.custom_time_method |= binds_method(&call, TIME_METHODS);
            self.custom_random_method |= binds_method(&call, RANDOM_METHODS);
            for name in [
                b"File".as_slice(),
                b"Dir",
                b"Pathname",
                b"Kernel",
                b"IO",
                b"Symbol",
                b"YAML",
                b"Psych",
                b"NULL",
                b"Encoding",
                b"ENV",
                b"Time",
                b"Class",
                b"Random",
                b"Integer",
                b"Range",
            ] {
                if binds_constant(&call, name) {
                    self.protect(name);
                }
            }
            let api = call
                .receiver()
                .as_ref()
                .and_then(|r| api_receiver(r, scope.map(|s| s.api)))
                .or_else(|| {
                    call.receiver()
                        .is_none()
                        .then(|| scope.map(|s| s.api))
                        .flatten()
                });
            if let Some(api) = api
                && (call_binds_api(&call, api) || opaque_api_eval(&call))
            {
                self.protect(api);
            }
        }
        if let Some(undef) = n.as_undef_node() {
            self.custom_dir_method |= undef.names().iter().any(|name| may_name_dir(&name));
            self.custom_pathname_method |= undef
                .names()
                .iter()
                .any(|n| may_name_method(&n, &[b"Pathname"]));
            self.custom_require |= undef
                .names()
                .iter()
                .any(|n| may_name_method(&n, &[b"require"]));
            self.custom_require_relative |= undef
                .names()
                .iter()
                .any(|n| may_name_method(&n, &[b"require_relative"]));
            self.custom_read_method |= undef
                .names()
                .iter()
                .any(|name| may_name_method(&name, READ_METHODS));
            self.custom_write_method |= undef
                .names()
                .iter()
                .any(|n| may_name_method(&n, WRITE_METHODS));
            self.custom_env_method |= undef
                .names()
                .iter()
                .any(|n| may_name_method(&n, ENV_METHODS));
            self.custom_time_method |= undef
                .names()
                .iter()
                .any(|n| may_name_method(&n, TIME_METHODS));
            self.custom_random_method |= undef
                .names()
                .iter()
                .any(|n| may_name_method(&n, RANDOM_METHODS));
            self.custom_yaml_method |= undef
                .names()
                .iter()
                .any(|n| may_name_method(&n, YAML_METHODS));
        }
        if let Some(alias) = n.as_alias_method_node() {
            // Literal aliases include identifier, symbol and quoted spellings.
            // A dynamic alias name may also introduce this method.
            self.custom_pathname_method |= may_name_method(&alias.new_name(), &[b"Pathname"]);
            self.custom_require_relative |=
                may_name_method(&alias.new_name(), &[b"require_relative"]);
            self.custom_require |= may_name_method(&alias.new_name(), &[b"require"]);
            self.custom_read_method |= may_name_method(&alias.new_name(), READ_METHODS);
            self.custom_write_method |= may_name_method(&alias.new_name(), WRITE_METHODS);
            self.custom_yaml_method |= may_name_method(&alias.new_name(), YAML_METHODS);
            self.custom_env_method |= may_name_method(&alias.new_name(), ENV_METHODS);
            self.custom_time_method |= may_name_method(&alias.new_name(), TIME_METHODS);
            self.custom_random_method |= may_name_method(&alias.new_name(), RANDOM_METHODS);
            self.custom_dir_method |= alias
                .new_name()
                .as_symbol_node()
                .is_none_or(|name| name.unescaped() == b"__dir__");
        }
        let target = match n {
            Node::ConstantPathWriteNode { .. } => {
                Some(n.as_constant_path_write_node().unwrap().target().as_node())
            }
            Node::ConstantPathOrWriteNode { .. } => Some(
                n.as_constant_path_or_write_node()
                    .unwrap()
                    .target()
                    .as_node(),
            ),
            Node::ConstantPathAndWriteNode { .. } => Some(
                n.as_constant_path_and_write_node()
                    .unwrap()
                    .target()
                    .as_node(),
            ),
            Node::ConstantPathOperatorWriteNode { .. } => Some(
                n.as_constant_path_operator_write_node()
                    .unwrap()
                    .target()
                    .as_node(),
            ),
            Node::ClassNode { .. } => Some(n.as_class_node().unwrap().constant_path()),
            Node::ModuleNode { .. } => Some(n.as_module_node().unwrap().constant_path()),
            Node::SingletonClassNode { .. } => {
                Some(n.as_singleton_class_node().unwrap().expression())
            }
            Node::DefNode { .. } => n.as_def_node().unwrap().receiver(),
            _ => None,
        };
        if let Some(name) = target.as_ref().and_then(ast::short_const_name) {
            self.protect(name.as_bytes());
        }
    }

    fn leave(&mut self) {
        self.api_scopes.pop();
    }
}

// Class#new invokes initialize; Time.now bypasses that hook. Keep visible
// instance initialization/allocator changes even without a class reopening.
const TIME_METHODS: &[&[u8]] = &[b"new", b"now", b"initialize", b"allocate"];
const RANDOM_METHODS: &[&[u8]] = &[
    b"rand", b"+", b"-", b"succ", b"pred", b"next", b"<=>", b"to_int",
];
const ENV_METHODS: &[&[u8]] = &[b"[]", b"fetch"];
const READ_METHODS: &[&[u8]] = &[b"open", b"read", b"binread", b"to_proc", b"close"];
const WRITE_METHODS: &[&[u8]] = &[b"open", b"write", b"binwrite", b"to_proc", b"close"];
const YAML_METHODS: &[&[u8]] = &[
    b"load",
    b"safe_load",
    b"parse",
    b"load_file",
    b"safe_load_file",
    b"parse_file",
    b"read",
    b"open",
    b"close",
];

pub(super) fn binds_constant(call: &CallNode<'_>, name: &[u8]) -> bool {
    let Some(arguments) = call.arguments() else {
        return false;
    };
    let mut args = arguments.arguments().iter();
    let binds = |method: &[u8], target: Option<Node<'_>>| {
        matches!(
            method,
            b"const_set" | b"remove_const" | b"autoload" | b"private_constant"
        ) && target.is_some_and(|n| may_name_method(&n, &[name]))
    };
    if matches!(
        call.name().as_slice(),
        b"send" | b"__send__" | b"public_send"
    ) {
        let Some(target) = args.next() else {
            return false;
        };
        if let Some(s) = target.as_symbol_node() {
            binds(s.unescaped(), args.next())
        } else if let Some(s) = target.as_string_node() {
            binds(s.unescaped(), args.next())
        } else {
            false
        }
    } else {
        binds(call.name().as_slice(), args.next())
    }
}

pub(super) fn binds_method(call: &CallNode<'_>, methods: &[&[u8]]) -> bool {
    let Some(arguments) = call.arguments() else {
        return false;
    };
    let mut args = arguments.arguments().iter();
    // The receiver may be self, a constant, or an object later used for
    // instance_exec. As for explicit defs, conservatively protect the whole
    // file, without attempting receiver/value-flow or execution-order inference.
    if matches!(
        call.name().as_slice(),
        b"send" | b"__send__" | b"public_send"
    ) {
        let Some(target) = args.next() else {
            return false;
        };
        // Only one explicit literal forwarding hop, not arbitrary dispatch.
        if let Some(name) = target.as_symbol_node() {
            binding_arguments(name.unescaped(), args, call.receiver(), methods)
        } else if let Some(name) = target.as_string_node() {
            binding_arguments(name.unescaped(), args, call.receiver(), methods)
        } else {
            false
        }
    } else {
        binding_arguments(call.name().as_slice(), args, call.receiver(), methods)
    }
}

fn binding_arguments<'pr>(
    name: &[u8],
    mut args: impl Iterator<Item = Node<'pr>>,
    receiver: Option<Node<'pr>>,
    methods: &[&[u8]],
) -> bool {
    match name {
        b"alias_method" | b"define_method" | b"define_singleton_method" => args
            .next()
            .is_some_and(|name| may_name_method(&name, methods)),
        // Reader names occur at every argument position. attr's historical
        // boolean flag affects only the writer, not the bare method lookup.
        b"attr" => args.any(|name| {
            name.as_true_node().is_none()
                && name.as_false_node().is_none()
                && may_name_method(&name, methods)
        }),
        b"attr_reader" | b"attr_accessor" | b"undef_method" | b"remove_method" => {
            args.any(|name| may_name_method(&name, methods))
        }
        b"new" | b"define" => {
            let receiver = receiver.as_ref().and_then(ast::short_const_name);
            let is_struct = name == b"new" && receiver.as_deref() == Some("Struct");
            let is_data = name == b"define" && receiver.as_deref() == Some("Data");
            if !is_struct && !is_data {
                return false;
            }
            let mut args = args.peekable();
            // Struct's optional leading string/nil names the class, not a
            // reader. Its keyword_init option likewise introduces no member.
            if is_struct {
                args.next_if(|n| n.as_string_node().is_some() || n.as_nil_node().is_some());
            }
            args.any(|n| n.as_keyword_hash_node().is_none() && may_name_method(&n, methods))
        }
        // attr_writer only defines __dir__=, not the bare __dir__ lookup.
        _ => false,
    }
}

fn api_self(receiver: Option<Node<'_>>, api: &'static [u8]) -> bool {
    receiver.is_none_or(|n| api_receiver(&n, Some(api)) == Some(api))
}

fn api_methods(api: &[u8]) -> &'static [&'static [u8]] {
    match api {
        b"File" => &[
            b"dirname",
            b"realpath",
            b"expand_path",
            b"open",
            b"read",
            b"binread",
            b"write",
            b"binwrite",
            b"close",
        ],
        b"Dir" => &[b"children", b"entries", b"each_child", b"empty?"],
        b"Pathname" => &[b"new", b"parent", b"expand_path"],
        b"Kernel" => &[
            b"__dir__",
            b"Pathname",
            b"require_relative",
            b"require",
            b"rand",
        ],
        b"Time" | b"Class" => TIME_METHODS,
        b"Random" | b"Integer" | b"Range" => RANDOM_METHODS,
        b"IO" | b"Symbol" => &[
            b"open",
            b"read",
            b"binread",
            b"write",
            b"binwrite",
            b"to_proc",
            b"close",
        ],
        b"ENV" => ENV_METHODS,
        b"YAML" | b"Psych" => YAML_METHODS,
        b"Encoding" => &[b"default_external=", b"default_internal="],
        _ => unreachable!("only literal standard-library APIs are indexed"),
    }
}

/// Fixed-depth literal receiver evidence, without value or execution-order
/// inference. Namespaced constants remain conservative as for reopenings.
fn literal_api(node: &Node<'_>) -> Option<&'static [u8]> {
    api_receiver(node, None)
}

fn api_receiver(node: &Node<'_>, self_api: Option<&'static [u8]>) -> Option<&'static [u8]> {
    if node.as_self_node().is_some() {
        return self_api;
    }
    if let Some(paren) = node.as_parentheses_node() {
        let body = paren.body()?;
        let statements = body.as_statements_node()?;
        let mut nodes = statements.body().iter();
        let child = nodes.next()?;
        return nodes
            .next()
            .is_none()
            .then(|| api_receiver(&child, self_api))
            .flatten();
    }
    if let Some(call) = node.as_call_node() {
        return (calls_literal_method(&call, &[b"singleton_class"], true)
            && call.block().is_none())
        .then(|| match call.receiver() {
            Some(receiver) => api_receiver(&receiver, self_api),
            None => self_api,
        })
        .flatten();
    }
    match ast::short_const_name(node)?.as_str() {
        "File" => Some(b"File"),
        "Dir" => Some(b"Dir"),
        "Pathname" => Some(b"Pathname"),
        "Kernel" => Some(b"Kernel"),
        "IO" => Some(b"IO"),
        "Symbol" => Some(b"Symbol"),
        "YAML" => Some(b"YAML"),
        "Psych" => Some(b"Psych"),
        "Encoding" => Some(b"Encoding"),
        "ENV" => Some(b"ENV"),
        "Time" => Some(b"Time"),
        "Class" => Some(b"Class"),
        "Random" => Some(b"Random"),
        "Integer" => Some(b"Integer"),
        "Range" => Some(b"Range"),
        _ => None,
    }
}

fn calls_literal_method(call: &CallNode<'_>, methods: &[&[u8]], no_args: bool) -> bool {
    if matches!(
        call.name().as_slice(),
        b"send" | b"__send__" | b"public_send"
    ) {
        return call.arguments().is_some_and(|a| {
            let mut args = a.arguments().iter();
            args.next().is_some_and(|name| {
                // Unlike a binding's unknown destination, an unknown forwarded
                // API name is not a recognized DSL invocation.
                let known = name.as_symbol_node().is_some() || name.as_string_node().is_some();
                known && may_name_method(&name, methods) && (!no_args || args.next().is_none())
            })
        });
    }
    methods.contains(&call.name().as_slice()) && (!no_args || no_arguments(call))
}

// String eval is explicit API mutation evidence, even when its text is dynamic.
// Never parse or execute Ruby strings; the one cached main-AST walk suffices.
fn opaque_api_eval(call: &CallNode<'_>) -> bool {
    calls_literal_method(
        call,
        &[b"instance_eval", b"class_eval", b"module_eval"],
        false,
    ) && call.block().is_none_or(|b| b.as_block_node().is_none())
        && call.arguments().is_some_and(|a| !a.arguments().is_empty())
}

fn api_refinement_target(call: &CallNode<'_>) -> Option<&'static [u8]> {
    if !calls_literal_method(call, &[b"refine"], false) {
        return None;
    }
    let forwarded = matches!(
        call.name().as_slice(),
        b"send" | b"__send__" | b"public_send"
    );
    let target = call
        .arguments()?
        .arguments()
        .iter()
        .nth(usize::from(forwarded))?;
    literal_api(&target)
}

fn api_eval_scope(call: &CallNode<'_>) -> Option<ApiScope> {
    if !calls_literal_method(
        call,
        &[
            b"instance_eval",
            b"instance_exec",
            b"class_eval",
            b"class_exec",
            b"module_eval",
            b"module_exec",
        ],
        false,
    ) {
        return None;
    }
    let api = call.receiver().as_ref().and_then(literal_api)?;
    let block = call.block()?.as_block_node()?;
    Some(ApiScope {
        start: block.location().start_offset(),
        end: block.location().end_offset(),
        api,
    })
}

fn call_binds_api(call: &CallNode<'_>, api: &[u8]) -> bool {
    if api == b"Encoding" && calls_literal_method(call, api_methods(api), false) {
        return true;
    }
    let Some(arguments) = call.arguments() else {
        return false;
    };
    let mut args = arguments.arguments().iter();
    let methods = api_methods(api);
    if matches!(
        call.name().as_slice(),
        b"send" | b"__send__" | b"public_send"
    ) {
        // One explicit literal forwarding hop, not arbitrary runtime dispatch.
        let Some(target) = args.next() else {
            return false;
        };
        if let Some(name) = target.as_symbol_node() {
            api_binding_arguments(name.unescaped(), args, methods)
        } else if let Some(name) = target.as_string_node() {
            api_binding_arguments(name.unescaped(), args, methods)
        } else {
            false
        }
    } else {
        api_binding_arguments(call.name().as_slice(), args, methods)
    }
}

fn api_binding_arguments<'pr>(
    name: &[u8],
    mut args: impl Iterator<Item = Node<'pr>>,
    methods: &[&[u8]],
) -> bool {
    match name {
        b"alias_method" | b"define_method" | b"define_singleton_method" => {
            // The alias source does not change its own binding.
            args.next().is_some_and(|n| may_name_method(&n, methods))
        }
        b"attr" => args.any(|n| {
            n.as_true_node().is_none()
                && n.as_false_node().is_none()
                && may_name_method(&n, methods)
        }),
        b"attr_reader"
        | b"attr_accessor"
        | b"undef_method"
        | b"remove_method"
        | b"private_class_method"
        | b"public_class_method"
        | b"private"
        | b"public"
        | b"protected" => args.any(|n| may_name_method(&n, methods)),
        // Writers introduce name=, not any of the APIs used in these edits.
        _ => false,
    }
}

pub(super) fn may_name_method(node: &Node<'_>, methods: &[&[u8]]) -> bool {
    // Unknown names are potential bindings, not values to evaluate or infer.
    if let Some(name) = node.as_symbol_node() {
        methods.contains(&name.unescaped())
    } else if let Some(name) = node.as_string_node() {
        methods.contains(&name.unescaped())
    } else {
        true
    }
}

fn may_name_dir(node: &Node<'_>) -> bool {
    may_name_method(node, &[b"__dir__"])
}

/// A regular send, not a safe-navigation or a block expression. Literal
/// blocks are separate enclosing nodes in RuboCop's AST, unlike Prism's.
pub(super) fn send<'pr>(node: &Node<'pr>) -> Option<CallNode<'pr>> {
    let call = node.as_call_node()?;
    (!call.is_safe_navigation() && call.block().is_none_or(|b| b.as_block_node().is_none()))
        .then_some(call)
}

pub(super) fn argument<'pr>(call: &CallNode<'pr>) -> Option<Node<'pr>> {
    let mut args = ast::call_arguments(call);
    (args.len() == 1).then(|| args.remove(0))
}

pub(super) fn no_arguments(call: &CallNode<'_>) -> bool {
    ast::call_arguments(call).is_empty()
}

pub(super) fn integer(node: &Node<'_>, expected: u32) -> bool {
    node.as_integer_node().is_some_and(|n| {
        let value = n.value();
        let (negative, digits) = value.to_u32_digits();
        (!negative || expected == 0)
            && (digits == [expected] || (expected == 0 && digits.is_empty()))
    })
}

/// Keep offenses when a canonical whole-expression rewrite would lose source
/// information. All queries reuse the main parse and cached source indexes.
pub(super) fn replacement(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    text: String,
    api: &str,
) -> Option<Correction> {
    replacement_inner(node, cx, file, text, api, false)
}

pub(super) fn read_replacement(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    text: String,
) -> Option<Correction> {
    replacement_inner(node, cx, file, text, "FileRead", true)
}

pub(super) fn write_replacement(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    text: String,
) -> Option<Correction> {
    replacement_inner(node, cx, file, text, "FileWrite", true)
}

pub(super) fn yaml_replacement(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    text: String,
) -> Option<Correction> {
    replacement_inner(node, cx, file, text, "YAMLFileRead", false)
}

fn replacement_inner(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    text: String,
    api: &str,
    allow_block: bool,
) -> Option<Correction> {
    let range = node.location().start_offset()..if allow_block {
        node.location().end_offset()
    } else {
        ast::source_end(node)
    };
    if !allow_block && node.as_call_node().is_some_and(|n| n.block().is_some()) {
        return None;
    }
    let comments = file.comments();
    let first = comments.partition_point(|c| c.end <= range.start);
    if comments.get(first).is_some_and(|c| c.start < range.end) {
        return None;
    }
    let root = cx.ancestors().next_back().unwrap_or(node);
    let bindings = file.file_api_bindings(root);
    if (api == "File" && (bindings.file || bindings.custom_dir_method || bindings.dir_local(cx)))
        || (api == "FilePath" && bindings.file)
        || (api == "Pathname"
            && (bindings.pathname
                || bindings.custom_pathname_method
                || bindings.custom_dir_method
                || bindings.dir_local(cx)))
        || (api == "RequireRelative" && bindings.custom_require_relative)
        || (api == "Require" && bindings.custom_require)
        || (api == "FileRead" && (bindings.file || bindings.custom_read_method))
        || (api == "FileWrite" && (bindings.file || bindings.custom_write_method))
        || (api == "YAMLFileRead"
            && (bindings.file
                || bindings.yaml
                || bindings.custom_read_method
                || bindings.custom_yaml_method))
        || (api == "Env" && (bindings.env || bindings.custom_env_method))
        || (api == "Time" && (bindings.time || bindings.custom_time_method))
        || (api == "Random" && (bindings.random || bindings.custom_random_method))
        || (api == "Dir" && bindings.dir)
        || file.contains_heredoc(root, node)
    {
        return None;
    }
    let correction = Correction {
        range,
        replacement: text,
    };
    let changes_lines = memchr::memchr_iter(b'\n', &file.bytes[correction.range.clone()]).count()
        != memchr::memchr_iter(b'\n', correction.replacement.as_bytes()).count();
    (preserves_line_values(file, &correction)
        && (!changes_lines || !touches_directive(file, correction.range.clone())))
    .then_some(correction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cops::{
        Cop, Registry,
        policy::RubyVersion,
        select_active_for_version,
        style::{Dir, DirEmpty, FileEmpty},
        testing::correction_offenses,
    };

    #[test]
    fn literal_api_refinements_protect_only_the_target_api() {
        for target in [
            "File",
            "::File.singleton_class",
            "File.send(:singleton_class)",
            "(File.singleton_class)",
        ] {
            for invocation in ["refine", "send(:refine,", "__send__('refine',"] {
                let call = if invocation == "refine" {
                    format!("refine {target}")
                } else {
                    format!("{invocation} {target})")
                };
                let source =
                    format!("module Paths; {call} do; def dirname(path); path; end; end; end");
                let file = SourceFile::new("x.rb".into(), source.into_bytes());
                let parsed = ruby_prism::parse(&file.bytes);
                assert!(parsed.errors().next().is_none());
                let bindings = Bindings::build(&parsed.node());
                assert!(bindings.file, "{target}: {invocation}");
                assert!(!bindings.pathname);
                assert!(!bindings.dir);
            }
        }
        for (target, expected) in [("Pathname", true), ("Other.singleton_class", false)] {
            let source =
                format!("module Paths; refine {target} do; def parent; self; end; end; end");
            let parsed = ruby_prism::parse(source.as_bytes());
            let bindings = Bindings::build(&parsed.node());
            assert_eq!(bindings.pathname, expected, "{source}");
            assert!(!bindings.file);
        }
    }

    #[test]
    fn time_and_random_share_one_bounded_binding_walk() {
        let source = "Time.new()\nRandom.rand(6).succ\n".repeat(2000);
        let file = SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        let root = parsed.node();
        let first = file.file_api_bindings(&root);
        for _ in 0..4000 {
            assert!(std::ptr::eq(first, file.file_api_bindings(&root)));
        }
        assert_eq!(
            first.visited,
            crate::cops::testing::preorder_kinds(&source).len()
        );
        assert!(first.visited < 40_000);
        for cop in [
            &super::super::TimeNow as &dyn Cop,
            &super::super::RandomWithOffset,
        ] {
            let found = correction_offenses(cop, &source);
            assert_eq!(found.len(), 2000);
            assert!(found.iter().all(|o| o.correction.is_some()));
        }
    }

    #[test]
    fn binding_index_visits_each_node_once_and_reuses_the_main_parse() {
        let source =
            "File.expand_path(File.dirname(__FILE__))\nDir.children(path).empty?\n".repeat(2000);
        let file = SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        let root = parsed.node();
        let first = file.file_api_bindings(&root);
        let second = file.file_api_bindings(&root);
        assert!(std::ptr::eq(first, second));
        let expected = crate::cops::testing::preorder_kinds(&source).len();
        assert_eq!(first.visited, expected);
        assert!(first.visited < 40_000);
        assert_eq!(correction_offenses(&Dir, &source).len(), 2000);
        assert_eq!(correction_offenses(&DirEmpty, &source).len(), 2000);
    }

    #[test]
    fn literal_api_eval_blocks_and_parentheses_keep_visible_mutation_evidence() {
        for binding in [
            "(File).define_singleton_method(:dirname) { 123 }",
            "File.send(:singleton_class).define_method(:dirname) { 123 }",
            "(::File).instance_eval { define_singleton_method(:dirname) { 123 } }",
            "File.instance_exec { self.define_singleton_method(:dirname) { 123 } }",
            "File.instance_eval { self.singleton_class.define_method(:dirname) { 123 } }",
            "File.instance_eval { singleton_class.alias_method(:dirname, :basename) }",
            "File.singleton_class.class_eval { alias_method :dirname, :basename }",
            "File.singleton_class.module_exec { attr_reader :dirname }",
            "File.instance_eval { def dirname(path); 123; end }",
            "File.instance_eval { alias dirname basename }",
            "File.singleton_class.class_eval { undef dirname }",
            "File.define_singleton_method(name) { 123 }",
            "File.singleton_class.alias_method(\"dir\\x6eame\", :basename)",
        ] {
            let source = format!("{binding}\nFile.dirname(File.realpath(__FILE__))\n");
            let found = correction_offenses(&Dir, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn literal_api_binding_evidence_is_cached_in_one_bounded_walk() {
        let source = format!(
            "File.define_singleton_method(:dirname) {{ 123 }}\nDir.singleton_class.alias_method(:empty?, :exist?)\n{}",
            "File.dirname(File.realpath(__FILE__))\nDir.children('.').empty?\n".repeat(2000)
        );
        let file = SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        let root = parsed.node();
        let first = file.file_api_bindings(&root);
        assert!(first.file && first.dir);
        assert!(first.api_scopes.is_empty());
        for _ in 0..4000 {
            assert!(std::ptr::eq(first, file.file_api_bindings(&root)));
        }
        assert_eq!(
            first.visited,
            crate::cops::testing::preorder_kinds(&source).len()
        );
        assert!(first.visited < 40_000);
        for cop in [&Dir as &dyn Cop, &DirEmpty] {
            let found = correction_offenses(cop, &source);
            assert_eq!(found.len(), 2000);
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }

    #[test]
    fn read_bindings_share_one_bounded_index_for_many_offenses() {
        use crate::cops::style::FileRead;
        let source = format!(
            "File.singleton_class.class_eval {{ private :read }}\n{}",
            "File.open('file').read\n".repeat(2000)
        );
        let file = SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        let root = parsed.node();
        let index = file.file_api_bindings(&root);
        assert!(index.file);
        for _ in 0..2000 {
            assert!(std::ptr::eq(index, file.file_api_bindings(&root)));
        }
        assert_eq!(
            index.visited,
            crate::cops::testing::preorder_kinds(&source).len()
        );
        assert!(index.visited < 25_000);
        let found = correction_offenses(&FileRead, &source);
        assert_eq!(found.len(), 2000);
        assert!(found.iter().all(|o| o.correction.is_none()));
    }

    #[test]
    fn write_and_yaml_matches_reuse_one_bounded_binding_index() {
        use crate::cops::style::{FileWrite, YAMLFileRead};
        let source = format!(
            "File.define_singleton_method(:binwrite) {{ 123 }}\nYAML.class_eval(code)\n{}",
            "File.open('file', 'wb') { |f| f.write('data') }\nYAML.load(File.read(File::NULL))\n"
                .repeat(2000)
        );
        let file = SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        let root = parsed.node();
        let index = file.file_api_bindings(&root);
        for _ in 0..4000 {
            assert!(std::ptr::eq(index, file.file_api_bindings(&root)));
        }
        assert_eq!(
            index.visited,
            crate::cops::testing::preorder_kinds(&source).len()
        );
        assert!(index.visited < 50_000);
        for cop in [&FileWrite as &dyn Cop, &YAMLFileRead] {
            let found = correction_offenses(cop, &source);
            assert_eq!(found.len(), 2000);
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }

    #[test]
    fn env_corrections_share_one_bounded_main_ast_binding_index() {
        use crate::cops::style::FetchEnvVar;
        let source = format!(
            "ENV.singleton_class.class_eval {{ alias fetch object_id }}\n{}",
            "ENV['X']\n".repeat(4000)
        );
        let file = SourceFile::new("many.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        let root = parsed.node();
        let index = file.file_api_bindings(&root);
        assert!(index.env && index.custom_env_method);
        assert!(index.api_scopes.is_empty());
        assert_eq!(
            index.visited,
            crate::cops::testing::preorder_kinds(&source).len()
        );
        assert!(index.visited < 20_100);
        for _ in 0..4000 {
            assert!(std::ptr::eq(index, file.file_api_bindings(&root)));
        }
        let found = correction_offenses(&FetchEnvVar, &source);
        assert_eq!(found.len(), 4000);
        assert!(found.iter().all(|o| o.correction.is_none()));
    }
    #[test]
    fn extension_corrections_share_the_existing_bounded_binding_index() {
        let source = format!(
            "Kernel.module_eval {{ alias require puts }}\n{}",
            "require 'feature.rb'\nrequire_relative 'feature.rb'\n".repeat(2000)
        );
        let file = SourceFile::new("many.rb".into(), source.into_bytes());
        let parsed = ruby_prism::parse(&file.bytes);
        let root = parsed.node();
        let index = file.file_api_bindings(&root);
        assert!(index.custom_require && index.custom_require_relative);
        assert!(index.visited < 20000);
        for _ in 0..4000 {
            assert!(std::ptr::eq(index, file.file_api_bindings(&root)));
        }
    }
    #[test]
    fn path_cops_share_one_bounded_cached_binding_walk() {
        use crate::cops::style::{
            ExpandPathArguments, NestedFileDirname, RedundantCurrentDirectoryInPath,
        };
        let source = format!(
            "Pathname.singleton_class.class_eval {{ alias new allocate }}\nKernel.module_eval {{ attr_reader :require_relative }}\n{}",
            "File.dirname(File.dirname(path))\nFile.expand_path('..', __FILE__)\nPathname(__FILE__).parent.expand_path\nrequire_relative './feature'\n".repeat(2000)
        );
        let file = SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        let root = parsed.node();
        let index = file.file_api_bindings(&root);
        assert!(index.pathname && index.custom_require_relative);
        for _ in 0..8000 {
            assert!(std::ptr::eq(index, file.file_api_bindings(&root)));
        }
        assert_eq!(
            index.visited,
            crate::cops::testing::preorder_kinds(&source).len()
        );
        assert!(index.visited < 80_000);
        for (cop, count) in [
            (&NestedFileDirname as &dyn Cop, 2000),
            (&ExpandPathArguments, 4000),
            (&RedundantCurrentDirectoryInPath, 2000),
        ] {
            assert_eq!(correction_offenses(cop, &source).len(), count);
        }
    }

    #[test]
    fn fixed_ruby_34_activates_real_matchers_and_older_versions_do_not() {
        let registry = Registry::new();
        let config = crate::config::ResolvedConfig::default();
        for (cop, min) in [
            (&Dir as &dyn Cop, RubyVersion(2, 0)),
            (&DirEmpty, RubyVersion(2, 4)),
            (&FileEmpty, RubyVersion(2, 4)),
            (&crate::cops::style::NestedFileDirname, RubyVersion(3, 1)),
        ] {
            assert_eq!(cop.ruby_versions().minimum, Some(min));
            assert!(
                select_active_for_version(
                    &registry,
                    &config,
                    &[cop.name().into()],
                    &[],
                    RubyVersion(3, 4)
                )
                .iter()
                .any(|c| c.name() == cop.name())
            );
            assert!(
                select_active_for_version(
                    &registry,
                    &config,
                    &[cop.name().into()],
                    &[],
                    if min.1 == 0 {
                        RubyVersion(min.0 - 1, 9)
                    } else {
                        RubyVersion(min.0, min.1 - 1)
                    }
                )
                .is_empty()
            );
        }
    }
}
