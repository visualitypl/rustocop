//! `Style/RedundantInterpolationUnfreeze` — Ruby >=3.0 (fixed target 3.4).
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantInterpolationUnfreeze;
impl Cop for RedundantInterpolationUnfreeze {
    fn name(&self) -> &'static str {
        "Style/RedundantInterpolationUnfreeze"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some((string, new)) = super::interpolation_safety::unfreeze(node) else {
            return;
        };
        let call = node.as_call_node().unwrap();
        let Some(selector) = call.message_loc() else {
            return;
        };
        let start = if new {
            node.location().start_offset()
        } else {
            selector.start_offset()
        };
        let (line, col) = file.line_col(start);
        let correction = cx
            .corrections_requested()
            .then(|| {
                let root = cx.ancestors().next_back()?;
                let facts = file.interpolation_facts(root);
                if !facts.core_before(node.location().end_offset())
                    || !facts.shape(&string).closed
                    || !facts.runtime_string(&string)
                    || call.block().is_some()
                {
                    return None;
                }
                // Heap observability is checked centrally for all emitted edits,
                // not just dup/new wrappers (which allocate additional Strings).
                let loc = string.location();
                let range = node.location().start_offset()..node.location().end_offset();
                if super::correction::contains_comment(file, range.clone()) {
                    return None;
                }
                // Prism's heredoc location is the header only. Deleting the wrapper
                // retains deferred bodies verbatim and never owns the terminator.
                let mut edits = Vec::new();
                if range.start < loc.start_offset() {
                    edits.push(Correction {
                        range: range.start..loc.start_offset(),
                        replacement: String::new(),
                    });
                }
                if loc.end_offset() < range.end {
                    edits.push(Correction {
                        range: loc.end_offset()..range.end,
                        replacement: String::new(),
                    });
                }
                let edit = super::correction::combine(file, edits)?;
                super::literal_spelling::safe_edit(file, cx, edit)
            })
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Don't unfreeze interpolated strings as they are already unfrozen.".into(),
            line,
            col,
            end_line: file.line_of(selector.end_offset().saturating_sub(1)),
            correction,
        });
    }
}
#[cfg(test)]
mod tests {
    //! RuboCop redundant_interpolation_unfreeze_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == RedundantInterpolationUnfreeze.name())
            .collect()
    }
    #[test]
    fn pinned_positions_and_message() {
        for (s, col) in [
            (r##"+"#{foo} bar""##, 1),
            (r##""#{foo} bar".+@"##, 14),
            (r##""#{foo} bar".dup"##, 14),
            ("foo(+<<~MSG)\n  foo #{bar}\n  baz\nMSG\n", 5),
            ("foo(<<~MSG.dup)\n  foo #{bar}\n  baz\nMSG\n", 12),
            (r##"+"#$var""##, 1),
            (r##"+"#@var""##, 1),
            (r##"+"#@@var""##, 1),
            (r##"String.new("#{1}")"##, 1),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (1, col), "{s}");
            assert_eq!(
                o[0].message,
                "Don't unfreeze interpolated strings as they are already unfrozen."
            );
        }
    }
    #[test]
    fn pinned_and_semantic_lookalikes() {
        for s in [
            "foo(+<<~'MSG')\n  foo #{bar}\nMSG\n",
            "\"foo\".dup",
            "local.dup",
            r##""#{foo} bar".dup(baz)"##,
            "+'foo' \\\n'bar'",
            "dup",
            r##"String.new("#{1}", encoding: Encoding::US_ASCII)"##,
            r##"::String.new("#{1}")"##,
            r##"Other::String.new("#{1}")"##,
            "`#{foo}`.dup",
            "%W[#{foo}].dup",
            r##""#{foo}"&.dup"##,
            r##"String&.new("#{1}")"##,
            r##"+"#$1""##,
            r##"+"#$+""##,
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn closed_string_branches_do_not_prove_mutable_interpolation() {
        for header in [
            "",
            "# frozen_string_literal: true\n",
            "# frozen_string_literal: false\n",
        ] {
            for predicate in ["true", "false", "nil", "0"] {
                for expr in [
                    "''".to_string(),
                    format!("{predicate} ? '' : 'yes'"),
                    format!("{predicate} ? 'yes' : ''"),
                    format!("if {predicate}; '' else 'yes' end"),
                    format!("unless {predicate}; '' else 'yes' end"),
                ] {
                    for body in [
                        format!("\"#{{{expr}}}\""),
                        format!("\"prefix#{{{expr}}}suffix\""),
                        format!("\"#{{\"#{{{expr}}}\"}}\""),
                    ] {
                        for wrapped in [
                            format!("+{body}"),
                            format!("{body}.+@"),
                            format!("{body}.dup"),
                            format!("String.new({body})"),
                        ] {
                            let source = format!("{header}{wrapped}");
                            let o = correction_offenses(&RedundantInterpolationUnfreeze, &source);
                            assert_eq!(o.len(), 1, "{source}");
                            assert_eq!(o[0].line, if header.is_empty() { 1 } else { 2 });
                            assert!(o[0].correction.is_none(), "{source}");
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn primitive_siblings_do_not_prove_whole_string_mutability() {
        for header in [
            "",
            "# frozen_string_literal: true\n",
            "# frozen_string_literal: false\n",
        ] {
            for body in [
                r##""#{1}#{''}""##,
                r##""#{''}#{1}""##,
                r##""#{1}#{true ? '' : 'yes'}#{''}""##,
                r##""#{1}#{false ? 'yes' : ''}#{''}""##,
            ] {
                for wrapped in [
                    format!("+{body}"),
                    format!("{body}.+@"),
                    format!("{body}.dup"),
                    format!("String.new({body})"),
                ] {
                    let source = format!("{header}{wrapped}");
                    let o = correction_offenses(&RedundantInterpolationUnfreeze, &source);
                    assert_eq!(o.len(), 1, "{source}");
                    assert!(o[0].correction.is_none(), "{source}");
                }
            }
        }
    }
    #[test]
    fn closed_primitive_canonical_corrections() {
        for header in [
            "",
            "# frozen_string_literal: true\n",
            "# frozen_string_literal: false\n",
        ] {
            for value in [
                "1",
                "0",
                "1.25",
                "true",
                "false",
                "nil",
                "1}#{nil",
                "nil}#{false",
                "nil}#{nil",
            ] {
                let body = format!("\"#{{{value}}}\"");
                for wrapped in [
                    format!("+{body}"),
                    format!("{body}.+@"),
                    format!("{body}.dup"),
                    format!("String.new({body})"),
                ] {
                    assert_correction(
                        &RedundantInterpolationUnfreeze,
                        &format!("{header}{wrapped}"),
                        &format!("{header}{body}"),
                    );
                }
            }
        }
        assert_correction(
            &RedundantInterpolationUnfreeze,
            r##"+"#{1} bar""##,
            r##""#{1} bar""##,
        );
    }
    #[test]
    fn all_pinned_unknown_effect_correction_pairs_are_individually_withheld() {
        for (before, after, count) in [
            (r##"+"#{foo} bar""##, r##""#{foo} bar""##, 1),
            (r##""#{foo} bar".+@"##, r##""#{foo} bar""##, 1),
            (r##""#{foo} bar".dup"##, r##""#{foo} bar""##, 1),
            (
                "foo(+<<~MSG)\n  foo #{bar}\n  baz\nMSG\n",
                "foo(<<~MSG)\n  foo #{bar}\n  baz\nMSG\n",
                1,
            ),
            (r##"+"#$var""##, r##""#$var""##, 1),
            (r##"+"#@var""##, r##""#@var""##, 1),
            (r##"+"#@@var""##, r##""#@@var""##, 1),
            (
                "foo(<<~MSG.dup)\n  foo #{bar}\n  baz\nMSG\n",
                "foo(<<~MSG)\n  foo #{bar}\n  baz\nMSG\n",
                1,
            ),
            (
                r##"+"#{String.new("#{foo}")} #{"#{bar}".dup}""##,
                r##""#{"#{foo}"} #{"#{bar}"}""##,
                3,
            ),
        ] {
            assert_eq!(ruby_prism::parse(after.as_bytes()).errors().count(), 0);
            let o = correction_offenses(&RedundantInterpolationUnfreeze, before);
            assert_eq!(o.len(), count, "{before}");
            assert!(o.iter().all(|o| o.correction.is_none()));
        }
    }
    #[test]
    fn dispatch_binding_visibility_alias_macro_and_prior_to_s_hazards() {
        for prefix in [
            "class String; def +@; 'changed'; end; end",
            "class String; def dup; 'changed'; end; end",
            "class String; def initialize_copy(x); raise; end; end",
            "class String; def initialize(x); raise; end; end",
            "class << String; private :new; end",
            "class String; alias dup to_s; end",
            "class String; attr_reader :dup; end",
            "module R; refine String do; def dup; 'changed'; end; end; end; using R",
            "String = Class.new",
            "effect",
            r##""#{object}""##,
            r##""#@object""##,
            r##""#{$object}""##,
        ] {
            for expr in [r##"+"#{1}""##, r##""#{1}".dup"##, r##"String.new("#{1}")"##] {
                let src = format!("{prefix}\n{expr}");
                let o = correction_offenses(&RedundantInterpolationUnfreeze, &src);
                assert_eq!(o.len(), 1, "{src}");
                assert!(o[0].correction.is_none(), "{src}");
            }
        }
    }
    #[test]
    fn closed_heredoc_header_keeps_deferred_body_and_sibling_ownership() {
        for (before, after) in [
            ("p(+<<~MSG)\n  #{1}\nMSG\n", "p(<<~MSG)\n  #{1}\nMSG\n"),
            ("p(<<~MSG.dup)\n  #{1}\nMSG\n", "p(<<~MSG)\n  #{1}\nMSG\n"),
            (
                "p(String.new(<<~MSG), <<~OTHER)\n  #{1}\nMSG\n  other\nOTHER\n",
                "p(<<~MSG, <<~OTHER)\n  #{1}\nMSG\n  other\nOTHER\n",
            ),
        ] {
            assert_correction(&RedundantInterpolationUnfreeze, before, after);
        }
    }
    #[test]
    fn unknown_interpolation_effects_withhold_edits_not_offenses() {
        for s in [
            r##"+"#{foo} bar""##,
            r##""#{foo} bar".dup"##,
            r##"String.new("#{foo}")"##,
            r##"+"#$var""##,
        ] {
            let o = correction_offenses(&RedundantInterpolationUnfreeze, s);
            assert_eq!(o.len(), 1);
            assert!(o[0].correction.is_none());
        }
    }
}
