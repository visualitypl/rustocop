//! `Security/MarshalLoad` — flags `Marshal.load` and `Marshal.restore`,
//! which can execute code when the data comes from an untrusted source.
//!
//! Matches RuboCop's semantics: a plain call (not `&.`) on `Marshal` or
//! `::Marshal` with one or two arguments, unless the first argument is a
//! `Marshal.dump(...)` call (the deep-copy idiom). The offense is reported at
//! the method name.

use ruby_prism::{CallNode, Node};

use crate::{
    cops::{
        Cop, NodeContext,
        ast::{call_arguments, is_global_const},
    },
    offense::Offense,
    source::SourceFile,
};

/// The `Security/MarshalLoad` cop.
pub struct MarshalLoad;

impl Cop for MarshalLoad {
    fn name(&self) -> &'static str {
        "Security/MarshalLoad"
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
        let method = call.name().as_slice();
        if !matches!(method, b"load" | b"restore") || !is_marshal_call(&call) {
            return;
        }
        let arguments = call_arguments(&call);
        let deep_copy = match arguments.as_slice() {
            [first] | [first, _] => first
                .as_call_node()
                .is_some_and(|inner| inner.name().as_slice() == b"dump" && is_marshal_call(&inner)),
            _ => return,
        };
        if deep_copy {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: format!("Avoid using `Marshal.{}`.", String::from_utf8_lossy(method)),
            line,
            end_line: line,
            col,
        });
    }
}

/// A plain call (not `&.`) on `Marshal` or `::Marshal`.
fn is_marshal_call(call: &CallNode<'_>) -> bool {
    !call.is_safe_navigation()
        && call
            .receiver()
            .is_some_and(|receiver| is_global_const(&receiver, &["Marshal"]))
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/security/marshal_load_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use crate::pipeline::lint_str;

    fn found(source: &str) -> Vec<(u32, u32, String)> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Security/MarshalLoad")
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    fn at(line: u32, col: u32, method: &str) -> (u32, u32, String) {
        (line, col, format!("Avoid using `Marshal.{method}`."))
    }

    #[test]
    fn flags_marshal_load() {
        assert_eq!(
            found("Marshal.load('{}')\n::Marshal.load('{}')\n"),
            [at(1, 9, "load"), at(2, 11, "load")]
        );
    }

    #[test]
    fn flags_marshal_restore() {
        assert_eq!(
            found("Marshal.restore('{}')\n::Marshal.restore('{}')\n"),
            [at(1, 9, "restore"), at(2, 11, "restore")]
        );
    }

    #[test]
    fn flags_the_two_argument_form_with_a_proc() {
        assert_eq!(
            found("Marshal.load(data, proc)\nMarshal.restore(data, proc)\n"),
            [at(1, 9, "load"), at(2, 9, "restore")]
        );
    }

    #[test]
    fn flags_keyword_splat_and_block_forms() {
        assert_eq!(
            found("x = Marshal.load(data, freeze: true)\n"),
            [at(1, 13, "load")]
        );
        assert_eq!(found("Marshal.load(*args)\n"), [at(1, 9, "load")]);
        assert_eq!(found("Marshal.load(io) { |o| o }\n"), [at(1, 9, "load")]);
        // As in RuboCop's AST, a block argument counts as an argument.
        assert_eq!(found("Marshal.load(io, &blk)\n"), [at(1, 9, "load")]);
        assert_eq!(
            found("Marshal\n  .load(data)\n"),
            [at(2, 4, "load")],
            "reported at the method name"
        );
    }

    #[test]
    fn only_a_plain_marshal_dump_call_is_the_deep_copy_idiom() {
        assert_eq!(
            found("Marshal.load(Marshal&.dump(obj))\n"),
            [at(1, 9, "load")]
        );
        assert_eq!(
            found("Marshal.load(Foo::Marshal.dump(obj))\n"),
            [at(1, 9, "load")]
        );
        assert_eq!(
            found("Marshal.load(Marshal.dup(obj))\n"),
            [at(1, 9, "load")]
        );
    }

    #[test]
    fn accepts_rubocop_spec_no_offenses() {
        for source in [
            "Marshal.dump({})\n::Marshal.dump({})\n",
            "SomeNamespace::Marshal.load('')\nSomeNamespace::Marshal.restore('')\n\
             SomeNamespace::Marshal.dump('')\n::SomeNamespace::Marshal.load('')\n\
             ::SomeNamespace::Marshal.restore('')\n::SomeNamespace::Marshal.dump('')\n",
            "Marshal.load(Marshal.dump({}))\nMarshal.restore(Marshal.dump({}))\n",
            "Marshal.load(Marshal.dump({}), proc)\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "Marshal.load\n",
            "Marshal.load(a, b, c)\n",
            "Marshal&.load(data)\n",
            "marshal.load(data)\n",
            "Marshal.loads(data)\n",
            "load(data)\n",
            "YAML.load(data)\n",
            "Marshal.load(::Marshal.dump(obj))\n",
            "Marshal.load(Marshal.dump(obj), &blk)\n",
            "Marshal.load(io, proc, &blk)\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }
}
