//! unsafe hash suggestions never modify Ruby.
use crate::support::cli::project;
use crate::support::stdin;
use serde::Deserialize;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 3] = [
    "Style/HashTransformKeys",
    "Style/HashTransformValues",
    "Style/PreferredHashMethods",
];
const SOURCES: [&str; 3] = [
    "{}.to_h { |k, v| [k.to_s, v] }\n",
    "{}.to_h { |k, v| [k, v.to_s] }\n",
    "{}.has_key?(:a)\n",
];
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
fn exact_default_registration_versions_and_unsafe_capabilities() {
    let registry = rustocop::cops::Registry::new();
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        let c = registry.cops().iter().find(|c| c.name() == *cop).unwrap();
        assert!(c.default_enabled());
        assert!(!c.safe_fixes());
        assert!(c.path_policy().is_none());
        let versions = c.ruby_versions();
        use rustocop::cops::policy::RubyVersion;
        assert_eq!(
            versions.minimum,
            [Some(RubyVersion(2, 5)), Some(RubyVersion(2, 4)), None][i]
        );
        assert_eq!(versions.maximum, None);
        let o = unchanged(d.path(), SOURCES[i].as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
    }
}
#[test]
fn selectors_config_excludes_and_fixed_policy_rejection() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        let s = SOURCES[i].as_bytes();
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        unchanged(d.path(), s, &["--only", cop, "--fix", "a.rb"], 1);
        unchanged(
            d.path(),
            s,
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            "[cops.Style]\nenabled = false\n".into(),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s, &["--fix", "a.rb"], 0);
            unchanged(d.path(), s, &["--only", cop, "--fix", "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        unchanged(d.path(), s, &["--fix", "a.rb"], 1);
        for config in [
            "exclude = [\"a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = [\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s, &["--only", cop, "--fix", "a.rb"], 0);
        }
        for knob in [
            "EnforcedStyle = \"verbose\"",
            "SafeAutoCorrect = true",
            "TargetRubyVersion = 3.4",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{knob}\n"),
            )
            .unwrap();
            unchanged(d.path(), s, &["--fix", "a.rb"], 2);
        }
    }
}
#[test]
fn both_directives_clean_and_error_files_hold_bytes_and_mtime() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        for prefix in ["rubocop", "rustocop"] {
            for target in [*cop, cop.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for s in [
                    format!("# {prefix}:disable {target}\n{}", SOURCES[i]),
                    format!("# {prefix}:disable-next {target}\n{}", SOURCES[i]),
                    format!("{} # {prefix}:disable {target}\n", SOURCES[i].trim_end()),
                ] {
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
                }
            }
            let s = format!(
                "# {prefix}:disable {cop}\n{}# {prefix}:enable {cop}\n{}",
                SOURCES[i], SOURCES[i]
            );
            let o = unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
            assert_eq!(
                String::from_utf8_lossy(&o.stdout)
                    .matches(&format!("({cop})"))
                    .count(),
                1
            );
        }
        for (s, code) in [("{}.transform_keys(&:to_s)\n", 0), ("(\n", 2)] {
            unchanged(
                d.path(),
                s.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                code,
            );
        }
    }
}
#[test]
fn stdin_reports_without_touching_disk_and_fix_stdin_is_rejected() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        for fix in [false, true] {
            let p = d.path().join("a.rb");
            fs::write(&p, "disk bytes\n").unwrap();
            let time = fs::metadata(&p).unwrap().modified().unwrap();
            let mut c = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            c.current_dir(d.path())
                .args(["--only", cop, "--stdin", "a.rb"]);
            if fix {
                c.arg("--fix");
            }
            let mut child = c
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            stdin::write_stdin(child.stdin.take().unwrap(), SOURCES[i].as_bytes(), fix).unwrap();
            let o = child.wait_with_output().unwrap();
            assert_eq!(o.status.code(), Some(if fix { 2 } else { 1 }), "{o:?}");
            if fix {
                assert!(o.stdout.is_empty());
                assert!(String::from_utf8_lossy(&o.stderr).contains("cannot be used with '--fix'"));
            } else {
                assert!(o.stderr.is_empty());
                assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
            }
            assert_eq!(fs::read(&p).unwrap(), b"disk bytes\n");
            assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
        }
    }
}
#[derive(Deserialize)]
struct Case {
    cop: String,
    source: String,
    positions: Vec<(u32, u32)>,
}
#[test]
fn all_85_pinned_examples_including_37_unsafe_correction_calls_never_write() {
    // MIT, (c) 2012-26 Bozhidar Batsov, RuboCop's three assigned specs.
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/hash_transform_pinned.json")).unwrap();
    assert_eq!(cases.len(), 85);
    assert_eq!(cases.iter().filter(|c| !c.positions.is_empty()).count(), 37);
    let d = project();
    for c in cases {
        for fix in [false, true] {
            let mut args = vec!["--only", c.cop.as_str(), "a.rb"];
            if fix {
                args.push("--fix");
            }
            let o = unchanged(
                d.path(),
                c.source.as_bytes(),
                &args,
                i32::from(!c.positions.is_empty()),
            );
            assert_eq!(
                String::from_utf8_lossy(&o.stdout)
                    .matches(&format!("({})", c.cop))
                    .count(),
                c.positions.len(),
                "{}",
                c.source
            );
        }
    }
}
#[test]
fn source_envelopes_data_line_encodings_and_heredocs_remain_exact() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        let mut envelopes = vec![
            format!(
                "\u{feff}# encoding: UTF-8\r\n{}p [__LINE__, DATA.read]\r\n__END__\r\nraw\0\r\n",
                SOURCES[i].replace('\n', "\r\n")
            )
            .into_bytes(),
            format!(
                "# encoding: ISO-8859-1\r\n{}p [__LINE__, \"caf",
                SOURCES[i].replace('\n', "\r\n")
            )
            .into_bytes(),
        ];
        envelopes[1].extend_from_slice(b"\xe9\"]\r\n");
        for s in envelopes {
            unchanged(d.path(), &s, &["--only", cop, "--fix", "a.rb"], 1);
        }
    }
    for (cop, s) in [
        (
            COPS[0],
            "{}.each_with_object({}) { |(k,v),h| h[<<~TEXT] = v }\n#{k.to_s} #{__LINE__}\nTEXT\n",
        ),
        (
            COPS[1],
            "{}.to_h { |k,v| [k, <<~TEXT] }\n#{v.to_s} #{__LINE__}\nTEXT\n",
        ),
        (COPS[2], "{}.has_key?(<<~TEXT)\n#{__LINE__}\nTEXT\n"),
    ] {
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
    }
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
fn observe(ruby: &str, s: &str) -> Vec<u8> {
    let o = Command::new(ruby).args(["-e", s]).output().unwrap();
    assert!(o.status.success() && o.stderr.is_empty(), "{ruby}: {o:?}");
    o.stdout
}
#[test]
fn controlled_runtime_preserves_order_defaults_collisions_dynamic_keys_and_dispatch() {
    let source = r#"
trace = []
h = Hash.new { |hash,key| trace << [:default,key]; hash[key] = :created }
h[:symbol] = 1
h["string"] = 2
h[[:dynamic]] = 3
out = h.to_h.each_with_object({}) { |(k,v),memo| memo[(trace << [:key,k]; k.to_s)] = v }
trace << [:out,out.to_a,out.default_proc.nil?,h.default_proc.nil?]
trace << h[:missing]
trace << h.to_h.map { |k,v| [k, (trace << [:value,v]; v.to_s)] }.to_h.to_a
trace << {a: 1, "a" => 2}.to_h { |k,v| [k.to_s,v] }.to_a
trace << h.has_key?(:absent)
trace << h.has_value?(:created)
class Custom
  def to_h; self; end
  def map; [yield(:custom, 7)]; end
  def has_key?(key); [:custom,key]; end
  def key?(key); [:different,key]; end
end
trace << Custom.new.to_h.map { |k,v| [k.to_s,v] }.to_h.to_a
trace << Custom.new.has_key?(:missing)
class Channels
  def has_key?(*args,**kwargs); [args.map { |h| h.to_a },kwargs.to_a]; end
end
trace << Channels.new.has_key?({positional: true})
trace << Channels.new.has_key?(keyword: true)
p trace
"#;
    let expected = "[[:key, :symbol], [:key, \"string\"], [:key, [:dynamic]], [:out, [[\"symbol\", 1], [\"string\", 2], [\"[:dynamic]\", 3]], true, false], [:default, :missing], :created, [:value, 1], [:value, 2], [:value, 3], [:value, :created], [[:symbol, \"1\"], [\"string\", \"2\"], [[:dynamic], \"3\"], [:missing, \"created\"]], [[\"a\", 2]], false, true, [[\"custom\", 7]], [:custom, :missing], [[[[:positional, true]]], []], [[], [[:keyword, true]]]]\n";
    let d = project();
    unchanged(
        d.path(),
        source.as_bytes(),
        &["--only", &COPS.join(","), "--fix", "a.rb"],
        1,
    );
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    for ruby in rubies() {
        assert_eq!(observe(&ruby, source), expected.as_bytes());
        assert_eq!(observe(&ruby, &after), expected.as_bytes());
        assert_ne!(
            observe(
                &ruby,
                &source.replace("Custom.new.has_key?", "Custom.new.key?")
            ),
            expected.as_bytes()
        );
    }
}
#[test]
fn mixed_safe_sibling_fixes_do_not_edit_unsafe_files() {
    let d = project();
    let s = SOURCES.join("");
    fs::write(d.path().join("safe.rb"), "value = 'x'\n").unwrap();
    let only = format!("{},Style/StringLiterals", COPS.join(","));
    unchanged(
        d.path(),
        s.as_bytes(),
        &["--only", &only, "--fix", "a.rb", "safe.rb"],
        1,
    );
    assert_eq!(
        fs::read(d.path().join("safe.rb")).unwrap(),
        b"value = \"x\"\n"
    );
}
