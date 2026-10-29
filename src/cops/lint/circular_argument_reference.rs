//! `Lint/CircularArgumentReference` — literal circular optional-argument defaults.

use std::collections::HashSet;

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct CircularArgumentReference;

impl Cop for CircularArgumentReference {
    fn name(&self) -> &'static str {
        "Lint/CircularArgumentReference"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let (name, mut value) = if let Some(arg) = node.as_optional_parameter_node() {
            (arg.name(), arg.value())
        } else if let Some(arg) = node.as_optional_keyword_parameter_node() {
            (arg.name(), arg.value())
        } else {
            return;
        };
        // Match only a literal chain of simple local assignments. In particular,
        // parentheses/begin, calls, operators and nested scopes are boundaries,
        // not transparent value-flow edges. Prism already resolves local reads;
        // there is no binding/liveness question requiring VariableForce here.
        let mut seen = HashSet::new();
        while let Some(write) = value.as_local_variable_write_node() {
            seen.insert(write.name().as_slice());
            value = write.value();
        }
        let Some(read) = value.as_local_variable_read_node() else {
            return;
        };
        if read.name().as_slice() != name.as_slice() && !seen.contains(read.name().as_slice()) {
            return;
        }
        let (line, col) = file.line_col(read.location().start_offset());
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Circular argument reference - `{}`.",
                String::from_utf8_lossy(name.as_slice())
            ),
            line,
            end_line: line,
            col,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's circular_argument_reference_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/CircularArgumentReference")
            .collect()
    }

    #[test]
    fn reports_assignment_chain_terminal_reads_including_intermediate_names() {
        for (value, col) in [
            ("msg = msg", 25),
            ("foo = msg", 25),
            ("foo = msg2 = foo", 32),
        ] {
            let source = format!("def omg_wow(msg = {value})\n  puts msg\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, col, 1)
            );
            assert_eq!(found[0].message, "Circular argument reference - `msg`.");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn ignores_all_pinned_no_offense_lookalikes() {
        for source in [
            "def omg_wow(msg = foo = self.msg)\n  puts msg\nend\n",
            "def omg_wow(msg)\n  puts msg\nend\n",
            "def omg_wow(msg = self.msg)\n  puts msg\nend\n",
            "def some_method(some_arg: nil)\n  puts some_arg\nend\n",
            "def some_method(some_arg: some_method)\n  puts some_arg\nend\n",
            "def puts_value(value: self.class.value, smile: self.smile)\n  puts value\nend\n",
            "def puts_length(length: mystring.length)\n  puts length\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn reports_direct_positional_and_keyword_reads_at_the_read() {
        for (source, expected) in [
            (
                "def omg_wow(msg = msg)\n  puts msg\nend\n",
                vec![(1, 19, "msg")],
            ),
            (
                "def some_method(some_arg: some_arg)\n  puts some_arg\nend\n",
                vec![(1, 27, "some_arg")],
            ),
            (
                "def some_method(some_arg: some_arg, other_arg: other_arg)\n  puts [some_arg, other_arg]\nend\n",
                vec![(1, 27, "some_arg"), (1, 48, "other_arg")],
            ),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), expected.len(), "{source}");
            for (o, (line, col, name)) in found.iter().zip(expected) {
                assert_eq!((o.line, o.col, o.end_line), (line, col, line));
                assert_eq!(
                    o.message,
                    format!("Circular argument reference - `{name}`.")
                );
                assert!(o.correction.is_none());
            }
        }
    }
}
