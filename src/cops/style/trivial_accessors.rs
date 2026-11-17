//! `Style/TrivialAccessors` — prefer the attribute family for exact-name accessors.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{DefNode, Node};
pub struct TrivialAccessors;
impl Cop for TrivialAccessors {
    fn name(&self) -> &'static str {
        "Style/TrivialAccessors"
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
        let Some(kind) = kind(&def) else {
            return;
        };
        if exempt_scope(cx) {
            return;
        }
        let (line, col) = file.line_col(def.def_keyword_loc().start_offset());
        let in_call = cx
            .ancestors()
            .find(|n| n.as_arguments_node().is_none())
            .is_some_and(|n| n.as_call_node().is_some());
        let correction = (cx.corrections_requested() && !in_call)
            .then(|| replacement(node, &def, kind, file))
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `attr_{kind}` to define trivial {kind} methods."),
            line,
            col,
            end_line: line,
            correction,
        });
    }
}

fn kind(def: &DefNode<'_>) -> Option<&'static str> {
    let name = def.name().as_slice();
    if matches!(
        name,
        b"initialize"
            | b"to_ary"
            | b"to_a"
            | b"to_c"
            | b"to_enum"
            | b"to_h"
            | b"to_hash"
            | b"to_i"
            | b"to_int"
            | b"to_io"
            | b"to_open"
            | b"to_path"
            | b"to_proc"
            | b"to_r"
            | b"to_regexp"
            | b"to_str"
            | b"to_s"
            | b"to_sym"
    ) {
        return None;
    }
    let body = def.body()?.as_statements_node()?;
    let mut statements = body.body().iter();
    let only = statements.next()?;
    if statements.next().is_some() {
        return None;
    }
    if let Some(read) = only.as_instance_variable_read_node() {
        return (def.parameters().is_none()
            && name.strip_suffix(b"=").unwrap_or(name) == &read.name().as_slice()[1..])
            .then_some("reader");
    }
    let write = only.as_instance_variable_write_node()?;
    let parameters = def.parameters()?;
    if parameters.requireds().len() != 1
        || !parameters.optionals().is_empty()
        || parameters.rest().is_some()
        || !parameters.posts().is_empty()
        || !parameters.keywords().is_empty()
        || parameters.keyword_rest().is_some()
        || parameters.block().is_some()
    {
        return None;
    }
    let parameter = parameters
        .requireds()
        .iter()
        .next()?
        .as_required_parameter_node()?;
    let value = write.value().as_local_variable_read_node()?;
    (name.strip_suffix(b"=") == Some(&write.name().as_slice()[1..])
        && value.name().as_slice() == parameter.name().as_slice())
    .then_some("writer")
}

fn exempt_scope(cx: &NodeContext<'_, '_>) -> bool {
    // RuboCop omits Program and singleton Statements wrappers. Only a lone
    // root method has no parent; multiple top-level definitions are checked.
    let mut parents = cx.ancestors();
    if let Some(statements) = parents.next().and_then(Node::as_statements_node)
        && statements.body().len() == 1
        && parents
            .next()
            .is_some_and(|n| n.as_program_node().is_some())
    {
        return true;
    }
    let mut parents = cx.ancestors().peekable();
    while let Some(parent) = parents.next() {
        match parent {
            Node::ClassNode { .. } | Node::SingletonClassNode { .. } => return false,
            Node::ModuleNode { .. } => return true,
            Node::BlockNode { .. }
                if parents
                    .peek()
                    .and_then(|n| n.as_call_node())
                    .is_some_and(|c| c.name().as_slice() == b"instance_eval") =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

/// RuboCop's `autocorrect_instance` and `autocorrect_class`.
fn replacement(
    node: &Node<'_>,
    def: &DefNode<'_>,
    kind: &str,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    let name = def.name().as_slice();
    if name.ends_with(b"?") {
        return None;
    }
    let name = std::str::from_utf8(name.strip_suffix(b"=").unwrap_or(name)).ok()?;
    let attr = format!("attr_{kind} :{name}");
    let loc = node.location();
    let text = match def.receiver() {
        None => attr,
        Some(receiver) if receiver.as_self_node().is_some() => {
            let start = loc.start_offset();
            let line_start = file.line_start(file.line_of(start));
            let indent = " ".repeat(
                std::str::from_utf8(&file.bytes[line_start..start])
                    .ok()?
                    .chars()
                    .count(),
            );
            format!("class << self\n{indent}  {attr}\n{indent}end")
        }
        Some(_) => return None,
    };
    Some(crate::offense::Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement: text,
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/trivial_accessors_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        assert_eq!(ruby_prism::parse(s.as_bytes()).errors().count(), 0, "{s}");
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/TrivialAccessors")
            .collect()
    }
    #[test]
    fn exact_keyword_positions_messages_and_method_shapes() {
        for (method, kind) in [
            ("def foo; @foo; end", "reader"),
            ("def foo(); @foo; end", "reader"),
            ("def self.foo; @foo; end", "reader"),
            ("def other.foo; @foo; end", "reader"),
            ("def foo = @foo", "reader"),
            ("def café; @café; end", "reader"),
            ("def foo=(val); @foo = val; end", "writer"),
            ("def foo= val; @foo=val; end", "writer"),
            ("def self.foo=(val); @foo=val; end", "writer"),
        ] {
            let source = format!("class Foo\n  {method}\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                format!("Use `attr_{kind}` to define trivial {kind} methods.")
            );
        }
        let found = offenses("class Foo\n  private def foo\n    @foo\n  end\nend\n");
        assert_eq!((found[0].line, found[0].col), (2, 11));
    }
    #[test]
    fn nontrivial_parameters_names_and_allowed_methods() {
        for method in [
            "def foo(val); @foo=val; end",
            "def foo?; @foo; end",
            "def foo; @other; end",
            "def foo=(val); @other=val; end",
            "def initialize(val); @initialize=val; end",
            "def initialize; @initialize; end",
            "def foo(*val); @foo=val; end",
            "def foo(&val); @foo=val; end",
            "def foo=(val); @foo=val+1; end",
            "def foo; log; @foo; end",
            "def foo(val=1); @foo; end",
            "def foo(val:); @foo; end",
            "def foo; return @foo; end",
            "def foo; (@foo); end",
            "def foo=(val); @foo=(@other=val); end",
            "def foo=(val); @foo=other; end",
            "def foo(val, other); @foo=val; end",
        ] {
            assert!(
                offenses(&format!("class Foo\n{method}\nend")).is_empty(),
                "{method}"
            );
        }
        for name in [
            "to_ary",
            "to_a",
            "to_c",
            "to_enum",
            "to_h",
            "to_hash",
            "to_i",
            "to_int",
            "to_io",
            "to_open",
            "to_path",
            "to_proc",
            "to_r",
            "to_regexp",
            "to_str",
            "to_s",
            "to_sym",
        ] {
            assert!(offenses(&format!("class Foo; def {name}; @{name}; end; end")).is_empty());
        }
    }
    #[test]
    fn canonical_pinned_instance_and_self_corrections() {
        for (method, family) in [
            ("def foo\n    @foo\n  end", "reader"),
            ("def foo()\n    @foo\n  end", "reader"),
            ("def foo; @foo; end", "reader"),
            ("def foo=(val)\n    @foo = val\n  end", "writer"),
            ("def foo= val\n    @foo=val\n  end", "writer"),
            ("def foo=(val); @foo=val; end", "writer"),
            ("def foo = @foo", "reader"),
        ] {
            crate::cops::testing::assert_correction(
                &super::TrivialAccessors,
                &format!("class Foo\n  {method}\nend\n"),
                &format!("class Foo\n  attr_{family} :foo\nend\n"),
            );
        }
        for (method, family) in [
            ("def self.foo; @foo; end", "reader"),
            ("def self.foo=(val); @foo=val; end", "writer"),
            ("def self.foo\n    @foo\n  end", "reader"),
            ("def self.foo=(val)\n    @foo = val\n  end", "writer"),
        ] {
            crate::cops::testing::assert_correction(
                &super::TrivialAccessors,
                &format!("class Foo\n  {method}\nend\n"),
                &format!("class Foo\n  class << self\n    attr_{family} :foo\n  end\nend\n"),
            );
        }
    }
    #[test]
    fn visibility_singletons_and_existing_attributes_stay_in_order() {
        crate::cops::testing::assert_correction(
            &super::TrivialAccessors,
            "class Foo\n  private\n  def foo; @foo; end\n  attr_writer :foo\n  public\n  def self.bar; @bar; end\n  class << self\n    protected\n    def baz; @baz; end\n  end\nend\n",
            "class Foo\n  private\n  attr_reader :foo\n  attr_writer :foo\n  public\n  class << self\n    attr_reader :bar\n  end\n  class << self\n    protected\n    attr_reader :baz\n  end\nend\n",
        );
    }
    #[test]
    fn module_eval_class_and_top_level_scopes() {
        for source in [
            "def foo; @foo; end",
            "def foo=(val); @foo=val; end",
            "module M; def foo; @foo; end; end",
            "module M; begin; def foo=(val); @foo=val; end; end; end",
            "obj.instance_eval { def foo; @foo; end }",
            "obj.instance_eval { _1; begin; def foo; @foo; end; end }",
            "class C; obj.instance_eval { def foo; @foo; end }; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "class C; def foo; @foo; end; end",
            "module M; class C; def foo; @foo; end; end; end",
            "module M; class << self; def foo; @foo; end; end; end",
            "obj.instance_eval { class << @blah; begin; def foo; @foo; end; end; end }",
            "obj.class_eval { def foo; @foo; end }",
            "factory { def foo; @foo; end }",
            "def outer; def foo; @foo; end; end",
            "def foo; @foo; end; nil",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
}
