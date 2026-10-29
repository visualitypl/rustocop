//! Prefer keyword parameters for literal boolean positional defaults (methods only).
use crate::{
    cops::{Cop, NodeContext},
    offense::{Offense, single_line},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct OptionalBooleanParameter;
impl Cop for OptionalBooleanParameter {
    fn name(&self) -> &'static str {
        "Style/OptionalBooleanParameter"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(def) = node.as_def_node() else {
            return;
        };
        if def.name().as_slice() == b"respond_to_missing?" {
            return;
        }
        let Some(params) = def.parameters() else {
            return;
        };
        for param in params.optionals().iter() {
            let Some(optional) = param.as_optional_parameter_node() else {
                continue;
            };
            let value = optional.value();
            if !matches!(value, Node::TrueNode { .. } | Node::FalseNode { .. }) {
                continue;
            }
            let loc = param.location();
            let (line, col) = file.line_col(loc.start_offset());
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                col,
                end_line: file.line_of(loc.end_offset().saturating_sub(1)),
                message: format!(
                    "Prefer keyword arguments for arguments with a boolean default value; use `{}: {}` instead of `{}`.",
                    single_line(optional.name().as_slice()),
                    single_line(value.location().as_slice()),
                    single_line(loc.as_slice()),
                ),
                correction: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop optional_boolean_parameter_spec.rb (MIT
    //! License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == OptionalBooleanParameter.name())
            .collect()
    }
    #[test]
    fn pinned_boolean_defaults_and_messages() {
        for (source, expected) in [
            (
                "def some_method(bar = false)\nend\n",
                vec![(17, "bar", "false", "bar = false")],
            ),
            (
                "def some_method(bar=false)\nend\n",
                vec![(17, "bar", "false", "bar=false")],
            ),
            (
                "def self.some_method(bar = false)\nend\n",
                vec![(22, "bar", "false", "bar = false")],
            ),
            (
                "def some_method(foo = true, bar = 1, baz = false, quux: true)\nend\n",
                vec![
                    (17, "foo", "true", "foo = true"),
                    (38, "baz", "false", "baz = false"),
                ],
            ),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), expected.len(), "{source}");
            for (o, (col, name, value, original)) in found.iter().zip(expected) {
                assert_eq!((o.line, o.col, o.end_line), (1, col, 1), "{source}");
                assert_eq!(
                    o.message,
                    format!(
                        "Prefer keyword arguments for arguments with a boolean default value; use `{name}: {value}` instead of `{original}`."
                    )
                );
                assert!(o.correction.is_none());
            }
        }
    }
    #[test]
    fn pinned_negative_lookalikes_and_fixed_exemption() {
        for source in [
            "def some_method(bar: false); end",
            "def some_method; end",
            "def some_method(bar = 'foo'); end",
            "def respond_to_missing?(method, include_all = false); end",
            "def self.respond_to_missing?(method, include_all = true); end",
            "proc { |flag=false| flag }",
            "->(flag=true) {flag}",
            "define_method(:f) { |flag=false| flag }",
            "def f(flag=(false)); end",
            "def f(flag=!true); end",
            "def f(flag = nil, hash = {}, flag2: true); end",
            "def f(...); g(...); end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_eq!(offenses("def other(flag=false); end").len(), 1);
    }
    #[test]
    fn endless_post_rest_channels_and_nested_methods() {
        for source in [
            "def f(flag=true,last) = [flag,last]",
            "def obj.f((a,b), flag=false, *rest, last, key:, **kw, &blk); super; end",
            "proc { def f(flag=false); end }",
            "def respond_to_missing?(name, flag=false); def f(flag=true); end; end",
        ] {
            let found = correction_offenses(&OptionalBooleanParameter, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn preserves_source_spelling_and_multiline_range() {
        let found = offenses("def f(\n  flag =\n    false\n)\nend");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 3));
        assert_eq!(
            found[0].message,
            "Prefer keyword arguments for arguments with a boolean default value; use `flag: false` instead of `flag =     false`."
        );
    }
}
