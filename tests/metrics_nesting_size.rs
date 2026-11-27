//! Metrics/BlockNesting and Metrics/CollectionLiteralLength
//! through the actual CLI — registry metadata, selectors, strict config,
//! both directive prefixes, file-local measuring, stdin/encoding/CRLF/BOM/
//! DATA, parser errors and the no-write guarantee of detection-only cops.
use crate::support::cli::{project_with_lib, run};
use rustocop::{cops::Registry, offense::Severity};
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

use crate::support::stdin as stdin_support;

const COPS: [&str; 2] = ["Metrics/BlockNesting", "Metrics/CollectionLiteralLength"];
const NESTING: &str = "Avoid more than 3 levels of block nesting.";
const SIZE: &str = "Avoid hard coding large quantities of data in code. Prefer reading the data from an external source.";

fn entries(count: usize) -> String {
    (0..count)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Three levels of conditionals around a loop (the fourth level, line 4)
/// and a 250-entry array (line 10).
fn source() -> String {
    format!(
        "if a\n  if b\n    if c\n      while d\n        x\n      end\n    end\n  end\nend\nX = [{}]\n",
        entries(250)
    )
}

/// `expect_early_error` only for runs whose asserted exit is the early
/// `--stdin --fix` rejection, which may exit before reading stdin.
fn run_stdin(root: &Path, args: &[&str], bytes: &[u8], expect_early_error: bool) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    stdin_support::write_stdin(child.stdin.take().unwrap(), bytes, expect_early_error).unwrap();
    child.wait_with_output().unwrap()
}

/// The findings of the two cops, as `file:line:col: R: message (cop)` lines.
fn found(out: &Output) -> Vec<String> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| COPS.iter().any(|c| l.contains(&format!("({c})"))))
        .map(str::to_owned)
        .collect()
}

/// Run on `lib/a.rb` holding `bytes`; assert the exit status and that the
/// file's bytes, mtime and permissions are untouched.
fn unchanged(root: &Path, bytes: &[u8], args: &[&str], status: i32) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, bytes).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    assert_eq!(out.status.code(), Some(status), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let new = fs::metadata(&path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    out
}

fn expected(file: &str, shift: u32) -> Vec<String> {
    vec![
        format!(
            "{file}:{}:7: R: {NESTING} (Metrics/BlockNesting)",
            4 + shift
        ),
        format!(
            "{file}:{}:5: R: {SIZE} (Metrics/CollectionLiteralLength)",
            10 + shift
        ),
    ]
}

#[test]
fn registry_metadata_is_fixed_opt_in_and_detection_only() {
    let registry = Registry::new();
    assert_eq!(registry.cops().len(), 613);
    for name in COPS {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(!cop.default_enabled(), "{name}");
        assert!(!cop.safe_fixes(), "{name}");
        assert_eq!(cop.severity(), Severity::Refactor);
        assert_eq!(cop.ruby_versions(), Default::default());
        assert!(cop.path_policy().is_none(), "{name}");
        // CollectionLiteralLength has a `LengthThreshold` upstream, no `Max`.
        let limited = name == "Metrics/BlockNesting";
        assert_eq!(cop.max_setting(), limited.then_some("max"), "{name}");
        let one = &rustocop::cops::settings::Values::default()
            .with("max", rustocop::cops::settings::Value::Count(1));
        assert_eq!(cop.configure(one).is_some(), limited, "{name}");
        assert!(cop.autocorrect_incompatible_with().is_empty());
    }
    assert!(!registry.has_cop("Metrics/Future"));
}

#[test]
fn enabled_runs_report_each_cop_and_fix_never_writes() {
    let d = project_with_lib();
    let bytes = source();
    // The cops are opt-in: a run without a selector or config reports none.
    let out = unchanged(d.path(), bytes.as_bytes(), &["lib/a.rb"], 1);
    assert_eq!(found(&out), Vec::<String>::new(), "{out:?}");
    fs::write(
        d.path().join("rustocop.toml"),
        "[cops.Metrics]\nenabled=true\n",
    )
    .unwrap();
    for args in [
        vec!["lib/a.rb"],
        vec!["--only", "Metrics", "lib/a.rb"],
        vec![
            "--only",
            "Metrics/BlockNesting,Metrics/CollectionLiteralLength",
            "--fix",
            "lib/a.rb",
        ],
        vec!["--only", "Metrics", "--fix", "lib/a.rb"],
    ] {
        let out = unchanged(d.path(), bytes.as_bytes(), &args, 1);
        assert_eq!(found(&out), expected("lib/a.rb", 0), "{args:?}");
    }
}

#[test]
fn boundaries_at_the_actual_defaults() {
    let d = project_with_lib();
    let three = "if a\n  if b\n    if c\n      x while d\n    end\n  end\nend\n";
    let out = unchanged(
        d.path(),
        three.as_bytes(),
        &["--only", "Metrics/BlockNesting", "lib/a.rb"],
        1,
    );
    assert_eq!(
        found(&out),
        [format!("lib/a.rb:4:7: R: {NESTING} (Metrics/BlockNesting)")]
    );
    let out = unchanged(
        d.path(),
        three.replace("x while d", "x if d").as_bytes(),
        &["--only", "Metrics/BlockNesting", "lib/a.rb"],
        0,
    );
    assert!(found(&out).is_empty());
    for (count, reported) in [(249, false), (250, true), (251, true)] {
        for literal in [
            format!("[{}]", entries(count)),
            format!(
                "{{{}}}",
                (0..count)
                    .map(|n| format!("{n} => {n}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            format!("Set[{}]", entries(count)),
        ] {
            let out = unchanged(
                d.path(),
                format!("{literal}\n").as_bytes(),
                &["--only", "Metrics/CollectionLiteralLength", "lib/a.rb"],
                i32::from(reported),
            );
            let want: Vec<String> = if reported {
                vec![format!(
                    "lib/a.rb:1:1: R: {SIZE} (Metrics/CollectionLiteralLength)"
                )]
            } else {
                vec![]
            };
            assert_eq!(found(&out), want, "{count} {}", &literal[..4]);
        }
    }
}

#[test]
fn selectors_department_and_strict_config() {
    let d = project_with_lib();
    let bytes = source();
    for (config, flags, count) in [
        (String::new(), vec!["--only", "Metrics"], 2),
        (String::new(), vec!["--only", "Metrics/BlockNesting"], 1),
        (
            String::new(),
            vec!["--only", "Metrics", "--except", "Metrics/BlockNesting"],
            1,
        ),
        (
            String::new(),
            vec!["--only", "Metrics/CollectionLiteralLength", "--except", "Metrics"],
            0,
        ),
        ("[cops.Metrics]\nenabled=false\n".into(), vec![], 0),
        (
            "[cops.Metrics]\nenabled=false\n[cops.\"Metrics/CollectionLiteralLength\"]\nenabled=true\n"
                .into(),
            vec![],
            1,
        ),
        ("[cops.Metrics]\nenabled=true\n".into(), vec![], 2),
        (
            "[cops.\"Metrics/BlockNesting\"]\nenabled=true\n".into(),
            vec![],
            1,
        ),
        (
            "[cops.Metrics]\nenabled=true\n[cops.\"Metrics/BlockNesting\"]\nenabled=false\n".into(),
            vec![],
            1,
        ),
        (
            "[cops.\"Metrics/BlockNesting\"]\nexclude=['lib/a.rb']\n".into(),
            vec!["--only", "Metrics"],
            1,
        ),
        (
            "exclude=['lib/a.rb']\n".into(),
            vec!["--only", "Metrics"],
            0,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), &config).unwrap();
        let mut args = flags.clone();
        args.push("lib/a.rb");
        let status = if flags.contains(&"--only") || config.is_empty() {
            i32::from(count > 0)
        } else {
            1
        };
        let out = unchanged(d.path(), bytes.as_bytes(), &args, status);
        assert_eq!(found(&out).len(), count, "{config:?} {flags:?} {out:?}");
    }
    // No counting knob is admitted; `max` is the one setting of BlockNesting.
    for name in COPS {
        for body in [
            if name == "Metrics/BlockNesting" {
                "Max=3"
            } else {
                "max=2"
            },
            "Max=2",
            "CountBlocks=true",
            "CountModifierForms=true",
            "Include=['**/*']",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{name}\"]\n{body}\n"),
            )
            .unwrap();
            let out = unchanged(
                d.path(),
                bytes.as_bytes(),
                &["--only", name, "--fix", "lib/a.rb"],
                2,
            );
            assert!(found(&out).is_empty(), "{name} {body}");
        }
    }
}

#[test]
fn both_directive_prefixes_disable_at_the_reported_line() {
    let d = project_with_lib();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rubocop", "rustocop"] {
        let bytes = format!("# {prefix}:disable Metrics\n{}", source());
        let out = unchanged(
            d.path(),
            bytes.as_bytes(),
            &["--only", "Metrics", "lib/a.rb"],
            0,
        );
        assert!(found(&out).is_empty());
        let bytes = source().replacen(
            "      while d\n",
            &format!("      while d # {prefix}:disable Metrics/BlockNesting\n"),
            1,
        );
        let out = unchanged(
            d.path(),
            bytes.as_bytes(),
            &["--only", "Metrics", "lib/a.rb"],
            1,
        );
        assert_eq!(found(&out), expected("lib/a.rb", 0)[1..]);
        let bytes = source().replacen(
            "X = [",
            &format!("# {prefix}:disable-next Metrics/CollectionLiteralLength\nX = ["),
            1,
        );
        let out = unchanged(
            d.path(),
            bytes.as_bytes(),
            &["--only", "Metrics", "lib/a.rb"],
            1,
        );
        assert_eq!(found(&out), expected("lib/a.rb", 0)[..1]);
        // A disabled offense does not hide the level below it (upstream
        // ignores the subtree only when the offense is enabled).
        let bytes = format!(
            "if a\n  if b\n    if c\n      while d # {prefix}:disable Metrics/BlockNesting\n        x if (e ? 1 : 2)\n      end\n    end\n  end\nend\n"
        );
        let out = unchanged(
            d.path(),
            bytes.as_bytes(),
            &["--only", "Metrics/BlockNesting", "lib/a.rb"],
            1,
        );
        assert_eq!(
            found(&out),
            [format!(
                "lib/a.rb:5:15: R: {NESTING} (Metrics/BlockNesting)"
            )]
        );
    }
}

#[test]
fn every_file_is_measured_on_its_own() {
    let d = project_with_lib();
    let deep = source();
    let shallow = "if a\n  if b\n    if c\n      x\n    end\n  end\nend\nX = [1]\n";
    fs::write(d.path().join("lib/a.rb"), &deep).unwrap();
    fs::write(d.path().join("lib/b.rb"), shallow).unwrap();
    for args in [
        vec!["--only", "Metrics", "lib/a.rb", "lib/b.rb"],
        vec!["--only", "Metrics", "lib/b.rb", "lib/a.rb"],
        vec!["--only", "Metrics", "lib"],
    ] {
        let out = run(d.path(), &args);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(found(&out), expected("lib/a.rb", 0), "{args:?}");
    }
    let out = run(d.path(), &["--only", "Metrics", "lib/b.rb"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        fs::read(d.path().join("lib/b.rb")).unwrap(),
        shallow.as_bytes()
    );
}

#[test]
fn stdin_crlf_bom_data_and_legacy_encodings_keep_positions() {
    let d = project_with_lib();
    let crlf = source().replace('\n', "\r\n");
    let bom = format!("\u{feff}{}", source());
    let data = format!(
        "{}__END__\nif a\n  if b\n    if c\n      if d\n      end\n    end\n  end\nend\n[{}]\n",
        source(),
        entries(300)
    );
    let mut latin = b"# coding: Windows-1252\n".to_vec();
    latin.extend(
        source()
            .replacen("        x\n", "        x # caf@\n", 1)
            .bytes()
            .map(|b| if b == b'@' { 0xe9 } else { b }),
    );
    for (bytes, shift) in [
        (crlf.into_bytes(), 0),
        (bom.into_bytes(), 0),
        (data.into_bytes(), 0),
        (latin, 1),
    ] {
        let out = unchanged(
            d.path(),
            &bytes,
            &["--only", "Metrics", "--fix", "lib/a.rb"],
            1,
        );
        assert_eq!(found(&out), expected("lib/a.rb", shift));
        let out = run_stdin(
            d.path(),
            &["--only", "Metrics", "--stdin", "lib/b.rb"],
            &bytes,
            false,
        );
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(found(&out), expected("lib/b.rb", shift));
    }
}

#[test]
fn parse_errors_missing_files_and_stdin_fix_never_write() {
    let d = project_with_lib();
    let bytes = format!("{}def broken(\n", source());
    let out = unchanged(
        d.path(),
        bytes.as_bytes(),
        &["--only", "Metrics", "--fix", "lib/a.rb"],
        2,
    );
    assert!(found(&out).is_empty());
    assert_eq!(
        run(d.path(), &["--only", "Metrics", "missing.rb"])
            .status
            .code(),
        Some(2)
    );
    let out = run_stdin(
        d.path(),
        &["--only", "Metrics", "--stdin", "lib/a.rb", "--fix"],
        source().as_bytes(),
        true,
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
        "{out:?}"
    );
}

#[test]
fn correcting_peers_compose_and_the_cops_keep_their_findings() {
    let d = project_with_lib();
    let path = d.path().join("lib/a.rb");
    let bytes = source().replacen("        x\n", "        x = 'y'\n", 1);
    fs::write(&path, &bytes).unwrap();
    let flags = [
        "--only",
        "Metrics,Style/StringLiterals",
        "--fix",
        "lib/a.rb",
    ];
    let out = run(d.path(), &flags);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(found(&out), expected("lib/a.rb", 0));
    assert_eq!(
        fs::read(&path).unwrap(),
        bytes.replacen("'y'", "\"y\"", 1).as_bytes()
    );
    // Stable: a second pass changes nothing and reports the same.
    let after = fs::read(&path).unwrap();
    let out = run(d.path(), &flags);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(found(&out), expected("lib/a.rb", 0));
    assert_eq!(fs::read(&path).unwrap(), after);
}
