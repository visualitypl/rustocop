//! real selectors, corrections, no-write and Ruby semantic witnesses.
use std::{fs, path::Path, process::Command};
const COP: &str = "Style/RedundantConditional";
const BAD: &str = "if 1 == 2\n  true\nelse\n  false\nend\n";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    fs::write(dir.path().join("lib/a.rb"), BAD).unwrap();
    dir
}
fn cli(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn count(output: &std::process::Output) -> usize {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| l.contains(COP))
        .count()
}
fn unchanged(root: &Path, source: &[u8], status: i32, offenses: usize) {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let output = cli(root, &["--only", COP, "--fix", "lib/a.rb"]);
    assert_eq!(output.status.code(), Some(status), "{output:?}");
    assert_eq!(count(&output), offenses, "{output:?}");
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn default_enablement_selectors_and_configuration() {
    let dir = project();
    for (config, selectors, expected) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Style", "--except", COP], 0),
        (String::new(), vec!["--only", COP, "--except", COP], 0),
        (format!("[cops.\"{COP}\"]\nenabled = false\n"), vec![], 0),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["--only", COP],
            1,
        ),
        (
            format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
            vec![],
            1,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = selectors;
        args.push("lib/a.rb");
        let output = cli(dir.path(), &args);
        assert_ne!(output.status.code(), Some(2), "{output:?}");
        assert_eq!(count(&output), expected, "{output:?}");
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            BAD
        );
        if expected > 0 {
            assert!(String::from_utf8_lossy(&output.stdout).contains(
                "lib/a.rb:1:1: C: This conditional expression can just be replaced by `1 == 2`."
            ));
        }
    }
    let registry = rustocop::cops::Registry::new();
    let cop = registry
        .cops()
        .iter()
        .find(|cop| cop.name() == COP)
        .unwrap();
    assert!(cop.default_enabled());
    assert!(cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none());
    assert!(cop.max_setting().is_none());
}

#[test]
fn safe_fixes_respect_config_relative_exclusions_and_directives() {
    let dir = project();
    for config in [
        "exclude = [\"lib/a.rb\"]\n".to_owned(),
        format!("[cops.\"{COP}\"]\nexclude = [\"lib/a.rb\"]\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let output = cli(&dir.path().join("lib"), &["--only", COP, "--fix", "a.rb"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(count(&output), 0);
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            BAD
        );
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "Style", "all"] {
            unchanged(
                dir.path(),
                format!("# {prefix}:disable {target}\n{BAD}").as_bytes(),
                0,
                0,
            );
        }
        unchanged(
            dir.path(),
            format!("# {prefix}:disable-next {COP}\n{BAD}").as_bytes(),
            0,
            0,
        );
        unchanged(
            dir.path(),
            BAD.replace("  true\n", &format!("  true # {prefix}:disable {COP}\n"))
                .as_bytes(),
            0,
            0,
        );
        let source = format!("# {prefix}:disable {COP}\n{BAD}# {prefix}:enable {COP}\n{BAD}");
        fs::write(dir.path().join("lib/a.rb"), source).unwrap();
        let output = cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            format!("# {prefix}:disable {COP}\n{BAD}# {prefix}:enable {COP}\n1 == 2\n")
        );
    }
}

#[test]
fn all_pinned_unknown_dispatch_corrections_are_byte_and_mtime_holds() {
    // RuboCop specs: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    let dir = project();
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/redundant_conditional_pinned.json")).unwrap();
    let mut corrections = 0;
    for c in cases.into_iter().filter(|c| c["correction"].is_string()) {
        unchanged(dir.path(), c["source"].as_str().unwrap().as_bytes(), 1, 1);
        corrections += 1;
    }
    assert_eq!(corrections, 8);
}

#[test]
fn plain_lint_clean_errors_and_withheld_fixes_never_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let output = cli(dir.path(), &["--only", COP, "lib/a.rb"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(count(&output), 1);
    assert_eq!(fs::read_to_string(&path).unwrap(), BAD);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    for (source, status, offenses) in [
        ("1 == 2\n", 0, 0),
        ("if 1 == 2\n", 2, 0),
        ("true if x == y\n", 0, 0),
        ("x == y ? true : false\n", 1, 1),
        ("if 1 == 2\n # intent\n true\nelse\n false\nend\n", 1, 1),
        (
            "begin\n if 1 == 2\n true\nelse\n false\nend\nrescue\n recover\nensure\n cleanup\nend\n",
            1,
            1,
        ),
        ("x = 0; true if (x = 1) == 2; p x\n", 0, 0),
        ("if 1 == 2; work; true; else; false; end\n", 0, 0),
        (
            "p DATA.pos\n1 == 2 ? true : false\n__END__\npayload\n",
            1,
            1,
        ),
        ("if 1 == 2\n true\nelse\n false\nend\np __LINE__\n", 1, 1),
        ("text = <<~TEXT\nhello\nTEXT\n1 == 2 ? true : false\n", 1, 1),
    ] {
        unchanged(dir.path(), source.as_bytes(), status, offenses);
    }
}

#[test]
fn bom_encoding_crlf_and_raw_data_survive_atomic_canonical_corrections() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let mut before = b"# encoding: ASCII-8BIT\r\n# raw \xff\r\n".to_vec();
    before.extend_from_slice(BAD.replace('\n', "\r\n").as_bytes());
    before.extend_from_slice(b"__END__\r\npayload\xff\r\n");
    let mut after =
        b"# encoding: ASCII-8BIT\r\n# raw \xff\r\n1 == 2\r\n__END__\r\npayload\xff\r\n".to_vec();
    for bom in [false, true] {
        if bom {
            before = format!(
                "\u{feff}# encoding: UTF-8\r\n{}__END__\r\npayload\r\n",
                BAD.replace('\n', "\r\n")
            )
            .into_bytes();
            after = "\u{feff}# encoding: UTF-8\r\n1 == 2\r\n__END__\r\npayload\r\n"
                .as_bytes()
                .to_vec();
        }
        fs::write(&path, &before).unwrap();
        let output = cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(fs::read(&path).unwrap(), after);
        unchanged(dir.path(), &after, 0, 0);
    }
}

fn rubies() -> Vec<String> {
    let mut result = Vec::new();
    if Command::new("ruby")
        .arg("-v")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        result.push("ruby".into());
    }
    if let Ok(ruby) = crate::support::runtime::ruby() {
        let output = Command::new(&ruby)
            .args(["-e", "print RUBY_VERSION"])
            .output()
            .expect("explicit Ruby 3.4 executable");
        assert!(output.status.success());
        assert!(
            output
                .stdout
                .starts_with(crate::support::runtime::version_prefix().as_bytes())
        );
        result.push(ruby);
    }
    result
}
fn observe(ruby: &str, source: &str) -> Vec<u8> {
    let output = Command::new(ruby).args(["-e", source]).output().unwrap();
    assert!(output.status.success(), "{ruby}: {source}: {output:?}");
    output.stdout
}

#[test]
fn controlled_ruby_witnesses_preserve_branch_values_effects_and_binding() {
    let dir = project();
    for (before, expected) in [
        (
            "result = 1 == 2 ? true : false\np result\n",
            "result = (1 == 2)\np result\n",
        ),
        (
            "result = 1 < 2 ? false : true\np result\n",
            "result = (!(1 < 2))\np result\n",
        ),
        (
            "result = if false\n true\nelsif 1 < 2\n true\nelse\n false\nend\np result\n",
            "result = if false\n true\nelse\n  1 < 2\nend\np result\n",
        ),
        (
            "result = 1 < 2 ? true : false\nx = 0; true if (x = 1) == 2\np [result, x]\n",
            "result = 1 < 2 ? true : false\nx = 0; true if (x = 1) == 2\np [result, x]\n",
        ),
        (
            "events = []; begin; result = 1 < 2 ? true : false; events << result; rescue; events << :rescue; ensure; events << :ensure; end; p events\n",
            "events = []; begin; result = 1 < 2 ? true : false; events << result; rescue; events << :rescue; ensure; events << :ensure; end; p events\n",
        ),
    ] {
        fs::write(dir.path().join("lib/a.rb"), before).unwrap();
        let output = cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
        assert_ne!(output.status.code(), Some(2), "{output:?}");
        let after = fs::read_to_string(dir.path().join("lib/a.rb")).unwrap();
        assert_eq!(after, expected, "{output:?}");
        for ruby in rubies() {
            assert_eq!(observe(&ruby, before), observe(&ruby, &after), "{ruby}");
        }
    }
}

#[test]
fn runtime_counterexamples_explain_unknown_dispatch_and_repeated_scope_withholds() {
    let dir = project();
    for (before, naive) in [
        (
            "class Strange; def ==(other); :truthy; end; end; x = Strange.new; p(x == nil ? true : false)",
            "class Strange; def ==(other); :truthy; end; end; x = Strange.new; p(x == nil)",
        ),
        (
            "class TrueClass; def !; :custom; end; end; p(1 == 1 ? false : true)",
            "class TrueClass; def !; :custom; end; end; p(!(1 == 1))",
        ),
        (
            "$calls = 0; def value; $calls += 1; 1; end; result = value == 1 ? true : false; p [result, $calls]",
            "",
        ),
        ("def f; return 1 == 2 ? true : false; end; p f", ""),
        ("p(2.times.map { |i| next 1 == 1 ? true : false })", ""),
    ] {
        unchanged(dir.path(), before.as_bytes(), 1, 1);
        for ruby in rubies() {
            let original = observe(&ruby, before);
            if !naive.is_empty() {
                assert_ne!(original, observe(&ruby, naive));
            }
            assert_eq!(
                original,
                observe(
                    &ruby,
                    &fs::read_to_string(dir.path().join("lib/a.rb")).unwrap()
                )
            );
        }
    }
}

#[test]
fn style_layout_composition_stabilizes_within_four_explicit_passes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "if  1 == 2\n  true\nelse\n  false\nend\n").unwrap();
    let selected = "Style/RedundantConditional,Style/OneLineConditional,Style/IfWithBooleanLiteralBranches,Style/IfUnlessModifier,Style/RedundantParentheses,Style/RedundantReturn,Style/Semicolon,Style/StringLiterals,Style/HashSyntax,Style/SignalException,Layout";
    let mut stable = false;
    for _ in 0..4 {
        let before = fs::read(&path).unwrap();
        let output = cli(dir.path(), &["--only", selected, "--fix", "lib/a.rb"]);
        assert_ne!(output.status.code(), Some(2), "{output:?}");
        if fs::read(&path).unwrap() == before {
            stable = true;
            break;
        }
    }
    assert!(stable);
    assert_eq!(fs::read_to_string(&path).unwrap(), "1 == 2\n");
}
