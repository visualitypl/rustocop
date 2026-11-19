//! `Style/StructInheritance` — detection only: changing ancestors is unsafe.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct StructInheritance;
impl Cop for StructInheritance {
    fn name(&self) -> &'static str {
        "Style/StructInheritance"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        check_inheritance(self, node, cx, file, out, ("Struct", b"new", "struct"));
    }
}

/// Shared structural policy, not runtime constant resolution: the pin matches
/// bare/rooted names even when shadowed.
pub(super) fn check_inheritance(
    cop: &dyn Cop,
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
    (constant, method, noun): (&str, &[u8], &str),
) {
    let Some(class) = node.as_class_node() else {
        return;
    };
    let Some(parent) = class.superclass() else {
        return;
    };
    let Some(call) = parent.as_call_node() else {
        return;
    };
    if call.is_safe_navigation()
        || call.name().as_slice() != method
        || !call
            .receiver()
            .is_some_and(|r| ast::is_global_const(&r, &[constant]))
    {
        return;
    }
    // RuboCop's ordinary `block` pattern excludes `numblock` and `itblock`.
    if call
        .block()
        .and_then(|block| block.as_block_node())
        .is_some_and(|block| {
            matches!(
                block.parameters(),
                Some(Node::NumberedParametersNode { .. } | Node::ItParametersNode { .. })
            )
        })
    {
        return;
    }
    if class.body().is_some() {
        let root = cx.ancestors().last().unwrap_or(node);
        if file.inheritance_constants(root).contains_class(node) {
            return;
        }
    }
    let correction = cx
        .corrections_requested()
        .then(|| correction(&class, &call, constant == "Data", file))
        .flatten();
    let (line, col) = file.line_col(parent.location().start_offset());
    let method = std::str::from_utf8(method).expect("fixed ASCII method");
    out.push(Offense {
        cop: cop.name(), severity: cop.severity(), correction,
        message: format!("Don't extend an instance initialized by `{constant}.{method}`. Use a block to customize the {noun}."),
        line, col, end_line: file.last_line_of(parent.location()),
    });
}

/// Upstream's corrector: `class Name < Struct.new(...)` becomes
/// `Name = Struct.new(...) do`, the class body becoming the block's.
fn correction(
    class: &ruby_prism::ClassNode<'_>,
    call: &ruby_prism::CallNode<'_>,
    data: bool,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    use crate::offense::space_right_blank;
    let bytes = &file.bytes;
    let span = |l: &ruby_prism::Location<'_>| l.start_offset()..l.end_offset();
    let blank_left = |mut pos: usize| {
        while pos > 0 && matches!(bytes[pos - 1], b' ' | b'\t') {
            pos -= 1;
        }
        pos
    };
    let keyword = class.class_keyword_loc();
    // Struct removes the blanks after `class`; Data those around it.
    let keyword_start = if data {
        blank_left(keyword.start_offset())
    } else {
        keyword.start_offset()
    };
    let mut edits = vec![
        (
            keyword_start..space_right_blank(bytes, keyword.end_offset()),
            String::new(),
        ),
        (span(&class.inheritance_operator_loc()?), "=".to_owned()),
    ];
    let parent_end = call.location().end_offset();
    if let Some(block) = call.block().and_then(|b| b.as_block_node()) {
        let close = block.closing_loc();
        edits.push((
            blank_left(close.start_offset())..space_right_blank(bytes, close.end_offset()),
            String::new(),
        ));
        if data && block.opening_loc().as_slice() == b"{" {
            edits.push((span(&block.opening_loc()), "do".to_owned()));
        }
    } else if class.body().is_none() {
        let end = class.end_keyword_loc();
        let single =
            file.line_of(class.location().start_offset()) == file.line_of(end.start_offset());
        edits.push((
            if single {
                parent_end..class.location().end_offset()
            } else {
                crate::cops::style::correction::whole_lines(file, span(&end))
            },
            String::new(),
        ));
    } else if !data && call.opening_loc().is_none() && !ast::call_arguments(call).is_empty() {
        let arguments = ast::call_arguments(call)
            .iter()
            .map(|a| String::from_utf8_lossy(a.location().as_slice()).into_owned())
            .collect::<Vec<_>>()
            .join(", ");
        edits.push((
            call.message_loc()?.end_offset()..parent_end,
            format!("({arguments}) do"),
        ));
    } else {
        edits.push((parent_end..parent_end, " do".to_owned()));
    }
    crate::offense::Correction::from_edits(bytes, edits)
}

/// One lazy complete main-AST walk for both cops, then constant-time queries.
/// Subtrees, not source containment: heredoc children may extend past a body's
/// location. The temporary visitor stack is discarded before caching results.
#[derive(Default)]
pub(crate) struct ConstantDefinitions {
    classes: std::collections::HashSet<usize>,
    #[cfg(test)]
    visits: usize,
}
impl ConstantDefinitions {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut visitor = DefinitionVisitor {
            index: Self::default(),
            stack: Vec::new(),
            definitions: 0,
        };
        walk(&mut visitor, root);
        visitor.index
    }
    fn contains_class(&self, class: &Node<'_>) -> bool {
        self.classes.contains(&class.location().start_offset())
    }
}
struct DefinitionVisitor<'pr> {
    index: ConstantDefinitions,
    // Each direct class-body child remembers its owning class and the global
    // definition count on entry; a changed count on leave means an exemption.
    stack: Vec<(Node<'pr>, Option<(usize, usize)>)>,
    definitions: usize,
}
impl<'pr> NodeVisitor<'pr> for DefinitionVisitor<'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        {
            self.index.visits += 1;
        }
        let body_owner = self.stack.last().and_then(|(parent, _)| {
            let class = parent.as_class_node()?;
            let body = class.body()?;
            (body.location().start_offset() == node.location().start_offset()
                && body.location().end_offset() == node.location().end_offset())
            .then_some((parent.location().start_offset(), self.definitions))
        });
        // Compound writes contain a casgn in RuboCop, but are dedicated Prism
        // nodes. Destructured targets also represent casgns without a value.
        if matches!(
            node,
            Node::ClassNode { .. }
                | Node::ModuleNode { .. }
                | Node::ConstantWriteNode { .. }
                | Node::ConstantPathWriteNode { .. }
                | Node::ConstantOrWriteNode { .. }
                | Node::ConstantPathOrWriteNode { .. }
                | Node::ConstantAndWriteNode { .. }
                | Node::ConstantPathAndWriteNode { .. }
                | Node::ConstantOperatorWriteNode { .. }
                | Node::ConstantPathOperatorWriteNode { .. }
                | Node::ConstantTargetNode { .. }
                | Node::ConstantPathTargetNode { .. }
        ) {
            self.definitions += 1;
        }
        self.stack.push((node, body_owner));
    }
    fn leave(&mut self) {
        if let Some((_, Some((class, before)))) = self.stack.pop()
            && self.definitions > before
        {
            self.index.classes.insert(class);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    //! Examples ported from RuboCop's style/struct_inheritance_spec.rb and
    //! style/data_inheritance_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};

    pub fn offenses(source: &str, cop: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == cop)
            .collect()
    }

    pub fn pinned_offenses(cop: &dyn Cop, constructor: &str, message: &str) {
        for receiver in [constructor.to_string(), format!("::{constructor}")] {
            for (suffix, body) in [
                ("(:first_name, :last_name)", "  def foo; end\n"),
                (" :first_name, :last_name", "  def foo; end\n"),
                ("(:first_name, :last_name) do end", ""),
                ("(:first_name, :last_name) { }", ""),
                ("(:first_name, :last_name) { |record| record }", ""),
                ("(:first_name, :last_name) do |record| record end", ""),
                ("(:first_name, :last_name) { || self }", ""),
                ("(:first_name, :last_name) { values.map { _1 } }", ""),
                ("(:first_name, :last_name) { def age = 42 }", ""),
                ("(:first_name, :last_name)", ""),
            ] {
                for prefix in ["", "# comment\n"] {
                    let source = format!("{prefix}class Person < {receiver}{suffix}\n{body}end\n");
                    let found = offenses(&source, cop.name());
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!(
                        (found[0].line, found[0].col),
                        (1 + u32::from(!prefix.is_empty()), 16)
                    );
                    assert_eq!(found[0].message, message);
                    assert!(found[0].correction.is_none());
                    let requested = correction_offenses(cop, &source);
                    assert_eq!(requested.len(), 1);
                    assert!(requested[0].correction.is_some());
                }
            }
        }
        for (source, line, col, end_line) in [
            (format!("class Person < {constructor}(:a); end\n"), 1, 16, 1),
            (
                format!(
                    "module M\n  class Person < {constructor}(\n    :a\n  ) {{\n    def foo; end\n  }}\n  end\nend\n"
                ),
                2,
                18,
                6,
            ),
        ] {
            let found = offenses(&source, cop.name());
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, end_line)
            );
            assert_eq!(found[0].message, message);
        }
    }

    pub fn constant_exemptions(cop: &str, constructor: &str) {
        for body in [
            "MAX_AGE = 120",
            "if something; MAX_AGE = 120; end",
            "class Error < StandardError; end",
            "module Helpers; end",
            "wrapper { Other::VALUE = 1 }",
            "begin; action; rescue; VALUE = 1; ensure; other; end",
            "class << self; VALUE = 1; end",
            "A ||= 1",
            "::A &&= 1",
            "Other::A += 1",
            "A, b = values",
            "text = <<~RUBY\n#{VALUE = 1}\nRUBY",
        ] {
            let source = format!("class Person < {constructor}(:a)\n{body}\nend\n");
            assert!(offenses(&source, cop).is_empty(), "{source}");
        }
        // Constants in the superclass block are not in the class body.
        let source = format!("class Person < {constructor}(:a) {{ VALUE = 1 }}; end\n");
        assert_eq!(offenses(&source, cop).len(), 1);
    }

    pub fn lookalikes(cop: &str, constructor: &str) {
        for source in [
            "class Person; end".into(),
            "class Person < DelegateClass(Animal); end".into(),
            format!("Person = {constructor}(:a)"),
            format!("Person = ::{constructor}(:a) {{ def age; 42; end }}"),
            format!("class Person < Other::{constructor}(:a); end"),
            format!("class Person < {constructor}(:a).other; end"),
            format!("class Person < {}(:a); end", constructor.replace('.', "&.")),
            format!("class Person < ({constructor}(:a)); end"),
            format!("class Person < {constructor}(:a) {{ _1 }}; end"),
            format!("class Person < ::{constructor}(:a) {{ _2 }}; end"),
            format!("class Person < {constructor}(:a) do _1 end; end"),
            format!("class Person < {constructor}(:a) {{ it }}; end"),
            format!("class Person < ::{constructor}(:a) do it end; end"),
            "class Person < Struct.define(:a); end".into(),
            "class Person < Data.new(:a); end".into(),
            "class Person < factory.new(:a); end".into(),
        ] {
            assert!(offenses(&source, cop).is_empty(), "{source}");
        }
    }

    #[test]
    fn lexical_reopened_singleton_and_shadowed_names_match_syntax_not_runtime_identity() {
        for constructor in ["Struct.new", "Data.define"] {
            let cop = if constructor == "Struct.new" {
                "Style/StructInheritance"
            } else {
                "Style/DataInheritance"
            };
            for source in [
                format!(
                    "module M; class Person < {constructor}(:a); private; def foo; end; end; end"
                ),
                format!("class M::Person < ::{constructor}(:a); include Mix; extend Other; end"),
                format!("class Person < {constructor}(:a); end; class Person; def foo; end; end"),
                format!("class << obj; class Person < {constructor}(:a); end; end"),
                format!(
                    "module M; {} = factory; class Person < {constructor}(:a); end; end",
                    constructor.split('.').next().unwrap()
                ),
                format!(
                    "class Base; {} = factory; end; class Outer < Base; class Person < {constructor}(:a); end; end",
                    constructor.split('.').next().unwrap()
                ),
            ] {
                assert_eq!(offenses(&source, cop).len(), 1, "{source}");
            }
        }
    }

    #[test]
    fn registered_ruby_gates_cannot_be_bypassed_by_force_selection() {
        use crate::{
            config::ResolvedConfig,
            cops::{Registry, policy::RubyVersion, select_active_at_version},
        };
        let registry = Registry::new();
        let config = ResolvedConfig::default();
        for version in [RubyVersion(3, 1), RubyVersion(3, 2), RubyVersion(3, 4)] {
            for cop in [
                "Style/DataInheritance",
                "Style/StructInheritance",
                "Style/RedundantStructKeywordInit",
            ] {
                let selected =
                    select_active_at_version(&registry, &config, &[cop.into()], &[], version);
                assert_eq!(
                    selected.len(),
                    usize::from(cop == "Style/StructInheritance" || version >= RubyVersion(3, 2))
                );
            }
        }
    }

    #[test]
    fn shared_constant_index_is_built_once_with_linear_node_work() {
        for count in [10, 100, 1000] {
            let source = (0..count).map(|i| format!("class C{i} < Struct.new(:a); call({{k: 1}}); end\nclass D{i} < Data.define(:b); VALUE = 1; end\n")).collect::<String>();
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0);
            let root = parsed.node();
            let file = SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
            let index = file.inheritance_constants(&root);
            assert_eq!(index.classes.len(), count);
            assert!(index.visits < count * 30);
            for _ in 0..100 {
                assert!(std::ptr::eq(index, file.inheritance_constants(&root)));
            }
            assert_eq!(file.inheritance_constants(&root).visits, index.visits);
        }
    }

    #[test]
    fn pinned_struct_offenses_and_unsafe_corrections() {
        pinned_offenses(
            &StructInheritance,
            "Struct.new",
            "Don't extend an instance initialized by `Struct.new`. Use a block to customize the struct.",
        );
    }
    #[test]
    fn struct_constant_definitions_exempt_the_class() {
        constant_exemptions("Style/StructInheritance", "Struct.new");
    }
    #[test]
    fn struct_lookalikes_are_ignored() {
        lookalikes("Style/StructInheritance", "Struct.new");
    }
}
