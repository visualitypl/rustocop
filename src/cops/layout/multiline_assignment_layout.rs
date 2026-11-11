//! `Layout/MultilineAssignmentLayout` (opt-in) — a multi-line right-hand side
//! of a supported type starts on the line after the assignment operator.
//!
//! Fixed `new_line` style and the default `SupportedTypes` (`block`, `case`,
//! `class`, `if`, `kwbegin`, `module`). Port of the `CheckAssignment` mixin's
//! callbacks: every parser `lvasgn`, `ivasgn`, `cvasgn`, `gvasgn`, `casgn`,
//! `masgn`, `op_asgn`, `or_asgn` and `and_asgn` with a value, and every `send`
//! (never `csend`) whose operator is `=` (attribute and index writes).
use ruby_prism::{Location, Node};

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct MultilineAssignmentLayout;

const MSG: &str = "Right hand side of multi-line assignment is on the same line as the \
                   assignment operator `=`.";

impl Cop for MultilineAssignmentLayout {
    fn name(&self) -> &'static str {
        "Layout/MultilineAssignmentLayout"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    /// Opt-in as upstream: neither `--only Layout` nor `[cops.Layout] enabled =
    /// true` runs it (RuboCop enables an `Enabled: false` cop only through its
    /// own name), so the accepted department compositions keep their results.
    fn department_selection_enables(&self) -> bool {
        false
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some((operator, rhs)) = assignment(node) else {
            return;
        };
        let Some(kind) = supported(&rhs) else {
            return;
        };
        let location = node.location();
        let rhs_location = rhs.location();
        let rhs_line = file.line_of(rhs_location.start_offset());
        // A single-line value is accepted, unless it is a block whose opening
        // is not on the assignment's first line.
        if rhs_line == file.last_line_of(rhs_location)
            && match kind {
                Supported::Plain => true,
                Supported::Block(opening) => {
                    file.line_of(opening.start_offset()) == file.line_of(location.start_offset())
                }
            }
        {
            return;
        }
        // `check_new_line_offense`
        if file.line_of(operator.start_offset()) != rhs_line {
            return;
        }
        let correction = cx
            .corrections_requested()
            .then(|| {
                let root = cx.ancestors().last().unwrap_or(node);
                if file
                    .class_method_bindings(root)
                    .has_listed_source_observer(file.data_start().is_some())
                {
                    return None;
                }
                // `insert_after(node.loc.operator, "\n")`
                let end = operator.end_offset();
                Some(Correction {
                    range: end..end,
                    replacement: "\n".to_owned(),
                })
            })
            .flatten();
        let (line, col) = file.line_col(location.start_offset());
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: MSG.to_owned(),
            line,
            end_line: file.last_line_of(location),
            col,
        });
    }
}

/// The operator and the right-hand side of a parser assignment with a value.
fn assignment<'pr>(node: &Node<'pr>) -> Option<(Location<'pr>, Node<'pr>)> {
    macro_rules! write {
        ($cast:ident, $operator:ident) => {{
            let n = node.$cast()?;
            Some((n.$operator(), n.value()))
        }};
    }
    match node {
        Node::LocalVariableWriteNode { .. } => write!(as_local_variable_write_node, operator_loc),
        Node::InstanceVariableWriteNode { .. } => {
            write!(as_instance_variable_write_node, operator_loc)
        }
        Node::ClassVariableWriteNode { .. } => write!(as_class_variable_write_node, operator_loc),
        Node::GlobalVariableWriteNode { .. } => write!(as_global_variable_write_node, operator_loc),
        Node::ConstantWriteNode { .. } => write!(as_constant_write_node, operator_loc),
        Node::ConstantPathWriteNode { .. } => write!(as_constant_path_write_node, operator_loc),
        Node::MultiWriteNode { .. } => write!(as_multi_write_node, operator_loc),
        Node::LocalVariableOperatorWriteNode { .. } => {
            write!(as_local_variable_operator_write_node, binary_operator_loc)
        }
        Node::InstanceVariableOperatorWriteNode { .. } => {
            write!(
                as_instance_variable_operator_write_node,
                binary_operator_loc
            )
        }
        Node::ClassVariableOperatorWriteNode { .. } => {
            write!(as_class_variable_operator_write_node, binary_operator_loc)
        }
        Node::GlobalVariableOperatorWriteNode { .. } => {
            write!(as_global_variable_operator_write_node, binary_operator_loc)
        }
        Node::ConstantOperatorWriteNode { .. } => {
            write!(as_constant_operator_write_node, binary_operator_loc)
        }
        Node::ConstantPathOperatorWriteNode { .. } => {
            write!(as_constant_path_operator_write_node, binary_operator_loc)
        }
        Node::CallOperatorWriteNode { .. } => {
            write!(as_call_operator_write_node, binary_operator_loc)
        }
        Node::IndexOperatorWriteNode { .. } => {
            write!(as_index_operator_write_node, binary_operator_loc)
        }
        Node::LocalVariableOrWriteNode { .. } => {
            write!(as_local_variable_or_write_node, operator_loc)
        }
        Node::InstanceVariableOrWriteNode { .. } => {
            write!(as_instance_variable_or_write_node, operator_loc)
        }
        Node::ClassVariableOrWriteNode { .. } => {
            write!(as_class_variable_or_write_node, operator_loc)
        }
        Node::GlobalVariableOrWriteNode { .. } => {
            write!(as_global_variable_or_write_node, operator_loc)
        }
        Node::ConstantOrWriteNode { .. } => write!(as_constant_or_write_node, operator_loc),
        Node::ConstantPathOrWriteNode { .. } => {
            write!(as_constant_path_or_write_node, operator_loc)
        }
        Node::CallOrWriteNode { .. } => write!(as_call_or_write_node, operator_loc),
        Node::IndexOrWriteNode { .. } => write!(as_index_or_write_node, operator_loc),
        Node::LocalVariableAndWriteNode { .. } => {
            write!(as_local_variable_and_write_node, operator_loc)
        }
        Node::InstanceVariableAndWriteNode { .. } => {
            write!(as_instance_variable_and_write_node, operator_loc)
        }
        Node::ClassVariableAndWriteNode { .. } => {
            write!(as_class_variable_and_write_node, operator_loc)
        }
        Node::GlobalVariableAndWriteNode { .. } => {
            write!(as_global_variable_and_write_node, operator_loc)
        }
        Node::ConstantAndWriteNode { .. } => write!(as_constant_and_write_node, operator_loc),
        Node::ConstantPathAndWriteNode { .. } => {
            write!(as_constant_path_and_write_node, operator_loc)
        }
        Node::CallAndWriteNode { .. } => write!(as_call_and_write_node, operator_loc),
        Node::IndexAndWriteNode { .. } => write!(as_index_and_write_node, operator_loc),
        Node::CallNode { .. } => {
            // `send` with `loc.operator` `=`: `a.b = 1` and `a[1] = 2`, never
            // `a&.b = 1` (`csend`, not checked upstream).
            let call = node.as_call_node()?;
            if call.is_safe_navigation() {
                return None;
            }
            let operator = call.equal_loc()?;
            let rhs = call.arguments()?.arguments().iter().last()?;
            Some((operator, rhs))
        }
        _ => None,
    }
}

/// A value of a supported type.
enum Supported<'pr> {
    /// `case`, `class`, `if`, `kwbegin` or `module`.
    Plain,
    /// A `block`, `numblock` or `itblock`, with its opening delimiter.
    Block(Location<'pr>),
}

fn supported<'pr>(rhs: &Node<'pr>) -> Option<Supported<'pr>> {
    let block = |block: Option<Node<'pr>>| {
        block
            .and_then(|b| b.as_block_node())
            .map(|b| Supported::Block(b.opening_loc()))
    };
    match rhs {
        Node::IfNode { .. } | Node::UnlessNode { .. } | Node::CaseNode { .. } => {
            Some(Supported::Plain)
        }
        Node::ClassNode { .. } | Node::ModuleNode { .. } => Some(Supported::Plain),
        // `kwbegin`: an explicit `begin ... end`.
        Node::BeginNode { .. } => rhs
            .as_begin_node()?
            .begin_keyword_loc()
            .map(|_| Supported::Plain),
        // `block`, `numblock` and `itblock`, lambdas included.
        Node::LambdaNode { .. } => Some(Supported::Block(rhs.as_lambda_node()?.opening_loc())),
        Node::CallNode { .. } => block(rhs.as_call_node()?.block()),
        Node::SuperNode { .. } => block(rhs.as_super_node()?.block()),
        Node::ForwardingSuperNode { .. } => {
            block(rhs.as_forwarding_super_node()?.block().map(|b| b.as_node()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
