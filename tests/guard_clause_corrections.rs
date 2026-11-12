//! Pinned RuboCop guard_clause_spec.rb examples (MIT License,
//! Copyright (c) 2012-26 Bozhidar Batsov), plus controlled safety witnesses.
use serde::Deserialize;
use std::{fs, path::Path, process::Command};

const COP: &str = "Style/GuardClause";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str], code: i32) {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(code), "{args:?}: {o:?}");
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], code: i32) {
    let file = root.join("a.rb");
    fs::write(&file, source).unwrap();
    let time = fs::metadata(&file).unwrap().modified().unwrap();
    cli(root, args, code);
    assert_eq!(fs::read(&file).unwrap(), source);
    assert_eq!(fs::metadata(file).unwrap().modified().unwrap(), time);
}
fn correct(root: &Path, before: &[u8], after: &[u8], selectors: &str) {
    fs::write(root.join("a.rb"), before).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(["--only", selectors, "--fix", "a.rb"])
        .output()
        .unwrap();
    assert_ne!(o.status.code(), Some(2), "{o:?}");
    assert_eq!(fs::read(root.join("a.rb")).unwrap(), after, "{o:?}");
}
#[derive(Deserialize)]
struct Case {
    spec_line: u32,
    source: String,
    positions: Vec<(u32, u32)>,
    messages: Vec<String>,
    correction: Option<String>,
    toml: String,
}
#[test]
fn all_applicable_pinned_diagnostics_and_correction_calls() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/guard_clause_pinned.json")).unwrap();
    assert_eq!(cases.len(), 77);
    let registry = rustocop::cops::Registry::new();
    let dir = project();
    let (mut canonical, mut block_local, mut line_preserving, mut withheld) = (0, 0, 0, 0);
    for c in cases {
        fs::write(dir.path().join("rustocop.toml"), &c.toml).unwrap();
        let config = rustocop::config::ResolvedConfig::resolve(
            toml::from_str(&c.toml).unwrap(),
            None,
            &registry,
        )
        .unwrap();
        let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
        let result = rustocop::pipeline::lint_one(
            Path::new("a.rb"),
            Path::new("a.rb"),
            Some(c.source.as_bytes()),
            &active,
            &config,
        );
        assert!(
            result.errors.is_empty() && result.internal_errors.is_empty(),
            "spec {}: {result:?}",
            c.spec_line
        );
        let found = result.result.offenses;
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            c.positions,
            "spec {}",
            c.spec_line
        );
        assert_eq!(
            found.iter().map(|o| o.message.clone()).collect::<Vec<_>>(),
            c.messages,
            "spec {}",
            c.spec_line
        );
        if let Some(mut after) = c.correction {
            if (c.spec_line != 622 && after.contains("raise ")) || c.spec_line == 327 {
                // A receiverless raise can return after overriding it. Its
                // deferred heredoc payload also cannot simply move to a header.
                unchanged(
                    dir.path(),
                    c.source.as_bytes(),
                    &["--only", COP, "--fix", "a.rb"],
                    1,
                );
                withheld += 1;
                continue;
            }
            if c.source.starts_with("define_") {
                // No core-dispatch assumption: next always exits this block.
                after = after.replace("  return ", "  next ");
                block_local += 1;
            } else if c.spec_line == 622 {
                // Preserve the deleted end's physical line, including all
                // literal bytes; upstream removes the whole line here.
                after = after.replace("MESSAGE\nend", "MESSAGE\n  \nend");
                line_preserving += 1;
            } else {
                canonical += 1;
            }
            correct(dir.path(), c.source.as_bytes(), after.as_bytes(), COP);
            unchanged(
                dir.path(),
                after.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                0,
            );
        } else {
            // Includes all four upstream expect_no_corrections logical guards
            // as well as every applicable no-offense lookalike.
            unchanged(
                dir.path(),
                c.source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                i32::from(!c.positions.is_empty()),
            );
        }
    }
    assert_eq!(
        (canonical, block_local, line_preserving, withheld),
        (16, 12, 1, 17)
    );
}

#[test]
fn fixing_obeys_config_selectors_and_reenabled_directives() {
    let d = project();
    let before = b"def f\n  if ready?\n    work\n  end\nend\n";
    let after = b"def f\n  return unless ready?\n    work\n  \nend\n";
    unchanged(d.path(), before, &["--only", COP, "a.rb"], 1);
    unchanged(
        d.path(),
        before,
        &["--only", COP, "--except", COP, "--fix", "a.rb"],
        0,
    );
    for config in [
        format!(
            "[cops.Style]\nenabled=false\n[cops.Layout]\nenabled=false\n[cops.\"{COP}\"]\nenabled=false\n"
        ),
        "[cops.Style]\nenabled=false\n[cops.Layout]\nenabled=false\n".into(),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), before, &["--fix", "a.rb"], 0);
        correct(d.path(), before, after, COP);
    }
    fs::write(d.path().join("rustocop.toml"), format!("[cops.Style]\nenabled=false\n[cops.Layout]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n")).unwrap();
    fs::write(d.path().join("a.rb"), before).unwrap();
    cli(d.path(), &["--fix", "a.rb"], 0);
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), after);
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rubocop", "rustocop"] {
        let first = format!(
            "# {prefix}:disable {COP}\n{}# {prefix}:enable {COP}\n",
            String::from_utf8_lossy(before)
        );
        correct(
            d.path(),
            format!("{first}{}", String::from_utf8_lossy(before)).as_bytes(),
            format!("{first}{}", String::from_utf8_lossy(after)).as_bytes(),
            COP,
        );
    }
}

#[test]
fn default_fix_is_a_single_atomic_pass_not_an_implicit_formatter_loop() {
    let d = project();
    let before = b"def f(c)\n  if c\n    value = { :foo => 'bar' }; side_effect(value)\n    return (value)\n  end\nend\n";
    let after = b"def f(c)\n  return unless c\n    value = { :foo => 'bar' }; side_effect(value)\n    return (value)\n  \nend\n";
    unchanged(d.path(), before, &["a.rb"], 1);
    fs::write(d.path().join("a.rb"), before).unwrap();
    cli(d.path(), &["--fix", "--fix-passes", "1", "a.rb"], 1);
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), after);
}

#[test]
fn bom_crlf_comments_literals_line_numbers_and_data_survive_exactly() {
    let d = project();
    for nl in ["\n", "\r\n"] {
        for body in [
            "__LINE__ # stay here",
            "\"first\nlast\"",
            "/first\nlast/",
            "`printf 'first\nlast'`",
            "<<~TEXT\n      payload\n    TEXT",
            "begin\n      effect\n    rescue\n      recover\n    ensure\n      cleanup\n    end",
        ] {
            let source = format!("\u{feff}# encoding: UTF-8\ndef f\n  if ready? # condition\n    # branch\n    {body}\n  end # end of condition\nend\np __LINE__\n__END__\nraw\0data\n").replace('\n', nl);
            let after = source
                .replace("if ready?", "return unless ready?")
                .replace("end # end of condition", " # end of condition");
            correct(d.path(), source.as_bytes(), after.as_bytes(), COP);
        }
    }
    let ascii = b"# encoding: ISO-8859-1\ndef f\n if ready?\n  value\n end\nend\n__END__\n\xff\n";
    let after =
        b"# encoding: ISO-8859-1\ndef f\n return unless ready?\n  value\n \nend\n__END__\n\xff\n";
    correct(d.path(), ascii, after, COP);
    // Replacement strings cannot represent the original non-UTF8 token/body.
    for source in [
        b"# encoding: ISO-8859-1\ndef f\n if ready?\n  '\xe9'\n end\nend\n".as_slice(),
        b"# encoding: ISO-8859-1\ndef f\n if '\xe9'\n  value\n end\nend\n",
    ] {
        unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], 1);
    }
}

#[test]
fn binding_comments_directive_spans_and_line_changing_edits_are_withheld() {
    let d = project();
    for source in [
        "def f\n if ready?\n  value = 1\n else\n  return value\n end\nend\n",
        "def f\n if (text in value)\n  return value\n else\n  effect\n end\nend\n",
        "def f\n if /(?<value>.)/ =~ text\n  return value\n else\n  effect\n end\nend\n",
        "def f\n if ready?\n  return (value = 1)\n else\n  effect\n end\nend\n",
        "def f\n consume(\n  if ready?\n   return 1\n  else\n   value\n  end\n )\nend\n",
        "def f\n [\n  if ready?\n   return 1\n  else\n   value\n  end\n ]\nend\n",
        "def f\n defined?(\n  if ready?\n   return 1\n  else\n   value\n  end\n )\nend\n",
        "def f\n defined?(begin\n  if ready?\n   return 1\n  else\n   value\n  end\n end)\nend\n",
        "def f\n if ready?\n  return object.call(value:)\n else\n  effect\n end\nend\n",
        "def f\n if ready?\n  return __LINE__\n else\n  effect\n end\nend\n",
        "def f\n if ready?\n  return <<~TEXT\n    payload\n  TEXT\n else\n  effect\n end\nend\n",
        "if ready?\n raise 'oops'\nelse\n effect\nend\n",
        "if ready?\n work || raise('oops')\nelse\n effect\nend\n",
        "def f\n if ready?\n  work # encoding: ISO-8859-1\n end\nend\n",
        "def f\n if # keep predicate annotation\n  ready?\n  work\n end\nend\n",
        "def f\n if ready?\n  work # rubocop:disable Style/GuardClause\n end\nend\n",
        "def f\n if ready?\n  work\n end\nend\nputs File.read(__FILE__)\n",
    ] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
    }
    fs::write(
        d.path().join("rustocop.toml"),
        "[cops.\"Layout/LineLength\"]\nmax=30\n",
    )
    .unwrap();
    let source =
        "def f\n if long_condition_name && more_work\n  work\n  more_work\n end\nend\np __LINE__\n";
    unchanged(
        d.path(),
        source.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        1,
    );
}

fn ruby_with(source: &[u8], program: &std::ffi::OsStr) -> std::process::Output {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(program)
        .args(["--disable-gems", "-W0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}
fn assert_runtime(before: &[u8], after: &[u8]) {
    let mut programs = vec![std::ffi::OsString::from("ruby")];
    if let Some(native) = crate::support::runtime::ruby_os() {
        programs.push(native);
    }
    for program in programs {
        let old = ruby_with(before, &program);
        let new = ruby_with(after, &program);
        assert!(
            old.status.success() && new.status.success(),
            "{program:?}: {old:?} {new:?}"
        );
        assert_eq!(old.stdout, new.stdout, "{program:?}");
        assert_eq!(old.stderr, new.stderr, "{program:?}");
    }
}

#[test]
fn controlled_return_values_effect_order_nested_guards_and_ensure_match_ruby() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let d = project();
    for source in [
        "def f(c, log)\n if c\n  log << :work\n  42\n end\nend\nl=[]; p [f(false,l), f(true,l), l]\n",
        "def f(c, log)\n unless c\n  log << :work\n  42\n end\nend\nl=[]; p [f(false,l), f(true,l), l]\n",
        "def f(c, log)\n if (log << :condition; c)\n  return (log << :exit; 42)\n else\n  log << :work\n  7\n end\nensure\n log << :ensure\nend\nl=[]; p [f(false,l), f(true,l), l]\n",
        "def f(c, log)\n if c\n  log << :work\n  7\n else\n  return (log << :exit; 42)\n end\nend\nl=[]; p [f(false,l), f(true,l), l]\n",
        "l=[]; p [false,true].map { |c|\n if c\n  next (l << :exit; 42)\n else\n  l << :work\n  7\n end\n}; p l\n",
        "l=[]; p [false,true].each { |c|\n if c\n  l << :work\n else\n  break (l << :exit; 42)\n end\n}; p l\n",
        "def f(a,b,log)\n if a\n  log << :outer\n  unless b\n   log << :inner\n   42\n  end\n end\nend\nl=[]; p [f(false,false,l),f(true,true,l),f(true,false,l),l]\n",
        "def f(c, log)\n if c\n  begin\n   log << :work\n   42\n  ensure\n   log << :ensure\n  end\n end\nend\nl=[]; p [f(false,l),f(true,l),l]\n",
        "def f(c, log)\n if c\n  __LINE__\n end\nend\np [f(false,[]),f(true,[]),__LINE__]\n",
        // Unknown define_method dispatch: next must not escape run's method.
        "def define_method(*)\n yield false\n yield true\nend\ndef run\n log=[]\n define_method(:f) do |c|\n  if c\n   log << :work\n   42\n  end\n end\n log << :after\nend\np run\n",
        "class C\n define_method(:f) do |c|\n  if c\n   42\n  end\n end\nend\np [C.new.f(false),C.new.f(true)]\n",
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        cli(d.path(), &["--only", COP, "--fix", "a.rb"], 0);
        let after = fs::read(d.path().join("a.rb")).unwrap();
        assert_ne!(
            after,
            source.as_bytes(),
            "fixture must really correct: {source}"
        );
        assert_runtime(source.as_bytes(), &after);
    }
}

#[test]
fn native_34_implicit_method_block_parameters_preserve_results() {
    let Some(native) = crate::support::runtime::ruby_os() else {
        return;
    };
    let d = project();
    for parameter in ["_1", "it"] {
        let source = format!(
            "class C\n define_method(:f) do\n  if {parameter}\n   42\n  end\n end\nend\np [C.new.f(false), C.new.f(true)]\n"
        );
        let after = source
            .replace(
                &format!("if {parameter}"),
                &format!("next unless {parameter}"),
            )
            .replace("   42\n  end", "   42\n  ");
        correct(d.path(), source.as_bytes(), after.as_bytes(), COP);
        let old = ruby_with(source.as_bytes(), &native);
        let new = ruby_with(after.as_bytes(), &native);
        assert!(
            old.status.success() && new.status.success(),
            "{old:?} {new:?}"
        );
        assert_eq!(old.stdout, b"[nil, 42]\n");
        assert_eq!(new.stdout, old.stdout);
        assert_eq!(new.stderr, old.stderr);
    }
}

#[test]
fn overridden_raise_and_short_circuit_guards_retain_branch_effects() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let d = project();
    for exit in [
        "raise('oops')",
        "fail('oops')",
        "true || raise('oops')",
        "false && raise('oops')",
    ] {
        let source = format!(
            "def raise(*); :returned; end\nalias fail raise\ndef f(c,l)\n if c\n  {exit}\n else\n  l << :work\n end\nend\nl=[]; p [f(true,l),l]\n"
        );
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        assert_runtime(source.as_bytes(), source.as_bytes());
    }
}
