//! controlled ordinary formatting fixtures only. Command literals are
//! source/AST-only and are NEVER executed (including through runtime helpers).
use crate::support::cli::project;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/FormatString";
fn cli(dir: &Path, args: &[&str], status: i32) -> Output {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(status), "{args:?}: {o:?}");
    o
}
fn unchanged(dir: &Path, source: &[u8], status: i32) {
    fs::write(dir.join("a.rb"), source).unwrap();
    let mtime = fs::metadata(dir.join("a.rb")).unwrap().modified().unwrap();
    cli(dir, &["--only", COP, "--fix", "a.rb"], status);
    assert_eq!(fs::read(dir.join("a.rb")).unwrap(), source);
    assert_eq!(
        fs::metadata(dir.join("a.rb")).unwrap().modified().unwrap(),
        mtime
    );
}
fn fix(dir: &Path, source: &[u8], status: i32) -> Vec<u8> {
    fs::write(dir.join("a.rb"), source).unwrap();
    cli(dir, &["--only", COP, "--fix", "a.rb"], status);
    fs::read(dir.join("a.rb")).unwrap()
}
#[test]
fn default_on_lint_stdin_and_selection_no_write() {
    let registry = rustocop::cops::Registry::new();
    let c = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(c.default_enabled() && c.safe_fixes());
    assert_eq!(c.ruby_versions(), Default::default());
    assert!(c.path_policy().is_none());
    let d = project();
    let source = b"p \"%d\" % 1\n";
    fs::write(d.path().join("a.rb"), source).unwrap();
    let mtime = fs::metadata(d.path().join("a.rb"))
        .unwrap()
        .modified()
        .unwrap();
    for args in [vec!["a.rb"], vec!["--only", COP, "a.rb"]] {
        let o = cli(d.path(), &args, 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(COP));
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", COP, "--stdin", "a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    assert_eq!(child.wait_with_output().unwrap().status.code(), Some(1));
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), source);
    assert_eq!(
        fs::metadata(d.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap(),
        mtime
    );
}
#[test]
fn config_only_except_and_canonical_idempotence() {
    let d = project();
    for config in [
        "[cops.Style]\nenabled = false\n".into(),
        format!("[cops.\"{COP}\"]\nenabled = false\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let source = b"p \"%d\" % 1\n";
        fs::write(d.path().join("a.rb"), source).unwrap();
        cli(d.path(), &["--fix", "a.rb"], 0);
        cli(
            d.path(),
            &["--only", COP, "--except", COP, "--fix", "a.rb"],
            0,
        );
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), source);
        assert_eq!(fix(d.path(), source, 0), b"p format(\"%d\", 1)\n");
        unchanged(d.path(), b"p format(\"%d\", 1)\n", 0);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
    )
    .unwrap();
    fs::write(d.path().join("a.rb"), b"sprintf(\"%s\", 1)\n").unwrap();
    cli(d.path(), &["--fix", "a.rb"], 0);
    assert_eq!(
        fs::read(d.path().join("a.rb")).unwrap(),
        b"format(\"%s\", 1)\n"
    );
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.\"{COP}\"]\nEnforcedStyle = \"percent\"\n"),
    )
    .unwrap();
    unchanged(d.path(), b"sprintf(\"%s\", 1)\n", 2);
}
#[test]
fn directives_excludes_clean_errors_and_withheld_keep_bytes_mtime() {
    let d = project();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "FormatString", "Style", "all"] {
            for s in [
                format!("# {prefix}:disable {target}\n\"%s\" % 1\n"),
                format!("# {prefix}:disable-next {target}\n\"%s\" % 1\n"),
                format!("\"%s\" % 1 # {prefix}:disable {target}\n"),
            ] {
                unchanged(d.path(), s.as_bytes(), 0);
            }
        }
    }
    for (source, status) in [
        (b"format(\"%s\", 1)\n".as_slice(), 0),
        (b"sprintf(\n", 2),
        (b"\"%s\" % value\n", 1),
    ] {
        unchanged(d.path(), source, status);
    }
    for config in [
        "exclude = [\"a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = [\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), b"\"%s\" % 1\n", 0);
    }
}
fn ruby(executable: &str, code: &str) -> Output {
    // All callers use controlled ordinary fixtures, NEVER command literals.
    Command::new(executable)
        .args(["-e", code])
        .output()
        .unwrap()
}
fn has_ruby() -> bool {
    Command::new("ruby").arg("--version").output().is_ok()
}
#[test]
fn fixed_34_literal_payload_flags_and_lexical_binding_evidence() {
    use rustocop::cops::walk::{NodeVisitor, walk};
    fn facts(source: &[u8]) -> Vec<String> {
        struct Facts(Vec<String>);
        impl<'pr> NodeVisitor<'pr> for Facts {
            fn enter(&mut self, n: ruby_prism::Node<'pr>) {
                if let Some(s) = n.as_string_node() {
                    self.0.push(format!(
                        "str:{:?}:{}:{}:{}:{}",
                        s.unescaped(),
                        s.is_forced_utf8_encoding(),
                        s.is_forced_binary_encoding(),
                        s.is_frozen(),
                        s.is_mutable()
                    ));
                }
                if let Some(s) = n.as_local_variable_read_node() {
                    self.0
                        .push(format!("read:{:?}:{}", s.name().as_slice(), s.depth()));
                }
                if let Some(s) = n.as_local_variable_write_node() {
                    self.0
                        .push(format!("write:{:?}:{}", s.name().as_slice(), s.depth()));
                }
            }
        }
        let parsed = ruby_prism::parse(source);
        assert_eq!(parsed.errors().count(), 0);
        let mut facts = Facts(vec![]);
        walk(&mut facts, &parsed.node());
        facts.0
    }
    let d = project();
    for header in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
    ] {
        for expr in [
            "\"%s\" % \"é\"",
            "\"%s/%s\" % [\"a\", \"\\xff\"]",
            "sprintf(\"%s\", \"x\")",
            "\"%<a>s\" % {a: \"x\"}",
        ] {
            let source = format!("{header}result = {expr}\np result\n");
            let after = fix(d.path(), source.as_bytes(), 0);
            assert_ne!(source.as_bytes(), after);
            assert_eq!(facts(source.as_bytes()), facts(&after), "{source}");
        }
    }
}
// Parse only, with an explicit 3.4 target independent of the host Ruby runtime.
#[test]
fn runtime_actual_defaults_and_selected_peers_preserve_values_each_pass() {
    if !has_ruby() {
        return;
    }
    let d = project();
    let source = "values = [sprintf('%s', ?x), '%s' % [%Q(hi)], '%<a>s' % {:a => 'x'}, %W(a b), %q(hi)]\np values\n";
    let before = ruby("ruby", source);
    assert!(before.status.success());
    for selected in [
        None,
        Some(
            "Style/FormatString,Style/StringLiterals,Style/HashSyntax,Style/RedundantPercentQ,Style/RedundantCapitalW,Style/CharacterLiteral,Style/PercentLiteralDelimiters,Layout",
        ),
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let mut stable = false;
        for _ in 0..6 {
            let old = fs::read(d.path().join("a.rb")).unwrap();
            let mut args = vec!["--fix", "a.rb"];
            if let Some(names) = selected {
                args.extend(["--only", names]);
            }
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(args)
                .output()
                .unwrap();
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
            let observed = ruby("ruby", &after);
            assert_eq!(before.stdout, observed.stdout, "{selected:?}: {after}");
            assert_eq!(before.stderr, observed.stderr);
            assert_eq!(before.status, observed.status);
            if after.as_bytes() == old {
                stable = true;
                break;
            }
        }
        assert!(stable);
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert!(!after.contains("sprintf"), "{after}");
        assert!(after.contains("format("), "{after}");
    }
}
