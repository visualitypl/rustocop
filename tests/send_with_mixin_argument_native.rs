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
#[test]
fn authored_autoload_argument_evaluation_and_hook_dispatch_are_not_assumed_pure() {
    let Some(ruby) = ruby() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    // A controlled native autoload target, never an application/library input.
    fs::write(
        dir.path().join("mixin.rb"),
        "module Foo::Bar; def self.included(base); puts :hook; end; end; puts :loaded\n",
    )
    .unwrap();
    let source = "module Foo; autoload :Bar, File.expand_path('mixin.rb', Dir.pwd); def self.send(*); puts :intercepted; :value; end; end; p Foo.send(:include, Foo::Bar)\n";
    let evaluate = |s: &str| {
        Command::new(&ruby)
            .current_dir(dir.path())
            .args(["--parser=prism", "-W0", "-e", s])
            .output()
            .unwrap()
    };
    let before = evaluate(source);
    let after = evaluate(&source.replace("send(:include, Foo::Bar)", "include Foo::Bar"));
    assert!(
        before.status.success() && after.status.success(),
        "{before:?} {after:?}"
    );
    assert!(before.stdout.starts_with(b"loaded\nintercepted\n"));
    assert!(after.stdout.starts_with(b"loaded\nhook\n"));
    fs::write(dir.path().join("probe.rb"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", COP, "--fix", "probe.rb"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(
        fs::read_to_string(dir.path().join("probe.rb")).unwrap(),
        source
    );
}
#[test]
fn ordinary_dispatch_visibility_hooks_exception_value_and_control_flow_rewrites_are_held() {
    let Some(ruby) = ruby() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    // These are separately authored controlled programs, never copied fixtures.
    // Indirect-dispatch overrides, private targets, hook/value differences,
    // const_missing evaluation, argument effects, warnings and exceptions.
    for (source, bad, good) in [
        (
            "module Bar; def self.included(base); puts :hook; end; end; module Foo; def self.send(*); :intercepted; end; end; p Foo.send(:include, Bar)\n",
            "send(:include, Bar)",
            "include Bar",
        ),
        (
            "module Bar; def self.prepended(base); puts :hook; end; end; module Foo; def self.public_send(*); :intercepted; end; end; p Foo.public_send(:prepend, Bar)\n",
            "public_send(:prepend, Bar)",
            "prepend Bar",
        ),
        (
            "module Bar; def self.extended(base); puts :hook; end; end; module Foo; def self.__send__(*); :intercepted; end; end; p Foo.__send__(:extend, Bar)\n",
            "__send__(:extend, Bar)",
            "extend Bar",
        ),
        (
            "module Bar; end; module Foo; class << self; private; def include(*); puts :private; :ok; end; end; end; begin; p Foo.send(:include, Bar); rescue NoMethodError; puts :raised; end\n",
            "send(:include, Bar)",
            "include Bar",
        ),
        (
            "module Bar; end; module Foo; def self.send(*); raise ArgumentError; end; end; begin; p Foo.send(:extend, Bar); rescue ArgumentError; puts :raised; end\n",
            "send(:extend, Bar)",
            "extend Bar",
        ),
        (
            "module Bar; def self.included(base); raise ArgumentError; end; end; module Foo; def self.send(*); puts :skipped; end; end; begin; Foo.send(:include, Bar); puts :returned; rescue ArgumentError; puts :raised; end\n",
            "send(:include, Bar)",
            "include Bar",
        ),
        (
            "$events=[]; module Bar; end; module Foo; def self.const_missing(n); $events << n; Bar; end; def self.send(*); $events << :dispatch; :value; end; end; p Foo.send(:include, Foo::Absent); p $events\n",
            "send(:include, Foo::Absent)",
            "include Foo::Absent",
        ),
        (
            "module Bar; end; module Foo; def self.send(*); warn 'indirect'; :value; end; end; p Foo.send(:prepend, Bar)\n",
            "send(:prepend, Bar)",
            "prepend Bar",
        ),
    ] {
        let evaluate = |text: &str| {
            Command::new(&ruby)
                .args(["--parser=prism", "-W0", "-e", text])
                .output()
                .unwrap()
        };
        let before = evaluate(source);
        let after = evaluate(&source.replace(bad, good));
        assert!(
            before.status.success() && after.status.success(),
            "{before:?} {after:?}"
        );
        assert_ne!(
            (&before.stdout, &before.stderr),
            (&after.stdout, &after.stderr),
            "{source}"
        );
        let path = dir.path().join("probe.rb");
        fs::write(&path, source).unwrap();
        let old = fs::metadata(&path).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--fix", "probe.rb"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(
            old.modified().unwrap(),
            fs::metadata(path).unwrap().modified().unwrap()
        );
    }
}
