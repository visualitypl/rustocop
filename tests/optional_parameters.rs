//! pipeline policy, byte preservation and parameter-binding regressions.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const CASES: [(&str, &str, bool); 3] = [
    (
        "Style/OptionalArguments",
        "def f(first = 1, last)\n  nil\nend\n",
        true,
    ),
    (
        "Style/OptionalBooleanParameter",
        "def f(flag = false)\n  nil\nend\n",
        true,
    ),
    (
        "Style/OptionHash",
        "def f(options = {})\n  nil\nend\n",
        false,
    ),
];
const ALL: &str = "Style/OptionalArguments,Style/OptionalBooleanParameter,Style/OptionHash";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn cli(dir: &Path, args: &[&str], status: i32) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(status), "{args:?}: {output:?}");
    output
}
fn unchanged(dir: &Path, cop: &str, bytes: &[u8], status: i32) {
    let path = dir.join("x.rb");
    fs::write(&path, bytes).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    for _ in 0..2 {
        cli(dir, &["--only", cop, "--fix", "x.rb"], status);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn defaults_opt_in_selectors_and_config_never_rewrite_signatures() {
    let dir = project();
    // Isolate this historical Style toggle test from the newly landed Lint cop.
    let baseline = "[cops.\"Lint/UnusedMethodArgument\"]\nenabled = false\n";
    fs::write(dir.path().join("rustocop.toml"), baseline).unwrap();
    for (cop, source, enabled) in CASES {
        fs::write(dir.path().join("x.rb"), source).unwrap();
        let output = cli(dir.path(), &["x.rb"], i32::from(enabled));
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).contains(cop),
            enabled
        );
        cli(dir.path(), &["--only", cop, "x.rb"], 1);
        cli(
            dir.path(),
            &["--only", cop, "--except", cop, "--fix", "x.rb"],
            0,
        );
        assert_eq!(fs::read_to_string(dir.path().join("x.rb")).unwrap(), source);
        unchanged(dir.path(), cop, source.as_bytes(), 1);
        for config in [
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("{baseline}{config}"),
            )
            .unwrap();
            let output = cli(dir.path(), &["x.rb"], 0);
            assert!(!String::from_utf8_lossy(&output.stdout).contains(cop));
            unchanged(dir.path(), cop, source.as_bytes(), 1);
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("{baseline}[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&cli(dir.path(), &["x.rb"], 1).stdout).contains(cop));
        cli(dir.path(), &["--except", cop, "--fix", "x.rb"], 0);
        assert_eq!(fs::read_to_string(dir.path().join("x.rb")).unwrap(), source);
        fs::write(dir.path().join("rustocop.toml"), baseline).unwrap();
    }
}
#[test]
fn directives_and_source_opt_in_respect_scopes_and_excepts() {
    let dir = project();
    for (cop, source, _) in CASES {
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                unchanged(
                    dir.path(),
                    cop,
                    format!("# {prefix}:disable {target}\n{source}").as_bytes(),
                    0,
                );
                unchanged(
                    dir.path(),
                    cop,
                    format!("# {prefix}:disable-next {target}\n{source}").as_bytes(),
                    0,
                );
            }
            // A directive on the last line of the parameter highlight suppresses it.
            let params = match cop {
                "Style/OptionalArguments" => "a = begin\n    1\n  end",
                "Style/OptionalBooleanParameter" => "flag =\n    false",
                _ => "options = {\n  }",
            };
            let (comma, tail) = if cop == "Style/OptionalArguments" {
                (",", "last")
            } else {
                ("", "")
            };
            unchanged(
                dir.path(),
                cop,
                format!("def f(\n  {params}{comma} # {prefix}:disable {cop}\n  {tail}\n)\nend\n")
                    .as_bytes(),
                0,
            );
        }
    }
    let cop = "Style/OptionHash";
    let source = format!("# rustocop:enable-next {cop}\n{}{}", CASES[2].1, CASES[2].1);
    fs::write(dir.path().join("x.rb"), &source).unwrap();
    let stdout = String::from_utf8_lossy(&cli(dir.path(), &["x.rb"], 1).stdout).into_owned();
    assert_eq!(stdout.matches("(Style/OptionHash)").count(), 1, "{stdout}");
    let stdout = String::from_utf8_lossy(&cli(dir.path(), &["--except", cop, "x.rb"], 1).stdout)
        .into_owned();
    assert!(!stdout.contains(cop));
}
#[test]
fn config_relative_excludes_and_stdin_do_not_write() {
    let dir = project();
    let nested = dir.path().join("nested");
    fs::create_dir(&nested).unwrap();
    for (cop, source, _) in CASES {
        for config in [
            "exclude = [\"nested/x.rb\"]\n".to_owned(),
            format!("[cops.\"{cop}\"]\nexclude = [\"nested/x.rb\"]\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            unchanged(&nested, cop, source.as_bytes(), 0);
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", cop, "--stdin", "nested/x.rb"])
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
            let output = child.wait_with_output().unwrap();
            assert_eq!(output.status.code(), Some(0), "{output:?}");
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", cop, "--stdin", "nested/x.rb"])
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
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains(cop));
        cli(
            dir.path(),
            &["--only", cop, "--fix", "--stdin", "nested/x.rb"],
            2,
        );
        assert_eq!(fs::read_to_string(nested.join("x.rb")).unwrap(), source);
    }
}
#[test]
fn bom_crlf_non_utf8_line_data_and_error_files_retain_bytes_and_mtime() {
    let dir = project();
    for (cop, source, _) in CASES {
        for header in [
            b"\xef\xbb\xbf# encoding: UTF-8\r\n".as_slice(),
            b"# encoding: ISO-8859-1\r\n# caf\xe9\r\n",
        ] {
            let bytes = [
                header,
                source.replace('\n', "\r\n").as_bytes(),
                b"p __LINE__\r\n__END__\r\nraw\xff\r\n",
            ]
            .concat();
            unchanged(dir.path(), cop, &bytes, 1);
        }
        unchanged(dir.path(), cop, b"def f(\n", 2);
        unchanged(dir.path(), cop, b"def f(value:, **kw, &block)\nend\n", 0);
        let deep = format!("{}x{}\n{source}", "[".repeat(70), "]".repeat(70));
        unchanged(dir.path(), cop, deep.as_bytes(), 2);
    }
}
#[test]
fn runtime_defaults_post_destructuring_super_and_ruby3_argument_channels_are_preserved() {
    if !Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("Ruby unavailable: controlled runtime test skipped");
        return;
    }
    let dir = project();
    let path = dir.path().join("x.rb");
    let source = r#"$events = []
class Parent
  def f(*args, **kwargs, &block)
    [args, kwargs.to_a, block && block.call]
  end
  def hash_super(options = {})
    options
  end
end
class Child < Parent
  def f(first = ($events << :first; 10), flag = false, *rest, last, mode: ($events << :mode; 20), **kw, &blk)
    super
  end
  def hash_super(options = {})
    super
  end
  def hash_only(options = {})
    options
  end
  def destructured(value = ($events << :destructured; 30), (a, b))
    [value, a, b]
  end
  def respond_to_missing?(name, include_all = false)
    false
  end
end
c = Child.new
p c.f(:last, extra: 3) { :block }
p c.f(1, true, 2, 3, :last, mode: 9, extra: 4)
p c.hash_super({position: 1})
p c.hash_only(keyword: 2)
p c.destructured([4, 5])
p $events
p [->(flag=false) { flag }.call, proc { |options={}| options }.call({a: 1})]
p __LINE__
"#;
    fs::write(&path, source).unwrap();
    let run = || Command::new("ruby").arg("-W0").arg(&path).output().unwrap();
    let before = run();
    assert!(before.status.success(), "{before:?}");
    let stdout = String::from_utf8_lossy(&before.stdout);
    assert!(
        stdout.contains("[[10, false, :last], [[:extra, 3], [:mode, 20]], :block]"),
        "{stdout}"
    );
    assert!(
        stdout.contains("[[1, true, 2, 3, :last], [[:extra, 4], [:mode, 9]], nil]"),
        "{stdout}"
    );
    assert!(
        stdout.contains("[30, 4, 5]\n[:first, :mode, :destructured]"),
        "{stdout}"
    );
    assert!(stdout.contains("{:position=>1}\n{:keyword=>2}"), "{stdout}");
    unchanged(dir.path(), ALL, source.as_bytes(), 1);
    let after = run();
    assert_eq!(before.status.code(), after.status.code());
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
}
#[test]
fn existing_style_layout_fixes_compose_without_signature_rewrites() {
    let dir = project();
    let path = dir.path().join("x.rb");
    fs::write(&path, "def f(a = 1, b)\n  return 'ok'\nend\ndef g(flag = false)\n  flag\nend\ndef h(options = {})\n  options\nend\n").unwrap();
    let selection = format!(
        "{ALL},Style/StringLiterals,Style/RedundantReturn,Style/HashSyntax,Style/RedundantParentheses,Style/SignalException,Style/Semicolon,Layout"
    );
    cli(dir.path(), &["--only", &selection, "--fix", "x.rb"], 1);
    let fixed = fs::read_to_string(&path).unwrap();
    assert!(fixed.contains("def f(a = 1, b)\n  \"ok\"\nend"), "{fixed}");
    assert!(fixed.contains("def g(flag = false)"));
    assert!(fixed.contains("def h(options = {})"));
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir.path(), &["--only", &selection, "--fix", "x.rb"], 1);
    assert_eq!(fs::read_to_string(&path).unwrap(), fixed);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    // The default Style/Layout pipeline must also retain both unsafe reports.
    let out = cli(dir.path(), &["--fix", "x.rb"], 1);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains(CASES[0].0) && text.contains(CASES[1].0),
        "{text}"
    );
    assert!(!text.contains(CASES[2].0));
    let fixed = fs::read_to_string(&path).unwrap();
    assert!(
        fixed.contains("(a = 1, b)")
            && fixed.contains("(flag = false)")
            && fixed.contains("(options = {})"),
        "{fixed}"
    );
}
