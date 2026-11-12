//! Migration/DepartmentName through the actual CLI — registry
//! metadata, selectors, strict config, both directive prefixes, stdin,
//! encodings/CRLF/BOM/DATA, parse errors, guarded atomic corrections, the
//! no-write holds and compositions with the directive lifecycle cops.
use rustocop::{cops::Registry, offense::Severity};
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

#[path = "support/stdin.rs"]
mod stdin_support;

const COP: &str = "Migration/DepartmentName";
const SOURCE: &str =
    "# rubocop:todo Alias, LineLength\nalias :ala :bala\n# rubocop:enable Alias, LineLength\n";
const FIXED: &str = "# rubocop:todo Style/Alias, Layout/LineLength\nalias :ala :bala\n# rubocop:enable Style/Alias, Layout/LineLength\n";

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

/// This cop's findings as `file:line:col`.
fn found(out: &Output) -> Vec<String> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.ends_with(&format!("({COP})")))
        .map(|l| {
            assert!(l.contains(": C: Department name is missing."), "{l}");
            l.split(": C:").next().unwrap().to_owned()
        })
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

/// `--fix` with only this cop rewrites `before` into `after`, keeping the
/// file's permissions; returns the post-fix output.
fn fixed(root: &Path, before: &[u8], after: &[u8]) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, before).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, &["--only", COP, "--fix", "lib/a.rb"]);
    assert!(out.stderr.is_empty(), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), after, "{out:?}");
    assert_eq!(
        old.permissions(),
        fs::metadata(&path).unwrap().permissions()
    );
    out
}

#[test]
fn registry_metadata_default_on_safe_and_no_knobs() {
    let registry = Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(cop.safe_fixes());
    assert_eq!(cop.severity(), Severity::Convention);
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none());
    assert!(cop.max_setting().is_none());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    assert_eq!(
        registry
            .cops()
            .iter()
            .filter(|c| c.name().starts_with("Migration/"))
            .count(),
        1
    );
    assert!(!registry.has_cop("Migration/Nonexistent"));
}

#[test]
fn default_on_selectors_department_and_strict_config() {
    let d = project();
    let all = vec![
        "lib/a.rb:1:16",
        "lib/a.rb:1:23",
        "lib/a.rb:3:18",
        "lib/a.rb:3:25",
    ];
    for (config, flags, expected) in [
        ("", vec![], all.clone()),
        ("", vec!["--only", COP], all.clone()),
        ("", vec!["--only", "Migration"], all.clone()),
        ("", vec!["--only", "Migration", "--except", COP], vec![]),
        ("", vec!["--only", COP, "--except", "Migration"], vec![]),
        ("", vec!["--except", "Migration"], vec![]),
        ("[cops.Migration]\nenabled=false\n", vec![], vec![]),
        (
            &format!("[cops.\"{COP}\"]\nenabled=false\n"),
            vec![],
            vec![],
        ),
        (
            &format!("[cops.Migration]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n"),
            vec![],
            all.clone(),
        ),
        // --only force-enables a configured-off cop.
        (
            &format!("[cops.\"{COP}\"]\nenabled=false\n"),
            vec!["--only", COP],
            all.clone(),
        ),
        (
            &format!("[cops.\"{COP}\"]\nexclude=['lib/a.rb']\n"),
            vec!["--only", COP],
            vec![],
        ),
        ("exclude=['lib/a.rb']\n", vec!["--only", COP], vec![]),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let mut args = flags.clone();
        args.push("lib/a.rb");
        let path = d.path().join("lib/a.rb");
        fs::write(&path, SOURCE).unwrap();
        let out = run(d.path(), &args);
        assert_eq!(found(&out), expected, "{config:?} {flags:?} {out:?}");
        assert_eq!(fs::read(&path).unwrap(), SOURCE.as_bytes());
    }
    // No style, threshold, version or whitelist knob is admitted.
    for body in [
        "max=1",
        "Max=1",
        "include=['**/*']",
        "AllowedNames=['Alias']",
        "enabled='true'",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{body}\n"),
        )
        .unwrap();
        unchanged(
            d.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            2,
        );
    }
    // An unknown name in the same department is still rejected.
    fs::write(
        d.path().join("rustocop.toml"),
        "[cops.\"Migration/Nonexistent\"]\nenabled=true\n",
    )
    .unwrap();
    unchanged(d.path(), SOURCE.as_bytes(), &["lib/a.rb"], 2);
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    unchanged(
        d.path(),
        SOURCE.as_bytes(),
        &["--only", "Migration/Nonexistent", "lib/a.rb"],
        2,
    );
}

#[test]
fn fix_qualifies_both_prefixes_and_is_stable() {
    let d = project();
    for prefix in ["rubocop", "rustocop"] {
        let before = SOURCE.replace("rubocop", prefix);
        let after = FIXED.replace("rubocop", prefix);
        let out = fixed(d.path(), before.as_bytes(), after.as_bytes());
        assert!(found(&out).is_empty(), "{out:?}");
        // A second pass finds nothing and writes nothing.
        let out = unchanged(
            d.path(),
            after.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            0,
        );
        assert!(found(&out).is_empty());
    }
    // The pinned legacy-name example.
    fixed(
        d.path(),
        b"# rubocop:disable SingleSpaceBeforeFirstArg, Layout/LineLength\nname             \"apache_kafka\"\n",
        b"# rubocop:disable Style/SingleSpaceBeforeFirstArg, Layout/LineLength\nname             \"apache_kafka\"\n",
    );
}

#[test]
fn comment_only_and_empty_files() {
    let d = project();
    let out = unchanged(
        d.path(),
        b"# rubocop:disable Alias\n",
        &["--only", COP, "lib/a.rb"],
        1,
    );
    assert_eq!(found(&out), ["lib/a.rb:1:19"]);
    fixed(
        d.path(),
        b"# rubocop:disable Alias",
        b"# rubocop:disable Style/Alias",
    );
    for empty in [&b""[..], b"\n", b"\xef\xbb\xbf"] {
        unchanged(d.path(), empty, &["--only", COP, "--fix", "lib/a.rb"], 0);
    }
}

#[test]
fn stdin_uses_display_path_and_never_writes() {
    let d = project();
    let path = d.path().join("lib/a.rb");
    fs::write(&path, "original\n").unwrap();
    let out = run_stdin(
        d.path(),
        &["--only", COP, "--stdin", "lib/a.rb"],
        SOURCE.as_bytes(),
        false,
    );
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(found(&out).len(), 4);
    assert!(found(&out)[0].starts_with("lib/a.rb:1:16"));
    let out = run_stdin(
        d.path(),
        &["--only", COP, "--stdin", "lib/a.rb", "--fix"],
        SOURCE.as_bytes(),
        true,
    );
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
        "{out:?}"
    );
    fs::write(d.path().join("rustocop.toml"), "exclude=['lib/a.rb']\n").unwrap();
    let out = run_stdin(
        d.path(),
        &["--only", COP, "--stdin", "lib/a.rb"],
        SOURCE.as_bytes(),
        false,
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), b"original\n");
}

#[test]
fn crlf_bom_legacy_encodings_and_data_keep_every_other_byte() {
    let d = project();
    fixed(
        d.path(),
        b"\xef\xbb\xbf# rubocop:disable Alias\r\nx = 1\r\n",
        b"\xef\xbb\xbf# rubocop:disable Style/Alias\r\nx = 1\r\n",
    );
    fixed(
        d.path(),
        b"# coding: Windows-1252\r\np \"\x80\" # rubocop:todo Alias, LineLength\r\n",
        b"# coding: Windows-1252\r\np \"\x80\" # rubocop:todo Style/Alias, Layout/LineLength\r\n",
    );
    fixed(
        d.path(),
        b"# coding: ASCII-8BIT\n# rustocop:enable Alias\np \"\xff\"\n",
        b"# coding: ASCII-8BIT\n# rustocop:enable Style/Alias\np \"\xff\"\n",
    );
    // A data section is not comments; without a DATA reader it is kept.
    fixed(
        d.path(),
        b"# rubocop:disable Alias\nx = 1\n__END__\n# rubocop:disable Alias\n\xff\n",
        b"# rubocop:disable Style/Alias\nx = 1\n__END__\n# rubocop:disable Alias\n\xff\n",
    );
}

#[test]
fn closed_observers_holds_and_errors_never_write() {
    let d = project();
    for source in [
        // The closed observer list holds the edit.
        "# rubocop:disable Alias\np __FILE__\n",
        "# rubocop:disable Alias\np __dir__\n",
        "# rubocop:disable Alias\np $0, $PROGRAM_NAME\n",
        "# rubocop:disable Alias\np caller, caller_locations\n",
        "# rubocop:disable Alias\np method(:p).source_location\n",
        "# rubocop:disable Alias\np SCRIPT_LINES__\n",
        "# rubocop:disable Alias\np DATA.read\n__END__\npayload\n",
        // The qualified name would change what is suppressed or opted in.
        "# rubocop:disable MultipleComparison\nx = 1\n",
        "# rubocop:disable SelfAssignment\nx = 1\n",
        "# rubocop:disable alias\nx = 1\n",
        "# rubocop:disable line_length\nx = 1\n",
        "# rubocop:enable frozenStringLiteralComment\nx = 1\n",
        // Nothing to qualify.
        "# rubocop:disable Foo\nx = 1\n",
        "# rubocop:disable Test\nx = 1\n",
        "# rubocop:disable \tAlias\nx = 1\n",
    ] {
        let out = unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            1,
        );
        assert_eq!(found(&out).len(), 1, "{source}: {out:?}");
    }
    // A parse error reports no cop finding and writes nothing.
    unchanged(
        d.path(),
        b"# rubocop:disable Alias\ndef broken(\n",
        &["--only", COP, "--fix", "lib/a.rb"],
        2,
    );
}

#[test]
fn qualified_names_suppress_exactly_what_they_did() {
    let d = project();
    let before = "# rubocop:disable Semicolon\nfoo; bar\n# rubocop:enable Semicolon\nfoo; bar\n# rubocop:disable-next Semicolon\nfoo; bar\n";
    let after = "# rubocop:disable Style/Semicolon\nfoo; bar\n# rubocop:enable Style/Semicolon\nfoo; bar\n# rubocop:disable-next Semicolon\nfoo; bar\n";
    let semicolons = |root: &Path| -> Vec<String> {
        let out = run(root, &["--only", "Style/Semicolon", "lib/a.rb"]);
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.contains("(Style/Semicolon)"))
            .map(|l| l.split(": C:").next().unwrap().to_owned())
            .collect()
    };
    fs::write(d.path().join("lib/a.rb"), before).unwrap();
    let original = semicolons(d.path());
    assert_eq!(original, ["lib/a.rb:4:4"]);
    fixed(d.path(), before.as_bytes(), after.as_bytes());
    assert_eq!(semicolons(d.path()), original);
}

#[test]
fn composes_with_directive_lifecycle_cops() {
    let d = project();
    let path = d.path().join("lib/a.rb");
    // Default selection: the qualification lands with the other safe fixes
    // and the directive peers report the corrected names.
    fs::write(&path, SOURCE).unwrap();
    let out = run(d.path(), &["--fix", "--fix-passes", "1", "lib/a.rb"]);
    assert!(out.stderr.is_empty(), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), FIXED.as_bytes());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("(Lint/RedundantCopDisableDirective)"),
        "{text}"
    );
    assert!(found(&out).is_empty(), "{text}");
    // A comment-only file: the qualification and RedundantCopDisableDirective's
    // name removal overlap; the single-pass fixer applies one and the next
    // pass the other, then nothing changes.
    let peers = format!("{COP},Lint/RedundantCopDisableDirective,Lint/Debugger");
    fs::write(&path, "# rubocop:disable Debugger, Alias\n").unwrap();
    for after in [
        &b"# rubocop:disable Lint/Debugger, Style/Alias\n"[..],
        b"# rubocop:disable Style/Alias\n",
        b"# rubocop:disable Style/Alias\n",
    ] {
        let out = run(
            d.path(),
            &["--only", &peers, "--fix", "--fix-passes", "1", "lib/a.rb"],
        );
        assert_eq!(fs::read(&path).unwrap(), after, "{out:?}");
    }
    // CopDirectiveSyntax still sees a legacy name as unknown after
    // qualification; neither cop's edit is lost and nothing loops.
    let syntax = format!("{COP},Lint/CopDirectiveSyntax");
    fs::write(&path, "x = 1 # rubocop:disable Tab\n").unwrap();
    let out = run(
        d.path(),
        &["--only", &syntax, "--fix", "--fix-passes", "1", "lib/a.rb"],
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        b"x = 1 # rubocop:disable Layout/Tab\n"
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("Unknown cop name `Layout/Tab`"),
        "{out:?}"
    );
    // The opt-in ban removes no active selector and keeps the qualification.
    let ban = format!("{COP},Style/DisableCopsWithinSourceCodeDirective");
    fs::write(&path, "x = 1 # rustocop:disable Alias\n").unwrap();
    let out = run(
        d.path(),
        &["--only", &ban, "--fix", "--fix-passes", "1", "lib/a.rb"],
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        b"x = 1 # rustocop:disable Style/Alias\n",
        "{out:?}"
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("directives are not permitted"),
        "{out:?}"
    );
    // Disabling this cop (or `all`) on the line suppresses its finding.
    for source in [
        "# rubocop:disable Migration, Alias\n",
        "x = 1 # rubocop:disable all, Alias\n",
        "# rustocop:disable Migration/DepartmentName\n# rubocop:disable Alias\n",
    ] {
        let out = unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "--fix-passes", "1", "lib/a.rb"],
            0,
        );
        assert!(found(&out).is_empty(), "{source}");
    }
}
