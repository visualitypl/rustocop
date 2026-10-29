//! `Style/GuardClause` — fixed defaults and conservative, atomic guard rewrites.

use ruby_prism::{Location, Node, StatementsNode};

use crate::{
    cops::{Cop, NodeContext, variables},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct GuardClause {
    max_line_length: Option<usize>,
}

impl Default for GuardClause {
    fn default() -> Self {
        Self {
            max_line_length: Some(120),
        }
    }
}

impl Cop for GuardClause {
    fn name(&self) -> &'static str {
        "Style/GuardClause"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &["Style/MissingElse"]
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
        if let Some(mut conditional) = Conditional::of(node)
            && let Some(other) = conditional.other.as_ref().and_then(|n| n.as_else_node())
        {
            conditional.statement = cx
                .parent()
                .is_some_and(|parent| parent.as_statements_node().is_some());
            // Prism's singleton statements and argument lists are transparent
            // in RuboCop's AST. Parentheses and multi-statement bodies are not.
            let mut ancestors = cx.ancestors().peekable();
            let parent = loop {
                let Some(parent) = ancestors.next() else {
                    break None;
                };
                if parent.as_arguments_node().is_some()
                    || parent
                        .as_statements_node()
                        .is_some_and(|s| s.body().len() == 1)
                {
                    continue;
                }
                // A constant-path write folds exactly its target path into
                // casgn upstream. Nested namespace paths remain const reads.
                if parent.as_constant_path_node().is_some()
                    && ancestors.peek().is_some_and(|outer| {
                        constant_path_target(outer)
                            .is_some_and(|target| same_location(&target, parent))
                    })
                {
                    continue;
                }
                break Some(parent);
            };
            let accepted_parent = parent.is_some_and(|parent| {
                assignment(parent, node)
                    || parent
                        .as_def_node()
                        .is_some_and(|d| d.equal_loc().is_some())
            });
            if !accepted_parent && !conditional.accepted(file) {
                let first = conditional
                    .body
                    .as_ref()
                    .and_then(|s| guard_source(s, file));
                let guard = first.map(|s| (s, conditional.keyword())).or_else(|| {
                    other
                        .statements()
                        .as_ref()
                        .and_then(|s| guard_source(s, file))
                        .map(|s| (s, conditional.inverse()))
                });
                if let Some((exit, keyword)) = guard {
                    self.report(&conditional, &exit, keyword, cx, file, out);
                }
            }
        }
        if let Some(def) = node.as_def_node() {
            if def.equal_loc().is_none() {
                self.check_ending_body(def.body(), false, false, cx, file, out);
            }
        } else if let Some(call) = node.as_call_node()
            && matches!(
                call.name().as_slice(),
                b"define_method" | b"define_singleton_method"
            )
            && let Some(block) = call.block().and_then(|b| b.as_block_node())
        {
            self.check_ending_body(block.body(), true, false, cx, file, out);
        }
    }
}

impl GuardClause {
    fn check_ending_body(
        &self,
        body: Option<Node<'_>>,
        block_tail: bool,
        covered: bool,
        cx: &NodeContext<'_, '_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(tail) = body.and_then(ending_tail) else {
            return;
        };
        let Some(mut conditional) = Conditional::of(&tail) else {
            return;
        };
        conditional.block_tail = block_tail;
        conditional.covered = covered;
        conditional.statement = true;
        if conditional.other.is_none()
            && !conditional.accepted(file)
            && file.line_of(conditional.end.start_offset())
                > file.line_of(conditional.keyword.start_offset()) + 1
        {
            let reported = out.len();
            self.report(&conditional, "return", conditional.inverse(), cx, file, out);
            self.check_ending_body(
                conditional.body.map(|s| s.as_node()),
                block_tail,
                covered
                    || out
                        .get(reported)
                        .is_some_and(|offense| offense.correction.is_some()),
                cx,
                file,
                out,
            );
        }
    }

    fn report(
        &self,
        conditional: &Conditional<'_>,
        exit: &str,
        keyword: &str,
        cx: &NodeContext<'_, '_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some((example, _)) = self.example(conditional, exit, keyword, file) else {
            return;
        };
        let (line, col) = file.line_col(conditional.keyword.start_offset());
        out.push(Offense {
            cop: self.name(), severity: self.severity(),
            message: format!("Use a guard clause (`{example}`) instead of wrapping the code inside a conditional expression."),
            line, col, end_line: line,
            correction: (cx.corrections_requested() && !conditional.covered).then(|| self.rewrite(conditional, cx, file)).flatten(),
        });
    }

    fn example(
        &self,
        conditional: &Conditional<'_>,
        exit: &str,
        keyword: &str,
        file: &SourceFile,
    ) -> Option<(String, bool)> {
        let condition = String::from_utf8_lossy(conditional.condition.location().as_slice());
        let mut example = format!("{exit} {keyword} {condition}");
        let (_, col) = file.line_col(conditional.keyword.start_offset());
        let long = self
            .max_line_length
            .is_some_and(|max| col as usize - 1 + example.chars().count() > max);
        if long {
            if conditional.trivial() {
                return None;
            }
            example = format!("{keyword} {condition}; {exit}; end");
        }
        Some((example, long))
    }

    fn rewrite(
        &self,
        c: &Conditional<'_>,
        cx: &NodeContext<'_, '_>,
        file: &SourceFile,
    ) -> Option<Correction> {
        #[cfg(test)]
        REWRITE_CALLS.with(|n| n.set(n.get() + 1));
        let start = c.keyword.start_offset();
        let end = c.end.end_offset();
        let root = cx.ancestors().next_back()?;
        std::str::from_utf8(c.condition.location().as_slice()).ok()?;
        if !c.statement
            || cx.ancestors().any(|node| node.as_defined_node().is_some())
            || !standalone(c, file)
            || super::correction::contains_comment(file, start..c.condition.location().end_offset())
            || file.heredoc_pending_at(root, start)
            || super::correction::touches_directive(file, start..end)
        {
            return None;
        }
        let comments = file.comments();
        let first = comments.partition_point(|comment| comment.end <= start);
        if comments[first..]
            .iter()
            .take_while(|comment| comment.start < file.line_start(file.line_of(end) + 1))
            .any(|comment| {
                !super::correction::comment_relocation_safe(&file.bytes[comment.clone()])
            })
        {
            return None;
        }
        let other = c.other.as_ref().and_then(|n| n.as_else_node());
        let mut moved = None;
        let (exit, keyword) = if let Some(other) = &other {
            let first = c
                .body
                .as_ref()
                .and_then(|b| guard_source(b, file).map(|s| (b.body().iter().next().unwrap(), s)));
            let (guard, exit, keyword) = if let Some((guard, exit)) = first {
                (guard, exit, c.keyword())
            } else {
                let body = other.statements()?;
                let exit = guard_source(&body, file)?;
                (body.body().iter().next()?, exit, c.inverse())
            };
            // Logical guards do not always exit. raise/fail are sends and can
            // be overridden to return, in which case unwrapping executes both
            // branches. Only syntax-level unconditional exits are proven here.
            if !matches!(
                guard,
                Node::ReturnNode { .. } | Node::NextNode { .. } | Node::BreakNode { .. }
            ) || file.contains_heredoc(root, &guard)
                || !movable_guard(&guard, &c.condition, start, cx, file, root)
            {
                return None;
            }
            std::str::from_utf8(guard.location().as_slice()).ok()?;
            moved = Some(guard);
            (exit, keyword)
        } else {
            // next is local to a literal block even if define_method is
            // overridden or forwarded; return could escape the caller instead.
            (
                (if c.block_tail { "next" } else { "return" }).into(),
                c.inverse(),
            )
        };
        let (example, long) = self.example(c, &exit, keyword, file)?;
        let newline = newline(file, start);
        let header = if long {
            format!(
                "{keyword} {}{newline}  {exit}{newline}end",
                std::str::from_utf8(c.condition.location().as_slice()).ok()?
            )
        } else {
            example
        };
        let mut edits = vec![
            Correction {
                range: start..c.condition.location().end_offset(),
                replacement: header,
            },
            Correction {
                range: c.end.start_offset()..end,
                replacement: String::new(),
            },
        ];
        if let Some(then) = c.then.as_ref() {
            edits.push(Correction {
                range: then.start_offset()..then.end_offset(),
                replacement: newline.into(),
            });
        }
        if let Some(other) = other {
            let guard = moved?;
            let loc = guard.location();
            edits.push(Correction {
                range: loc.start_offset()..loc.end_offset(),
                replacement: String::new(),
            });
            let loc = other.else_keyword_loc();
            edits.push(Correction {
                range: loc.start_offset()..loc.end_offset(),
                replacement: String::new(),
            });
        } else {
            // An outer correction owns its eligible nested tail, so the one-pass
            // overlap resolver cannot strand a canonical inner ending guard.
            if let Some(tail) = c.body.as_ref().and_then(|b| ending_tail(b.as_node()))
                && let Some(mut inner) = Conditional::of(&tail)
                && inner.other.is_none()
                && !inner.accepted(file)
                && file.line_of(inner.end.start_offset())
                    > file.line_of(inner.keyword.start_offset()) + 1
            {
                inner.block_tail = c.block_tail;
                inner.statement = true;
                if let Some(edit) = self.rewrite(&inner, cx, file) {
                    edits.push(edit);
                }
            }
        }
        let edit = super::correction::combine(file, edits)?;
        #[cfg(test)]
        REWRITE_BYTES.with(|n| n.set(n.get() + edit.range.len()));
        super::correction::preserves_line_values(file, &edit).then_some(edit)
    }
}

fn newline(file: &SourceFile, offset: usize) -> &'static str {
    let line = file.line_of(offset);
    if file.bytes[file.line_start(line)..file.line_start(line + 1)].ends_with(b"\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn movable_guard(
    guard: &Node<'_>,
    condition: &Node<'_>,
    at: usize,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    root: &Node<'_>,
) -> bool {
    use crate::cops::walk::{NodeVisitor, walk};
    struct Facts<'a, 'cx, 'pr> {
        file: &'a SourceFile,
        root: &'a Node<'pr>,
        cx: &'a NodeContext<'cx, 'pr>,
        at: usize,
        safe: bool,
        condition: bool,
    }
    impl<'pr> NodeVisitor<'pr> for Facts<'_, '_, 'pr> {
        fn enter(&mut self, node: Node<'pr>) {
            #[cfg(test)]
            REWRITE_VISITS.with(|n| n.set(n.get() + 1));
            if matches!(
                node,
                Node::MatchWriteNode { .. }
                    | Node::MatchPredicateNode { .. }
                    | Node::MatchRequiredNode { .. }
            ) {
                self.safe = false;
            }
            if !self.condition {
                self.safe &= !variables::is_local_assignment(&node)
                    && !matches!(
                        node,
                        Node::SourceLineNode { .. } | Node::ImplicitNode { .. }
                    );
                if let Some(read) = node.as_local_variable_read_node() {
                    self.safe &= self.file.visible_locals(self.root).declared_before(
                        read.name().as_slice(),
                        &node,
                        self.cx,
                        self.at,
                    );
                }
            }
        }
    }
    let mut facts = Facts {
        file,
        root,
        cx,
        at,
        safe: true,
        condition: true,
    };
    walk(&mut facts, condition);
    facts.condition = false;
    walk(&mut facts, guard);
    facts.safe
}

#[cfg(test)]
thread_local! {
    static REWRITE_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static REWRITE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn standalone(c: &Conditional<'_>, file: &SourceFile) -> bool {
    let start = c.keyword.start_offset();
    let prefix = &file.bytes[file.line_start(file.line_of(start))..start];
    let prefix = prefix.strip_prefix(b"\xef\xbb\xbf").unwrap_or(prefix);
    let end = c.end.end_offset();
    let suffix = &file.line_text(file.line_of(end))[end - file.line_start(file.line_of(end))..];
    prefix.iter().all(|b| matches!(b, b' ' | b'\t'))
        && suffix
            .iter()
            .copied()
            .find(|b| !b.is_ascii_whitespace())
            .is_none_or(|b| b == b'#')
}

fn ending_tail(body: Node<'_>) -> Option<Node<'_>> {
    let body = collapse(body);
    // Explicit begin/rescue/ensure do not create method tails upstream.
    if let Some(statements) = body.as_statements_node() {
        statements.body().iter().last()
    } else if let Some(group) = body.as_parentheses_node() {
        group.body().map(collapse).and_then(|body| {
            if let Some(statements) = body.as_statements_node() {
                statements.body().iter().last()
            } else {
                Some(body)
            }
        })
    } else {
        Some(body)
    }
}

struct Conditional<'pr> {
    keyword: Location<'pr>,
    end: Location<'pr>,
    then: Option<Location<'pr>>,
    condition: Node<'pr>,
    body: Option<StatementsNode<'pr>>,
    other: Option<Node<'pr>>,
    unless: bool,
    block_tail: bool,
    covered: bool,
    statement: bool,
}

impl<'pr> Conditional<'pr> {
    fn of(node: &Node<'pr>) -> Option<Self> {
        if let Some(n) = node.as_if_node() {
            Some(Self {
                keyword: n.if_keyword_loc()?,
                end: n.end_keyword_loc()?,
                then: n.then_keyword_loc(),
                condition: n.predicate(),
                body: n.statements(),
                other: n.subsequent(),
                unless: false,
                block_tail: false,
                covered: false,
                statement: false,
            })
        } else if let Some(n) = node.as_unless_node() {
            Some(Self {
                keyword: n.keyword_loc(),
                end: n.end_keyword_loc()?,
                then: n.then_keyword_loc(),
                condition: n.predicate(),
                body: n.statements(),
                other: n.else_clause().map(|e| e.as_node()),
                unless: true,
                block_tail: false,
                covered: false,
                statement: false,
            })
        } else {
            None
        }
    }

    fn keyword(&self) -> &'static str {
        if self.unless { "unless" } else { "if" }
    }

    fn inverse(&self) -> &'static str {
        if self.unless { "if" } else { "unless" }
    }

    fn accepted(&self, file: &SourceFile) -> bool {
        self.keyword.as_slice() == b"elsif"
            || file.line_of(self.condition.location().start_offset())
                != file.last_line_of(self.condition.location())
            || self.body.as_ref().is_some_and(|body| {
                variables::condition_assignment_used_in_branch(
                    &self.condition,
                    &collapse(body.as_node()),
                )
            })
    }

    fn trivial(&self) -> bool {
        let Some(body) = self.body.as_ref() else {
            return false;
        };
        let mut expressions = body.body().iter();
        let Some(first) = expressions.next() else {
            return false;
        };
        self.other.is_none()
            && expressions.next().is_none()
            && first.as_if_node().is_none()
            && first.as_unless_node().is_none()
            && first.as_parentheses_node().is_none()
    }
}

/// A branch is a guard only when it consists of a single, single-line
/// scope exit (or a boolean expression whose immediate RHS is such an exit).
pub(super) fn guard_source(body: &StatementsNode<'_>, file: &SourceFile) -> Option<String> {
    if body.body().len() != 1 {
        return None;
    }
    let expression = body.body().iter().next()?;
    let exit = if let Some(n) = expression.as_and_node() {
        n.right()
    } else if let Some(n) = expression.as_or_node() {
        n.right()
    } else {
        body.body().iter().next()?
    };
    let guard = matches!(
        exit,
        Node::ReturnNode { .. } | Node::BreakNode { .. } | Node::NextNode { .. }
    ) || exit.as_call_node().is_some_and(|call| {
        call.receiver().is_none()
            && !call
                .block()
                .is_some_and(|block| block.as_block_node().is_some())
            && matches!(call.name().as_slice(), b"raise" | b"fail")
    });
    if !guard || file.line_of(exit.location().start_offset()) != file.last_line_of(exit.location())
    {
        return None;
    }
    Some(String::from_utf8_lossy(expression.location().as_slice()).into_owned())
}

fn constant_path_target<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    let target = match node {
        Node::ConstantPathWriteNode { .. } => node.as_constant_path_write_node().unwrap().target(),
        Node::ConstantPathAndWriteNode { .. } => {
            node.as_constant_path_and_write_node().unwrap().target()
        }
        Node::ConstantPathOrWriteNode { .. } => {
            node.as_constant_path_or_write_node().unwrap().target()
        }
        Node::ConstantPathOperatorWriteNode { .. } => node
            .as_constant_path_operator_write_node()
            .unwrap()
            .target(),
        _ => return None,
    };
    Some(target.as_node())
}

fn same_location(a: &Node<'_>, b: &Node<'_>) -> bool {
    a.location().start_offset() == b.location().start_offset()
        && a.location().end_offset() == b.location().end_offset()
}

fn assignment(node: &Node<'_>, child: &Node<'_>) -> bool {
    // Prism folds the receiver/indexing send into compound assignments. Only
    // their value has an assignment parent upstream, not receiver or indices.
    let compound_value = match node {
        Node::CallAndWriteNode { .. } => Some(node.as_call_and_write_node().unwrap().value()),
        Node::CallOrWriteNode { .. } => Some(node.as_call_or_write_node().unwrap().value()),
        Node::CallOperatorWriteNode { .. } => {
            Some(node.as_call_operator_write_node().unwrap().value())
        }
        Node::IndexAndWriteNode { .. } => Some(node.as_index_and_write_node().unwrap().value()),
        Node::IndexOrWriteNode { .. } => Some(node.as_index_or_write_node().unwrap().value()),
        Node::IndexOperatorWriteNode { .. } => {
            Some(node.as_index_operator_write_node().unwrap().value())
        }
        _ => None,
    };
    if let Some(value) = compound_value {
        return same_location(&value, child);
    }
    matches!(
        node,
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
            // Multiple assignments and for indices use a target node directly,
            // without the separate ConstantPathNode of a simple write.
            | Node::ConstantPathTargetNode { .. }
            | Node::ConstantPathWriteNode { .. }
            | Node::ConstantPathAndWriteNode { .. }
            | Node::ConstantPathOrWriteNode { .. }
            | Node::ConstantPathOperatorWriteNode { .. }
            | Node::MultiWriteNode { .. }
    ) || node
        .as_call_node()
        .is_some_and(|call| call.equal_loc().is_some())
}

fn collapse(node: Node<'_>) -> Node<'_> {
    if let Some(statements) = node.as_statements_node()
        && statements.body().len() == 1
    {
        return statements.body().iter().next().unwrap();
    }
    node
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/style/guard_clause_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/GuardClause")
            .collect()
    }

    fn assert_guard(source: &str, line: u32, col: u32, example: &str) {
        let found = offenses(source);
        assert_eq!(found.len(), 1, "{source}: {found:?}");
        assert_eq!(
            (found[0].line, found[0].col, found[0].end_line),
            (line, col, line),
            "{source}"
        );
        assert_eq!(
            found[0].message,
            format!(
                "Use a guard clause (`{example}`) instead of wrapping the code inside a conditional expression."
            )
        );
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn corrects_canonical_ending_method_guards() {
        use crate::cops::testing::assert_correction;
        for signature in [
            "def f",
            "def self.f",
            "define_method(:f) do",
            "define_singleton_method(:f) do",
        ] {
            for (keyword, inverse) in [("if", "unless"), ("unless", "if")] {
                for body in ["work", "# TODO", "effect; value"] {
                    assert_correction(
                        &super::GuardClause::default(),
                        &format!("{signature}\n  {keyword} ready?\n    {body}\n  end\nend\n"),
                        &format!(
                            "{signature}\n  {} {inverse} ready?\n    {body}\n  \nend\n",
                            if signature.starts_with("def ") {
                                "return"
                            } else {
                                "next"
                            }
                        ),
                    );
                }
            }
        }
    }

    #[test]
    fn corrects_unconditional_scope_exit_branches() {
        use crate::cops::testing::assert_correction;
        for exit in ["return", "return value", "next", "break value"] {
            let (open, close) = if exit.starts_with("return") {
                ("def f\n", "end\n")
            } else {
                ("each do\n", "end\n")
            };
            for (keyword, inverse) in [("if", "unless"), ("unless", "if")] {
                assert_correction(
                    &super::GuardClause::default(),
                    &format!(
                        "{open}  {keyword} ready?\n    {exit}\n  else\n    effect\n    value\n  end\n{close}"
                    ),
                    &format!(
                        "{open}  {exit} {keyword} ready?\n    \n  \n    effect\n    value\n  \n{close}"
                    ),
                );
                assert_correction(
                    &super::GuardClause::default(),
                    &format!(
                        "{open}  {keyword} ready?\n    effect\n    value\n  else\n    {exit}\n  end\n{close}"
                    ),
                    &format!(
                        "{open}  {exit} {inverse} ready?\n    effect\n    value\n  \n    \n  \n{close}"
                    ),
                );
            }
        }
    }

    #[test]
    fn modifier_binding_and_ambiguous_control_flow_edits_are_withheld() {
        use crate::cops::testing::correction_offenses;
        for source in [
            "if ready?\n work || raise('oops')\nelse\n value\nend\n",
            "if ready?\n raise 'oops'\nelse\n value\nend\n",
            "if ready?\n fail 'oops'\nelse\n value\nend\n",
            "def f\n if (text in value)\n  return value\n else\n  effect\n end\nend\n",
            "def f\n if /(?<value>.)/ =~ text\n  return value\n else\n  effect\n end\nend\n",
            "def f\n if ready?\n  value = 1\n else\n  return value\n end\nend\n",
            "def f\n if ready?\n  return (value = 1)\n else\n  effect\n end\nend\n",
            "consume(if ready?\n raise 'oops'\nelse\n value\nend)\n",
            "def f\n consume(\n  if ready?\n   return 1\n  else\n   value\n  end\n )\nend\n",
            "def f\n [\n  if ready?\n   return 1\n  else\n   value\n  end\n ]\nend\n",
            "def f\n defined?(\n  if ready?\n   return 1\n  else\n   value\n  end\n )\nend\n",
            "def f\n defined?(begin\n  if ready?\n   return 1\n  else\n   value\n  end\n end)\nend\n",
            "def f\n if ready?\n  return __LINE__\n else\n  effect\n end\nend\n",
        ] {
            let out = correction_offenses(&super::GuardClause::default(), source);
            assert!(!out.is_empty(), "{source}");
            assert!(
                out.iter().all(|o| o.correction.is_none()),
                "{source}: {out:?}"
            );
        }
    }

    #[test]
    fn incompatible_missing_else_selection_withholds_only_guard_corrections() {
        use crate::{
            cops::{Cop, run_cops},
            source::SourceFile,
        };
        struct MissingElse;
        impl Cop for MissingElse {
            fn name(&self) -> &'static str {
                "Style/MissingElse"
            }
            fn safe_fixes(&self) -> bool {
                true
            }
        }
        let cop = super::GuardClause::default();
        assert_eq!(cop.autocorrect_incompatible_with(), &["Style/MissingElse"]);
        let source = "def f\n if ready?\n  work\n end\nend\n";
        let parsed = ruby_prism::parse(source.as_bytes());
        let file = SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
        for cops in [
            vec![&cop as &dyn Cop, &MissingElse],
            vec![&MissingElse as &dyn Cop, &cop],
        ] {
            let mut out = Vec::new();
            assert!(run_cops(&cops, &parsed, &file, &mut out).is_empty());
            assert_eq!(out.len(), 1);
            assert!(out[0].correction.is_none());
        }
        let mut out = Vec::new();
        assert!(run_cops(&[&cop], &parsed, &file, &mut out).is_empty());
        assert!(out[0].correction.is_some());
    }

    #[test]
    fn ordinary_comments_then_and_long_block_guards_have_exact_edits() {
        use crate::cops::testing::assert_correction;
        let cop = super::GuardClause::default();
        assert_correction(
            &cop,
            "def f(value)\n  if ready? # condition\n    # exit\n    return value # value\n  else # continue\n    work\n  end # conditional\nend\n",
            "def f(value)\n  return value if ready? # condition\n    # exit\n     # value\n   # continue\n    work\n   # conditional\nend\n",
        );
        assert_correction(
            &cop,
            "if condition then return 1 else work end\n",
            "return 1 if condition \n   work \n",
        );
        let exit = format!("return '{}'", "e".repeat(120));
        assert_correction(
            &cop,
            &format!("if x\nelse\n  {exit}\nend\n"),
            &format!("unless x\n  {exit}\nend\n\n  \n\n"),
        );
    }

    #[test]
    fn nested_tail_correction_is_atomic_and_sibling_safety_work_is_linear() {
        use crate::{
            cops::{testing::correction_offenses, variables::VISIBLE_LOCAL_BUILDS},
            pipeline::lint_str_with,
        };
        let cop = super::GuardClause::default();
        for methods in [1000, 2000] {
            let source = (0..methods).map(|i| format!("def f{i}(value, ready)\n if ready\n  return value\n else\n  effect\n  42\n end\nend\n")).collect::<String>();
            super::REWRITE_VISITS.with(|n| n.set(0));
            super::REWRITE_CALLS.with(|n| n.set(0));
            VISIBLE_LOCAL_BUILDS.with(|n| n.set(0));
            assert_eq!(lint_str_with(&[&cop], &source).len(), methods);
            super::REWRITE_VISITS.with(|n| assert_eq!(n.get(), 0));
            super::REWRITE_CALLS.with(|n| assert_eq!(n.get(), 0));
            VISIBLE_LOCAL_BUILDS.with(|n| assert_eq!(n.get(), 0));
            let found = correction_offenses(&cop, &source);
            assert_eq!(
                found.iter().filter(|o| o.correction.is_some()).count(),
                methods
            );
            super::REWRITE_VISITS.with(|n| assert_eq!(n.get(), methods * 4));
            super::REWRITE_CALLS.with(|n| assert_eq!(n.get(), methods));
            VISIBLE_LOCAL_BUILDS.with(|n| assert_eq!(n.get(), 1));
        }
        let depth = 20;
        let source = format!(
            "def f\n{}work\nmore_work\n{}end\n",
            "if ready?\n".repeat(depth),
            "end\n".repeat(depth)
        );
        super::REWRITE_CALLS.with(|n| n.set(0));
        super::REWRITE_BYTES.with(|n| n.set(0));
        let found = correction_offenses(&cop, &source);
        assert_eq!(found.len(), depth);
        assert_eq!(found.iter().filter(|o| o.correction.is_some()).count(), 1);
        super::REWRITE_CALLS.with(|n| assert_eq!(n.get(), depth));
        super::REWRITE_BYTES.with(|n| assert!(n.get() <= source.len() * depth));
        let after = crate::corrections::apply(source.as_bytes(), &found).unwrap();
        let after = String::from_utf8(after.bytes).unwrap();
        assert!(lint_str_with(&[&cop], &after).is_empty());
    }

    #[test]
    fn style_and_layout_overlaps_are_order_independent_and_bounded() {
        use crate::{
            config::ResolvedConfig,
            cops::{Registry, select_active},
            pipeline::lint_str_with_corrections,
        };
        let registry = Registry::new();
        let config = ResolvedConfig::default();
        let active = select_active(
            &registry,
            &config,
            &[
                "Style/GuardClause".into(),
                "Style/StringLiterals".into(),
                "Style/HashSyntax".into(),
                "Style/RedundantReturn".into(),
                "Style/Semicolon".into(),
                "Style/RedundantParentheses".into(),
                "Style/SignalException".into(),
                "Layout".into(),
            ],
            &[],
        );
        let source = "def f(c)\n  if c\n    value = { :foo => 'bar' }; side_effect(value)\n    return (value)\n  end\nend\n";
        let first = "def f(c)\n  return unless c\n    value = { :foo => 'bar' }; side_effect(value)\n    return (value)\n  \nend\n";
        let mut current = source.to_owned();
        let mut passes = 0;
        loop {
            let mut found = lint_str_with_corrections(&active, &current);
            let forward = crate::corrections::apply(current.as_bytes(), &found).unwrap();
            found.reverse();
            assert_eq!(
                crate::corrections::apply(current.as_bytes(), &found)
                    .unwrap()
                    .bytes,
                forward.bytes
            );
            if forward.count == 0 {
                break;
            }
            passes += 1;
            assert!(passes <= 6, "bounded curated fixture: {current}");
            current = String::from_utf8(forward.bytes).unwrap();
            assert_eq!(ruby_prism::parse(current.as_bytes()).errors().count(), 0);
            if passes == 1 {
                assert_eq!(current, first);
            }
        }
        assert_eq!(passes, 3, "the CLI must not silently run multiple passes");
        assert_eq!(
            current,
            "def f(c)\n  return unless c\n    value = { foo: \"bar\" }\n side_effect(value)\n    value\nend\n"
        );
    }

    #[test]
    fn ending_method_conditionals() {
        for signature in [
            "def func",
            "def self.func",
            "define_method(:func) do",
            "object.define_singleton_method(:func) do",
        ] {
            for (keyword, inverse) in [("if", "unless"), ("unless", "if")] {
                for body in ["work", "# TODO", "do_something(foo)", "side_effect; value"] {
                    for before in ["", "  test\n"] {
                        let source = format!(
                            "{signature}\n{before}  {keyword} something\n    {body}\n  end\nend\n"
                        );
                        assert_guard(
                            &source,
                            if before.is_empty() { 2 } else { 3 },
                            3,
                            &format!("return {inverse} something"),
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn scope_exits_in_either_branch() {
        for exit in [
            "return",
            "return value",
            "raise 'error'",
            "fail Error, 'message'",
            "work || raise('message')",
            "work and return",
        ] {
            for (keyword, inverse) in [("if", "unless"), ("unless", "if")] {
                assert_guard(
                    &format!("{keyword} something\n {exit}\nelse\n side_effect\n value\nend\n"),
                    1,
                    1,
                    &format!("{exit} {keyword} something"),
                );
                assert_guard(
                    &format!("{keyword} something\n side_effect\n value\nelse\n {exit}\nend\n"),
                    1,
                    1,
                    &format!("{exit} {inverse} something"),
                );
            }
        }
        for exit in ["next", "next value", "break", "break value"] {
            assert_guard(
                &format!("each do\n if condition\n {exit}\n else\n work\n end\nend\n"),
                2,
                2,
                &format!("{exit} if condition"),
            );
        }
        assert_guard(
            "if something then work else raise('message') end\n",
            1,
            1,
            "raise('message') unless something",
        );
    }

    #[test]
    fn block_pass_raise_and_fail_are_guards_but_literal_blocks_are_not() {
        for method in ["raise", "fail"] {
            for prefix in ["", "work && ", "work || ", "work and ", "work or "] {
                let exit = format!("{prefix}{method}(\"oops\", &handler)");
                for (keyword, inverse) in [("if", "unless"), ("unless", "if")] {
                    assert_guard(
                        &format!("{keyword} ready?\n {exit}\nelse\n work\nend\n"),
                        1,
                        1,
                        &format!("{exit} {keyword} ready?"),
                    );
                    assert_guard(
                        &format!("{keyword} ready?\n work\nelse\n {exit}\nend\n"),
                        1,
                        1,
                        &format!("{exit} {inverse} ready?"),
                    );
                }
                for block in ["{ handle }", "do handle end"] {
                    let exit = format!("{prefix}{method}(\"oops\") {block}");
                    for source in [
                        format!("if ready?\n {exit}\nelse\n work\nend\n"),
                        format!("if ready?\n work\nelse\n {exit}\nend\n"),
                    ] {
                        assert!(offenses(&source).is_empty(), "{source}");
                    }
                }
            }
        }
    }

    #[test]
    fn indexed_compound_assignments_only_exempt_the_value() {
        for operator in ["&&=", "||=", "+="] {
            assert_guard(
                &format!("items[if ready?\n  raise \"oops\"\nelse\n  key\nend] {operator} value\n"),
                1,
                7,
                "raise \"oops\" if ready?",
            );
            let source =
                format!("items[key] {operator} if ready?\n  raise \"oops\"\nelse\n  value\nend\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }

    #[test]
    fn call_compound_assignments_only_exempt_the_value() {
        for operator in ["&&=", "||=", "+="] {
            for access in [".", "&."] {
                assert_guard(
                    &format!(
                        "if ready?\n raise \"oops\"\nelse\n object\nend{access}attr {operator} value\n"
                    ),
                    1,
                    1,
                    "raise \"oops\" if ready?",
                );
                let source = format!(
                    "object{access}attr {operator} if ready?\n raise \"oops\"\nelse\n value\nend\n"
                );
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        // A simple setter send, unlike compound writes, is itself an
        // assignment upstream, including its receiver edge.
        for access in [".", "&."] {
            let source =
                format!("if ready?\n raise \"oops\"\nelse\n object\nend{access}attr = value\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }

    #[test]
    fn constant_path_assignment_only_folds_the_immediate_target() {
        for operator in ["=", "&&=", "||=", "+="] {
            // Prism's target ConstantPathNode is part of the casgn upstream,
            // for both simple and compound writes.
            let source =
                format!("if ready?\n raise \"oops\"\nelse\n object\nend::Const {operator} value\n");
            assert!(offenses(&source).is_empty(), "{source}");
            for target in ["Namespace::Const", "Namespace::Inner::Const"] {
                let source =
                    format!("{target} {operator} if ready?\n raise \"oops\"\nelse\n value\nend\n");
                assert!(offenses(&source).is_empty(), "{source}");
            }
            // The inner namespace is still a const read, not the casgn.
            assert_guard(
                &format!(
                    "if ready?\n raise \"oops\"\nelse\n object\nend::Inner::Const {operator} value\n"
                ),
                1,
                1,
                "raise \"oops\" if ready?",
            );
        }
        for suffix in [
            "::Const",
            "::Const.attr ||= value",
            ".namespace::Const = value",
        ] {
            assert_guard(
                &format!("if ready?\n raise \"oops\"\nelse\n object\nend{suffix}\n"),
                1,
                1,
                "raise \"oops\" if ready?",
            );
        }
        // Do not skip a const read merely because an assignment encloses it.
        assert_guard(
            "result = if ready?\n raise \"oops\"\nelse\n object\nend::Const\n",
            1,
            10,
            "raise \"oops\" if ready?",
        );
    }

    #[test]
    fn constant_path_targets_exempt_only_the_immediate_namespace() {
        for (prefix, suffix) in [
            ("", ", other = values\n"),
            ("other, ", " = values\n"),
            ("for ", " in items\n work\nend\n"),
        ] {
            let conditional = "if ready?\n raise \"oops\"\nelse\n object\nend";
            let source = format!("{prefix}{conditional}::Const{suffix}");
            assert!(offenses(&source).is_empty(), "{source}");
            // The target is an assignment, but its nested namespace and
            // receiver calls still have read/send parents upstream.
            for namespace in ["::Inner", ".namespace"] {
                assert_guard(
                    &format!("{prefix}{conditional}{namespace}::Const{suffix}"),
                    1,
                    prefix.len() as u32 + 1,
                    "raise \"oops\" if ready?",
                );
            }
        }
    }

    #[test]
    fn accepts_non_guard_branches_and_value_contexts() {
        for body in [
            "side_effect; return",
            "return(\n value\n)",
            "Kernel.raise 'error'",
            "object.fail",
            "(return)",
            "-> { return }",
            "work || (return)",
            "return if other",
        ] {
            let source = format!("if x\n {body}\nelse\n value\nend\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        for assignment in [
            "result =",
            "@result =",
            "@@result =",
            "$result =",
            "Result =",
            "self.result =",
            "object[0] =",
            "a, b =",
            "result ||=",
            "result +=",
        ] {
            let source = format!("{assignment} if x\n raise 'error'\nelse\n value\nend\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }

    #[test]
    fn condition_local_assignment_dependency() {
        for condition in [
            "(foo = bar)",
            "(foo = bar && baz = qux)",
            "foo ||= bar",
            "foo &&= bar",
            "foo += bar",
            "((foo, baz = values))",
            "((foo, baz = values) in [first, last])",
        ] {
            let source = format!("def f\n if {condition}\n return foo\n else\n value\n end\nend\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        assert_guard(
            "def f\n  if (foo = bar)\n    return baz\n  else\n    qux\n  end\nend\n",
            2,
            3,
            "return baz if (foo = bar)",
        );
        assert!(offenses("def f\n if (foo = bar)\n consume(foo)\n end\nend\n").is_empty());
        // Only the syntactic first branch's descendant reads participate,
        // including nested blocks. This is not an assignment-liveness rule.
        assert!(
            offenses("if (foo = bar)\n return work { |foo| consume(foo) }\nelse\n value\nend\n")
                .is_empty()
        );
        assert_guard(
            "if (foo = bar)\n return baz\nelse\n consume(foo)\nend\n",
            1,
            1,
            "return baz if (foo = bar)",
        );
        // Named captures and pattern bindings aren't lvasgn descendants upstream.
        assert_guard(
            "if /(?<foo>.)/ =~ text\n return foo\nelse\n value\nend\n",
            1,
            1,
            "return foo if /(?<foo>.)/ =~ text",
        );
        assert_guard(
            "if (value in foo)\n return foo\nelse\n value\nend\n",
            1,
            1,
            "return foo if (value in foo)",
        );
    }

    #[test]
    fn nested_guards_and_consecutive_conditionals() {
        let source = "def f\n if first\n  work\n end\n if outer\n  before\n  unless inner\n   effect\n   value\n  end\n end\nend\n";
        let found = offenses(source);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(5, 2), (7, 3)]
        );
        for (offense, example) in found.iter().zip(["return unless outer", "return if inner"]) {
            assert_eq!(
                offense.message,
                format!(
                    "Use a guard clause (`{example}`) instead of wrapping the code inside a conditional expression."
                )
            );
        }
        // An accepted outer condition must not invent an ending-body scope
        // for an arbitrary nested conditional.
        assert!(offenses("def f\n if a &&\n b\n  if c\n   work\n  end\n end\nend\n").is_empty());
        assert_guard(
            "def f\n (before; if condition\n  work\n end)\nend\n",
            2,
            11,
            "return unless condition",
        );
    }

    #[test]
    fn begin_rescue_ensure_do_not_create_ending_method_tails() {
        for source in [
            "def f\n begin\n  if condition\n   work\n  end\n end\nend\n",
            "def f\n if condition\n  work\n end\nrescue\n fallback\nend\n",
            "def f\n work\nrescue\n if condition\n  fallback\n end\nend\n",
            "def f\n if condition\n  work\n end\nensure\n cleanup\nend\n",
            "def f\n work\nensure\n if condition\n  cleanup\n end\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_guard(
            "def f\n begin\n  if condition\n   raise 'error'\n  else\n   work\n  end\n ensure\n  cleanup\n end\nend\n",
            3,
            3,
            "raise 'error' if condition",
        );
        assert_guard(
            "def f\n if condition\n  begin\n   work\n  rescue\n   fallback\n  ensure\n   cleanup\n  end\n end\nend\n",
            2,
            2,
            "return unless condition",
        );
    }

    #[test]
    fn numbered_and_it_method_blocks_but_not_ordinary_blocks() {
        for parameter in ["_1", "it"] {
            for method in ["define_method", "define_singleton_method"] {
                assert_guard(
                    &format!("{method}(:f) do\n  if {parameter}\n    work\n  end\nend\n"),
                    2,
                    3,
                    &format!("return unless {parameter}"),
                );
            }
        }
        for method in [
            "each",
            "proc",
            "lambda",
            "define_methods",
            "define_method_like",
        ] {
            assert!(
                offenses(&format!("{method} do\n if condition\n  work\n end\nend\n")).is_empty()
            );
        }
    }

    #[test]
    fn heredoc_guard_messages_use_the_header_not_literal_contents() {
        for argument in [
            "<<~MESSAGE",
            "<<~MESSAGE.strip",
            "do_something(<<~MESSAGE)",
            "obj&.do_something(<<~MESSAGE)",
            "<<~`MESSAGE`",
        ] {
            assert_guard(
                &format!(
                    "if condition\n  work\nelse\n  raise {argument}\n    oops\n  MESSAGE\nend\n"
                ),
                1,
                1,
                &format!("raise {argument} unless condition"),
            );
        }
        assert_guard(
            "if condition\n  # comment\n  return __LINE__ # don't move\nelse\n  effect\n  value\nend\n",
            1,
            1,
            "return __LINE__ if condition",
        );
        assert!(offenses("# if condition\ntext = 'raise if condition'\n").is_empty());
    }

    #[test]
    fn length_limit_controls_trivial_bodies_and_examples() {
        use crate::{
            config::{RawConfig, ResolvedConfig},
            cops::{Registry, select_active},
            pipeline::lint_str_with,
        };
        let registry = Registry::new();
        let source = format!("def f\n  if {}\n    work\n  end\nend\n", "x".repeat(108));
        // 2 columns + `return unless ` (14) + 108 characters exceeds 120.
        assert!(offenses(&source).is_empty());
        for (config, expected) in [
            ("[cops.\"Layout/LineLength\"]\nmax = 140\n", 1),
            ("[cops.\"Layout/LineLength\"]\nenabled = false\n", 1),
            ("[cops.Layout]\nenabled = false\n", 1),
            (
                "[cops.Layout]\nenabled = false\n[cops.\"Layout/LineLength\"]\nenabled = true\n",
                0,
            ),
            ("[cops.\"Layout/LineLength\"]\nmax = 80\n", 0),
        ] {
            let raw: RawConfig = toml::from_str(config).unwrap();
            let config = ResolvedConfig::resolve(raw, None, &registry).unwrap();
            let active = select_active(&registry, &config, &["Style/GuardClause".into()], &[]);
            assert_eq!(lint_str_with(&active, &source).len(), expected);
        }
        let condition = "x".repeat(108);
        assert_guard(
            &format!("def f\n  if {condition}\n    work\n    more_work\n  end\nend\n"),
            2,
            3,
            &format!("unless {condition}; return; end"),
        );
        assert_guard(
            &format!("if x\nelse\n  raise '{}'\nend\n", "e".repeat(120)),
            1,
            1,
            &format!("unless x; raise '{}'; end", "e".repeat(120)),
        );
    }

    #[test]
    fn subtree_work_is_bounded_by_ast_depth_and_disjoint_scopes_scale_linearly() {
        use crate::{
            cops::{
                depth::MAX_AST_DEPTH,
                variables::CONDITION_BRANCH_VISITS,
                walk::{NodeVisitor, walk},
            },
            pipeline::lint_str_with,
        };
        struct Count(usize);
        impl<'pr> NodeVisitor<'pr> for Count {
            fn enter(&mut self, _: ruby_prism::Node<'pr>) {
                self.0 += 1;
            }
        }
        let mut previous = 0;
        for methods in [200, 400] {
            let mut source = String::new();
            for index in 0..methods {
                source.push_str(&format!("def f{index}\n"));
                for level in 0..10 {
                    source.push_str(&format!("if (local{level} = obtain)\n"));
                }
                source.push_str("side_effect\nvalue\n");
                source.push_str(&"end\n".repeat(11));
            }
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0);
            let mut count = Count(0);
            walk(&mut count, &parsed.node());
            CONDITION_BRANCH_VISITS.with(|n| n.set(0));
            assert_eq!(
                lint_str_with(&[&super::GuardClause::default()], &source).len(),
                methods * 10
            );
            let visits = CONDITION_BRANCH_VISITS.with(|n| n.get());
            // Only condition/branch subtrees are walked, at most once per
            // enclosing conditional. No file scan or reparsing per candidate.
            assert!(visits > count.0);
            assert!(visits <= count.0 * MAX_AST_DEPTH);
            if previous != 0 {
                assert_eq!(visits, previous * 2);
            }
            previous = visits;
        }
    }

    #[test]
    fn non_ending_and_associated_conditionals_are_accepted() {
        for source in [
            "if something\n work\nend\n",
            "foo do\n if something\n  work\n end\nend\n",
            "def f\n if x\n  work\n end\n after\nend\n",
            "def f\n work if x\nend\n",
            "def f\n x ? a : b\nend\n",
            "def f\n if x\n a\n else\n b\n end\nend\n",
            "def f\n if x\n return\n elsif y\n raise 'error'\n else\n return\n end\nend\n",
            "def f\n if x &&\n y\n work\n end\nend\n",
            "def f = if x\n work\nend\n",
            "def self.f = if x\n raise 'error'\nelse\n work\nend\n",
            "def f\n ()\nend\n",
            "def f\n if x\n end\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
