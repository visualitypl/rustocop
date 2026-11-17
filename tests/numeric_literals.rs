//! pinned numeric spellings, CLI contracts and controlled Ruby observations.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const CASES: [(&str, &str, bool); 3] = [
    ("Style/FloatDivision", "a.to_f / b.to_f\n", false),
    ("Style/NumericLiteralPrefix", "01234\n", true),
    ("Style/NumericLiterals", "12345\n", true),
];
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(dir: &Path, args: &[&str], code: i32) -> Output {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(code), "{args:?}: {o:?}");
    assert!(o.stderr.is_empty() || code == 2, "{o:?}");
    o
}
fn unchanged(dir: &Path, s: &[u8], args: &[&str], code: i32) -> Output {
    let p = dir.join("a.rb");
    fs::write(&p, s).unwrap();
    let mtime = fs::metadata(&p).unwrap().modified().unwrap();
    let o = cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), s);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), mtime);
    o
}
#[test]
fn defaults_selectors_enabling_config_excludes_and_fixed_settings() {
    let r = rustocop::cops::Registry::new();
    let d = project();
    for (cop, source, safe) in CASES {
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        let c = r.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(c.default_enabled());
        assert_eq!(c.safe_fixes(), safe);
        assert!(
            String::from_utf8_lossy(&unchanged(d.path(), source.as_bytes(), &["a.rb"], 1).stdout)
                .contains(cop)
        );
        unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            "[cops.Style]\nenabled=false\n".into(),
            format!("[cops.\"{cop}\"]\nenabled=false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), source.as_bytes(), &["--fix", "a.rb"], 0);
            unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
        )
        .unwrap();
        unchanged(d.path(), source.as_bytes(), &["a.rb"], 1);
        for config in [
            "exclude=[\"a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude=[\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        for key in [
            "EnforcedStyle=\"fdiv\"",
            "MinDigits=4",
            "Strict=true",
            "AllowedNumbers=[12345]",
            "EnforcedOctalStyle=\"zero_only\"",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(d.path(), source.as_bytes(), &["--fix", "a.rb"], 2);
        }
    }
}
#[test]
fn directives_and_clean_or_error_files_do_not_write() {
    let d = project();
    for (cop, source, _) in CASES {
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, cop.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for s in [
                    format!("# {prefix}:disable {target}\n{source}"),
                    format!("# {prefix}:disable-next {target}\n{source}"),
                    format!("{} # {prefix}:disable {target}\n", source.trim_end()),
                ] {
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
                }
            }
            let s = format!("# {prefix}:disable {cop}\n{source}# {prefix}:enable {cop}\n{source}");
            let out = unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
            assert_eq!(
                String::from_utf8_lossy(&out.stdout)
                    .matches(&format!("({cop})"))
                    .count(),
                1
            );
        }
        for (source, code) in [("p :clean\n", 0), ("(\n", 2), ("0d0008\n(\n", 2)] {
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                code,
            );
        }
    }
}
#[test]
fn normalized_minimum_length_clean_files_do_not_write() {
    let d = project();
    for source in ["-#é\n1\n", "-\r\n123\n"] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", "Style/NumericLiterals", "--fix", "a.rb"],
            0,
        );
    }
}
#[test]
fn folded_sign_delimiters_follow_declared_character_boundaries() {
    let r = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let active =
        rustocop::cops::select_active(&r, &config, &["Style/NumericLiteralPrefix".into()], &[]);
    for tail in *b"eE" {
        let mut source = b"# coding: Windows-31J\n-\n# \x81".to_vec();
        source.push(tail);
        source.extend_from_slice(b"\n01234\n");
        let report = rustocop::pipeline::lint_one(
            Path::new("a.rb"),
            Path::new("a.rb"),
            Some(&source),
            &active,
            &config,
        );
        assert!(report.errors.is_empty() && report.internal_errors.is_empty());
        let found = report.result.offenses;
        assert_eq!(found.len(), 1, "{source:?}");
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 1, 4));
        assert_eq!(found[0].message, "Use 0o for octal literals.");
    }
}
fn rubies() -> Vec<String> {
    let mut r = vec!["ruby".into()];
    if let Ok(p) = crate::support::runtime::ruby() {
        r.push(p);
    }
    r
}
fn observe(ruby: &str, dir: &Path) -> Vec<u8> {
    let o = Command::new(ruby)
        .current_dir(dir)
        .arg("a.rb")
        .output()
        .unwrap();
    assert!(o.status.success(), "{ruby}: {o:?}");
    o.stdout
}
#[test]
fn runtime_exact_values_bases_float_bits_signs_and_huge_integers() {
    let d = project();
    let source = "p [01234, -0O1234, +0XABC, 0B10101, 0d0008, 0D000123, 0d000, -0d0008, 123456789012345678901234567890, -12345, +12345, 0, -0, __LINE__]\np [123456.789, -123456.789, 12345.6e3, 12345.6E-3, -0.0].pack(\"G*\").unpack1(\"H*\")\np [12345r, 12345i, 0XABr, 12345.67ri]\np __LINE__\n";
    for ruby in rubies() {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let before = observe(&ruby, d.path());
        cli(
            d.path(),
            &[
                "--only",
                "Style/NumericLiteralPrefix,Style/NumericLiterals",
                "--fix",
                "a.rb",
            ],
            0,
        );
        assert_ne!(fs::read_to_string(d.path().join("a.rb")).unwrap(), source);
        assert_eq!(before, observe(&ruby, d.path()));
        assert!(
            String::from_utf8_lossy(&before).starts_with("[668, -668, 2748, 21, 8, 123, 0, -8,")
        );
    }
}
#[test]
fn runtime_float_division_custom_operands_and_zero_are_never_rewritten() {
    let d = project();
    let source = "class NumericLike\n def to_f; puts :coerce; self; end\n def /(other); puts :divide; :result; end\nend\ndef value; puts :value; NumericLike.new; end\np(value.to_f / value.to_f)\np(0.to_f / 0.to_f)\np((-1).to_f / 0.to_f)\na = \"1.2\"; b = \"3.4\"\np(a.to_f / b.to_f)\n";
    for ruby in rubies() {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let before = observe(&ruby, d.path());
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", "Style/FloatDivision", "--fix", "a.rb"],
            1,
        );
        assert_eq!(before, observe(&ruby, d.path()));
        assert!(
            String::from_utf8_lossy(&before)
                .starts_with("value\ncoerce\nvalue\ncoerce\ndivide\n:result\nNaN\n-Infinity\n")
        );
    }
}
#[test]
fn stdin_never_writes_to_the_named_disk_file() {
    use crate::support::stdin;
    use std::process::Stdio;
    let d = project();
    for (cop, source, _) in CASES {
        for fix in [false, true] {
            fs::write(d.path().join("a.rb"), "disk bytes\n").unwrap();
            let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            command
                .current_dir(d.path())
                .args(["--only", cop, "--stdin", "a.rb"]);
            if fix {
                command.arg("--fix");
            }
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            stdin::write_stdin(child.stdin.take().unwrap(), source.as_bytes(), fix).unwrap();
            let output = child.wait_with_output().unwrap();
            assert_eq!(output.status.code(), Some(if fix { 2 } else { 1 }));
            if fix {
                assert!(output.stdout.is_empty(), "{output:?}");
                assert!(
                    String::from_utf8_lossy(&output.stderr)
                        .contains("the argument '--stdin <PATH>' cannot be used with '--fix'"),
                    "{output:?}"
                );
            }
            assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), b"disk bytes\n");
        }
    }
}
#[test]
fn selected_and_default_peer_corrections_stabilize_without_cycles() {
    let d = project();
    let source = "values = [0D123456, 0XFFFF, 123456];\np values\n";
    for only in [
        Some("Style/NumericLiteralPrefix,Style/NumericLiterals,Style/Semicolon,Layout"),
        None,
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let mut stable = false;
        for pass in 0..4 {
            let before = fs::read(d.path().join("a.rb")).unwrap();
            let args = if let Some(c) = only {
                vec!["--only", c, "--fix", "--fix-passes", "1", "a.rb"]
            } else {
                vec!["--fix", "--fix-passes", "1", "a.rb"]
            };
            cli(d.path(), &args, if pass == 0 { 1 } else { 0 });
            stable |= before == fs::read(d.path().join("a.rb")).unwrap();
        }
        assert!(stable);
        assert_eq!(
            fs::read_to_string(d.path().join("a.rb")).unwrap(),
            "values = [123_456, 0xFFFF, 123_456]\np values\n"
        );
    }
}
