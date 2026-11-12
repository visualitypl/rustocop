//! all three fixed hash suggestions are detection-only.
use crate::support::stdin;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const FETCH: &str = "Style/HashFetchChain";
const CASE: &str = "Style/HashLikeCase";
const LOOKUP: &str = "Style/HashLookupMethod";
const CASE_SOURCE: &str = "case x\nwhen :a\n  1\nwhen :b\n  2\nwhen :c\n  3\nend\n";
fn examples() -> [(&'static str, &'static str, bool); 3] {
    [
        (FETCH, "hash.fetch(:a, {}).fetch(:b, nil)\n", true),
        (CASE, CASE_SOURCE, true),
        (LOOKUP, "hash.fetch(key)\n", false),
    ]
}
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: i32) -> Output {
    let p = dir.join("a.rb");
    fs::write(&p, source).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(code), "{args:?}: {o:?}");
    assert!(o.stderr.is_empty() || code == 2, "{o:?}");
    assert_eq!(fs::read(&p).unwrap(), source);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
    o
}
#[test]
fn default_opt_in_version_and_pipeline_no_corrections() {
    use rustocop::cops::policy::{DEFAULT_RUBY_VERSION, RubyVersion};
    let registry = rustocop::cops::Registry::new();
    for (name, s, enabled) in examples() {
        let c = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert_eq!(c.default_enabled(), enabled);
        assert!(!c.safe_fixes());
        assert!(c.path_policy().is_none());
        assert_eq!(
            c.ruby_versions().minimum,
            if name == FETCH {
                Some(RubyVersion(2, 3))
            } else {
                None
            }
        );
        assert_eq!(c.ruby_versions().maximum, None);
        assert_eq!(DEFAULT_RUBY_VERSION, RubyVersion(3, 4));
        let r = rustocop::pipeline::lint_one(
            Path::new("a.rb"),
            Path::new("a.rb"),
            Some(s.as_bytes()),
            &[c.as_ref()],
            &Default::default(),
        );
        assert!(r.errors.is_empty());
        assert!(r.internal_errors.is_empty());
        assert_eq!(r.result.offenses.len(), 1);
        assert!(r.result.offenses[0].correction.is_none());
        let d = project();
        for args in [vec!["a.rb"], vec!["--fix", "a.rb"]] {
            let o = unchanged(d.path(), s.as_bytes(), &args, i32::from(enabled));
            assert_eq!(String::from_utf8_lossy(&o.stdout).contains(name), enabled);
        }
    }
}
#[test]
fn selectors_config_excludes_and_no_new_knobs() {
    for (name, s, _) in examples() {
        let d = project();
        for fix in [false, true] {
            let mut args = vec!["--only", name, "a.rb"];
            if fix {
                args.push("--fix");
            }
            unchanged(d.path(), s.as_bytes(), &args, 1);
            args.extend(["--except", name]);
            unchanged(d.path(), s.as_bytes(), &args, 0);
        }
        for config in [
            format!("[cops.\"{name}\"]\nenabled = false\n"),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s.as_bytes(), &["--fix", "a.rb"], 0);
            unchanged(
                d.path(),
                s.as_bytes(),
                &["--only", name, "--fix", "a.rb"],
                1,
            );
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{name}\"]\nenabled = true\n"),
        )
        .unwrap();
        unchanged(d.path(), s.as_bytes(), &["--fix", "a.rb"], 1);
        for config in [
            "exclude = [\"a.rb\"]\n".into(),
            format!("[cops.\"{name}\"]\nexclude = [\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                s.as_bytes(),
                &["--only", name, "--fix", "a.rb"],
                0,
            );
        }
        for setting in [
            "EnforcedStyle = \"fetch\"",
            "MinBranchesCount = 2",
            "AllowedReceivers = []",
            "TargetRubyVersion = 2.2",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{name}\"]\n{setting}\n"),
            )
            .unwrap();
            unchanged(d.path(), s.as_bytes(), &["--fix", "a.rb"], 2);
        }
    }
}
#[test]
fn directives_clean_error_and_no_write() {
    for (name, s, _) in examples() {
        let d = project();
        for prefix in ["rubocop", "rustocop"] {
            for target in [name, name.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\n{s}"),
                    format!("# {prefix}:disable-next {target}\n{s}"),
                    format!(
                        "{} # {prefix}:disable {target}\n{}",
                        s.lines().next().unwrap(),
                        s.lines().skip(1).collect::<Vec<_>>().join("\n")
                    ),
                ] {
                    unchanged(
                        d.path(),
                        source.as_bytes(),
                        &["--only", name, "--fix", "a.rb"],
                        0,
                    );
                }
            }
            let source = format!("# {prefix}:disable {name}\n{s}# {prefix}:enable {name}\n{s}");
            let o = unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", name, "--fix", "a.rb"],
                1,
            );
            assert_eq!(
                String::from_utf8_lossy(&o.stdout)
                    .matches(&format!("({name})"))
                    .count(),
                1
            );
        }
        for (source, code) in [("hash.dig(:a,:b)\n", 0), ("(\n", 2)] {
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", name, "--fix", "a.rb"],
                code,
            );
        }
    }
}
#[test]
fn stdin_reports_opt_in_and_never_changes_display_path() {
    use std::process::Stdio;
    for (name, source, _) in examples() {
        let d = project();
        let path = d.path().join("a.rb");
        fs::write(&path, b"disk bytes\n").unwrap();
        let time = fs::metadata(&path).unwrap().modified().unwrap();
        for fix in [false, true] {
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            cmd.current_dir(d.path())
                .args(["--only", name, "--stdin", "a.rb"]);
            if fix {
                cmd.arg("--fix");
            }
            let mut child = cmd
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            stdin::write_stdin(child.stdin.take().unwrap(), source.as_bytes(), fix).unwrap();
            let o = child.wait_with_output().unwrap();
            assert_eq!(o.status.code(), Some(if fix { 2 } else { 1 }), "{o:?}");
            if fix {
                assert!(o.stdout.is_empty());
                assert!(String::from_utf8_lossy(&o.stderr).contains("cannot be used with '--fix'"));
            } else {
                assert!(o.stderr.is_empty());
                assert!(String::from_utf8_lossy(&o.stdout).contains(name));
            }
            assert_eq!(fs::read(&path).unwrap(), b"disk bytes\n");
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
        }
    }
}

#[test]
fn env_dependency_is_configured_not_selected_or_directive_suppressed() {
    let d = project();
    for (config, code) in [
        ("", 0),
        ("[cops.\"Style/FetchEnvVar\"]\nenabled = false\n", 1),
        ("[cops.Style]\nenabled = false\n", 1),
        (
            "[cops.Style]\nenabled = false\n[cops.\"Style/FetchEnvVar\"]\nenabled = true\n",
            0,
        ),
        ("[cops.\"Style/FetchEnvVar\"]\nexclude = [\"a.rb\"]\n", 0),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        for source in [
            "ENV.fetch(key)\n",
            "::ENV.fetch(key)\n",
            "# rustocop:disable Style/FetchEnvVar\nENV.fetch(key)\n",
        ] {
            unchanged(
                d.path(),
                source.as_bytes(),
                &[
                    "--only",
                    LOOKUP,
                    "--except",
                    "Style/FetchEnvVar",
                    "--fix",
                    "a.rb",
                ],
                code,
            );
        }
    }
}
#[test]
fn every_applicable_pinned_unsafe_correction_call_is_a_byte_and_mtime_hold() {
    // RuboCop specs, MIT (c) 2012-26 Bozhidar Batsov. FetchChain's six
    // shared correction calls x four defaults, plus its multiline call = 25.
    let d = project();
    let mut count = 0;
    for default in ["nil", "{}", "Hash.new", "::Hash.new"] {
        for (source, message) in [
            (
                format!("hash.fetch('foo', {default})&.fetch('bar', nil)"),
                "dig('foo', 'bar')",
            ),
            (
                format!("hash.fetch(:foo, {default})&.fetch(:bar, nil)"),
                "dig(:foo, :bar)",
            ),
            (
                format!("hash.fetch(foo, {default})&.fetch(bar, nil)"),
                "dig(foo, bar)",
            ),
            (
                format!("hash.fetch('foo', {default})&.fetch(:bar, {default})&.fetch(baz, nil)"),
                "dig('foo', :bar, baz)",
            ),
            (
                format!("hash&.fetch('foo', {default})&.fetch('bar', nil)"),
                "dig('foo', 'bar')",
            ),
            (
                format!("x.hash.fetch('foo', {default})&.fetch('bar', nil)"),
                "dig('foo', 'bar')",
            ),
        ] {
            let o = unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", FETCH, "--fix", "a.rb"],
                1,
            );
            assert!(
                String::from_utf8_lossy(&o.stdout).contains(&format!("Use `{message}` instead."))
            );
            count += 1;
        }
    }
    unchanged(
        d.path(),
        b"hash\n  .fetch('foo', nil)\n  .fetch('bar', nil)\n",
        &["--only", FETCH, "--fix", "a.rb"],
        1,
    );
    count += 1;
    assert_eq!(count, 25);
    // LookupMethod correction lines 13,24,38,65,106,130,142. The pending
    // reference dependency at 106 projects to enabled=false, like line 130.
    for (source, env_disabled) in [
        ("hash.fetch(key)", false),
        ("some_method.hash.fetch(key)", false),
        ("do_something(\n  hash\n    .fetch(key)\n)", false),
        ("result.fetch(:foo).fetch(:bar)", false),
        ("ENV.fetch(key)", true),
        ("ENV.fetch(key)", true),
        ("a.fetch(b.fetch(:y))", false),
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            if env_disabled {
                "[cops.\"Style/FetchEnvVar\"]\nenabled = false\n"
            } else {
                ""
            },
        )
        .unwrap();
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", LOOKUP, "--fix", "a.rb"],
            1,
        );
    }
}
#[test]
fn source_envelopes_and_original_scalar_columns_never_rewrite() {
    let d = project();
    for (name,source) in [
        (FETCH,b"\xef\xbb\xbf# encoding: UTF-8\r\nhash.fetch(__LINE__, {}).fetch(effect(), nil)\r\np DATA.read\r\n__END__\r\nraw\0\r\n".to_vec()),
        (FETCH,b"# encoding: ISO-8859-1\r\nhash.fetch('caf\xe9', {}).fetch(:b, nil)\r\n".to_vec()),
        (FETCH,b"hash.fetch(<<~KEY, {}).fetch(:b, nil)\n#{effect()}\nKEY\n".to_vec()),
        (LOOKUP,b"hash.fetch(<<~KEY)\n#{other.fetch(:inner)}\nKEY\n".to_vec()),
        (LOOKUP,b"# encoding: ISO-8859-1\r\nhash.fetch('caf\xe9')\r\n".to_vec()),
        (CASE,format!("\u{feff}# encoding: UTF-8\r\n{}p __LINE__\r\np DATA.read\r\n__END__\r\nraw\0\r\n",CASE_SOURCE.replace('\n',"\r\n")).into_bytes()),
    ] {unchanged(d.path(),&source,&["--only",name,"--fix","a.rb"],1);}
    for (name, s, col) in [
        (FETCH, "é = 1; hash.fetch(:a, {}).fetch(:b, nil)\n", 13),
        (LOOKUP, "é = 1; hash.fetch(key)\n", 13),
    ] {
        let o = unchanged(d.path(), s.as_bytes(), &["--only", name, "a.rb"], 1);
        assert!(
            String::from_utf8_lossy(&o.stdout).contains(&format!("a.rb:1:{col}:")),
            "{o:?}"
        );
    }
}
#[test]
fn broad_and_nested_cases_keep_branch_work_bounded() {
    let mut s = String::from("case x\n");
    for i in 0..6000 {
        s.push_str(&format!("when :k{i}\n  [{i}]\n"));
    }
    s.push_str("end\n");
    s.push_str(&CASE_SOURCE.repeat(6000));
    let r = rustocop::cops::Registry::new();
    let c = r.cops().iter().find(|c| c.name() == CASE).unwrap();
    let report = rustocop::pipeline::lint_one(
        Path::new("a.rb"),
        Path::new("a.rb"),
        Some(s.as_bytes()),
        &[c.as_ref()],
        &Default::default(),
    );
    assert!(report.errors.is_empty());
    assert!(report.internal_errors.is_empty());
    assert_eq!(report.result.offenses.len(), 6001);
    assert!(
        report
            .result
            .offenses
            .iter()
            .all(|o| o.correction.is_none())
    );
}
#[test]
fn controlled_runtime_witnesses_evaluation_order_defaults_missing_keys_and_dispatch() {
    let source = r#"
trace = []
h = { a: {} }
key = ->(k) { trace << k; k }
raise unless h.fetch(key.call(:missing), nil)&.fetch(key.call(:b), nil).nil?
raise unless trace == [:missing]
trace.clear
raise unless h.dig(key.call(:missing), key.call(:b)).nil?
raise unless trace == [:missing,:b]
h = Hash.new { |_,k| trace << [:default,k]; {b: :fallback} }
trace.clear
raise unless h.fetch(:missing, {}).fetch(:b,nil).nil?
raise unless trace.empty?
raise unless h.dig(:missing,:b) == :fallback && trace == [[:default,:missing]]
trace.clear
raise unless h[:absent] == {b: :fallback}
begin; h.fetch(:absent); raise 'no KeyError'; rescue KeyError; end
raise unless trace == [[:default,:absent]]
h = {}; h[:b] = 2; h[:a] = 1
raise unless h.keys == [:b,:a]
class Lookalike
  def fetch(*); :original; end
  def [](*); :different; end
end
raise unless Lookalike.new.fetch(:k) != Lookalike.new[:k]
# case evaluates only its chosen body; a literal-looking array hash would
# eagerly allocate every body. These cops never introduce that rewrite.
trace.clear
v = case :a
when :a; trace << :a; [1]
when :b; trace << :b; [2]
when :c; trace << :c; [3]
end
raise unless v == [1] && trace == [:a]
puts :ok
"#;
    let mut rubies = vec!["ruby".to_string()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        let o = Command::new(&ruby)
            .args([
                "-e",
                crate::support::runtime::script("raise unless RUBY_VERSION.start_with?('3.4.')")
                    .as_str(),
            ])
            .output()
            .unwrap();
        assert!(o.status.success(), "{o:?}");
        rubies.push(ruby);
    } else {
        eprintln!("SKIP native Ruby 3.4; set RUSTOCOP_RUBY34");
    }
    for ruby in rubies {
        let o = Command::new(&ruby).args(["-e", source]).output().unwrap();
        assert!(o.status.success(), "{ruby}: {o:?}");
        assert_eq!(o.stdout, b"ok\n");
    }
    let d = project();
    unchanged(
        d.path(),
        source.as_bytes(),
        &[
            "--only",
            "Style/HashFetchChain,Style/HashLikeCase,Style/HashLookupMethod",
            "--fix",
            "a.rb",
        ],
        1,
    );
}
