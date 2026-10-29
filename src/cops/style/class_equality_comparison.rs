//! `Style/ClassEqualityComparison` — class equality checks (detection only).
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
pub struct ClassEqualityComparison;
const EQUALITY: &[&[u8]] = &[b"==", b"equal?", b"eql?"];
const NAMES: &[&[u8]] = &[b"name", b"to_s", b"inspect"];
impl Cop for ClassEqualityComparison {
    fn name(&self) -> &'static str {
        "Style/ClassEqualityComparison"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node
            .as_call_node()
            .filter(|c| !c.is_safe_navigation() && EQUALITY.contains(&c.name().as_slice()))
        else {
            return;
        };
        if cx
            .ancestors()
            .find_map(|n| n.as_def_node())
            .is_some_and(|d| EQUALITY.contains(&d.name().as_slice()))
        {
            return;
        }
        let args = ast::call_arguments(&call);
        let [rhs] = args.as_slice() else { return };
        if reference_dstr(rhs) {
            return;
        }
        let Some(receiver) = call.receiver() else {
            return;
        };
        let Some(outer) = plain_send(&receiver) else {
            return;
        };
        let (class, named) =
            if NAMES.contains(&outer.name().as_slice()) && ast::call_arguments(&outer).is_empty() {
                let Some(inner) = outer.receiver() else {
                    return;
                };
                let Some(class) = plain_send(&inner) else {
                    return;
                };
                (class, true)
            } else {
                (outer, false)
            };
        if class.name().as_slice() != b"class" || !ast::call_arguments(&class).is_empty() {
            return;
        }
        let Some(selector) = class.message_loc() else {
            return;
        };
        let argument = class_argument(rhs, named, cx);
        let argument = argument.map(|a| format!("({a})")).unwrap_or_default();
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1)),
            message: format!("Use `instance_of?{argument}` instead of comparing classes."),
            correction: None,
        });
    }
}
// Prism can store a plain multiline literal as one StringNode, while the
// reference splits its physical segments into str children of a dstr. Escaped
// newlines are not physical boundaries; interpolating line continuations glue
// segments together. A sole trailing newline still belongs to one segment.
fn reference_dstr(node: &Node<'_>) -> bool {
    if let Some(string) = node.as_interpolated_string_node() {
        let opening = string
            .opening_loc()
            .map(|loc| loc.as_slice())
            .unwrap_or_default();
        if !opening.starts_with(b"<<") {
            return true;
        }
        // Heredocs with line continuations can have several plain Prism
        // parts that the reference glues into a single str, not a dstr.
        let mut location = None;
        let mut empty = true;
        for part in string.parts().iter() {
            let Some(part) = part.as_string_node() else {
                return true;
            };
            let content = part.content_loc();
            location = Some(location.map_or(content, |start: ruby_prism::Location<'_>| {
                start.join(&part.content_loc()).unwrap()
            }));
            empty &= part.unescaped().is_empty();
        }
        return literal_dstr(
            opening,
            location.map(|loc| loc.as_slice()).unwrap_or_default(),
            empty,
        );
    }
    let Some(string) = node.as_string_node() else {
        return false;
    };
    let opening = string
        .opening_loc()
        .map(|loc| loc.as_slice())
        .unwrap_or_default();
    opening != b"?"
        && literal_dstr(
            opening,
            string.content_loc().as_slice(),
            string.unescaped().is_empty(),
        )
}
fn literal_dstr(opening: &[u8], bytes: &[u8], empty: bool) -> bool {
    if empty && (opening.starts_with(b"<<") || opening.starts_with(b"%") || bytes.contains(&b'\n'))
    {
        return true;
    }
    let literal = opening.ends_with(b"'") || opening.starts_with(b"%q");
    let mut parts = 0;
    let mut content = false;
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        let before_newline = line
            .strip_suffix(b"\n")
            .map(|s| s.strip_suffix(b"\r").unwrap_or(s));
        let continuation = !literal
            && before_newline
                .is_some_and(|s| s.iter().rev().take_while(|b| **b == b'\\').count() % 2 == 1);
        if continuation {
            content |= before_newline.is_some_and(|s| s.len() > 1);
        } else {
            content |= !line.is_empty();
            parts += usize::from(content);
            content = false;
        }
    }
    parts + usize::from(content) > 1
}
fn plain_send<'pr>(node: &Node<'pr>) -> Option<CallNode<'pr>> {
    node.as_call_node().filter(|c| {
        !c.is_safe_navigation() && !c.block().is_some_and(|b| b.as_block_node().is_some())
    })
}
fn source(node: &Node<'_>) -> String {
    String::from_utf8_lossy(node.location().as_slice()).into_owned()
}
fn class_argument(rhs: &Node<'_>, named: bool, cx: &NodeContext<'_, '_>) -> Option<String> {
    let string = rhs.as_string_node().is_some() || rhs.as_interpolated_string_node().is_some();
    if !named {
        return (!string).then(|| source(rhs));
    }
    if let Some(call) = rhs.as_call_node()
        && NAMES.contains(&call.name().as_slice())
        && let Some(receiver) = call.receiver()
    {
        return Some(source(&receiver));
    }
    if string {
        // Match the reference's source spelling, not an executable correction.
        let mut name = source(rhs).replace(['\'', '"'], "");
        if !name.starts_with("::")
            && cx
                .ancestors()
                .any(|n| matches!(n, Node::ClassNode { .. } | Node::ModuleNode { .. }))
        {
            name.insert_str(0, "::");
        }
        return Some(name);
    }
    if ast::is_variable(rhs) || rhs.as_call_node().is_some() {
        return None;
    }
    Some(source(rhs))
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/class_equality_comparison_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};
    fn check(source: &str, line: u32, col: u32, argument: &str) {
        let found: Vec<_> = lint_str(source)
            .into_iter()
            .filter(|o| o.cop == ClassEqualityComparison.name())
            .collect();
        assert_eq!(found.len(), 1, "{source}");
        assert_eq!((found[0].line, found[0].col), (line, col), "{source}");
        assert_eq!(
            found[0].message,
            format!("Use `instance_of?{argument}` instead of comparing classes."),
            "{source}"
        );
        assert!(!ClassEqualityComparison.safe_fixes());
        assert!(
            correction_offenses(&ClassEqualityComparison, source)
                .iter()
                .all(|o| o.correction.is_none())
        );
    }
    #[test]
    fn pinned_comparison_selectors() {
        for source in [
            "var.class == Date",
            "var.class.equal?(Date)",
            "var.class.eql?(Date)",
        ] {
            check(source, 1, 5, "(Date)");
        }
    }
    #[test]
    fn pinned_name_selectors_strings_constants_and_unknowns() {
        for name in ["name", "to_s", "inspect"] {
            for value in [
                "'Date'",
                "\"Date\"",
                "Date",
                "Date.name",
                "Date.to_s",
                "Date.inspect",
            ] {
                check(&format!("var.class.{name} == {value}"), 1, 5, "(Date)");
            }
            for value in [
                "@class_name",
                "@@class_name",
                "$class_name",
                "class_name",
                "obj&.class_name",
            ] {
                check(&format!("var.class.{name} == {value}"), 1, 5, "");
            }
            check(
                &format!("class_name = 'Model'\nvar.class.{name} == class_name"),
                2,
                5,
                "",
            );
        }
        check("var.class == 'Date'", 1, 5, "");
        check(
            "module Foo\n  def bar?(value)\n    bar.class.name == 'Bar'\n  end\n  class Bar; end\nend",
            3,
            9,
            "(::Bar)",
        );
        check(
            "module Foo\n  def bar?(value)\n    bar.class.name == '::Bar'\n  end\nend",
            3,
            9,
            "(::Bar)",
        );
        check(
            "module Foo\n  def bar?(value)\n    bar.class.name == Model\n  end\nend",
            3,
            9,
            "(Model)",
        );
    }
    #[test]
    fn fixed_allowed_methods_use_the_nearest_def_including_singletons() {
        for name in ["==", "equal?", "eql?"] {
            for receiver in ["", "self.", "object."] {
                let source = format!(
                    "def {receiver}{name}(other)\n self.class == other.class && name == other.name\nend"
                );
                assert!(
                    correction_offenses(&ClassEqualityComparison, &source).is_empty(),
                    "{source}"
                );
            }
        }
        check(
            "def eq(other)\n self.class == other.class\nend",
            2,
            7,
            "(other.class)",
        );
        check(
            "def ==(other)\n def eq; self.class == other.class; end\nend",
            2,
            15,
            "(other.class)",
        );
        check(
            "define_method(:==) { self.class == other.class }",
            1,
            27,
            "(other.class)",
        );
    }
    #[test]
    fn no_offense_lookalikes_and_interpolation() {
        for source in [
            "var.instance_of?(Date)",
            "Date == var.class",
            "var.class != Date",
            "var.klass == Date",
            "var.class(1) == Date",
            "var.class.name(1) == 'Date'",
            "var&.class == Date",
            "var.class&.name == 'Date'",
            "var.class&.equal?(Date)",
            "var.class == \"String#{interpolation}\"",
            "var.class.name == \"String#{interpolation}\"",
            "var.class.equal?(Date, Other)",
            "var.class.equal?(Date, &block)",
            "var.class { x } == Date",
            "'var.class == Date' # var.class == Date\n",
        ] {
            assert!(
                correction_offenses(&ClassEqualityComparison, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn reference_dstr_literals_are_exempt_even_without_interpolation() {
        for literal in [
            "\"Da\nte\"",
            "'Da\nte'",
            "%q{Da\nte}",
            "%Q{Da\r\nte}",
            "'Da\\\nte'",
            "\"Da\\\nte\nOther\"",
            "<<~RUBY\nDa\nte\nRUBY\n",
            "<<-'RUBY'\nDa\nte\nRUBY\n",
            "<<~RUBY\nRUBY\n",
            "%q{}",
            "\"\\\n\"",
        ] {
            for comparison in ["var.class ==", "var.class.name =="] {
                let source = format!("{comparison} {literal}");
                assert!(
                    correction_offenses(&ClassEqualityComparison, &source).is_empty(),
                    "{source}"
                );
            }
        }
        // A single physical segment, an escaped newline, and line continuations
        // still produce reference str nodes, not dstr nodes.
        for literal in [
            "\"Da\\nte\"",
            "\"Date\n\"",
            "\"Da\\\nte\"",
            "\"Da\\\r\nte\"",
            "<<~RUBY\nDate\nRUBY\n",
            "<<~RUBY\nDa\\\nte\nRUBY\n",
            "\"\"",
        ] {
            let source = format!("var.class == {literal}");
            check(&source, 1, 5, "");
        }
    }
    #[test]
    fn shadowed_constants_and_selectors_remain_detection_only() {
        for prefix in [
            "Date = Custom",
            "def object.class; Custom; end",
            "class Object; def instance_of?(x); false; end; end",
        ] {
            check(&format!("{prefix}\nobject.class == Date"), 2, 8, "(Date)");
        }
        let found = correction_offenses(
            &ClassEqualityComparison,
            "obj.class.==(\n Date\n) { break :x }",
        );
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 5, 3));
    }
}
