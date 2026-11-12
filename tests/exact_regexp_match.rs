//! Pinned RuboCop exact_regexp_match_spec.rb examples (MIT,
//! Copyright (c) 2012-26 Bozhidar Batsov), CLI policy and semantic witnesses.
use serde::Deserialize;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const COP: &str = "Style/ExactRegexpMatch";
const SOURCE: &str = "\"x\".match?(/\\Ax\\z/)\n";
const AFTER: &str = "\"x\" == 'x'\n";
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
    o
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: i32) {
    let p = dir.join("a.rb");
    fs::write(&p, source).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), source);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
}
fn lint(source: &str) -> Vec<rustocop::offense::Offense> {
    let r = rustocop::cops::Registry::new();
    let cfg = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&r, &cfg, &[COP.into()], &[]);
    let result = rustocop::pipeline::lint_one(
        Path::new("a.rb"),
        Path::new("a.rb"),
        Some(source.as_bytes()),
        &active,
        &cfg,
    );
    assert!(
        result.errors.is_empty() && result.internal_errors.is_empty(),
        "{source}: {result:?}"
    );
    result.result.offenses
}
#[test]
fn defaults_selectors_config_and_atomic_canonical_fix() {
    let registry = rustocop::cops::Registry::new();
    let c = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(c.default_enabled() && c.safe_fixes());
    assert_eq!(c.ruby_versions(), Default::default());
    assert!(c.path_policy().is_none());
    let d = project();
    unchanged(d.path(), SOURCE.as_bytes(), &["a.rb"], 1);
    unchanged(d.path(), SOURCE.as_bytes(), &["--only", COP, "a.rb"], 1);
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
    cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), AFTER);
    unchanged(
        d.path(),
        AFTER.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        0,
    );
}
#[test]
fn excludes_directives_clean_errors_and_strict_settings_preserve_bytes() {
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
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "ExactRegexpMatch", "Style", "all"] {
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
fn legacy_multibyte_trailing_backslash_is_literal_not_regexp_syntax() {
    let d = project();
    let s = b"# coding: Windows-31J\ns.match?(/\\A\x83\x5c\\z/)\n";
    fs::write(d.path().join("a.rb"), s).unwrap();
    let o = cli(d.path(), &["--only", COP, "a.rb"], 1);
    assert!(String::from_utf8_lossy(&o.stdout).contains("a.rb:2:1:"));
    unchanged(d.path(), s, &["--only", COP, "--fix", "a.rb"], 1);
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
fn all_25_pinned_calls_and_13_individually_withheld_corrections() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/exact_regexp_match_pinned.json")).unwrap();
    assert_eq!(cases.len(), 25);
    let d = project();
    let mut corrections = vec![];
    for c in cases {
        assert_eq!(c.cop, COP);
        let found = lint(&c.source);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            c.positions,
            "spec:{}",
            c.spec_line
        );
        assert_eq!(
            found.iter().map(|o| &o.message).collect::<Vec<_>>(),
            c.messages.iter().collect::<Vec<_>>(),
            "spec:{}",
            c.spec_line
        );
        unchanged(
            d.path(),
            c.source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            i32::from(!c.positions.is_empty()),
        );
        if let Some(after) = c.correction {
            assert_ne!(after, c.source);
            corrections.push(c.spec_line);
        }
    }
    assert_eq!(
        corrections,
        [5, 16, 27, 44, 55, 70, 81, 145, 156, 167, 178, 189, 131]
    );
}
#[test]
fn source_envelopes_bom_crlf_encoding_line_and_data_bytes() {
    let d = project();
    for prefix in [
        "",
        "\u{feff}",
        "# coding: UTF-8\n# café\n",
        "# coding: ISO-8859-1\n",
        "# frozen_string_literal: true\n",
    ] {
        for nl in ["\n", "\r\n"] {
            let before =
                format!("{prefix}{SOURCE}p __LINE__\n__END__\nraw payload\n").replace('\n', nl);
            let after =
                format!("{prefix}{AFTER}p __LINE__\n__END__\nraw payload\n").replace('\n', nl);
            fs::write(d.path().join("a.rb"), &before).unwrap();
            cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
        }
    }
    for tail in [
        "p DATA.pos\n__END__\npayload\n",
        "p File.binread(__FILE__)\n",
        "p $PROGRAM_NAME\n",
    ] {
        let s = format!("{SOURCE}{tail}");
        unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 1);
    }
    let mut before = b"# coding: ISO-8859-1\n# caf\xe9\n".to_vec();
    before.extend_from_slice(SOURCE.as_bytes());
    let mut after = b"# coding: ISO-8859-1\n# caf\xe9\n".to_vec();
    after.extend_from_slice(AFTER.as_bytes());
    fs::write(d.path().join("a.rb"), &before).unwrap();
    cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), after);
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
    if let Ok(ruby) = crate::support::runtime::ruby() {
        r.push(ruby);
    }
    r
}
fn observe(ruby: &str, s: &str) -> Vec<u8> {
    let o = Command::new(ruby).args(["-e", s]).output().unwrap();
    assert!(o.status.success() && o.stderr.is_empty(), "{s}: {o:?}");
    o.stdout
}
#[test]
fn runtime_match_state_result_values_nil_dispatch_and_literal_semantics() {
    let d = project();
    let witnesses = [
        ("p('x' =~ /\\Ax\\z/); p $~", "p('x' == 'x'); p $~"),
        (
            "p 'x'.match(/\\Ax\\z/).class; p $~",
            "p('x' == 'x').class; p $~",
        ),
        ("if 'x' !~ /\\Ax\\z/; end; p $~", "if 'x' != 'x'; end; p $~"),
        (
            "if /\\Ax\\z/ === 'x'; end; p $~",
            "if 'x' == 'x'; end; p $~",
        ),
        ("s=nil; p s&.match?(/\\Ax\\z/)", "s=nil; p(s == 'x')"),
        (
            "class String; def ==(x); false; end; end; p 'x'.match?(/\\Ax\\z/)",
            "class String; def ==(x); false; end; end; p('x' == 'x')",
        ),
        (
            "s='x'; s.define_singleton_method(:match?) { |r| false }; p s.match?(/\\Ax\\z/)",
            "s='x'; s.define_singleton_method(:match?) { |r| false }; p(s == 'x')",
        ),
    ];
    for (source, unsafe_after) in witnesses {
        assert_eq!(lint(source).len(), 1);
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        for ruby in rubies() {
            assert_ne!(observe(&ruby, source), observe(&ruby, unsafe_after));
        }
    }
    // The prior captures survive match? and its safe equality replacement.
    for subject in ["'x'", "'not x'", r#""foo'bar""#, "'foo/bar'", "''"] {
        let s = format!(
            "/(seed)/ =~ 'seed'; p [{subject}.match?(/\\Afoo'bar\\z/), $~, $1, __LINE__]\n"
        );
        // A state-changing prior call is deliberately not trusted by the proof.
        unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 1);
        for ruby in rubies() {
            let after = s.replace(
                &format!("{subject}.match?(/\\Afoo'bar\\z/)"),
                &format!("{subject} == 'foo\\'bar'"),
            );
            assert_eq!(observe(&ruby, &s), observe(&ruby, &after));
        }
    }
}
#[test]
fn modifier_predicate_effects_execute_before_the_source_earlier_body() {
    let d = project();
    for (modifier, effect, expected) in [
        ("if", "''.class.undef_method('==')", "true\n"),
        ("unless", "(''.class.undef_method('=='); false)", "true\n"),
        ("if", "(true if ''.class.undef_method('=='))", "true\n"),
        (
            "if",
            "(''.class.undef_method('match?'); true)",
            "NoMethodError\n",
        ),
    ] {
        let source = format!("a = 'x'.match?(/\\Ax\\z/) {modifier} {effect}; p a\n");
        assert_eq!(lint(&source).len(), 1);
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        let observed = format!("begin; {source}; rescue NoMethodError; puts 'NoMethodError'; end");
        let unsafe_after = observed.replace("'x'.match?(/\\Ax\\z/)", "('x' == 'x')");
        for ruby in rubies() {
            assert_eq!(observe(&ruby, &observed), expected.as_bytes());
            assert_ne!(observe(&ruby, &observed), observe(&ruby, &unsafe_after));
        }
    }
}
#[test]
fn escaped_delimiters_quotes_decoded_subjects_and_boolean_consumers_observe_equally() {
    let d = project();
    for expr in [
        r#""foo'bar".match?(/\Afoo'bar\z/)"#,
        r"'foo/bar'.match?(/\Afoo\/bar\z/)",
        r"!('x'.match?(/\Ax\z/))",
        r"'x'.match?(/\Ax\z/).to_s",
        r#""\u0078"&.match?(/\Ax\z/)"#,
        r"if 'x'.match?(/\Ax\z/); true; else; false; end",
        r"'y'.match?(/\Ax\z/)",
    ] {
        let source = format!("p({expr}); p [$~, __LINE__]\n");
        let path = d.path().join("a.rb");
        fs::write(&path, &source).unwrap();
        cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
        let after = fs::read_to_string(&path).unwrap();
        assert!(!after.contains("match?"), "{after}");
        for ruby in rubies() {
            assert_eq!(observe(&ruby, &source), observe(&ruby, &after));
        }
        unchanged(
            d.path(),
            after.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
    }
}
#[test]
fn canonical_fixes_preserve_runtime_and_peer_fixes_stabilize() {
    for only in [
        Some(COP.to_string()),
        Some(format!(
            "{COP},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout"
        )),
        None,
    ] {
        let d = project();
        let path = d.path().join("a.rb");
        let source = "a = 'x'.match?(/\\Ax\\z/); b = 'y'.match?(/\\Ax\\z/); p [a,b,$~,__LINE__]\n";
        fs::write(&path, source).unwrap();
        let mut seen = std::collections::HashSet::new();
        let mut stable = false;
        for _ in 0..5 {
            let before = fs::read(&path).unwrap();
            assert!(seen.insert(before.clone()));
            let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            command.current_dir(d.path());
            if let Some(only) = &only {
                command.args(["--only", only]);
            }
            let o = command.args(["--fix", "a.rb"]).output().unwrap();
            assert!(matches!(o.status.code(), Some(0 | 1)), "{o:?}");
            let after = fs::read_to_string(&path).unwrap();

            for ruby in rubies() {
                assert_eq!(observe(&ruby, source), observe(&ruby, &after));
            }
            if before == after.as_bytes() {
                assert!(!after.contains("match?"), "{after}");
                stable = true;
                break;
            }
        }
        assert!(stable);
    }
}
