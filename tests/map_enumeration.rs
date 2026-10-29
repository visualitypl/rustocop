//! fixed-policy detection-only array mapping cops.
#[path = "support/stdin.rs"]
mod stdin;
use serde::Deserialize;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 3] = ["Style/MapIntoArray", "Style/MapJoin", "Style/MapToHash"];
const SOURCES: [&str; 3] = [
    "dest = []\nsrc.each { |e| dest << e }\n",
    "array.map(&:to_s).join\n",
    "array.map { |x| [x, x] }.to_h\n",
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
fn exact_defaults_versions_and_unsafe_capabilities() {
    let r = rustocop::cops::Registry::new();
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        let c = r.cops().iter().find(|c| c.name() == *cop).unwrap();
        assert!(c.default_enabled());
        assert!(!c.safe_fixes());
        assert!(c.path_policy().is_none());
        assert_eq!(c.ruby_versions().maximum, None);
        assert_eq!(
            c.ruby_versions().minimum,
            if i == 2 {
                Some(rustocop::cops::policy::RubyVersion(2, 6))
            } else {
                None
            }
        );
        let o = unchanged(d.path(), SOURCES[i].as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        unchanged(
            d.path(),
            SOURCES[i].as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            1,
        );
    }
}
#[test]
fn selectors_config_excludes_and_no_style_options() {
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
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s, &["--fix", "a.rb"], 0);
            unchanged(d.path(), s, &["--only", cop, "--fix", "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
        )
        .unwrap();
        let o = unchanged(d.path(), s, &["--fix", "a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        for config in [
            "exclude=[\"a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude=[\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s, &["--only", cop, "--fix", "a.rb"], 0);
        }
        for option in [
            "PreferredMethods={map=\"collect\"}",
            "SafeAutoCorrect=true",
            "TargetRubyVersion=3.4",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{option}\n"),
            )
            .unwrap();
            unchanged(d.path(), s, &["--fix", "a.rb"], 2);
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn directives_clean_error_and_reenabled_files_do_not_write() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        for prefix in ["rubocop", "rustocop"] {
            for target in [*cop, cop.strip_prefix("Style/").unwrap(), "Style", "all"] {
                // Use block-wide directives for the multiline destination pattern.
                for directive in ["disable", "todo"] {
                    let s = format!("# {prefix}:{directive} {target}\n{}", SOURCES[i]);
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
                }
                let s = format!("{} # {prefix}:disable {target}\n", SOURCES[i].trim_end());
                unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
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
        for (s, code) in [("p :clean\n", 0), ("(\n", 2)] {
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
fn stdin_reports_but_fix_is_rejected_without_touching_disk() {
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
fn all_128_pinned_calls_including_obsolete_prism_exclusions_and_85_unsafe_corrections() {
    // Ported from the three pinned RuboCop specs (MIT, (c) 2012-26 Bozhidar Batsov).
    // Includes every applicable call, not deduplicated. Only map=>collect config is omitted.
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/map_enumeration_pinned.json")).unwrap();
    assert_eq!(cases.len(), 128);
    assert_eq!(cases.iter().filter(|c| c.correction.is_some()).count(), 85);
    let d = project();
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
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{}:{}",
            c.cop,
            c.spec_line
        );
        assert_eq!(
            report
                .result
                .offenses
                .iter()
                .map(|o| (o.line, o.col))
                .collect::<Vec<_>>(),
            c.positions,
            "{}:{}",
            c.cop,
            c.spec_line
        );
        assert!(
            report
                .result
                .offenses
                .iter()
                .all(|o| o.correction.is_none())
        );
        unchanged(
            d.path(),
            c.source.as_bytes(),
            &["--only", &c.cop, "a.rb"],
            i32::from(!c.positions.is_empty()),
        );
        if c.correction.is_some() {
            unchanged(
                d.path(),
                c.source.as_bytes(),
                &["--only", &c.cop, "--fix", "a.rb"],
                1,
            );
        }
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
fn observe(ruby: &str, source: &str) -> Vec<u8> {
    let o = Command::new(ruby).args(["-e", source]).output().unwrap();
    assert!(o.status.success(), "{source}: {o:?}");
    assert!(o.stderr.is_empty(), "{o:?}");
    o.stdout
}
#[test]
fn runtime_binding_control_flow_lazy_receivers_arity_visibility_and_equality() {
    // Controlled witnesses, never arbitrary corpus code. These rewrites are
    // parseable but observably different; all selected fixes must retain bytes.
    let witnesses = [
        (
            COPS[0],
            "events=[]; src=[1,2].lazy.map { |x| events << x; x }; dest=[]; src.each { |x| dest << x }; p [dest.class,events]",
            "events=[]; src=[1,2].lazy.map { |x| events << x; x }; dest=src.map { |x| x }; p [dest.class,events]",
        ),
        (
            COPS[0],
            "trace=[]; dest=nil; src=proc { trace << dest; [1] }; dest=[]; src.call.each { |x| dest << x }; p trace",
            "trace=[]; dest=nil; src=proc { trace << dest; [1] }; dest=src.call.map { |x| x }; p trace",
        ),
        (
            COPS[0],
            "class X; def each; yield 1; end; private; def map; end; end; dest=[]; X.new.each { |x| dest << x }; p dest",
            "class X; def each; yield 1; end; private; def map; end; end; begin; dest=X.new.map { |x| x }; rescue NoMethodError; puts :private; end",
        ),
        (
            COPS[0],
            "dest=[]; [1,2].each { |x| dest << begin; next 3 if x==1; x; end }; p dest",
            "dest=[1,2].map { |x| begin; next 3 if x==1; x; end }; p dest",
        ),
        (
            COPS[1],
            "p [[1,2],nil,false].map(&:to_s).join(',')",
            "p [[1,2],nil,false].join(',')",
        ),
        (
            COPS[1],
            "class X; def map; [yield(1)]; end; private; def join; end; end; p X.new.map { |x| x.to_s }.join",
            "class X; def map; [yield(1)]; end; private; def join; end; end; begin; p X.new.join; rescue NoMethodError; puts :private; end",
        ),
        (
            COPS[2],
            "$trace=[]; class Key; def hash; $trace << :hash; 0; end; def eql?(x); $trace << :equal; true; end; end; xs=[Key.new,Key.new]; xs.map { |k| $trace << :transform; [k,1] }.to_h; p $trace",
            "$trace=[]; class Key; def hash; $trace << :hash; 0; end; def eql?(x); $trace << :equal; true; end; end; xs=[Key.new,Key.new]; xs.to_h { |k| $trace << :transform; [k,1] }; p $trace",
        ),
        (
            COPS[2],
            "class X; def map; [yield(1,2)]; end; def to_h; [yield(1)].to_h; end; end; p X.new.map { |a,b| [a,b] }.to_h.to_a",
            "class X; def map; [yield(1,2)]; end; def to_h; [yield(1)].to_h; end; end; p X.new.to_h { |a,b| [a,b] }.to_a",
        ),
    ];
    let d = project();
    for (cop, source, alternative) in witnesses {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            1,
        );
        for ruby in rubies() {
            assert_ne!(
                observe(&ruby, source),
                observe(&ruby, alternative),
                "{ruby}: {source}"
            );
        }
    }
}
#[test]
fn selected_peer_and_actual_default_fixes_preserve_runtime_and_unsafe_findings() {
    let source = "dest=[]; [1,2].each { |e| dest << e }\nparts = ['a',1].map(&:to_s).join(',')\npairs = [1,2].map { |x| [x,x] }.to_h\np [dest,parts,pairs.to_a]\n";
    let selected = format!(
        "{},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout",
        COPS.join(",")
    );
    for only in [Some(selected.as_str()), None] {
        let d = project();
        let path = d.path().join("a.rb");
        fs::write(&path, source).unwrap();
        let expected: Vec<_> = rubies().iter().map(|r| observe(r, source)).collect();
        let mut seen = std::collections::HashSet::new();
        let mut stable = false;
        for _ in 0..6 {
            let before = fs::read(&path).unwrap();
            assert!(seen.insert(before.clone()));
            let args = match only {
                Some(cops) => vec!["--only", cops, "--fix", "a.rb"],
                None => vec!["--fix", "a.rb"],
            };
            let o = cli(d.path(), &args, 1);
            let after = fs::read_to_string(&path).unwrap();
            for (i, ruby) in rubies().iter().enumerate() {
                assert_eq!(observe(ruby, &after), expected[i]);
            }
            for cop in COPS {
                assert!(String::from_utf8_lossy(&o.stdout).contains(cop), "{o:?}");
            }
            if before == after.as_bytes() {
                stable = true;
                break;
            }
        }
        assert!(stable);
    }
}
#[test]
fn source_envelopes_and_observers_are_byte_and_mtime_stable() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        for prefix in [
            "",
            "\u{feff}",
            "# coding: UTF-8\n# café\n",
            "# frozen_string_literal: true\n",
        ] {
            let s = format!(
                "{prefix}{}p [__LINE__,__FILE__]\n__END__\nraw data\n",
                SOURCES[i]
            );
            for s in [s.clone(), s.replace('\n', "\r\n")] {
                unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
            }
        }
        let mut s = b"# coding: ISO-8859-1\n# caf\xe9\n".to_vec();
        s.extend_from_slice(SOURCES[i].as_bytes());
        unchanged(d.path(), &s, &["--only", cop, "--fix", "a.rb"], 1);
    }
    let source = "dest=[]\n[1].each do |x|\n  dest << <<~TEXT\n    café #{__LINE__}\n  TEXT\nend\np [dest,__LINE__,DATA.read]\n__END__\npayload\n";
    unchanged(
        d.path(),
        source.as_bytes(),
        &["--only", COPS[0], "--fix", "a.rb"],
        1,
    );
}
