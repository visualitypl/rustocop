//! `Style/ConditionalAssignment` — extract matching branch assignments safely.

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{Node, NodeList};
use std::collections::HashSet;

#[cfg(test)]
thread_local! {
    static LENGTH_SOURCE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub struct ConditionalAssignment {
    max_line_length: Option<usize>,
}

impl Default for ConditionalAssignment {
    fn default() -> Self {
        Self {
            max_line_length: Some(120),
        }
    }
}

impl Cop for ConditionalAssignment {
    fn name(&self) -> &'static str {
        "Style/ConditionalAssignment"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &["Style/MethodCallWithArgsParentheses"]
    }
    fn with_line_length_limit(&self, max: Option<usize>) -> Option<Box<dyn Cop>> {
        Some(Box::new(Self {
            max_line_length: max,
        }))
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(branches) = branches(node) else {
            return;
        };
        let Some(assignments) = branches.iter().map(assignment).collect::<Option<Vec<_>>>() else {
            return;
        };
        if assignments.is_empty()
            || !assignments
                .iter()
                .all(|a| a.kind == assignments[0].kind && a.lhs == assignments[0].lhs)
        {
            return;
        }
        let ternary = node
            .as_if_node()
            .is_some_and(|n| n.if_keyword_loc().is_none());
        let if_unless = node.as_if_node().is_some() || node.as_unless_node().is_some();
        for (index, assignment) in assignments.iter().enumerate() {
            // The pinned correctors bracket implicit arrays only at the first
            // if/unless branch and the final else, never elsif/when/in or a
            // ternary. Other argument-only syntax needs separate checks.
            let brackets_array =
                !ternary && (index + 1 == assignments.len() || (if_unless && index == 0));
            // A bare rescue before `:` cannot parse after extraction. The
            // final branch can use rescue, as can a parenthesized value.
            if ternary && index == 0 && assignment.value.as_rescue_modifier_node().is_some() {
                return;
            }
            let allows_match = !ternary && !(if_unless && index == 0);
            if !standalone_value(&assignment.value, brackets_array, allows_match) {
                return;
            }
        }
        if let Some(max) = self.max_line_length {
            // Mirror the pinned cop's hypothetical-correction length gate.
            // Linear regex matching handles source-sensitive LHS whitespace literally.
            let lhs = &assignments[0].lhs;
            let length = lhs.chars().count();
            if length > max {
                return;
            }
            let mut pattern = None;
            #[cfg(test)]
            LENGTH_SOURCE_BYTES
                .with(|count| count.set(count.get() + node.location().as_slice().len()));
            // Scan only this eligible conditional, not its enclosing file.
            // Nested scans are bounded by the pipeline's fixed AST-depth cap.
            if text(node.location().as_slice()).lines().any(|line| {
                let line = line.strip_suffix('\r').unwrap_or(line);
                if length + line.chars().count() <= max {
                    return false;
                }
                // Most conditionals need no regex at all. Build only when a
                // potentially overlong line needs its first LHS stripped.
                let pattern = pattern.get_or_insert_with(|| {
                    let whitespace = r"[ \t\r\n\x0B\x0C]*"; // Ruby's ASCII whitespace.
                    let pattern = format!(
                        "{whitespace}{}",
                        regex::escape(lhs).replace(' ', whitespace)
                    );
                    regex::RegexBuilder::new(&pattern)
                        .size_limit(usize::MAX)
                        .build()
                        .expect("escaped literal and whitespace repetitions")
                });
                length + pattern.replace(line, "").chars().count() > max
            }) {
                return;
            }
        }
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use the return of the conditional for variable assignment and comparison."
                .into(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction: cx
                .corrections_requested()
                .then(|| correction(node, &branches, &assignments, cx, file, ternary))
                .flatten(),
        });
    }
}

#[cfg(test)]
thread_local! {
    static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Keep the upstream diagnostic even when extracting its LHS would evaluate
/// a receiver, index, getter or compound/short-circuit read before the guard.
/// Plain stores have no such evaluation. `self` setters are also stable, but
/// only the assignment syntax (not an explicit setter call) is admitted.
fn stable_store(branch: &Node<'_>) -> bool {
    matches!(
        branch,
        Node::LocalVariableWriteNode { .. }
            | Node::InstanceVariableWriteNode { .. }
            | Node::ClassVariableWriteNode { .. }
            | Node::GlobalVariableWriteNode { .. }
            | Node::ConstantWriteNode { .. }
    ) || branch
        .as_constant_path_write_node()
        .is_some_and(|n| n.target().parent().is_none())
        || branch.as_call_node().is_some_and(|n| {
            n.receiver().is_some_and(|r| r.as_self_node().is_some())
                && n.name().as_slice().ends_with(b"=")
                && n.equal_loc().is_some()
                && n.opening_loc().is_none()
        })
}

fn correction(
    node: &Node<'_>,
    branches: &[Node<'_>],
    assignments: &[Assignment<'_>],
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    ternary: bool,
) -> Option<Correction> {
    use crate::cops::walk::{NodeVisitor, walk};
    // Bound local source reconstruction and semantic walks, even for nested
    // eligible conditionals. Detection remains uncapped by this fix budget.
    const BUDGET: usize = 32 * 1024;
    let loc = node.location();
    if loc.as_slice().len() > BUDGET || !branches.iter().all(stable_store) {
        return None;
    }
    // LHS generation is UTF-8. Never substitute lossy replacement characters
    // for an identifier (or discard other non-UTF-8 bytes in an edited prefix).
    std::str::from_utf8(loc.as_slice()).ok()?;
    #[cfg(test)]
    REWRITE_BYTES.with(|n| n.set(n.get() + loc.as_slice().len()));
    // Inserting an assignment in a receiver/operator/predicate could capture
    // following operators or change a void-valued control-flow expression.
    if !cx.parent().is_some_and(|p| {
        matches!(
            p,
            Node::StatementsNode { .. }
                | Node::ArgumentsNode { .. }
                | Node::ParenthesesNode { .. }
                | Node::ArrayNode { .. }
                | Node::LocalVariableWriteNode { .. }
                | Node::InstanceVariableWriteNode { .. }
                | Node::ClassVariableWriteNode { .. }
                | Node::GlobalVariableWriteNode { .. }
                | Node::ConstantWriteNode { .. }
        )
    }) || cx.ancestors().any(|n| n.as_defined_node().is_some())
    {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    if file.heredoc_pending_at(root, loc.start_offset())
        || super::correction::touches_directive(file, loc.start_offset()..loc.end_offset())
    {
        return None;
    }
    // Declaring a local at the start of the conditional can turn an earlier
    // bare method call into a local read, including within blocks/defaults.
    // Inspect the existing main AST, not a reparse of each candidate.
    struct Binding<'a> {
        name: Option<&'a [u8]>,
        changed: bool,
    }
    impl<'pr> NodeVisitor<'pr> for Binding<'_> {
        fn enter(&mut self, n: Node<'pr>) {
            self.changed |= n
                .as_call_node()
                .is_some_and(|c| c.is_variable_call() && self.name == Some(c.name().as_slice()));
        }
    }
    let local = branches[0].as_local_variable_write_node();
    let name = local.as_ref().map(|n| n.name());
    let mut binding = Binding {
        name: name.as_ref().map(|n| n.as_slice()),
        changed: false,
    };
    if let Some(name) = binding.name {
        // A predicate block can own this name without any bare call. Moving
        // the outer declaration ahead of it makes those writes capture the
        // outer local instead. Reuse the lazy shared scope index, including
        // pattern/regexp/parallel declarations but not shadowing parameters.
        if file.visible_locals(root).block_declaration_in(
            name,
            loc.start_offset()..branches[0].location().start_offset(),
        ) {
            return None;
        }
        walk(&mut binding, node);
        if binding.changed {
            return None;
        }
    }
    let mut edits = vec![Correction {
        range: loc.start_offset()..loc.start_offset(),
        replacement: assignments[0].lhs.clone(),
    }];
    for (branch, assignment) in branches.iter().zip(assignments) {
        let b = branch.location();
        let v = assignment.value.location();
        let prefix = b.start_offset()..v.start_offset();
        let suffix = v.end_offset()..b.end_offset();
        // Do not discard comments or physical lines embedded in a split LHS.
        if [prefix.clone(), suffix.clone()].iter().any(|r| {
            file.bytes[r.clone()].contains(&b'\n')
                || super::correction::contains_comment(file, r.clone())
        }) {
            return None;
        }
        let array = assignment
            .value
            .as_array_node()
            .is_some_and(|n| n.opening_loc().is_none());
        if ternary && array {
            return None;
        }
        edits.push(Correction {
            range: prefix,
            replacement: if array { "[".into() } else { String::new() },
        });
        edits.push(Correction {
            range: suffix,
            replacement: if array { "]".into() } else { String::new() },
        });
    }
    let edit = super::correction::combine(file, edits)?;
    (super::correction::preserves_line_values(file, &edit)
        && super::correction::preserves_data_positions(file, cx, &edit))
    .then_some(edit)
}

/// A fixed-policy branch must contain exactly one expression, not merely
/// have its assignment on one physical line. Comments are not statements.
fn single(body: Option<ruby_prism::StatementsNode<'_>>) -> Option<Node<'_>> {
    let body = body?;
    (body.body().len() == 1).then(|| body.body().iter().next().unwrap())
}

fn branches<'pr>(node: &Node<'pr>) -> Option<Vec<Node<'pr>>> {
    if let Some(n) = node.as_if_node() {
        if n.if_keyword_loc().is_some_and(|k| k.as_slice() == b"elsif") {
            return None;
        }
        let mut result = vec![single(n.statements())?];
        let mut subsequent = n.subsequent()?;
        loop {
            if let Some(elsif) = subsequent.as_if_node() {
                result.push(single(elsif.statements())?);
                subsequent = elsif.subsequent()?;
            } else {
                result.push(single(subsequent.as_else_node()?.statements())?);
                return Some(result);
            }
        }
    }
    if let Some(n) = node.as_unless_node() {
        return Some(vec![
            single(n.statements())?,
            single(n.else_clause()?.statements())?,
        ]);
    }
    if let Some(n) = node.as_case_node() {
        let mut result = n
            .conditions()
            .iter()
            .map(|branch| single(branch.as_when_node()?.statements()))
            .collect::<Option<Vec<_>>>()?;
        result.push(single(n.else_clause()?.statements())?);
        return Some(result);
    }
    if let Some(n) = node.as_case_match_node() {
        let mut result = n
            .conditions()
            .iter()
            .map(|branch| single(branch.as_in_node()?.statements()))
            .collect::<Option<Vec<_>>>()?;
        result.push(single(n.else_clause()?.statements())?);
        return Some(result);
    }
    None
}

/// RuboCop distinguishes plain variable assignment kinds, but folds every
/// compound target into op_asgn/and_asgn/or_asgn. LHS matching is deliberately
/// source-based (not structural equality), as in the pinned cop.
struct Assignment<'pr> {
    kind: &'static str,
    lhs: String,
    value: Node<'pr>,
}

fn standalone_value(value: &Node<'_>, brackets_array: bool, allows_match: bool) -> bool {
    if let Some(hash) = value.as_keyword_hash_node() {
        // A single hash rocket can reparse as a rightward match rather than a
        // hash. It is void-valued, so the first if/unless branch and ternaries
        // cannot use it. Labels, multiple pairs and keyword splats never work.
        return allows_match
            && hash.elements().len() == 1
            && hash.elements().iter().next().is_some_and(|element| {
                element.as_assoc_node().is_some_and(|pair| {
                    pair.operator_loc()
                        .is_some_and(|loc| loc.as_slice() == b"=>")
                        && expression_pattern(&pair.value(), &mut HashSet::new())
                })
            });
    }
    !matches!(
        value,
        Node::SplatNode { .. }
            | Node::BlockArgumentNode { .. }
            | Node::ForwardingArgumentsNode { .. }
    ) && (brackets_array
        || !value
            .as_array_node()
            .is_some_and(|a| a.opening_loc().is_none()))
}

/// Whether expression source can also spell a pattern after a hash rocket.
/// Inspect the existing AST instead of reparsing every eligible conditional.
fn expression_pattern(node: &Node<'_>, bindings: &mut HashSet<Vec<u8>>) -> bool {
    if let Some(local) = node.as_local_variable_read_node() {
        return pattern_binding(local.name().as_slice(), bindings);
    }
    if let Some(call) = node.as_call_node() {
        if call.is_variable_call() {
            return pattern_binding(call.name().as_slice(), bindings);
        }
        let receiver = call.receiver();
        if call.name().as_slice() == b"|" && call.call_operator_loc().is_none() {
            let arguments = ast::call_arguments(&call);
            let count = bindings.len();
            return arguments.len() == 1
                && receiver.is_some_and(|r| expression_pattern(&r, bindings))
                && expression_pattern(&arguments[0], bindings)
                && bindings.len() == count; // Only underscore captures in alternatives.
        }
        let constant_call = std::str::from_utf8(call.name().as_slice())
            .ok()
            .and_then(|s| s.chars().next())
            .is_some_and(char::is_uppercase)
            && call
                .call_operator_loc()
                .is_none_or(|loc| loc.as_slice() == b"::")
            && call.opening_loc().is_some_and(|loc| loc.as_slice() == b"(")
            && receiver.as_ref().is_none_or(pattern_constant);
        let constant_index = call.name().as_slice() == b"[]"
            && call.call_operator_loc().is_none()
            && call.opening_loc().is_some_and(|loc| loc.as_slice() == b"[")
            && receiver.as_ref().is_some_and(pattern_constant);
        return (constant_call || constant_index)
            && call.block().is_none()
            && call
                .arguments()
                .is_none_or(|a| pattern_elements(a.arguments(), bindings, true));
    }
    if let Some(array) = node.as_array_node() {
        return pattern_elements(array.elements(), bindings, false);
    }
    if let Some(hash) = node.as_hash_node() {
        return pattern_hash(hash.elements(), bindings);
    }
    if let Some(paren) = node.as_parentheses_node() {
        return single(paren.body().and_then(|n| n.as_statements_node()))
            .is_some_and(|n| expression_pattern(&n, bindings));
    }
    if let Some(range) = node.as_range_node() {
        return range
            .left()
            .iter()
            .chain(range.right().iter())
            .all(pattern_primitive);
    }
    pattern_constant(node) || pattern_primitive(node)
}

fn pattern_elements(
    elements: NodeList<'_>,
    bindings: &mut HashSet<Vec<u8>>,
    deconstruct: bool,
) -> bool {
    let elements = elements.iter().collect::<Vec<_>>();
    let splats = elements
        .iter()
        .enumerate()
        .filter_map(|(i, element)| element.as_splat_node().map(|_| i))
        .collect::<Vec<_>>();
    // Ordinary array patterns have one rest; find patterns have two,
    // strictly at the ends and with at least one required element.
    if splats.len() > 1
        && !(splats.len() == 2 && elements.len() > 2 && splats == [0, elements.len() - 1])
    {
        return false;
    }
    elements.iter().all(|element| {
        if let Some(hash) = element.as_keyword_hash_node() {
            // A sole label hash spells a hash deconstructor, but not an
            // array pattern. Hash rockets instead spell capture patterns.
            if deconstruct && elements.len() == 1 && pattern_hash(hash.elements(), bindings) {
                return true;
            }
            return hash.elements().iter().all(|element| {
                element.as_assoc_node().is_some_and(|pair| {
                    pair.operator_loc()
                        .is_some_and(|loc| loc.as_slice() == b"=>")
                        && expression_pattern(&pair.key(), bindings)
                        && pattern_rest(&pair.value(), bindings)
                })
            });
        }
        if let Some(splat) = element.as_splat_node() {
            splat
                .expression()
                .is_none_or(|value| pattern_rest(&value, bindings))
        } else {
            expression_pattern(element, bindings)
        }
    })
}

fn pattern_hash(elements: NodeList<'_>, bindings: &mut HashSet<Vec<u8>>) -> bool {
    let mut keys = HashSet::new();
    elements.iter().enumerate().all(|(index, element)| {
        if let Some(splat) = element.as_assoc_splat_node() {
            return index + 1 == elements.len()
                && splat.value().is_none_or(|value| {
                    value.as_nil_node().is_some() || pattern_rest(&value, bindings)
                });
        }
        let Some(pair) = element.as_assoc_node() else {
            return false;
        };
        let key = pair.key();
        let Some(symbol) = key.as_symbol_node() else {
            return false;
        };
        pair.operator_loc().is_none()
            && symbol
                .closing_loc()
                .is_some_and(|loc| loc.as_slice().ends_with(b":"))
            && keys.insert(symbol.unescaped().to_vec())
            && if pair.value().as_implicit_node().is_some() {
                pattern_binding(symbol.unescaped(), bindings)
            } else {
                expression_pattern(&pair.value(), bindings)
            }
    })
}

fn pattern_constant(node: &Node<'_>) -> bool {
    node.as_constant_read_node().is_some()
        || node
            .as_constant_path_node()
            .is_some_and(|path| path.parent().as_ref().is_none_or(pattern_constant))
}

fn pattern_binding(name: &[u8], bindings: &mut HashSet<Vec<u8>>) -> bool {
    !name.ends_with(b"?")
        && !name.ends_with(b"!")
        && !std::str::from_utf8(name)
            .ok()
            .and_then(|s| s.chars().next())
            .is_some_and(char::is_uppercase)
        && (name.starts_with(b"_") || bindings.insert(name.to_vec()))
}

fn pattern_rest(node: &Node<'_>, bindings: &mut HashSet<Vec<u8>>) -> bool {
    node.as_local_variable_read_node()
        .is_some_and(|n| pattern_binding(n.name().as_slice(), bindings))
        || node
            .as_call_node()
            .is_some_and(|n| n.is_variable_call() && pattern_binding(n.name().as_slice(), bindings))
}

fn pattern_primitive(node: &Node<'_>) -> bool {
    (ast::is_literal(node)
        && !matches!(
            node,
            Node::ArrayNode { .. }
                | Node::HashNode { .. }
                | Node::KeywordHashNode { .. }
                | Node::RangeNode { .. }
        ))
        || matches!(
            node,
            Node::SelfNode { .. } | Node::SourceEncodingNode { .. } | Node::LambdaNode { .. }
        )
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn assignment<'pr>(node: &Node<'pr>) -> Option<Assignment<'pr>> {
    macro_rules! variable {
        ($($access:ident => $kind:literal),* $(,)?) => {$(
            if let Some(n) = node.$access() {
                return Some(Assignment { kind: $kind, lhs: format!("{} = ", text(n.name().as_slice())), value: n.value() });
            }
        )*};
    }
    variable! {
        as_local_variable_write_node => "local",
        as_instance_variable_write_node => "instance",
        as_class_variable_write_node => "class",
        as_global_variable_write_node => "global",
        as_constant_write_node => "constant",
    }
    if let Some(n) = node.as_constant_path_write_node() {
        let target = n.target();
        let name = text(target.name()?.as_slice());
        let namespace = target.parent().map_or_else(
            || "::".into(),
            |p| format!("{}::", ast::const_name(&p).unwrap_or_default()),
        );
        return Some(Assignment {
            kind: "constant",
            lhs: format!("{namespace}{name} = "),
            value: n.value(),
        });
    }
    macro_rules! target_end {
        ($n:ident, name) => {
            $n.name_loc().end_offset()
        };
        ($n:ident, path) => {
            $n.target().location().end_offset()
        };
        ($n:ident, call) => {
            $n.message_loc()?.end_offset()
        };
        ($n:ident, index) => {
            $n.closing_loc().end_offset()
        };
    }
    macro_rules! compound {
        ($($access:ident => ($kind:literal, $operator:ident, $target:ident)),* $(,)?) => {$(
            if let Some(n) = node.$access() {
                let operator = n.$operator();
                let loc = node.location();
                let target = &loc.as_slice()[..target_end!(n, $target) - loc.start_offset()];
                return Some(Assignment { kind: $kind, lhs: format!("{} {} ", text(target), text(operator.as_slice())), value: n.value() });
            }
        )*};
    }
    compound! {
        as_local_variable_operator_write_node => ("op", binary_operator_loc, name),
        as_instance_variable_operator_write_node => ("op", binary_operator_loc, name),
        as_class_variable_operator_write_node => ("op", binary_operator_loc, name),
        as_global_variable_operator_write_node => ("op", binary_operator_loc, name),
        as_constant_operator_write_node => ("op", binary_operator_loc, name),
        as_constant_path_operator_write_node => ("op", binary_operator_loc, path),
        as_call_operator_write_node => ("op", binary_operator_loc, call),
        as_index_operator_write_node => ("op", binary_operator_loc, index),
        as_local_variable_and_write_node => ("and", operator_loc, name),
        as_instance_variable_and_write_node => ("and", operator_loc, name),
        as_class_variable_and_write_node => ("and", operator_loc, name),
        as_global_variable_and_write_node => ("and", operator_loc, name),
        as_constant_and_write_node => ("and", operator_loc, name),
        as_constant_path_and_write_node => ("and", operator_loc, path),
        as_call_and_write_node => ("and", operator_loc, call),
        as_index_and_write_node => ("and", operator_loc, index),
        as_local_variable_or_write_node => ("or", operator_loc, name),
        as_instance_variable_or_write_node => ("or", operator_loc, name),
        as_class_variable_or_write_node => ("or", operator_loc, name),
        as_global_variable_or_write_node => ("or", operator_loc, name),
        as_constant_or_write_node => ("or", operator_loc, name),
        as_constant_path_or_write_node => ("or", operator_loc, path),
        as_call_or_write_node => ("or", operator_loc, call),
        as_index_or_write_node => ("or", operator_loc, index),
    }
    let call = node.as_call_node()?;
    let name = call.name().as_slice();
    if call.is_safe_navigation()
        || call.block().is_some_and(|b| b.as_block_node().is_some())
        || !(name.ends_with(b"=") || matches!(name, b"<<" | b"=~" | b"!~" | b"<=>" | b"<" | b">"))
    {
        return None;
    }
    let mut arguments = ast::call_arguments(&call);
    if arguments.is_empty() {
        return None;
    }
    let receiver = call
        .receiver()
        .map_or_else(String::new, |r| text(r.location().as_slice()));
    let lhs = if name == b"[]=" {
        let indices = arguments[..arguments.len() - 1]
            .iter()
            .map(|n| text(n.location().as_slice()))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{receiver}[{indices}] = ")
    } else if name.ends_with(b"=") && !matches!(name, b"==" | b"!=" | b"===" | b"<=" | b">=") {
        format!("{receiver}.{} = ", text(&name[..name.len() - 1]))
    } else {
        format!("{receiver} {} ", text(name))
    };
    Some(Assignment {
        kind: "send",
        lhs,
        value: arguments.pop()?,
    })
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's two `conditional_assignment_*_spec.rb` files
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    #[test]
    fn correction_work_is_lazy_bounded_and_linear_over_siblings() {
        use crate::{
            cops::{testing::correction_offenses, variables::VISIBLE_LOCAL_BUILDS},
            pipeline::lint_str_with,
        };
        let cop = super::ConditionalAssignment::default();
        let mut previous = 0;
        for count in [1500, 3000] {
            let source = "if ready?\n  value = 1\nelse\n  value = 2\nend\n".repeat(count);
            super::REWRITE_BYTES.with(|n| n.set(0));
            VISIBLE_LOCAL_BUILDS.with(|n| n.set(0));
            assert_eq!(lint_str_with(&[&cop], &source).len(), count);
            super::REWRITE_BYTES.with(|n| assert_eq!(n.get(), 0));
            VISIBLE_LOCAL_BUILDS.with(|n| assert_eq!(n.get(), 0));
            let found = correction_offenses(&cop, &source);
            VISIBLE_LOCAL_BUILDS.with(|n| assert_eq!(n.get(), 1));
            assert_eq!(found.len(), count);
            assert!(found.iter().all(|o| o.correction.is_some()));
            let work = super::REWRITE_BYTES.with(|n| n.get());
            assert!(work <= source.len());
            if previous != 0 {
                assert_eq!(work, 2 * previous);
            }
            previous = work;
        }
        let source = format!(
            "if ready?\n{}  value = 1\nelse\n  value = 2\nend\n",
            "  # bounded comment\n".repeat(3000)
        );
        withheld(&source);
    }
    #[test]
    fn begin_rescue_ensure_values_and_surrounding_return_survive_extraction() {
        for prefix in ["", "result = ", "consume("] {
            let suffix = if prefix == "consume(" { ")" } else { "" };
            let body = "if ready?\n  value = begin\n    work\n  rescue\n    fallback\n  ensure\n    cleanup\n  end\nelse\n  value = 2\nend";
            correct(
                &format!("{prefix}{body}{suffix}\n"),
                &format!(
                    "{prefix}value = {}{suffix}\n",
                    body.replace("value = begin", "begin")
                        .replace("value = 2", "2")
                ),
            );
        }
        correct(
            "cond ? value = (work rescue fallback) : value = 2",
            "value = cond ? (work rescue fallback) : 2",
        );
    }
    #[test]
    fn split_lhs_comments_and_defined_are_withheld_but_interpolation_stays_valid() {
        withheld("if c\n bar = # retain\n 1\nelse\n bar = 2\nend");
        correct(
            "\"#{if c; bar = 1; else; bar = 2; end}\"",
            "\"#{bar = if c; 1; else; 2; end}\"",
        );
        withheld("defined?(cond ? bar = 1 : bar = 2)");
    }
    fn correct(before: &str, after: &str) {
        crate::cops::testing::assert_correction(
            &super::ConditionalAssignment::default(),
            before,
            after,
        );
    }
    fn withheld(source: &str) {
        let found = crate::cops::testing::correction_offenses(
            &super::ConditionalAssignment::default(),
            source,
        );
        assert!(!found.is_empty(), "{source}");
        assert!(
            found.iter().all(|o| o.correction.is_none()),
            "{source}: {found:?}"
        );
    }
    #[test]
    fn corrects_plain_assignment_families_and_ternaries() {
        for target in [
            "bar",
            "@bar",
            "@@bar",
            "$bar",
            "BAR",
            "::BAR",
            "self.attribute",
        ] {
            correct(
                &format!("if cond\n  {target} = 1\nelse\n  {target} = 2\nend\n"),
                &format!("{target} = if cond\n  1\nelse\n  2\nend\n"),
            );
        }
        correct(
            "foo? ? bar = \"a\" : bar = \"b\"\n",
            "bar = foo? ? \"a\" : \"b\"\n",
        );
        correct(
            "unless c; bar=1; else; bar =2; end",
            "bar = unless c; 1; else; 2; end",
        );
        correct(
            "if c; bar=1; elsif d; bar=2; else; bar=3; end",
            "bar = if c; 1; elsif d; 2; else; 3; end",
        );
        for header in ["case c; when 1", "case c; in Integer"] {
            correct(
                &format!("{header}; bar=1; else; bar=2; end"),
                &format!("bar = {header}; 1; else; 2; end"),
            );
        }
    }
    #[test]
    fn preserves_comments_literals_and_line_values_in_place() {
        let before = "if cond # header\n  # first\n  bar = {a: __LINE__, b: \"one\\ntwo\"} # value\nelse # other\n  bar = 2, 3 # array\nend # end\np __LINE__\n__END__\nopaque\n";
        correct(
            before,
            "bar = if cond # header\n  # first\n  {a: __LINE__, b: \"one\\ntwo\"} # value\nelse # other\n  [2, 3] # array\nend # end\np __LINE__\n__END__\nopaque\n",
        );
    }
    #[test]
    fn withholds_evaluation_order_short_circuit_and_binding_changes() {
        for statement in [
            "bar += 1",
            "bar ||= 1",
            "bar &&= 1",
            "bar << 1",
            "bar == 1",
            "object.value = 1",
            "array[1] = 1",
            "Foo::BAR = 1",
        ] {
            withheld(&format!("if cond; {statement}; else; {statement}; end"));
        }
        withheld("if foo; foo = 1; else; foo = 2; end");
        withheld("if ready?(-> { foo }); foo = 1; else; foo = 2; end");
        withheld("defined?(if c; bar=1; else; bar=2; end)");
    }
    #[test]
    fn withholds_ambiguous_envelopes_and_preserves_nested_association() {
        withheld("if c; bar=1; else; bar=2; end.to_s");
        withheld("if c; bar \\\n = 1; else; bar = 2; end");
        correct(
            "if outer\n  bar = 1\nelse\n  if inner\n    bar = 2\n  else\n    bar = 3\n  end\nend\n",
            "if outer\n  bar = 1\nelse\n  bar = if inner\n    2\n  else\n    3\n  end\nend\n",
        );
    }
    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/ConditionalAssignment")
            .collect()
    }
    fn assert_offense(source: &str, line: u32, col: u32, end_line: u32) {
        let found = offenses(source);
        assert_eq!(found.len(), 1, "{source}: {found:?}");
        assert_eq!(
            (found[0].line, found[0].col, found[0].end_line),
            (line, col, end_line),
            "{source}"
        );
        assert_eq!(
            found[0].message,
            "Use the return of the conditional for variable assignment and comparison."
        );
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn if_unless_and_ternary() {
        assert_offense(
            "# example\n  if foo\n    bar = 1\n  else\n    bar = 2\n  end\n",
            2,
            3,
            6,
        );
        assert_offense("unless foo\n  bar = 1\nelse\n  bar = 2\nend\n", 1, 1, 5);
        assert_offense("foo? ? bar = 'a' : bar = 'b'", 1, 1, 1);
    }
    #[test]
    fn elsif_case_and_pattern_branches() {
        for source in [
            "if foo; bar = 1; elsif baz; bar = 2; elsif qux; bar = 3; else; bar = 4; end",
            "case foo; when 1, 2; bar = 1; when 3; bar = 2; else; bar = 3; end",
            "case; when foo; bar = 1; else; bar = 2; end",
            "case foo; in [x] if x > 0; bar = 1; in Integer; bar = 2; else; bar = 3; end",
        ] {
            assert_offense(source, 1, 1, 1);
        }
        for source in [
            "if foo; bar = 1; elsif baz; bar = 2; end",
            "if foo; bar = 1; elsif baz; else; bar = 2; end",
            "if foo; bar = 1; elsif baz; baz = 2; else; bar = 3; end",
            "case foo; when 1; bar = 1; end",
            "case foo; in Integer; bar = 1; end",
            "case foo; when 1; else; bar = 1; end",
            "case foo; in Integer; bar = 1; else; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn variable_targets_and_operators() {
        for target in [
            "bar",
            "CONST",
            "@ivar",
            "@@cvar",
            "$gvar",
            "::CONST",
            "Foo::CONST",
            "::Foo::Bar::CONST",
            "obj.value",
            "self.value",
            "array[1]",
            "array[1, 2]",
        ] {
            for operator in [
                "=", "+=", "-=", "*=", "**=", "/=", "%=", "^=", "&=", "|=", "<<=", ">>=", "||=",
                "&&=", "<<",
            ] {
                assert_offense(
                    &format!("if foo\n  {target} {operator} 1\nelse\n  {target} {operator} 2\nend"),
                    1,
                    1,
                    5,
                );
                assert_offense(
                    &format!("foo? ? {target} {operator} 1 : {target} {operator} 2"),
                    1,
                    1,
                    1,
                );
            }
        }
        for operator in ["==", "!=", "=~", "!~", "<=>", "===", "<=", ">=", "<", ">"] {
            assert_offense(
                &format!("foo? ? bar {operator} 1 : bar {operator} 2"),
                1,
                1,
                1,
            );
        }
    }
    #[test]
    fn distinct_targets_types_and_operators_are_not_candidates() {
        for (a, b) in [
            ("bar = 1", "@bar = 2"),
            ("bar = 1", "bar += 2"),
            ("bar += 1", "bar -= 2"),
            ("bar ||= 1", "bar &&= 2"),
            ("foo.bar = 1", "baz.bar = 2"),
            ("foo.bar = 1", "foo.baz = 2"),
            ("foo[1] = 1", "foo[2] = 2"),
            ("foo[1, 2] = 1", "foo[1, 3] = 2"),
            ("Foo::BAR = 1", "Other::BAR = 2"),
            ("::BAR = 1", "BAR = 2"),
            ("bar << 1", "bar == 2"),
            ("bar[1] = 1", "bar << 2"),
            ("a, b = 1, 2", "a, b = 2, 1"),
            ("foo&.bar = 1", "foo&.bar = 2"),
        ] {
            let source = format!("if foo; {a}; else; {b}; end");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
    #[test]
    fn line_length_counts_prospective_lhs_and_removes_branch_assignments() {
        let condition = format!("if {}\n  bar = 1\nelse\n  bar = 2\nend", "a".repeat(111));
        assert_offense(&condition, 1, 1, 5); // Exactly 120 after moving the assignment.
        let too_long = condition.replacen(&"a".repeat(111), &"a".repeat(112), 1);
        assert!(offenses(&too_long).is_empty());
        let branch = format!(
            "if foo\n  bar  =  {}\nelse\n  bar = 2\nend",
            "a".repeat(114)
        );
        assert_offense(&branch, 1, 1, 5); // Strip the first assignment, including whitespace.
        let unicode = format!("if {}\n  bar = 1\nelse\n  bar = 2\nend", "é".repeat(111));
        assert_offense(&unicode, 1, 1, 5);
        let regex_chars = "if condition\n  default['key-with-dash'] << a\nelse\n  default['key-with-dash'] << b\nend";
        assert_offense(regex_chars, 1, 1, 5);
    }
    #[test]
    fn single_statement_is_not_single_physical_line() {
        for source in [
            "if foo; work; bar = 1; else; bar = 2; end",
            "if foo; bar = 1; else; work; bar = 2; end",
            "unless foo; bar = 1; else; work; bar = 2; end",
            "case foo; when 1; work; bar = 1; else; bar = 2; end",
            "case foo; in Integer; bar = 1; else; work; bar = 2; end",
            "if foo; (bar = 1); else; (bar = 2); end",
            "if foo; begin; bar = 1; end; else; begin; bar = 2; end; end",
            "if foo; bar = 1; return bar; else; bar = 2; return bar; end",
            "if foo; return bar = 1; else; return bar = 2; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_offense(
            "if foo\n  # keep comment\n  bar = {\n    a: 1,\n    b: 2\n  }\nelse\n  # else comment\n  bar = {}\nend\n",
            1,
            1,
            10,
        );
        assert_offense("if foo\n  bar = 1\nelse\n  bar = 2, 5, 6\nend", 1, 1, 5);
    }
    #[test]
    fn nested_else_association_and_guards() {
        assert_offense(
            "if outer\n  bar = 1\nelse\n  if inner\n    bar = 2\n  elsif other\n    bar = 3\n  else\n    bar = 4\n  end\nend",
            4,
            3,
            10,
        );
        assert_offense(
            "unless outer\n  if inner\n    bar = 1\n  else\n    bar = 2\n  end\nelse\n  bar = 3\nend",
            2,
            3,
            6,
        );
        for source in [
            "if foo; bar = 1; else; bar = 2 if baz; end",
            "if foo; bar = 1; else; bar = 2 unless baz; end",
            "if foo; bar = 1; elsif baz; bar = 2; else; if qux; bar = 3; end; end",
            "if foo; bar = 1; elsif baz; bar = 2; else; unless qux; bar = 3; end; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn assignment_parents_receivers_and_targets_do_not_hide_conditionals() {
        for (source, col) in [
            ("result = if foo; bar = 1; else; bar = 2; end", 10),
            ("consume(if foo; bar = 1; else; bar = 2; end)", 9),
            ("(if foo; bar = 1; else; bar = 2; end).value = 3", 2),
            ("object[if foo; bar = 1; else; bar = 2; end] ||= 3", 8),
            ("object.value += if foo; bar = 1; else; bar = 2; end", 17),
            ("object[0] ||= if foo; bar = 1; else; bar = 2; end", 15),
            ("(if foo; bar = 1; else; bar = 2; end)::CONST = 3", 2),
            (
                "obj.value, object[if foo; bar = 1; else; bar = 2; end] = values",
                19,
            ),
        ] {
            assert_offense(source, 1, col, 1);
        }
    }
    #[test]
    fn source_sensitive_lhs_matching() {
        assert_offense("if foo; bar=1; else; bar  =  2; end", 1, 1, 1);
        assert_offense(
            "if foo; array[1,2] = 1; else; array[1, 2] = 2; end",
            1,
            1,
            1,
        );
        // A setter's receiver and a compound assignment's whole target retain
        // source spelling; equivalent ASTs are not enough upstream.
        for (a, b) in [
            ("(foo).bar = 1", "foo.bar = 2"),
            ("array[1,2] += 1", "array[1, 2] += 2"),
            ("default['key'] << 1", "default[\"key\"] << 2"),
            ("array[1 + 2] = 1", "array[1+2] = 2"),
        ] {
            let source = format!("if foo; {a}; else; {b}; end");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
    #[test]
    fn begin_rescue_ensure_and_block_boundaries() {
        for (source, line, col, end_line) in [
            (
                "begin\n  if foo\n    bar = 1\n  else\n    bar = 2\n  end\nrescue\n  recover\nensure\n  cleanup\nend",
                2,
                3,
                6,
            ),
            (
                "def f\n  work\nrescue\n  if foo\n    bar = 1\n  else\n    bar = 2\n  end\nensure\n  cleanup\nend",
                4,
                3,
                8,
            ),
            (
                "items.each do\n  if foo\n    bar = 1\n  else\n    bar = 2\n  end\nend",
                2,
                3,
                6,
            ),
            (
                "if foo; bar = 1 rescue fallback; else; bar = 2 rescue fallback; end",
                1,
                1,
                1,
            ),
            (
                "if foo\n  bar = (work rescue fallback)\nelse\n  bar = 2\nend",
                1,
                1,
                5,
            ),
            (
                "if foo\n  bar = begin\n    work\n  rescue\n    fallback\n  ensure\n    cleanup\n  end\nelse\n  bar = 2\nend",
                1,
                1,
                11,
            ),
        ] {
            assert_offense(source, line, col, end_line);
        }
        for source in [
            "if foo; begin; bar = 1; rescue; bar = 3; end; else; bar = 2; end",
            "if foo; bar.<<(1) do; end; else; bar.<<(2) do; end; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn opposite_fixed_style_and_modifier_local_scope_remain_untouched() {
        for source in [
            "bar = if foo; 1; else; 2; end",
            "bar += case foo; when 1; 2; else; 3; end",
            "bar << (foo? ? 1 : 2)",
            "bar = unless foo; 1; end",
            "bar = case foo; in Integer; 1; else; 2; end",
            "bar = 1 if bar.nil?",
            "bar = 1 unless bar",
            "bar = (foo? ? 1 : 2)",
            "foo.[]=()",
            "begin; work; rescue => bar; bar; end",
            "for bar in values; bar; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_offense(
            "if (bar = condition)\n  bar = __LINE__\nelse\n  bar = __LINE__\nend",
            1,
            1,
            5,
        );
    }
    #[test]
    fn extracted_values_must_be_standalone_expressions() {
        // Upstream suppresses findings when its hypothetical rewrite cannot
        // parse. Match the AST shapes without reparsing each candidate.
        for branch in [
            "foo.==()",
            "foo.==(&blk)",
            "foo.==(*args)",
            "foo.==(key: 1)",
            "foo.==(**opts)",
            "foo.[]=(1, &blk)",
            "foo.[]=(*args)",
            "foo.[]=(key: 1)",
            "foo.[]=(1, **opts)",
            "foo.[]=(1, *args)",
        ] {
            for source in [
                format!("if cond\n {branch}\nelse\n {branch}\nend"),
                format!("case cond\nwhen 1\n {branch}\nelse\n {branch}\nend"),
            ] {
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        for target in ["bar", "foo.attr", "foo[0]"] {
            assert_offense(
                &format!("if cond\n {target} = *args\nelse\n {target} = *other\nend"),
                1,
                1,
                5,
            );
            for source in [
                format!("case cond\nwhen 1\n {target} = *args\nelse\n {target} = *other\nend"),
                format!(
                    "if cond\n {target} = 1\nelsif other\n {target} = 2, 3\nelse\n {target} = 4\nend"
                ),
            ] {
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        assert_offense(
            "case cond\nwhen 1\n bar = [*args]\nelse\n bar = *other\nend",
            1,
            1,
            6,
        );
        assert_offense(
            "if cond\n foo.==({key: 1})\nelse\n foo.==({key: 2})\nend",
            1,
            1,
            5,
        );
    }
    #[test]
    fn ternary_rescue_values_depend_on_branch_position() {
        for assignment in ["bar =", "bar +=", "foo.bar =", "foo[0] ="] {
            for other in ["1", "1 rescue 2"] {
                let source = format!("cond ? {assignment} 1 rescue 2 : {assignment} {other}");
                assert!(offenses(&source).is_empty(), "{source}");
            }
            for (first, last) in [
                ("1", "1 rescue 2"),
                ("(1 rescue 2)", "1"),
                ("(1 rescue 2)", "1 rescue 2"),
                ("(1 rescue 2)", "(1 rescue 2)"),
            ] {
                assert_offense(
                    &format!("cond ? {assignment} {first} : {assignment} {last}"),
                    1,
                    1,
                    1,
                );
            }
            assert_offense(
                &format!("if cond\n {assignment} 1 rescue 2\nelse\n {assignment} 1 rescue 2\nend"),
                1,
                1,
                5,
            );
        }
    }
    #[test]
    fn extracted_hash_rockets_can_be_rightward_patterns() {
        for value in [":a => 1", "\"a\" => Integer", ":a => x", ":a => -1"] {
            for branch in [format!("foo.==({value})"), format!("foo.[]=(0, {value})")] {
                for condition in ["case cond\nwhen 1", "case cond\nin Integer"] {
                    assert_offense(
                        &format!("{condition}\n {branch}\nelse\n {branch}\nend"),
                        1,
                        1,
                        6,
                    );
                }
                // A rightward assignment is a void expression: it cannot be
                // the first value of an if/unless or either ternary branch.
                for source in [
                    format!("if cond\n {branch}\nelse\n {branch}\nend"),
                    format!("unless cond\n {branch}\nelse\n {branch}\nend"),
                    format!("cond ? {branch} : {branch}"),
                ] {
                    assert!(offenses(&source).is_empty(), "{source}");
                }
            }
            assert_offense(
                &format!(
                    "if cond\n foo.==(1)\nelsif other\n foo.==({value})\nelse\n foo.==({value})\nend"
                ),
                1,
                1,
                7,
            );
        }
        for value in [
            "key: 1",
            "**opts",
            ":a => 1, :b => 2",
            ":a => 1, **opts",
            ":a => obj.foo",
            ":a => foo()",
            ":a => foo[0]",
            ":a => 1 + 2",
        ] {
            let source =
                format!("case cond\nwhen 1\n foo.==({value})\nelse\n foo.==({value})\nend");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
    #[test]
    fn extracted_hash_rocket_patterns_require_pattern_syntax() {
        // A hash's ordinary expression values need not be valid patterns.
        // These expectations were checked with the pinned Ruby 3.4/Prism cop.
        for value in [
            "self",
            "__ENCODING__",
            "Foo::Bar",
            "[x]",
            "[_x, _x]",
            "[x, *rest]",
            "[*head, x, *tail]",
            "{a: x}",
            "{a:}",
            "{\"a\": x}",
            "{a: x, **rest}",
            "1..2",
            "..2",
            "1..",
            "(1)",
            "%w[a b]",
            "Foo(x)",
            "Foo[x]",
            "Foo(a: x)",
            "Foo[a: x]",
            "Foo[1, *rest]",
            "1 | 2",
            "_x | _y",
            "[(1)]",
            "Foo::Bar(1)",
            "{a: x, **nil}",
            "[:a => x]",
            "[1, :a => x]",
            "Foo(:a => x, :b => y)",
        ] {
            assert_offense(
                &format!(
                    "case cond\nwhen 1\n foo.==(:a => {value})\nelse\n foo.==(:a => {value})\nend"
                ),
                1,
                1,
                6,
            );
        }
        for value in [
            "x()",
            "x?",
            "x!",
            "@x",
            "(foo)::Bar",
            "[x, x]",
            "[*head, *tail]",
            "[x, *head, *tail]",
            "[*obj.foo]",
            "{a: x, b: x}",
            "{a: x, a: y}",
            "{:\"a\" => x}",
            "{**rest, a: x}",
            "{**obj.foo}",
            "Integer..2",
            "1..Integer",
            "x..2",
            "1..x",
            "1..(2)",
            "x | y",
            "(Foo [1])",
            "(Foo 1)",
            "Foo.[](1)",
            "(Foo::Bar 1)",
            "Foo(1, a: x)",
            "[a: x]",
            "[1, a: x]",
            "1.|(2)",
            "{A:}",
            "[:a => x, :b => x]",
        ] {
            let source = format!(
                "case cond\nwhen 1\n foo.==(:a => {value})\nelse\n foo.==(:a => {value})\nend"
            );
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
    #[test]
    fn source_work_is_bounded_by_depth_and_linear_across_siblings() {
        use crate::{cops::depth::MAX_AST_DEPTH, pipeline::lint_str_with};
        let mut conditional = "1".to_owned();
        for _ in 0..8 {
            conditional = format!("if condition\n  value = {conditional}\nelse\n  value = 2\nend");
        }
        let mut previous = 0;
        for copies in [100, 200] {
            let source = format!("{conditional}\n").repeat(copies);
            super::LENGTH_SOURCE_BYTES.with(|count| count.set(0));
            assert_eq!(
                lint_str_with(&[&super::ConditionalAssignment::default()], &source).len(),
                copies * 8
            );
            let work = super::LENGTH_SOURCE_BYTES.with(|count| count.get());
            assert!(work > source.len());
            assert!(work <= MAX_AST_DEPTH * source.len());
            if previous != 0 {
                assert_eq!(work, previous * 2);
            }
            previous = work;
        }
        super::LENGTH_SOURCE_BYTES.with(|count| count.set(0));
        assert_eq!(
            lint_str_with(
                &[&super::ConditionalAssignment {
                    max_line_length: None
                }],
                &conditional
            )
            .len(),
            8
        );
        assert_eq!(super::LENGTH_SOURCE_BYTES.with(|count| count.get()), 0);
    }
    #[test]
    fn compound_target_ends_before_line_continuations() {
        for target in [
            "bar",
            "@bar",
            "@@bar",
            "$bar",
            "CONST",
            "Foo::CONST",
            "object.value",
            "object[0]",
        ] {
            for operator in ["+=", "||=", "&&="] {
                assert_offense(
                    &format!(
                        "if condition\n  {target} \\\n    {operator} 1\nelse\n  {target} {operator} 2\nend"
                    ),
                    1,
                    1,
                    6,
                );
            }
        }
    }
    #[test]
    fn missing_else_and_nonassignments() {
        for source in [
            "if foo; bar = 1; end",
            "unless foo; bar = 1; end",
            "bar = 1 if foo",
            "if foo; bar = 1; else; baz = 2; end",
            "if foo; 1; else; 2; end",
            "bar = foo? ? 1 : 2",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
