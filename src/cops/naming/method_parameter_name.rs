//! `Naming/MethodParameterName` — flags method parameter names that are
//! too short or hold uppercase characters.
//!
//! Matches RuboCop's semantics at its defaults: `MinNameLength: 3`,
//! `AllowNamesEndingInNumbers: true`, no `ForbiddenNames`, and
//! `AllowedNames` `as at by cc db id if in io ip of on os pp to`. Every named
//! parameter of a `def` (block and lambda parameters are not checked) is
//! judged without its leading underscores; `_` alone is accepted. The
//! offense starts at the parameter, sigil included (`*c`, `**opts`, `&b`).
//! RuboCop keeps one offense per location, so a short name with an
//! uppercase character reports only the uppercase. A destructured parameter
//! (`(a, b)`) is flagged, at its parenthesis, only for an uppercase
//! character in its first element (RuboCop stringifies that element's AST
//! as the parameter's name, excluding comments).

use ruby_prism::{MultiTargetNode, Node};

use super::block_parameter_name::destructured_has_uppercase;

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The `Naming/MethodParameterName` cop.
pub struct MethodParameterName;

/// RuboCop's default `MinNameLength`.
const MIN_NAME_LENGTH: usize = 3;

/// RuboCop's default `AllowedNames`.
const ALLOWED_NAMES: [&str; 15] = [
    "as", "at", "by", "cc", "db", "id", "if", "in", "io", "ip", "of", "on", "os", "pp", "to",
];

impl Cop for MethodParameterName {
    fn name(&self) -> &'static str {
        "Naming/MethodParameterName"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(parameters) = node.as_def_node().and_then(|def| def.parameters()) else {
            return;
        };
        let all = parameters
            .requireds()
            .iter()
            .chain(parameters.optionals().iter())
            .chain(parameters.rest())
            .chain(parameters.posts().iter())
            .chain(parameters.keywords().iter())
            .chain(parameters.keyword_rest())
            .chain(parameters.block().map(|block| block.as_node()));
        for parameter in all {
            let message = if let Some(target) = parameter.as_multi_target_node() {
                destructured_problem(&target, file)
            } else if let Some(name) = parameter_name(&parameter) {
                problem(name, file)
            } else {
                None
            };
            let Some(message) = message else {
                continue;
            };
            let start = parameter.location().start_offset();
            let (line, col) = file.line_col(start);
            let named = parameter_name(&parameter);
            let length = if let Some(first) = parameter
                .as_multi_target_node()
                .and_then(|target| destructured_first(&target))
            {
                // RuboCop stringifies the first child, in buffer characters.
                parameter_node_text_length_with(&first, 0, |name| encoded_name_length(name, file))
            } else {
                let name = named.expect("an offending parameter has a name");
                // UncommunicativeName adds rest1/kwrest2, but NOT block's &.
                // Start at the sigil even when trivia separates it from the
                // name; neither name_loc.end nor parameter.location.end is
                // the synthetic endpoint (defaults must remain excluded).
                encoded_name_length(name, file)
                    + usize::from(parameter.as_rest_parameter_node().is_some())
                    + 2 * usize::from(parameter.as_keyword_rest_parameter_node().is_some())
            };
            let mut end = start;
            let mut remaining = length;
            while remaining > 0 && end < file.bytes.len() {
                // Parser's source buffer collapses CRLF for both ordinary
                // names and synthetic first-child S-expression ranges.
                let (width, count) = if file.bytes[end..].starts_with(b"\r\n") {
                    (2, 1)
                } else {
                    file.composed_character_chunk(&file.bytes[end..])
                };
                end += width;
                remaining = remaining.saturating_sub(count);
            }
            out.push(Offense {
                correction: None,
                cop: self.name(),
                severity: self.severity(),
                message: message.to_string(),
                line,
                // Parser::Source::Range#last_line uses the end position
                // itself, even at the start of a line. Retain the EOF clamp.
                end_line: file.line_of(end),
                col,
            });
        }
    }
}

/// The name of a named parameter; `None` for anonymous ones (`*`, `**`,
/// `&`), `...`, `**nil` and destructuring (see [`destructured_problem`]).
pub(super) fn parameter_name<'pr>(parameter: &Node<'pr>) -> Option<&'pr [u8]> {
    let name = match parameter {
        Node::RequiredParameterNode { .. } => parameter.as_required_parameter_node()?.name(),
        Node::OptionalParameterNode { .. } => parameter.as_optional_parameter_node()?.name(),
        Node::RestParameterNode { .. } => parameter.as_rest_parameter_node()?.name()?,
        Node::RequiredKeywordParameterNode { .. } => {
            parameter.as_required_keyword_parameter_node()?.name()
        }
        Node::OptionalKeywordParameterNode { .. } => {
            parameter.as_optional_keyword_parameter_node()?.name()
        }
        Node::KeywordRestParameterNode { .. } => {
            parameter.as_keyword_rest_parameter_node()?.name()?
        }
        Node::BlockParameterNode { .. } => parameter.as_block_parameter_node()?.name()?,
        _ => return None,
    };
    Some(name.as_slice())
}

/// What RuboCop finds wrong with a destructured parameter. It takes the
/// parameter's first child node as its name, so the "name" is that node's
/// text (`(arg :aB)`, `(restarg :aB)`, or a nested `(mlhs …)` with every
/// name in it): it is never short or allowed, but holds an uppercase
/// character exactly when one of the names in the first element does.
fn destructured_problem(target: &MultiTargetNode<'_>, file: &SourceFile) -> Option<&'static str> {
    let first = destructured_first(target)?;
    destructured_has_uppercase(&first, file)
        .then_some("Only use lowercase characters for method parameter.")
}

pub(super) fn destructured_first<'pr>(target: &MultiTargetNode<'pr>) -> Option<Node<'pr>> {
    target
        .lefts()
        .iter()
        .next()
        .or_else(|| target.rest())
        .or_else(|| target.rights().iter().next())
}

/// Character length of RuboCop's parameter-node S-expression: `(arg :aB)`,
/// `(restarg :aB)`, or `(mlhs\n  (arg :a)\n  (arg :bC))`. Nested children
/// are indented two spaces per level. Only these nodes occur in destructuring.
/// Each caller supplies its source-buffer character count.
pub(super) fn parameter_node_text_length_with<F: Fn(&[u8]) -> usize + Copy>(
    node: &Node<'_>,
    depth: usize,
    name_length: F,
) -> usize {
    if let Some(parameter) = node.as_required_parameter_node() {
        return 7 + name_length(parameter.name().as_slice());
    }
    if let Some(splat) = node.as_splat_node() {
        return splat.expression().map_or(9, |parameter| {
            // `(restarg :name)` has four more characters than `(arg :name)`.
            4 + parameter_node_text_length_with(&parameter, depth, name_length)
        });
    }
    let target = node
        .as_multi_target_node()
        .expect("destructured parameter must be an arg, restarg or mlhs");
    6 + target
        .lefts()
        .iter()
        .chain(target.rest())
        .chain(target.rights().iter())
        .map(|child| {
            1 + 2 * (depth + 1) + parameter_node_text_length_with(&child, depth + 1, name_length)
        })
        .sum::<usize>()
}

/// Count projected characters without transcoding legacy identifier bytes.
/// UTF8-MAC can compose multiple raw scalars into one buffer character.
pub(super) fn encoded_name_length(mut name: &[u8], file: &SourceFile) -> usize {
    let mut count = 0;
    while !name.is_empty() {
        let (width, characters) = file.composed_character_chunk(name);
        name = &name[width..];
        count += characters;
    }
    count
}

/// What is wrong with a parameter name, as RuboCop's first offense for it.
fn problem(full_name: &[u8], file: &SourceFile) -> Option<&'static str> {
    if full_name == b"_" {
        return None;
    }
    // A leading underscore marks the parameter unused; judge the rest.
    let name = &full_name[full_name.iter().take_while(|&&byte| byte == b'_').count()..];
    if ALLOWED_NAMES
        .iter()
        .any(|allowed| allowed.as_bytes() == name)
    {
        None
    } else if file.identifier_has_uppercase(name) {
        Some("Only use lowercase characters for method parameter.")
    } else if encoded_name_length(name, file) < MIN_NAME_LENGTH {
        Some("Method parameter must be at least 3 characters long.")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/naming/method_parameter_name_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for the default
    //! configuration. The spec runs with `AllowNamesEndingInNumbers: false`;
    //! at the default `true`, its names ending in a number are accepted.

    use crate::pipeline::lint_str;

    const CASE: &str = "Only use lowercase characters for method parameter.";
    const LENGTH: &str = "Method parameter must be at least 3 characters long.";

    fn found(source: &str) -> Vec<(u32, u32, &'static str)> {
        let parsed = ruby_prism::parse(source.as_bytes());
        assert!(
            parsed.errors().next().is_none(),
            "fixture must parse: {source:?}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Naming/MethodParameterName")
            .map(|o| {
                let message = match o.message.as_str() {
                    CASE => CASE,
                    LENGTH => LENGTH,
                    other => panic!("unexpected message {other:?}"),
                };
                (o.line, o.col, message)
            })
            .collect()
    }

    #[test]
    fn every_pinned_encoded_commented_and_split_sigil_reference_endpoint_matches() {
        use std::path::Path;
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/method_parameter_encoding.json"
        ))
        .unwrap();
        let comments: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/method_parameter_comments.json"
        ))
        .unwrap();
        let ranges: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/method_parameter_ranges.json"
        ))
        .unwrap();
        let crlf: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/method_parameter_crlf.json"
        ))
        .unwrap();
        for row in fixture["pinned"]
            .as_array()
            .unwrap()
            .iter()
            .chain(fixture["encoded"].as_array().unwrap())
            .chain(comments["comments"].as_array().unwrap())
            .chain(comments["source"].as_array().unwrap())
            .chain(ranges["ranges"].as_array().unwrap())
            .chain(crlf["crlf"].as_array().unwrap())
        {
            let source = row["source"]
                .as_str()
                .map(|s| s.as_bytes().to_vec())
                .unwrap_or_else(|| {
                    let hex = row["source_hex"].as_str().unwrap();
                    (0..hex.len())
                        .step_by(2)
                        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                        .collect()
                });
            let path = Path::new("a.rb");
            let report = crate::pipeline::lint_one(
                path,
                path,
                Some(&source),
                &[&super::MethodParameterName],
                &crate::config::ResolvedConfig::default(),
            );
            assert!(report.errors.is_empty(), "{row}: {:?}", report.errors);
            assert!(report.internal_errors.is_empty(), "{row}");
            let actual: Vec<_> = report.result.offenses.into_iter().map(|o| {
                assert!(o.correction.is_none());
                serde_json::json!({"line": o.line, "col": o.col, "end_line": o.end_line, "message": o.message})
            }).collect();
            assert_eq!(serde_json::json!(actual), row["offenses"], "{row}");
            if let Some(source) = row["source"].as_str() {
                // Preserve the reference cop's standard full-registry test seam.
                let expected: Vec<_> = actual
                    .iter()
                    .map(|o| {
                        (
                            o["line"].as_u64().unwrap() as u32,
                            o["col"].as_u64().unwrap() as u32,
                            o["message"].as_str().unwrap(),
                        )
                    })
                    .collect();
                assert_eq!(found(source), expected, "{row}");
            }
        }
    }

    #[test]
    fn split_sigils_use_synthetic_name_ranges_not_name_locations() {
        for sigil in ["*", "**", "&"] {
            let source = format!("def foo({sigil}\naBc); end\n");
            assert_eq!(found(&source), [(1, 9, CASE)], "{source:?}");
            let offense = lint_str(&source)
                .into_iter()
                .find(|o| o.cop == "Naming/MethodParameterName")
                .unwrap();
            assert_eq!(offense.end_line, 2, "{source:?}");
            for prefix in ["rubocop", "rustocop"] {
                assert!(
                    found(&format!(
                        "def foo({sigil}\naBc); end # {prefix}:disable Naming/MethodParameterName\n"
                    ))
                    .is_empty()
                );
            }
        }
    }

    #[test]
    fn destructured_crlf_is_one_source_buffer_character() {
        let source = "def foo((aB,    \r\n second)); end\n";
        assert_eq!(found(source), [(1, 9, CASE)]);
        let offense = lint_str(source)
            .into_iter()
            .find(|o| o.cop == "Naming/MethodParameterName")
            .unwrap();
        assert_eq!(offense.end_line, 2);
        assert!(offense.correction.is_none());
        for prefix in ["rubocop", "rustocop"] {
            assert!(
                found(&format!(
                    "def foo((aB,    \r\n second)); end # {prefix}:disable Naming/MethodParameterName\n"
                ))
                .is_empty()
            );
        }
    }

    #[test]
    fn flags_short_parameters() {
        assert_eq!(
            found("def something(ab)\n  do_stuff\nend\n"),
            [(1, 15, LENGTH)]
        );
        assert_eq!(
            found("def something(_a, __b, *c, **__d)\n  do_stuff\nend\n"),
            [
                (1, 15, LENGTH),
                (1, 19, LENGTH),
                (1, 24, LENGTH),
                (1, 28, LENGTH)
            ]
        );
    }

    #[test]
    fn flags_uppercase_parameters() {
        assert_eq!(
            found("def something(number_One)\n  do_stuff\nend\n"),
            [(1, 15, CASE)]
        );
        assert_eq!(
            found("def something(fooBar:)\n  do_stuff\nend\n"),
            [(1, 15, CASE)]
        );
    }

    #[test]
    fn flags_several_parameters_of_one_definition() {
        assert_eq!(
            found("def self.something(y, num1, oFo)\n  do_stuff\nend\n"),
            [(1, 20, LENGTH), (1, 29, CASE)]
        );
    }

    #[test]
    fn flags_every_kind_of_parameter_at_its_start() {
        assert_eq!(
            found("def m(a, b = 1, *c, d, e:, f: 2, **g, &h); end\n"),
            [
                (1, 7, LENGTH),
                (1, 10, LENGTH),
                (1, 17, LENGTH),
                (1, 21, LENGTH),
                (1, 24, LENGTH),
                (1, 28, LENGTH),
                (1, 34, LENGTH),
                (1, 39, LENGTH),
            ]
        );
        assert_eq!(found("def m(__); end\n"), [(1, 7, LENGTH)]);
        assert_eq!(found("def obj.m(größE); end\n"), [(1, 11, CASE)]);
    }

    #[test]
    fn reports_one_offense_per_parameter() {
        // Both too short and uppercase: RuboCop keeps the first offense at a
        // location, the uppercase one.
        assert_eq!(found("def m(aB); end\n"), [(1, 7, CASE)]);
        assert_eq!(found("def m(_aB); end\n"), [(1, 7, CASE)]);
    }

    #[test]
    fn nested_destructuring_checks_ast_names_not_comments() {
        for source in [
            "def foo(((abc, # UPPER\n defg), second)); end\n",
            "def foo(((abc, # lower\n defg), second)); end\n",
            "def foo(((abc, # UPPER\n *rest), second)); end\n",
            "def foo((((abc, # UPPER\n defg), last), second)); end\n",
            "def foo(((abc, defg), # UPPER\n second)); end\n",
            "def foo(((abc, defg), secondUpper)); end\n",
            "def foo((abc, # UPPER\n defUpper)); end\n",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
        for source in [
            "def foo(((abc, # lower\n defUpper), second)); end\n",
            "def foo(((abc, # UPPER\n defUpper), second)); end\n",
            "def foo(((abc, # lower\n *restUpper), second)); end\n",
            "def foo((((abc, # lower\n defUpper), last), second)); end\n",
        ] {
            assert_eq!(found(source), [(1, 9, CASE)], "{source}");
        }
    }

    #[test]
    fn flags_destructured_parameter_by_its_first_element() {
        // RuboCop names a destructured parameter by its first element's
        // node text (`(arg :aB)`), so uppercase there is reported at the
        // parenthesis; the length check cannot fire on that text.
        assert_eq!(found("def m((aB, c)); end\n"), [(1, 7, CASE)]);
        assert_eq!(found("def m((*aB, c)); end\n"), [(1, 7, CASE)]);
        assert_eq!(found("def m(((aB, c), d)); end\n"), [(1, 7, CASE)]);
        assert_eq!(found("def m(((a, bC), d)); end\n"), [(1, 7, CASE)]);
        assert_eq!(found("def m(((a, *bC), d)); end\n"), [(1, 7, CASE)]);
        assert_eq!(found("def m((_aB, c)); end\n"), [(1, 7, CASE)]);
        assert_eq!(found("def m(abc, (xY, c)); end\n"), [(1, 12, CASE)]);
        assert_eq!(found("def self.m((aB, c)); end\n"), [(1, 12, CASE)]);
        assert_eq!(found("def m((aÉ, c)); end\n"), [(1, 7, CASE)]);
    }

    #[test]
    fn destructured_highlight_uses_first_child_node_text_length() {
        // Checked against RuboCop 1.92: the range length is the first
        // child's S-expression length, not the whole destructuring.
        for (source, end_line) in [
            ("def foo((aB,\n  second))\nend\n", 2),
            ("def foo((aB,       \n  second))\nend\n", 1),
            ("def foo((aB,    \n  second))\nend\n", 2),
            ("def foo((*aB,\n  second))\nend\n", 2),
            ("def foo(((a, bC),\n  second))\n  work\nend\n", 4),
            ("def foo(((aÉ, bC),\n  second))\n  work\nend\n", 4),
            ("def foo(((a, *bC),\n  second))\n  work\nend\n", 5),
            (
                "def foo((((a,bC),*rest,last),\n  second))\n  work\nend\n",
                5,
            ),
            ("def foo((aB,\n  second,\n  third))\nend\n", 2),
        ] {
            assert_eq!(found(source), [(1, 9, CASE)], "{source:?}");
            let offense = lint_str(source)
                .into_iter()
                .find(|o| o.cop == "Naming/MethodParameterName")
                .unwrap();
            assert_eq!(offense.end_line, end_line, "{source:?}");
        }
    }

    #[test]
    fn directives_overlap_destructured_but_not_default_value_highlights() {
        assert!(
            found("def foo((aB,\n  second)) # rubocop:disable Naming/MethodParameterName\nend\n")
                .is_empty()
        );
        assert_eq!(
            found(
                "def foo((aB,       \n  second)) # rubocop:disable Naming/MethodParameterName\nend\n"
            ),
            [(1, 9, CASE)]
        );
        assert_eq!(
            found(
                "def foo((aB,\n  second,\n  third)) # rubocop:disable Naming/MethodParameterName\nend\n"
            ),
            [(1, 9, CASE)]
        );
        assert_eq!(
            found(
                "def foo(aB = [\n  1\n], cD: [\n  2\n]) # rubocop:disable Naming/MethodParameterName\nend\n"
            ),
            [(1, 9, CASE), (3, 4, CASE)]
        );
    }

    #[test]
    fn accepts_rubocop_spec_no_offenses() {
        for source in [
            "def foo(...); end\n",
            "def something\n  do_stuff\nend\n",
            "def something(foo, bar)\n  do_stuff\nend\n",
            "def self.something(foo, bar)\n  do_stuff\nend\n",
            "def self.something(foo = Pwd.dir, bar = 1)\n  do_stuff\nend\n",
            "def self.something(foo: Pwd.dir, bar: 1)\n  do_stuff\nend\n",
            "def qux(*)\n  stuff!\nend\n",
            "def qux(**)\n  stuff!\nend\n",
            "def quux(_)\n  do_stuff\nend\n",
            "def baz(arg)\n  arg.do_things\nend\n",
            "def baz(foo_parameter)\n  foo_parameter.do_things\nend\n",
            // Names ending in numbers are allowed by default.
            "def something(foo1, bar)\n  do_stuff\nend\n",
            "def self.something(foo, bar1)\n  do_stuff\nend\n",
            "def self.something(foo1 = Pwd.dir)\n  do_stuff\nend\n",
            "def something(foo1, bar2, qux3)\n  do_stuff\nend\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn accepts_default_allowed_names() {
        for source in [
            "def m(as, at, by, cc, db, id, io, ip, of, on, os, pp, to); end\n",
            // `if` and `in` are keywords, so only their prefixed forms parse.
            "def m(_if, __in); end\n",
            "def m(_id, __io, *at, **of, &to); end\n",
            "def m(id: 1, to:); end\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "foo { |a, b| a + b }\n",
            "foo do |x, yZ| end\n",
            "->(a, bC) { a }\n",
            "def m(*, **, &); end\n",
            "def m(**nil); end\n",
            "def m(abc, *def_rest, &blk); end\n",
            // RuboCop reads a destructured parameter as one odd name.
            "def m(abc, (b, cD)); end\n",
            "def m((a, *bC)); end\n",
            "def m((a, c)); end\n",
            "def m((*, c)); end\n",
            "def m((x)); end\n",
            "def m; ab = 1; end\n",
            "m(ab, cD)\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }
}
