//! real selector/config/version and safe-correction contracts.
use rustocop::{
    config::ResolvedConfig,
    cops::{Registry, policy::RubyVersion, select_active},
    pipeline::lint_one,
};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const THEN: &str = "Style/ObjectThen";
const REVERSE: &str = "Style/ReverseFind";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(dir: &Path, args: &[&str], status: i32) -> Output {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(status), "{args:?}: {o:?}");
    o
}
fn unchanged(dir: &Path, bytes: &[u8], args: &[&str], status: i32) {
    let path = dir.join("a.rb");
    fs::write(&path, bytes).unwrap();
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir, args, status);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
}
#[test]
fn registry_defaults_gates_and_real_pipeline_positions() {
    let r = Registry::new();
    let config = ResolvedConfig::default();
    for (name, minimum, fixes) in [
        (THEN, RubyVersion(2, 6), true),
        (REVERSE, RubyVersion(4, 0), false),
    ] {
        let c = r.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(c.default_enabled());
        assert_eq!(c.safe_fixes(), fixes);
        assert_eq!(c.ruby_versions().minimum, Some(minimum));
        assert_eq!(c.ruby_versions().maximum, None);
        assert!(c.path_policy().is_none());
    }
    let active = select_active(&r, &config, &[], &[]);
    assert!(active.iter().any(|c| c.name() == THEN));
    assert!(!active.iter().any(|c| c.name() == REVERSE));
    let active = select_active(&r, &config, &[THEN.into(), REVERSE.into()], &[]);
    let report = lint_one(
        Path::new("a.rb"),
        Path::new("a.rb"),
        Some(b"obj.yield_self { it }\narray.reverse.find { it }\n"),
        &active,
        &config,
    );
    assert!(report.errors.is_empty() && report.internal_errors.is_empty());
    let o = &report.result.offenses;
    assert_eq!(o.len(), 1);
    assert_eq!(
        (o[0].line, o[0].col, o[0].message.as_str()),
        (1, 5, "Prefer `then` over `yield_self`.")
    );
    assert!(o[0].correction.is_none());
}
#[test]
fn selectors_enabling_excludes_and_version_inactivity() {
    let d = project();
    let source = b"obj.yield_self { |x| x }\n";
    unchanged(d.path(), source, &["a.rb"], 1);
    unchanged(d.path(), source, &["--only", THEN, "a.rb"], 1);
    unchanged(
        d.path(),
        source,
        &["--only", THEN, "--except", THEN, "--fix", "a.rb"],
        0,
    );
    for config in [
        "[cops.Style]\nenabled=false\n".into(),
        format!("[cops.\"{THEN}\"]\nenabled=false\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), source, &["--fix", "a.rb"], 0);
        fs::write(d.path().join("a.rb"), source).unwrap();
        cli(d.path(), &["--only", THEN, "--fix", "a.rb"], 0);
        assert_eq!(
            fs::read(d.path().join("a.rb")).unwrap(),
            b"obj.then { |x| x }\n"
        );
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled=false\n[cops.\"{THEN}\"]\nenabled=true\n"),
    )
    .unwrap();
    unchanged(d.path(), source, &["a.rb"], 1);
    for config in [
        "exclude=[\"a.rb\"]\n".into(),
        format!("[cops.\"{THEN}\"]\nexclude=[\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), source, &["--only", THEN, "--fix", "a.rb"], 0);
    }
    for name in [THEN, REVERSE] {
        for key in [
            "EnforcedStyle='yield_self'",
            "TargetRubyVersion=4.0",
            "max=1",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{name}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(d.path(), source, &["--fix", "a.rb"], 2);
        }
    }
    for config in ["".into(), format!("[cops.\"{REVERSE}\"]\nenabled=true\n")] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        for args in [
            vec!["a.rb"],
            vec!["--only", REVERSE, "a.rb"],
            vec!["--only", REVERSE, "--fix", "a.rb"],
        ] {
            unchanged(d.path(), b"array.reverse.find { it }\n", &args, 0);
        }
    }
}
#[test]
fn directives_errors_and_clean_files_never_write() {
    let d = project();
    for prefix in ["rustocop", "rubocop"] {
        for name in [THEN, "ObjectThen", "Style", "all"] {
            for source in [
                format!("# {prefix}:disable {name}\nobj.yield_self {{ it }}\n"),
                format!("obj.yield_self {{ it }} # {prefix}:todo {name}\n"),
            ] {
                unchanged(
                    d.path(),
                    source.as_bytes(),
                    &["--only", THEN, "--fix", "a.rb"],
                    0,
                );
            }
        }
        let source = format!(
            "# {prefix}:disable {THEN}\na.yield_self {{ it }}\n# {prefix}:enable {THEN}\nb.yield_self {{ it }}\n"
        );
        fs::write(d.path().join("a.rb"), &source).unwrap();
        cli(d.path(), &["--only", THEN, "--fix", "a.rb"], 0);
        assert_eq!(
            fs::read_to_string(d.path().join("a.rb")).unwrap(),
            source.replace("b.yield_self", "b.then")
        );
    }
    for (source, code) in [
        (b"obj.then { it }\n".as_slice(), 0),
        (b"(\n", 2),
        (b"obj.yield_self\n", 0),
    ] {
        unchanged(d.path(), source, &["--only", THEN, "--fix", "a.rb"], code);
    }
}
#[test]
fn selector_only_edits_preserve_envelopes_and_stabilize() {
    let d = project();
    for source in [
        b"\xef\xbb\xbf# encoding: UTF-8\r\n# before\r\nobj.yield_self { |x| [x, __LINE__, \"\xc3\xa9\"] } # after\r\n__END__\r\npayload\xff\r\n".as_slice(),
        b"# encoding: ISO-8859-1\nobj.yield_self { |x| [x, \"\xe9\"] }\n",
        b"def f\n  yield_self do |x|\n    x\n  end\nend\n",
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        cli(d.path(), &["--only", THEN, "--fix", "a.rb"], 0);
        let at = source.windows(10).position(|w| w == b"yield_self").unwrap();
        let mut expected = source[..at].to_vec();
        expected.extend_from_slice(if source[at.saturating_sub(1)] == b'.' { b"then" } else { b"self.then" });
        expected.extend_from_slice(&source[at+10..]);
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), expected);
        unchanged(d.path(), &expected, &["--only", THEN, "--fix", "a.rb"], 0);
    }
}
#[test]
fn observable_source_and_data_positions_retain_diagnostics_without_edits() {
    let d = project();
    for source in [
        "p DATA.pos\nobj.yield_self { it }\n__END__\npayload\n",
        "p File.read(__FILE__)\nobj.yield_self { it }\n",
        "def then; :different; end\nobj.yield_self { it }\n",
        "obj.yield_self(&:test)\n",
        "obj.yield_self(&block)\n",
    ] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", THEN, "--fix", "a.rb"],
            1,
        );
    }
}
fn observe(dir: &Path) -> Option<Vec<u8>> {
    let ruby = crate::support::runtime::ruby().unwrap_or_else(|_| "ruby".into());
    let output = match Command::new(&ruby).current_dir(dir).arg("a.rb").output() {
        Ok(output) => output,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => panic!("{e}"),
    };
    assert!(output.status.success(), "{output:?}");
    Some(output.stdout)
}
#[test]
fn runtime_preserves_receiver_count_binding_control_flow_visibility_and_equality() {
    // Controlled fixtures only; never execute corpus/application code.
    let d = project();
    let source = r#"$events = []
def produce
  $events << :receiver
  [1, 2]
end
value = produce.yield_self { |x| $events << :block; [x.equal?(x), x] }
nil_value = nil&.yield_self { raise "not called" }
broken = produce.yield_self { break :stopped }
next_value = 2.yield_self { |x| next x + 1 }
proc_value = -> { produce.yield_self { return :returned }; :wrong }.call
lazy = [1, 2].lazy.yield_self { |x| x.equal?(x) }
def implicit
  yield_self { |x| x.equal?(self) }
end
p [value, nil_value, broken, next_value, proc_value, lazy, implicit, $events]
"#;
    fs::write(d.path().join("a.rb"), source).unwrap();
    let before = observe(d.path());
    cli(d.path(), &["--only", THEN, "--fix", "a.rb"], 0);
    assert!(
        !fs::read_to_string(d.path().join("a.rb"))
            .unwrap()
            .contains("yield_self")
    );
    if let Some(before) = before {
        assert_eq!(
            String::from_utf8_lossy(&before),
            "[[true, [1, 2]], nil, :stopped, 3, :returned, true, true, [:receiver, :block, :receiver, :receiver]]\n"
        );
        assert_eq!(observe(d.path()).unwrap(), before);
    }
}
#[test]
fn mixed_safe_style_and_layout_edits_compose_and_stabilize() {
    let d = project();
    let before = "value = 'x'.yield_self { |x| { :answer => x } };puts value\n";
    let after = "value = \"x\".then { |x| { answer: x } }\nputs value\n";
    let only = "Style/ObjectThen,Style/StringLiterals,Style/HashSyntax,Style/Semicolon,Layout/TrailingWhitespace";
    fs::write(d.path().join("a.rb"), before).unwrap();
    let observed = observe(d.path());
    cli(d.path(), &["--only", only, "--fix", "a.rb"], 0);
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
    if let Some(observed) = observed {
        assert_eq!(observe(d.path()).unwrap(), observed);
    }
    unchanged(
        d.path(),
        after.as_bytes(),
        &["--only", only, "--fix", "a.rb"],
        0,
    );
}

#[test]
fn custom_method_visibility_remains_unchanged_at_runtime() {
    let d = project();
    let source = "class X\n  private\n  def then; :different; end\n  public\n  def run; yield_self { |x| x.equal?(self) }; end\nend\np X.new.run\n";
    unchanged(
        d.path(),
        source.as_bytes(),
        &["--only", THEN, "--fix", "a.rb"],
        1,
    );
    if let Some(output) = observe(d.path()) {
        assert_eq!(output, b"true\n");
    }
}
