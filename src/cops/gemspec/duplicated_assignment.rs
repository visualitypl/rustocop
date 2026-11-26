//! `Gemspec/DuplicatedAssignment`: repeated setters or literal indexed keys,
//! grouped by the nearest structural specification (not a project/runtime type).
use crate::{
    cops::{Cop, ast},
    offense::Offense,
    source::SourceFile,
};
use std::collections::HashMap;
pub struct DuplicatedAssignment;
impl Cop for DuplicatedAssignment {
    fn name(&self) -> &'static str {
        "Gemspec/DuplicatedAssignment"
    }
    fn check_parse(
        &self,
        result: &ruby_prism::ParseResult<'_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let mut first = HashMap::new();
        let assignments = super::help::assignments(&result.node(), file);
        // The pin hashes enclosing block ASTs, not object identity/offsets.
        // Prism's call includes its block. Intern each structural group once,
        // reusing this investigation's source/local facts and Gemspec profile.
        // Setters then hash small group IDs, not the entire enclosing AST again.
        let mut groups = HashMap::new();
        let specifications: Vec<_> = assignments
            .specifications
            .iter()
            .map(|node| {
                let key = ast::fingerprint_for_gemspec_key(node, file, &assignments.key_locals);
                let next = groups.len();
                *groups.entry(key).or_insert(next)
            })
            .collect();
        for a in assignments.nodes {
            let index = a
                .index
                .as_ref()
                .map(|key| ast::fingerprint_for_gemspec_key(key, file, &assignments.key_locals));
            let key = (
                a.specification.map(|id| specifications[id]),
                a.attribute.clone(),
                index,
            );
            let (line, col) = file.reference_line_col(a.node.location().start_offset());
            if let Some(first_line) = first.get(&key) {
                // Selectors are already projected by the structural helper;
                // only the raw index source still needs source-buffer decoding.
                let attribute = text(&a.attribute);
                let assignment = if let Some(index) = a.index {
                    format!(
                        "{attribute}[{}]=",
                        text(&file.match_pattern_source(index.location().as_slice()))
                    )
                } else {
                    attribute
                };
                out.push(Offense { cop: self.name(), severity: self.severity(), line, col, end_line: line,
                    message: format!("`{assignment}` method calls already given on line {first_line} of the gemspec."), correction: None });
            } else {
                first.insert(key, line);
            }
        }
    }
}
fn text(bytes: &[u8]) -> String {
    bytes
        .utf8_chunks()
        .map(|chunk| {
            let mut text = chunk.valid().to_owned();
            for b in chunk.invalid() {
                text.push_str(&format!("\\x{b:02X}"));
            }
            text
        })
        .collect::<String>()
        .replace("\r\n", "\n")
}
#[cfg(test)]
mod tests {
    //! RuboCop duplicated_assignment_spec.rb, MIT (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{
        config::ResolvedConfig,
        cops::{Registry, select_active},
    };
    fn lint(source: &[u8]) -> Vec<Offense> {
        let config = ResolvedConfig::default();
        let registry = Registry::new();
        let selected = select_active(
            &registry,
            &config,
            &[DuplicatedAssignment.name().into()],
            &[],
        );
        let report = crate::pipeline::lint_one(
            std::path::Path::new("a.gemspec"),
            std::path::Path::new("a.gemspec"),
            Some(source),
            &selected,
            &config,
        );
        assert!(report.errors.is_empty(), "{report:?}");
        assert!(report.internal_errors.is_empty(), "{report:?}");
        report.result.offenses
    }
    #[test]
    fn interpolation_grouping_is_significant_in_keys_and_specification_groups() {
        for template in [
            "\"#{EXPR}\"",
            ":\"#{EXPR}\"",
            "`#{EXPR}`",
            "/#{EXPR}/",
            "%W[#{EXPR}]",
            "%I[#{EXPR}]",
            "{key: [\"#{EXPR}\"]}",
        ] {
            for (a, b, duplicate) in [
                ("x; y", "(x; y)", false),
                ("(x; y)", "x; y", false),
                ("x; y", "x; y", true),
                ("(x; y)", "(x; y)", true),
            ] {
                let first = template.replace("EXPR", a);
                let second = template.replace("EXPR", b);
                for declaration in ["", "  x = 0\n"] {
                    let source = format!(
                        "Gem::Specification.new do |s|\n{declaration}  s.metadata[{first}] = 1\n  s.metadata[{second}] = 2\nend\n"
                    );
                    let found = lint(source.as_bytes());
                    assert_eq!(found.len(), usize::from(duplicate), "{source}: {found:?}");
                    if duplicate {
                        let line = if declaration.is_empty() { 3 } else { 4 };
                        assert_eq!((found[0].line, found[0].col), (line, 3));
                        assert_eq!(
                            found[0].message,
                            format!(
                                "`metadata[{second}]=` method calls already given on line {} of the gemspec.",
                                line - 1
                            )
                        );
                        assert!(found[0].correction.is_none());
                    }
                    let source = format!(
                        "Gem::Specification.new do |s|\n{declaration}  {first}\n  s.name = 1\nend\nGem::Specification.new do |s|\n{declaration}  {second}\n  s.name = 1\nend\n"
                    );
                    let found = lint(source.as_bytes());
                    assert_eq!(found.len(), usize::from(duplicate), "{source}: {found:?}");
                    if duplicate {
                        let (line, first_line) = if declaration.is_empty() {
                            (7, 3)
                        } else {
                            (9, 4)
                        };
                        assert_eq!((found[0].line, found[0].col), (line, 3));
                        assert_eq!(
                            found[0].message,
                            format!(
                                "`name=` method calls already given on line {first_line} of the gemspec."
                            )
                        );
                        assert!(found[0].correction.is_none());
                    }
                }
            }
        }
    }
    #[test]
    fn protected_rescue_empty_else_is_an_absent_ast_child() {
        let source = "Gem::Specification.new do |s|\n  s.metadata[[begin; x; rescue; y; else; end]] = 1\n  s.metadata[[begin; x; rescue; y; end]] = 2\nend\n";
        let found = lint(source.as_bytes());
        assert_eq!(found.len(), 1, "{found:?}");
        let o = &found[0];
        assert_eq!((o.line, o.col, o.end_line), (3, 3, 3));
        assert_eq!(
            o.message,
            "`metadata[[begin; x; rescue; y; end]]=` method calls already given on line 2 of the gemspec."
        );
        assert!(o.correction.is_none());
    }
    #[test]
    fn protected_rescue_slot_projection_preserves_children_and_spec_groups() {
        let empty = "begin; x; rescue; y; else; end";
        let absent = "begin; x; rescue; y; end";
        for (other, duplicate) in [
            (absent, true),
            ("begin; x; rescue; y; else; nil; end", false),
            ("(begin; x; rescue; y; end)", false),
            ("begin; different; rescue; y; end", false),
            ("begin; x; rescue; different; end", false),
            ("begin; x; rescue; y; ensure; z; end", false),
        ] {
            for (a, b) in [(empty, other), (other, empty)] {
                let source = format!(
                    "Gem::Specification.new do |s|\n  [{a}]\n  s.name = 1\nend\nGem::Specification.new do |s|\n  [{b}]\n  s.name = 1\nend\n"
                );
                let found = lint(source.as_bytes());
                assert_eq!(found.len(), usize::from(duplicate), "{source}: {found:?}");
                if duplicate {
                    let o = &found[0];
                    assert_eq!((o.line, o.col, o.end_line), (7, 3, 7));
                    assert_eq!(
                        o.message,
                        "`name=` method calls already given on line 3 of the gemspec."
                    );
                    assert!(o.correction.is_none());
                }
            }
        }
    }
    #[test]
    fn review_shorthand_and_explicit_hash_keys_have_identical_ast() {
        for (first, second, duplicate) in [
            ("{x:}", "{x: x}", true),
            ("{x: x}", "{x:}", true),
            ("[{x:}]", "[{x: x}]", true),
            ("{x:}", "{x: y}", false),
            ("{x:}", "{x: x()}", false),
            ("{x:}", "{y: x}", false),
        ] {
            let source = format!(
                "Gem::Specification.new do |s|\n  x = 0; y = 1\n  s.metadata[{first}] = 1\n  s.metadata[{second}] = 2\nend\n"
            );
            let found = lint(source.as_bytes());
            assert_eq!(found.len(), usize::from(duplicate), "{source}: {found:?}");
            if duplicate {
                let o = &found[0];
                assert_eq!((o.line, o.col, o.end_line), (4, 3, 4));
                assert_eq!(
                    o.message,
                    format!(
                        "`metadata[{second}]=` method calls already given on line 3 of the gemspec."
                    )
                );
                assert!(o.correction.is_none());
            }
        }
    }
    #[test]
    fn review_specification_groups_use_structural_block_equality() {
        for (first, second, assignment, col) in [
            ("s.name = 1", "s.name=1", "name=", 3),
            ("s.metadata[:x] = 1", "s.metadata[:x]=1", "metadata[:x]=", 3),
            (
                "x = 0; s.metadata[{x:}] = 1",
                "x=0; s.metadata[{x: x}]=1",
                "metadata[{x: x}]=",
                8,
            ),
        ] {
            let source = format!(
                "Gem::Specification.new do |s|\n  {first}\nend\nGem::Specification.new {{ |s|\n  {second}\n}}\n"
            );
            let found = lint(source.as_bytes());
            assert_eq!(found.len(), 1, "{source}: {found:?}");
            let o = &found[0];
            assert_eq!((o.line, o.col, o.end_line), (5, col, 5));
            assert_eq!(
                o.message,
                format!("`{assignment}` method calls already given on line 2 of the gemspec.")
            );
            assert!(o.correction.is_none());
        }
        for second in ["s.name = 2", "other; s.name = 1"] {
            let source = format!(
                "Gem::Specification.new {{ |s| s.name = 1 }}\nGem::Specification.new {{ |s| {second} }}\n"
            );
            assert!(lint(source.as_bytes()).is_empty(), "{source}");
        }
    }
    #[test]
    fn regexp_source_composes_before_escape_interpretation() {
        let source = "# coding: UTF8-MAC\nGem::Specification.new do |s|\n  s.metadata[/\\e\u{301}/] = 1\n  s.metadata[/\\é/] = 2\nend\n";
        let found = lint(source.as_bytes());
        assert_eq!(found.len(), 1, "{found:?}");
        let o = &found[0];
        assert_eq!((o.line, o.col, o.end_line), (4, 3, 4));
        assert_eq!(
            o.message,
            "`metadata[/\\é/]=` method calls already given on line 3 of the gemspec."
        );
        assert!(o.correction.is_none());
    }
    #[test]
    fn reference_buffer_columns_compose_mac_and_count_first_line_bom() {
        for (source, line, col) in [
            (
                "# coding: UTF8-MAC\nGem::Specification.new { |s| cafe\u{301} = 0; s.name = 1; s.name = 2 }\n",
                2,
                52,
            ),
            (
                "\u{feff}Gem::Specification.new { |s| s.name = 1; s.name = 2 }\n",
                1,
                43,
            ),
        ] {
            let found = lint(source.as_bytes());
            assert_eq!(found.len(), 1, "{source}: {found:?}");
            let o = &found[0];
            assert_eq!((o.line, o.col, o.end_line), (line, col, line), "{source}");
            assert_eq!(
                o.message,
                format!("`name=` method calls already given on line {line} of the gemspec.")
            );
            assert!(o.correction.is_none());
        }
    }
    #[test]
    fn implicit_it_and_declared_local_keys_have_pinned_ast_identity() {
        for key in ["[it]", "{x: it}", "\"#{it}\"", "/#{it}/", "1..it", "[[it]]"] {
            let first = format!("  [1].each {{ s.metadata[{key}] = 1 }}\n");
            for (body, expected) in [
                (
                    format!("{first}  it = 2\n  s.metadata[{key}] = 2\n"),
                    Some((4, 3, 2)),
                ),
                (
                    format!("{first}  def f(s,it); s.metadata[{key}] = 2; end\n"),
                    Some((3, 16, 2)),
                ),
                (
                    format!("  it = 2\n{first}  s.metadata[{key}] = 2\n"),
                    Some((4, 3, 3)),
                ),
                (
                    format!("{first}  [2].each {{ s.metadata[{key}] = 2 }}\n"),
                    Some((3, 14, 2)),
                ),
                (
                    format!(
                        "{first}  it = 2\n  s.metadata[{}] = 2\n",
                        key.replace("it", "it()")
                    ),
                    None,
                ),
            ] {
                let source = format!("Gem::Specification.new do |s|\n{body}end\n");
                let found = lint(source.as_bytes());
                assert_eq!(
                    found.len(),
                    usize::from(expected.is_some()),
                    "{source}: {found:?}"
                );
                if let Some((line, col, first_line)) = expected {
                    let o = &found[0];
                    assert_eq!((o.line, o.col, o.end_line), (line, col, line), "{source}");
                    assert_eq!(
                        o.message,
                        format!(
                            "`metadata[{key}]=` method calls already given on line {first_line} of the gemspec."
                        )
                    );
                    assert!(o.correction.is_none());
                }
            }
        }
    }
    #[test]
    fn composite_keys_project_locals_with_declaration_order_and_closed_scopes() {
        for key in ["[café]", "{x: café}", "\"#{café}\"", "[-> { café }]"] {
            let other = key.replace("café", "cafe\u{301}");
            for (body, expected) in [
                (
                    format!("café = 1\n  s.metadata[{key}] = 1\n  s.metadata[{other}] = 2"),
                    1,
                ),
                (
                    format!("café = 1\n  s.metadata[{other}] = 1\n  s.metadata[{key}] = 2"),
                    1,
                ),
                (
                    format!("s.metadata[{other}] = 1\n  café = 1\n  s.metadata[{key}] = 2"),
                    0,
                ),
                (
                    format!(
                        "café = 1\n  s.metadata[{key}] = 1\n  s.metadata[{}] = 2",
                        other.replace("cafe\u{301}", "cafe\u{301}()")
                    ),
                    0,
                ),
                (
                    format!(
                        "café = 1\n  s.metadata[{key}] = 1\n  def example(s)\n    s.metadata[{other}] = 2\n  end"
                    ),
                    0,
                ),
            ] {
                let source =
                    format!("# coding: UTF8-MAC\nGem::Specification.new do |s|\n  {body}\nend\n");
                let offenses = lint(source.as_bytes());
                assert_eq!(offenses.len(), expected, "{source}: {offenses:?}");
                if expected > 0 {
                    let o = &offenses[0];
                    assert_eq!((o.line, o.col, o.end_line), (5, 3, 5));
                    assert_eq!(
                        o.message,
                        format!(
                            "`metadata[{key}]=` method calls already given on line 4 of the gemspec."
                        )
                    );
                    assert!(o.correction.is_none());
                }
            }
        }
    }
    #[test]
    fn legacy_attribute_messages_project_selectors_once_and_raw_indices_once() {
        let source = b"# coding: Windows-1252\nGem::Specification.new do |s|\n  s.caf\xe9 = 1\n  s.caf\xe9 = 2\n  s.m\xe9tadata['cl\xe9'] = 1\n  s.m\xe9tadata['cl\xe9'] = 2\nend\n";
        let offenses = lint(source);
        assert_eq!(offenses.len(), 2);
        for (o, line, assignment, first) in [
            (&offenses[0], 4, "café=", 3),
            (&offenses[1], 6, "métadata['clé']=", 5),
        ] {
            assert_eq!((o.line, o.col, o.end_line), (line, 3, line));
            assert_eq!(
                o.message,
                format!(
                    "`{assignment}` method calls already given on line {first} of the gemspec."
                )
            );
            assert!(o.correction.is_none());
        }
    }
    #[test]
    fn duplicate_position_message_and_independent_specifications() {
        let config = ResolvedConfig::default();
        let registry = Registry::new();
        let selected = select_active(
            &registry,
            &config,
            &[DuplicatedAssignment.name().into()],
            &[],
        );
        for (source, n) in [
            (
                "Gem::Specification.new do |spec|\n  spec.name = 1\n  spec.name = 2\nend\n",
                1,
            ),
            (
                "Gem::Specification.new { |spec| spec.name = 1 }\nGem::Specification.new { |spec| spec.name = 2 }",
                0,
            ),
        ] {
            let r = crate::pipeline::lint_one(
                std::path::Path::new("a.gemspec"),
                std::path::Path::new("a.gemspec"),
                Some(source.as_bytes()),
                &selected,
                &config,
            );
            assert_eq!(r.result.offenses.len(), n);
            if n > 0 {
                let o = &r.result.offenses[0];
                assert_eq!((o.line, o.col, o.end_line), (3, 3, 3));
                assert_eq!(
                    o.message,
                    "`name=` method calls already given on line 2 of the gemspec."
                );
            }
        }
    }
}
