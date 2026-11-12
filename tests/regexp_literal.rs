//! Pinned regexp_literal_spec.rb examples (MIT, Copyright
//! (c) 2012-26 Bozhidar Batsov), CLI policy and regexp semantic witnesses.
use serde::Deserialize;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/RegexpLiteral";
const SOURCE: &str = "p(%r{a})\n";
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
fn default_selection_config_exclusions_and_no_write_contract() {
    let r = rustocop::cops::Registry::new();
    let c = r.cops().iter().find(|c| c.name() == COP).unwrap();
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
        cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
        assert_eq!(
            fs::read_to_string(d.path().join("a.rb")).unwrap(),
            "p(/a/)\n"
        );
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n"),
    )
    .unwrap();
    fs::write(d.path().join("a.rb"), SOURCE).unwrap();
    cli(d.path(), &["--fix", "a.rb"], 0);
    assert_eq!(
        fs::read_to_string(d.path().join("a.rb")).unwrap(),
        "p(/a/)\n"
    );
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
        "EnforcedStyle=\"percent_r\"",
        "AllowInnerSlashes=true",
        "PreferredDelimiters=\"[]\"",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{knob}\n"),
        )
        .unwrap();
        unchanged(d.path(), SOURCE.as_bytes(), &["--fix", "a.rb"], 2);
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for (s, code) in [("p(/a/)\n", 0), ("(\n", 2)] {
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            code,
        );
    }
}
#[test]
fn directives_disable_whole_atomic_envelopes_and_stdin_never_writes() {
    let d = project();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "RegexpLiteral", "Style", "all"] {
            for s in [
                format!("# {prefix}:disable {target}\n{SOURCE}"),
                format!("# {prefix}:disable-next {target}\n{SOURCE}"),
                format!("{} # {prefix}:disable {target}\n", SOURCE.trim_end()),
            ] {
                unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 0);
            }
        }
        let s = format!("# {prefix}:disable {COP}\n{SOURCE}# {prefix}:enable {COP}\n{SOURCE}");
        assert_eq!(lint(&s).iter().map(|o| o.line).collect::<Vec<_>>(), [4]);
    }
    let s = "%r{\n#{1 # rubocop:disable Style/RegexpLiteral\n}\n}\n";
    unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 0);
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", COP, "--stdin", "a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(SOURCE.as_bytes())
        .unwrap();
    assert_eq!(child.wait_with_output().unwrap().status.code(), Some(1));
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), s);
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
fn all_37_pinned_calls_and_24_correction_calls() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/regexp_literal_pinned.json")).unwrap();
    assert_eq!(cases.len(), 37);
    let d = project();
    let mut corrected = 0;
    let mut withheld = vec![];
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
        if let Some(after) = c.correction {
            if [244, 255].contains(&c.spec_line) {
                withheld.push(c.spec_line);
                unchanged(
                    d.path(),
                    c.source.as_bytes(),
                    &["--only", COP, "--fix", "a.rb"],
                    1,
                );
            } else {
                fs::write(d.path().join("a.rb"), &c.source).unwrap();
                cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
                assert_eq!(
                    fs::read_to_string(d.path().join("a.rb")).unwrap(),
                    after,
                    "spec:{}",
                    c.spec_line
                );
                unchanged(
                    d.path(),
                    after.as_bytes(),
                    &["--only", COP, "--fix", "a.rb"],
                    0,
                );
                corrected += 1;
            }
        }
    }
    assert_eq!(corrected, 22);
    assert_eq!(withheld, [244, 255]);
}
#[test]
fn unusual_encoding_source_observers_and_data_positions_withhold_individually() {
    let d = project();
    for s in [
        "%r hello \n",
        "%r\thello\t\n",
        "%r\nhello\n",
        "%r\\hello\\\n",
        "%r#a\\#{b}#\n",
        "%r#a\\#{b}#x\n",
        "%r#a\\#@b#\n",
        "%r#a\\#$b#\n",
        "%r@#\\@foo@\n",
        "%r@#\\@foo@x\n",
        "%r@#\\@\\@foo@\n",
        "%r@#\\@\\@foo@x\n",
        "%r@#{1}#\\@foo@\n",
        "if %r@#\\@foo@; end\n",
    ] {
        assert_eq!(lint(s).len(), 1, "{s:?}");
        unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 1);
    }
    for observer in [
        "p __FILE__",
        "p File.binread($0)",
        "p method(:p).source_location",
        "p DATA.pos\n__END__\npayload",
    ] {
        unchanged(
            d.path(),
            format!("{SOURCE}{observer}\n").as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
    }
    for s in [
        b"# coding: ISO-8859-1\np %r{caf\xe9}\n".as_slice(),
        b"# coding: Windows-31J\np %r{\x83\x5c}\n",
    ] {
        unchanged(d.path(), s, &["--only", COP, "--fix", "a.rb"], 1);
    }
}
#[test]
fn multibyte_trail_backslashes_and_braces_are_not_regexp_syntax() {
    let d = project();
    for (source, code) in [
        (b"# coding: Windows-31J\nr = /\x83\x5c{\\//\n".as_slice(), 0),
        (b"# coding: Windows-31J\nr = /\x83\x5c}\\//\n".as_slice(), 0),
        (b"# coding: Windows-31J\nr = /\x83\x7b\\//\n".as_slice(), 1),
    ] {
        unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], code);
    }
}
#[test]
fn bom_crlf_options_line_and_end_data_are_preserved() {
    let d = project();
    for prefix in [
        "",
        "\u{feff}",
        "# coding: UTF-8\n",
        "# coding: ISO-8859-1\n",
    ] {
        for nl in ["\n", "\r\n"] {
            let source =
                format!("{prefix}p [%r{{a}}imx, /home\\//n]\np __LINE__\n__END__\n%r{{raw}}\n")
                    .replace('\n', nl);
            let after =
                format!("{prefix}p [/a/imx, %r{{home/}}n]\np __LINE__\n__END__\n%r{{raw}}\n")
                    .replace('\n', nl);
            fs::write(d.path().join("a.rb"), &source).unwrap();
            cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
        }
    }
}
use crate::support::source_getter;

fn ruby(dir: &Path) -> Output {
    Command::new(std::env::var_os("RUSTOCOP_RUBY34").unwrap_or_else(|| "ruby".into()))
        .current_dir(dir)
        .arg("a.rb")
        .output()
        .unwrap()
}
#[test]
fn runtime_regexp_sources_options_encodings_captures_backrefs_and_match_state() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let d = project();
    let s = r##"$events = []
def mark(x); $events << x; x; end
patterns = [%r!(?\!x)!, %r!a\\\!b!, %r-[a\-z]-, %r#a\#b#x, %r{a\{b},
 /users\/#{mark('abc')}\/forms/im, /{foo}\//, /\{foo\//, %r{^#{mark('a')}\s}x,
 %r{(?<part>a)(b)\k<part>}im, %r{(a)(b)\2\1}im, %r{café}u, /abc\//n, %r'a\'b', %r{a}o,
 %r{a}u, %r{a}n]
patterns.each do |r|
  p [r.source.bytes, r.options, r.encoding.name]
  ['a', 'ab', 'abba', 'x', 'az', 'a-z', 'a{b', 'users/abc/forms', '{foo}/', 'café'].each do |s|
    match = begin; r.match(s); rescue Encoding::CompatibilityError => e; p e.class; next; end
    p [match&.to_a, $~&.to_a, $1, $2, $&, $`, $', Regexp.last_match&.to_a]
  end
end
%r{(?<local>a)} =~ 'a'
p [local, $1, $~.to_a]
$_ = 'a'
if %r{(?<implicit>a)}; p [$1, $~.to_a]; end
if /#{mark('a')}\//; p $~; end
2.times { p /#{mark('a')}\//o }
p [$events, __LINE__]
"##;
    let s = source_getter::pattern_sibling(d.path(), "a.rb", s, COP);
    fs::write(d.path().join("a.rb"), s).unwrap();
    let before = ruby(d.path());
    assert!(before.status.success(), "{before:?}");
    cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
    let after = ruby(d.path());
    assert_eq!(before.status, after.status);
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
    let text = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert!(text.contains("/(?<local>a)/ =~"));
    assert!(text.contains("if /(?<implicit>a)/"));
    assert!(text.contains("%r{users/#{mark('abc')}/forms}im"));
    assert!(text.contains("%r{#{mark('a')}/}o"));
}
#[test]
fn runtime_legacy_regexp_encoding_options_use_the_full_host_ruby() {
    // miniruby lacks the dynamically loaded EUC-JP / Windows-31J encodings.
    // Keep explicit e/s runtime coverage on the full host Ruby, in addition to
    // fixed-3.4 parsing and exact correction bytes in the cop's unit tests.
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let d = project();
    let source = "p [%r{a}e, %r{a}s].map { |r| [r.source.bytes, r.options, r.encoding.name, r.match('abc').to_a] }\n";
    let source = source_getter::pattern_sibling(d.path(), "a.rb", source, COP);
    fs::write(d.path().join("a.rb"), source).unwrap();
    let observe = || {
        Command::new("ruby")
            .current_dir(d.path())
            .arg("a.rb")
            .output()
            .unwrap()
    };
    let before = observe();
    assert!(before.status.success(), "{before:?}");
    cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
    let after = observe();
    assert_eq!(before.status, after.status);
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
}
#[test]
fn withheld_hash_interpolation_rewrites_would_change_regexp_source() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let d = project();
    for (s, naive) in [
        (r"p %r#a\#{b}#.source", r"p /a\#{b}/.source"),
        (r"p %r#a\#{b}#x.source", r"p /a#\{b}/x.source"),
    ] {
        fs::write(d.path().join("a.rb"), s).unwrap();
        let before = ruby(d.path());
        assert!(before.status.success());
        unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 1);
        fs::write(d.path().join("a.rb"), naive).unwrap();
        let changed = ruby(d.path());
        assert!(changed.status.success());
        assert_ne!(before.stdout, changed.stdout);
    }
}
#[test]
fn withheld_at_delimiter_unescaping_would_introduce_variable_interpolation() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let d = project();
    for (pattern, naive, subject) in [
        (r"%r@#\@foo@", "/#@foo/", "#@foo"),
        (r"%r@#\@\@foo@", "/#@@foo/", "#@@foo"),
    ] {
        for options in ["", "x"] {
            let program = |regexp: &str| {
                format!(
                    "class Witness\n  @foo = 'a'\n  @@foo = 'a'\n  r = {regexp}{options}\n  p [r.source.bytes, r.options, r.match?('{subject}'), r.match?('a')]\nend\n"
                )
            };
            let source = program(pattern);
            assert_eq!(lint(&source).len(), 1);
            fs::write(d.path().join("a.rb"), &source).unwrap();
            let before = ruby(d.path());
            assert!(before.status.success(), "{before:?}");
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                1,
            );
            let after = ruby(d.path());
            assert_eq!(before.stdout, after.stdout);
            assert_eq!(before.stderr, after.stderr);
            fs::write(d.path().join("a.rb"), program(naive)).unwrap();
            let changed = ruby(d.path());
            assert!(changed.status.success(), "{changed:?}");
            assert_ne!(before.stdout, changed.stdout);
            if options.is_empty() {
                assert!(String::from_utf8_lossy(&before.stdout).ends_with(", true, false]\n"));
                assert!(String::from_utf8_lossy(&changed.stdout).ends_with(", false, true]\n"));
            }
        }
    }
}
#[test]
fn nested_fragments_are_bounded_and_composed_fixes_stabilize() {
    let d = project();
    // Each literal's static text is visited once; even nested atomic envelopes
    // are bounded by the pipeline's fixed 64-node depth budget.
    let mut s = "%r{a}".to_string();
    for _ in 0..8 {
        s = format!("%r{{#{{{s}}}}}");
    }
    fs::write(d.path().join("a.rb"), &s).unwrap();
    for _ in 0..9 {
        cli(
            d.path(),
            &["--only", COP, "--fix", "--fix-passes", "1", "a.rb"],
            i32::from(s.matches("%r{").count() > 1),
        );
        s = fs::read_to_string(d.path().join("a.rb")).unwrap();
    }
    assert!(!s.contains("%r{"));
    let source = "def f\n  return ({ :a => 'hi' });\nend\np [f, %r!abc!, /a\\//]\n";
    for only in [
        Some(
            "Style/RegexpLiteral,Style/PercentLiteralDelimiters,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout",
        ),
        None,
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let before = Command::new("ruby")
            .arg("--version")
            .output()
            .ok()
            .map(|_| ruby(d.path()));
        let mut stable = false;
        for _ in 0..6 {
            let old = fs::read(d.path().join("a.rb")).unwrap();
            let mut args = vec!["--fix", "--fix-passes", "1", "a.rb"];
            if let Some(only) = only {
                args.extend(["--only", only]);
            }
            let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(args)
                .output()
                .unwrap();
            assert_ne!(o.status.code(), Some(2), "{o:?}");
            if let Some(before) = &before {
                let after = ruby(d.path());
                assert_eq!(before.stdout, after.stdout);
                assert_eq!(before.stderr, after.stderr);
            }
            if old == fs::read(d.path().join("a.rb")).unwrap() {
                stable = true;
                break;
            }
        }
        assert!(stable);
        let s = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert!(s.contains("/abc/"));
        assert!(s.contains("%r{a/}"));
    }
}
