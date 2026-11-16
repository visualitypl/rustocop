//! Authored bounded runtime witnesses ONLY; no reference/corpus source executes.
use std::{fs, process::Command};
const COP: &str = "Lint/SendWithMixinArgument";
fn ruby() -> Option<String> {
    crate::support::runtime::ruby().ok()
}
#[test]
fn closed_canonical_preserves_vm_warnings_events_value_and_bindings() {
    if crate::support::runtime::skips(
        "pins compiled code or VM state that Ruby 4.0 builds differently",
    ) {
        return;
    }
    let Some(ruby) = ruby() else {
        return;
    };
    let driver = r##"
source = "if false\n  #{ARGV[0]}(:#{ARGV[1]}, Bar, Baz)\nend\n"
source = source.sub("#{ARGV[0]}(:#{ARGV[1]}, Bar, Baz)", "#{ARGV[1]} Bar, Baz") if ARGV[2] == 'after'
iseq = RubyVM::InstructionSequence.compile(source, 'closed.rb', 'closed.rb', 1)
events = []
trace = TracePoint.new(:line, :call, :return, :raise, :c_call, :c_return) { |t| events << [t.event,t.lineno,t.method_id] if t.path == 'closed.rb' }
trace.enable
result = iseq.eval
trace.disable
p [iseq.to_a,result,events,local_variables.sort]
"##;
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for receiver in ["", "self.", "Foo.", "::Foo.", "A::Foo."] {
        for dispatch in ["send", "public_send", "__send__"] {
            for method in ["include", "prepend", "extend"] {
                let name = format!("{receiver}{dispatch}");
                let evaluate = |mode| {
                    Command::new(&ruby)
                        .args(["--parser=prism", "-w", "-e", driver, &name, method, mode])
                        .output()
                        .unwrap()
                };
                let before = evaluate("before");
                let after = evaluate("after");
                assert!(
                    before.status.success() && after.status.success(),
                    "{name}/{method}: {before:?} {after:?}"
                );
                assert_eq!(before.stdout, after.stdout, "{name}/{method}");
                assert_eq!(before.stderr, after.stderr, "{name}/{method}");
                let source = format!("if false\n  {name}(:{method}, Bar, Baz)\nend\n");
                let path = dir.path().join("closed.rb");
                fs::write(&path, &source).unwrap();
                let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                    .current_dir(dir.path())
                    .args(["--only", COP, "--fix", "closed.rb"])
                    .output()
                    .unwrap();
                assert_eq!(out.status.code(), Some(0), "{out:?}");
                assert_eq!(
                    fs::read_to_string(path).unwrap(),
                    format!("if false\n  {receiver}{method} Bar, Baz\nend\n")
                );
            }
        }
    }
}
