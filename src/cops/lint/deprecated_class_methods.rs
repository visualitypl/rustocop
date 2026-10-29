//! `Lint/DeprecatedClassMethods` — flags deprecated (or removed) core
//! methods: `File.exists?`, `Dir.exists?`, `ENV.freeze`, `ENV.clone`,
//! `ENV.dup`, `Socket.gethostbyname`, `Socket.gethostbyaddr`, `iterator?`,
//! and `attr :name, true|false`.
//!
//! Matches RuboCop's semantics: receivers are `Name` or `::Name`, safe
//! navigation calls are not checked, and argument counts are exact
//! (`File.exists?` takes one argument, `ENV.dup` and `iterator?` none,
//! `attr` a name and a boolean literal). The offense is reported at the
//! start of the call, and the message quotes the call up to the method name.

use ruby_prism::{CallNode, Node};

use crate::{
    cops::{Cop, NodeContext},
    offense::{Offense, single_line},
    source::SourceFile,
};

/// The `Lint/DeprecatedClassMethods` cop.
pub struct DeprecatedClassMethods;

impl Cop for DeprecatedClassMethods {
    fn name(&self) -> &'static str {
        "Lint/DeprecatedClassMethods"
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
        let Some((current, prefer)) = deprecated_call(&call) else {
            return;
        };
        let (line, col) = file.line_col(call.location().start_offset());
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: format!("`{current}` is deprecated in favor of `{prefer}`."),
            line,
            end_line: file.line_of(if call.name().as_slice() == b"attr" {
                crate::cops::ast::source_end(node).saturating_sub(1)
            } else {
                call.message_loc().unwrap().end_offset().saturating_sub(1)
            }),
            col,
        });
    }
}

/// For a deprecated call, the offending source and its replacement.
fn deprecated_call(call: &CallNode<'_>) -> Option<(String, String)> {
    let name = call.name().as_slice();
    // RuboCop's RESTRICT_ON_SEND: bail out before any other work.
    if !matches!(
        name,
        b"attr"
            | b"clone"
            | b"dup"
            | b"exists?"
            | b"freeze"
            | b"gethostbyaddr"
            | b"gethostbyname"
            | b"iterator?"
    ) || call.is_safe_navigation()
    {
        return None;
    }
    let arguments = call.arguments().map(|arguments| arguments.arguments());
    // A block argument (`&blk`) is an argument in RuboCop's AST.
    let block_argument = call
        .block()
        .is_some_and(|block| block.as_block_argument_node().is_some());
    let argument_count =
        arguments.as_ref().map_or(0, |arguments| arguments.len()) + usize::from(block_argument);
    let Some(receiver) = call.receiver() else {
        return match name {
            b"iterator?" if argument_count == 0 => {
                Some(("iterator?".to_string(), "block_given?".to_string()))
            }
            // `(send nil? :attr _ boolean)`: exactly two positional
            // arguments, so a block argument never matches.
            b"attr" if argument_count == 2 && !block_argument => {
                let arguments = arguments?;
                let (attribute, flag) = (arguments.iter().next()?, arguments.iter().nth(1)?);
                let accessor = match flag {
                    Node::TrueNode { .. } => "attr_accessor",
                    Node::FalseNode { .. } => "attr_reader",
                    _ => return None,
                };
                // The send alone: through the closing paren or the last
                // argument, never a literal block.
                let end = call
                    .closing_loc()
                    .map_or(flag.location().end_offset(), |closing| closing.end_offset());
                let start = call.location().start_offset();
                let source = call.location().as_slice();
                Some((
                    single_line(&source[..end - start]),
                    format!("{accessor} {}", text(&attribute)),
                ))
            }
            _ => None,
        };
    };
    let constant = global_const_name(&receiver)?;
    let prefer = match (constant, name) {
        (b"ENV", b"clone" | b"dup") if argument_count == 0 => {
            format!("{}.to_h", text(&receiver))
        }
        (b"ENV", b"freeze") if argument_count == 0 => "ENV".to_string(),
        (b"File" | b"Dir", b"exists?") if argument_count == 1 => {
            format!("{}.exist?", text(&receiver))
        }
        (b"Socket", b"gethostbyaddr") => "Addrinfo#getnameinfo".to_string(),
        (b"Socket", b"gethostbyname") => "Addrinfo.getaddrinfo".to_string(),
        _ => return None,
    };
    // From the receiver through the method name, e.g. `::File.exists?`.
    let message = call.message_loc()?;
    let start = call.location().start_offset();
    let source = call.location().as_slice();
    Some((single_line(&source[..message.end_offset() - start]), prefer))
}

/// The name of a `Name` or `::Name` constant.
fn global_const_name<'pr>(node: &Node<'pr>) -> Option<&'pr [u8]> {
    match node {
        Node::ConstantReadNode { .. } => {
            Some(node.as_constant_read_node().unwrap().name().as_slice())
        }
        Node::ConstantPathNode { .. } => {
            let path = node.as_constant_path_node().unwrap();
            if path.parent().is_some() {
                return None;
            }
            path.name().map(|name| name.as_slice())
        }
        _ => None,
    }
}

/// A node's source text, on one line.
fn text(node: &Node<'_>) -> String {
    single_line(node.location().as_slice())
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/deprecated_class_methods_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/DeprecatedClassMethods")
            .collect()
    }

    #[test]
    fn flags_rubocop_spec_offenses_with_messages() {
        for (source, message) in [
            (
                "File.exists?(o)",
                "`File.exists?` is deprecated in favor of `File.exist?`.",
            ),
            (
                "::File.exists?(o)",
                "`::File.exists?` is deprecated in favor of `::File.exist?`.",
            ),
            (
                "Dir.exists?(o)",
                "`Dir.exists?` is deprecated in favor of `Dir.exist?`.",
            ),
            (
                "::Dir.exists?(o)",
                "`::Dir.exists?` is deprecated in favor of `::Dir.exist?`.",
            ),
            (
                "iterator?",
                "`iterator?` is deprecated in favor of `block_given?`.",
            ),
            (
                "attr :name, true",
                "`attr :name, true` is deprecated in favor of `attr_accessor :name`.",
            ),
            (
                "attr 'name', true",
                "`attr 'name', true` is deprecated in favor of `attr_accessor 'name'`.",
            ),
            (
                "attr :name, false",
                "`attr :name, false` is deprecated in favor of `attr_reader :name`.",
            ),
            (
                "attr 'name', false",
                "`attr 'name', false` is deprecated in favor of `attr_reader 'name'`.",
            ),
            (
                "ENV.freeze",
                "`ENV.freeze` is deprecated in favor of `ENV`.",
            ),
            (
                "ENV.clone",
                "`ENV.clone` is deprecated in favor of `ENV.to_h`.",
            ),
            ("ENV.dup", "`ENV.dup` is deprecated in favor of `ENV.to_h`."),
            (
                "Socket.gethostbyaddr([221,186,184,68].pack(\"CCCC\"))",
                "`Socket.gethostbyaddr` is deprecated in favor of `Addrinfo#getnameinfo`.",
            ),
            (
                "::Socket.gethostbyaddr([221,186,184,68].pack(\"CCCC\"))",
                "`::Socket.gethostbyaddr` is deprecated in favor of `Addrinfo#getnameinfo`.",
            ),
            (
                "Socket.gethostbyaddr([221,186,184,68].pack(\"CCCC\"), Socket::AF_INET)",
                "`Socket.gethostbyaddr` is deprecated in favor of `Addrinfo#getnameinfo`.",
            ),
            (
                "Socket.gethostbyname(\"hal\")",
                "`Socket.gethostbyname` is deprecated in favor of `Addrinfo.getaddrinfo`.",
            ),
            (
                "::Socket.gethostbyname(\"hal\")",
                "`::Socket.gethostbyname` is deprecated in favor of `Addrinfo.getaddrinfo`.",
            ),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "should flag {source:?}");
            assert_eq!((found[0].line, found[0].col), (1, 1), "{source:?}");
            assert_eq!(found[0].message, message, "{source:?}");
        }
    }

    #[test]
    fn accepts_rubocop_spec_no_offenses() {
        for source in [
            "File.exist?(o)",
            "Dir.exist?(o)",
            "Foo.exists?(o)",
            "block_given?",
            "Foo.iterator?",
            "attr :name",
            "attr :name, attribute",
            "attr_accessor :name",
            "attr_reader :name",
            "ENV.values",
            "Foo.gethostbyaddr",
            "Foo.gethostbyname",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn reports_nested_and_multiline_calls_at_their_start() {
        let found = offenses("if File.exists?(path)\n  x = ENV\n    .dup\nend\n");
        let positions: Vec<(u32, u32)> = found.iter().map(|o| (o.line, o.col)).collect();
        assert_eq!(positions, vec![(1, 4), (2, 7)]);
        assert_eq!(
            found[1].message,
            "`ENV     .dup` is deprecated in favor of `ENV.to_h`."
        );
        assert_eq!(
            offenses("::ENV.freeze")[0].message,
            "`::ENV.freeze` is deprecated in favor of `ENV`."
        );
        assert_eq!(
            offenses("attr(:name, true)")[0].message,
            "`attr(:name, true)` is deprecated in favor of `attr_accessor :name`."
        );
        assert_eq!(offenses("ENV.dup { }").len(), 1);
        // A splat or block argument is one argument in RuboCop's AST.
        assert_eq!(offenses("File.exists?(*a)").len(), 1);
        assert_eq!(offenses("File.exists?(&b)").len(), 1);
        assert_eq!(
            offenses("attr :name,\n  false\n")[0].message,
            "`attr :name,   false` is deprecated in favor of `attr_reader :name`."
        );
    }

    #[test]
    fn attr_message_excludes_a_literal_block() {
        assert_eq!(
            offenses("attr :name, true do\nend\n")[0].message,
            "`attr :name, true` is deprecated in favor of `attr_accessor :name`."
        );
        assert_eq!(
            offenses("attr(:a, false) { 1 }")[0].message,
            "`attr(:a, false)` is deprecated in favor of `attr_reader :a`."
        );
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "File&.exists?(o)",
            "ENV&.dup",
            "File.exists?",
            "File.exists?(a, b)",
            "File.exists?(a, &b)",
            "Foo::File.exists?(o)",
            "file.exists?(o)",
            "ENV.dup(1)",
            "ENV.dup(&blk)",
            "ENV['X'].dup",
            "Env.freeze",
            "obj.attr :name, true",
            "attr :name, true, false",
            "attr :name, nil",
            "iterator?(1)",
            "self.iterator?",
            "Foo::Socket.gethostbyname('x')",
            "attr :name, &blk",
            "attr(:name, &blk)",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
