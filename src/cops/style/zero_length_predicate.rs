//! `Style/ZeroLengthPredicate` — prefer empty predicates.
use ruby_prism::{CallNode, Node};

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct ZeroLengthPredicate;

impl Cop for ZeroLengthPredicate {
    fn name(&self) -> &'static str {
        "Style/ZeroLengthPredicate"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        let Some(receiver) = call.receiver() else {
            return;
        };
        let args = ast::call_arguments(&call);
        let op = call.name();
        let op = op.as_slice();
        let (length, nonzero, reversed) = if op == b"zero?" && args.is_empty() {
            let Some(length) = length_call(&receiver) else {
                return;
            };
            (length, false, false)
        } else {
            let [argument] = args.as_slice() else { return };
            let integer = |n: &Node<'_>| super::even_odd::integer(n, Some(file));
            if let Some(length) = length_call(&receiver) {
                let nonzero = match (op, integer(argument)) {
                    (b"==", Some(0)) | (b"<", Some(1)) => false,
                    (b">" | b"!=", Some(0)) => true,
                    _ => return,
                };
                (length, nonzero, false)
            } else if let Some(length) = length_call(argument) {
                let nonzero = match (integer(&receiver), op) {
                    (Some(0), b"==") | (Some(1), b">") => false,
                    (Some(0), b"<" | b"!=") => true,
                    _ => return,
                };
                (length, nonzero, true)
            } else {
                return;
            }
        };
        // Upstream's csend callback deliberately omits nonzero comparisons.
        if nonzero && length.is_safe_navigation() {
            return;
        }
        // These exceptions are directional, ordinary-send patterns, not type
        // inference: a stored stat object or a reversed comparison still flags.
        if !reversed && non_polymorphic_collection(&call, &length) {
            return;
        }
        let end = ast::source_end(node);
        let start = if op == b"zero?" {
            length.message_loc().unwrap().start_offset()
        } else {
            node.location().start_offset()
        };
        let method = std::str::from_utf8(length.name().as_slice()).unwrap();
        let current = if op == b"zero?" {
            crate::offense::single_line(&file.bytes[start..end])
        } else {
            let op = std::str::from_utf8(op).unwrap();
            let number = if matches!(op, "<" | ">") && !nonzero {
                1
            } else {
                0
            };
            if reversed {
                format!("{number} {op} {method}")
            } else {
                format!("{method} {op} {number}")
            }
        };
        let preferred = if nonzero { "!empty?" } else { "empty?" };
        // `zero?` replaces `size.zero?`; a comparison is replaced whole by
        // `recv.empty?`, or `!recv.empty?` when `zero_length_node` (a plain
        // send) does not match.
        let correction = cx
            .corrections_requested()
            .then(|| {
                if op == b"zero?" {
                    return Some(Correction {
                        range: start..node.location().end_offset(),
                        replacement: "empty?".into(),
                    });
                }
                let receiver = length.receiver()?;
                let dot = length.call_operator_loc()?;
                let bang = if nonzero || call.is_safe_navigation() {
                    "!"
                } else {
                    ""
                };
                Some(Correction {
                    range: node.location().start_offset()..node.location().end_offset(),
                    replacement: format!(
                        "{bang}{}{}empty?",
                        String::from_utf8_lossy(receiver.location().as_slice()),
                        String::from_utf8_lossy(dot.as_slice())
                    ),
                })
            })
            .flatten();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: format!("Use `{preferred}` instead of `{current}`."),
            correction,
        });
    }
}

fn length_call<'pr>(node: &Node<'pr>) -> Option<CallNode<'pr>> {
    let call = node.as_call_node()?;
    (matches!(call.name().as_slice(), b"size" | b"length")
        && call.receiver().is_some()
        && ast::call_arguments(&call).is_empty()
        && call.block().is_none())
    .then_some(call)
}

fn non_polymorphic_collection(parent: &CallNode<'_>, length: &CallNode<'_>) -> bool {
    if parent.is_safe_navigation() || length.is_safe_navigation() {
        return false;
    }
    let Some(base) = length.receiver().and_then(|n| n.as_call_node()) else {
        return false;
    };
    // Literal blocks wrap the send upstream; &block remains an argument
    // and is permitted by constructor patterns (and counted for File.stat).
    if base.is_safe_navigation()
        || base
            .block()
            .is_some_and(|block| block.as_block_node().is_some())
    {
        return false;
    }
    let Some(constant) = base.receiver() else {
        return false;
    };
    let name = base.name();
    let name = name.as_slice();
    if name == b"stat" && ast::is_global_const(&constant, &["File"]) {
        return ast::call_arguments(&base).len() == 1;
    }
    matches!(name, b"new" | b"open")
        && ast::is_global_const(&constant, &["File", "Tempfile", "StringIO"])
        || name == b"new"
            && constant.as_constant_path_node().is_some_and(|path| {
                path.name().is_some_and(|name| name.as_slice() == b"Stat")
                    && path
                        .parent()
                        .is_some_and(|parent| ast::is_global_const(&parent, &["File"]))
            })
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop style/zero_length_predicate_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/ZeroLengthPredicate")
            .collect()
    }

    #[test]
    fn zero_predicate_selector_ranges_and_source_text() {
        for (source, line, col, end_line, current) in [
            ("[1, 2, 3].length.zero?", 1, 11, 1, "length.zero?"),
            ("![1, 2, 3].size.zero?", 1, 12, 1, "size.zero?"),
            ("array&.length&.zero?", 1, 8, 1, "length&.zero?"),
            ("array.length().zero?()", 1, 7, 1, "length().zero?()"),
            ("array.length.\n  zero?", 1, 7, 2, "length.   zero?"),
            ("array.length.zero? { work }", 1, 7, 1, "length.zero?"),
            ("# header\r\n  array.size.zero?\r\n", 2, 9, 2, "size.zero?"),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, end_line)
            );
            assert_eq!(
                found[0].message,
                format!("Use `empty?` instead of `{current}`.")
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn safe_navigation_keeps_pinned_zero_nonzero_asymmetry() {
        for source in [
            "x&.size == 0",
            "0 == x&.size",
            "x&.length < 1",
            "1 > x&.length",
            "x&.size.zero?",
            "x&.size&.zero?",
            "x.size&.zero?",
            "x.size&.==(0)",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "x&.size > 0",
            "0 < x&.size",
            "x&.length != 0",
            "0 != x&.length",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn exact_integer_values_bases_unary_signs_and_line_macros() {
        for zero in [
            "0", "00", "0x0", "0o0", "0b0", "0d0", "+0", "-0", "- 0", "-\n0", "- -0",
        ] {
            let source = format!("x.size == {zero}");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(found[0].message, "Use `empty?` instead of `size == 0`.");
        }
        for one in ["1", "0x1", "0o1", "0b1", "0d1", "+1", "- -1", "__LINE__"] {
            let source = format!("x.length < {one}");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(found[0].message, "Use `empty?` instead of `length < 1`.");
        }
        for number in [
            "0.0",
            "-0.0",
            "0e0",
            "0r",
            "0i",
            "0ri",
            "-1",
            "1",
            "(0)",
            "+ 0",
            "ZERO",
            "Integer(0)",
            "100000000000000000000000000000000000000",
        ] {
            for source in [
                format!("x.size == {number}"),
                format!("{number} == x.length"),
            ] {
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        assert!(offenses("# header\nx.length < __LINE__").is_empty());
    }

    #[test]
    fn standard_library_exemptions_are_structural_and_directional() {
        for base in [
            "File.stat(path)",
            "::File.stat(path)",
            "File.new(path)",
            "File.open(path)",
            "Tempfile.new",
            "Tempfile.open(path)",
            "::StringIO.new(text)",
            "StringIO.open",
            "File::Stat.new(path)",
            "::File::Stat.new(path)",
        ] {
            for suffix in [
                ".size == 0",
                ".length < 1",
                ".size > 0",
                ".size != 0",
                ".length.zero?",
            ] {
                assert!(
                    offenses(&format!("{base}{suffix}")).is_empty(),
                    "{base}{suffix}"
                );
            }
            // The reverse-order pattern and csend are not exempt upstream.
            for source in [
                format!("0 == {base}.size"),
                format!("{base}&.size == 0"),
                format!("{base}.size&.zero?"),
            ] {
                assert_eq!(offenses(&source).len(), 1, "{source}");
            }
        }
        for source in [
            "stat = File.stat(path); stat.size == 0",
            "File = Other; File.new(path).size == 0",
            "module Foo; File.new(path).size.zero?; end",
        ] {
            let expected = usize::from(source.starts_with("stat ="));
            // No speculative variable/constant resolution: upstream matches spelling.
            assert_eq!(offenses(source).len(), expected, "{source}");
        }
        for base in [
            "Other::File.new(path)",
            "File::Stat.open(path)",
            "file.new(path)",
            "File.stat",
            "File.stat(path, other)",
            "File&.new(path)",
            "File.new(path) { work }",
            "File.stat(path) { work }",
        ] {
            assert_eq!(offenses(&format!("{base}.size == 0")).len(), 1, "{base}");
        }
    }

    #[test]
    fn constructor_block_arguments_preserve_exemptions_and_stat_arity() {
        for base in [
            "File.open(\"/dev/null\", &nil)",
            "::File.new(path, &block)",
            "Tempfile.open(path, &block)",
            "Tempfile.new(&nil)",
            "StringIO.open(&block)",
            "::StringIO.new(text, &block)",
            "File::Stat.new(path, &block)",
            "::File::Stat.new(&nil)",
            "File.stat(&block)",
        ] {
            for suffix in [".size == 0", ".length.zero?"] {
                let source = format!("{base}{suffix}");
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        // A block argument is a send child upstream, so it counts toward
        // File.stat's exact single-argument exemption. Literal blocks wrap
        // the constructor send instead and are never exempt.
        for base in [
            "File.stat(path, &nil)",
            "File.open(path) { work }",
            "StringIO.open { work }",
            "File::Stat.new(path) { work }",
        ] {
            for (suffix, col, current) in [
                (".size == 0", 1, "size == 0"),
                (".length.zero?", base.len() as u32 + 2, "length.zero?"),
            ] {
                let source = format!("{base}{suffix}");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
                assert_eq!(
                    found[0].message,
                    format!("Use `empty?` instead of `{current}`.")
                );
                assert!(found[0].correction.is_none());
            }
        }
    }

    #[test]
    fn no_offense_lookalikes_arguments_blocks_and_grouping() {
        for source in [
            "size == 0",
            "length > 0",
            "0 != size",
            "size.zero?",
            "size = 0; size == 0",
            "x.size <= 0",
            "x.size >= 1",
            "0 > x.size",
            "x.size < 0",
            "x.length == 1",
            "x.count == 0",
            "x.empty?",
            "x.size.positive?",
            "x.length.nonzero?",
            "x.size(arg) == 0",
            "x.length(&block).zero?",
            "x.size { work } == 0",
            "x.size.zero?(arg)",
            "x.size.zero?(&block)",
            "x.size.==(0, 1)",
            "(x.size) == 0",
            "(x.size).zero?",
            "x.size == (0)",
            "x.size == + 0",
            "# x.size == 0\n\"x.length.zero?\"",
            "def length; 0; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn file_stat_constructors_are_exempt_but_namespaced_lookalikes_are_not() {
        for base in ["File::Stat.new(path)", "::File::Stat.new(path)"] {
            assert!(offenses(&format!("{base}.size == 0")).is_empty());
            assert!(offenses(&format!("{base}.size.zero?")).is_empty());
        }
        assert_eq!(offenses("Other::File::Stat.new(path).size == 0").len(), 1);
    }

    #[test]
    fn pinned_comparison_matrix_has_exact_positions_and_messages() {
        for receiver in ["[1, 2, 3]", "{ a: 1, b: 2 }", "\"string\"", "collection"] {
            for method in ["length", "size"] {
                for (expression, current, preferred) in [
                    (
                        format!("{receiver}.{method} == 0"),
                        format!("{method} == 0"),
                        "empty?",
                    ),
                    (
                        format!("0 == {receiver}.{method}"),
                        format!("0 == {method}"),
                        "empty?",
                    ),
                    (
                        format!("{receiver}.{method} < 1"),
                        format!("{method} < 1"),
                        "empty?",
                    ),
                    (
                        format!("1 > {receiver}.{method}"),
                        format!("1 > {method}"),
                        "empty?",
                    ),
                    (
                        format!("{receiver}.{method} > 0"),
                        format!("{method} > 0"),
                        "!empty?",
                    ),
                    (
                        format!("0 < {receiver}.{method}"),
                        format!("0 < {method}"),
                        "!empty?",
                    ),
                    (
                        format!("{receiver}.{method} != 0"),
                        format!("{method} != 0"),
                        "!empty?",
                    ),
                    (
                        format!("0 != {receiver}.{method}"),
                        format!("0 != {method}"),
                        "!empty?",
                    ),
                ] {
                    let found = offenses(&format!("# header\n  {expression}\n"));
                    assert_eq!(found.len(), 1, "{expression}");
                    assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                    assert_eq!(
                        found[0].message,
                        format!("Use `{preferred}` instead of `{current}`.")
                    );
                    assert!(found[0].correction.is_none());
                }
            }
        }
    }
}
