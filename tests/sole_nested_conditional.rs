//! SoleNestedConditional's default-on CLI and safe correction contract.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

const COP: &str = "Style/SoleNestedConditional";
const BAD: &str = "if first\n  if second\n    work\n  end\nend\n";
const MESSAGE: &str = "Consider merging nested conditions into outer `if` conditions.";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    fs::write(dir.path().join("lib/a.rb"), BAD).unwrap();
    dir
}

fn run(root: &Path, args: &[&str], status: Option<i32>, count: usize) {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    if let Some(status) = status {
        assert_eq!(output.status.code(), Some(status), "{output:?}");
    } else {
        assert_ne!(output.status.code(), Some(2), "{output:?}");
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.lines().filter(|line| line.contains(COP)).count(),
        count,
        "{output:?}"
    );
    if count > 0 {
        assert!(
            stdout.contains("Consider merging nested conditions into outer `"),
            "{stdout}"
        );
    }
}

#[test]
fn defaults_selectors_and_config_overrides() {
    let dir = project();
    for (config, selectors, count) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Style", "--except", COP], 0),
        (String::new(), vec!["--only", COP, "--except", COP], 0),
        (format!("[cops.\"{COP}\"]\nenabled = false\n"), vec![], 0),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["--only", COP],
            1,
        ),
        (
            format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
            vec![],
            1,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = selectors;
        args.push("lib/a.rb");
        run(dir.path(), &args, None, count);
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            BAD
        );
    }
    let registry = rustocop::cops::Registry::new();
    let cop = registry
        .cops()
        .iter()
        .find(|cop| cop.name() == COP)
        .unwrap();
    assert!(cop.default_enabled());
    assert!(cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none());
    assert!(cop.max_setting().is_none());
}

#[test]
fn config_relative_exclusions_and_keyword_directives_apply_without_writes() {
    let dir = project();
    let file = dir.path().join("lib/a.rb");
    for config in [
        "exclude = [\"lib/a.rb\"]\n".to_owned(),
        format!("[cops.\"{COP}\"]\nexclude = [\"lib/a.rb\"]\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        run(
            &dir.path().join("lib"),
            &["--only", COP, "--fix", "a.rb"],
            Some(0),
            0,
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), BAD);
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for (source, count) in [
            (format!("# {prefix}:disable {COP}\n{BAD}"), 0),
            (format!("# {prefix}:disable-next {COP}\n{BAD}"), 0),
            (format!("# {prefix}:disable Style\n{BAD}"), 0),
            (
                BAD.replace("if second", &format!("if second # {prefix}:disable {COP}")),
                0,
            ),
        ] {
            fs::write(&file, &source).unwrap();
            run(
                dir.path(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(i32::from(count > 0)),
                count,
            );
            assert_eq!(fs::read_to_string(&file).unwrap(), source);
        }
        // Only the inner keyword is highlighted, so, as in RuboCop, its
        // correction reaches the disabled body line.
        let source = BAD.replace("work", &format!("work # {prefix}:disable {COP}"));
        fs::write(&file, &source).unwrap();
        run(
            dir.path(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(0),
            0,
        );
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            format!("if first && second\n    work # {prefix}:disable {COP}\n  end\n")
        );
    }
}

#[test]
fn plain_lint_preserves_bytes_and_line_values_for_control_flow() {
    let dir = project();
    let file = dir.path().join("lib/a.rb");
    for (source, status, count) in [
        (BAD, 1, 1),
        ("if first && second\n  work\nend\n", 0, 0),
        ("if var = foo\n  work if var\nend\n", 0, 0),
        ("if first\n  work if second\nelse\n  fallback\nend\n", 0, 0),
        ("if first\n", 2, 0),
        (
            "if foo(<<~TEXT)\n  hello\nTEXT\n  if bar\n    work\n  end\nend\n",
            0,
            0,
        ),
        (
            "def f\n  if first\n    if second\n      return __LINE__\n    end\n  end\nend\n",
            1,
            1,
        ),
        (
            "each do\n  if first\n    if second\n      next __LINE__\n    end\n  end\nend\n",
            1,
            1,
        ),
        (
            "\u{feff}# encoding: UTF-8\r\nif first\r\n  # branch\r\n  if second\r\n    __LINE__ # untouched\r\n  end\r\nend\r\n__END__\r\ndata\r\n",
            1,
            1,
        ),
        (
            "begin\n  if first\n    if second\n      work\n    end\n  end\nrescue\n  recover\nensure\n  cleanup\nend\n",
            1,
            1,
        ),
        (
            "if first\n  if second\n    <<~TEXT\n      literal # untouched\n    TEXT\n  end\nend\n",
            1,
            1,
        ),
    ] {
        fs::write(&file, source).unwrap();
        run(
            dir.path(),
            &["--only", COP, "lib/a.rb"],
            Some(status),
            count,
        );
        assert_eq!(fs::read(&file).unwrap(), source.as_bytes());
    }
}

#[test]
fn stdin_honors_excludes_and_directives_and_reports_exact_positions() {
    let dir = project();
    for (config, source, count) in [
        (String::new(), BAD.to_owned(), 1),
        (
            format!("[cops.\"{COP}\"]\nexclude = [\"lib/a.rb\"]\n"),
            BAD.to_owned(),
            0,
        ),
        (String::new(), format!("# rubocop:disable {COP}\n{BAD}"), 0),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path().join("lib"))
            .args(["--only", COP, "--stdin", "a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(count), "{output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(stdout.contains(COP), count == 1, "{stdout}");
        if count == 1 {
            assert!(stdout.contains("a.rb:2:3:"), "{stdout}");
            assert!(stdout.contains(MESSAGE), "{stdout}");
        }
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            BAD
        );
    }
}

fn fix(root: &Path, before: &str, after: &str, selected: &str) {
    let path = root.join("lib/a.rb");
    fs::write(&path, before).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(["--only", selected, "--fix", "--fix-passes", "1", "lib/a.rb"])
        .output()
        .unwrap();
    assert_ne!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(fs::read_to_string(path).unwrap(), after, "{output:?}");
}

#[test]
fn safe_fixes_obey_enablement_and_reenabled_directive_spans() {
    let dir = project();
    let good = "if first && second\n    work\n  end\n";
    fix(dir.path(), BAD, good, COP);
    for prefix in ["rustocop", "rubocop"] {
        fix(
            dir.path(),
            &format!("# {prefix}:disable {COP}\n{BAD}# {prefix}:enable {COP}\n{BAD}"),
            &format!("# {prefix}:disable {COP}\n{BAD}# {prefix}:enable {COP}\n{good}"),
            COP,
        );
    }
    for (config, args) in [
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["--fix", "--fix-passes", "1", "lib/a.rb"],
        ),
        (
            String::new(),
            vec![
                "--only",
                COP,
                "--except",
                COP,
                "--fix",
                "--fix-passes",
                "1",
                "lib/a.rb",
            ],
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), &config).unwrap();
        let path = dir.path().join("lib/a.rb");
        fs::write(&path, BAD).unwrap();
        let time = fs::metadata(&path).unwrap().modified().unwrap();
        run(dir.path(), &args, None, 0);
        assert_eq!(fs::read_to_string(&path).unwrap(), BAD);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
    }
    fs::write(
        dir.path().join("rustocop.toml"),
        format!("[cops.\"{COP}\"]\nenabled = false\n"),
    )
    .unwrap();
    fix(dir.path(), BAD, good, COP); // --only force-enables.
    fs::write(
        dir.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
    )
    .unwrap();
    fs::write(dir.path().join("lib/a.rb"), BAD).unwrap();
    run(
        dir.path(),
        &["--fix", "--fix-passes", "1", "lib/a.rb"],
        None,
        0,
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
        good
    );
}

#[test]
fn real_incompatible_peers_withhold_only_the_declaring_cop() {
    let dir = project();
    let source = "if !true\n  if second\n    work\n  end\nend\n";
    for peer in ["Style/NegatedIf", "Style/NegatedUnless"] {
        let selected = format!("{COP},{peer}");
        let after = if peer == "Style/NegatedIf" {
            source.replacen("if !true", "unless true", 1)
        } else {
            source.into()
        };
        fix(dir.path(), source, &after, &selected);
    }
    // Both negated peers are default-on: full defaults retain diagnostics but
    // do not quietly select an incompatible correction combination.
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, BAD).unwrap();
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    run(dir.path(), &["--fix", "lib/a.rb"], Some(1), 1);
    assert_eq!(fs::read_to_string(&path).unwrap(), BAD);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
    // Reverse declarations: these modifier cops defer to SoleNestedConditional.
    for peer in ["Style/IfUnlessModifier", "Style/IfUnlessModifierOfIfUnless"] {
        let selected = format!("{COP},{peer}");
        fix(
            dir.path(),
            "if a\n  work if b\nend\n",
            "if a && b\n  work\nend\n",
            &selected,
        );
    }
}

#[test]
fn bom_crlf_literal_and_end_payload_bytes_are_preserved_by_real_fix() {
    let dir = project();
    let before = "\u{feff}# encoding: UTF-8\r\nif a\r\n  # branch\r\n  if b\r\n    \"é\\ntext\"\r\n  end\r\nend\r\n__END__\r\ndata\r\n";
    let after = "\u{feff}# encoding: UTF-8\r\n# branch\r\nif a && b\r\n    \"é\\ntext\"\r\n  end\r\n__END__\r\ndata\r\n";
    fix(dir.path(), before, after, COP);
    let path = dir.path().join("lib/a.rb");
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    run(
        dir.path(),
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(0),
        0,
    );
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
    let mut before = b"if a\n  if b\n    work\n  end\nend\n__END__\n".to_vec();
    before.extend_from_slice(b"\xff\x00raw\r\n");
    let mut after = b"if a && b\n    work\n  end\n__END__\n".to_vec();
    after.extend_from_slice(b"\xff\x00raw\r\n");
    fs::write(&path, before).unwrap();
    run(
        dir.path(),
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(0),
        0,
    );
    assert_eq!(fs::read(&path).unwrap(), after);
}

#[test]
fn mixed_style_layout_overlap_is_atomic_deterministic_and_stabilizes() {
    let dir = project();
    let before = "if first\n  if second\n    work('x');\n  end\nend\n";
    let selected = format!(
        "{COP},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout/EndAlignment,Layout/IndentationWidth"
    );
    let first = "if first && second\n    work('x');\n  end\n";
    for _ in 0..2 {
        fix(dir.path(), before, first, &selected);
    }
    let path = dir.path().join("lib/a.rb");
    let mut stable = false;
    for _ in 0..6 {
        let old = fs::read(&path).unwrap();
        run(
            dir.path(),
            &[
                "--only",
                &selected,
                "--fix",
                "--fix-passes",
                "1",
                "lib/a.rb",
            ],
            None,
            0,
        );
        let new = fs::read(&path).unwrap();
        if old == new {
            stable = true;
            break;
        }
    }
    assert!(stable);
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "if first && second\n  work(\"x\")\nend\n"
    );
}

#[test]
fn modifier_policy_and_ruby_version_are_not_public_options() {
    let dir = project();
    for option in [
        "AllowModifier = true",
        "TargetRubyVersion = 3.4",
        "max = 80",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{option}\n"),
        )
        .unwrap();
        run(
            dir.path(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(2),
            0,
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            BAD
        );
    }
}
