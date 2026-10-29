//! controlled Ruby Binding ownership and arity, independently of the
//! main-Prism/fixed-3.4 evidence tests. No user/corpus source is executed.
use std::{fs, process::Command};

#[test]
fn ruby_binding_block_channels_and_reflection_match_existing_liveness_consumer() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("case.rb");
    for (params, acquisition) in [
        ("", "binding"),
        ("", "binding(&nil)"),
        ("", "binding(&-> { raise 'not yielded' })"),
        ("", "binding { raise 'not yielded' }"),
        ("", "binding(&:to_s)"),
        ("&cb", "binding(&cb)"),
        ("&", "binding(&)"),
        ("...", "binding(...)"),
        ("", "self.binding(&nil)"),
        ("", "((::Kernel)).binding(&nil)"),
        ("", "send(:binding, &nil)"),
        ("", "Object.new.__send__('binding', &-> {})"),
        ("", "binding(*[])"),
        ("", "binding(**{})"),
        ("", "K.binding(&nil)"),
        ("", "((::K)).binding(&-> {})"),
        ("receiver = Kernel", "receiver.binding(&nil)"),
        ("", "(self.binding ||= nil)"),
        ("", "(Kernel.binding ||= nil)"),
        ("", "(K.binding ||= nil)"),
    ] {
        let source = format!(
            "K = Kernel\ndef f({params})\n  a = 2\n  {acquisition}.local_variable_get(:a)\nend\np f\n"
        );
        fs::write(&path, &source).unwrap();
        let out = Command::new("ruby")
            .args(["--disable-gems"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(out.status.success(), "{source}: {out:?}");
        assert_eq!(out.stdout, b"2\n", "{source}");
        assert!(out.stderr.is_empty(), "{out:?}");
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", "Lint/UselessAssignment", "--fix", "case.rb"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{source}: {out:?}");
        assert!(out.stderr.is_empty(), "{out:?}");
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
    // In particular Method#call cannot manufacture a caller Binding. A method
    // getter is correction uncertainty, not proof that a's value was read.
    for (acquisition, exception) in [
        ("binding(nil)", "ArgumentError"),
        ("binding(1, &nil)", "ArgumentError"),
        ("binding(&1)", "TypeError"),
        ("method(:binding).call", "RuntimeError"),
        ("Kernel.method(:binding).call", "RuntimeError"),
        ("Kernel.public_send(:binding, &nil)", "RuntimeError"),
        ("public_send(:binding)", "NoMethodError"),
        ("Object.new.binding", "NoMethodError"),
    ] {
        let source = format!(
            "def f\n  a = 2\n  {acquisition}\nrescue StandardError => e\n  puts e.class\nend\nf\n"
        );
        fs::write(&path, &source).unwrap();
        let out = Command::new("ruby")
            .arg("--disable-gems")
            .arg(&path)
            .output()
            .unwrap();
        assert!(out.status.success(), "{source}: {out:?}");
        assert_eq!(out.stdout, format!("{exception}\n").as_bytes(), "{source}");
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", "Lint/UselessAssignment", "case.rb"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{source}: {out:?}");
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("case.rb:2:3:"),
            "{out:?}"
        );
    }
}
