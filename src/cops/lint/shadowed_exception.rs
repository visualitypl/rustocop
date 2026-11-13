//! `Lint/ShadowedException` — ordered rescue groups, not source type inference.
use std::{cmp::Ordering, collections::HashMap, sync::OnceLock};

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct ShadowedException;
impl Cop for ShadowedException {
    fn name(&self) -> &'static str {
        "Lint/ShadowedException"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_begin_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Begin owns exactly one rescue chain (also in implicit method/block
        // bodies). Modifier rescue is a distinct Prism node. No suffix rescans.
        let Some(begin) = node.as_begin_node() else {
            return;
        };
        let mut next = begin.rescue_clause();
        let mut rescues = Vec::new();
        let mut groups = Vec::new();
        let facts = facts();
        while let Some(rescue) = next {
            let mut group: Vec<_> = rescue
                .exceptions()
                .iter()
                .map(|e| {
                    let source = file.match_pattern_source(e.location().as_slice());
                    facts.resolve(&source)
                })
                .collect();
            if group.is_empty() {
                group.push(Some("StandardError"));
            }
            groups.push(group);
            next = rescue.subsequent();
            rescues.push(rescue);
        }
        // Upstream prioritizes ANY same-group problem over earlier ordering
        // problems; it does not flag every shadowed handler independently.
        let index = groups
            .iter()
            .position(|g| facts.multiple_levels(g))
            .or_else(|| {
                groups
                    .windows(2)
                    .position(|pair| !facts.sorted(&pair[0], &pair[1]))
            });
        let Some(index) = index else {
            return;
        };
        let node = rescues[index].as_node();
        let start = node.location().start_offset();
        let (line, col) = super::array_literal_in_regexp::reference_line_col(start, file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(ast::source_end(&node).saturating_sub(1)),
            message: "Do not shadow rescued Exceptions.".into(),
            correction: None,
        });
    }
}

/// Cop-private frozen metadata, not a resolver for the inspected application.
/// Aliases, nonexception modules/classes and loaded stdlib/reference constants
/// matter because upstream calls Kernel.const_get on the exact source string.
/// No production Ruby, autoload, require, const_missing or source class inference.
struct Facts {
    constants: HashMap<(&'static str, &'static str), &'static str>,
    ancestors: HashMap<&'static str, Vec<&'static str>>,
    errno: HashMap<&'static str, &'static str>,
}
fn facts() -> &'static Facts {
    static FACTS: OnceLock<Facts> = OnceLock::new();
    FACTS.get_or_init(|| {
        let mut facts = Facts {
            constants: HashMap::new(),
            ancestors: HashMap::new(),
            errno: HashMap::new(),
        };
        for line in include_str!("shadowed_exception_constants.tsv")
            .lines()
            .filter(|s| !s.starts_with('#'))
        {
            let parts: Vec<_> = line.split('\t').collect();
            match parts[0] {
                "C" => {
                    facts.constants.insert((parts[1], parts[2]), parts[3]);
                }
                "A" => {
                    facts.ancestors.insert(parts[1], parts[2..].to_vec());
                }
                "E" => {
                    facts.errno.insert(parts[1], parts[2]);
                }
                _ => unreachable!("generated constant metadata"),
            }
        }
        facts
    })
}
impl Facts {
    fn resolve(&self, source: &[u8]) -> Option<&'static str> {
        let source = std::str::from_utf8(source).ok()?;
        let mut parts = source.strip_prefix("::").unwrap_or(source).split("::");
        let mut value = *self.constants.get(&("Object", parts.next()?))?;
        for part in parts {
            // Nested const_get searches ancestors but explicitly excludes
            // Object's top-level constants (unless Object is the receiver).
            value = self
                .ancestors
                .get(value)?
                .iter()
                .filter(|a| **a != "Object" || value == "Object")
                .find_map(|a| self.constants.get(&(*a, part)).copied())?;
        }
        Some(value)
    }
    fn compare(&self, a: Option<&str>, b: Option<&str>) -> Option<Ordering> {
        let (a, b) = (a?, b?);
        if a == b {
            Some(Ordering::Equal)
        } else if self.ancestors.get(a)?.contains(&b) {
            Some(Ordering::Less)
        } else if self.ancestors.get(b)?.contains(&a) {
            Some(Ordering::Greater)
        } else {
            None
        }
    }
    fn multiple_levels(&self, group: &[Option<&str>]) -> bool {
        if group.len() > 1 && group.contains(&Some("Exception")) {
            return true;
        }
        group.iter().enumerate().any(|(i, a)| {
            group[i + 1..].iter().any(|b| {
                // Ruby's Module#<=> returning ZERO is truthy. Only the same-code
                // SystemCallError subclass special case suppresses duplicates.
                if let (Some(a), Some(b)) = (a, b)
                    && let (Some(a), Some(b)) = (self.errno.get(a), self.errno.get(b))
                    && a == b
                {
                    return false;
                }
                self.compare(*a, *b).is_some()
            })
        })
    }
    fn sorted(&self, a: &[Option<&str>], b: &[Option<&str>]) -> bool {
        if a.contains(&Some("Exception")) {
            return false;
        }
        if b.contains(&Some("Exception"))
            || a.iter().all(Option::is_none)
            || b.iter().all(Option::is_none)
        {
            return true;
        }
        // Ruby Array#<=> is lexicographic, first testing element == (nil==nil
        // too), then <=>. A nil comparison makes the ENTIRE comparison nil,
        // which upstream replaces with zero, before checking group lengths.
        for (a, b) in a.iter().zip(b) {
            if a == b {
                continue;
            }
            return self.compare(*a, *b).is_none_or(|c| c != Ordering::Greater);
        }
        a.len() <= b.len()
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/shadowed_exception_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/ShadowedException")
            .collect()
    }
    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }
    #[test]
    fn upstream_single_group_and_unknown_controls() {
        for types in [
            "",
            "Exception",
            "TimeoutError",
            "NonStandardException",
            "Error, NonStandardException",
            "CustomError, NonStandardException",
            "Errno::EAGAIN, Errno::EWOULDBLOCK, Errno::ECONNABORTED",
            "Exception => e",
            "NoMethodError, ZeroDivisionError",
            "*FOO",
        ] {
            assert!(
                offenses(&format!(
                    "begin\n something\nrescue {types}\n handle_exception\nend\n"
                ))
                .is_empty(),
                "{types}"
            );
        }
        assert!(offenses("foo rescue nil").is_empty());
        for types in [
            "StandardError, RuntimeError",
            "NonStandardError, Exception",
            "StandardError, NameError",
            "StandardError, NameError, NoMethodError",
            "NameError, NameError",
        ] {
            assert_eq!(
                positions(&format!("begin\n something\nrescue {types}\n foo\nend\n")),
                [(3, 1)],
                "{types}"
            );
        }
    }
    #[test]
    fn adjacent_group_order_and_same_group_priority() {
        for (groups, index) in [
            (vec!["Exception", ""], Some(0)),
            (vec!["NoMethodError", "Exception", "StandardError"], Some(1)),
            (
                vec!["Exception", "NoMethodError, ZeroDivisionError"],
                Some(0),
            ),
            (
                vec!["ZeroDivisionError", "NoMethodError, StandardError"],
                Some(1),
            ),
            (
                vec!["Exception", "StandardError", "NameError, NameError"],
                Some(2),
            ),
            (vec!["StandardError", "Exception"], None),
            (vec!["StandardError", "A", "B", "C", "D", "E", "F"], None),
            (vec!["NonStandardError, NoMethodError", "Exception"], None),
            (vec!["NonStandardError, OtherError", "CustomError"], None),
            (vec!["*FOO", "*BAR"], None),
            (vec!["StandardError", "*BAR"], None),
            (vec!["Exception", "*BAR"], Some(0)),
            (vec!["ArgumentError", "Interrupt"], None),
            (vec!["Interrupt", "ArgumentError"], None),
            (
                vec!["UnknownException", "StandardError", "AnotherUnknown"],
                None,
            ),
            (vec!["StandardError", "Unknown"], None),
            (vec!["Exception", "Unknown"], Some(0)),
            (vec!["foo", "[bar]"], None),
            (vec!["A, B", "C", ""], None),
            // Upstream is NOT a general all-prior hierarchy check.
            (vec!["StandardError", "Unknown", "RuntimeError"], None),
            (vec!["NameError", "NameError"], None),
            (
                vec!["Unknown, NameError", "Unknown, NoMethodError"],
                Some(0),
            ),
            (vec!["NameError, Unknown", "NameError"], Some(0)),
            (
                vec!["Unknown, NameError", "NameError, NoMethodError"],
                Some(1),
            ),
        ] {
            let mut source = "def f\n work\n".to_string();
            for g in groups {
                source.push_str(&format!("rescue {g}\n handle\n"));
            }
            source.push_str("else\n okay\nensure\n cleanup\nend\n");
            let expected: Vec<_> = index.into_iter().map(|i| (3 + 2 * i, 1)).collect();
            assert_eq!(positions(&source), expected, "{source}");
        }
    }
    #[test]
    fn const_get_uses_exact_source_and_known_aliases_not_user_types() {
        for (types, n) in [
            ("::StandardError, ::RuntimeError", 1),
            ("Object::Exception, Unknown", 1),
            ("File, IO", 1),
            ("String, Object", 1),
            ("Enumerable, Array", 1),
            ("YAML::Exception, Psych::SyntaxError", 1),
            ("RuboCop::Cop::Base, RuboCop::Cop::Lint::Debugger", 1),
            ("Regexp::TimeoutError, RegexpError", 1),
            ("Errno::EAGAIN, Errno::EWOULDBLOCK", 0),
            ("Errno::EAGAIN, Errno::EAGAIN", 0),
            ("Errno::EAGAIN, SystemCallError", 1),
            ("Thread::Queue, Queue", 1),
            ("Numeric, Integer", 1),
            ("Other::Exception, RuntimeError", 0),
            ("Kernel::Exception, RuntimeError", 0),
            ("Object ::Exception, Unknown", 0),
            ("Object::\n Exception, Unknown", 0),
            ("(Exception), Unknown", 0),
            ("Exception, *errors", 1),
            ("E, RuntimeError", 0),
            ("Unknown, Unknown", 0),
            ("RSpec, RSpec", 0),
            ("Oracle, Oracle", 0),
            ("FixedPolicy, FixedPolicy", 0),
        ] {
            let source =
                format!("class E < Exception; end\nbegin; work; rescue {types}; handle; end\n");
            assert_eq!(offenses(&source).len(), n, "{source}");
        }
    }
    #[test]
    fn fixed_ruby34_additions_and_removals_not_reference_host_version() {
        for (types, n) in [
            ("ObjectSpace::WeakKeyMap, Object", 1),
            ("RubyVM::AbstractSyntaxTree::Location, Object", 1),
            ("RubyVM::RJIT, RubyVM::RJIT", 1),
            ("RubyVM::MJIT, RubyVM::MJIT", 0),
        ] {
            assert_eq!(
                offenses(&format!("begin; work; rescue {types}; handle; end\n")).len(),
                n,
                "{types}"
            );
        }
    }
    #[test]
    fn nested_typed_handlers_each_receive_one_report() {
        let source = "begin\n work\nrescue Exception\n begin\n  nested\n rescue StandardError, RuntimeError\n  handle\n end\nrescue StandardError\n other\nend\n";
        assert_eq!(positions(source), [(3, 1), (6, 2)]);
        let found = offenses(source);
        assert_eq!(found[0].end_line, 8);
        assert_eq!(found[1].end_line, 7);
    }
    #[test]
    fn same_group_reports_the_full_first_handler() {
        let found = offenses(
            "def foo\n  something\nrescue StandardError, RuntimeError\n  handle_exception\nend\n",
        );
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 1, 4));
        assert_eq!(found[0].message, "Do not shadow rescued Exceptions.");
        assert!(found[0].correction.is_none());
    }
}
