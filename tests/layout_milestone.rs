//! v0.3 integration contract, exercised through the real CLI.

use std::{
    collections::HashSet,
    io::Write as _,
    path::Path,
    process::{Command, Output, Stdio},
};

use rustocop::cops::Registry;

fn project(source: &[u8], config: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    std::fs::write(dir.path().join("x.rb"), source).unwrap();
    std::fs::write(dir.path().join("rustocop.toml"), config).unwrap();
    dir
}

fn cli(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn summary(output: &Output, expected: &str) {
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).lines().last(),
        Some(expected),
        "{output:?}"
    );
}

// Correction examples ported from the corresponding RuboCop Layout specs at
// 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). These intentionally
// isolate nonoverlapping corrections; mixed interactions are tested below.
const CANONICAL: &[(&str, &str, &str)] = &[
    ("TrailingWhitespace", "foo  \n", "foo\n"),
    ("TrailingEmptyLines", "foo\n\n", "foo\n"),
    ("SpaceAfterComma", "foo(1,2)\n", "foo(1, 2)\n"),
    ("SpaceBeforeComma", "foo(1 , 2)\n", "foo(1, 2)\n"),
    ("SpaceAfterSemicolon", "foo;bar\n", "foo; bar\n"),
    ("SpaceAroundOperators", "foo+bar\n", "foo + bar\n"),
    (
        "SpaceAroundEqualsInParameterDefault",
        "def foo(bar=1)\nend\n",
        "def foo(bar = 1)\nend\n",
    ),
    ("SpaceInsideParens", "foo( 1 )\n", "foo(1)\n"),
    ("SpaceInsideArrayLiteralBrackets", "[ 1 ]\n", "[1]\n"),
    ("SpaceInsideReferenceBrackets", "foo[ 1 ]\n", "foo[1]\n"),
    ("SpaceInsideHashLiteralBraces", "{foo: 1}\n", "{ foo: 1 }\n"),
    ("SpaceInsideBlockBraces", "foo {bar}\n", "foo { bar }\n"),
    (
        "IndentationStyle",
        "def foo\n\tbar\nend\n",
        "def foo\n  bar\nend\n",
    ),
    (
        "IndentationWidth",
        "def foo\n    bar\nend\n",
        "def foo\n  bar\nend\n",
    ),
    (
        "EndAlignment",
        "if foo\n  bar\n  end\n",
        "if foo\n  bar\nend\n",
    ),
    ("EmptyLines", "foo\n\n\nbar\n", "foo\n\nbar\n"),
    (
        "EmptyLinesAroundClassBody",
        "class Foo\n\nend\n",
        "class Foo\nend\n",
    ),
    (
        "EmptyLinesAroundModuleBody",
        "module Foo\n\nend\n",
        "module Foo\nend\n",
    ),
    (
        "EmptyLinesAroundMethodBody",
        "def foo\n\nend\n",
        "def foo\nend\n",
    ),
    (
        "EmptyLineBetweenDefs",
        "def foo\nend\ndef bar\nend\n",
        "def foo\nend\n\ndef bar\nend\n",
    ),
];

#[test]
fn exactly_the_authorized_layout_cops_ship_enabled_with_safe_fixes() {
    let registry = Registry::new();
    let layout: Vec<_> = registry
        .cops()
        .iter()
        .filter(|c| c.name().starts_with("Layout/"))
        .collect();
    assert_eq!(layout.len(), 100);
    let expected: HashSet<_> = CANONICAL
        .iter()
        .map(|(name, _, _)| format!("Layout/{name}"))
        .chain([
            "Layout/LineLength".into(),
            "Layout/AccessModifierIndentation".into(),
            "Layout/AssignmentIndentation".into(),
            "Layout/ArgumentAlignment".into(),
            "Layout/ArrayAlignment".into(),
            "Layout/HashAlignment".into(),
            "Layout/ParameterAlignment".into(),
            "Layout/FirstParameterIndentation".into(),
            "Layout/BeginEndAlignment".into(),
            "Layout/DefEndAlignment".into(),
            "Layout/BlockAlignment".into(),
            "Layout/ClosingParenthesisIndentation".into(),
            "Layout/FirstArgumentIndentation".into(),
            "Layout/FirstArrayElementIndentation".into(),
            "Layout/FirstHashElementIndentation".into(),
            "Layout/CaseIndentation".into(),
            "Layout/ElseAlignment".into(),
            "Layout/RescueEnsureAlignment".into(),
            "Layout/HeredocIndentation".into(),
            "Layout/ClosingHeredocIndentation".into(),
            "Layout/CommentIndentation".into(),
            "Layout/InitialIndentation".into(),
            "Layout/HeredocArgumentClosingParenthesis".into(),
            "Layout/IndentationConsistency".into(),
            "Layout/MultilineMethodCallIndentation".into(),
            "Layout/MultilineOperationIndentation".into(),
            "Layout/DotPosition".into(),
            "Layout/ConditionPosition".into(),
            "Layout/MultilineAssignmentLayout".into(),
            "Layout/SpaceBeforeComment".into(),
            "Layout/LeadingCommentSpace".into(),
            "Layout/EmptyComment".into(),
            "Layout/LeadingEmptyLines".into(),
            "Layout/EndOfLine".into(),
            "Layout/EmptyLineAfterMagicComment".into(),
            "Layout/EmptyLineAfterGuardClause".into(),
            "Layout/EmptyLineAfterMultilineCondition".into(),
            "Layout/EmptyLinesAfterModuleInclusion".into(),
            "Layout/EmptyLinesAroundAttributeAccessor".into(),
            "Layout/EmptyLinesAroundAccessModifier".into(),
            "Layout/EmptyLinesAroundBeginBody".into(),
            "Layout/EmptyLinesAroundBlockBody".into(),
            "Layout/EmptyLinesAroundExceptionHandlingKeywords".into(),
            "Layout/EmptyLinesAroundArguments".into(),
            "Layout/ExtraSpacing".into(),
            "Layout/SpaceBeforeFirstArg".into(),
            "Layout/SpaceBeforeSemicolon".into(),
            "Layout/FirstArrayElementLineBreak".into(),
            "Layout/FirstHashElementLineBreak".into(),
            "Layout/FirstMethodArgumentLineBreak".into(),
            "Layout/FirstMethodParameterLineBreak".into(),
            "Layout/MultilineArrayLineBreaks".into(),
            "Layout/MultilineHashKeyLineBreaks".into(),
            "Layout/MultilineMethodArgumentLineBreaks".into(),
            "Layout/MultilineMethodParameterLineBreaks".into(),
            "Layout/MultilineArrayBraceLayout".into(),
            "Layout/MultilineHashBraceLayout".into(),
            "Layout/MultilineMethodCallBraceLayout".into(),
            "Layout/MultilineMethodDefinitionBraceLayout".into(),
            "Layout/MultilineBlockLayout".into(),
            "Layout/BlockEndNewline".into(),
            "Layout/SingleLineBlockChain".into(),
            "Layout/RedundantLineBreak".into(),
            "Layout/LineContinuationLeadingSpace".into(),
            "Layout/LineContinuationSpacing".into(),
            "Layout/LineEndStringConcatenationIndentation".into(),
            "Layout/SpaceAfterColon".into(),
            "Layout/SpaceAfterMethodName".into(),
            "Layout/SpaceAfterNot".into(),
            "Layout/SpaceAroundKeyword".into(),
            "Layout/SpaceAroundMethodCallOperator".into(),
            "Layout/SpaceAroundBlockParameters".into(),
            "Layout/SpaceInLambdaLiteral".into(),
            "Layout/SpaceBeforeBlockBraces".into(),
            "Layout/SpaceBeforeBrackets".into(),
            "Layout/SpaceInsideArrayPercentLiteral".into(),
            "Layout/SpaceInsidePercentLiteralDelimiters".into(),
            "Layout/SpaceInsideRangeLiteral".into(),
            "Layout/SpaceInsideStringInterpolation".into(),
            "Layout/ClassStructure".into(),
        ])
        .collect();
    assert_eq!(
        layout
            .iter()
            .map(|c| c.name().to_owned())
            .collect::<HashSet<_>>(),
        expected
    );
    for cop in layout {
        // HeredocArgumentClosingParenthesis, MultilineAssignmentLayout,
        // EmptyLineAfterMultilineCondition, the first-element line breaks,
        // the multi-line item line breaks, SingleLineBlockChain,
        // RedundantLineBreak and ClassStructure are opt-in upstream; all
        // but the first are never enabled by a department selection.
        let opt_in = matches!(
            cop.name(),
            "Layout/HeredocArgumentClosingParenthesis"
                | "Layout/MultilineAssignmentLayout"
                | "Layout/EmptyLineAfterMultilineCondition"
                | "Layout/FirstArrayElementLineBreak"
                | "Layout/FirstHashElementLineBreak"
                | "Layout/FirstMethodArgumentLineBreak"
                | "Layout/FirstMethodParameterLineBreak"
                | "Layout/MultilineArrayLineBreaks"
                | "Layout/MultilineHashKeyLineBreaks"
                | "Layout/MultilineMethodArgumentLineBreaks"
                | "Layout/MultilineMethodParameterLineBreaks"
                | "Layout/SingleLineBlockChain"
                | "Layout/RedundantLineBreak"
                | "Layout/ClassStructure"
        );
        assert_eq!(cop.default_enabled(), !opt_in, "{}", cop.name());
        assert_eq!(
            cop.department_selection_enables(),
            !matches!(
                cop.name(),
                "Layout/MultilineAssignmentLayout"
                    | "Layout/EmptyLineAfterMultilineCondition"
                    | "Layout/FirstArrayElementLineBreak"
                    | "Layout/FirstHashElementLineBreak"
                    | "Layout/FirstMethodArgumentLineBreak"
                    | "Layout/FirstMethodParameterLineBreak"
                    | "Layout/MultilineArrayLineBreaks"
                    | "Layout/MultilineHashKeyLineBreaks"
                    | "Layout/MultilineMethodArgumentLineBreaks"
                    | "Layout/MultilineMethodParameterLineBreaks"
                    | "Layout/SingleLineBlockChain"
                    | "Layout/RedundantLineBreak"
                    | "Layout/ClassStructure"
            ),
            "{}",
            cop.name()
        );
        // EndOfLine is detection-only upstream; the ClassStructure reorder
        // is unsafe upstream.
        assert_eq!(
            cop.safe_fixes(),
            !matches!(cop.name(), "Layout/EndOfLine" | "Layout/ClassStructure")
        );
        assert_eq!(
            cop.max_setting(),
            (cop.name() == "Layout/LineLength").then_some("max")
        );
    }
}

#[test]
fn all_twenty_attributed_canonical_corrections_stabilize_in_one_pass() {
    for (name, before, after) in CANONICAL {
        let cop = format!("Layout/{name}");
        let dir = project(before.as_bytes(), "");
        let path = dir.path().join("x.rb");
        let plain = cli(dir.path(), &["--only", &cop, "x.rb"]);
        assert_eq!(plain.status.code(), Some(1), "{cop}: {plain:?}");
        assert_eq!(std::fs::read(&path).unwrap(), before.as_bytes());
        let fixed = cli(dir.path(), &["--fix", "--only", &cop, "x.rb"]);
        assert_eq!(fixed.status.code(), Some(0), "{cop}: {fixed:?}");
        assert_eq!(std::fs::read(&path).unwrap(), after.as_bytes(), "{cop}");
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let stable = cli(dir.path(), &["--fix", "--only", &cop, "x.rb"]);
        assert_eq!(stable.status.code(), Some(0), "{cop}: {stable:?}");
        summary(
            &stable,
            "1 file inspected, 0 corrections applied, 0 offenses remaining",
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified
        );
    }
}

#[test]
fn all_twenty_layout_cops_and_style_run_together_through_the_cli() {
    // --only Style also selects opt-in Copyright and DocumentationMethod.
    // Supply notices and method docs rather than exempting requested rules
    // from this correction gate; CANONICAL's definitions are all top-level.
    let notice = "# Copyright 2026 Test\n";
    let dir = project(format!("{notice}puts 'style'  \n").as_bytes(), "");
    for (name, before, _) in CANONICAL {
        std::fs::write(
            // Fixture filenames must not introduce Naming/FileName findings.
            dir.path().join(format!("{}.rb", name.to_lowercase())),
            format!(
                "{notice}{}",
                before.replace("def ", "# Method documentation.\ndef ")
            ),
        )
        .unwrap();
    }
    let plain = cli(dir.path(), &["."]);
    assert_eq!(plain.status.code(), Some(1), "{plain:?}");
    let text = String::from_utf8_lossy(&plain.stdout);
    for (name, _, _) in CANONICAL {
        assert!(text.contains(&format!("(Layout/{name})")), "{name}: {text}");
    }
    assert!(text.contains("(Style/StringLiterals)"));
    assert!(text.contains("(Style/IfUnlessModifier)"));
    assert!(text.contains("(Style/EmptyMethod)"));
    // The empty-class layout fixture now also has its independent Lint finding.
    assert!(text.contains("(Lint/EmptyClass)"));
    summary(&plain, "21 files inspected, 35 offenses detected");
    let mut seen = HashSet::new();
    let paths: Vec<_> = CANONICAL
        .iter()
        .map(|(name, _, _)| dir.path().join(format!("{}.rb", name.to_lowercase())))
        .chain([dir.path().join("x.rb")])
        .collect();
    for pass in 1..=5 {
        let snapshot: Vec<_> = paths
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect();
        assert!(seen.insert(snapshot.clone()), "cycle on pass {pass}");
        let fixed = cli(
            dir.path(),
            &[
                "--fix",
                "--only",
                "Layout,Style",
                "--except",
                // This v0.3 correction gate excludes the new detection-only cop.
                "Style/FrozenStringLiteralComment,Style/IfUnlessModifier",
                ".",
            ],
        );
        assert!(matches!(fixed.status.code(), Some(0 | 1)), "{fixed:?}");
        assert!(fixed.stderr.is_empty(), "{fixed:?}");
        let after: Vec<_> = paths
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect();
        if after == snapshot {
            // Style intentionally selects opt-in TopLevelMethodDefinition
            // and MissingElse (fixed EmptyElse=both gives it no edit). Stable
            // detection-only findings are not correction cycles.
            let report = String::from_utf8_lossy(&fixed.stdout);
            let findings: Vec<_> = report
                .lines()
                .filter(|line| line.contains(": C:"))
                .collect();
            assert_eq!(findings.len(), 7, "{fixed:?}");
            assert_eq!(
                findings
                    .iter()
                    .filter(|line| line.ends_with("(Style/MissingElse)"))
                    .count(),
                1
            );
            assert!(
                findings
                    .iter()
                    .all(|line| line.ends_with("(Style/TopLevelMethodDefinition)")
                        || line.ends_with("(Style/MissingElse)")),
                "{fixed:?}"
            );
            break;
        }
        assert!(pass < 5, "{fixed:?}");
    }
    let stable = cli(
        dir.path(),
        &[
            "--fix",
            "--only",
            "Layout,Style",
            "--except",
            "Style/FrozenStringLiteralComment,Style/IfUnlessModifier",
            ".",
        ],
    );
    assert_eq!(stable.status.code(), Some(1), "{stable:?}");
    summary(
        &stable,
        "21 files inspected, 0 corrections applied, 7 offenses remaining",
    );
}

#[test]
fn mixed_selection_config_and_count_contract() {
    for (config, args, after, count) in [
        ("", vec![], "puts \"x\"\n", 2),
        ("", vec!["--only", "Layout"], "puts 'x'\n", 1),
        (
            "",
            vec![
                "--only",
                "Layout,Style/StringLiterals",
                "--except",
                "Layout",
            ],
            "puts \"x\"  \n",
            1,
        ),
        (
            "[cops.\"Layout\"]\nenabled = false\n",
            vec![],
            "puts \"x\"  \n",
            1,
        ),
        (
            "[cops.\"Layout/TrailingWhitespace\"]\nenabled = false\n",
            vec![],
            "puts \"x\"  \n",
            1,
        ),
        (
            "[cops.\"Layout\"]\nenabled = false\n",
            vec!["--only", "Layout,Style/StringLiterals"],
            "puts \"x\"\n",
            2,
        ),
        (
            "[cops.\"Style\"]\nenabled = false\n",
            vec![],
            "puts 'x'\n",
            1,
        ),
        (
            "[cops.\"Layout/TrailingWhitespace\"]\nexclude = [\"x.rb\"]\n",
            vec!["--only", "Layout,Style/StringLiterals"],
            "puts \"x\"  \n",
            1,
        ),
    ] {
        let dir = project(b"puts 'x'  \n", config);
        let mut args = args;
        args.extend(["--fix", "x.rb"]);
        let fixed = cli(dir.path(), &args);
        assert_eq!(fixed.status.code(), Some(0), "{config} {args:?}: {fixed:?}");
        assert_eq!(
            std::fs::read(dir.path().join("x.rb")).unwrap(),
            after.as_bytes()
        );
        summary(
            &fixed,
            &format!(
                "1 file inspected, {count} correction{} applied, 0 offenses remaining",
                if count == 1 { "" } else { "s" }
            ),
        );
    }
    let dir = project(b"puts 'x'  \n", "exclude = [\"x.rb\"]\n");
    let fixed = cli(dir.path(), &["--fix", "x.rb"]);
    assert_eq!(fixed.status.code(), Some(0));
    summary(
        &fixed,
        "0 files inspected, 0 corrections applied, 0 offenses remaining",
    );
    assert_eq!(
        std::fs::read(dir.path().join("x.rb")).unwrap(),
        b"puts 'x'  \n"
    );
}

#[test]
fn department_exclusions_are_config_errors_before_any_lint_or_write() {
    for department in ["Layout", "Style"] {
        for exclude in ["[\"x.rb\"]", "[]"] {
            let config = format!("[cops.\"{department}\"]\nexclude = {exclude}\n");
            for args in [
                vec!["x.rb"],
                vec!["--fix", "."],
                vec!["--fix", "--only", "Layout,Style", "."],
            ] {
                let before = b"puts 'x'  \n";
                let dir = project(before, &config);
                let paths = [dir.path().join("x.rb"), dir.path().join("other.rb")];
                std::fs::write(&paths[1], before).unwrap();
                let metadata = paths.each_ref().map(|p| std::fs::metadata(p).unwrap());
                let output = cli(dir.path(), &args);
                assert_eq!(
                    output.status.code(),
                    Some(2),
                    "{config} {args:?}: {output:?}"
                );
                assert!(output.stdout.is_empty(), "{output:?}");
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains(&format!(
                        "`exclude` is not a valid setting for department `{department}`"
                    )),
                    "{output:?}"
                );
                for (path, original) in paths.iter().zip(&metadata) {
                    assert_eq!(std::fs::read(path).unwrap(), before);
                    let current = std::fs::metadata(path).unwrap();
                    assert_eq!(current.modified().unwrap(), original.modified().unwrap());
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        assert_eq!(current.ino(), original.ino());
                    }
                }
                assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 4);
            }
        }
    }
}

#[test]
fn both_directive_prefixes_protect_layout_and_style_bytes_together() {
    for prefix in ["rustocop", "rubocop"] {
        let disabled = format!(
            "# {prefix}:disable Layout, Style\nputs( [ 'disabled' ,2+3 ] );\n# {prefix}:enable Layout, Style\n"
        );
        let before = disabled.clone() + "puts 'enabled'  \n";
        let after = disabled + "puts \"enabled\"\n";
        let dir = project(before.as_bytes(), "");
        let fixed = cli(dir.path(), &["--fix", "x.rb"]);
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        summary(
            &fixed,
            "1 file inspected, 2 corrections applied, 0 offenses remaining",
        );
        assert_eq!(
            std::fs::read(dir.path().join("x.rb")).unwrap(),
            after.as_bytes()
        );
    }
}

fn runtime(source: &[u8]) -> Vec<u8> {
    // Execute only these controlled synthetic fixtures, never public corpus code.
    let mut child = Command::new("ruby")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success(), "{result:?}");
    result.stdout
}

#[test]
fn mixed_layout_and_style_converge_without_cycles_or_runtime_changes() {
    let before = include_bytes!("fixtures/layout_milestone/mixed.before.rb");
    let after = include_bytes!("fixtures/layout_milestone/mixed.after.rb");
    let expected_output = runtime(before);
    // This v0.3 correction fixture predates GuardClause and IfUnlessModifier.
    // Their corrections are tested separately.
    // Documentation is noncorrectable and was added after this fixture too.
    // MapJoin, SymbolProc and NumericPredicate are unsafe/detection-only and
    // must not block this old fix gate.
    // RescueStandardError adds an explicit class in v0.4; keep this historical
    // byte fixture on its v0.3 correcting set.
    // Naming/RescuedExceptionsVariableName adds safe renames in v0.5;
    // The Naming rename cops separately test current default composition. Keep this historical
    // convergence assertion about the v0.3 correcting set.
    // SpaceAroundBlockParameters and
    // SpaceBeforeBlockBraces (v1.0) would add blanks around the fixture's
    // `map{ |entry|`; tests/block_bracket_spacing.rs covers them.
    // SpaceInsideStringInterpolation (v1.0) would remove the
    // blanks of the fixture's `#{ "message".upcase }`;
    // tests/literal_spacing.rs covers it.
    let dir = project(
        before,
        "[cops.\"Layout/SpaceInsideStringInterpolation\"]\nenabled = false\n[cops.\"Layout/SpaceAroundBlockParameters\"]\nenabled = false\n[cops.\"Layout/SpaceBeforeBlockBraces\"]\nenabled = false\n[cops.\"Style/GuardClause\"]\nenabled = false\n[cops.\"Style/IfUnlessModifier\"]\nenabled = false\n[cops.\"Style/Documentation\"]\nenabled = false\n[cops.\"Style/MapJoin\"]\nenabled = false\n[cops.\"Style/SymbolProc\"]\nenabled = false\n[cops.\"Style/NumericPredicate\"]\nenabled = false\n[cops.\"Style/RescueStandardError\"]\nenabled = false\n[cops.\"Naming/RescuedExceptionsVariableName\"]\nenabled = false\n",
    );
    let path = dir.path().join("x.rb");
    let plain = cli(dir.path(), &["x.rb"]);
    assert_eq!(plain.status.code(), Some(1), "{plain:?}");
    let text = String::from_utf8_lossy(&plain.stdout);
    for name in [
        "StringLiterals",
        "StringLiteralsInInterpolation",
        "HashSyntax",
        "RedundantReturn",
        "Semicolon",
        "RedundantParentheses",
        "SignalException",
    ] {
        assert!(text.contains(&format!("(Style/{name})")), "{name}: {text}");
    }
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let mut seen = HashSet::new();
    let mut converged = false;
    for pass in 1..=5 {
        let original = std::fs::read(&path).unwrap();
        assert!(seen.insert(original), "cycle before pass {pass}");
        let fixed = cli(dir.path(), &["--fix", "x.rb"]);
        assert!(
            matches!(fixed.status.code(), Some(0 | 1)),
            "pass {pass}: {fixed:?}"
        );
        assert!(fixed.stderr.is_empty(), "{fixed:?}");
        let actual = std::fs::read(&path).unwrap();
        assert_eq!(runtime(&actual), expected_output, "pass {pass}");
        if fixed.status.success() {
            converged = true;
            break;
        }
    }
    assert!(
        converged,
        "{}",
        String::from_utf8_lossy(&std::fs::read(&path).unwrap())
    );
    assert_eq!(std::fs::read(&path).unwrap(), after);
    let stable = cli(dir.path(), &["--fix", "x.rb"]);
    assert_eq!(stable.status.code(), Some(0), "{stable:?}");
    summary(
        &stable,
        "1 file inspected, 0 corrections applied, 0 offenses remaining",
    );
}

#[cfg(unix)]
#[test]
fn mixed_cli_rewrites_are_atomic_and_clean_files_are_not_rewritten() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = project(b"puts 'x'  \n", "");
    let path = dir.path().join("x.rb");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o750)).unwrap();
    symlink("x.rb", dir.path().join("alias.rb")).unwrap();
    // An open descriptor must retain the old inode, never observe an in-place edit.
    let old_file = std::fs::File::open(&path).unwrap();
    let original_inode = old_file.metadata().unwrap().ino();
    let fixed = cli(dir.path(), &["--fix", "x.rb", "alias.rb"]);
    assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
    summary(
        &fixed,
        "2 files inspected, 2 corrections applied, 0 offenses remaining",
    );
    assert_ne!(std::fs::metadata(&path).unwrap().ino(), original_inode);
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o750
    );
    assert!(
        std::fs::symlink_metadata(dir.path().join("alias.rb"))
            .unwrap()
            .is_symlink()
    );
    use std::io::Read as _;
    let mut old_bytes = Vec::new();
    (&old_file).read_to_end(&mut old_bytes).unwrap();
    assert_eq!(old_bytes, b"puts 'x'  \n");
    assert_eq!(std::fs::read(&path).unwrap(), b"puts \"x\"\n");
    let metadata = std::fs::metadata(&path).unwrap();
    let clean = cli(dir.path(), &["--fix", "x.rb"]);
    assert_eq!(clean.status.code(), Some(0), "{clean:?}");
    summary(
        &clean,
        "1 file inspected, 0 corrections applied, 0 offenses remaining",
    );
    assert_eq!(std::fs::metadata(&path).unwrap().ino(), metadata.ino());
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        metadata.modified().unwrap()
    );
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        4,
        "no temporary files remain"
    );
}

#[test]
fn mixed_errors_and_detection_only_line_length_keep_exit_and_write_contracts() {
    let long = format!("puts \"{}\"\n", "a".repeat(120));
    let dir = project(long.as_bytes(), "");
    let fixed = cli(dir.path(), &["--fix", "x.rb"]);
    assert_eq!(fixed.status.code(), Some(1));
    summary(
        &fixed,
        "1 file inspected, 0 corrections applied, 1 offense remaining",
    );
    assert_eq!(
        std::fs::read(dir.path().join("x.rb")).unwrap(),
        long.as_bytes()
    );
    let dir = project(b"puts 'x'  \n", "");
    std::fs::write(dir.path().join("broken.rb"), b"def broken(\n").unwrap();
    let fixed = cli(dir.path(), &["--fix", "."]);
    assert_eq!(fixed.status.code(), Some(2));
    summary(
        &fixed,
        "2 files inspected, 2 corrections applied, 0 offenses remaining, 3 errors",
    );
    assert_eq!(
        std::fs::read(dir.path().join("x.rb")).unwrap(),
        b"puts \"x\"\n"
    );
    assert_eq!(
        std::fs::read(dir.path().join("broken.rb")).unwrap(),
        b"def broken(\n"
    );
}
