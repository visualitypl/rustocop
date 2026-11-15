//! `Style/EndlessMethod`: fixed `allow_single_line`, with conservative expansion.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct EndlessMethod;

impl Cop for EndlessMethod {
    fn name(&self) -> &'static str {
        "Style/EndlessMethod"
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
        // Only endless definitions have `=` here. Ordinary setters are allowed;
        // endless setters (including []=) are syntax errors in supported Ruby.
        if def.equal_loc().is_none() {
            return;
        }
        let (line, col) = file.line_col(def.location().start_offset());
        let end_line = file.last_line_of(def.location());
        if line == end_line {
            return;
        }
        if let Some(body) = def.body() {
            let root = cx.ancestors().next_back().unwrap_or(node);
            if file.contains_heredoc(root, &body) {
                return;
            }
        }
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Avoid endless method definitions with multiple lines.".into(),
            line,
            col,
            end_line,
            correction: cx
                .corrections_requested()
                .then(|| rewrite(file, node, cx))
                .flatten(),
        });
    }
}

fn rewrite(file: &SourceFile, node: &Node<'_>, cx: &NodeContext<'_, '_>) -> Option<Correction> {
    let def = node.as_def_node()?;
    let equal = def.equal_loc()?;
    let body = def.body()?.location();
    let start = def.location().start_offset();
    let end = def.location().end_offset();
    let root = cx.ancestors().next_back()?;
    // One outer expansion per pass. An inner expansion could overlap its
    // enclosing definition, or escape a heredoc/inline context the outer rule
    // cannot safely rewrite. Keep both diagnostics, never a partial edit.
    if cx
        .ancestors()
        .any(|n| n.as_def_node().is_some_and(|d| d.equal_loc().is_some()))
        || file.contains_heredoc(root, node)
        || super::correction::touches_directive(file, start..end)
        || start < file.line_value_boundary()
        || {
            let observers = file.class_method_bindings(root);
            observers.observes_sources || observers.observes_data
        }
    {
        return None;
    }
    let comments = file.comments();
    let first = comments.partition_point(|comment| comment.end <= start);
    if comments[first..]
        .iter()
        .take_while(|comment| comment.start < end)
        .any(|comment| super::correction::contains_magic_comment_key(&file.bytes[comment.clone()]))
    {
        return None;
    }
    let signature_end = def
        .rparen_loc()
        .unwrap_or_else(|| def.name_loc())
        .end_offset();
    if !file.bytes[signature_end..equal.start_offset()]
        .iter()
        .all(|b| matches!(b, b' ' | b'\t'))
    {
        return None;
    }
    let line = file.line_of(start);
    let prefix = &file.bytes[file.line_start(line)..start];
    let prefix = prefix.strip_prefix(b"\xef\xbb\xbf").unwrap_or(prefix);
    // Known visibility wrappers stay outside the replacement. Other inline
    // expression/definition prefixes need a separate binding proof.
    let prefix = [b"private ".as_slice(), b"protected ", b"public "]
        .iter()
        .find_map(|visibility| prefix.strip_suffix(*visibility))
        .unwrap_or(prefix);
    if !prefix.iter().rev().all(|b| matches!(b, b' ' | b'\t')) {
        return None;
    }
    let indent = std::str::from_utf8(prefix).ok()?;
    // Do not discard comments, continuations or deferred literal payloads in
    // the signature/body gap. All remaining body bytes are copied verbatim:
    // no reindentation of multiline strings, regexps, commands or blocks.
    if !file.bytes[equal.end_offset()..body.start_offset()]
        .iter()
        .all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
    {
        return None;
    }
    let header_end = if def.parameters().is_none() {
        if let (Some(left), Some(right)) = (def.lparen_loc(), def.rparen_loc()) {
            if !file.bytes[left.end_offset()..right.start_offset()]
                .iter()
                .all(|b| matches!(b, b' ' | b'\t'))
            {
                return None;
            }
            left.start_offset()
        } else {
            equal.start_offset()
        }
    } else {
        equal.start_offset()
    };
    let header = std::str::from_utf8(&file.bytes[start..header_end])
        .ok()?
        .trim_end_matches([' ', '\t']);
    let body = std::str::from_utf8(body.as_slice()).ok()?;
    let newline = if file.bytes[file.line_start(line)..file.line_start(line + 1)].ends_with(b"\r\n")
    {
        "\r\n"
    } else {
        "\n"
    };
    Some(Correction {
        range: start..end,
        replacement: format!("{header}{newline}{indent}  {body}{newline}{indent}end"),
    })
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/style/endless_method_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/EndlessMethod")
            .collect()
    }

    #[test]
    fn pinned_allow_single_line_corrections() {
        use crate::cops::testing::assert_correction;
        for (signature, padding) in [
            ("my_method()", 19),
            ("self.my_method()", 23),
            ("self::my_method()", 25),
            ("my_method(a, b)", 23),
        ] {
            let before = format!(
                "def {signature} = x.foo\n{}.bar\n{}.baz\n",
                " ".repeat(padding),
                " ".repeat(padding)
            );
            let after = format!(
                "def {}\n  x.foo\n{}.bar\n{}.baz\nend\n",
                signature.trim_end_matches("()"),
                " ".repeat(padding),
                " ".repeat(padding)
            );
            assert_correction(&super::EndlessMethod, &before, &after);
        }
        assert_correction(
            &super::EndlessMethod,
            "def my_method() = begin\n  foo && bar\nend\n",
            "def my_method\n  begin\n  foo && bar\nend\nend\n",
        );
    }

    #[test]
    fn signatures_visibility_operators_and_crlf_expand_without_reindentation() {
        use crate::cops::testing::assert_correction;
        for signature in [
            "self.f(a = 1, *rest, key:, **kw)",
            "self::f(a)",
            "+(other)",
            "==(other)",
            "[](index)",
            "f(\n    a,\n    b: 1\n  )",
        ] {
            for visibility in ["", "private ", "protected ", "public "] {
                for newline in ["\n", "\r\n"] {
                    let before =
                        format!("class C\n  {visibility}def {signature} = [\n    1]\nend\n")
                            .replace('\n', newline);
                    let after = format!(
                        "class C\n  {visibility}def {signature}\n    [\n    1]\n  end\nend\n"
                    )
                    .replace('\n', newline);
                    assert_correction(&super::EndlessMethod, &before, &after);
                }
            }
        }
    }

    #[test]
    fn literal_payloads_comments_bom_and_data_are_unchanged() {
        use crate::cops::testing::assert_correction;
        for body in [
            "\"first\nsecond\"",
            "\"#{1}\nsecond\"",
            "`printf 'first\nsecond'`",
            "/first\nsecond/",
            "%w[first\nsecond]",
            "%W[first\n#{1}]",
            "proc {\n  1\n}",
            "[ # body comment\n  1]",
            "begin\n  # ordinary comment\n  1\nend",
        ] {
            let before = format!(
                "\u{feff}# encoding: UTF-8\ndef f() = {body} # tail\n__END__\nraw\x00data\n"
            );
            let after = format!(
                "\u{feff}# encoding: UTF-8\ndef f\n  {body}\nend # tail\n__END__\nraw\x00data\n"
            );
            assert_correction(&super::EndlessMethod, &before, &after);
        }
    }

    fn assert_withheld(source: &str, count: usize) {
        let found = crate::cops::testing::correction_offenses(&super::EndlessMethod, source);
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        assert_eq!(found.len(), count, "{source}");
        assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        assert_eq!(
            crate::corrections::apply(source.as_bytes(), &found)
                .unwrap()
                .bytes,
            source.as_bytes()
        );
    }

    #[test]
    fn line_observations_directives_and_signature_heredocs_withhold_only_edits() {
        for source in [
            "def f = [\n  __LINE__]\n",
            "def f = [\n  1]\np __LINE__\n",
            "def f(a = <<~TEXT) = [\nhello\nTEXT\n  a]\n",
            "def f(a = <<~OUT) = [\n#{<<~INNER}\nhi\nINNER\nOUT\n a]\n",
            "def f = [ # rustocop:disable Layout\n  1]\n",
            "def f = [\n  1] # rubocop:disable Layout\n",
            "def f() = # keep me\n  [1]\n",
            "def f( # keep me\n) = [\n  1]\n",
            "def f() \\\n = [\n  1]\n",
        ] {
            assert_withheld(source, 1);
        }
        crate::cops::testing::assert_correction(
            &super::EndlessMethod,
            "p __LINE__\ndef f = [\n  1]\n",
            "p __LINE__\ndef f\n  [\n  1]\nend\n",
        );
    }

    #[test]
    fn qualified_data_references_withhold_edits_without_changing_detection() {
        for reference in [
            "DATA",
            "::DATA",
            "Object::DATA",
            "::Object::DATA",
            "Root::Object::DATA",
            "namespace::DATA",
        ] {
            let source = format!(
                "def f = [\n  1]\np [{reference}.pos, {reference}.lineno, {reference}.read]\n__END__\nraw\n"
            );
            assert_withheld(&source, 1);
            let found = offenses(&source);
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 2));
            assert_eq!(
                found[0].message,
                "Avoid endless method definitions with multiple lines."
            );
        }
    }

    #[test]
    fn inline_contexts_and_nested_definitions_defer_uncertain_edits() {
        assert_withheld("value = def f = [\n  1]\n", 1);
        assert_withheld("consume(def f = [\n  1])\n", 1);
        let source = "def outer = def inner = [\n  1]\n";
        let found = crate::cops::testing::correction_offenses(&super::EndlessMethod, source);
        assert_eq!(found.len(), 2);
        assert!(found[0].correction.is_some());
        assert!(found[1].correction.is_none());
        crate::cops::testing::assert_correction(
            &super::EndlessMethod,
            source,
            "def outer\n  def inner = [\n  1]\nend\n",
        );
    }

    #[test]
    fn magic_comments_are_not_moved_out_of_header_positions() {
        assert_withheld("def f = [ # frozen_string_literal: true\n  'value']\n", 1);
        assert_withheld("def f = [ # coding: UTF-8\n  'value']\n", 1);
    }

    #[test]
    fn flags_multiline_endless_definitions() {
        for signature in [
            "my_method()",
            "self.my_method()",
            "self::my_method()",
            "my_method(a, b)",
        ] {
            let source = format!("  def {signature} = x.foo\n    .bar\n    .baz\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 3, 3));
            assert_eq!(
                found[0].message,
                "Avoid endless method definitions with multiple lines."
            );
            assert!(found[0].correction.is_none());
        }
        let found = offenses("def my_method() = begin\n  foo && bar\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 3));
    }

    #[test]
    fn exempts_heredocs_anywhere_in_the_body() {
        for literal in [
            "<<~TEXT",
            "<<-TEXT",
            "<<TEXT",
            "<<~'TEXT'",
            "<<~\"TEXT\"",
            "<<~`TEXT`",
        ] {
            for body in [
                format!("{literal}\nhello\nTEXT\n"),
                format!("puts({literal},\nhello\nTEXT\n  other)\n"),
                format!("[{literal},\nhello #{{world}}\nTEXT\n  other]\n"),
                format!("begin\n  {literal}\nhello\nTEXT\nend\n"),
                format!("proc {{\n  {literal}\nhello\nTEXT\n}}\n"),
                format!("def nested\n  {literal}\nhello\nTEXT\nend\n"),
            ] {
                let source = format!("def my_method = {body}");
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
    }

    #[test]
    fn counts_signature_lines_and_ignores_heredocs_outside_the_body() {
        for source in [
            "def my_method(\n  a,\n  b: 1\n) = a\n",
            "def my_method(a = <<~TEXT) = [\nhello\nTEXT\n  a]\n",
            "value = <<~TEXT\nhello\nTEXT\ndef my_method = [\n  value]\n",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
        }
    }

    #[test]
    fn parameter_heredoc_interpolation_is_not_a_body_descendant() {
        let source = "def foo(a = <<~OUT) = [\n#{<<~INNER}\nhi\nINNER\nOUT\n a]\n";
        let found = offenses(source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 6));
        assert_eq!(
            found[0].message,
            "Avoid endless method definitions with multiple lines."
        );
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn reports_visibility_receivers_and_operators_at_def_keyword() {
        for (signature, body) in [
            ("self.my_method(a = 1, *rest, k:, **kw, &block)", "a"),
            ("object.my_method(...) ", "target(...)"),
            ("+(other)", "other"),
            ("==(other)", "other"),
            ("===(other)", "other"),
            ("<=(other)", "other"),
            (">=(other)", "other"),
            ("[](index)", "index"),
        ] {
            let source =
                format!("class Example\n  private def {signature} = [\n    {body}]\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 11, 3));
            assert_eq!(
                found[0].message,
                "Avoid endless method definitions with multiple lines."
            );
        }
    }

    #[test]
    fn reports_nested_endless_definitions_independently() {
        let found = offenses("def a = def b = [\n  1]\n");
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 2));
        assert_eq!((found[1].line, found[1].col, found[1].end_line), (1, 9, 2));
        // A heredoc in the outer method must not exempt a separate inner body.
        let found = offenses("def a = [<<~TEXT, def b = [\nhello\nTEXT\n  1]]\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 19));
    }

    #[test]
    fn checks_multiline_literals_and_blocks_but_not_lookalike_text() {
        for body in [
            "\"first\nsecond\"",
            "\"#{value}\nsecond\"",
            "`first\nsecond`",
            "/first\nsecond/",
            "%w[first\nsecond]",
            "proc {\n  value\n}",
            "foo(\n  \"<<~TEXT\")",
            "foo(\n  # <<~TEXT\n  value)",
            "[\n  __LINE__]",
        ] {
            let source = format!("def my_method = {body}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
        }
        assert!(offenses("# def a = [\n\"def a = [\\n 1]\"\n").is_empty());
    }

    #[test]
    fn ruby_gate_uses_the_real_matcher() {
        use crate::{
            config::ResolvedConfig,
            cops::{Cop, Registry, policy::RubyVersion, select_active_for_version},
            pipeline::lint_str_with,
        };
        let registry = Registry::new();
        let config = ResolvedConfig::default();
        let only = ["Style/EndlessMethod".to_string()];
        assert!(super::EndlessMethod.safe_fixes());
        assert!(super::EndlessMethod.default_enabled());
        for (version, count) in [
            (RubyVersion(2, 7), 0),
            (RubyVersion(3, 0), 1),
            (RubyVersion(3, 4), 1),
        ] {
            let active = select_active_for_version(&registry, &config, &only, &[], version);
            assert_eq!(lint_str_with(&active, "def foo = [\n  1]\n").len(), count);
        }
    }

    #[test]
    fn accepts_single_line_endless_and_ordinary_definitions() {
        for source in [
            "def my_method() = x\n",
            "private def my_method = x\n",
            "def my_method=(arg)\n  arg.foo\n    .bar\nend\n",
            "def self.my_method=(arg); @value = arg; end\n",
            "def []=(index, value)\n  @values[index] = value\nend\n",
            "def my_method(a = 1)\n  a\nend\n",
            "def my_method\nend\n",
            "def self.my_method() = x\n",
            "def self::my_method() = x\n",
            "def my_method(a, b) = x\n",
            "def my_method\n  x\nend\n",
            "def my_method; x; end\n",
            "def my_method\n  x.foo\n    .bar\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
