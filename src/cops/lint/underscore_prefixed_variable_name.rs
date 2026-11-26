//! `Lint/UnderscorePrefixedVariableName` — unused-by-name locals with explicit reads.

use crate::{
    cops::{
        Cop, NodeContext,
        variables::{Declaration, ScopeAnalysis},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

/// Detection only, with fixed `AllowKeywordBlockArguments=false`.
pub struct UnderscorePrefixedVariableName;
impl Cop for UnderscorePrefixedVariableName {
    fn name(&self) -> &'static str {
        "Lint/UnderscorePrefixedVariableName"
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
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
        // Reuse the pinned force projection: RHS-first multiple declarations,
        // sequential defaults, encoded name equality and no invented num/it args.
        // Raw liveness/correction and ShadowedArgument modes remain untouched.
        let analysis = ScopeAnalysis::for_underscore_variables(&program, file);
        for scope in analysis.scopes_in_exit_order() {
            for id in analysis.scope(scope).variables() {
                let variable = analysis.variable(id);
                if !variable.should_be_unused()
                    || !variable
                        .references
                        .iter()
                        .any(|r| analysis.reference(*r).is_explicit())
                {
                    continue;
                }
                let mut location = variable.declaration_range.clone();
                if let Declaration::Parameter(_) = variable.declaration {
                    // The private projection preserves Prism's actual name start
                    // for sigiled parameters. Trim optional defaults/keyword colons.
                    location.end = location.start + variable.name.len();
                }
                // Named captures deliberately retain the entire regexp receiver.
                let (line, col) = file.reference_line_col(location.start);
                out.push(Offense {
                    correction: None,
                    cop: self.name(),
                    severity: self.severity(),
                    message: "Do not use prefix `_` for a variable that is used.".into(),
                    line,
                    col,
                    end_line: file.line_of(location.end.saturating_sub(1).max(location.start)),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's underscore_prefixed_variable_name_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/UnderscorePrefixedVariableName")
            .collect()
    }
    #[test]
    fn reports_original_declaration_not_read_or_reassignment() {
        for (source, line, col, end_line) in [
            ("def some_method\n  _foo = 1\n  puts _foo\nend", 2, 3, 2),
            ("def some_method(_foo)\n puts _foo\nend", 1, 17, 1),
            ("1.times do |_foo|\n puts _foo\nend", 1, 13, 1),
            (
                "define_method(:foo) do |_foo: 'default'|\n puts _foo\nend",
                1,
                25,
                1,
            ),
            ("_foo = 1\nputs _foo", 1, 1, 1),
            ("/(?<_foo>\\w+)/ =~ 'FOO'\nputs _foo", 1, 1, 1),
            ("_foo=1\n_foo=2\nputs _foo", 1, 1, 1),
            ("def f(_foo=(\n 1\n)); _foo; end", 1, 7, 1),
            ("def f(*_foo); _foo; end", 1, 8, 1),
            ("def f(**_foo); _foo; end", 1, 9, 1),
            ("def f(&_foo); _foo; end", 1, 8, 1),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}: {found:?}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, end_line),
                "{source}"
            );
            assert_eq!(
                found[0].message,
                "Do not use prefix `_` for a variable that is used."
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn separated_parameter_names_report_the_actual_name_location() {
        for sigil in ["*", "**", "&"] {
            for gap in [" ", "\t", "\n  ", " # spacer\n  ", " # spacer\r\n  "] {
                let parameter = format!("{sigil}{gap}_foo");
                for source in [
                    format!("def f({parameter}); _foo; end"),
                    format!("xs.each {{ |{parameter}| _foo }}"),
                    format!("->({parameter}) {{ _foo }}"),
                ] {
                    let parsed = ruby_prism::parse(source.as_bytes());
                    assert_eq!(parsed.errors().count(), 0, "{source}");
                    let start = source.find("_foo").unwrap();
                    let prefix = &source[..start];
                    let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32 + 1;
                    let col = prefix.rsplit('\n').next().unwrap().len() as u32 + 1;
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}: {found:?}");
                    assert_eq!(
                        (found[0].line, found[0].col, found[0].end_line),
                        (line, col, line),
                        "{source}"
                    );
                }
            }
        }
    }

    #[test]
    fn captures_preserve_actual_send_and_encoded_symbol_distinctions() {
        for source in [
            "/(?<_foo>.)/=~(_foo)",
            "/(?<_foo>.)/=~(proc { _foo })",
            "/(?<_é>.)/=~text; _é",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        let found = offenses("/(?<_é>.)/=~text; _é=1; _é");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 19));
    }

    #[test]
    fn only_explicit_references_not_capture_flags_or_implicit_reads() {
        for keyword in ["super", "binding"] {
            for body in [keyword.to_owned(), format!("{keyword}(:something)")] {
                assert!(offenses(&format!("def some_method(*_); {body}; end")).is_empty());
            }
            for body in [format!("{keyword}; puts _"), format!("{keyword}(*_)")] {
                let found = offenses(&format!("def some_method(*_); {body}; end"));
                assert_eq!(found.len(), 1);
                assert_eq!((found[0].line, found[0].col), (1, 18));
            }
        }
        for source in [
            "def f; foo=1; puts foo; end",
            "def f; _foo=1; _foo=2; end",
            "_foo=1; 1.times { _foo=2 }",
            "xs.each { _1 }; xs.each { it }",
            "@_foo=1; @_foo",
            "$_foo=1; $_foo",
            "def _foo; end; _foo",
            "# _foo\n'_foo'",
            "def f(*, **, &); super; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
