//! `Style/AccessModifierDeclarations` — fixed group style, detection only.

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct AccessModifierDeclarations;
impl Cop for AccessModifierDeclarations {
    fn name(&self) -> &'static str {
        "Style/AccessModifierDeclarations"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Process sibling groups once, not a suffix scan for every modifier.
        // Singleton Statements and all argument lists are transparent in
        // RuboCop's AST. Class-like nodes own their singleton bodies; direct
        // single-statement if/block bodies have an exempt parent.
        let children: Vec<_> = if let Some(statements) = node.as_statements_node() {
            if statements.body().len() == 1
                && cx.parent().is_some_and(|parent| {
                    matches!(
                        parent,
                        Node::ClassNode { .. }
                            | Node::ModuleNode { .. }
                            | Node::SingletonClassNode { .. }
                            | Node::BlockNode { .. }
                            | Node::LambdaNode { .. }
                            | Node::IfNode { .. }
                            | Node::UnlessNode { .. }
                            | Node::ElseNode { .. }
                    )
                })
            {
                return;
            }
            if !super::method_call_with_args_parentheses::macro_scope(node, cx) {
                return;
            }
            statements.body().iter().collect()
        } else if let Some(class) = node.as_class_node() {
            class
                .superclass()
                .into_iter()
                .chain(singleton_body(class.body()))
                .collect()
        } else if let Some(module) = node.as_module_node() {
            singleton_body(module.body()).into_iter().collect()
        } else if let Some(class) = node.as_singleton_class_node() {
            std::iter::once(class.expression())
                .chain(singleton_body(class.body()))
                .collect()
        } else if super::method_call_with_args_parentheses::class_constructor(node) {
            let call = node.as_call_node().unwrap();
            call.receiver()
                .into_iter()
                .chain(ast::call_arguments(&call))
                .collect()
        } else {
            return;
        };
        check_group(&children, file, out);
    }
}

fn allowed_arguments(args: &[Node<'_>]) -> bool {
    // Anonymous `*` is forwarded_restarg upstream, not a dynamic list splat.
    if args.iter().any(|arg| {
        arg.as_splat_node()
            .is_some_and(|splat| splat.expression().is_some())
    }) || args.iter().all(|arg| arg.as_symbol_node().is_some())
    {
        return true;
    }
    let [arg] = args else { return false };
    arg.as_call_node().is_some_and(|call| {
        call.receiver().is_none()
            && !call.is_safe_navigation()
            && match call.name().as_slice() {
                b"attr" | b"attr_reader" | b"attr_writer" | b"attr_accessor" => {
                    !ast::call_arguments(&call).is_empty()
                }
                b"alias_method" => ast::call_arguments(&call).len() == 2,
                _ => false,
            }
            && !call.block().is_some_and(|b| b.as_block_node().is_some())
    })
}

fn singleton_body(body: Option<Node<'_>>) -> Option<Node<'_>> {
    let statements = body?.as_statements_node()?;
    (statements.body().len() == 1)
        .then(|| statements.body().iter().next())
        .flatten()
}

/// Returns the number of direct siblings examined, for bounded-work tests.
fn check_group(children: &[Node<'_>], file: &SourceFile, out: &mut Vec<Offense>) -> usize {
    let mut seen = [false; 4];
    let mut visited = 0;
    for child in children.iter().rev() {
        visited += 1;
        let Some(call) = child.as_call_node() else {
            continue;
        };
        let Some(index) = [
            b"private".as_slice(),
            b"protected",
            b"public",
            b"module_function",
        ]
        .iter()
        .position(|name| *name == call.name().as_slice()) else {
            continue;
        };
        if call.receiver().is_some() || call.block().is_some_and(|b| b.as_block_node().is_some()) {
            continue;
        }
        let args = ast::call_arguments(&call);
        if args.is_empty() || allowed_arguments(&args) || std::mem::replace(&mut seen[index], true)
        {
            continue;
        }
        let (line, col) = file.line_col(call.message_loc().unwrap().start_offset());
        let cop = AccessModifierDeclarations;
        out.push(Offense {
            cop: cop.name(),
            severity: cop.severity(),
            message: format!(
                "`{}` should not be inlined in method definitions.",
                String::from_utf8_lossy(call.name().as_slice())
            ),
            line,
            col,
            end_line: line,
            correction: None,
        });
    }
    visited
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's
    //! `spec/rubocop/cop/style/access_modifier_declarations_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};

    const MODIFIERS: [&str; 4] = ["private", "protected", "public", "module_function"];
    fn offenses(source: &str) -> Vec<Offense> {
        let parsed = ruby_prism::parse(source.as_bytes());
        assert!(
            parsed.errors().next().is_none(),
            "invalid fixture: {source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/AccessModifierDeclarations")
            .collect()
    }
    fn assert_positions(source: &str, modifier: &str, positions: &[(u32, u32)]) {
        let found = offenses(source);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            positions,
            "{source}"
        );
        for offense in found {
            assert_eq!(offense.end_line, offense.line);
            assert_eq!(
                offense.message,
                format!("`{modifier}` should not be inlined in method definitions.")
            );
            assert!(offense.correction.is_none());
        }
    }
    #[test]
    fn accepts_group_symbols_dynamic_lists_attrs_and_aliases() {
        for modifier in MODIFIERS {
            for form in [
                "",
                " # hey",
                " :bar",
                " :bar, :baz",
                " *%i[bar baz]",
                " *%I[bar #{name}]",
                " *METHOD_NAMES",
                " *bar",
                "(*names)",
                "(*[:bar, :baz])",
                "(:bar, *names)",
                " attr_reader :foo",
                " attr_writer :bar, :baz",
                " attr_accessor :foo",
                " attr :qux",
                " alias_method :bar, :foo",
            ] {
                let source = format!("class Foo\n  names = []\n  {modifier}{form}\nend\n");
                assert_positions(&source, modifier, &[]);
            }
        }
    }
    #[test]
    fn anonymous_rest_forwarding_is_not_a_dynamic_method_list() {
        for modifier in MODIFIERS {
            for argument in ["*", ":foo, *"] {
                let source = format!("def f(*); Class.new({modifier}({argument})); end");
                assert_positions(&source, modifier, &[(1, 21)]);
            }
            for argument in ["*names", ":foo, *names", "*, *names"] {
                let source =
                    format!("def f(*); names = []; Class.new({modifier}({argument})); end");
                assert_positions(&source, modifier, &[]);
            }
        }
    }
    #[test]
    fn respects_direct_if_block_and_receiver_exemptions() {
        for modifier in MODIFIERS {
            for source in [
                format!("class Foo; bar(key: {modifier}); end"),
                format!("module M; methods.each {{ |method| {modifier}(method) }}; end"),
                format!("module M; methods.each {{ {modifier}(_1) }}; end"),
                format!("module M; methods.each {{ {modifier}(it) }}; end"),
                format!("class Foo; {modifier} name if condition; end"),
                format!("class Foo; if condition; {modifier} name; end; end"),
                format!("class Foo; unless condition; {modifier} name; end; end"),
                format!("class Foo; condition ? {modifier}(name) : other; end"),
                format!("class Foo; self.{modifier}(name); end"),
                format!("class Foo; Other&.{modifier}(name); end"),
                format!("{modifier}(name) {{ body }}"),
                "private_class_method def self.foo; end".to_string(),
                format!("def f; {modifier} name; end"),
                format!("# {modifier} def foo; end\n\"{modifier} def foo; end\""),
                format!("def {modifier}(name); name; end"),
            ] {
                assert_positions(&source, modifier, &[]);
            }
        }
    }
    #[test]
    fn reports_only_last_groupable_same_modifier_sibling() {
        for modifier in MODIFIERS {
            let source = format!(
                "class Test\n  {modifier} def foo; end\n  {modifier} def bar; end\n  def baz; end\n  QUX = ['qux']\nend\n"
            );
            assert_positions(&source, modifier, &[(3, 3)]);
            // A different visibility declaration, allowed attr/symbol, or
            // conditional does not reset or join upstream's sibling group.
            let source = format!(
                "class Test\n  {modifier} def foo; end\n  public\n  {modifier} :foo\n  {modifier} attr_reader :x\n  {modifier} name if condition\n  {modifier} def bar; end\nend\n"
            );
            assert_positions(&source, modifier, &[(7, 3)]);
            let source = format!(
                "class One\n  {modifier} def foo; end\nend\nclass Two\n  {modifier} def foo; end\nend\n"
            );
            assert_positions(&source, modifier, &[(2, 3), (5, 3)]);
        }
        let source = "module M\n  private def first; end\n  protected def second; end\n  public def third; end\n  private def fourth; end\n  module_function def fifth; end\nend\n";
        let found = offenses(source);
        assert_eq!(found.len(), 4);
        for (offense, (line, modifier)) in found.iter().zip([
            (3, "protected"),
            (4, "public"),
            (5, "private"),
            (6, "module_function"),
        ]) {
            assert_eq!(
                (offense.line, offense.col, offense.end_line),
                (line, 3, line)
            );
            assert_eq!(
                offense.message,
                format!("`{modifier}` should not be inlined in method definitions.")
            );
        }
    }
    #[test]
    fn detects_singleton_endless_and_visibility_scopes() {
        for modifier in MODIFIERS {
            for definition in [
                "def foo; end",
                "def self.foo; end",
                "def Other.foo; end",
                "def foo = 1",
                "def self.foo = 1",
            ] {
                for opening in [
                    "class Foo",
                    "class Foo < Parent",
                    "module Foo",
                    "class << self",
                    "class Outer::Foo",
                ] {
                    let source = format!("{opening}\n  {modifier} {definition}\nend\n");
                    assert_positions(&source, modifier, &[(2, 3)]);
                }
            }
            // Explicit grouping, and multi-statement conditional/block bodies,
            // have a begin parent rather than the exempt if/block parent.
            for source in [
                format!("class Foo\n  if condition\n    nil\n    {modifier} name\n  end\nend\n"),
                format!("class Foo\n  factory do\n    nil\n    {modifier} name\n  end\nend\n"),
                format!("class Foo\n  -> do\n    nil\n    {modifier} name\n  end\nend\n"),
            ] {
                assert_positions(&source, modifier, &[(4, 5)]);
            }
            assert_positions(
                &format!("class Foo\n  begin\n    {modifier} name\n  end\nend"),
                modifier,
                &[(3, 5)],
            );
            assert_positions(
                &format!("class Foo\n  factory {{ ({modifier} name) }}\nend"),
                modifier,
                &[(2, 14)],
            );
            for source in [
                format!("class Foo; while condition; nil; {modifier} name; end; end"),
                format!("class Foo; begin; {modifier} name; rescue; nil; end; end"),
                format!("class Foo; begin; nil; ensure; {modifier} name; end; end"),
                format!("class Foo; result = ({modifier} name); end"),
                format!("class Foo; consume({modifier} name); end"),
                format!("class Foo; case value; when 1; nil; {modifier} name; end; end"),
                format!("class Foo; if {modifier}(name); nil; end; end"),
            ] {
                assert_positions(&source, modifier, &[]);
            }
        }
    }
    #[test]
    fn constructors_and_class_children_have_shared_siblings() {
        assert_positions(
            "Class.new(private(name), private(other))",
            "private",
            &[(1, 26)],
        );
        assert_positions(
            "class C < private(name)\n  private(other)\nend",
            "private",
            &[(2, 3)],
        );
        assert_positions(
            "class << private(name)\n  private(other)\nend",
            "private",
            &[(2, 3)],
        );
        for constructor in ["Class.new", "::Module.new", "Struct.new", "::Data.define"] {
            let source = format!("def outer\n  {constructor}(private(name))\nend");
            assert_positions(&source, "private", &[(2, (constructor.len() + 4) as u32)]);
            let source = format!(
                "def outer\n  {constructor} do\n    nil\n    private def foo; end\n  end\nend"
            );
            assert_positions(&source, "private", &[(4, 5)]);
        }
        for constructor in ["Other::Class.new", "klass.new", "Class&.new"] {
            let source = format!(
                "def outer\n  {constructor} do\n    nil\n    private def foo; end\n  end\nend"
            );
            assert_positions(&source, "private", &[]);
        }
    }
    #[test]
    fn exact_argument_shapes_do_not_overextend_exemptions() {
        for modifier in MODIFIERS {
            for argument in [
                "name",
                "\"foo\"",
                "attr_reader",
                "attr_accessor()",
                "Other.attr_reader(:x)",
                "self.attr_writer(:x)",
                "alias_method(:x)",
                "alias_method(:x, :y, :z)",
                "Other.alias_method(:x, :y)",
                "attr_reader(:x) { body }",
                "&block",
                "**names",
                ":foo, name",
                "attr_reader(:x), :foo",
                ":\"#{name}\"",
            ] {
                let source = format!("class Foo\n  {modifier}({argument})\nend\n");
                assert_positions(&source, modifier, &[(2, 3)]);
            }
            // Block-pass is an argument; literal blocks instead exempt the send.
            assert_positions(
                &format!("{modifier}(attr_reader(:x, &block))"),
                modifier,
                &[],
            );
            assert_positions(
                &format!("{modifier}(attr_reader(:x), &block)"),
                modifier,
                &[(1, 1)],
            );
        }
    }
    #[test]
    fn comments_documentation_and_conditionals_keep_group_boundaries() {
        let source = "class Test\n  private name if condition\n  # Docs for foo.\n  private def foo\n    # Body comment.\n  end # trailing\n  private other if condition\nend\n";
        assert_positions(source, "private", &[(4, 3)]);
        let source = "class Test\n  private def foo; end\n  # Docs for bar.\n  private def bar; end # trailing\n  private\n  # Docs stay on baz.\n  def baz; end\nend\n";
        assert_positions(source, "private", &[(4, 3)]);
        assert_positions(
            "class TestOne\n  private\nend\nclass TestTwo\n  private def foo; end\nend\nclass TestThree\n  private def foo; end\nend\n",
            "private",
            &[(5, 3), (8, 3)],
        );
    }
    #[test]
    fn detection_only_never_emits_corrections_even_when_requested() {
        for modifier in MODIFIERS {
            for source in [
                format!("{modifier} def foo; end\n"),
                format!("class Foo\n  {modifier} def foo; end\n  {modifier} def bar; end\nend\n"),
                format!("class << self\n  # docs\n  {modifier} def foo = __LINE__\nend\n"),
            ] {
                let found = crate::cops::testing::correction_offenses(
                    &super::AccessModifierDeclarations,
                    &source,
                );
                assert_eq!(found.len(), 1, "{source}");
                assert!(found[0].correction.is_none(), "{source}");
            }
        }
        assert!(!crate::cops::Cop::safe_fixes(
            &super::AccessModifierDeclarations
        ));
    }
    #[test]
    fn flat_sibling_work_is_linear_without_cross_scope_grouping() {
        let source = format!("{}private(name)\n", "private(other)\n".repeat(4000));
        let parsed = ruby_prism::parse(source.as_bytes());
        let children: Vec<_> = parsed
            .node()
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .iter()
            .collect();
        let file = crate::source::SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
        let mut found = Vec::new();
        assert_eq!(super::check_group(&children, &file, &mut found), 4001);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (4001, 1));
        assert_positions(&source, "private", &[(4001, 1)]);
    }
    #[test]
    fn flags_inline_definitions_in_class_and_at_top_level() {
        for modifier in MODIFIERS {
            assert_positions(
                &format!("class Test\n  {modifier} def foo; end\nend\n"),
                modifier,
                &[(2, 3)],
            );
            assert_positions(&format!("{modifier} def foo; end\n"), modifier, &[(1, 1)]);
        }
    }
}
