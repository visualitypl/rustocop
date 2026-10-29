//! Cases ported/adapted from RuboCop's spec/rubocop/comment_config_spec.rb
//! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
use super::Directives;
use crate::source::SourceFile;

fn directives_for(source: &[u8]) -> Directives {
    let file = SourceFile::new("test.rb".into(), source.to_vec());
    let result = ruby_prism::parse(&file.bytes);
    Directives::parse(&result, &file)
}
fn debugger_lines(source: &str) -> Vec<u32> {
    crate::pipeline::lint_str(source)
        .into_iter()
        .filter(|o| o.cop == "Lint/Debugger")
        .map(|o| o.line)
        .collect()
}
fn disabled_lines(d: &Directives, cop: &str, max: u32) -> Vec<u32> {
    (1..=max).filter(|&line| !d.is_enabled(cop, line)).collect()
}

#[test]
fn default_off_cops_are_enabled_only_in_the_requested_source_scope() {
    for prefix in ["rubocop", "rustocop"] {
        for mode in ["enable-next AsciiComments", "next +AsciiComments"] {
            let source = format!("foo # café\n# {prefix}:{mode}\nfoo # café\nfoo # café\n");
            let positions: Vec<_> = crate::pipeline::lint_str(&source)
                .into_iter()
                .filter(|o| o.cop == "Style/AsciiComments")
                .map(|o| (o.line, o.col))
                .collect();
            assert_eq!(positions, [(3, 10)], "{source}");
        }
    }
}

#[test]
fn source_enabled_ban_tracks_activation_and_restoration_but_ignores_disables() {
    let ban = super::comments::BAN;
    for prefix in ["rubocop", "rustocop"] {
        for (source, expected) in [
            (
                format!(
                    "# {prefix}:push +{ban}\n# {prefix}:disable all\nfoo\n# {prefix}:pop\n# {prefix}:disable Unknown/Cop\n"
                ),
                vec![1, 4, 5, 6],
            ),
            (
                format!(
                    "# {prefix}:enable {ban}\n# {prefix}:push -{ban}\n# {prefix}:todo-next all\nfoo\n# {prefix}:pop\n# {prefix}:disable {ban}\n"
                ),
                vec![1],
            ),
        ] {
            let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
            let result = ruby_prism::parse(&file.bytes);
            let d = Directives::parse_with_disabled(&result, &file, &[ban]);
            assert_eq!(disabled_lines(&d, ban, 6), expected, "{source}");
            for first in 1..=6 {
                for last in first..=6 {
                    assert_eq!(
                        d.is_enabled_for_lines(ban, first, last),
                        (first..=last).all(|line| d.is_enabled(ban, line)),
                        "{source} {first}..{last}"
                    );
                }
            }
        }
    }
}

#[test]
fn signed_operations_follow_first_sign_group_order() {
    for prefix in ["rubocop", "rustocop"] {
        for (args, expected) in [
            ("+Debugger -Lint +Debugger", vec![5]),
            ("-Debugger +Lint -Debugger", vec![3, 5]),
        ] {
            let source = format!(
                "# {prefix}:push {args}\n# comment\nbinding.pry\n# {prefix}:pop\nbinding.pry\n"
            );
            assert_eq!(debugger_lines(&source), expected, "{source}");
        }
    }
}

#[test]
fn lexical_comment_ranges_do_not_change_merged_suppression_or_source_records() {
    for prefix in ["rubocop", "rustocop"] {
        for (mode, close) in [
            ("disable Debugger", "enable Debugger"),
            ("todo Debugger", "enable Debugger"),
            ("push -Debugger", "pop"),
            ("next -Debugger", "pop"),
            ("disable-next Debugger", "enable Debugger"),
            ("todo-next Debugger", "enable Debugger"),
        ] {
            let source = format!(
                "=begin\n# {prefix}:{mode}\n=end\nbinding.pry\n# {prefix}:{close}\nbinding.pry\n"
            );
            let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
            let result = ruby_prism::parse(&file.bytes);
            let d = Directives::parse(&result, &file);
            let records = file.directive_records();
            assert_eq!(records.len(), 2);
            assert_eq!(records[0].line, 1);
            assert!(records[0].standalone && records[0].embedded);
            assert!(file.bytes[records[0].range.clone()].starts_with(b"=begin\n"));
            assert_eq!(super::comments::suppression_records(&file).count(), 1);
            // Populate every old suppression cache before the independent
            // CommentsHelp cache, then prove no cop's effective state changes.
            let opt_ins: Vec<_> = super::comments::registered_names()
                .iter()
                .map(|cop| {
                    assert!(d.is_enabled_for_lines(cop, 1, 6), "{source} {cop}");
                    d.opts_in(cop)
                })
                .collect();
            assert!(d.contains_disables("Lint/Debugger", 1..5), "{source}");
            for (cop, opted_in) in super::comments::registered_names().iter().zip(opt_ins) {
                assert!(d.is_enabled_for_lines(cop, 1, 6), "{source} {cop}");
                assert_eq!(d.opts_in(cop), opted_in, "{source} {cop}");
            }
            assert_eq!(debugger_lines(&source), [4, 6], "{source}");
        }
    }
}

#[test]
fn lexical_block_enables_cannot_mobilize_config_disabled_cops() {
    for prefix in ["rubocop", "rustocop"] {
        for mode in [
            "enable Style/AsciiComments",
            "enable-next Style",
            "push +Style",
            "next +Style/AsciiComments",
        ] {
            let source = format!("=begin\n# {prefix}:{mode}\n=end\nwork # café\n");
            let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
            let result = ruby_prism::parse(&file.bytes);
            let cop = "Style/AsciiComments";
            let d = Directives::parse_with_disabled(&result, &file, &[cop]);
            assert!(!d.opts_in(cop), "{source}");
            // Even after the lexical diagnostic analysis has processed its
            // enable/push/next, the effective initial disable stays untouched.
            d.contains_disables(cop, 1..5);
            assert_eq!(disabled_lines(&d, cop, 6), [1, 2, 3, 4, 5, 6], "{source}");
            assert!(!d.opts_in(cop), "{source}");
        }
    }
}

#[test]
fn prior_inline_only_and_unsigned_target_policy_is_preserved() {
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("=begin\n# {prefix}:disable Debugger\n=end\nbinding.pry\n"),
            format!("# {prefix}:disable -Debugger\nbinding.pry\n"),
            format!("# {prefix}:enable +Debugger\nbinding.pry\n"),
        ] {
            assert!(directives_for(source.as_bytes()).is_empty(), "{source}");
            assert_eq!(debugger_lines(&source).len(), 1, "{source}");
        }
    }
}

#[test]
fn temporary_enables_suspend_only_open_disables() {
    for prefix in ["rubocop", "rustocop"] {
        for mode in ["enable-next Lint", "next +Debugger -Style/Not"] {
            let source = format!(
                "# {prefix}:disable Lint/Debugger\nbinding.pry\n# {prefix}:{mode}\nif ready\n  binding.pry\nend\nbinding.pry\n# {prefix}:enable Lint/Debugger\nbinding.pry\n"
            );
            assert_eq!(debugger_lines(&source), [5, 9], "{source}");
            assert_eq!(
                disabled_lines(&directives_for(source.as_bytes()), "Lint/Debugger", 9),
                [1, 2, 3, 7, 8]
            );
        }
    }
}

#[test]
fn push_pop_restores_nested_state_including_unmentioned_cops() {
    for prefix in ["rubocop", "rustocop"] {
        let source = format!(
            "# {prefix}:disable Lint/Debugger\n# {prefix}:push +Debugger -Lint/Void\nbinding.pry\n# {prefix}:push\n# {prefix}:disable Debugger\nbinding.pry\n# {prefix}:pop\nbinding.pry\n# {prefix}:pop\nbinding.pry\n"
        );
        assert_eq!(debugger_lines(&source), [3, 8]);
        let d = directives_for(source.as_bytes());
        assert_eq!(disabled_lines(&d, "Lint/Debugger", 10), [1, 2, 5, 6, 9, 10]);
        assert_eq!(disabled_lines(&d, "Lint/Void", 10), [2, 3, 4, 5, 6, 7, 8]);
    }
}

#[test]
fn detached_and_embedded_statement_forms_match_the_reference() {
    for prefix in ["rubocop", "rustocop"] {
        for mode in [
            "disable-next Debugger",
            "todo-next Debugger",
            "next -Debugger",
        ] {
            for source in [
                format!("# {prefix}:{mode}\n\nbinding.pry\n"),
                format!("work # {prefix}:{mode}\nbinding.pry\n"),
                format!("# # {prefix}:{mode}\nbinding.pry\n"),
            ] {
                assert_eq!(debugger_lines(&source).len(), 1, "{source}");
            }
            let source = format!("# note # {prefix}:{mode}\n# comment\nbinding.pry\nbinding.pry\n");
            assert_eq!(debugger_lines(&source), [4]);
        }
        for mode in ["enable-next Debugger", "next +Debugger"] {
            for middle in [
                format!("# {prefix}:{mode}\n\n"),
                format!("work # {prefix}:{mode}\n"),
            ] {
                let source = format!("# {prefix}:disable Debugger\n{middle}binding.pry\n");
                assert!(debugger_lines(&source).is_empty(), "{source}");
            }
        }
    }
}

#[test]
fn all_statement_forms_cover_heredocs_and_restore_after_the_terminator() {
    for prefix in ["rubocop", "rustocop"] {
        for (setup, mode, expected) in [
            ("", "disable-next Lint/EmptyInterpolation", vec![8]),
            ("", "todo-next Lint/EmptyInterpolation", vec![8]),
            ("", "next -Lint", vec![8]),
            (
                "disable Lint/EmptyInterpolation",
                "enable-next all",
                vec![4, 6],
            ),
            ("disable Lint", "next +EmptyInterpolation", vec![4, 6]),
        ] {
            let source = format!(
                "# {prefix}:{setup}\n# {prefix}:{mode}\nfoo(<<~A, <<~B)\n  #{{}}\nA\n  #{{}}\nB\n\"#{{}}\"\n"
            );
            let lines: Vec<_> = crate::pipeline::lint_str(&source)
                .into_iter()
                .filter(|o| o.cop == "Lint/EmptyInterpolation")
                .map(|o| o.line)
                .collect();
            assert_eq!(lines, expected, "{source}");
        }
    }
}

#[test]
fn explicit_statement_disables_are_not_undone_by_temporary_enables() {
    for prefix in ["rubocop", "rustocop"] {
        let source = format!(
            "# {prefix}:disable Debugger\n# {prefix}:todo-next Debugger\n# {prefix}:next +Debugger\nbinding.pry\n"
        );
        assert!(debugger_lines(&source).is_empty());
        let source = format!(
            "# {prefix}:disable Debugger\n# {prefix}:enable-next Debugger\nbinding.pry # {prefix}:disable Debugger\n"
        );
        assert!(debugger_lines(&source).is_empty());
    }
}

#[test]
fn pop_closes_before_its_line_and_ignores_underflow() {
    for prefix in ["rubocop", "rustocop"] {
        let source = format!(
            "# {prefix}:pop\n# {prefix}:push -Debugger\nbinding.pry\nbinding.pry # {prefix}:pop\nbinding.pry\n"
        );
        assert_eq!(debugger_lines(&source), [4, 5]);
        let source = format!(
            "# {prefix}:push -Debugger\n# {prefix}:next +Debugger\nbinding.pry\n# {prefix}:pop\nbinding.pry\n"
        );
        assert_eq!(debugger_lines(&source), [3, 5]);
        assert_eq!(
            disabled_lines(&directives_for(source.as_bytes()), "Lint/Debugger", 5),
            [1, 2]
        );
    }
}

#[test]
fn signed_departments_expand_but_signed_all_is_not_a_wildcard() {
    for prefix in ["rubocop", "rustocop"] {
        for mode in ["next", "push"] {
            let source = format!("# {prefix}:{mode} -all\nbinding.pry\n# {prefix}:pop\n");
            assert_eq!(debugger_lines(&source), [2]);
            let source =
                format!("# {prefix}:{mode} -Lint\nbinding.pry\n# {prefix}:pop\nbinding.pry\n");
            assert_eq!(debugger_lines(&source), [4]);
        }
    }
}

#[test]
fn span_queries_match_line_queries_for_restoration_and_holes() {
    for prefix in ["rubocop", "rustocop"] {
        let source = format!(
            "# {prefix}:disable Lint\n# {prefix}:push +Debugger -Style/Not\n# {prefix}:push\n# {prefix}:disable Debugger\n# {prefix}:enable-next Debugger\nbinding.pry\n# {prefix}:pop\nbinding.pry\n# {prefix}:pop\nbinding.pry\n"
        );
        let d = directives_for(source.as_bytes());
        for cop in [
            "Lint/Debugger",
            "Lint/Void",
            "Style/Not",
            "Style/DisableCopsWithinSourceCodeDirective",
        ] {
            for first in 1..=12 {
                for last in first..=12 {
                    assert_eq!(
                        d.is_enabled_for_lines(cop, first, last),
                        (first..=last).all(|line| d.is_enabled(cop, line)),
                        "{source} {cop} {first}..{last}"
                    );
                }
            }
        }
    }
}

#[test]
fn bom_crlf_and_directive_text_in_heredocs_do_not_corrupt_the_stack() {
    for prefix in ["rubocop", "rustocop"] {
        let source = format!(
            "\u{feff}# {prefix}:push -Debugger\r\nvalue = <<~TEXT\r\n  # {prefix}:pop\r\nTEXT\r\nbinding.pry\r\n# {prefix}:pop\r\nbinding.pry\r\n"
        );
        assert_eq!(debugger_lines(&source), [7]);
    }
}

#[test]
fn deep_stacks_and_stacked_attachments_share_bounded_analysis() {
    let source = format!(
        "{}{}binding.pry\n{}binding.pry\n",
        "# rubocop:push\n".repeat(3000),
        "# rubocop:todo-next Debugger\n".repeat(3000),
        "# rubocop:pop\n".repeat(3000)
    );
    let d = directives_for(source.as_bytes());
    assert!(!d.is_enabled("Lint/Debugger", 6001));
    assert!(d.is_enabled("Lint/Debugger", 9002));
    assert!(!d.is_enabled_for_lines("Lint/Debugger", 1, u32::MAX));
}

#[test]
fn todo_next_is_a_statement_scoped_alias_in_both_spellings() {
    for prefix in ["rubocop", "rustocop"] {
        let source = format!(
            "# {prefix}:todo-next Debugger -- reviewed\nif ready\n  binding.pry\nend\nbinding.pry\n"
        );
        assert_eq!(debugger_lines(&source), [5]);
        assert_eq!(
            disabled_lines(&directives_for(source.as_bytes()), "Lint/Debugger", 5),
            [2, 3, 4]
        );
    }
}
