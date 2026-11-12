//! The metrics-length family through the actual CLI —
//! registry metadata, selectors, strict config, both directive prefixes,
//! the pinned BlockLength gemspec exclusion, stdin/encoding/CRLF/BOM/DATA,
//! parser errors and the no-write guarantee of a detection-only family.
use rustocop::{cops::Registry, offense::Severity};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const COPS: [&str; 4] = [
    "Metrics/MethodLength",
    "Metrics/BlockLength",
    "Metrics/ClassLength",
    "Metrics/ModuleLength",
];

fn lines(count: usize, indent: &str) -> String {
    (1..=count).map(|i| format!("{indent}a = {i}\n")).collect()
}

/// One finding of each cop: a module with 101 lines of its own around a
/// class with 101 lines holding an 11-line method and a 26-line block.
fn source() -> String {
    format!(
        "module M\n  class C\n    def m\n{}    end\n    foo do\n{}    end\n{}  end\n{}end\n",
        lines(11, "      "),
        lines(26, "      "),
        lines(60, "    "),
        lines(101, "  ")
    )
}

fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::create_dir(d.path().join("lib")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

/// The findings of the family, as `(file:line:col: C: message (cop))` lines.
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

fn expected(file: &str) -> Vec<String> {
    vec![
        format!("{file}:1:1: C: Module has too many lines. [101/100] (Metrics/ModuleLength)"),
        format!("{file}:2:3: C: Class has too many lines. [101/100] (Metrics/ClassLength)"),
        format!("{file}:3:5: C: Method has too many lines. [11/10] (Metrics/MethodLength)"),
        format!("{file}:16:5: C: Block has too many lines. [26/25] (Metrics/BlockLength)"),
    ]
}

#[test]
fn registry_metadata_is_fixed_default_on_and_detection_only() {
    let registry = Registry::new();
    assert_eq!(registry.cops().len(), 613);
    for name in COPS {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(cop.default_enabled(), "{name}");
        assert!(!cop.safe_fixes(), "{name}");
        assert_eq!(cop.severity(), Severity::Convention);
        assert_eq!(cop.ruby_versions(), Default::default());
        assert!(cop.max_setting().is_none(), "{name}");
        assert!(cop.with_max(1).is_none(), "{name}");
        assert_eq!(cop.path_policy().is_some(), name == "Metrics/BlockLength");
    }
    assert!(!registry.has_cop("Metrics/Future"));
}

#[test]
fn default_run_reports_each_cop_at_its_node_and_fix_never_writes() {
    let d = project();
    let bytes = source();
    for args in [
        vec!["lib/a.rb"],
        vec!["--only", "Metrics", "lib/a.rb"],
        vec!["--only", "Metrics", "--fix", "lib/a.rb"],
    ] {
        let out = unchanged(d.path(), bytes.as_bytes(), &args, 1);
        assert_eq!(found(&out), expected("lib/a.rb"), "{args:?}");
    }
}

#[test]
fn selectors_department_and_strict_config() {
    let d = project();
    let bytes = source();
    for (config, flags, count) in [
        (String::new(), vec!["--only", "Metrics"], 4),
        (String::new(), vec!["--only", "Metrics/MethodLength"], 1),
        (
            String::new(),
            vec!["--only", "Metrics", "--except", "Metrics/BlockLength"],
            3,
        ),
        (
            String::new(),
            vec!["--only", "Metrics/ClassLength", "--except", "Metrics"],
            0,
        ),
        ("[cops.Metrics]\nenabled=false\n".into(), vec![], 0),
        (
            "[cops.Metrics]\nenabled=false\n[cops.\"Metrics/ModuleLength\"]\nenabled=true\n".into(),
            vec![],
            1,
        ),
        (
            "[cops.\"Metrics/BlockLength\"]\nenabled=false\n".into(),
            vec![],
            3,
        ),
        (
            "[cops.\"Metrics/MethodLength\"]\nexclude=['lib/a.rb']\n".into(),
            vec![],
            3,
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
        let status = if flags.contains(&"--only") {
            i32::from(count > 0)
        } else {
            1
        };
        let out = unchanged(d.path(), bytes.as_bytes(), &args, status);
        assert_eq!(found(&out).len(), count, "{config:?} {flags:?} {out:?}");
    }
    // No threshold, style or allow-list knob is admitted.
    for name in COPS {
        for body in [
            "max=200",
            "Max=200",
            "CountComments=true",
            "CountAsOne=['array']",
            "AllowedMethods=['foo']",
            "AllowedPatterns=['foo']",
            "include=['**/*']",
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
    let d = project();
    for prefix in ["rubocop", "rustocop"] {
        let bytes = format!("# {prefix}:disable Metrics\n{}", source());
        let out = unchanged(
            d.path(),
            bytes.as_bytes(),
            &["--only", "Metrics", "lib/a.rb"],
            0,
        );
        assert!(found(&out).is_empty());
        // Disabling on the header line silences that node only.
        let bytes = source().replacen(
            "    def m\n",
            &format!("    def m # {prefix}:disable Metrics/MethodLength\n"),
            1,
        );
        let out = unchanged(
            d.path(),
            bytes.as_bytes(),
            &["--only", "Metrics", "lib/a.rb"],
            1,
        );
        let mut want = expected("lib/a.rb");
        want.remove(2);
        assert_eq!(found(&out), want);
        let bytes = source().replacen(
            "    foo do\n",
            &format!("    # {prefix}:disable-next Metrics/BlockLength\n    foo do\n"),
            1,
        );
        let out = unchanged(
            d.path(),
            bytes.as_bytes(),
            &["--only", "Metrics/BlockLength", "lib/a.rb"],
            0,
        );
        assert!(found(&out).is_empty(), "{out:?}");
    }
}

#[test]
fn the_pinned_gemspec_exclusion_holds_even_when_forced() {
    let d = project();
    let block = format!("Gem::Specification.new do |s|\n{}end\n", lines(30, "  "));
    fs::write(d.path().join("foo.gemspec"), &block).unwrap();
    fs::write(d.path().join("lib/foo.rb"), &block).unwrap();
    for config in ["", "[cops.\"Metrics/BlockLength\"]\nenabled=true\n"] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        for flags in [
            vec![],
            vec!["--only", "Metrics/BlockLength"],
            vec!["--only", "Metrics"],
        ] {
            let mut args = flags.clone();
            args.push("foo.gemspec");
            let out = run(d.path(), &args);
            assert!(found(&out).is_empty(), "{config:?} {flags:?} {out:?}");
            let mut args = flags.clone();
            args.push("lib/foo.rb");
            let out = run(d.path(), &args);
            assert_eq!(
                found(&out),
                ["lib/foo.rb:1:1: C: Block has too many lines. [30/25] (Metrics/BlockLength)"],
                "{config:?} {flags:?}"
            );
        }
    }
    // Stdin takes the supplied name's path policy.
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", "Metrics/BlockLength", "--stdin", "foo.gemspec"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(block.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        fs::read(d.path().join("foo.gemspec")).unwrap(),
        block.as_bytes()
    );
}

#[test]
fn stdin_crlf_bom_data_and_legacy_encodings_keep_positions() {
    let d = project();
    let crlf = source().replace('\n', "\r\n");
    let bom = format!("\u{feff}{}", source());
    let data = format!("{}__END__\n{}", source(), lines(200, "  "));
    let mut latin = b"# coding: Windows-1252\n".to_vec();
    latin.extend(
        source()
            .replacen("foo do", "foo do # caf@", 1)
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
        let want: Vec<_> = expected("lib/a.rb")
            .into_iter()
            .map(|l| shift_line(&l, shift))
            .collect();
        assert_eq!(found(&out), want);
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", "Metrics", "--stdin", "lib/b.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&bytes).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        let want: Vec<_> = expected("lib/b.rb")
            .into_iter()
            .map(|l| shift_line(&l, shift))
            .collect();
        assert_eq!(found(&out), want);
    }
}

/// `file:line:` with `line` moved down by `shift`.
fn shift_line(finding: &str, shift: u32) -> String {
    let (file, rest) = finding.split_once(':').unwrap();
    let (line, rest) = rest.split_once(':').unwrap();
    format!("{file}:{}:{rest}", line.parse::<u32>().unwrap() + shift)
}

#[test]
fn parse_errors_missing_files_and_stdin_fix_never_write() {
    let d = project();
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
    let out = run(
        d.path(),
        &["--only", "Metrics", "--stdin", "lib/a.rb", "--fix"],
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
}

#[test]
fn correcting_peers_compose_and_the_family_keeps_its_findings() {
    let d = project();
    let path = d.path().join("lib/a.rb");
    let bytes = source().replacen("a = 1\n", "a = 'x'\n", 1);
    fs::write(&path, &bytes).unwrap();
    let flags = [
        "--only",
        "Metrics,Style/StringLiterals",
        "--fix",
        "lib/a.rb",
    ];
    let out = run(d.path(), &flags);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(found(&out), expected("lib/a.rb"));
    assert_eq!(
        fs::read(&path).unwrap(),
        bytes.replacen("'x'", "\"x\"", 1).as_bytes()
    );
    // Stable: a second pass changes nothing and reports the same.
    let after = fs::read(&path).unwrap();
    let out = run(d.path(), &flags);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(found(&out), expected("lib/a.rb"));
    assert_eq!(fs::read(&path).unwrap(), after);
}
