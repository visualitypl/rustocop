//! unsafe suggestions never rewrite Ruby. Controlled fixtures only.
use crate::support::stdin;

use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/HashEachMethods";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(dir: &Path, args: &[&str], code: i32) -> Output {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(code), "{args:?}: {o:?}");
    assert!(o.stderr.is_empty() || code == 2, "{o:?}");
    o
}
fn unchanged(dir: &Path, s: &[u8], args: &[&str], code: i32) -> Output {
    let p = dir.join("a.rb");
    fs::write(&p, s).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    let o = cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), s);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
    o
}
#[test]
fn exact_registry_defaults_and_unsafe_capability() {
    let r = rustocop::cops::Registry::new();
    let c = r.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(c.default_enabled());
    assert!(!c.safe_fixes());
    assert_eq!(c.ruby_versions(), Default::default());
    assert!(c.path_policy().is_none());
    let d = project();
    let o = unchanged(d.path(), b"foo.keys.each { |k| p k }\n", &["a.rb"], 1);
    assert!(String::from_utf8_lossy(&o.stdout).contains(COP));
}
#[test]
fn selectors_config_excludes_and_no_new_knobs() {
    let d = project();
    let s = b"foo.keys.each { |k| p k }\n";
    unchanged(d.path(), s, &["--only", COP, "--fix", "a.rb"], 1);
    unchanged(
        d.path(),
        s,
        &["--only", COP, "--except", COP, "--fix", "a.rb"],
        0,
    );
    for config in [
        "[cops.Style]\nenabled = false\n".into(),
        format!("[cops.\"{COP}\"]\nenabled = false\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), s, &["--fix", "a.rb"], 0);
        unchanged(d.path(), s, &["--only", COP, "--fix", "a.rb"], 1);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
    )
    .unwrap();
    unchanged(d.path(), s, &["--fix", "a.rb"], 1);
    for config in [
        "exclude = [\"a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = [\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), s, &["--only", COP, "--fix", "a.rb"], 0);
    }
    for knob in [
        "AllowedReceivers = []",
        "SafeAutoCorrect = true",
        "TargetRubyVersion = 3.4",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{knob}\n"),
        )
        .unwrap();
        unchanged(d.path(), s, &["--fix", "a.rb"], 2);
    }
}
#[test]
fn directives_clean_errors_and_stdin_hold_bytes_and_mtime() {
    let d = project();
    for prefix in ["rubocop", "rustocop"] {
        for target in [COP, "HashEachMethods", "Style", "all"] {
            for s in [
                format!("# {prefix}:disable {target}\nfoo.keys.each {{ p it }}\n"),
                format!("# {prefix}:disable-next {target}\nfoo.keys.each {{ p it }}\n"),
                format!("foo.keys.each {{ p it }} # {prefix}:disable {target}\n"),
            ] {
                unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 0);
            }
        }
        let s = format!(
            "# {prefix}:disable {COP}\nfoo.keys.each {{ p it }}\n# {prefix}:enable {COP}\nfoo.keys.each {{ p it }}\n"
        );
        let o = unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 1);
        assert_eq!(
            String::from_utf8_lossy(&o.stdout)
                .matches(&format!("({COP})"))
                .count(),
            1
        );
    }
    for (s, code) in [("foo.each_key { p it }\n", 0), ("(\n", 2)] {
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            code,
        );
    }
    for fix in [false, true] {
        let p = d.path().join("a.rb");
        fs::write(&p, "disk bytes\n").unwrap();
        let time = fs::metadata(&p).unwrap().modified().unwrap();
        let mut c = Command::new(env!("CARGO_BIN_EXE_rustocop"));
        c.current_dir(d.path())
            .args(["--only", COP, "--stdin", "a.rb"]);
        if fix {
            c.arg("--fix");
        }
        let mut child = c
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        stdin::write_stdin(
            child.stdin.take().unwrap(),
            b"foo.keys.each { p it }\n",
            fix,
        )
        .unwrap();
        let o = child.wait_with_output().unwrap();
        assert_eq!(o.status.code(), Some(if fix { 2 } else { 1 }), "{o:?}");
        if fix {
            assert!(o.stdout.is_empty(), "{o:?}");
            assert!(
                String::from_utf8_lossy(&o.stderr)
                    .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
                "{o:?}"
            );
        } else {
            assert!(o.stderr.is_empty(), "{o:?}");
            assert!(String::from_utf8_lossy(&o.stdout).contains(COP), "{o:?}");
        }
        assert_eq!(fs::read(&p).unwrap(), b"disk bytes\n");
        assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
    }
}
#[test]
fn all_24_pinned_correction_calls_are_individually_detection_only() {
    // RuboCop style/hash_each_methods_spec.rb, MIT, (c) 2012-26 Bozhidar Batsov.
    let sources = [
        "foo.keys.each { |k| p k }",
        "foo&.keys&.each { |k| p k }",
        "foo.values.each { |v| p v }",
        "foo&.values&.each { |v| p v }",
        "foo.keys.each(&:bar)",
        "foo.values.each(&:bar)",
        "foo.each { |k, *v| do_something(*v) }",
        "foo.each { |*k, v| do_something(*k) }",
        "foo.each { |k, unused_value| do_something(k) }",
        "foo&.each { |k, unused_value| do_something(k) }",
        "foo.each { |k, (_, unused_value)| do_something(k) }",
        "foo.each { |unused_key, v| do_something(v) }",
        "foo.each { |(_, unused_key), v| do_something(v) }",
        "{hash: :literal}.keys.each { |k| p k }",
        "foo.keys.each do |key|\n  bar[\"#{key}_copy\"] = value\nend",
        "foo.values.each do |value|\n  bar[\"#{value}_copy\"] = value\nend",
        "foo.keys.each { p _1 }",
        "foo.keys.each { p it }",
        "{}.keys.each { |k| p k }",
        "{}.values.each { |k| p k }",
        "{}.keys.each(&:bar)",
        "{}&.keys&.each(&:bar)",
        "{}.values.each(&:bar)",
        "{}&.values&.each(&:bar)",
    ];
    assert_eq!(sources.len(), 24);
    let d = project();
    for s in sources {
        let o = unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(COP));
    }
}
#[test]
fn callbacks_keywords_positional_hashes_and_receiver_lookalikes_never_rewrite() {
    let d = project();
    for (s, code) in [
        ("unknown.keys.each { |k| callback(k) }", 1),
        ("array_like.each { |k,v| callback(k) }", 1),
        ("foo.each { |k,v:| callback(k, v: v) }", 1),
        ("foo.each { |k,**v| callback(k, **v) }", 1),
        ("foo.each { |k,&v| callback(k, &v) }", 1),
        ("foo.keys.each(callback: proc {}) { p it }", 0),
        ("foo.keys.each({callback: proc {}}) { p it }", 0),
        ("foo.keys.each(&callback)", 0),
        ("foo.keys.each(&nil)", 0),
        ("foo.each_pair { |k,v| callback(k) }", 0),
        ("Thread.current.keys.each(&:callback)", 0),
        ("Thread.currently.keys.each(&:callback)", 1),
        ("::Thread.current.keys.each(&:callback)", 1),
        ("Thread&.current.keys.each(&:callback)", 1),
    ] {
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            code,
        );
    }
}
#[test]
fn source_envelopes_line_data_heredocs_and_original_columns_are_unchanged() {
    let d = project();
    for s in [
        "\u{feff}# encoding: UTF-8\r\nhash.keys.each { |k| p [k,__LINE__] }\r\np DATA.read\r\n__END__\r\nraw\0\r\n".as_bytes().to_vec(),
        b"# encoding: ISO-8859-1\r\nhash.values.each { |v| p [v, \"caf\xe9\", __LINE__] }\r\n".to_vec(),
        b"hash.keys.each do |k|\n  p <<~TEXT\n    #{k} __LINE__\n  TEXT\nend\n".to_vec(),
        "é = 1; hash.keys().each() { p it }\n".as_bytes().to_vec(),
    ] { unchanged(d.path(), &s, &["--only", COP, "--fix", "a.rb"], 1); }
    let o = unchanged(
        d.path(),
        "é = 1; hash.keys().each() { p it }\n".as_bytes(),
        &["--only", COP, "a.rb"],
        1,
    );
    assert!(
        String::from_utf8_lossy(&o.stdout).contains("a.rb:1:13:"),
        "{o:?}"
    );
    assert!(String::from_utf8_lossy(&o.stdout).contains("instead of `keys().each()`"));
}
fn rubies() -> Vec<String> {
    let mut r = vec![];
    if Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        r.push("ruby".into());
    }
    if let Ok(path) = crate::support::runtime::ruby() {
        let o = Command::new(&path)
            .args(["-e", "print RUBY_VERSION"])
            .output()
            .unwrap();
        assert!(
            o.status.success()
                && o.stdout
                    .starts_with(crate::support::runtime::version_prefix().as_bytes())
        );
        r.push(path);
    }
    r
}
fn observe(ruby: &str, source: &str) -> Vec<u8> {
    let o = Command::new(ruby).args(["-e", source]).output().unwrap();
    assert!(o.status.success(), "{ruby}: {o:?}");
    assert!(o.stderr.is_empty(), "{o:?}");
    o.stdout
}
#[test]
fn controlled_runtime_preserves_defaults_order_mutation_dispatch_and_argument_channels() {
    let source = r#"
trace = []
h = Hash.new { |hash, key| trace << [:default, key]; hash[key] = :created }
h[:symbol] = 1
h["string"] = 2
h[[:dynamic]] = 3
h.keys.each { |key| trace << [:key, key]; h.delete(key) }
trace << h[:missing]
h.values.each { |value| trace << [:value, value] }
class Custom
  def keys; [:snapshot]; end
  def each_key; yield :different; end
end
Custom.new.keys.each { |key| trace << [:custom, key] }
class Channels
  def each(*args, **kwargs); yield :key, :value; end
end
Channels.new.each({positional: true}) { |key, value| trace << [:positional, key] }
Channels.new.each(keyword: true) { |key, value| trace << [:keyword, value] }
trace << [:remaining, h.to_a, h.default_proc.nil?]
p trace
"#;
    let expected = "[[:key, :symbol], [:key, \"string\"], [:key, [:dynamic]], [:default, :missing], :created, [:value, :created], [:custom, :snapshot], [:positional, :key], [:keyword, :value], [:remaining, [[:missing, :created]], false]]\n";
    let d = project();
    unchanged(
        d.path(),
        source.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        1,
    );
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    for ruby in rubies() {
        assert_eq!(observe(&ruby, source), expected.as_bytes());
        assert_eq!(observe(&ruby, &after), expected.as_bytes());
        assert_ne!(
            observe(
                &ruby,
                &source.replace("Custom.new.keys.each", "Custom.new.each_key")
            ),
            expected.as_bytes()
        );
    }
}
#[test]
fn shared_fix_composition_preserves_unsafe_file_and_corrects_safe_sibling() {
    let d = project();
    let source = b"unknown.keys.each { |key| consume(key) }\n";
    fs::write(d.path().join("safe.rb"), "value = 'x'\n").unwrap();
    unchanged(
        d.path(),
        source,
        &[
            "--only",
            "Style/HashEachMethods,Style/StringLiterals",
            "--fix",
            "a.rb",
            "safe.rb",
        ],
        1,
    );
    assert_eq!(
        fs::read(d.path().join("safe.rb")).unwrap(),
        b"value = \"x\"\n"
    );
}
