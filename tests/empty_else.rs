//! EmptyElse controls, real companion configuration and semantic witnesses.
//! Pinned examples: RuboCop MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const COP: &str = "Style/EmptyElse";
const BAD: &str = "if ready\n  work\nelse\n  nil\nend\n";
const GOOD: &str = "if ready\n  work\nend\n";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn count(o: &Output) -> usize {
    assert_ne!(o.status.code(), Some(2), "{o:?}");
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter(|l| l.contains("(Style/EmptyElse)"))
        .count()
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], expected: usize) {
    let p = root.join("a.rb");
    fs::write(&p, source).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    assert_eq!(count(&cli(root, args)), expected);
    assert_eq!(fs::read(&p).unwrap(), source);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
}
#[test]
fn default_selectors_config_and_no_write() {
    let d = project();
    for (cfg, flags, expected) in [
        ("", vec![], 1),
        ("", vec!["--only", COP], 1),
        ("", vec!["--only", "Style", "--except", COP], 0),
        ("[cops.\"Style/EmptyElse\"]\nenabled = false\n", vec![], 0),
        (
            "[cops.\"Style/EmptyElse\"]\nenabled = false\n",
            vec!["--only", COP],
            1,
        ),
        (
            "[cops.Style]\nenabled = false\n[cops.\"Style/EmptyElse\"]\nenabled = true\n",
            vec![],
            1,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), cfg).unwrap();
        let mut args = flags;
        args.push("a.rb");
        unchanged(d.path(), BAD.as_bytes(), &args, expected);
    }
    let registry = rustocop::cops::Registry::new();
    let c = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(c.default_enabled() && c.safe_fixes());
    assert_eq!(c.ruby_versions(), Default::default());
    assert!(c.max_setting().is_none() && c.path_policy().is_none());
}
#[test]
fn excludes_and_both_directive_prefixes_prevent_writes() {
    let d = project();
    for cfg in [
        "exclude = [\"a.rb\"]\n",
        "[cops.\"Style/EmptyElse\"]\nexclude = [\"a.rb\"]\n",
    ] {
        fs::write(d.path().join("rustocop.toml"), cfg).unwrap();
        unchanged(
            d.path(),
            BAD.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "Style", "all"] {
            unchanged(
                d.path(),
                format!("# {prefix}:disable {target}\n{BAD}").as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                0,
            );
        }
        unchanged(
            d.path(),
            format!("if a; foo; else nil; end # {prefix}:disable {COP}\n").as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
        unchanged(
            d.path(),
            format!("if a\n foo\n # {prefix}:disable-next {COP}\nelse\nend\n").as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
        unchanged(
            d.path(),
            format!("if a\n foo\nelse\n # {prefix}:disable Lint/Debugger\n nil\nend\n").as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
    }
}
#[test]
fn every_pinned_call_and_canonical_correction_through_cli() {
    let d = project();
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/empty_else_pinned.json")).unwrap();
    for case in cases {
        let source = case["source"].as_str().unwrap();
        fs::write(
            d.path().join("rustocop.toml"),
            if case["missing_else"] == true {
                "[cops.\"Style/MissingElse\"]\nenabled = true\n"
            } else {
                ""
            },
        )
        .unwrap();
        let args = ["--only", COP, "--fix", "a.rb"];
        let expected = case["positions"].as_array().unwrap().len();
        if let Some(after) = case["correction"].as_str() {
            fs::write(d.path().join("a.rb"), source).unwrap();
            assert_eq!(count(&cli(d.path(), &args)), 0, "{case}");
            assert_eq!(
                fs::read_to_string(d.path().join("a.rb")).unwrap(),
                after,
                "{case}"
            );
            unchanged(d.path(), after.as_bytes(), &args, 0);
        } else {
            unchanged(d.path(), source.as_bytes(), &args, expected);
        }
    }
}
#[test]
fn actual_missing_else_configuration_and_selection_are_both_honored() {
    let d = project();
    for cfg in [
        "[cops.\"Style/MissingElse\"]\nenabled = true\n",
        "[cops.Style]\nenabled = true\n",
    ] {
        fs::write(d.path().join("rustocop.toml"), cfg).unwrap();
        // Configured enablement matters even when --only/--except hide MissingElse.
        unchanged(
            d.path(),
            BAD.as_bytes(),
            &[
                "--only",
                COP,
                "--except",
                "Style/MissingElse",
                "--fix",
                "a.rb",
            ],
            1,
        );
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for selected in [
        "Style/EmptyElse,Style/MissingElse",
        "Style/MissingElse,Style/EmptyElse",
    ] {
        unchanged(
            d.path(),
            BAD.as_bytes(),
            &["--only", selected, "--fix", "a.rb"],
            1,
        );
    }
    fs::write(
        d.path().join("rustocop.toml"),
        "[cops.Style]\nenabled = true\n[cops.\"Style/MissingElse\"]\nenabled = false\n",
    )
    .unwrap();
    fs::write(d.path().join("a.rb"), BAD).unwrap();
    assert_eq!(count(&cli(d.path(), &["--only", COP, "--fix", "a.rb"])), 0);
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), GOOD);
}
#[test]
fn source_envelopes_are_preserved_and_sensitive_edits_stay_reported() {
    let d = project();
    for (before, after) in [
        (BAD.as_bytes().to_vec(), GOOD.as_bytes().to_vec()),
        (
            format!("\u{feff}{BAD}").into_bytes(),
            format!("\u{feff}{GOOD}").into_bytes(),
        ),
        (
            BAD.replace('\n', "\r\n").into_bytes(),
            GOOD.replace('\n', "\r\n").into_bytes(),
        ),
        (
            [b"# coding: ISO-8859-1\n# \xe9\n".as_slice(), BAD.as_bytes()].concat(),
            [
                b"# coding: ISO-8859-1\n# \xe9\n".as_slice(),
                GOOD.as_bytes(),
            ]
            .concat(),
        ),
        (
            format!("{BAD}__END__\npayload\x00\n").into_bytes(),
            format!("{GOOD}__END__\npayload\x00\n").into_bytes(),
        ),
        (
            b"if a; foo; else nil; end\np __LINE__\n".to_vec(),
            b"if a; foo; end\np __LINE__\n".to_vec(),
        ),
    ] {
        fs::write(d.path().join("a.rb"), before).unwrap();
        assert_eq!(count(&cli(d.path(), &["--only", COP, "--fix", "a.rb"])), 0);
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), after);
        unchanged(d.path(), &after, &["--only", COP, "--fix", "a.rb"], 0);
    }
    for source in [
        format!("{BAD}p __LINE__\n"),
        format!("{BAD}p DATA.pos\n__END__\npayload\n"),
        format!("{BAD}p File.binread(__FILE__)\n"),
        "if a; consume <<~TEXT; else\n  payload\nTEXT\n nil\nend\n".into(),
        "if a; foo; else\n # keep this comment\n nil\nend\n".into(),
    ] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
    }
    unchanged(
        d.path(),
        GOOD.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        0,
    );
    let bad = b"if (\n";
    fs::write(d.path().join("a.rb"), bad).unwrap();
    let t = fs::metadata(d.path().join("a.rb"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        cli(d.path(), &["--only", COP, "--fix", "a.rb"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), bad);
    assert_eq!(
        fs::metadata(d.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap(),
        t
    );
}
fn rubies() -> Vec<String> {
    let mut r = vec!["ruby".into()];
    if let Ok(path) = std::env::var("RUSTOCOP_RUBY34") {
        r.push(path);
    }
    r
}
#[test]
fn runtime_values_effect_counts_binding_and_rescue_ensure_are_unchanged() {
    let d = project();
    let source = r#"$events = []
def hit(value)
  $events << value
  value
end
def f(a, b)
  x = if hit(a)
    begin
      hit(:yes)
    rescue StandardError
      hit(:rescue)
    ensure
      hit(:ensure)
    end
  elsif hit(b)
    unless hit(a)
      hit(:elsif)
    else
      nil
    end
  else
    nil
  end
  y = case hit(a)
  when true
    hit(:case)
  else
    nil
  end
  [x, y, defined?(x), defined?(y)]
end
p [f(false, false), f(false, true), f(true, true), $events]
"#;
    fs::write(d.path().join("a.rb"), source).unwrap();
    assert_eq!(count(&cli(d.path(), &["--only", COP, "a.rb"])), 3);
    assert_eq!(count(&cli(d.path(), &["--only", COP, "--fix", "a.rb"])), 0);
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert_ne!(after, source);
    for ruby in rubies() {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        let observe = |s: &str| Command::new(&ruby).args(["-e", s]).output().unwrap();
        let before = observe(source);
        let fixed = observe(&after);
        assert!(
            before.status.success() && fixed.status.success(),
            "{ruby}: {before:?} {fixed:?}"
        );
        assert_eq!(before.stdout, fixed.stdout, "{ruby}");
        assert_eq!(
            String::from_utf8_lossy(&before.stdout),
            "[[nil, nil, \"local-variable\", \"local-variable\"], [:elsif, nil, \"local-variable\", \"local-variable\"], [:yes, :case, \"local-variable\", \"local-variable\"], [false, false, false, false, true, false, :elsif, false, true, :yes, :ensure, true, :case]]\n"
        );
    }
    unchanged(
        d.path(),
        after.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        0,
    );
}
#[test]
fn early_returns_exceptions_loop_control_and_modifier_locals_are_preserved() {
    let d = project();
    let source = r#"$events = []
def g(mode)
  if mode == :raise
    begin
      raise "boom"
    rescue RuntimeError
      $events << :rescued
      :recovered
    ensure
      $events << :ensured
    end
  elsif mode == :return
    return :returned
  else
    nil
  end
end
nil if (local = false)
result = if false
  local
else
  nil
end
loop_result = [1, 2].each do |n|
  if n == 1
    next :skipped
  elsif n == 2
    break :broken
  else
    nil
  end
end
p [g(:raise), g(:return), g(:none), loop_result, result, defined?(local), local, $events]
"#;
    fs::write(d.path().join("a.rb"), source).unwrap();
    assert_eq!(count(&cli(d.path(), &["--only", COP, "a.rb"])), 3);
    assert_eq!(count(&cli(d.path(), &["--only", COP, "--fix", "a.rb"])), 0);
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    for ruby in rubies() {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        let observe = |s: &str| Command::new(&ruby).args(["-e", s]).output().unwrap();
        let before = observe(source);
        let fixed = observe(&after);
        assert!(
            before.status.success() && fixed.status.success(),
            "{ruby}: {before:?} {fixed:?}"
        );
        assert_eq!(before.stdout, fixed.stdout, "{ruby}");
        assert_eq!(before.stdout, b"[:recovered, :returned, nil, :broken, nil, \"local-variable\", false, [:rescued, :ensured]]\n");
    }
}

#[test]
fn default_style_layout_and_conditional_peers_stabilize_without_cycles() {
    let d = project();
    let source = "def choose(a, b)\n  if a\n    if b\n      return 'yes'\n    else\n      nil\n    end\n  elsif b\n    {'key' => 'value'}\n  else\n    nil\n  end\nend\np [choose(true, true), choose(false, true), choose(false, false)]\n";
    for selected in [
        "Style,Layout",
        "Style/EmptyElse,Style/IfInsideElse,Style/UnlessElse,Style/RedundantReturn,Style/StringLiterals,Style/HashSyntax,Style/Semicolon,Style/GuardClause,Style/IfUnlessModifier,Style/RedundantParentheses,Style/SignalException,Layout",
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let mut stable = false;
        for _ in 0..6 {
            let old = fs::read(d.path().join("a.rb")).unwrap();
            let o = cli(
                d.path(),
                &[
                    "--only",
                    selected,
                    "--except",
                    "Style/MissingElse",
                    "--fix",
                    "a.rb",
                ],
            );
            assert_ne!(o.status.code(), Some(2), "{o:?}");
            if old == fs::read(d.path().join("a.rb")).unwrap() {
                stable = true;
                break;
            }
        }
        assert!(stable, "{selected}");
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        for ruby in rubies() {
            if Command::new(&ruby).arg("--version").output().is_err() {
                continue;
            }
            let observe = |s: &str| Command::new(&ruby).args(["-e", s]).output().unwrap();
            let before = observe(source);
            let fixed = observe(&after);
            assert!(
                before.status.success() && fixed.status.success(),
                "{before:?} {fixed:?}"
            );
            assert_eq!(before.stdout, fixed.stdout, "{selected}: {ruby}");
        }
    }
}
