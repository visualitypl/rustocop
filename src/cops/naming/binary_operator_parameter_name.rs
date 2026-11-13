//! `Naming/BinaryOperatorParameterName` — binary operators take `other`.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct BinaryOperatorParameterName;
impl Cop for BinaryOperatorParameterName {
    fn name(&self) -> &'static str {
        "Naming/BinaryOperatorParameterName"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_def_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(def) = node.as_def_node() else {
            return;
        };
        if def.receiver().is_some() {
            return;
        }
        let op = def.name().as_slice();
        if !matches!(
            op,
            b"+" | b"-"
                | b"*"
                | b"/"
                | b"%"
                | b"**"
                | b"=="
                | b"!="
                | b"!~"
                | b"<"
                | b">"
                | b"<="
                | b">="
                | b"<=>"
                | b"&"
                | b"|"
                | b"^"
                | b">>"
                | b"!"
                | b"~"
                | b"eql?"
                | b"equal?"
        ) {
            return;
        }
        let Some(params) = def.parameters() else {
            return;
        };
        if params.requireds().len() != 1
            || !params.optionals().is_empty()
            || params.rest().is_some()
            || !params.posts().is_empty()
            || !params.keywords().is_empty()
            || params.keyword_rest().is_some()
            || params.block().is_some()
        {
            return;
        }
        let Some(arg) = params
            .requireds()
            .iter()
            .next()
            .and_then(|n| n.as_required_parameter_node())
        else {
            return;
        };
        if matches!(arg.name().as_slice(), b"other" | b"_other") {
            return;
        }
        let (line, col) = file.line_col(arg.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "When defining the `{}` operator, name its argument `other`.",
                String::from_utf8_lossy(op)
            ),
            line,
            col,
            end_line: line,
            correction: None,
        });
        super::local_rename::attach(out, cx, file, arg.location().start_offset(), "other");
    }
}

#[cfg(test)]
mod tests {
    //! Ported RuboCop examples (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::pipeline::lint_str;
    #[test]
    fn operators_required_sole_argument_and_locations() {
        for op in [
            "+", "-", "*", "/", "%", "**", "==", "!=", "<", ">", "<=", ">=", "<=>", "&", "|", "^",
            ">>", "!~", "!", "~", "eql?", "equal?",
        ] {
            let source = format!("def {op}(arg); arg; end\n");
            let found: Vec<_> = lint_str(&source)
                .into_iter()
                .filter(|o| o.cop == "Naming/BinaryOperatorParameterName")
                .collect();
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 6 + op.len() as u32));
            assert_eq!(
                found[0].message,
                format!("When defining the `{op}` operator, name its argument `other`.")
            );
        }
    }
    #[test]
    fn canonical_atomic_renames_resolve_captures_shadowing_writes_and_shorthand() {
        use crate::cops::testing::assert_correction;
        assert_correction(
            &super::BinaryOperatorParameterName,
            "def +(arg)\n result = arg\n xs.each { |arg| result += arg }\n xs.each { |;arg| arg = 2 }\n f { |value=arg| arg += value }\n {arg:}\nend\n",
            "def +(other)\n result = other\n xs.each { |arg| result += arg }\n xs.each { |;arg| arg = 2 }\n f { |value=other| other += value }\n {arg: other}\nend\n",
        );
    }
    #[test]
    fn uncertain_binding_and_metadata_keep_findings_without_edits() {
        for body in [
            "other = 1; arg + other",
            "f { |other| arg }",
            "f { |other=arg| arg }",
            "other; arg",
            "binding; arg",
            "local_variables; arg",
            "method(:+).parameters; arg",
            "eval('arg')",
            "send(:local_variables)",
            "send(selector)",
            "alias snapshot binding; snapshot",
            "f { |;other| arg }",
            "f { |other: arg| arg }",
            "f { |(other, value)| arg }",
            "f { |other| arg }; arg",
            "case value; in {other:}; arg; end",
            "arg => [arg]; arg",
            "/(?<arg>.)/ =~ 'a'; arg",
        ] {
            let s = format!("def +(arg); {body}; end");
            let found =
                crate::cops::testing::correction_offenses(&super::BinaryOperatorParameterName, &s);
            assert_eq!(found.len(), 1, "{s}");
            assert!(found[0].correction.is_none(), "{s}");
        }
    }
    #[test]
    fn excluded_methods_and_parameter_shapes() {
        for source in [
            "def +(other); end",
            "def <=>(_other); end",
            "def [](index); end",
            "def []=(index,value); end",
            "def <<(cop); end",
            "def ===(string); end",
            "def =~(regexp); end",
            "def `(cmd); end",
            "def -@; end",
            "def +; end",
            "def *(a,b); end",
            "def +(arg=1); end",
            "def +(*arg); end",
            "def +(arg:); end",
            "def +(arg, &block); end",
            "def self.+(arg); end",
            "def ｄｏ＿ｓｏｍｅｔｈｉｎｇ(string); end",
        ] {
            assert!(
                !lint_str(source)
                    .iter()
                    .any(|o| o.cop == "Naming/BinaryOperatorParameterName"),
                "{source}"
            );
        }
    }
}
