//! Test-only helpers: deliberately broken cops for the panic-isolation
//! tests, and an independent oracle for tree walks.

use ruby_prism::Node;

use super::{Cop, NodeContext};
use crate::{
    offense::{Offense, Severity},
    source::SourceFile,
};

/// Collect corrections explicitly, without writing a file. Ordinary `lint_str`
/// intentionally does not request expensive correction generation.
pub(crate) fn correction_offenses(cop: &dyn Cop, source: &str) -> Vec<Offense> {
    crate::pipeline::lint_str_with_corrections(&[cop], source)
}

/// The source after applying a cop's corrections until none apply, as the
/// spec's `expect_correction` loops (at most 20 passes).
pub(crate) fn fix_until_stable(cop: &dyn Cop, source: &str) -> String {
    let mut source = source.to_string();
    for _ in 0..20 {
        let found = correction_offenses(cop, &source);
        let applied = crate::corrections::apply(source.as_bytes(), &found).unwrap();
        if applied.count == 0 {
            break;
        }
        source = String::from_utf8(applied.bytes).unwrap();
    }
    source
}

/// Assert a cop's single-pass correction, validating both sides with the same
/// fixed Ruby 3.4 grammar and bounded parser stack/cleanup as production.
pub(crate) fn assert_correction(cop: &dyn Cop, before: &str, after: &str) {
    assert!(
        cop.safe_fixes() || cop.unsafe_fixes(),
        "{} must opt in to fixes",
        cop.name()
    );
    assert_eq!(
        crate::parser::parse(before.as_bytes()).errors().count(),
        0,
        "{before}"
    );
    let offenses = correction_offenses(cop, before);
    assert!(!offenses.is_empty(), "{}: {before}", cop.name());
    let applied = crate::corrections::apply(before.as_bytes(), &offenses).unwrap();
    assert_eq!(applied.bytes, after.as_bytes(), "{}: {before}", cop.name());
    assert_eq!(
        crate::parser::parse(&applied.bytes).errors().count(),
        0,
        "{after}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offense::Correction;

    #[test]
    fn correction_validation_accepts_ruby34_english_not_on_both_sides() {
        assert_correction(
            &crate::cops::style::empty_case_condition::EmptyCaseCondition,
            "case\nwhen [not false]\n :yes\nend",
            "if ([not false])\n :yes\nend",
        );
    }

    #[test]
    fn correction_validation_rejects_latest_only_output() {
        // Deliberately broken cop: latest Prism accepts its Ruby 4 output, but
        // the production parser must reject it even in correction unit tests.
        struct Ruby4Correction;
        impl Cop for Ruby4Correction {
            fn name(&self) -> &'static str {
                "Test/Ruby4Correction"
            }
            fn safe_fixes(&self) -> bool {
                true
            }
            fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    line: 1,
                    end_line: 1,
                    col: 1,
                    message: "invalid correction".into(),
                    correction: Some(Correction {
                        range: 0..file.bytes.len(),
                        replacement: "true\n && false\n".into(),
                    }),
                });
            }
        }
        let after = "true\n && false\n";
        assert_eq!(ruby_prism::parse(after.as_bytes()).errors().count(), 0);
        // Prove the panic isn't from a failure to collect/apply the test cop.
        let offenses = correction_offenses(&Ruby4Correction, "true\n");
        let applied = crate::corrections::apply(b"true\n", &offenses).unwrap();
        assert_eq!(applied.bytes, after.as_bytes());
        let result = std::panic::catch_unwind(|| {
            assert_correction(&Ruby4Correction, "true\n", after);
        });
        assert!(result.is_err(), "Ruby 4 output must fail syntax validation");
    }
}

/// `Lint/PanicOnCall`: reports every call to `mark`, and panics on a call to
/// `boom` without arguments: the indexing bug of a cop that assumes one.
pub(crate) struct PanicOnCall;

impl Cop for PanicOnCall {
    fn name(&self) -> &'static str {
        "Lint/PanicOnCall"
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
        match call.name().as_slice() {
            b"mark" => mark(self.name(), node, file, out),
            b"boom" => {
                let arguments: Vec<Node<'pr>> = call
                    .arguments()
                    .map(|arguments| arguments.arguments().iter().collect())
                    .unwrap_or_default();
                let _first = &arguments[0];
            }
            _ => {}
        }
    }
}

/// `Lint/PanicOnSource`: panics in `check_source` on any file mentioning
/// `boom`, and otherwise reports every call to `mark` from `check_node`.
pub(crate) struct PanicOnSource;

impl Cop for PanicOnSource {
    fn name(&self) -> &'static str {
        "Lint/PanicOnSource"
    }

    fn check_source(&self, file: &SourceFile, _out: &mut Vec<Offense>) {
        if file.bytes.windows(4).any(|window| window == b"boom") {
            panic!("PanicOnSource saw `boom`");
        }
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node
            .as_call_node()
            .is_some_and(|call| call.name().as_slice() == b"mark")
        {
            mark(self.name(), node, file, out);
        }
    }
}

/// `Lint/PanicOnLeave`: takes `leave_node` calls and reports every call to
/// `mark` from there; panics in `check_node` on a call to `boom` without
/// arguments (as [`PanicOnCall`] does), and in `leave_node` on a call to
/// `bang`.
pub(crate) struct PanicOnLeave;

impl Cop for PanicOnLeave {
    fn name(&self) -> &'static str {
        "Lint/PanicOnLeave"
    }

    fn wants_leave_node(&self) -> bool {
        true
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        _file: &SourceFile,
        _out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.name().as_slice() == b"boom" {
            let arguments: Vec<Node<'pr>> = call
                .arguments()
                .map(|arguments| arguments.arguments().iter().collect())
                .unwrap_or_default();
            let _first = &arguments[0];
        }
    }

    fn leave_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        match call.name().as_slice() {
            b"mark" => mark(self.name(), node, file, out),
            b"bang" => panic!("PanicOnLeave left `bang`"),
            _ => {}
        }
    }
}

/// Report `Marked.` at the node.
fn mark(cop: &'static str, node: &Node<'_>, file: &SourceFile, out: &mut Vec<Offense>) {
    let (line, col) = file.line_col(node.location().start_offset());
    out.push(Offense {
        correction: None,
        cop,
        severity: Severity::Error,
        message: "Marked.".to_string(),
        line,
        end_line: file.last_line_of(node.location()),
        col,
    });
}

/// One step of a depth-first walk: a node of this kind (`CallNode`) is
/// entered or left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    Enter(String),
    Leave(String),
}

/// An independent oracle for tree walks: ruby-prism's `Debug` output dumps
/// the whole tree recursively, children nested in their parent's
/// `FooNode(..)` in field order. Reading the nesting of those parentheses
/// (outside the quoted source slices) gives every node's enter and leave, in
/// depth-first order. Walks the tree Prism recovers from syntax errors too.
pub(crate) fn tree_steps(source: &str) -> Vec<Step> {
    let result = ruby_prism::parse(source.as_bytes());
    let dump = format!("{:?}", result.node());
    let mut steps = Vec::new();
    // One entry per open parenthesis: the node kind it opens, if any.
    let mut open: Vec<Option<String>> = Vec::new();
    let mut word = String::new();
    let mut chars = dump.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                // A source slice, with backslash escapes: skip it.
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            chars.next();
                        }
                        '"' => break,
                        _ => {}
                    }
                }
                word.clear();
            }
            '(' => {
                let kind = word.ends_with("Node").then(|| word.clone());
                if let Some(kind) = &kind {
                    steps.push(Step::Enter(kind.clone()));
                }
                open.push(kind);
                word.clear();
            }
            ')' => {
                if let Some(Some(kind)) = open.pop() {
                    steps.push(Step::Leave(kind));
                }
                word.clear();
            }
            c if c.is_ascii_alphanumeric() || c == '_' => word.push(c),
            _ => word.clear(),
        }
    }
    assert!(open.is_empty(), "unbalanced dump for {source:?}");
    steps
}

/// Every node kind in the tree, in pre-order (see [`tree_steps`]).
pub(crate) fn preorder_kinds(source: &str) -> Vec<String> {
    tree_steps(source)
        .into_iter()
        .filter_map(|step| match step {
            Step::Enter(kind) => Some(kind),
            Step::Leave(_) => None,
        })
        .collect()
}

/// One snippet per place ruby-prism visits a typed child directly (through
/// its `visit_*_node` method, without the enter hook), with the kind it
/// reaches that way.
pub(crate) const TYPED_CHILD_CASES: &[(&str, &str)] = &[
    ("a", "StatementsNode"),
    ("begin; a; end", "StatementsNode"),
    ("begin; rescue; a; end", "RescueNode"),
    ("begin; rescue A; rescue B; end", "RescueNode"),
    ("begin; rescue; else; a; end", "ElseNode"),
    ("begin; ensure; a; end", "EnsureNode"),
    ("foo { |a, b| }", "ParametersNode"),
    ("loop { break 1 }", "ArgumentsNode"),
    ("foo(1)", "ArgumentsNode"),
    ("foo(&b)", "BlockArgumentNode"),
    ("case x; in Integer => n; end", "LocalVariableTargetNode"),
    ("case x; in 1; else; 2; end", "ElseNode"),
    ("case x; when 1; else; 2; end", "ElseNode"),
    ("A::B &&= 1", "ConstantPathNode"),
    ("A::B += 1", "ConstantPathNode"),
    ("A::B ||= 1", "ConstantPathNode"),
    ("A::B = 1", "ConstantPathNode"),
    ("def m(a); end", "ParametersNode"),
    ("def m(&b); end", "BlockParameterNode"),
    ("\"#{a}\"", "StatementsNode"),
    ("case x; in [*, 1, *]; end", "SplatNode"),
    ("for i in a; b; end", "StatementsNode"),
    ("def m; super { }; end", "BlockNode"),
    ("if a; b; end", "StatementsNode"),
    ("case x; in 1; a; end", "StatementsNode"),
    ("a[1] &&= 2", "ArgumentsNode"),
    ("a[1] += 2", "ArgumentsNode"),
    ("a[1] ||= 2", "ArgumentsNode"),
    ("a[1], b = 1, 2", "ArgumentsNode"),
    ("/(?<n>x)/ =~ y", "CallNode"),
    ("loop { next 1 }", "ArgumentsNode"),
    ("END { a }", "StatementsNode"),
    ("BEGIN { a }", "StatementsNode"),
    ("def m; return 1; end", "ArgumentsNode"),
    ("def m; super(1); end", "ArgumentsNode"),
    ("unless a; b; end", "StatementsNode"),
    ("unless a; b; else; c; end", "ElseNode"),
    ("until a; b; end", "StatementsNode"),
    ("case x; when 1; a; end", "StatementsNode"),
    ("while a; b; end", "StatementsNode"),
    ("def m; yield 1; end", "ArgumentsNode"),
];

/// Typed children reached only in trees Prism recovers from syntax errors:
/// block arguments in index assignments (`BlockArgumentNode`).
pub(crate) const ERROR_TREE_TYPED_CHILDREN: &[&str] = &[
    "a[&b] ||= 1",
    "a[&b] &&= 1",
    "a[&b] += 1",
    "a[&b], c = 1, 2",
];

/// A file exercising most node kinds, among them every kind ruby-prism
/// visits as a typed child in a valid tree.
pub(crate) const FULL_TREE: &str = r#"
BEGIN { setup }
END { teardown }
module M
  class C < Base
    A::B = 1
    A::B ||= 2
    A::B &&= 3
    A::B += 4
    class A::B; end
    Top = ::Top + A::B

    def m(a, b = 1, *c, d:, e: 2, **f, &g)
      super
      super { |x| x }
      super(a, &g)
      yield a, b
      return a if b
      foo(a, *c, d: 1, **f, &g).bar { |y, (z, w)| y + z }
      x = [1, 2.0, 3r, 4i, :s, "s#{a}", 'q', %w[p q], { k: 1, "l" => 2 }]
      x[1] ||= 2
      x[1] &&= 3
      x[1] += 4
      x[1], y = 1, 2
      /(?<cap>x)/ =~ y
      ->(l) { l * 2 }
      for i in 1..3 do next i; end
      while a; break b; end
      until a; redo; end
      unless a then b else c end
      if a then b elsif c then d else e end
      case a
      when 1, 2 then b
      else c
      end
      case a
      in [*, 1, *] then b
      in Integer => n if n > 0 then c
      in { k: } then k
      else d
      end
      begin
        a
      rescue A, B => err
        retry
      rescue C
        b
      else
        c
      ensure
        d
      end
      @ivar ||= @@cvar
      $gvar &&= defined?(a)
      a&.b ? !a : a.c
    end
  end
end
"#;
