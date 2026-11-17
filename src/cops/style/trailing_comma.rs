//! Shared fixed-policy literal separator detection. Delimiter/last-item ownership
//! comes from the main AST; the existing lazy punctuation cache excludes deferred
//! heredoc text, comments and literal contents without another parse.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub(super) fn check_literal(
    cop: &dyn Cop,
    last: Node<'_>,
    close: usize,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
    kind: &str,
) {
    let start = last.location().end_offset();
    let Some(margin) = file.bytes.get(start..close) else {
        return;
    };
    // Probe only the gap following this final item, not its AST subtree or the
    // full source. Deferred heredoc text can occupy that gap: the token cache,
    // not this whitespace probe, decides code ownership. Clean lists stay lazy.
    let Some(relative) = margin.iter().position(|&b| !super::is_ruby_whitespace(b)) else {
        return;
    };
    if margin[relative] != b',' {
        return;
    }
    let offset = start + relative;
    let Some(root) = cx.ancestors().next_back() else {
        return;
    };
    let tokens = file.punctuation(root);
    let first = tokens.partition_point(|token| token.offset < offset);
    if tokens.get(first).is_none_or(|token| token.offset != offset) {
        return;
    }
    let (line, col) = file.line_col(offset);
    out.push(Offense {
        cop: cop.name(),
        severity: cop.severity(),
        message: format!("Avoid comma after the last item of {kind}."),
        line,
        col,
        end_line: line,
        correction: cx.corrections_requested().then(|| Correction {
            range: offset..offset + 1,
            replacement: String::new(),
        }),
    });
}

#[cfg(test)]
mod tests {
    use crate::cops::{
        style::{TrailingCommaInArrayLiteral, TrailingCommaInHashLiteral},
        testing::correction_offenses,
    };

    #[test]
    fn fixed_ruby_34_binding_and_decoded_literal_shapes_are_unchanged() {
        let before = "def f(*, **); sink(*, **); end\nx = 1; sink([x:,], {x:,})\nxs.map { [_1,] }; xs.map { [it,] }\nsink([\"café#{x}\",], {a: <<~X,})\n, #{[1,]} #{ {a: x,} }\nX\nxs.map { |(a,b),| [a,b,] }\n";
        let mut found = correction_offenses(&TrailingCommaInArrayLiteral, before);
        found.extend(correction_offenses(&TrailingCommaInHashLiteral, before));
        assert_eq!(found.len(), 9);
        let applied = crate::corrections::apply(before.as_bytes(), &found).unwrap();
        let original = ruby_prism::parse(before.as_bytes());
        let corrected = ruby_prism::parse(&applied.bytes);
        assert_eq!(original.errors().count(), 0);
        assert_eq!(corrected.errors().count(), 0);
        use crate::cops::walk::{NodeVisitor, walk};
        #[derive(Default)]
        struct Shape(Vec<String>);
        impl<'pr> NodeVisitor<'pr> for Shape {
            fn enter(&mut self, node: ruby_prism::Node<'pr>) {
                self.0.push(format!("{:?}", std::mem::discriminant(&node)));
                if let Some(call) = node.as_call_node() {
                    self.0.push(format!("call:{:?}", call.name().as_slice()));
                }
                if let Some(read) = node.as_local_variable_read_node() {
                    self.0.push(format!(
                        "local:{:?}:{}",
                        read.name().as_slice(),
                        read.depth()
                    ));
                }
                if let Some(param) = node.as_required_parameter_node() {
                    self.0.push(format!("param:{:?}", param.name().as_slice()));
                }
                if let Some(string) = node.as_string_node() {
                    self.0.push(format!("string:{:?}", string.unescaped()));
                }
                if let Some(symbol) = node.as_symbol_node() {
                    self.0.push(format!("symbol:{:?}", symbol.unescaped()));
                }
            }
            fn leave(&mut self) {
                self.0.push("end".into());
            }
        }
        let mut before_shape = Shape::default();
        let mut after_shape = Shape::default();
        walk(&mut before_shape, &original.node());
        walk(&mut after_shape, &corrected.node());
        assert_eq!(before_shape.0, after_shape.0);
        let after = std::str::from_utf8(&applied.bytes).unwrap();
        assert!(after.contains("|(a,b),|"));
        assert!(after.contains("sink([x:], {x:})"));
        let probe = std::process::Command::new("ruby")
            .args(["-rprism", "-e", "exit"])
            .output();
        if !probe.is_ok_and(|out| out.status.success()) {
            eprintln!("Ruby Prism unavailable: explicit 3.4 shape comparison not executed");
            return;
        }
        let script = r#"require 'prism'
r = Prism.parse(ARGV[0], version: '3.4')
abort r.errors.inspect unless r.success?
w = ->(n) {
  info = [n.class.name]
  info << n.name if n.respond_to?(:name)
  info << n.depth if n.respond_to?(:depth)
  info << n.unescaped if n.respond_to?(:unescaped)
  info << n.compact_child_nodes.map { |c| w.call(c) }
  info
}
p w.call(r.value)
"#;
        let snapshots: Vec<_> = [before, after]
            .into_iter()
            .map(|source| {
                let out = std::process::Command::new("ruby")
                    .args(["-e", script, source])
                    .output()
                    .unwrap();
                assert!(out.status.success(), "{out:?}");
                out.stdout
            })
            .collect();
        assert_eq!(snapshots[0], snapshots[1]);
    }
}
