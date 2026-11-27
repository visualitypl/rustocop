//! Real CLI/pipeline and authored native correction-safety controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use crate::support::cli::{project_with_lib, run};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/RedundantRegexpQuantifiers";
const SAFE: &[u8] = b"[defined?(/(?:a+)+/)]\n";
const ORDINARY: &[u8] = b"value = /(?:a+)+/\n";
fn count(out: &Output) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.ends_with(&format!("({COP})")))
        .count()
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], code: Option<i32>) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    if let Some(code) = code {
        assert_eq!(out.status.code(), Some(code), "{out:?}");
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    let new = fs::metadata(&path).unwrap();
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(
        (old.modified().unwrap(), old.permissions()),
        (new.modified().unwrap(), new.permissions())
    );
    out
}
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/redundant_regexp_quantifiers.json")).unwrap()
}
#[test]
fn nested_overlapping_atomic_envelopes_retain_findings_and_converge() {
    let dir = project_with_lib();
    for (before, after) in [
        ("[defined?(/(?:(?:a?)+)+/)]\n", "[defined?(/(?:(?:a*))/)]\n"),
        ("[defined?(/(?:(?:a+)?)+/)]\n", "[defined?(/(?:(?:a*))/)]\n"),
        ("[defined?(/(?:(?:a+)+)?/)]\n", "[defined?(/(?:(?:a*))/)]\n"),
        (
            "[defined?(/(?:a+)+(?:b+)+/)]\n",
            "[defined?(/(?:a+)(?:b+)/)]\n",
        ),
    ] {
        unchanged(
            dir.path(),
            before.as_bytes(),
            &["--only", COP, "lib/a.rb"],
            Some(1),
        );
        for _ in 0..3 {
            let out = run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"]);
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            if out.status.success() {
                break;
            }
            assert!(count(&out) > 0, "deferred finding was lost: {out:?}");
        }
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            after
        );
        unchanged(
            dir.path(),
            after.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(0),
        );
    }
}
#[test]
fn all_reference_inputs_and_upstream_edits_are_native_syntax_only() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    for row in rows() {
        for key in ["source", "upstream_after"] {
            let out = syntax(&ruby, row[key].as_str().unwrap().as_bytes());
            assert_eq!(
                out.status.success(),
                row["invalid_syntax"] != true || row["fixed_native_valid"] == true,
                "{row}\n{key}\n{out:?}"
            );
        }
    }
}
fn syntax(ruby: &str, source: &[u8]) -> Output {
    let mut child = Command::new(ruby)
        .args(["--parser=prism", "-c", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}
#[test]
fn default_on_metadata_selectors_strict_toml_and_department_precedence() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 613);
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    let dir = project_with_lib();
    for (config, selectors, n) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Lint"], 1),
        (String::new(), vec!["--only", COP, "--except", COP], 0),
        (String::new(), vec!["--only", COP, "--except", "Lint"], 0),
        ("[cops.Lint]\nenabled = false\n".into(), vec![], 0),
        (
            format!("[cops.Lint]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
            vec![],
            1,
        ),
        (
            format!("[cops.Lint]\nenabled = true\n[cops.\"{COP}\"]\nenabled = false\n"),
            vec![],
            0,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["--only", COP],
            1,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut args = selectors;
        args.push("lib/a.rb");
        assert_eq!(count(&unchanged(dir.path(), ORDINARY, &args, None)), n);
    }
    for key in [
        "max = 1",
        "Include = ['**/*']",
        "Enabled = true",
        "enabled = 'true'",
        "target_ruby = 3.3",
        "SafeAutoCorrect = false",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            SAFE,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(2),
        );
    }
    for config in [
        "exclude = ['lib/**']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/**']\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        assert_eq!(
            count(&unchanged(
                dir.path(),
                SAFE,
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0)
            )),
            0
        );
    }
}
#[test]
fn supplied_paths_stdin_provenance_and_stdin_fix_rejection() {
    let dir = project_with_lib();
    for path in [
        "lib/a.rb",
        "a.jb",
        "view.erb",
        "db/schema.rb",
        "vendor/a.rb",
        ".hidden.rb",
    ] {
        let p = dir.path().join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, ORDINARY).unwrap();
        let n = usize::from(!path.starts_with("vendor/") && path != "db/schema.rb");
        assert_eq!(count(&run(dir.path(), &["--only", COP, path])), n, "{path}");
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", path])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(ORDINARY).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(count(&out), n, "{path}\n{out:?}");
        if n == 1 {
            assert!(
                String::from_utf8_lossy(&out.stdout).contains(&format!("{path}:1:14:")),
                "{out:?}"
            );
        }
    }
    assert_eq!(
        run(dir.path(), &["--only", COP, "--stdin", "lib/a.rb", "--fix"])
            .status
            .code(),
        Some(2)
    );
}
