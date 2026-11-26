//! Pinned regexp_literal_spec.rb examples (MIT, Copyright
//! (c) 2012-26 Bozhidar Batsov), CLI policy and regexp semantic witnesses.
use crate::support::cli::project;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/RegexpLiteral";
const SOURCE: &str = "p(%r{a})\n";
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
            assert_eq!(
                fs::read_to_string(d.path().join("a.rb")).unwrap(),
                crate::support::lf::lf_str(&after)
            );
        }
    }
}
use crate::support::source_getter;

fn ruby(dir: &Path) -> Output {
    Command::new(crate::support::runtime::ruby_os().unwrap_or_else(|| "ruby".into()))
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
