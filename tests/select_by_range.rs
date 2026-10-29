//! pinned examples, real CLI controls and unsafe-rewrite witnesses.
#[path = "support/stdin.rs"]
mod stdin;
use serde::Deserialize;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/SelectByRange";
const SOURCE: &str = "array.select { |x| x.between?(1, 10) }\n";
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
fn lint(source: &str) -> Vec<rustocop::offense::Offense> {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
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
fn default_on_unsafe_correction_policy_and_selectors() {
    let r = rustocop::cops::Registry::new();
    let c = r.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(c.default_enabled());
    assert!(!c.safe_fixes());
    assert_eq!(c.ruby_versions(), Default::default());
    assert!(c.path_policy().is_none());
    let d = project();
    let o = unchanged(d.path(), SOURCE.as_bytes(), &["a.rb"], 1);
    assert!(String::from_utf8_lossy(&o.stdout).contains(COP));
    unchanged(d.path(), SOURCE.as_bytes(), &["--only", COP, "a.rb"], 1);
    unchanged(
        d.path(),
        SOURCE.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        1,
    );
    unchanged(
        d.path(),
        SOURCE.as_bytes(),
        &["--only", COP, "--except", COP, "--fix", "a.rb"],
        0,
    );
    for config in [
        "[cops.Style]\nenabled=false\n".into(),
        format!("[cops.\"{COP}\"]\nenabled=false\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), SOURCE.as_bytes(), &["--fix", "a.rb"], 0);
        unchanged(d.path(), SOURCE.as_bytes(), &["--only", COP, "a.rb"], 1);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n"),
    )
    .unwrap();
    unchanged(d.path(), SOURCE.as_bytes(), &["a.rb"], 1);
}
#[test]
fn excludes_and_unsupported_configuration_are_not_ignored() {
    let d = project();
    for config in [
        "exclude=[\"a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude=[\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            d.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
    }
    for knob in [
        "EnforcedStyle=\"other\"",
        "SafeAutoCorrect=true",
        "TargetRubyVersion=3.4",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{knob}\n"),
        )
        .unwrap();
        unchanged(d.path(), SOURCE.as_bytes(), &["--fix", "a.rb"], 2);
    }
}
#[test]
fn directives_clean_and_error_files_preserve_bytes_and_mtime() {
    let d = project();
    for prefix in ["rubocop", "rustocop"] {
        for target in [COP, "SelectByRange", "Style", "all"] {
            for s in [
                format!("# {prefix}:disable {target}\n{SOURCE}"),
                format!("# {prefix}:disable-next {target}\n{SOURCE}"),
                format!("{} # {prefix}:disable {target}\n", SOURCE.trim_end()),
            ] {
                unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 0);
            }
        }
        let s = format!("# {prefix}:disable {COP}\n{SOURCE}# {prefix}:enable {COP}\n{SOURCE}");
        let found = lint(&s);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 4);
        unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 1);
    }
    for (s, code) in [("p :clean\n", 0), ("(\n", 2)] {
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            code,
        );
    }
}
#[test]
fn stdin_and_fix_rejection_do_not_touch_disk() {
    let d = project();
    for fix in [false, true] {
        let p = d.path().join("a.rb");
        fs::write(&p, "disk bytes\n").unwrap();
        let time = fs::metadata(&p).unwrap().modified().unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
        command
            .current_dir(d.path())
            .args(["--only", COP, "--stdin", "a.rb"]);
        if fix {
            command.arg("--fix");
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        stdin::write_stdin(child.stdin.take().unwrap(), SOURCE.as_bytes(), fix).unwrap();
        let o = child.wait_with_output().unwrap();
        assert_eq!(o.status.code(), Some(if fix { 2 } else { 1 }));
        if fix {
            assert!(String::from_utf8_lossy(&o.stderr).contains("cannot be used with '--fix'"));
        } else {
            assert!(String::from_utf8_lossy(&o.stdout).contains(COP));
        }
        assert_eq!(fs::read(&p).unwrap(), b"disk bytes\n");
        assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
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
fn all_120_pinned_examples_and_88_unsafe_correction_calls() {
    // RuboCop select_by_range_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/select_by_range_pinned.json")).unwrap();
    assert_eq!(cases.len(), 120);
    let d = project();
    let mut corrections = 0;
    for c in cases {
        assert_eq!(c.cop, COP);
        let found = lint(&c.source);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            c.positions,
            "{}:{}",
            COP,
            c.spec_line
        );
        assert_eq!(
            found.iter().map(|o| o.message.as_str()).collect::<Vec<_>>(),
            c.messages,
            "{}:{}",
            COP,
            c.spec_line
        );
        assert!(found.iter().all(|o| o.correction.is_none()));
        unchanged(
            d.path(),
            c.source.as_bytes(),
            &["--only", COP, "a.rb"],
            i32::from(!c.positions.is_empty()),
        );
        if let Some(after) = c.correction {
            assert_ne!(c.source, after);
            corrections += 1;
            unchanged(
                d.path(),
                c.source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                1,
            );
        }
    }
    assert_eq!(corrections, 88);
}
#[test]
fn source_envelopes_encoding_line_data_and_heredoc_membership() {
    let d = project();
    for prefix in [
        "",
        "\u{feff}",
        "# coding: UTF-8\n# café\n",
        "# frozen_string_literal: true\n",
    ] {
        let s = format!("{prefix}{SOURCE}p [__LINE__,__FILE__,DATA.read]\n__END__\nraw payload\n");
        for s in [s.clone(), s.replace('\n', "\r\n")] {
            unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 1);
        }
    }
    let mut s = b"# coding: ISO-8859-1\n# caf\xe9\n".to_vec();
    s.extend_from_slice(SOURCE.as_bytes());
    unchanged(d.path(), &s, &["--only", COP, "--fix", "a.rb"], 1);
    let s = "p <<~TEXT\n  #{xs.select { it.between?(1,10) }}\nTEXT\n";
    let found = lint(s);
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].line, found[0].col), (2, 5));
    unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 1);
    assert!(
        lint("xs.select do |x| # rustocop:disable Style/SelectByRange\n  x.between?(1,10)\nend\n")
            .is_empty()
    );
}
#[test]
fn many_siblings_require_only_bounded_local_ast_work() {
    // Fixed immediate parameter/receiver/body shapes, no reparsing or file scans.
    let found = lint(&SOURCE.repeat(6000));
    assert_eq!(found.len(), 6000);
    assert_eq!(found.last().unwrap().line, 6000);
    assert!(found.iter().all(|o| o.correction.is_none()));
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
fn runtime_witnesses_for_binding_arity_laziness_dispatch_and_equality() {
    // Execute only these controlled programs, never user/corpus code. Each
    // plausible grep rewrite differs observably, even though both parse.
    let witnesses = [
        (
            "def lo; $n+=1; 1; end; $n=0; p [1,2,3].select { |x| x.between?(lo,10) }; p $n",
            "def lo; $n+=1; 1; end; $n=0; p [1,2,3].grep(lo..10); p $n",
        ),
        (
            "p [1,2].select { |x| x.between?(x,x) }",
            "x=10; p [1,2].grep(x..x)",
        ),
        (
            "seen=[]; xs=[1,2,3].lazy; p xs.select { |x| x.between?((seen << x; 1),10) }.take(2).force; p seen",
            "seen=[]; xs=[1,2,3].lazy; p xs.grep((seen << :bound; 1)..10).take(2).force; p seen",
        ),
        (
            "seen=[]; xs=Enumerator.new { |y| [1,2,3].each { |x| seen << x; y << x } }; p xs.find { |x| x.between?(1,10) }; p seen",
            "seen=[]; xs=Enumerator.new { |y| [1,2,3].each { |x| seen << x; y << x } }; p xs.grep(1..10).first; p seen",
        ),
        (
            "class X; include Enumerable; def each; yield 2,3; end; end; begin; p X.new.select { |x| x.between?(1,10) }; rescue NoMethodError; puts :packed; end",
            "class X; include Enumerable; def each; yield 2,3; end; end; begin; p X.new.grep(1..10); rescue NoMethodError; puts :packed; end",
        ),
        (
            "class X; def select; [yield(1)]; end; private; def grep(r); []; end; end; p X.new.select { |x| x.between?(1,10) }",
            "class X; def select; [yield(1)]; end; private; def grep(r); []; end; end; begin; p X.new.grep(1..10); rescue NoMethodError; puts :private; end",
        ),
        (
            "p ['aab','b'].select { |x| ('a'..'z').include?(x) }",
            "p ['aab','b'].grep('a'..'z')",
        ),
        (
            "class X; def between?(a,b); true; end; end; p [X.new].select { |x| x.between?(1,10) }.size",
            "class X; def between?(a,b); true; end; end; p [X.new].grep(1..10).size",
        ),
        (
            "def f; [].select { |x| x.between?((return :early;1),10) }; :done; end; p f",
            "def f; [].grep((return :early;1)..10); :done; end; p f",
        ),
    ];
    let d = project();
    for (source, alternative) in witnesses {
        let found = lint(source);
        assert_eq!(found.len(), 1, "{source}");
        assert!(found[0].correction.is_none());
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
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
fn selected_peer_and_default_fixes_preserve_runtime_observations() {
    let source = "label = 'value'; xs = [1, 2, 3]\na = xs.select { |x| x.between?(1, 2) }\nb = xs.find { |x| (1..2).cover?(x) }\np [label, a, b]\n";
    let selected = format!(
        "{COP},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout"
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
            for (i, r) in rubies().iter().enumerate() {
                assert_eq!(observe(r, &after), expected[i]);
            }
            assert!(String::from_utf8_lossy(&o.stdout).contains(COP));
            if before == after.as_bytes() {
                stable = true;
                break;
            }
        }
        assert!(stable);
    }
}
