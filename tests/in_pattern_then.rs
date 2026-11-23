//! real CLI controls, canonical pinned edits and controlled semantic witnesses.
use std::{fs, path::Path, process::Command};
const IN: &str = "Style/InPatternThen";
const MULTI: &str = "Style/MultilineInPatternThen";
const BOTH: &str = "Style/InPatternThen,Style/MultilineInPatternThen";
const CASES: [(&str, &str, &str); 2] = [
    (
        IN,
        "case value\nin x; work\nend\n",
        "case value\nin x then work\nend\n",
    ),
    (
        MULTI,
        "case value\nin x then\n work\nend\n",
        "case value\nin x\n work\nend\n",
    ),
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
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
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
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
#[test]
fn default_pending_rules_selectors_config_and_explicit_ruby27_gates() {
    use rustocop::cops::{Registry, policy::RubyVersion};
    let d = project();
    let registry = Registry::new();
    for (cop, source, _) in CASES {
        let rule = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(rule.default_enabled() && rule.safe_fixes());
        assert_eq!(rule.ruby_versions().minimum, Some(RubyVersion(2, 7)));
        for (config, selectors, count) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Style", "--except", cop], 0),
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
            unchanged(d.path(), cop, source.as_bytes(), &args, count);
        }
    }
}
#[test]
fn exclusions_and_both_directive_prefixes_preserve_bytes_and_mtime() {
    let d = project();
    for (cop, source, _) in CASES {
        for config in [
            "exclude = [\"a.rb\"]\n".to_owned(),
            format!("[cops.\"{cop}\"]\nexclude = [\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                cop,
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rubocop", "rustocop"] {
            for target in [cop, "Style", "all"] {
                let source = format!("# {prefix}:disable {target}\n{source}");
                unchanged(
                    d.path(),
                    cop,
                    source.as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    0,
                );
            }
            let next = source.replacen('\n', &format!("\n# {prefix}:disable-next {cop}\n"), 1);
            unchanged(
                d.path(),
                cop,
                next.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
            let inline = source.replacen("work\n", &format!("work # {prefix}:disable {cop}\n"), 1);
            // Multiline reports at `then`, not at the body. Suppress its header.
            let inline = if cop == MULTI {
                source.replace("then\n", &format!("then # {prefix}:disable {cop}\n"))
            } else {
                inline
            };
            unchanged(
                d.path(),
                cop,
                inline.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
    }
}
#[test]
fn every_pinned_safe_correction_is_exact_and_single_pass_stable() {
    // RuboCop specs: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    let d = project();
    let mut count = 0;
    for (cop, fixture) in [
        (IN, include_str!("fixtures/in_pattern_then_pinned.json")),
        (
            MULTI,
            include_str!("fixtures/multiline_in_pattern_then_pinned.json"),
        ),
    ] {
        let cases: Vec<serde_json::Value> = serde_json::from_str(fixture).unwrap();
        for c in cases {
            let Some(after) = c["correction"].as_str() else {
                continue;
            };
            count += 1;
            fs::write(d.path().join("a.rb"), c["source"].as_str().unwrap()).unwrap();
            let o = cli(d.path(), &["--only", cop, "--fix", "a.rb"]);
            assert_eq!(o.status.code(), Some(0), "{c}: {o:?}");
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
            unchanged(
                d.path(),
                cop,
                after.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
    }
    assert_eq!(count, 10);
}
fn observe(root: &Path, source: &str, selected: &str, minimum_edits: usize) {
    fs::write(root.join("before.rb"), source).unwrap();
    fs::write(root.join("a.rb"), source).unwrap();
    let o = cli(root, &["--only", selected, "--fix", "a.rb"]);
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    let after = fs::read_to_string(root.join("a.rb")).unwrap();
    assert_ne!(source, after);
    assert!(
        String::from_utf8_lossy(&o.stdout).contains(&format!("{minimum_edits} correction")),
        "{o:?}"
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
            .arg(root.join("before.rb"))
            .output()
            .unwrap();
        let after = Command::new(&ruby).arg(root.join("a.rb")).output().unwrap();
        assert!(
            before.status.success() && after.status.success(),
            "{ruby}: {before:?} {after:?}"
        );
        assert_eq!(before.stdout, after.stdout, "{ruby}");
        assert_eq!(before.stderr, after.stderr, "{ruby}");
        assert!(!before.stdout.is_empty());
    }
}
#[test]
fn runtime_pins_guards_failed_matches_binding_and_deconstruction_effect_counts() {
    let d = project();
    let source = r#"$trace = []
class Subject
  def deconstruct; $trace << :deconstruct; [1, 2]; end
  def deconstruct_keys(keys); $trace << [:keys, keys]; {a: 3}; end
end
def guard(x); $trace << [:guard, x]; x > 0; end
def choose(value)
  pin = 2
  result = case value
  in [x, ^pin] if guard(x); $trace << :array; [x, pin]
  in {a: y} unless y.nil? then
    $trace << :hash
    y
  in 0|1 then
    :small
  else
    :other
  end
  [result, defined?(x), x, defined?(y), y]
end
[Subject.new, {a: 9}, [0, 2], [4, 5], 1].each { |v| p choose(v) }
p $trace
"#;
    observe(d.path(), source, BOTH, 3);
}
#[test]
fn runtime_control_flow_scopes_comments_crlf_bom_and_line_values() {
    let d = project();
    let source = r#"# coding: utf-8
$trace = []
def run(value)
  begin
    result = case value
    in [x] if x > 0; return [:early, x, __LINE__] # keep
    in [x] then
      raise "é"
    else
      :other
    end
    result
  rescue RuntimeError => e
    [:rescued, e.message, x, __LINE__]
  ensure
    $trace << :ensure
  end
end
p [run([1]), run([-1]), run(nil)]
p [1, 2, 3].map { |v| case v; in 2; next :skip; else v; end }
p [1, 2, 3].each { |v| case v; in 2; break :stop; else $trace << v; end }
p $trace
__END__
untouched payload
"#;
    for source in [
        source.to_owned(),
        format!("\u{feff}{}", source.replace('\n', "\r\n")),
    ] {
        observe(d.path(), &source, BOTH, 4);
        assert!(
            fs::read_to_string(d.path().join("a.rb"))
                .unwrap()
                .ends_with("__END__\nuntouched payload\n")
        );
    }
}
#[test]
fn continued_guard_then_removal_is_withheld_without_changing_runtime_binding() {
    let d = project();
    for source in [
        "p(case 1\nin x if true \\\n then -1\nend)\n",
        "p(case 1\nin x if\n true then -1\nend)\n",
        "p(case 1\nin x if true.. then\n -1\nend)\n",
    ] {
        unchanged(
            d.path(),
            MULTI,
            source.as_bytes(),
            &["--only", MULTI, "--fix", "a.rb"],
            1,
        );
        let Ok(o) = Command::new("ruby").arg(d.path().join("a.rb")).output() else {
            continue;
        };
        assert!(o.status.success(), "{o:?}");
        assert_eq!(o.stdout, b"-1\n");
    }
}

#[test]
fn style_layout_composition_is_deterministic_and_stabilizes() {
    let d = project();
    let source = "def choose(value)\n  case value\n  in {a: x}; return 'yes'\n  in [y] then\n    y; work\n  else\n    raise RuntimeError.new('no')\n  end\nend\n";
    let selected = "Style/InPatternThen,Style/MultilineInPatternThen,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout";
    fs::write(d.path().join("a.rb"), source).unwrap();
    fs::write(d.path().join("b.rb"), source).unwrap();
    let mut previous = source.as_bytes().to_vec();
    let mut stable = false;
    for _ in 0..4 {
        for path in ["a.rb", "b.rb"] {
            let o = cli(d.path(), &["--only", selected, "--fix", path]);
            assert_ne!(o.status.code(), Some(2), "{o:?}");
        }
        let bytes = fs::read(d.path().join("a.rb")).unwrap();
        assert_eq!(bytes, fs::read(d.path().join("b.rb")).unwrap());
        if bytes == previous {
            stable = true;
            break;
        }
        previous = bytes;
    }
    assert!(stable);
    let o = cli(d.path(), &["--only", BOTH, "a.rb"]);
    assert_eq!(o.status.code(), Some(0), "{o:?}");
}
