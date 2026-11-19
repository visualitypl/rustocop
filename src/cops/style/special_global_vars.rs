//! `Style/SpecialGlobalVars` — fixed English-name policy; unsafe correction.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct SpecialGlobalVars;
impl Cop for SpecialGlobalVars {
    fn name(&self) -> &'static str {
        "Style/SpecialGlobalVars"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_global_variable_read_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(read) = node.as_global_variable_read_node() else {
            return;
        };
        let name = read.name().as_slice();
        let (preferred, suffix) = match name {
            b"$:" => ("$LOAD_PATH", ""),
            b"$\"" => ("$LOADED_FEATURES", ""),
            b"$0" => ("$PROGRAM_NAME", ""),
            b"$*" => (
                "$ARGV",
                " from the stdlib 'English' module (don't forget to require it) or `ARGV`",
            ),
            _ => {
                let preferred = match name {
                    b"$!" => "$ERROR_INFO",
                    b"$@" => "$ERROR_POSITION",
                    b"$;" => "$FIELD_SEPARATOR` or `$FS",
                    b"$," => "$OUTPUT_FIELD_SEPARATOR` or `$OFS",
                    b"$/" => "$INPUT_RECORD_SEPARATOR` or `$RS",
                    b"$\\" => "$OUTPUT_RECORD_SEPARATOR` or `$ORS",
                    b"$." => "$INPUT_LINE_NUMBER` or `$NR",
                    b"$_" => "$LAST_READ_LINE",
                    b"$>" => "$DEFAULT_OUTPUT",
                    b"$<" => "$DEFAULT_INPUT",
                    b"$$" => "$PROCESS_ID` or `$PID",
                    b"$?" => "$CHILD_STATUS",
                    b"$~" => "$LAST_MATCH_INFO",
                    b"$=" => "$IGNORECASE",
                    _ => return,
                };
                (
                    preferred,
                    " from the stdlib 'English' module (don't forget to require it)",
                )
            }
        };
        let correction = cx
            .corrections_requested()
            .then(|| correction(node, cx, preferred, file))
            .flatten();
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Prefer `{preferred}`{suffix} over `{}`.",
                String::from_utf8_lossy(name)
            ),
            line,
            col,
            end_line: line,
            correction,
        });
    }
}

/// The English name a global becomes, if it has one.
fn english(name: &[u8]) -> Option<&'static str> {
    Some(match name {
        b"$:" => "$LOAD_PATH",
        b"$\"" => "$LOADED_FEATURES",
        b"$0" => "$PROGRAM_NAME",
        b"$*" => "$ARGV",
        b"$!" => "$ERROR_INFO",
        b"$@" => "$ERROR_POSITION",
        b"$;" => "$FIELD_SEPARATOR",
        b"$," => "$OUTPUT_FIELD_SEPARATOR",
        b"$/" => "$INPUT_RECORD_SEPARATOR",
        b"$\\" => "$OUTPUT_RECORD_SEPARATOR",
        b"$." => "$INPUT_LINE_NUMBER",
        b"$_" => "$LAST_READ_LINE",
        b"$>" => "$DEFAULT_OUTPUT",
        b"$<" => "$DEFAULT_INPUT",
        b"$$" => "$PROCESS_ID",
        b"$?" => "$CHILD_STATUS",
        b"$~" => "$LAST_MATCH_INFO",
        b"$=" => "$IGNORECASE",
        _ => return None,
    })
}

/// `RequireEnglish`: the names that come from the English library.
fn from_english(name: &str) -> bool {
    !matches!(name, "$LOAD_PATH" | "$LOADED_FEATURES" | "$PROGRAM_NAME")
}

/// Upstream's `autocorrect`: the global (or the one-statement parentheses or
/// interpolation around it) becomes its English name, and the file's first
/// such offense also adds `require 'English'` before the first top-level
/// statement, unless one comes first, removing later top-level ones.
fn correction(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    preferred: &str,
    file: &SourceFile,
) -> Option<Correction> {
    let name = preferred.split("` or `").next()?;
    let mut target = node.location().start_offset()..node.location().end_offset();
    let mut begin = false;
    let mut parent = None;
    for ancestor in cx.ancestors() {
        match ancestor {
            Node::StatementsNode { .. } if ancestor.as_statements_node()?.body().len() == 1 => {}
            Node::EmbeddedVariableNode { .. } => {}
            Node::ParenthesesNode { .. } | Node::EmbeddedStatementsNode { .. }
                if parent.is_none() =>
            {
                let single = match ancestor {
                    Node::ParenthesesNode { .. } => {
                        ancestor.as_parentheses_node()?.body().is_some_and(|b| {
                            b.as_statements_node().is_none_or(|s| s.body().len() == 1)
                        })
                    }
                    _ => ancestor
                        .as_embedded_statements_node()?
                        .statements()
                        .is_some_and(|s| s.body().len() == 1),
                };
                if !single {
                    parent = Some(ancestor);
                    break;
                }
                target = ancestor.location().start_offset()..ancestor.location().end_offset();
                begin = true;
            }
            _ => {
                parent = Some(ancestor);
                break;
            }
        }
    }
    let interpolated = parent.is_some_and(|p| {
        matches!(
            p,
            Node::InterpolatedStringNode { .. }
                | Node::InterpolatedXStringNode { .. }
                | Node::InterpolatedRegularExpressionNode { .. }
                | Node::InterpolatedMatchLastLineNode { .. }
        )
    });
    let replacement = match (interpolated, begin) {
        (false, _) => name.to_owned(),
        (true, true) => format!("#{{{name}}}"),
        (true, false) => format!("{{{name}}}"),
    };
    let mut edits = vec![(target, replacement)];
    if from_english(name) && first_english_offense(node, cx, file) {
        edits.extend(require_english(node, cx, file)?);
    }
    Correction::from_edits(&file.bytes, edits)
}

/// Whether no enabled offense before this one adds the require (upstream's
/// `@required_english`).
fn first_english_offense(node: &Node<'_>, cx: &NodeContext<'_, '_>, file: &SourceFile) -> bool {
    use crate::cops::walk::{NodeVisitor, walk};
    struct Earlier<'a> {
        before: usize,
        file: &'a SourceFile,
        found: bool,
    }
    impl<'pr> NodeVisitor<'pr> for Earlier<'_> {
        fn enter(&mut self, node: Node<'pr>) {
            if let Some(read) = node.as_global_variable_read_node()
                && node.location().start_offset() < self.before
                && english(read.name().as_slice()).is_some_and(from_english)
            {
                let line = self.file.line_of(node.location().start_offset());
                self.found |=
                    self.file
                        .cop_enabled_for_lines("Style/SpecialGlobalVars", line, line);
            }
        }
    }
    let Some(root) = cx.ancestors().last() else {
        return true;
    };
    let mut earlier = Earlier {
        before: node.location().start_offset(),
        file,
        found: false,
    };
    walk(&mut earlier, root);
    !earlier.found
}

/// `ensure_required(corrector, node, 'English')`.
fn require_english(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Vec<(std::ops::Range<usize>, String)>> {
    let program = cx.ancestors().last()?.as_program_node()?;
    let statements: Vec<_> = program.statements().body().iter().collect();
    let start = node.location().start_offset();
    let index = statements
        .iter()
        .position(|s| s.location().start_offset() <= start && start < s.location().end_offset())?;
    let requires_english = |n: &Node<'_>| {
        n.as_call_node().is_some_and(|call| {
            call.name().as_slice() == b"require"
                && call.receiver().is_none_or(|r| {
                    r.as_constant_read_node()
                        .is_some_and(|c| c.name().as_slice() == b"Kernel")
                        || r.as_constant_path_node().is_some_and(|c| {
                            c.parent().is_none()
                                && c.name().is_some_and(|n| n.as_slice() == b"Kernel")
                        })
                })
                && matches!(
                    crate::cops::ast::call_arguments(&call).as_slice(),
                    [arg] if arg.as_string_node().is_some_and(|s| s.unescaped() == b"English")
                )
        })
    };
    let mut edits = Vec::new();
    if statements.len() > 1 {
        if statements[..index].iter().any(requires_english) {
            return Some(Vec::new());
        }
        for sibling in statements[index + 1..]
            .iter()
            .filter(|s| requires_english(s))
        {
            let loc = sibling.location();
            edits.push((
                super::correction::whole_lines(file, loc.start_offset()..loc.end_offset()),
                String::new(),
            ));
        }
    }
    let at = statements[0].location().start_offset();
    edits.push((at..at, "require 'English'\n".into()));
    Some(edits)
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/special_global_vars_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Unsafe corrections are omitted.
    use super::*;
    use crate::pipeline::lint_str;
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/SpecialGlobalVars")
            .collect()
    }
    #[test]
    fn pinned_and_remaining_english_mappings_report_exact_messages_and_edits() {
        for (name, preferred, english) in [
            ("$:", "$LOAD_PATH", false),
            ("$\"", "$LOADED_FEATURES", false),
            ("$0", "$PROGRAM_NAME", false),
            ("$!", "$ERROR_INFO", true),
            ("$@", "$ERROR_POSITION", true),
            ("$;", "$FIELD_SEPARATOR` or `$FS", true),
            ("$,", "$OUTPUT_FIELD_SEPARATOR` or `$OFS", true),
            ("$/", "$INPUT_RECORD_SEPARATOR` or `$RS", true),
            ("$\\", "$OUTPUT_RECORD_SEPARATOR` or `$ORS", true),
            ("$.", "$INPUT_LINE_NUMBER` or `$NR", true),
            ("$_", "$LAST_READ_LINE", true),
            ("$>", "$DEFAULT_OUTPUT", true),
            ("$<", "$DEFAULT_INPUT", true),
            ("$$", "$PROCESS_ID` or `$PID", true),
            ("$?", "$CHILD_STATUS", true),
            ("$~", "$LAST_MATCH_INFO", true),
            ("$=", "$IGNORECASE", true),
        ] {
            let source = format!("# header\nputs {name}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 6, 2));
            let suffix = if english {
                " from the stdlib 'English' module (don't forget to require it)"
            } else {
                ""
            };
            assert_eq!(
                found[0].message,
                format!("Prefer `{preferred}`{suffix} over `{name}`.")
            );
            // Unsafe: the first English name, and `require 'English'` for the
            // names that need it.
            let first = preferred.split("` or `").next().unwrap();
            let require = if english { "require 'English'\n" } else { "" };
            assert_eq!(
                crate::cops::testing::fix_until_stable(&SpecialGlobalVars, &source),
                format!("# header\n{require}puts {first}\n"),
                "{source}"
            );
        }
        assert_eq!(
            offenses("puts $*")[0].message,
            "Prefer `$ARGV` from the stdlib 'English' module (don't forget to require it) or `ARGV` over `$*`."
        );
    }
    #[test]
    fn interpolation_locations_and_english_require_do_not_change_detection() {
        for (source, col) in [("\"#$:\"", 3), ("\"#{$!}\"", 4), ("`#$0`", 3), ("/#$$/", 3)] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col));
        }
        assert_eq!(offenses("require 'English'\nputs $$\n").len(), 1);
        assert_eq!(offenses("puts $$\nrequire 'English'\n").len(), 1);
        assert_eq!(offenses("puts $*[0]\nputs $*[1]\n").len(), 2);
    }
    #[test]
    fn writes_backrefs_and_already_english_names_are_not_reads_but_aliases_are() {
        for source in [
            "$/ = 'x'",
            "$/ ||= 'x'",
            "$_ &&= 'x'",
            "$0 += 'x'",
            "$0, x = 'x', 1",
            "puts $1; puts $+; puts $&; puts $MATCH; puts $LOAD_PATH; puts $FS; puts $PID; puts $ARGV; puts ARGV",
            "'$/' # $/\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_eq!(offenses("$/ = $/").len(), 1);
        let found = offenses("alias $custom $/");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 15));
    }
}
