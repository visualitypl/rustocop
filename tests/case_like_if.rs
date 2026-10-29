//! pinned RuboCop style/case_like_if_spec.rb examples (MIT License,
//! Copyright (c) 2012-26 Bozhidar Batsov), fixed-policy and no-write contracts.
use serde::Deserialize;
use std::{fs, path::Path, process::Command};
const COP: &str = "Style/CaseLikeIf";
const BAD: &str =
    "if x == 1\n  :one\nelsif x == 2\n  :two\nelsif x == 3\n  :three\nelse\n  :other\nend\n";
const MESSAGE: &str = "Convert `if-elsif` to `case-when`.";
#[derive(Deserialize)]
struct Case {
    spec_line: u32,
    source: String,
    positions: Vec<(u32, u32)>,
    messages: Vec<String>,
    correction: Option<String>,
    adapted: Option<String>,
}
fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!("fixtures/case_like_if_pinned.json")).unwrap()
}
fn lint(source: &[u8]) -> Vec<rustocop::offense::Offense> {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    let result = rustocop::pipeline::lint_one(
        Path::new("a.rb"),
        Path::new("a.rb"),
        Some(source),
        &active,
        &config,
    );
    assert!(
        result.errors.is_empty() && result.internal_errors.is_empty(),
        "{result:?}"
    );
    result.result.offenses
}
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn cli(dir: &Path, args: &[&str], code: i32) -> std::process::Output {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(code), "{args:?}: {output:?}");
    output
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: i32) -> std::process::Output {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    let output = cli(dir, args, code);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
    output
}
#[test]
fn all_pinned_calls_and_three_branch_adaptations_have_exact_diagnostics() {
    let mut adapted = 0;
    for case in cases() {
        let original = lint(case.source.as_bytes());
        let source = if let Some(s) = &case.adapted {
            adapted += 1;
            assert!(
                original.is_empty(),
                "two branches at spec:{}",
                case.spec_line
            );
            s
        } else {
            &case.source
        };
        let found = lint(source.as_bytes());
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            case.positions,
            "spec:{} {source}",
            case.spec_line
        );
        assert_eq!(
            found.iter().map(|o| o.message.clone()).collect::<Vec<_>>(),
            case.messages,
            "spec:{}",
            case.spec_line
        );
        assert!(found.iter().all(|o| o.correction.is_none()));
    }
    assert_eq!(cases().len(), 42);
    assert_eq!(adapted, 17);
}
#[test]
fn all_eighteen_unsafe_correction_calls_remain_byte_and_mtime_identical() {
    let dir = project();
    let mut count = 0;
    for case in cases().into_iter().filter(|c| c.correction.is_some()) {
        count += 1;
        // The original two-branch source is intentionally below our threshold;
        // both it and its three-branch adaptation must remain unedited.
        for source in std::iter::once(&case.source).chain(case.adapted.iter()) {
            let expected = if lint(source.as_bytes()).is_empty() {
                0
            } else {
                1
            };
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                expected,
            );
        }
    }
    assert_eq!(count, 18);
}
#[test]
fn legacy_encoded_constant_names_use_source_character_classes() {
    // Pinned RuboCop exempts lowercase-bearing class names, but accepts
    // multi-character uppercase constants in the same source encoding.
    for (name, count) in [
        (b"\xc9\xe9".as_slice(), 0),
        (b"NS::\xc9\xe9".as_slice(), 0),
        (b"\xc9\xc9".as_slice(), 1),
        (b"\xc9".as_slice(), 0),
    ] {
        for reversed in [false, true] {
            let mut source = b"# coding: ISO-8859-1\nif ".to_vec();
            if reversed {
                source.extend_from_slice(name);
                source.extend_from_slice(b" == x");
            } else {
                source.extend_from_slice(b"x == ");
                source.extend_from_slice(name);
            }
            source.extend_from_slice(b"; elsif x == 1; elsif x == 2; end\n");
            let found = lint(&source);
            assert_eq!(found.len(), count, "{source:?}");
            if count == 1 {
                assert_eq!((found[0].line, found[0].col), (2, 1));
                assert_eq!(found[0].message, MESSAGE);
                assert!(found[0].correction.is_none());
            }
        }
    }
    // The exemption also applies after the subject was chosen in branch one.
    assert!(
        lint(b"# coding: ISO-8859-1\nif x == 1; elsif x == \xc9\xe9; elsif x == 2; end\n")
            .is_empty()
    );
}
#[test]
fn defaults_selectors_and_config_controls() {
    let dir = project();
    let registry = rustocop::cops::Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(!cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    for (config, selectors, count) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", COP, "--except", COP], 0),
        (String::new(), vec!["--only", COP, "--except", "Style"], 0),
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
        args.push("a.rb");
        fs::write(dir.path().join("a.rb"), BAD).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(&args)
            .output()
            .unwrap();
        assert_ne!(output.status.code(), Some(2), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|l| l.contains(COP))
                .count(),
            count,
            "{output:?}"
        );
        assert_eq!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), BAD);
    }
}
#[test]
fn excludes_and_both_directive_prefixes_apply_without_writes() {
    let dir = project();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("lib/a.rb"), BAD).unwrap();
    for config in [
        "exclude = [\"lib/a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = [\"lib/a.rb\"]\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            &dir.path().join("lib"),
            BAD.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for source in [
            format!("# {prefix}:disable {COP}\n{BAD}"),
            format!("# {prefix}:disable-next {COP}\n{BAD}"),
            format!("# {prefix}:disable Style\n{BAD}"),
            BAD.replacen(
                "if x == 1",
                &format!("if x == 1 # {prefix}:disable {COP}"),
                1,
            ),
        ] {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                0,
            );
        }
        let source = format!("# {prefix}:disable {COP}\n{BAD}# {prefix}:enable {COP}\n{BAD}");
        let output = unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|l| l.contains(COP))
                .count(),
            1
        );
    }
}
#[test]
fn source_envelopes_control_flow_and_error_files_never_change() {
    let dir = project();
    let mut sources = vec![
        BAD.as_bytes().to_vec(),
        [
            format!(
                "\u{feff}# encoding: UTF-8\r\n{}__END__\r\n",
                BAD.replace('\n', "\r\n")
            )
            .into_bytes(),
            b"payload\0\xff\r\n".to_vec(),
        ]
        .concat(),
        format!("# encoding: ISO-8859-1\n# caf\u{e9}\n{BAD}").into_bytes(),
        format!(
            "def f(x)\n{}end\n",
            BAD.replace(":one", "return __LINE__")
                .replace(":two", "return if (bound = x)")
                .replace(":three", "bound")
        )
        .into_bytes(),
        format!(
            "begin\n{}rescue StandardError\n  recover\nensure\n  cleanup\nend\n",
            BAD.replace(":one", "raise 'oops'")
        )
        .into_bytes(),
        BAD.replace(":one", "if ready; effect; else; other; end")
            .replace(":other", "x == 4 ? :four : :five")
            .into_bytes(),
        BAD.replace(":two", "# keep branch comment\n  side_effect; __LINE__")
            .into_bytes(),
        BAD.replace(":three", "<<~TEXT\n    data # not code\n  TEXT")
            .into_bytes(),
    ];
    // A genuine Latin-1 character, not UTF-8 bytes under a Latin-1 declaration.
    sources[2] = b"# encoding: ISO-8859-1\n# caf\xe9\n"
        .iter()
        .copied()
        .chain(BAD.bytes())
        .collect();
    for source in sources {
        assert_eq!(lint(&source).len(), 1);
        for fix in [false, true] {
            let args = if fix {
                vec!["--only", COP, "--fix", "a.rb"]
            } else {
                vec!["--only", COP, "a.rb"]
            };
            let output = unchanged(dir.path(), &source, &args, 1);
            assert!(String::from_utf8_lossy(&output.stdout).contains(MESSAGE));
        }
    }
    for (source, code) in [
        ("if x == 1\n", 2),
        ("if x == 1; elsif x == 2; end", 0),
        ("case x; when 1; :one; end", 0),
    ] {
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            code,
        );
    }
}
#[path = "support/stdin.rs"]
mod stdin_support;
#[test]
fn stdin_positions_and_no_write_contract() {
    let dir = project();
    fs::write(dir.path().join("a.rb"), "# sentinel\n").unwrap();
    for fix in [false, true] {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rustocop"));
        cmd.current_dir(dir.path())
            .args(["--only", COP, "--stdin", "a.rb"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if fix {
            cmd.arg("--fix");
        }
        let mut child = cmd.spawn().unwrap();
        stdin_support::write_stdin(child.stdin.take().unwrap(), BAD.as_bytes(), fix).unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(if fix { 2 } else { 1 }),
            "{output:?}"
        );
        if !fix {
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                stdout.contains("a.rb:1:1:") && stdout.contains(MESSAGE),
                "{stdout}"
            );
        }
        assert_eq!(
            fs::read_to_string(dir.path().join("a.rb")).unwrap(),
            "# sentinel\n"
        );
    }
}
#[test]
fn threshold_and_version_are_not_public_options() {
    let dir = project();
    for option in ["MinBranchesCount = 2", "TargetRubyVersion = 3.4", "max = 2"] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{option}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            BAD.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            2,
        );
    }
}
#[test]
fn controlled_runtime_values_binding_and_effects_survive_detection_only_fix() {
    let source = r#"$trace = []
def f(x)
  begin
    if x == 1
      $trace << :one
      return :early
    elsif x == 2
      $trace << :two
      :bound if (bound = x)
      bound
    elsif x == 3
      raise "three"
    else
      if x == 4
        :four
      else
        :other
      end
    end
  rescue RuntimeError
    :rescued
  ensure
    $trace << :ensure
  end
end
p [1,2,3,4,5].map { |x| f(x) }
p $trace
$count = 0
def status
  $count += 1
  $count == 1 ? 0 : 2
end
value = if status == 1
  :one
elsif status == 2
  :two
elsif status == 3
  :three
else
  :other
end
p [value, $count]
"#;
    let expected = b"[:early, 2, :rescued, :four, :other]\n[:one, :ensure, :two, :ensure, :ensure, :ensure, :ensure]\n[:two, 2]\n";
    let dir = project();
    assert_eq!(lint(source.as_bytes()).len(), 2);
    unchanged(
        dir.path(),
        source.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        1,
    );
    let mut rubies = vec!["ruby".to_owned()];
    if let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") {
        rubies.push(ruby);
    }
    for ruby in rubies {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        let output = Command::new(&ruby)
            .arg(dir.path().join("a.rb"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{ruby}: {output:?}");
        assert_eq!(output.stdout, expected, "{ruby}: {output:?}");
    }
}
