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

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_program_node().is_none() {
            return;
        }
        // One file-local traversal, never state shared across investigations.
        let mut requires = Requires {
            file,
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
                .filter_map(|(offense, duplicate)| duplicate.then_some(offense)),
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
                    ast::fingerprint_for_require(arg, self.file),
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
                    let (line, col) = self.file.line_col(start);
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
                            correction: None,
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
            let expected: Vec<(u32, u32, u32, String)> =
                serde_json::from_value(row["findings"].clone()).unwrap();
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
                    assert!(found[0].correction.is_none());
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
            ast::fingerprint_for_require(&a, &file),
            ast::fingerprint_for_require(&b, &file)
        );
        assert_eq!((ast::fingerprint(&a), ast::fingerprint(&b)), before);
        assert_eq!(a.as_string_node().unwrap().unescaped(), b"\xe9");
        assert_eq!(b.as_string_node().unwrap().unescaped(), "é".as_bytes());
        assert_eq!(file.encoded_character_width(b"\xe9"), 1);
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
