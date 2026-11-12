//! controlled fixtures only. No application or public-corpus code executes.
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    process::{Command, Output},
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    fixture: String,
    selection: String,
    passes: Vec<String>,
}
fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!("fixtures/style_composition/passes.json")).unwrap()
}
fn source(fixture: &str) -> &'static str {
    match fixture {
        "mixed" => include_str!("fixtures/style_composition/mixed.before.rb"),
        "binding" => include_str!("fixtures/style_composition/binding.before.rb"),
        "bytes" => include_str!("fixtures/style_composition/bytes.before.rb"),
        _ => panic!("unknown fixture: {fixture}"),
    }
}
fn cli(dir: &Path, selection: &str, fix: bool) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_rustocop"));
    c.current_dir(dir);
    if selection == "all-style-layout" {
        c.args(["--only", "Style,Layout"]);
    }
    if fix {
        c.arg("--fix");
    }
    let o = c.arg("x.rb").output().unwrap();
    assert!(matches!(o.status.code(), Some(0 | 1)), "{o:?}");
    assert!(o.stderr.is_empty(), "{o:?}");
    o
}
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
fn encoded(source: &str, fixture: &str, variant: u8) -> Vec<u8> {
    let mut s = source.to_owned();
    if variant == 2 {
        s = s.replace("coding: utf-8", "coding: ascii-8bit");
    }
    if variant != 0 {
        s = s.replace('\n', "\r\n");
    }
    if variant == 1 {
        s.insert(0, '\u{feff}');
    }
    let mut bytes = s.into_bytes();
    if variant == 2 && fixture == "bytes" {
        let pos = bytes.windows(2).position(|b| b == "é".as_bytes()).unwrap();
        bytes.splice(pos..pos + 2, [0xff]);
    }
    if fixture == "bytes" {
        bytes.extend_from_slice(b"\xff\x00raw\r\n");
    }
    bytes
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
        .chain(std::env::var("RUSTOCOP_RUBY34"))
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
        .chain(std::env::var("RUSTOCOP_RUBY34"))
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

#[test]
fn default_style_and_style_layout_and_explicit_opt_ins_have_exact_finite_pass_bounds() {
    let cases = cases();
    assert_eq!(cases.len(), 9);
    assert_eq!(
        cases
            .iter()
            .map(|c| (&c.fixture, &c.selection))
            .collect::<BTreeSet<_>>()
            .len(),
        9
    );
    for c in cases {
        // the default-on Layout/MultilineBlockLayout and
        // BlockEndNewline break the `mixed` lambda that Style/Semicolon
        // splits, and the indentation cops then settle its body: two more
        // passes, still ending in an exact stable zero-correction run.
        // the default-on Layout/SpaceInsideStringInterpolation
        // removes the `mixed` fixture's `#{ "message".upcase }` blanks in
        // the first Layout pass, within the same bounds.
        let bound = if c.fixture == "mixed" && c.selection != "default-style" {
            5
        } else {
            3
        };
        assert!(
            !c.passes.is_empty() && c.passes.len() <= bound,
            "{} / {}",
            c.fixture,
            c.selection
        );
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join(".git")).unwrap();
        let config = format!(
            "[cops.Lint]\nenabled = false\n[cops.Naming]\nenabled = false\n[cops.Security]\nenabled = false\n[cops.Layout]\nenabled = {}\n",
            c.selection != "default-style"
        );
        assert!(
            ["default-style", "default-style-layout", "all-style-layout"]
                .contains(&c.selection.as_str())
        );
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let path = d.path().join("x.rb");
        let rubies: Vec<_> = std::iter::once("ruby".to_owned())
            .chain(std::env::var("RUSTOCOP_RUBY34"))
            .collect();
        for variant in 0..=2 {
            // Binary-source witness only has an encoding header in this fixture.
            if variant == 2 && c.fixture != "bytes" {
                continue;
            }
            let before = encoded(source(&c.fixture), &c.fixture, variant);
            fs::write(&path, &before).unwrap();
            let observations: Vec<_> = rubies.iter().map(|r| observe(r, &path)).collect();
            let original = fs::metadata(&path).unwrap().modified().unwrap();
            let plain = cli(d.path(), &c.selection, false);
            assert_eq!(plain.status.code(), Some(1));
            assert_eq!(fs::read(&path).unwrap(), before);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), original);
            let mut seen = BTreeSet::new();
            let mut previous = before;
            for expected in &c.passes {
                assert!(
                    seen.insert(previous.clone()),
                    "cycle: {} / {}",
                    c.fixture,
                    c.selection
                );
                let fixed = cli(d.path(), &c.selection, true);
                let after = fs::read(&path).unwrap();
                assert_eq!(
                    after,
                    encoded(expected, &c.fixture, variant),
                    "{} / {} variant {variant}: {fixed:?}",
                    c.fixture,
                    c.selection
                );
                for (ruby, observation) in rubies.iter().zip(&observations) {
                    assert_eq!(
                        &observe(ruby, &path),
                        observation,
                        "{ruby}: {} / {}",
                        c.fixture,
                        c.selection
                    );
                }
                previous = after;
            }
            let time = fs::metadata(&path).unwrap().modified().unwrap();
            let stable = cli(d.path(), &c.selection, true);
            assert!(
                String::from_utf8_lossy(&stable.stdout).contains("0 corrections applied"),
                "{stable:?}"
            );
            assert_eq!(fs::read(&path).unwrap(), previous);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
        }
    }
}
