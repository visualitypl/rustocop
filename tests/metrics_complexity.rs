//! Metrics/CyclomaticComplexity,
//! Metrics/PerceivedComplexity and Metrics/AbcSize through the actual CLI —
//! registry metadata, selectors, strict config, both directive prefixes,
//! per-method measuring, stdin/encoding/CRLF/BOM/DATA, parser errors and
//! the no-write guarantee of detection-only cops.
use crate::support::cli::{project_with_lib, run};
use rustocop::{cops::Registry, offense::Severity};
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

use crate::support::stdin as stdin_support;

const COPS: [&str; 3] = [
    "Metrics/CyclomaticComplexity",
    "Metrics/PerceivedComplexity",
    "Metrics/AbcSize",
];

/// One point over each fixed limit: complexity 8/7, perceived 9/8 and ABC
/// <3, 18, 8> 19.92/17 (values the pinned cops give for it).
const COMPLEX: &str = "def complex(a)
  if a && b
    c || d
  elsif e
    f while g
  else
    h until i
  end
  j.each { |k| k }
  @l = m.n(o, p)
  @q = r.s(t, u)
end
";

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

/// The findings of the three cops, as `file:line:col: R: message (cop)`.
fn found(out: &Output) -> Vec<String> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| COPS.iter().any(|c| l.contains(&format!("({c})"))))
        .map(str::to_owned)
        .collect()
}

/// Run on `lib/a.rb` holding `bytes`; assert the exit status and that the
/// file's bytes, mtime, permissions and inode are untouched.
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(old.ino(), new.ino());
    }
    out
}

/// The three findings for `COMPLEX` starting on `line` of `file`, in output
/// order.
fn expected(file: &str, line: u32, name: &str) -> Vec<String> {
    vec![
        format!(
            "{file}:{line}:1: R: Cyclomatic complexity for `{name}` is too high. [8/7] (Metrics/CyclomaticComplexity)"
        ),
        format!(
            "{file}:{line}:1: R: Perceived complexity for `{name}` is too high. [9/8] (Metrics/PerceivedComplexity)"
        ),
        format!(
            "{file}:{line}:1: R: Assignment Branch Condition size for `{name}` is too high. [<3, 18, 8> 19.92/17] (Metrics/AbcSize)"
        ),
    ]
}

/// `found` ordered like [`expected`] (by line, then cop order).
fn ordered(out: &Output) -> Vec<String> {
    let mut lines = found(out);
    let key = |l: &String| {
        let line: u32 = l.split(':').nth(1).unwrap().parse().unwrap();
        (line, COPS.iter().position(|c| l.contains(c)).unwrap())
    };
    lines.sort_by_key(key);
    lines
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
        assert_eq!(cop.max_setting(), Some("max"), "{name}");
        let one = &rustocop::cops::settings::Values::default()
            .with("max", rustocop::cops::settings::Value::Count(1));
        assert!(cop.configure(one).is_some(), "{name}");
        assert!(cop.autocorrect_incompatible_with().is_empty());
    }
    assert!(!registry.has_cop("Metrics/Future"));
}

#[test]
fn enabled_runs_report_each_cop_and_fix_never_writes() {
    let d = project_with_lib();
    // The cops are opt-in: a run without a selector or config reports none.
    let out = unchanged(d.path(), COMPLEX.as_bytes(), &["lib/a.rb"], 1);
    assert_eq!(ordered(&out), Vec::<String>::new(), "{out:?}");
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
            "Metrics/CyclomaticComplexity,Metrics/PerceivedComplexity,Metrics/AbcSize",
            "--fix",
            "lib/a.rb",
        ],
        vec!["--only", "Metrics", "--fix", "lib/a.rb"],
    ] {
        let out = unchanged(d.path(), COMPLEX.as_bytes(), &args, 1);
        assert_eq!(
            ordered(&out),
            expected("lib/a.rb", 1, "complex"),
            "{args:?}"
        );
    }
}

#[test]
fn boundaries_at_the_actual_defaults() {
    let d = project_with_lib();
    // `tap` is not an iterating method: complexity 7/7, perceived 8/8, ABC
    // <3, 18, 7> 19.54 (pinned values).
    let at_limit = COMPLEX.replace("j.each", "j.tap");
    let out = unchanged(
        d.path(),
        at_limit.as_bytes(),
        &["--only", "Metrics", "lib/a.rb"],
        1,
    );
    assert_eq!(
        found(&out),
        [
            "lib/a.rb:1:1: R: Assignment Branch Condition size for `complex` is too high. [<3, 18, 7> 19.54/17] (Metrics/AbcSize)"
        ]
    );
    // Without the two attribute assignments only the complexities remain.
    let small = COMPLEX
        .replace("  @l = m.n(o, p)\n", "")
        .replace("  @q = r.s(t, u)\n", "");
    let out = unchanged(
        d.path(),
        small.as_bytes(),
        &["--only", "Metrics", "lib/a.rb"],
        1,
    );
    assert_eq!(ordered(&out), expected("lib/a.rb", 1, "complex")[..2]);
    let out = unchanged(
        d.path(),
        small.replace("j.each", "j.tap").as_bytes(),
        &["--only", "Metrics", "lib/a.rb"],
        0,
    );
    assert!(found(&out).is_empty());
}

#[test]
fn every_definition_is_measured_and_reported_whole() {
    let d = project_with_lib();
    let nested = format!(
        "class X\n  define_method(:dm) do\n{}  end\n\n  def self.outer\n    def inner; end\n{}  end\nend\n",
        indent(
            &COMPLEX
                .lines()
                .skip(1)
                .take(10)
                .collect::<Vec<_>>()
                .join("\n"),
            4
        ),
        indent(
            &COMPLEX
                .lines()
                .skip(1)
                .take(10)
                .collect::<Vec<_>>()
                .join("\n"),
            4
        ),
    );
    let out = unchanged(
        d.path(),
        nested.as_bytes(),
        &["--only", "Metrics", "lib/a.rb"],
        1,
    );
    let mut want = expected("lib/a.rb", 2, "dm");
    want.extend(expected("lib/a.rb", 15, "outer"));
    // `a` is a method call here, one more branch (pinned values).
    let want: Vec<String> = want
        .into_iter()
        .map(|l| {
            l.replacen(":1: R:", ":3: R:", 1)
                .replace("<3, 18, 8> 19.92", "<3, 19, 8> 20.83")
        })
        .collect();
    assert_eq!(ordered(&out), want);
    // A method without a body, a receiver's `define_method` and a dynamic
    // name are not measured.
    for lookalike in [
        "def complex(a, b = c ? d : e)\nend\n",
        "self.define_method(:x) do\n  a if b\nend\n",
        "define_method(\"x#{y}\") do\n  a if b\nend\n",
    ] {
        let out = unchanged(
            d.path(),
            lookalike.as_bytes(),
            &["--only", "Metrics", "lib/a.rb"],
            0,
        );
        assert!(found(&out).is_empty(), "{lookalike}");
    }
}

fn indent(text: &str, by: usize) -> String {
    text.lines()
        .map(|l| format!("{}{l}\n", " ".repeat(by)))
        .collect()
}

#[test]
fn selectors_department_and_strict_config() {
    let d = project_with_lib();
    for (config, flags, count) in [
        (String::new(), vec!["--only", "Metrics"], 3),
        (String::new(), vec!["--only", "Metrics/AbcSize"], 1),
        (
            String::new(),
            vec!["--only", "Metrics", "--except", "Metrics/PerceivedComplexity"],
            2,
        ),
        (
            String::new(),
            vec!["--only", "Metrics/AbcSize", "--except", "Metrics"],
            0,
        ),
        ("[cops.Metrics]\nenabled=false\n".into(), vec![], 0),
        (
            "[cops.Metrics]\nenabled=false\n[cops.\"Metrics/CyclomaticComplexity\"]\nenabled=true\n"
                .into(),
            vec![],
            1,
        ),
        ("[cops.Metrics]\nenabled=true\n".into(), vec![], 3),
        (
            "[cops.\"Metrics/AbcSize\"]\nenabled=true\n".into(),
            vec![],
            1,
        ),
        (
            "[cops.Metrics]\nenabled=true\n[cops.\"Metrics/AbcSize\"]\nenabled=false\n".into(),
            vec![],
            2,
        ),
        (
            "[cops.\"Metrics/AbcSize\"]\nexclude=['lib/a.rb']\n".into(),
            vec!["--only", "Metrics"],
            2,
        ),
        ("exclude=['lib/a.rb']\n".into(), vec!["--only", "Metrics"], 0),
    ] {
        fs::write(d.path().join("rustocop.toml"), &config).unwrap();
        let mut args = flags.clone();
        args.push("lib/a.rb");
        let status = if flags.contains(&"--only") || config.is_empty() {
            i32::from(count > 0)
        } else {
            1
        };
        let out = unchanged(d.path(), COMPLEX.as_bytes(), &args, status);
        assert_eq!(found(&out).len(), count, "{config:?} {flags:?} {out:?}");
    }
    // No allowance or counting knob is admitted; `max` is the one setting.
    for name in COPS {
        for body in [
            "Max=20",
            "AllowedMethods=['complex']",
            "AllowedPatterns=['complex']",
            "CountRepeatedAttributes=false",
            "include=['**/*']",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{name}\"]\n{body}\n"),
            )
            .unwrap();
            let out = unchanged(
                d.path(),
                COMPLEX.as_bytes(),
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
    for prefix in ["rubocop", "rustocop"] {
        let bytes = format!("# {prefix}:disable Metrics\n{COMPLEX}");
        let out = unchanged(
            d.path(),
            bytes.as_bytes(),
            &["--only", "Metrics", "lib/a.rb"],
            0,
        );
        assert!(found(&out).is_empty());
        // The finding spans the whole method: as upstream's
        // `enabled_lines?`, a directive on any of its lines disables it, one
        // after its `end` does not.
        for (from, to) in [
            (
                "def complex(a)\n",
                "def complex(a) # {prefix}:disable Metrics/AbcSize\n",
            ),
            (
                "    c || d\n",
                "    c || d # {prefix}:disable Metrics/AbcSize\n",
            ),
            ("\nend\n", "\nend # {prefix}:disable Metrics/AbcSize\n"),
        ] {
            let bytes = COMPLEX.replacen(from, &to.replace("{prefix}", prefix), 1);
            let out = unchanged(
                d.path(),
                bytes.as_bytes(),
                &["--only", "Metrics", "lib/a.rb"],
                1,
            );
            assert_eq!(
                ordered(&out),
                expected("lib/a.rb", 1, "complex")[..2],
                "{to}"
            );
        }
        let bytes = format!("{COMPLEX}x # {prefix}:disable Metrics/AbcSize\n");
        let out = unchanged(
            d.path(),
            bytes.as_bytes(),
            &["--only", "Metrics", "lib/a.rb"],
            1,
        );
        assert_eq!(ordered(&out), expected("lib/a.rb", 1, "complex"));
        let bytes = format!(
            "# {prefix}:disable-next Metrics/CyclomaticComplexity, Metrics/PerceivedComplexity\n{COMPLEX}"
        );
        let out = unchanged(
            d.path(),
            bytes.as_bytes(),
            &["--only", "Metrics", "lib/a.rb"],
            1,
        );
        assert_eq!(ordered(&out), expected("lib/a.rb", 2, "complex")[2..]);
    }
}

#[test]
fn stdin_crlf_bom_data_and_legacy_encodings_keep_positions() {
    let d = project_with_lib();
    let crlf = COMPLEX.replace('\n', "\r\n");
    let bom = format!("\u{feff}{COMPLEX}");
    let data = format!("{COMPLEX}__END__\n{COMPLEX}");
    let mut latin = b"# coding: Windows-1252\n".to_vec();
    latin.extend(
        COMPLEX
            .replacen("    c || d\n", "    c || d # caf@\n", 1)
            .bytes()
            .map(|b| if b == b'@' { 0xe9 } else { b }),
    );
    for (bytes, line) in [
        (crlf.into_bytes(), 1),
        (bom.into_bytes(), 1),
        (data.into_bytes(), 1),
        (latin, 2),
    ] {
        let out = unchanged(
            d.path(),
            &bytes,
            &["--only", "Metrics", "--fix", "lib/a.rb"],
            1,
        );
        assert_eq!(ordered(&out), expected("lib/a.rb", line, "complex"));
        let out = run_stdin(
            d.path(),
            &["--only", "Metrics", "--stdin", "lib/b.rb"],
            &bytes,
            false,
        );
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(ordered(&out), expected("lib/b.rb", line, "complex"));
    }
}

#[test]
fn parse_errors_missing_files_and_stdin_fix_never_write() {
    let d = project_with_lib();
    let bytes = format!("{COMPLEX}def broken(\n");
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
        COMPLEX.as_bytes(),
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
    let bytes = COMPLEX.replacen("    c || d\n", "    c || 'd'\n", 1);
    fs::write(&path, &bytes).unwrap();
    let flags = [
        "--only",
        "Metrics,Style/StringLiterals",
        "--fix",
        "lib/a.rb",
    ];
    let out = run(d.path(), &flags);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let want: Vec<String> = expected("lib/a.rb", 1, "complex")
        .into_iter()
        .map(|l| l.replace("<3, 18, 8> 19.92", "<3, 17, 8> 19.03"))
        .collect();
    assert_eq!(ordered(&out), want);
    assert_eq!(
        fs::read(&path).unwrap(),
        bytes.replacen("'d'", "\"d\"", 1).as_bytes()
    );
    // Stable: a second pass changes nothing and reports the same.
    let after = fs::read(&path).unwrap();
    let out = run(d.path(), &flags);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(ordered(&out), want);
    assert_eq!(fs::read(&path).unwrap(), after);
}
