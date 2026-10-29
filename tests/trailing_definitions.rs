//! real CLI controls, bytes, runtime observations and composition.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const ONLY: &str = "Style/TrailingMethodEndStatement,Style/TrailingBodyOnClass,Style/TrailingBodyOnModule,Style/TrailingBodyOnMethodDefinition";
const CASES: [(&str, &str, &str); 4] = [
    (
        "Style/TrailingBodyOnMethodDefinition",
        "def f; 1\nend\n",
        "def f \n  1\nend\n",
    ),
    (
        "Style/TrailingMethodEndStatement",
        "def f\n  1 end\n",
        "def f\n  1 \nend\n",
    ),
    (
        "Style/TrailingBodyOnClass",
        "class C; 1\nend\n",
        "class C \n  1\nend\n",
    ),
    (
        "Style/TrailingBodyOnModule",
        "module M; 1\nend\n",
        "module M \n  1\nend\n",
    ),
];
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn cli(dir: &Path, args: &[&str], status: i32) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    out
}
#[test]
fn defaults_selectors_config_and_no_write_contract() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let registry = rustocop::cops::Registry::new();
    for (cop, before, after) in CASES {
        let registered: Vec<_> = registry.cops().iter().filter(|c| c.name() == cop).collect();
        assert_eq!(registered.len(), 1);
        assert!(registered[0].default_enabled());
        assert!(registered[0].safe_fixes());
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        fs::write(&path, before).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let out = cli(dir.path(), &["lib/a.rb"], 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
        cli(dir.path(), &["--only", cop, "lib/a.rb"], 1);
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        for config in [
            format!(
                "[cops.\"{cop}\"]\nenabled = false\n[cops.\"Style/Documentation\"]\nenabled = false\n[cops.Layout]\nenabled = false\n"
            ),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            cli(dir.path(), &["--fix", "lib/a.rb"], 0);
            cli(dir.path(), &["--only", cop, "lib/a.rb"], 1);
            cli(
                dir.path(),
                &["--only", cop, "--except", cop, "--fix", "lib/a.rb"],
                0,
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), before);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
        fs::write(dir.path().join("rustocop.toml"), format!("[cops.Style]\nenabled = false\n[cops.Layout]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n")).unwrap();
        cli(dir.path(), &["--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn exclusions_directives_and_cross_highlight_atomicity() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, before, _) in CASES {
        for config in [
            "exclude = [\"lib/**\"]\n".to_owned(),
            format!("[cops.\"{cop}\"]\nexclude = [\"lib/**\"]\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            fs::write(&path, before).unwrap();
            cli(
                &dir.path().join("lib"),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), before);
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rubocop", "rustocop"] {
            for target in [cop, "Style", "all"] {
                for directive in ["disable", "disable-next"] {
                    let source = format!("# {prefix}:{directive} {target}\n{before}");
                    fs::write(&path, &source).unwrap();
                    let modified = fs::metadata(&path).unwrap().modified().unwrap();
                    cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
                    assert_eq!(fs::read_to_string(&path).unwrap(), source);
                    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
                }
            }
        }
    }
    // A multiline first expression also honors disabled body lines.
    let source = "module M; begin\n  foo # rubocop:disable Style/TrailingBodyOnModule\nend\nend\n";
    fs::write(&path, source).unwrap();
    cli(dir.path(), &["--only", ONLY, "--fix", "lib/a.rb"], 0);
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
}
#[test]
fn parenthesized_namespace_directives_use_only_the_inner_highlight() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, header) in [
        ("Style/TrailingBodyOnClass", "class C"),
        ("Style/TrailingBodyOnClass", "class << self"),
        ("Style/TrailingBodyOnModule", "module M"),
        ("Style/TrailingBodyOnMethodDefinition", "def f"),
    ] {
        for prefix in ["rubocop", "rustocop"] {
            for (body, status, correctable) in [
                (format!("( # {prefix}:disable {cop}\n1\n)"), 1, false),
                (format!("(\n1\n) # {prefix}:disable {cop}"), 1, true),
                (format!("(\n1 # {prefix}:disable {cop}\n)"), 0, false),
            ] {
                let source = format!("{header};{body}\nend\n");
                fs::write(&path, &source).unwrap();
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                let out = cli(dir.path(), &["--only", cop, "lib/a.rb"], status);
                if status == 1 {
                    assert!(String::from_utf8_lossy(&out.stdout).contains("lib/a.rb:2:1:"));
                }
                cli(
                    dir.path(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    if correctable { 0 } else { status },
                );
                if correctable {
                    assert_eq!(
                        fs::read_to_string(&path).unwrap(),
                        format!("{header}\n  {body}\nend\n")
                    );
                } else {
                    // Disabled offenses and directive-adjacent edits never write.
                    assert_eq!(fs::read_to_string(&path).unwrap(), source);
                    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
                }
            }
        }
    }
}
#[test]
fn stdin_reports_without_writing_display_path() {
    use std::{io::Write, process::Stdio};
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, source, _) in CASES {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", cop, "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn clean_error_and_individually_uncertain_files_preserve_bytes_and_mtime() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, status) in [
        ("def f = 1\nclass C; 1; end\nmodule M; 1; end\n", 0),
        ("def f(\n", 2),
        (
            "def f\n 1 end\nclass C; 2\nend\nmodule M; 3\nend\np __LINE__\n",
            1,
        ),
        ("def f\n <<~TXT; end\npayload\nTXT\n", 1),
        ("class C; <<~TXT\npayload\nTXT\nend\n", 1),
        ("module M; <<~TXT\npayload\nTXT\nend\n", 1),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", ONLY, "--fix", "lib/a.rb"], status);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
    let source = "def f\n 1 end\np __LINE__\nmodule M; 3\nend\n";
    fs::write(&path, source).unwrap();
    cli(dir.path(), &["--only", ONLY, "--fix", "lib/a.rb"], 1);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        source.replace("module M; 3", "module M \n  3")
    );
}
#[test]
fn bom_crlf_encoding_literal_and_raw_data_bytes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for source in [
        "\u{feff}# encoding: UTF-8\r\ndef f\r\n  \"café\" end\r\nclass C; \"a\r\nb\"\r\nend\r\nmodule M; 3 # docs\r\nend\r\n__END__\r\nraw; end\r\n".as_bytes(),
        b"# encoding: ISO-8859-1\r\ndef f\r\n  \"caf\xe9\" end\r\nclass C; \"a\r\nb\"\r\nend\r\nmodule M; 3 # docs\r\nend\r\n__END__\r\nraw\xff; end\r\n",
    ] {
        fs::write(&path, source).unwrap();
        let mut expected = source.to_vec();
        for (from, to) in [(b" end\r\nclass".as_slice(), b" \r\nend\r\nclass".as_slice()), (b"class C; ", b"class C \r\n  "), (b"module M; 3 # docs", b"# docs\r\nmodule M \r\n  3 ")] {
            let start = expected.windows(from.len()).position(|w| w == from).unwrap();
            expected.splice(start..start + from.len(), to.iter().copied());
        }
        cli(dir.path(), &["--only", ONLY, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read(&path).unwrap(), expected);
    }
    for source in [
        b"# encoding: ISO-8859-1\r\nclass C; 1 # caf\xe9\r\nend\r\n".as_slice(),
        b"# encoding: ISO-8859-1\r\nmodule M; 1 # caf\xe9\r\nend\r\n",
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", ONLY, "--fix", "lib/a.rb"], 1);
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
fn ruby(dir: &Path, harness: Option<&str>) -> Output {
    let mut command = Command::new("ruby");
    command.current_dir(dir);
    if let Some(harness) = harness {
        command.args(["-e", harness]);
    }
    command.arg("lib/a.rb").output().unwrap()
}
#[test]
fn controlled_ruby_binding_evaluation_visibility_returns_control_flow_and_literals() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = r#"$events = []
p __LINE__
def base; $events << :base; Object; end
def receiver; $events << :receiver; $object; end
$object = Object.new
class C < (base); $events << :class
  private def text(value = 2, key: 3)
    return [value + key, "a
b"] end
  def value=(value)
    @value = value end
  def [](key)
    [$events << key, @value] end
  def +(value)
    value + 1 end
  def flow
    yield :yielded
    raise "failure"
  rescue
    :rescued
  ensure
    $events << :ensured end
end
module M; extend self
  def number
    7 end
end
class << (receiver); def value
  42 end
end
"#;
    let harness = "load ARGV[0]; p [C.new.send(:text), C.private_instance_methods(false), M.number, $object.value]; object = C.new; object.value = 9; p [object[:key], object + 3, object.flow { |x| $events << x }, $events]";
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path(), Some(harness));
    assert!(before.status.success(), "{before:?}");
    cli(dir.path(), &["--only", ONLY, "--fix", "lib/a.rb"], 0);
    let fixed = fs::read_to_string(&path).unwrap();
    assert_ne!(fixed, source);
    assert!(fixed.contains("class C < (base) \n  $events"));
    assert!(fixed.contains("b\"] \n          end"));
    let after = ruby(dir.path(), Some(harness));
    assert!(after.status.success(), "{after:?}");
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir.path(), &["--only", ONLY, "--fix", "lib/a.rb"], 0);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
#[test]
fn controlled_ruby_withheld_line_data_and_deferred_heredoc_observations() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, harness) in [
        (
            "def f\n 1 end\nclass C; 2\nend\nmodule M; 3\nend\np __LINE__\n",
            None,
        ),
        (
            "def f\n 1 end\nclass C; 2\nend\nmodule M; 3\nend\np [DATA.pos, DATA.lineno, DATA.read]\n__END__\nraw\n",
            None,
        ),
        (
            "def f\n <<~TXT; end\n  payload\nTXT\n",
            Some("load ARGV[0]; p f"),
        ),
        (
            "class C; TEXT = <<~TXT\n  payload\nTXT\nend\n",
            Some("load ARGV[0]; p C::TEXT"),
        ),
        (
            "module M; TEXT = <<~TXT\n  payload\nTXT\nend\n",
            Some("load ARGV[0]; p M::TEXT"),
        ),
    ] {
        fs::write(&path, source).unwrap();
        let before = ruby(dir.path(), harness);
        assert!(before.status.success(), "{before:?}");
        cli(dir.path(), &["--only", ONLY, "--fix", "lib/a.rb"], 1);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        let after = ruby(dir.path(), harness);
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
    }
}
#[test]
fn controlled_ruby_declaration_results_inline_line_values_and_interpolated_heredocs() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, harness) in [
        (
            "class C; [1, 2]\nend\n",
            "p eval(File.read(ARGV[0]), TOPLEVEL_BINDING)",
        ),
        (
            "module M; [1, 2]\nend\n",
            "p eval(File.read(ARGV[0]), TOPLEVEL_BINDING)",
        ),
        (
            "class C;($events = [:once]; [1, 2])\nend\n",
            "p eval(File.read(ARGV[0]), TOPLEVEL_BINDING); p $events",
        ),
        (
            "module M;($events = [:once]; [1, 2])\nend\n",
            "p eval(File.read(ARGV[0]), TOPLEVEL_BINDING); p $events",
        ),
        (
            "class << self;($events = [:once]; [1, 2])\nend\n",
            "p eval(File.read(ARGV[0]), TOPLEVEL_BINDING); p $events",
        ),
        ("def f\n [__LINE__, \"a\\nb\"] end\n", "load ARGV[0]; p f"),
        (
            "text = <<~TEXT\n  #{def f\n    1 end} tail\n    content\nTEXT\np text\n",
            "load ARGV[0]; p f",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let before = ruby(dir.path(), Some(harness));
        assert!(before.status.success(), "{before:?}");
        cli(dir.path(), &["--only", ONLY, "--fix", "lib/a.rb"], 0);
        assert_ne!(fs::read_to_string(&path).unwrap(), source);
        let after = ruby(dir.path(), Some(harness));
        assert!(after.status.success(), "{after:?}");
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
    }
}
#[test]
fn trailing_method_bodies_preserve_runtime_binding_defaults_visibility_and_control_flow() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = r#"$events = []
p __LINE__
$object = Object.new
def receiver; $events << :receiver; $object; end
class C
  private def text(x = ($events << :default; 2), *rest, k: 3, **kw, &block); $events << :body # docs
    [x, rest, k, kw, block.call, "a
b", /a
b/.source, %w[a
b]]
  end
  def next_method; :public; end
  def value=(value); @value = value
  end
  def [](key); [$events << key, @value]
  end
  def +(other); other + 1
  end
  def flow; yield :yielded
    raise "failure"
  rescue
    :rescued
  ensure
    $events << :ensured
  end
  def grouped;(
    $events << :grouped; [1, 2]
  )
  end
end
def (receiver).value; $events << :singleton
  42
end
"#;
    let harness = "load ARGV[0]; o = C.new; p [o.send(:text) { :block }, C.private_instance_methods(false).sort, o.next_method]; o.value = 9; p [o[:key], o + 3, o.flow { |x| $events << x }, o.grouped, $object.value, $events]";
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path(), Some(harness));
    assert!(before.status.success(), "{before:?}");
    cli(
        dir.path(),
        &[
            "--only",
            "Style/TrailingBodyOnMethodDefinition",
            "--fix",
            "lib/a.rb",
        ],
        0,
    );
    let fixed = fs::read_to_string(&path).unwrap();
    assert_ne!(fixed, source);
    assert!(fixed.contains("# docs\n  private def text"));
    assert!(fixed.contains("($events << :default; 2)"));
    assert!(fixed.contains("def grouped\n    (\n"));
    let after = ruby(dir.path(), Some(harness));
    assert!(after.status.success(), "{after:?}");
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(
        dir.path(),
        &[
            "--only",
            "Style/TrailingBodyOnMethodDefinition",
            "--fix",
            "lib/a.rb",
        ],
        0,
    );
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
#[test]
fn trailing_method_body_declaration_results_preceding_expressions_and_all_visibility_wrappers() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let mut cases = vec![(
        "$events = []\ndef foo; $events << :foo; end\nfoo;def f; $events << :body\n  7\nend\n"
            .to_owned(),
        "p eval(File.read(ARGV[0]), TOPLEVEL_BINDING); p [f, $events]".to_owned(),
    )];
    for visibility in ["private", "protected", "public"] {
        cases.push((format!("class C\n  {visibility} def f; 1 # docs\n  end\n  def g; :public; end\nend\n"),
            "load ARGV[0]; p [C.private_instance_methods(false).sort, C.protected_instance_methods(false).sort, C.public_instance_methods(false).sort, C.new.send(:f), C.new.g]".to_owned()));
    }
    for (source, harness) in cases {
        fs::write(&path, &source).unwrap();
        let before = ruby(dir.path(), Some(&harness));
        assert!(before.status.success(), "{before:?}");
        cli(
            dir.path(),
            &[
                "--only",
                "Style/TrailingBodyOnMethodDefinition",
                "--fix",
                "lib/a.rb",
            ],
            0,
        );
        assert_ne!(fs::read_to_string(&path).unwrap(), source);
        let after = ruby(dir.path(), Some(&harness));
        assert!(after.status.success(), "{after:?}");
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
    }
}
#[test]
fn trailing_method_body_withholding_preserves_line_data_and_heredoc_observations() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, harness) in [
        ("def f; __LINE__\nend\np f\n", None),
        ("def f; 1\nend\np __LINE__\n", None),
        (
            "def f; 1\nend\np [DATA.pos, DATA.lineno, DATA.read]\n__END__\nraw\n",
            None,
        ),
        (
            "def f; <<~TXT\n  payload\nTXT\nend\n",
            Some("load ARGV[0]; p f"),
        ),
        ("def f; 1\nend\np method(:f).source_location\n", None),
        (
            "def outer\n def inner; 1\n end\nend\n",
            Some("load ARGV[0]; p [outer, inner]"),
        ),
    ] {
        fs::write(&path, source).unwrap();
        let before = ruby(dir.path(), harness);
        assert!(before.status.success(), "{before:?}");
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(
            dir.path(),
            &[
                "--only",
                "Style/TrailingBodyOnMethodDefinition",
                "--fix",
                "lib/a.rb",
            ],
            1,
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        let after = ruby(dir.path(), harness);
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
    }
    // A later independent offense still fixes after the last __LINE__.
    let source = "def f; 1\nend\np __LINE__\ndef g; 2\nend\n";
    fs::write(&path, source).unwrap();
    cli(
        dir.path(),
        &[
            "--only",
            "Style/TrailingBodyOnMethodDefinition",
            "--fix",
            "lib/a.rb",
        ],
        1,
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        source.replace("def g; 2", "def g \n  2")
    );
}
#[test]
fn sibling_deferred_heredoc_values_and_method_returns_are_preserved() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for opener in ["<<TXT", "<<-TXT", "<<~TXT", "<<'TXT'", "<<\"TXT\""] {
        let source = format!("x = {opener};def f; 1\npayload\nTXT\nend\np [x, f]\n");
        fs::write(&path, &source).unwrap();
        let before = ruby(dir.path(), None);
        assert!(before.status.success(), "{before:?}");
        assert_eq!(before.stdout, b"[\"payload\\n\", 1]\n");
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(
            dir.path(),
            &[
                "--only",
                "Style/TrailingBodyOnMethodDefinition",
                "--fix",
                "lib/a.rb",
            ],
            1,
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        let after = ruby(dir.path(), None);
        assert!(after.status.success(), "{after:?}");
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
    }
}
#[test]
fn trailing_shebang_comments_do_not_activate_interpreter_options() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, header, observation) in [
        ("Style/TrailingBodyOnMethodDefinition", "def f", "puts f"),
        ("Style/TrailingBodyOnClass", "class C", "puts 1"),
        ("Style/TrailingBodyOnClass", "class << self", "puts 1"),
        ("Style/TrailingBodyOnModule", "module M", "puts 1"),
    ] {
        let source = format!("{header}; 1 #!/usr/bin/ruby -n\nend\n{observation}\n");
        fs::write(&path, &source).unwrap();
        // Run as the main script with empty stdin, so an activated -n is observable.
        let run = || {
            Command::new("ruby")
                .current_dir(dir.path())
                .arg("lib/a.rb")
                .stdin(std::process::Stdio::null())
                .output()
                .unwrap()
        };
        let before = run();
        assert!(before.status.success(), "{before:?}");
        assert_eq!(before.stdout, b"1\n");
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 1);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        let after = run();
        assert!(after.status.success(), "{after:?}");
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
    }
}
#[test]
fn trailing_method_body_encoding_bom_crlf_comments_and_data_bytes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, expected, status) in [
        (
            "\u{feff}def f; \"café\"\r\nend\r\n__END__\r\nraw\r\n".as_bytes(),
            "\u{feff}def f \r\n  \"café\"\r\nend\r\n__END__\r\nraw\r\n".as_bytes(),
            0,
        ),
        (
            b"# encoding: ISO-8859-1\r\ndef f; \"caf\xe9\"\r\nend\r\n__END__\r\nraw\xff\r\n",
            b"# encoding: ISO-8859-1\r\ndef f \r\n  \"caf\xe9\"\r\nend\r\n__END__\r\nraw\xff\r\n",
            0,
        ),
        (
            b"# encoding: ISO-8859-1\r\ndef f; 1 # caf\xe9\r\nend\r\n",
            b"# encoding: ISO-8859-1\r\ndef f; 1 # caf\xe9\r\nend\r\n",
            1,
        ),
        (
            b"  private def f; 1 # docs\r\n  end\r\n",
            b"  # docs\r\n  private def f \r\n            1 \r\n  end\r\n",
            0,
        ),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(
            dir.path(),
            &["--only", "Style/TrailingBodyOnMethodDefinition", "lib/a.rb"],
            1,
        );
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        cli(
            dir.path(),
            &[
                "--only",
                "Style/TrailingBodyOnMethodDefinition",
                "--fix",
                "lib/a.rb",
            ],
            status,
        );
        assert_eq!(fs::read(&path).unwrap(), expected);
        if status == 1 {
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
    }
}
#[test]
fn trailing_method_body_default_style_layout_composition_preserves_runtime_and_stabilizes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(
        dir.path().join("rustocop.toml"),
        "[cops.\"Style/Documentation\"]\nenabled = false\n",
    )
    .unwrap();
    let source = "class C\n def foo; return ({ :key => 'x' })\n end\n def bar = [\n  2]\nend\n";
    fs::write(&path, source).unwrap();
    let has_ruby = Command::new("ruby").arg("--version").output().is_ok();
    let harness = "load ARGV[0]; p [C.new.foo, C.new.bar]";
    let before = has_ruby.then(|| ruby(dir.path(), Some(harness)));
    for _ in 0..6 {
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--fix", "lib/a.rb"])
            .output()
            .unwrap();
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir.path(), &["--fix", "lib/a.rb"], 0);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    let fixed = fs::read_to_string(&path).unwrap();
    assert_ne!(fixed, source);
    assert!(fixed.contains("{ key: \"x\" }"), "{fixed}");
    if let Some(before) = before {
        assert!(before.status.success(), "{before:?}");
        let after = ruby(dir.path(), Some(harness));
        assert!(after.status.success(), "{after:?}");
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
    }
}
#[test]
fn mixed_style_layout_composition_stabilizes_within_six_explicit_passes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "class C; def one; return ('x'); end\n  def two = [\n    2]\n  def three\n    3; end\nend\nmodule M; extend self\nend\n").unwrap();
    let only = format!(
        "{ONLY},Style/SingleLineMethods,Style/EndlessMethod,Style/Semicolon,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/SignalException,Layout"
    );
    for _ in 0..6 {
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", &only, "--fix", "lib/a.rb"])
            .output()
            .unwrap();
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir.path(), &["--only", &only, "--fix", "lib/a.rb"], 0);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("def one() = \"x\"")
    );
}
