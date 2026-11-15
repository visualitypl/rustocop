//! RuboCop next_spec.rb examples (MIT License, Copyright (c)
//! 2012-26 Bozhidar Batsov), fixed-policy corrections and semantic witnesses.
use serde::Deserialize;
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
#[derive(Deserialize)]
struct Case {
    spec_line: u32,
    source: String,
    positions: Vec<(u32, u32)>,
    messages: Vec<String>,
    correction: Option<String>,
}
#[test]
fn every_applicable_pinned_call_and_correction() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/next_pinned.json")).unwrap();
    assert_eq!(cases.len(), 31);
    let registry = rustocop::cops::Registry::new();
    let config =
        rustocop::config::ResolvedConfig::resolve(Default::default(), None, &registry).unwrap();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    let d = project();
    let (mut canonical, mut fixed_threshold, mut heredoc_boundary, mut withheld) = (0, 0, 0, 0);
    for c in cases {
        let r = rustocop::pipeline::lint_one(
            Path::new("a.rb"),
            Path::new("a.rb"),
            Some(c.source.as_bytes()),
            &active,
            &config,
        );
        assert!(
            r.errors.is_empty() && r.internal_errors.is_empty(),
            "spec {}: {r:?}",
            c.spec_line
        );
        let found = r.result.offenses;
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
            if c.spec_line == 357 {
                // Flattening the multiline condition deletes a physical line;
                // retaining it in modifier syntax has ambiguous Ruby binding.
                unchanged(
                    d.path(),
                    c.source.as_bytes(),
                    &["--only", COP, "--fix", "a.rb"],
                    1,
                );
                withheld += 1;
                continue;
            }
            if c.spec_line == 512 {
                // The inner two-line body is legal at fixed MinBodyLength=3.
                after = after.replace(
                    "next unless num != 2\n  puts 'hello'\n  puts 'world'",
                    "if num != 2\n    puts 'hello'\n    puts 'world'\n  end",
                );
                fixed_threshold += 1;
            } else if matches!(c.spec_line, 581 | 607) {
                // Retain exact heredoc closing anchors for the shared physical
                // ownership proof; only ordinary Ruby indentation changes.
                after = after.replace("\n  BLAH\n", "\n    BLAH\n");
                heredoc_boundary += 1;
            } else {
                canonical += 1;
            }
            correct(d.path(), c.source.as_bytes(), after.as_bytes(), COP);
            unchanged(
                d.path(),
                after.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                0,
            );
        } else {
            unchanged(
                d.path(),
                c.source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                c.positions.len(),
            );
        }
    }
    assert_eq!(
        (canonical, fixed_threshold, heredoc_boundary, withheld),
        (4, 2, 2, 2)
    );
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
fn inline_then_bodies_keep_their_statement_boundary_and_runtime_behavior() {
    let d = project();
    for keyword in ["if", "unless"] {
        for value in [false, true] {
            let source = format!(
                "def ready?; {value}; end\ndef first; puts :first; end\n[1].each do\n  {keyword} ready? then first\n    puts :second\n    puts :third\n    puts :fourth\n  end\nend\n"
            );
            fs::write(d.path().join("a.rb"), &source).unwrap();
            cli(d.path(), &["--only", COP, "--fix", "a.rb"]);
            let after = fs::read(d.path().join("a.rb")).unwrap();
            if Command::new("ruby").arg("--version").output().is_ok() {
                same_runtime(source.as_bytes(), &after);
            }
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                1,
            );
        }
    }
}

#[test]
fn return_values_assignment_binding_side_effect_order_and_ensure_match_ruby() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let d = project();
    for source in [
        "log=[]; p [false,true].map do |x|\n if (log << :condition; x)\n  log << :first\n  log << :second\n  42\n end\nend\np log\n",
        "log=[]; p [false,true].map do |x|\n unless (log << :condition; x)\n  log << :first\n  log << :second\n  42\n end\nend\np log\n",
        "log=[]; p [false,true].map do |x|\n if (v = x)\n  log << v\n  v = :new\n  v\n end\nend\np log\n",
        "log=[]; p [false,true].map do |x|\n if /(?<v>.)/ =~ (x ? 'a' : '')\n  log << v\n  log << :work\n  v\n end\nend\np log\n",
        "log=[]; p [false,true].map do |x|\n if x\n  begin\n   log << :work\n   raise 'failure'\n  rescue\n   log << :rescue\n   42\n  ensure\n   log << :ensure\n  end\n end\nend\np log\n",
        "log=[]; p [false,true].map do |x|\n if x\n  log << :work\n  next 7\n  log << :unreachable\n end\nend\np log\n",
        "log=[]; n=0\nwhile n < 4\n n+=1\n if n.odd?\n  log << n\n  log << :work\n  break 7 if n==3\n end\nend\np log\n",
        "log=[]\nfor x in [false,true]\n unless x\n  log << :work\n  value=42\n  log << value\n end\nend\np [log,value,x]\n",
        "log=[]; p [false,true].map do |x|\n if x\n  text = <<~TEXT\n    hello\n    world\n  TEXT\n  log << text\n  __LINE__\n end\nend\np [log,__LINE__]\n",
        "def each_custom\n p [yield(false),yield(true)]\nend\neach_custom do |x|\n if x\n  a=1\n  b=2\n  a+b\n end\nend\n",
    ] {
        // Use an assignment so do/end binds to map, not an outer print call.
        let source = if source.contains("p [false,true].map do") {
            source
                .replace("p [false,true].map do", "result = [false,true].map do")
                .replace("end\np log", "end\np result\np log")
                .replace("end\np [log,__LINE__]", "end\np [result,log,__LINE__]")
        } else {
            source.to_owned()
        };
        fs::write(d.path().join("a.rb"), &source).unwrap();
        let o = cli(d.path(), &["--only", COP, "--fix", "a.rb"]);
        assert_eq!(o.status.code(), Some(0), "{o:?}");
        let after = fs::read(d.path().join("a.rb")).unwrap();
        assert_ne!(after, source.as_bytes(), "must really correct: {source}");
        same_runtime(source.as_bytes(), &after);
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
