//! pinned RuboCop redundant_regexp_escape_spec.rb examples (MIT,
//! Copyright (c) 2012-26 Bozhidar Batsov).
use serde::Deserialize;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const COP: &str = "Style/RedundantRegexpEscape";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(dir: &Path, args: &[&str], code: i32) {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(code), "{args:?}: {o:?}");
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: i32) {
    let p = dir.join("a.rb");
    fs::write(&p, source).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), source);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
}
fn lint(s: &str) -> Vec<rustocop::offense::Offense> {
    let r = rustocop::cops::Registry::new();
    let cfg = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&r, &cfg, &[COP.into()], &[]);
    let result = rustocop::pipeline::lint_one(
        Path::new("a.rb"),
        Path::new("a.rb"),
        Some(s.as_bytes()),
        &active,
        &cfg,
    );
    assert!(
        result.errors.is_empty() && result.internal_errors.is_empty(),
        "{s}: {result:?}"
    );
    result.result.offenses
}
#[derive(Deserialize)]
struct Case {
    spec_line: u32,
    source: String,
    positions: Vec<(u32, u32)>,
    messages: Vec<String>,
    correction: Option<String>,
}
#[test]
fn all_pinned_diagnostics_and_corrections() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/redundant_regexp_escape_pinned.json")).unwrap();
    assert_eq!(cases.len(), 218);
    let d = project();
    let mut corrected = 0;
    let mut withheld = 0;
    for c in cases {
        let found = lint(&c.source);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            c.positions,
            "spec line {}: {}",
            c.spec_line,
            c.source
        );
        assert_eq!(
            found.iter().map(|o| o.message.clone()).collect::<Vec<_>>(),
            c.messages,
            "spec line {}",
            c.spec_line
        );
        if let Some(after) = c.correction {
            // Every original correction example assigns/exports the Regexp.
            // Retain those exact sources and findings, rather than treating
            // matching equivalence as proof that their values cannot escape.
            unchanged(
                d.path(),
                c.source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                1,
            );
            withheld += 1;
            // Independently retain exact safe discarded-literal counterparts.
            // 249 interpolates a nested Regexp; 390 is dynamic; 646 indexes s.
            if let Some(before) = c
                .source
                .strip_prefix("foo = ")
                .or_else(|| c.source.strip_prefix("r = "))
                && c.spec_line != 249
            {
                let after = after
                    .strip_prefix("foo = ")
                    .or_else(|| after.strip_prefix("r = "))
                    .unwrap();
                fs::write(d.path().join("a.rb"), before).unwrap();
                cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
                assert_eq!(
                    fs::read_to_string(d.path().join("a.rb")).unwrap(),
                    after,
                    "spec line {}",
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
    assert_eq!((corrected, withheld), (28, 31));
}
#[test]
fn default_selectors_config_excludes_directives_and_no_write() {
    let d = project();
    let source = b"/a\\-b/\n";
    let after = b"/a-b/\n";
    let registry = rustocop::cops::Registry::new();
    let rule = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(rule.default_enabled() && rule.safe_fixes());
    unchanged(d.path(), source, &["a.rb"], 1);
    unchanged(d.path(), source, &["--only", COP, "a.rb"], 1);
    unchanged(
        d.path(),
        source,
        &["--only", COP, "--except", COP, "--fix", "a.rb"],
        0,
    );
    for config in [
        "[cops.Style]\nenabled=false\n".into(),
        format!("[cops.\"{COP}\"]\nenabled=false\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), source, &["--fix", "a.rb"], 0);
        cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), after);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n"),
    )
    .unwrap();
    fs::write(d.path().join("a.rb"), source).unwrap();
    cli(d.path(), &["--fix", "a.rb"], 0);
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), after);
    for config in [
        "exclude=[\"a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude=[\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], 0);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.\"{COP}\"]\nEnforcedStyle=\"other\"\n"),
    )
    .unwrap();
    unchanged(d.path(), source, &["--fix", "a.rb"], 2);
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "Style", "all"] {
            for s in [
                format!("# {prefix}:disable {target}\n/a\\-b/\n"),
                format!("# {prefix}:disable-next {target}\n/a\\-b/\n"),
                format!("/a\\-b/ # {prefix}:disable {target}\n"),
            ] {
                unchanged(d.path(), s.as_bytes(), &["--only", COP, "--fix", "a.rb"], 0);
            }
        }
    }
    unchanged(d.path(), after, &["--only", COP, "--fix", "a.rb"], 0);
    unchanged(d.path(), b"(\n", &["--only", COP, "--fix", "a.rb"], 2);
}
#[test]
fn independent_safe_siblings_apply_beside_withheld_dynamic_patterns() {
    let d = project();
    let source = "/a\\-b/\nx = '[a'; /#{x}\\-z]/\n/[a\\&&b]/\n/[\\+]/\n";
    let after = "/a-b/\nx = '[a'; /#{x}\\-z]/\n/[a\\&&b]/\n/[+]/\n";
    fs::write(d.path().join("a.rb"), source).unwrap();
    cli(d.path(), &["--only", COP, "--fix", "a.rb"], 1);
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
    unchanged(
        d.path(),
        after.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        1,
    );
}
#[test]
fn source_observers_and_data_position_barriers_preserve_bytes() {
    let d = project();
    for observer in [
        "p __FILE__",
        "p File.binread($0)",
        "p method(:p).source_location",
        "p DATA.pos\n__END__\npayload",
    ] {
        let source = format!("/a\\-b/\n{observer}\n");
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
    }
}
#[test]
fn encoding_bom_crlf_line_and_data_bytes() {
    let d = project();
    for prefix in [
        "",
        "\u{feff}",
        "# coding: UTF-8\n",
        "# coding: ISO-8859-1\n",
    ] {
        for nl in ["\n", "\r\n"] {
            let source =
                format!("{prefix}/a\\-b/imx\np __LINE__\n__END__\n/a\\-b/\n").replace('\n', nl);
            let after =
                format!("{prefix}/a-b/imx\np __LINE__\n__END__\n/a\\-b/\n").replace('\n', nl);
            fs::write(d.path().join("a.rb"), source).unwrap();
            cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
        }
    }
    for (source, after) in [
        (
            b"# coding: Windows-31J\n/\x83\x5c\\-\x83\x7b/\n".as_slice(),
            b"# coding: Windows-31J\n/\x83\x5c-\x83\x7b/\n".as_slice(),
        ),
        (
            b"# coding: ISO-8859-1\n/[\\\xa7]\\-\\\xe9/\n",
            b"# coding: ISO-8859-1\n/[\xa7]-\\\xe9/\n",
        ),
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), after);
    }
}
fn ruby(dir: &Path, executable: &std::ffi::OsStr) -> Output {
    Command::new(executable)
        .current_dir(dir)
        .arg("a.rb")
        .output()
        .unwrap()
}
#[test]
fn runtime_options_encodings_captures_backreferences_evaluation_and_match_state() {
    let d = project();
    let source = r##"$events = []
def mark; $events << :mark; 'a'; end
patterns = [/a\-b/im, /[\+\?\$\|]/, %r{a\/b}x, /(?<part>a)\-\k<part>/,
 /(a)(b)\-\2\1/, /(?x:a # [\.] \-
 \-)/, /(?-x:#\-)/, /[[:punct:]&&[^\.]]/, /[\§]/u, /\-/n,
 /a#{/\-/}b/, /#{mark}[\+]/o]
patterns.each do |r|
 p [r.options, r.encoding.name, r.names]
 ['', '-', '+', 'a-b', 'a-a', 'ab-ba', 'a/b', '?', '#-', '.', '§', 'a+'].each do |s|
  begin
   match = r.match(s)
   p [match&.to_a, match&.named_captures, $~&.to_a, $1, $2, $&, $`, $']
  rescue Encoding::CompatibilityError => e; p e.class
  end
 end
end
/(?<local>a)\-/ =~ 'a-'
p [local, $1, $~.to_a]
$_ = 'a-'
if /(?<implicit>a)\-/; p [$1, $~.to_a]; end
2.times { p /#{mark}[\+]/o.match?('a+') }
p [$events, __LINE__]
"##;
    let mut rubies = vec!["ruby".into()];
    if let Some(native) = std::env::var_os("RUSTOCOP_RUBY34") {
        rubies.push(native);
    }
    for executable in rubies {
        if Command::new(&executable).arg("--version").output().is_err() {
            continue;
        }
        for prefix in [
            "",
            "# frozen_string_literal: true\n",
            "# frozen_string_literal: false\n",
        ] {
            for nl in ["\n", "\r\n"] {
                let source = format!("{prefix}{source}").replace('\n', nl);
                fs::write(d.path().join("a.rb"), &source).unwrap();
                let before = ruby(d.path(), &executable);
                assert!(before.status.success(), "{before:?}");
                // Stored/interpolated and implicit-match values can escape.
                cli(d.path(), &["--only", COP, "--fix", "a.rb"], 1);
                assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), source.as_bytes());
                let after = ruby(d.path(), &executable);
                assert_eq!(before.status, after.status);
                assert_eq!(before.stdout, after.stdout);
                assert_eq!(before.stderr, after.stderr);
            }
        }
    }
}
#[test]
fn withheld_edits_have_concrete_runtime_semantic_witnesses() {
    let d = project();
    let mut rubies = vec!["ruby".into()];
    if let Some(native) = std::env::var_os("RUSTOCOP_RUBY34") {
        rubies.push(native);
    }
    for executable in rubies {
        if Command::new(&executable).arg("--version").output().is_err() {
            continue;
        }
        for (source, naive) in [
            (r"p /a{1\,2}/.match?('aa')", r"p /a{1,2}/.match?('aa')"),
            (r"p /a{\,2}/.match?('aa')", r"p /a{,2}/.match?('aa')"),
            (r"p /a{1\,}/.match?('aa')", r"p /a{1,}/.match?('aa')"),
            (r"p /[a\&&b]/.match?('a')", r"p /[a&&b]/.match?('a')"),
            (
                r"p /[[\:digit:]]/.match?('d')",
                r"p /[[:digit:]]/.match?('d')",
            ),
            (
                r"r = /[[:digit\:]]/; p [r.match?('1'), r.match?('d')]",
                r"r = /[[:digit:]]/; p [r.match?('1'), r.match?('d')]",
            ),
            (
                r"r = /[[\:digit\:]]/; p [r.match?('1'), r.match?('d')]",
                r"r = /[[:digit:]]/; p [r.match?('1'), r.match?('d')]",
            ),
            (
                r"name = 'a'; p /[#\{name}]/.match?('#')",
                r"name = 'a'; p /[#{name}]/.match?('#')",
            ),
            (
                r"x = '[a'; p /#{x}\-z]/.match?('m')",
                r"x = '[a'; p /#{x}-z]/.match?('m')",
            ),
            (
                r"@name = 'a'; @name_ = 'z'; p /#@name\_/.match?('a_')",
                r"@name = 'a'; @name_ = 'z'; p /#@name_/.match?('a_')",
            ),
        ] {
            fs::write(d.path().join("a.rb"), source).unwrap();
            let before = ruby(d.path(), &executable);
            assert!(before.status.success(), "{source}: {before:?}");
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                1,
            );
            fs::write(d.path().join("a.rb"), naive).unwrap();
            let after = ruby(d.path(), &executable);
            assert!(after.status.success(), "{naive}: {after:?}");
            assert_ne!(before.stdout, after.stdout, "{source}: {before:?}");
        }
    }
}
#[test]
fn runtime_named_tokens_preserve_necessary_escapes_and_safe_siblings() {
    let d = project();
    let mut rubies = vec!["ruby".into()];
    if let Some(native) = std::env::var_os("RUSTOCOP_RUBY34") {
        rubies.push(native);
    }
    for executable in rubies {
        if Command::new(&executable).arg("--version").output().is_err() {
            continue;
        }
        for pattern in [
            r"/(?<a[b>x)\+/",
            r"/(?'a[b'x)\+/",
            r"/(?<a[b>x)\k<a[b>\+/",
            r"/(?'a[b'x)\k'a[b'\+/",
            r"/(?<a[b>x)\g<a[b>\+/",
            r"/(?'a[b'x)\g'a[b'\+/",
        ] {
            let source = format!(
                "r = {pattern}; p ['x', 'xx', 'x+', 'xx+'].map {{ |s| r.match?(s) }}\n/a\\-b/\n"
            );
            let expected = source.replace(r"/a\-b/", "/a-b/");
            fs::write(d.path().join("a.rb"), &source).unwrap();
            let before = ruby(d.path(), &executable);
            assert!(before.status.success(), "{source}: {before:?}");
            cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), expected);
            let after = ruby(d.path(), &executable);
            assert_eq!(before.status, after.status);
            assert_eq!(before.stdout, after.stdout);
            assert_eq!(before.stderr, after.stderr);
            // The previously emitted edit was parseable but changed matches.
            let naive = source.replace(r"\+", "+");
            fs::write(d.path().join("a.rb"), naive).unwrap();
            let changed = ruby(d.path(), &executable);
            assert!(changed.status.success(), "{changed:?}");
            assert_ne!(before.stdout, changed.stdout, "{source}");
        }
    }
}
#[test]
fn runtime_legacy_encoding_and_option_observations() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let d = project();
    for source in [
        b"# coding: Windows-31J\nr = /\x83\x5c\\-/; p [r.options, r.encoding.name, r.match(\"\x83\x5c-\").to_a]\n".as_slice(),
        b"# coding: ISO-8859-1\nr = /[\\\xa7]\\-/; p [r.options, r.encoding.name, r.match(\"\xa7-\").to_a]\n",
        b"p [/a\\-/e, /a\\-/s].map { |r| [r.options, r.encoding.name, r.match('a-').to_a] }\n",
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let before = ruby(d.path(), std::ffi::OsStr::new("ruby"));
        assert!(before.status.success(), "{before:?}");
        unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], 1);
        let after = ruby(d.path(), std::ffi::OsStr::new("ruby"));
        assert_eq!(before.status, after.status);
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
    }
}
#[test]
fn regexp_literal_incompatibility_and_peer_default_composition_converge() {
    let d = project();
    let source = "patterns = [%r!a\\-b!, /[\\+]/, %r{a\\/b}]\n";
    fs::write(d.path().join("a.rb"), source).unwrap();
    cli(
        d.path(),
        &[
            "--only",
            "Style/RedundantRegexpEscape,Style/RegexpLiteral,Style/RedundantRegexpCharacterClass",
            "--fix",
            "a.rb",
        ],
        1,
    );
    assert_eq!(
        fs::read_to_string(d.path().join("a.rb")).unwrap(),
        "patterns = [/a\\-b/, /[\\+]/, %r{a\\/b}]\n"
    );
    for only in [
        Some(COP),
        Some("Style/RedundantRegexpEscape,Style/RegexpLiteral,Style/RedundantRegexpCharacterClass"),
        Some("Style,Layout"),
        None,
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let mut stable = false;
        for _ in 0..6 {
            let before = fs::read(d.path().join("a.rb")).unwrap();
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            cmd.current_dir(d.path()).args(["--fix", "a.rb"]);
            if let Some(only) = only {
                cmd.args(["--only", only]);
            }
            let output = cmd.output().unwrap();
            assert!(matches!(output.status.code(), Some(0 | 1)), "{output:?}");
            let after = fs::read(d.path().join("a.rb")).unwrap();
            if before == after {
                stable = true;
                break;
            }
        }
        assert!(stable);
        // Requested cops remain active, but exported patterns retain spelling.
        assert!(
            fs::read_to_string(d.path().join("a.rb"))
                .unwrap()
                .contains("\\-")
        );
    }
}
