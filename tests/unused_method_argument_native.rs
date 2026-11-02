//! Independently authored finite Safety programs ONLY. Never execute reference fixtures.
use std::{
    fs,
    io::{Read, Seek},
    path::Path,
    process::{Command, Output, Stdio},
};
fn limited(ruby: &str, root: &Path, code: &str, warning: &str) -> Output {
    // External wall-time/address-space/output bounds. Regular output files make
    // RLIMIT_FSIZE effective; guard absence, overflow and timeout are failures.
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
            "--parser=prism",
            warning,
            "-e",
            code,
        ])
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(stdout.try_clone().unwrap())
        .stderr(stderr.try_clone().unwrap())
        .status()
        .expect("external native guards required");
    assert!(!matches!(status.code(), Some(124 | 137)), "native timeout");
    assert!(
        stdout.metadata().unwrap().len() < 65536 && stderr.metadata().unwrap().len() < 65536,
        "native output cap"
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
fn success(o: &Output) {
    assert!(o.status.success(), "{o:?}");
}
#[test]
fn authored_inactive_parameter_rename_preserves_native_values_binding_trace_warnings_vm_and_heap() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    // Authored witness: the method is never defined, dispatched or evaluated.
    // No fixture is read or interpolated into any executable program.
    let a = "if false\ndef some_method(foo, bar)\n  puts bar\nend\nend\n";
    let b = "if false\ndef some_method(_foo, bar)\n  puts bar\nend\nend\n";
    for warning in ["-W0", "-W1", "-w"] {
        for setup in [
            "",
            "$events=[]; class Object; def self.method_added(name); $events << name; raise 'authored'; end; end;",
            "$events=[]; t=TracePoint.new(:line,:call,:return,:c_call,:c_return,:raise) { |e| if e.path == '-e'; raise 'trace cap exceeded' if $events.size >= 128; $events << [e.event,e.lineno,e.method_id]; end }; t.enable;",
        ] {
            let tail = if setup.is_empty() {
                "p TOPLEVEL_BINDING.local_variables\n"
            } else if setup.contains("TracePoint") {
                "t.disable; p $events\n"
            } else {
                "p $events\n"
            };
            let first = limited(
                &ruby,
                d.path(),
                &format!("{setup}\np(begin\n{a}end)\n{tail}"),
                warning,
            );
            let second = limited(
                &ruby,
                d.path(),
                &format!("{setup}\np(begin\n{b}end)\n{tail}"),
                warning,
            );
            success(&first);
            success(&second);
            assert_eq!(first.stdout, second.stdout);
            assert_eq!(first.stderr, second.stderr);
            assert!(first.stderr.is_empty(), "{first:?}");
        }
    }
    let hook = limited(
        &ruby,
        d.path(),
        "class Object; def self.method_added(name); raise 'authored hook fired'; end; end; begin; def authored_hook_probe; nil; end; rescue => error; puts error.message; end",
        "-W0",
    );
    success(&hook);
    assert_eq!(hook.stdout, b"authored hook fired\n");
    let code = r#"
a="if false\ndef some_method(foo, bar)\n  puts bar\nend\nend\n"
b="if false\ndef some_method(_foo, bar)\n  puts bar\nend\nend\n"
x=RubyVM::InstructionSequence.compile(a,'authored.rb')
y=RubyVM::InstructionSequence.compile(b,'authored.rb')
raise 'VM code changed' unless x.to_a == y.to_a && x.disasm == y.disasm
# Warm both sequences before the bounded allocation comparison.
10.times { x.eval; y.eval }
GC.start
n=GC.stat(:total_allocated_objects); 100.times { x.eval }; first=GC.stat(:total_allocated_objects)-n
n=GC.stat(:total_allocated_objects); 100.times { y.eval }; second=GC.stat(:total_allocated_objects)-n
raise 'allocation changed' unless (first-second).abs <= 2
puts 'authored VM and heap control passed'
"#;
    let out = limited(&ruby, d.path(), code, "-w");
    success(&out);
    assert!(out.stderr.is_empty(), "{out:?}");
    // Actual rebuilt CLI output, not an invented no-op correction.
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    fs::write(d.path().join("safety.rb"), a).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", "Lint/UnusedMethodArgument", "--fix", "safety.rb"])
        .output()
        .unwrap();
    success(&out);
    assert_eq!(fs::read_to_string(d.path().join("safety.rb")).unwrap(), b);
}
#[test]
fn authored_live_binding_reflection_keyword_and_collision_counterexamples_are_held() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    for (before, after) in [
        (
            "def f(authored); nil; end; p method(:f).parameters",
            "def f(_authored); nil; end; p method(:f).parameters",
        ),
        (
            "def f(authored); p binding.local_variables; end; f(1)",
            "def f(_authored); p binding.local_variables; end; f(1)",
        ),
        (
            "def f(authored:); nil; end; begin; p f(authored: 1); rescue => e; p e.class; end",
            "def f(_authored:); nil; end; begin; p f(authored: 1); rescue => e; p e.class; end",
        ),
        (
            "def f(authored); eval('authored'); end; p f(42)",
            "def f(_authored); eval('authored') rescue :missing; end; p f(42)",
        ),
        (
            "def f(&authored); nil; end; p method(:f).parameters",
            "def f; nil; end; p method(:f).parameters",
        ),
    ] {
        let a = limited(&ruby, d.path(), before, "-W0");
        let b = limited(&ruby, d.path(), after, "-W0");
        success(&a);
        success(&b);
        assert_ne!(a.stdout, b.stdout, "{before}");
    }
}
