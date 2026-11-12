//! Shared port of RuboCop's `MethodComplexity` mixin with its
//! `IteratingBlock` and `RepeatedCsendDiscount` utilities, for
//! `Metrics/CyclomaticComplexity`, `Metrics/PerceivedComplexity` and
//! `Metrics/AbcSize` (RuboCop 1.92.0, MIT, Copyright (c) 2012-26 Bozhidar
//! Batsov).
//!
//! A *method* is every `def`/`def self.x` and every `define_method` literal
//! block (`block`, `numblock` or `itblock`) whose call has no receiver and a
//! sole `sym`/`str` name argument (pinned matcher
//! `(any_block (send nil? :define_method ({sym str} $_)) _ _)`). A method
//! without a body is accepted. Otherwise its body is measured and, above the
//! fixed limit, the whole definition is reported. The pinned defaults allow
//! no method name or pattern.
//!
//! Upstream measures RuboCop's parser AST: the complexity cops through
//! `each_node` (pre-order), the ABC calculator through a post-order walk. The
//! repeated `&.` discount depends on that order, as do lvasgn resets, so
//! [`events`] walks the Prism body in the parser's shape and child order and
//! reports the parser nodes these metrics distinguish. Its `Visit`
//! implementation reorders children, which the complete [`crate::cops::walk`]
//! cannot, and only walks the measured subtree, as upstream does.

use ruby_prism::{
    BlockNode, CallNode, CaseMatchNode, CaseNode, InNode, Location, Node, UnlessNode, Visit,
};
use std::collections::HashSet;

use crate::{cops::Cop, offense::Offense, source::SourceFile};

/// A method definition measured by the complexity cops.
pub(super) struct Method<'pr> {
    /// Upstream's `method_name` (`def`) or captured name (`define_method`).
    pub name: String,
    /// The non-empty parser body.
    pub body: Node<'pr>,
    /// The reported range: the whole `def` or block (with its send).
    pub location: Location<'pr>,
}

impl<'pr> Method<'pr> {
    /// The method `node` defines, if it is one and has a body.
    pub fn of(node: &Node<'pr>) -> Option<Self> {
        if let Some(def) = node.as_def_node() {
            return Some(Self {
                name: lossy(def.name().as_slice()),
                body: def.body()?,
                location: node.location(),
            });
        }
        let call = node.as_call_node()?;
        if call.name().as_slice() != b"define_method" || call.receiver().is_some() {
            return None;
        }
        let block = call.block()?.as_block_node()?;
        let arguments = call.arguments()?.arguments();
        if arguments.len() != 1 {
            return None;
        }
        let name = literal_name(&arguments.iter().next()?)?;
        Some(Self {
            name,
            body: block.body()?,
            location: node.location(),
        })
    }

    /// Report the whole method with `message` (no correction).
    pub fn report(
        &self,
        cop: &dyn Cop,
        message: String,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let (line, col) = file.line_col(self.location.start_offset());
        let end = self
            .location
            .end_offset()
            .saturating_sub(1)
            .max(self.location.start_offset());
        out.push(Offense {
            cop: cop.name(),
            severity: cop.severity(),
            line,
            col,
            end_line: file.line_of(end),
            message,
            correction: None,
        });
    }
}

/// The value of a `define_method` name argument when the parser builds it
/// as a `sym` or `str` node: Prism's translation splits a literal spanning
/// several source lines (except at backslash continuations of interpolating
/// literals) into a `dstr`/`dsym` with one part per line, and an empty
/// `%()`, `:""` or heredoc has no part at all. An interpolation always
/// makes a `dstr`/`dsym`.
fn literal_name(node: &Node<'_>) -> Option<String> {
    if let Some(symbol) = node.as_symbol_node() {
        if symbol.closing_loc().is_none() {
            return Some(lossy(symbol.unescaped()));
        }
        let opening = symbol.opening_loc().map_or(&b""[..], |l| l.as_slice());
        let value = symbol.value_loc()?.as_slice();
        return (!value.is_empty() && one_part(value, opening)).then(|| lossy(symbol.unescaped()));
    }
    if let Some(heredoc) = node.as_interpolated_string_node() {
        // Prism splits a heredoc at a line continuation; the translation
        // glues its plain parts back into lines.
        let opening = heredoc.opening_loc().map_or(&b""[..], |l| l.as_slice());
        if !opening.starts_with(b"<<") {
            return None;
        }
        let (mut content, mut name) = (Vec::new(), Vec::new());
        for part in &heredoc.parts() {
            let part = part.as_string_node()?;
            content.extend_from_slice(part.content_loc().as_slice());
            name.extend_from_slice(part.unescaped());
        }
        return (!content.is_empty() && one_part(&content, opening)).then(|| lossy(&name));
    }
    let string = node.as_string_node()?;
    let opening = string.opening_loc().map_or(&b""[..], |l| l.as_slice());
    let content = string.content_loc().as_slice();
    let single = if opening.starts_with(b"<<") {
        !content.is_empty() && one_part(content, opening)
    } else if opening == b"?" {
        true
    } else if opening.starts_with(b"%") && string.unescaped().is_empty() {
        false
    } else {
        one_part(content, opening)
    };
    single.then(|| lossy(string.unescaped()))
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Whether a literal's source `content` is one parser string part
/// (Prism's `string_nodes_from_line_continuations`).
fn one_part(content: &[u8], opening: &[u8]) -> bool {
    let lines: Vec<&[u8]> = content.split_inclusive(|&b| b == b'\n').collect();
    if lines.len() <= 1 {
        return true;
    }
    let raw = opening.ends_with(b"'")
        || [&b"%q"[..], b"%s", b"%w", b"%i"]
            .iter()
            .any(|p| opening.starts_with(p));
    if raw {
        return false;
    }
    // Lines ending in an odd number of backslashes continue the part.
    lines[..lines.len() - 1].iter().all(|line| {
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        line.iter().rev().take_while(|&&b| b == b'\\').count() % 2 == 1
    })
}

/// `IteratingBlock::KNOWN_ITERATING_METHODS`.
const ITERATING_METHODS: &[&str] = &[
    // Enumerable
    "all?",
    "any?",
    "chain",
    "chunk",
    "chunk_while",
    "collect",
    "collect_concat",
    "count",
    "cycle",
    "detect",
    "drop",
    "drop_while",
    "each",
    "each_cons",
    "each_entry",
    "each_slice",
    "each_with_index",
    "each_with_object",
    "entries",
    "filter",
    "filter_map",
    "find",
    "find_all",
    "find_index",
    "flat_map",
    "grep",
    "grep_v",
    "group_by",
    "inject",
    "lazy",
    "map",
    "max",
    "max_by",
    "min",
    "min_by",
    "minmax",
    "minmax_by",
    "none?",
    "one?",
    "partition",
    "reduce",
    "reject",
    "reverse_each",
    "select",
    "slice_after",
    "slice_before",
    "slice_when",
    "sort",
    "sort_by",
    "sum",
    "take",
    "take_while",
    "tally",
    "to_h",
    "uniq",
    "zip",
    // Enumerator
    "with_index",
    "with_object",
    // Array
    "bsearch",
    "bsearch_index",
    "collect!",
    "combination",
    "d_permutation",
    "delete_if",
    "each_index",
    "keep_if",
    "map!",
    "permutation",
    "product",
    "reject!",
    "repeat",
    "repeated_combination",
    "select!",
    "sort!",
    // Hash
    "each_key",
    "each_pair",
    "each_value",
    "fetch",
    "fetch_values",
    "has_key?",
    "merge",
    "merge!",
    "transform_keys",
    "transform_keys!",
    "transform_values",
    "transform_values!",
];

/// Whether a block or block-pass on a call to `name` iterates.
fn iterating(name: &[u8]) -> bool {
    ITERATING_METHODS.iter().any(|m| m.as_bytes() == name)
}

/// `COMPARISON_OPERATORS` of rubocop-ast (`<=>` is not one).
fn comparison(name: &[u8]) -> bool {
    matches!(name, b"==" | b"===" | b"!=" | b"<=" | b">=" | b">" | b"<")
}

/// A parser `send`/`csend` node.
#[derive(Clone, Copy)]
pub(super) struct Send<'pr> {
    /// `csend` (`&.`).
    pub csend: bool,
    /// The receiver's name when it is a local variable (`lvar`).
    pub lvar_receiver: Option<&'pr [u8]>,
    /// A comparison method (`==`, `!=`, `<`, ...).
    pub comparison: bool,
    /// `setter_method?`: written with an `=` operator (`a.b = 1`, `a[i] = 1`).
    pub setter: bool,
}

/// Which compound assignment an [`Event::Compound`] is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Compound {
    /// `masgn`.
    Multiple,
    /// `op_asgn` (`+=`, `-=`, ...).
    Operator,
    /// `or_asgn` (`||=`).
    Or,
    /// `and_asgn` (`&&=`).
    And,
}

/// A parser node these metrics distinguish.
#[derive(Clone, Copy)]
pub(super) enum Event<'a, 'pr> {
    /// `if` (also `unless`, `elsif`, modifiers and ternaries, not `in` guards).
    If {
        /// `else?`: an `else` or `elsif` clause (a ternary has neither).
        else_clause: bool,
        /// `elsif?`: this node is an `elsif` clause.
        elsif: bool,
        /// The `else` clause is the keyword `else` (not `elsif`).
        else_keyword: bool,
    },
    /// `case` with `when` clauses.
    Case(&'a CaseNode<'pr>),
    /// `case` with `in` clauses.
    CaseMatch(&'a CaseMatchNode<'pr>),
    /// `while`/`until` (not the `begin ... end while` forms).
    Loop,
    /// `for` (its index assignments are separate events).
    For,
    /// `send`/`csend`.
    Send(Send<'pr>),
    /// `yield`.
    Yield,
    /// A `block` or `block_pass` on a known iterating method (other blocks,
    /// `numblock` and `itblock` never count).
    IteratingBlock,
    /// A `rescue` construct, however many clauses it has.
    Rescue,
    /// `when`.
    When,
    /// `in_pattern`.
    In,
    /// `and` (`&&`, `and`).
    And,
    /// `or` (`||`, `or`).
    Or,
    /// `masgn`/`op_asgn`/`or_asgn`/`and_asgn`, with how many of its parser
    /// children (`masgn`: left-hand assignments) are method dispatches that
    /// are not setters, which the ABC calculator adds as assignments.
    Compound {
        /// The assignment kind.
        kind: Compound,
        /// Dispatch children that are not setters.
        dispatches: usize,
    },
    /// `lvasgn`, with the variable name.
    Lvasgn(&'pr [u8]),
    /// `ivasgn`, `cvasgn`, `gvasgn` or `casgn`.
    Asgn,
    /// An argument (`arg`, `optarg`, `restarg`, `kwarg`, `kwoptarg`,
    /// `kwrestarg`, `blockarg`, `shadowarg`) with a name not starting with
    /// `_` (an anonymous `*`, `**`, `&` or `...` captures nothing).
    CapturingArgument,
}

/// The order of [`events`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Order {
    /// Parents before children (`each_node`).
    Pre,
    /// Children before parents (`visit_depth_last`).
    Post,
}

/// Every parser node of `root`'s parser subtree (`root` included) that
/// these metrics distinguish, in parser `order`.
pub(super) fn events<'pr>(root: &Node<'pr>, order: Order, sink: impl FnMut(Event<'_, 'pr>)) {
    let mut walker = Walker { order, sink };
    walker.visit(root);
}

/// `RepeatedCsendDiscount`: a `csend` on a local variable that already had
/// one since its last assignment is discounted.
#[derive(Default)]
pub(super) struct RepeatedCsend {
    seen: HashSet<Vec<u8>>,
}

impl RepeatedCsend {
    /// `discount_for_repeated_csend?` (recording the first `csend`).
    pub fn discount(&mut self, send: &Send<'_>) -> bool {
        send.lvar_receiver
            .is_some_and(|name| !self.seen.insert(name.to_vec()))
    }

    /// `reset_on_lvasgn`.
    pub fn reset(&mut self, name: &[u8]) {
        self.seen.remove(name);
    }
}

/// Calls the sink with parser-shaped events (see [`events`]).
struct Walker<F> {
    order: Order,
    sink: F,
}

impl<'pr, F: FnMut(Event<'_, 'pr>)> Walker<F> {
    fn pre(&mut self, event: Event<'_, 'pr>) {
        if self.order == Order::Pre {
            (self.sink)(event);
        }
    }

    fn post(&mut self, event: Event<'_, 'pr>) {
        if self.order == Order::Post {
            (self.sink)(event);
        }
    }

    /// `event` around `children`, in walk order.
    fn around(&mut self, event: Event<'_, 'pr>, children: impl FnOnce(&mut Self)) {
        self.pre(event);
        children(self);
        self.post(event);
    }

    /// A shorthand assignment `(kind target value)`; the target's own event
    /// (none for `casgn` paths, which carry their scope) is `target`.
    fn shorthand(
        &mut self,
        kind: Compound,
        target: impl FnOnce(&mut Self),
        target_dispatch: bool,
        value: &Node<'pr>,
    ) {
        let event = Event::Compound {
            kind,
            dispatches: usize::from(target_dispatch) + usize::from(dispatch_value(value)),
        };
        self.pre(event);
        target(self);
        self.visit(value);
        self.post(event);
    }

    /// A pattern: captures are `match_var`, not `lvasgn`; values, pins and
    /// constants inside it are ordinary expressions.
    fn visit_pattern(&mut self, node: &Node<'pr>) {
        if node.as_local_variable_target_node().is_some() {
        } else if let Some(n) = node.as_array_pattern_node() {
            if let Some(c) = n.constant() {
                self.visit(&c);
            }
            n.requireds().iter().for_each(|p| self.visit_pattern(&p));
            if let Some(rest) = n.rest() {
                self.visit_pattern(&rest);
            }
            n.posts().iter().for_each(|p| self.visit_pattern(&p));
        } else if let Some(n) = node.as_find_pattern_node() {
            if let Some(c) = n.constant() {
                self.visit(&c);
            }
            self.visit_pattern(&n.left().as_node());
            n.requireds().iter().for_each(|p| self.visit_pattern(&p));
            self.visit_pattern(&n.right());
        } else if let Some(n) = node.as_hash_pattern_node() {
            if let Some(c) = n.constant() {
                self.visit(&c);
            }
            n.elements().iter().for_each(|p| self.visit_pattern(&p));
            if let Some(rest) = n.rest() {
                self.visit_pattern(&rest);
            }
        } else if let Some(n) = node.as_assoc_node() {
            self.visit(&n.key());
            self.visit_pattern(&n.value());
        } else if let Some(n) = node.as_assoc_splat_node() {
            if let Some(value) = n.value() {
                self.visit_pattern(&value);
            }
        } else if let Some(n) = node.as_splat_node() {
            if let Some(expression) = n.expression() {
                self.visit_pattern(&expression);
            }
        } else if let Some(n) = node.as_implicit_node() {
            self.visit_pattern(&n.value());
        } else if let Some(n) = node.as_capture_pattern_node() {
            self.visit_pattern(&n.value());
        } else if let Some(n) = node.as_alternation_pattern_node() {
            self.visit_pattern(&n.left());
            self.visit_pattern(&n.right());
        } else if let Some(n) = node.as_parentheses_node() {
            if let Some(body) = n.body() {
                self.visit_pattern(&body);
            }
        } else if let Some(n) = node.as_statements_node() {
            n.body().iter().for_each(|p| self.visit_pattern(&p));
        } else {
            self.visit(node);
        }
    }

    /// The parser `(block (send ...) args body)` around a call's block:
    /// the block node precedes its send in pre-order and follows its body
    /// in post-order, while the send's own post-order position precedes
    /// the block arguments.
    fn call(&mut self, call: &CallNode<'pr>, send: Option<Send<'pr>>) {
        let block = call.block();
        let literal = block.as_ref().and_then(Node::as_block_node);
        let counted = literal
            .as_ref()
            .is_some_and(|b| plain_block(b) && iterating(call.name().as_slice()));
        if counted {
            self.pre(Event::IteratingBlock);
        }
        if let Some(send) = send {
            self.pre(Event::Send(send));
        }
        if let Some(receiver) = call.receiver() {
            self.visit(&receiver);
        }
        if let Some(arguments) = call.arguments() {
            self.visit_arguments_node(&arguments);
        }
        if let Some(pass) = block.as_ref().and_then(Node::as_block_argument_node) {
            let event = iterating(call.name().as_slice()).then_some(Event::IteratingBlock);
            if let Some(event) = event {
                self.pre(event);
            }
            if let Some(expression) = pass.expression() {
                self.visit(&expression);
            }
            if let Some(event) = event {
                self.post(event);
            }
        }
        if let Some(send) = send {
            self.post(Event::Send(send));
        }
        if let Some(literal) = literal {
            ruby_prism::visit_block_node(self, &literal);
        }
        if counted {
            self.post(Event::IteratingBlock);
        }
    }
}

/// A parser `block` (not `numblock` or `itblock`).
fn plain_block(block: &BlockNode<'_>) -> bool {
    block.parameters().is_none_or(|p| {
        p.as_numbered_parameters_node().is_none() && p.as_it_parameters_node().is_none()
    })
}

/// The name of a local variable read (`lvar`, including `it` and `_1`).
fn lvar_name<'pr>(node: &Node<'pr>) -> Option<&'pr [u8]> {
    if let Some(read) = node.as_local_variable_read_node() {
        Some(read.name().as_slice())
    } else if node.as_it_local_variable_read_node().is_some() {
        Some(b"it")
    } else {
        None
    }
}

/// The parser `send`/`csend` of a Prism call.
fn send_of<'pr>(call: &CallNode<'pr>) -> Send<'pr> {
    Send {
        csend: call.is_safe_navigation(),
        lvar_receiver: call.receiver().as_ref().and_then(lvar_name),
        comparison: comparison(call.name().as_slice()),
        setter: call.equal_loc().is_some(),
    }
}

/// The target `(send receiver :name)` of an operator, `||=` or `&&=` call
/// or index assignment.
fn read_send<'pr>(receiver: Option<Node<'pr>>, csend: bool, name: &[u8]) -> Send<'pr> {
    Send {
        csend,
        lvar_receiver: receiver.as_ref().and_then(lvar_name),
        comparison: comparison(name),
        setter: false,
    }
}

/// Whether a shorthand assignment's value is a parser method dispatch
/// (`send`, `csend`, `super`, `zsuper`, `yield`, `defined?`) that is not a
/// setter. A call or `super` with a literal block is a `block` node.
fn dispatch_value(value: &Node<'_>) -> bool {
    if let Some(call) = value.as_call_node() {
        call.equal_loc().is_none() && !call.block().is_some_and(|b| b.as_block_node().is_some())
    } else if let Some(sup) = value.as_super_node() {
        !sup.block().is_some_and(|b| b.as_block_node().is_some())
    } else if let Some(sup) = value.as_forwarding_super_node() {
        sup.block().is_none()
    } else {
        value.as_yield_node().is_some() || value.as_defined_node().is_some()
    }
}

/// How many left-hand assignments of a multiple assignment (nested
/// `mlhs` flattened, a splat's target) are attribute or index targets:
/// method dispatches without a setter operator.
fn dispatch_targets<'pr>(targets: impl Iterator<Item = Node<'pr>>) -> usize {
    targets
        .map(|target| {
            if let Some(nested) = target.as_multi_target_node() {
                dispatch_targets(
                    nested
                        .lefts()
                        .iter()
                        .chain(nested.rest())
                        .chain(nested.rights().iter()),
                )
            } else if let Some(splat) = target.as_splat_node() {
                splat
                    .expression()
                    .map_or(0, |e| dispatch_targets(std::iter::once(e)))
            } else {
                usize::from(
                    target.as_call_target_node().is_some()
                        || target.as_index_target_node().is_some(),
                )
            }
        })
        .sum()
}

/// The [`Event::If`] of an `if`/`elsif`/ternary/modifier node.
fn if_event<'a, 'pr>(node: &ruby_prism::IfNode<'pr>) -> Event<'a, 'pr> {
    let keyword = node.if_keyword_loc().map(|l| l.as_slice());
    let subsequent = node.subsequent();
    Event::If {
        // A ternary's `:` is not an `else` (parser's ternary map).
        else_clause: keyword.is_some() && subsequent.is_some(),
        elsif: keyword == Some(b"elsif"),
        else_keyword: keyword.is_some() && subsequent.is_some_and(|s| s.as_else_node().is_some()),
    }
}

/// The [`Event::If`] of an `unless` node.
fn unless_event<'a, 'pr>(node: &UnlessNode<'pr>) -> Event<'a, 'pr> {
    let else_clause = node.else_clause().is_some();
    Event::If {
        else_clause,
        elsif: false,
        else_keyword: else_clause,
    }
}

/// The pattern of an `in` clause and its guard, if any (Prism wraps a
/// guarded pattern in an `if`/`unless` node).
pub(super) fn in_pattern<'pr>(node: &InNode<'pr>) -> (Node<'pr>, Option<Node<'pr>>) {
    let pattern = node.pattern();
    let guard = pattern
        .as_if_node()
        .map(|g| (g.predicate(), g.statements()))
        .or_else(|| {
            pattern
                .as_unless_node()
                .map(|g| (g.predicate(), g.statements()))
        });
    match guard {
        Some((predicate, Some(statements))) => match statements.body().iter().next() {
            Some(inner) => (inner, Some(predicate)),
            None => (pattern, None),
        },
        _ => (pattern, None),
    }
}

impl<'pr, F: FnMut(Event<'_, 'pr>)> Visit<'pr> for Walker<F> {
    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'pr>) {
        self.around(if_event(node), |w| ruby_prism::visit_if_node(w, node));
    }

    fn visit_unless_node(&mut self, node: &UnlessNode<'pr>) {
        // Parser `(if cond else_body then_body)`.
        self.around(unless_event(node), |w| {
            w.visit(&node.predicate());
            if let Some(else_clause) = node.else_clause() {
                w.visit_else_node(&else_clause);
            }
            if let Some(statements) = node.statements() {
                w.visit_statements_node(&statements);
            }
        });
    }

    fn visit_case_node(&mut self, node: &CaseNode<'pr>) {
        self.around(Event::Case(node), |w| ruby_prism::visit_case_node(w, node));
    }

    fn visit_case_match_node(&mut self, node: &CaseMatchNode<'pr>) {
        self.around(Event::CaseMatch(node), |w| {
            ruby_prism::visit_case_match_node(w, node);
        });
    }

    fn visit_when_node(&mut self, node: &ruby_prism::WhenNode<'pr>) {
        self.around(Event::When, |w| ruby_prism::visit_when_node(w, node));
    }

    fn visit_in_node(&mut self, node: &InNode<'pr>) {
        // Parser `(in_pattern pattern guard body)`; the guard is not an `if`.
        self.around(Event::In, |w| {
            let (pattern, guard) = in_pattern(node);
            w.visit_pattern(&pattern);
            if let Some(guard) = guard {
                w.visit(&guard);
            }
            if let Some(statements) = node.statements() {
                w.visit_statements_node(&statements);
            }
        });
    }

    fn visit_match_predicate_node(&mut self, node: &ruby_prism::MatchPredicateNode<'pr>) {
        self.visit(&node.value());
        self.visit_pattern(&node.pattern());
    }

    fn visit_match_required_node(&mut self, node: &ruby_prism::MatchRequiredNode<'pr>) {
        self.visit(&node.value());
        self.visit_pattern(&node.pattern());
    }

    fn visit_while_node(&mut self, node: &ruby_prism::WhileNode<'pr>) {
        if node.is_begin_modifier() {
            ruby_prism::visit_while_node(self, node);
        } else {
            self.around(Event::Loop, |w| ruby_prism::visit_while_node(w, node));
        }
    }

    fn visit_until_node(&mut self, node: &ruby_prism::UntilNode<'pr>) {
        if node.is_begin_modifier() {
            ruby_prism::visit_until_node(self, node);
        } else {
            self.around(Event::Loop, |w| ruby_prism::visit_until_node(w, node));
        }
    }

    fn visit_for_node(&mut self, node: &ruby_prism::ForNode<'pr>) {
        self.around(Event::For, |w| ruby_prism::visit_for_node(w, node));
    }

    fn visit_begin_node(&mut self, node: &ruby_prism::BeginNode<'pr>) {
        if node.rescue_clause().is_some() {
            self.around(Event::Rescue, |w| ruby_prism::visit_begin_node(w, node));
        } else {
            ruby_prism::visit_begin_node(self, node);
        }
    }

    fn visit_rescue_modifier_node(&mut self, node: &ruby_prism::RescueModifierNode<'pr>) {
        self.around(Event::Rescue, |w| {
            ruby_prism::visit_rescue_modifier_node(w, node);
        });
    }

    fn visit_and_node(&mut self, node: &ruby_prism::AndNode<'pr>) {
        self.around(Event::And, |w| ruby_prism::visit_and_node(w, node));
    }

    fn visit_or_node(&mut self, node: &ruby_prism::OrNode<'pr>) {
        self.around(Event::Or, |w| ruby_prism::visit_or_node(w, node));
    }

    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        // Prism's parser translation folds `- 1` into a numeric literal and
        // builds `/re/ =~ x` as `match_with_lvasgn`, neither a send.
        let receiver = node.receiver();
        let operator = node.call_operator_loc().is_none();
        let name = node.name().as_slice();
        let literal_minus = name == b"-@"
            && receiver.as_ref().is_some_and(|r| {
                matches!(
                    r,
                    Node::IntegerNode { .. }
                        | Node::FloatNode { .. }
                        | Node::RationalNode { .. }
                        | Node::ImaginaryNode { .. }
                )
            });
        let match_op = name == b"=~"
            && receiver
                .as_ref()
                .is_some_and(|r| r.as_regular_expression_node().is_some());
        let send = (!(operator && (literal_minus || match_op))).then(|| send_of(node));
        self.call(node, send);
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        // `(block (send nil :lambda) args body)`; never an iterating block.
        let send = Send {
            csend: false,
            lvar_receiver: None,
            comparison: false,
            setter: false,
        };
        self.around(Event::Send(send), |_| {});
        ruby_prism::visit_lambda_node(self, node);
    }

    fn visit_match_write_node(&mut self, node: &ruby_prism::MatchWriteNode<'pr>) {
        // `(match_with_lvasgn regexp value)`: no send, no lvasgn.
        let call = node.call();
        if let Some(receiver) = call.receiver() {
            self.visit(&receiver);
        }
        if let Some(arguments) = call.arguments() {
            self.visit_arguments_node(&arguments);
        }
    }

    fn visit_yield_node(&mut self, node: &ruby_prism::YieldNode<'pr>) {
        self.around(Event::Yield, |w| ruby_prism::visit_yield_node(w, node));
    }

    fn visit_call_target_node(&mut self, node: &ruby_prism::CallTargetNode<'pr>) {
        let send = read_send(
            Some(node.receiver()),
            node.is_safe_navigation(),
            node.name().as_slice(),
        );
        self.around(Event::Send(send), |w| {
            ruby_prism::visit_call_target_node(w, node);
        });
    }

    fn visit_index_target_node(&mut self, node: &ruby_prism::IndexTargetNode<'pr>) {
        let send = read_send(Some(node.receiver()), node.is_safe_navigation(), b"[]=");
        self.around(Event::Send(send), |w| {
            ruby_prism::visit_index_target_node(w, node);
        });
    }

    fn visit_multi_write_node(&mut self, node: &ruby_prism::MultiWriteNode<'pr>) {
        let dispatches = dispatch_targets(
            node.lefts()
                .iter()
                .chain(node.rest())
                .chain(node.rights().iter()),
        );
        let event = Event::Compound {
            kind: Compound::Multiple,
            dispatches,
        };
        self.around(event, |w| ruby_prism::visit_multi_write_node(w, node));
    }

    // Local variables.

    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        let event = Event::Lvasgn(node.name().as_slice());
        self.around(event, |w| {
            ruby_prism::visit_local_variable_write_node(w, node)
        });
    }

    fn visit_local_variable_target_node(
        &mut self,
        node: &ruby_prism::LocalVariableTargetNode<'pr>,
    ) {
        self.around(Event::Lvasgn(node.name().as_slice()), |_| {});
    }

    fn visit_local_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOperatorWriteNode<'pr>,
    ) {
        let name = node.name().as_slice();
        self.shorthand(
            Compound::Operator,
            |w| w.around(Event::Lvasgn(name), |_| {}),
            false,
            &node.value(),
        );
    }

    fn visit_local_variable_or_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOrWriteNode<'pr>,
    ) {
        let name = node.name().as_slice();
        self.shorthand(
            Compound::Or,
            |w| w.around(Event::Lvasgn(name), |_| {}),
            false,
            &node.value(),
        );
    }

    fn visit_local_variable_and_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableAndWriteNode<'pr>,
    ) {
        let name = node.name().as_slice();
        self.shorthand(
            Compound::And,
            |w| w.around(Event::Lvasgn(name), |_| {}),
            false,
            &node.value(),
        );
    }

    // Instance, class and global variables and constants.

    fn visit_instance_variable_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableWriteNode<'pr>,
    ) {
        self.around(Event::Asgn, |w| {
            ruby_prism::visit_instance_variable_write_node(w, node)
        });
    }

    fn visit_instance_variable_target_node(
        &mut self,
        _node: &ruby_prism::InstanceVariableTargetNode<'pr>,
    ) {
        self.around(Event::Asgn, |_| {});
    }

    fn visit_instance_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableOperatorWriteNode<'pr>,
    ) {
        self.shorthand(
            Compound::Operator,
            |w| w.around(Event::Asgn, |_| {}),
            false,
            &node.value(),
        );
    }

    fn visit_instance_variable_or_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableOrWriteNode<'pr>,
    ) {
        self.shorthand(
            Compound::Or,
            |w| w.around(Event::Asgn, |_| {}),
            false,
            &node.value(),
        );
    }

    fn visit_instance_variable_and_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableAndWriteNode<'pr>,
    ) {
        self.shorthand(
            Compound::And,
            |w| w.around(Event::Asgn, |_| {}),
            false,
            &node.value(),
        );
    }

    fn visit_class_variable_write_node(&mut self, node: &ruby_prism::ClassVariableWriteNode<'pr>) {
        self.around(Event::Asgn, |w| {
            ruby_prism::visit_class_variable_write_node(w, node)
        });
    }

    fn visit_class_variable_target_node(
        &mut self,
        _node: &ruby_prism::ClassVariableTargetNode<'pr>,
    ) {
        self.around(Event::Asgn, |_| {});
    }

    fn visit_class_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableOperatorWriteNode<'pr>,
    ) {
        self.shorthand(
            Compound::Operator,
            |w| w.around(Event::Asgn, |_| {}),
            false,
            &node.value(),
        );
    }

    fn visit_class_variable_or_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableOrWriteNode<'pr>,
    ) {
        self.shorthand(
            Compound::Or,
            |w| w.around(Event::Asgn, |_| {}),
            false,
            &node.value(),
        );
    }

    fn visit_class_variable_and_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableAndWriteNode<'pr>,
    ) {
        self.shorthand(
            Compound::And,
            |w| w.around(Event::Asgn, |_| {}),
            false,
            &node.value(),
        );
    }

    fn visit_global_variable_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableWriteNode<'pr>,
    ) {
        self.around(Event::Asgn, |w| {
            ruby_prism::visit_global_variable_write_node(w, node)
        });
    }

    fn visit_global_variable_target_node(
        &mut self,
        _node: &ruby_prism::GlobalVariableTargetNode<'pr>,
    ) {
        self.around(Event::Asgn, |_| {});
    }

    fn visit_global_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableOperatorWriteNode<'pr>,
    ) {
        self.shorthand(
            Compound::Operator,
            |w| w.around(Event::Asgn, |_| {}),
            false,
            &node.value(),
        );
    }

    fn visit_global_variable_or_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableOrWriteNode<'pr>,
    ) {
        self.shorthand(
            Compound::Or,
            |w| w.around(Event::Asgn, |_| {}),
            false,
            &node.value(),
        );
    }

    fn visit_global_variable_and_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableAndWriteNode<'pr>,
    ) {
        self.shorthand(
            Compound::And,
            |w| w.around(Event::Asgn, |_| {}),
            false,
            &node.value(),
        );
    }

    fn visit_constant_write_node(&mut self, node: &ruby_prism::ConstantWriteNode<'pr>) {
        self.around(Event::Asgn, |w| {
            ruby_prism::visit_constant_write_node(w, node)
        });
    }

    fn visit_constant_target_node(&mut self, _node: &ruby_prism::ConstantTargetNode<'pr>) {
        self.around(Event::Asgn, |_| {});
    }

    fn visit_constant_operator_write_node(
        &mut self,
        node: &ruby_prism::ConstantOperatorWriteNode<'pr>,
    ) {
        self.shorthand(
            Compound::Operator,
            |w| w.around(Event::Asgn, |_| {}),
            false,
            &node.value(),
        );
    }

    fn visit_constant_or_write_node(&mut self, node: &ruby_prism::ConstantOrWriteNode<'pr>) {
        self.shorthand(
            Compound::Or,
            |w| w.around(Event::Asgn, |_| {}),
            false,
            &node.value(),
        );
    }

    fn visit_constant_and_write_node(&mut self, node: &ruby_prism::ConstantAndWriteNode<'pr>) {
        self.shorthand(
            Compound::And,
            |w| w.around(Event::Asgn, |_| {}),
            false,
            &node.value(),
        );
    }

    fn visit_constant_path_write_node(&mut self, node: &ruby_prism::ConstantPathWriteNode<'pr>) {
        self.around(Event::Asgn, |w| {
            ruby_prism::visit_constant_path_write_node(w, node)
        });
    }

    fn visit_constant_path_target_node(&mut self, node: &ruby_prism::ConstantPathTargetNode<'pr>) {
        self.around(Event::Asgn, |w| {
            ruby_prism::visit_constant_path_target_node(w, node)
        });
    }

    fn visit_constant_path_operator_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathOperatorWriteNode<'pr>,
    ) {
        let target = node.target();
        self.shorthand(
            Compound::Operator,
            |w| w.around(Event::Asgn, |w| w.visit_constant_path_node(&target)),
            false,
            &node.value(),
        );
    }

    fn visit_constant_path_or_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathOrWriteNode<'pr>,
    ) {
        let target = node.target();
        self.shorthand(
            Compound::Or,
            |w| w.around(Event::Asgn, |w| w.visit_constant_path_node(&target)),
            false,
            &node.value(),
        );
    }

    fn visit_constant_path_and_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathAndWriteNode<'pr>,
    ) {
        let target = node.target();
        self.shorthand(
            Compound::And,
            |w| w.around(Event::Asgn, |w| w.visit_constant_path_node(&target)),
            false,
            &node.value(),
        );
    }

    // Attribute and index shorthand assignments: `(op_asgn (send r :m) v)`.

    fn visit_call_operator_write_node(&mut self, node: &ruby_prism::CallOperatorWriteNode<'pr>) {
        let send = read_send(
            node.receiver(),
            node.is_safe_navigation(),
            node.read_name().as_slice(),
        );
        let receiver = node.receiver();
        self.shorthand(
            Compound::Operator,
            |w| {
                w.around(Event::Send(send), |w| {
                    receiver.iter().for_each(|r| w.visit(r))
                })
            },
            true,
            &node.value(),
        );
    }

    fn visit_call_or_write_node(&mut self, node: &ruby_prism::CallOrWriteNode<'pr>) {
        let send = read_send(
            node.receiver(),
            node.is_safe_navigation(),
            node.read_name().as_slice(),
        );
        let receiver = node.receiver();
        self.shorthand(
            Compound::Or,
            |w| {
                w.around(Event::Send(send), |w| {
                    receiver.iter().for_each(|r| w.visit(r))
                })
            },
            true,
            &node.value(),
        );
    }

    fn visit_call_and_write_node(&mut self, node: &ruby_prism::CallAndWriteNode<'pr>) {
        let send = read_send(
            node.receiver(),
            node.is_safe_navigation(),
            node.read_name().as_slice(),
        );
        let receiver = node.receiver();
        self.shorthand(
            Compound::And,
            |w| {
                w.around(Event::Send(send), |w| {
                    receiver.iter().for_each(|r| w.visit(r))
                })
            },
            true,
            &node.value(),
        );
    }

    fn visit_index_operator_write_node(&mut self, node: &ruby_prism::IndexOperatorWriteNode<'pr>) {
        let send = read_send(node.receiver(), node.is_safe_navigation(), b"[]");
        let (receiver, arguments, block) = (node.receiver(), node.arguments(), node.block());
        self.shorthand(
            Compound::Operator,
            |w| {
                w.around(Event::Send(send), |w| {
                    index_children(w, receiver, arguments, block);
                });
            },
            true,
            &node.value(),
        );
    }

    fn visit_index_or_write_node(&mut self, node: &ruby_prism::IndexOrWriteNode<'pr>) {
        let send = read_send(node.receiver(), node.is_safe_navigation(), b"[]");
        let (receiver, arguments, block) = (node.receiver(), node.arguments(), node.block());
        self.shorthand(
            Compound::Or,
            |w| {
                w.around(Event::Send(send), |w| {
                    index_children(w, receiver, arguments, block);
                });
            },
            true,
            &node.value(),
        );
    }

    fn visit_index_and_write_node(&mut self, node: &ruby_prism::IndexAndWriteNode<'pr>) {
        let send = read_send(node.receiver(), node.is_safe_navigation(), b"[]");
        let (receiver, arguments, block) = (node.receiver(), node.arguments(), node.block());
        self.shorthand(
            Compound::And,
            |w| {
                w.around(Event::Send(send), |w| {
                    index_children(w, receiver, arguments, block);
                });
            },
            true,
            &node.value(),
        );
    }

    // Arguments.

    fn visit_required_parameter_node(&mut self, node: &ruby_prism::RequiredParameterNode<'pr>) {
        self.argument(Some(node.name().as_slice()));
    }

    fn visit_optional_parameter_node(&mut self, node: &ruby_prism::OptionalParameterNode<'pr>) {
        let name = node.name().as_slice();
        if capturing(Some(name)) {
            self.around(Event::CapturingArgument, |w| w.visit(&node.value()));
        } else {
            self.visit(&node.value());
        }
    }

    fn visit_rest_parameter_node(&mut self, node: &ruby_prism::RestParameterNode<'pr>) {
        self.argument(node.name().map(|n| n.as_slice()));
    }

    fn visit_required_keyword_parameter_node(
        &mut self,
        node: &ruby_prism::RequiredKeywordParameterNode<'pr>,
    ) {
        self.argument(Some(node.name().as_slice()));
    }

    fn visit_optional_keyword_parameter_node(
        &mut self,
        node: &ruby_prism::OptionalKeywordParameterNode<'pr>,
    ) {
        let name = node.name().as_slice();
        if capturing(Some(name)) {
            self.around(Event::CapturingArgument, |w| w.visit(&node.value()));
        } else {
            self.visit(&node.value());
        }
    }

    fn visit_keyword_rest_parameter_node(
        &mut self,
        node: &ruby_prism::KeywordRestParameterNode<'pr>,
    ) {
        self.argument(node.name().map(|n| n.as_slice()));
    }

    fn visit_block_parameter_node(&mut self, node: &ruby_prism::BlockParameterNode<'pr>) {
        self.argument(node.name().map(|n| n.as_slice()));
    }

    fn visit_block_local_variable_node(&mut self, node: &ruby_prism::BlockLocalVariableNode<'pr>) {
        self.argument(Some(node.name().as_slice()));
    }
}

impl<'pr, F: FnMut(Event<'_, 'pr>)> Walker<F> {
    /// A leaf argument named `name`.
    fn argument(&mut self, name: Option<&[u8]>) {
        if capturing(name) {
            self.around(Event::CapturingArgument, |_| {});
        }
    }
}

/// `capturing_variable?`: a name not starting with `_`.
fn capturing(name: Option<&[u8]>) -> bool {
    name.is_some_and(|n| !n.starts_with(b"_"))
}

/// The receiver and index arguments of an index read target.
fn index_children<'pr, F: FnMut(Event<'_, 'pr>)>(
    walker: &mut Walker<F>,
    receiver: Option<Node<'pr>>,
    arguments: Option<ruby_prism::ArgumentsNode<'pr>>,
    block: Option<ruby_prism::BlockArgumentNode<'pr>>,
) {
    if let Some(receiver) = receiver {
        walker.visit(&receiver);
    }
    if let Some(arguments) = arguments {
        walker.visit_arguments_node(&arguments);
    }
    if let Some(block) = block {
        walker.visit_block_argument_node(&block);
    }
}

/// `(line, col, name, measure(body))` of every method of `source`, in
/// source order (test support for the cops' upstream measurements).
#[cfg(test)]
pub(super) fn measured<T>(
    source: &str,
    measure: impl Fn(&Node<'_>) -> T,
) -> Vec<(u32, u32, String, T)> {
    use crate::cops::walk::{NodeVisitor, walk};

    struct Methods<'pr>(Vec<Node<'pr>>);
    impl<'pr> NodeVisitor<'pr> for Methods<'pr> {
        fn enter(&mut self, node: Node<'pr>) {
            self.0.push(node);
        }
    }
    let file = SourceFile::new("example.rb".into(), source.as_bytes().to_vec());
    let parsed = crate::parser::parse(source.as_bytes());
    assert_eq!(parsed.errors().count(), 0, "{source:?}");
    let mut nodes = Methods(Vec::new());
    walk(&mut nodes, &parsed.node());
    nodes
        .0
        .iter()
        .filter_map(Method::of)
        .map(|m| {
            let (line, col) = file.line_col(m.location.start_offset());
            (line, col, m.name, measure(&m.body))
        })
        .collect()
}

#[cfg(test)]
mod tests;
