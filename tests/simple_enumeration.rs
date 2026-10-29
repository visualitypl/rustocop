//! controlled programs only; no corpus Ruby is executed.
#[path = "support/stdin.rs"]
mod stdin;
use serde::Deserialize;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 3] = [
    "Style/EachForSimpleLoop",
    "Style/EachWithObject",
    "Style/MapCompactWithConditionalBlock",
];
const SOURCES: [&str; 3] = [
    "(0...3).each {}\n",
    "[].reduce({}) { |a, e| a }\n",
    "foo.map { |e| e if e.ok? }.compact\n",
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
fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: i32) -> Output {
    let p = dir.join("a.rb");
    fs::write(&p, source).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    let o = cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), source);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
    o
}
#[test]
fn exact_defaults_capabilities_and_default_cli_detection() {
    let r = rustocop::cops::Registry::new();
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        let c = r.cops().iter().find(|c| c.name() == *cop).unwrap();
        assert!(c.default_enabled());
        assert_eq!(c.safe_fixes(), i < 2);
        assert_eq!(c.ruby_versions(), Default::default());
        assert!(c.path_policy().is_none());
        let o = unchanged(d.path(), SOURCES[i].as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
    }
}
#[test]
fn selectors_config_exclusions_and_strict_fixed_policy() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        let s = SOURCES[i].as_bytes();
        unchanged(d.path(), s, &["--only", cop, "a.rb"], 1);
        unchanged(
            d.path(),
            s,
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            "[cops.Style]\nenabled=false\n".into(),
            format!("[cops.\"{cop}\"]\nenabled=false\n"),
        ] {
            // Isolate the historical Style disable assertion from EmptyBlock.
            fs::write(
                d.path().join("rustocop.toml"),
                format!("{config}\n[cops.\"Lint/EmptyBlock\"]\nenabled=false\n"),
            )
            .unwrap();
            unchanged(d.path(), s, &["--fix", "a.rb"], 0);
            unchanged(d.path(), s, &["--only", cop, "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
        )
        .unwrap();
        let o = unchanged(d.path(), s, &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        for config in [
            "exclude=[\"a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude=[\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s, &["--only", cop, "--fix", "a.rb"], 0);
        }
        for knob in [
            "EnforcedStyle=\"other\"",
            "SafeAutoCorrect=true",
            "TargetRubyVersion=3.4",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{knob}\n"),
            )
            .unwrap();
            unchanged(d.path(), s, &["--fix", "a.rb"], 2);
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn directives_clean_and_error_files_hold_bytes_and_mtime() {
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
            let o = unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
            assert_eq!(
                String::from_utf8_lossy(&o.stdout)
                    .matches(&format!("({cop})"))
                    .count(),
                1
            );
        }
        for (s, code) in [("p :clean\n", 0), ("(\n", 2)] {
            unchanged(
                d.path(),
                s.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                code,
            );
        }
    }
    let s = "[].reduce({}) do |a,e|\n  a[e]=e\n  a # rustocop:disable Style/EachWithObject\nend\n";
    unchanged(
        d.path(),
        s.as_bytes(),
        &["--only", COPS[1], "--fix", "a.rb"],
        1,
    );
}
#[test]
fn stdin_reports_without_writes_and_rejects_fix() {
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
            assert_eq!(o.status.code(), Some(if fix { 2 } else { 1 }));
            if fix {
                assert!(o.stdout.is_empty());
                assert!(String::from_utf8_lossy(&o.stderr).contains("cannot be used with '--fix'"));
            } else {
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
    correction: Option<String>,
}
#[test]
fn all_69_pinned_examples_and_43_correction_calls() {
    // RuboCop's three assigned specs, MIT, (c) 2012-26 Bozhidar Batsov.
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/simple_enumeration_pinned.json")).unwrap();
    assert_eq!(cases.len(), 69);
    assert_eq!(cases.iter().filter(|c| c.correction.is_some()).count(), 43);
    let d = project();
    let mut applied = 0;
    let mut withheld = 0;
    for c in cases {
        let registry = rustocop::cops::Registry::new();
        let config = rustocop::config::ResolvedConfig::default();
        let active =
            rustocop::cops::select_active(&registry, &config, std::slice::from_ref(&c.cop), &[]);
        let report = rustocop::pipeline::lint_one(
            Path::new("a.rb"),
            Path::new("a.rb"),
            Some(c.source.as_bytes()),
            &active,
            &config,
        );
        assert!(report.errors.is_empty() && report.internal_errors.is_empty());
        let found = report.result.offenses;
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            c.positions,
            "{}:{}",
            c.cop,
            c.spec_line
        );
        unchanged(
            d.path(),
            c.source.as_bytes(),
            &["--only", &c.cop, "a.rb"],
            i32::from(!c.positions.is_empty()),
        );
        if let Some(after) = c.correction {
            if c.cop == COPS[2] {
                unchanged(
                    d.path(),
                    c.source.as_bytes(),
                    &["--only", &c.cop, "--fix", "a.rb"],
                    1,
                );
                withheld += 1;
            } else {
                fs::write(d.path().join("a.rb"), &c.source).unwrap();
                cli(d.path(), &["--only", &c.cop, "--fix", "a.rb"], 0);
                assert_eq!(
                    fs::read_to_string(d.path().join("a.rb")).unwrap(),
                    after,
                    "{}:{}",
                    c.cop,
                    c.spec_line
                );
                applied += 1;
            }
        }
    }
    assert_eq!((applied, withheld), (17, 26));
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
    if let Ok(p) = std::env::var("RUSTOCOP_RUBY34") {
        let o = Command::new(&p)
            .args(["-e", "print RUBY_VERSION"])
            .output()
            .unwrap();
        assert!(o.status.success() && o.stdout.starts_with(b"3.4."));
        r.push(p);
    }
    r
}
fn observe(ruby: &str, dir: &Path) -> Vec<u8> {
    let o = Command::new(ruby)
        .current_dir(dir)
        .arg("a.rb")
        .output()
        .unwrap();
    assert!(o.status.success(), "{ruby}: {o:?}");
    assert!(o.stderr.is_empty(), "{o:?}");
    // Ruby 3.4 adds spaces around hash rockets in inspect output.
    String::from_utf8(o.stdout)
        .unwrap()
        .replace(" => ", "=>")
        .into_bytes()
}
#[test]
fn runtime_count_binding_evaluation_order_closures_and_numbered_parameters() {
    let d = project();
    let source = r#"trace = []
(-3..2).each { trace << :tick }
(7..3).each { trace << :never }
(0...0).each { trace << :never }
result = [(trace << :first; 1), (trace << :second; 2)].reduce({}) do |memo,item|
  memo[item] = item * 2
  proc { trace << [:capture, memo.keys, item] }.call
  memo
end
numbered = [3,4].reduce({}) do
  trace << [:numbered, _1.keys, _2]
  _1[_2] = _2 * 2
  _1
end
p [trace, result, numbered]
"#;
    let expected = "[[:tick, :tick, :tick, :tick, :tick, :tick, :first, :second, [:capture, [1], 1], [:capture, [1, 2], 2], [:numbered, [], 3], [:numbered, [3], 4]], {1=>2, 2=>4}, {3=>6, 4=>8}]\n";
    fs::write(d.path().join("a.rb"), source).unwrap();
    for ruby in rubies() {
        assert_eq!(observe(&ruby, d.path()), expected.as_bytes());
    }
    cli(
        d.path(),
        &["--only", &COPS[..2].join(","), "--fix", "a.rb"],
        0,
    );
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert!(after.contains("(-3).times"));
    assert!(after.contains("each_with_object"));
    assert!(!after.contains(".reduce"));
    for ruby in rubies() {
        assert_eq!(observe(&ruby, d.path()), expected.as_bytes());
    }
}
#[test]
fn duplicate_underscore_bindings_preserve_runtime_values_and_file_metadata() {
    let d = project();
    for name in ["_", "_memo"] {
        let s = format!(
            "result = [1,2].reduce({{}}) {{ |{name}, {name}| {name}[:key] = 1; {name} }}\np result.to_a\n"
        );
        fs::write(d.path().join("a.rb"), &s).unwrap();
        for ruby in rubies() {
            assert_eq!(observe(&ruby, d.path()), b"[[:key, 1]]\n", "{s}");
        }
        let o = unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", COPS[1], "--fix", "a.rb"],
            1,
        );
        assert!(String::from_utf8_lossy(&o.stdout).contains(COPS[1]));
        for ruby in rubies() {
            assert_eq!(observe(&ruby, d.path()), b"[[:key, 1]]\n", "{s}");
        }
    }
}
#[test]
fn unsafe_values_visibility_lazy_receivers_and_control_flow_are_never_rewritten() {
    let d = project();
    let cases = [
        (COPS[0], "p((0..3).each {})\n", "0..3\n"),
        (
            COPS[0],
            "class Range; def each; puts :custom; self; end; end\n(0..3).each {}\n",
            "custom\n",
        ),
        (
            COPS[1],
            "p [\"x\"].reduce({}) { |a,e| e => a; a }\n",
            "\"x\"\n",
        ),
        (
            COPS[1],
            "p [\"x\"].reduce({}) { |a,e| /(?<a>x)/ =~ e; a }\n",
            "\"x\"\n",
        ),
        (
            COPS[1],
            "x = [1].reduce({}) { _2; _1[:k] = {_1:}.keys; _1 }; p x.to_a\n",
            "[[:k, [:_1]]]\n",
        ),
        (
            COPS[1],
            "class Custom; def reduce(x); yield x, :key; x; end; private; def each_with_object(x); raise; end; end\nx = Custom.new.reduce({}) { |a,e| a[e]=1; a }; p x.to_a\n",
            "[[:key, 1]]\n",
        ),
        (
            COPS[2],
            "class Custom; def map; [yield(1)]; end; private; def select; raise; end; end\np Custom.new.map { |e| e if e == 1 }.compact\n",
            "[1]\n",
        ),
        (
            COPS[1],
            "x = [false,true].reduce({}) { |a,e| next nil if e; a }; p x\n",
            "nil\n",
        ),
        (
            COPS[1],
            "x = [1,2].lazy.reduce({}) { |a,e| a[e]=e; a }; p x\n",
            "{1=>1, 2=>2}\n",
        ),
        (
            COPS[1],
            "class Array; private :each_with_object; end\np [1].reduce({}) { |a,e| a[e]=e; a }\n",
            "{1=>1}\n",
        ),
        (
            COPS[2],
            "p [nil,false,1].map { |e| e if true }.compact\n",
            "[false, 1]\n",
        ),
        (
            COPS[2],
            "p [nil,false,1].filter_map { |e| e if true }\n",
            "[1]\n",
        ),
        (
            COPS[2],
            "class Eq; def initialize(log); @log=log; end; def ==(x); @log << :compare; true; end; end\nlog=[]; e=Eq.new(log); result=[nil,e].map { |item| item if item == :tag }.compact; p [result.length, result.first.equal?(e), log]\n",
            "[1, true, [:compare]]\n",
        ),
        (
            COPS[2],
            "events=[]; xs=[nil,false,1].lazy.map { |e| events << e; e }.map { |e| e if true }.compact; before=events.dup; result=xs.force; p [before, result, events]\n",
            "[[], [false, 1], [nil, false, 1]]\n",
        ),
    ];
    for (cop, s, expected) in cases {
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
        for ruby in rubies() {
            assert_eq!(observe(&ruby, d.path()), expected.as_bytes(), "{s}");
        }
    }
}
#[test]
fn comments_line_data_bom_crlf_and_legacy_encoding() {
    let d = project();
    for (cop, s) in [
        (COPS[0], "(0.. # retained\n3).each {}\n"),
        (COPS[0], "(0..\n3).each {}; p __LINE__\n"),
        (COPS[1], "[].reduce({}) do |a,e|\n a\nend\np __LINE__\n"),
        (COPS[1], "[].reduce({}) { |a,e| a # retained\n}\n"),
    ] {
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
    }
    for (i, cop) in COPS.iter().enumerate() {
        let s = format!(
            "\u{feff}# encoding: UTF-8\r\n{}p [__LINE__, DATA.pos, DATA.read]\r\n__END__\r\nraw\0\r\n",
            SOURCES[i].replace('\n', "\r\n")
        );
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
    }
    let s = b"# encoding: ISO-8859-1\r\n[1].reduce({}) { |a,e| a[\"caf\xe9\"]=e; a }\r\n";
    unchanged(d.path(), s, &["--only", COPS[1], "--fix", "a.rb"], 1);
    let before = "\u{feff}# encoding: UTF-8\r\n(0...3).each { p __LINE__ }\r\n__END__\r\nraw\0\r\n";
    let after = before.replace("(0...3).each", "3.times");
    fs::write(d.path().join("a.rb"), before).unwrap();
    cli(d.path(), &["--only", COPS[0], "--fix", "a.rb"], 0);
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), after.as_bytes());
}
#[test]
fn safe_siblings_and_curated_peer_default_layout_composition_stabilize() {
    let d = project();
    let s = "trace = []\n(0...2).each { trace << 'x' }\nresult = [1,2].inject({}) { |a,e| a[e] = 'v'; a }\np [trace,result]\n";
    let expected = "[[\"x\", \"x\"], {1=>\"v\", 2=>\"v\"}]\n";
    for selectors in [
        vec![
            "--only",
            "Style/EachForSimpleLoop,Style/EachWithObject,Style/StringLiterals,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/HashSyntax,Style/SignalException",
        ],
        vec!["--only", "Style,Layout"],
        vec![],
    ] {
        fs::write(d.path().join("a.rb"), s).unwrap();
        let mut stable = false;
        for _ in 0..6 {
            let before = fs::read(d.path().join("a.rb")).unwrap();
            let mut args = selectors.clone();
            args.extend(["--fix", "a.rb"]);
            let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(args)
                .output()
                .unwrap();
            assert!(matches!(o.status.code(), Some(0 | 1)), "{o:?}");
            assert!(o.stderr.is_empty(), "{o:?}");
            for ruby in rubies() {
                assert_eq!(observe(&ruby, d.path()), expected.as_bytes());
            }
            if fs::read(d.path().join("a.rb")).unwrap() == before {
                stable = true;
                break;
            }
        }
        assert!(stable);
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert!(after.contains("times"));
        assert!(after.contains("each_with_object"));
    }
    let source = "x = (0..3).each {}\n(0...3).each {}\n";
    let o = unchanged(d.path(), source.as_bytes(), &["--only", COPS[0], "a.rb"], 1);
    assert_eq!(
        String::from_utf8_lossy(&o.stdout).matches(COPS[0]).count(),
        2
    );
    cli(d.path(), &["--only", COPS[0], "--fix", "a.rb"], 1);
    assert_eq!(
        fs::read_to_string(d.path().join("a.rb")).unwrap(),
        "x = (0..3).each {}\n3.times {}\n"
    );
}
