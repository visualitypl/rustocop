//! Source-only RuboCop MIT spec replay; authored native controls separate.
#[macro_use]
#[path = "support/parallel_rows.rs"]
mod parallel_rows;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Lint/RedundantTypeConversion";
const SOURCE: &[u8] = b"alpha.to_s.to_s\n";
const SAFE: &[u8] = b"if false\n  alpha.to_s.to_s\nend\n";
const AFTER: &[u8] = b"if false\n  alpha.to_s\nend\n";
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/redundant_type_conversion.json")).unwrap()
}
fn bytes(row: &serde_json::Value, key: &str) -> Vec<u8> {
    serde_json::from_value(row[key].clone()).unwrap()
}
fn expected(row: &serde_json::Value) -> Vec<Finding> {
    serde_json::from_value(row["findings"].clone()).unwrap()
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
fn count(out: &Output) -> usize {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.ends_with(&format!("({COP})")))
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
fn all_original_bytes_match_pipeline_cli_and_guarded_canonical_outputs() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active = rustocop::cops::select_active(&registry, &config, &[COP.into()], &[]);
    let rows = rows();
    assert_eq!(rows.iter().filter(|r| r["origin"] == "spec").count(), 613);
    assert_eq!(
        rows.iter().filter(|r| r["origin"] == "canonical").count(),
        320
    );
    parallel_rows!(for row in rows => {
        let d = project();
        let path = d.path().join("lib/a.rb");
        let source = bytes(&row, "bytes");
        let after = if row.get("project_after").is_some() {
            bytes(&row, "project_after")
        } else {
            source.clone()
        };
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&source),
            &active,
            &config,
        );
        if row.get("parser_rejection").is_some() {
            assert!(
                !report.errors.is_empty() && report.result.offenses.is_empty(),
                "{row} {report:?}"
            );
            unchanged(d.path(), &source, &["--only", COP, "lib/a.rb"], 2);
            unchanged(d.path(), &source, &["--only", COP, "--fix", "lib/a.rb"], 2);
            continue;
        }
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{row}\n{report:?}"
        );
        let mut got: Vec<_> = report
            .result
            .offenses
            .iter()
            .map(|o| {
                assert!(o.correction.is_none());
                assert_eq!(o.severity, rustocop::offense::Severity::Error);
                (o.line, o.col, o.end_line, o.message.clone())
            })
            .collect();
        got.sort();
        assert_eq!(got, expected(&row), "{row}");
        let status = i32::from(!got.is_empty());
        let out = unchanged(d.path(), &source, &["--only", COP, "lib/a.rb"], status);
        let mut got: Vec<_> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.ends_with(&format!("({COP})")))
            .map(str::to_owned)
            .collect();
        got.sort();
        let mut want: Vec<_> = expected(&row)
            .into_iter()
            .map(|(l, c, _, m)| format!("lib/a.rb:{l}:{c}: E: {m} ({COP})"))
            .collect();
        want.sort();
        assert_eq!(got, want, "{row}");
        if source == after {
            unchanged(
                d.path(),
                &source,
                &["--only", COP, "--fix", "lib/a.rb"],
                status,
            );
        } else {
            let old = fs::metadata(&path).unwrap();
            let out = run(d.path(), &["--only", COP, "--fix", "lib/a.rb"]);
            assert!(out.status.success(), "{row}\n{out:?}");
            assert_eq!(fs::read(&path).unwrap(), after, "{row}");
            assert_eq!(
                old.permissions(),
                fs::metadata(&path).unwrap().permissions()
            );
            unchanged(d.path(), &after, &["--only", COP, "--fix", "lib/a.rb"], 0);
        }
    });
}
#[test]
fn defaults_selectors_toml_precedence_and_path_stdin_provenance() {
    let registry = rustocop::cops::Registry::new();
    let rule = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(rule.default_enabled() && rule.safe_fixes());
    assert_eq!(rule.ruby_versions(), Default::default());
    assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
    let d = project();
    for (config, select, want) in [
        (String::new(), vec![], 1),
        (String::new(), vec!["--only", COP], 1),
        (String::new(), vec!["--only", "Lint"], 1),
        (String::new(), vec!["--only", COP, "--except", "Lint"], 0),
        (String::new(), vec!["--only", "Lint/Debugger"], 0),
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
        ("exclude = ['lib/a.rb']\n".into(), vec!["--only", COP], 0),
        (
            format!("[cops.\"{COP}\"]\nexclude = ['lib/a.rb']\n"),
            vec!["--only", COP],
            0,
        ),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        fs::write(d.path().join("lib/a.rb"), SOURCE).unwrap();
        let mut args = select.clone();
        args.push("lib/a.rb");
        let out = run(d.path(), &args);
        assert_ne!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(count(&out), want);
        args.pop();
        args.push("a.rb");
        assert_eq!(count(&run(&d.path().join("lib"), &args)), want);
        let mut args = select;
        args.extend(["--stdin", "lib/a.rb"]);
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(SOURCE).unwrap();
        assert_eq!(count(&child.wait_with_output().unwrap()), want);
        assert_eq!(fs::read(d.path().join("lib/a.rb")).unwrap(), SOURCE);
    }
    for key in [
        "max = 1",
        "include = ['**/*']",
        "enabled = 'true'",
        "TargetRubyVersion = 3.3",
        "Style = 'other'",
    ] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{key}\n"),
        )
        .unwrap();
        unchanged(d.path(), SAFE, &["--only", COP, "--fix", "lib/a.rb"], 2);
    }
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    for path in ["Gemfile", "lib/.hidden.rb", "lib/a.jb", "lib/a_spec.rb"] {
        fs::write(d.path().join(path), SOURCE).unwrap();
        assert_eq!(count(&run(d.path(), &["--only", COP, path])), 1);
    }
    assert_eq!(
        run(d.path(), &["--stdin", "lib/a.rb", "--fix"])
            .status
            .code(),
        Some(2)
    );
}
#[test]
fn directives_multiline_next_push_and_selection_boundaries() {
    let d = project();
    let source = String::from_utf8_lossy(SOURCE);
    for prefix in ["rubocop", "rustocop"] {
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
        )
        .unwrap();
        for directive in [
            format!("enable {COP}"),
            format!("enable-next {COP}"),
            format!("next +{COP}"),
            format!("push +{COP}"),
        ] {
            let text = format!("# {prefix}:{directive}\n{source}");
            assert_eq!(
                count(&unchanged(d.path(), text.as_bytes(), &["lib/a.rb"], 1)),
                1
            );
            unchanged(
                d.path(),
                text.as_bytes(),
                &["--only", "Lint/Debugger", "lib/a.rb"],
                0,
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for text in [
            format!("# {prefix}:disable Lint\n{source}"),
            format!("# {prefix}:push -{COP}\n{source}# {prefix}:pop\n"),
            format!("# {prefix}:disable-next {COP}\n{source}"),
            format!("alpha.to_s.\n to_s # {prefix}:disable {COP}\n"),
            format!(
                "# {prefix}:disable {COP}\n{}",
                String::from_utf8_lossy(SAFE)
            ),
        ] {
            unchanged(
                d.path(),
                text.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                0,
            );
        }
        let text = format!(
            "# {prefix}:disable {COP}\n{source}# {prefix}:enable {COP}\n{source}# {prefix}:push -{COP}\n{source}# {prefix}:pop\n{source}"
        );
        assert_eq!(
            count(&unchanged(
                d.path(),
                text.as_bytes(),
                &["--only", COP, "lib/a.rb"],
                1
            )),
            2
        );
    }
}
#[test]
fn observers_delayed_scopes_invalid_parser_and_independent_composition() {
    let d = project();
    for suffix in [
        "p __LINE__",
        "File.binread(__FILE__)",
        "ObjectSpace.each_object {}",
        "RubyVM::InstructionSequence.compile('1')",
        "DATA.read\n__END__\ndata\n",
        "x = <<~DOC\ntext\nDOC\n",
        "TracePoint.new(:line) {}.enable",
        "GC.stat",
        "binding",
    ] {
        let text = format!("{}{suffix}\n", String::from_utf8_lossy(SAFE));
        assert_eq!(
            count(&unchanged(
                d.path(),
                text.as_bytes(),
                &["--only", COP, "--fix", "lib/a.rb"],
                1
            )),
            1
        );
    }
    for wrapper in [
        "def deferred\n%s\nend\n",
        "-> {\n%s\n}\n",
        "if true\n%s\nend\n",
    ] {
        let text = wrapper.replace("%s", &String::from_utf8_lossy(SAFE));
        unchanged(
            d.path(),
            text.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            1,
        );
    }
    let invalid = format!("{}def broken(\n", String::from_utf8_lossy(SAFE));
    assert_eq!(
        count(&unchanged(
            d.path(),
            invalid.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            2
        )),
        0
    );
    let mixed = format!("{}x = [1,2]\n", String::from_utf8_lossy(SAFE));
    fs::write(d.path().join("lib/a.rb"), &mixed).unwrap();
    let out = run(
        d.path(),
        &[
            "--only",
            "Lint/RedundantTypeConversion,Layout/SpaceAfterComma",
            "--fix",
            "lib/a.rb",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(count(&out), 1);
    assert_eq!(
        fs::read(d.path().join("lib/a.rb")).unwrap(),
        mixed.replace("1,2", "1, 2").as_bytes()
    );
    let composed = b"if false\n  alpha.to_s.to_s;  \nend\n";
    fs::write(d.path().join("lib/a.rb"), composed).unwrap();
    let selection = "Lint/RedundantTypeConversion,Style/Semicolon,Layout/TrailingWhitespace";
    let out = run(d.path(), &["--only", selection, "--fix", "lib/a.rb"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(fs::read(d.path().join("lib/a.rb")).unwrap(), AFTER);
    unchanged(
        d.path(),
        AFTER,
        &["--only", selection, "--fix", "lib/a.rb"],
        0,
    );
}
#[cfg(unix)]
#[test]
fn atomic_permissions_symlink_and_failure_controls() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let d = project();
    let path = d.path().join("lib/a.rb");
    fs::write(&path, SAFE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    symlink(&path, d.path().join("link.rb")).unwrap();
    let out = run(d.path(), &["--only", COP, "--fix", "link.rb"]);
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
    assert_eq!(
        run(d.path(), &["--only", COP, "--fix", "link.rb"])
            .status
            .code(),
        Some(2)
    );
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
fn authored_native_dead_branch_values_bindings_warnings_and_live_counterexamples() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    assert!(
        String::from_utf8_lossy(
            &Command::new(&ruby)
                .arg("--version")
                .output()
                .unwrap()
                .stdout
        )
        .contains("ruby 3.4.")
    );
    // Authored bounded controls only: no fixture, spec, documentation or corpus execution.
    for m in [
        "to_s", "to_sym", "to_i", "to_f", "to_d", "to_r", "to_c", "to_a", "to_h", "to_set",
    ] {
        for dot in [".", "&."] {
            let a = format!("if false\n alpha{dot}{m}{dot}{m}\nend\n");
            let b = format!("if false\n alpha{dot}{m}\nend\n");
            for (a, b) in [
                (a.clone(), b.clone()),
                (
                    format!(
                        "$events=[]; def alpha; $events << :receiver; raise; end; p(begin; {a}end); p $events; p TOPLEVEL_BINDING.local_variables"
                    ),
                    format!(
                        "$events=[]; def alpha; $events << :receiver; raise; end; p(begin; {b}end); p $events; p TOPLEVEL_BINDING.local_variables"
                    ),
                ),
            ] {
                let a = native(&ruby, &a);
                let b = native(&ruby, &b);
                assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
                assert_eq!(a.stdout, b.stdout);
                assert_eq!(a.stderr, b.stderr);
                assert!(a.stderr.is_empty());
            }
        }
    }
    let setup = "$events=[]; class String; def to_s; $events << :conversion; self; end; end; ";
    let a = native(&ruby, &format!("{setup}p 'authored'.to_s; p $events"));
    let b = native(&ruby, &format!("{setup}p 'authored'; p $events"));
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
    let setup = "class Authored; def to_s; raise 'authored'; end; end; x=Authored.new; def x.inspect; self; end; ";
    let a = native(
        &ruby,
        &format!("{setup}begin; x.inspect.to_s; rescue => e; puts e.message; end"),
    );
    let b = native(&ruby, &format!("{setup}x.inspect"));
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
    let setup = "$n=0; class String; def to_s; $n+=1; String.new(self); end; end; x='authored'; ";
    let a = native(&ruby, &format!("{setup}20.times {{ x.to_s.to_s }}; p $n"));
    let b = native(&ruby, &format!("{setup}20.times {{ x.to_s }}; p $n"));
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
    let setup = "$n=0; TracePoint.new(:c_call) { |t| $n+=1 if t.method_id == :to_s }.enable; x='authored'; ";
    let a = native(&ruby, &format!("{setup}x.to_s.to_s; p $n"));
    let b = native(&ruby, &format!("{setup}x.to_s; p $n"));
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
    let a = native(
        &ruby,
        "p RubyVM::InstructionSequence.compile('def authored(x); x.to_s.to_s; end').disasm",
    );
    let b = native(
        &ruby,
        "p RubyVM::InstructionSequence.compile('def authored(x); x.to_s; end').disasm",
    );
    assert!(a.status.success() && b.status.success());
    assert_ne!(a.stdout, b.stdout);
}
#[test]
fn native_syntax_only_for_every_reference_input_and_project_output() {
    let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
        return;
    };
    parallel_rows!(for row in rows() => {
        for source in [
            bytes(&row, "bytes"),
            if row.get("project_after").is_some() {
                bytes(&row, "project_after")
            } else {
                bytes(&row, "bytes")
            },
        ] {
            let mut child = Command::new(&ruby)
                .args(["-c", "-W0"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&source).unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(
                out.status.success(),
                row.get("parser_rejection").is_none(),
                "{row}\n{out:?}"
            );
        }
    });
}
