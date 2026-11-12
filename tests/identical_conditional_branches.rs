//! pinned RuboCop style/identical_conditional_branches_spec.rb examples
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), selection and no-write contracts.
use serde::Deserialize;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
const COP: &str = "Style/IdenticalConditionalBranches";
const BAD: &str = "if condition\n  work\nelse\n  work\nend\n";
#[derive(Deserialize)]
struct Case {
    spec_line: u32,
    source: String,
    positions: Vec<(u32, u32)>,
    messages: Vec<String>,
    correction: Option<String>,
}
fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!(
        "fixtures/identical_conditional_branches_pinned.json"
    ))
    .unwrap()
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
fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: Option<i32>) -> std::process::Output {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    if let Some(code) = code {
        assert_eq!(output.status.code(), Some(code), "{args:?}: {output:?}");
    } else {
        assert_ne!(output.status.code(), Some(2), "{output:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
    output
}
#[test]
fn all_53_pinned_calls_have_exact_diagnostics() {
    assert_eq!(cases().len(), 53);
    for case in cases() {
        let found = lint(case.source.as_bytes());
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            case.positions,
            "spec:{} {}",
            case.spec_line,
            case.source
        );
        assert_eq!(
            found.iter().map(|o| o.message.clone()).collect::<Vec<_>>(),
            case.messages,
            "spec:{}",
            case.spec_line
        );
        assert!(found.iter().all(|o| o.correction.is_none()));
    }
}
#[test]
fn all_53_pinned_sources_including_23_unsafe_corrections_remain_unedited() {
    let dir = project();
    let mut count = 0;
    for case in cases() {
        count += usize::from(case.correction.is_some());
        unchanged(
            dir.path(),
            case.source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            Some(i32::from(!case.positions.is_empty())),
        );
    }
    assert_eq!(count, 23);
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
        (String::new(), vec![], 2),
        (String::new(), vec!["--only", COP], 2),
        (String::new(), vec!["--only", COP, "--except", COP], 0),
        (String::new(), vec!["--only", COP, "--except", "Style"], 0),
        (format!("[cops.\"{COP}\"]\nenabled = false\n"), vec![], 0),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["--only", COP],
            2,
        ),
        (
            format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
            vec![],
            2,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = selectors;
        args.push("a.rb");
        let output = unchanged(dir.path(), BAD.as_bytes(), &args, None);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|l| l.contains(COP))
                .count(),
            count,
            "{output:?}"
        );
    }
}
#[test]
fn excludes_and_both_directive_prefixes_apply_without_writes() {
    let dir = project();
    fs::create_dir(dir.path().join("lib")).unwrap();
    for config in [
        "exclude = [\"lib/a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = [\"lib/a.rb\"]\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            &dir.path().join("lib"),
            BAD.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            Some(0),
        );
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for (source, count) in [
            (format!("# {prefix}:disable {COP}\n{BAD}"), 0),
            (format!("# {prefix}:disable Style\n{BAD}"), 0),
            (format!("# {prefix}:disable all\n{BAD}"), 0),
            (
                BAD.replacen("  work", &format!("  work # {prefix}:disable {COP}"), 1),
                1,
            ),
            (
                BAD.replacen(
                    "  work",
                    &format!("  # {prefix}:disable-next {COP}\n  work"),
                    1,
                ),
                1,
            ),
            (
                format!("# {prefix}:disable {COP}\n{BAD}# {prefix}:enable {COP}\n{BAD}"),
                2,
            ),
        ] {
            let output = unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                Some(i32::from(count > 0)),
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .filter(|l| l.contains(COP))
                    .count(),
                count,
                "{output:?}"
            );
        }
    }
}
#[test]
fn controlled_values_binding_evaluation_order_and_exception_flow_are_unchanged() {
    let source = r#"$trace = []
def guard(value)
  $trace << [:guard, value]
  value
end
def effect
  $trace << :common
  :value
end
def f(value)
  begin
    if guard(value)
      effect
      return :yes
    else
      effect
      return :no
    end
  ensure
    $trace << :ensure
  end
end
def bound; :method; end
def binding_case(value)
  if value
    bound if (bound = 7)
    bound
  else
    bound if (bound = 7)
    bound
  end
end
def g(value)
  begin
    case value
    when :raise
      raise "failure"
    else
      raise "failure"
    end
  rescue RuntimeError
    :rescued
  ensure
    $trace << :cleanup
  end
end
p [f(true), f(false), binding_case(true), binding_case(false), g(:raise)]
p $trace
p 1.times.map { if false; next :skip; else; next :skip; end }
"#;
    let expected = b"[:yes, :no, 7, 7, :rescued]\n[[:guard, true], :common, :ensure, [:guard, false], :common, :ensure, :cleanup]\n[:skip]\n";
    let dir = project();
    assert_eq!(lint(source.as_bytes()).len(), 8);
    unchanged(
        dir.path(),
        source.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        Some(1),
    );
    let mut rubies = vec!["ruby".to_owned()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
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
#[test]
fn stdin_reports_exact_positions_and_honors_exclusions() {
    let dir = project();
    for (config, source, count) in [
        (String::new(), BAD.to_owned(), 2),
        (
            format!("[cops.\"{COP}\"]\nexclude = [\"a.rb\"]\n"),
            BAD.to_owned(),
            0,
        ),
        (String::new(), format!("# rubocop:disable {COP}\n{BAD}"), 0),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
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
        assert_eq!(
            output.status.code(),
            Some(i32::from(count > 0)),
            "{output:?}"
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(stdout.lines().filter(|l| l.contains(COP)).count(), count);
        if count > 0 {
            assert!(
                stdout.contains("a.rb:2:3:") && stdout.contains("a.rb:4:3:"),
                "{stdout}"
            );
            assert!(stdout.contains("Move `work` out of the conditional."));
        }
        assert!(!dir.path().join("a.rb").exists());
    }
}
#[test]
fn detection_only_preserves_source_envelopes_and_errors() {
    let dir = project();
    for (source, count) in [
        (b"\xef\xbb\xbf# encoding: UTF-8\r\nif test\r\n  work # first\r\nelse\r\n  work # second\r\nend\r\n__END__\r\npayload\xff\r\n".as_slice(), 2),
        (b"# coding: ISO-8859-1\nif test; puts '\xe9'; else; puts '\xe9'; end\n".as_slice(), 2),
        (b"if test\n  __LINE__\nelse\n  __LINE__\nend\n".as_slice(), 0),
        (b"if test\n  <<~TEXT\n    hello\n  TEXT\nelse\n  <<~TEXT\n    hello\n  TEXT\nend\n".as_slice(), 2),
    ] {
        assert_eq!(lint(source).len(), count);
        for fix in [false, true] {
            let mut args = vec!["--only", COP];
            if fix { args.push("--fix"); }
            args.push("a.rb");
            unchanged(dir.path(), source, &args, Some(i32::from(count > 0)));
        }
    }
    unchanged(
        dir.path(),
        b"if test\n",
        &["--only", COP, "--fix", "a.rb"],
        Some(2),
    );
    for option in [
        "EnforcedStyle = 'hoist'",
        "max = 80",
        "TargetRubyVersion = 3.4",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{option}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            BAD.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            Some(2),
        );
    }
}
