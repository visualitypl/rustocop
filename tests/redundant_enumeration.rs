//! real pipeline/CLI and controlled Ruby safety observations.
use crate::support::stdin;
use serde::Deserialize;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 3] = [
    "Style/RedundantEach",
    "Style/RedundantFilterChain",
    "Style/SelectByKind",
];
const SOURCES: [&str; 3] = [
    "array.each.each { |x| x }\n",
    "array.select { |x| x.odd? }.any?\n",
    "array.select { |x| x.is_a?(Foo) }\n",
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
    let active = rustocop::cops::select_active(&registry, &config, &[cop.into()], &[]);
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
fn exact_defaults_and_no_safe_correction_capability() {
    let r = rustocop::cops::Registry::new();
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        let c = r.cops().iter().find(|c| c.name() == *cop).unwrap();
        assert!(c.default_enabled());
        assert!(!c.safe_fixes());
        assert_eq!(c.ruby_versions(), Default::default());
        assert!(c.path_policy().is_none());
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
fn selectors_config_exclusions_and_fixed_policy() {
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
            // Disabling FilterChain alone leaves SymbolProc's independent
            // finding on its predicate block enabled.
            let symbol_proc_remains = i == 1 && config.starts_with("[cops.\"");
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
            unchanged(d.path(), s, &["--only", cop, "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
        )
        .unwrap();
        unchanged(d.path(), s, &["a.rb"], 1);
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
            "ActiveSupportExtensionsEnabled=true",
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
fn directives_clean_and_error_files_retain_bytes_and_mtime() {
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
}
#[test]
fn stdin_selection_and_fix_rejection_leave_disk_unchanged() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        for fix in [false, true] {
            let p = d.path().join("a.rb");
            fs::write(&p, "disk bytes\n").unwrap();
            let time = fs::metadata(&p).unwrap().modified().unwrap();
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
    messages: Vec<String>,
    correction: Option<String>,
}
#[test]
fn all_223_pinned_examples_and_126_unsafe_correction_calls() {
    // RuboCop's three assigned specs (MIT, (c) 2012-26 Bozhidar Batsov).
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/redundant_enumeration_pinned.json")).unwrap();
    assert_eq!(cases.len(), 223);
    let mut corrections = [0; 3];
    let d = project();
    for c in cases {
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
            corrections[COPS.iter().position(|cop| *cop == c.cop).unwrap()] += 1;
            unchanged(
                d.path(),
                c.source.as_bytes(),
                &["--only", &c.cop, "--fix", "a.rb"],
                1,
            );
        }
    }
    assert_eq!(corrections, [16, 30, 80]);
}
#[test]
fn source_envelopes_encoding_line_and_data_retain_exact_bytes() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        for prefix in [
            "",
            "\u{feff}",
            "# coding: UTF-8\n# café\n",
            "# frozen_string_literal: true\n",
        ] {
            let s = format!(
                "{prefix}{}p [__LINE__,__FILE__,DATA.read]\n__END__\nraw payload\n",
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
}
#[test]
fn deferred_heredoc_and_directive_ranges_use_main_ast_membership() {
    let d = project();
    let source = "p <<~TEXT\n  #{xs.each.each { |x| x }}\n  #{xs.select { |x| x }.any?}\n  #{xs.select { it.is_a?(Foo) }}\nTEXT\n";
    for (i, cop) in COPS.iter().enumerate() {
        let found = lint(cop, source);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, i as u32 + 2);
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            1,
        );
    }
    for (cop, source) in [
        (COPS[0], "xs.each\n  .each { it }\n"),
        (COPS[1], "xs.select { it }\n  .any?\n"),
        (COPS[2], "xs.select do |x|\n  x.is_a?(Foo)\nend\n"),
    ] {
        let found = lint(cop, source);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].end_line, if cop == COPS[2] { 3 } else { 2 });
        let s = source.replacen('\n', &format!(" # rustocop:disable {cop}\n"), 1);
        assert!(lint(cop, &s).is_empty());
    }
}
#[test]
fn peer_and_actual_default_fixes_preserve_observations_and_stabilize() {
    let source = "label = 'value'; xs = [1, 2]\na = xs.each.each { |x| x }\nb = xs.select { |x| x.odd? }.any?\nc = xs.select { |x| x.is_a?(Integer) }\np [label,a,b,c]\n";
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
fn many_siblings_use_bounded_local_main_ast_work() {
    // Each cop inspects only a bounded number of immediate edges per call.
    // No whole-file scans, per-node reparses, or scope indexes are needed.
    for (i, cop) in COPS.iter().enumerate() {
        let source = SOURCES[i].repeat(6000);
        let found = lint(cop, &source);
        assert_eq!(found.len(), 6000);
        assert_eq!(found.last().unwrap().line, 6000);
        assert!(found.iter().all(|o| o.correction.is_none()));
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
    assert!(o.status.success() && o.stderr.is_empty(), "{source}: {o:?}");
    o.stdout
}
#[test]
fn controlled_arity_binding_control_flow_laziness_visibility_and_equality_witnesses() {
    // Only these controlled programs execute. Every plausible rewrite changes
    // runtime observations, not merely parseability. No corpus code executes.
    let witnesses = [
        (
            COPS[0],
            "class X; def each; [1,2]; end; end; seen=[]; X.new.each.each { |x| seen << x }; p seen",
            "class X; def each; [1,2]; end; end; seen=[]; X.new.each { |x| seen << x }; p seen",
        ),
        (
            COPS[0],
            "class X; def each_child; self; end; def each_with_index; yield(1,0); end; private; def with_index; end; end; p X.new.each_child.each_with_index { |x,i| [x,i] }",
            "class X; def each_child; self; end; def each_with_index; yield(1,0); end; private; def with_index; end; end; begin; p X.new.each_child.with_index { |x,i| [x,i] }; rescue NoMethodError; puts :private; end",
        ),
        (
            COPS[0],
            "class X; def each_child; self; end; def each_with_index; yield(1,2,3); end; def with_index; yield(1,0); end; end; a=[]; X.new.each_child.each_with_index { |*x| a << x }; p a",
            "class X; def each_child; self; end; def each_with_index; yield(1,2,3); end; def with_index; yield(1,0); end; end; a=[]; X.new.each_child.with_index { |*x| a << x }; p a",
        ),
        (
            COPS[1],
            "seen=[]; xs=[1,2,3]; p xs.select { |x| seen << x; true }.any?; p seen",
            "seen=[]; xs=[1,2,3]; p xs.any? { |x| seen << x; true }; p seen",
        ),
        (
            COPS[1],
            "n=0; predicate=->(x) { n+=1; x.odd? }; p [1,2,3].select(&predicate).any?; p n",
            "n=0; predicate=->(x) { n+=1; x.odd? }; p [1,2,3].any?(&predicate); p n",
        ),
        (
            COPS[1],
            "p [1,2,3].select { |x| break [] if x==2; true }.any?",
            "p [1,2,3].any? { |x| break [] if x==2; true }",
        ),
        (
            COPS[1],
            "seen=[]; p [nil,false].lazy.select { |x| seen << x; true }.any?; p seen",
            "seen=[]; p [nil,false].lazy.any? { |x| seen << x; true }; p seen",
        ),
        (
            COPS[1],
            "class X; def select; [yield(1)]; end; private; def any?; end; end; p X.new.select { |x| x }.any?",
            "class X; def select; [yield(1)]; end; private; def any?; end; end; begin; p X.new.any? { |x| x }; rescue NoMethodError; puts :private; end",
        ),
        (
            COPS[2],
            "module K; def self.===(x); false; end; end; obj=Object.new.extend(K); p [obj].select { |x| x.is_a?(K) }.length",
            "module K; def self.===(x); false; end; end; obj=Object.new.extend(K); p [obj].grep(K).length",
        ),
        (
            COPS[2],
            "class X; def select; [yield(1)]; end; private; def grep(k); []; end; end; p X.new.select { |x| x.is_a?(Integer) }",
            "class X; def select; [yield(1)]; end; private; def grep(k); []; end; end; begin; p X.new.grep(Integer); rescue NoMethodError; puts :private; end",
        ),
        (
            COPS[2],
            "def klass; $n+=1; Integer; end; $n=0; p [1,2,3].select { |x| x.is_a?(klass) }; p $n",
            "def klass; $n+=1; Integer; end; $n=0; p [1,2,3].grep(klass); p $n",
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
