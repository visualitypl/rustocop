//! `Style/TrivialAccessors` — prefer the attribute family for exact-name accessors.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{DefNode, Node};
use std::collections::HashSet;
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
        let correction = if cx.corrections_requested()
            && cx.ancestors().last().is_some_and(|root| {
                file.trivial_accessor_bindings(root)
                    .proven
                    .contains(&node.location().start_offset())
            }) {
            replacement(node, &def, kind, file)
        } else {
            None
        };
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

/// A separate proof: accepting defs here must never broaden the older attr
/// regrouping proof (method metadata/definition order have different hazards).
/// Only discarded, fresh root classes with no source observers, inheritance,
/// callbacks, reopening, calls or consumed definition values qualify. External
/// preloads/monkey patches are outside this source-only proof boundary.
#[derive(Default)]
pub(crate) struct Bindings {
    proven: HashSet<usize>,
    #[cfg(test)]
    pub(crate) visited: usize,
}
impl Bindings {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut result = Self::default();
        let Some(program) = root.as_program_node() else {
            return result;
        };
        let mut classes = HashSet::new();
        for node in program.statements().body().iter() {
            #[cfg(test)]
            {
                result.visited += 1;
            }
            let Some(class) = node.as_class_node() else {
                return Self::default();
            };
            let Some(name) = class.constant_path().as_constant_read_node() else {
                return Self::default();
            };
            if class.superclass().is_some()
                || super::mixin_grouping::core_namespace(name.name().as_slice())
                || !classes.insert(name.name().as_slice().to_vec())
                || !result.body(class.body(), 0, &mut Vec::new())
            {
                return Self::default();
            }
        }
        result
    }
    fn body(
        &mut self,
        body: Option<Node<'_>>,
        depth: usize,
        defined: &mut Vec<HashSet<Vec<u8>>>,
    ) -> bool {
        while defined.len() <= depth + 1 {
            defined.push(HashSet::new());
        }
        let Some(body) = body else {
            return true;
        };
        let Some(statements) = body.as_statements_node() else {
            return false;
        };
        for node in statements.body().iter() {
            #[cfg(test)]
            {
                self.visited += 1;
            }
            if let Some(class) = node.as_singleton_class_node() {
                if class.expression().as_self_node().is_none()
                    || !self.body(class.body(), depth + 1, defined)
                {
                    return false;
                }
            } else if let Some(def) = node.as_def_node() {
                if kind(&def).is_none()
                    || def.receiver().is_some_and(|r| r.as_self_node().is_none())
                {
                    return false;
                }
                let name = def.name().as_slice();
                let base = name.strip_suffix(b"=").unwrap_or(name);
                let level = depth + usize::from(def.receiver().is_some());
                if !valid_name(base) || !defined[level].insert(name.to_vec()) {
                    return false;
                }
                self.proven.insert(node.location().start_offset());
            } else if let Some(call) = node.as_call_node() {
                if super::accessors::visibility(&call).is_some() {
                    continue;
                }
                if !super::accessors::family(&call) || call.block().is_some() {
                    return false;
                }
                for arg in crate::cops::ast::call_arguments(&call) {
                    let Some(symbol) = arg.as_symbol_node() else {
                        return false;
                    };
                    let name = symbol.unescaped();
                    if !valid_name(name) {
                        return false;
                    }
                    if call.name().as_slice() != b"attr_writer"
                        && !defined[depth].insert(name.to_vec())
                    {
                        return false;
                    }
                    if matches!(call.name().as_slice(), b"attr_writer" | b"attr_accessor") {
                        let mut setter = name.to_vec();
                        setter.push(b'=');
                        if !defined[depth].insert(setter) {
                            return false;
                        }
                    }
                }
            } else {
                return false;
            }
        }
        true
    }
}
fn valid_name(name: &[u8]) -> bool {
    !name.is_empty()
        && (name[0].is_ascii_alphabetic() || name[0] == b'_')
        && name.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_')
        && !matches!(
            name,
            b"method_added"
                | b"singleton_method_added"
                | b"attr"
                | b"attr_reader"
                | b"attr_writer"
                | b"attr_accessor"
                | b"private"
                | b"protected"
                | b"public"
        )
}
fn replacement(
    node: &Node<'_>,
    def: &DefNode<'_>,
    kind: &str,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    let name = def.name().as_slice();
    // Detection follows upstream's normalization, but attr_reader would replace
    // a zero-argument setter with a differently named method.
    if kind == "reader" && name.ends_with(b"=") {
        return None;
    }
    let name = std::str::from_utf8(name.strip_suffix(b"=").unwrap_or(name)).ok()?;
    let attr = format!("attr_{kind} :{name}");
    let text = if def.receiver().is_some() {
        let start = node.location().start_offset();
        let line = file.line_of(start);
        let indent = &file.bytes[file.line_start(line)..start];
        // Reject inline siblings before decoding/copying a potentially enormous
        // shared physical-line prefix for each singleton def.
        if !indent.iter().all(|b| matches!(b, b' ' | b'\t')) {
            return None;
        }
        let indent = std::str::from_utf8(indent).ok()?;
        let newline =
            if file.bytes[file.line_start(line)..file.line_start(line + 1)].ends_with(b"\r\n") {
                "\r\n"
            } else {
                "\n"
            };
        format!("class << self{newline}{indent}  {attr}{newline}{indent}end")
    } else {
        attr
    };
    super::accessors::rewrite(file, &[node], &text)
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
    fn setter_shaped_readers_report_but_never_change_method_identity() {
        for method in [
            "def foo=; @foo; end",
            "def foo=(); @foo; end",
            "def self.foo=; @foo; end",
            "def self.foo=(); @foo; end",
        ] {
            let source = format!("class Foo\n  {method}\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                "Use `attr_reader` to define trivial reader methods."
            );
            let found =
                crate::cops::testing::correction_offenses(&super::TrivialAccessors, &source);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none(), "{source}");
        }
        for method in ["def foo=; @other; end", "def foo=(val); @foo; end"] {
            assert!(offenses(&format!("class Foo\n  {method}\nend\n")).is_empty());
        }
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
    fn many_inline_singleton_definitions_never_copy_their_shared_line_prefix() {
        let source = format!(
            "class Foo; {}; end",
            (0..2000)
                .map(|i| format!("def self.a{i}; @a{i}; end"))
                .collect::<Vec<_>>()
                .join("; ")
        );
        let found = crate::cops::testing::correction_offenses(&super::TrivialAccessors, &source);
        assert_eq!(found.len(), 2000);
        assert!(found.iter().all(|o| o.correction.is_none()));
    }
    #[test]
    fn uncertain_pinned_eval_and_visibility_modifier_corrections_withheld() {
        for source in [
            "class Foo\n  private def foo\n    @foo\n  end\nend\n",
            "something.instance_eval do\n  class << @blah\n    begin\n      def bar\n        @bar\n      end\n    end\n  end\nend\n",
        ] {
            let found = crate::cops::testing::correction_offenses(&super::TrivialAccessors, source);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none());
        }
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
