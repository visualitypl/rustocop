//! real CLI controls, byte fidelity and controlled Ruby observations.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const NIL: &str = "Style/NilLambda";
const LIMIT: &str = "Style/NumberedParametersLimit";
const BOTH: &str = "Style/NilLambda,Style/NumberedParametersLimit";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn cli(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], status: i32) {
    let path = dir.join("x.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let out = cli(dir, args);
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), mtime);
}
fn fix(dir: &Path, cop: &str) {
    let out = cli(dir, &["--only", cop, "--fix", "x.rb"]);
    assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
}
#[test]
fn directives_and_exclusions_suppress_both_detection_and_fix_with_bytes_and_mtime() {
    let dir = project();
    for (cop, source) in [(NIL, "-> { nil }"), (LIMIT, "foo { [_1, _2] }")] {
        for prefix in ["rubocop", "rustocop"] {
            for target in [cop, "Style", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\n{source}\n"),
                    format!("# {prefix}:disable-next {target}\n{source}\n"),
                    format!("{source} # {prefix}:disable {target}\n"),
                ] {
                    unchanged(
                        dir.path(),
                        source.as_bytes(),
                        &["--only", cop, "--fix", "x.rb"],
                        0,
                    );
                }
            }
        }
        for config in [
            format!("[cops.\"{cop}\"]\nexclude=[\"x.rb\"]\n"),
            "exclude=[\"x.rb\"]\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "x.rb"],
                0,
            );
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
    unchanged(
        dir.path(),
        b"-> do\n nil # rustocop:disable Style/NilLambda\nend\n",
        &["--only", NIL, "--fix", "x.rb"],
        0,
    );
}
#[test]
fn stdin_paths_only_and_except_work_but_fix_never_writes() {
    let dir = project();
    let path = dir.path().join("x.rb");
    fs::write(&path, "on disk\n").unwrap();
    for (cop, source) in [(NIL, "-> { nil }\n"), (LIMIT, "foo { [_1,_2] }\n")] {
        for except in [false, true] {
            let mut args = vec!["--only", cop, "--stdin", "x.rb"];
            if except {
                args.extend(["--except", cop]);
            }
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(&args)
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
            let out = child.wait_with_output().unwrap();
            assert_eq!(
                out.status.code(),
                Some(if except { 0 } else { 1 }),
                "{out:?}"
            );
            assert_eq!(String::from_utf8_lossy(&out.stdout).contains(cop), !except);
        }
        let out = cli(dir.path(), &["--only", cop, "--fix", "--stdin", "x.rb"]);
        assert_eq!(out.status.code(), Some(2));
        assert_eq!(fs::read_to_string(&path).unwrap(), "on disk\n");
    }
}
fn ruby(dir: &Path) -> Output {
    Command::new("ruby")
        .current_dir(dir)
        .arg("x.rb")
        .output()
        .unwrap()
}
fn has_ruby() -> bool {
    Command::new("ruby").arg("--version").output().is_ok()
}
fn observed_fix(source: &str, expected: &str) {
    let dir = project();
    let path = dir.path().join("x.rb");
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path());
    assert!(before.status.success(), "{source}: {before:?}");
    assert_eq!(
        String::from_utf8_lossy(&before.stdout),
        expected,
        "{source}"
    );
    fix(dir.path(), NIL);
    let after_bytes = fs::read(&path).unwrap();
    assert_ne!(after_bytes, source.as_bytes(), "must actually fix {source}");
    let after = ruby(dir.path());
    assert_eq!(
        (after.status.code(), after.stdout, after.stderr),
        (before.status.code(), before.stdout, before.stderr),
        "{source}"
    );
    fix(dir.path(), NIL);
    assert_eq!(fs::read(path).unwrap(), after_bytes);
}
#[test]
fn runtime_lambda_return_break_next_and_proc_next_keep_arity_and_binding() {
    if !has_ruby() {
        return;
    }
    for head in ["->", "lambda", "proc", "Proc.new"] {
        for flow in ["nil", "next nil", "return nil", "break nil"] {
            if matches!(head, "proc" | "Proc.new") && matches!(flow, "return nil" | "break nil") {
                continue;
            }
            let source = format!(
                "f = {head} {{ |x| {flow} }}; p [f.lambda?, f.arity, f.call(1)]; begin; p f.call; rescue ArgumentError; p :arity; end\n"
            );
            let source = if head == "->" {
                source.replace("-> { |x|", "->(x) {")
            } else {
                source
            };
            observed_fix(
                &source,
                if matches!(head, "->" | "lambda") {
                    "[true, 1, nil]\n:arity\n"
                } else {
                    "[false, 1, nil]\nnil\n"
                },
            );
        }
    }
    observed_fix(
        "x=7; f = ->(y = -> { nil }) { x+y.call.to_i }; p [f.call, x]\n",
        "[7, 7]\n",
    );
    observed_fix(
        "events=[]; f = ->(x=(events << :default; 2), key: 3, **kw, &blk) { nil }; p events; p f.call(other:4) { 5 }; p events\n",
        "[]\nnil\n[:default]\n",
    );
    observed_fix(
        "f = proc { |x,y; z| nil }; p [f.lambda?, f.arity, f.call(1)]\n",
        "[false, 2, nil]\n",
    );
}
#[test]
fn fixed_prism34_parameter_scope_evidence_distinguishes_numbered_it_and_explicit() {
    let dir = project();
    let path = dir.path().join("x.rb");
    let source = "f = ->(x = -> { nil }) { nil }; g = -> { it }; h = -> { [_1, _2] }; k = ->(x) { -> { it + x } }\n";
    let before = ruby_prism::parse(source.as_bytes());
    assert_eq!(before.errors().count(), 0);
    fs::write(&path, source).unwrap();
    fix(dir.path(), BOTH);
    let bytes = fs::read(&path).unwrap();
    assert_ne!(bytes, source.as_bytes());
    let after = ruby_prism::parse(&bytes);
    assert_eq!(after.errors().count(), 0);
    for shape in [
        "ItParametersNode",
        "NumberedParametersNode",
        "BlockParametersNode",
        "ItLocalVariableReadNode",
        "LocalVariableReadNode",
    ] {
        assert_eq!(
            format!("{:?}", before.node()).matches(shape).count(),
            format!("{:?}", after.node()).matches(shape).count(),
            "{shape}"
        );
    }
    // Explicit version evidence rather than executing implicit it on Ruby 3.2.
    if !Command::new("ruby")
        .args(["-rprism", "-e", "exit"])
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return;
    }
    let script = "require 'prism'; r=Prism.parse(File.binread(ARGV[0]),version:'3.4'); abort r.errors.inspect unless r.success?; counts=Hash.new(0); w=->(n) { counts[n.class.name] += 1 if n.class.name =~ /ParametersNode|VariableReadNode/; n.compact_child_nodes.each { |c| w.call(c) } }; w.call(r.value); p counts.sort";
    fs::write(&path, source).unwrap();
    let before = Command::new("ruby")
        .args(["-e", script])
        .arg(&path)
        .output()
        .unwrap();
    fs::write(&path, &bytes).unwrap();
    let after = Command::new("ruby")
        .args(["-e", script])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        before.status.success() && after.status.success(),
        "{before:?}: {after:?}"
    );
    assert_eq!(after.stdout, before.stdout);
}
