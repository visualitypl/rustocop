//! real CLI policy/no-write and controlled Ruby semantic witnesses.
//! Pinned examples: RuboCop style/infinite_loop_spec.rb (MIT License,
//! Copyright (c) 2012-26 Bozhidar Batsov).
use std::{fs, path::Path, process::Command};

const COP: &str = "Style/InfiniteLoop";
const SOURCE: &str = "while true\n  work\n  other\nend\n";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: i32) -> std::process::Output {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(code),
        "{args:?}: {}: {out:?}",
        String::from_utf8_lossy(source)
    );
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
    out
}

#[test]
fn default_on_registration_selectors_configuration_and_exclusions() {
    let registry = rustocop::cops::Registry::new();
    let cops: Vec<_> = registry.cops().iter().filter(|c| c.name() == COP).collect();
    assert_eq!(cops.len(), 1);
    assert!(cops[0].default_enabled());
    assert!(!cops[0].safe_fixes());
    assert_eq!(cops[0].ruby_versions(), Default::default());
    assert!(cops[0].path_policy().is_none() && cops[0].max_setting().is_none());
    let d = project();
    let out = unchanged(d.path(), SOURCE.as_bytes(), &["a.rb"], 1);
    assert!(
        String::from_utf8_lossy(&out.stdout)
            .contains("a.rb:1:1: C: Use `Kernel#loop` for infinite loops. (Style/InfiniteLoop)")
    );
    for selector in [COP, "Style"] {
        let out = unchanged(
            d.path(),
            SOURCE.as_bytes(),
            &["--only", selector, "a.rb"],
            1,
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains(COP));
        unchanged(
            d.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--except", selector, "--fix", "a.rb"],
            0,
        );
    }
    for config in [
        "[cops.Style]\nenabled=false\n".into(),
        format!("[cops.\"{COP}\"]\nenabled=false\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), SOURCE.as_bytes(), &["a.rb"], 0);
        unchanged(
            d.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n"),
    )
    .unwrap();
    unchanged(d.path(), SOURCE.as_bytes(), &["a.rb"], 1);
    for config in [
        "exclude=[\"a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude=[\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            d.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.\"{COP}\"]\nEnforcedStyle=\"loop\"\n"),
    )
    .unwrap();
    unchanged(
        d.path(),
        SOURCE.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        2,
    );
}

#[test]
fn directives_suppress_keyword_only_and_reenable_siblings() {
    let d = project();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "Style", "all"] {
            for source in [
                format!("# {prefix}:disable {target}\n{SOURCE}"),
                format!("# {prefix}:disable-next {target}\n{SOURCE}"),
                SOURCE.replacen('\n', &format!(" # {prefix}:disable {target}\n"), 1),
                format!("begin\n work\nend until false # {prefix}:disable {target}\n"),
            ] {
                unchanged(
                    d.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "a.rb"],
                    0,
                );
            }
            // The pinned offense range is the keyword, not the body/end.
            for source in [
                format!("while true\n work # {prefix}:disable {target}\nend\n"),
                format!("while true\n work\nend # {prefix}:disable {target}\n"),
            ] {
                unchanged(
                    d.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "a.rb"],
                    1,
                );
            }
            let source = format!(
                "# {prefix}:disable {target}\n{SOURCE}# {prefix}:enable {target}\n{SOURCE}"
            );
            let out = unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                1,
            );
            let text = String::from_utf8_lossy(&out.stdout);
            assert_eq!(text.matches("(Style/InfiniteLoop)").count(), 1, "{out:?}");
            assert!(text.contains("a.rb:7:1:"), "{out:?}");
        }
    }
}

#[derive(serde::Deserialize)]
struct Case {
    spec_line: u32,
    source: String,
    positions: Vec<(u32, u32)>,
    correction: Option<String>,
}
#[test]
fn all_pinned_unsafe_correction_calls_are_no_write_holds() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/infinite_loop_pinned.json")).unwrap();
    let d = project();
    let mut corrections = 0;
    for case in cases {
        corrections += usize::from(case.correction.is_some());
        let out = unchanged(
            d.path(),
            case.source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            i32::from(!case.positions.is_empty()),
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout)
                .matches("(Style/InfiniteLoop)")
                .count(),
            case.positions.len(),
            "spec {}: {out:?}",
            case.spec_line
        );
    }
    assert_eq!(corrections, 23);
}

#[test]
fn source_envelopes_clean_and_error_files_retain_bytes_and_mtime() {
    let d = project();
    for source in [
        format!("\u{feff}# coding: UTF-8\r\n{}p __LINE__\r\n", SOURCE.replace('\n', "\r\n")).into_bytes(),
        b"# coding: windows-1252\nwhile true\n puts \"caf\xe9\"\nend\n__END__\nraw\0payload\r\nwhile true; end\n".to_vec(),
        b"while true\n puts <<~TEXT\nuntil false\nTEXT\n break\nend\np [__LINE__, DATA.pos]\n__END__\nraw\n".to_vec(),
    ] {
        let out = unchanged(d.path(), &source, &["--only", COP, "--fix", "a.rb"], 1);
        assert_eq!(String::from_utf8_lossy(&out.stdout).matches("(Style/InfiniteLoop)").count(), 1, "{out:?}");
    }
    unchanged(
        d.path(),
        b"loop { work }\n",
        &["--only", COP, "--fix", "a.rb"],
        0,
    );
    unchanged(
        d.path(),
        b"while true\n",
        &["--only", COP, "--fix", "a.rb"],
        2,
    );
}

#[path = "support/stdin.rs"]
mod stdin_support;
#[test]
fn stdin_reports_without_writing_and_rejects_fix() {
    let d = project();
    fs::write(d.path().join("a.rb"), b"# sentinel\n").unwrap();
    for fix in [false, true] {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rustocop"));
        cmd.current_dir(d.path())
            .args(["--only", COP, "--stdin", "a.rb"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if fix {
            cmd.arg("--fix");
        }
        let mut child = cmd.spawn().unwrap();
        stdin_support::write_stdin(child.stdin.take().unwrap(), SOURCE.as_bytes(), fix).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(if fix { 2 } else { 1 }), "{out:?}");
        if fix {
            assert!(out.stdout.is_empty(), "{out:?}");
            assert!(
                String::from_utf8_lossy(&out.stderr)
                    .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
                "{out:?}"
            );
        }
        if !fix {
            assert!(String::from_utf8_lossy(&out.stdout).contains("a.rb:1:1:"));
        }
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), b"# sentinel\n");
    }
}

fn rubies() -> Vec<String> {
    std::iter::once("ruby".to_owned())
        .chain(std::env::var("RUSTOCOP_RUBY34"))
        .filter(|ruby| Command::new(ruby).arg("--version").output().is_ok())
        .collect()
}
fn run(ruby: &str, source: &str) -> Vec<u8> {
    let out = Command::new(ruby).args(["-e", source]).output().unwrap();
    assert!(out.status.success(), "{ruby}: {source}: {out:?}");
    out.stdout
}

#[test]
fn runtime_preserves_pre_post_tests_locals_jumps_effects_exceptions_and_values() {
    let d = project();
    let cases = [
        (
            "begin; while true; raise StopIteration, 'keep'; end; rescue StopIteration; puts :rescued; end",
            "rescued\n",
            1,
        ),
        (
            "def loop; :custom; end; value = while true; break :done; end; p value",
            ":done\n",
            1,
        ),
        ("while true; a = 42; break; end; p a", "42\n", 0),
        (
            "events=[]; value=while [events << :guard]; events << :body; break :done; end; p [events,value]",
            "[[:guard, :body], :done]\n",
            1,
        ),
        (
            "i=0; seen=[]; value=while true; i+=1; next if i==1; seen << i; redo if i==2; break :done; end; p [i,seen,value]",
            "[3, [2, 3], :done]\n",
            1,
        ),
        (
            "i=0; value=(begin; i+=1; break :done if i==2; end until false); p [i,value]",
            "[2, :done]\n",
            1,
        ),
        (
            "events=[]; begin; while true; events << :body; raise 'boom'; end; rescue; events << :rescue; ensure; events << :ensure; end; p events",
            "[:body, :rescue, :ensure]\n",
            1,
        ),
        (
            "i=0; while false; i+=1; end; begin; i+=1; end while false; p i",
            "1\n",
            0,
        ),
    ];
    for (source, expected, code) in cases {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            code,
        );
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        for ruby in rubies() {
            assert_eq!(run(&ruby, source), expected.as_bytes(), "{ruby}: {source}");
            assert_eq!(run(&ruby, &after), expected.as_bytes(), "{ruby}: {source}");
        }
    }
    // Positive evidence for why even the simplest-looking rewrite is unsafe.
    for ruby in rubies() {
        assert_eq!(
            run(
                &ruby,
                "begin; loop { raise StopIteration }; puts :swallowed; rescue StopIteration; puts :rescued; end"
            ),
            b"swallowed\n"
        );
        assert_eq!(
            run(&ruby, "def loop; :custom; end; p(loop { break :done })"),
            b":custom\n"
        );
    }
}
