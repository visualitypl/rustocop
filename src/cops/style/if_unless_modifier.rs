//! `Style/IfUnlessModifier` — simple conditionals and overlong modifiers.

use crate::{
    cops::{
        Cop, NodeContext, ast, variables,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{Location, Node, StatementsNode};
use std::collections::{HashMap, HashSet};

pub struct IfUnlessModifier {
    max: Option<usize>,
}
impl Default for IfUnlessModifier {
    fn default() -> Self {
        Self { max: Some(120) }
    }
}
impl Cop for IfUnlessModifier {
    fn name(&self) -> &'static str {
        "Style/IfUnlessModifier"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &["Style/Next", "Style/SoleNestedConditional"]
    }
    fn with_line_length_limit(&self, max: Option<usize>) -> Option<Box<dyn Cop>> {
        Some(Box::new(Self { max }))
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(c) = Conditional::of(node) else {
            return;
        };
        let body = c.body.as_ref().and_then(|s| {
            (s.body().len() == 1)
                .then(|| s.body().iter().next())
                .flatten()
        });
        if body
            .as_ref()
            .is_some_and(|b| b.as_def_node().is_some_and(|d| d.equal_loc().is_some()))
            || cx
                .ancestors()
                .any(|n| n.as_interpolated_string_node().is_some())
        {
            return;
        }
        let root = cx.ancestors().next_back().unwrap_or(node);
        let index = file.modifier_index(root);
        let mut conditions = ConditionFacts::default();
        walk(&mut conditions, &c.condition);
        if conditions.pattern
            || conditions.defined.iter().any(|name| {
                let parent = semantic_parent(cx);
                !parent
                    .and_then(|p| p.as_statements_node())
                    .is_some_and(|s| {
                        index
                            .assignments
                            .get(&(s.location().start_offset(), name.clone()))
                            .is_some_and(|&at| at < node.location().start_offset())
                    })
            })
        {
            return;
        }
        let parens = semantic_parent(cx).is_some_and(needs_parens);
        let msg = if c.end.is_some()
            && c.keyword.as_slice() != b"elsif"
            && !c.other
            && !index.shared.contains(&node.location().start_offset())
            && !conditions.assignment
            && !regexp_match_statement(&c.condition)
            && !semantic_parent(cx).is_some_and(|p| {
                p.as_call_node()
                    .is_some_and(|call| call.receiver().is_some_and(|r| same(&r, node)))
            })
            && body
                .as_ref()
                .is_some_and(|body| self.eligible_body(&c, body, file))
            && self.fits(&c, body.as_ref().unwrap(), parens, file)
        {
            format!(
                "Favor modifier `{}` usage when having a single-line body. {}",
                c.word(),
                if parens {
                    "Wrap the expression in parentheses to keep the current behavior, as it is part of a larger expression."
                } else {
                    "Another good alternative is the usage of control flow `&&`/`||`."
                }
            )
        } else if c.end.is_none()
            && file.line_of(node.location().start_offset()) == file.last_line_of(node.location())
            && self.max.is_some_and(|max| {
                !crate::cops::layout::line_length::modifier_line_acceptable(
                    file,
                    file.line_of(node.location().start_offset()),
                    max,
                )
            })
            && !following_statement(cx, node, index)
            && !logical_modifiers_share_line(cx, node, file)
        {
            format!("Modifier form of `{}` makes the line too long.", c.word())
        } else {
            return;
        };
        let (line, col) = file.line_col(c.keyword.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: msg,
            line,
            col,
            end_line: line,
            correction: if cx.corrections_requested()
                && !index
                    .modifier_shared(root, file)
                    .contains(&node.location().start_offset())
                && !cx
                    .ancestors()
                    .any(|ancestor| Conditional::of(ancestor).is_some_and(|c| c.end.is_none()))
            {
                self.rewrite(node, &c, body.as_ref(), cx, file)
            } else {
                None
            },
        });
    }
}
impl IfUnlessModifier {
    fn rewrite(
        &self,
        node: &Node<'_>,
        c: &Conditional<'_>,
        body: Option<&Node<'_>>,
        cx: &NodeContext<'_, '_>,
        file: &SourceFile,
    ) -> Option<Correction> {
        let loc = node.location();
        let mut span = loc.start_offset()..loc.end_offset();
        if span.len() > REWRITE_BUDGET || super::correction::touches_directive(file, span.clone()) {
            return None;
        }
        let body = body?;
        let root = cx.ancestors().next_back()?;
        // Deferred payloads may lie outside the node range; neither relocate
        // nor insert lines while a heredoc is pending. Keep their diagnostics.
        if file.contains_heredoc(root, node) || file.heredoc_pending_at(root, span.start) {
            return None;
        }
        #[cfg(test)]
        REWRITE_BYTES.with(|n| n.set(n.get() + span.len()));
        let mut predicate = LexicalFacts::default();
        walk(&mut predicate, &c.condition);
        let mut branch = LexicalFacts::default();
        walk(&mut branch, body);
        // Moving declarations across reads can change calls into locals,
        // named-capture bindings, block capture or defined? observations.
        let references = if c.end.is_some() {
            &predicate.calls
        } else {
            &predicate.references
        };
        if predicate.declarations
            || predicate.capture
            || branch.capture
            || branch.ambiguous
            || branch.names.iter().any(|name| references.contains(name))
        {
            return None;
        }
        let condition = std::str::from_utf8(c.condition.location().as_slice()).ok()?;
        let body_text = std::str::from_utf8(body.location().as_slice()).ok()?;
        if condition.contains(['\n', '\r']) || body_text.contains(['\n', '\r']) {
            return None;
        }
        let mut replacement;
        if c.end.is_some() {
            let parens = correction_parens(cx)?;
            // Extra operand boundaries are correction-only: retain detection's
            // positions/messages/eligibility, but never emit an overlong fix.
            if !self.fits(c, body, parens, file) {
                return None;
            }
            // Only the header's ordinary trailing comment may move. All other
            // comments (including embedded docs) remain in their original place.
            let comment = first_comment(file, file.line_of(c.keyword.start_offset()));
            let comments = file.comments();
            let first = comments.partition_point(|r| r.end <= span.start);
            if comments[first..]
                .iter()
                .take_while(|r| r.start < span.end)
                .any(|r| Some(&file.bytes[r.clone()]) != comment)
                || comment.is_some_and(|b| !super::correction::comment_relocation_safe(b))
            {
                return None;
            }
            let rendered_body = body_source(body);
            // The length renderer supports calls and simple assignment/return
            // wrappers. A terminal omitted hash under an unsupported wrapper
            // would consume the new modifier as its missing value (or fail to
            // parse); do not guess how to parenthesize that expression.
            if branch.omitted_hash && rendered_body.ends_with(b":") {
                return None;
            }
            // An unbounded range cannot be followed directly by a modifier.
            // This also covers terminal ranges in calls/assignments/returns
            // without guessing at the boundaries of unsupported wrappers.
            if branch.endless_range && rendered_body.ends_with(b"..") {
                return None;
            }
            replacement = format!(
                "{}{} {} {}{}",
                if parens { "(" } else { "" },
                std::str::from_utf8(&rendered_body).ok()?,
                c.word(),
                condition,
                if parens { ")" } else { "" }
            );
            if let Some(comment) = comment {
                replacement.push(' ');
                replacement.push_str(
                    std::str::from_utf8(comment.strip_suffix(b"\r").unwrap_or(comment)).ok()?,
                );
            }
        } else {
            let line = file.line_of(span.start);
            let prefix = &file.bytes[file.line_start(line)..span.start];
            let prefix = if line == 1 {
                prefix.strip_prefix(b"\xef\xbb\xbf").unwrap_or(prefix)
            } else {
                prefix
            };
            if prefix.len() > REWRITE_BUDGET {
                return None;
            }
            // Expand standalone statements or explicitly parenthesized
            // expressions; leave other lexical contexts alone. The opening
            // parenthesis' physical column determines the end's indentation.
            if !prefix.iter().all(u8::is_ascii_whitespace) && !prefix.ends_with(b"(") {
                return None;
            }
            let indent = if prefix.iter().all(u8::is_ascii_whitespace) {
                std::str::from_utf8(prefix).ok()?.to_owned()
            } else {
                " ".repeat(prefix.len())
            };
            let nl = if file
                .bytes
                .get(file.line_start(line + 1).saturating_sub(2)..file.line_start(line + 1))
                == Some(b"\r\n")
            {
                "\r\n"
            } else {
                "\n"
            };
            if let Some(comment) = first_comment(file, line) {
                let comments = file.comments();
                let first = comments.partition_point(|r| r.start < file.line_start(line));
                let comment_at = comments.get(first)?;
                if comment_at.end - span.start > REWRITE_BUDGET {
                    return None;
                }
                let length_without = crate::cops::layout::line_length::display_width(
                    &file.bytes[file.line_start(line)..comment_at.start],
                );
                if self.max.is_some_and(|max| length_without <= max) {
                    if !super::correction::comment_relocation_safe(comment) {
                        return None;
                    }
                    // A trailing suffix such as ')' must not move into a comment.
                    if !file.bytes[span.end..comment_at.start]
                        .iter()
                        .all(u8::is_ascii_whitespace)
                    {
                        return None;
                    }
                    replacement = format!(
                        "{}{nl}{indent}{body_text} {} {condition}",
                        std::str::from_utf8(comment.strip_suffix(b"\r").unwrap_or(comment)).ok()?,
                        c.word()
                    );
                    span.end = comment_at.end - usize::from(comment.ends_with(b"\r"));
                } else {
                    replacement = format!(
                        "{} {condition}{nl}{indent}  {body_text}{nl}{indent}end",
                        c.word()
                    );
                }
            } else {
                replacement = format!(
                    "{} {condition}{nl}{indent}  {body_text}{nl}{indent}end",
                    c.word()
                );
            }
        }
        let edit = Correction {
            range: span,
            replacement,
        };
        (super::correction::preserves_line_values(file, &edit)
            && super::correction::preserves_data_positions(file, cx, &edit))
        .then_some(edit)
    }
    fn eligible_body(&self, c: &Conditional<'_>, body: &Node<'_>, file: &SourceFile) -> bool {
        if body.as_parentheses_node().is_some() {
            return false;
        }
        struct Nested(bool);
        impl<'pr> NodeVisitor<'pr> for Nested {
            fn enter(&mut self, n: Node<'pr>) {
                self.0 |= n.as_if_node().is_some() || n.as_unless_node().is_some();
            }
        }
        let mut nested = Nested(false);
        walk(&mut nested, body);
        if nested.0 {
            return false;
        }
        let start = file.line_of(c.keyword.start_offset());
        let end = file.line_of(c.end.as_ref().unwrap().start_offset());
        // At most three nonempty lines: the keyword, body and end. Stop early
        // rather than scanning a large ineligible subtree for every ancestor.
        if (start..=end)
            .filter(|&line| !file.line_text(line).iter().all(u8::is_ascii_whitespace))
            .take(4)
            .count()
            > 3
        {
            return false;
        }
        if file.has_comment_in_lines(end..end + 1) {
            return false;
        }
        let loc = body.location();
        if file.has_comment_in_lines(file.line_of(loc.start_offset())..file.last_line_of(loc) + 1) {
            return false;
        }
        true
    }
    fn fits(&self, c: &Conditional<'_>, body: &Node<'_>, parens: bool, file: &SourceFile) -> bool {
        let line = file.line_of(c.keyword.start_offset());
        let prefix = &file.bytes[file.line_start(line)..c.keyword.start_offset()];
        let end = c.end.as_ref().unwrap();
        let end_line = file.line_of(end.start_offset());
        let suffix = &file.line_text(end_line)[end.end_offset() - file.line_start(end_line)..];
        let comment = first_comment(file, line);
        if comment.is_some() && !suffix.is_empty() {
            return false;
        }
        let Some(max) = self.max else {
            return true;
        };
        let body = body_source(body);
        let mut rendered = Vec::new();
        rendered.extend_from_slice(prefix);
        if parens {
            rendered.push(b'(');
        }
        rendered.extend_from_slice(&body);
        rendered.push(b' ');
        rendered.extend_from_slice(c.keyword.as_slice());
        rendered.push(b' ');
        rendered.extend_from_slice(c.condition.location().as_slice());
        if parens {
            rendered.push(b')');
        }
        if let Some(comment) = comment {
            rendered.push(b' ');
            rendered.extend_from_slice(comment);
        }
        rendered.extend_from_slice(suffix);
        crate::cops::layout::line_length::display_width(&rendered) <= max
    }
}
pub(super) fn first_comment(file: &SourceFile, line: u32) -> Option<&[u8]> {
    let start = file.line_start(line);
    let comments = file.comments();
    let at = comments.partition_point(|r| r.start < start);
    comments
        .get(at)
        .filter(|r| file.line_of(r.start) == line)
        .map(|r| &file.bytes[r.clone()])
}
struct Conditional<'pr> {
    keyword: Location<'pr>,
    end: Option<Location<'pr>>,
    condition: Node<'pr>,
    body: Option<StatementsNode<'pr>>,
    other: bool,
}
impl<'pr> Conditional<'pr> {
    fn of(n: &Node<'pr>) -> Option<Self> {
        if let Some(n) = n.as_if_node() {
            Some(Self {
                keyword: n.if_keyword_loc()?,
                end: n.end_keyword_loc(),
                condition: n.predicate(),
                body: n.statements(),
                other: n.subsequent().is_some(),
            })
        } else {
            n.as_unless_node().map(|n| Self {
                keyword: n.keyword_loc(),
                end: n.end_keyword_loc(),
                condition: n.predicate(),
                body: n.statements(),
                other: n.else_clause().is_some(),
            })
        }
    }
    fn word(&self) -> &'static str {
        if self.keyword.as_slice() == b"unless" {
            "unless"
        } else {
            "if"
        }
    }
}
fn same(a: &Node<'_>, b: &Node<'_>) -> bool {
    a.location().start_offset() == b.location().start_offset()
        && a.location().end_offset() == b.location().end_offset()
}
pub(super) fn semantic_parent<'a, 'pr>(cx: &NodeContext<'a, 'pr>) -> Option<&'a Node<'pr>> {
    cx.ancestors().find(|n| {
        n.as_arguments_node().is_none()
            && n.as_else_node().is_none()
            && !n.as_statements_node().is_some_and(|s| s.body().len() == 1)
    })
}
pub(super) fn needs_parens(n: &Node<'_>) -> bool {
    // Assignment variants are named *WriteNode in Prism. Targets alone are
    // not ancestors of the RHS; include setter/index calls (RuboCop sends).
    matches!(
        n,
        Node::LocalVariableWriteNode { .. }
            | Node::LocalVariableAndWriteNode { .. }
            | Node::LocalVariableOrWriteNode { .. }
            | Node::LocalVariableOperatorWriteNode { .. }
            | Node::InstanceVariableWriteNode { .. }
            | Node::InstanceVariableAndWriteNode { .. }
            | Node::InstanceVariableOrWriteNode { .. }
            | Node::InstanceVariableOperatorWriteNode { .. }
            | Node::ClassVariableWriteNode { .. }
            | Node::ClassVariableAndWriteNode { .. }
            | Node::ClassVariableOrWriteNode { .. }
            | Node::ClassVariableOperatorWriteNode { .. }
            | Node::GlobalVariableWriteNode { .. }
            | Node::GlobalVariableAndWriteNode { .. }
            | Node::GlobalVariableOrWriteNode { .. }
            | Node::GlobalVariableOperatorWriteNode { .. }
            | Node::ConstantWriteNode { .. }
            | Node::ConstantAndWriteNode { .. }
            | Node::ConstantOrWriteNode { .. }
            | Node::ConstantOperatorWriteNode { .. }
            | Node::ConstantPathWriteNode { .. }
            | Node::ConstantPathAndWriteNode { .. }
            | Node::ConstantPathOrWriteNode { .. }
            | Node::ConstantPathOperatorWriteNode { .. }
            | Node::CallAndWriteNode { .. }
            | Node::CallOrWriteNode { .. }
            | Node::CallOperatorWriteNode { .. }
            | Node::IndexAndWriteNode { .. }
            | Node::IndexOrWriteNode { .. }
            | Node::IndexOperatorWriteNode { .. }
            | Node::MultiWriteNode { .. }
    ) || n.as_and_node().is_some()
        || n.as_or_node().is_some()
        || matches!(
            n,
            Node::ArrayNode { .. }
                | Node::RangeNode { .. }
                | Node::AssocNode { .. }
                | Node::CallNode { .. }
                | Node::YieldNode { .. }
                | Node::SuperNode { .. }
        )
        || n.as_if_node().is_some_and(|c| c.if_keyword_loc().is_none())
        || n.as_def_node().is_some_and(|d| d.equal_loc().is_some())
}
/// Detection mirrors upstream's parent list; corrections need a stricter
/// boundary proof. In particular `value if condition in pattern` matches the
/// condition, not the conditional value, and `*value if condition` is invalid
/// in a collection. Do not infer safety for an unfamiliar expression wrapper.
pub(super) fn correction_parens(cx: &NodeContext<'_, '_>) -> Option<bool> {
    let parent = semantic_parent(cx)?;
    if needs_parens(parent) {
        return Some(true);
    }
    if cx
        .parent()
        .is_some_and(|p| p.as_statements_node().is_some())
    {
        // Statement lists delimit branch/block/method bodies. Existing required
        // parentheses (e.g. endless methods) were handled above.
        return Some(false);
    }
    match parent {
        Node::MatchPredicateNode { .. }
        | Node::MatchRequiredNode { .. }
        | Node::SplatNode { .. }
        | Node::AssocSplatNode { .. }
        | Node::DefinedNode { .. }
        | Node::PinnedExpressionNode { .. }
        | Node::FlipFlopNode { .. }
        | Node::IfNode { .. }
        | Node::UnlessNode { .. }
        | Node::WhileNode { .. }
        | Node::UntilNode { .. }
        | Node::ForNode { .. }
        | Node::CaseNode { .. }
        | Node::CaseMatchNode { .. }
        | Node::WhenNode { .. } => Some(true),
        // Parentheses already supply an operand boundary, even if Prism has
        // omitted an intervening single-statement list.
        Node::ParenthesesNode { .. } => Some(false),
        _ => None,
    }
}
#[derive(Default)]
struct ConditionFacts {
    pattern: bool,
    assignment: bool,
    defined: Vec<Vec<u8>>,
    captures: Vec<bool>,
}
impl<'pr> NodeVisitor<'pr> for ConditionFacts {
    fn enter(&mut self, n: Node<'pr>) {
        self.pattern |= matches!(
            n,
            Node::MatchPredicateNode { .. } | Node::MatchRequiredNode { .. }
        );
        // MatchWrite's immediate targets bind captures, not lvasgns. Actual
        // assignments nested in its regexp/call operands still count.
        let capture = self.captures.last().copied().unwrap_or(false);
        self.captures.push(n.as_match_write_node().is_some());
        self.assignment |= variables::is_local_assignment(&n)
            && !(capture && n.as_local_variable_target_node().is_some());
        if let Some(d) = n.as_defined_node() {
            let value = d.value();
            if let Some(v) = value.as_local_variable_read_node() {
                self.defined.push(v.name().as_slice().to_vec());
            } else if value.as_call_node().is_some() {
                // RuboCop uses node_parts[0] for sends (the receiver), not the method name.
                self.defined.push(Vec::new());
            }
        }
    }
    fn leave(&mut self) {
        self.captures.pop();
    }
}
/// Add parentheses around a final omitted hash argument, including wrappers.
pub(super) fn body_source(n: &Node<'_>) -> Vec<u8> {
    if let Some(call) = n.as_call_node() {
        let args = ast::call_arguments(&call);
        if call.opening_loc().is_none()
            && call.name().as_slice() != b"[]="
            && args
                .last()
                .and_then(|a| a.as_keyword_hash_node())
                .and_then(|h| h.elements().iter().last())
                .and_then(|p| p.as_assoc_node())
                .is_some_and(|p| p.value().as_implicit_node().is_some())
            && let Some(first) = args.first()
        {
            let start = n.location().start_offset();
            let selector_end = call
                .message_loc()
                .map_or(first.location().start_offset(), |l| l.end_offset());
            let mut result = n.location().as_slice()[..selector_end - start].to_vec();
            result.push(b'(');
            for (i, arg) in args.iter().enumerate() {
                if i > 0 {
                    result.extend_from_slice(b", ");
                }
                result.extend_from_slice(arg.location().as_slice());
            }
            result.push(b')');
            // Prism includes literal blocks in the call's source. Closing the
            // arguments must not drop that suffix (or omit its display width).
            if let Some(last) = args.last() {
                result.extend_from_slice(
                    &n.location().as_slice()[last.location().end_offset() - start..],
                );
            }
            return result;
        }
    }
    let value = if let Some(v) = n.as_local_variable_write_node() {
        Some(v.value())
    } else if let Some(v) = n.as_instance_variable_write_node() {
        Some(v.value())
    } else if let Some(v) = n.as_class_variable_write_node() {
        Some(v.value())
    } else if let Some(v) = n.as_global_variable_write_node() {
        Some(v.value())
    } else if let Some(v) = n.as_constant_write_node() {
        Some(v.value())
    } else if let Some(v) = n.as_constant_path_write_node() {
        Some(v.value())
    } else {
        let args = if let Some(v) = n.as_return_node() {
            v.arguments()
        } else if let Some(v) = n.as_next_node() {
            v.arguments()
        } else if let Some(v) = n.as_break_node() {
            v.arguments()
        } else {
            None
        };
        args.filter(|a| a.arguments().len() == 1)
            .and_then(|a| a.arguments().iter().next())
    };
    if let Some(value) = value {
        let loc = n.location();
        let inner = value.location();
        let mut result = loc.as_slice()[..inner.start_offset() - loc.start_offset()].to_vec();
        result.extend_from_slice(&body_source(&value));
        result.extend_from_slice(&loc.as_slice()[inner.end_offset() - loc.start_offset()..]);
        result
    } else {
        n.location().as_slice().to_vec()
    }
}

/// Source-order facts are indexed once from the main parse, not rescanned for
/// every conditional in a long statement list or collection. No AST is retained.
#[derive(Default)]
pub(crate) struct ModifierIndex {
    assignments: HashMap<(usize, Vec<u8>), usize>,
    shared: HashSet<usize>,
    following: HashSet<usize>,
    modifier_shared: std::sync::OnceLock<HashSet<usize>>,
}
impl ModifierIndex {
    fn modifier_shared(&self, root: &Node<'_>, file: &SourceFile) -> &HashSet<usize> {
        self.modifier_shared.get_or_init(|| {
            struct Sharing<'a> {
                file: &'a SourceFile,
                shared: HashSet<usize>,
            }
            impl<'pr> NodeVisitor<'pr> for Sharing<'_> {
                fn enter(&mut self, n: Node<'pr>) {
                    #[cfg(test)]
                    CORRECTION_INDEX_VISITS.with(|v| v.set(v.get() + 1));
                    let Some(children) = collection_children(&n) else {
                        return;
                    };
                    let mut modifiers = HashMap::<u32, Vec<usize>>::new();
                    for child in children {
                        let key = child.as_assoc_node().map(|p| unwrap(p.key()));
                        for candidate in key.into_iter().chain(std::iter::once(unwrap(child))) {
                            if Conditional::of(&candidate).is_some_and(|c| c.end.is_none()) {
                                let at = candidate.location().start_offset();
                                modifiers.entry(self.file.line_of(at)).or_default().push(at);
                            }
                        }
                    }
                    for offsets in modifiers.values().filter(|v| v.len() > 1) {
                        self.shared.extend(offsets);
                    }
                }
            }
            let mut sharing = Sharing {
                file,
                shared: HashSet::new(),
            };
            walk(&mut sharing, root);
            sharing.shared
        })
    }
    pub(crate) fn build(root: &Node<'_>, file: &SourceFile) -> Self {
        struct Indexer<'a> {
            index: ModifierIndex,
            file: &'a SourceFile,
        }
        impl<'pr> NodeVisitor<'pr> for Indexer<'_> {
            fn enter(&mut self, n: Node<'pr>) {
                #[cfg(test)]
                INDEX_VISITS.with(|v| v.set(v.get() + 1));
                if let Some(s) = n.as_statements_node() {
                    let mut previous: Option<Node<'pr>> = None;
                    for child in s.body().iter() {
                        if let Some(write) = child.as_local_variable_write_node() {
                            self.index
                                .assignments
                                .entry((
                                    s.location().start_offset(),
                                    write.name().as_slice().to_vec(),
                                ))
                                .or_insert(child.location().start_offset());
                        }
                        if let Some(prev) = previous
                            && self.file.last_line_of(prev.location())
                                == self.file.line_of(child.location().start_offset())
                        {
                            self.index.following.insert(prev.location().start_offset());
                        }
                        previous = Some(child);
                    }
                }
                let Some(children) = collection_children(&n) else {
                    return;
                };
                let mut starts = HashSet::new();
                let mut ends = HashSet::new();
                let mut candidates = Vec::new();
                for child in children {
                    // Hash keys can share a line with a conditional value, but
                    // only values supply sibling boundaries (as in RuboCop).
                    if let Some(pair) = child.as_assoc_node() {
                        candidates.push(unwrap(pair.key()));
                    }
                    let child = unwrap(child);
                    if let Some(c) = Conditional::of(&child)
                        && let Some(end) = c.end
                    {
                        starts.insert(self.file.line_of(child.location().start_offset()));
                        ends.insert(self.file.line_of(end.start_offset()));
                    }
                    candidates.push(child);
                }
                for child in candidates {
                    if let Some(c) = Conditional::of(&child)
                        && let Some(end) = c.end
                    {
                        let at = child.location().start_offset();
                        if ends.contains(&self.file.line_of(at))
                            || starts.contains(&self.file.line_of(end.start_offset()))
                        {
                            self.index.shared.insert(at);
                        }
                    }
                }
            }
        }
        let mut builder = Indexer {
            index: Self::default(),
            file,
        };
        walk(&mut builder, root);
        builder.index
    }
}
fn collection_children<'pr>(n: &Node<'pr>) -> Option<Vec<Node<'pr>>> {
    Some(if let Some(a) = n.as_array_node() {
        a.elements().iter().collect()
    } else if let Some(h) = n.as_hash_node() {
        h.elements().iter().collect()
    } else if let Some(h) = n.as_keyword_hash_node() {
        h.elements().iter().collect()
    } else if let Some(c) = n.as_call_node() {
        ast::call_arguments(&c)
    } else if let Some(a) = n.as_and_node() {
        vec![a.left(), a.right()]
    } else {
        let a = n.as_or_node()?;
        vec![a.left(), a.right()]
    })
}
fn unwrap(mut n: Node<'_>) -> Node<'_> {
    if let Some(pair) = n.as_assoc_node() {
        n = pair.value();
    }
    if let Some(p) = n.as_parentheses_node()
        && let Some(body) = p.body().and_then(|b| b.as_statements_node())
        && let Some(first) = body.body().iter().next()
    {
        n = first;
    }
    n
}
fn following_statement(cx: &NodeContext<'_, '_>, node: &Node<'_>, index: &ModifierIndex) -> bool {
    let mut at = node.location().start_offset();
    for parent in cx.ancestors() {
        if parent
            .as_statements_node()
            .is_some_and(|s| s.body().len() > 1)
            || parent.as_parentheses_node().is_some()
        {
            return index.following.contains(&at);
        }
        at = parent.location().start_offset();
    }
    false
}
fn logical_modifiers_share_line(
    cx: &NodeContext<'_, '_>,
    node: &Node<'_>,
    file: &SourceFile,
) -> bool {
    let mut parents = cx
        .ancestors()
        .filter(|n| !n.as_statements_node().is_some_and(|s| s.body().len() == 1));
    let mut parent = parents.next();
    if parent.is_some_and(|p| p.as_parentheses_node().is_some()) {
        parent = parents.next();
    }
    let Some(parent) = parent.filter(|p| p.as_and_node().is_some() || p.as_or_node().is_some())
    else {
        return false;
    };
    struct Count<'a> {
        file: &'a SourceFile,
        line: u32,
        count: usize,
    }
    impl<'pr> NodeVisitor<'pr> for Count<'_> {
        fn enter(&mut self, n: Node<'pr>) {
            if Conditional::of(&n).is_some_and(|c| c.end.is_none())
                && self.file.line_of(n.location().start_offset()) == self.line
            {
                self.count += 1;
            }
        }
    }
    let mut count = Count {
        file,
        line: file.line_of(node.location().start_offset()),
        count: 0,
    };
    walk(&mut count, parent);
    count.count > 1
}
// A fixed per-candidate budget bounds reconstruction and lexical walks even
// for deeply nested eligible expressions. Main-parse safety indexes are cached.
const REWRITE_BUDGET: usize = 16 * 1024;
#[derive(Default)]
struct LexicalFacts {
    declarations: bool,
    capture: bool,
    ambiguous: bool,
    names: HashSet<Vec<u8>>,
    references: HashSet<Vec<u8>>,
    calls: HashSet<Vec<u8>>,
    omitted_hash: bool,
    endless_range: bool,
}
impl<'pr> NodeVisitor<'pr> for LexicalFacts {
    fn enter(&mut self, n: Node<'pr>) {
        self.declarations |= variables::is_local_assignment(&n);
        self.endless_range |= n.as_range_node().is_some_and(|r| r.right().is_none());
        self.omitted_hash |= n
            .as_keyword_hash_node()
            .and_then(|h| h.elements().iter().last())
            .and_then(|p| p.as_assoc_node())
            .is_some_and(|p| p.value().as_implicit_node().is_some());
        self.capture |= matches!(
            n,
            Node::MatchWriteNode { .. }
                | Node::MatchPredicateNode { .. }
                | Node::MatchRequiredNode { .. }
        );
        self.ambiguous |= matches!(
            n,
            Node::BeginNode { .. }
                | Node::RescueModifierNode { .. }
                | Node::DefNode { .. }
                | Node::ClassNode { .. }
                | Node::ModuleNode { .. }
                | Node::SingletonClassNode { .. }
                | Node::WhileNode { .. }
                | Node::UntilNode { .. }
                | Node::ForNode { .. }
                | Node::CaseNode { .. }
                | Node::CaseMatchNode { .. }
                | Node::PreExecutionNode { .. }
                | Node::PostExecutionNode { .. }
        );
        let name = match n {
            Node::LocalVariableWriteNode { .. } => {
                Some(n.as_local_variable_write_node().unwrap().name())
            }
            Node::LocalVariableTargetNode { .. } => {
                Some(n.as_local_variable_target_node().unwrap().name())
            }
            Node::LocalVariableAndWriteNode { .. } => {
                Some(n.as_local_variable_and_write_node().unwrap().name())
            }
            Node::LocalVariableOrWriteNode { .. } => {
                Some(n.as_local_variable_or_write_node().unwrap().name())
            }
            Node::LocalVariableOperatorWriteNode { .. } => {
                Some(n.as_local_variable_operator_write_node().unwrap().name())
            }
            _ => None,
        };
        if let Some(name) = name {
            self.names.insert(name.as_slice().to_vec());
        }
        if let Some(call) = n.as_call_node().filter(|c| c.is_variable_call()) {
            self.references.insert(call.name().as_slice().to_vec());
            self.calls.insert(call.name().as_slice().to_vec());
        } else if let Some(read) = n.as_local_variable_read_node() {
            self.references.insert(read.name().as_slice().to_vec());
        }
    }
}
/// Shared lexical proof for moving a pre-test predicate after its body. Reads
/// already bound as locals keep their binding; only bare calls can be captured
/// by a declaration moved before them.
pub(super) fn compression_binding_safe(body: &Node<'_>, predicate: &Node<'_>) -> bool {
    let mut condition = LexicalFacts::default();
    walk(&mut condition, predicate);
    let mut branch = LexicalFacts::default();
    walk(&mut branch, body);
    !condition.declarations
        && !condition.capture
        && !branch.capture
        && !branch.ambiguous
        && !branch
            .names
            .iter()
            .any(|name| condition.calls.contains(name))
}

/// Shared lexical proof for expanding modifier syntax. Uses the same local
/// assignment classification as the scope analysis; never reparse a candidate.
pub(super) fn expansion_binding_safe(body: &Node<'_>, predicate: &Node<'_>) -> bool {
    let mut condition = LexicalFacts::default();
    walk(&mut condition, predicate);
    let mut branch = LexicalFacts::default();
    walk(&mut branch, body);
    !condition.declarations
        && !condition.capture
        && !branch.capture
        && !branch.ambiguous
        && !branch
            .names
            .iter()
            .any(|name| condition.references.contains(name))
}

#[cfg(test)]
thread_local! { static CORRECTION_INDEX_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
thread_local! { static INDEX_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
fn regexp_match_statement(condition: &Node<'_>) -> bool {
    condition.as_match_write_node().is_some()
        || condition.as_call_node().is_some_and(|call| {
            call.name().as_slice() == b"=~"
                && call.call_operator_loc().is_none()
                && call
                    .receiver()
                    .is_some_and(|receiver| receiver.as_regular_expression_node().is_some())
        })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's if_unless_modifier_spec.rb and
    //! spec/support/condition_modifier_cop.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str;
    fn found(source: &str) -> Vec<Offense> {
        assert!(
            ruby_prism::parse(source.as_bytes())
                .errors()
                .next()
                .is_none(),
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/IfUnlessModifier")
            .collect()
    }
    #[test]
    fn static_regexp_match_conditions_keep_the_block_form() {
        for source in [
            "if /\\Aevent_/ =~ name\n  work\nend\n",
            "unless /\\Aevent_/ =~ name\n  work\nend\n",
            "if /(?<head>\\w+)/ =~ name\n  work\nend\n",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
        for source in [
            "if name =~ /\\Aevent_/\n  work\nend\n",
            "if /#{prefix}_/ =~ name\n  work\nend\n",
            "if /\\Aevent_/.=~(name)\n  work\nend\n",
            "if /\\Aevent_/.match?(name)\n  work\nend\n",
            "if (/\\Aevent_/ =~ name)\n  work\nend\n",
        ] {
            assert_eq!(found(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn canonical_safe_corrections() {
        use crate::cops::testing::assert_correction;
        for keyword in ["if", "unless"] {
            assert_correction(
                &IfUnlessModifier::default(),
                &format!("{keyword} ready\n  work\nend\n"),
                &format!("work {keyword} ready\n"),
            );
            assert_correction(
                &IfUnlessModifier::default(),
                &format!("x = {keyword} ready\n  1\nend\n"),
                &format!("x = (1 {keyword} ready)\n"),
            );
            assert_correction(
                &IfUnlessModifier { max: Some(15) },
                &format!("  work {keyword} long_condition\n"),
                &format!("  {keyword} long_condition\n    work\n  end\n"),
            );
        }
    }
    #[test]
    fn range_endpoints_keep_their_conditional_boundaries() {
        use crate::cops::testing::assert_correction;
        for keyword in ["if", "unless"] {
            for operator in ["..", "..."] {
                assert_correction(
                    &IfUnlessModifier::default(),
                    &format!("p((1{operator}{keyword} false\n  3\nend))\n"),
                    &format!("p((1{operator}(3 {keyword} false)))\n"),
                );
                assert_correction(
                    &IfUnlessModifier::default(),
                    &format!("p(({keyword} false\n  1\nend{operator}3))\n"),
                    &format!("p(((1 {keyword} false){operator}3))\n"),
                );
            }
        }
    }
    #[test]
    fn match_and_splat_operands_keep_exact_boundaries_without_changing_detection() {
        use crate::cops::testing::assert_correction;
        for keyword in ["if", "unless"] {
            for (prefix, body, suffix) in [
                ("p((", "3", " in Integer))\n"),
                ("", "3", " => Integer\n"),
                ("p((", "3", " in Integer => captured)); p captured\n"),
                ("p([*", "3", "])\n"),
                ("p({**", "{}", "})\n"),
                ("p({**", "3", "})\n"),
                ("p(*", "3", ")\n"),
                ("p(**", "{}", ")\n"),
                ("p(defined?(", "3", "))\n"),
                ("while ", "false", "\n break\nend\n"),
                ("if ", "true", "..true\n p :yes\nend\n"),
                ("if true..", "true", "\n p :yes\nend\n"),
                ("until ", "true", "\n break\nend\n"),
                ("if ", "true", "\n p :yes\nend\n"),
                ("unless ", "true", "\n p :yes\nend\n"),
                ("case ", "3", "\nwhen nil then p :nil\nend\n"),
                ("case ", "3", "\nin nil then p :nil\nend\n"),
                ("case nil\nwhen ", "3", " then p :nil\nend\n"),
                ("for x in ", "[]", "\n p x\nend\n"),
                ("case nil; in ^(", "3", "); p :nil; end\n"),
            ] {
                let before = format!("{prefix}{keyword} false\n {body}\nend{suffix}");
                let after = format!("{prefix}({body} {keyword} false){suffix}");
                let findings = found(&before);
                assert_eq!(findings.len(), 1, "{before}");
                // These newly guarded parents retain their detection message.
                assert_eq!(findings[0].message, message(keyword, false), "{before}");
                assert_correction(&IfUnlessModifier::default(), &before, &after);
            }
        }
    }
    #[test]
    fn uncertain_wrapper_corrections_are_individually_withheld() {
        use crate::cops::testing::correction_offenses;
        for keyword in ["if", "unless"] {
            let source = format!("p(({keyword} false\n 3\nend rescue 9))\nif ready\n work\nend\n");
            let offenses = correction_offenses(&IfUnlessModifier::default(), &source);
            assert_eq!(offenses.len(), 2);
            assert!(offenses[0].correction.is_none(), "{source}");
            assert!(offenses[1].correction.is_some(), "{source}");
            // Dynamic block conversion is also subject to the existing global
            // heap-observation barrier; do not weaken it to force a correction.
            let source = format!("p(&{keyword} false\n nil\nend)\n");
            let offenses = correction_offenses(&IfUnlessModifier::default(), &source);
            assert_eq!(offenses.len(), 1);
            assert!(offenses[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn correction_only_parentheses_must_still_fit_the_configured_width() {
        use crate::cops::testing::correction_offenses;
        let source = "p([*if false\n 3\nend])\n";
        // Detection still uses its existing unparenthesized prospective width.
        let detected_width = "p([*3 if false])".len();
        for extra in [0, 1, 2] {
            let offenses = correction_offenses(
                &IfUnlessModifier {
                    max: Some(detected_width + extra),
                },
                source,
            );
            assert_eq!(offenses.len(), 1);
            assert_eq!(offenses[0].correction.is_some(), extra == 2);
        }
    }
    #[test]
    fn terminal_endless_ranges_are_reported_without_edits() {
        use crate::cops::testing::{assert_correction, correction_offenses};
        for keyword in ["if", "unless"] {
            for operator in ["..", "..."] {
                for body in [
                    format!("1{operator}"),
                    format!("x = 1{operator}"),
                    format!("x ||= 1{operator}"),
                    format!("object.x = 1{operator}"),
                    format!("object[0] = 1{operator}"),
                    format!("foo 1{operator}"),
                    format!("return 1{operator}"),
                    format!("yield 1{operator}"),
                ] {
                    let source = format!("def f\n {keyword} true\n  {body}\n end\nend\n");
                    assert!(
                        ruby_prism::parse(source.as_bytes())
                            .errors()
                            .next()
                            .is_none(),
                        "{source}"
                    );
                    let offenses = correction_offenses(&IfUnlessModifier::default(), &source);
                    assert_eq!(offenses.len(), 1, "{source}");
                    assert!(offenses[0].correction.is_none(), "{source}: {offenses:?}");
                }
                // Explicit boundaries already make these bodies safe.
                assert_correction(
                    &IfUnlessModifier::default(),
                    &format!("{keyword} true\n foo(1{operator})\nend\n"),
                    &format!("foo(1{operator}) {keyword} true\n"),
                );
            }
        }
    }
    #[test]
    fn lexical_order_and_ambiguous_control_flow_are_withheld() {
        use crate::cops::testing::correction_offenses;
        let cop = IfUnlessModifier { max: Some(15) };
        for source in [
            "if value\n value = 1\nend",
            "if ready\n work rescue recover\nend",
            "if ready\n begin; work; rescue; recover; ensure; cleanup; end\nend",
            "if ready\n def f; 1; end\nend",
            "if ready\n [1] => [value]\nend",
            "if ready\n END { effect }\nend",
            "value = work if value",
            "work(value) if (value = ready)",
            "work if /(?<value>x)/ =~ input",
            "before; work if long_condition",
        ] {
            assert!(
                ruby_prism::parse(source.as_bytes())
                    .errors()
                    .next()
                    .is_none(),
                "{source}"
            );
            let offenses = correction_offenses(&cop, source);
            // Use no length limit for normal forms; the reverse forms remain long.
            let offenses = if source.starts_with("if ") {
                correction_offenses(&IfUnlessModifier { max: None }, source)
            } else {
                offenses
            };
            assert!(!offenses.is_empty(), "{source}");
            assert!(
                offenses.iter().all(|o| o.correction.is_none()),
                "{source}: {offenses:?}"
            );
        }
    }
    #[test]
    fn comments_omitted_hash_arguments_and_nested_guards_correct_atomically() {
        use crate::cops::testing::assert_correction;
        let cop = IfUnlessModifier::default();
        for (before, after) in [
            (
                "if ready # ordinary\n work\nend\n",
                "work if ready # ordinary\n",
            ),
            (
                "if ready\n object.call 1,    value:\nend\n",
                "object.call(1, value:) if ready\n",
            ),
            (
                "if outer\n if inner\n  return value\n end\nelse\n fallback\nend\n",
                "if outer\n return value if inner\nelse\n fallback\nend\n",
            ),
            (
                "begin\n if ready\n  work\n end\nrescue\n recover\nensure\n cleanup\nend\n",
                "begin\n work if ready\nrescue\n recover\nensure\n cleanup\nend\n",
            ),
        ] {
            assert_correction(&cop, before, after);
        }
        assert_correction(
            &IfUnlessModifier { max: Some(30) },
            "work if ready # a very very very long ordinary comment\n",
            "# a very very very long ordinary comment\nwork if ready\n",
        );
    }
    #[test]
    fn source_sensitive_edits_remain_reported_without_corrections() {
        use crate::cops::testing::correction_offenses;
        for source in [
            "if ready # encoding: ascii-8bit\n work\nend\n",
            "if ready # frozen-string-literal: true\n work\nend\n",
            "if ready\n __LINE__\nend\n",
            "if ready\n work\nend\np __LINE__\n",
            "if ready\n work\nend\n# rustocop:disable Layout\n",
            "if ready\n \"a\nb\"\nend\n",
            "if ready\n work(<<~TXT)\nhello\nTXT\nend\n",
        ] {
            let found = correction_offenses(&IfUnlessModifier::default(), source);
            if source.contains("# rustocop:disable Layout") {
                // A directive on a later untouched line does not block this edit.
                assert!(found[0].correction.is_some());
            } else if source.contains("\"a\nb\"") || source.contains("<<~TXT") {
                assert!(
                    found.is_empty(),
                    "multiline body remains detection-exempt: {source}"
                );
            } else {
                assert_eq!(found.len(), 1, "{source}");
                assert!(
                    found.iter().all(|o| o.correction.is_none()),
                    "{source}: {found:?}"
                );
            }
        }
        for source in [
            "work if ready #! /usr/bin/ruby with a very long comment\n",
            "work if ready # coding: ascii-8bit with a very long comment\n",
            "long_work if ready # rustocop:disable Style/StringLiterals\n",
            "work if long_condition\np __LINE__\n",
        ] {
            let found = correction_offenses(&IfUnlessModifier { max: Some(15) }, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn shorthand_argument_parenthesizing_keeps_literal_blocks_and_their_width() {
        use crate::cops::testing::assert_correction;
        let before = "if ready\n object.call value: do; effect; end\nend\n";
        let after = "object.call(value:) do; effect; end if ready\n";
        assert_correction(&IfUnlessModifier::default(), before, after);
        assert!(with_limit(before, Some(after.trim_end().len() - 1)).is_empty());
        assert_eq!(with_limit(before, Some(after.trim_end().len())).len(), 1);
    }
    #[test]
    fn unsupported_omitted_hash_wrappers_stay_reported_instead_of_breaking_syntax() {
        use crate::cops::testing::correction_offenses;
        for body in [
            "x ||= object.call value:",
            "x += object.call value:",
            "object.x = object.call value:",
            "object[0] = object.call value:",
            "x, y = object.call value:",
            "return foo, value:",
            "yield value:",
            "super value:",
        ] {
            let source = format!("def f\n if ready\n  {body}\n end\nend\n");
            assert!(
                ruby_prism::parse(source.as_bytes())
                    .errors()
                    .next()
                    .is_none()
            );
            let found = correction_offenses(&IfUnlessModifier::default(), &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn bom_and_crlf_do_not_prevent_canonical_expansion() {
        use crate::cops::testing::assert_correction;
        assert_correction(
            &IfUnlessModifier { max: Some(15) },
            "\u{feff}work if long_condition\r\n",
            "\u{feff}if long_condition\r\n  work\r\nend\r\n",
        );
        assert_correction(
            &IfUnlessModifier { max: Some(20) },
            "work if ready # a very long ordinary comment\r\n",
            "# a very long ordinary comment\r\nwork if ready\r\n",
        );
    }
    #[test]
    fn correction_work_is_lazy_and_bounded() {
        use crate::cops::testing::correction_offenses;
        let cop = IfUnlessModifier::default();
        let source = "if ready\n work # no correction\nend\n".repeat(2000);
        REWRITE_BYTES.with(|v| v.set(0));
        CORRECTION_INDEX_VISITS.with(|v| v.set(0));
        crate::pipeline::lint_str_with(&[&cop], &source);
        REWRITE_BYTES.with(|v| assert_eq!(v.get(), 0));
        CORRECTION_INDEX_VISITS.with(|v| assert_eq!(v.get(), 0));
        let source = "if ready\n work\nend\n".repeat(2000);
        let found = correction_offenses(&cop, &source);
        assert_eq!(
            found.iter().filter(|o| o.correction.is_some()).count(),
            2000
        );
        REWRITE_BYTES.with(|v| assert!(v.get() <= source.len()));
        CORRECTION_INDEX_VISITS.with(|v| assert!(v.get() > 0 && v.get() < 14_000));
        let source = format!("if ready\n \"{}\"\nend\n", "x".repeat(REWRITE_BUDGET));
        REWRITE_BYTES.with(|v| v.set(0));
        let found = correction_offenses(&IfUnlessModifier { max: None }, &source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
        REWRITE_BYTES.with(|v| assert_eq!(v.get(), 0));
        for source in [
            format!("{}work if long_condition\n", " ".repeat(REWRITE_BUDGET + 1)),
            format!("work if ready # {}\n", "c".repeat(REWRITE_BUDGET + 1)),
        ] {
            let found = correction_offenses(&cop, &source);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn incompatible_peers_keep_findings_and_withhold_only_modifier_edits() {
        use crate::{
            cops::style::{next::Next, sole_nested_conditional::SoleNestedConditional},
            pipeline::lint_str_with_corrections,
        };
        let cop = IfUnlessModifier::default();
        assert_eq!(
            cop.autocorrect_incompatible_with(),
            &["Style/Next", "Style/SoleNestedConditional"]
        );
        for peer in [&Next as &dyn Cop, &SoleNestedConditional] {
            let source = "if ready\n work\nend\n";
            let found = lint_str_with_corrections(&[&cop, peer], source);
            let found: Vec<_> = found.iter().filter(|o| o.cop == cop.name()).collect();
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none());
        }
    }
    #[derive(serde::Deserialize)]
    struct PinnedCase {
        spec_line: u32,
        source: String,
        positions: Vec<(u32, u32)>,
        messages: Vec<String>,
        correction: Option<String>,
        no_correction: bool,
        toml: String,
    }
    #[test]
    fn every_applicable_pinned_call_and_correction() {
        use crate::cops::testing::{assert_correction, correction_offenses};
        let cases: Vec<PinnedCase> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/if_unless_modifier_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 162);
        let (mut canonical, mut withheld) = (0, 0);
        for c in cases {
            let config: toml::Value = toml::from_str(&c.toml).unwrap();
            let limit = &config["cops"]["Layout/LineLength"];
            let cop = IfUnlessModifier {
                max: if limit.get("enabled").and_then(toml::Value::as_bool) == Some(false) {
                    None
                } else {
                    Some(
                        limit
                            .get("max")
                            .and_then(toml::Value::as_integer)
                            .unwrap_or(120) as usize,
                    )
                },
            };
            let found = crate::pipeline::lint_str_with(&[&cop], &c.source);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                c.positions,
                "spec {}",
                c.spec_line
            );
            for (offense, expected) in found.iter().zip(&c.messages) {
                if let Some(prefix) = expected.strip_suffix("[...]") {
                    assert!(offense.message.starts_with(prefix), "spec {}", c.spec_line);
                } else {
                    assert_eq!(&offense.message, expected, "spec {}", c.spec_line);
                }
            }
            if let Some(after) = c.correction {
                if matches!(
                    c.spec_line,
                    163 | 182 | 201 | 218 | 241 | 266 | 291 | 314 | 337 | 360
                ) {
                    assert!(c.source.contains("<<"));
                    let offenses = correction_offenses(&cop, &c.source);
                    assert!(
                        offenses.iter().all(|o| o.correction.is_none()),
                        "spec {}",
                        c.spec_line
                    );
                    assert_eq!(
                        crate::corrections::apply(c.source.as_bytes(), &offenses)
                            .unwrap()
                            .bytes,
                        c.source.as_bytes()
                    );
                    withheld += 1;
                } else {
                    assert_correction(&cop, &c.source, &after);
                    canonical += 1;
                }
            } else if c.positions.is_empty() || c.no_correction {
                assert!(
                    correction_offenses(&cop, &c.source)
                        .iter()
                        .all(|o| o.correction.is_none()),
                    "spec {}",
                    c.spec_line
                );
            }
        }
        assert_eq!((canonical, withheld), (73, 10));
    }
    fn message(keyword: &str, parens: bool) -> String {
        format!(
            "Favor modifier `{keyword}` usage when having a single-line body. {}",
            if parens {
                "Wrap the expression in parentheses to keep the current behavior, as it is part of a larger expression."
            } else {
                "Another good alternative is the usage of control flow `&&`/`||`."
            }
        )
    }
    #[test]
    fn single_statement_and_exact_keyword_positions() {
        for keyword in ["if", "unless"] {
            for body in [
                "do_something",
                "x = 0",
                "return value",
                "obj[:key] = { foo: }",
                "obj&.call foo:",
            ] {
                let source = format!("def f\n  {keyword} condition\n    {body}\n  end\nend\n");
                let out = found(&source);
                assert_eq!(out.len(), 1, "{source}");
                assert_eq!((out[0].line, out[0].col, out[0].end_line), (2, 3, 2));
                assert_eq!(out[0].message, message(keyword, false));
                assert!(out[0].correction.is_none());
            }
            assert_eq!(
                found(&format!("{keyword} foo; bar; end"))[0].message,
                message(keyword, false)
            );
        }
    }
    fn with_limit(source: &str, max: Option<usize>) -> Vec<Offense> {
        assert!(
            ruby_prism::parse(source.as_bytes())
                .errors()
                .next()
                .is_none(),
            "{source}"
        );
        crate::pipeline::lint_str_with(&[&IfUnlessModifier { max }], source)
    }
    #[test]
    fn precedence_and_branch_return_values() {
        for source in [
            "x = if a\n 1\nend",
            "@x = unless a\n 1\nend",
            "a + if b\n 1\nend",
            "a || if b\n 1\nend",
            "if b\n 1\nend || a",
            "puts(if a\n 1\nend)",
            "[if a\n 1\nend]",
            "{x: if a\n 1\nend}",
            "def f = if a\n 1\nend",
            "def f; yield(if a\n 1\nend); end",
            "def f; super(if a\n 1\nend); end",
            "a ? if b\n 1\nend : 2",
            "a ? 1 : if b\n 2\nend",
        ] {
            let out = found(source);
            assert_eq!(out.len(), 1, "{source}");
            assert_eq!(
                out[0].message,
                message(
                    if source.contains("unless") {
                        "unless"
                    } else {
                        "if"
                    },
                    true
                ),
                "{source}"
            );
        }
        assert_eq!(found("(if a\n 1\nend)")[0].message, message("if", false));
    }
    #[test]
    fn defined_requires_a_direct_earlier_sibling_assignment() {
        for source in [
            "value = :custom\nunless defined?(value)\n value = :default\nend",
            "unless defined?(yield)\n value = :default\nend",
            "unless defined?(super)\n value = :default\nend",
            "unless defined?(Object)\n value = :default\nend",
        ] {
            assert_eq!(found(source).len(), 1, "{source}");
        }
        for source in [
            "other = 1\nunless condition && defined?(value)\n value = :default\nend",
            "value = 1\ndef f\n unless defined?(value)\n  value = :default\n end\nend",
            "def f(value)\n unless defined?(value)\n  value = :default\n end\nend",
            "if a\n work\n value = 1\nend\nunless defined?(value)\n value = :default\nend",
            "unless defined?(value)\n value = :default\nend\nvalue = 1",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn assignment_patterns_and_branch_effects_are_ast_based() {
        for condition in [
            "(x, y = values)",
            "(x &&= foo)",
            "(x += 1)",
            "(for x in items; work; end)",
            "ok && /(?<foo>x)/ =~ (x, y = values)",
            "ok && ([42] in [x])",
        ] {
            let source = format!("x = 0\nif {condition}\n puts x\nend");
            assert!(found(&source).is_empty(), "{source}");
        }
        for condition in [
            "(@x = foo)",
            "(object.x = foo)",
            "ok && /(?<foo>\\d)/ =~ text",
            "object.update!",
        ] {
            let source = format!("if {condition}\n object.update!\nend");
            assert_eq!(found(&source).len(), 1, "{source}");
        }
        assert_eq!(
            found("items.each do |item|\n if condition\n  next(item.foo)\n end\nend").len(),
            1
        );
    }
    #[test]
    fn nested_guards_else_association_and_rescue_ensure() {
        let out = found("if outer\n if inner\n  return value\n end\nelse\n fallback\nend");
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].line, out[0].col), (2, 2));
        for source in [
            "begin\n if ready\n  work\n end\nrescue\n recover\nensure\n cleanup\nend",
            "if ready\n begin; work; rescue; recover; ensure; cleanup; end\nend",
            "if ready\n work rescue recover\nend",
            "def f\n if ready\n  return __LINE__\n end\nensure\n cleanup\nend",
        ] {
            assert_eq!(found(source).len(), 1, "{source}");
        }
        assert!(
            found("if ready\n begin\n  work\n rescue\n  recover\n ensure\n  cleanup\n end\nend")
                .is_empty()
        );
    }
    #[test]
    fn prospective_length_accounts_for_context_comments_and_tabs() {
        for (prefix, suffix, length) in [
            ("", "", 75),
            ("x = ", "", 69),
            ("  ", "", 73),
            ("[", "]", 71),
        ] {
            for delta in [0, 1] {
                let source = format!("{prefix}if a\n {}\nend{suffix}", "b".repeat(length + delta));
                assert_eq!(
                    with_limit(&source, Some(80)).len(),
                    usize::from(delta == 0),
                    "{source}"
                );
            }
        }
        for (length, expected) in [(67, 1), (68, 0)] {
            assert_eq!(
                with_limit(
                    &format!("if foo # {}\n bar\nend", "c".repeat(length)),
                    Some(80)
                )
                .len(),
                expected
            );
        }
        assert!(with_limit("[if foo # keep\n bar\nend]", None).is_empty());
        assert_eq!(with_limit("if a\n b\n\n\nend", Some(10)).len(), 1);
        assert_eq!(with_limit("\t\tif a\n\t\t b\n\t\tend", Some(10)).len(), 1);
        assert!(with_limit("\t\tif aa\n\t\t b\n\t\tend", Some(10)).is_empty());
    }
    #[test]
    fn omitted_hash_arguments_include_added_parentheses_in_length() {
        for wrapper in ["", "return ", "x = "] {
            let source = format!("def f(foo)\n if a\n  {wrapper}object.call foo:\n end\nend");
            let expected = format!(" {wrapper}object.call(foo:) if a").len();
            assert_eq!(with_limit(&source, Some(expected)).len(), 1, "{source}");
            assert!(
                with_limit(&source, Some(expected - 1)).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn omitted_hash_argument_separators_are_normalized_for_length() {
        for separator in [",", ", ", ",    "] {
            for length in [105, 106] {
                let name = "f".repeat(length);
                let source = format!("def f(bar)\n if a\n  {name} 1{separator}bar:\n end\nend");
                assert_eq!(format!(" {name}(1, bar:) if a").len(), length + 15);
                let out = found(&source);
                assert_eq!(out.len(), usize::from(length == 105), "{source}");
                if let Some(offense) = out.first() {
                    assert_eq!((offense.line, offense.col), (2, 2));
                    assert_eq!(offense.message, message("if", false));
                }
            }
        }
    }
    #[test]
    fn long_modifiers_report_keywords_with_heredocs_or_trailing_comments() {
        for keyword in ["if", "unless"] {
            let body = "f".repeat(80);
            for source in [
                format!("{body} {keyword} a"),
                format!("{body}(<<~EOS) {keyword} a\n text\nEOS"),
            ] {
                let out = with_limit(&source, Some(80));
                assert_eq!(out.len(), 1, "{source}");
                let col = source
                    .lines()
                    .next()
                    .unwrap()
                    .find(&format!(" {keyword}"))
                    .unwrap() as u32
                    + 2;
                assert_eq!((out[0].line, out[0].col, out[0].end_line), (1, col, 1));
                assert_eq!(
                    out[0].message,
                    format!("Modifier form of `{keyword}` makes the line too long.")
                );
            }
        }
        let source = format!("work if ready # {}", "c".repeat(80));
        assert_eq!(with_limit(&source, Some(80)).len(), 1);
        assert!(with_limit(&source, None).is_empty());
        assert!(with_limit(&format!("work if {} ||\n other", "x".repeat(90)), Some(80)).is_empty());
    }
    #[test]
    fn exemptions_apply_to_existing_lines_not_prospective_lines() {
        for source in [
            format!("work if url == 'https://example.com/{}'", "x".repeat(90)),
            format!("work if ready # rubocop:disable {}", "x".repeat(90)),
            format!("{} if ready; following", "x".repeat(90)),
            format!(
                "# rustocop:disable Layout/LineLength\n{} if ready",
                "x".repeat(90)
            ),
            format!("\"#{{{} if ready}}\"", "x".repeat(90)),
        ] {
            assert!(with_limit(&source, Some(80)).is_empty(), "{source}");
        }
        let source = format!(
            "if url == 'https://example.com/{}'\n work\nend",
            "x".repeat(90)
        );
        assert!(with_limit(&source, Some(80)).is_empty());
        assert_eq!(with_limit(&source, None).len(), 1);
        assert_eq!(
            with_limit(&format!("{} if ready", "x".repeat(90)), Some(80)).len(),
            1
        );
    }
    #[test]
    fn collections_and_logical_operators_distinguish_normal_and_modifier_forms() {
        for source in [
            "[if a\n b\nend, if c\n d\nend]",
            "foo((if a\n b\nend), (unless c\n d\nend))",
            "{x: if a\n b\nend, y: if c\n d\nend}",
            "x = (if a\n b\nend) || (if c\n d\nend)",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
        assert_eq!(found("[(if a\n b\nend),\n(if c\n d\nend)]").len(), 2);
        let a = "a".repeat(70);
        assert_eq!(
            with_limit(&format!("[({a} if x), ({a} unless y)]"), Some(80)).len(),
            2
        );
        assert!(with_limit(&format!("({a} if x) || ({a} unless y)"), Some(80)).is_empty());
    }
    #[test]
    fn keyword_hash_collections_exempt_shared_line_conditionals() {
        for source in [
            "foo(x: if a\n b\nend, y: if c\n d\nend)",
            "foo x: if a\n b\nend, y: if c\n d\nend",
            "foo(x: (if a\n b\nend), y: (unless c\n d\nend))",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
        for source in [
            "foo(x: if a\n b\nend,\ny: if c\n d\nend)",
            "foo x: if a\n b\nend,\ny: if c\n d\nend",
        ] {
            let out = found(source);
            assert_eq!(out.len(), 2, "{source}");
            assert_eq!((out[0].line, out[0].col), (1, 8));
            assert_eq!((out[1].line, out[1].col), (4, 4));
            assert!(out.iter().all(|o| o.message == message("if", true)));
        }
    }
    #[test]
    fn conditional_hash_keys_compare_only_against_value_boundaries() {
        let source = "{ if a\n b\nend => 1, x: if c\n d\nend }";
        let out = found(source);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].line, out[0].col), (3, 14));
        assert_eq!(out[0].message, message("if", true));
        // Keys are candidates for the exemption, not sibling value boundaries.
        let out = found("{ if a\n b\nend => 1, if c\n d\nend => 2 }");
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].line, out[0].col), (1, 3));
        assert_eq!((out[1].line, out[1].col), (3, 11));
    }
    #[test]
    fn source_order_index_is_built_once_for_many_siblings() {
        let source = (0..1000)
            .map(|i| format!("v{i} = 1\nunless defined?(v{i})\n v{i} = 2\nend\n"))
            .collect::<String>();
        INDEX_VISITS.with(|v| v.set(0));
        assert_eq!(with_limit(&source, Some(120)).len(), 1000);
        INDEX_VISITS.with(|v| assert!(v.get() < 14_000, "{} visits", v.get()));
        let source = format!("[\n{}\n]", "(if a\n b\nend),\n".repeat(1000));
        INDEX_VISITS.with(|v| v.set(0));
        assert_eq!(with_limit(&source, Some(120)).len(), 1000);
        INDEX_VISITS.with(|v| assert!(v.get() < 10_000, "{} visits", v.get()));
    }
    #[test]
    fn excludes_non_simple_branches_and_scope_sensitive_conditions() {
        for source in [
            "if a\nend",
            "if a\n b; c\nend",
            "if a\n b\n c\nend",
            "if a\n b\nelse\n c\nend",
            "if a\n b\nelsif c\n d\nend",
            "a ? b : c",
            "if a\n b ? c : d\nend",
            "if a\n b if c\nend",
            "if (a = foo)\n puts a\nend",
            "if (a ||= foo)\n puts a\nend",
            "if [42] in [x]\n x\nend",
            "if /(?<foo>\\d)/ =~ 'bar'\n foo\nend",
            "if a\n def method_name = body\nend",
            "if a\n def self.method_name = body\nend",
            "unless defined?(value)\n value = :default\nend",
            "if a\n b # comment\nend",
            "if a\n b\nend # comment",
            "if a\n b\n # comment\nend",
            "if a\n b\nend.inspect",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
    }
}
