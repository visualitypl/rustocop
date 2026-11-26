//! `Bundler/GemComment` (opt-in): a gem declaration needs a description
//! comment. Fixed `OnlyFor: []` and `AllowedGems: []`, so every declaration
//! counts. A declaration is commented when parser's comment association
//! (`ast_with_comments`) gives its send, or one of the send's descendants, a
//! last preceding `#` comment on the node's line or the line before.
use std::collections::HashMap;

use ruby_prism::Node;

use super::declarations::gem_name;
use crate::{
    cops::{
        Cop, ast,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};

pub struct GemComment;

impl Cop for GemComment {
    fn name(&self) -> &'static str {
        "Bundler/GemComment"
    }

    fn check_parse(
        &self,
        result: &ruby_prism::ParseResult<'_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let mut builder = Builder {
            file,
            nodes: Vec::new(),
            roots: Vec::new(),
            frames: Vec::new(),
            gems: Vec::new(),
        };
        walk(&mut builder, &result.node());
        if builder.gems.is_empty() {
            return;
        }
        let comments = comments(result, file);
        let mut associator = Associator {
            nodes: &builder.nodes,
            comments: &comments,
            current: skip_directives(&comments, file),
            mapping: HashMap::new(),
        };
        // The pinned root is the program's only statement or its `begin`.
        for root in builder.roots.clone() {
            associator.visit(root);
        }
        for &(send, start) in &builder.gems {
            if commented_any_descendant(send, &builder.nodes, &associator.mapping, &comments) {
                continue;
            }
            let (line, col) = file.reference_line_col(start);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                col,
                end_line: line,
                message: "Missing gem description comment.".into(),
                correction: None,
            });
        }
    }
}

/// A node of the pinned (parser) AST, as far as comment association sees it.
struct Parser {
    start: usize,
    end: usize,
    line: u32,
    last_line: u32,
    /// `begin` nodes take no leading comments.
    begin: bool,
    /// Heredoc maps are always descended into.
    heredoc: bool,
    children: Vec<usize>,
}

struct Comment {
    start: usize,
    end: usize,
    line: u32,
    /// `comment_line?`: the comment's source starts with `#`.
    hash: bool,
}

fn comments(result: &ruby_prism::ParseResult<'_>, file: &SourceFile) -> Vec<Comment> {
    result
        .comments()
        .map(|comment| {
            let location = comment.location();
            let start = location.start_offset();
            let mut end = location.end_offset();
            if file.bytes[start..end].ends_with(b"\r") {
                end -= 1; // parser's comment range stops before CR
            }
            Comment {
                start,
                end,
                line: file.line_of(start),
                hash: file.bytes[start] == b'#',
            }
        })
        .collect()
}

/// `advance_through_directives`: a leading shebang, then one magic comment,
/// then one encoding comment are never associated.
fn skip_directives(comments: &[Comment], file: &SourceFile) -> usize {
    let text = |i: usize| comments.get(i).map(|c| &file.bytes[c.start..c.end]);
    let mut current = 0;
    if text(current).is_some_and(|t| t.starts_with(b"#!")) {
        current += 1;
    }
    if text(current).is_some_and(magic_comment) {
        current += 1;
    }
    if text(current).is_some_and(encoding_comment) {
        current += 1;
    }
    current
}

/// `/^#\s*(-\*-|)\s*(frozen_string_literal|warn_indent|warn_past_scope):.*\1$/`
fn magic_comment(text: &[u8]) -> bool {
    let Some(rest) = text.strip_prefix(b"#") else {
        return false;
    };
    let rest = rest.trim_ascii_start();
    let (emacs, rest) = match rest.strip_prefix(b"-*-") {
        Some(rest) => (true, rest.trim_ascii_start()),
        None => (false, rest),
    };
    ["frozen_string_literal:", "warn_indent:", "warn_past_scope:"]
        .iter()
        .any(|key| {
            rest.starts_with(key.as_bytes()) && (!emacs || rest[key.len()..].ends_with(b"-*-"))
        })
}

/// Parser's `Buffer::ENCODING_RE`: `[\s#](en)?coding\s*[:=]\s*` and a name.
fn encoding_comment(text: &[u8]) -> bool {
    (1..text.len()).any(|i| {
        if !(text[i - 1] == b'#' || text[i - 1].is_ascii_whitespace()) {
            return false;
        }
        let rest = &text[i..];
        let rest = rest.strip_prefix(b"en").unwrap_or(rest);
        let Some(rest) = rest.strip_prefix(b"coding") else {
            return false;
        };
        let rest = rest.trim_ascii_start();
        let Some(rest) = rest.strip_prefix(b":").or_else(|| rest.strip_prefix(b"=")) else {
            return false;
        };
        rest.trim_ascii_start()
            .first()
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
    })
}

/// Builds the parser-shaped tree in one complete walk.
struct Builder<'f> {
    file: &'f SourceFile,
    nodes: Vec<Parser>,
    roots: Vec<usize>,
    /// Per entered Prism node: where its children attach, and (for a call
    /// with a literal block) where the block's own children attach.
    frames: Vec<(Option<usize>, Option<usize>)>,
    /// Each gem declaration's `send` node and start offset.
    gems: Vec<(usize, usize)>,
}

impl Builder<'_> {
    fn add(
        &mut self,
        parent: Option<usize>,
        start: usize,
        end: usize,
        begin: bool,
        heredoc: bool,
    ) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Parser {
            start,
            end,
            line: self.file.line_of(start),
            last_line: self.file.line_of(end),
            begin,
            heredoc,
            children: Vec::new(),
        });
        match parent {
            Some(parent) => self.nodes[parent].children.push(id),
            None => self.roots.push(id),
        }
        id
    }
}

fn heredoc(node: &Node<'_>) -> bool {
    let opening = match node {
        Node::StringNode { .. } => node.as_string_node().unwrap().opening_loc(),
        Node::InterpolatedStringNode { .. } => {
            node.as_interpolated_string_node().unwrap().opening_loc()
        }
        Node::XStringNode { .. } => Some(node.as_x_string_node().unwrap().opening_loc()),
        Node::InterpolatedXStringNode { .. } => {
            Some(node.as_interpolated_x_string_node().unwrap().opening_loc())
        }
        _ => None,
    };
    opening.is_some_and(|o| o.as_slice().starts_with(b"<<"))
}

impl<'pr> NodeVisitor<'pr> for Builder<'_> {
    fn enter(&mut self, node: Node<'pr>) {
        let (parent, block_parent) = self.frames.last().copied().unwrap_or((None, None));
        let location = node.location();
        let (start, end) = (location.start_offset(), location.end_offset());
        let frame = match &node {
            // Not parser nodes: their children belong to the parent.
            Node::ProgramNode { .. }
            | Node::ArgumentsNode { .. }
            | Node::ElseNode { .. }
            | Node::NumberedParametersNode { .. }
            | Node::ItParametersNode { .. } => (parent, None),
            Node::StatementsNode { .. } if node.as_statements_node().unwrap().body().len() < 2 => {
                (parent, None)
            }
            Node::StatementsNode { .. } | Node::ParenthesesNode { .. } => {
                (Some(self.add(parent, start, end, true, false)), None)
            }
            // A literal block's parameters and body are children of the
            // `block` node, a sibling of the `send`.
            Node::BlockNode { .. } => (block_parent.or(parent), None),
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                let block = call.block().is_some_and(|b| b.as_block_node().is_some());
                let send = if block {
                    let block = self.add(parent, start, end, false, false);
                    let send = self.add(Some(block), start, ast::source_end(&node), false, false);
                    (Some(send), Some(block))
                } else {
                    (Some(self.add(parent, start, end, false, false)), None)
                };
                if gem_name(&call, self.file).is_some() {
                    self.gems.push((send.0.unwrap(), start));
                }
                send
            }
            _ => (
                Some(self.add(parent, start, end, false, heredoc(&node))),
                None,
            ),
        };
        self.frames.push(frame);
    }

    fn leave(&mut self) {
        self.frames.pop();
    }
}

/// Parser's `Source::Comment::Associator`, mapping by identity.
struct Associator<'a> {
    nodes: &'a [Parser],
    comments: &'a [Comment],
    current: usize,
    mapping: HashMap<usize, Vec<usize>>,
}

impl Associator<'_> {
    fn comment(&self) -> Option<&Comment> {
        self.comments.get(self.current)
    }

    fn associate(&mut self, node: usize) {
        self.mapping.entry(node).or_default().push(self.current);
        self.current += 1;
    }

    fn visit(&mut self, id: usize) {
        let node = &self.nodes[id];
        if !node.begin {
            while self.comment().is_some_and(|c| c.end <= node.start) {
                self.associate(id);
            }
        }
        let Some(comment) = self.comment() else {
            return;
        };
        if comment.line <= node.last_line || node.heredoc {
            let mut children = node.children.clone();
            children.sort_by_key(|&child| self.nodes[child].start);
            for child in children {
                self.visit(child);
            }
            while self.comment().is_some_and(|c| c.end <= node.end) {
                self.associate(id);
            }
            while self.comment().is_some_and(|c| c.line == node.last_line) {
                self.associate(id);
            }
        }
    }
}

/// `commented?(node) || node.each_descendant.any? { |n| commented?(n) }`.
fn commented_any_descendant(
    id: usize,
    nodes: &[Parser],
    mapping: &HashMap<usize, Vec<usize>>,
    comments: &[Comment],
) -> bool {
    let node = &nodes[id];
    let commented = mapping.get(&id).is_some_and(|associated| {
        associated
            .iter()
            .map(|&i| &comments[i])
            .rfind(|c| c.line <= node.line)
            .is_some_and(|c| node.line - c.line <= 1 && c.hash)
    });
    commented
        || node
            .children
            .iter()
            .any(|&child| commented_any_descendant(child, nodes, mapping, comments))
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's bundler/gem_comment_spec.rb (MIT, Copyright (c)
    //! 2012-26 Bozhidar Batsov), every example at the fixed defaults
    //! (`OnlyFor: []`, `AllowedGems: []`), plus association lookalikes.
    use super::*;
    use crate::cops::bundler::tests::{assert_offenses, lint};

    fn expect(source: &str, expected: &[(u32, u32)]) {
        let expected: Vec<_> = expected
            .iter()
            .map(|&(line, col)| (line, col, "Missing gem description comment.".to_owned()))
            .collect();
        assert_offenses(&GemComment, "Gemfile", source, &expected);
    }

    #[test]
    fn ruby_files_and_empty_gemfiles_are_ignored() {
        assert!(lint(&GemComment, "foo.rb", "gem('rubocop')\n").is_empty());
        expect("", &[]);
    }

    #[test]
    fn commented_gems() {
        expect("# Style-guide enforcer.\ngem 'rubocop'\n", &[]);
        expect("gem 'rubocop' # Style-guide enforcer.\n", &[]);
        expect(
            "source 'http://rubygems.org'\n\n# Style-guide enforcer.\ngroup :development do\n  # …\nend\n",
            &[],
        );
    }

    #[test]
    fn uncommented_gems_at_fixed_defaults() {
        expect("gem 'rubocop'\n", &[(1, 1)]);
        // AllowedGems is fixed empty: `rake` is not exempt.
        expect("gem 'rake'\n", &[(1, 1)]);
        // OnlyFor is fixed empty: every declaration needs a comment.
        for source in [
            "gem 'rubocop', group: development\n",
            "gem 'rubocop', '~> 12.0'\n",
            "gem 'rubocop', '~> 12.0', '>= 11.0'\n",
            "gem 'rubocop', '~> 12.0', required: true\n",
            "gem 'rubocop', '>= 12.0'\n",
            "gem 'rubocop', ' ~> 12.0'\n",
            "gem 'rubocop', github: 'some_user/some_fork'\n",
            "gem 'rubocop', key => 'value'\n",
        ] {
            expect(source, &[(1, 1)]);
        }
    }

    #[test]
    fn documented_examples() {
        expect("gem 'foo'\n", &[(1, 1)]);
        expect("# Helpers for the foo things.\ngem 'foo'\n", &[]);
        expect(
            "gem 'foo', '>= 1.0'\n\n# Version 2.1 introduces breaking change baz\ngem 'foo', '< 2.1'\n",
            &[(1, 1)],
        );
    }

    #[test]
    fn association_follows_the_pinned_associator() {
        // A blank line between comment and gem.
        expect("# Helper\n\ngem 'a'\n", &[(3, 1)]);
        // Magic and encoding comments are never associated.
        expect("# frozen_string_literal: true\ngem 'a'\n", &[(2, 1)]);
        expect("# encoding: utf-8\ngem 'a'\n", &[(2, 1)]);
        // A later ordinary comment counts.
        expect("# frozen_string_literal: true\n# Helper\ngem 'a'\n", &[]);
        // The comment goes to the enclosing node that starts after it.
        expect("# Helper\ngem 'a' do\nend\n", &[(2, 1)]);
        expect("# Helper\ngem 'a' if x\n", &[(2, 1)]);
        expect("# Helper\nx = gem('a')\n", &[(2, 5)]);
        // Inside the argument list, a descendant is commented.
        expect("gem 'a',\n  # Why\n  require: false\n", &[]);
        // A decorating comment of an earlier multi-line call is consumed there.
        expect("foo(1,\n  2) # Helper\ngem 'a'\n", &[(3, 1)]);
        // Each gem needs its own comment.
        expect("# Helper\ngem 'a'\ngem 'b'\n", &[(3, 1)]);
        expect("gem 'a' # Helper\ngem 'b'\n", &[(2, 1)]);
        // =begin blocks are not `#` comments.
        expect("=begin\nHelper\n=end\ngem 'a'\n", &[(4, 1)]);
        // Inside a group block.
        expect(
            "group :x do\n  # Helper\n  gem 'a'\n  gem 'b'\nend\n",
            &[(4, 3)],
        );
    }

    #[test]
    fn crlf_and_bom() {
        expect("# Helper\r\ngem 'a'\r\ngem 'b'\r\n", &[(3, 1)]);
        // The pinned column counts the BOM on the first line.
        expect("\u{feff}gem 'a'\n", &[(1, 2)]);
        expect("\u{feff}# Helper\ngem 'a'\n", &[]);
    }
}
