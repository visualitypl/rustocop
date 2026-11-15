//! Pinned source-only matcher/correction fixtures and actual CLI proofs.
//! RuboCop examples: MIT, Copyright (c) 2012–26 Bozhidar Batsov.
use rustocop::{
    config::ResolvedConfig,
    cops::{Registry, select_active},
    offense::Severity,
};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const COP: &str = "Layout/ArgumentAlignment";
fn rows() -> Vec<serde_json::Value> {
    serde_json::from_str(include_str!("fixtures/argument_alignment.json")).unwrap()
}
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
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
fn bytes(row: &serde_json::Value, field: &str) -> Vec<u8> {
    serde_json::from_value(row[field].clone()).unwrap()
}
fn unchanged(path: &Path, before: &[u8], old: &fs::Metadata) {
    assert_eq!(fs::read(path).unwrap(), before);
    let new = fs::metadata(path).unwrap();
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    assert_eq!(old.permissions(), new.permissions());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(old.ino(), new.ino());
    }
}
#[test]
fn complete_main_custom_source_encoding_and_adversarial_exact_detection() {
    let registry = Registry::new();
    let config = ResolvedConfig::default();
    let active = select_active(&registry, &config, &[COP.into()], &[]);
    let mut mismatches = Vec::new();
    for row in rows() {
        let source = bytes(&row, "bytes");
        let report = rustocop::pipeline::lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(&source),
            &active,
            &config,
        );
        assert!(report.internal_errors.is_empty(), "{row}: {report:?}");
        let rejected = row.get("parser_rejection").is_some();
        if !report.errors.is_empty() != rejected {
            mismatches.push(format!("parser {row}: {report:?}"));
            continue;
        }
        let mut found: Vec<_> = report
            .result
            .offenses
            .into_iter()
            .map(|o| {
                assert_eq!(o.severity, Severity::Convention);
                assert!(o.correction.is_none());
                (o.line, o.col, o.end_line, o.message)
            })
            .collect();
        found.sort();
        let expected: Vec<(u32, u32, u32, String)> = if rejected {
            vec![]
        } else {
            serde_json::from_value(row["findings"].clone()).unwrap()
        };
        if found != expected {
            mismatches.push(format!(
                "{}:{} {:?}\nproject {found:?}\nreference {expected:?}",
                row["origin"],
                row["line"],
                String::from_utf8_lossy(&source)
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatches:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}
#[test]
fn metadata_default_only_except_config_and_no_future_names_or_knobs() {
    let registry = Registry::new();
    let config = ResolvedConfig::default();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(cop.safe_fixes());
    assert_eq!(cop.severity(), Severity::Convention);
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(
        select_active(&registry, &config, &[], &[])
            .iter()
            .any(|c| c.name() == COP)
    );
    // Landed HashAlignment with the same fixed `key` styles; the
    // projection above is unchanged by its presence.
    assert!(
        registry
            .cops()
            .iter()
            .any(|c| c.name() == "Layout/HashAlignment")
    );
    let d = project();
    let path = d.path().join("example.rb");
    fs::write(&path, "f(a,\nb)\n").unwrap();
    for setting in [
        format!("[cops.\"{COP}\"]\nenabled = false\n"),
        "[cops.Layout]\nenabled = false\n".into(),
    ] {
        fs::write(d.path().join("rustocop.toml"), setting).unwrap();
        assert!(!String::from_utf8_lossy(&run(d.path(), &["example.rb"]).stdout).contains(COP));
        assert_eq!(
            run(d.path(), &["--only", COP, "example.rb"]).status.code(),
            Some(1)
        );
        assert_eq!(
            run(d.path(), &["--only", COP, "--except", COP, "example.rb"])
                .status
                .code(),
            Some(0)
        );
    }
    for setting in [
        "exclude=['example.rb']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude=['example.rb']\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), setting).unwrap();
        let old = fs::metadata(&path).unwrap();
        assert_eq!(
            run(d.path(), &["--fix", "--only", COP, "example.rb"])
                .status
                .code(),
            Some(0)
        );
        unchanged(&path, b"f(a,\nb)\n", &old);
    }
    for setting in [
        format!("[cops.\"{COP}\"]\nmax = 4\n"),
        format!("[cops.\"{COP}\"]\nEnforcedStyle = 'with_fixed_indentation'\n"),
        "[cops.\"Layout/HashAlignment\"]\nEnforcedColonStyle = 'separator'\n".into(),
    ] {
        fs::write(d.path().join("rustocop.toml"), setting).unwrap();
        let old = fs::metadata(&path).unwrap();
        assert_eq!(
            run(d.path(), &["--fix", "--only", COP, "example.rb"])
                .status
                .code(),
            Some(2)
        );
        unchanged(&path, b"f(a,\nb)\n", &old);
    }
}
#[test]
fn both_directive_prefixes_multiline_span_push_next_inline_and_reenable() {
    let d = project();
    let path = d.path().join("example.rb");
    for prefix in ["rustocop", "rubocop"] {
        for source in [
            format!("# {prefix}:disable {COP}\nf(a,\nb)\n"),
            format!("# {prefix}:push -Layout\nf(a,\nb)\n# {prefix}:pop\n"),
            format!("f(a,\nb) # {prefix}:todo {COP}\n"),
            format!("f(a,\ng do\n  1 # {prefix}:disable {COP}\nend)\n"),
            format!("# {prefix}:disable-next {COP}\nf(a,\nb)\n"),
        ] {
            fs::write(&path, &source).unwrap();
            let old = fs::metadata(&path).unwrap();
            let out = run(d.path(), &["--fix", "--only", COP, "example.rb"]);
            assert_eq!(out.status.code(), Some(0), "{source}: {out:?}");
            unchanged(&path, source.as_bytes(), &old);
        }
        let source =
            format!("# {prefix}:disable {COP}\nf(a,\nb)\n# {prefix}:enable {COP}\nf(a,\nb)\n");
        fs::write(&path, &source).unwrap();
        assert_eq!(
            run(d.path(), &["--fix", "--only", COP, "example.rb"])
                .status
                .code(),
            Some(0)
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            source.replace(
                "enable Layout/ArgumentAlignment\nf(a,\nb)",
                "enable Layout/ArgumentAlignment\nf(a,\n  b)"
            )
        );
    }
}
#[test]
fn vertical_tab_cli_positions_canonical_positive_shifts_and_shared_assignment_consumer() {
    let d = project();
    let path = d.path().join("example.rb");
    for (cop, source, after, col) in [
        (COP, "f(a,\n\u{b}b)\n", "f(a,\n\u{b}  b)\n", Some(2)),
        (COP, "f(a,\n   \u{b}b)\n", "f(a,\n   \u{b}b)\n", Some(5)),
        (COP, "f(a,\n \u{b} b)\n", "f(a,\n \u{b} b)\n", None),
        (
            COP,
            "f(\n\u{b}a,\n\u{b}b)\n",
            "f(\n\u{b}a,\n\u{b}b)\n",
            None,
        ),
        (
            "Layout/AssignmentIndentation",
            "a =\n\u{b}1\n",
            "a =\n\u{b}  1\n",
            Some(2),
        ),
        (
            "Layout/AssignmentIndentation",
            "a =\n\u{b}  1\n",
            "a =\n\u{b}  1\n",
            None,
        ),
    ] {
        fs::write(&path, source).unwrap();
        let old = fs::metadata(&path).unwrap();
        let lint = run(d.path(), &["--only", cop, "example.rb"]);
        assert_eq!(
            lint.status.code(),
            Some(i32::from(col.is_some())),
            "{lint:?}"
        );
        if let Some(col) = col {
            assert!(
                String::from_utf8_lossy(&lint.stdout).contains(&format!("example.rb:2:{col}:")),
                "{lint:?}"
            );
        }
        unchanged(&path, source.as_bytes(), &old);
        let fix = run(d.path(), &["--fix", "--only", cop, "example.rb"]);
        assert_eq!(
            fix.status.code(),
            Some(i32::from(col.is_some() && source == after)),
            "{fix:?}"
        );
        assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
        if source == after {
            unchanged(&path, source.as_bytes(), &old);
        }
        if let Ok(ruby) = crate::support::runtime::ruby() {
            let before = syntax(&ruby, source.as_bytes());
            let after = syntax(&ruby, after.as_bytes());
            assert!(
                before.status.success() && after.status.success(),
                "{before:?} {after:?}"
            );
            assert_eq!(before.stderr, after.stderr);
        }
    }
}
#[test]
fn literal_and_external_filename_readers_allow_real_canonical_edits() {
    let d = project();
    let path = d.path().join("example.rb");
    let absolute = format!("{:?}", path.to_str().unwrap());
    for observer in [
        format!("puts File.binread({absolute})"),
        "puts File.read('example.rb')".into(),
        "puts IO.binread('./example.rb')".into(),
        format!("F = File; path = {absolute}; puts F.read(path)"),
        format!("puts File.method(:binread).call({absolute})"),
        format!("puts File.open({absolute}, &:read)"),
        "File.foreach('example.rb') { |line| print line }".into(),
        // A supplied path/handle need not be acquired through __FILE__ at all.
        "path = ENV.fetch('TASK412_SOURCE'); puts IO.read(path)".into(),
    ] {
        // Separately authored, finite runtime witnesses; never reference inputs.
        let source = format!("def f(*xs); p xs; end\nf(1,\n2)\n{observer}\n");
        fs::write(&path, &source).unwrap();
        let old = fs::metadata(&path).unwrap();
        // Parse only: source-reader output is outside the safety boundary.
        let out = run(d.path(), &["--only", COP, "example.rb"]);
        assert_eq!(out.status.code(), Some(1), "{observer}: {out:?}");
        assert!(String::from_utf8_lossy(&out.stdout).contains("example.rb:3:1:"));
        unchanged(&path, source.as_bytes(), &old);
        let expected = source.replacen("\n2)", "\n  2)", 1);
        let out = run(d.path(), &["--fix", "--only", COP, "example.rb"]);
        assert_eq!(out.status.code(), Some(0), "{observer}: {out:?}");
        assert_eq!(fs::read(&path).unwrap(), expected.as_bytes());
        assert_eq!(
            old.permissions(),
            fs::metadata(&path).unwrap().permissions()
        );
        let old = fs::metadata(&path).unwrap();
        assert_eq!(
            run(d.path(), &["--fix", "--only", COP, "example.rb"])
                .status
                .code(),
            Some(0)
        );
        unchanged(&path, expected.as_bytes(), &old);
    }
}
#[test]
fn supplied_paths_stdin_fatal_resource_encoding_bom_crlf_and_opaque_data() {
    use std::{io::Write, process::Stdio};
    let d = project();
    let path = d.path().join("input.arb");
    for (source, after) in [
        (
            b"\xef\xbb\xbff(a,\r\nb)\r\n__END__\r\nraw\xff\0\r\n".as_slice(),
            b"\xef\xbb\xbff(a,\r\n  b)\r\n__END__\r\nraw\xff\0\r\n".as_slice(),
        ),
        (
            b"# coding: Windows-1252\r\nf('\xe9',\r\nb)\r\n",
            b"# coding: Windows-1252\r\nf('\xe9',\r\n  b)\r\n",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let out = run(d.path(), &["--fix", "--only", COP, "input.arb"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fs::read(&path).unwrap(), after);
        fs::write(&path, b"sentinel\n").unwrap();
        let old = fs::metadata(&path).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", COP, "--stdin", "input.arb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(source).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        unchanged(&path, b"sentinel\n", &old);
        assert_eq!(
            run(d.path(), &["--fix", "--only", COP, "--stdin", "input.arb"])
                .status
                .code(),
            Some(2)
        );
        unchanged(&path, b"sentinel\n", &old);
    }
    for source in [
        b"f(a,\nb)\ndef broken(\n".to_vec(),
        b"# coding: UTF-8\nf(a,\n'\xff')\n".to_vec(),
        b"# coding: NotAnEncoding\nf(a,\nb)\n".to_vec(),
        format!("{}a{}\n", "f(".repeat(90), ")".repeat(90)).into_bytes(),
    ] {
        fs::write(&path, &source).unwrap();
        let old = fs::metadata(&path).unwrap();
        assert_eq!(
            run(d.path(), &["--fix", "--only", COP, "input.arb"])
                .status
                .code(),
            Some(2)
        );
        unchanged(&path, &source, &old);
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", COP, "--stdin", "input.arb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&source).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert!(!String::from_utf8_lossy(&out.stdout).contains(COP));
        unchanged(&path, &source, &old);
    }
    assert_eq!(
        run(d.path(), &["--fix", "--only", COP, "missing.rb"])
            .status
            .code(),
        Some(2)
    );
}
#[test]
#[cfg(unix)]
fn atomic_aliases_retained_open_descriptor_mode_and_read_only_failure() {
    use std::{
        io::Read,
        os::unix::fs::{PermissionsExt, symlink},
    };
    let d = project();
    let path = d.path().join("example.rb");
    let alias = d.path().join("alias.rb");
    let source = b"f(a,\nb)\n";
    fs::write(&path, source).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    symlink("example.rb", &alias).unwrap();
    let mut old_fd = fs::File::open(&path).unwrap();
    assert_eq!(
        run(
            d.path(),
            &["--fix", "--only", COP, "alias.rb", "example.rb"]
        )
        .status
        .code(),
        Some(0)
    );
    assert_eq!(fs::read(&path).unwrap(), b"f(a,\n  b)\n");
    assert!(fs::symlink_metadata(&alias).unwrap().is_symlink());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    let mut retained = Vec::new();
    old_fd.read_to_end(&mut retained).unwrap();
    assert_eq!(retained, source);
    fs::write(&path, source).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    assert_eq!(
        run(
            d.path(),
            &["--fix", "--only", COP, "alias.rb", "example.rb"]
        )
        .status
        .code(),
        Some(2)
    );
    unchanged(&path, source, &old);
    assert!(fs::symlink_metadata(&alias).unwrap().is_symlink());
}
#[test]
fn real_layout_and_style_composition_converges_without_cycles() {
    let d = project();
    let path = d.path().join("example.rb");
    let source = "module A\nprivate\n  def f\n    x =\n    g('a',\n'b')\n    x\n  end\nend\n";
    fs::write(&path, source).unwrap();
    let args = [
        "--fix",
        "--only",
        "Layout,Style/StringLiterals",
        "example.rb",
    ];
    let mut snapshots = vec![source.as_bytes().to_vec()];
    let mut clean = false;
    for _ in 0..8 {
        let out = run(d.path(), &args);
        assert_ne!(out.status.code(), Some(2), "{out:?}");
        let next = fs::read(&path).unwrap();
        if out.status.success() {
            clean = true;
            break;
        }
        assert!(!snapshots.contains(&next), "cycle {out:?}");
        snapshots.push(next);
    }
    assert!(clean);
    let after = fs::read(&path).unwrap();
    assert!(String::from_utf8_lossy(&after).contains("\"b\""));
    let old = fs::metadata(&path).unwrap();
    assert!(run(d.path(), &args).status.success());
    unchanged(&path, &after, &old);
}
fn syntax(ruby: &str, source: &[u8]) -> Output {
    use std::{io::Write, process::Stdio};
    let stdout = tempfile::tempfile().unwrap();
    let stderr = tempfile::tempfile().unwrap();
    let mut child = Command::new("timeout")
        .args([
            "--kill-after=1s",
            "10",
            "prlimit",
            "--as=2147483648",
            "--fsize=1048576",
            "--core=0",
            "--",
        ])
        .arg(ruby)
        .args(["--parser=prism", "-w", "-c"])
        .stdin(Stdio::piped())
        .stdout(stdout.try_clone().unwrap())
        .stderr(stderr.try_clone().unwrap())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    let status = child.wait().unwrap();
    use std::io::{Read, Seek};
    fn read(mut f: fs::File) -> Vec<u8> {
        assert!(
            f.metadata().unwrap().len() < 1048576,
            "native output overflow"
        );
        f.rewind().unwrap();
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes).unwrap();
        bytes
    }
    assert!(
        !matches!(status.code(), None | Some(124 | 137)),
        "native timeout/signal {status:?}"
    );
    Output {
        status,
        stdout: read(stdout),
        stderr: read(stderr),
    }
}
#[test]
fn native_ruby34_syntax_only_all_reference_sides_never_execute() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    for row in rows() {
        let mut sides = vec!["bytes"];
        if row.get("parser_rejection").is_none() {
            sides.push(if row.get("original_token_after_bytes").is_some() {
                "original_token_after_bytes"
            } else {
                "after_bytes"
            });
        }
        for side in sides {
            let out = syntax(&ruby, &bytes(&row, side));
            assert_eq!(
                out.status.success(),
                row.get("parser_rejection").is_none(),
                "{row}: {out:?}"
            );
        }
    }
}
use crate::support::deprecated_constructor_native as native;
#[test]
fn bounded_authored_runtime_dispatch_values_control_literals_lines_and_warnings() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let d = project();
    let path = d.path().join("example.rb");
    // These finite programs are separately AUTHORED, not reference/spec/corpus.
    for source in [
        "def f(*a); p a; end\nf(1,\nbegin\n  puts :before\n  2\nensure\n  puts :ensure\nend)\n",
        "def f(*a); p a; end\nf(__LINE__,\nbegin\n  s = <<~H\n raw\nH\n  [s, __LINE__]\nend)\n",
        "def f(*a); p a; end\nf(1,\n\"two\nraw\")\n",
        "def f(*a); p a; end\nf(1,\n\tif true\n\t  2\n\tend)\n",
        "def f(*a); p a; end\nf(1,\n\u{b}2)\n",
        "def f(*a); p a; end\nf(1,\n   \u{b}2)\n",
        "def f(*a); p a; end\nf(1,\n          if true\n  2\n  end)\n",
        "def f(*a); p a; end\nf(1,\n  \tif true\n  \t  2\n  \tend)\n",
        "def f(*a); p a; end\nf(1,\n2)\np binding.local_variables\n",
    ] {
        let before = native::run(&ruby, source);
        assert!(before.status.success(), "{before:?}");
        fs::write(&path, source).unwrap();
        let out = run(d.path(), &["--fix", "--only", COP, "example.rb"]);
        assert_ne!(out.status.code(), Some(2), "{out:?}");
        let fixed = fs::read_to_string(&path).unwrap();
        let after = native::run(&ruby, &fixed);
        assert_eq!(before.status.code(), after.status.code());
        assert_eq!(before.stdout, after.stdout, "{source}\n{fixed}");
        assert_eq!(before.stderr, after.stderr, "{source}\n{fixed}");
    }
}
