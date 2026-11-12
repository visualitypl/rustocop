//! Source-only pinned replay and real CLI/correction safety controls.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const ID: &str = "Lint/HashCompareByIdentity";
const NEW: &str = "Lint/HashNewWithKeywordArgumentsAsDefault";
const BOTH: &str = "Lint/HashCompareByIdentity,Lint/HashNewWithKeywordArgumentsAsDefault";
const SOURCE: &[u8] = b"hash[foo.object_id]\nHash.new(key: :value)\n";
const SAFE: &[u8] = b"[defined?(Hash.new(key: :value))]\n";
const AFTER: &[u8] = b"[defined?(Hash.new({key: :value}))]\n";
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/hash_identity_defaults.json")).unwrap()
}
fn bytes(row: &serde_json::Value, key: &str) -> Vec<u8> {
    serde_json::from_value(row[key].clone()).unwrap()
}
fn expected(row: &serde_json::Value, cop: &str) -> Vec<Finding> {
    serde_json::from_value(row["findings"][cop].clone()).unwrap()
}
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::create_dir(d.path().join("lib")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn count(out: &Output, cop: &str) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.ends_with(&format!("({cop})")))
        .count()
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], status: i32) -> Output {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(root, args);
    assert_eq!(out.status.code(), Some(status), "{out:?}\n{source:?}");
    assert_eq!(fs::read(&path).unwrap(), source);
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
#[test]
fn all_pinned_sources_and_original_bytes_match_pipeline() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[ID.into(), NEW.into()], &[]);
    assert_eq!(rows().len(), 776);
    let mut errors = Vec::new();
    for row in rows() {
        let source = bytes(&row, "bytes");
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&source),
            &active,
            &config,
        );
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{row}\n{report:?}"
        );
        for cop in [ID, NEW] {
            let mut found: Vec<_> = report
                .result
                .offenses
                .iter()
                .filter(|o| o.cop == cop)
                .map(|o| {
                    assert_eq!(o.severity, rustocop::offense::Severity::Error);
                    assert!(o.correction.is_none());
                    (o.line, o.col, o.end_line, o.message.clone())
                })
                .collect();
            found.sort();
            if found != expected(&row, cop) {
                errors.push(format!(
                    "{} {cop}: {found:?} != {:?}",
                    String::from_utf8_lossy(&source),
                    expected(&row, cop)
                ));
            }
        }
    }
    assert!(
        errors.is_empty(),
        "{} mismatches:\n{}",
        errors.len(),
        errors.join("\n")
    );
}
#[test]
fn every_fixture_matches_rebuilt_cli_no_write_or_exact_canonical_atomic_edit() {
    let d = project();
    let path = d.path().join("lib/a.rb");
    for row in rows() {
        let source = bytes(&row, "bytes");
        let after = bytes(&row, "project_after");
        let has_findings = [ID, NEW].iter().any(|cop| !expected(&row, cop).is_empty());
        let out = unchanged(
            d.path(),
            &source,
            &["--only", BOTH, "lib/a.rb"],
            i32::from(has_findings),
        );
        for cop in [ID, NEW] {
            let mut want: Vec<_> = expected(&row, cop)
                .into_iter()
                .map(|(l, c, _, m)| format!("lib/a.rb:{l}:{c}: E: {m} ({cop})"))
                .collect();
            want.sort();
            let mut got: Vec<_> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|l| l.ends_with(&format!("({cop})")))
                .map(str::to_owned)
                .collect();
            got.sort();
            assert_eq!(got, want, "{row}");
        }
        if source == after {
            unchanged(
                d.path(),
                &source,
                &["--only", BOTH, "--fix", "lib/a.rb"],
                i32::from(has_findings),
            );
        } else {
            let old = fs::metadata(&path).unwrap();
            let out = run(d.path(), &["--only", BOTH, "--fix", "lib/a.rb"]);
            assert_eq!(out.status.code(), Some(0), "{row}\n{out:?}");
            assert_eq!(fs::read(&path).unwrap(), after);
            assert_eq!(
                old.permissions(),
                fs::metadata(&path).unwrap().permissions()
            );
            let old = fs::metadata(&path).unwrap();
            assert!(
                run(d.path(), &["--only", BOTH, "--fix", "lib/a.rb"])
                    .status
                    .success()
            );
            assert_eq!(
                old.modified().unwrap(),
                fs::metadata(&path).unwrap().modified().unwrap()
            );
        }
    }
}
#[test]
fn live_metadata_defaults_selectors_strict_toml_and_path_provenance() {
    let registry = rustocop::cops::Registry::new();
    assert_eq!(registry.cops().len(), 613);
    for name in [ID, NEW] {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(cop.default_enabled());
        assert_eq!(cop.safe_fixes(), name == NEW);
        assert_eq!(cop.ruby_versions(), Default::default());
        assert!(cop.path_policy().is_none());
        assert!(cop.max_setting().is_none());
    }
    let d = project();
    for (config, selection, want) in [
        (String::new(), vec![], (1, 1)),
        (String::new(), vec!["--only", BOTH], (1, 1)),
        (
            String::new(),
            vec!["--only", "Lint", "--except", ID],
            (0, 1),
        ),
        (
            String::new(),
            vec!["--only", BOTH, "--except", "Lint"],
            (0, 0),
        ),
        ("[cops.Lint]\nenabled = false\n".into(), vec![], (0, 0)),
        (
            format!("[cops.Lint]\nenabled = false\n[cops.\"{ID}\"]\nenabled = true\n"),
            vec![],
            (1, 0),
        ),
        (
            format!("[cops.Lint]\nenabled = true\n[cops.\"{NEW}\"]\nenabled = false\n"),
            vec![],
            (1, 0),
        ),
        (
            format!("[cops.\"{NEW}\"]\nenabled = false\n"),
            vec!["--only", NEW],
            (0, 1),
        ),
        (
            "exclude = ['lib/a.rb']\n".into(),
            vec!["--only", BOTH],
            (0, 0),
        ),
        (
            format!("[cops.\"{ID}\"]\nexclude = ['lib/a.rb']\n"),
            vec!["--only", BOTH],
            (0, 1),
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        let mut args = selection;
        args.push("lib/a.rb");
        fs::write(d.path().join("lib/a.rb"), SOURCE).unwrap();
        let out = run(d.path(), &args);
        assert_ne!(out.status.code(), Some(2));
        assert_eq!((count(&out, ID), count(&out, NEW)), want, "{out:?}");
        if args.contains(&BOTH) && !args.contains(&"--except") {
            let out = run(&d.path().join("lib"), &["--only", BOTH, "a.rb"]);
            assert_eq!((count(&out, ID), count(&out, NEW)), want);
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(["--only", BOTH, "--stdin", "lib/a.rb"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(SOURCE).unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!((count(&out, ID), count(&out, NEW)), want);
        }
        assert_eq!(fs::read(d.path().join("lib/a.rb")).unwrap(), SOURCE);
    }
    for cop in [ID, NEW] {
        for key in [
            "max = 1",
            "include = ['**/*']",
            "enabled = 'true'",
            "TargetRubyVersion = 3.3",
            "Style = 'other'",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(d.path(), SAFE, &["--only", BOTH, "--fix", "lib/a.rb"], 2);
        }
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for path in ["Gemfile", "lib/.hidden.rb", "lib/a.jb", "lib/a_spec.rb"] {
        fs::write(d.path().join(path), SOURCE).unwrap();
        let out = run(d.path(), &["--only", BOTH, path]);
        assert_eq!((count(&out, ID), count(&out, NEW)), (1, 1));
    }
    assert_eq!(
        run(d.path(), &["--stdin", "lib/a.rb", "--fix"])
            .status
            .code(),
        Some(2)
    );
}
#[test]
fn both_directive_prefixes_multiline_next_push_pop_and_enables() {
    let d = project();
    for prefix in ["rubocop", "rustocop"] {
        for source in [
            format!("# {prefix}:disable Lint\nhash[foo.object_id]\nHash.new(key: :value)\n"),
            format!(
                "# {prefix}:disable-next {ID}\nhash[\nfoo.object_id\n]\n# {prefix}:disable-next {NEW}\nHash.new(\nkey: :value\n)\n"
            ),
            format!(
                "hash[\nfoo.object_id\n] # {prefix}:disable {ID}\nHash.new(\nkey: :value # {prefix}:disable {NEW}\n)\n"
            ),
            format!(
                "# {prefix}:push -{ID} -{NEW}\nhash[foo.object_id]\nHash.new(key: :value)\n# {prefix}:pop\n"
            ),
            format!("# {prefix}:disable-next {NEW}\n[defined?(Hash.new(key: :value))]\n"),
        ] {
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", BOTH, "--fix", "lib/a.rb"],
                0,
            );
        }
        let source = format!(
            "# {prefix}:disable Lint\nhash[foo.object_id]\nHash.new(key: :value)\n# {prefix}:push +{ID} +{NEW}\nhash[foo.object_id]\nHash.new(key: :value)\n# {prefix}:pop\nhash[foo.object_id]\nHash.new(key: :value)\n"
        );
        let out = unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", BOTH, "--fix", "lib/a.rb"],
            1,
        );
        assert_eq!((count(&out, ID), count(&out, NEW)), (1, 1));
    }
}
#[test]
fn source_enable_scopes_cannot_bypass_selectors_and_peer_edits_keep_detections() {
    let d = project();
    for prefix in ["rubocop", "rustocop"] {
        fs::write(
            d.path().join("rustocop.toml"),
            "[cops.Lint]\nenabled = false\n",
        )
        .unwrap();
        let source =
            format!("# {prefix}:enable-next {NEW}\nHash.new(key: :value)\nHash.new(key: :value)\n");
        let out = unchanged(d.path(), source.as_bytes(), &["lib/a.rb"], 1);
        assert_eq!(count(&out, NEW), 1);
        let out = unchanged(d.path(), source.as_bytes(), &["--only", ID, "lib/a.rb"], 0);
        assert_eq!(count(&out, NEW), 0);
        let out = unchanged(
            d.path(),
            source.as_bytes(),
            &["--except", NEW, "lib/a.rb"],
            0,
        );
        assert_eq!(count(&out, NEW), 0);
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    let source = b"hash[foo.object_id]\nHash.new(key: :value)\nx = [1,2]\n";
    let path = d.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let out = run(
        d.path(),
        &[
            "--only",
            "Lint/HashCompareByIdentity,Lint/HashNewWithKeywordArgumentsAsDefault,Layout/SpaceAfterComma",
            "--fix",
            "lib/a.rb",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    assert_eq!((count(&out, ID), count(&out, NEW)), (1, 1));
    assert_eq!(
        fs::read(&path).unwrap(),
        b"hash[foo.object_id]\nHash.new(key: :value)\nx = [1, 2]\n"
    );
}
#[test]
fn withheld_fixes_observers_and_invalid_syntax_preserve_bytes() {
    let d = project();
    for suffix in [
        "p __LINE__",
        "File.binread(__FILE__)",
        "ObjectSpace.each_object {}",
        "RubyVM::InstructionSequence.compile('1')",
        "DATA.read\n__END__\ndata\n",
        "x = <<~DOC\ntext\nDOC\n",
    ] {
        let source = format!("[defined?(Hash.new(key: :value))]\n{suffix}\n");
        let out = unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", BOTH, "--fix", "lib/a.rb"],
            1,
        );
        assert_eq!(count(&out, NEW), 1);
    }
    let out = unchanged(
        d.path(),
        b"Hash.new(key: :value)\ndef broken(\n",
        &["--only", BOTH, "--fix", "lib/a.rb"],
        2,
    );
    assert_eq!(count(&out, NEW), 0);
}
#[cfg(unix)]
#[test]
fn real_canonical_atomic_permissions_symlink_and_failure_controls() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let d = project();
    let path = d.path().join("lib/a.rb");
    fs::write(&path, SAFE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    symlink(&path, d.path().join("link.rb")).unwrap();
    let out = run(d.path(), &["--only", NEW, "--fix", "link.rb"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), AFTER);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        fs::symlink_metadata(d.path().join("link.rb"))
            .unwrap()
            .is_symlink()
    );
    fs::write(&path, SAFE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = run(d.path(), &["--only", NEW, "--fix", "link.rb"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), SAFE);
    assert_eq!(
        old.modified().unwrap(),
        fs::metadata(&path).unwrap().modified().unwrap()
    );
    assert_eq!(
        old.permissions(),
        fs::metadata(&path).unwrap().permissions()
    );
}
fn native(ruby: &str, source: &str) -> Output {
    Command::new(ruby)
        .args(["--parser=prism", "-w", "-e", source])
        .output()
        .unwrap()
}
#[test]
fn authored_native_value_binding_evaluation_control_flow_heap_and_observer_proofs() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let version = Command::new(&ruby).arg("--version").output().unwrap();
    assert!(
        String::from_utf8_lossy(&version.stdout).contains(crate::support::runtime::banner_prefix())
    );
    for receiver in ["Hash", "::Hash"] {
        for arg in ["key: :value", "capacity: 42, key: :value", ":key => :value"] {
            let a = format!("[defined?({receiver}.new({arg}))]");
            let b = format!("[defined?({receiver}.new({{{arg}}}))]");
            // Authored instrumentation is not itself an admitted edit context.
            for (a, b) in [
                (a.clone(), b.clone()),
                (
                    format!("p(begin; {a}; end); p TOPLEVEL_BINDING.local_variables"),
                    format!("p(begin; {b}; end); p TOPLEVEL_BINDING.local_variables"),
                ),
                (
                    format!(
                        "count = 0; trace = TracePoint.new(:c_call) {{ |t| count += 1 if t.method_id == :new && t.self == Hash }}; trace.enable; p({a}); trace.disable; p count"
                    ),
                    format!(
                        "count = 0; trace = TracePoint.new(:c_call) {{ |t| count += 1 if t.method_id == :new && t.self == Hash }}; trace.enable; p({b}); trace.disable; p count"
                    ),
                ),
            ] {
                let x = native(&ruby, &a);
                let y = native(&ruby, &b);
                assert!(x.status.success() && y.status.success(), "{x:?}\n{y:?}");
                assert_eq!(x.stdout, y.stdout);
                assert_eq!(x.stderr, y.stderr);
                assert!(x.stderr.is_empty());
                if a.starts_with("p(begin") {
                    assert_eq!(x.stdout, b"[\"method\"]\n[]\n");
                }
                if a.starts_with("count") {
                    assert_eq!(x.stdout, b"[\"method\"]\n0\n");
                }
            }
        }
    }
    // Real Ruby3.4 error/value tension: ordinary keyword calls must NOT be repaired.
    let a = native(
        &ruby,
        "begin; h = Hash.new(key: :value); p h[:missing]; rescue ArgumentError; puts 'keyword error'; end",
    );
    let b = native(
        &ruby,
        "begin; h = Hash.new({key: :value}); p h[:missing]; rescue ArgumentError; puts 'keyword error'; end",
    );
    assert!(a.status.success() && b.status.success());
    assert_eq!(a.stdout, b"keyword error\n");
    assert_eq!(b.stdout, b"{key: :value}\n");
    // The capacity carveout is semantic, not an ignored option spelling.
    let a = native(&ruby, "p Hash.new(capacity: 42)[:missing]");
    let b = native(&ruby, "p Hash.new({capacity: 42})[:missing]");
    assert_eq!(a.stdout, b"nil\n");
    assert_eq!(b.stdout, b"{capacity: 42}\n");
    // Reference arguments don't evaluate even a raising method; no general
    // argument admission is inferred from this independently authored control.
    let out = native(
        &ruby,
        "p [defined?(Hash.new(key: raise('not evaluated'))), defined?(Hash.new({key: raise('not evaluated')}))]",
    );
    assert!(out.status.success(), "{out:?}");
    assert_eq!(out.stdout, b"[\"method\", \"method\"]\n");
    // VM source objects genuinely differ: their observers are outside the closed proof.
    let out = native(
        &ruby,
        "a = RubyVM::InstructionSequence.compile('[defined?(Hash.new(key: :value))]'); b = RubyVM::InstructionSequence.compile('[defined?(Hash.new({key: :value}))]'); p(a.to_a == b.to_a)",
    );
    assert!(out.status.success());
    assert_eq!(out.stdout, b"false\n");
    // The unsafe identity recommendation is not general hash equivalence.
    let out = native(
        &ruby,
        "a = +'x'; b = +'x'; ordinary = {a => :value}; identity = {}.compare_by_identity; identity[a] = :value; p [ordinary[b], identity[b]]",
    );
    assert!(out.status.success());
    assert_eq!(out.stdout, b"[:value, nil]\n");
}
#[test]
fn native_ruby34_syntax_only_for_all_reference_inputs_and_canonical_outputs() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    for row in rows() {
        for source in [bytes(&row, "bytes"), bytes(&row, "project_after")] {
            let mut child = Command::new(&ruby)
                .args(["-c", "-W0"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&source).unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "{row}\n{out:?}");
        }
    }
}
