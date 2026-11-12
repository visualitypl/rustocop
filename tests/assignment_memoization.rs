//! real CLI policy, pinned corrections and controlled Ruby witnesses.
//! RuboCop spec fixtures: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
const MEMO: &str = "Style/MultilineMemoization";
const OR: &str = "Style/OrAssignment";
const BOTH: &str = "Style/MultilineMemoization,Style/OrAssignment";
const CASES: [(&str, &str, &str); 2] = [
    (
        MEMO,
        "foo ||= (\n  work\n)\n",
        "foo ||= begin\n  work\nend\n",
    ),
    (OR, "foo = foo ? foo : 'default'\n", "foo ||= 'default'\n"),
];
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn unchanged(root: &Path, cop: &str, source: &[u8], args: &[&str], count: usize) {
    let path = root.join("a.rb");
    fs::write(&path, source).unwrap();
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    let o = cli(root, args);
    assert_ne!(o.status.code(), Some(2), "{o:?}");
    assert_eq!(
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| l.contains(cop))
            .count(),
        count,
        "{o:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), source, "{o:?}");
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), time);
}
fn fix(root: &Path, cop: &str, before: &str, after: &str) {
    fs::write(root.join("a.rb"), before).unwrap();
    let o = cli(root, &["--only", cop, "--fix", "a.rb"]);
    assert_ne!(o.status.code(), Some(2), "{o:?}");
    assert_eq!(
        fs::read_to_string(root.join("a.rb")).unwrap(),
        after,
        "{o:?}"
    );
}
#[test]
fn default_enablement_selectors_and_config_never_implicitly_write() {
    let d = project();
    let registry = rustocop::cops::Registry::new();
    for (cop, before, _) in CASES {
        let rule = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(rule.default_enabled() && rule.safe_fixes());
        assert_eq!(rule.ruby_versions(), Default::default());
        assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
        for (config, selectors, count) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Style", "--except", cop], 0),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
            (format!("[cops.\"{cop}\"]\nenabled = false\n"), vec![], 0),
            (
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
                vec!["--only", cop],
                1,
            ),
            (
                format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
                vec![],
                1,
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            let mut args = selectors;
            args.push("a.rb");
            unchanged(d.path(), cop, before.as_bytes(), &args, count);
        }
    }
}
#[test]
fn excludes_and_both_directive_prefixes_protect_atomic_corrections() {
    let d = project();
    for (cop, before, after) in CASES {
        for config in [
            "exclude = [\"a.rb\"]\n".to_owned(),
            format!("[cops.\"{cop}\"]\nexclude = [\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                cop,
                before.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                let source = format!("# {prefix}:disable {target}\n{before}");
                unchanged(
                    d.path(),
                    cop,
                    source.as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    0,
                );
            }
            let source =
                format!("# {prefix}:disable {cop}\n{before}# {prefix}:enable {cop}\n{before}");
            let corrected =
                format!("# {prefix}:disable {cop}\n{before}# {prefix}:enable {cop}\n{after}");
            fix(d.path(), cop, &source, &corrected);
        }
    }
    let source = "foo ||= (\n  work # rubocop:disable Lint/Debugger\n)\n";
    unchanged(
        d.path(),
        MEMO,
        source.as_bytes(),
        &["--only", MEMO, "--fix", "a.rb"],
        1,
    );
}
#[test]
fn all_twenty_pinned_correction_calls_are_canonical_or_individually_withheld() {
    let d = project();
    let mut canonical = 0;
    let mut held = Vec::new();
    for (cop, fixture) in [
        (
            MEMO,
            include_str!("fixtures/multiline_memoization_pinned.json"),
        ),
        (OR, include_str!("fixtures/or_assignment_pinned.json")),
    ] {
        let cases: Vec<serde_json::Value> = serde_json::from_str(fixture).unwrap();
        for c in cases {
            let Some(after) = c["correction"].as_str() else {
                continue;
            };
            let before = c["source"].as_str().unwrap();
            let line = c["spec_line"].as_u64().unwrap();
            // Four class-variable reads can raise instead of initialize; two
            // global self-writes can invoke hooks, two instance self-writes can
            // raise on frozen receivers instead of silently skipping a write.
            if cop == OR
                && (before.contains("@@foo")
                    || ((before.starts_with("$foo = ") || before.starts_with("@foo = "))
                        && !before.contains("unless")))
            {
                held.push(line);
                unchanged(
                    d.path(),
                    cop,
                    before.as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    1,
                );
            } else {
                fix(d.path(), cop, before, after);
                unchanged(
                    d.path(),
                    cop,
                    after.as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    0,
                );
                canonical += 1;
            }
        }
    }
    assert_eq!(canonical, 12);
    assert_eq!(held, [17, 28, 39, 72, 87, 102, 158, 221]);
}
#[test]
fn bom_crlf_encoding_line_values_and_unread_data_bytes_are_preserved() {
    let d = project();
    for (cop, before, after) in CASES {
        let before = format!(
            "\u{feff}# coding: UTF-8\r\n{}__END__\r\npayload é\r\n",
            before.replace('\n', "\r\n")
        );
        let after = format!(
            "\u{feff}# coding: UTF-8\r\n{}__END__\r\npayload é\r\n",
            after.replace('\n', "\r\n")
        );
        fix(d.path(), cop, &before, &after);
        unchanged(
            d.path(),
            cop,
            after.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            0,
        );
    }
    fix(
        d.path(),
        MEMO,
        "x ||= (\n __LINE__\n)\np __LINE__\n",
        "x ||= begin\n __LINE__\nend\np __LINE__\n",
    );
    fix(
        d.path(),
        OR,
        "x = x ? x : __LINE__\np __LINE__\n",
        "x ||= __LINE__\np __LINE__\n",
    );
    for (cop, source) in [
        (
            MEMO,
            b"# coding: ISO-8859-1\nx ||= (\n '\xe9'\n)\n".as_slice(),
        ),
        (OR, b"# coding: ISO-8859-1\nx = x ? x : '\xe9'\n".as_slice()),
        (MEMO, b"x ||= (work ||\n other)\np __LINE__\n".as_slice()),
        (OR, b"x = if x\n x\nelse\n 3\nend\np __LINE__\n".as_slice()),
        (
            MEMO,
            b"x ||= (\n DATA.pos\n)\n__END__\npayload\n".as_slice(),
        ),
        (OR, b"x = x ? x : DATA.pos\n__END__\npayload\n".as_slice()),
    ] {
        unchanged(d.path(), cop, source, &["--only", cop, "--fix", "a.rb"], 1);
    }
}
#[test]
fn clean_uncorrectable_and_parse_error_files_keep_bytes_and_mtime() {
    let d = project();
    for (cop, source, count) in [
        (OR, "result = (@x = 3 unless @x)\n", 1),
        (OR, "@@x = @@x ? @@x : 3\n", 1),
        (OR, "$x = $x ? $x : 3\n", 1),
        (MEMO, "x ||= (\n <<~TEXT\n value\nTEXT\n)\n", 1),
    ] {
        unchanged(
            d.path(),
            cop,
            source.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            count,
        );
    }
    for (cop, _, after) in CASES {
        unchanged(
            d.path(),
            cop,
            after.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            0,
        );
    }
    fs::write(d.path().join("a.rb"), b"x ||= (\n").unwrap();
    let time = fs::metadata(d.path().join("a.rb"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        cli(d.path(), &["--only", BOTH, "--fix", "a.rb"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), b"x ||= (\n");
    assert_eq!(
        fs::metadata(d.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap(),
        time
    );
    for cop in [OR, MEMO] {
        for setting in [
            "EnforcedStyle = 'braces'",
            "max = 80",
            "TargetRubyVersion = 3.4",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{setting}\n"),
            )
            .unwrap();
            assert_eq!(
                cli(d.path(), &["--only", cop, "a.rb"]).status.code(),
                Some(2)
            );
        }
    }
}
#[test]
fn stdin_obeys_selectors_excludes_and_reports_exact_positions() {
    let d = project();
    for (cop, source, _) in CASES {
        for (config, count) in [
            (String::new(), 1),
            (format!("[cops.\"{cop}\"]\nexclude = ['a.rb']\n"), 0),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(["--only", cop, "--stdin", "a.rb"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(source.as_bytes())
                .unwrap();
            let o = child.wait_with_output().unwrap();
            assert_eq!(o.status.code(), Some(count), "{o:?}");
            let text = String::from_utf8_lossy(&o.stdout);
            assert_eq!(text.contains(cop), count == 1, "{text}");
            if count == 1 {
                assert!(text.contains("a.rb:1:1:"), "{text}");
            }
            assert!(!d.path().join("a.rb").exists());
        }
    }
}
#[test]
fn controlled_truthiness_evaluation_order_binding_setter_and_flow_witnesses() {
    let source = r#"$trace = []
def default
  $trace << :default
  :value
end
def choose(x)
  x = x ? x : default
end
def conditional(x)
  x = :value unless x
end
def discarded(x)
  x = default unless x
  x
end
def low_precedence(x)
  x = if x; x; else; false or :value; end
end
p [nil, false, true].map { |x| [choose(x), conditional(x), discarded(x), low_precedence(x)] }
x = x ? x : -> { x }
p x.call.equal?(x)
x = nil
f = -> { x = x ? x : :captured }
p [f.call, x]
p [1].map { |x| x = x ? x : :wrong }
p [1].map { |; x| x = x ? x : :block_local }
class Cache
  def value
    $trace << :get
    @value
  end
  def value=(x)
    $trace << [:set, x]
    @value = x
    :setter_return_is_not_assignment_result
  end
end
cache = Cache.new
2.times { p(cache.value ||= (
  $trace << :rhs
  :cached
)) }
left, right = :left, (memo = memo ? memo : default)
p [left, right]
$watched = true
trace_var(:$watched) { |v| $trace << [:watch, v] }
$watched = $watched ? $watched : :wrong
untrace_var(:$watched)
class FrozenInstance
  def initialize
    @value = :already_set
    freeze
  end
  def read
    @value = @value ? @value : :wrong
  end
end
begin
  FrozenInstance.new.read
rescue FrozenError
  $trace << :frozen_instance
end
class UndefinedClassVariable
  def self.read
    @@unset = @@unset ? @@unset : :wrong
  end
end
begin
  UndefinedClassVariable.read
rescue NameError
  $trace << :undefined_class_variable
end
def flow(flag)
  @flow = nil
  @flow ||= (
    return :early if flag
    raise 'failure'
  )
rescue RuntimeError
  :rescued
ensure
  $trace << :ensure
end
p [flow(true), flow(false)]
p 2.times.map { |i| @skip = false; @skip ||= (
  next :skip if i == 0
  break :stop
  :unreachable
) }
p $trace
"#;
    let d = project();
    fs::write(d.path().join("before.rb"), source).unwrap();
    fs::write(d.path().join("a.rb"), source).unwrap();
    let o = cli(d.path(), &["--only", BOTH, "--fix", "a.rb"]);
    assert_ne!(o.status.code(), Some(2), "{o:?}");
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert_ne!(after, source);
    assert!(after.contains("x ||= (-> { x })") && after.contains("cache.value ||= begin"));
    assert!(
        after.contains("x = :value unless x")
            && after.contains("@@unset = @@unset")
            && after.contains("$watched = $watched")
    );
    let mut rubies = vec!["ruby".to_owned()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        rubies.push(ruby);
    }
    for ruby in rubies {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        let before = Command::new(&ruby)
            .arg(d.path().join("before.rb"))
            .output()
            .unwrap();
        let after = Command::new(&ruby)
            .arg(d.path().join("a.rb"))
            .output()
            .unwrap();
        assert!(
            before.status.success() && after.status.success(),
            "{ruby}: {before:?} {after:?}"
        );
        assert_eq!(before.stdout, after.stdout, "{ruby}");
        assert_eq!(before.stderr, after.stderr, "{ruby}");
        let observed = String::from_utf8_lossy(&after.stdout);
        assert!(
            observed.contains("[:early, :rescued]")
                && observed.contains(":frozen_instance")
                && observed.contains(":undefined_class_variable")
                && observed.contains("[:watch, true]")
                && observed.contains(":stop"),
            "{observed}"
        );
    }
}
#[test]
fn mixed_assignment_and_existing_style_layout_fixes_stabilize_within_six_passes() {
    let d = project();
    let before = "def f\n  foo = if foo\n    foo\n  else\n    ({ :key => 'value' });\n  end\nend\n";
    let selected = format!(
        "{BOTH},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout/EndAlignment,Layout/IndentationWidth"
    );
    fs::write(d.path().join("a.rb"), before).unwrap();
    let mut stable = false;
    for _ in 0..6 {
        let old = fs::read(d.path().join("a.rb")).unwrap();
        let o = cli(d.path(), &["--only", &selected, "--fix", "a.rb"]);
        assert_ne!(o.status.code(), Some(2), "{o:?}");
        let new = fs::read(d.path().join("a.rb")).unwrap();
        if old == new {
            stable = true;
            break;
        }
    }
    assert!(stable);
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert!(
        after.contains("foo ||=") && after.contains("key: \"value\""),
        "{after}"
    );
}
