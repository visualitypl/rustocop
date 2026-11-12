//! `Style/Documentation` — document classes and non-namespace modules.
use std::sync::LazyLock;

use regex::bytes::Regex;
use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};

// Match only directives recognized by the pinned documentation mixin, plus
// rustocop's equivalent spelling. Other tooling comments are not generalized
// into exclusions (for example, `typed:` is documentation at this pin).
static INTERPRETER_DIRECTIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^#[ \t\r\n\x0b\x0c]*(?:frozen_string_literal|encoding):")
        .expect("fixed interpreter directive pattern")
});
// Parser::Source::Comment::Associator skips these only at the beginning of
// the comment stream, in shebang/magic/encoding order (one of each). Expand
// MAGIC_COMMENT_RE's paired Emacs-marker backreference into two alternatives.
static INITIAL_MAGIC_COMMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?m)^#[ \t\r\n\x0b\x0c]*(?:",
        r"-\*-[ \t\r\n\x0b\x0c]*(?:frozen_string_literal|warn_indent|warn_past_scope):.*-\*-",
        r"|(?:frozen_string_literal|warn_indent|warn_past_scope):.*)$"
    ))
    .expect("fixed initial magic comment pattern")
});
static INITIAL_ENCODING_COMMENT: LazyLock<Regex> = LazyLock::new(|| {
    // Only existence matters here, not Buffer::ENCODING_RE's encoding captures.
    Regex::new(
        r"[ \t\r\n\x0b\x0c#](?:en)?coding[ \t\r\n\x0b\x0c]*[:=][ \t\r\n\x0b\x0c]*[A-Za-z0-9_-]+",
    )
    .expect("fixed initial encoding comment pattern")
});
static COP_DIRECTIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"#[ \t\r\n\x0b\x0c]*(?:rubocop|rustocop)[ \t\r\n\x0b\x0c]*:",
        r"[ \t\r\n\x0b\x0c]*",
        r"(?:disable-next|enable-next|todo-next|disable|enable|todo|push|pop|next)",
        // Ruby's Unicode word set excludes Join_Control (U+200C/U+200D),
        // unlike Rust's \b. Only the match start is used, so consuming the
        // delimiter preserves the pinned first-header/quoted-prefix behavior.
        r"(?:[^\p{Alphabetic}\p{M}\p{N}\p{Pc}]|$)"
    ))
    .expect("fixed cop directive pattern")
});

fn non_documentation(text: &[u8]) -> bool {
    INTERPRETER_DIRECTIVE.is_match(text)
        || COP_DIRECTIVE.find(text).is_some_and(|directive| {
            // Pinned DirectiveComment ignores a directive quoted by exactly
            // one preceding comment marker, but accepts inline prose prefixes.
            let prefix = &text[..directive.start()];
            !prefix
                .strip_prefix(b"#")
                .is_some_and(|prefix| trim_start(prefix).is_empty())
        })
}

pub struct Documentation;

impl Cop for Documentation {
    fn name(&self) -> &'static str {
        "Style/Documentation"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let (kind, name, body) = if let Some(class) = node.as_class_node() {
            let Some(body) = class.body() else { return };
            ("class", class.constant_path(), Some(body))
        } else if let Some(module) = node.as_module_node() {
            ("module", module.constant_path(), module.body())
        } else {
            return;
        };
        if body
            .as_ref()
            .is_some_and(|body| namespace(body) || mixins_only(body))
            || documentation_comment(node, cx, file)
            || nodoc(node, file, false)
            || cx.ancestors().any(|ancestor| nodoc(ancestor, file, true))
        {
            return;
        }
        let mut names: Vec<_> = cx
            .ancestors()
            .rev()
            .filter_map(definition_name)
            .map(|name| qualify(&name))
            .collect();
        names.push(qualify(&name));
        let identifier = names.join("::").replacen("::::", "::", 1);
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Missing top-level documentation comment for `{kind} {}`.",
                identifier
            ),
            line,
            col,
            end_line: file.last_line_of(name.location()),
            correction: None,
        });
    }
}

fn definition_name<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    node.as_class_node()
        .map(|class| class.constant_path())
        .or_else(|| node.as_module_node().map(|module| module.constant_path()))
}

fn qualify(node: &Node<'_>) -> String {
    if let Some(path) = node.as_constant_path_node() {
        return format!(
            "{}::{}",
            path.parent()
                .map_or_else(String::new, |parent| qualify(&parent)),
            ast::short_const_name(node).unwrap_or_default()
        );
    }
    ast::short_const_name(node)
        .unwrap_or_else(|| String::from_utf8_lossy(node.location().as_slice()).into_owned())
}

/// Per-file comment facts. Classification and the whitespace/empty-statement
/// gaps are scanned once, not once per definition or nodoc ancestor. Queries
/// are binary searches over the main parse's comment tokens.
#[derive(Default)]
pub(crate) struct DocumentationComments {
    entries: Vec<Comment>,
}

struct Comment {
    start: usize,
    line: u32,
    next_code: usize,
    documenting_group: bool,
    nodoc: bool,
    nodoc_all: bool,
}

impl DocumentationComments {
    pub(crate) fn build(file: &SourceFile) -> Self {
        let comments = file.comments();
        let mut entries: Vec<Comment> = Vec::with_capacity(comments.len());
        let mut headers = comments
            .iter()
            .map(|range| {
                comment_text(
                    &file.bytes[range.clone()],
                    file.bytes.get(range.end) == Some(&b'\n'),
                )
            })
            .peekable();
        headers.next_if(|text| text.starts_with(b"#!"));
        headers.next_if(|text| INITIAL_MAGIC_COMMENT.is_match(text));
        headers.next_if(|text| INITIAL_ENCODING_COMMENT.is_match(text));
        let skipped = comments.len() - headers.len();
        for (i, range) in comments.iter().enumerate().skip(skipped) {
            let line = file.line_of(range.start);
            let prefix = &file.bytes[file.line_start(line)..range.start];
            let prefix = prefix.strip_prefix(b"\xef\xbb\xbf").unwrap_or(prefix);
            let standalone = prefix.iter().all(|b| empty_statement_separator(*b));
            let text = comment_text(
                &file.bytes[range.clone()],
                file.bytes.get(range.end) == Some(&b'\n'),
            );
            let documenting = text.starts_with(b"#")
                && !super::comment_annotation::annotation_comment(text)
                && !non_documentation(text);
            let documenting_group = standalone
                && (documenting
                    || entries.last().is_some_and(|previous| {
                        previous.documenting_group && previous.next_code == range.start
                    }));
            let next_start = comments
                .get(i + 1)
                .map_or(file.bytes.len(), |next| next.start);
            let gap = &file.bytes[range.end..next_start];
            let next_code = range.end
                + gap
                    .iter()
                    .position(|b| !empty_statement_separator(*b))
                    .unwrap_or(gap.len());
            let suffix = text
                .strip_prefix(b"#")
                .map(trim_start)
                .and_then(|text| text.strip_prefix(b":nodoc:"));
            let nodoc_all = suffix.is_some_and(|suffix| {
                suffix
                    .first()
                    .is_some_and(|b| super::is_ruby_whitespace(*b))
                    && trim_start(suffix)
                        .strip_prefix(b"all")
                        .is_some_and(|tail| tail.iter().all(|b| super::is_ruby_whitespace(*b)))
            });
            entries.push(Comment {
                start: range.start,
                line,
                next_code,
                documenting_group,
                nodoc: suffix.is_some(),
                nodoc_all,
            });
        }
        Self { entries }
    }
}

fn comment_text(text: &[u8], followed_by_lf: bool) -> &[u8] {
    // Prism retains CR in line-comment tokens; Parser excludes it.
    // A bare `# TODO` must not become an annotation just due to CRLF.
    if text.starts_with(b"#") && followed_by_lf {
        text.strip_suffix(b"\r").unwrap_or(text)
    } else {
        text
    }
}

// Parser comment association ignores empty statements, both before a comment
// and between it and the following expression. This is NOT whitespace for
// comment text/directive parsing (notably `:nodoc: all;`).
fn empty_statement_separator(byte: u8) -> bool {
    super::is_ruby_whitespace(byte) || byte == b';'
}

pub(super) fn documentation_comment(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> bool {
    let start = node.location().start_offset();
    // Parser's :begin grouping nodes do not consume leading comments. Only
    // grouping around the first statement is transparent, not call arguments.
    let mut grouped_start = start;
    for ancestor in cx.ancestors() {
        if let Some(statements) = ancestor.as_statements_node() {
            if !statements
                .body()
                .iter()
                .next()
                .is_some_and(|first| first.location().start_offset() == grouped_start)
            {
                break;
            }
        } else if ancestor.as_parentheses_node().is_some() {
            grouped_start = ancestor.location().start_offset();
        } else {
            break;
        }
    }
    let entries = &file.documentation_comments().entries;
    let end = entries.partition_point(|comment| comment.start < start);
    // Parser associates earlier comments across blank lines, but the last
    // comment must start immediately before the definition's first line.
    end.checked_sub(1).is_some_and(|i| {
        let comment = &entries[i];
        comment.documenting_group
            && comment.next_code == grouped_start
            && comment.line + 1 == file.line_of(start)
    })
}

fn nodoc(node: &Node<'_>, file: &SourceFile, require_all: bool) -> bool {
    let start = node.location().start_offset();
    let comments = &file.documentation_comments().entries;
    let first = comments.partition_point(|comment| comment.start < start);
    comments.get(first).is_some_and(|comment| {
        comment.line == file.line_of(start)
            && if require_all {
                comment.nodoc_all
            } else {
                comment.nodoc
            }
            && nodoc_associated(node, comment, file)
    })
}

/// The pinned check queries comments on the first *Parser expression child*,
/// not on the enclosing definition itself. Prism represents grouping, statement
/// lists and literal blocks differently, so account for those here as well.
fn nodoc_associated(node: &Node<'_>, comment: &Comment, file: &SourceFile) -> bool {
    if definition_name(node).is_some() {
        // Compact names additionally query their outer constant in the pinned
        // cop, so qualified and unqualified definition headers both qualify.
        return true;
    }
    if let Some(call) = node.as_call_node()
        && call
            .block()
            .is_some_and(|block| block.as_block_node().is_some())
    {
        // Parser's block starts with the send expression. Its receiver or
        // arguments, if any, consume the decorating comment before the send.
        return call.receiver().is_none() && ast::call_arguments(&call).is_empty();
    }
    nodoc_child(node).is_some_and(|child| comment_attaches_to(&child, comment, file))
}

fn nodoc_child<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    match node {
        Node::SingletonClassNode { .. } => {
            Some(node.as_singleton_class_node().unwrap().expression())
        }
        Node::IfNode { .. } => Some(node.as_if_node().unwrap().predicate()),
        Node::UnlessNode { .. } => Some(node.as_unless_node().unwrap().predicate()),
        Node::WhileNode { .. } => Some(node.as_while_node().unwrap().predicate()),
        Node::UntilNode { .. } => Some(node.as_until_node().unwrap().predicate()),
        Node::ForNode { .. } => Some(node.as_for_node().unwrap().index()),
        Node::CaseNode { .. } => node.as_case_node().unwrap().predicate(),
        Node::CaseMatchNode { .. } => node.as_case_match_node().unwrap().predicate(),
        Node::AndNode { .. } => Some(node.as_and_node().unwrap().left()),
        Node::OrNode { .. } => Some(node.as_or_node().unwrap().left()),
        Node::RangeNode { .. } => node.as_range_node().unwrap().left(),
        Node::DefinedNode { .. } => Some(node.as_defined_node().unwrap().value()),
        Node::ArrayNode { .. } => node.as_array_node().unwrap().elements().iter().next(),
        Node::HashNode { .. } => node.as_hash_node().unwrap().elements().iter().next(),
        Node::AssocNode { .. } => Some(node.as_assoc_node().unwrap().key()),
        Node::CallNode { .. } => node.as_call_node().unwrap().receiver(),
        Node::ConstantPathNode { .. } => node.as_constant_path_node().unwrap().parent(),
        Node::StatementsNode { .. } => {
            let statements = node.as_statements_node().unwrap();
            // Single-expression statement lists have no Parser :begin node.
            (statements.body().len() > 1)
                .then(|| statements.body().iter().next())
                .flatten()
        }
        Node::ParenthesesNode { .. } => node
            .as_parentheses_node()
            .unwrap()
            .body()
            .and_then(|body| body.as_statements_node())
            .and_then(|body| body.body().iter().next()),
        Node::BeginNode { .. } => node
            .as_begin_node()
            .unwrap()
            .statements()
            .and_then(|body| body.body().iter().next()),
        // In particular, def/send/assignments with a scalar or nil first
        // Parser child cannot carry inherited nodoc via that child.
        _ => None,
    }
}

fn comment_attaches_to(node: &Node<'_>, comment: &Comment, file: &SourceFile) -> bool {
    if comment.start < node.location().start_offset() {
        return true; // Leading comment on the first expression child.
    }
    if file.last_line_of(node.location()) != comment.line {
        return false;
    }
    // Decorating comments go to expression children before their parents.
    // Parser regexps always end with a regopt child (even for //); interpolated
    // strings/symbols attach the comment to a part, not the whole literal.
    if matches!(
        node,
        Node::RegularExpressionNode { .. }
            | Node::InterpolatedRegularExpressionNode { .. }
            | Node::MatchLastLineNode { .. }
            | Node::InterpolatedMatchLastLineNode { .. }
    ) {
        return false;
    }
    if let Some(string) = node.as_interpolated_string_node() {
        return string
            .parts()
            .iter()
            .all(|part| file.last_line_of(part.location()) < comment.line);
    }
    if let Some(symbol) = node.as_interpolated_symbol_node() {
        return symbol
            .parts()
            .iter()
            .all(|part| file.last_line_of(part.location()) < comment.line);
    }
    if let Some(call) = node.as_call_node() {
        return call.receiver().is_none() && ast::call_arguments(&call).is_empty();
    }
    nodoc_child(node).is_none_or(|child| file.last_line_of(child.location()) < comment.line)
}

fn trim_start(text: &[u8]) -> &[u8] {
    &text[text
        .iter()
        .position(|b| !super::is_ruby_whitespace(*b))
        .unwrap_or(text.len())..]
}

fn constant_definition(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::ClassNode { .. }
            | Node::ModuleNode { .. }
            | Node::ConstantWriteNode { .. }
            | Node::ConstantPathWriteNode { .. }
    )
}

fn constant_visibility(node: &Node<'_>) -> bool {
    node.as_call_node().is_some_and(|call| {
        call.receiver().is_none()
            && call.block().is_none()
            && matches!(call.name().as_slice(), b"public_constant" | b"private_constant")
            && matches!(ast::call_arguments(&call).as_slice(), [argument] if matches!(argument, Node::SymbolNode { .. } | Node::StringNode { .. }))
    })
}

fn namespace(node: &Node<'_>) -> bool {
    if let Some(statements) = node.as_statements_node() {
        let children: Vec<_> = statements.body().iter().collect();
        return match children.as_slice() {
            [child] => namespace(child),
            [] => false,
            _ => children
                .iter()
                .all(|child| constant_definition(child) || constant_visibility(child)),
        };
    }
    if let Some(parens) = node.as_parentheses_node() {
        return parens
            .body()
            .and_then(|body| body.as_statements_node())
            .is_some_and(|statements| {
                statements
                    .body()
                    .iter()
                    .all(|child| constant_definition(&child) || constant_visibility(&child))
            });
    }
    constant_definition(node)
}

fn mixins_only(node: &Node<'_>) -> bool {
    if let Some(statements) = node.as_statements_node() {
        return statements.body().iter().all(|child| mixins_only(&child));
    }
    // Mirror the pinned recursive check: wrappers/containers recurse, whereas
    // scalar-bearing expressions (numbers, strings, names, ordinary calls)
    // cannot be mixin-only. No descendant walk through nested definitions.
    match node {
        Node::NilNode { .. }
        | Node::TrueNode { .. }
        | Node::FalseNode { .. }
        | Node::SelfNode { .. }
        | Node::RedoNode { .. }
        | Node::RetryNode { .. } => true,
        Node::ParenthesesNode { .. } => node
            .as_parentheses_node()
            .unwrap()
            .body()
            .is_none_or(|body| mixins_only(&body)),
        Node::BeginNode { .. } => {
            let begin = node.as_begin_node().unwrap();
            // A rescue clause always contains a nil/scalar exception target
            // in Parser's AST. An ensure has two required expression children.
            begin.rescue_clause().is_none()
                && begin.else_clause().is_none()
                && match begin.ensure_clause() {
                    Some(ensure) => {
                        begin
                            .statements()
                            .is_some_and(|body| mixins_only(&body.as_node()))
                            && ensure
                                .statements()
                                .is_some_and(|body| mixins_only(&body.as_node()))
                    }
                    None => begin
                        .statements()
                        .is_none_or(|body| mixins_only(&body.as_node())),
                }
        }
        Node::IfNode { .. } => {
            let expr = node.as_if_node().unwrap();
            mixins_only(&expr.predicate())
                && expr
                    .statements()
                    .is_some_and(|body| mixins_only(&body.as_node()))
                && expr.subsequent().is_some_and(|body| mixins_only(&body))
        }
        Node::UnlessNode { .. } => {
            let expr = node.as_unless_node().unwrap();
            mixins_only(&expr.predicate())
                && expr
                    .statements()
                    .is_some_and(|body| mixins_only(&body.as_node()))
                && expr
                    .else_clause()
                    .is_some_and(|body| mixins_only(&body.as_node()))
        }
        Node::ElseNode { .. } => node
            .as_else_node()
            .unwrap()
            .statements()
            .is_some_and(|body| mixins_only(&body.as_node())),
        Node::CaseNode { .. } => {
            let expr = node.as_case_node().unwrap();
            expr.predicate()
                .is_some_and(|predicate| mixins_only(&predicate))
                && expr.conditions().iter().all(|child| mixins_only(&child))
                && expr
                    .else_clause()
                    .is_some_and(|body| mixins_only(&body.as_node()))
        }
        Node::WhenNode { .. } => {
            let expr = node.as_when_node().unwrap();
            expr.conditions().iter().all(|child| mixins_only(&child))
                && expr
                    .statements()
                    .is_some_and(|body| mixins_only(&body.as_node()))
        }
        Node::ArrayNode { .. } => node
            .as_array_node()
            .unwrap()
            .elements()
            .iter()
            .all(|child| mixins_only(&child)),
        Node::HashNode { .. } => node
            .as_hash_node()
            .unwrap()
            .elements()
            .iter()
            .all(|child| mixins_only(&child)),
        Node::AssocNode { .. } => {
            let pair = node.as_assoc_node().unwrap();
            mixins_only(&pair.key()) && mixins_only(&pair.value())
        }
        Node::AndNode { .. } => {
            let expr = node.as_and_node().unwrap();
            mixins_only(&expr.left()) && mixins_only(&expr.right())
        }
        Node::OrNode { .. } => {
            let expr = node.as_or_node().unwrap();
            mixins_only(&expr.left()) && mixins_only(&expr.right())
        }
        Node::RangeNode { .. } => {
            let range = node.as_range_node().unwrap();
            range.left().is_some_and(|left| mixins_only(&left))
                && range.right().is_some_and(|right| mixins_only(&right))
        }
        Node::DefinedNode { .. } => mixins_only(&node.as_defined_node().unwrap().value()),
        Node::SingletonClassNode { .. } => {
            let singleton = node.as_singleton_class_node().unwrap();
            mixins_only(&singleton.expression())
                && singleton.body().is_some_and(|body| mixins_only(&body))
        }
        Node::WhileNode { .. } => {
            let expr = node.as_while_node().unwrap();
            mixins_only(&expr.predicate())
                && expr
                    .statements()
                    .is_some_and(|body| mixins_only(&body.as_node()))
        }
        Node::UntilNode { .. } => {
            let expr = node.as_until_node().unwrap();
            mixins_only(&expr.predicate())
                && expr
                    .statements()
                    .is_some_and(|body| mixins_only(&body.as_node()))
        }
        Node::CallNode { .. } => {
            let call = node.as_call_node().unwrap();
            call.receiver().is_none()
                && call.block().is_none()
                && matches!(call.name().as_slice(), b"include" | b"extend" | b"prepend")
                && matches!(ast::call_arguments(&call).as_slice(), [argument] if matches!(argument, Node::ConstantReadNode { .. } | Node::ConstantPathNode { .. }))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/documentation_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/Documentation")
            .collect()
    }

    fn assert_offense(source: &str, line: u32, col: u32, identifier: &str) {
        let found = offenses(source);
        assert_eq!(found.len(), 1, "{source}: {found:?}");
        assert_eq!((found[0].line, found[0].col), (line, col), "{source}");
        assert_eq!(found[0].end_line, line, "{source}");
        assert_eq!(
            found[0].message,
            format!("Missing top-level documentation comment for `{identifier}`.")
        );
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn exempts_empty_classes_namespaces_and_mixin_only_bodies() {
        for source in [
            "class C; end",
            "class C < Parent; end",
            "class C; # comment\nend",
            "module N; class A; end; class B; end; end",
            "class N; class A; end; class B; end; end",
            "module N; A = Class.new; B = Class.new(A); C = Class.new { call_method }; D = 1; end",
            "class N; A = 1; end",
            "module N; A::B = 1; end",
            "module N; class Private; end; private_constant :Private; end",
            "class N; A = 1; public_constant 'A'; end",
            "module Foo; include Bar; end",
            "module Foo; extend ::Bar; end",
            "module Foo; prepend A::B; end",
            "module Foo; include A; include B; extend C; prepend D; end",
            "module ::Foo; class << self; prepend A; prepend B; end; end",
            "class Foo; class << self; include A; end; end",
            "A = Class.new { def method; end }; B = Module.new { def method; end }",
            "class << self; def method; end; end",
            "def method; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for body in [
            "private_constant :A",
            "private_constant :A, :B",
            "self.private_constant :A",
            "include a",
            "include A, B",
            "self.include A",
            "extend B; include C; def foo; end",
            "private; def foo; end",
            "def self.foo; end",
            "def foo = 1",
            "belongs_to :bar",
            "if //; end",
            "class << self; def foo; end; end",
            "class << self; end",
            "class << Bar; include A; end",
            "class << self; include A; def foo; end; end",
        ] {
            assert_offense(&format!("class C\n  {body}\nend"), 1, 1, "class C");
        }
    }

    #[test]
    fn accepts_attached_documentation_but_not_annotations_or_directives() {
        for comment in [
            "# Class comment.",
            "#",
            "# TODO",
            "# FIXME",
            "# typed: strict",
            "# Note this is a sentence.",
            "# TODO: fix\n# Class comment.",
            "# Does fooing.\n# FIXME: Not yet implemented.",
            "# doc\n\n# TODO: fix",
        ] {
            for keyword in ["class", "module"] {
                let source = format!("{comment}\n{keyword} C\n  def foo; end\nend");
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        for comment in ["# TODO", "# FIXME", "# Note this is a sentence."] {
            assert!(
                offenses(&format!(
                    "{comment}\r\nclass C\r\n  def foo; end\r\nend\r\n"
                ))
                .is_empty(),
                "{comment}"
            );
        }
        for comment in [
            "# TODO: fix",
            "# OPTIMIZE: Make this faster.",
            "# note: explain",
            "# frozen_string_literal: true",
            "# encoding: ascii-8bit",
            "# rubocop:disable Style/For",
            "# rustocop:disable Style/For",
            "# Copyright 2014\n# Some company\n",
            "x = 1 # A comment",
        ] {
            let source = format!("{comment}\nclass C\n  def foo; end\nend");
            let line = comment.bytes().filter(|b| *b == b'\n').count() as u32 + 2;
            assert_offense(&source, line, 1, "class C");
        }
        assert_offense(
            "class C\n  def foo; end\n  # sparse comment\nend # decorating comment",
            1,
            1,
            "class C",
        );
        assert_offense(
            "module A # The A Module\n  class B\n    def method; end\n  end\nend",
            2,
            3,
            "class A::B",
        );
        assert_offense(
            "# documented first definition\nclass C; end\nclass C; def foo; end; end",
            3,
            1,
            "class C",
        );
        assert_offense(
            "# doc for method\ndef foo; end\nclass C; def foo; end; end",
            3,
            1,
            "class C",
        );
    }

    #[test]
    fn ruby_whitespace_keeps_standalone_comments_and_definition_gaps_attached() {
        for source in [
            "\x0b# Documentation.\nclass C; def f; end; end\n",
            "# Documentation.\n\x0bclass C; def f; end; end\n",
            "\u{feff}\x0b# Documentation.\r\n\x0bclass C; def f; end; end\r\n",
            "\x0b# Documentation.\n\x0b# TODO: fix\n\x0bclass C; def f; end; end\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
        for whitespace in [" ", "\t", "\r", "\x0b", "\x0c", " \t\x0b\x0c"] {
            let source =
                format!("{whitespace}# Documentation.\n{whitespace}class C; def f; end; end\n");
            assert!(offenses(&source).is_empty(), "{source:?}");
        }
        for prefix in ["0;", "work; ", "\u{a0}", "\u{200c}", "\u{200d}"] {
            assert_offense(
                &format!("{prefix}# Documentation.\nclass C; def f; end; end\n"),
                2,
                1,
                "class C",
            );
        }
        assert_offense(
            "# Documentation.\n\x0b0;class C; def f; end; end\n",
            2,
            4,
            "class C",
        );
        assert_offense(
            "# Documentation.\n\x0bwork; class C; def f; end; end\n",
            2,
            8,
            "class C",
        );
        assert_offense(
            "# Documentation.\n\x0b\nclass C; def f; end; end\n",
            3,
            1,
            "class C",
        );
    }

    #[test]
    fn empty_semicolon_prefixes_do_not_make_documentation_inline() {
        // Independently verified with pinned RuboCop 1.92, Prism / Ruby 3.4.
        for keyword in ["class", "module"] {
            for prefix in [";", ";; \t;", "\x0b;", "\u{feff}\x0b;"] {
                let source = format!("{prefix}# Documentation.\n{keyword} C; def f; end; end\n");
                assert!(offenses(&source).is_empty(), "{source:?}");
            }
            for prefix in ["work; ;", "0; ;", "\x0bwork; ;", "\x0b0; ;"] {
                assert_offense(
                    &format!("{prefix}# Documentation.\n{keyword} C; def f; end; end\n"),
                    2,
                    1,
                    &format!("{keyword} C"),
                );
            }
        }
    }

    #[test]
    fn documentation_attaches_across_empty_semicolon_statements() {
        // Independently verified with pinned RuboCop 1.92, Prism / Ruby 3.4.
        for keyword in ["class", "module"] {
            for (comments, gap) in [
                ("# Documentation.\n", ";"),
                ("# Documentation.\n", "\x0b;"),
                ("# Documentation.\n", ";; \t;"),
                (";# Documentation.\n", ";"),
                ("# Documentation.\n;# TODO: fix\n", ";"),
                ("# Documentation.\n;\n;# TODO: fix\n", ";"),
                ("\u{feff}\x0b;# Documentation.\r\n", "\x0b;"),
            ] {
                let source = format!("{comments}{gap}{keyword} C; def f; end; end\n");
                assert!(offenses(&source).is_empty(), "{source:?}");
            }
            for gap in ["(;", ";(;", ";(;("] {
                let closes = ")".repeat(gap.bytes().filter(|b| *b == b'(').count());
                let source =
                    format!("# Documentation.\n{gap}{keyword} C; def f; end; end{closes}\n");
                assert!(offenses(&source).is_empty(), "{source:?}");
            }
            for gap in [";work;", ";0;", "\x0b;work;", ";(work;"] {
                let closes = if gap.contains('(') { ")" } else { "" };
                assert_offense(
                    &format!("# Documentation.\n{gap}{keyword} C; def f; end; end{closes}\n"),
                    2,
                    gap.len() as u32 + 1,
                    &format!("{keyword} C"),
                );
            }
            assert_offense(
                &format!("# Documentation.\n;\n{keyword} C; def f; end; end\n"),
                3,
                1,
                &format!("{keyword} C"),
            );
            assert_offense(
                &format!("# Documentation.\n;work;# TODO: fix\n;{keyword} C; def f; end; end\n"),
                3,
                2,
                &format!("{keyword} C"),
            );
        }
        assert_offense(
            "# Documentation.\n;class C; def f; end; end; class D; def f; end; end\n",
            2,
            28,
            "class D",
        );
    }

    #[test]
    fn semicolon_attachment_does_not_change_comment_classification() {
        for comment in [
            "# TODO: fix",
            "# frozen_string_literal: true",
            "# encoding: utf-8",
            "# rubocop:disable Style/For",
            "# rustocop:disable Style/For",
        ] {
            assert_offense(
                &format!(";{comment}\n;class C; def f; end; end\n"),
                2,
                2,
                "class C",
            );
        }
        assert_offense(
            "class C #:nodoc: all;\n class D; def f; end; end\nend\n",
            2,
            2,
            "class C::D",
        );
    }

    #[test]
    fn initial_headers_do_not_document_across_empty_statements() {
        // Independently verified with pinned RuboCop 1.92, Prism / Ruby 3.4.
        for keyword in ["class", "module"] {
            for header in [
                "# coding: utf-8",
                "#! /usr/bin/ruby",
                "# warn_indent: true",
                "# warn_past_scope: true",
                "# -*- warn_indent: true -*-",
                "# -*- coding: utf-8 -*-",
                "# coding = utf-8",
            ] {
                for prefix in ["", ";", ";; \t;", "\x0b;", "\u{feff}\x0b;"] {
                    for newline in ["\n", "\r\n"] {
                        assert_offense(
                            &format!(
                                "{prefix}{header}{newline}{keyword} C; def f; end; end{newline}"
                            ),
                            2,
                            1,
                            &format!("{keyword} C"),
                        );
                    }
                }
            }
            for headers in [
                ";#! /usr/bin/ruby\n;# warn_indent: true\n;# coding: utf-8\n",
                ";# coding: utf-8\n;# TODO: fix\n",
                "work\n;# coding: utf-8\n",
            ] {
                let line = headers.bytes().filter(|b| *b == b'\n').count() as u32 + 1;
                assert_offense(
                    &format!("{headers};{keyword} C; def f; end; end\n"),
                    line,
                    2,
                    &format!("{keyword} C"),
                );
            }
        }
    }

    #[test]
    fn initial_header_skipping_is_ordered_and_does_not_discard_documentation() {
        for comments in [
            ";# Documentation.\n",
            ";# coding: utf-8\n;# Documentation.\n",
            ";#! /usr/bin/ruby\n;# warn_indent: true\n;# coding: utf-8\n;# Documentation.\n",
            // Only one of each header is skipped, in shebang/magic/encoding order.
            ";# warn_indent: true\n;# warn_indent: true\n",
            ";# coding: utf-8\n;# coding: utf-8\n",
            ";# coding: utf-8\n;# warn_indent: true\n",
            ";# warn_indent: true\n;#! /usr/bin/ruby\n",
            ";# Documentation.\n;# warn_indent: true\n",
            ";# -*- warn_indent: true\n",
            ";# WARN_INDENT: true\n",
            ";# decoding: utf-8\n",
        ] {
            let source = format!("{comments};class C; def f; end; end\n");
            assert!(offenses(&source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn honors_nodoc_only_on_self_and_all_on_ancestors() {
        for keyword in ["class", "module"] {
            for name in ["Test", "A::B::Test", "::Test"] {
                for comment in ["#:nodoc:", "# :nodoc:", "#:nodoc: all"] {
                    let source = format!("{keyword} {name} {comment}\n  def method; end\nend");
                    assert!(offenses(&source).is_empty(), "{source}");
                }
            }
            let source = format!(
                "module A #:nodoc: all\n  module B\n    TEST = 20\n    {keyword} Test\n      def method; end\n    end\n  end\nend"
            );
            assert!(offenses(&source).is_empty(), "{source}");
            assert_offense(
                &format!(
                    "module TestModule #:nodoc:\n  TEST = 20\n  {keyword} Test\n    def method; end\n  end\nend"
                ),
                3,
                3,
                &format!("{keyword} TestModule::Test"),
            );
        }
        assert!(offenses("class Test < Parent #:nodoc:\n  def method; end\nend").is_empty());
        assert!(
            offenses(
                "class A #:nodoc: all\n  class << self\n    module B; def foo; end; end\n  end\nend"
            )
            .is_empty()
        );
        assert_offense("class A\n  def method; end #:nodoc:\nend", 1, 1, "class A");
        assert_offense(
            "module A #:nodoc: all extra\n  class B; def foo; end; end\nend",
            2,
            3,
            "class A::B",
        );
        assert_offense(
            "module A #:nodoc: all\nend\nmodule A; def foo; end; end",
            3,
            1,
            "module A",
        );
    }

    #[test]
    fn qualifies_identifiers_from_ast_ancestors_not_inferred_runtime_scopes() {
        for name in [
            "::MyClass",
            "self::MyClass",
            "my_method::MyClass",
            "obj&.my_method::MyClass",
            "A::B",
        ] {
            assert_offense(
                &format!("class {name}\n  def method; end\nend"),
                1,
                1,
                &format!("class {name}"),
            );
        }
        for (prefix, name) in [
            ("m = Module.new\n", "m::N"),
            ("", "@m::N"),
            ("", "@@m::N"),
            ("", "$m::N"),
            ("m = Module.new\nn = Module.new\n", "m::n::M"),
        ] {
            assert_offense(
                &format!("{prefix}module {name}\n  def method; end\nend"),
                prefix.lines().count() as u32 + 1,
                1,
                &format!("module {name}"),
            );
        }
        assert_offense(
            "module A::B\n  module C\n    class D\n      class E::F\n        def method; end\n      end\n    end\n  end\nend",
            4,
            7,
            "class A::B::C::D::E::F",
        );
        assert_offense(
            "module A\n  class ::B\n    def method; end\n  end\nend",
            2,
            3,
            "class A::B",
        );
        assert_offense(
            "# outer documented\nclass A\n  class << self\n    class B; def foo; end; end\n  end\nend",
            4,
            5,
            "class A::B",
        );
    }

    #[test]
    fn recognizes_transparent_body_wrappers_and_hollow_expression_bodies() {
        // The pinned mixin-only check recurses through expression children:
        // zero-child nodes also satisfy it, unlike scalar-valued literals.
        for body in [
            "(include A)",
            "begin; include A; end",
            "(A = 1; private_constant :A)",
            "nil",
            "true",
            "false",
            "self",
            "[]",
            "{}",
            "[nil, true]",
            "while true; include A; end",
            "defined?(nil)",
            "nil..nil",
        ] {
            let source = format!("module M; {body}; end");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        for body in [
            "begin; A = 1; private_constant :A; end",
            "include A if true",
            "[1]",
            "{a: nil}",
            "\"\"",
            "0",
        ] {
            assert_offense(&format!("module M; {body}; end"), 1, 1, "module M");
        }
    }

    #[test]
    fn recursively_exempts_conditional_and_ensure_mixin_wrappers() {
        for body in [
            "if true; include A; else; include B; end",
            "unless false; include A; else; include B; end",
            "true ? include(A) : include(B)",
            "begin; include A; ensure; include B; end",
            "if true; begin; include A; ensure; include B; end; else; extend C; end",
            "case true; when nil; include A; when true; extend B; else; prepend C; end",
            "if true; include A; elsif false; extend B; else; prepend C; end",
        ] {
            let source = format!("module M; {body}; end");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        for body in [
            "if true; include A; end",
            "if condition; include A; else; include B; end",
            "if true; include A; else; work; end",
            "begin; include A; ensure; work; end",
            "begin; include A; rescue; include B; end",
            "begin; include A; ensure; end",
            "begin; ensure; include A; end",
            "case true; when nil; include A; end",
            "case true; when nil; include A; else; work; end",
        ] {
            assert_offense(&format!("module M; {body}; end"), 1, 1, "module M");
        }
    }

    #[test]
    fn inherits_nodoc_all_from_expression_ancestors() {
        for source in [
            "if cond #:nodoc: all\n class Inner; def f; end; end\nend\n",
            "foo do #:nodoc: all\n class Inner; def f; end; end\nend\n",
            "unless cond #:nodoc: all\n class Inner; def f; end; end\nend\n",
            "while cond #:nodoc: all\n class Inner; def f; end; end\nend\n",
            "foo do |x| #:nodoc: all\n class Inner; def f; end; end\nend\n",
            "case cond #:nodoc: all\n when x; class Inner; def f; end; end\nend\n",
            "for x in xs #:nodoc: all\n class Inner; def f; end; end\nend\n",
            "begin #:nodoc: all\n class Inner; def f; end; end\nend\n",
            "if cond\n unrelated #:nodoc: all\n class Inner; def f; end; end\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "if cond #:nodoc:\n class Inner; def f; end; end\nend\n",
            "foo do #:nodoc:\n class Inner; def f; end; end\nend\n",
            "if cond(x) #:nodoc: all\n class Inner; def f; end; end\nend\n",
            "foo(x) do #:nodoc: all\n class Inner; def f; end; end\nend\n",
            "object.foo do #:nodoc: all\n class Inner; def f; end; end\nend\n",
            "if \"#{value}\" #:nodoc: all\n class Inner; def f; end; end\nend\n",
            "if // #:nodoc: all\n class Inner; def f; end; end\nend\n",
        ] {
            assert_offense(source, 2, 2, "class Inner");
        }
    }

    #[test]
    fn documentation_attaches_through_grouping_parentheses_only() {
        for source in [
            "# Documentation.\n(module M; def f; end; end)\n",
            "# Documentation.\n((module M; def f; end; end))\n",
            "# Documentation.\n( class C; def f; end; end)\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_offense(
            "# Documentation.\n(call; module M; def f; end; end)",
            2,
            8,
            "module M",
        );
        assert_offense(
            "# Documentation.\nfoo(module M; def f; end; end)",
            2,
            5,
            "module M",
        );
        assert_offense(
            "# Documentation.\n(\nmodule M; def f; end; end)",
            3,
            1,
            "module M",
        );
    }

    #[test]
    fn quoted_directives_are_documentation_but_embedded_active_directives_are_not() {
        for comment in [
            "# # rubocop:disable Style/For",
            "#   # rubocop: disable Style/For",
            "# # rustocop:disable Style/For",
            "# rubocop:unknown Style/For",
        ] {
            let source = format!("{comment}\nclass C; def f; end; end");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        for comment in [
            "# rubocop:disable Style/For",
            "# prose # rubocop:disable Style/For",
            "# ## rubocop:disable Style/For",
            "# prose # rustocop:disable Style/For",
        ] {
            assert_offense(
                &format!("{comment}\nclass C; def f; end; end"),
                2,
                1,
                "class C",
            );
        }
    }

    #[test]
    fn directive_boundaries_use_rubys_word_set_without_join_controls() {
        for alias in ["rubocop", "rustocop"] {
            for mode in [
                "pop",
                "push",
                "next",
                "disable",
                "enable",
                "todo",
                "disable-next",
                "enable-next",
                "todo-next",
            ] {
                // Join controls are word characters in Rust's Unicode regexes,
                // but not in Ruby's. Punctuation and whitespace also delimit.
                for suffix in ["\u{200c}", "\u{200d}", "", ".", "-more", "\x0b"] {
                    for prefix in ["# ", "# prose # ", "# ## "] {
                        assert_offense(
                            &format!("{prefix}{alias}:{mode}{suffix}\nclass C; def f; end; end\n"),
                            2,
                            1,
                            "class C",
                        );
                    }
                }
                // These Unicode word characters must NOT make the header a
                // directive. The -next modes may still match their short form
                // at the preceding hyphen, just as the pinned regex does.
                for suffix in [
                    "x", "é", "中", "\u{301}", "\u{903}", "\u{20dd}", "0", "٢", "²", "Ⅷ", "_",
                    "\u{203f}",
                ] {
                    for prefix in ["# ", "# prose # ", "# ## "] {
                        let source =
                            format!("{prefix}{alias}:{mode}{suffix}\nclass C; def f; end; end\n");
                        if mode.ends_with("-next") {
                            assert_offense(&source, 2, 1, "class C");
                        } else {
                            assert!(offenses(&source).is_empty(), "{source:?}");
                        }
                    }
                }
                // Exactly one preceding # quotes the directive; a second
                // marker or intervening prose does not. Keep both aliases.
                for suffix in ["\u{200c}", "\u{200d}", "", "é", "\u{301}", "0", "_"] {
                    for prefix in ["# # ", "# \x0b# "] {
                        let source =
                            format!("{prefix}{alias}:{mode}{suffix}\nclass C; def f; end; end\n");
                        assert!(offenses(&source).is_empty(), "{source:?}");
                    }
                }
            }
            // DirectiveComment uses only the first matching header, even if a
            // later header would be active. An earlier word lookalike is not
            // a match and must not hide a later actual directive.
            let quoted_first =
                format!("# # {alias}:pop\u{200c} # {alias}:pop\nclass C; def f; end; end\n");
            assert!(offenses(&quoted_first).is_empty(), "{quoted_first:?}");
            assert_offense(
                &format!("# # {alias}:popé # {alias}:pop\nclass C; def f; end; end\n"),
                2,
                1,
                "class C",
            );
        }
    }

    #[test]
    fn comment_classification_is_cached_across_many_definitions_and_long_nodoc_ancestors() {
        use crate::cops::run_cops_with_corrections;
        let separators = "; \x0b".repeat(8000);
        let source = format!(
            ";#! /usr/bin/ruby\n;# warn_indent: true\n;# coding: utf-8\nmodule Outer #:nodoc: all{}\n{}end\n{}{separators}# Long documentation.\n{separators}module Long; def f; end; end\n",
            " \x0b".repeat(8000),
            "module Inner; end\n".repeat(2000),
            ";\x0b# TODO: fix\n;\x0bmodule Undocumented; end\n;\x0b# A documented module.\n;\x0bmodule Documented; end\n".repeat(2000)
        );
        let file = SourceFile::new("lib/a.rb".into(), source.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        assert_eq!(parsed.errors().count(), 0);
        file.record_comments(&parsed);
        file.record_encoding(&parsed);
        let index = file.documentation_comments();
        assert_eq!(index.entries.len(), 4002);
        let pointer = index.entries.as_ptr();
        assert!(index.entries[0].nodoc_all);
        let last = index.entries.last().unwrap();
        assert!(last.documenting_group);
        assert!(file.bytes[last.next_code..].starts_with(b"module Long"));
        let mut found = Vec::new();
        for corrections in [false, true] {
            found.clear();
            assert!(
                run_cops_with_corrections(
                    &[&Documentation],
                    &parsed,
                    &file,
                    &mut found,
                    corrections
                )
                .is_empty()
            );
            assert_eq!(found.len(), 2000);
            assert!(found.iter().all(|o| o.correction.is_none()));
            assert_eq!(file.documentation_comments().entries.as_ptr(), pointer);
        }
    }

    #[test]
    fn comments_in_literals_and_data_do_not_document_definitions() {
        assert_offense(
            "text = \"# Class documentation.\"\nclass C; def foo; end; end\n__END__\n# Docs\nmodule Data",
            2,
            1,
            "class C",
        );
        assert_offense(
            "text = <<~H\n# Class documentation.\nH\nclass C; def foo; end; end",
            4,
            1,
            "class C",
        );
        assert_offense(
            "=begin\nDocs\n=end\nclass C; def foo; end; end",
            4,
            1,
            "class C",
        );
        assert!(offenses("\u{feff}# 日本語\r\nclass C\r\n  def foo; end\r\nend\r\n").is_empty());
    }

    #[test]
    fn reports_classes_and_modules_at_the_keyword() {
        assert_offense(
            "class MyClass\n  def method\n  end\nend\n",
            1,
            1,
            "class MyClass",
        );
        assert_offense(
            "module MyModule\n  def method\n  end\nend\n",
            1,
            1,
            "module MyModule",
        );
        assert_offense("  module Test\n  end\n", 1, 3, "module Test");
        assert_offense(
            "class Foo < ApplicationRecord\n  belongs_to :bar\nend\n",
            1,
            1,
            "class Foo",
        );
    }
}
