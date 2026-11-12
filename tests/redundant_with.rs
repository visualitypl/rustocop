//! RuboCop MIT spec replay is source-only; native runtime controls authored separately.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const INDEX: &str = "Lint/RedundantWithIndex";
const OBJECT: &str = "Lint/RedundantWithObject";
const BOTH: &str = "Lint/RedundantWithIndex,Lint/RedundantWithObject";
const SOURCE: &[u8] = b"ary.each_with_index { |v| v }\nary.each_with_object([]) { |v| v }\n";
const SAFE: &[u8] = b"if false\n  alpha.each_with_index { |v| v }\nend\n";
const AFTER: &[u8] = b"if false\n  alpha.each { |v| v }\nend\n";
type Finding = (u32, u32, u32, String);
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/redundant_with.json")).unwrap()
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
fn all_original_bytes_match_pipeline_cli_and_guarded_canonical_outputs() {
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active =
        rustocop::cops::select_active(&registry, &config, &[INDEX.into(), OBJECT.into()], &[]);
    let rows = rows();
    assert_eq!(rows.iter().filter(|r| r["origin"] == "spec").count(), 34);
    assert_eq!(
        rows.iter().filter(|r| r["origin"] == "canonical").count(),
        160
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
            unchanged(d.path(), &source, &["--only", BOTH, "lib/a.rb"], 2);
            unchanged(d.path(), &source, &["--only", BOTH, "--fix", "lib/a.rb"], 2);
            continue;
        }
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{row}\n{report:?}"
        );
        for cop in [INDEX, OBJECT] {
            let mut got: Vec<_> = report
                .result
                .offenses
                .iter()
                .filter(|o| o.cop == cop)
                .map(|o| {
                    assert!(o.correction.is_none());
                    assert_eq!(o.severity, rustocop::offense::Severity::Error);
                    (o.line, o.col, o.end_line, o.message.clone())
                })
                .collect();
            got.sort();
            assert_eq!(got, expected(&row, cop), "{row} {cop}");
        }
        let status = i32::from(!report.result.offenses.is_empty());
        let out = unchanged(d.path(), &source, &["--only", BOTH, "lib/a.rb"], status);
        for cop in [INDEX, OBJECT] {
            let mut got: Vec<_> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|l| l.ends_with(&format!("({cop})")))
                .map(str::to_owned)
                .collect();
            got.sort();
            let mut want: Vec<_> = expected(&row, cop)
                .into_iter()
                .map(|(l, c, _, m)| format!("lib/a.rb:{l}:{c}: E: {m} ({cop})"))
                .collect();
            want.sort();
            assert_eq!(got, want, "{row}");
        }
        if source == after {
            unchanged(
                d.path(),
                &source,
                &["--only", BOTH, "--fix", "lib/a.rb"],
                status,
            );
        } else {
            let old = fs::metadata(&path).unwrap();
            let out = run(d.path(), &["--only", BOTH, "--fix", "lib/a.rb"]);
            assert!(out.status.success(), "{row}\n{out:?}");
            assert_eq!(fs::read(&path).unwrap(), after, "{row}");
            assert_eq!(
                old.permissions(),
                fs::metadata(&path).unwrap().permissions()
            );
            unchanged(d.path(), &after, &["--only", BOTH, "--fix", "lib/a.rb"], 0);
        }
    });
}
#[test]
fn defaults_selectors_toml_precedence_and_path_stdin_provenance() {
    let registry = rustocop::cops::Registry::new();
    for cop in [INDEX, OBJECT] {
        let rule = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(rule.default_enabled());
        assert_eq!(rule.safe_fixes(), cop == INDEX);
        assert_eq!(rule.ruby_versions(), Default::default());
        assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
        let d = project();
        for (config, select, want) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Lint"], 1),
            (String::new(), vec!["--only", cop, "--except", "Lint"], 0),
            (String::new(), vec!["--only", "Lint/Debugger"], 0),
            ("[cops.Lint]\nenabled = false\n".into(), vec![], 0),
            (
                format!("[cops.Lint]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
                vec![],
                1,
            ),
            (
                format!("[cops.Lint]\nenabled = true\n[cops.\"{cop}\"]\nenabled = false\n"),
                vec![],
                0,
            ),
            (
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
                vec!["--only", cop],
                1,
            ),
            ("exclude = ['lib/a.rb']\n".into(), vec!["--only", cop], 0),
            (
                format!("[cops.\"{cop}\"]\nexclude = ['lib/a.rb']\n"),
                vec!["--only", cop],
                0,
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            fs::write(d.path().join("lib/a.rb"), SOURCE).unwrap();
            let mut args = select.clone();
            args.push("lib/a.rb");
            let out = run(d.path(), &args);
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            assert_eq!(count(&out, cop), want);
            args.pop();
            args.push("a.rb");
            assert_eq!(count(&run(&d.path().join("lib"), &args), cop), want);
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
            assert_eq!(count(&child.wait_with_output().unwrap(), cop), want);
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
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(d.path(), SAFE, &["--only", cop, "--fix", "lib/a.rb"], 2);
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for path in ["Gemfile", "lib/.hidden.rb", "lib/a.jb", "lib/a_spec.rb"] {
            fs::write(d.path().join(path), SOURCE).unwrap();
            assert_eq!(count(&run(d.path(), &["--only", cop, path]), cop), 1);
        }
        assert_eq!(
            run(d.path(), &["--stdin", "lib/a.rb", "--fix"])
                .status
                .code(),
            Some(2)
        );
    }
}
#[test]
fn directives_multiline_next_push_and_selection_boundaries() {
    let d = project();
    for (cop, source) in [
        (INDEX, "ary.each.with_index(\n 1\n) { |v| v }\n"),
        (OBJECT, "ary.each.with_object(\n []\n) { |v| v }\n"),
    ] {
        for prefix in ["rubocop", "rustocop"] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\nenabled = false\n"),
            )
            .unwrap();
            for directive in [
                format!("enable {cop}"),
                format!("enable-next {cop}"),
                format!("next +{cop}"),
                format!("push +{cop}"),
            ] {
                let text = format!("# {prefix}:{directive}\n{source}");
                assert_eq!(
                    count(&unchanged(d.path(), text.as_bytes(), &["lib/a.rb"], 1), cop),
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
                format!("# {prefix}:push -{cop}\n{source}# {prefix}:pop\n"),
                format!("# {prefix}:disable-next {cop}\n{source}"),
                source.replace("\n 1", &format!(" # {prefix}:disable {cop}\n 1")),
                source.replace("\n []", &format!(" # {prefix}:disable {cop}\n []")),
            ] {
                // The two replacements are identical to source when for the other cop.
                if text == source {
                    continue;
                }
                unchanged(
                    d.path(),
                    text.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    0,
                );
            }
            let text = format!(
                "# {prefix}:disable {cop}\n{source}# {prefix}:enable {cop}\n{source}# {prefix}:push -{cop}\n{source}# {prefix}:pop\n{source}"
            );
            assert_eq!(
                count(
                    &unchanged(d.path(), text.as_bytes(), &["--only", cop, "lib/a.rb"], 1),
                    cop
                ),
                2
            );
        }
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
            count(
                &unchanged(
                    d.path(),
                    text.as_bytes(),
                    &["--only", INDEX, "--fix", "lib/a.rb"],
                    1
                ),
                INDEX
            ),
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
            &["--only", INDEX, "--fix", "lib/a.rb"],
            1,
        );
    }
    let invalid = format!("{}def broken(\n", String::from_utf8_lossy(SAFE));
    assert_eq!(
        count(
            &unchanged(
                d.path(),
                invalid.as_bytes(),
                &["--only", BOTH, "--fix", "lib/a.rb"],
                2
            ),
            INDEX
        ),
        0
    );
    let mixed = format!("{}x = [1,2]\n", String::from_utf8_lossy(SAFE));
    fs::write(d.path().join("lib/a.rb"), &mixed).unwrap();
    let out = run(
        d.path(),
        &[
            "--only",
            "Lint/RedundantWithIndex,Layout/SpaceAfterComma",
            "--fix",
            "lib/a.rb",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(count(&out, INDEX), 1);
    assert_eq!(
        fs::read(d.path().join("lib/a.rb")).unwrap(),
        mixed.replace("1,2", "1, 2").as_bytes()
    );
    let composed = b"if false\n  alpha.each_with_index { |v| v };  \nend\n";
    fs::write(d.path().join("lib/a.rb"), composed).unwrap();
    let selection = "Lint/RedundantWithIndex,Style/Semicolon,Layout/TrailingWhitespace";
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
    let out = run(d.path(), &["--only", INDEX, "--fix", "link.rb"]);
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
        run(d.path(), &["--only", INDEX, "--fix", "link.rb"])
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
    let Ok(ruby) = crate::support::runtime::ruby() else {
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
        .contains(crate::support::runtime::banner_prefix())
    );
    // Only bounded authored programs execute. No reference/spec/corpus inputs.
    for call in [
        "alpha.each_with_index",
        "alpha&.each_with_index",
        "alpha.each.with_index",
        "alpha&.each&.with_index",
    ] {
        for block in [
            " { |v| v }",
            " { _1 }",
            " { it }",
            " { nil }",
            " { || nil }",
        ] {
            let before = format!("if false\n {call}{block}\nend\n");
            let after = before
                .replace("each_with_index", "each")
                .replace("&.with_index", "")
                .replace(".with_index", "");
            for (a, b) in [
                (before.clone(), after.clone()),
                (
                    format!(
                        "$events=[]; def alpha; $events << :receiver; raise; end; p(begin; {before}end); p $events; p TOPLEVEL_BINDING.local_variables"
                    ),
                    format!(
                        "$events=[]; def alpha; $events << :receiver; raise; end; p(begin; {after}end); p $events; p TOPLEVEL_BINDING.local_variables"
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
    // The supported inactive rewrite performs no receiver/block allocations.
    // Count after compilation/startup, with a fixed bounded authored loop.
    let a = native(
        &ruby,
        "GC.disable; n=GC.stat(:total_allocated_objects); 200.times { if false; alpha.each_with_index { |v| v }; end }; p(GC.stat(:total_allocated_objects)-n)",
    );
    let b = native(
        &ruby,
        "GC.disable; n=GC.stat(:total_allocated_objects); 200.times { if false; alpha.each { |v| v }; end }; p(GC.stat(:total_allocated_objects)-n)",
    );
    assert!(a.status.success() && b.status.success());
    assert_eq!(a.stdout, b.stdout);
    assert_eq!(a.stderr, b.stderr);
    // Ordinary Enumerable wrapping really allocates extra packed yield arrays.
    let setup = "class Authored; include Enumerable; def each; 2000.times { yield 4,5 }; end; end; x=Authored.new; GC.disable; n=GC.stat(:total_allocated_objects); ";
    let a = native(
        &ruby,
        &format!("{setup}x.each_with_index {{ |v| v }}; p(GC.stat(:total_allocated_objects)-n)"),
    );
    let b = native(
        &ruby,
        &format!("{setup}x.each {{ |v| v }}; p(GC.stat(:total_allocated_objects)-n)"),
    );
    assert!(a.status.success() && b.status.success());
    let allocated = |o: &Output| {
        String::from_utf8_lossy(&o.stdout)
            .trim()
            .parse::<u64>()
            .unwrap()
    };
    assert!(allocated(&a) > allocated(&b) + 1000, "{a:?}\n{b:?}");
    for (a, b) in [
        // Dispatch and values, including custom index methods.
        (
            "class Authored; def each_with_index; yield 1; :index; end; def each; yield 2; :each; end; end; p Authored.new.each_with_index { |v| p v }",
            "class Authored; def each_with_index; yield 1; :index; end; def each; yield 2; :each; end; end; p Authored.new.each { |v| p v }",
        ),
        // Enumerable index wrapping changes multi-value yield packing.
        (
            "class Authored; include Enumerable; def each; yield 4,5; end; end; Authored.new.each_with_index { |v| p v }",
            "class Authored; include Enumerable; def each; yield 4,5; end; end; Authored.new.each { |v| p v }",
        ),
        // Offset evaluation is removed by a with_index correction.
        (
            "$n=0; [1].each.with_index(($n+=1)) { |v| v }; p $n",
            "$n=0; [1].each { |v| v }; p $n",
        ),
        // Memo value and evaluation make object corrections unsafe.
        (
            "$n=0; p [1].each_with_object(($n+=1; [])) { |v| v }; p $n",
            "$n=0; p [1].each { |v| v }; p $n",
        ),
        // Block control flow and exception behavior cannot be inferred from names.
        (
            "class Authored; def each_with_index; raise 'authored'; end; def each; yield 1; end; end; begin; p Authored.new.each_with_index { |v| break v }; rescue => e; puts e.message; end",
            "class Authored; def each_with_index; raise 'authored'; end; def each; yield 1; end; end; begin; p Authored.new.each { |v| break v }; rescue => e; puts e.message; end",
        ),
        (
            "p RubyVM::InstructionSequence.compile('if false; a.each_with_index { |v| v }; end').disasm",
            "p RubyVM::InstructionSequence.compile('if false; a.each { |v| v }; end').disasm",
        ),
        (
            "$n=0; TracePoint.new(:c_call) { |t| $n+=1 if t.method_id == :each_with_index }.enable; [1].each_with_index { |v| v }; p $n",
            "$n=0; TracePoint.new(:c_call) { |t| $n+=1 if t.method_id == :each_with_index }.enable; [1].each { |v| v }; p $n",
        ),
    ] {
        let a = native(&ruby, a);
        let b = native(&ruby, b);
        assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
        assert_ne!(a.stdout, b.stdout);
    }
}
#[test]
fn native_syntax_only_for_every_reference_input_and_project_output() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
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
