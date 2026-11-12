//! real selectors/config/directives/no-write and controlled Ruby witnesses.
use crate::support::source_getter;
use std::{fs, path::Path, process::Command};
const MISSING: &str = "Style/MissingElse";
const MULTI: &str = "Style/MultilineIfModifier";
const BAD: &str = "[\n  work\n] if ready\n";
const GOOD: &str = "if ready\n  [\n    work\n  ]\nend\n";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn unchanged(
    root: &Path,
    cop: &str,
    source: &[u8],
    args: &[&str],
    count: usize,
) -> std::process::Output {
    let path = root.join("a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let output = cli(root, args);
    assert_ne!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| l.contains(cop))
            .count(),
        count,
        "{output:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), source, "{output:?}");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    output
}
#[test]
fn default_opt_in_selectors_config_and_no_write_contract() {
    let d = project();
    for (cop, source, default) in [(MISSING, "if ready; work; end\n", 0), (MULTI, BAD, 1)] {
        for (config, selectors, count) in [
            (String::new(), vec![], default),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Style", "--except", cop], 0),
            (format!("[cops.\"{cop}\"]\nenabled = false\n"), vec![], 0),
            (
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
                vec!["--only", cop],
                1,
            ),
            (
                format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
                vec![],
                1,
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            let mut args = selectors;
            args.push("a.rb");
            unchanged(d.path(), cop, source.as_bytes(), &args, count);
        }
        let registry = rustocop::cops::Registry::new();
        let rule = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert_eq!(rule.default_enabled(), default == 1);
        assert!(rule.safe_fixes());
        assert_eq!(rule.ruby_versions(), Default::default());
        assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
    }
}
#[test]
fn excludes_both_directives_and_disabled_interior_spans_prevent_writes() {
    let d = project();
    for (cop, source) in [(MISSING, "if ready; work; end\n"), (MULTI, BAD)] {
        for config in [
            "exclude = [\"a.rb\"]\n".to_owned(),
            format!("[cops.\"{cop}\"]\nexclude = [\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                cop,
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                unchanged(
                    d.path(),
                    cop,
                    format!("# {prefix}:disable {target}\n{source}").as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    0,
                );
            }
            unchanged(
                d.path(),
                cop,
                format!("# {prefix}:disable-next {cop}\n{source}").as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
            unchanged(
                d.path(),
                cop,
                format!("{} # {prefix}:disable {cop}\n", source.trim_end()).as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
    }
    unchanged(
        d.path(),
        MULTI,
        b"[\n # rubocop:disable Lint/Debugger\n] if ready\n",
        &["--only", MULTI, "--fix", "a.rb"],
        1,
    );
}
#[test]
fn canonical_corrections_stabilize_and_all_fixed_missing_else_examples_never_write() {
    let d = project();
    fs::write(d.path().join("a.rb"), BAD).unwrap();
    let o = cli(d.path(), &["--only", MULTI, "--fix", "a.rb"]);
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), GOOD);
    unchanged(
        d.path(),
        MULTI,
        GOOD.as_bytes(),
        &["--only", MULTI, "--fix", "a.rb"],
        0,
    );
    // RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/missing_else_pinned.json")).unwrap();
    for c in cases {
        unchanged(
            d.path(),
            MISSING,
            c["source"].as_str().unwrap().as_bytes(),
            &["--only", MISSING, "--fix", "a.rb"],
            c["positions"].as_array().unwrap().len(),
        );
    }
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/multiline_if_modifier_pinned.json")).unwrap();
    for c in cases {
        if [70, 89, 108, 137, 158, 179].contains(&c["spec_line"].as_u64().unwrap()) {
            unchanged(
                d.path(),
                MULTI,
                c["source"].as_str().unwrap().as_bytes(),
                &["--only", MULTI, "--fix", "a.rb"],
                1,
            );
        }
    }
}
#[test]
fn actual_guard_clause_companion_incompatibility_and_configured_exemptions() {
    let d = project();
    let source = b"def f\n  if ready\n    work\n  end\nend\n";
    for selected in [
        "Style/MissingElse,Style/GuardClause",
        "Style/GuardClause,Style/MissingElse",
    ] {
        let o = unchanged(
            d.path(),
            MISSING,
            source,
            &["--only", selected, "--fix", "a.rb"],
            1,
        );
        assert!(
            String::from_utf8_lossy(&o.stdout).contains("(Style/GuardClause)"),
            "{o:?}"
        );
    }
    for (config, source, count) in [
        ("", "if a; raise 'boom'; end\n", 0),
        (
            "[cops.\"Style/GuardClause\"]\nenabled = false\n",
            "if a; raise 'boom'; end\n",
            1,
        ),
        (
            "[cops.Style]\nenabled = false\n",
            "unless a; work; end\n",
            1,
        ),
        ("", "unless a; work; end\n", 0),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            d.path(),
            MISSING,
            source.as_bytes(),
            &[
                "--only",
                MISSING,
                "--except",
                "Style/GuardClause",
                "--fix",
                "a.rb",
            ],
            count,
        );
    }
}
#[test]
fn sensitive_encoding_line_data_clean_and_error_files_keep_bytes_and_mtime() {
    let d = project();
    for source in [
        b"# coding: ISO-8859-1\n[\n '\xe9'\n] if ready\n".as_slice(),
        b"[\n] if ready\np __LINE__\n",
        b"[\n DATA.pos\n] if ready\n__END__\npayload\n",
        b"[\n x\n] if (x = 1)\n",
    ] {
        unchanged(
            d.path(),
            MULTI,
            source,
            &["--only", MULTI, "--fix", "a.rb"],
            1,
        );
    }
    for cop in [MISSING, MULTI] {
        unchanged(
            d.path(),
            cop,
            b"work\n",
            &["--only", cop, "--fix", "a.rb"],
            0,
        );
        fs::write(d.path().join("a.rb"), b"if broken\n").unwrap();
        let modified = fs::metadata(d.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap();
        let o = cli(d.path(), &["--only", cop, "--fix", "a.rb"]);
        assert_eq!(o.status.code(), Some(2), "{o:?}");
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), b"if broken\n");
        assert_eq!(
            fs::metadata(d.path().join("a.rb"))
                .unwrap()
                .modified()
                .unwrap(),
            modified
        );
    }
}
#[test]
fn controlled_runtime_values_binding_evaluation_order_effects_and_rescue_flow() {
    let source = r#"$trace = []
def observe(x); $trace << x; x; end
def choose(a,b)
  [
    observe(:body)
  ] if observe(a) if observe(b)
end
def inverse(a)
  {
    result: observe(:unless)
  } unless observe(a)
end
def lexical(a)
  [
    x
  ] if (x = a)
end
def x; :method_not_local; end
def branch_binding(a)
  [
    x = 1
  ] if defined?(x) && a
end
def protected_flow(a)
  begin
    return :early if a
    [
      raise('failure')
    ] if observe(true)
  rescue RuntimeError
    :rescued
  ensure
    $trace << :ensure
  end
end
def branches(a,b)
  if a
    observe(:a)
  elsif b
    observe(:b)
  end
end
p [nil,false,true].product([nil,false,true]).map { |a,b| choose(a,b) }
p [nil,false,true].map { |a| inverse(a) }
p [lexical(false),lexical(true),branch_binding(true)]
p [protected_flow(true),protected_flow(false)]
p [branches(true,false),branches(false,true),branches(false,false)]
p $trace
"#;
    let d = project();
    fs::write(d.path().join("before.rb"), source).unwrap();
    fs::write(d.path().join("a.rb"), source).unwrap();
    let o = cli(
        d.path(),
        &[
            "--only",
            "Style/MissingElse,Style/MultilineIfModifier",
            "--fix",
            "a.rb",
        ],
    );
    assert_ne!(o.status.code(), Some(2), "{o:?}");
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert_ne!(after, source);
    assert!(
        after.contains("] if (x = a)") && after.contains("] if defined?(x) && a"),
        "{after}"
    );
    let mut rubies = vec!["ruby".to_owned()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        rubies.push(ruby);
    }
    for ruby in rubies {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        let before = Command::new(&ruby)
            .arg(d.path().join("before.rb"))
            .output()
            .unwrap();
        let after = Command::new(&ruby)
            .arg(d.path().join("a.rb"))
            .output()
            .unwrap();
        assert!(
            before.status.success() && after.status.success(),
            "{ruby}: {before:?} {after:?}"
        );
        assert_eq!(before.stdout, after.stdout, "{ruby}");
        let observed = String::from_utf8_lossy(&after.stdout);
        assert!(
            observed.contains("[nil, [:method_not_local], [1]]")
                && observed.contains("[:early, :rescued]")
                && observed.contains("[:a, :b, nil]"),
            "{observed}"
        );
    }
}
#[test]
fn literal_values_encodings_frozen_state_bom_and_crlf_survive_runtime() {
    let d = project();
    let body = "def sample(ready)\n  [\n    \"é\\nline\n  b\", %q(c\n  d), /e\n  f/x, %s(g\n  h)\n  ] unless ready\nend\np [false,true].map { |r| sample(r)&.map { |v| [v.class.to_s, v.is_a?(Regexp) ? v.source.bytes : v.to_s.bytes, v.encoding.name, v.frozen?] } }\n";
    let mut rubies = vec!["ruby".to_owned()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        rubies.push(ruby);
    }
    for frozen in ["true", "false"] {
        for newline in ["\n", "\r\n"] {
            let source =
                format!("\u{feff}# coding: utf-8\n# frozen_string_literal: {frozen}\n{body}")
                    .replace('\n', newline);
            let source = source_getter::pattern_sibling(d.path(), "a.rb", &source, MULTI);
            fs::write(d.path().join("before.rb"), &source).unwrap();
            fs::write(d.path().join("a.rb"), &source).unwrap();
            let o = cli(d.path(), &["--only", MULTI, "--fix", "a.rb"]);
            assert_eq!(o.status.code(), Some(0), "{o:?}");
            assert_ne!(fs::read(d.path().join("a.rb")).unwrap(), source.as_bytes());
            for ruby in &rubies {
                if Command::new(ruby).arg("--version").output().is_err() {
                    continue;
                }
                let before = Command::new(ruby)
                    .arg(d.path().join("before.rb"))
                    .output()
                    .unwrap();
                let after = Command::new(ruby)
                    .arg(d.path().join("a.rb"))
                    .output()
                    .unwrap();
                assert!(
                    before.status.success() && after.status.success(),
                    "{ruby}: {before:?} {after:?}"
                );
                assert_eq!(before.stdout, after.stdout, "{ruby} {frozen} {newline:?}");
                assert!(String::from_utf8_lossy(&after.stdout).contains("UTF-8"));
            }
        }
    }
}
#[test]
fn implicit_match_regexps_preserve_runtime_results() {
    let d = project();
    let mut rubies = vec!["ruby".to_owned()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        rubies.push(ruby);
    }
    for regexp in ["/a\n  b/", "/#{'a'}\n  b/"] {
        let source = format!("$_ = \"a\\n  b\"\np [\n  (:matched if {regexp})\n] if true\n");
        fs::write(d.path().join("before.rb"), &source).unwrap();
        fs::write(d.path().join("a.rb"), &source).unwrap();
        let o = cli(d.path(), &["--only", MULTI, "--fix", "a.rb"]);
        assert_eq!(o.status.code(), Some(0), "{o:?}");
        assert_ne!(fs::read(d.path().join("a.rb")).unwrap(), source.as_bytes());
        for ruby in &rubies {
            if Command::new(ruby).arg("--version").output().is_err() {
                continue;
            }
            let before = Command::new(ruby)
                .arg(d.path().join("before.rb"))
                .output()
                .unwrap();
            let after = Command::new(ruby)
                .arg(d.path().join("a.rb"))
                .output()
                .unwrap();
            assert!(
                before.status.success() && after.status.success(),
                "{ruby}: {before:?} {after:?}"
            );
            assert_eq!(before.stdout, b"[:matched]\n", "{ruby} {regexp}");
            assert_eq!(before.stdout, after.stdout, "{ruby} {regexp}");
        }
    }
}
#[test]
fn modifier_header_comments_keep_bytes_and_mtime() {
    let d = project();
    for keyword in ["if", "unless"] {
        for source in [
            format!("[\n  work\n] {keyword} # important explanation\n ready\n"),
            format!("[\n  work\n] {keyword} # inner explanation\n ready {keyword} outer\n"),
        ] {
            unchanged(
                d.path(),
                MULTI,
                source.as_bytes(),
                &["--only", MULTI, "--fix", "a.rb"],
                1,
            );
        }
    }
}
#[test]
fn stdin_and_strict_config_errors_never_write() {
    use std::{io::Write, process::Stdio};
    let d = project();
    for cop in [MISSING, MULTI] {
        for (config, detail) in [
            (format!("[cops.\"{cop}\"]\nmax = 1\n"), "max"),
            (
                format!("[cops.\"{cop}\"]\nEnforcedStyle = \"other\"\n"),
                "EnforcedStyle",
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            fs::write(d.path().join("a.rb"), BAD).unwrap();
            let o = cli(d.path(), &["--only", cop, "--fix", "a.rb"]);
            assert_eq!(o.status.code(), Some(2), "{o:?}");
            assert!(String::from_utf8_lossy(&o.stderr).contains(detail), "{o:?}");
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), BAD);
        }
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    fs::write(d.path().join("a.rb"), "sentinel\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", MULTI, "--stdin", "a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(BAD.as_bytes())
        .unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(1), "{o:?}");
    assert!(String::from_utf8_lossy(&o.stdout).contains(MULTI));
    assert_eq!(
        fs::read_to_string(d.path().join("a.rb")).unwrap(),
        "sentinel\n"
    );
}
#[test]
fn selected_and_default_style_layout_composition_stabilizes_within_eight_explicit_passes() {
    let d = project();
    let source = "def f(ready)\n [\n  { :key => 'value' }\n ] if ready\nend\n";
    for selected in [
        Some("Style/MissingElse,Style/MultilineIfModifier"),
        Some(
            "Style/MultilineIfModifier,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout",
        ),
        None,
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let mut stable = false;
        for _ in 0..8 {
            let old = fs::read(d.path().join("a.rb")).unwrap();
            let mut args = vec![];
            if let Some(selected) = selected {
                args.extend(["--only", selected]);
            }
            args.extend(["--fix", "a.rb"]);
            let o = cli(d.path(), &args);
            assert_ne!(o.status.code(), Some(2), "{o:?}");
            if old == fs::read(d.path().join("a.rb")).unwrap() {
                stable = true;
                break;
            }
        }
        assert!(stable, "{selected:?}");
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert!(!after.contains("] if ready"), "{after}");
    }
}
