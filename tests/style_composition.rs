//! controlled fixtures only. No application or public-corpus code executes.
use std::{fs, path::Path, process::Command};
fn observe(ruby: &str, path: &Path) -> (Option<i32>, Vec<u8>, Vec<u8>) {
    let observation = observe_with_warnings(ruby, path);
    assert!(observation.2.is_empty(), "{ruby}: {observation:?}");
    observation
}
fn observe_with_warnings(ruby: &str, path: &Path) -> (Option<i32>, Vec<u8>, Vec<u8>) {
    let o = Command::new(ruby)
        .arg("--disable-gems")
        .arg(path)
        .output()
        .unwrap();
    assert!(o.status.success(), "{ruby}: {o:?}");
    assert!(!o.stdout.is_empty());
    (o.status.code(), o.stdout, o.stderr)
}
#[test]
fn regexp_value_observers_preserve_runtime_and_keep_findings() {
    let recorded: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "fixtures/style_correction_counterexamples.json"
    ))
    .unwrap();
    let mut cases: Vec<(String, String)> = recorded
        .iter()
        .map(|c| {
            (
                c["selection"].as_str().unwrap().into(),
                c["before"].as_str().unwrap().into(),
            )
        })
        .collect();
    for (cop, pattern, simplified) in [
        ("Style/RedundantRegexpCharacterClass", "/[a]/", "/a/"),
        ("Style/RedundantRegexpEscape", r"/a\-b/", "/a-b/"),
    ] {
        for source in [
            format!("p {pattern}.source\n"),
            format!("p({pattern} == {simplified})\n"),
            format!("p({{{pattern} => 1}}[{simplified}])\n"),
            format!("r = {pattern}\np r\n"),
            format!("def value\n  {pattern}\nend\np value\n"),
            format!("p [1].map {{ {pattern} }}\n"),
            format!("p /#{{{pattern}}}/\n"),
            // A match exposes the original regexp through MatchData#regexp.
            format!("$_ = 'a-b'\np :yes if {pattern}\np $~.regexp\n"),
            format!(
                "class String\n  def match?(r)\n    r.source\n  end\nend\np 'a'.match?({pattern})\n"
            ),
        ] {
            for selection in [cop, "Style,Layout"] {
                cases.push((selection.into(), source.clone()));
            }
        }
    }
    let rubies: Vec<_> = std::iter::once("ruby".to_owned())
        .chain(crate::support::runtime::ruby())
        .collect();
    for (selection, source) in cases {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join(".git")).unwrap();
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        let path = d.path().join("x.rb");
        fs::write(&path, &source).unwrap();
        let before: Vec<_> = rubies
            .iter()
            .map(|r| observe_with_warnings(r, &path))
            .collect();
        let fixed = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", &selection, "--fix", "x.rb"])
            .output()
            .unwrap();
        assert_eq!(fixed.status.code(), Some(1), "{source}: {fixed:?}");
        assert!(fixed.stderr.is_empty(), "{fixed:?}");
        assert!(
            String::from_utf8_lossy(&fixed.stdout).contains("Style/RedundantRegexp"),
            "{source}: {fixed:?}"
        );
        for (ruby, expected) in rubies.iter().zip(before) {
            assert_eq!(
                observe_with_warnings(ruby, &path),
                expected,
                "{selection}: {source}"
            );
        }
    }
}

#[test]
fn closed_regexp_match_predicates_still_correct_and_preserve_runtime() {
    let rubies: Vec<_> = std::iter::once("ruby".to_owned())
        .chain(crate::support::runtime::ruby())
        .collect();
    for (cop, before, after) in [
        (
            "Style/RedundantRegexpCharacterClass",
            "p 'a'.match?(/[a]/)\n",
            "p 'a'.match?(/a/)\n",
        ),
        (
            "Style/RedundantRegexpEscape",
            "p 'a-b'.match?(/a\\-b/)\n",
            "p 'a-b'.match?(/a-b/)\n",
        ),
    ] {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join(".git")).unwrap();
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        let path = d.path().join("x.rb");
        fs::write(&path, before).unwrap();
        let observations: Vec<_> = rubies.iter().map(|r| observe(r, &path)).collect();
        let fixed = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", cop, "--fix", "x.rb"])
            .output()
            .unwrap();
        assert_eq!(fixed.status.code(), Some(0), "{fixed:?}");
        assert!(fixed.stderr.is_empty(), "{fixed:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
        for (ruby, expected) in rubies.iter().zip(observations) {
            assert_eq!(observe(ruby, &path), expected);
        }
    }
}
