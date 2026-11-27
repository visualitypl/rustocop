//! Actual CLI, source-only reference replay and authored native proofs.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use crate::support::cli::{project_with_lib, run};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/DuplicateMagicComment";
const SAFE: &[u8] = b"# frozen_string_literal: true\n# frozen_string_literal: true\n";
const HELD: &[u8] = b"# frozen_string_literal: true\n# frozen_string_literal: false\n";
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
        assert_eq!(
            out.status.code(),
            Some(code),
            "{}\n{args:?}\n{out:?}",
            String::from_utf8_lossy(source)
        );
    } else {
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), source, "{out:?}");
    let new = fs::metadata(&path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(old.ino(), new.ino());
    }
    out
}
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/duplicate_magic_comment.json")).unwrap()
}
#[test]
fn defaults_gates_selectors_strict_toml_and_department_precedence() {
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
        assert_eq!(count(&unchanged(dir.path(), HELD, &args, None)), n);
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
        unchanged(
            dir.path(),
            SAFE,
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(0),
        );
    }
}
#[test]
fn supplied_file_stdin_paths_built_in_exclusions_and_stdin_fix_rejection() {
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
        fs::write(&p, HELD).unwrap();
        let n = usize::from(!path.starts_with("vendor/") && path != "db/schema.rb");
        assert_eq!(count(&run(dir.path(), &["--only", COP, path])), n);
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", path])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(HELD).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(count(&out), n, "{out:?}");
        if n == 1 {
            assert!(
                String::from_utf8_lossy(&out.stdout).contains(&format!("{path}:2:1:")),
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

#[test]
fn reference_sources_and_upstream_outputs_native_syntax_only() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let mut sources: Vec<(Vec<u8>, bool)> = rows()
        .into_iter()
        .flat_map(|r| {
            ["source", "upstream_after"].map(|key| {
                (
                    r[key].as_str().unwrap().as_bytes().to_vec(),
                    r["native_loader_rejection"].is_string(),
                )
            })
        })
        .collect();
    let bytes: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/duplicate_magic_bytes.json")).unwrap();
    sources.extend(bytes.into_iter().map(|r| {
        (
            crate::support::fixture::value::<Vec<u8>>(&r["bytes"]),
            false,
        )
    }));
    for (source, loader_rejected) in sources {
        let mut child = Command::new(&ruby)
            .args(["--parser=prism", "-c", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&source).unwrap();
        let out = child.wait_with_output().unwrap();
        if loader_rejected {
            assert!(!out.status.success());
            assert!(
                String::from_utf8_lossy(&out.stderr).contains("invalid option -*"),
                "{out:?}"
            );
        } else {
            assert!(
                out.status.success(),
                "{}\n{out:?}",
                String::from_utf8_lossy(&source)
            );
        }
    }
}
