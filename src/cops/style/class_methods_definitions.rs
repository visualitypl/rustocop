//! `Style/ClassMethodsDefinitions` — opt-in fixed `def_self` policy.
use super::class_methods::elements;
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
            .then(|| flatten(node, &children, file))
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

/// RuboCop's `autocorrect_sclass`.
fn flatten(node: &Node<'_>, children: &[Node<'_>], file: &SourceFile) -> Option<Correction> {
    let singleton = node.as_singleton_class_node()?;
    let column = |offset: usize| -> Option<usize> {
        let start = file.line_start(file.line_of(offset));
        Some(
            std::str::from_utf8(&file.bytes[start..offset])
                .ok()?
                .chars()
                .count(),
        )
    };
    let sclass_column = column(node.location().start_offset())?;
    let mut previous_end = singleton.expression().location().end_offset();
    let mut removals = Vec::new();
    let mut rewritten = Vec::new();
    for child in children {
        let child_end = child.location().end_offset();
        let Some(def) = child.as_def_node().filter(|d| d.receiver().is_none()) else {
            previous_end = child_end;
            continue;
        };
        let start = child.location().start_offset();
        // `source_range_with_comment`: from the newline before the first
        // leading comment's line (or the def's) to the end of its last line.
        let first_comment = file
            .comments()
            .iter()
            .find(|c| c.start >= previous_end && c.end <= start)
            .map(|c| c.start)
            .filter(|&c| file.line_of(c) < file.line_of(start));
        let begin = file
            .line_start(file.line_of(first_comment.unwrap_or(start)))
            .checked_sub(1)?;
        let mut end = file.line_start(file.line_of(child_end - 1) + 1);
        while end > child_end && matches!(file.bytes[end - 1], b'\n' | b'\r') {
            end -= 1;
        }
        let keyword = def.def_keyword_loc().start_offset();
        let name_end = def.name_loc().end_offset();
        let source = format!(
            "{}def self.{}{}",
            std::str::from_utf8(&file.bytes[begin..keyword]).ok()?,
            std::str::from_utf8(def.name().as_slice()).ok()?,
            std::str::from_utf8(&file.bytes[name_end..end]).ok()?
        );
        let diff = column(start)?.saturating_sub(sclass_column);
        let prefix = " ".repeat(diff);
        let source: String = source
            .split_inclusive('\n')
            .map(|line| line.strip_prefix(prefix.as_str()).unwrap_or(line))
            .collect();
        rewritten.push(source.strip_suffix('\n').unwrap_or(&source).to_owned());
        removals.push(begin..end);
        previous_end = child_end;
    }
    let loc = node.location();
    if children
        .iter()
        .all(|c| c.as_def_node().is_some_and(|d| d.receiver().is_none()))
    {
        if let Some(first) = rewritten.first_mut() {
            *first = first.trim().to_owned();
        }
        return Some(Correction {
            range: loc.start_offset()..loc.end_offset(),
            replacement: rewritten.join("\n"),
        });
    }
    let mut edits: Vec<Correction> = removals
        .into_iter()
        .map(|range| Correction {
            range,
            replacement: String::new(),
        })
        .collect();
    let at = loc.end_offset();
    edits.push(Correction {
        range: at..at,
        replacement: format!("\n{}", rewritten.join("\n")),
    });
    super::correction::combine(file, edits)
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
}
