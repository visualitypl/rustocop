//! `Style/DocumentDynamicEvalDefinition` — document interpolated eval strings.
//!
//! Matching follows RuboCop's cop of the same name (MIT License,
//! Copyright (c) 2012-26 Bozhidar Batsov); see tools/oracle/README.md.

use std::ops::Range;

use regex::bytes::RegexBuilder;
use ruby_prism::{InterpolatedStringNode, Node, StringNode};

use super::is_ruby_whitespace;

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};

pub struct DocumentDynamicEvalDefinition;

impl Cop for DocumentDynamicEvalDefinition {
    fn name(&self) -> &'static str {
        "Style/DocumentDynamicEvalDefinition"
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
        if call.is_safe_navigation()
            || !matches!(
                call.name().as_slice(),
                b"eval" | b"class_eval" | b"module_eval" | b"instance_eval"
            )
        {
            return;
        }
        let Some(argument) = call
            .arguments()
            .and_then(|args| args.arguments().iter().next())
        else {
            return;
        };
        let Some(string) = argument.as_interpolated_string_node() else {
            return;
        };
        if !string
            .parts()
            .iter()
            .any(|part| part.as_embedded_statements_node().is_some())
        {
            return;
        }
        if inline_docs(&string, file) || block_docs(&string, node, file) {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Add a comment block showing its appearance if interpolated.".into(),
            line,
            col,
            end_line: file.line_of(selector.end_offset() - 1),
            correction: None,
        });
    }
}

// RuboCop intentionally checks the physical source line, not Ruby comment
// tokens: a comment *inside* the eval string (or even a quoted #) counts too.
fn comment_start(bytes: &[u8]) -> Option<usize> {
    bytes
        .iter()
        .enumerate()
        .position(|(i, &byte)| byte == b'#' && bytes.get(i + 1) != Some(&b'{'))
}

fn inline_docs(string: &InterpolatedStringNode<'_>, file: &SourceFile) -> bool {
    let mut previous_line = None;
    for part in string.parts().iter() {
        if part.as_embedded_statements_node().is_none() {
            continue;
        }
        let line = file.line_of(part.location().start_offset());
        if previous_line == Some(line) {
            continue;
        }
        if !file.line_has_non_interpolation_hash(line) {
            return false;
        }
        previous_line = Some(line);
    }
    true
}

// Only heredocs accept block documentation. The candidate text is local to
// this heredoc and the send's physical line span, never the enclosing scope.
fn block_docs(string: &InterpolatedStringNode<'_>, call: &Node<'_>, file: &SourceFile) -> bool {
    if !string
        .opening_loc()
        .is_some_and(|loc| loc.as_slice().starts_with(b"<<"))
    {
        return false;
    }
    let Some(closing) = string.closing_loc() else {
        return false;
    };
    let Some(first) = string.parts().iter().next() else {
        return false;
    };
    let mut blocks = Vec::new();
    let mut previous = None;
    for line in file.line_of(first.location().start_offset())..file.line_of(closing.start_offset())
    {
        merge_comment(file.line_text(line), line, &mut previous, &mut blocks);
    }
    previous = None;
    let start_line = file.line_of(call.location().start_offset());
    let end_line = file.line_of(ast::source_end(call).saturating_sub(1));
    let comments = file.comments();
    let start = comments.partition_point(|c| c.start < file.line_start(start_line));
    let end = comments.partition_point(|c| c.start < file.line_start(end_line + 1));
    for comment in &comments[start..end] {
        merge_comment(
            &file.bytes[comment.clone()],
            file.line_of(comment.start),
            &mut previous,
            &mut blocks,
        );
    }
    if blocks.is_empty() {
        return false;
    }

    let mut pattern = String::new();
    let mut pending: Option<(Range<usize>, bool)> = None;
    for part in string.parts().iter() {
        if let Some(string) = part.as_string_node() {
            heredoc_static_children(&string, |range, ends_in_newline| {
                // The reference also joins adjacent static children when the
                // previous decoded value has no final newline. This matters
                // for squiggly heredocs, whose Prism parts are per-line.
                if let Some((previous, newline)) = pending.as_mut()
                    && !*newline
                {
                    previous.end = range.end;
                    *newline = ends_in_newline;
                } else {
                    if let Some((previous, _)) = pending.take() {
                        source_pattern(&file.bytes[previous], &mut pattern);
                    }
                    pending = Some((range, ends_in_newline));
                }
            });
            continue;
        }
        if let Some((previous, _)) = pending.take() {
            source_pattern(&file.bytes[previous], &mut pattern);
        }
        if part.as_embedded_statements_node().is_some() {
            pattern.push_str(".+"); // not newline-matching, just like upstream
        } else {
            let source = part.location().as_slice();
            let source = source.strip_prefix(b"#").unwrap_or(source);
            source_pattern(source, &mut pattern);
        }
    }
    if let Some((previous, _)) = pending {
        source_pattern(&file.bytes[previous], &mut pattern);
    }
    // Only escaped bytes and fixed operators enter this expression. The regex
    // engine is non-backtracking; construction/search are bounded by the local
    // argument and documentation, including on repetitive near-matches. Lift
    // the generic 10 MiB compiled-size cutoff so large valid docs are not misses.
    let regexp = RegexBuilder::new(&pattern)
        .unicode(false)
        .size_limit(usize::MAX)
        .build()
        .expect("escaped eval documentation pattern");
    blocks.iter().any(|block| regexp.is_match(block)) || regexp.is_match(&blocks.concat())
}

// Port of ruby/prism's Parser translation (MIT, Copyright 2022-present,
// Shopify Inc.; full notice in vendor/ruby-prism-sys/LICENSE.md).
// Raw physical lines ending in an odd number of backslashes form one static
// child. Decoded lines determine which children exist, but their *raw* source
// supplies the pattern.
// In particular, a trailing continuation-only child has no decoded line and
// must be omitted even when Prism coalesces it with earlier nonempty content.
fn heredoc_static_children(string: &StringNode<'_>, mut emit: impl FnMut(Range<usize>, bool)) {
    let source = string.location().as_slice();
    let decoded = string.unescaped();
    if !source.contains(&b'\n') {
        if !decoded.is_empty() {
            emit(
                string.location().start_offset()..string.location().end_offset(),
                decoded.ends_with(b"\n"),
            );
        }
        return;
    }

    let mut decoded_lines = decoded.split_inclusive(|&byte| byte == b'\n');
    let mut start = string.location().start_offset();
    let mut length = 0;
    let mut decoded_count = 1;
    let mut lines = source.split_inclusive(|&byte| byte == b'\n').peekable();
    while let Some(line) = lines.next() {
        length += line.len();
        // An escaped `\n` adds a decoded line, but not a new source child.
        let mut backslashes = 0;
        let mut extra_lines = 0;
        for &byte in line {
            if byte == b'n' && backslashes % 2 == 1 {
                extra_lines += 1;
            }
            backslashes = if byte == b'\\' { backslashes + 1 } else { 0 };
        }
        if !line.ends_with(b"\n") && extra_lines > 0 {
            extra_lines -= 1;
        }
        decoded_count += extra_lines;
        let without_newline = line.strip_suffix(b"\n").unwrap_or(line);
        let without_newline = without_newline
            .strip_suffix(b"\r")
            .unwrap_or(without_newline);
        let continued = without_newline
            .iter()
            .rev()
            .take_while(|&&b| b == b'\\')
            .count()
            % 2
            == 1;
        if continued && lines.peek().is_some() {
            continue;
        }

        let mut ends_in_newline = false;
        for _ in 0..decoded_count {
            let Some(decoded_line) = decoded_lines.next() else {
                return;
            };
            ends_in_newline = decoded_line.ends_with(b"\n");
        }
        emit(start..start + length, ends_in_newline);
        start += length;
        length = 0;
        decoded_count = 1;
    }
}

fn merge_comment(bytes: &[u8], line: u32, previous: &mut Option<u32>, blocks: &mut Vec<Vec<u8>>) {
    let text = &bytes[bytes
        .iter()
        .position(|&b| !is_ruby_whitespace(b))
        .unwrap_or(bytes.len())..];
    if !text.starts_with(b"#") || text.get(1) == Some(&b'{') {
        return;
    }
    if previous.is_some_and(|last| last + 1 == line) {
        let block = blocks.last_mut().unwrap();
        block.push(b'\n');
        block.extend_from_slice(&text[1..]);
    } else {
        blocks.push(text[1..].to_vec());
    }
    *previous = Some(line);
}

fn blank(bytes: &[u8]) -> bool {
    bytes.iter().all(|&b| is_ruby_whitespace(b))
}

// Port the reference's raw-source matching (not decoded Ruby string values):
// strip comments, trim each static part, permit indentation, and let escaped
// interpolation markers appear either escaped or unescaped in the example.
fn source_pattern(source: &[u8], pattern: &mut String) {
    if blank(source) {
        pattern.push_str(r"\s+");
        return;
    }
    let mut stripped = Vec::with_capacity(source.len());
    let mut cursor = 0;
    while let Some(relative) = comment_start(&source[cursor..]) {
        let hash = cursor + relative;
        let mut start = hash;
        while start > cursor && is_ruby_whitespace(source[start - 1]) {
            start -= 1;
        }
        stripped.extend_from_slice(&source[cursor..start]);
        cursor = source[hash..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(source.len(), |n| hash + n);
    }
    stripped.extend_from_slice(&source[cursor..]);
    if blank(&stripped) {
        return;
    }
    let start = stripped
        .iter()
        .position(|&b| !is_ruby_whitespace(b) && b != 0)
        .unwrap_or(stripped.len());
    let end = stripped
        .iter()
        .rposition(|&b| !is_ruby_whitespace(b) && b != 0)
        .map_or(start, |n| n + 1);
    let source = &stripped[start..end];
    pattern.push_str(r"\s*");
    let mut i = 0;
    while i < source.len() {
        if source[i..].starts_with(b"\\#") {
            pattern.push_str(r"\\?#");
            i += 2;
        } else {
            // Hex literals keep arbitrary source encodings lossless and escape
            // all regex metacharacters without interpreting source as UTF-8.
            use std::fmt::Write;
            write!(pattern, "\\x{:02x}", source[i]).unwrap();
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's
    //! `spec/rubocop/cop/style/document_dynamic_eval_definition_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    const MESSAGE: &str = "Add a comment block showing its appearance if interpolated.";

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/DocumentDynamicEvalDefinition")
            .collect()
    }

    #[test]
    fn inline_docs_are_required_on_each_interpolation_start_line() {
        for source in [
            "class_eval <<-EOT\n  def #{name} # def foo\n    to_str.#{name} # to_str.foo\n  end\nEOT\n",
            "stringio.instance_eval(\"def original_filename; 'stringio#{n}.txt'; end # example\")",
            "eval(\"#{code}\") # documentation outside the string also counts",
            "eval \"#{x} #{y} # example\"",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "eval \"#{x}\\n#{y}\"",
            "eval <<~EOT\n  #{x} # x\n  #{y}\nEOT\n",
            "eval \"#{\n code\n} # not on the start line\"",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }

    #[test]
    fn matching_comment_blocks_inside_or_within_the_call_document_heredocs() {
        let code = "  def #{name}(*params, &block)\n    to_str.#{name}(*params, &block)\n  end\n";
        let docs = "  # def capitalize(*params, &block)\n  #   to_str.capitalize(*params, &block)\n  # end\n";
        for source in [
            format!("class_eval <<-EOT\n{docs}\n{code}EOT\n"),
            format!("class_eval(\n{docs}\n  <<~EOT\n{code}EOT\n)\n"),
            format!("class_eval(\n  <<~EOT\n{code}EOT\n{docs})\n"),
            format!(
                "class_eval <<~EOT\n{}\n{code}EOT\n",
                docs.replace("# def", "# EXAMPLE: def")
            ),
            format!(
                "class_eval <<~EOT\n{docs}\n{}EOT\n",
                code.replace(
                    "to_str.#{name}(*params, &block)",
                    "to_str.#{name}(*params, &block) # {{ note: etc. }}"
                )
            ),
        ] {
            assert!(offenses(&source).is_empty(), "{source}");
        }
        for source in [
            format!(
                "class_eval <<~EOT\n{}\n{code}EOT\n",
                docs.replace("to_str", "str")
            ),
            format!("{docs}class_eval <<~EOT\n{code}EOT\n"), // outside send range
            format!("class_eval <<~EOT\n{code}EOT\n{docs}"),
        ] {
            assert_eq!(offenses(&source).len(), 1, "{source}");
        }
    }

    #[test]
    fn joins_split_blocks_for_multiple_dynamic_methods() {
        let first = "  def #{name}(*params, &block)\n    to_str.#{name}(*params, &block)\n  end\n";
        let second = "  def #{name}!(*params)\n    @dirty = true\n    super\n  end\n";
        let docs1 = "  # def capitalize(*params, &block)\n  #   to_str.capitalize(*params, &block)\n  # end\n";
        let docs2 = "  # def capitalize!(*params)\n  #   @dirty = true\n  #   super\n  # end\n";
        for source in [
            format!("class_eval <<-EOT\n{docs1}  #\n{docs2}\n{first}\n{second}EOT\n"),
            format!("class_eval <<-EOT\n{docs1}{first}\n{docs2}{second}EOT\n"),
            format!("class_eval(\n{docs1}  #\n{docs2}\n  <<-EOT\n{first}\n{second}EOT\n)\n"),
        ] {
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }

    #[test]
    fn escaped_interpolation_can_show_source_or_runtime_spelling_in_block_docs() {
        for escaped in ["#{interpolation}", "\\#{interpolation}"] {
            let source = format!(
                "class_eval(\n  # def something\n  #   puts \"escaped {escaped}\"\n  # end\n  <<~EOT, __FILE__, __LINE__ + 1\n    def #{{my_method}}\n      puts \"escaped \\#{{interpolation}}\"\n    end\n  EOT\n)\n"
            );
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }

    #[test]
    fn ignores_nonmatching_calls_and_literal_shapes() {
        for source in [
            "class_eval <<-EOT\n  def capitalize; end\nEOT\n",
            "class_eval { define_method(name) {} }",
            "eval",
            "eval(code)",
            "eval(:\"#{name}\")",
            "eval(/#{name}/)",
            "eval('#{name}')",
            "eval %q(#{name})",
            "eval(\"\\#{name}\")",
            "eval <<'EOT'\n#{name}\nEOT\n",
            "eval \"#@name #$name\"", // only #{...} is a Parser begin child
            "eval \"#{name}\" \"suffix\"", // concatenation has nested dstr children
            "eval(1, \"#{name}\")",
            "eval(*[\"#{name}\"])",
            "object&.class_eval(\"#{name}\")", // upstream only hooks on_send
            "evaluate(\"#{name}\")",
            "define_method(name) {}",
            "# eval \"#{name}\"\n'instance_eval'\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn receiver_names_binding_visibility_and_enclosing_docs_do_not_exempt_calls() {
        for method in ["eval", "class_eval", "module_eval", "instance_eval"] {
            for receiver in [
                "",
                "self.",
                "Kernel.",
                "::Kernel.",
                "Other::Kernel.",
                "object.",
            ] {
                for wrapper in [
                    ("", ""),
                    (
                        "# Class docs\nclass A\n  private\n  def call\n",
                        "\n  end\nend\n",
                    ),
                    ("module M\nclass A\nclass << self\n", "\nend\nend\nend\n"),
                    ("class A; end\nclass A\ndef self.call\n", "\nend\nend\n"),
                    ("Class.new do\n define_method(:call) do\n", "\nend\nend\n"),
                ] {
                    let source = format!(
                        "{}{receiver}{method}(\"#{{name}}\"){}",
                        wrapper.0, wrapper.1
                    );
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!(
                        (found[0].line, found[0].col),
                        (
                            wrapper.0.lines().count() as u32 + 1,
                            receiver.len() as u32 + 1
                        )
                    );
                    assert_eq!(found[0].message, MESSAGE);
                }
            }
        }
        // Interpolation is inspected as AST, never evaluated or expanded.
        assert_eq!(offenses("eval(\"#{raise 'must not execute'}\")").len(), 1);
        assert_eq!(offenses("eval(\"#{ }\")").len(), 1);
        assert_eq!(offenses("eval = code; eval\neval(\"#{eval}\")").len(), 1);
    }

    #[test]
    fn matching_is_raw_source_not_decoded_values_or_regex_syntax() {
        for opener in ["<<EOT", "<<-EOT", "<<~EOT"] {
            for (literal, doc, count) in [
                ("[#{name}].*", "[foo].*", 0),
                ("[#{name}].*", "fooooo", 1),
                ("#{name}\\nend", "foo\\nend", 0),
                ("#{name}\\nend", "foo end", 1),
                ("def #{name}; # ignored", "def foo;", 0),
                ("#{name}", "", 1), // .+ requires a nonempty example
            ] {
                let source = format!("eval {opener}\n# {doc}\n{literal}\nEOT\n");
                assert_eq!(offenses(&source).len(), count, "{source}");
            }
        }
        // Inline docs deliberately accept arbitrary # text, even inside a string.
        assert!(offenses("eval(\"#{name}\", \"# not a Ruby comment\")").is_empty());
        // Interpolation's nested strings must not be mistaken for first arguments.
        assert_eq!(offenses("eval(\"#{eval('literal')}\")").len(), 1);
    }

    #[test]
    fn squiggly_heredoc_drops_empty_decoded_parts_like_the_reference_ast() {
        let source = "eval <<~EOT\n# fooend\n#{name}\\\nend\nEOT\n";
        assert!(offenses(source).is_empty());
        // Plain heredocs coalesce this static part, so upstream's raw-source
        // regexp still includes the line continuation. Do not decode all text.
        assert_eq!(offenses(&source.replace("<<~", "<<-")).len(), 1);
    }

    #[test]
    fn heredoc_continuations_preserve_reference_static_child_boundaries() {
        for opener in ["<<EOT", "<<-EOT", "<<~EOT"] {
            for newline in ["\n", "\r\n"] {
                let source = format!("eval {opener}\n# foo\\\n# end\n#{{name}}\\\nend\nEOT\n")
                    .replace('\n', newline);
                let found = offenses(&source);
                // Squiggly heredocs have a separate empty decoded child after
                // the interpolation, which the reference drops entirely.
                if opener == "<<~EOT" {
                    assert!(found.is_empty(), "{source}");
                } else {
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col), (1, 1));
                    assert_eq!(found[0].message, MESSAGE);
                }
            }
        }
    }

    #[test]
    fn heredoc_omits_empty_decoded_children_inside_coalesced_prism_parts() {
        for opener in ["<<EOT", "<<-EOT", "<<~EOT"] {
            for newline in ["\n", "\r\n"] {
                let source = format!("eval {opener}\n# foo end\n\\\n#{{name}}\n  end\nEOT\n")
                    .replace('\n', newline);
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
    }

    #[test]
    fn source_work_stays_local_for_many_calls_and_comment_near_matches() {
        let offenses = |source: &str| {
            crate::pipeline::lint_str_with(&[&DocumentDynamicEvalDefinition], source)
        };
        let source = format!("{} # example\n", "eval(\"#{name}\"); ".repeat(4000));
        assert!(offenses(&source).is_empty());
        let call = "eval(\n# def foo; end\n<<~EOT\ndef #{name}; end\nEOT\n)\n";
        let source = format!("{}{}", "# unrelated\n".repeat(10000), call.repeat(500));
        assert!(offenses(&source).is_empty());
        // Many wildcard choices must not trigger regex backtracking explosions.
        let source = format!(
            "eval <<~EOT\n# {}X\n{}Y\nEOT\n",
            "a".repeat(10000),
            "#{name}a".repeat(100)
        );
        assert_eq!(offenses(&source).len(), 1);
    }

    #[test]
    fn reports_selector_for_undocumented_interpolated_eval() {
        for (source, line, col) in [
            (
                "class_eval <<-EOT, __FILE__, __LINE__ + 1\n  def #{unsafe_method}(*params, &block)\n    to_str.#{unsafe_method}(*params, &block)\n  end\nEOT\n",
                1,
                1,
            ),
            (
                "stringio.instance_eval(\"def original_filename; 'stringio#{n}.txt'; end\")",
                1,
                10,
            ),
            ("# heading\n  Kernel.eval(\"#{code}\", binding)", 2, 10),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, line)
            );
            assert_eq!(found[0].message, MESSAGE);
            assert!(found[0].correction.is_none());
        }
    }
}
