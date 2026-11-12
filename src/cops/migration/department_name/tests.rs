//! Ported from RuboCop's spec/rubocop/cop/migration/department_name_spec.rb
//! (MIT, Copyright (c) 2012-26 Bozhidar Batsov). Further cases replay the
//! pinned 1.92.0 Team on source strings.
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    offense::Offense,
    pipeline::lint_str,
};

const MSG: &str = "Department name is missing.";

fn offenses(source: &str) -> Vec<Offense> {
    lint_str(source)
        .into_iter()
        .filter(|o| o.cop == DepartmentName.name())
        .collect()
}

fn positions(source: &str) -> Vec<(u32, u32)> {
    offenses(source)
        .into_iter()
        .map(|o| {
            assert_eq!(o.message, MSG);
            assert_eq!(o.end_line, o.line);
            (o.line, o.col)
        })
        .collect()
}

/// Offenses found, and that none of them carries an edit.
fn assert_held(source: &str, expected: &[(u32, u32)]) {
    assert_eq!(positions(source), expected, "{source:?}");
    let found = correction_offenses(&DepartmentName, source);
    assert_eq!(found.len(), expected.len(), "{source:?}");
    assert!(
        found.iter().all(|o| o.correction.is_none()),
        "{source:?}: {found:?}"
    );
}

fn assert_no_offenses(source: &str) {
    assert_eq!(positions(source), [], "{source:?}");
}

// --- Pinned spec examples -------------------------------------------------

#[test]
fn spec_todo_and_enable_comments_without_departments() {
    let source =
        "# rubocop:todo Alias, LineLength\nalias :ala :bala\n# rubocop:enable Alias, LineLength\n";
    assert_eq!(positions(source), [(1, 16), (1, 23), (3, 18), (3, 25)]);
    assert_correction(
        &DepartmentName,
        source,
        "# rubocop:todo Style/Alias, Layout/LineLength\nalias :ala :bala\n# rubocop:enable Style/Alias, Layout/LineLength\n",
    );
}

#[test]
fn spec_space_around_colon() {
    let source = "# rubocop : todo Alias, LineLength\nalias :ala :bala\n# rubocop : enable Alias, LineLength\n";
    assert_eq!(positions(source), [(1, 18), (1, 25), (3, 20), (3, 27)]);
    assert_correction(
        &DepartmentName,
        source,
        "# rubocop : todo Style/Alias, Layout/LineLength\nalias :ala :bala\n# rubocop : enable Style/Alias, Layout/LineLength\n",
    );
}

#[test]
fn spec_legacy_cop_name() {
    let source = "# rubocop:disable SingleSpaceBeforeFirstArg, Layout/LineLength\nname             \"apache_kafka\"\n";
    assert_eq!(positions(source), [(1, 19)]);
    // The department comes from the pinned legacy (renamed) cop name.
    assert_correction(
        &DepartmentName,
        source,
        "# rubocop:disable Style/SingleSpaceBeforeFirstArg, Layout/LineLength\nname             \"apache_kafka\"\n",
    );
}

#[test]
fn spec_accepted_comments() {
    for source in [
        "alias :ala :bala # rubocop:disable all\n# rubocop:disable Style/Alias\n",
        "# rubocop:disable Style/Alias # Plain code comment\nalias :ala :bala\n",
        "# rubocop:disable Style/Alias -- because something, something, and something\nalias :ala :bala\n",
        "# rubocop:disable Style:Alias\nalias :ala :bala\n",
        "# rubocop:disable Style\nalias :ala :bala\n",
    ] {
        assert_no_offenses(source);
    }
}

// --- Lifecycle ------------------------------------------------------------

#[test]
fn metadata_is_default_on_convention_and_safe() {
    let registry = crate::cops::Registry::new();
    let cop = registry
        .cops()
        .iter()
        .find(|c| c.name() == "Migration/DepartmentName")
        .expect("registered");
    assert!(cop.default_enabled());
    assert!(cop.safe_fixes());
    assert_eq!(cop.severity(), crate::offense::Severity::Convention);
    assert!(cop.path_policy().is_none());
    assert!(cop.max_setting().is_none());
}

#[test]
fn runs_on_comment_only_and_empty_programs() {
    assert_eq!(positions("# rubocop:disable Alias\n"), [(1, 19)]);
    assert_eq!(positions("# rubocop:disable Alias"), [(1, 19)]);
    assert_eq!(positions("\n\n# rubocop:enable Alias\n\n"), [(3, 18)]);
    assert_no_offenses("");
    assert_no_offenses("\n");
    assert_correction(
        &DepartmentName,
        "# rubocop:disable Alias",
        "# rubocop:disable Style/Alias",
    );
}

#[test]
fn every_comment_is_checked_but_not_strings_block_comments_or_data() {
    let source = "x = \"# rubocop:disable Alias\"\ny = <<~H\n  # rubocop:disable Alias\nH\n=begin\n# rubocop:disable Alias\n=end\nz = 1 # rubocop:disable Alias\n__END__\n# rubocop:disable Alias\n";
    assert_eq!(positions(source), [(8, 25)]);
}

// --- Directive grammar (pinned DISABLE_COMMENT_FORMAT) ---------------------

#[test]
fn header_is_anchored_at_the_comment_start_with_spaces_only() {
    for (source, expected) in [
        ("x = 1 # rubocop:disable Alias\n", vec![(1, 25)]),
        ("#rubocop:disable Alias\n", vec![(1, 18)]),
        ("#  rubocop:disable Alias\n", vec![(1, 20)]),
        ("# rubocop: disable Alias\n", vec![(1, 20)]),
        ("#rubocop :disable Alias\n", vec![(1, 19)]),
        ("#   rubocop  :   enable   Alias\n", vec![(1, 27)]),
        ("# rubocop:todo  Alias\n", vec![(1, 17)]),
        ("# rubocop:disable  Alias\n", vec![(1, 20)]),
        ("# note # rubocop:disable Alias\n", vec![]),
        ("## rubocop:disable Alias\n", vec![]),
        ("#\trubocop:disable Alias\n", vec![]),
        ("# rubocop:disable\tAlias\n", vec![]),
        ("# RUBOCOP:disable Alias\n", vec![]),
        ("# rubocop:Disable Alias\n", vec![]),
        ("# rubocop:disable-next Alias\n", vec![]),
        ("# rubocop:todo-next Alias\n", vec![]),
        ("# rubocop:enable-next Alias\n", vec![]),
        ("# rubocop:push -Alias\n", vec![]),
        ("# rubocop:next -Alias\n", vec![]),
        ("# rubocop:disabled Alias\n", vec![]),
        ("# rubocop:enable\n", vec![]),
        ("# rubocop:disable \n", vec![]),
    ] {
        assert_eq!(positions(source), expected, "{source:?}");
    }
}

#[test]
fn both_accepted_prefixes_follow_the_same_grammar() {
    for prefix in ["rubocop", "rustocop"] {
        let source = format!("# {prefix}:disable Alias, LineLength\n");
        let col = 12 + prefix.len() as u32;
        assert_eq!(positions(&source), [(1, col), (1, col + 7)], "{source}");
        assert_correction(
            &DepartmentName,
            &source,
            &format!("# {prefix}:disable Style/Alias, Layout/LineLength\n"),
        );
        assert_no_offenses(&format!("# {prefix}:disable-next Alias\n"));
        assert_no_offenses(&format!("# note # {prefix}:disable Alias\n"));
    }
    // Only the two accepted spellings; near misses are CopDirectiveSyntax's.
    for source in [
        "# robocop:disable Alias\n",
        "# rubocops:disable Alias\n",
        "# rustcop:disable Alias\n",
    ] {
        assert_no_offenses(source);
    }
}

#[test]
fn comma_lists_offsets_and_unexpected_character_break() {
    for (source, expected) in [
        ("# rubocop:todo Alias,LineLength\n", vec![(1, 16), (1, 22)]),
        (
            "# rubocop:disable Alias ,LineLength\n",
            vec![(1, 19), (1, 26)],
        ),
        (
            "# rubocop:disable Alias,, LineLength\n",
            vec![(1, 19), (1, 27)],
        ),
        (
            "# rubocop:disable Alias, , LineLength\n",
            vec![(1, 19), (1, 28)],
        ),
        (
            "# rubocop:disable Alias, Style/Alias, LineLength\n",
            vec![(1, 19), (1, 39)],
        ),
        (
            "# rubocop:disable Alias, LineLength, RedundantCopDisableDirective\n",
            vec![(1, 19), (1, 26), (1, 38)],
        ),
        // Rationale and trailing comments stop the scan after their piece.
        ("# rubocop:disable Alias -- reason\n", vec![]),
        (
            "# rubocop:disable Alias, LineLength -- reason\n",
            vec![(1, 19)],
        ),
        (
            "foo # rubocop:todo Alias, LineLength # note\n",
            vec![(1, 20)],
        ),
        ("# rubocop:disable Alias # LineLength\n", vec![]),
        ("# rubocop:disable Alias LineLength\n", vec![]),
        ("# rubocop:disable Alias\t, LineLength\n", vec![(1, 19)]),
        ("# rubocop:disable Alias,\tLineLength\n", vec![(1, 19)]),
        ("# rubocop:disable Alias\u{c}, LineLength\n", vec![(1, 19)]),
        ("# rubocop:disable Alias-foo, LineLength\n", vec![]),
        ("# rubocop:disable Alias-, LineLength\n", vec![]),
        ("# rubocop:disable Aliasé, LineLength\n", vec![]),
        (
            "# rubocop:disable Style/Alias/X, LineLength\n",
            vec![(1, 34)],
        ),
        // A name with an unexpected character is checked, then ends the scan.
        ("# rubocop:disable line_length, Alias\n", vec![(1, 19)]),
        ("# rubocop:disable Foo1, Alias\n", vec![(1, 19)]),
        ("# rubocop:disable A, LineLength\n", vec![(1, 19), (1, 22)]),
        ("# rubocop:enable  Alias \n", vec![(1, 19)]),
    ] {
        assert_eq!(positions(source), expected, "{source:?}");
    }
}

#[test]
fn departments_all_and_valid_content_tokens() {
    for source in [
        "# rubocop:disable Lint\n",
        "# rubocop:disable Migration\n",
        "# rubocop:disable Bundler, Gemspec, Layout, Metrics, Naming, Security, Style\n",
        "# rubocop:disable all\n",
        // The pinned token pattern is unanchored: any `all` is valid.
        "# rubocop:disable SmallCaps\n",
        "# rubocop:disable Install\n",
        "# rubocop:disable Style/alias\n",
    ] {
        assert_no_offenses(source);
    }
    // Only departments of the actual registry, never spec-only ones.
    assert_held("# rubocop:disable Test\n", &[(1, 19)]);
    assert_held("# rubocop:disable Rails\n", &[(1, 19)]);
    // `all` after a name still checks the name before it. Upstream reports
    // (and qualifies) `Alias` here; rustocop's existing directive scopes
    // read a listed `all` as every cop, which disables this cop on its own
    // line (a known deviation).
    assert_no_offenses("# rubocop:disable Alias, all\n");
    assert_correction(
        &DepartmentName,
        "# rubocop:enable Alias, all\n",
        "# rubocop:enable Style/Alias, all\n",
    );
}

#[test]
fn directives_disabling_this_cop_suppress_its_findings() {
    // The finding is on a line that `all` (and its own name) disables.
    for source in [
        "# rubocop:disable all, Alias\n",
        "# rubocop:disable Migration, Alias\n",
        "# rubocop:disable Migration/DepartmentName, Alias\n",
        "# rubocop:disable DepartmentName\n",
    ] {
        assert_no_offenses(source);
    }
    assert_no_offenses("# rubocop:disable Migration/DepartmentName\n# rubocop:disable Alias\n");
    assert_eq!(
        positions("x = 1 # rubocop:disable Migration/DepartmentName\n# rubocop:disable Alias\n"),
        [(2, 19)],
        "a trailing disable covers its own line only"
    );
}

// --- Correction ------------------------------------------------------------

#[test]
fn qualifies_through_the_actual_registry_and_legacy_names() {
    for (before, after) in [
        (
            "# rubocop:disable RedundantCopDisableDirective\n",
            "# rubocop:disable Lint/RedundantCopDisableDirective\n",
        ),
        (
            "# rubocop:disable Eval\n",
            "# rubocop:disable Security/Eval\n",
        ),
        (
            "# rubocop:disable AbcSize\n",
            "# rubocop:disable Metrics/AbcSize\n",
        ),
        (
            "# rubocop:disable SpaceBeforeFirstArg\n",
            "# rubocop:disable Layout/SpaceBeforeFirstArg\n",
        ),
        (
            "# rubocop:enable FrozenStringLiteralComment\n",
            "# rubocop:enable Style/FrozenStringLiteralComment\n",
        ),
        // Legacy names, first pinned obsoletion rule wins.
        ("# rubocop:disable Tab\n", "# rubocop:disable Layout/Tab\n"),
        (
            "# rubocop:disable PredicateName\n",
            "# rubocop:disable Naming/PredicateName\n",
        ),
        (
            "# rubocop:disable MultipleCompare\n",
            "# rubocop:disable Lint/MultipleCompare\n",
        ),
        (
            "# rubocop:disable TrailingComma\n",
            "# rubocop:disable Style/TrailingComma\n",
        ),
        (
            "# rubocop:disable MethodMissing\n",
            "# rubocop:disable Style/MethodMissing\n",
        ),
        (
            "x = 1 # rubocop:disable Alias\n",
            "x = 1 # rubocop:disable Style/Alias\n",
        ),
        (
            "# rubocop:disable Alias,, LineLength\n",
            "# rubocop:disable Style/Alias,, Layout/LineLength\n",
        ),
        (
            "# rubocop:disable Alias, LineLength -- reason\n",
            "# rubocop:disable Style/Alias, LineLength -- reason\n",
        ),
        (
            "# rubocop:disable Style/Alias/X, LineLength\n",
            "# rubocop:disable Style/Alias/X, Layout/LineLength\n",
        ),
        (
            "# rubocop:disable A, LineLength\n",
            "# rubocop:disable A, Layout/LineLength\n",
        ),
        (
            "# rubocop:disable Alias, Style/Alias, LineLength\n",
            "# rubocop:disable Style/Alias, Style/Alias, Layout/LineLength\n",
        ),
        (
            "# rubocop:enable  Alias \n",
            "# rubocop:enable  Style/Alias \n",
        ),
    ] {
        assert_correction(&DepartmentName, before, after);
    }
}

#[test]
fn unknown_names_keep_the_finding_without_an_edit() {
    for (source, expected) in [
        ("# rubocop:disable Foo\n", vec![(1, 19)]),
        ("# rubocop:disable Foo1, Alias\n", vec![(1, 19)]),
        ("# rubocop:disable Al1as, LineLength\n", vec![(1, 19)]),
        ("# rubocop:disable Ali_as\n", vec![(1, 19)]),
        ("# rubocop:disable Al__ias\n", vec![(1, 19)]),
        ("# rubocop:disable line_Length\n", vec![(1, 19)]),
        ("# rubocop:disable Nonexistent\n", vec![(1, 19)]),
        // Extracted departments are not names: `Rails/*` has no cop part.
        ("# rubocop:disable Performance\n", vec![(1, 19)]),
    ] {
        assert_held(source, &expected);
    }
}

#[test]
fn misaligned_and_empty_ranges_keep_upstream_positions_without_edits() {
    // Upstream highlights from the untrimmed piece start for the trimmed
    // length, so the range source is not a name and nothing is edited.
    assert_held("# rubocop:disable \tAlias\n", &[(1, 19)]);
    assert_held("# rubocop:disable \u{b}Alias, LineLength\n", &[(1, 19)]);
    assert_held("# rubocop:disable \t\n", &[(1, 19)]);
    assert_held("# rubocop:disable \u{b}\n", &[(1, 19)]);
    assert_no_offenses("x # rubocop:disable \t # c\n");
}

#[test]
fn ambiguous_names_are_reported_but_never_qualified() {
    // Upstream raises AmbiguousCopName from its corrector (a cop error that
    // drops every finding of the file); the name selects two cops here.
    assert_held("# rubocop:disable MultipleComparison\n", &[(1, 19)]);
    assert_eq!(
        positions("# rubocop:disable SelfAssignment, Alias\n"),
        [(1, 19), (1, 35)]
    );
    let found = correction_offenses(&DepartmentName, "# rubocop:disable SelfAssignment, Alias\n");
    // The unambiguous name in the same list is still qualified.
    assert!(found[0].correction.is_none());
    assert_eq!(
        found[1].correction.as_ref().unwrap().replacement,
        "Style/Alias"
    );
}

#[test]
fn case_variants_that_would_start_suppressing_are_held() {
    // Upstream camel-cases these into registered names, but rustocop's
    // directives match names literally: the qualified name would newly
    // suppress (or opt in) a cop, so the comment is left as written.
    for source in [
        "# rubocop:disable alias\n",
        "# rubocop:disable lineLength\n",
        "# rubocop:disable line_length, Alias\n",
        "# rubocop:disable Line_length\n",
        "# rubocop:disable _alias\n",
        "# rubocop:enable frozenStringLiteralComment\n",
    ] {
        assert_held(source, &[(1, offenses(source)[0].col)]);
    }
}

#[test]
fn edits_only_the_name_bytes_of_crlf_bom_and_legacy_encodings() {
    assert_correction(
        &DepartmentName,
        "# rubocop:disable Alias\r\nx = 1\r\n",
        "# rubocop:disable Style/Alias\r\nx = 1\r\n",
    );
    assert_correction(
        &DepartmentName,
        "\u{feff}# rubocop:disable Alias\n",
        "\u{feff}# rubocop:disable Style/Alias\n",
    );
    assert_correction(
        &DepartmentName,
        "# coding: ISO-8859-1\n# rubocop:todo Alias, LineLength\n",
        "# coding: ISO-8859-1\n# rubocop:todo Style/Alias, Layout/LineLength\n",
    );
    let latin1 = b"# coding: ISO-8859-1\nx = \"\xe9\" # rubocop:disable Alias\n";
    let file = SourceFile::new(std::path::PathBuf::from("t.rb"), latin1.to_vec());
    let result = crate::parser::parse(&file.bytes);
    let mut out = Vec::new();
    crate::cops::run_cops(&[&DepartmentName], &result, &file, &mut out);
    let correction = out[0].correction.as_ref().unwrap();
    assert_eq!(&latin1[correction.range.clone()], b"Alias");
    assert_eq!(correction.replacement, "Style/Alias");
}

#[test]
fn closed_source_observers_hold_the_edit() {
    for observer in [
        "p __FILE__",
        "p __dir__",
        "p $0",
        "p $PROGRAM_NAME",
        "p caller",
        "p caller_locations",
        "p method(:p).source_location",
        "p SCRIPT_LINES__",
    ] {
        let source = format!("# rubocop:disable Alias\n{observer}\n");
        assert_held(&source, &[(1, 19)]);
    }
    assert_held(
        "# rubocop:disable Alias\np DATA.read\n__END__\n# rubocop:disable Alias\n",
        &[(1, 19)],
    );
    // An ordinary program is corrected.
    assert_correction(
        &DepartmentName,
        "# rubocop:disable Alias\np 1\n",
        "# rubocop:disable Style/Alias\np 1\n",
    );
}

#[test]
fn qualification_preserves_suppression_and_opt_in_exactly() {
    // The edit is emitted only when the qualified name selects the same
    // registered cops as the original token, for suppression and opt-in.
    for (old, new, same) in [
        ("Alias", "Style/Alias", true),
        (
            "FrozenStringLiteralComment",
            "Style/FrozenStringLiteralComment",
            true,
        ),
        (
            "SingleSpaceBeforeFirstArg",
            "Style/SingleSpaceBeforeFirstArg",
            true,
        ),
        ("MultipleComparison", "Lint/MultipleComparison", false),
        ("alias", "Style/Alias", false),
    ] {
        assert_eq!(selects_same_cops(old, new), same, "{old} -> {new}");
    }
    // End to end: the corrected file suppresses exactly what it did.
    let source = "# rubocop:disable Semicolon\nfoo; bar\n# rubocop:enable Semicolon\nfoo; bar\n";
    let semicolons = |s: &str| -> Vec<u32> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/Semicolon")
            .map(|o| o.line)
            .collect()
    };
    let after =
        "# rubocop:disable Style/Semicolon\nfoo; bar\n# rubocop:enable Style/Semicolon\nfoo; bar\n";
    assert_correction(&DepartmentName, source, after);
    assert_eq!(semicolons(source), [4]);
    assert_eq!(semicolons(after), [4]);
}

#[test]
fn legacy_list_matches_the_pinned_obsoletion_rules() {
    let Some(src) = std::env::var_os("RUBOCOP_SRC") else {
        return;
    };
    let Ok(yaml) =
        std::fs::read_to_string(std::path::Path::new(&src).join("config/obsoletion.yml"))
    else {
        return;
    };
    // ConfigObsoletion's cop rules, in COP_RULE_CLASSES then YAML order.
    let mut names = Vec::new();
    for section in ["renamed", "removed", "split", "extracted"] {
        let mut inside = false;
        for line in yaml.lines() {
            if !line.starts_with(' ') && !line.starts_with('#') && !line.is_empty() {
                inside = line == format!("{section}:");
                continue;
            }
            if inside
                && let Some(key) = line.strip_prefix("  ")
                && !key.starts_with(' ')
                && let Some((name, _)) = key.split_once(':')
            {
                names.push(name.to_owned());
            }
        }
    }
    assert_eq!(names, LEGACY_COP_NAMES);
}
