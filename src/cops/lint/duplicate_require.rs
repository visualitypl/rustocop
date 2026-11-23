//! `Lint/DuplicateRequire` — duplicate requires under the same immediate AST parent.
//! Unsafe upstream whole-line deletions are deliberately not exposed.

use std::collections::HashMap;

use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct DuplicateRequire;

impl Cop for DuplicateRequire {
    fn name(&self) -> &'static str {
        "Lint/DuplicateRequire"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_program_node().is_none() {
            return;
        }
        // Reuse genuine VariableForce lexical facts once for the file, only
        // where source-buffer composition can change local/vcall AST kinds.
        // The ordinary parse/bindings and correction fingerprints stay intact.
        let program = node.as_program_node().unwrap();
        let ancestors = [program.as_node()];
        let locals = file
            .match_pattern_composes_identifiers()
            .then(|| {
                super::non_atomic_file_operation::projected_local_spans(
                    &program.statements().as_node(),
                    &NodeContext {
                        ancestors: &ancestors,
                        ..*cx
                    },
                    file,
                )
            })
            .flatten();
        // One file-local traversal, never state shared across investigations.
        let mut requires = Requires {
            file,
            locals: locals.as_ref(),
            candidates: Vec::new(),
            stack: Vec::new(),
            next: 1,
            seen: HashMap::new(),
        };
        walk(&mut requires, node);
        out.extend(
            requires
                .candidates
                .into_iter()
                .filter_map(|(offense, duplicate)| duplicate.then_some(offense))
                .map(|mut offense| {
                    if !cx.corrections_requested() {
                        offense.correction = None;
                    }
                    offense
                }),
        );
    }
}

struct Frame<'pr> {
    node: Node<'pr>,
    id: usize,
    order: u8,
}
type FirstByArgument<'pr> = HashMap<(Vec<u8>, ast::Fingerprint<'pr>), (u8, usize)>;

struct Requires<'a, 'pr> {
    file: &'a SourceFile,
    locals: Option<&'a std::collections::HashSet<(usize, usize)>>,
    candidates: Vec<(Offense, bool)>,
    stack: Vec<Frame<'pr>>,
    next: usize,
    seen: HashMap<usize, FirstByArgument<'pr>>,
}

impl<'pr> NodeVisitor<'pr> for Requires<'_, 'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        let id = self.next * 4;
        self.next += 1;
        let parent = self.stack.last().map_or(0, |f| {
            // Parser inserts an Array for the exception list; it is not the
            // resbody parent of the handler's single-statement body.
            let exception = f.node.as_rescue_node().is_some_and(|r| {
                let exceptions = r.exceptions();
                exceptions
                    .first()
                    .zip(exceptions.last())
                    .is_some_and(|(first, last)| {
                        node.location().start_offset() >= first.location().start_offset()
                            && node.location().end_offset() <= last.location().end_offset()
                    })
            });
            if exception {
                return f.id + 1;
            }
            if let Some(begin) = f.node.as_begin_node() {
                // kwbegin / ensure / rescue are separate parser nodes even
                // though Prism folds their protected region into BeginNode.
                if node.as_ensure_node().is_some() {
                    return f.id + 2;
                }
                if node.as_statements_node().is_some() || node.as_else_node().is_some() {
                    if begin.rescue_clause().is_some() {
                        return f.id + 1;
                    }
                    if begin.ensure_clause().is_some() {
                        return f.id + 2;
                    }
                }
            }
            if let Some(rescue) = f.node.as_rescue_modifier_node()
                && rescue.rescue_expression().location().start_offset()
                    == node.location().start_offset()
            {
                return f.id + 1;
            }
            let value = match &f.node {
                Node::CallAndWriteNode { .. } => {
                    Some(f.node.as_call_and_write_node().unwrap().value())
                }
                Node::CallOrWriteNode { .. } => {
                    Some(f.node.as_call_or_write_node().unwrap().value())
                }
                Node::CallOperatorWriteNode { .. } => {
                    Some(f.node.as_call_operator_write_node().unwrap().value())
                }
                Node::IndexAndWriteNode { .. } => {
                    Some(f.node.as_index_and_write_node().unwrap().value())
                }
                Node::IndexOrWriteNode { .. } => {
                    Some(f.node.as_index_or_write_node().unwrap().value())
                }
                Node::IndexOperatorWriteNode { .. } => {
                    Some(f.node.as_index_operator_write_node().unwrap().value())
                }
                _ => None,
            };
            if value.is_some_and(|v| node.location().end_offset() <= v.location().start_offset()) {
                return f.id + 1;
            }
            f.id
        });
        let transparent = (node.as_parentheses_node().is_some()
            && self.stack.last().is_some_and(|f| {
                f.node
                    .as_def_node()
                    .and_then(|d| d.receiver())
                    .is_some_and(|r| r.location().start_offset() == node.location().start_offset())
            }))
            || matches!(
                node,
                Node::ProgramNode { .. } | Node::ArgumentsNode { .. } | Node::ElseNode { .. }
            )
            || node.as_statements_node().is_some_and(|s| {
                s.body().len() == 1
                    || self.stack.last().is_some_and(|f| {
                        f.node.as_begin_node().is_some_and(|b| {
                            b.begin_keyword_loc().is_some()
                                && b.rescue_clause().is_none()
                                && b.ensure_clause().is_none()
                        })
                    })
            })
            || node.as_begin_node().is_some_and(|b| {
                b.begin_keyword_loc().is_none()
                    && b.rescue_clause().is_none()
                    && b.ensure_clause().is_none()
            });
        // RuboCop visits truth-value slots: an unless's else precedes its
        // source-earlier body. All other immediate siblings keep walk order.
        let order = self.stack.last().map_or(0, |f| {
            if f.node.as_unless_node().is_some() {
                if node.as_else_node().is_some() {
                    1
                } else if node.as_statements_node().is_some() {
                    2
                } else {
                    0
                }
            } else {
                f.order
            }
        });
        let semantic_id = if transparent || node.as_ensure_node().is_some() {
            parent
        } else if node.as_block_node().is_some() {
            parent + 1
        } else {
            id
        };
        if let Some(call) = node.as_call_node() {
            let method = call.name().as_slice();
            let args = ast::call_arguments(&call);
            if !call.is_safe_navigation()
                && matches!(method, b"require" | b"require_relative")
                && call
                    .receiver()
                    .is_none_or(|r| ast::short_const_name(&r).as_deref() == Some("Kernel"))
                && let [arg] = args.as_slice()
            {
                let parent = if call.block().is_some_and(|b| b.as_block_node().is_some()) {
                    id + 1
                } else {
                    parent
                };
                let key = (
                    method.to_vec(),
                    ast::fingerprint_for_require(arg, self.file, self.locals),
                );
                let index = self.candidates.len();
                let seen = self.seen.entry(parent).or_default();
                let duplicate = if let Some((first_order, first_index)) = seen.get_mut(&key) {
                    if order < *first_order {
                        self.candidates[*first_index].1 = true;
                        *first_order = order;
                        *first_index = index;
                        false
                    } else {
                        true
                    }
                } else {
                    seen.insert(key, (order, index));
                    false
                };
                {
                    let start = call.location().start_offset();
                    let end = ast::source_end(&node);
                    let (line, col) =
                        super::array_literal_in_regexp::reference_line_col(start, self.file);
                    self.candidates.push((
                        Offense {
                            cop: "Lint/DuplicateRequire",
                            severity: crate::offense::Severity::Error,
                            message: format!(
                                "Duplicate `{}` detected.",
                                String::from_utf8_lossy(method)
                            ),
                            line,
                            col,
                            end_line: self.file.line_of(end.saturating_sub(1).max(start)),
                            // The duplicate's whole lines go.
                            correction: Some(crate::offense::Correction {
                                range: crate::cops::style::correction::whole_lines(
                                    self.file,
                                    start..call.location().end_offset(),
                                ),
                                replacement: String::new(),
                            }),
                        },
                        duplicate,
                    ));
                }
            }
        }
        self.stack.push(Frame {
            node,
            id: semantic_id,
            order: if transparent { order } else { 0 },
        });
    }
    fn leave(&mut self) {
        self.stack.pop();
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's duplicate_require_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/DuplicateRequire")
            .collect()
    }

    #[test]
    fn duplicate_requires_report_exact_positions_without_corrections() {
        for (source, line, col, method) in [
            ("require 'foo'\nrequire 'foo'\n", 2, 1, "require"),
            (
                "require_relative '../bar'\nrequire_relative '../bar'\n",
                2,
                1,
                "require_relative",
            ),
            ("require 'foo'\nKernel.require 'foo'\n", 2, 1, "require"),
            (
                "require 'foo'\ndef m\nend\nrequire 'foo'\n",
                4,
                1,
                "require",
            ),
            (
                "def m\n  require 'foo'\n  require 'foo'\nend\n",
                3,
                3,
                "require",
            ),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (line, col));
            assert_eq!(found[0].message, format!("Duplicate `{method}` detected."));
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn multiple_duplicates_are_independent_per_method() {
        let found = offenses(
            "require 'foo'\nrequire_relative '../bar'\nrequire 'foo/baz'\nrequire_relative '../bar'\nKernel.require 'foo'\nrequire 'quux'\n",
        );
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(4, 1), (5, 1)]
        );
    }

    #[test]
    fn all_pinned_specs_source_examples_and_parent_argument_controls() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/duplicate_require.json"
        ))
        .unwrap();
        for row in rows {
            // lint_str's supplied path is test.rb, while CLI fixtures use lib/a.rb.
            let source = row["source"]
                .as_str()
                .unwrap()
                .replace("'lib/a.rb'", "'test.rb'");
            let actual: Vec<_> = offenses(&source)
                .into_iter()
                .map(|o| (o.line, o.col, o.end_line, o.message))
                .collect();
            let expected: Vec<(u32, u32, u32, String)> = crate::fixture::value(&row["findings"]);
            assert_eq!(actual, expected, "{source}");
        }
    }

    #[test]
    fn command_literals_preserve_physical_parts_and_report_real_duplicates() {
        for (source, duplicate, line) in [
            (
                "require(`a\nb`)\nrequire(`a\\nb`)\n",
                "require(%x[a\\nb])\n",
                4,
            ),
            (
                "require(%x[a\nb])\nrequire(%x[a\\nb])\n",
                "require(`a\\nb`)\n",
                4,
            ),
            (
                "require <<`A`\na\nb\nA\nrequire <<`B`\na\\nb\nB\n",
                "require <<`C`\na\\nb\nC\n",
                8,
            ),
        ] {
            assert!(offenses(source).is_empty(), "{source}");
            let found = offenses(&format!("{source}{duplicate}"));
            assert_eq!(found.len(), 1, "{source}{duplicate}");
            assert_eq!((found[0].line, found[0].col), (line, 1));
            assert_eq!(found[0].message, "Duplicate `require` detected.");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn empty_dynamic_literals_remain_distinct_from_empty_quoted_strings() {
        for (source, duplicate, line) in [
            ("require('')\nrequire(%q[])\n", "require(%Q[])\n", 3),
            ("require(\"\")\nrequire(%Q[])\n", "require(%q[])\n", 3),
            ("require('')\nrequire <<A\nA\n", "require <<'B'\nB\n", 4),
            ("require('')\nrequire <<~A\nA\n", "require <<-B\nB\n", 4),
            ("require('')\nrequire(\"\\\n\")\n", "require(%Q[])\n", 4),
        ] {
            assert!(offenses(source).is_empty(), "{source}");
            let found = offenses(&format!("{source}{duplicate}"));
            assert_eq!(found.len(), 1, "{source}{duplicate}");
            assert_eq!((found[0].line, found[0].col), (line, 1));
            assert_eq!(found[0].message, "Duplicate `require` detected.");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn continuations_with_empty_unicode_escapes_report_exact_duplicates() {
        let source = "require(\"\\\n\")\nrequire(\"\\\n\\u{}\")\n";
        let found = offenses(source);
        assert_eq!(found.len(), 1, "{source}");
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 1, 4));
        assert_eq!(found[0].message, "Duplicate `require` detected.");
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn continuations_with_empty_unicode_escapes_are_not_empty_str_tokens() {
        let source = "require(\"\\u{}\\\n\\u{}\")\nrequire(\"\\u{ }\")\n";
        assert!(offenses(source).is_empty(), "{source}");
    }

    #[test]
    fn zero_child_dynamic_symbols_are_not_static_empty_symbols() {
        // Same runtime value, different AST#to_s.
        for dynamic in [":\"\"", ":''", "%s[]", ":\"\\\n\"", ":\"\\u{}\\\n\\u{}\""] {
            for static_symbol in [":\"\\u{}\"", ":\"\\u{ }\""] {
                let source = format!("require({dynamic})\nrequire({static_symbol})\n");
                assert!(offenses(&source).is_empty(), "{source}");
                let found = offenses(&format!("{source}require({static_symbol})\n"));
                assert_eq!(found.len(), 1, "{source}");
                let line = source.lines().count() as u32 + 1;
                assert_eq!(
                    (found[0].line, found[0].col, found[0].end_line),
                    (line, 1, line)
                );
                assert_eq!(found[0].message, "Duplicate `require` detected.");
                assert!(found[0].correction.is_none());
            }
        }
    }

    #[test]
    fn quoted_labels_keep_static_symbol_identity_including_empty_and_multiline() {
        for source in [
            "require({\"\": 1})\nrequire({\"\\u{}\": 1})\n",
            "require({\"\": 1})\nrequire({\"\\u{ }\": 1})\n",
            "require({'': 1})\nrequire({\"\\u{}\": 1})\n",
            "require({\"a\\n\nb\": 1})\nrequire({\"a\n\\nb\": 1})\n",
            "require({\"a\nb\": 1})\nrequire({:\"a\\nb\" => 1})\n",
            "require({'a\\(b\nc': 1})\nrequire({:\"a\\\\(b\\nc\" => 1})\n",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            let line = source
                .find("\nrequire")
                .map(|i| source[..i].lines().count() as u32 + 1)
                .unwrap();
            assert_eq!((found[0].line, found[0].col), (line, 1));
            assert_eq!(found[0].end_line, source.lines().count() as u32);
            assert_eq!(found[0].message, "Duplicate `require` detected.");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn quoted_labels_do_not_become_dynamic_symbol_literals() {
        for source in [
            "require({\"\": 1})\nrequire({:\"\" => 1})\n",
            "require({:\"\" => 1})\nrequire({\"\": 1})\n",
            "require({'': 1})\nrequire({%s[] => 1})\n",
            "require({\"a\nb\": 1})\nrequire({:\"a\nb\" => 1})\n",
            "require({\"\\u{}\\\n\\u{}\": 1})\nrequire({:\"\\u{}\\\n\\u{}\" => 1})\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn command_empty_unicode_payload_keeps_its_single_string_child() {
        for (empty, token) in [("``", "`\\u{}`"), ("%x[]", "%x[\\u{ }]")] {
            let source = format!("require({empty})\nrequire({token})\n");
            assert!(offenses(&source).is_empty(), "{source}");
            let found = offenses(&format!("{source}require({token})\n"));
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 1, 3));
            assert_eq!(found[0].message, "Duplicate `require` detected.");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn empty_symbol_require_identity_does_not_change_other_fingerprint_modes() {
        use crate::{cops::ast, source::SourceFile};
        for source in ["[:\"\"]; [:\"\\u{}\"]", "{\"\": 1}; {:\"\" => 1}"] {
            let file = SourceFile::new("test.rb".into(), source.into());
            let parsed = crate::parser::parse(&file.bytes);
            assert!(parsed.errors().next().is_none());
            file.record_encoding(&parsed);
            let body = parsed.node().as_program_node().unwrap().statements().body();
            let a = body.iter().next().unwrap();
            let b = body.iter().nth(1).unwrap();
            let raw = (ast::fingerprint(&a), ast::fingerprint(&b));
            let correction = (
                ast::fingerprint_with_source_lines(&a, &file),
                ast::fingerprint_with_source_lines(&b, &file),
            );
            assert_eq!(raw.0, raw.1);
            assert_eq!(correction.0, correction.1);
            assert_ne!(
                ast::fingerprint_for_require(&a, &file, None),
                ast::fingerprint_for_require(&b, &file, None)
            );
            assert_eq!(raw, (ast::fingerprint(&a), ast::fingerprint(&b)));
            assert_eq!(
                correction,
                (
                    ast::fingerprint_with_source_lines(&a, &file),
                    ast::fingerprint_with_source_lines(&b, &file)
                )
            );
        }
    }

    #[test]
    fn escaped_source_glyphs_have_reference_buffer_identity() {
        use crate::{cops::run_cops, source::SourceFile};
        for (encoding, glyph, unicode) in [
            ("ISO-8859-1", b"\xe9".as_slice(), "e9"),
            ("Windows-1252", b"\xf6".as_slice(), "f6"),
            ("Shift_JIS", b"\x95\x5c".as_slice(), "8868"),
            ("EUC-JP", b"\xa4\xa2".as_slice(), "3042"),
            ("GB18030", b"\xd2\xbb".as_slice(), "4e00"),
        ] {
            for (open, close) in [("\"", "\""), (":\"", "\""), ("`", "`")] {
                for other in [glyph.to_vec(), format!("\\u{{{unicode}}}").into_bytes()] {
                    let mut bytes = format!("# coding: {encoding}\nrequire({open}\\").into_bytes();
                    bytes.extend_from_slice(glyph);
                    bytes.extend_from_slice(format!("{close})\nrequire({open}").as_bytes());
                    bytes.extend_from_slice(&other);
                    bytes.extend_from_slice(format!("{close})\n").as_bytes());
                    let file = SourceFile::new("test.rb".into(), bytes);
                    let result = crate::parser::parse(&file.bytes);
                    assert!(result.errors().next().is_none(), "{:?}", file.bytes);
                    file.record_encoding(&result);
                    let mut found = Vec::new();
                    assert!(
                        run_cops(&[&super::DuplicateRequire], &result, &file, &mut found)
                            .is_empty()
                    );
                    assert_eq!(found.len(), 1, "{:?}", file.bytes);
                    assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 1, 3));
                    assert_eq!(found[0].message, "Duplicate `require` detected.");
                    assert!(found[0].correction.is_some());
                }
            }
        }
    }

    #[test]
    fn require_projection_does_not_mutate_main_ast_or_correction_fingerprints() {
        use crate::{cops::ast, source::SourceFile};
        let file = SourceFile::new(
            "test.rb".into(),
            b"# coding: ISO-8859-1\nrequire(\"\\\xe9\")\nrequire(\"\\u{e9}\")\n".to_vec(),
        );
        let result = crate::parser::parse(&file.bytes);
        assert!(result.errors().next().is_none());
        file.record_encoding(&result);
        let calls: Vec<_> = result
            .node()
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .iter()
            .collect();
        let a = ast::call_arguments(&calls[0].as_call_node().unwrap()).remove(0);
        let b = ast::call_arguments(&calls[1].as_call_node().unwrap()).remove(0);
        let before = (ast::fingerprint(&a), ast::fingerprint(&b));
        assert_ne!(before.0, before.1);
        assert_eq!(
            ast::fingerprint_for_require(&a, &file, None),
            ast::fingerprint_for_require(&b, &file, None)
        );
        assert_eq!((ast::fingerprint(&a), ast::fingerprint(&b)), before);
        assert_eq!(a.as_string_node().unwrap().unescaped(), b"\xe9");
        assert_eq!(b.as_string_node().unwrap().unescaped(), "é".as_bytes());
        assert_eq!(file.encoded_character_width(b"\xe9"), 1);
    }

    #[test]
    fn reference_buffer_names_selectors_and_regexp_source_are_require_keys() {
        for (a, b) in [
            ("ae\u{301}", "aé"),
            ("obj.ae\u{301}(1)", "obj.aé(1)"),
            ("Ae\u{301}", "Aé"),
            ("@ae\u{301}", "@aé"),
            ("$ae\u{301}", "$aé"),
            ("-> { [ae\u{301}] }", "-> { [aé] }"),
            ("/e\u{301}/", "/é/"),
        ] {
            let source = format!("# coding: UTF8-MAC\nrequire({a})\nrequire({b})\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 1, 3));
            assert_eq!(found[0].message, "Duplicate `require` detected.");
            assert!(found[0].correction.is_none());
            assert!(offenses(&source.replace("UTF8-MAC", "UTF-8")).is_empty());
        }
    }

    #[test]
    fn projected_local_kinds_use_declarations_scopes_and_order_not_only_names() {
        for body in [
            "ae\u{301} = 1; require(aé); require(aé())",
            "def f(ae\u{301}); require(aé); require(aé()); end",
            "task { |ae\u{301}| require(aé); require(aé()) }",
            "->(ae\u{301}) { require(aé); require(aé()) }",
            "ae\u{301} = 1; task { require([aé]); require([aé()]) }",
        ] {
            let source = format!("# coding: UTF8-MAC\n{body}\n");
            assert!(offenses(&source).is_empty(), "{source}");
            let found = offenses(&source.replace("UTF8-MAC", "UTF-8"));
            assert_eq!(found.len(), 1, "{source}");
        }
    }

    #[test]
    fn composed_reference_columns_keep_positive_hard_scope_and_order_controls() {
        for (body, col) in [
            ("require(aé); ae\u{301} = 1; require(aé())", 22),
            ("ae\u{301} = 1; def f; require(aé); require(aé()); end", 29),
        ] {
            let source = format!("# coding: UTF8-MAC\n{body}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (2, col, 2)
            );
            assert_eq!(found[0].message, "Duplicate `require` detected.");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn regexp_escape_letters_compose_with_source_marks_before_escape_interpretation() {
        for (raw, composed) in [
            ("e\u{301}", "é"),
            ("o\u{308}", "ö"),
            ("a\u{30a}\u{301}", "ǻ"),
        ] {
            let source =
                format!("# coding: UTF8-MAC\nrequire(/\\{raw}/); require(/\\{composed}/)\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 16, 2));
            assert_eq!(found[0].message, "Duplicate `require` detected.");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn all_projected_fixtures_have_exact_ranges_messages_and_corrections() {
        byte_fixtures(
            include_str!("../../../tests/fixtures/duplicate_require_projected.json"),
            1752,
        );
    }

    #[test]
    fn v10_lexical_regexp_and_column_neighbours_keep_exact_ranges() {
        byte_fixtures(
            include_str!("../../../tests/fixtures/duplicate_require_v10_neighbours.json"),
            209,
        );
    }

    fn byte_fixtures(json: &str, count: usize) {
        use crate::{cops::run_cops, source::SourceFile};
        let rows: Vec<serde_json::Value> = serde_json::from_str(json).unwrap();
        assert_eq!(rows.len(), count);
        for row in rows {
            let file = SourceFile::new(
                "lib/a.rb".into(),
                crate::fixture::value::<Vec<u8>>(&row["bytes"]),
            );
            let parsed = crate::parser::parse(&file.bytes);
            assert!(parsed.errors().next().is_none(), "{row}");
            file.record_encoding(&parsed);
            let mut found = Vec::new();
            assert!(run_cops(&[&super::DuplicateRequire], &parsed, &file, &mut found).is_empty());
            assert!(found.iter().all(|o| o.correction.is_some()));
            let actual: Vec<_> = found
                .into_iter()
                .map(|o| (o.line, o.col, o.end_line, o.message))
                .collect();
            let expected: Vec<(u32, u32, u32, String)> = crate::fixture::value(&row["findings"]);
            assert_eq!(actual, expected, "{row}");
        }
    }

    #[test]
    fn source_name_projection_does_not_relax_other_modes_or_numeric_metadata() {
        use crate::{cops::ast, source::SourceFile};
        for (a, b) in [
            ("ae\u{301}", "aé"),
            ("/\\e\u{301}/", "/\\é/"),
            ("->(ae\u{301}) { ae\u{301} }", "->(aé) { aé }"),
        ] {
            let file = SourceFile::new(
                "lib/a.rb".into(),
                format!("# coding: UTF8-MAC\n{a}; {b}\n").into_bytes(),
            );
            let parsed = crate::parser::parse(&file.bytes);
            assert!(parsed.errors().next().is_none());
            file.record_encoding(&parsed);
            let body = parsed.node().as_program_node().unwrap().statements().body();
            let a = body.iter().next().unwrap();
            let b = body.iter().nth(1).unwrap();
            let raw = (ast::fingerprint(&a), ast::fingerprint(&b));
            let correction = (
                ast::fingerprint_with_source_lines(&a, &file),
                ast::fingerprint_with_source_lines(&b, &file),
            );
            assert_ne!(raw.0, raw.1);
            assert_ne!(correction.0, correction.1);
            assert_eq!(
                ast::fingerprint_for_require(&a, &file, None),
                ast::fingerprint_for_require(&b, &file, None)
            );
            assert_eq!(raw, (ast::fingerprint(&a), ast::fingerprint(&b)));
            assert_eq!(
                correction,
                (
                    ast::fingerprint_with_source_lines(&a, &file),
                    ast::fingerprint_with_source_lines(&b, &file)
                )
            );
        }
        for source in [
            "require(0.0); require(-0.0)",
            "require(128); require(256)",
            "require(1i); require(1.0i)",
        ] {
            assert!(offenses(&format!("# coding: UTF8-MAC\n{source}\n")).is_empty());
        }
    }

    #[test]
    fn implicit_regexp_conditions_normalize_delimiters_and_option_sets() {
        for (a, b) in [
            ("if /x/im; y; end", "if /x/mi; y; end"),
            ("if /x/; y; end", "if %r{x}; y; end"),
            ("if /x#{z}/im; y; end", "if %r{x#{z}}mi; y; end"),
            ("if /x/ii; y; end", "if /x/i; y; end"),
        ] {
            let source = format!("require({a})\nrequire({b})\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 1, 2));
            assert_eq!(found[0].message, "Duplicate `require` detected.");
            assert!(found[0].correction.is_none());
        }
        for (a, b) in [
            ("if /x/i; y; end", "if /x/m; y; end"),
            ("if /x/; y; end", "if /z/; y; end"),
            ("if /x#{z}/i; y; end", "if /x#{z}/m; y; end"),
            ("if /x#{z}/; y; end", "if /x#{w}/; y; end"),
        ] {
            let source = format!("require({a})\nrequire({b})\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }

    #[test]
    fn explicit_rescue_empty_else_is_absent_but_nil_and_grouping_are_not() {
        for (a, b) in [
            ("begin; x; rescue; y; else; end", "begin; x; rescue; y; end"),
            ("begin; x; rescue; y; end", "begin; x; rescue; y; else; end"),
            (
                "begin; x; rescue; y; else; ensure; z; end",
                "begin; x; rescue; y; ensure; z; end",
            ),
        ] {
            let source = format!("require({a})\nrequire({b})\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 1, 2));
            assert_eq!(found[0].message, "Duplicate `require` detected.");
            assert!(found[0].correction.is_none());
        }
        for (a, b) in [
            (
                "begin; x; rescue; y; else; nil; end",
                "begin; x; rescue; y; end",
            ),
            ("begin; x; rescue; y; end", "(begin; x; rescue; y; end)"),
            ("begin; x; rescue; y; end", "begin; x; rescue; z; end"),
        ] {
            let source = format!("require({a})\nrequire({b})\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }

    #[test]
    fn v10_review_regexp_and_rescue_neighbours_keep_exact_ranges() {
        byte_fixtures(
            include_str!("../../../tests/fixtures/duplicate_require_identity.json"),
            232,
        );
    }

    #[test]
    fn regexp_and_rescue_require_normalization_leaves_other_modes_unchanged() {
        use crate::{cops::ast, source::SourceFile};
        for source in [
            "if /x/im; y; end; if /x/mi; y; end",
            "if /x#{z}/im; y; end; if %r{x#{z}}mi; y; end",
            "begin; x; rescue; y; else; end; begin; x; rescue; y; end",
        ] {
            let file = SourceFile::new("test.rb".into(), source.into());
            let parsed = crate::parser::parse(&file.bytes);
            assert!(parsed.errors().next().is_none());
            file.record_encoding(&parsed);
            let body = parsed.node().as_program_node().unwrap().statements().body();
            let a = body.iter().next().unwrap();
            let b = body.iter().nth(1).unwrap();
            let raw = (ast::fingerprint(&a), ast::fingerprint(&b));
            let correction = (
                ast::fingerprint_with_source_lines(&a, &file),
                ast::fingerprint_with_source_lines(&b, &file),
            );
            assert_ne!(raw.0, raw.1, "{source}");
            assert_ne!(correction.0, correction.1, "{source}");
            // Branch/scope consumers retain their prior keys.
            assert_ne!(
                ast::fingerprint_for_branch(&a, &file),
                ast::fingerprint_for_branch(&b, &file),
                "{source}"
            );
            assert_ne!(
                ast::fingerprint_for_conditional_scope(&a, &file),
                ast::fingerprint_for_conditional_scope(&b, &file),
                "{source}"
            );
            assert_eq!(
                ast::fingerprint_for_require(&a, &file, None),
                ast::fingerprint_for_require(&b, &file, None),
                "{source}"
            );
            assert_eq!(raw, (ast::fingerprint(&a), ast::fingerprint(&b)));
            assert_eq!(
                correction,
                (
                    ast::fingerprint_with_source_lines(&a, &file),
                    ast::fingerprint_with_source_lines(&b, &file)
                )
            );
        }
    }

    #[test]
    fn no_offense_lookalikes() {
        for source in [
            "require 'foo'\nrequire 'bar'\n",
            "require 'foo'\n",
            "require 'feature'\nrequire_relative 'feature'\n",
            "params.require(:user)\nparams.require(:user)\n",
            "require\nrequire\n",
            "require 'x', 'y'\nrequire 'x', 'y'\n",
            "Kernel&.require 'x'\nKernel&.require 'x'\n",
            "require 'x'\ndef f; require 'x'; end\n",
            "def f; require 'x'; end\ndef g; require 'x'; end\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
