//! `Naming/BlockParameterName`: fixed minimum one, numbers allowed, no lists.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct BlockParameterName;
impl Cop for BlockParameterName {
    fn name(&self) -> &'static str {
        "Naming/BlockParameterName"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        use super::method_parameter_name::{
            destructured_first, encoded_name_length, parameter_name,
            parameter_node_text_length_with,
        };
        let Some(block) = node.as_block_parameters_node() else {
            return;
        };
        for parameter in block
            .parameters()
            .into_iter()
            .flat_map(|parameters| {
                parameters
                    .requireds()
                    .iter()
                    .chain(parameters.optionals().iter())
                    .chain(parameters.rest())
                    .chain(parameters.posts().iter())
                    .chain(parameters.keywords().iter())
                    .chain(parameters.keyword_rest())
                    .chain(parameters.block().map(|p| p.as_node()))
            })
            .chain(block.locals().iter())
        {
            let message = if let Some(target) = parameter.as_multi_target_node() {
                destructured_first(&target)
                    .filter(|first| destructured_has_uppercase(first, file))
                    .map(|_| "Only use lowercase characters for block parameter.")
            } else if let Some(name) = parameter_name(&parameter).or_else(|| {
                parameter
                    .as_block_local_variable_node()
                    .map(|p| p.name().as_slice())
            }) {
                if name == b"_" {
                    None
                } else if file.identifier_has_uppercase(name) {
                    Some("Only use lowercase characters for block parameter.")
                } else if name.iter().all(|&b| b == b'_') {
                    Some("Block parameter must be at least 1 characters long.")
                } else {
                    None
                }
            } else {
                None
            };
            let Some(message) = message else {
                continue;
            };
            let start = parameter.location().start_offset();
            let (line, col) = file.line_col(start);
            // Names and physical endpoints use the same bounded UTF8-MAC
            // composition facts as MethodParameterName, not raw scalar counts.
            let name_length = |name: &[u8]| encoded_name_length(name, file);
            let length = if let Some(first) = parameter
                .as_multi_target_node()
                .and_then(|p| destructured_first(&p))
            {
                parameter_node_text_length_with(&first, 0, name_length)
            } else {
                let name = parameter_name(&parameter)
                    .or_else(|| {
                        parameter
                            .as_block_local_variable_node()
                            .map(|p| p.name().as_slice())
                    })
                    .expect("an offending parameter has a name");
                // UncommunicativeName adds sigils only for restarg/kwrestarg,
                // not blockarg. Ranges start at the whole parameter, even if
                // whitespace/newlines separate the sigil and name.
                name_length(name)
                    + usize::from(parameter.as_rest_parameter_node().is_some())
                    + 2 * usize::from(parameter.as_keyword_rest_parameter_node().is_some())
            };
            let mut end = start;
            let mut remaining = length;
            while remaining > 0 && end < file.bytes.len() {
                // Parser's source buffer collapses CRLF before constructing
                // this synthetic range, even for destructured parameters.
                let (width, count) = if file.bytes[end..].starts_with(b"\r\n") {
                    (2, 1)
                } else {
                    file.composed_character_chunk(&file.bytes[end..])
                };
                end += width;
                remaining = remaining.saturating_sub(count);
            }
            // Parser::Source::Range#last_line includes the end position itself.
            let end_line = file.line_of(end);
            out.push(Offense {
                correction: None,
                cop: self.name(),
                severity: self.severity(),
                message: message.into(),
                line,
                end_line,
                col,
            });
        }
    }
}

/// The reference stringifies the first child AST, not its source range:
/// nested names count, but comments and later sibling subtrees do not.
pub(super) fn destructured_has_uppercase(first: &Node<'_>, file: &SourceFile) -> bool {
    use crate::cops::walk::{NodeVisitor, walk};
    struct Names<'a> {
        file: &'a SourceFile,
        uppercase: bool,
    }
    impl<'pr> NodeVisitor<'pr> for Names<'_> {
        fn enter(&mut self, node: Node<'pr>) {
            if let Some(name) = super::method_parameter_name::parameter_name(&node) {
                self.uppercase |= self.file.identifier_has_uppercase(name);
            }
        }
    }
    let mut names = Names {
        file,
        uppercase: false,
    };
    walk(&mut names, first);
    names.uppercase
}

#[cfg(test)]
mod tests {
    //! Adapted from RuboCop's naming/block_parameter_name_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Fixed MinNameLength=1 and numbers allowed.
    use crate::pipeline::lint_str;
    fn found(source: &str) -> Vec<(u32, u32, String)> {
        assert_eq!(
            crate::parser::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Naming/BlockParameterName")
            .map(|o| {
                assert!(o.correction.is_none());
                (o.line, o.col, o.message)
            })
            .collect()
    }
    #[test]
    fn mac_late_name_composition_keeps_the_endpoint_before_the_tail_disable() {
        for encoding in ["UTF8-MAC", "UTF-8-HFS"] {
            for prefix in ["rubocop", "rustocop"] {
                let source = format!(
                    "# encoding: {encoding}\nfoo {{ |((aB,\n{}\nabce\u{301}), second)| }} # {prefix}:disable Naming/BlockParameterName\n",
                    " ".repeat(26)
                );
                let offenses: Vec<_> = lint_str(&source)
                    .into_iter()
                    .filter(|o| o.cop == "Naming/BlockParameterName")
                    .collect();
                assert_eq!(offenses.len(), 1, "{source:?}");
                assert_eq!(
                    (offenses[0].line, offenses[0].col, offenses[0].end_line),
                    (2, 8, 3)
                );
                assert!(offenses[0].correction.is_none());
            }
        }
    }

    #[test]
    fn destructured_crlf_endpoint_includes_the_next_line_start() {
        for prefix in ["rubocop", "rustocop"] {
            let bare = "foo { |(aB,    \r\n second)| }\n";
            let offenses: Vec<_> = lint_str(bare)
                .into_iter()
                .filter(|o| o.cop == "Naming/BlockParameterName")
                .collect();
            assert_eq!(offenses.len(), 1);
            assert_eq!(
                (offenses[0].line, offenses[0].col, offenses[0].end_line),
                (1, 8, 2)
            );
            assert!(
                found(&bare.replace(
                    "| }",
                    &format!("| }} # {prefix}:disable Naming/BlockParameterName")
                ))
                .is_empty()
            );
        }
    }

    #[test]
    fn every_pinned_source_spec_and_encoded_boundary_endpoint_matches() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/block_parameter_ranges.json"
        ))
        .unwrap();
        let rows = fixture["ranges"].as_array().unwrap();
        assert_eq!(rows.len(), 1886);
        for row in rows {
            let hex = row["source_hex"].as_str().unwrap();
            let source: Vec<_> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            let path = std::path::Path::new("a.rb");
            let report = crate::pipeline::lint_one(
                path,
                path,
                Some(&source),
                &[&super::BlockParameterName],
                &crate::config::ResolvedConfig::default(),
            );
            assert!(report.errors.is_empty(), "{row}: {:?}", report.errors);
            assert!(report.internal_errors.is_empty(), "{row}");
            let actual: Vec<_> = report.result.offenses.into_iter().map(|o| {
                assert!(o.correction.is_none());
                serde_json::json!({"line": o.line, "col": o.col, "end_line": o.end_line, "message": o.message})
            }).collect();
            assert_eq!(serde_json::json!(actual), row["offenses"], "{row}");
        }
    }

    #[test]
    fn composed_prefix_does_not_change_raw_display_columns() {
        // Source-buffer projection is for the synthetic endpoint only. Like
        // other cops, display columns still count the original source scalars.
        for encoding in ["UTF-8", "UTF8-MAC", "UTF-8-HFS"] {
            let source = format!("# encoding: {encoding}\nfooe\u{301} {{ |aB| }}\n");
            assert_eq!(
                found(&source),
                [(
                    2,
                    10,
                    "Only use lowercase characters for block parameter.".into()
                )]
            );
        }
    }

    #[test]
    fn pinned_uppercase_and_default_adaptations() {
        for (source, cols) in [
            ("something { |number_One| do_stuff }", vec![14]),
            ("something do |y, num1, oFo|\nend", vec![24]),
        ] {
            assert_eq!(
                found(source),
                cols.into_iter()
                    .map(|col| (
                        1,
                        col,
                        "Only use lowercase characters for block parameter.".into()
                    ))
                    .collect::<Vec<_>>()
            );
        }
        for source in [
            "something do\n do_stuff\nend",
            "something { do_stuff }",
            "something { |foo, bar| do_stuff }",
            "something { |foo1, bar2, qux3| do_stuff }",
            "something { |x, _a, __b, *c, **__d| }",
            "something { |arg, foo_arg| }",
            "something { |_| }",
            "foo { _1 }; bar { it }",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn split_parameter_highlights_respect_directive_end_lines() {
        for sigil in ["*", "**", "&"] {
            let source = format!("foo {{ |{sigil}\naB|\n}}\n");
            assert_eq!(
                found(&source),
                [(
                    1,
                    8,
                    "Only use lowercase characters for block parameter.".into()
                )]
            );
            for prefix in ["rubocop", "rustocop"] {
                let suppressed = source.replace(
                    "aB|",
                    &format!("aB| # {prefix}:disable Naming/BlockParameterName"),
                );
                assert!(found(&suppressed).is_empty(), "{suppressed}");
                // A directive past the reference range must not suppress it.
                let outside = source.replace(
                    "}\n",
                    &format!("}} # {prefix}:disable Naming/BlockParameterName\n"),
                );
                assert_eq!(found(&outside).len(), 1, "{outside}");
            }
        }
        // Blockarg ranges omit the '&' length, unlike restarg/kwrestarg.
        let source = "foo { |&\n\naB| # rubocop:disable Naming/BlockParameterName\n}\n";
        assert_eq!(found(source).len(), 1);
    }
    #[test]
    fn destructured_names_exclude_comments_but_include_first_subtree_descendants() {
        for source in [
            "foo { |((a, # UPPER\n b), c)| }\n",
            "foo { |((a, # lower\n b), c)| }\n",
            "foo { |((a, b), # UPPER\n c)| }\n",
            "foo { |((a, b), cD)| }\n",
            "foo { |(a, # UPPER\n bC)| }\n",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
        for source in [
            "foo { |((a, # lower\n bC), c)| }\n",
            "foo { |((a, # UPPER\n bC), c)| }\n",
            "foo { |((a, *bC), c)| }\n",
            "foo { |(((a, bC), d), c)| }\n",
        ] {
            assert_eq!(
                found(source),
                [(
                    1,
                    8,
                    "Only use lowercase characters for block parameter.".into()
                )],
                "{source}"
            );
        }
    }
    #[test]
    fn scopes_and_argument_forms() {
        for source in [
            "def m(fooBar); end",
            "foo(fooBar: 1)",
            "foo { localVar = 1 }",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
        for source in [
            "foo { |aB| }",
            "foo { |abc; localVar| }",
            "foo { |; localVar| }",
            "->(; localVar) {}",
            "->(aB) {}",
            "foo { |aB = 1| }",
            "foo { |*aB| }",
            "foo { |aB:| }",
            "foo { |aB: 1| }",
            "foo { |**aB| }",
            "foo { |&aB| }",
            "foo { |(aB, c)| }",
            "foo { |((a, bC), d)| }",
        ] {
            assert_eq!(found(source).len(), 1, "{source}");
        }
        for source in [
            "foo { |(a, bC)| }",
            "foo { |*, **, &| }",
            "foo { |**nil| }",
            "foo { |(a, *bC)| }",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
        assert_eq!(
            found("foo { |__| }")[0],
            (
                1,
                8,
                "Block parameter must be at least 1 characters long.".into()
            )
        );
    }
}
