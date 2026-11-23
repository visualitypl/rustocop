//! Safe correction pipeline/CLI and controlled runtime evidence.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

const COP: &str = "Style/ArgumentsForwarding";
const BEFORE: &str = "def foo(*args, **kwargs, &block)\n  bar(*args, **kwargs, &block)\nend\n";
const AFTER: &str = "def foo(...)\n  bar(...)\nend\n";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn cli(dir: &Path, args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(matches!(output.status.code(), Some(0..=2)), "{output:?}");
    output
}
fn fix(dir: &Path, only: &str) -> Output {
    let out = cli(dir, &["--only", only, "--fix", "--fix-passes", "1", "a.rb"]);
    assert_ne!(out.status.code(), Some(2), "{out:?}");
    out
}

#[test]
fn cli_default_selectors_and_config_correct_only_when_requested() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for (config, args, correct) in [
        ("".to_owned(), vec!["--fix", "a.rb"], true),
        ("".to_owned(), vec!["--only", COP, "a.rb"], false),
        ("".to_owned(), vec!["--only", COP, "--fix", "a.rb"], true),
        (
            "".to_owned(),
            vec!["--only", COP, "--except", COP, "--fix", "a.rb"],
            false,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["--fix", "a.rb"],
            false,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["--only", COP, "--fix", "a.rb"],
            true,
        ),
        (
            format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
            vec!["--fix", "a.rb"],
            true,
        ),
        (
            "exclude = [\"a.rb\"]\n".to_owned(),
            vec!["--only", COP, "--fix", "a.rb"],
            false,
        ),
        (
            format!("[cops.\"{COP}\"]\nexclude = [\"a.rb\"]\n"),
            vec!["--only", COP, "--fix", "a.rb"],
            false,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        fs::write(&path, BEFORE).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let out = cli(dir.path(), &args);
        assert_ne!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            if correct { AFTER } else { BEFORE },
            "{out:?}"
        );
        if !correct {
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
    }
    fs::write(
        dir.path().join("rustocop.toml"),
        format!("[cops.\"{COP}\"]\nenabled = false\n"),
    )
    .unwrap();
    for prefix in ["rustocop", "rubocop"] {
        fs::write(&path, format!("# {prefix}:enable {COP}\n{BEFORE}")).unwrap();
        let out = cli(dir.path(), &["--fix", "a.rb"]);
        assert_ne!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("# {prefix}:enable {COP}\n{AFTER}")
        );
    }
}

#[test]
fn disabled_uses_or_signatures_withhold_the_entire_atomic_edit() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for prefix in ["rustocop", "rubocop"] {
        for source in [
            format!("# {prefix}:disable {COP}\n{BEFORE}"),
            format!("# {prefix}:disable-next {COP}\n{BEFORE}"),
            format!(
                "def foo(*args, **kwargs, &block)\n  # {prefix}:disable-next {COP}\n  bar(*args, **kwargs, &block)\nend\n"
            ),
            format!(
                "def foo(*args, **kwargs, &block) # {prefix}:disable {COP}\n  bar(*args, **kwargs, &block)\nend\n"
            ),
            format!(
                "def foo(*args, **kwargs, &block)\n  bar(*args, **kwargs, &block) # {prefix}:disable {COP}\nend\n"
            ),
        ] {
            fs::write(&path, &source).unwrap();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            fix(dir.path(), COP);
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
        let source = format!("# {prefix}:disable {COP}\n{BEFORE}# {prefix}:enable {COP}\n{BEFORE}");
        fs::write(&path, &source).unwrap();
        fix(dir.path(), COP);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("# {prefix}:disable {COP}\n{BEFORE}# {prefix}:enable {COP}\n{AFTER}")
        );
    }
}

#[test]
fn bom_crlf_legacy_literals_line_values_and_data_bytes_are_preserved() {
    let dir = project();
    let path = dir.path().join("a.rb");
    let mut before = b"\xef\xbb\xbf# encoding: UTF-8\r\n".to_vec();
    before.extend(BEFORE.replace('\n', "\r\n").bytes());
    before.extend_from_slice(b"p __LINE__\r\n__END__\r\n\xff *args **kwargs &block\r\n");
    let mut after = b"\xef\xbb\xbf# encoding: UTF-8\r\n".to_vec();
    after.extend(AFTER.replace('\n', "\r\n").bytes());
    after.extend_from_slice(b"p __LINE__\r\n__END__\r\n\xff *args **kwargs &block\r\n");
    fs::write(&path, &before).unwrap();
    fix(dir.path(), COP);
    assert_eq!(fs::read(&path).unwrap(), crate::support::lf::lf(&after));
    // Non-UTF-8 outside the atomic span remains byte-for-byte intact.
    let mut legacy = b"# encoding: ISO-8859-1\ntext = '\xe9'\n".to_vec();
    legacy.extend_from_slice(BEFORE.as_bytes());
    fs::write(&path, &legacy).unwrap();
    fix(dir.path(), COP);
    let mut expected = b"# encoding: ISO-8859-1\ntext = '\xe9'\n".to_vec();
    expected.extend_from_slice(AFTER.as_bytes());
    assert_eq!(fs::read(&path).unwrap(), expected);
    // An indivisible envelope with legacy bytes cannot be represented by the
    // engine's String replacement; preserve the diagnostic and do not write.
    let legacy = b"# encoding: ISO-8859-1\ndef foo(*args); text = '\xe9'; bar(*args); end\n";
    fs::write(&path, legacy).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let out = fix(dir.path(), COP);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(fs::read(&path).unwrap(), legacy);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn withheld_clean_and_error_files_are_not_written_and_safe_methods_still_correct() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for source in [
        AFTER,
        "def foo(*args)\n",
        "def foo(*args); bar(*args); [*args]; end\n",
        "def foo(*args, **kwargs, &block); p local_variables; bar(*args, **kwargs, &block); end\n",
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", COP, "--fix", "a.rb"]);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
    let uncertain = "def uncertain(*args); bar(*args); [*args]; end\n";
    fs::write(&path, format!("{uncertain}{BEFORE}")).unwrap();
    fix(dir.path(), COP);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        format!("{uncertain}{AFTER}")
    );
}

#[test]
fn uncovered_anonymous_splats_preserve_the_method_without_blocking_safe_siblings() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for extra_use in ["[*]", "{**}"] {
        let uncertain = format!("def foo(*, **, &); bar(*, **, &); {extra_use}; end\n");
        for sibling in ["", "def good(*args); baz(*args); end\n"] {
            let source = format!("{uncertain}{sibling}");
            fs::write(&path, &source).unwrap();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            let out = fix(dir.path(), COP);
            assert_eq!(out.status.code(), Some(1), "{out:?}");
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                format!("{uncertain}{}", sibling.replace("*args", "*")),
                "{out:?}"
            );
            assert!(String::from_utf8_lossy(&out.stdout).contains(COP));
            assert_eq!(
                ruby_prism::parse(&fs::read(&path).unwrap())
                    .errors()
                    .count(),
                0
            );
            if sibling.is_empty() {
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            }
            // The withheld offense must remain stable on another explicit run.
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            assert_eq!(fix(dir.path(), COP).status.code(), Some(1));
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
    }
}

#[test]
fn incompatible_peers_and_style_layout_are_deterministic_single_passes() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for only in [
        "Style/ArgumentsForwarding,Style/MethodDefParentheses,Style/EndlessMethod,Layout",
        "Layout,Style/EndlessMethod,Style/MethodDefParentheses,Style/ArgumentsForwarding",
    ] {
        fs::write(
            &path,
            "def foo *args, **kwargs, &block\n  bar(*args, **kwargs, &block)  \nend\n",
        )
        .unwrap();
        fix(dir.path(), only);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "def foo(*args, **kwargs, &block)\n  bar(*args, **kwargs, &block)\nend\n"
        );
        fix(dir.path(), only);
        assert_eq!(fs::read_to_string(&path).unwrap(), AFTER);
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fix(dir.path(), only);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
    let only = "Style/ArgumentsForwarding,Style/EndlessMethod";
    fs::write(&path, "def foo(*args) =\n  bar(*args)\n").unwrap();
    fix(dir.path(), only);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "def foo(*args)\n  bar(*args)\nend\n"
    );
    fix(dir.path(), only);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "def foo(*)\n  bar(*)\nend\n"
    );
    // The BlockForwarding peer is now real; full argument forwarding wins atomically.
    fs::write(&path, BEFORE).unwrap();
    fix(
        dir.path(),
        "Style/ArgumentsForwarding,Naming/BlockForwarding",
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), AFTER);
}

fn ruby(source: &str) -> Option<Output> {
    match Command::new("ruby").args(["-e", source]).output() {
        Ok(out) => Some(out),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("Ruby unavailable: controlled runtime observation skipped");
            None
        }
        Err(error) => panic!("Ruby: {error}"),
    }
}

#[test]
fn ruby_observes_equal_arguments_effects_block_identity_exceptions_and_control_flow() {
    let dir = project();
    let path = dir.path().join("a.rb");
    let source = r#"$events = []
def mark(value)
  $events << value
  value
end
def sink(*values, **keywords, &callback)
  $events << [values, keywords, callback.equal?($callback)]
  callback&.call(:invoked)
  raise 'controlled' if keywords[:fail]
  :result
end
def foo(x = mark(:default), *args, **kwargs, &block)
  begin
    sink(mark(x), mark(__LINE__), *args, **kwargs, &block)
  rescue RuntimeError => error
    $events << error.message
  ensure
    $events << :ensure
  end
end
def separate(*args, **kwargs, &block)
  sink(*args, {}, **kwargs, &block)
end
def post_rest(head = :default, *args, tail, **kwargs, &block)
  sink(mark(head), *args, mark(tail), **kwargs, &block)
end
def positional(*args)
  sink(*args)
end
def via_lambda(*args, **kwargs, &block)
  -> { sink(*args, **kwargs, &block) }.call
end
def keywords(**kwargs)
  sink(**kwargs)
end
def blocked(&block)
  sink(&block)
end
$callback = ->(value) { $events << value }
foo(& $callback)
foo(:explicit, {}, **{}, &$callback)
foo(:explicit, {k: 1}, k: 2, fail: true, &$callback)
separate(1, **{}, &$callback)
separate({}, k: 1, &$callback)
post_rest(1, 2, 3, k: 4, &$callback)
post_rest(:tail, &$callback)
via_lambda({}, k: 1, &$callback)
positional({k: 1})
keywords(k: 1)
blocked(&$callback)
p $events
"#;
    let Some(before) = ruby(source) else { return };
    assert!(before.status.success(), "{before:?}");
    // Lint only forwarding methods, not the controlled external Proc harness.
    let start = source.find("def foo").unwrap();
    let end = source.find("$callback =").unwrap();
    fs::write(&path, &source[start..end]).unwrap();
    fix(dir.path(), COP);
    let corrected = fs::read_to_string(&path).unwrap();
    assert!(
        corrected.contains("def foo(x = mark(:default), ...)"),
        "{corrected}"
    );
    assert!(corrected.contains("sink(*, {}, **, &)"));
    assert!(corrected.contains("def post_rest(head = :default, *, tail, **, &)"));
    assert!(corrected.contains("def positional(*)"));
    assert!(corrected.contains("def keywords(**)"));
    assert!(corrected.contains("def blocked(&)"));
    assert!(corrected.contains("-> { sink(...) }.call"));
    let after = ruby(&format!(
        "{}{corrected}{}",
        &source[..start],
        &source[end..]
    ))
    .unwrap();
    assert_eq!(before.status.code(), after.status.code(), "{after:?}");
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
}

#[test]
fn ruby_witnesses_why_reflection_and_dynamic_callback_edits_are_withheld() {
    let dir = project();
    let path = dir.path().join("a.rb");
    let runtime_prefix = "def bar(*values, **keywords, &callback); end\ndef factory; -> { p method(:foo).parameters }; end\n";
    for (source, naive) in [
        (format!("{BEFORE}p method(:foo).parameters\n"), format!("{AFTER}p method(:foo).parameters\n")),
        (format!("{BEFORE}callback = factory\ncallback.call\n"), format!("{AFTER}callback = factory\ncallback.call\n")),
        ("def foo(*args, **kwargs, &block)\n p local_variables.sort\n bar(*args, **kwargs, &block)\nend\nfoo(1, k: 2)\n".to_owned(), "def foo(...)\n p local_variables.sort\n bar(...)\nend\nfoo(1, k: 2)\n".to_owned()),
        ("def foo(*args, **kwargs, &block)\n p eval('defined?(args)')\n bar(*args, **kwargs, &block)\nend\nfoo(1, k: 2)\n".to_owned(), "def foo(...)\n p eval('defined?(args)')\n bar(...)\nend\nfoo(1, k: 2)\n".to_owned()),
    ] {
        let Some(before) = ruby(&format!("{runtime_prefix}{source}")) else { return };
        assert!(before.status.success(), "{before:?}");
        fs::write(&path, &source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let out = fix(dir.path(), COP);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        let changed = ruby(&format!("{runtime_prefix}{naive}")).unwrap();
        assert!(changed.status.success(), "{changed:?}");
        assert_ne!(before.stdout, changed.stdout, "unsafe naive witness must actually differ");
    }
}

#[test]
fn ruby_34_nested_block_forwarding_runtime_is_explicitly_version_gated() {
    let Some(version) = ruby("print RUBY_VERSION.split('.').take(2).join('.')") else {
        return;
    };
    let version = String::from_utf8_lossy(&version.stdout);
    let version: Vec<u32> = version
        .split('.')
        .map(|part| part.parse().unwrap())
        .collect();
    if (version[0], version[1]) < (3, 4) {
        eprintln!(
            "Host Ruby below 3.4: nested anonymous block runtime not claimed; pinned Prism correction/parse cases are covered separately."
        );
        return;
    }
    let before = "def foo(*args, &block)\n [1].each { bar(*args, &block) }\nend\n";
    let after = "def foo(*, &)\n [1].each { bar(*, &) }\nend\n";
    let dir = project();
    let path = dir.path().join("a.rb");
    fs::write(&path, before).unwrap();
    fix(dir.path(), COP);
    assert_eq!(fs::read_to_string(&path).unwrap(), after);
    let harness =
        "def bar(*values, &callback); p [values, callback.call]; end\nfoo({}) { :called }\n";
    let original = ruby(&format!("{before}{harness}")).unwrap();
    let corrected = ruby(&format!("{after}{harness}")).unwrap();
    assert!(original.status.success(), "{original:?}");
    assert_eq!(
        original.status.code(),
        corrected.status.code(),
        "{corrected:?}"
    );
    assert_eq!(original.stdout, corrected.stdout);
}

#[test]
fn ruby_inherited_super_and_destructuring_preserve_positional_keyword_separation() {
    let source = r#"class Parent
  def foo(*values, **keywords, &callback)
    p [values, keywords, callback.equal?($callback)]
    callback&.call
  end
end
class Child < Parent
  def foo(*args, **kwargs, &block)
    super(*args, **kwargs, &block)
    super
  end
end
def destructure((x, y), **kwargs)
  target(x, y, **kwargs)
end
def target(*values, **keywords)
  p [values, keywords]
end
$callback = -> { puts :called }
Child.new.foo({}, **{}, &$callback)
Child.new.foo(1, k: 2, &$callback)
destructure([1, 2], k: 3)
"#;
    let Some(before) = ruby(source) else { return };
    assert!(before.status.success(), "{before:?}");
    let dir = project();
    let start = source.find("class Child").unwrap();
    let end = source.find("def target").unwrap();
    fs::write(dir.path().join("a.rb"), &source[start..end]).unwrap();
    fix(dir.path(), COP);
    let corrected = fs::read_to_string(dir.path().join("a.rb")).unwrap();
    assert!(corrected.contains("super(...)\n    super"));
    assert!(corrected.contains("def destructure((x, y), **)"));
    let after = ruby(&format!(
        "{}{corrected}{}",
        &source[..start],
        &source[end..]
    ))
    .unwrap();
    assert_eq!(before.status.code(), after.status.code(), "{after:?}");
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
}
