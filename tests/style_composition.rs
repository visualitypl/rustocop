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
