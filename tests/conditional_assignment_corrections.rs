//! Examples from RuboCop's two conditional_assignment_*_spec.rb files
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). Only fixed assign_to_condition
//! calls apply; both exact files are evaluated by the companion tool test.
use serde::Deserialize;
use std::{fs, path::Path, process::Command};
const COP: &str = "Style/ConditionalAssignment";
const BAD: &str = "if condition\n  value = 1\nelse\n  value = 2\nend\n";
const GOOD: &str = "value = if condition\n  1\nelse\n  2\nend\n";
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
    assert_eq!(
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| l.contains(COP))
            .count(),
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
    spec: String,
    spec_line: u32,
    source: String,
    positions: Vec<(u32, u32)>,
    messages: Vec<String>,
    correction: Option<String>,
    toml: String,
}
#[test]
fn every_applicable_pinned_call_and_correction() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/conditional_assignment_pinned.json")).unwrap();
    assert_eq!(cases.len(), 573);
    let d = project();
    let registry = rustocop::cops::Registry::new();
    let store =
        regex::Regex::new(r"^(?:[a-zA-Z_]\w*|@@?\w+|\$\w+|::\w+|self\.attribute) = ").unwrap();
    let (mut canonical, mut withheld, mut single_pass, mut fixed_alignment) = (0, 0, 0, 0);
    for c in cases {
        assert_eq!(c.spec, "conditional_assignment_assign_to_condition_spec.rb");
        let raw = toml::from_str::<rustocop::config::RawConfig>(&c.toml).unwrap();
        let config = rustocop::config::ResolvedConfig::resolve(raw, None, &registry).unwrap();
        let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
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
        assert_eq!(
            r.result
                .offenses
                .iter()
                .map(|o| (o.line, o.col))
                .collect::<Vec<_>>(),
            c.positions,
            "spec {}",
            c.spec_line
        );
        assert_eq!(
            r.result
                .offenses
                .iter()
                .map(|o| o.message.clone())
                .collect::<Vec<_>>(),
            c.messages,
            "spec {}",
            c.spec_line
        );
        fs::write(d.path().join("rustocop.toml"), &c.toml).unwrap();
        if let Some(mut after) = c.correction {
            if c.spec_line == 801 || !(store.is_match(&after) || matches!(c.spec_line, 2262 | 2278))
            {
                // 801: moving `foo =` makes the predicate's bare `foo` a local.
                // Other LHSes evaluate a receiver/index/getter or compound read
                // before the guard, or change ||= / &&= short circuiting.
                unchanged(
                    d.path(),
                    c.source.as_bytes(),
                    &["--only", COP, "--fix", "a.rb"],
                    c.positions.len(),
                );
                withheld += 1;
                continue;
            }
            if c.spec_line == 2278 {
                // Upstream requests a multipass result. Our contract is one
                // explicit pass, identical to its loop:false sibling at 2262.
                after="if outer\n  bar = 1\nelse\n  bar = if inner\n    2\n  else\n    3\n  end\nend\n".into();
                single_pass += 1;
            } else if after
                .lines()
                .last()
                .is_some_and(|l| l != "end" && l.trim() == "end")
                && c.source.ends_with("\nend\n")
            {
                // Detection is unchanged by upstream keyword end alignment,
                // but correction uses our pinned start_of_line policy.
                let at = after[..after.len() - 1].rfind('\n').unwrap() + 1;
                after.replace_range(at.., "end\n");
                fixed_alignment += 1;
            } else {
                canonical += 1;
            }
            correct(d.path(), c.source.as_bytes(), after.as_bytes(), COP);
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
        (canonical, withheld, single_pass, fixed_alignment),
        (47, 456, 1, 10)
    );
}
#[test]
fn actual_fix_selectors_config_and_no_write_contract() {
    let d = project();
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
    fs::write(d.path().join("a.rb"), BAD).unwrap();
    cli(d.path(), &["--fix", "a.rb"]);
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), GOOD);
}
#[test]
fn source_envelopes_literals_crlf_bom_line_and_end_payload_stay_exact() {
    let d = project();
    for (before, after) in [
        (
            "\u{feff}# encoding: UTF-8\r\nif condition\r\n  value = __LINE__ # first\r\nelse\r\n  # second\r\n  value = __LINE__\r\nend\r\np __LINE__\r\n__END__\r\nopaque\r\n",
            "\u{feff}# encoding: UTF-8\r\nvalue = if condition\r\n  __LINE__ # first\r\nelse\r\n  # second\r\n  __LINE__\r\nend\r\np __LINE__\r\n__END__\r\nopaque\r\n",
        ),
        (
            "if condition\n  value = <<~TEXT\n    literal # not a comment\n  TEXT\nelse\n  value = \"other\"\nend\n",
            "value = if condition\n  <<~TEXT\n    literal # not a comment\n  TEXT\nelse\n  \"other\"\nend\n",
        ),
        (
            "# encoding: US-ASCII\nif condition\n  value = \"first\\nsecond\"\nelse\n  value = %q{#{untouched}}\nend\n",
            "# encoding: US-ASCII\nvalue = if condition\n  \"first\\nsecond\"\nelse\n  %q{#{untouched}}\nend\n",
        ),
        (
            "if condition\n  value = \"first\nsecond\"\nelse\n  value = 2\nend\n",
            "value = if condition\n  \"first\nsecond\"\nelse\n  2\nend\n",
        ),
    ] {
        correct(d.path(), before.as_bytes(), after.as_bytes(), COP);
        unchanged(
            d.path(),
            after.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
    }
    let mut before = BAD.as_bytes().to_vec();
    before.extend_from_slice(b"__END__\n\xff\0raw\n");
    let mut after = GOOD.as_bytes().to_vec();
    after.extend_from_slice(b"__END__\n\xff\0raw\n");
    correct(d.path(), &before, &after, COP);
    for source in [
        &b"# encoding: ISO-8859-1\nif condition\n  value = '\xe9'\nelse\n  value = 'b'\nend\n"[..],
        &b"# encoding: ISO-8859-1\nif condition\n  \xe9 = 1\nelse\n  \xe9 = 2\nend\np \xe9\n"[..],
    ] {
        unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], 1);
    }
}
#[test]
fn data_and_own_source_observations_keep_the_offense_without_writes() {
    let d = project();
    for source in [
        format!("{BAD}p DATA.pos\n__END__\ndata\n"),
        format!("{BAD}p File.read(__FILE__)\n"),
    ] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
    }
}
fn ruby(source: &[u8], program: &std::ffi::OsStr) -> std::process::Output {
    use std::{io::Write, process::Stdio};
    let mut c = Command::new(program)
        .args(["--disable-gems", "-W0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(source).unwrap();
    c.wait_with_output().unwrap()
}
fn same_runtime(before: &[u8], after: &[u8]) {
    let mut programs = vec![std::ffi::OsString::from("ruby")];
    if let Some(native) = std::env::var_os("RUSTOCOP_RUBY34") {
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
fn runtime_return_values_guard_count_branch_effects_and_rescue_ensure_are_preserved() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let d = project();
    for header in [
        "if (log << :guard; x)",
        "unless (log << :guard; x)",
        "case (log << :guard; x)\n when true",
        "case (log << :guard; x)\n in true",
    ] {
        let before = format!(
            "def f(x,log)\n {header}\n  value = begin\n   log << :first\n   raise 'oops'\n  rescue\n   log << :rescue\n   __LINE__\n  ensure\n   log << :ensure\n  end\n else\n  value = (log << :second; 2)\n end\n [value,log]\nend\np [f(false,[]), f(true,[])]\n"
        );
        let after = before
            .replacen(&format!(" {header}"), &format!(" value = {header}"), 1)
            .replace("  value = ", "  ");
        correct(d.path(), before.as_bytes(), after.as_bytes(), COP);
        same_runtime(before.as_bytes(), after.as_bytes());
    }
    for before in [
        "def f(x,log)\n x ? value = (log << :first; 1) : value = (log << :second; 2)\nend\nlog=[]; p [f(false,log),f(true,log),log]\n",
        "def attribute=(v); @value=v; :ignored; end\ndef f(x)\n if x\n  self.attribute = 1\n else\n  self.attribute = 2\n end\nend\np [f(true),@value,f(false),@value]\n",
        "def f(x)\n begin\n  if x\n   value = 1\n  else\n   value = 2\n  end\n rescue\n  :rescue\n ensure\n  puts :ensure\n end\nend\np [f(false),f(true)]\n",
        "def f(x)\n if x\n  value = <<~TEXT\n    literal #{__LINE__}\n      keep indentation\n  TEXT\n else\n  value = 2, 3\n end\nend\np [f(false),f(true),__LINE__]\n",
    ] {
        fs::write(d.path().join("a.rb"), before).unwrap();
        cli(d.path(), &["--only", COP, "--fix", "a.rb"]);
        let after = fs::read(d.path().join("a.rb")).unwrap();
        assert_ne!(after, before.as_bytes());
        same_runtime(before.as_bytes(), &after);
    }
}
#[test]
fn unsafe_upstream_extractions_have_controlled_runtime_counterexamples() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let d = project();
    for (before, unsafe_after) in [
        (
            "def value; true; end\nif value; value = 1; else; value = 2; end\np value\n",
            "def value; true; end\nvalue = if value; 1; else; 2; end\np value\n",
        ),
        (
            "value=1\nif (value=10; true); value += 1; else; value += 2; end\np value\n",
            "value=1\nvalue += if (value=10; true); 1; else; 2; end\np value\n",
        ),
        (
            "value=1\nif (puts :guard; true); value ||= 2; else; value ||= 3; end\np value\n",
            "value=1\nvalue ||= if (puts :guard; true); 2; else; 3; end\np value\n",
        ),
        (
            "value=false\nif (puts :guard; true); value &&= 2; else; value &&= 3; end\np value\n",
            "value=false\nvalue &&= if (puts :guard; true); 2; else; 3; end\np value\n",
        ),
        (
            "log=[]\nif (log << :guard; log=[]; true); log << 1; else; log << 2; end\np log\n",
            "log=[]\nlog << if (log << :guard; log=[]; true); 1; else; 2; end\np log\n",
        ),
    ] {
        // Each original is detected yet untouched. These counterpart rewrites
        // demonstrate why reparsing alone is insufficient.
        unchanged(
            d.path(),
            before.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        let mut programs = vec![std::ffi::OsString::from("ruby")];
        if let Some(native) = std::env::var_os("RUSTOCOP_RUBY34") {
            programs.push(native);
        }
        for program in programs {
            let old = ruby(before.as_bytes(), &program);
            let new = ruby(unsafe_after.as_bytes(), &program);
            assert!(
                old.status.success() && new.status.success(),
                "{program:?}: {old:?} {new:?}"
            );
            assert_ne!(old.stdout, new.stdout, "{program:?}");
        }
    }
}
#[test]
fn predicate_block_declarations_remain_independent_and_do_not_write() {
    let d = project();
    for guard in [
        "[1].any? { value = 42; true }",
        "[1].any? { [1].any? { value = 42; true } }",
        "-> { value = 42; true }.call",
        "[1].any? { value, other = 42, 0; true }",
        "[1].any? { 42 => value; true }",
        "[1].any? { /(?<value>.)/ =~ 'x'; true }",
    ] {
        let before = format!("if {guard}\n  value = value\nelse\n  value = 0\nend\np value\n");
        unchanged(
            d.path(),
            before.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
        if Command::new("ruby").arg("--version").output().is_ok() {
            same_runtime(before.as_bytes(), &fs::read(d.path().join("a.rb")).unwrap());
        }
    }
    // A parseable extraction changes the block's binding, not just its value.
    let before =
        "if [1].any? { value = 42; true }\n  value = value\nelse\n  value = 0\nend\np value\n";
    let unsafe_after =
        "value = if [1].any? { value = 42; true }\n  value\nelse\n  0\nend\np value\n";
    let mut programs = vec![std::ffi::OsString::from("ruby")];
    if let Some(native) = std::env::var_os("RUSTOCOP_RUBY34") {
        programs.push(native);
    }
    for program in programs {
        if Command::new(&program).arg("--version").output().is_err() {
            continue;
        }
        let old = ruby(before.as_bytes(), &program);
        let new = ruby(unsafe_after.as_bytes(), &program);
        assert!(
            old.status.success() && new.status.success(),
            "{old:?} {new:?}"
        );
        assert_eq!(old.stdout, b"nil\n", "{program:?}");
        assert_eq!(new.stdout, b"42\n", "{program:?}");
    }
}
#[test]
fn unchanged_block_bindings_still_allow_extraction() {
    let d = project();
    for (prefix, guard) in [
        ("value = nil\n", "[1].any? { value = 42; true }"),
        ("", "[1].any? { |value| value = 42; true }"),
        ("", "[1].any? { |item; value| value = 42; true }"),
        ("", "[1].any? { other = 42; true }"),
    ] {
        let before =
            format!("{prefix}if {guard}\n  value = value\nelse\n  value = 0\nend\np value\n");
        let after = format!("{prefix}value = if {guard}\n  value\nelse\n  0\nend\np value\n");
        correct(d.path(), before.as_bytes(), after.as_bytes(), COP);
        if Command::new("ruby").arg("--version").output().is_ok() {
            same_runtime(before.as_bytes(), after.as_bytes());
        }
    }
}
#[test]
fn upstream_incompatibility_withholds_only_conditional_assignment_edits() {
    let d = project();
    let before = "if condition\n  value = foo 1\nelse\n  value = foo 2\nend\n";
    let peer = "Style/MethodCallWithArgsParentheses";
    for selectors in [format!("{COP},{peer}"), format!("{peer},{COP}")] {
        correct(
            d.path(),
            before.as_bytes(),
            b"if condition\n  value = foo(1)\nelse\n  value = foo(2)\nend\n",
            &selectors,
        );
        unchanged(
            d.path(),
            b"if condition\n  value = foo(1)\nelse\n  value = foo(2)\nend\n",
            &["--only", &selectors, "--fix", "a.rb"],
            1,
        );
    }
    correct(
        d.path(),
        before.as_bytes(),
        b"value = if condition\n  foo 1\nelse\n  foo 2\nend\n",
        COP,
    );
}
#[test]
fn atomic_overlap_uses_original_snapshot_and_settles_on_the_next_explicit_pass() {
    let d = project();
    let before = "if condition\n  value = 'one'  \nelse\n  value = 'two'\nend\n";
    let first = "value = if condition\n  'one'  \nelse\n  'two'\nend\n";
    let second = "value = if condition\n  \"one\"\nelse\n  \"two\"\nend\n";
    for selectors in [
        format!("{COP},Style/StringLiterals,Layout/TrailingWhitespace"),
        format!("Layout/TrailingWhitespace,Style/StringLiterals,{COP}"),
    ] {
        correct(d.path(), before.as_bytes(), first.as_bytes(), &selectors);
        correct(d.path(), first.as_bytes(), second.as_bytes(), &selectors);
        unchanged(
            d.path(),
            second.as_bytes(),
            &["--only", &selectors, "--fix", "a.rb"],
            0,
        );
    }
}
#[test]
fn clean_uncorrectable_and_parse_error_files_keep_bytes_and_mtime() {
    let d = project();
    for (source, count) in [
        (GOOD, 0),
        ("value = 1 if value.nil?\n", 0),
        (
            "if condition\n  work\n  value = 1\nelse\n  value = 2\nend\n",
            0,
        ),
        ("if condition; value += 1; else; value += 2; end\n", 1),
    ] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            count,
        );
    }
    let file = d.path().join("a.rb");
    let source = "if condition\n";
    fs::write(&file, source).unwrap();
    let time = fs::metadata(&file).unwrap().modified().unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", COP, "--fix", "a.rb"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(fs::read_to_string(&file).unwrap(), source);
    assert_eq!(fs::metadata(file).unwrap().modified().unwrap(), time);
}
#[test]
fn nested_and_style_layout_overlaps_are_deterministic_and_finitely_stable() {
    let d = project();
    let before =
        "if outer\n  bar = 1\nelse\n  if inner\n    bar = 2\n  else\n    bar = 3\n  end\nend\n";
    let first = "if outer\n  bar = 1\nelse\n  bar = if inner\n    2\n  else\n    3\n  end\nend\n";
    let second = "bar = if outer\n  1\nelse\n  if inner\n    2\n  else\n    3\n  end\nend\n";
    correct(d.path(), before.as_bytes(), first.as_bytes(), COP);
    correct(d.path(), first.as_bytes(), second.as_bytes(), COP);
    unchanged(
        d.path(),
        second.as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        0,
    );
    let before = "def f(condition)\n if condition\n  value = { :key => 'one' }  \n else\n  value = { :key => 'two' }\n end\n return (value)\nend\np [f(false), f(true)]\n";
    let selectors = format!(
        "{COP},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout"
    );
    fs::write(d.path().join("a.rb"), before).unwrap();
    let mut snapshots = vec![before.as_bytes().to_vec()];
    let mut settled = false;
    for _ in 0..5 {
        cli(d.path(), &["--only", &selectors, "--fix", "a.rb"]);
        let after = fs::read(d.path().join("a.rb")).unwrap();
        if after == *snapshots.last().unwrap() {
            settled = true;
            break;
        }
        assert!(!snapshots.contains(&after), "correction cycle");
        if Command::new("ruby").arg("--version").output().is_ok() {
            same_runtime(before.as_bytes(), &after);
        }
        snapshots.push(after);
    }
    assert!(
        settled,
        "curated fixture must settle in at most four changing passes"
    );
    let last = snapshots.last().unwrap();
    assert!(String::from_utf8_lossy(last).contains("value = if condition"));
    unchanged(d.path(), last, &["--only", &selectors, "--fix", "a.rb"], 0);
}
#[test]
fn directives_and_exclusions_protect_whole_corrections() {
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
    for prefix in ["rustocop", "rubocop"] {
        unchanged(
            d.path(),
            format!("# {prefix}:disable {COP}\n{BAD}").as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
        let disabled = format!("# {prefix}:disable {COP}\n{BAD}# {prefix}:enable {COP}\n");
        correct(
            d.path(),
            format!("{disabled}{BAD}").as_bytes(),
            format!("{disabled}{GOOD}").as_bytes(),
            COP,
        );
        let source = BAD.replace(
            "value = 1",
            &format!("value = 1 # {prefix}:disable-next Style/StringLiterals"),
        );
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
    }
}
