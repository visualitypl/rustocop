//! Separately authored finite Safety programs only; NEVER execute reference inputs.
use std::{
    fs,
    io::{Read, Seek},
    path::Path,
    process::{Command, Output, Stdio},
};
fn limited(ruby: &str, root: &Path, code: &str, warning: &str) -> Output {
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
        "native output overflow"
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
fn authored_inactive_identities_preserve_values_binding_warnings_trace_vm_and_heap() {
    if crate::support::runtime::skips(
        "pins compiled code or VM state that Ruby 4.0 builds differently",
    ) {
        return;
    }
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for (op, n) in [("+", 0), ("-", 0), ("*", 1), ("/", 1), ("**", 1)] {
        // Authored witnesses, not fixture/spec strings. Dispatch is unreachable.
        let a = format!("if false\n  alpha {op} {n}\nend\n");
        let b = "if false\n  alpha\nend\n";
        fs::write(d.path().join("safety.rb"), &a).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args([
                "--only",
                "Lint/UselessNumericOperation",
                "--fix",
                "safety.rb",
            ])
            .output()
            .unwrap();
        success(&out);
        assert_eq!(fs::read_to_string(d.path().join("safety.rb")).unwrap(), b);
        for warning in ["-W0", "-W1", "-w"] {
            for setup in [
                "",
                "$events=[]; def alpha; $events << :alpha; raise 'authored dispatch'; end;",
                "$events=[]; t=TracePoint.new(:line,:call,:return,:c_call,:c_return,:raise) { |e| if e.path == '-e'; raise 'trace cap exceeded' if $events.size >= 128; $events << [e.event,e.lineno,e.method_id]; end }; t.enable;",
            ] {
                let tail = if setup.contains("TracePoint") {
                    "t.disable; p $events\n"
                } else if setup.is_empty() {
                    "p TOPLEVEL_BINDING.local_variables\n"
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
        // Controlled code-object/heap inspection of authored strings ONLY.
        let code = format!(
            r#"
a={a:?}
b={b:?}
x=RubyVM::InstructionSequence.compile(a,'authored.rb')
y=RubyVM::InstructionSequence.compile(b,'authored.rb')
raise 'VM changed' unless x.to_a == y.to_a && x.disasm == y.disasm
10.times {{ x.eval; y.eval }}
GC.start
n=GC.stat(:total_allocated_objects); 100.times {{ x.eval }}; first=GC.stat(:total_allocated_objects)-n
n=GC.stat(:total_allocated_objects); 100.times {{ y.eval }}; second=GC.stat(:total_allocated_objects)-n
raise 'allocation changed' unless (first-second).abs <= 2
puts 'authored VM and heap passed'
"#
        );
        let out = limited(&ruby, d.path(), &code, "-w");
        success(&out);
        assert!(out.stderr.is_empty(), "{out:?}");
    }
}
#[test]
fn authored_live_dispatch_binding_assignment_value_source_and_deferred_counterexamples() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for (op, n) in [("+", 0), ("-", 0), ("*", 1), ("/", 1), ("**", 1)] {
        let setup = format!(
            "$events=[]; class Authored; def {op}(n); $events << [:operator,n]; :changed; end; def inspect; ':original'; end; end; def alpha; $events << :read; Authored.new; end;"
        );
        let a = format!("{setup}p(alpha {op} {n}); p $events");
        let b = format!("{setup}p(alpha); p $events");
        let first = limited(&ruby, d.path(), &a, "-w");
        let second = limited(&ruby, d.path(), &b, "-w");
        success(&first);
        success(&second);
        assert_ne!(first.stdout, second.stdout);
        fs::write(d.path().join("safety.rb"), &a).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args([
                "--only",
                "Lint/UselessNumericOperation",
                "--fix",
                "safety.rb",
            ])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(fs::read_to_string(d.path().join("safety.rb")).unwrap(), a);
    }
    for (a, b) in [
        (
            "begin; x += 0; rescue => e; p e.class; end; p local_variables",
            "begin; x = x; rescue => e; p e.class; end; p local_variables",
        ),
        ("x='abc'; p((x * 1).equal?(x))", "x='abc'; p(x.equal?(x))"),
        (
            "x=nil; begin; p(x + 0); rescue => e; p e.class; end",
            "x=nil; p x",
        ),
        (
            "p RubyVM::InstructionSequence.compile('def f(x); x + 0; end').disasm",
            "p RubyVM::InstructionSequence.compile('def f(x); x; end').disasm",
        ),
        (
            "p \"if false\\n x + 0\\nend\\n\".bytes",
            "p \"if false\\n x\\nend\\n\".bytes",
        ),
    ] {
        let first = limited(&ruby, d.path(), a, "-w");
        let second = limited(&ruby, d.path(), b, "-w");
        success(&first);
        success(&second);
        assert_ne!(first.stdout, second.stdout, "{a}");
    }
}
