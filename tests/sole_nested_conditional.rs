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
            // Only the inner keyword is highlighted, not the whole body.
            (
                BAD.replace("work", &format!("work # {prefix}:disable {COP}")),
                1,
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
    }
}

#[test]
fn fix_preserves_comments_before_predicates_without_writing() {
    let dir = project();
    let file = dir.path().join("lib/a.rb");
    for source in [
        "if true\n  if # keep this explanation\n    true\n    puts :ok\n  end\nend\n",
        "if true\n  puts :ok if # important explanation\n    true\nend\n",
        "if true\n  puts :ok\nend if # important explanation\n  true\n",
        "puts :ok if true if # important explanation\n  true\n",
    ] {
        for newline in ["\n", "\r\n"] {
            let source = source.replace('\n', newline);
            fs::write(&file, &source).unwrap();
            let modified = fs::metadata(&file).unwrap().modified().unwrap();
            run(
                dir.path(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(1),
                1,
            );
            assert_eq!(fs::read(&file).unwrap(), source.as_bytes());
            assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), modified);
        }
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
fn unsafe_source_envelopes_stay_byte_and_mtime_identical() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for source in [
        b"# coding: ISO-8859-1\nif a\n  if b\n    puts '\xe9'\n  end\nend\n".as_slice(),
        b"if a\n  if b\n    return __LINE__\n  end\nend\n".as_slice(),
        b"if a\n  if b\n    work\n  end\nend\np __LINE__\n".as_slice(),
        b"if a\n  if b\n    DATA.read\n  end\nend\n__END__\npayload\n".as_slice(),
        b"if a\n  if b\n    File.read(__FILE__)\n  end\nend\n".as_slice(),
        b"if a\n  if b\n    <<~TEXT\nhello\nTEXT\n  end\nend\n".as_slice(),
        b"if a\n  work unless b\nend\n".as_slice(),
        b"if a\n  x = 1 if x\nend\n".as_slice(),
    ] {
        fs::write(&path, source).unwrap();
        let time = fs::metadata(&path).unwrap().modified().unwrap();
        run(
            dir.path(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1),
            1,
        );
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
    }
    for (source, status) in [
        ("if first\n", 2),
        ("if first && second\n  work\nend\n", 0),
        ("if first\n  work if second\nelse\n  fallback\nend\n", 0),
    ] {
        fs::write(&path, source).unwrap();
        let time = fs::metadata(&path).unwrap().modified().unwrap();
        run(
            dir.path(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(status),
            0,
        );
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
    }
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
fn every_pinned_unknown_negation_correction_keeps_exact_bytes_and_mtime() {
    // RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/sole_nested_conditional_pinned.json")).unwrap();
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let mut count = 0;
    for c in cases
        .iter()
        .filter(|c| c["correction"].is_string() && c["source"].as_str().unwrap().contains("unless"))
    {
        count += 1;
        let source = c["source"].as_str().unwrap();
        fs::write(&path, source).unwrap();
        let time = fs::metadata(&path).unwrap().modified().unwrap();
        run(
            dir.path(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(1),
            c["positions"].as_array().unwrap().len(),
        );
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
    }
    assert_eq!(count, 24);
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
fn controlled_ruby_values_scope_order_guards_rescue_and_line_observations() {
    // Only this controlled fixture is executed, never corpus or application code.
    let source = r#"$trace = []
def guard(x)
  $trace << x
  x
end
def line(a)
  if a
    if a
      __LINE__
    end
  end
end
def choose(a, b)
  if guard(a)
    if guard(b)
      "value\ntext"
    end
  end
end
def flow(a, b)
  begin
    if a
      return :early if b
    end
    :late
  rescue
    :rescued
  ensure
    $trace << :ensure
  end
end
def precedence(a, b, c)
  if guard(a)
    if guard(b) and guard(c)
      :precedence
    end
  end
end
def yielding(item, enabled)
  if yield item
    if guard(enabled)
      :yielded
    end
  end
end
def exceptional(a, b)
  if a
    if b
      begin
        $trace << :work
        raise "failure"
      rescue RuntimeError
        :recovered
      ensure
        $trace << :cleanup
      end
    end
  end
end
def assignment(a)
  if value = a
    if value && other = :assigned
      [value, other]
    end
  end
end
def scope(a)
  if a
    x = 1 if x
  end
end
p [false, nil, true].repeated_permutation(2).map { |a, b| choose(a, b) }
p [flow(true, true), flow(true, false), flow(false, true)]
p [false, nil, true].repeated_permutation(3).map { |a, b, c| precedence(a, b, c) }.count(:precedence)
p [yielding(false, true) { |x| $trace << [:yield, x]; x }, yielding(true, false) { |x| $trace << [:yield, x]; x }]
p [exceptional(false, true), exceptional(true, false), exceptional(true, true)]
p [assignment(false), assignment(true), scope(true), line(true)]
p 2.times.map { |i|
  if i == 0
    next :skipped if true
  end
  if i == 1
    break :stopped if true
  end
}
class OddTruth
  def !; $trace << :bad_dispatch; true; end
end
if true
  :kept unless OddTruth.new
end
p $trace
"#;
    let dir = project();
    let before_path = dir.path().join("before.rb");
    fs::write(&before_path, source).unwrap();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    run(
        dir.path(),
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(1),
        3,
    );
    let after = fs::read_to_string(&path).unwrap();
    assert_ne!(after, source);
    assert!(after.contains("if guard(a) && guard(b)"));
    assert!(after.contains("x = 1 if x") && after.contains("__LINE__"));
    let mut rubies = vec!["ruby".to_owned()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        rubies.push(ruby);
    }
    for ruby in rubies {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        let before = Command::new(&ruby).arg(&before_path).output().unwrap();
        let after = Command::new(&ruby).arg(&path).output().unwrap();
        assert!(
            before.status.success() && after.status.success(),
            "{before:?} {after:?}"
        );
        assert_eq!(before.stdout, after.stdout, "{ruby}");
        let observed = String::from_utf8_lossy(&after.stdout);
        assert!(
            observed.contains("[nil, nil, nil, nil, nil, nil, nil, nil, \"value\\ntext\"]"),
            "{observed}"
        );
        assert!(
            observed.contains("[nil, nil, :recovered]")
                && observed.contains("[:yield, false]")
                && observed.contains("[:yield, true]"),
            "{observed}"
        );
        assert!(
            observed.contains("[:early, :late, :late]")
                && observed.contains(":stopped")
                && !observed.contains(":bad_dispatch"),
            "{observed}"
        );
    }
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
