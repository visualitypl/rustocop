//! `Style/ClassMethodsDefinitions` — opt-in fixed `def_self` policy.
use super::class_methods::{declaration, elements};
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ClassMethodsDefinitions;
impl Cop for ClassMethodsDefinitions {
    fn name(&self) -> &'static str {
        "Style/ClassMethodsDefinitions"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_singleton_class_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(singleton) = node.as_singleton_class_node() else {
            return;
        };
        if singleton.expression().as_self_node().is_none() {
            return;
        }
        let children = elements(singleton.body());
        let methods: Vec<_> = children
            .iter()
            .filter_map(|n| n.as_def_node().filter(|d| d.receiver().is_none()))
            .collect();
        if methods.is_empty()
            || !all_public(&children)
            || methods.iter().any(|d| !no_code_after(&d.as_node(), file))
        {
            return;
        }
        let (line, col) = file.line_col(node.location().start_offset());
        let correction = cx
            .corrections_requested()
            .then(|| flatten(node, &children, cx, file))
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            message: "Do not define public methods within class << self.".into(),
            correction,
        });
    }
}

fn visibility<'pr>(node: &Node<'pr>) -> Option<(bool, Vec<Node<'pr>>)> {
    let call = node.as_call_node()?;
    (call.receiver().is_none()
        && call
            .block()
            .is_none_or(|block| block.as_block_argument_node().is_some())
        && matches!(
            call.name().as_slice(),
            b"public" | b"protected" | b"private"
        ))
    .then(|| {
        (
            call.name().as_slice() != b"public",
            ast::call_arguments(&call),
        )
    })
}

// Same direct-sibling visibility policy as pinned VisibilityHelp: last named
// right sibling wins over a preceding blanket modifier. O(n), not O(defs*n).
fn all_public(children: &[Node<'_>]) -> bool {
    let mut named = std::collections::HashMap::new();
    let mut overrides = std::collections::HashMap::new();
    for child in children.iter().rev() {
        if let Some(def) = child.as_def_node()
            && let Some(&non_public) = named.get(def.name().as_slice())
        {
            overrides.insert(child.location().start_offset(), non_public);
        }
        if let Some((non_public, args)) = visibility(child) {
            for arg in args {
                if let Some(symbol) = arg.as_symbol_node() {
                    named
                        .entry(symbol.unescaped().to_vec())
                        .or_insert(non_public);
                }
            }
        }
    }
    let mut non_public = false;
    for child in children {
        if child.as_def_node().is_some_and(|d| d.receiver().is_none())
            && *overrides
                .get(&child.location().start_offset())
                .unwrap_or(&non_public)
        {
            return false;
        }
        if let Some((value, args)) = visibility(child)
            && args.is_empty()
        {
            non_public = value;
        }
    }
    true
}
fn no_code_after(node: &Node<'_>, file: &SourceFile) -> bool {
    let end = node.location().end_offset();
    let line_end = file.line_start(file.line_of(end) + 1).min(file.bytes.len());
    let trailing = &file.bytes[end..line_end];
    let whitespace = |&b: &u8| b == 0 || super::is_ruby_whitespace(b);
    let start = trailing
        .iter()
        .position(|b| !whitespace(b))
        .unwrap_or(trailing.len());
    let rest = &trailing[start..];
    let semicolons = rest.iter().take_while(|&&b| b == b';').count();
    let rest = &rest[semicolons..];
    let start = rest
        .iter()
        .position(|b| !whitespace(b))
        .unwrap_or(rest.len());
    rest[start..].is_empty() || rest[start..].starts_with(b"#")
}

// Deliberately do not move methods past accessors, callbacks, mixins or any
// other executable sibling. Do not flatten a lexical singleton scope unless
// *every* method has an inert body/defaults and thus cannot observe its CREF.
fn flatten(
    node: &Node<'_>,
    children: &[Node<'_>],
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let owner = cx.ancestors().find(|n| n.as_statements_node().is_none())?;
    declaration(owner)?;
    let bindings = file.class_method_bindings(cx.ancestors().last().unwrap_or(node));
    // Removing the wrapper shifts DATA's initial byte/line position even when
    // the __END__ payload itself is byte-for-byte unchanged.
    if !bindings.contains(owner) || bindings.observes_sources || bindings.observes_data {
        return None;
    }
    let singleton = node.as_singleton_class_node()?;
    let methods: Vec<_> = children
        .iter()
        .map(|n| n.as_def_node())
        .collect::<Option<_>>()?;
    if methods
        .iter()
        .any(|d| d.receiver().is_some() || !inert_method(&d.as_node()))
    {
        return None;
    }
    let start_line = file.line_of(node.location().start_offset());
    let end_line = file.last_line_of(node.location());
    if start_line == end_line {
        return None;
    }
    let header_end = singleton.expression().location().end_offset();
    if !file.bytes[header_end..file.line_start(start_line + 1)]
        .trim_ascii()
        .is_empty()
    {
        return None;
    }
    let wrapper_indent = &file.bytes[file.line_start(start_line)..node.location().start_offset()];
    if !wrapper_indent.iter().all(|&b| b == b' ') {
        return None;
    }
    let end_keyword = singleton.end_keyword_loc();
    if !file.bytes[file.line_start(end_line)..end_keyword.start_offset()]
        .iter()
        .all(|&b| b == b' ')
    {
        return None;
    }
    let first = methods.first()?;
    let method_indent = &file.bytes[file.line_start(file.line_of(first.location().start_offset()))
        ..first.location().start_offset()];
    if !method_indent.iter().all(|&b| b == b' ') || method_indent.len() <= wrapper_indent.len() {
        return None;
    }
    let diff = method_indent.len() - wrapper_indent.len();
    let range = node.location().start_offset()..node.location().end_offset();
    if super::correction::touches_directive(file, range.clone())
        || super::correction::contains_magic_comment_key(&file.bytes[range.clone()])
    {
        return None;
    }
    let body_start = file.line_start(start_line + 1);
    let body_end = file.line_start(end_line);
    let mut inserts = methods
        .iter()
        .map(|d| d.name_loc().start_offset())
        .peekable();
    let mut replacement = Vec::new();
    let mut offset = body_start;
    for line in file.bytes[body_start..body_end].split_inclusive(|&b| b == b'\n') {
        let remove = line.iter().take_while(|&&b| b == b' ').count().min(diff);
        for (i, &byte) in line.iter().enumerate().skip(remove) {
            if inserts.peek() == Some(&(offset + i)) {
                replacement.extend_from_slice(b"self.");
                inserts.next();
            }
            replacement.push(byte);
        }
        offset += line.len();
    }
    if inserts.next().is_some() {
        return None;
    }
    if replacement.ends_with(b"\r\n") {
        replacement.truncate(replacement.len() - 2);
    } else if replacement.ends_with(b"\n") {
        replacement.pop();
    }
    if !replacement.starts_with(wrapper_indent) {
        return None;
    }
    let correction = Correction {
        range,
        replacement: String::from_utf8(replacement[wrapper_indent.len()..].to_vec()).ok()?,
    };
    super::correction::preserves_line_values(file, &correction).then_some(correction)
}

fn inert_method(node: &Node<'_>) -> bool {
    use crate::cops::walk::{NodeVisitor, walk};
    struct Inert {
        safe: bool,
        root: usize,
    }
    impl<'pr> NodeVisitor<'pr> for Inert {
        fn enter(&mut self, node: Node<'pr>) {
            self.safe &= (node.as_def_node().is_some()
                && node.location().start_offset() == self.root)
                || matches!(
                    node,
                    Node::StatementsNode { .. }
                        | Node::ParametersNode { .. }
                        | Node::RequiredParameterNode { .. }
                        | Node::OptionalParameterNode { .. }
                        | Node::RequiredKeywordParameterNode { .. }
                        | Node::OptionalKeywordParameterNode { .. }
                        | Node::RestParameterNode { .. }
                        | Node::KeywordRestParameterNode { .. }
                        | Node::BlockParameterNode { .. }
                        | Node::LocalVariableReadNode { .. }
                        | Node::IntegerNode { .. }
                        | Node::FloatNode { .. }
                        | Node::SymbolNode { .. }
                        | Node::StringNode { .. }
                        | Node::TrueNode { .. }
                        | Node::FalseNode { .. }
                        | Node::NilNode { .. }
                        | Node::ArrayNode { .. }
                        | Node::HashNode { .. }
                        | Node::AssocNode { .. }
                        | Node::ReturnNode { .. }
                        | Node::ArgumentsNode { .. }
                );
            if matches!(node, Node::StringNode { .. } | Node::SymbolNode { .. }) {
                self.safe &= !node.location().as_slice().contains(&b'\n');
                self.safe &= !node.location().as_slice().starts_with(b"<<");
            }
        }
    }
    let mut proof = Inert {
        safe: true,
        root: node.location().start_offset(),
    };
    walk(&mut proof, node);
    proof.safe
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/class_methods_definitions_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    fn offenses(source: &str) -> Vec<Offense> {
        correction_offenses(&ClassMethodsDefinitions, source)
    }
    #[test]
    fn public_singleton_body_has_exact_offense() {
        let source = "class A\n  class << self\n    attr_reader :two\n\n    def three\n    end\n  end\nend\n";
        let found = offenses(source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 7));
        assert_eq!(
            found[0].message,
            "Do not define public methods within class << self."
        );
    }
    #[test]
    fn pinned_no_offenses_and_visibility_lookalikes() {
        for body in [
            "def one; end\nprivate\ndef two; end",
            "def one; end\ndef two; end\nprivate :one, :two",
            "attr_reader :one",
            "def self.one; end",
            "def b; end; end",
            "protected\ndef one; end",
            "private def one; end",
            "public def one; end",
            "def one; end\nprotected :one",
            "def one; end; ; # separated semicolons",
            "(def one; end\ndef two; end)",
            "private\ndef one; end\npublic\ndef two; end",
            "begin; def one; end; end",
            "",
        ] {
            // `def b; end; end` deliberately shares its line with the sclass end.
            let source = if body == "def b; end; end" {
                "class A\nclass << self\ndef b; end; end\nend".into()
            } else {
                format!("class A\nclass << self\n{body}\nend\nend")
            };
            assert!(offenses(&source).is_empty(), "{source}");
        }
        for source in [
            "class A; def self.one; end; end",
            "class A; class << other; def one; end; end; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn public_overrides_and_trailing_comments_are_detected() {
        for body in [
            "private\npublic\ndef one; end",
            "private\ndef one; end\npublic :one",
            "def one; end\nprivate :one\npublic :one",
            "def one; end # TODO",
            "def one; end; # TODO",
            "def one; end\x0b",
            "def one; end;;; # TODO",
            "def one; end\ninclude Mix\nextend Other",
        ] {
            let source = format!("class A\nclass << self\n{body}\nend\nend");
            assert_eq!(offenses(&source).len(), 1, "{source}");
        }
    }
    #[test]
    fn named_visibility_with_block_pass_works_in_both_directions() {
        for block in ["nil", "proc {}"] {
            for modifier in ["private", "protected"] {
                let source = format!(
                    "class A\n  class << self\n    def foo; end\n    {modifier} :foo, &{block}\n  end\nend\n"
                );
                assert!(offenses(&source).is_empty(), "{source}");
                let source = format!(
                    "class A\n  class << self\n    {modifier}\n    def foo; end\n    public :foo, &{block}\n  end\nend\n"
                );
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (2, 3));
                assert_eq!(
                    found[0].message,
                    "Do not define public methods within class << self."
                );
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn pinned_method_only_corrections_keep_comments_and_blank_lines() {
        for (body, expected) in [
            (
                "    def one\n      :one\n    end\n\n    def two\n      :two\n    end\n",
                "  def self.one\n    :one\n  end\n\n  def self.two\n    :two\n  end\n",
            ),
            (
                "    # See def two for usage\n    def two\n      :two\n    end\n",
                "  # See def two for usage\n  def self.two\n    :two\n  end\n",
            ),
            (
                "    def do_something\n      # TODO\n    end\n",
                "  def self.do_something\n    # TODO\n  end\n",
            ),
            (
                "    def do_something # TODO\n    end\n",
                "  def self.do_something # TODO\n  end\n",
            ),
            (
                "    def do_something\n    end # TODO\n",
                "  def self.do_something\n  end # TODO\n",
            ),
            (
                "    def do_something\n    end; # TODO\n",
                "  def self.do_something\n  end; # TODO\n",
            ),
        ] {
            let before = format!("class A\n  class << self\n{body}  end\nend\n");
            assert_correction(
                &ClassMethodsDefinitions,
                &before,
                &format!("class A\n{expected}end\n"),
            );
        }
        let before = "class A\n  def self.one\n  end\n\n  class << self\n    def two\n    end\n\n    def three\n    end\n  end\nend\n";
        let after = "class A\n  def self.one\n  end\n\n  def self.two\n  end\n\n  def self.three\n  end\nend\n";
        assert_correction(&ClassMethodsDefinitions, before, after);
    }
    #[test]
    fn lexical_and_source_observers_keep_offenses_without_corrections() {
        for body in [
            "def one; VALUE; end",
            "def one; Module.nesting; end",
            "def one; binding; end",
            "def one; eval('VALUE'); end",
            "def one; class_eval('VALUE'); end",
            "def one; @@value; end",
            "def one; super; end",
            "def one(x = VALUE); end",
            "def one; def two; :two; end; end",
            "def one\n  <<~TEXT\n    value\n  TEXT\nend",
            "def one; \"multi\n  line\"; end",
            "def one; __LINE__; end",
            "def one; -> { :one }; end",
        ] {
            let source = format!("class A\n  class << self\n    {body}\n  end\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        for suffix in [
            "p __LINE__",
            "p A.method(:one).source_location",
            "p exception.backtrace",
            "p exception.backtrace_locations",
            "p exception.full_message",
            "p A.singleton_class.instance_method(:one)",
            "p eval('A.method(:one).source_location')",
            "TracePoint.new(:call) { |t| p t.lineno }.enable { A.one }",
        ] {
            let source =
                format!("class A\n  class << self\n    def one; :one; end\n  end\nend\n{suffix}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        for source in [
            "class A\n  class << self # retain wrapper\n    def one; end\n  end\nend\n",
            "class A\n  class << self\n    # frozen_string_literal: true\n    def one; end\n  end\nend\n",
            "class A\n  class << self\n    # rubocop:disable Style/StringLiterals\n    def one; end\n  end\nend\n",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn data_observations_withhold_flattening_not_diagnostics() {
        let before = "class A\n  class << self\n    def one; :one; end\n  end\nend\n";
        for observer in [
            "p [DATA.pos, DATA.lineno, DATA.read]",
            "p ::DATA.pos",
            "p Object::DATA.lineno",
            "p ::Object::DATA.read",
            "Root = Object; p Root::DATA.pos",
            "Handle = DATA; p Handle.lineno",
            "p Object.const_get(:DATA).read",
        ] {
            let source = format!("{before}{observer}\n__END__\nraw\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 4));
            assert_eq!(
                found[0].message,
                "Do not define public methods within class << self."
            );
            assert!(found[0].correction.is_none(), "{source}");
        }
        assert_correction(
            &ClassMethodsDefinitions,
            &format!("{before}# DATA.pos\np 'DATA.lineno'\n__END__\nDATA.read\n"),
            "class A\n  def self.one; :one; end\nend\n# DATA.pos\np 'DATA.lineno'\n__END__\nDATA.read\n",
        );
    }
    #[test]
    fn individually_withholds_pinned_reordering_and_nested_corrections() {
        for body in [
            "attr_reader :two\n\ndef three; end",
            "def three; end\nattr_reader :two",
            "attr_reader :one\n\n# Multiline\n# comment.\ndef two; end",
            "def self.one; end\n\ndef two; end",
            "def one\nfoo.instance_exec do\nclass << self\ndef two; end\nend\nend\nend",
        ] {
            let source = format!("class A\nclass << self\n{body}\nend\nend");
            let found = offenses(&source);
            assert_eq!(
                found.len(),
                if body.contains("instance_exec") { 2 } else { 1 },
                "{source}"
            );
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        }
    }
}
