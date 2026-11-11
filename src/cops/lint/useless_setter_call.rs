//! `Lint/UselessSetterCall` — final setter calls on syntactically local objects.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        variables::ScopeAnalysis,
        walk::{self, NodeVisitor},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::collections::HashMap;
pub struct UselessSetterCall;
impl Cop for UselessSetterCall {
    fn name(&self) -> &'static str {
        "Lint/UselessSetterCall"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(program) = node.as_program_node() else {
            return;
        };
        let analysis = ScopeAnalysis::of(&program);
        let mut candidates = HashMap::new();
        for id in analysis.scopes_in_exit_order() {
            let scope = analysis.scope(id);
            if scope.node.as_def_node().is_none() {
                continue;
            }
            let Some(body) = scope.body() else { continue };
            let key = node_key(&body);
            let Some(last) = last_expression(body) else {
                continue;
            };
            let Some(call) = last.as_call_node().filter(|c| {
                !c.is_safe_navigation()
                    && c.equal_loc().is_some()
                    && !c.block().is_some_and(|b| b.as_block_node().is_some())
            }) else {
                continue;
            };
            let Some(receiver) = call
                .receiver()
                .and_then(|n| n.as_local_variable_read_node())
            else {
                continue;
            };
            let name = receiver.name().as_slice();
            // VariableForce supplies actual lexical local declarations. Its value
            // liveness/escape model must NOT replace the pinned raw-name tracker.
            if !scope.variables().any(|v| analysis.variable(v).name == name) {
                continue;
            }
            candidates.insert(key, (name, receiver.location().start_offset()));
        }
        if candidates.is_empty() {
            return;
        }
        let mut tracker = Tracker {
            file,
            out,
            candidates,
            state: HashMap::new(),
            frames: Vec::new(),
            clock: 0,
        };
        walk::walk(&mut tracker, node);
    }
}
// One begin projection only: a singleton StatementsNode is transparent, while
// parentheses are real begin nodes. Explicit begin/end is kwbegin, not begin.
fn last_expression<'pr>(body: Node<'pr>) -> Option<Node<'pr>> {
    if let Some(s) = body.as_statements_node() {
        if s.body().len() != 1 {
            return s.body().iter().last();
        }
        return last_expression(s.body().iter().next()?);
    }
    if let Some(p) = body.as_parentheses_node() {
        return p
            .body()
            .and_then(|n| n.as_statements_node())
            .and_then(|s| s.body().iter().last());
    }
    Some(body)
}

// A true origin ordinal is copied by variable aliases. Starting a method's
// tracker empty is equivalent to requiring that origin to lie inside its body.
// This evaluates ALL nested method windows in one preorder walk, without
// rescanning each def subtree or inferring runtime bindings/types/control flow.
type NodeKey<'pr> = (std::mem::Discriminant<Node<'pr>>, usize, usize);
fn node_key<'pr>(node: &Node<'pr>) -> NodeKey<'pr> {
    (
        std::mem::discriminant(node),
        node.location().start_offset(),
        node.location().end_offset(),
    )
}
struct Tracker<'a, 'pr> {
    file: &'a SourceFile,
    out: &'a mut Vec<Offense>,
    candidates: HashMap<NodeKey<'pr>, (&'pr [u8], usize)>,
    state: HashMap<&'pr [u8], usize>,
    frames: Vec<Frame<'pr>>,
    clock: usize,
}
struct Frame<'pr> {
    candidate: Option<(&'pr [u8], usize)>,
    start: usize,
    shadow: Option<HashMap<&'pr [u8], usize>>,
}
impl<'pr> Tracker<'_, 'pr> {
    fn assign(&mut self, name: &'pr [u8], rhs: &Node<'pr>) {
        let origin = if let Some(name) = read_name(rhs) {
            self.state.get(name).copied().unwrap_or(0)
        } else if ast::is_literal(rhs)
            || ast::negated_numeric_literal(rhs).is_some()
            || rhs.as_call_node().is_some_and(|c| {
                !c.is_safe_navigation()
                    && c.name().as_slice() == b"new"
                    && !c.block().is_some_and(|b| b.as_block_node().is_some())
            })
        {
            self.clock
        } else {
            0
        };
        self.state.insert(name, origin);
    }
}
impl<'pr> NodeVisitor<'pr> for Tracker<'_, 'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        self.clock += 1;
        let mut skip = false;
        if let Some(multi) = node.as_multi_write_node() {
            let rhs = multi.value();
            let values = rhs
                .as_array_node()
                .map(|a| a.elements().iter().collect::<Vec<_>>());
            for (i, target) in multi
                .lefts()
                .iter()
                .chain(multi.rest())
                .chain(multi.rights().iter())
                .enumerate()
            {
                if let Some(name) = target_name(&target) {
                    if let Some(value) = values.as_ref().and_then(|v| v.get(i)) {
                        self.assign(name, value);
                    } else {
                        self.state.insert(name, self.clock);
                    }
                }
            }
            skip = true;
        } else if let Some((name, value, compound)) = assignment(&node) {
            if compound == 2 {
                self.state.insert(name, self.clock);
            } else {
                self.assign(name, &value);
            }
            skip = compound != 0;
        }
        let shadow = skip.then(|| std::mem::take(&mut self.state));
        self.frames.push(Frame {
            candidate: self.candidates.remove(&node_key(&node)),
            start: self.clock,
            shadow,
        });
    }
    fn leave(&mut self) {
        let frame = self.frames.pop().unwrap();
        if let Some(state) = frame.shadow {
            self.state = state;
        }
        if let Some((name, offset)) = frame.candidate
            && self
                .state
                .get(name)
                .is_some_and(|origin| *origin >= frame.start)
        {
            let (line, col) = super::array_literal_in_regexp::reference_line_col(offset, self.file);
            self.out.push(Offense {
                cop: "Lint/UselessSetterCall",
                severity: crate::offense::Severity::Error,
                line,
                col,
                end_line: line,
                correction: None,
                message: format!(
                    "Useless setter call to local variable `{}`.",
                    super::useless_or::source_text(name, self.file)
                ),
            });
        }
    }
}
fn read_name<'pr>(n: &Node<'pr>) -> Option<&'pr [u8]> {
    match n {
        Node::LocalVariableReadNode { .. } => {
            Some(n.as_local_variable_read_node()?.name().as_slice())
        }
        Node::ItLocalVariableReadNode { .. } => Some(b"it"),
        Node::InstanceVariableReadNode { .. } => {
            Some(n.as_instance_variable_read_node()?.name().as_slice())
        }
        Node::ClassVariableReadNode { .. } => {
            Some(n.as_class_variable_read_node()?.name().as_slice())
        }
        Node::GlobalVariableReadNode { .. } => {
            Some(n.as_global_variable_read_node()?.name().as_slice())
        }
        Node::BackReferenceReadNode { .. } | Node::NumberedReferenceReadNode { .. } => {
            Some(n.location().as_slice())
        }
        _ => None,
    }
}
fn target_name<'pr>(n: &Node<'pr>) -> Option<&'pr [u8]> {
    match n {
        Node::LocalVariableTargetNode { .. } => {
            Some(n.as_local_variable_target_node()?.name().as_slice())
        }
        Node::InstanceVariableTargetNode { .. } => {
            Some(n.as_instance_variable_target_node()?.name().as_slice())
        }
        Node::ClassVariableTargetNode { .. } => {
            Some(n.as_class_variable_target_node()?.name().as_slice())
        }
        Node::GlobalVariableTargetNode { .. } => {
            Some(n.as_global_variable_target_node()?.name().as_slice())
        }
        _ => None,
    }
}
// 0 = ordinary preorder assignment; 1 = logical (use RHS, skip children);
// 2 = binary (unconditionally local, skip children), exactly as upstream.
fn assignment<'pr>(n: &Node<'pr>) -> Option<(&'pr [u8], Node<'pr>, u8)> {
    macro_rules! assignments { ($($kind:ident, $as:ident, $compound:literal;)*) => { match n {
        $(Node::$kind { .. } => { let a = n.$as()?; Some((a.name().as_slice(), a.value(), $compound)) },)*
        _ => None,
    } } }
    assignments! {
        LocalVariableWriteNode, as_local_variable_write_node, 0;
        InstanceVariableWriteNode, as_instance_variable_write_node, 0;
        ClassVariableWriteNode, as_class_variable_write_node, 0;
        GlobalVariableWriteNode, as_global_variable_write_node, 0;
        LocalVariableOrWriteNode, as_local_variable_or_write_node, 1;
        InstanceVariableOrWriteNode, as_instance_variable_or_write_node, 1;
        ClassVariableOrWriteNode, as_class_variable_or_write_node, 1;
        GlobalVariableOrWriteNode, as_global_variable_or_write_node, 1;
        LocalVariableAndWriteNode, as_local_variable_and_write_node, 1;
        InstanceVariableAndWriteNode, as_instance_variable_and_write_node, 1;
        ClassVariableAndWriteNode, as_class_variable_and_write_node, 1;
        GlobalVariableAndWriteNode, as_global_variable_and_write_node, 1;
        LocalVariableOperatorWriteNode, as_local_variable_operator_write_node, 2;
        InstanceVariableOperatorWriteNode, as_instance_variable_operator_write_node, 2;
        ClassVariableOperatorWriteNode, as_class_variable_operator_write_node, 2;
        GlobalVariableOperatorWriteNode, as_global_variable_operator_write_node, 2;
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/useless_setter_call_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Lint/UselessSetterCall")
            .collect()
    }
    #[test]
    fn nested_method_windows_preserve_raw_name_preorder_not_runtime_scopes() {
        let source =
            "def f(arg); top = arg; def g; top = Top.new; top.attr = 5; end; top.attr = 5; end";
        assert_eq!(offenses(source).len(), 2);
        assert!(offenses("top = Top.new; def f(top); top.attr = 5; end").is_empty());
        assert!(offenses("def f(top = Top.new); top.attr = 5; end").is_empty());
        assert_eq!(offenses("def f(arg); a, b = [def g; top = Top.new; top.attr = 5; end, 2]; top = arg; top.attr = 5; end").len(), 1);
        let mut source = String::new();
        for i in 0..24 {
            source.push_str(&format!("def f{i}(arg); top = arg; "));
        }
        source.push_str("top = Top.new; ");
        for _ in 0..24 {
            source.push_str("top.attr = 5; end; ");
        }
        assert_eq!(offenses(&source).len(), 24);
    }
    #[test]
    fn skip_children_and_aliases_use_assignment_time_state() {
        for body in [
            "top = Top.new; other = top; top = arg; top = other; top.attr = 5",
            "top = arg; top += arg; other = top; top = other; top.attr = 5",
        ] {
            assert_eq!(offenses(&format!("def f(arg); {body}; end")).len(), 1);
        }
        assert!(
            offenses("def f(arg); a, top = (arg = Top.new), arg; top.attr = 5; end").is_empty()
        );
        assert!(
            offenses(
                "def f(arg); top = arg; top ||= (arg = Top.new); top = arg; top.attr = 5; end"
            )
            .is_empty()
        );
    }
    #[test]
    fn pinned_local_constructors_assignments_and_singleton_methods() {
        for header in ["def test(arg)", "def Top.test(arg)"] {
            for assignment in [
                "top = Top.new",
                "top = {}",
                "top = arg; top += arg",
                "a, top, c = do_something",
                "top = nil; top ||= Top.new",
                "@x = Top.new; top = @x",
            ] {
                for setter in ["top.attr = 5", "top[:attr] = 5"] {
                    let source = format!("{header}\n  {assignment}\n  {setter}\nend\n");
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col), (3, 3));
                    assert_eq!(
                        found[0].message,
                        "Useless setter call to local variable `top`."
                    );
                    assert!(found[0].correction.is_none());
                }
            }
        }
    }
    #[test]
    fn pinned_lookalikes_and_argument_aliases() {
        for body in [
            "something; @top = 5",
            "something; @top.attr = 5",
            "something; @@top = 5",
            "something; @@top.attr = 5",
            "something; $top = 5",
            "something; $top.attr = 5",
            "x = Top.new; arg.attr = 5",
            "(a,b), top = arg, arg; top.attr = 5",
            "@x = arg; @x.work; top = @x; top.work; top.attr = 5",
            "a, top, c = 1, arg, 3; top.attr = 5",
            "top = nil; top ||= arg; top.attr = 5",
            "top = Foo.shared_object; top[:attr] = 1",
            "top.attr == 5",
            "begin; work; rescue StandardError => _; end; arg[:baz] = true",
            "top = Top.new; top&.attr = 5",
            "top = Top.new; top.attr = 5; top",
            "top = Top.new; return top.attr = 5",
        ] {
            let source = format!("def test(arg); {body}; end");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
}
