//! RuboCop next_spec.rb examples (MIT License, Copyright (c)
//! 2012-26 Bozhidar Batsov), fixed-policy corrections and semantic witnesses.
use std::{fs, path::Path, process::Command};
const COP: &str = "Style/Next";
const BAD: &str = "[].each do\n  if ready?\n    a\n    b\n    c\n  end\nend\n";
const GOOD: &str = "[].each do\n  next unless ready?\n  a\n  b\n  c\nend\n";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str]) -> std::process::Output {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert_ne!(o.status.code(), Some(2), "{args:?}: {o:?}");
    o
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], count: usize) {
    let file = root.join("a.rb");
    fs::write(&file, source).unwrap();
    let time = fs::metadata(&file).unwrap().modified().unwrap();
    let o = cli(root, args);
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert_eq!(
        stdout.lines().filter(|l| l.contains(COP)).count(),
        count,
        "{o:?}"
    );
    assert_eq!(fs::read(&file).unwrap(), source);
    assert_eq!(fs::metadata(file).unwrap().modified().unwrap(), time);
}
fn correct(root: &Path, before: &[u8], after: &[u8], selectors: &str) {
    fs::write(root.join("a.rb"), before).unwrap();
    let o = cli(root, &["--only", selectors, "--fix", "a.rb"]);
    assert_eq!(fs::read(root.join("a.rb")).unwrap(), after, "{o:?}");
}
#[test]
fn default_selectors_config_and_no_write_contract() {
    let d = project();
    let registry = rustocop::cops::Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    unchanged(d.path(), BAD.as_bytes(), &["a.rb"], 1);
    unchanged(d.path(), BAD.as_bytes(), &["--only", COP, "a.rb"], 1);
    unchanged(
        d.path(),
        BAD.as_bytes(),
        &["--only", COP, "--except", COP, "--fix", "a.rb"],
        0,
    );
    for config in [
        format!("[cops.\"{COP}\"]\nenabled=false\n"),
        "[cops.Style]\nenabled=false\n".into(),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), BAD.as_bytes(), &["a.rb"], 0);
        correct(d.path(), BAD.as_bytes(), GOOD.as_bytes(), COP);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled=false\n[cops.\"{COP}\"]\nenabled=true\n"),
    )
    .unwrap();
    unchanged(d.path(), BAD.as_bytes(), &["a.rb"], 1);
}
#[test]
fn directives_exclusions_and_stdin_honor_the_real_pipeline() {
    use std::io::Write;
    use std::process::Stdio;
    let d = project();
    for config in [
        "exclude=[\"a.rb\"]\n".to_owned(),
        format!("[cops.\"{COP}\"]\nexclude=[\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            d.path(),
            BAD.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:disable {COP}\n{BAD}"),
            BAD.replace("if ready?", &format!("if ready? # {prefix}:disable {COP}")),
        ] {
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                0,
            );
        }
        // Body-only disabling suppresses the atomic edit, not the header offense.
        let source = BAD.replace("    b", &format!("    b # {prefix}:disable {COP}"));
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        let disabled = format!("# {prefix}:disable {COP}\n{BAD}# {prefix}:enable {COP}\n");
        correct(
            d.path(),
            format!("{disabled}{BAD}").as_bytes(),
            format!("{disabled}{GOOD}").as_bytes(),
            COP,
        );
    }
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
        .write_all(BAD.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains("a.rb:2:3:") && text.contains("Use `next` to skip iteration."),
        "{text}"
    );
}
#[test]
fn bom_crlf_line_numbers_literals_comments_and_data_keep_their_values() {
    let d = project();
    for newline in ["\n", "\r\n"] {
        for body in [
            "value = __LINE__\n    log << value\n    value",
            "text = \"first\n    second\n    last\"\n    text",
            "text = <<~TEXT\n      payload\n    TEXT\n    text",
            "text = /first\n    last/\n    text\n    text",
        ] {
            let before = format!("\u{feff}# encoding: UTF-8\nxs.map do |x|\n  if x # condition\n    {body}\n  end # conditional\nend\np __LINE__\n__END__\nraw\0data\n").replace('\n',newline);
            fs::write(d.path().join("a.rb"), &before).unwrap();
            cli(d.path(), &["--only", COP, "--fix", "a.rb"]);
            let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
            assert!(after.contains("next unless x # condition"));
            assert_eq!(before.matches('\n').count(), after.matches('\n').count());
            assert!(after.ends_with(&format!("__END__{newline}raw\0data{newline}")));
            assert!(after.starts_with('\u{feff}'));
            assert_eq!(before.matches('\r').count(), after.matches('\r').count());
        }
    }
    let ascii = b"# encoding: ISO-8859-1\nxs.each do\n  if ready?\n    a\n    b\n    c\n  end\nend\n__END__\n\xff\n";
    let after = b"# encoding: ISO-8859-1\nxs.each do\n  next unless ready?\n  a\n  b\n  c\nend\n__END__\n\xff\n";
    correct(d.path(), ascii, after, COP);
    // The correction API carries UTF-8 strings, so non-UTF8 replacement spans
    // must retain their offense without lossy decoding or partial edits.
    for bytes in [
        b"# encoding: ISO-8859-1\nxs.each do\n if ready?\n  '\xe9'\n  b\n  c\n end\nend\n"
            .as_slice(),
        b"# encoding: ISO-8859-1\nxs.each do\n if '\xe9'\n  a\n  b\n  c\n end\nend\n",
    ] {
        unchanged(d.path(), bytes, &["--only", COP, "--fix", "a.rb"], 1);
    }
}
fn ruby(source: &[u8], program: &std::ffi::OsStr) -> std::process::Output {
    use std::{io::Write, process::Stdio};
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
fn same_runtime(before: &[u8], after: &[u8]) {
    let mut programs = vec![std::ffi::OsString::from("ruby")];
    if let Some(native) = crate::support::runtime::ruby_os() {
        programs.push(native);
    }
    for program in programs {
        let old = ruby(before, &program);
        let new = ruby(after, &program);
        assert!(
            old.status.success() && new.status.success(),
            "{program:?}: {old:?} {new:?}"
        );
        assert_eq!(old.stdout, new.stdout, "{program:?}");
        assert_eq!(old.stderr, new.stderr, "{program:?}");
    }
}

#[test]
fn native_34_numbered_and_it_parameters_preserve_results() {
    let Some(native) = crate::support::runtime::ruby_os() else {
        return;
    };
    let d = project();
    for arg in ["_1", "it"] {
        let before = format!(
            "result = [false,true].map do\n if {arg}\n  a=1\n  b=2\n  a+b\n end\nend\np result\n"
        );
        let after = format!(
            "result = [false,true].map do\n next unless {arg}\n a=1\n b=2\n a+b\nend\np result\n"
        );
        correct(d.path(), before.as_bytes(), after.as_bytes(), COP);
        let old = ruby(before.as_bytes(), &native);
        let new = ruby(after.as_bytes(), &native);
        assert!(
            old.status.success() && new.status.success(),
            "{old:?} {new:?}"
        );
        assert_eq!(old.stdout, b"[nil, 3]\n");
        assert_eq!(old.stdout, new.stdout);
    }
}

#[test]
fn nested_overlap_is_deterministic_and_stabilizes_in_two_explicit_passes() {
    let d = project();
    let before = "log=[]\n[false,true].each do |x|\n if x\n  [false,true].each do |y|\n   if y\n    log << :first\n    log << :second\n    log << :third\n   end\n  end\n end\nend\np log\n";
    fs::write(d.path().join("a.rb"), before).unwrap();
    let first = cli(
        d.path(),
        &["--only", COP, "--fix", "--fix-passes", "1", "a.rb"],
    );
    assert_eq!(first.status.code(), Some(1), "{first:?}");
    let after_first = fs::read(d.path().join("a.rb")).unwrap();
    assert_ne!(after_first, before.as_bytes());
    let second = cli(
        d.path(),
        &["--only", COP, "--fix", "--fix-passes", "1", "a.rb"],
    );
    assert_eq!(second.status.code(), Some(0), "{second:?}");
    let after_second = fs::read(d.path().join("a.rb")).unwrap();
    assert_ne!(after_second, after_first);
    unchanged(
        d.path(),
        &after_second,
        &["--only", COP, "--fix", "--fix-passes", "1", "a.rb"],
        0,
    );
    if Command::new("ruby").arg("--version").output().is_ok() {
        same_runtime(before.as_bytes(), &after_first);
        same_runtime(before.as_bytes(), &after_second);
    }
}

#[test]
fn mixed_style_and_layout_fixes_keep_control_flow_and_settle_without_cycles() {
    let d = project();
    let before = "def f(x,log)\n [x].map do |value|\n  if value\n   result = { :key => 'value' }; log << result\n   log << :work\n   return (result)\n  end\n end\nend\nlog=[]; p [f(false,log),f(true,log),log]\n";
    fs::write(d.path().join("a.rb"), before).unwrap();
    let selectors = "Style/Next,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout/TrailingWhitespace";
    let mut snapshots = vec![before.as_bytes().to_vec()];
    for _ in 0..4 {
        cli(d.path(), &["--only", selectors, "--fix", "a.rb"]);
        let after = fs::read(d.path().join("a.rb")).unwrap();
        if after == *snapshots.last().unwrap() {
            break;
        }
        assert!(!snapshots.contains(&after), "correction cycle");
        if Command::new("ruby").arg("--version").output().is_ok() {
            same_runtime(before.as_bytes(), &after);
        }
        snapshots.push(after);
    }
    let after = fs::read(d.path().join("a.rb")).unwrap();
    assert!(String::from_utf8_lossy(&after).contains("next unless value"));
    let time = fs::metadata(d.path().join("a.rb"))
        .unwrap()
        .modified()
        .unwrap();
    cli(d.path(), &["--only", selectors, "--fix", "a.rb"]);
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), after);
    assert_eq!(
        fs::metadata(d.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap(),
        time
    );
    assert!(
        snapshots.len() <= 4,
        "curated fixture must stabilize within three changing passes"
    );
}
