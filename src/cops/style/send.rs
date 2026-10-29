//! `Style/Send` — prefer unambiguous reflective selectors (opt-in).
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct Send;
impl Cop for Send {
    fn name(&self) -> &'static str {
        "Style/Send"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.name().as_slice() != b"send" || ast::call_arguments(&call).is_empty() {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Prefer `Object#__send__` or `Object#public_send` to `send`.".into(),
            correction: None,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/send_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::correction_offenses;
    #[test]
    fn pinned_calls_report_only_the_selector_without_corrections() {
        for (source, col) in [
            ("Object.send(foo)", 8),
            ("Object&.send(foo)", 9),
            ("send(foo)", 1),
            ("x.send(*args)", 3),
            ("send(&block)", 1),
        ] {
            let found = correction_offenses(&Send, source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col));
            assert_eq!(
                found[0].message,
                "Prefer `Object#__send__` or `Object#public_send` to `send`."
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn pinned_no_argument_and_other_selector_lookalikes() {
        for source in [
            "Object.send",
            "send",
            "send()",
            "send { foo }",
            "Object.__send__(foo)",
            "Object.__send__",
            "__send__(foo)",
            "__send__",
            "Object.public_send(foo)",
            "Object.public_send",
            "public_send(foo)",
            "public_send",
            "send = 1; send",
            "def send(foo); end",
        ] {
            assert!(correction_offenses(&Send, source).is_empty(), "{source}");
        }
    }
}
