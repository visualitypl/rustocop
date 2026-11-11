//! real CLI policy and detection-only preservation of mutable values.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/MutableConstant";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn run(dir: &Path, args: &[&str], status: i32, count: usize) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|line| line.contains(COP))
            .count(),
        count,
        "{args:?}: {out:?}"
    );
    assert!(out.stderr.is_empty(), "{out:?}");
    out
}
#[test]
fn default_selectors_config_enabling_excluding_and_never_writing() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    // The blank line keeps Layout/EmptyLineAfterMagicComment quiet.
    let source = "# frozen_string_literal: true\n\nA = []\n";
    for (config, args, count) in [
        (String::new(), vec!["lib/a.rb"], 1),
        (String::new(), vec!["--only", COP, "--fix", "lib/a.rb"], 1),
        (String::new(), vec!["--except", COP, "lib/a.rb"], 0),
        (
            String::new(),
            vec!["--only", COP, "--except", COP, "--fix", "lib/a.rb"],
            0,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["lib/a.rb"],
            0,
        ),
        (
            "[cops.Style]\nenabled = false\n".into(),
            vec!["--only", COP, "lib/a.rb"],
            1,
        ),
        (
            format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
            vec!["lib/a.rb"],
            1,
        ),
        (
            format!("[cops.\"{COP}\"]\nexclude = ['lib/**']\n"),
            vec!["--only", COP, "--fix", "lib/a.rb"],
            0,
        ),
        (
            "exclude = ['lib/**']\n".into(),
            vec!["--only", COP, "--fix", "lib/a.rb"],
            0,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(dir.path(), &args, i32::from(count > 0), count);
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn directives_suppress_and_reenable_reports_without_freezing() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for prefix in ["rustocop", "rubocop"] {
        for directive in [COP, "Style", "all"] {
            let source = format!(
                "# {prefix}:disable {directive}\nA = []\n# {prefix}:enable {directive}\nB = []\n"
            );
            fs::write(&path, &source).unwrap();
            run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 1, 1);
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
        }
        for source in [
            format!("A = [] # {prefix}:disable {COP}\n"),
            format!("# {prefix}:disable-next {COP}\nA = []\n"),
        ] {
            fs::write(&path, &source).unwrap();
            run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0, 0);
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
        }
    }
}
#[test]
fn fixed_settings_are_not_public_configuration() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "A = []\n").unwrap();
    for setting in [
        "EnforcedStyle = 'strict'",
        "Recursive = true",
        "TargetRubyVersion = '2.7'",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{setting}\n"),
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--fix", "lib/a.rb"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("unknown"));
        assert_eq!(fs::read(&path).unwrap(), b"A = []\n");
    }
}
#[test]
fn unsafe_clean_error_and_encoded_data_files_keep_bytes_and_metadata() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, status, count) in [
        (b"A = [1, { a: [] }]\n".as_slice(), 1, 1),
        (b"A = [{ a: [] }].freeze\n", 0, 0),
        (b"A = [\n", 2, 0),
        (b"\xef\xbb\xbf# coding: utf-8\r\nA = [\"caf\xc3\xa9\", __LINE__]\r\n__END__\r\nB = []\xff\r\n", 1, 1),
        (b"# coding: ISO-8859-1\r\nA = '\xe9'\r\n__END__\r\nB = []\r\n", 1, 1),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop")).current_dir(dir.path()).args(["--only", COP, "--fix", "lib/a.rb"]).output().unwrap();
        assert_eq!(out.status.code(), Some(status), "{out:?}");
        assert_eq!(String::from_utf8_lossy(&out.stdout).matches(COP).count(), count, "{out:?}");
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn runtime_mutation_binding_order_and_control_flow_remain_observable() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "$events = []\nmodule Outer\n A = []\n private_constant :A\n module Inner\n  B = A\n  def self.value; B; end\n end\nend\nmodule Outer::Inner\n C ||= []\nend\nclass Holder\n class << self\n  D = {}\n end\nend\nmodule Mix\n def value; :mix; end\nend\nclass Holder\n include Mix\n extend Mix\nend\ndef item; $events << :item; 1; end\nX = [item]\nY = { key: [] }\nZ = \"text\"\nX << 2\nY[:key] << 3\nZ << '!'\nOuter::Inner.value << :nested\nOuter::Inner::C << :qualified\nHolder.singleton_class::D[:a] = :singleton\np [X, Y, Z, Outer::Inner.value, Outer::Inner::C, Holder.singleton_class::D, Holder.value, $events, catch(:done) { X << 4; throw :done, :exit }, __LINE__]\n";
    fs::write(&path, source).unwrap();
    let ruby = || {
        Command::new("ruby")
            .arg(&path)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .output()
    };
    let before = ruby();
    run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 1, 6);
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    if let Ok(before) = before {
        let after = ruby().unwrap();
        assert!(before.status.success(), "{before:?}");
        assert_eq!(before.status.code(), after.status.code());
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
        assert!(String::from_utf8_lossy(&after.stdout).contains("text!"));
    }
}
#[test]
fn style_layout_fixes_never_freeze_literals_and_stabilize() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "A = [ 'a',{:b => []} ];\nA[0] << '!'\nA[1][:b] << 2\np A\n";
    fs::write(&path, source).unwrap();
    let ruby = || {
        Command::new("ruby")
            .arg(&path)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .output()
    };
    let before = ruby();
    let only = "Style/MutableConstant,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout";
    let mut stable = false;
    for _ in 0..4 {
        let old = fs::read(&path).unwrap();
        run(dir.path(), &["--only", only, "--fix", "lib/a.rb"], 1, 1);
        if fs::read(&path).unwrap() == old {
            stable = true;
            break;
        }
    }
    assert!(stable, "at most three changing passes and a no-write check");
    let fixed = fs::read_to_string(&path).unwrap();
    assert!(!fixed.contains(".freeze"));
    assert!(fixed.contains("b: []"), "{fixed}");
    assert!(fixed.contains("\"a\""), "{fixed}");
    if let Ok(before) = before {
        let after = ruby().unwrap();
        assert!(before.status.success(), "{before:?}");
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
        assert_eq!(before.status.code(), after.status.code());
    }
}
#[test]
fn stdin_is_lint_only_with_path_excludes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "disk bytes").unwrap();
    for (config, count) in [("", 1), ("exclude = ['lib/**']\n", 0)] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"A = []\n").unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(count));
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).matches(COP).count(),
            count as usize
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "disk bytes");
    }
}
