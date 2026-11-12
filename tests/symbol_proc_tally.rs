//! Pinned RuboCop examples (MIT, (c) 2012-26 Bozhidar Batsov),
//! selectors and detection-only guarantees through the real pipeline and CLI.
use crate::support::stdin;
use serde::Deserialize;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 2] = ["Style/SymbolProc", "Style/TallyMethod"];
const SOURCES: [&str; 2] = [
    "array.map { |x| x.upcase }\n",
    "array.each_with_object(Hash.new(0)) { |x, h| h[x] += 1 }\n",
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
fn lint(cop: &str, source: &str) -> Vec<rustocop::offense::Offense> {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let selectors: Vec<_> = cop.split(',').map(str::to_owned).collect();
    let active = rustocop::cops::select_active(&registry, &config, &selectors, &[]);
    let r = rustocop::pipeline::lint_one(
        Path::new("a.rb"),
        Path::new("a.rb"),
        Some(source.as_bytes()),
        &active,
        &config,
    );
    assert!(
        r.errors.is_empty() && r.internal_errors.is_empty(),
        "{source}: {r:?}"
    );
    r.result.offenses
}
#[test]
fn default_enabled_unsafe_metadata_and_selectors() {
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        let r = rustocop::cops::Registry::new();
        let c = r.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(c.default_enabled());
        assert!(!c.safe_fixes());
        assert!(c.path_policy().is_none());
        use rustocop::cops::policy::{RubyVersion, RubyVersions};
        assert_eq!(
            c.ruby_versions(),
            RubyVersions {
                minimum: (cop == COPS[1]).then_some(RubyVersion(2, 7)),
                maximum: None
            }
        );
        let d = project();
        let o = unchanged(d.path(), source.as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        for args in [
            vec!["--only", cop, "a.rb"],
            vec!["--only", cop, "--fix", "a.rb"],
        ] {
            unchanged(d.path(), source.as_bytes(), &args, 1);
        }
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            "[cops.Style]\nenabled=false\n".into(),
            format!("[cops.\"{cop}\"]\nenabled=false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), source.as_bytes(), &["--fix", "a.rb"], 0);
            unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
        )
        .unwrap();
        unchanged(d.path(), source.as_bytes(), &["a.rb"], 1);
    }
}
#[test]
fn excludes_strict_configuration_and_directives() {
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        let d = project();
        for config in [
            "exclude=[\"a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude=[\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        for knob in [
            "AllowComments=true",
            "AllowMethodsWithArguments=true",
            "TargetRubyVersion=2.6",
            "SafeAutoCorrect=true",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{knob}\n"),
            )
            .unwrap();
            unchanged(d.path(), source.as_bytes(), &["--fix", "a.rb"], 2);
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rubocop", "rustocop"] {
            for target in [cop, cop.split('/').nth(1).unwrap(), "Style", "all"] {
                for s in [
                    format!("# {prefix}:disable {target}\n{source}"),
                    format!("# {prefix}:disable-next {target}\n{source}"),
                    format!("{} # {prefix}:disable {target}\n", source.trim_end()),
                ] {
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
                }
            }
            let s = format!("# {prefix}:disable {cop}\n{source}# {prefix}:enable {cop}\n{source}");
            let found = lint(cop, &s);
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].line, 4);
            unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
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
fn stdin_reports_and_fix_rejection_do_not_touch_disk() {
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        let d = project();
        let p = d.path().join("a.rb");
        fs::write(&p, "disk bytes\n").unwrap();
        let time = fs::metadata(&p).unwrap().modified().unwrap();
        for fix in [false, true] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            command
                .current_dir(d.path())
                .args(["--only", cop, "--stdin", "a.rb"]);
            if fix {
                command.arg("--fix");
            }
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            stdin::write_stdin(child.stdin.take().unwrap(), source.as_bytes(), fix).unwrap();
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
    messages: Vec<String>,
    correction: Option<String>,
}
#[test]
fn all_99_applicable_pinned_calls_and_61_unsafe_correction_calls() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/symbol_proc_tally_pinned.json")).unwrap();
    assert_eq!(cases.len(), 99);
    let d = project();
    let mut calls = [0; 2];
    let mut corrections = [0; 2];
    for c in cases {
        let i = COPS.iter().position(|cop| *cop == c.cop).unwrap();
        calls[i] += 1;
        let found = lint(&c.cop, &c.source);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            c.positions,
            "{}:{}",
            c.cop,
            c.spec_line
        );
        assert_eq!(
            found.iter().map(|o| o.message.as_str()).collect::<Vec<_>>(),
            c.messages,
            "{}:{}",
            c.cop,
            c.spec_line
        );
        assert!(found.iter().all(|o| o.correction.is_none()));
        unchanged(
            d.path(),
            c.source.as_bytes(),
            &["--only", &c.cop, "a.rb"],
            i32::from(!c.positions.is_empty()),
        );
        if let Some(after) = c.correction {
            assert_ne!(c.source, after);
            corrections[i] += 1;
            unchanged(
                d.path(),
                c.source.as_bytes(),
                &["--only", &c.cop, "--fix", "a.rb"],
                1,
            );
        }
    }
    assert_eq!(calls, [69, 30]);
    assert_eq!(corrections, [41, 20]);
}
#[test]
fn byte_envelopes_bom_crlf_encoding_data_line_and_heredocs() {
    let d = project();
    for (cop, source) in COPS.into_iter().zip(SOURCES) {
        for prefix in [
            "",
            "\u{feff}",
            "# coding: UTF-8\n# café\n",
            "# frozen_string_literal: true\n",
        ] {
            let s =
                format!("{prefix}{source}p [__LINE__,__FILE__,DATA.read]\n__END__\nraw payload\n");
            for s in [s.clone(), s.replace('\n', "\r\n")] {
                unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
            }
        }
        let mut s = b"# coding: ISO-8859-1\n# caf\xe9\n".to_vec();
        s.extend_from_slice(source.as_bytes());
        unchanged(d.path(), &s, &["--only", cop, "--fix", "a.rb"], 1);
        let s = format!("p <<~TEXT\n  #{{{}}}\nTEXT\n", source.trim_end());
        assert_eq!(lint(cop, &s).len(), 1);
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
        let s = format!("{source}__END__\n{source}");
        assert_eq!(lint(cop, &s).len(), 1);
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
    if let Ok(p) = crate::support::runtime::ruby() {
        let o = Command::new(&p)
            .args(["-e", "print RUBY_VERSION"])
            .output()
            .unwrap();
        assert!(
            o.status.success()
                && o.stdout
                    .starts_with(crate::support::runtime::version_prefix().as_bytes())
        );
        r.push(p);
    }
    r
}
fn observe(ruby: &str, source: &str) -> Vec<u8> {
    let o = Command::new(ruby).args(["-e", source]).output().unwrap();
    assert!(o.status.success() && o.stderr.is_empty(), "{source}: {o:?}");
    o.stdout
}
#[test]
fn runtime_witnesses_arity_visibility_hash_defaults_shadowing_and_equality() {
    // Controlled programs only, never corpus/application Ruby. Both proposed
    // alternatives parse, but values, exceptions or evaluation count differ.
    let witnesses = [
        (
            COPS[0],
            "def consume; yield 'x', :extra; end; begin; p consume { |x| x.upcase }; rescue ArgumentError; puts :arity; end",
            "def consume; yield 'x', :extra; end; begin; p consume(&:upcase); rescue ArgumentError; puts :arity; end",
        ),
        (
            COPS[0],
            "class X; def initialize(n); @n=n; end; def same(xs); xs.map { |x| x.secret }; end; protected; def secret; @n; end; end; begin; p X.new(1).same([X.new(2)]); rescue NoMethodError; puts :visibility; end",
            "class X; def initialize(n); @n=n; end; def same(xs); xs.map(&:secret); end; protected; def secret; @n; end; end; begin; p X.new(1).same([X.new(2)]); rescue NoMethodError; puts :visibility; end",
        ),
        (
            COPS[0],
            "p(proc { |x| x.to_s }.lambda?)",
            "p(proc(&:to_s).lambda?)",
        ),
        (
            COPS[1],
            "h=[1,1].each_with_object(Hash.new(0)) { |x,h| h[x]+=1 }; p [h,h[9]]",
            "h=[1,1].tally; p [h,h[9]]",
        ),
        (
            COPS[1],
            "class X; def itself; :same; end; end; a=[X.new,X.new]; p a.group_by(&:itself).transform_values(&:count).values",
            "class X; def itself; :same; end; end; a=[X.new,X.new]; p a.tally.values",
        ),
        (
            COPS[1],
            "class X; def group_by; {a:[1]}; end; private; def tally; {}; end; end; begin; p X.new.group_by(&:itself).transform_values(&:count); rescue NoMethodError; puts :private; end",
            "class X; def group_by; {a:[1]}; end; private; def tally; {}; end; end; begin; p X.new.tally; rescue NoMethodError; puts :private; end",
        ),
        (
            COPS[1],
            "module M; class Hash; def self.new(n); $calls+=1; ::Hash.new(n); end; end; def self.f; [1,1].each_with_object(Hash.new(0)) { |x,h| h[x]+=1 }; end; end; $calls=0; p M.f; p $calls",
            "module M; class Hash; def self.new(n); $calls+=1; ::Hash.new(n); end; end; def self.f; [1,1].tally; end; end; $calls=0; p M.f; p $calls",
        ),
    ];
    let d = project();
    for (cop, source, alternative) in witnesses {
        assert_eq!(lint(cop, source).len(), 1, "{source}");
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
fn lazy_side_effecting_receivers_control_flow_and_captured_bindings_are_unedited() {
    let source = "seen=[]\ndef source(seen); seen << :receiver; [1,2,1].lazy; end\na=source(seen).map { |x| x.to_s }.take(2).force\nb=source(seen).each_with_object(Hash.new(0)) { |x,h| h[x]+=1 }\nc=source(seen).group_by { |x| x }.transform_values { |v| v.size }\nouter=:kept\nd=[1,2].map { |x| next outer if x==1; break :stop }\ne=[1].map { |x| -> { outer } }.first.call\np [a,b.to_a,c.to_a,d,e,outer,seen]\n";
    let both = COPS.join(",");
    let found = lint(&both, source);
    assert_eq!(found.len(), 4);
    assert!(found.iter().all(|o| o.correction.is_none()));
    let d = project();
    unchanged(
        d.path(),
        source.as_bytes(),
        &["--only", &both, "--fix", "a.rb"],
        1,
    );
    for ruby in rubies() {
        assert_eq!(observe(&ruby,source),b"[[\"1\", \"2\"], [[1, 2], [2, 1]], [[1, 2], [2, 1]], :stop, :kept, :kept, [:receiver, :receiver, :receiver]]\n");
    }
}
#[test]
fn bounded_six_thousand_siblings_use_the_main_ast_only() {
    let source = SOURCES.join("").repeat(3000);
    let found = lint(&COPS.join(","), &source);
    assert_eq!(found.len(), 6000);
    assert!(found.iter().all(|o| o.correction.is_none()));
    assert_eq!(found.last().unwrap().line, 6000);
}
#[test]
fn selected_peer_and_default_composition_preserve_values_and_leave_unsafe_findings() {
    let source = "label = 'value'; xs = [1, 2, 1]\na = xs.map { |x| x.to_s }\nb = xs.group_by { |x| x }.transform_values { |v| v.size }\np [label, a, b.to_a]\n";
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
                assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
            }
            if before == after.as_bytes() {
                stable = true;
                break;
            }
        }
        assert!(stable);
        assert_ne!(fs::read_to_string(path).unwrap(), source);
    }
}
