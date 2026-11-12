//! real CLI policy/no-write checks and controlled Ruby loop witnesses.
//! Pinned examples: RuboCop style/for_spec.rb (MIT License,
//! Copyright (c) 2012-26 Bozhidar Batsov).
use std::{fs, path::Path, process::Command};

const COP: &str = "Style/For";
const SOURCE: &str = "for n in [1, 2, 3] do\n  puts n\nend\n";

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
    assert_eq!(out.status.code(), Some(code), "{args:?}: {out:?}");
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
    out
}

#[test]
fn default_on_exact_registration_selectors_config_and_excludes() {
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
            .contains("a.rb:1:1: C: Prefer `each` over `for`. (Style/For)")
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
        format!("[cops.\"{COP}\"]\nEnforcedStyle=\"for\"\n"),
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
fn both_directive_prefixes_cover_whole_loop_and_reenable_siblings() {
    let d = project();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "Style", "all"] {
            for source in [
                format!("# {prefix}:disable {target}\n{SOURCE}"),
                format!("# {prefix}:disable-next {target}\n{SOURCE}"),
                SOURCE.replacen('\n', &format!(" # {prefix}:disable {target}\n"), 1),
                format!("{} # {prefix}:disable {target}\n", SOURCE.trim_end()),
            ] {
                unchanged(
                    d.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "a.rb"],
                    0,
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
            assert_eq!(text.matches("(Style/For)").count(), 1, "{out:?}");
            assert!(text.contains("a.rb:6:1:"), "{out:?}");
        }
    }
}

#[test]
fn registered_for_selectors_are_not_removed_by_the_opt_in_directive_ban() {
    let d = project();
    let peers = "Style/For,Style/DisableCopsWithinSourceCodeDirective";
    for prefix in ["rustocop", "rubocop"] {
        for (mode, for_count) in [("next -", 0), ("next +", 1), ("disable ", 0)] {
            let source = format!("# {prefix}:{mode}{COP}\n{SOURCE}");
            let out = unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", peers, "--fix", "a.rb"],
                1,
            );
            let text = String::from_utf8_lossy(&out.stdout);
            assert_eq!(text.matches("(Style/For)").count(), for_count, "{out:?}");
            assert_eq!(
                text.matches("(Style/DisableCopsWithinSourceCodeDirective)")
                    .count(),
                1,
                "{out:?}"
            );
        }
    }
}

#[derive(serde::Deserialize)]
struct Case {
    spec_line: u32,
    source: String,
    positions: Vec<(u32, u32)>,
    correction: Option<String>,
    no_correction: bool,
}
#[test]
fn all_pinned_unsafe_correction_and_explicit_no_correction_calls_retain_bytes() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/for_pinned.json")).unwrap();
    let d = project();
    let mut counts = (0, 0);
    for case in cases {
        counts.0 += usize::from(case.correction.is_some());
        counts.1 += usize::from(case.no_correction);
        let code = if case.positions.is_empty() { 0 } else { 1 };
        let out = unchanged(
            d.path(),
            case.source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            code,
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout)
                .matches("(Style/For)")
                .count(),
            case.positions.len(),
            "spec {}: {out:?}",
            case.spec_line
        );
    }
    assert_eq!(counts, (20, 6));
}

#[test]
fn encoding_bom_crlf_literals_data_and_line_values_are_never_rewritten() {
    let d = project();
    for source in [
        format!("\u{feff}# coding: UTF-8\r\n{}p __LINE__\r\n", SOURCE.replace('\n', "\r\n")).into_bytes(),
        b"# coding: windows-1252\nfor x in items\n puts \"caf\xe9\"\nend\n__END__\nraw\0payload\r\nfor x in items; end\n".to_vec(),
        b"for x in items\n puts <<~TEXT\nfor not ruby\nTEXT\nend\np [__LINE__, DATA.pos]\n__END__\nraw\n".to_vec(),
    ] {
        let out = unchanged(d.path(), &source, &["--only", COP, "--fix", "a.rb"], 1);
        assert_eq!(String::from_utf8_lossy(&out.stdout).matches("(Style/For)").count(), 1, "{out:?}");
    }
    unchanged(
        d.path(),
        b"items.each { work }\n",
        &["--only", COP, "--fix", "a.rb"],
        0,
    );
    unchanged(
        d.path(),
        b"for x in (\n",
        &["--only", COP, "--fix", "a.rb"],
        2,
    );
}

use crate::support::stdin as stdin_support;
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
fn binding_visibility_witness_distinguishes_unsafe_for_to_each_conversion() {
    let d = project();
    let before = "x=:outer; y=:outer; for x in [1,2]; y=x*2; end; p [x,y]\n";
    let unsafe_after = "x=:outer; y=:outer; [1,2].each { |x| y=x*2 }; p [x,y]\n";
    unchanged(
        d.path(),
        before.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        1,
    );
    for ruby in rubies() {
        assert_eq!(run(&ruby, before), b"[2, 4]\n");
        assert_eq!(run(&ruby, unsafe_after), b"[:outer, 4]\n");
        assert_eq!(
            run(&ruby, &fs::read_to_string(d.path().join("a.rb")).unwrap()),
            b"[2, 4]\n"
        );
    }
}

#[test]
fn runtime_jumps_exceptions_effects_results_locals_and_pre_post_test_contexts_are_preserved() {
    let d = project();
    for (source, expected) in [
        (
            "log=[]; again=true; r=for i in [1,2,3]; log << i; next if i==1; if again; again=false; redo; end; break :stop; end; p [i,r,log]\n",
            "[2, :stop, [1, 2, 2]]\n",
        ),
        (
            "log=[]; r=:not_returned; begin; r=for x in [1,2,3]; log << x; raise 'boom' if x==2; end; rescue => e; log << e.message; ensure; log << :ensure; end; p [x,r,log]\n",
            "[2, :not_returned, [1, 2, \"boom\", :ensure]]\n",
        ),
        (
            "$calls=0; def collection(items); $calls+=1; items; end; items=[1,2]; result=for i in collection(items); local=i*10; end; p [$calls,i,local,result.equal?(items)]\n",
            "[1, 2, 20, true]\n",
        ),
        (
            "r=for i in []; local=1; end; p [i,local,r]\n",
            "[nil, nil, []]\n",
        ),
        (
            "n=0; m=0; log=[]; while n<1; n+=1; for x in [1,2]; log << [:pre,x]; end; end; begin; m+=1; for y in [3,4]; log << [:post,y]; end; end until m>=1; p [n,m,x,y,log]\n",
            "[1, 1, 2, 4, [[:pre, 1], [:pre, 2], [:post, 3], [:post, 4]]]\n",
        ),
    ] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        let retained = fs::read_to_string(d.path().join("a.rb")).unwrap();
        for ruby in rubies() {
            assert_eq!(run(&ruby, source), expected.as_bytes(), "{source}");
            assert_eq!(run(&ruby, &retained), expected.as_bytes());
        }
    }
}
