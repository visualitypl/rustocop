//! `Style/SingleLineMethods` — allow empty and endless definitions.
use crate::{
    cops::{Companions, Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct SingleLineMethods {
    endless: bool,
}
impl Default for SingleLineMethods {
    fn default() -> Self {
        Self { endless: true }
    }
}
impl Cop for SingleLineMethods {
    fn name(&self) -> &'static str {
        "Style/SingleLineMethods"
    }
    /// Upstream's Style/AccessModifierDeclarations lists this cop as
    /// autocorrect-incompatible and runs first.
    fn autocorrect_deferred_to(&self) -> &'static [&'static str] {
        &["Style/AccessModifierDeclarations"]
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn with_companions(&self, companions: &Companions) -> Option<Box<dyn Cop>> {
        Some(Box::new(Self {
            endless: companions.endless_method,
        }))
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
        if def.equal_loc().is_some() || def.body().is_none() {
            return;
        }
        let (line, col) = file.line_col(def.location().start_offset());
        if line != file.last_line_of(def.location()) {
            return;
        }
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Avoid single-line method definitions.".into(),
            line,
            col,
            end_line: line,
            correction: cx
                .corrections_requested()
                .then(|| rewrite(node, cx, file, self.endless))
                .flatten(),
        });
    }
}

fn rewrite(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    endless: bool,
) -> Option<Correction> {
    let def = node.as_def_node()?;
    let start = def.location().start_offset();
    let end = def.location().end_offset();
    if !super::correction::method_newline_safe(node, cx, file, start..end) {
        return None;
    }
    let line = file.line_of(start);
    let line_start = file.line_start(line);
    let prefix = &file.bytes[line_start..start];
    let prefix = prefix.strip_prefix(b"\xef\xbb\xbf").unwrap_or(prefix);
    let indent_prefix = [b"private ".as_slice(), b"protected ", b"public "]
        .iter()
        .find_map(|v| prefix.strip_suffix(*v))
        .unwrap_or(prefix);
    if !indent_prefix.iter().all(|b| matches!(b, b' ' | b'\t')) {
        return None;
    }
    let declaration_start = start - (prefix.len() - indent_prefix.len());
    let indent = std::str::from_utf8(indent_prefix).ok()?;
    let newline = if file.bytes[line_start..file.line_start(line + 1)].ends_with(b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let comments = file.comments();
    let first = comments.partition_point(|c| c.start < end);
    let comment = comments
        .get(first)
        .filter(|c| file.line_of(c.start) == line);
    let suffix_end = comment.map_or(file.line_start(line + 1), |c| c.start);
    if !file.bytes[end..suffix_end]
        .iter()
        .all(u8::is_ascii_whitespace)
    {
        return None;
    }
    let body = def.body()?;
    let statement = body
        .as_statements_node()
        .filter(|s| s.body().len() == 1)
        .and_then(|s| s.body().iter().next());
    if endless
        && let Some(statement) = statement.as_ref()
        && let Some(body_source) = endless_body(statement, file)
        && !(def.name().as_slice().ends_with(b"=")
            && !matches!(
                def.name().as_slice(),
                b"==" | b"===" | b"!=" | b"<=" | b">="
            ))
    {
        let signature_end = def
            .rparen_loc()
            .map(|l| l.end_offset())
            .or_else(|| def.parameters().map(|p| p.location().end_offset()))
            .unwrap_or(def.name_loc().end_offset());
        let name_end = def.name_loc().end_offset();
        let header = std::str::from_utf8(&file.bytes[start..name_end]).ok()?;
        let args = if def.lparen_loc().is_some() {
            std::str::from_utf8(&file.bytes[name_end..signature_end])
                .ok()?
                .to_string()
        } else if let Some(parameters) = def.parameters() {
            format!(
                "({})",
                std::str::from_utf8(parameters.location().as_slice()).ok()?
            )
        } else {
            "()".into()
        };
        return Some(Correction {
            range: start..end,
            replacement: format!("{header}{args} = {body_source}"),
        });
    }
    let mut parts: Vec<usize> = if let Some(statements) = body.as_statements_node() {
        statements
            .body()
            .iter()
            .map(|n| n.location().start_offset())
            .collect()
    } else {
        vec![body.location().start_offset()]
    };
    parts.push(def.end_keyword_loc()?.start_offset());
    let mut edits = Vec::new();
    for (i, offset) in parts.iter().enumerate() {
        edits.push(Correction {
            range: *offset..*offset,
            replacement: format!(
                "{newline}{indent}{}",
                if i + 1 == parts.len() { "" } else { "  " }
            ),
        });
    }
    if let Some(comment) = comment {
        // Prism includes CR in a CRLF comment range. Leave it with the original
        // LF rather than moving it and duplicating CR in the new line ending.
        let comment =
            comment.start..comment.end - usize::from(file.bytes[comment.clone()].ends_with(b"\r"));
        let text = std::str::from_utf8(&file.bytes[comment.clone()]).ok()?;
        if !super::correction::comment_relocation_safe(text.as_bytes()) {
            return None;
        }
        edits.push(Correction {
            // Keep a visibility wrapper attached to its `def`: inserting at
            // `def` would turn `private def` into a scope-changing `private`.
            range: declaration_start..declaration_start,
            replacement: format!("{text}{newline}{indent}"),
        });
        edits.push(Correction {
            range: comment,
            replacement: String::new(),
        });
    }
    super::correction::combine(file, edits)
}
// A deliberately narrow structural proof. Unsupported bodies still receive
// the canonical multiline correction; command/assignment/low-precedence shapes
// never get blindly pasted after `=`.
fn endless_body(node: &Node<'_>, file: &SourceFile) -> Option<String> {
    if !safe_endless_body(node) {
        return None;
    }
    if let Some(call) = node.as_call_node() {
        let source = std::str::from_utf8(call.location().as_slice()).ok()?;
        if call.opening_loc().is_some()
            || call.arguments().is_none()
            || matches!(
                call.name().as_slice(),
                b"+" | b"-"
                    | b"*"
                    | b"/"
                    | b"%"
                    | b"**"
                    | b"<<"
                    | b">>"
                    | b"&"
                    | b"|"
                    | b"^"
                    | b"=="
                    | b"==="
                    | b"!="
                    | b"<"
                    | b">"
                    | b"<="
                    | b">="
                    | b"<=>"
                    | b"=~"
                    | b"!~"
                    | b"[]"
                    | b"[]="
            )
            || call.name().as_slice().ends_with(b"=")
        {
            return Some(source.into());
        }
        let message_end = call.message_loc()?.end_offset();
        let prefix =
            std::str::from_utf8(&file.bytes[call.location().start_offset()..message_end]).ok()?;
        let arguments = std::str::from_utf8(&file.bytes[message_end..call.location().end_offset()])
            .ok()?
            .trim();
        return Some(format!("{prefix}({arguments})"));
    }
    Some(std::str::from_utf8(node.location().as_slice()).ok()?.into())
}

// Inspect each operand once and render only the final body. Rendering every
// prefix of a left-deep boolean chain would create quadratic temporary strings.
fn safe_endless_body(node: &Node<'_>) -> bool {
    if let Some(call) = node.as_call_node() {
        return call.block().is_none()
            && call.equal_loc().is_none()
            && !matches!(call.name().as_slice(), b"!" | b"not" | b"[]=")
            && (!call.name().as_slice().ends_with(b"=")
                || matches!(
                    call.name().as_slice(),
                    b"==" | b"===" | b"!=" | b"<=" | b">="
                ));
    }
    if let Some(and) = node.as_and_node() {
        return and.operator_loc().as_slice() == b"&&"
            && safe_endless_body(&and.left())
            && safe_endless_body(&and.right());
    }
    if let Some(or) = node.as_or_node() {
        return or.operator_loc().as_slice() == b"||"
            && safe_endless_body(&or.left())
            && safe_endless_body(&or.right());
    }
    if let Some(write) = node.as_instance_variable_write_node() {
        return write
            .value()
            .as_array_node()
            .is_some_and(|a| a.opening_loc().is_some());
    }
    if let Some(array) = node.as_array_node() {
        return array.opening_loc().is_some();
    }
    matches!(
        node,
        Node::IntegerNode { .. }
            | Node::FloatNode { .. }
            | Node::StringNode { .. }
            | Node::SymbolNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. }
            | Node::NilNode { .. }
            | Node::SelfNode { .. }
            | Node::LocalVariableReadNode { .. }
            | Node::InstanceVariableReadNode { .. }
            | Node::ConstantReadNode { .. }
    )
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/single_line_methods_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/SingleLineMethods")
            .collect()
    }
    #[test]
    fn pinned_offenses_and_empty_endless_lookalikes() {
        for source in [
            "def some_method; body end",
            "def link_to(name, url); {:name => name}; end",
            "def @table.columns; super; end",
            "private def f(x); x; end",
            "def self.[]=(k,v); @x = v; end",
            "def +(other); other; end",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col),
                (1, if source.starts_with("private") { 9 } else { 1 })
            );
            assert_eq!(found[0].message, "Avoid single-line method definitions.");
        }
        for source in [
            "def no_op; end",
            "def self.resource_class=(klass); end",
            "def @table.columns; end",
            "def NoSnakeCase\nend",
            "def f\n body\nend",
            "def f() = x",
            "def f = [\n1]\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn pinned_allow_single_line_endless_corrections() {
        for (before, after) in [
            ("def some_method; body end", "def some_method() = body"),
            (
                "def self.some_method; body end",
                "def self.some_method() = body",
            ),
            (
                "def foo.some_method; body end",
                "def foo.some_method() = body",
            ),
            (
                "def some_method; body end # comment",
                "def some_method() = body # comment",
            ),
            (
                "def some_method(a, b, c) body end",
                "def some_method(a, b, c) = body",
            ),
            (
                "def some_method a, b = nil; body end",
                "def some_method(a, b = nil) = body",
            ),
            ("def some_method; 42 end", "def some_method() = 42"),
            ("def index() head(:ok) end", "def index() = head(:ok)"),
            ("def index() head :ok end", "def index() = head(:ok)"),
            ("def some_method() body end", "def some_method() = body"),
            ("def foo; bar && baz; end", "def foo() = bar && baz"),
            ("def foo; @foo = [1, 2]; end", "def foo() = @foo = [1, 2]"),
            (
                "private def a(x); bar :baz, x; end",
                "private def a(x) = bar(:baz, x)",
            ),
        ] {
            assert_correction(&SingleLineMethods::default(), before, after);
        }
        for op in ["==", "!=", "===", "<", "<=", ">", ">=", "<=>", "=~", "!~"] {
            assert_correction(
                &SingleLineMethods::default(),
                &format!("def {op}(other) self {op} other end"),
                &format!("def {op}(other) = self {op} other"),
            );
        }
        for op in ["+", "-", "*", "/", "%", "**"] {
            assert_correction(
                &SingleLineMethods::default(),
                &format!("def foo() bar {op} baz end"),
                &format!("def foo() = bar {op} baz"),
            );
        }
    }
    #[test]
    fn pinned_low_precedence_assignment_setter_and_multi_statement_expansion() {
        for body in [
            "bar if baz",
            "bar unless baz",
            "bar while baz",
            "bar until baz",
            "bar and baz",
            "bar or baz",
            "x => Integer",
            "x in Integer",
            "begin bar end while baz",
            "a, b = bar",
            "@foo = 1, 2",
            "@foo = *bar",
            "@foo = bar baz",
            "!obj.do_something bar",
            "return bar(argument)",
        ] {
            assert_correction(
                &SingleLineMethods::default(),
                &format!("def foo; {body}; end"),
                &format!("def foo; \n  {body}; \nend"),
            );
        }
        assert_correction(
            &SingleLineMethods::default(),
            "def foo=(foo) @foo = foo end",
            "def foo=(foo) \n  @foo = foo \nend",
        );
        assert_correction(
            &SingleLineMethods::default(),
            "def some_method; foo; bar end",
            "def some_method; \n  foo; \n  bar \nend",
        );
    }
    #[test]
    fn setter_bodies_with_commands_or_unbracketed_arrays_never_become_invalid_endless_bodies() {
        for body in ["obj.x = bar baz", "obj[k] = 1, 2", "obj.x = *values"] {
            assert_correction(
                &SingleLineMethods::default(),
                &format!("def f; {body}; end\n"),
                &format!("def f; \n  {body}; \nend\n"),
            );
        }
    }
    #[test]
    fn multiline_body_literals_and_comments_are_never_reindented_or_split() {
        assert_correction(
            &SingleLineMethods { endless: false },
            "def f; a = %q(x; y); (a); end\n",
            "def f; \n  a = %q(x; y); \n  (a); \nend\n",
        );
        assert_correction(
            &SingleLineMethods { endless: false },
            "def f; return 1 rescue 2; end\n",
            "def f; \n  return 1 rescue 2; \nend\n",
        );
    }
    #[test]
    fn shebang_comment_relocation_keeps_offenses_without_changing_bytes() {
        use crate::cops::testing::correction_offenses;
        for (cop, body) in [
            (SingleLineMethods::default(), "1; 2"),
            (SingleLineMethods { endless: false }, "2"),
        ] {
            for bom in ["", "\u{feff}"] {
                for newline in ["\n", "\r\n"] {
                    for prefix in ["", "  ", "\t", "# docs\n"] {
                        for visibility in ["", "private ", "protected ", "public "] {
                            let source = format!(
                                "{bom}{prefix}{visibility}def f; {body}; end #!/usr/bin/ruby -n{newline}puts 2{newline}"
                            );
                            let found = correction_offenses(&cop, &source);
                            assert_eq!(found.len(), 1, "{source}");
                            let (line, indent) = if prefix == "# docs\n" {
                                (2, 0)
                            } else {
                                (1, prefix.len())
                            };
                            assert_eq!(
                                (found[0].line, found[0].col, found[0].end_line),
                                (line, (indent + visibility.len() + 1) as u32, line),
                                "{source}"
                            );
                            assert_eq!(found[0].message, "Avoid single-line method definitions.");
                            assert!(found[0].correction.is_none(), "{source}");
                            assert_correction(&cop, &source, &source);
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn shebang_lookalikes_and_unmoved_comments_keep_canonical_corrections() {
        for bom in ["", "\u{feff}"] {
            for newline in ["\n", "\r\n"] {
                for prefix in ["", "  ", "\tprivate ", "protected ", "public "] {
                    // A real shebang comment is harmless when endless syntax
                    // leaves it trailing, rather than promoting it above def.
                    assert_correction(
                        &SingleLineMethods::default(),
                        &format!("{bom}{prefix}def f; 2; end #!/usr/bin/ruby -n{newline}"),
                        &format!("{bom}{prefix}def f() = 2 #!/usr/bin/ruby -n{newline}"),
                    );
                }
                for (cop, body, expanded) in [
                    (
                        SingleLineMethods::default(),
                        "1; \"#!\"",
                        format!("1; {newline}  \"#!\""),
                    ),
                    (
                        SingleLineMethods { endless: false },
                        "\"#!\"",
                        "\"#!\"".into(),
                    ),
                ] {
                    for comment in ["# docs", "# docs #!/usr/bin/ruby -n"] {
                        assert_correction(
                            &cop,
                            &format!("{bom}def f; {body}; end {comment}{newline}"),
                            &format!(
                                "{bom}{comment}{newline}def f; {newline}  {expanded}; {newline}end {newline}"
                            ),
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn trailing_comment_moves_before_the_entire_visibility_declaration() {
        for visibility in ["private", "protected", "public"] {
            for newline in ["\n", "\r\n"] {
                for bom in ["", "\u{feff}"] {
                    assert_correction(
                        &SingleLineMethods::default(),
                        &format!("{bom}  {visibility} def f; 1; 2; end # docs{newline}"),
                        &format!(
                            "{bom}  # docs{newline}  {visibility} def f; {newline}    1; {newline}    2; {newline}  end {newline}"
                        ),
                    );
                }
            }
        }
    }
    #[test]
    fn pinned_multiline_corrections() {
        // Pinned examples explicitly disable EndlessMethod.
        for (before, after) in [
            (
                "def some_method; body end",
                "def some_method; \n  body \nend",
            ),
            (
                "def link_to(name, url); {:name => name}; end",
                "def link_to(name, url); \n  {:name => name}; \nend",
            ),
            (
                "def @table.columns; super; end",
                "def @table.columns; \n  super; \nend",
            ),
            (
                "def foo() (do_something) end",
                "def foo() \n  (do_something) \nend",
            ),
            (
                "  def some_method; body end # Cmnt\n",
                "  # Cmnt\n  def some_method; \n    body \n  end \n",
            ),
            (
                "  def self.some_method() body end",
                "  def self.some_method() \n    body \n  end",
            ),
            (
                "  def some_method(arg) body end",
                "  def some_method(arg) \n    body \n  end",
            ),
            (
                "  def some_method arg; body end",
                "  def some_method arg; \n    body \n  end",
            ),
            (
                "  def some_method; b1; b2; end",
                "  def some_method; \n    b1; \n    b2; \n  end",
            ),
        ] {
            assert_correction(&SingleLineMethods { endless: false }, before, after);
        }
    }
}
