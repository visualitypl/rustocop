//! Shared AST helpers for cops: structural node equality and the literal,
//! variable and constant predicates RuboCop's cops are written against.
//!
//! RuboCop compares nodes with `==`, which is structural: same node type,
//! same children, regardless of formatting. [`Fingerprint`] is the analogue
//! for Prism nodes: a token stream of node kinds, leaf values and the
//! node-owned source text between children (operators, names, keywords), with
//! whitespace, parentheses, semicolons and line continuations dropped.
//!
//! Spellings that the parser gem (and so RuboCop) maps to the same node are
//! normalized: `and`/`&&`, `or`/`||`, `not x`/`!x`, `a.b`/`a::b`,
//! `:a => 1`/`a: 1`, `do…end`/`{…}`, array and hash delimiters (`%w[a]`,
//! `['a']`), keyword hashes vs braced hashes (`foo(a: 1)`/`foo({a: 1})`),
//! and implicit keyword values (`foo(a:)`/`foo(a: a)`).
//!
//! The approximation errs towards "different": any other two spellings of
//! the same value that Prism keeps apart (e.g. `x.!` vs `!x`)
//! compare unequal, so a cop built on it can miss a duplicate but
//! never invents one.

use std::{
    hash::{Hash, Hasher},
    marker::PhantomData,
    mem::discriminant,
};

use ruby_prism::{CallNode, Location, Node};

pub(crate) mod require_literal;

use crate::{
    cops::walk::{NodeVisitor, walk},
    source::SourceFile,
};

/// End-exclusive end of the node's expression as RuboCop's AST sees it.
/// Prism includes a literal block in a call and subsequent clauses in a
/// `rescue`; neither belongs to the send/resbody highlight. Cops reporting
/// the entire block expression instead use the call's full location.
pub fn source_end(node: &Node<'_>) -> usize {
    if let Some(call) = node.as_call_node() {
        let mut end = call
            .message_loc()
            .map_or(call.location().start_offset(), |loc| loc.end_offset());
        if let Some(closing) = call.closing_loc() {
            end = end.max(closing.end_offset());
        }
        // For `object[index] = value`, the closing `]` precedes the final
        // argument (the value). For unary calls, the receiver follows the
        // selector. Both still belong to the send highlight.
        for argument in call_arguments(&call).iter().chain(call.receiver().iter()) {
            end = end.max(argument.location().end_offset());
        }
        return end;
    }
    if let Some(rescue) = node.as_rescue_node() {
        let mut end = rescue.keyword_loc().end_offset();
        for exception in rescue.exceptions().iter().chain(rescue.reference()) {
            end = end.max(exception.location().end_offset());
        }
        if let Some(then) = rescue.then_keyword_loc() {
            end = end.max(then.end_offset());
        }
        if let Some(body) = rescue.statements() {
            end = end.max(body.location().end_offset());
        }
        return end;
    }
    node.location().end_offset()
}

/// A structural fingerprint of a node: equal fingerprints mean structurally
/// equal nodes. Only meaningful for nodes from the same parse (except those
/// from [`fingerprint_ignoring_grouping`]). The lifetime is only nominal
/// (and covariant), so fingerprints of two parses can be compared.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Fingerprint<'pr>(Vec<Token>, PhantomData<&'pr ()>);

impl Fingerprint<'_> {
    /// Ruby Complex#== compares its numeric components across numeric kinds.
    /// This is deliberately separate from Eq/Hash: mixed Rational/Float equality
    /// can be non-transitive, and must not weaken any existing fingerprint mode.
    pub(crate) fn conditional_scope_eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self
                .0
                .iter()
                .zip(&other.0)
                .all(|(left, right)| match (left, right) {
                    (Token::ConditionalScopeLiteral(a), Token::ConditionalScopeLiteral(b)) => {
                        a.same_ast_value(b)
                    }
                    _ => left == right,
                })
    }

    /// Detection-only scope equality in the reference source buffer. UTF8-MAC
    /// conversion applies to identifier/selector tokens too, not just literals.
    /// Opt-in for NonAtomicFileOperation; no old fingerprint keys or correction
    /// equivalence claims change. Complex components use Ruby numeric equality.
    pub(crate) fn source_buffer_scope_eq(&self, other: &Self, file: &SourceFile) -> bool {
        use super::lint::literal_assignment_in_condition::ScopeLiteral;
        fn literal_eq(a: &ScopeLiteral, b: &ScopeLiteral, file: &SourceFile) -> bool {
            match (a, b) {
                (ScopeLiteral::Ruby(a), ScopeLiteral::Ruby(b)) => a.source_buffer_scope_eq(b, file),
                (ScopeLiteral::Compound(ak, a), ScopeLiteral::Compound(bk, b)) => {
                    ak == bk
                        && a.len() == b.len()
                        && a.iter().zip(b).all(|(a, b)| literal_eq(a, b, file))
                }
                _ => a.same_ast_value(b),
            }
        }
        self.0.len() == other.0.len()
            && self.0.iter().zip(&other.0).all(|(a, b)| match (a, b) {
                (Token::Leaf(ak, a), Token::Leaf(bk, b)) => {
                    // Numeric limbs/bits, offsets and arities are not encoded
                    // source characters, even when their bytes are valid UTF-8.
                    ak == bk
                        && (a == b
                            || (!matches!(
                                ak,
                                Kind::Integer
                                    | Kind::Float
                                    | Kind::Rational
                                    | Kind::SourceLine
                                    | Kind::NumberedParameters
                            ) && file.match_pattern_source(a) == file.match_pattern_source(b)))
                }
                (Token::Close(a), Token::Close(b)) => {
                    file.match_pattern_source(a) == file.match_pattern_source(b)
                }
                (Token::ConditionalScopeLiteral(a), Token::ConditionalScopeLiteral(b)) => {
                    literal_eq(a, b, file)
                }
                _ => a == b,
            })
    }

    /// Fingerprints own all their tokens, so source-local caches can retain them
    /// without borrowing the parse tree.
    pub(crate) fn into_owned(self) -> Fingerprint<'static> {
        Fingerprint(self.0, PhantomData)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Token {
    /// A branch node opens; its children's tokens follow.
    Open(Kind),
    /// A branch node closes, carrying its own (normalized) source text.
    Close(Vec<u8>),
    /// A leaf node and its value.
    Leaf(Kind, Vec<u8>),
    /// Exact literal AST shape for LiteralAssignmentInCondition's scope edges
    /// ONLY; never changes existing detection keys or correction proofs.
    ConditionalScopeLiteral(Box<super::lint::literal_assignment_in_condition::ScopeLiteral>),
}

/// A node kind, with kinds the parser gem does not tell apart merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    /// The node's variant: the value of its `Discriminant`, which (unlike
    /// the `Discriminant` itself) carries no invariant lifetime.
    Node(u64),
    /// `HashNode` and `KeywordHashNode`: both `hash` in RuboCop's AST.
    Hash,
    /// Binary values, never source text for transcoding/composition.
    Integer,
    Float,
    Rational,
    NumberedParameters,
    String,
    SourceLine,
    InterpolatedString,
    InterpolatedSymbol,
    /// Require-specific xstr shape, shared by static and interpolated commands.
    RequireXString,
    ConstantRead,
    ConstantPath,
    /// Opt-in lexical projection: a reference-buffer local read.
    FileOperationLocal,
    /// Detection-only arrow / bare `lambda` block equivalence.
    Lambda,
    /// Detection-only if/unless/ternary, ordered by predicate truth value.
    Conditional,
    /// Detection-only case kinds, with clause delimiters omitted.
    Case,
    CaseMatch,
    /// Detection-only implicit rescue / rescue-modifier and handler shapes.
    Rescue,
    RescueBody,
    /// Gemspec-only parser begin shape: grouping/interpolation owns its body.
    GemspecBegin,
}

/// A `Hasher` that keeps the bytes hashed instead of mixing them: hashing a
/// `Discriminant` writes its integer value, which this recovers exactly.
#[derive(Default)]
struct DiscriminantValue(u64);

impl Hasher for DiscriminantValue {
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(byte);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

fn kind(node: &Node<'_>) -> Kind {
    match node {
        Node::HashNode { .. } | Node::KeywordHashNode { .. } => Kind::Hash,
        Node::ImplicitNode { .. } => kind(&node.as_implicit_node().unwrap().value()),
        Node::IntegerNode { .. } => Kind::Integer,
        Node::FloatNode { .. } => Kind::Float,
        Node::RationalNode { .. } => Kind::Rational,
        Node::NumberedParametersNode { .. } => Kind::NumberedParameters,
        Node::StringNode { .. } => Kind::String,
        Node::InterpolatedStringNode { .. } => Kind::InterpolatedString,
        Node::InterpolatedSymbolNode { .. } => Kind::InterpolatedSymbol,
        Node::SourceLineNode { .. } => Kind::SourceLine,
        Node::ConstantReadNode { .. } => Kind::ConstantRead,
        Node::ConstantPathNode { .. } => Kind::ConstantPath,
        _ => {
            let mut value = DiscriminantValue::default();
            discriminant(node).hash(&mut value);
            Kind::Node(value.finish())
        }
    }
}

/// Compute the structural fingerprint of `node`.
pub fn fingerprint<'pr>(node: &Node<'pr>) -> Fingerprint<'pr> {
    structural_fingerprint(node, None, None)
}

/// SelfAssignment's detection-only parser AST equality, including numeric
/// values, branch child slots, regexp option sets and exact literal/local
/// source-buffer projection. Raw fingerprints/correction keys never opt in.
pub(crate) fn fingerprint_for_self_assignment<'pr>(
    node: &Node<'pr>,
    file: &SourceFile,
    locals: Option<&std::collections::HashSet<(usize, usize)>>,
) -> Fingerprint<'pr> {
    structural_fingerprint(node, locals, Some(file))
}

fn structural_fingerprint<'pr>(
    node: &Node<'pr>,
    locals: Option<&std::collections::HashSet<(usize, usize)>>,
    self_assignment_file: Option<&SourceFile>,
) -> Fingerprint<'pr> {
    let mut builder = FingerprintBuilder {
        self_assignment_file,
        gemspec_keys: false,
        duplicate_equality: None,
        file_operation_locals: locals,
        tokens: Vec::new(),
        frames: Vec::new(),
        regrouping: None,
        line_file: self_assignment_file,
        require_literals: false,
        conditional_scope_literals: false,
        numeric_zero_equality: self_assignment_file.is_some(),
        case_subject_equality: self_assignment_file.is_some(),
        loop_collection_equality: self_assignment_file.is_some(),
        normalize_branches: self_assignment_file.is_some(),
    };
    walk(&mut builder, node);
    let key = Fingerprint(builder.tokens, PhantomData);
    if self_assignment_file.is_some() {
        integer_source_lines(key)
    } else {
        key
    }
}

/// Like [`fingerprint`], but compare `__LINE__` by its evaluated line number.
/// Uses the file's cached line index without relaxing grouping equality.
pub(crate) fn fingerprint_with_source_lines<'pr>(
    node: &Node<'pr>,
    file: &SourceFile,
) -> Fingerprint<'pr> {
    let mut builder = FingerprintBuilder {
        self_assignment_file: None,
        duplicate_equality: None,
        file_operation_locals: None,
        tokens: Vec::new(),
        frames: Vec::new(),
        regrouping: None,
        line_file: Some(file),
        require_literals: false,
        conditional_scope_literals: false,
        numeric_zero_equality: false,
        case_subject_equality: false,
        loop_collection_equality: false,
        normalize_branches: false,
        gemspec_keys: false,
    };
    walk(&mut builder, node);
    Fingerprint(builder.tokens, PhantomData)
}

/// Detection-only equality for numeric node patterns: RuboCop represents
/// `__LINE__` as an Integer and compares Rational values and signed float zeros
/// by value, including inside compound/imaginary bit flag expressions.
/// Keep the ordinary source-line fingerprints distinct for correction proofs.
pub(crate) fn fingerprint_with_integer_source_lines<'pr>(
    node: &Node<'pr>,
    file: &SourceFile,
) -> Fingerprint<'pr> {
    integer_source_lines(fingerprint_for_comparison(node, file))
}

/// Detection-only subject equality for `Style/CaseLikeIf`. The pinned parser
/// represents `__ENCODING__` as `Encoding::UTF_8` (even in legacy source files)
/// and explicit `x.!` as the same send as `!x`. Do not use these source/dispatch
/// normalizations to prove correction safety.
pub(crate) fn fingerprint_for_case_subject<'pr>(
    node: &Node<'pr>,
    file: &SourceFile,
) -> Fingerprint<'pr> {
    let mut builder = FingerprintBuilder {
        self_assignment_file: None,
        duplicate_equality: None,
        file_operation_locals: None,
        tokens: Vec::new(),
        frames: Vec::new(),
        regrouping: None,
        line_file: Some(file),
        require_literals: false,
        numeric_zero_equality: true,
        conditional_scope_literals: false,
        case_subject_equality: true,
        loop_collection_equality: false,
        normalize_branches: false,
        gemspec_keys: false,
    };
    walk(&mut builder, node);
    integer_source_lines(Fingerprint(builder.tokens, PhantomData))
}

/// Detection-only structural equality for collection and assignment dependency
/// matching. Compare call selectors rather than their bracket/operator spelling,
/// numeric and source-line values, and regexp options as sets. Never use this
/// for correction proofs.
pub(crate) fn fingerprint_for_detection<'pr>(
    node: &Node<'pr>,
    file: &SourceFile,
) -> Fingerprint<'pr> {
    let mut builder = FingerprintBuilder {
        self_assignment_file: None,
        duplicate_equality: None,
        file_operation_locals: None,
        tokens: Vec::new(),
        frames: Vec::new(),
        regrouping: None,
        line_file: Some(file),
        require_literals: false,
        numeric_zero_equality: true,
        conditional_scope_literals: false,
        case_subject_equality: false,
        loop_collection_equality: true,
        normalize_branches: false,
        gemspec_keys: false,
    };
    walk(&mut builder, node);
    integer_source_lines(Fingerprint(builder.tokens, PhantomData))
}

/// Detection-only branch-body equality for `Lint/DuplicateBranch`: combine
/// parser-value constants/unary sends with selector and regexp normalization.
/// Canonicalize parser child shapes and truth-value order, not source spelling:
/// empty/missing else, unless inversion, elsif nesting, loop test position and
/// def/defs slots. Grouping and structurally distinct children remain distinct.
/// This is not a correction equivalence proof.
pub(crate) fn fingerprint_for_branch<'pr>(node: &Node<'pr>, file: &SourceFile) -> Fingerprint<'pr> {
    let mut builder = FingerprintBuilder {
        gemspec_keys: false,
        self_assignment_file: None,
        duplicate_equality: None,
        file_operation_locals: None,
        tokens: Vec::new(),
        frames: Vec::new(),
        regrouping: None,
        line_file: Some(file),
        require_literals: false,
        numeric_zero_equality: true,
        conditional_scope_literals: false,
        case_subject_equality: true,
        loop_collection_equality: true,
        normalize_branches: true,
    };
    walk(&mut builder, node);
    integer_source_lines(Fingerprint(builder.tokens, PhantomData))
}

/// LiteralAssignmentInCondition compares actual block/def child AST shapes.
/// Its literal-token projection is deliberately isolated from old consumers,
/// especially source corrections and DuplicateRequire's different identity.
pub(crate) fn fingerprint_for_conditional_scope<'pr>(
    node: &Node<'pr>,
    file: &SourceFile,
) -> Fingerprint<'pr> {
    fingerprint_for_file_operation(node, file, None)
}

/// Private detection projection for NonAtomicFileOperation. The supplied spans
/// are actual local reads/projected vcalls and their ancestor subtrees, collected
/// with real scope edges. No old fingerprint consumer or correction key opts in.
pub(crate) fn fingerprint_for_file_operation<'pr>(
    node: &Node<'pr>,
    file: &SourceFile,
    locals: Option<&std::collections::HashSet<(usize, usize)>>,
) -> Fingerprint<'pr> {
    file_operation_fingerprint(node, file, locals, None)
}

/// Existing duplicate cops need reference-buffer names/regexp options as well
/// as exact literal shapes. Other modes and correction keys remain unchanged.
/// Hash-backed consumers preserve numeric component kinds in Complex#eql?.
pub(crate) fn fingerprint_for_duplicate<'pr>(
    node: &Node<'pr>,
    file: &SourceFile,
) -> Fingerprint<'pr> {
    file_operation_fingerprint(node, file, None, Some(DuplicateEquality::Hash))
}

/// Gemspec keys compare pinned ASTs, not runtime binding identities: implicit
/// `it` and ordinary local `it` are both lvar(:it). Regexp text uses the existing
/// source alignment (composition precedes escape lexing). The helper supplies
/// local/regexp/control ancestor spans so literal fast paths cannot hide these
/// projections, including protected rescue's absent/empty else child slot.
/// Interpolation/grouping begin boundaries and exact literal emission propagate
/// through literal fast paths too. Other duplicate/correction profiles do not opt in.
pub(crate) fn fingerprint_for_gemspec_key<'pr>(
    node: &Node<'pr>,
    file: &SourceFile,
    locals: &std::collections::HashSet<(usize, usize)>,
) -> Fingerprint<'pr> {
    let mut builder = file_operation_builder(file, Some(locals), Some(DuplicateEquality::Hash));
    builder.gemspec_keys = true;
    walk(&mut builder, node);
    integer_source_lines(Fingerprint(builder.tokens, PhantomData))
}

/// Elsif uses Array#include? / Complex#==, not hash equality.
pub(crate) fn fingerprint_for_duplicate_condition<'pr>(
    node: &Node<'pr>,
    file: &SourceFile,
) -> Fingerprint<'pr> {
    file_operation_fingerprint(node, file, None, Some(DuplicateEquality::Value))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DuplicateEquality {
    Hash,
    Value,
}

fn file_operation_fingerprint<'pr>(
    node: &Node<'pr>,
    file: &SourceFile,
    locals: Option<&std::collections::HashSet<(usize, usize)>>,
    duplicate_equality: Option<DuplicateEquality>,
) -> Fingerprint<'pr> {
    let mut builder = file_operation_builder(file, locals, duplicate_equality);
    walk(&mut builder, node);
    integer_source_lines(Fingerprint(builder.tokens, PhantomData))
}

fn file_operation_builder<'a, 'pr>(
    file: &'a SourceFile,
    locals: Option<&'a std::collections::HashSet<(usize, usize)>>,
    duplicate_equality: Option<DuplicateEquality>,
) -> FingerprintBuilder<'a, 'pr> {
    FingerprintBuilder {
        self_assignment_file: None,
        gemspec_keys: false,
        duplicate_equality,
        file_operation_locals: locals,
        tokens: Vec::new(),
        frames: Vec::new(),
        regrouping: None,
        line_file: Some(file),
        require_literals: false,
        conditional_scope_literals: true,
        numeric_zero_equality: true,
        case_subject_equality: true,
        loop_collection_equality: true,
        normalize_branches: true,
    }
}

/// DuplicateRequire keys arguments by AST#to_s, not Ruby numeric equality.
/// Reuse parser-shape normalization but preserve the sign of floating zero.
/// Source names/regexp text follow the reference buffer; supplied local spans
/// come from genuine VariableForce declarations and lexical order, once per file.
/// This detection-only key must never be used to prove correction safety.
pub(crate) fn fingerprint_for_require<'pr>(
    node: &Node<'pr>,
    file: &SourceFile,
    locals: Option<&std::collections::HashSet<(usize, usize)>>,
) -> Fingerprint<'pr> {
    let mut builder = FingerprintBuilder {
        self_assignment_file: None,
        duplicate_equality: None,
        file_operation_locals: locals,
        gemspec_keys: false,
        tokens: Vec::new(),
        frames: Vec::new(),
        regrouping: None,
        line_file: Some(file),
        require_literals: true,
        conditional_scope_literals: false,
        numeric_zero_equality: false,
        case_subject_equality: true,
        loop_collection_equality: true,
        normalize_branches: true,
    };
    walk(&mut builder, node);
    integer_source_lines(Fingerprint(builder.tokens, PhantomData))
}

fn integer_source_lines(mut fingerprint: Fingerprint<'_>) -> Fingerprint<'_> {
    for token in &mut fingerprint.0 {
        if let Token::Leaf(kind @ Kind::SourceLine, value) = token {
            *kind = Kind::Integer;
            // SourceFile's line numbers are nonzero u32s; the Integer encoding
            // is a sign byte followed by little-endian u32 magnitude digits.
            value.insert(0, 0);
        }
    }
    fingerprint
}

/// Detection-only AST equality matching Ruby's numeric zero and exact Rational
/// equality, including inside compound subjects. Keep ordinary fingerprints bit-exact: signed Float
/// zero is observable, so correction safety must not use this normalization.
pub(crate) fn fingerprint_for_comparison<'pr>(
    node: &Node<'pr>,
    file: &SourceFile,
) -> Fingerprint<'pr> {
    let mut builder = FingerprintBuilder {
        self_assignment_file: None,
        duplicate_equality: None,
        file_operation_locals: None,
        tokens: Vec::new(),
        frames: Vec::new(),
        regrouping: None,
        line_file: Some(file),
        require_literals: false,
        numeric_zero_equality: true,
        conditional_scope_literals: false,
        case_subject_equality: false,
        loop_collection_equality: false,
        normalize_branches: false,
        gemspec_keys: false,
    };
    walk(&mut builder, node);
    Fingerprint(builder.tokens, PhantomData)
}

/// A fingerprint that is blind to grouping, for checking that removing
/// parentheses leaves a parse unchanged (RuboCop's `ReparsedEquivalence`
/// as `Style/RedundantParentheses` uses it):
///
/// - grouping parentheses with a body are transparent: `(x)` fingerprints
///   as `x`, and a parenthesized statement sequence inside another sequence
///   as its statements spliced in (`a; (b; c)` as `a; b; c`);
/// - chains of the same `&&`/`||` operator are flattened, so `a && (b && c)`
///   and `a && b && c` compare equal (the operators cannot be redefined);
/// - `__LINE__` compares by its line number, read from `source` (the bytes
///   `node` was parsed from), so it can match across two parses.
///
/// Unlike [`fingerprint`], meant for comparing whole trees of two parses.
pub fn fingerprint_ignoring_grouping<'pr>(node: &Node<'pr>, source: &'pr [u8]) -> Fingerprint<'pr> {
    let mut builder = FingerprintBuilder {
        self_assignment_file: None,
        duplicate_equality: None,
        file_operation_locals: None,
        tokens: Vec::new(),
        frames: Vec::new(),
        regrouping: Some(source),
        gemspec_keys: false,
        line_file: None,
        conditional_scope_literals: false,
        require_literals: false,
        numeric_zero_equality: false,
        case_subject_equality: false,
        loop_collection_equality: false,
        normalize_branches: false,
    };
    walk(&mut builder, node);
    Fingerprint(builder.tokens, PhantomData)
}

/// Whether two nodes are structurally equal (RuboCop's node `==`).
pub fn same_node(a: &Node<'_>, b: &Node<'_>) -> bool {
    kind(a) == kind(b) && fingerprint(a) == fingerprint(b)
}

/// Whether two optional nodes are structurally equal (`nil == nil` holds).
pub fn same_optional_node(a: Option<&Node<'_>>, b: Option<&Node<'_>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => same_node(a, b),
        _ => false,
    }
}

/// An open branch node while fingerprinting.
struct Frame<'pr> {
    /// The node's kind.
    kind: Kind,
    /// The node's source bytes.
    source: &'pr [u8],
    /// The node's start offset.
    start: usize,
    /// Byte ranges cut out of the node's own text: its direct children seen
    /// so far, and own tokens that are layout or have a canonical spelling.
    children: Vec<(usize, usize, Cut)>,
    /// Whether the node's own text is ignored (string-like delimiters).
    ignore_own_text: bool,
    /// Detection-only canonical selector or regexp options, when requested.
    canonical_own_text: Option<Vec<u8>>,
    /// Direct regexp parts in the Require/Gemspec source-alignment profiles.
    source_regexp_parts: bool,
    /// Explicit protected begins retain kwbegin, but rescue else is a child slot.
    protected_rescue_else: bool,
    /// Direct emitted child boundaries for an unless. The complete walk emits
    /// predicate/body/else in Prism order; at close we rotate just the two body
    /// token spans into parser's predicate/true/false order. No subtree rescan.
    inverted_children: Option<Vec<usize>>,
    /// A def target's outer receiver parentheses are target syntax, not a
    /// grouping expression. Only that exact wrapper is transparent.
    receiver_parentheses: Option<(usize, usize)>,
    /// A bare kwbegin owns its statements directly in parser's AST, unlike
    /// bodies of blocks/defs or a protected rescue/ensure body.
    flat_kwbegin: bool,
    /// Count the modifier's direct children to wrap its second child in a
    /// synthetic resbody during the same walk, without shifting token spans.
    rescue_modifier_children: Option<usize>,
}

/// How a range is cut out of a node's own text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Cut {
    /// A child node: leaves a separator so positions stay significant.
    Child,
    /// Layout, e.g. empty block parameters `||` or a block's `do`/`end`.
    Layout,
    /// An alternative spelling, replaced by the canonical one (`and` → `&&`).
    Replace(&'static [u8]),
}

/// An open branch node's place on the builder's stack.
enum Slot<'pr> {
    /// A node that emits tokens.
    Open(Frame<'pr>),
    /// A node that emits nothing and has no children (empty block
    /// parameters).
    Silent,
    /// A scope-literal subtree already emitted by its exact local projection.
    ConditionalScopeLiteral,
    /// An implicit keyword value; its child belongs to the nearest open
    /// ancestor and its source location also includes the label's colon.
    Implicit,
    /// A node whose children belong to the nearest open ancestor (grouping
    /// parentheses and their statements, flattened `&&`/`||`). `paren`
    /// marks grouping parentheses.
    Transparent { paren: bool },
}

struct FingerprintBuilder<'a, 'pr> {
    tokens: Vec<Token>,
    /// The open branch nodes, innermost last.
    frames: Vec<Slot<'pr>>,
    /// For [`fingerprint_ignoring_grouping`]: the parsed source.
    regrouping: Option<&'pr [u8]>,
    /// Optional cached physical-line lookup, independent of grouping handling.
    line_file: Option<&'a SourceFile>,
    require_literals: bool,
    conditional_scope_literals: bool,
    self_assignment_file: Option<&'a SourceFile>,
    duplicate_equality: Option<DuplicateEquality>,
    numeric_zero_equality: bool,
    case_subject_equality: bool,
    loop_collection_equality: bool,
    normalize_branches: bool,
    file_operation_locals: Option<&'a std::collections::HashSet<(usize, usize)>>,
    /// Only the Gemspec AST-key profile unifies implicit and ordinary `it` reads.
    gemspec_keys: bool,
}

impl<'pr> FingerprintBuilder<'_, 'pr> {
    fn scope_literal(&mut self, node: &Node<'pr>) -> bool {
        // Compound literals must carry this profile into embedded Ruby keys;
        // ordinary scope/correction consumers retain their original identity.
        let location = node.location();
        if self.gemspec_keys
            && node.as_string_node().is_some()
            && matches!(self.frames.last(), Some(Slot::Open(frame)) if frame.source_regexp_parts)
        {
            // Regexp text needs source alignment, not the ordinary string-value
            // fast path. Embedded Ruby keeps its own frames/component kinds.
            return false;
        }
        if (self.conditional_scope_literals || self.self_assignment_file.is_some())
            && !self.file_operation_locals.is_some_and(|locals| {
                locals.contains(&(location.start_offset(), location.end_offset()))
                    // A shorthand label's SymbolNode shares its span with the
                    // implicit local read. Only that read needs projection;
                    // the symbol must retain the same literal token as `x: x`.
                    && (!self.gemspec_keys || node.as_symbol_node().is_none())
            })
            && let Some(literal) = if let Some(file) = self.self_assignment_file {
                super::lint::literal_assignment_in_condition::self_assignment_literal(
                    node,
                    file,
                    self.file_operation_locals,
                )
            } else if self.gemspec_keys {
                super::lint::literal_assignment_in_condition::gemspec_literal(
                    node,
                    self.line_file.unwrap(),
                    self.file_operation_locals.unwrap(),
                )
            } else {
                super::lint::literal_assignment_in_condition::scope_literal_with_profile(
                    node,
                    self.line_file.unwrap(),
                    self.duplicate_equality,
                )
            }
        {
            self.record_heredoc_body(node);
            self.record_child(node);
            self.tokens
                .push(Token::ConditionalScopeLiteral(Box::new(literal)));
            return true;
        }
        false
    }

    fn file_operation_local(&mut self, node: &Node<'pr>) -> bool {
        let Some(locals) = self.file_operation_locals else {
            return false;
        };
        let location = node.location();
        let name = if let Some(read) = node.as_local_variable_read_node() {
            read.name().as_slice()
        } else if self.gemspec_keys && node.as_it_local_variable_read_node().is_some() {
            &b"it"[..]
        } else if let Some(call) = node.as_call_node()
            && call.is_variable_call()
            && call.receiver().is_none()
            && call.arguments().is_none()
            && call.block().is_none()
            && locals.contains(&(location.start_offset(), location.end_offset()))
        {
            call.name().as_slice()
        } else {
            return false;
        };
        self.record_child(node);
        let name = if self.require_literals || self.duplicate_equality.is_some() {
            self.line_file
                .unwrap()
                .match_pattern_source(name)
                .into_owned()
        } else {
            name.to_vec()
        };
        self.tokens
            .push(Token::Leaf(Kind::FileOperationLocal, name));
        true
    }

    /// Keep branch-specific parser normalization out of correction proofs.
    fn branch_equality(&self) -> bool {
        self.normalize_branches
    }

    fn branch_transparent(&self, node: &Node<'pr>) -> bool {
        if let Some(statements) = node.as_statements_node() {
            // A bare kwbegin owns these direct children, not a nested begin.
            // Do not flatten through parentheses: their sequence is a child.
            return statements.body().len() == 1
                || matches!(self.frames.last(), Some(Slot::Open(frame))
                    if frame.flat_kwbegin || (self.gemspec_keys && frame.kind == Kind::GemspecBegin));
        }
        if let Some(paren) = node.as_parentheses_node() {
            // Ordinary branch profiles elide the wrapper on `(a; b)`. Gemspec
            // instead flattens its implicit Statements child, retaining this
            // begin boundary even when nested under interpolation/grouping.
            let location = paren.location();
            return matches!(self.frames.last(), Some(Slot::Open(frame))
                if frame.receiver_parentheses == Some((location.start_offset(), location.end_offset())))
                || (!self.gemspec_keys
                    && paren.body().is_some_and(|body| {
                        body.as_statements_node()
                            .is_some_and(|s| s.body().len() > 1)
                    }));
        }
        (node.as_block_node().is_some()
            && matches!(self.frames.last(), Some(Slot::Open(frame)) if frame.kind == Kind::Lambda))
            || (node.as_else_node().is_some()
                && matches!(self.frames.last(), Some(Slot::Open(frame)) if matches!(frame.kind, Kind::Conditional | Kind::Case | Kind::Rescue) || frame.protected_rescue_else))
    }

    fn record_child(&mut self, node: &Node<'pr>) {
        let start = self.tokens.len();
        let mut open_rescue_body = false;
        if self.branch_equality()
            && let Some(parent) = self.nearest_open_frame()
        {
            if let Some(children) = &mut parent.inverted_children {
                children.push(start);
            }
            if let Some(children) = &mut parent.rescue_modifier_children {
                open_rescue_body = *children == 1;
                *children += 1;
            }
        }
        if open_rescue_body {
            self.tokens.push(Token::Open(Kind::RescueBody));
        }
        self.record_range(&node.location(), Cut::Child);
    }

    fn record_range(&mut self, location: &Location<'pr>, cut: Cut) {
        if let Some(parent) = self.nearest_open_frame() {
            parent
                .children
                .push((location.start_offset(), location.end_offset(), cut));
        }
    }

    /// Cut a heredoc's body and terminator out of every open node's own
    /// text. Prism's heredoc location covers only the opener, so an
    /// ancestor that ends after the body (`x = (<<~A … A\n)`) would
    /// otherwise own the body's bytes; the body is already fingerprinted by
    /// the heredoc's value or parts.
    fn record_heredoc_body(&mut self, node: &Node<'pr>) {
        let body = match node {
            Node::StringNode { .. } => {
                let string = node.as_string_node().unwrap();
                string
                    .opening_loc()
                    .filter(|opening| opening.as_slice().starts_with(b"<<"))
                    .zip(string.closing_loc())
                    .map(|(_, closing)| (string.content_loc().start_offset(), closing))
            }
            Node::XStringNode { .. } => {
                let string = node.as_x_string_node().unwrap();
                string
                    .opening_loc()
                    .as_slice()
                    .starts_with(b"<<")
                    .then(|| (string.content_loc().start_offset(), string.closing_loc()))
            }
            Node::InterpolatedStringNode { .. } => {
                let string = node.as_interpolated_string_node().unwrap();
                match (string.opening_loc(), string.closing_loc()) {
                    (Some(opening), Some(closing)) if opening.as_slice().starts_with(b"<<") => {
                        let start = string
                            .parts()
                            .iter()
                            .next()
                            .map_or(closing.start_offset(), |part| {
                                part.location().start_offset()
                            });
                        Some((start, closing))
                    }
                    _ => None,
                }
            }
            Node::InterpolatedXStringNode { .. } => {
                let string = node.as_interpolated_x_string_node().unwrap();
                let closing = string.closing_loc();
                string.opening_loc().as_slice().starts_with(b"<<").then(|| {
                    let start = string
                        .parts()
                        .iter()
                        .next()
                        .map_or(closing.start_offset(), |part| {
                            part.location().start_offset()
                        });
                    (start, closing)
                })
            }
            _ => None,
        };
        let Some((start, closing)) = body else {
            return;
        };
        let end = closing.end_offset();
        for slot in &mut self.frames {
            if let Slot::Open(frame) = slot {
                frame.children.push((start, end, Cut::Layout));
            }
        }
    }

    /// The innermost node that emits tokens.
    fn nearest_open_frame(&mut self) -> Option<&mut Frame<'pr>> {
        self.frames.iter_mut().rev().find_map(|slot| match slot {
            Slot::Open(frame) => Some(frame),
            Slot::Silent
            | Slot::ConditionalScopeLiteral
            | Slot::Implicit
            | Slot::Transparent { .. } => None,
        })
    }

    /// Whether `node` is transparent in [`fingerprint_ignoring_grouping`].
    fn is_transparent(&mut self, node: &Node<'pr>) -> bool {
        match node {
            Node::ParenthesesNode { .. } => node.as_parentheses_node().unwrap().body().is_some(),
            Node::StatementsNode { .. } => {
                matches!(self.frames.last(), Some(Slot::Transparent { paren: true }))
            }
            Node::AndNode { .. } | Node::OrNode { .. } => {
                let own = kind(node);
                self.nearest_open_frame()
                    .is_some_and(|frame| frame.kind == own)
            }
            _ => false,
        }
    }

    fn canonical_own_text(&self, node: &Node<'pr>) -> Option<Vec<u8>> {
        if self.gemspec_keys
            && matches!(
                node,
                Node::StatementsNode { .. }
                    | Node::ParenthesesNode { .. }
                    | Node::EmbeddedStatementsNode { .. }
            )
        {
            // Parser's begin(builder.compstmt(...)) flattens only the implicit
            // statement list, never an explicit grouped begin within it.
            return Some(Vec::new());
        }
        if self.branch_equality() {
            // Presence belongs to the truth-value slots, not the spelling of
            // an Else wrapper. An empty else and no else are both absent;
            // explicit nil is a present child. Keep then-if distinct from elsif.
            if let Some(n) = node.as_if_node() {
                let has_else = n.subsequent().is_some_and(|next| {
                    next.as_else_node().is_none_or(|e| e.statements().is_some())
                });
                return Some(vec![u8::from(n.statements().is_some()), u8::from(has_else)]);
            }
            if let Some(n) = node.as_unless_node() {
                return Some(vec![
                    u8::from(n.else_clause().is_some_and(|e| e.statements().is_some())),
                    u8::from(n.statements().is_some()),
                ]);
            }
            if let Some(n) = node.as_case_node() {
                let mut shape = vec![
                    u8::from(n.predicate().is_some()),
                    u8::from(n.else_clause().is_some_and(|e| e.statements().is_some())),
                ];
                shape.extend_from_slice(&n.conditions().len().to_le_bytes());
                return Some(shape);
            }
            if let Some(n) = node.as_case_match_node() {
                // Unlike case/when, case/in preserves an empty_else node.
                let mut shape = vec![
                    u8::from(n.predicate().is_some()),
                    u8::from(n.else_clause().is_some()),
                ];
                shape.extend_from_slice(&n.conditions().len().to_le_bytes());
                return Some(shape);
            }
            if let Some(n) = node.as_when_node() {
                let mut shape = vec![u8::from(n.statements().is_some())];
                shape.extend_from_slice(&n.conditions().len().to_le_bytes());
                return Some(shape);
            }
            if let Some(n) = node.as_in_node() {
                return Some(vec![u8::from(n.statements().is_some())]);
            }
            if let Some(n) = node.as_begin_node()
                && (n.begin_keyword_loc().is_none() || self.require_literals || self.gemspec_keys)
                && n.rescue_clause().is_some()
            {
                // Pinned Require/Gemspec AST equality omits empty rescue else
                // even in kwbegin; neither profile drops explicit grouping.
                // Keep the explicit Begin kind and present nil/grouping children.
                return Some(vec![
                    u8::from(n.statements().is_some()),
                    u8::from(n.else_clause().is_some_and(|e| e.statements().is_some())),
                    u8::from(n.ensure_clause().is_some()),
                ]);
            }
            if node.as_rescue_modifier_node().is_some() {
                return Some(vec![1, 0, 0]);
            }
            if let Some(n) = node.as_rescue_node() {
                return Some(rescue_body_shape(
                    n.exceptions().len(),
                    n.reference().is_some(),
                    n.statements().is_some(),
                    n.subsequent().is_some(),
                ));
            }
            // Keyword/modifier and optional `do` are layout. Begin-modifier
            // loops are distinct post-test node kinds in parser's AST.
            if let Some(n) = node.as_while_node() {
                return Some(vec![u8::from(n.is_begin_modifier())]);
            }
            if let Some(n) = node.as_until_node() {
                return Some(vec![u8::from(n.is_begin_modifier())]);
            }
            if node.as_for_node().is_some() {
                return Some(Vec::new());
            }
            if let Some(n) = node.as_def_node() {
                // Normal/endless def and defs differ only in delimiters. Keep
                // the name and all optional child slots (especially receiver).
                let mut shape = vec![
                    u8::from(n.receiver().is_some()),
                    u8::from(n.parameters().is_some()),
                    u8::from(n.body().is_some()),
                ];
                if self.require_literals {
                    shape.extend_from_slice(
                        &self
                            .line_file
                            .unwrap()
                            .match_pattern_source(n.name().as_slice()),
                    );
                } else {
                    shape.extend_from_slice(n.name().as_slice());
                }
                return Some(shape);
            }
        }
        if self.branch_equality()
            && (matches!(
                node,
                Node::ElseNode { .. }
                    | Node::LambdaNode { .. }
                    | Node::BlockParametersNode { .. }
                    | Node::ArgumentsNode { .. }
            ) || is_bare_lambda_block(node))
        {
            // Child nodes retain structure and ordering. Conditional keywords
            // vs `?:`, lambda/parameter delimiters and argument punctuation
            // (including the `] =` inside index-write Arguments) are layout.
            return Some(Vec::new());
        }
        if (self.duplicate_equality.is_some() || self.require_literals)
            && let Some(regexp) = node.as_interpolated_match_last_line_node()
        {
            return Some(regexp_options(regexp.closing_loc().as_slice()));
        }
        if self.loop_collection_equality {
            if let Some(call) = node.as_call_node() {
                // Child tokens retain receiver/argument/block structure; keep
                // safe navigation distinct but discard selector punctuation.
                let mut selector = vec![u8::from(call.is_safe_navigation())];
                if self.require_literals {
                    selector.extend_from_slice(
                        &self
                            .line_file
                            .unwrap()
                            .match_pattern_source(call.name().as_slice()),
                    );
                } else {
                    selector.extend_from_slice(call.name().as_slice());
                }
                return Some(selector);
            }
            if let Some(regexp) = node.as_interpolated_regular_expression_node() {
                return Some(regexp_options(regexp.closing_loc().as_slice()));
            }
        }
        if self.case_subject_equality
            && node.as_call_node().is_some_and(|call| {
                call.name().as_slice() == b"!"
                    && call.receiver().is_some()
                    && !call.is_safe_navigation()
                    && call.arguments().is_none()
                    && call.block().is_none()
            })
        {
            return Some(vec![0, b'!']);
        }
        None
    }

    /// Cut the open node's own tokens whose spelling RuboCop's AST does not
    /// keep: `and`/`&&`, `not`/`!`, `::`/`.` calls, `=>` in pairs, and
    /// `do`/`end` vs braces.
    fn record_own_spellings(&mut self, node: &Node<'pr>) {
        match node {
            Node::AndNode { .. } => {
                let operator = node.as_and_node().unwrap().operator_loc();
                self.record_range(&operator, Cut::Layout);
            }
            Node::OrNode { .. } => {
                let operator = node.as_or_node().unwrap().operator_loc();
                self.record_range(&operator, Cut::Layout);
            }
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                if let Some(operator) = call.call_operator_loc()
                    && operator.as_slice() == b"::"
                {
                    self.record_range(&operator, Cut::Replace(b"."));
                }
                if let Some(message) = call.message_loc()
                    && message.as_slice() == b"not"
                {
                    self.record_range(&message, Cut::Replace(b"!"));
                }
            }
            Node::AssocNode { .. } => {
                if let Some(operator) = node.as_assoc_node().unwrap().operator_loc() {
                    self.record_range(&operator, Cut::Layout);
                }
            }
            Node::BlockNode { .. } => {
                let block = node.as_block_node().unwrap();
                self.record_range(&block.opening_loc(), Cut::Layout);
                self.record_range(&block.closing_loc(), Cut::Layout);
            }
            Node::LambdaNode { .. } => {
                let lambda = node.as_lambda_node().unwrap();
                self.record_range(&lambda.opening_loc(), Cut::Layout);
                self.record_range(&lambda.closing_loc(), Cut::Layout);
            }
            _ => {}
        }
    }
}

fn rescue_body_shape(exceptions: usize, reference: bool, body: bool, subsequent: bool) -> Vec<u8> {
    let mut shape = vec![u8::from(reference), u8::from(body), u8::from(subsequent)];
    shape.extend_from_slice(&exceptions.to_le_bytes());
    shape
}

impl<'pr> NodeVisitor<'pr> for FingerprintBuilder<'_, 'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        if matches!(self.frames.last(), Some(Slot::ConditionalScopeLiteral)) {
            self.record_heredoc_body(&node);
            self.frames.push(Slot::ConditionalScopeLiteral);
            return;
        }
        if self.scope_literal(&node) {
            self.frames.push(Slot::ConditionalScopeLiteral);
            return;
        }
        if self.file_operation_local(&node) {
            self.frames.push(Slot::Silent);
            return;
        }
        let location = node.location();
        // Empty block parameters (`-> () {}`, `{ || }`) are layout, as in
        // RuboCop's AST: keep the `||` out of the block's own text, emit
        // nothing.
        if is_empty_block_parameters(&node) {
            self.record_range(&location, Cut::Layout);
            self.frames.push(Slot::Silent);
            return;
        }
        self.record_heredoc_body(&node);
        // Prism wraps shorthand keyword values in an ImplicitNode; RuboCop
        // sees only the underlying local/constant read or method call. Let
        // that child supply both its tokens and its range in the pair.
        if node.as_implicit_node().is_some() {
            self.frames.push(Slot::Implicit);
            return;
        }
        if self.branch_equality() && self.branch_transparent(&node) {
            self.frames.push(Slot::Transparent { paren: false });
            return;
        }
        if self.regrouping.is_some() && self.is_transparent(&node) {
            let paren = node.as_parentheses_node().is_some();
            if !paren {
                // A flattened operator: its spelling is layout.
                self.record_own_spellings(&node);
            }
            self.frames.push(Slot::Transparent { paren });
            return;
        }
        self.record_child(&node);
        let node_kind = if self.gemspec_keys
            && matches!(
                node,
                Node::StatementsNode { .. }
                    | Node::ParenthesesNode { .. }
                    | Node::EmbeddedStatementsNode { .. }
            ) {
            Kind::GemspecBegin
        } else if self.require_literals && node.as_interpolated_x_string_node().is_some() {
            Kind::RequireXString
        } else if self.branch_equality()
            && (node.as_lambda_node().is_some() || is_bare_lambda_block(&node))
        {
            Kind::Lambda
        } else if self.branch_equality()
            && matches!(node, Node::IfNode { .. } | Node::UnlessNode { .. })
        {
            Kind::Conditional
        } else if self.branch_equality() && node.as_case_node().is_some() {
            Kind::Case
        } else if self.branch_equality() && node.as_case_match_node().is_some() {
            Kind::CaseMatch
        } else if self.branch_equality()
            && (node.as_rescue_modifier_node().is_some()
                || node.as_begin_node().is_some_and(|n| {
                    n.begin_keyword_loc().is_none() && n.rescue_clause().is_some()
                }))
        {
            Kind::Rescue
        } else if self.branch_equality() && node.as_rescue_node().is_some() {
            Kind::RescueBody
        } else {
            kind(&node)
        };
        self.tokens.push(Token::Open(node_kind));
        let source = location.as_slice();
        let source = if matches!(self.frames.last(), Some(Slot::Implicit)) {
            source.strip_suffix(b":").unwrap_or(source)
        } else {
            source
        };
        self.frames.push(Slot::Open(Frame {
            kind: node_kind,
            source,
            start: location.start_offset(),
            children: Vec::new(),
            canonical_own_text: self.canonical_own_text(&node),
            source_regexp_parts: (self.require_literals || self.gemspec_keys)
                && (node.as_interpolated_regular_expression_node().is_some()
                    || node.as_interpolated_match_last_line_node().is_some()),
            protected_rescue_else: (self.require_literals || self.gemspec_keys)
                && node
                    .as_begin_node()
                    .is_some_and(|n| n.rescue_clause().is_some()),
            inverted_children: (self.branch_equality() && node.as_unless_node().is_some())
                .then(Vec::new),
            receiver_parentheses: if self.branch_equality() {
                node.as_def_node().and_then(|n| n.receiver()).map(|r| {
                    let loc = r.location();
                    (loc.start_offset(), loc.end_offset())
                })
            } else {
                None
            },
            flat_kwbegin: self.branch_equality()
                && node.as_begin_node().is_some_and(|n| {
                    n.begin_keyword_loc().is_some()
                        && n.rescue_clause().is_none()
                        && n.ensure_clause().is_none()
                }),
            rescue_modifier_children: (self.branch_equality()
                && node.as_rescue_modifier_node().is_some())
            .then_some(0),
            // Delimiters and separators only: `"…"` vs `%Q(…)`, `#{…}`,
            // `#@var`, `[a, b]` vs `%w[a b]`, `{ a: 1 }` vs `a: 1`.
            ignore_own_text: matches!(
                node,
                Node::InterpolatedStringNode { .. }
                    | Node::InterpolatedSymbolNode { .. }
                    | Node::InterpolatedXStringNode { .. }
                    | Node::EmbeddedStatementsNode { .. }
                    | Node::EmbeddedVariableNode { .. }
                    | Node::ArrayNode { .. }
                    | Node::HashNode { .. }
                    | Node::KeywordHashNode { .. }
            ),
        }));
        self.record_own_spellings(&node);
    }

    fn leave(&mut self) {
        let Some(Slot::Open(frame)) = self.frames.pop() else {
            return;
        };
        if let Some(children) = &frame.inverted_children
            && let [_, body, else_body] = children.as_slice()
        {
            self.tokens[*body..].rotate_left(else_body - body);
        }
        if let Some(children) = frame.rescue_modifier_children {
            debug_assert_eq!(children, 2);
            self.tokens
                .push(Token::Close(rescue_body_shape(0, false, true, false)));
        }
        let own = if let Some(text) = frame.canonical_own_text {
            text
        } else if frame.ignore_own_text {
            Vec::new()
        } else {
            let own = own_text(&frame);
            if self.require_literals {
                // Canonical shape/numeric bytes and decoded escape results are
                // not source text. Only the remaining owned source is projected.
                self.line_file
                    .unwrap()
                    .match_pattern_source(&own)
                    .into_owned()
            } else {
                own
            }
        };
        let own = if self.duplicate_equality.is_some() {
            self.line_file
                .unwrap()
                .match_pattern_source(&own)
                .into_owned()
        } else {
            own
        };
        self.tokens.push(Token::Close(own));
    }

    fn leaf(&mut self, node: Node<'pr>) {
        if self.gemspec_keys && node.as_implicit_rest_node().is_some() {
            // `|s,|` and `|s|` translate to the same args(arg(:s)) AST.
            // This matters for enclosing specification hash equality too.
            self.record_range(&node.location(), Cut::Layout);
            return;
        }
        if matches!(self.frames.last(), Some(Slot::ConditionalScopeLiteral)) {
            self.record_heredoc_body(&node);
            return;
        }
        if self.scope_literal(&node) || self.file_operation_local(&node) {
            return;
        }
        self.record_heredoc_body(&node);
        self.record_child(&node);
        if self.require_literals
            && let Some(string) = node.as_x_string_node()
        {
            // Parser always wraps command text in xstr, including zero/one
            // parts. Static Prism commands still retain physical line parts.
            self.tokens.push(Token::Open(Kind::RequireXString));
            let raw = string.content_loc();
            let raw = raw.as_slice();
            // A single-line nonempty token (e.g. an empty Unicode escape)
            // still supplies str(""); truly empty/multiline-empty content
            // supplies no children. Keep the xstr wrapper in either case.
            if !raw.is_empty() && (!raw.contains(&b'\n') || !string.unescaped().is_empty()) {
                for value in
                    require_literal::parts(raw, string.unescaped(), false, self.line_file.unwrap())
                {
                    self.tokens.push(Token::Leaf(Kind::String, value));
                }
            }
            self.tokens.push(Token::Close(Vec::new()));
            return;
        }
        if (self.require_literals || self.gemspec_keys)
            && let Some(string) = node.as_string_node()
            && matches!(self.frames.last(), Some(Slot::Open(frame)) if frame.source_regexp_parts)
        {
            let value = require_literal::regexp_source(string.unescaped(), self.line_file.unwrap());
            // Require retains translator physical-line tokens. Gemspec keeps
            // its existing part shape and changes source alignment only.
            if self.require_literals && string.content_loc().as_slice().contains(&b'\n') {
                for line in value.split_inclusive(|&b| b == b'\n') {
                    self.tokens.push(Token::Leaf(Kind::String, line.to_vec()));
                }
            } else {
                self.tokens.push(Token::Leaf(Kind::String, value));
            }
            return;
        }
        if self.require_literals
            && let Some(string) = node.as_string_node()
        {
            let opening = string.opening_loc();
            let single = opening.as_ref().is_some_and(|o| {
                o.as_slice().starts_with(b"'")
                    || o.as_slice().starts_with(b"%q")
                    || o.as_slice().starts_with(b"<<'")
            });
            let mut parts = require_literal::parts(
                string.content_loc().as_slice(),
                string.unescaped(),
                single,
                self.line_file.unwrap(),
            );
            // Empty quoted strings are str(""); empty percent/heredoc
            // literals are zero-child dstrs. For multiline content the pinned
            // translator emits tokens by iterating decoded lines: an empty
            // value has no lines, even if Unicode escapes surround a removed
            // continuation. Do not synthesize an empty str token in that case.
            if string.unescaped().is_empty()
                && opening.as_ref().is_some_and(|o| {
                    !matches!(o.as_slice(), b"'" | b"\"")
                        || string.content_loc().as_slice().contains(&b'\n')
                })
            {
                parts.clear();
            }
            let dynamic = parts.len() != 1 && opening.is_some();
            if dynamic {
                self.tokens.push(Token::Open(Kind::InterpolatedString));
            }
            for value in parts {
                self.tokens.push(Token::Leaf(Kind::String, value));
            }
            if dynamic {
                self.tokens.push(Token::Close(Vec::new()));
            }
            return;
        }
        if self.require_literals
            && let Some(symbol) = node.as_symbol_node()
        {
            let opening = symbol.opening_loc();
            let single = opening.as_ref().is_some_and(|o| {
                matches!(o.as_slice(), b":'" | b"'") || o.as_slice().starts_with(b"%s")
            });
            let raw = symbol.value_loc();
            let raw = raw.as_ref().map_or(symbol.unescaped(), |r| r.as_slice());
            let mut parts =
                require_literal::parts(raw, symbol.unescaped(), single, self.line_file.unwrap());
            // Assoc labels bypass visit_symbol_node in the pinned translator:
            // pair_keyword/pair_quoted keep one static symbol even for empty
            // or multiline text. Project source glyphs as usual, but do not
            // impose ordinary symbol literal part boundaries or dsym shape.
            if symbol
                .closing_loc()
                .is_some_and(|c| c.as_slice().ends_with(b":"))
            {
                self.tokens.push(Token::Leaf(kind(&node), parts.concat()));
                return;
            }
            // The pinned translator composes a quoted/percent symbol from
            // zero children when its source payload is empty, or when its
            // multiline decoded value has no lines. A nonempty single-line
            // empty Unicode escape instead supplies one str("") child and
            // composes to sym(""). Runtime symbol equality is not AST#to_s.
            if symbol.closing_loc().is_some()
                && (raw.is_empty() || (raw.contains(&b'\n') && symbol.unescaped().is_empty()))
            {
                parts.clear();
            }
            let dynamic = parts.len() != 1;
            if dynamic {
                self.tokens.push(Token::Open(Kind::InterpolatedSymbol));
            }
            for value in parts {
                self.tokens.push(Token::Leaf(
                    if dynamic { Kind::String } else { kind(&node) },
                    value,
                ));
            }
            if dynamic {
                self.tokens.push(Token::Close(Vec::new()));
            }
            return;
        }
        if self.branch_equality() && node.as_source_file_node().is_some() {
            // The parser's value is the supplied input name, not its rebased
            // display path or a canonicalized disk/symlink target.
            let path = self
                .line_file
                .expect("branch equality requires a source file")
                .source_path();
            self.tokens.push(Token::Leaf(
                Kind::String,
                path.as_os_str().as_encoded_bytes().to_vec(),
            ));
            return;
        }
        if self.case_subject_equality && node.as_source_encoding_node().is_some() {
            // Exactly the ordinary fingerprint of the unqualified constant
            // path Encoding::UTF_8. Qualified ::Encoding stays distinct.
            self.tokens.extend([
                Token::Open(Kind::ConstantPath),
                Token::Leaf(Kind::ConstantRead, b"Encoding".to_vec()),
                Token::Close(b"\0::UTF_8".to_vec()),
            ]);
            return;
        }
        let value = match (self.line_file, self.regrouping, &node) {
            (Some(file), _, Node::SourceLineNode { .. }) => file
                .line_of(node.location().start_offset())
                .to_le_bytes()
                .to_vec(),
            (_, Some(source), Node::SourceLineNode { .. }) => {
                let offset = node.location().start_offset();
                let line = memchr::memchr_iter(b'\n', &source[..offset]).count() + 1;
                line.to_le_bytes().to_vec()
            }
            _ if (self.numeric_zero_equality || self.branch_equality())
                && node.as_rational_node().is_some() =>
            {
                // Prism already reduces this exact ratio, independently of its
                // decimal/base spelling. Do not pass through f64 (large and
                // closely spaced rationals must remain distinct).
                let rational = node.as_rational_node().unwrap();
                let numerator = rational.numerator();
                let zero = numerator.to_u32_digits().1.iter().all(|&d| d == 0);
                let mut value = integer_value(numerator);
                if zero {
                    value[0] = 0;
                }
                let mut ratio = value.len().to_le_bytes().to_vec();
                ratio.extend(value);
                ratio.extend(integer_value(rational.denominator()));
                ratio
            }
            _ if (self.duplicate_equality.is_some() || self.require_literals)
                && node.as_match_last_line_node().is_some() =>
            {
                let regexp = node.as_match_last_line_node().unwrap();
                let mut value = if self.require_literals || self.gemspec_keys {
                    require_literal::regexp_source(regexp.unescaped(), self.line_file.unwrap())
                } else {
                    self.line_file
                        .unwrap()
                        .match_pattern_source(regexp.unescaped())
                        .into_owned()
                };
                value.push(0);
                value.extend(regexp_options(regexp.closing_loc().as_slice()));
                value
            }
            _ if self.loop_collection_equality && node.as_regular_expression_node().is_some() => {
                let regexp = node.as_regular_expression_node().unwrap();
                let mut value = if self.require_literals || self.gemspec_keys {
                    require_literal::regexp_source(regexp.unescaped(), self.line_file.unwrap())
                } else if self.duplicate_equality.is_some() {
                    self.line_file
                        .unwrap()
                        .match_pattern_source(regexp.unescaped())
                        .into_owned()
                } else {
                    regexp.unescaped().to_vec()
                };
                value.push(0);
                value.extend(regexp_options(regexp.closing_loc().as_slice()));
                value
            }
            _ if self.duplicate_equality.is_some()
                && (is_variable(&node) || node.as_constant_read_node().is_some()) =>
            {
                self.line_file
                    .unwrap()
                    .match_pattern_source(&leaf_value(&node))
                    .into_owned()
            }
            _ if self.require_literals
                && !matches!(
                    node,
                    Node::IntegerNode { .. }
                        | Node::FloatNode { .. }
                        | Node::RationalNode { .. }
                        | Node::NumberedParametersNode { .. }
                        | Node::ItParametersNode { .. }
                ) =>
            {
                // Remaining leaves contain source names/tokens, not binary
                // numeric or arity metadata. Literals took their path above.
                self.line_file
                    .unwrap()
                    .match_pattern_source(&leaf_value(&node))
                    .into_owned()
            }
            _ => leaf_value(&node),
        };
        let value = if ((self.numeric_zero_equality || self.branch_equality())
            && node
                .as_integer_node()
                .is_some_and(|n| n.value().to_u32_digits().1.iter().all(|&digit| digit == 0)))
            || (self.numeric_zero_equality
                && node.as_float_node().is_some_and(|n| n.value() == 0.0))
        {
            // Node kinds remain distinct: integer 0 does not match float 0.0.
            vec![0]
        } else {
            value
        };
        let value = if matches!(self.frames.last(), Some(Slot::Implicit)) {
            value.strip_suffix(b":").unwrap_or(&value).to_vec()
        } else {
            value
        };
        self.tokens.push(Token::Leaf(kind(&node), value));
    }
}

/// Prism represents an arrow directly, but parser represents both forms as
/// a block on a receiverless, argumentless `lambda` send. Other calls differ.
fn is_bare_lambda_block(node: &Node<'_>) -> bool {
    node.as_call_node().is_some_and(|call| {
        call.name().as_slice() == b"lambda"
            && call.receiver().is_none()
            && call.arguments().is_none()
            && call
                .block()
                .is_some_and(|block| block.as_block_node().is_some())
    })
}

/// `||` or `()` block parameters: no parameters and no block-locals.
fn is_empty_block_parameters(node: &Node<'_>) -> bool {
    node.as_block_parameters_node().is_some_and(|parameters| {
        parameters.parameters().is_none() && parameters.locals().iter().next().is_none()
    })
}

/// The node's source minus its children's, normalized; gaps are separated by
/// a NUL so their positions stay significant.
fn own_text(frame: &Frame<'_>) -> Vec<u8> {
    let end = frame.start + frame.source.len();
    let mut children = frame.children.clone();
    children.sort_unstable();
    let mut text = Vec::new();
    let mut cursor = frame.start;
    for (child_start, child_end, cut) in children {
        let gap_end = child_start.clamp(cursor, end);
        push_normalized(
            &mut text,
            &frame.source[cursor - frame.start..gap_end - frame.start],
        );
        match cut {
            Cut::Child => text.push(0),
            Cut::Layout => {}
            Cut::Replace(spelling) => text.extend_from_slice(spelling),
        }
        cursor = child_end.clamp(cursor, end);
    }
    push_normalized(&mut text, &frame.source[cursor - frame.start..]);
    text
}

/// Append `gap` without layout-only bytes: whitespace, grouping parentheses,
/// statement separators, line continuations and comments. (Literal text never
/// reaches a gap: it lives in leaf nodes, and the `#` of interpolation belongs
/// to nodes whose own text is ignored.)
fn push_normalized(out: &mut Vec<u8>, gap: &[u8]) {
    let mut bytes = gap.iter().peekable();
    while let Some(&byte) = bytes.next() {
        match byte {
            b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'\x0b' | b'(' | b')' | b';' => {}
            b'\\' if matches!(bytes.peek(), Some(b'\n' | b'\r')) => {}
            b'#' => while bytes.next_if(|&&next| next != b'\n').is_some() {},
            _ => out.push(byte),
        }
    }
}

/// The value of a leaf node: decoded for literals whose spelling varies
/// (`'a'` and `"a"`, `:a` and `a:`), the source text otherwise.
fn leaf_value(node: &Node<'_>) -> Vec<u8> {
    match node {
        Node::StringNode { .. } => node.as_string_node().unwrap().unescaped().to_vec(),
        Node::SymbolNode { .. } => node.as_symbol_node().unwrap().unescaped().to_vec(),
        Node::XStringNode { .. } => node.as_x_string_node().unwrap().unescaped().to_vec(),
        Node::IntegerNode { .. } => integer_value(node.as_integer_node().unwrap().value()),
        Node::FloatNode { .. } => node
            .as_float_node()
            .unwrap()
            .value()
            .to_bits()
            .to_le_bytes()
            .to_vec(),
        Node::RegularExpressionNode { .. } => {
            let regexp = node.as_regular_expression_node().unwrap();
            let mut out = regexp.unescaped().to_vec();
            out.push(0);
            // The closing delimiter followed by the options, e.g. `/ix`.
            out.extend_from_slice(regexp.closing_loc().as_slice().get(1..).unwrap_or(&[]));
            out
        }
        // Implicit block parameters (`_1`, `it`): only the arity matters.
        Node::NumberedParametersNode { .. } => {
            vec![node.as_numbered_parameters_node().unwrap().maximum()]
        }
        Node::ItParametersNode { .. } => Vec::new(),
        // `__LINE__` differs per line; never treat two of them as equal.
        Node::SourceLineNode { .. } => node.location().start_offset().to_le_bytes().to_vec(),
        _ => node.location().as_slice().to_vec(),
    }
}

/// RuboCop stores regexp options in a sorted, deduplicated regopt node.
fn regexp_options(closing: &[u8]) -> Vec<u8> {
    let mut options = closing.get(1..).unwrap_or_default().to_vec();
    options.sort_unstable();
    options.dedup();
    options
}

fn integer_value(integer: ruby_prism::Integer<'_>) -> Vec<u8> {
    let (negative, digits) = integer.to_u32_digits();
    let mut out = vec![u8::from(negative)];
    for digit in digits {
        out.extend_from_slice(&digit.to_le_bytes());
    }
    out
}

/// Render arbitrary-precision little-endian base-2^32 limbs without truncation.
pub(crate) fn unsigned_integer_decimal(digits: &[u32]) -> String {
    let mut decimal = vec![0_u64];
    for digit in digits.iter().rev() {
        let mut carry = u64::from(*digit);
        for part in &mut decimal {
            let next = (*part << 32) + carry;
            *part = next % 1_000_000_000;
            carry = next / 1_000_000_000;
        }
        while carry != 0 {
            decimal.push(carry % 1_000_000_000);
            carry /= 1_000_000_000;
        }
    }
    let mut number = decimal.pop().unwrap().to_string();
    for part in decimal.iter().rev() {
        use std::fmt::Write;
        let _ = write!(number, "{part:09}");
    }
    number
}

/// The Prism-to-parser adapter folds one prefix minus applied directly to a
/// numeric literal, not an arbitrary chain of calls or explicit `number.-@`.
/// This is for detection only: the actual Ruby operation still dispatches.
pub(crate) fn negated_numeric_literal<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    let call = node.as_call_node()?;
    if call.name().as_slice() != b"-@"
        || call.call_operator_loc().is_some()
        || call.arguments().is_some()
        || call.block().is_some()
    {
        return None;
    }
    let receiver = call.receiver()?;
    matches!(
        receiver,
        Node::IntegerNode { .. }
            | Node::FloatNode { .. }
            | Node::RationalNode { .. }
            | Node::ImaginaryNode { .. }
    )
    .then_some(receiver)
}

/// RuboCop's `recursive_basic_literal?`: a literal built only from basic
/// literals, possibly combined with comparison operators, `*`, `!`, `<=>`,
/// `&&`/`||`, parentheses, collections, ranges, and interpolation of such.
pub fn is_recursive_basic_literal(node: &Node<'_>) -> bool {
    match node {
        Node::StringNode { .. }
        | Node::XStringNode { .. }
        | Node::SymbolNode { .. }
        | Node::IntegerNode { .. }
        | Node::FloatNode { .. }
        | Node::RationalNode { .. }
        | Node::ImaginaryNode { .. }
        | Node::TrueNode { .. }
        | Node::FalseNode { .. }
        | Node::NilNode { .. }
        | Node::RegularExpressionNode { .. }
        | Node::SourceFileNode { .. }
        | Node::SourceLineNode { .. } => true,
        Node::CallNode { .. } => {
            let call = node.as_call_node().unwrap();
            const METHODS: [&[u8]; 10] = [
                b"==", b"===", b"!=", b"<=", b">=", b">", b"<", b"*", b"!", b"<=>",
            ];
            // RuboCop represents safe navigation as `csend`, not `send`.
            !call.is_safe_navigation()
                && METHODS.contains(&call.name().as_slice())
                && call.block().is_none()
                && call
                    .receiver()
                    .is_some_and(|receiver| is_recursive_basic_literal(&receiver))
                && call.arguments().is_none_or(|arguments| {
                    arguments
                        .arguments()
                        .iter()
                        .all(|argument| is_recursive_basic_literal(&argument))
                })
        }
        Node::AndNode { .. } => {
            let and = node.as_and_node().unwrap();
            is_recursive_basic_literal(&and.left()) && is_recursive_basic_literal(&and.right())
        }
        Node::OrNode { .. } => {
            let or = node.as_or_node().unwrap();
            is_recursive_basic_literal(&or.left()) && is_recursive_basic_literal(&or.right())
        }
        Node::InterpolatedStringNode { .. } => node
            .as_interpolated_string_node()
            .unwrap()
            .parts()
            .iter()
            .all(|part| is_recursive_basic_literal(&part)),
        Node::InterpolatedSymbolNode { .. } => node
            .as_interpolated_symbol_node()
            .unwrap()
            .parts()
            .iter()
            .all(|part| is_recursive_basic_literal(&part)),
        Node::InterpolatedXStringNode { .. } => node
            .as_interpolated_x_string_node()
            .unwrap()
            .parts()
            .iter()
            .all(|part| is_recursive_basic_literal(&part)),
        Node::InterpolatedRegularExpressionNode { .. } => node
            .as_interpolated_regular_expression_node()
            .unwrap()
            .parts()
            .iter()
            .all(|part| is_recursive_basic_literal(&part)),
        Node::EmbeddedStatementsNode { .. } => node
            .as_embedded_statements_node()
            .unwrap()
            .statements()
            .is_none_or(|statements| {
                statements
                    .body()
                    .iter()
                    .all(|statement| is_recursive_basic_literal(&statement))
            }),
        Node::StatementsNode { .. } => node
            .as_statements_node()
            .unwrap()
            .body()
            .iter()
            .all(|statement| is_recursive_basic_literal(&statement)),
        Node::ParenthesesNode { .. } => node
            .as_parentheses_node()
            .unwrap()
            .body()
            .is_none_or(|body| is_recursive_basic_literal(&body)),
        Node::ArrayNode { .. } => node
            .as_array_node()
            .unwrap()
            .elements()
            .iter()
            .all(|element| is_recursive_basic_literal(&element)),
        Node::HashNode { .. } => node
            .as_hash_node()
            .unwrap()
            .elements()
            .iter()
            .all(|element| is_recursive_basic_literal(&element)),
        Node::KeywordHashNode { .. } => node
            .as_keyword_hash_node()
            .unwrap()
            .elements()
            .iter()
            .all(|element| is_recursive_basic_literal(&element)),
        Node::AssocNode { .. } => {
            let pair = node.as_assoc_node().unwrap();
            is_recursive_basic_literal(&pair.key()) && is_recursive_basic_literal(&pair.value())
        }
        Node::RangeNode { .. } => {
            let range = node.as_range_node().unwrap();
            range
                .left()
                .is_none_or(|left| is_recursive_basic_literal(&left))
                && range
                    .right()
                    .is_none_or(|right| is_recursive_basic_literal(&right))
        }
        _ => false,
    }
}

/// RuboCop's `literal?`: the node's own type is a literal (contents are not
/// inspected, so `[foo]` counts).
pub fn is_literal(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::StringNode { .. }
            | Node::InterpolatedStringNode { .. }
            | Node::XStringNode { .. }
            | Node::InterpolatedXStringNode { .. }
            | Node::SymbolNode { .. }
            | Node::InterpolatedSymbolNode { .. }
            | Node::IntegerNode { .. }
            | Node::FloatNode { .. }
            | Node::RationalNode { .. }
            | Node::ImaginaryNode { .. }
            | Node::ArrayNode { .. }
            | Node::HashNode { .. }
            | Node::KeywordHashNode { .. }
            | Node::RegularExpressionNode { .. }
            | Node::InterpolatedRegularExpressionNode { .. }
            | Node::RangeNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. }
            | Node::NilNode { .. }
            | Node::SourceFileNode { .. }
            | Node::SourceLineNode { .. }
    )
}

/// RuboCop's `basic_literal?`: a literal that is not composite (no
/// interpolation, collection, range, regexp or backticks).
pub fn is_basic_literal(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::StringNode { .. }
            | Node::SymbolNode { .. }
            | Node::IntegerNode { .. }
            | Node::FloatNode { .. }
            | Node::RationalNode { .. }
            | Node::ImaginaryNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. }
            | Node::NilNode { .. }
            | Node::SourceFileNode { .. }
            | Node::SourceLineNode { .. }
    )
}

/// RuboCop's `operator_method?`, for Prism's method names (`-@`, `!`, `[]`,
/// `[]=`, `==`, …).
pub fn is_operator_method(name: &[u8]) -> bool {
    const OPERATOR_METHODS: [&[u8]; 30] = [
        b"|", b"^", b"&", b"<=>", b"==", b"===", b"=~", b">", b">=", b"<", b"<=", b"<<", b">>",
        b"+", b"-", b"*", b"/", b"%", b"**", b"~", b"+@", b"-@", b"!@", b"~@", b"[]", b"[]=", b"!",
        b"!=", b"!~", b"`",
    ];
    OPERATOR_METHODS.contains(&name)
}

/// RuboCop's `variable?`: a local, instance, class or global variable read.
pub fn is_variable(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::LocalVariableReadNode { .. }
            | Node::ItLocalVariableReadNode { .. }
            | Node::InstanceVariableReadNode { .. }
            | Node::ClassVariableReadNode { .. }
            | Node::GlobalVariableReadNode { .. }
    )
}

/// Whether a standalone static symbol is `sym`, not a physically composed
/// `dsym`, in the pinned reference. Extracted unchanged from SymbolArray for
/// Naming/VariableNumber; quoted hash labels and percent-array elements have
/// different builder ownership, so callers handle those contexts separately.
pub(crate) fn is_reference_symbol(n: &Node<'_>, file: &SourceFile) -> bool {
    let Some(s) = n.as_symbol_node() else {
        return false;
    };
    // The pinned builder represents empty quoted/%s symbols as dsym.
    if s.unescaped().is_empty() {
        return false;
    }
    let Some(loc) = s.value_loc() else {
        return true;
    };
    let single = s
        .opening_loc()
        .is_some_and(|l| l.as_slice() == b":'" || l.as_slice().starts_with(b"%s"));
    let mut heads = file
        .character_offsets(loc.start_offset()..loc.end_offset())
        .peekable();
    while let Some(i) = heads.next() {
        if !single && file.bytes[i] == b'\\' {
            if let Some(j) = heads.next()
                && file.bytes[j] == b'\r'
                && heads.peek().is_some_and(|&k| file.bytes[k] == b'\n')
            {
                heads.next();
            }
        } else if file.bytes[i] == b'\n' && i + 1 < loc.end_offset() {
            return false;
        }
    }
    true
}

/// RuboCop's `const_type?`: a constant reference (`Foo`, `Foo::Bar`, `::Foo`).
/// `__ENCODING__` counts: RuboCop's AST spells it `Encoding::UTF_8`.
pub fn is_const(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::ConstantReadNode { .. }
            | Node::ConstantPathNode { .. }
            | Node::SourceEncodingNode { .. }
    )
}

/// RuboCop's `const_name`: the constant's written path without a leading
/// `::` (`Foo::Bar`). `None` for non-constants; a non-constant namespace
/// renders empty, as in RuboCop (`self::Foo` -> `::Foo`).
pub fn const_name(node: &Node<'_>) -> Option<String> {
    match node {
        Node::ConstantReadNode { .. } => Some(
            String::from_utf8_lossy(node.as_constant_read_node().unwrap().name().as_slice())
                .into_owned(),
        ),
        Node::ConstantPathNode { .. } => {
            let path = node.as_constant_path_node().unwrap();
            let short = short_const_name(node).unwrap_or_default();
            match path.parent() {
                None => Some(short),
                Some(parent) => Some(format!(
                    "{}::{short}",
                    const_name(&parent).unwrap_or_default()
                )),
            }
        }
        _ => None,
    }
}

/// The last segment of a constant reference (`Bar` in `Foo::Bar`).
pub fn short_const_name(node: &Node<'_>) -> Option<String> {
    match node {
        Node::ConstantReadNode { .. } => Some(
            String::from_utf8_lossy(node.as_constant_read_node().unwrap().name().as_slice())
                .into_owned(),
        ),
        Node::ConstantPathNode { .. } => node
            .as_constant_path_node()
            .unwrap()
            .name()
            .map(|name| String::from_utf8_lossy(name.as_slice()).into_owned()),
        _ => None,
    }
}

/// Whether `node` is `Name` or `::Name` for one of `names` (RuboCop's
/// `global_const?`).
pub fn is_global_const(node: &Node<'_>, names: &[&str]) -> bool {
    let top_level = match node {
        Node::ConstantReadNode { .. } => true,
        Node::ConstantPathNode { .. } => node.as_constant_path_node().unwrap().parent().is_none(),
        _ => false,
    };
    top_level && short_const_name(node).is_some_and(|name| names.contains(&name.as_str()))
}

/// Read-side `[]` send of a compound index assignment. Plain setters and
/// multiple-assignment targets have no getter. This borrows the original AST;
/// in particular it must not be used as a correctable literal expression.
pub(crate) struct IndexGetter<'pr> {
    pub receiver: Node<'pr>,
    pub arguments: Vec<Node<'pr>>,
    pub safe_navigation: bool,
}
pub(crate) fn compound_index_getter<'pr>(node: &Node<'pr>) -> Option<IndexGetter<'pr>> {
    macro_rules! getter {
        ($access:ident) => {{
            let n = node.$access().unwrap();
            let mut arguments: Vec<_> = n
                .arguments()
                .map(|a| a.arguments().iter().collect())
                .unwrap_or_default();
            if let Some(block) = n.block() {
                arguments.push(block.as_node());
            }
            IndexGetter {
                receiver: n.receiver()?,
                arguments,
                safe_navigation: n.is_safe_navigation(),
            }
        }};
    }
    Some(match node {
        Node::IndexOperatorWriteNode { .. } => getter!(as_index_operator_write_node),
        Node::IndexOrWriteNode { .. } => getter!(as_index_or_write_node),
        Node::IndexAndWriteNode { .. } => getter!(as_index_and_write_node),
        _ => return None,
    })
}

#[cfg(test)]
thread_local! {
    /// Number of argument nodes collected by `call_arguments` on this thread.
    pub(crate) static CALL_ARGUMENT_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A call's arguments as RuboCop's AST sees them: the positional ones, then
/// a block argument (`&blk`) as the last one. A literal block is not one.
pub fn call_arguments<'pr>(call: &CallNode<'pr>) -> Vec<Node<'pr>> {
    let mut arguments: Vec<Node<'pr>> = call
        .arguments()
        .map(|arguments| arguments.arguments().iter().collect())
        .unwrap_or_default();
    if let Some(block) = call
        .block()
        .filter(|block| block.as_block_argument_node().is_some())
    {
        arguments.push(block);
    }
    #[cfg(test)]
    CALL_ARGUMENT_VISITS.set(CALL_ARGUMENT_VISITS.get() + arguments.len());
    arguments
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether the first two statements of `source` are structurally equal.
    fn same(source: &str) -> bool {
        let result = ruby_prism::parse(source.as_bytes());
        assert_eq!(result.errors().count(), 0, "{source:?} must parse");
        let program = result.node();
        let statements = program.as_program_node().unwrap().statements().body();
        let first = statements.iter().next().unwrap();
        let second = statements.iter().nth(1).unwrap();
        same_node(&first, &second)
    }

    #[test]
    fn gemspec_interpolation_begins_preserve_grouping_without_changing_old_profiles() {
        for source in [
            r##"["#{x; y}"]; ["#{(x; y)}"]"##,
            r##"{k: "#{x; y}"}; {k: "#{(x; y)}"}"##,
            r##"[/#{x; y}/]; [/#{(x; y)}/]"##,
        ] {
            let file = SourceFile::new("test.rb".into(), source.into());
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0);
            let nodes: Vec<_> = parsed
                .node()
                .as_program_node()
                .unwrap()
                .statements()
                .body()
                .iter()
                .collect();
            let spans = std::collections::HashSet::new();
            let old_keys = |node| {
                vec![
                    fingerprint(node),
                    fingerprint_with_source_lines(node, &file),
                    fingerprint_with_integer_source_lines(node, &file),
                    fingerprint_for_duplicate(node, &file),
                    fingerprint_for_duplicate_condition(node, &file),
                    fingerprint_for_file_operation(node, &file, Some(&spans)),
                    fingerprint_for_require(node, &file, Some(&spans)),
                    fingerprint_for_self_assignment(node, &file, Some(&spans)),
                    fingerprint_for_comparison(node, &file),
                    fingerprint_for_detection(node, &file),
                    fingerprint_for_case_subject(node, &file),
                    fingerprint_for_conditional_scope(node, &file),
                    fingerprint_for_branch(node, &file),
                    fingerprint_ignoring_grouping(node, source.as_bytes()),
                ]
            };
            let before: Vec<_> = nodes.iter().map(&old_keys).collect();
            // Existing branch/scope profiles elide this boundary; the ordinary
            // structural/correction profiles already distinguish parentheses.
            for (i, (a, b)) in before[0].iter().zip(&before[1]).enumerate() {
                assert_eq!(
                    a == b,
                    matches!(i, 3..=7 | 11..=13),
                    "profile {i}: {source}"
                );
            }
            assert_ne!(
                fingerprint_for_gemspec_key(&nodes[0], &file, &spans),
                fingerprint_for_gemspec_key(&nodes[1], &file, &spans),
                "{source}"
            );
            assert_eq!(before, nodes.iter().map(old_keys).collect::<Vec<_>>());
        }
    }
    #[test]
    fn gemspec_shorthand_symbols_do_not_inherit_local_span_projection() {
        struct Spans(std::collections::HashSet<(usize, usize)>);
        impl<'pr> NodeVisitor<'pr> for Spans {
            fn enter(&mut self, node: Node<'pr>) {
                let loc = node.location();
                self.0.insert((loc.start_offset(), loc.end_offset()));
            }
            fn leave(&mut self) {}
        }
        let source = "x=0; {x:}; {x: x}; {x: x()}; {y: x}";
        let file = SourceFile::new("test.rb".into(), source.into());
        let parsed = ruby_prism::parse(source.as_bytes());
        assert_eq!(parsed.errors().count(), 0);
        let nodes: Vec<_> = parsed
            .node()
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .iter()
            .skip(1)
            .collect();
        let mut spans = Spans(Default::default());
        walk(&mut spans, &parsed.node());
        let old: Vec<_> = nodes
            .iter()
            .map(|n| fingerprint_for_duplicate(n, &file))
            .collect();
        assert_eq!(old[0], old[1]);
        let keys: Vec<_> = nodes
            .iter()
            .map(|n| fingerprint_for_gemspec_key(n, &file, &spans.0))
            .collect();
        assert_eq!(keys[0], keys[1]);
        assert_ne!(keys[0], keys[2]);
        assert_ne!(keys[0], keys[3]);
        assert_eq!(
            old,
            nodes
                .iter()
                .map(|n| fingerprint_for_duplicate(n, &file))
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn gemspec_it_key_identity_does_not_relax_existing_fingerprints() {
        struct Keys<'pr> {
            arrays: Vec<Node<'pr>>,
            spans: std::collections::HashSet<(usize, usize)>,
        }
        impl<'pr> NodeVisitor<'pr> for Keys<'pr> {
            fn enter(&mut self, node: Node<'pr>) {
                let loc = node.location();
                self.spans.insert((loc.start_offset(), loc.end_offset()));
                if node.as_array_node().is_some() {
                    self.arrays.push(node);
                }
            }
            fn leave(&mut self) {}
        }
        let source = "each { [it] }\nit = 1\n[it]\n[it()]\n";
        let file = SourceFile::new("test.rb".into(), source.into());
        let parsed = ruby_prism::parse(source.as_bytes());
        assert_eq!(parsed.errors().count(), 0);
        let mut keys = Keys {
            arrays: vec![],
            spans: Default::default(),
        };
        walk(&mut keys, &parsed.node());
        assert_eq!(keys.arrays.len(), 3);
        let old_keys = |node| {
            vec![
                fingerprint(node),
                fingerprint_with_source_lines(node, &file),
                fingerprint_with_integer_source_lines(node, &file),
                fingerprint_for_duplicate(node, &file),
                fingerprint_for_duplicate_condition(node, &file),
                fingerprint_for_file_operation(node, &file, Some(&keys.spans)),
                fingerprint_for_require(node, &file, Some(&keys.spans)),
                fingerprint_for_self_assignment(node, &file, Some(&keys.spans)),
                fingerprint_for_comparison(node, &file),
                fingerprint_for_detection(node, &file),
                fingerprint_for_case_subject(node, &file),
                fingerprint_for_conditional_scope(node, &file),
                fingerprint_for_branch(node, &file),
                fingerprint_ignoring_grouping(node, source.as_bytes()),
            ]
        };
        let before: Vec<_> = keys.arrays.iter().map(&old_keys).collect();
        for (a, b) in before[0].iter().zip(&before[1]) {
            assert_ne!(a, b);
        }
        let projected: Vec<_> = keys
            .arrays
            .iter()
            .map(|node| fingerprint_for_gemspec_key(node, &file, &keys.spans))
            .collect();
        assert_eq!(projected[0], projected[1]);
        assert_ne!(projected[0], projected[2]);
        assert_eq!(before, keys.arrays.iter().map(old_keys).collect::<Vec<_>>());
    }

    #[test]
    fn gemspec_regexp_source_alignment_is_opt_in_including_literal_wrappers() {
        struct Spans(std::collections::HashSet<(usize, usize)>);
        impl<'pr> NodeVisitor<'pr> for Spans {
            fn enter(&mut self, node: Node<'pr>) {
                let loc = node.location();
                self.0.insert((loc.start_offset(), loc.end_offset()));
            }
            fn leave(&mut self) {}
        }
        for (a, b) in [
            ("/\\e\u{301}/im", "/\\é/mi"),
            ("[/\\e\u{301}/im]", "[/\\é/mi]"),
            ("{x: /\\e\u{301}#{z}/im}", "{x: /\\é#{z}/mi}"),
        ] {
            let source = format!("# coding: UTF8-MAC\n{a}; {b}");
            let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0);
            let nodes: Vec<_> = parsed
                .node()
                .as_program_node()
                .unwrap()
                .statements()
                .body()
                .iter()
                .collect();
            let mut spans = Spans(Default::default());
            walk(&mut spans, &parsed.node());
            let old_keys = |node| {
                vec![
                    fingerprint(node),
                    fingerprint_with_source_lines(node, &file),
                    fingerprint_with_integer_source_lines(node, &file),
                    fingerprint_for_duplicate(node, &file),
                    fingerprint_for_duplicate_condition(node, &file),
                    fingerprint_for_file_operation(node, &file, Some(&spans.0)),
                    fingerprint_for_require(node, &file, Some(&spans.0)),
                    fingerprint_for_self_assignment(node, &file, Some(&spans.0)),
                    fingerprint_for_comparison(node, &file),
                    fingerprint_for_detection(node, &file),
                    fingerprint_for_case_subject(node, &file),
                    fingerprint_for_conditional_scope(node, &file),
                    fingerprint_for_branch(node, &file),
                    fingerprint_ignoring_grouping(node, source.as_bytes()),
                ]
            };
            let before: Vec<_> = nodes.iter().map(&old_keys).collect();
            for (i, (left, right)) in before[0].iter().zip(&before[1]).enumerate() {
                // Require already had this alignment; none of the other old
                // profiles gains Gemspec's new equality (especially fixes).
                assert_eq!(left == right, i == 6, "profile {i}: {source}");
            }
            assert_eq!(
                fingerprint_for_gemspec_key(&nodes[0], &file, &spans.0),
                fingerprint_for_gemspec_key(&nodes[1], &file, &spans.0)
            );
            assert_eq!(before, nodes.iter().map(old_keys).collect::<Vec<_>>());
        }
    }

    #[test]
    fn gemspec_protected_rescue_slots_are_opt_in_through_literal_wrappers() {
        struct Spans(std::collections::HashSet<(usize, usize)>);
        impl<'pr> NodeVisitor<'pr> for Spans {
            fn enter(&mut self, node: Node<'pr>) {
                let loc = node.location();
                self.0.insert((loc.start_offset(), loc.end_offset()));
            }
            fn leave(&mut self) {}
        }
        for template in ["EXPR", "[EXPR]", "{x: [EXPR]}"] {
            let a = template.replace("EXPR", "begin; x; rescue; y; else; end");
            let b = template.replace("EXPR", "begin; x; rescue; y; end");
            let source = format!("{a}; {b}");
            let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0);
            let nodes: Vec<_> = parsed
                .node()
                .as_program_node()
                .unwrap()
                .statements()
                .body()
                .iter()
                .collect();
            let mut spans = Spans(Default::default());
            walk(&mut spans, &parsed.node());
            let old_keys = |node| {
                vec![
                    fingerprint(node),
                    fingerprint_with_source_lines(node, &file),
                    fingerprint_with_integer_source_lines(node, &file),
                    fingerprint_for_duplicate(node, &file),
                    fingerprint_for_duplicate_condition(node, &file),
                    fingerprint_for_file_operation(node, &file, Some(&spans.0)),
                    fingerprint_for_require(node, &file, Some(&spans.0)),
                    fingerprint_for_self_assignment(node, &file, Some(&spans.0)),
                    fingerprint_for_comparison(node, &file),
                    fingerprint_for_detection(node, &file),
                    fingerprint_for_case_subject(node, &file),
                    fingerprint_for_conditional_scope(node, &file),
                    fingerprint_for_branch(node, &file),
                    fingerprint_ignoring_grouping(node, source.as_bytes()),
                ]
            };
            let before: Vec<_> = nodes.iter().map(&old_keys).collect();
            for (i, (left, right)) in before[0].iter().zip(&before[1]).enumerate() {
                assert_eq!(left == right, i == 6, "profile {i}: {source}");
            }
            assert_eq!(
                fingerprint_for_gemspec_key(&nodes[0], &file, &spans.0),
                fingerprint_for_gemspec_key(&nodes[1], &file, &spans.0)
            );
            assert_eq!(before, nodes.iter().map(old_keys).collect::<Vec<_>>());
        }
    }

    #[test]
    fn duplicate_buffer_normalization_does_not_change_other_modes() {
        let source = "if /x/im; elsif /x/mi; end";
        let file = SourceFile::new("test.rb".into(), source.into());
        let parsed = ruby_prism::parse(source.as_bytes());
        let head = parsed
            .node()
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .iter()
            .next()
            .unwrap()
            .as_if_node()
            .unwrap();
        let first = head.predicate();
        let second = head.subsequent().unwrap().as_if_node().unwrap().predicate();
        assert_ne!(fingerprint(&first), fingerprint(&second));
        assert_ne!(
            fingerprint_for_conditional_scope(&first, &file),
            fingerprint_for_conditional_scope(&second, &file)
        );
        assert_eq!(
            fingerprint_for_duplicate(&first, &file),
            fingerprint_for_duplicate(&second, &file)
        );
    }

    #[test]
    fn duplicate_compound_profiles_are_isolated_from_scope_and_correction_keys() {
        for (source, scope_equal, hash_equal) in [
            ("[1i]; [1ri]", true, false),
            ("[0.0i]; [-0.0i]", false, true),
            (
                "# coding: UTF8-MAC\n[[/e\u{301}/im]]; [[/é/mi]]",
                false,
                true,
            ),
            (
                "# coding: UTF8-MAC\n\"#{e\u{301}.f}\"; \"#{é.f}\"",
                false,
                true,
            ),
        ] {
            let file = SourceFile::new("test.rb".into(), source.into());
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0);
            let body = parsed.node().as_program_node().unwrap().statements().body();
            let a = body.iter().next().unwrap();
            let b = body.iter().nth(1).unwrap();
            assert_ne!(fingerprint(&a), fingerprint(&b), "{source}");
            assert_ne!(
                fingerprint_with_source_lines(&a, &file),
                fingerprint_with_source_lines(&b, &file),
                "{source}"
            );
            assert_eq!(
                fingerprint_for_conditional_scope(&a, &file)
                    == fingerprint_for_conditional_scope(&b, &file),
                scope_equal,
                "{source}"
            );
            assert_eq!(
                fingerprint_for_duplicate(&a, &file) == fingerprint_for_duplicate(&b, &file),
                hash_equal,
                "{source}"
            );
            assert!(
                fingerprint_for_duplicate_condition(&a, &file)
                    .source_buffer_scope_eq(&fingerprint_for_duplicate_condition(&b, &file), &file),
                "{source}"
            );
        }
    }

    #[test]
    fn self_assignment_literal_projection_leaves_old_correction_keys_unchanged() {
        for (body, raw_equal, detection_equal) in [
            (r#"[%q()]; [""]"#, true, false),
            (r#"[%q()]; [%Q()]"#, true, true),
            (r#"[{"": 1}]; [{:"" => 1}]"#, true, false),
            (r#"[{"": 1}]; [{:"\u{}" => 1}]"#, true, true),
            (r#"[{'': 1}]; [{:"\u{}" => 1}]"#, true, true),
            ("[\"é\"]; [\"e\u{301}\"]", false, true),
            (r#"["é"]; ["\u0065\u0301"]"#, false, false),
            (r#"["\u00e9"]; ["\u0065\u0301"]"#, false, false),
            ("[186657005489253]; [142714875718083]", false, false),
            ("[1.0414461810403506]; [1.0316890681957467]", false, false),
            ("[186657005489253r]; [142714875718083r]", false, false),
            ("[1r]; [1.0r]", false, true),
            (r#"[1i]; [1ri]"#, false, true),
            (r#"[1i]; [1.0i]"#, false, true),
            (r#"[0.0]; [-0.0]"#, false, true),
            (r#"[(x ? 1 : 2)]; [(if x; 1; else; 2; end)]"#, false, true),
            (r#"[/a/im]; [/a/mi]"#, false, true),
            (r#"[/a/im]; [/a/iim]"#, false, true),
            (r#"[/a#{x}/im]; [/a#{x}/mi]"#, false, true),
            (r#"[0]; [0.0]"#, false, false),
            (r#"[/a/im]; [/a/i]"#, false, false),
            (r#"[(x ? 1 : 2)]; [(x ? 2 : 1)]"#, false, false),
        ] {
            let source = format!("# coding: UTF8-MAC\n{body}\n");
            let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0);
            let nodes: Vec<_> = parsed
                .node()
                .as_program_node()
                .unwrap()
                .statements()
                .body()
                .iter()
                .collect();
            let (a, b) = (&nodes[0], &nodes[1]);
            let raw = (fingerprint(a), fingerprint(b));
            let correction = (
                fingerprint_with_source_lines(a, &file),
                fingerprint_with_source_lines(b, &file),
            );
            assert_eq!(raw.0 == raw.1, raw_equal, "{source}");
            assert_eq!(correction.0 == correction.1, raw_equal, "{source}");
            assert_eq!(
                fingerprint_for_self_assignment(a, &file, None).source_buffer_scope_eq(
                    &fingerprint_for_self_assignment(b, &file, None),
                    &file
                ),
                detection_equal,
                "{source}"
            );
            assert_eq!(raw, (fingerprint(a), fingerprint(b)));
            assert_eq!(
                correction,
                (
                    fingerprint_with_source_lines(a, &file),
                    fingerprint_with_source_lines(b, &file)
                )
            );
        }
    }

    #[test]
    fn equal_structures_ignore_layout() {
        assert!(same("a.x(arg)\na.x(arg)\n"));
        assert!(same("a + b\na+b\n"));
        assert!(same("foo(a, b)\nfoo a,b\n"));
        assert!(same("foo()\nfoo\n"));
        assert!(same("x && y\nx  &&  y\n"));
        assert!(same("[1,\n 2]\n[1, 2]\n"));
    }

    #[test]
    fn numeric_source_line_equality_is_detection_only() {
        let source = "flags(__LINE__); flags(1); flags(__LINE__)\nflags(__LINE__)";
        let file = SourceFile::new("test.rb".into(), source.into());
        let parsed = ruby_prism::parse(source.as_bytes());
        let nodes: Vec<_> = parsed
            .node()
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .iter()
            .collect();
        for b in &nodes[1..3] {
            assert_eq!(
                fingerprint_with_integer_source_lines(&nodes[0], &file),
                fingerprint_with_integer_source_lines(b, &file)
            );
        }
        assert_ne!(
            fingerprint_with_integer_source_lines(&nodes[0], &file),
            fingerprint_with_integer_source_lines(&nodes[3], &file)
        );
        assert_ne!(
            fingerprint_with_source_lines(&nodes[0], &file),
            fingerprint_with_source_lines(&nodes[1], &file)
        );
        assert_ne!(fingerprint(&nodes[0]), fingerprint(&nodes[2]));
    }

    #[test]
    fn loop_syntax_normalization_does_not_relax_correction_fingerprints() {
        for source in [
            "items[0]; items.[](0)",
            "each_slice(/a/im); each_slice(/a/mi)",
            "each_slice(/#{pattern}/im); each_slice(/#{pattern}/mi)",
        ] {
            let file = SourceFile::new("test.rb".into(), source.into());
            let parsed = crate::parser::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0);
            let body = parsed.node().as_program_node().unwrap().statements().body();
            let a = body.iter().next().unwrap();
            let b = body.iter().nth(1).unwrap();
            assert_eq!(
                fingerprint_for_detection(&a, &file),
                fingerprint_for_detection(&b, &file),
                "{source}"
            );
            assert_ne!(fingerprint(&a), fingerprint(&b));
            assert_ne!(
                fingerprint_with_source_lines(&a, &file),
                fingerprint_with_source_lines(&b, &file)
            );
            assert_ne!(
                fingerprint_with_integer_source_lines(&a, &file),
                fingerprint_with_integer_source_lines(&b, &file)
            );
            assert!(!same_ignoring_grouping(
                std::str::from_utf8(a.location().as_slice()).unwrap(),
                std::str::from_utf8(b.location().as_slice()).unwrap()
            ));
        }
    }

    #[test]
    fn branch_normalization_does_not_relax_other_fingerprints() {
        for source in [
            "if a; b; else; c; end; a ? b : c",
            "->(a) { a }; lambda { |a| a }",
            "obj[1] = 2; obj.[]=(1, 2)",
            "foo { a; b }; foo { (a; b) }",
            "[__FILE__]; ['test.rb']",
            "unless a; b; else; c; end; a ? c : b",
            "unless a; b; end; if a; else; b; end",
            "if a; b; else; end; if a; b; end",
            "if a; elsif b; c; end; if a; else; if b; c; end; end",
            "if a; b; elsif c; d; end; if a; b; else; if c; d; end; end",
            "while a; b; end; b while a",
            "until a; b; end; b until a",
            "for a in xs; f(a); end; for a in xs do f(a) end",
            "unless a; b; end; b unless a",
            "while a; b; end; while a do b end",
            "until a; b; end; until a do b end",
            "def f(a); a; end; def f(a) = a",
            "def self.f(a); a; end; def self.f(a) = a",
            "case x; when 1; a; end; case x; when 1 then a end",
            "case x; in [a]; a; end; case x; in [a] then a end",
            "case x; when 1; a; end; case x; when 1; a; else; end",
            "def f; a; rescue; b; end; def f = a rescue b",
        ] {
            let file = SourceFile::new("test.rb".into(), source.into());
            let parsed = crate::parser::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0);
            let body = parsed.node().as_program_node().unwrap().statements().body();
            let a = body.iter().next().unwrap();
            let b = body.iter().nth(1).unwrap();
            assert_eq!(
                fingerprint_for_branch(&a, &file),
                fingerprint_for_branch(&b, &file),
                "{source}"
            );
            assert_ne!(fingerprint(&a), fingerprint(&b), "{source}");
            assert_ne!(
                fingerprint_with_source_lines(&a, &file),
                fingerprint_with_source_lines(&b, &file),
                "{source}"
            );
            assert_ne!(
                fingerprint_for_detection(&a, &file),
                fingerprint_for_detection(&b, &file),
                "{source}"
            );
            assert_ne!(
                fingerprint_for_case_subject(&a, &file),
                fingerprint_for_case_subject(&b, &file),
                "{source}"
            );
            assert_ne!(
                fingerprint_for_comparison(&a, &file),
                fingerprint_for_comparison(&b, &file),
                "{source}"
            );
            assert_ne!(
                fingerprint_with_integer_source_lines(&a, &file),
                fingerprint_with_integer_source_lines(&b, &file),
                "{source}"
            );
            // Grouping-blind mode already flattens this sequence pair by
            // design. None of the new control-structure normalizations apply.
            if source != "foo { a; b }; foo { (a; b) }" {
                assert_ne!(
                    fingerprint_ignoring_grouping(&a, source.as_bytes()),
                    fingerprint_ignoring_grouping(&b, source.as_bytes()),
                    "{source}"
                );
            }
        }
    }

    #[test]
    fn case_subject_normalization_does_not_relax_other_fingerprints() {
        for source in [
            "foo(Encoding::UTF_8); foo(__ENCODING__)",
            "# coding: ISO-8859-1\nfoo(Encoding::UTF_8); foo(__ENCODING__)",
            "foo(!x); foo(x.!)",
            "foo(not x); foo(x.!())",
        ] {
            let file = SourceFile::new("test.rb".into(), source.into());
            let parsed = crate::parser::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0);
            let body = parsed.node().as_program_node().unwrap().statements().body();
            let a = body.iter().next().unwrap();
            let b = body.iter().nth(1).unwrap();
            assert_eq!(
                fingerprint_for_case_subject(&a, &file),
                fingerprint_for_case_subject(&b, &file),
                "{source}"
            );
            assert_ne!(fingerprint(&a), fingerprint(&b));
            assert_ne!(
                fingerprint_with_source_lines(&a, &file),
                fingerprint_with_source_lines(&b, &file)
            );
            assert_ne!(
                fingerprint_with_integer_source_lines(&a, &file),
                fingerprint_with_integer_source_lines(&b, &file)
            );
            assert!(!same_ignoring_grouping(
                std::str::from_utf8(a.location().as_slice()).unwrap(),
                std::str::from_utf8(b.location().as_slice()).unwrap()
            ));
        }
    }

    #[test]
    fn rational_value_equality_does_not_relax_correction_fingerprints() {
        for source in ["flags(1r); flags(1.0r)", "flags(1ri); flags(1.0ri)"] {
            let file = SourceFile::new("test.rb".into(), source.into());
            let parsed = ruby_prism::parse(source.as_bytes());
            let body = parsed.node().as_program_node().unwrap().statements().body();
            let a = body.iter().next().unwrap();
            let b = body.iter().nth(1).unwrap();
            assert_eq!(
                fingerprint_with_integer_source_lines(&a, &file),
                fingerprint_with_integer_source_lines(&b, &file)
            );
            assert_ne!(fingerprint(&a), fingerprint(&b));
            assert_ne!(
                fingerprint_with_source_lines(&a, &file),
                fingerprint_with_source_lines(&b, &file)
            );
        }
    }

    #[test]
    fn numeric_zero_equality_is_detection_only() {
        let source = "obj.f(0.0); obj.f(-0.0)";
        let file = SourceFile::new("test.rb".into(), source.into());
        let parsed = ruby_prism::parse(source.as_bytes());
        let body = parsed.node().as_program_node().unwrap().statements().body();
        let a = body.iter().next().unwrap();
        let b = body.iter().nth(1).unwrap();
        assert_eq!(
            fingerprint_for_comparison(&a, &file),
            fingerprint_for_comparison(&b, &file)
        );
        assert_eq!(
            fingerprint_with_integer_source_lines(&a, &file),
            fingerprint_with_integer_source_lines(&b, &file)
        );
        assert_ne!(fingerprint(&a), fingerprint(&b));
        assert_ne!(
            fingerprint_with_source_lines(&a, &file),
            fingerprint_with_source_lines(&b, &file)
        );
        assert!(!same_ignoring_grouping("0.0", "-0.0"));
    }

    /// Whether two programs parse to equal trees, grouping aside.
    fn same_ignoring_grouping(a: &str, b: &str) -> bool {
        let first = ruby_prism::parse(a.as_bytes());
        let second = ruby_prism::parse(b.as_bytes());
        assert_eq!(first.errors().count(), 0, "{a:?} must parse");
        assert_eq!(second.errors().count(), 0, "{b:?} must parse");
        fingerprint_ignoring_grouping(&first.node(), a.as_bytes())
            == fingerprint_ignoring_grouping(&second.node(), b.as_bytes())
    }

    #[test]
    fn grouping_parentheses_are_transparent_when_ignoring_grouping() {
        for (a, b) in [
            ("(x)\n", "x\n"),
            ("foo((1))\n", "foo(1)\n"),
            ("foo (x)\n", "foo x\n"),
            ("(x).y(z)\n", "x.y(z)\n"),
            ("foo (x).y\n", "foo x.y\n"),
            ("[(1)]\n", "[1]\n"),
            ("\"#{(foo)}\"\n", "\"#{foo}\"\n"),
            ("def f\n  (a; b)\nend\n", "def f\n  a; b\nend\n"),
            ("x\n(a; b)\n", "x\na; b\n"),
            ("2**(-2)\n", "2**-2\n"),
            ("((x))\n", "x\n"),
            ("(__LINE__)\n", "__LINE__\n"),
        ] {
            assert!(same_ignoring_grouping(a, b), "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn heredoc_bodies_are_not_own_text_when_ignoring_grouping() {
        for (a, b) in [
            ("x = (<<~A\n  a\nA\n)\n", "x = <<~A\n  a\nA\n"),
            ("x = (<<~A\n  a #{b}\nA\n)\n", "x = <<~A\n  a #{b}\nA\n"),
            ("x = (foo(<<~A)\n  a\nA\n)\n", "x = foo(<<~A)\n  a\nA\n"),
            ("x = (<<~`A`\n  a\nA\n)\n", "x = <<~`A`\n  a\nA\n"),
        ] {
            assert!(same_ignoring_grouping(a, b), "{a:?} vs {b:?}");
        }
        assert!(!same_ignoring_grouping(
            "x = (<<~A\n  a\nA\n)\n",
            "x = <<~A\n  b\nA\n"
        ));
    }

    #[test]
    fn same_operator_chains_are_flattened_when_ignoring_grouping() {
        assert!(same_ignoring_grouping("a && (b && c)\n", "a && b && c\n"));
        assert!(same_ignoring_grouping("(a && b) && c\n", "a && b && c\n"));
        assert!(same_ignoring_grouping(
            "a || (b || c) || d\n",
            "a || b || c || d\n"
        ));
        assert!(same_ignoring_grouping("x && (y and z)\n", "x && y and z\n"));
    }

    #[test]
    fn regrouping_that_changes_the_tree_is_seen() {
        for (a, b) in [
            ("(a || b) && c\n", "a || b && c\n"),
            ("a && (b || c)\n", "a && b || c\n"),
            ("(a + b) * c\n", "a + b * c\n"),
            ("-(1)\n", "-1\n"),
            ("(-2)**2\n", "-2**2\n"),
            ("-(1.foo)\n", "-1.foo\n"),
            ("x = (a; b)\n", "x = a; b\n"),
            ("(a; b).c\n", "a; b.c\n"),
            ("x = (a and b)\n", "x = a and b\n"),
            ("(\n__LINE__)\n", "__LINE__\n"),
            ("(x)\n", "y\n"),
            ("a && b\n", "a || b\n"),
        ] {
            assert!(!same_ignoring_grouping(a, b), "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn plain_fingerprints_keep_grouping() {
        assert!(!same("(x)\nx\n"));
        assert!(!same("a && (b && c)\na && b && c\n"));
    }

    #[test]
    fn implicit_block_parameters_and_comments_are_layout() {
        assert!(same("foo {  it }\nfoo { it }\n"));
        assert!(same("foo {_1 * 2}\nfoo { _1 * 2 }\n"));
        assert!(same("-> () { x }\n-> { x }\n"));
        assert!(same("foo { || x }\nfoo { x }\n"));
        assert!(same("(qux # one\n)\n(qux # two\n)\n"));
        assert!(same("[a, # one\n b]\n[a, b]\n"));
    }

    #[test]
    fn interpolation_and_block_parameters_still_differ() {
        assert!(!same("\"#{a}\"\n\"#{b}\"\n"));
        assert!(!same("\"#@a\"\n\"#@b\"\n"));
        assert!(!same("foo { |a| a }\nfoo { |b| b }\n"));
        assert!(!same("foo { _1 }\nfoo { _2 }\n"));
        assert!(!same("foo { |;x| 1 }\nfoo { 1 }\n"));
        assert!(!same("[\"#\", a]\n[\"\", a]\n"));
    }

    #[test]
    fn literal_spellings_compare_by_value() {
        assert!(same("'a'\n\"a\"\n"));
        assert!(same(":a\n:\"a\"\n"));
        assert!(same("1.0\n1.00\n"));
        assert!(same("0x10\n16\n"));
        assert!(same("/./\n%r{.}\n"));
    }

    #[test]
    fn spellings_rubocop_does_not_keep_are_normalized() {
        assert!(same("a && b\na and b\n"));
        assert!(same("(a or b)\n(a || b)\n"));
        assert!(same("!a\nnot a\n"));
        assert!(same("!a\nnot(a)\n"));
        assert!(same("a.b\na::b\n"));
        assert!(same("a.b(1)\na::b 1\n"));
        assert!(same("{ a: 1 }\n{ :a => 1 }\n"));
        assert!(same("foo(a: 1)\nfoo({ a: 1 })\n"));
        assert!(same("foo(:a => 1, b: 2)\nfoo({ a: 1, :b => 2 })\n"));
        assert!(same("a.b { 1 }\na.b do 1 end\n"));
        assert!(same("foo { |x| x }\nfoo do |x| x end\n"));
        assert!(same("-> { a }\n-> do a end\n"));
        assert!(same("%w[a b]\n['a', 'b']\n"));
        assert!(same("%w[a b]\n%w(a b)\n"));
        assert!(same("%i[a b]\n[:a, :b]\n"));
        assert!(same("[1, 2,]\n[1, 2]\n"));
    }

    #[test]
    fn implicit_keyword_value_wrappers_are_transparent() {
        for source in [
            "foo(x:)\nfoo(x: x)\n",
            "foo(x: x)\nfoo(x:)\n",
            "foo { |x| bar(x:) }\nfoo { |x| bar(x: x) }\n",
            "{ X: }\n{ X: X }\n",
            "{ x:, y: }\n{ x: x, y: y }\n",
        ] {
            assert!(same(source), "{source}");
        }
        for source in [
            "foo(x:)\nfoo(x: y)\n",
            "{ x: }\n{ y: x }\n",
            "foo { |x| bar(x:) }\nfoo { |y| bar(x: x) }\n",
        ] {
            assert!(!same(source), "{source}");
        }
        assert!(same_ignoring_grouping("foo(x:)\n", "foo(x: x)\n"));
    }

    #[test]
    fn normalized_spellings_keep_structure() {
        assert!(!same("a && b\na || b\n"));
        assert!(!same("a and b\na or b\n"));
        assert!(!same("!a\n-a\n"));
        assert!(!same("!a\na\n"));
        assert!(!same("a::b\na&.b\n"));
        assert!(!same("a::b\na::B\n"));
        assert!(!same("{ a: 1 }\n{ a: 2 }\n"));
        assert!(!same("{ a: 1 }\n{ 'a' => 1 }\n"));
        assert!(!same("foo(a: 1)\nfoo(**{ a: 1 })\n"));
        assert!(!same("foo(a: 1)\nfoo([a: 1])\n"));
        assert!(!same("[1, 2]\n%w[1 2]\n"));
        assert!(!same("[[1], 2]\n[1, [2]]\n"));
        assert!(!same("[a, *b]\n[a, b]\n"));
        assert!(!same("{ a: 1, b: 2 }\n{ a: 1 }\n"));
        assert!(!same("a.b { 1 }\na.b { 2 }\n"));
        assert!(!same("a.b { 1 }\na.b\n"));
        assert!(!same("foo { |x| x }\nfoo { |y| y }\n"));
    }

    #[test]
    fn typed_children_are_fingerprinted_like_other_nodes() {
        // ruby-prism visits these as typed children, without its enter hook.
        // `super`'s block: `do…end` and `{…}`, `||` and no parameters.
        assert!(same("super { x }\nsuper do x end\n"));
        assert!(same("super { || x }\nsuper { x }\n"));
        assert!(!same("super { x }\nsuper { y }\n"));
        // Statements, arguments and `rescue`/`else`/`ensure` clauses.
        assert!(same(
            "begin; a; rescue; b; else; c; ensure; d; end\nbegin\n  a\nrescue\n  b\nelse\n  c\nensure\n  d\nend\n"
        ));
        assert!(!same(
            "begin; a; b; rescue; end\nbegin; a; rescue; b; end\n"
        ));
        assert!(!same(
            "begin; a; rescue; b; end\nbegin; a; ensure; b; end\n"
        ));
        assert!(same("super 1, 2\nsuper(1, 2)\n"));
        assert!(same("a[1, 2] += 3\na[1,2] += 3\n"));
        // Parameters, pattern captures, constant path targets.
        assert!(same("def m(a, &b); end\ndef m a, &b\nend\n"));
        assert!(!same("def m(&b); end\ndef m(&c); end\n"));
        assert!(!same(
            "case x; in Integer => n; end\ncase x; in Integer => m; end\n"
        ));
        assert!(!same("A::B ||= 1\nA::C ||= 1\n"));
        assert!(!same("a[1] += 2\na[2] += 2\n"));
    }

    #[test]
    fn every_node_of_the_full_tree_is_fingerprinted() {
        use crate::cops::testing::{FULL_TREE, preorder_kinds};
        let result = ruby_prism::parse(FULL_TREE.as_bytes());
        let Fingerprint(tokens, _) = fingerprint(&result.node());
        let opened = tokens
            .iter()
            .filter(|token| matches!(token, Token::Open(_)))
            .count();
        let closed = tokens
            .iter()
            .filter(|token| matches!(token, Token::Close(_)))
            .count();
        let leaves = tokens
            .iter()
            .filter(|token| matches!(token, Token::Leaf(..)))
            .count();
        assert_eq!(opened, closed);
        let kinds = preorder_kinds(FULL_TREE);
        // Implicit keyword-value wrappers deliberately emit no token.
        let implicit = kinds.iter().filter(|kind| *kind == "ImplicitNode").count();
        assert!(implicit > 0);
        assert_eq!(opened + leaves, kinds.len() - implicit);
    }

    #[test]
    fn different_structures_differ() {
        assert!(!same("a + b\na - b\n"));
        assert!(!same("a.b\na&.b\n"));
        assert!(!same("1\n1.0\n"));
        assert!(!same("'a'\n:a\n"));
        assert!(!same("foo(a)\nfoo(b)\n"));
        assert!(!same("foo(a, b)\nfoo(a, [b])\n"));
        assert!(!same("foo(bar(a), b)\nfoo(bar(a, b))\n"));
        assert!(!same("/a/i\n/a/\n"));
        assert!(!same("/(a)/\n/a/\n"));
        assert!(!same("\"(\"\n\")\"\n"));
        assert!(!same("x\n(x)\n"));
        assert!(!same("a && b && c\na && b\n"));
        assert!(!same("__LINE__\n__LINE__\n"));
    }

    #[test]
    fn variables_and_calls_differ() {
        // `x` is a local variable read in the second statement only.
        let result = ruby_prism::parse(b"x = 1\nx\n");
        let program = result.node();
        let read = program.as_program_node().unwrap().statements().body();
        let read = read.iter().nth(1).unwrap();
        let call_result = ruby_prism::parse(b"x\n");
        let call_program = call_result.node();
        let call = call_program.as_program_node().unwrap().statements().body();
        let call = call.iter().next().unwrap();
        assert!(!same_node(&read, &call));
    }

    fn recursive_literal(source: &str) -> bool {
        let result = ruby_prism::parse(source.as_bytes());
        let program = result.node();
        let statements = program.as_program_node().unwrap().statements().body();
        is_recursive_basic_literal(&statements.iter().next().unwrap())
    }

    #[test]
    fn recursive_basic_literals() {
        for source in [
            "!true",
            "1.==(2)",
            "[1.==(2)]",
            "\"#{2}\"",
            "(1)",
            "(false && true)",
            "(false <=> true)",
            "(false or true)",
            "[1, 2, 3]",
            "{ :a => 1, :b => 2 }",
            "{ a: 1, b: 2 }",
            "/./",
            "%r{abx}ixo",
            "1.0",
            "1",
            "false",
            "nil",
            "'str'",
            "(42..)",
        ] {
            assert!(recursive_literal(source), "{source} is a literal");
        }
    }

    #[test]
    fn non_literals() {
        for source in [
            "\"#{some_method_call}\"",
            "(x && false)",
            "(x == false)",
            "(x or false)",
            "[some_method_call]",
            "1&.==(2)",
            "[1&.==(2)]",
            "{ a: [1&.==(2)] }",
            "1.==(2&.==(3))",
            "1&.==(2).==(false)",
            "{ :sym => some_method_call }",
            "{ some_method_call => :sym }",
            "/.#{some_method_call}/",
            "%r{abx#{foo}}ixo",
            "some_method_call",
            "some_method_call(x, y)",
            "1 + 1",
            "Foo",
            "[*a]",
        ] {
            assert!(!recursive_literal(source), "{source} is not a literal");
        }
    }

    #[test]
    fn basic_literals_exclude_composites() {
        let basic = |source: &str| {
            let result = ruby_prism::parse(source.as_bytes());
            let program = result.node();
            let statements = program.as_program_node().unwrap().statements().body();
            is_basic_literal(&statements.iter().next().unwrap())
        };
        for source in [
            "1", "1.0", "1r", "1i", "'a'", ":a", "nil", "true", "__FILE__",
        ] {
            assert!(basic(source), "{source} is a basic literal");
        }
        for source in [
            "\"#{a}\"",
            "`a`",
            ":\"#{a}\"",
            "[1]",
            "{}",
            "1..2",
            "/a/",
            "a",
        ] {
            assert!(!basic(source), "{source} is not a basic literal");
        }
    }

    #[test]
    fn operator_methods() {
        for name in ["-@", "!", "[]", "[]=", "==", "<=>", "**", "`"] {
            assert!(is_operator_method(name.as_bytes()), "{name} is an operator");
        }
        for name in ["foo", "foo=", "call", "<<~"] {
            assert!(
                !is_operator_method(name.as_bytes()),
                "{name} is not an operator"
            );
        }
    }

    #[test]
    fn const_names() {
        let names: Vec<Option<String>> = ["Foo", "Foo::Bar", "::Foo", "foo"]
            .iter()
            .map(|source| {
                let result = ruby_prism::parse(source.as_bytes());
                let program = result.node();
                let statements = program.as_program_node().unwrap().statements().body();
                const_name(&statements.iter().next().unwrap())
            })
            .collect();
        assert_eq!(
            names,
            vec![
                Some("Foo".to_string()),
                Some("Foo::Bar".to_string()),
                Some("Foo".to_string()),
                None
            ]
        );
    }

    #[test]
    fn call_arguments_end_with_a_block_argument() {
        let kinds: Vec<Vec<String>> = ["foo(a, *b, k: 1, &c)", "foo(a) { }", "foo", "foo(&c)"]
            .iter()
            .map(|source| {
                let result = ruby_prism::parse(source.as_bytes());
                let program = result.node();
                let statements = program.as_program_node().unwrap().statements().body();
                let call = statements.iter().next().unwrap().as_call_node().unwrap();
                call_arguments(&call)
                    .iter()
                    .map(|argument| {
                        let dump = format!("{argument:?}");
                        dump[..dump.find('(').unwrap_or(dump.len())].to_string()
                    })
                    .collect()
            })
            .collect();
        assert_eq!(
            kinds,
            [
                vec![
                    "CallNode",
                    "SplatNode",
                    "KeywordHashNode",
                    "BlockArgumentNode"
                ],
                vec!["CallNode"],
                vec![],
                vec!["BlockArgumentNode"],
            ]
        );
    }
}
