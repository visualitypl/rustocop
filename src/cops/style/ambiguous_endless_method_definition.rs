//! `Style/AmbiguousEndlessMethodDefinition` — clarify outer low-precedence operations.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct AmbiguousEndlessMethodDefinition;
impl Cop for AmbiguousEndlessMethodDefinition {
    fn name(&self) -> &'static str {
        "Style/AmbiguousEndlessMethodDefinition"
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
        let Some(definition) = node.as_def_node() else {
            return;
        };
        if definition.equal_loc().is_none() {
            return;
        }
        // Prism's conditional/loop body has a transparent StatementsNode.
        // Parentheses, calls (including visibility sends), and other expressions
        // are real boundaries, not ancestors to search past.
        let Some(operation) = cx.ancestors().find(|n| n.as_statements_node().is_none()) else {
            return;
        };
        let Some(keyword) = outer_keyword(operation, node) else {
            return;
        };
        let location = operation.location();
        let (line, col) = file.line_col(location.start_offset());
        let correction = cx
            .corrections_requested()
            .then(|| rewrite(file, node, cx))
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Avoid using `{}` statements with endless methods.",
                String::from_utf8_lossy(keyword.as_slice())
            ),
            line,
            col,
            end_line: file.last_line_of(location),
            correction,
        });
    }
}
/// Match only the direct definition operand/body, never a definition in the
/// predicate or the right operand of a boolean operation.
fn outer_keyword<'pr>(
    operation: &Node<'pr>,
    definition: &Node<'pr>,
) -> Option<ruby_prism::Location<'pr>> {
    let start = definition.location().start_offset();
    let keyword = match operation {
        Node::AndNode { .. } => {
            let n = operation.as_and_node()?;
            if n.left().location().start_offset() != start {
                return None;
            }
            n.operator_loc()
        }
        Node::OrNode { .. } => {
            let n = operation.as_or_node()?;
            if n.left().location().start_offset() != start {
                return None;
            }
            n.operator_loc()
        }
        Node::IfNode { .. } => operation.as_if_node()?.if_keyword_loc()?,
        Node::UnlessNode { .. } => operation.as_unless_node()?.keyword_loc(),
        Node::WhileNode { .. } => operation.as_while_node()?.keyword_loc(),
        Node::UntilNode { .. } => operation.as_until_node()?.keyword_loc(),
        _ => return None,
    };
    (keyword.start_offset() > start
        && matches!(
            keyword.as_slice(),
            b"and" | b"or" | b"if" | b"unless" | b"while" | b"until"
        ))
    .then_some(keyword)
}

fn rewrite(file: &SourceFile, node: &Node<'_>, cx: &NodeContext<'_, '_>) -> Option<Correction> {
    let definition = node.as_def_node()?;
    let equal = definition.equal_loc()?;
    let body = definition.body()?.location();
    let location = node.location();
    let start = location.start_offset();
    let end = location.end_offset();
    let line = file.line_of(start);
    let root = cx.ancestors().last()?;
    // Expansion shifts both this method's body and later declaration/call
    // locations. Reuse ClassMethodsDefinitions' correction-only cached observer
    // walk: explicit exception/source/eval/trace observations anywhere in the
    // file make newline edits uncertain, without suppressing the diagnostic.
    // DATA's initial byte/line position also changes even if its payload does not.
    let bindings = file.class_method_bindings(root);
    if bindings.observes_sources || bindings.observes_data {
        return None;
    }
    // Multiline bodies/signatures and deferred heredocs require literal-aware
    // reindentation; retain their offense without moving any payload bytes.
    if file.last_line_of(location) != line
        || file.contains_heredoc(root, node)
        || super::correction::touches_directive(file, start..end)
    {
        return None;
    }
    let prefix = &file.bytes[file.line_start(line)..start];
    let prefix = prefix.strip_prefix(b"\xef\xbb\xbf").unwrap_or(prefix);
    // Search backward: later inline definitions stop at the preceding code,
    // never rescan a potentially huge shared indentation prefix.
    if !prefix.iter().rev().all(|b| matches!(b, b' ' | b'\t')) {
        return None;
    }
    let indent = std::str::from_utf8(prefix).ok()?;
    let header = std::str::from_utf8(&file.bytes[start..equal.start_offset()])
        .ok()?
        .trim_end_matches([' ', '\t']);
    // Equal-to-body trivia must be horizontal whitespace only: no comments or
    // continuations are silently discarded by the multiline conversion.
    if !file.bytes[equal.end_offset()..body.start_offset()]
        .iter()
        .all(|b| matches!(b, b' ' | b'\t'))
    {
        return None;
    }
    let body = std::str::from_utf8(body.as_slice()).ok()?;
    let newline = if file.bytes[file.line_start(line)..file.line_start(line + 1)].ends_with(b"\r\n")
    {
        "\r\n"
    } else {
        "\n"
    };
    let correction = Correction {
        range: start..end,
        replacement: format!("{header}{newline}{indent}  {body}{newline}{indent}end"),
    };
    super::correction::preserves_line_values(file, &correction).then_some(correction)
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/ambiguous_endless_method_definition_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/AmbiguousEndlessMethodDefinition")
            .collect()
    }
    #[test]
    fn pinned_six_operators_ordinary_and_singleton_corrections() {
        for keyword in ["and", "or", "if", "unless", "while", "until"] {
            for receiver in ["", "self."] {
                let source = format!("def {receiver}foo = true {keyword} bar\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 1));
                assert_eq!(
                    found[0].message,
                    format!("Avoid using `{keyword}` statements with endless methods.")
                );
                assert!(found[0].correction.is_none());
                assert_correction(
                    &AmbiguousEndlessMethodDefinition,
                    &source,
                    &format!("def {receiver}foo\n  true\nend {keyword} bar\n"),
                );
            }
        }
    }
    #[test]
    fn indented_bodies_parameters_operators_comments_and_rescue_stay_inside_method() {
        for (source, after) in [
            (
                "class C\n  def self.[](key) = (key.fetch(:a) rescue :fallback) and effect\nend\n",
                "class C\n  def self.[](key)\n    (key.fetch(:a) rescue :fallback)\n  end and effect\nend\n",
            ),
            (
                "def f = risky and effect rescue fallback\n",
                "def f\n  risky\nend and effect rescue fallback\n",
            ),
            (
                "def f(x = 1, key: 2, &block) = [x, key, block] if test # retain\n",
                "def f(x = 1, key: 2, &block)\n  [x, key, block]\nend if test # retain\n",
            ),
            (
                "def f(...) = true unless test\n",
                "def f(...)\n  true\nend unless test\n",
            ),
            (
                "def self.+(x) = \"café #{x}\" or effect\n",
                "def self.+(x)\n  \"café #{x}\"\nend or effect\n",
            ),
            (
                "def f = true if first unless second\n",
                "def f\n  true\nend if first unless second\n",
            ),
            (
                "p __LINE__\ndef f = true and effect\n",
                "p __LINE__\ndef f\n  true\nend and effect\n",
            ),
            (
                "\u{feff}def f = true and effect\r\n",
                "\u{feff}def f\r\n  true\r\nend and effect\r\n",
            ),
        ] {
            assert_correction(&AmbiguousEndlessMethodDefinition, source, after);
        }
    }
    #[test]
    fn line_directive_heredoc_multiline_and_inline_boundaries_withhold_individually() {
        for source in [
            "def f = __LINE__ and effect\n",
            "def f = true and effect\np __LINE__\n",
            "def f = true and effect # rubocop:disable Layout\n",
            "def f = true if condition # rustocop:disable-next Lint\np :later\n",
            "def f = <<~TEXT and effect\n  café\nTEXT\n",
            "def f = \"first\nsecond\" if condition\n",
            "def f(x =\n  1) = x if condition\n",
            // Forwarding includes an unresolved block value, not just args.
            "def f(...) = g(...) unless test\n",
            "class C; def f = true and effect; end\n",
            "puts \"#{def f = true and effect}\"\n",
        ] {
            assert_eq!(
                ruby_prism::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            let found = crate::cops::testing::correction_offenses(
                &AmbiguousEndlessMethodDefinition,
                source,
            );
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn source_metadata_observers_withhold_edits_not_offenses() {
        for observer in [
            "error.backtrace",
            "error.backtrace_locations",
            "error.full_message",
            "method(:later).source_location",
            "public_method(:later)",
            "self.singleton_method(:later)",
            "C.instance_method(:later)",
            "C.public_instance_method(:later)",
            "C.const_source_location(:X)",
            "binding",
            "caller",
            "caller_locations",
            "eval(code)",
            "class_eval(code)",
            "module_eval(code)",
            "instance_eval(code)",
            "TracePoint.new(:line) {}",
            "RubyVM::InstructionSequence.of(method(:later))",
            "set_trace_func(proc {})",
            "send(:caller)",
            "__send__(name)",
            "public_send(name)",
            "load(path)",
            "require(path)",
            "require_relative(path)",
            "File.read(__FILE__)",
        ] {
            // Observers before a candidate can install tracing or call a later
            // method; observers after it can see shifted declarations/callers.
            for source in [
                format!("{observer}\ndef f = true if condition\ndef later; end\n"),
                format!("def f = true if condition\ndef later; end\n{observer}\n"),
            ] {
                let found = crate::cops::testing::correction_offenses(
                    &AmbiguousEndlessMethodDefinition,
                    &source,
                );
                assert_eq!(found.len(), 1, "{source}");
                assert!(found[0].correction.is_none(), "{source}");
                assert_eq!(
                    found[0].message,
                    "Avoid using `if` statements with endless methods."
                );
            }
        }
        // Mere spellings in comments/literals and method declarations are not
        // calls or metadata observers. Keep the canonical correction available.
        assert_correction(
            &AmbiguousEndlessMethodDefinition,
            "# backtrace TracePoint __FILE__\ndef backtrace; end\ndef f = 'source_location eval' if true\n",
            "# backtrace TracePoint __FILE__\ndef backtrace; end\ndef f\n  'source_location eval'\nend if true\n",
        );
    }
    #[test]
    fn data_observations_withhold_expansion_not_diagnostics() {
        for observer in [
            "p [DATA.pos, DATA.lineno, DATA.read]",
            "p ::DATA.pos",
            "p Object::DATA.lineno",
            "p ::Object::DATA.read",
            "Root = Object; p Root::DATA.pos",
            "Handle = DATA; p Handle.lineno",
            "p Object.const_get(:DATA).read",
        ] {
            let source = format!("def f = true if true\n{observer}\n__END__\nraw\n");
            let found = crate::cops::testing::correction_offenses(
                &AmbiguousEndlessMethodDefinition,
                &source,
            );
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 1));
            assert_eq!(
                found[0].message,
                "Avoid using `if` statements with endless methods."
            );
            assert!(found[0].correction.is_none(), "{source}");
        }
        assert_correction(
            &AmbiguousEndlessMethodDefinition,
            "# DATA.pos\ndef f = 'DATA.lineno' if true\n__END__\nDATA.read\n",
            "# DATA.pos\ndef f\n  'DATA.lineno'\nend if true\n__END__\nDATA.read\n",
        );
    }
    #[test]
    fn multiple_findings_use_main_parse_and_local_edits() {
        let source = "def f = true and effect\n".repeat(2000);
        let found = offenses(&source);
        assert_eq!(found.len(), 2000);
        assert!(found.iter().all(|o| o.correction.is_none()));
        let found =
            crate::cops::testing::correction_offenses(&AmbiguousEndlessMethodDefinition, &source);
        assert_eq!(found.len(), 2000);
        let applied = crate::corrections::apply(source.as_bytes(), &found).unwrap();
        assert_eq!(
            applied.bytes,
            "def f\n  true\nend and effect\n".repeat(2000).as_bytes()
        );
    }
    #[test]
    fn exact_indented_position_and_ruby_applicability() {
        let found = offenses("module C\n  def foo = true if bar\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
        assert_eq!(
            found[0].message,
            "Avoid using `if` statements with endless methods."
        );
        let versions = AmbiguousEndlessMethodDefinition.ruby_versions();
        assert!(!versions.supports(crate::cops::policy::RubyVersion(2, 7)));
        assert!(versions.supports(crate::cops::policy::RubyVersion(3, 0)));
        assert!(versions.supports(crate::cops::policy::DEFAULT_RUBY_VERSION));
    }
    #[test]
    fn later_inline_definitions_do_not_copy_the_shared_indentation_prefix() {
        let source = format!(
            "{}{}\n",
            " ".repeat(20_000),
            "def f = true and effect; ".repeat(500)
        );
        let found =
            crate::cops::testing::correction_offenses(&AmbiguousEndlessMethodDefinition, &source);
        assert_eq!(found.len(), 500);
        assert_eq!(found.iter().filter(|o| o.correction.is_some()).count(), 1);
        let first = found[0].correction.as_ref().unwrap();
        assert_eq!(first.range, 20_000..20_012);
    }
    #[test]
    fn pinned_unambiguous_forms() {
        for keyword in ["and", "or", "if", "unless", "while", "until"] {
            for source in [
                format!("def foo\nend {keyword} bar\n"),
                format!("def foo = (true {keyword} bar)\n"),
                format!("(def foo = true) {keyword} bar\n"),
            ] {
                assert!(offenses(&source).is_empty(), "{source}");
            }
            if !["and", "or"].contains(&keyword) {
                let source = format!("{keyword} bar\n  def foo = true\nend\n");
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        for source in [
            "def foo\nend\n",
            "def foo = true && bar",
            "def foo = true || bar",
            "def foo = true rescue false",
            "def foo = true rescue false and effect",
            "private def foo = true if bar",
            "if (def foo = true); bar; end",
            "while (def foo = true); break; end",
            "bar and def foo = true",
            "bar or def foo = true",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
