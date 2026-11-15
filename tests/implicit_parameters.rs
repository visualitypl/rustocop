//! real CLI contracts, fixed-3.4 Prism binding evidence and controlled
//! runtime observations. Never executes arbitrary corpus code.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const IT: &str = "Style/ItBlockParameter";
const ALL: &str = "Style/ItAssignment,Style/ItBlockParameter,Style/NumberedParameters";
const CASES: [(&str, &str, bool); 3] = [
    ("Style/ItAssignment", "it = 5\np it\n", false),
    (IT, "foo { puts _1 }\n", true),
    (
        "Style/NumberedParameters",
        "foo do\n  puts _2\nend\n",
        false,
    ),
];
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(dir: &Path, args: &[&str], status: i32) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    out
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], status: i32) -> Output {
    let path = dir.join("x.rb");
    fs::write(&path, source).unwrap();
    let m = fs::metadata(&path).unwrap().modified().unwrap();
    let out = cli(dir, args, status);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), m);
    out
}
#[test]
fn defaults_only_except_config_enabling_and_no_write() {
    let d = project();
    for (cop, before, safe) in CASES {
        let registry = rustocop::cops::Registry::new();
        let registered: Vec<_> = registry.cops().iter().filter(|c| c.name() == cop).collect();
        assert_eq!(registered.len(), 1);
        assert!(registered[0].default_enabled());
        assert_eq!(registered[0].safe_fixes(), safe);
        assert!(registered[0].path_policy().is_none());
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        let out = unchanged(d.path(), before.as_bytes(), &["x.rb"], 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
        unchanged(d.path(), before.as_bytes(), &["--only", cop, "x.rb"], 1);
        unchanged(
            d.path(),
            before.as_bytes(),
            &["--only", cop, "--except", cop, "--fix", "x.rb"],
            0,
        );
        for config in [
            format!("[cops.\"{cop}\"]\nenabled=false\n"),
            "[cops.Style]\nenabled=false\n".into(),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), before.as_bytes(), &["--fix", "x.rb"], 0);
            unchanged(d.path(), before.as_bytes(), &["--only", cop, "x.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
        )
        .unwrap();
        if safe {
            cli(d.path(), &["--fix", "x.rb"], 0);
            let after = before.replace("_1", "it");
            assert_eq!(fs::read_to_string(d.path().join("x.rb")).unwrap(), after);
            unchanged(
                d.path(),
                after.as_bytes(),
                &["--only", cop, "--fix", "x.rb"],
                0,
            );
        } else {
            unchanged(d.path(), before.as_bytes(), &["--fix", "x.rb"], 1);
        }
    }
}
#[test]
fn excludes_both_directive_prefixes_and_source_enable() {
    let d = project();
    for (cop, before, safe) in CASES {
        for config in [
            "exclude=[\"*.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude=[\"*.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                before.as_bytes(),
                &["--only", cop, "--fix", "x.rb"],
                0,
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rubocop", "rustocop"] {
            for target in [cop, "Style", "all"] {
                for directive in ["disable", "disable-next"] {
                    let s = format!("# {prefix}:{directive} {target}\n{before}");
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "x.rb"], 0);
                }
            }
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nenabled=false\n"),
        )
        .unwrap();
        let source = format!("# rustocop:enable {cop}\n{before}");
        fs::write(d.path().join("x.rb"), &source).unwrap();
        cli(d.path(), &["--fix", "x.rb"], if safe { 0 } else { 1 });
        assert_eq!(
            fs::read_to_string(d.path().join("x.rb")).unwrap(),
            if safe {
                source.replace("_1", "it")
            } else {
                source
            }
        );
    }
}
#[test]
fn stdin_reports_and_never_writes_and_fix_stdin_is_rejected() {
    use std::{io::Write, process::Stdio};
    let d = project();
    for (cop, source, _) in CASES {
        let path = d.path().join("x.rb");
        fs::write(&path, source).unwrap();
        let m = fs::metadata(&path).unwrap().modified().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", cop, "--stdin", "x.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
        cli(d.path(), &["--only", cop, "--fix", "--stdin", "x.rb"], 2);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), m);
    }
}
#[test]
fn directive_envelopes_forbid_partial_all_use_renames() {
    let d = project();
    for prefix in ["rubocop", "rustocop"] {
        for middle in [
            format!("# {prefix}:disable {IT}\n  use(_1)\n# {prefix}:enable {IT}"),
            format!("use(_1) # {prefix}:disable {IT}"),
            format!("# {prefix}:disable {IT}\n  other\n# {prefix}:enable {IT}"),
        ] {
            let s = format!("foo do\n  use(_1)\n{middle}\n  use(_1)\nend\n");
            unchanged(d.path(), s.as_bytes(), &["--only", IT, "--fix", "x.rb"], 1);
        }
    }
}
#[test]
fn unsafe_uncorrectable_clean_and_error_files_keep_bytes_and_mtime() {
    let d = project();
    for (s, status) in [
        ("it=7; foo { use(_1) }\n", 1),
        ("f=proc { use(_1) }; p f.parameters\n", 1),
        ("foo do\n puts it\nend\n", 1),
        ("foo { _1 }\n", 0),
        ("foo { puts _2 }\n", 0),
        ("foo { || _1 }\n", 2),
        ("foo { |x| _1 }\n", 2),
        ("foo { use(_1)\n", 2),
    ] {
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", IT, "--fix", "x.rb"],
            status,
        );
    }
    for (cop, s, _) in CASES.into_iter().filter(|(_, _, safe)| !safe) {
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "x.rb"], 1);
    }
    // The atomic envelope contains legacy bytes. Do not decode/re-encode them.
    unchanged(
        d.path(),
        b"# encoding: ISO-8859-1\nfoo { use(_1); \"\xe9\"; use(_1) }\n",
        &["--only", IT, "--fix", "x.rb"],
        1,
    );
}
#[test]
fn multiline_rewrite_is_one_pass_and_keeps_the_new_detection_only_offense() {
    let d = project();
    let s = "foo do\n  use(_1)\n  use(_1)\nend\n";
    fs::write(d.path().join("x.rb"), s).unwrap();
    let out = cli(d.path(), &["--only", ALL, "--fix", "x.rb"], 1);
    assert!(
        String::from_utf8_lossy(&out.stdout)
            .contains("Avoid using `it` block parameter for multi-line blocks.")
    );
    let after = s.replace("_1", "it");
    assert_eq!(fs::read_to_string(d.path().join("x.rb")).unwrap(), after);
    unchanged(
        d.path(),
        after.as_bytes(),
        &["--only", ALL, "--fix", "x.rb"],
        1,
    );
}
fn ruby(dir: &Path) -> Output {
    Command::new("ruby")
        .current_dir(dir)
        .arg("x.rb")
        .output()
        .unwrap()
}
fn ruby_version() -> Option<(u32, u32)> {
    let o = Command::new("ruby")
        .args(["-e", "print RUBY_VERSION"])
        .output()
        .ok()?;
    let s = String::from_utf8(o.stdout).ok()?;
    let mut v = s.split('.');
    Some((v.next()?.parse().ok()?, v.next()?.parse().ok()?))
}
#[test]
fn runtime_older_ruby_capture_and_metadata_withholding_witnesses() {
    if ruby_version().is_none() {
        eprintln!("SKIP Ruby unavailable");
        return;
    }
    let d = project();
    let s = "it=99\nf=proc { _1 + 1 }\np [f.arity, f.call(4)]\n";
    fs::write(d.path().join("x.rb"), s).unwrap();
    let original = ruby(d.path());
    assert!(original.status.success());
    assert_eq!(String::from_utf8_lossy(&original.stdout), "[1, 5]\n");
    unchanged(d.path(), s.as_bytes(), &["--only", IT, "--fix", "x.rb"], 1);
    fs::write(d.path().join("x.rb"), s.replace("_1", "it")).unwrap();
    let naive = ruby(d.path());
    assert!(naive.status.success());
    assert_eq!(String::from_utf8_lossy(&naive.stdout), "[0, 100]\n");
    for s in [
        "f=proc { _1 + 1 }; p [f.arity,f.parameters,f.call(4)]\n",
        "f=-> { _1 + 1 }; p [f.lambda?,f.parameters,f.call(4)]\n",
        "p [3].map { use = _1; [use,local_variables] }\n",
    ] {
        fs::write(d.path().join("x.rb"), s).unwrap();
        let before = ruby(d.path());
        assert!(before.status.success());
        unchanged(d.path(), s.as_bytes(), &["--only", IT, "--fix", "x.rb"], 1);
        let after = ruby(d.path());
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
    }
}
#[test]
fn runtime_ruby34_canonical_control_flow_arity_effects_and_literal_values() {
    if ruby_version().is_none_or(|v| v < (3, 4)) {
        eprintln!(
            "SKIP implicit-it runtime equivalence requires Ruby >=3.4; fixed-3.4 Prism binding tests run independently"
        );
        return;
    }
    let d = project();
    let s = "$events=[]\ndef effect(x); $events << x; x; end\ndef build; proc { return effect(_1) }; end\nf=-> { next effect(_1)+1 }\ng=proc { next effect(_1)*2 }\np [f.lambda?,f.arity,g.lambda?,g.arity,f.call(4),g.call(5)]\nbegin; f.call; rescue ArgumentError; puts :arity; end\nh=-> { return effect(_1) }; p h.call(6)\nbegin; build.call(8); rescue LocalJumpError; puts :return_error; end\np [1,2,3].map { next effect(_1) if _1<3; break effect(_1) }\np [9].map { pair=_1; inner=->((x,y)) { effect(pair+x+y) }; inner.call([1,2]) }\np [7].map { [\"#{_1}\",__LINE__,effect(_1)] }\np $events\n";
    fs::write(d.path().join("x.rb"), s).unwrap();
    let before = ruby(d.path());
    assert!(before.status.success(), "{before:?}");
    cli(d.path(), &["--only", IT, "--fix", "x.rb"], 0);
    let after = ruby(d.path());
    assert!(after.status.success(), "{after:?}");
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
    assert_eq!(
        fs::read_to_string(d.path().join("x.rb")).unwrap(),
        s.replace("_1", "it")
    );
}
