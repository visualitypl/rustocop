//! unsafe collection suggestions never edit Ruby.
use crate::support::stdin;
use serde::Deserialize;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 3] = [
    "Style/CollectionCompact",
    "Style/CollectionMethods",
    "Style/CollectionQuerying",
];
const SOURCES: [&str; 3] = [
    "array.reject { |e| e.nil? }\n",
    "items.collect { |e| e + 1 }\n",
    "x.count(&:foo?).positive?\n",
];
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
fn offenses(source: &str, cop: &str) -> Vec<rustocop::offense::Offense> {
    use rustocop::{
        config::ResolvedConfig,
        cops::{Registry, select_active},
    };
    let registry = Registry::new();
    let config = ResolvedConfig::default();
    let active = select_active(&registry, &config, &[cop.into()], &[]);
    let report = rustocop::pipeline::lint_one(
        Path::new("a.rb"),
        Path::new("a.rb"),
        Some(source.as_bytes()),
        &active,
        &config,
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.internal_errors.is_empty());
    report.result.offenses
}
#[test]
fn compact_exact_diagnostic() {
    let o = offenses("# header\n  array.reject { |e| e.nil? }\n", COPS[0]);
    assert_eq!(o.len(), 1);
    assert_eq!((o[0].line, o[0].col), (2, 9));
    assert_eq!(
        o[0].message,
        "Use `compact` instead of `reject { |e| e.nil? }`."
    );
    assert!(o[0].correction.is_none());
}
#[test]
fn methods_exact_opt_in_diagnostic() {
    let o = offenses("# header\n  items.collect { |e| e + 1 }\n", COPS[1]);
    assert_eq!(o.len(), 1);
    assert_eq!((o[0].line, o[0].col), (2, 9));
    assert_eq!(o[0].message, "Prefer `map` over `collect`.");
    assert!(o[0].correction.is_none());
}
#[test]
fn querying_exact_diagnostic() {
    let o = offenses("# header\n  x.count(&:foo?).positive?\n", COPS[2]);
    assert_eq!(o.len(), 1);
    assert_eq!((o[0].line, o[0].col), (2, 5));
    assert_eq!(o[0].message, "Use `any?` instead.");
    assert!(o[0].correction.is_none());
}
#[test]
fn exact_defaults_versions_and_no_safe_fix_capabilities() {
    let registry = rustocop::cops::Registry::new();
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        let c = registry.cops().iter().find(|c| c.name() == *cop).unwrap();
        assert_eq!(c.default_enabled(), i != 1);
        assert!(!c.safe_fixes());
        assert!(c.path_policy().is_none());
        use rustocop::cops::policy::{DEFAULT_RUBY_VERSION, RubyVersion};
        assert_eq!(DEFAULT_RUBY_VERSION, RubyVersion(3, 4));
        assert_eq!(
            c.ruby_versions().minimum,
            if i == 0 {
                Some(RubyVersion(2, 4))
            } else {
                None
            }
        );
        assert_eq!(c.ruby_versions().maximum, None);
        let o = unchanged(
            d.path(),
            SOURCES[i].as_bytes(),
            &["a.rb"],
            i32::from(i != 1),
        );
        assert_eq!(String::from_utf8_lossy(&o.stdout).contains(cop), i != 1);
    }
}
#[test]
fn selectors_config_excludes_and_no_new_policy_knobs() {
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
            // SymbolProc now also reports the compact fixture. Disabling
            // only Compact must preserve that independent default-on finding.
            let symbol_proc_remains = i == 0 && config.starts_with("[cops.\"");
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            let o = unchanged(
                d.path(),
                s,
                &["--fix", "a.rb"],
                i32::from(symbol_proc_remains),
            );
            assert!(!String::from_utf8_lossy(&o.stdout).contains(cop));
            assert_eq!(
                String::from_utf8_lossy(&o.stdout).contains("Style/SymbolProc"),
                symbol_proc_remains
            );
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
            "PreferredMethods = {}",
            "AllowedReceivers = []",
            "ActiveSupportExtensionsEnabled = true",
            "SafeAutoCorrect = true",
            "TargetRubyVersion = 4.0",
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
fn both_directive_prefixes_clean_and_error_files_hold_bytes_and_mtime() {
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
        for (s, code) in [("x.compact\nx.map(&:foo)\nx.empty?\n", 0), ("(\n", 2)] {
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
fn stdin_reports_without_writing_and_fix_stdin_is_rejected() {
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
    spec_line: u32,
    source: String,
    positions: Vec<(u32, u32)>,
}
#[test]
fn all_122_applicable_pinned_examples_and_94_correction_calls_hold_bytes_and_mtime() {
    // RuboCop's three assigned specs, MIT, (c) 2012-26 Bozhidar Batsov.
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/collections_pinned.json")).unwrap();
    assert_eq!(cases.len(), 122);
    assert_eq!(cases.iter().filter(|c| !c.positions.is_empty()).count(), 94);
    let d = project();
    for c in cases {
        let o = offenses(&c.source, &c.cop);
        assert_eq!(
            o.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            c.positions,
            "{}:{} {}",
            c.cop,
            c.spec_line,
            c.source
        );
        assert!(o.iter().all(|o| o.correction.is_none()));
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
                "{}:{}",
                c.cop,
                c.spec_line
            );
        }
    }
}
#[test]
fn source_envelopes_data_line_encodings_and_heredocs_hold_exact_bytes() {
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
            "a.reject { |e| e.nil? }\np <<~TEXT\n#{__LINE__}\nTEXT\n",
        ),
        (COPS[1], "a.collect { <<~TEXT }\n#{__LINE__}\nTEXT\n"),
        (
            COPS[2],
            "a.count { <<~TEXT }.positive?\n#{__LINE__}\nTEXT\n",
        ),
    ] {
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
    }
}
#[test]
fn mixed_safe_sibling_fixes_leave_unsafe_file_untouched() {
    let d = project();
    fs::write(d.path().join("safe.rb"), "value = 'x'\n").unwrap();
    let only = format!("{},Style/StringLiterals", COPS.join(","));
    unchanged(
        d.path(),
        SOURCES.join("").as_bytes(),
        &["--only", &only, "--fix", "a.rb", "safe.rb"],
        1,
    );
    assert_eq!(
        fs::read(d.path().join("safe.rb")).unwrap(),
        b"value = \"x\"\n"
    );
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
fn controlled_runtime_keeps_capture_control_flow_arity_evaluation_dispatch_and_laziness() {
    let source = r#"
trace = []
receiver = -> { trace << :receiver; [nil, 1, 2] }
trace << receiver.call.reject { |e| e.nil? }
trace << [[1,2],[3,nil]].reject { |k,v| v.nil? }
captured = 0
trace << [1,2,3].collect { |e| captured += e; next captured }
trace << captured
trace << [1,2,3].collect { |a,b=7| [a,b] }
trace << [1,2,3].collect { |e| break e + 10 }
lazy_trace = []
lazy = [nil,1,2].lazy.map { |e| lazy_trace << e; e }.reject { |e| e.nil? }
trace << lazy_trace.dup
trace << lazy.take(1).force
trace << lazy_trace
count_trace = []
trace << [1,2,3].count { |e| count_trace << e; e > 0 }.positive?
trace << count_trace
trace << [false,nil].count.positive?
trace << [false,nil].count.zero?
trace << (1...1).count.zero?
class Dispatch
  def collect; [:custom, yield(4)]; end
  def map; :different; end
  def count; 3; end
  private def empty?; true; end
end
trace << Dispatch.new.collect { |x| x + 1 }
trace << Dispatch.new.count.positive?
class Equality
  def ==(other); true; end
  def ===(other); false; end
end
pattern = Equality.new
trace << [1,2].count(pattern).positive?
class Channels
  def count(*args, **kwargs); args.empty? ? kwargs.length : args.length + 1; end
end
trace << Channels.new.count(key: :value).positive?
trace << Channels.new.count({key: :value}).positive?
p trace
"#;
    let expected = "[:receiver, [1, 2], [[1, 2]], [1, 3, 6], 6, [[1, 7], [2, 7], [3, 7]], 11, [], [1], [nil, 1], true, [1, 2, 3], true, false, true, [:custom, 5], true, true, true, true]\n";
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
        for (from, to) in [
            ("Dispatch.new.collect", "Dispatch.new.map"),
            ("[false,nil].count.positive?", "[false,nil].any?"),
        ] {
            assert_ne!(
                observe(&ruby, &source.replace(from, to)),
                expected.as_bytes()
            );
        }
        // Short-circuiting changes both the trace and captured state. The
        // receiver is called once; neither eager traversal nor blocks move.
        let rewritten = source.replace(
            "[1,2,3].count { |e| count_trace << e; e > 0 }.positive?",
            "[1,2,3].any? { |e| count_trace << e; e > 0 }",
        );
        assert_ne!(observe(&ruby, &rewritten), expected.as_bytes());
    }
}
#[test]
fn fixed_34_filter_examples_excluded_by_upstream_old_prism_metadata_still_run() {
    // collection_compact_spec.rb:319,333,347. The >=2.6 group is tagged
    // unsupported_on: :prism upstream, but all three behaviors apply at 3.4.
    let d = project();
    for (s, count) in [
        (
            "array.filter { |e| !e.nil? }\nhash.filter! { |k,v| !v.nil? }\n",
            2,
        ),
        (
            "array&.filter { |e| e&.nil?&.! }\nhash&.filter! { |k,v| v&.nil?&.! }\n",
            2,
        ),
        ("filter { |e| !e.nil? }\nfilter! { |k,v| !v.nil? }\n", 0),
    ] {
        assert_eq!(offenses(s, COPS[0]).len(), count);
        for fix in [false, true] {
            let mut args = vec!["--only", COPS[0], "a.rb"];
            if fix {
                args.push("--fix");
            }
            unchanged(d.path(), s.as_bytes(), &args, i32::from(count != 0));
        }
    }
}
#[test]
fn controlled_equality_visibility_and_enum_method_availability_witnesses() {
    let source = r#"
class Pattern
  def ==(other); true; end
  def ===(other); false; end
end
pattern = Pattern.new
p [[1,2].count(pattern).positive?, [1,2].any?(pattern)]
class PrivatePredicate
  def count; 1; end
  private def empty?; false; end
end
p PrivatePredicate.new.count.positive?
begin
  PrivatePredicate.new.empty?
rescue NoMethodError
  p :private
end
p (1...1).count.zero?
begin
  (1...1).empty?
rescue NoMethodError
  p :unavailable
end
p [[1,2],[3,nil]].reject { |k,v| v.nil? }
p [[1,2],[3,nil]].compact
"#;
    let d = project();
    unchanged(
        d.path(),
        source.as_bytes(),
        &["--only", &COPS.join(","), "--fix", "a.rb"],
        1,
    );
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    for ruby in rubies() {
        let expected =
            b"[true, false]\ntrue\n:private\ntrue\n:unavailable\n[[1, 2]]\n[[1, 2], [3, nil]]\n";
        assert_eq!(observe(&ruby, source), expected);
        assert_eq!(observe(&ruby, &after), expected);
    }
}
#[test]
fn querying_assignment_parent_exemption_and_rhs_reports_hold_cli_bytes() {
    let d = project();
    let cop = COPS[2];
    let clean = "x.count.positive?.foo += 1\nx.count.positive?.foo ||= 1\nx.count.positive?.foo &&= 1\nx.count.positive?.foo, other = values\n";
    let positive = "other.foo += x.count.positive?\n(x.count.positive?).foo ||= 1\nx.count.positive?[0] &&= 1\nx.count.zero?.foo, other = values\nx.count(&:foo?).positive?.foo += 1\n";
    for fix in [false, true] {
        let mut args = vec!["--only", cop, "a.rb"];
        if fix {
            args.push("--fix");
        }
        unchanged(d.path(), clean.as_bytes(), &args, 0);
        let output = unchanged(d.path(), positive.as_bytes(), &args, 1);
        let text = String::from_utf8(output.stdout).unwrap();
        assert_eq!(text.matches(&format!("({cop})")).count(), 5, "{text}");
    }
    let found = offenses(positive, cop);
    assert_eq!(
        found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
        [(1, 16), (2, 4), (3, 3), (4, 3), (5, 3)]
    );
    assert!(found.iter().all(|o| o.correction.is_none()));
}
#[test]
fn native_implicit_it_binding_remains_unchanged_when_available() {
    let Some(ruby) = crate::support::runtime::ruby().ok() else {
        return;
    };
    let s = "p [nil,1,2].reject { it.nil? }\np [1,2].collect { it + 1 }\np [1,2].count { it > 0 }.positive?\n";
    let d = project();
    unchanged(
        d.path(),
        s.as_bytes(),
        &["--only", &COPS.join(","), "--fix", "a.rb"],
        1,
    );
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert_eq!(observe(&ruby, s), b"[1, 2]\n[2, 3]\ntrue\n");
    assert_eq!(observe(&ruby, &after), observe(&ruby, s));
}
