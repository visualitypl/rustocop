//! Separately authored finite Safety programs ONLY; no reference execution.
use std::{
    fs,
    io::{Read, Seek},
    path::Path,
    process::{Command, Output, Stdio},
};
fn limited(ruby: &str, root: &Path, args: &[&str]) -> Output {
    let mut stdout = tempfile::tempfile().unwrap();
    let mut stderr = tempfile::tempfile().unwrap();
    let status = Command::new("timeout")
        .args([
            "--kill-after=1s",
            "10s",
            "prlimit",
            "--as=536870912",
            "--fsize=65536",
            "--core=0",
            "--",
            ruby,
        ])
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(stdout.try_clone().unwrap())
        .stderr(stderr.try_clone().unwrap())
        .status()
        .expect("external guards required");
    assert!(!matches!(status.code(), Some(124 | 137)), "native timeout");
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert!(status.signal().is_none(), "native signal: {status}");
    }
    assert!(
        stdout.metadata().unwrap().len() < 65536 && stderr.metadata().unwrap().len() < 65536,
        "native output limit"
    );
    stdout.rewind().unwrap();
    stderr.rewind().unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    stdout.read_to_end(&mut out).unwrap();
    stderr.read_to_end(&mut err).unwrap();
    Output {
        status,
        stdout: out,
        stderr: err,
    }
}
fn program(ruby: &str, root: &Path, code: &str, warning: &str) -> Output {
    let mut args = vec!["--parser=prism"];
    if !warning.is_empty() {
        args.push(warning);
    }
    args.extend(["-e", code]);
    limited(ruby, root, &args)
}
const A: &str = "if false\n  def to_json\n  end\nend\n";
const B: &str = "if false\n  def to_json(*_args)\n  end\nend\n";
#[test]
fn authored_inactive_canonical_native_value_binding_hooks_events_warnings_vm_and_heap() {
    if crate::support::runtime::skips(
        "pins compiled code or VM state that Ruby 4.0 builds differently",
    ) {
        return;
    }
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    for warning in ["", "-W1", "-w"] {
        for setup in [
            "",
            "$events=[]; class Object; def self.method_added(name); $events << name; raise 'unexpected definition'; end; end;",
            "$events=[]; def self.singleton_method_added(name); return if name == :singleton_method_added; $events << name; raise 'unexpected singleton definition'; end;",
            "$events=[]; t=TracePoint.new(:line,:call,:return,:c_call,:c_return,:raise) { |e| if e.path == '-e'; raise 'trace cap exceeded' if $events.size >= 128; $events << [e.event,e.lineno,e.method_id]; end }; t.enable;",
        ] {
            let tail = if setup.contains("TracePoint") {
                "t.disable; p $events\n"
            } else if setup.is_empty() {
                "p TOPLEVEL_BINDING.local_variables\n"
            } else {
                "p $events\n"
            };
            for (a, b) in [
                (A.to_owned(), B.to_owned()),
                (
                    A.replace("to_json", "self.to_json()"),
                    B.replace("to_json", "self.to_json"),
                ),
            ] {
                let first = program(
                    &ruby,
                    d.path(),
                    &format!("{setup}\np(begin\n{a}end)\n{tail}"),
                    warning,
                );
                let second = program(
                    &ruby,
                    d.path(),
                    &format!("{setup}\np(begin\n{b}end)\n{tail}"),
                    warning,
                );
                assert!(
                    first.status.success() && second.status.success(),
                    "{first:?}\n{second:?}"
                );
                assert_eq!(first.stdout, second.stdout);
                assert_eq!(first.stderr, second.stderr);
                assert!(first.stderr.is_empty(), "{first:?}");
            }
        }
    }
    let code = r#"
a="if false\n  def to_json\n  end\nend\n"
b="if false\n  def to_json(*_args)\n  end\nend\n"
x=RubyVM::InstructionSequence.compile(a,'authored.rb')
y=RubyVM::InstructionSequence.compile(b,'authored.rb')
raise 'VM changed' unless x.to_a == y.to_a && x.disasm == y.disasm
10.times { x.eval; y.eval }; GC.start
n=GC.stat(:total_allocated_objects); 100.times { x.eval }; first=GC.stat(:total_allocated_objects)-n
n=GC.stat(:total_allocated_objects); 100.times { y.eval }; second=GC.stat(:total_allocated_objects)-n
raise 'heap changed' unless (first-second).abs <= 2
puts 'authored VM/heap passed'
"#;
    let out = program(&ruby, d.path(), code, "-w");
    assert!(out.status.success(), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for (a, b) in [
        (A.to_owned(), B.to_owned()),
        (A.replace("to_json", "to_json()"), B.to_owned()),
        (A.replace('\n', "\r\n"), B.replace('\n', "\r\n")),
        (format!("\u{feff}{A}"), format!("\u{feff}{B}")),
        (
            format!("# coding: ISO-8859-1\n# authored ascii comment\n{A}"),
            format!("# coding: ISO-8859-1\n# authored ascii comment\n{B}"),
        ),
        (
            format!("# coding: UTF8-MAC\n{A}"),
            format!("# coding: UTF8-MAC\n{B}"),
        ),
        (
            format!("# coding: ASCII-8BIT\n{A}"),
            format!("# coding: ASCII-8BIT\n{B}"),
        ),
    ] {
        fs::write(d.path().join("safety.rb"), &a).unwrap();
        let cli = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", "Lint/ToJSON", "--fix", "safety.rb"])
            .output()
            .unwrap();
        assert!(cli.status.success(), "{cli:?}");
        assert_eq!(fs::read_to_string(d.path().join("safety.rb")).unwrap(), b);
        for warning in ["", "-W1", "-w"] {
            let first = program(&ruby, d.path(), &a, warning);
            let mut args = vec!["--parser=prism"];
            if !warning.is_empty() {
                args.push(warning);
            }
            args.push("safety.rb");
            let second = limited(&ruby, d.path(), &args);
            assert!(first.status.success() && second.status.success());
            assert_eq!(first.stdout, second.stdout);
            assert_eq!(first.stderr, second.stderr);
        }
    }
}
#[test]
fn authored_live_signature_changes_are_real_counterexamples_not_safe_edits() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    for source in [
        "def to_json; nil; end; p method(:to_json).arity; p method(:to_json).parameters",
        "def to_json; p binding.local_variables; end; to_json",
        "def to_json; :value; end; begin; p to_json(1); rescue ArgumentError; puts :rejected; end",
        "class Object; def self.method_added(name); puts instance_method(name).arity; end; end; def to_json; nil; end",
        "def to_json; :body; end; begin; p to_json(raise('argument')); rescue => e; puts e.message; end",
        "def to_json; puts __LINE__; end; to_json",
    ] {
        let after = source.replacen("def to_json;", "def to_json(*_args);", 1);
        let a = program(&ruby, d.path(), source, "-w");
        let b = program(&ruby, d.path(), &after, "-w");
        assert!(a.status.success() && b.status.success(), "{a:?} {b:?}");
        // Last two are negative controls: argument evaluation and __LINE__ do
        // not change, but their live definitions still are not correction-safe.
        if !source.contains("raise('argument')") && !source.contains("__LINE__") {
            assert_ne!(a.stdout, b.stdout, "{source}");
        } else {
            assert_eq!(a.stdout, b.stdout);
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        fs::write(d.path().join("safety.rb"), source).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", "Lint/ToJSON", "--fix", "safety.rb"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(
            fs::read_to_string(d.path().join("safety.rb")).unwrap(),
            source
        );
    }
}
#[test]
fn trace_overflow_is_a_failure_never_truncated_success() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let code = format!(
        "events=[]; trace=TracePoint.new(:line) {{ |e| if e.path=='-e'; raise 'trace cap exceeded' if events.size>=8; events << e.lineno; end }}; trace.enable\n{}trace.disable; puts events.size",
        "x=1\n".repeat(20)
    );
    let out = program(&ruby, d.path(), &code, "-w");
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("trace cap exceeded"),
        "{out:?}"
    );
}
