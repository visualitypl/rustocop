//! Actual parser deliveries precede selection, directives and fixes.
//! Syntax fixture sources: RuboCop (MIT), Copyright (c) 2012-26 Bozhidar Batsov.
use rustocop::{
    config::ResolvedConfig,
    cops::{Cop, Registry},
    offense::Offense,
    pipeline::lint_one,
    source::SourceFile,
};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

const WARNINGS: &str = "Lint/AmbiguousOperator,Lint/AmbiguousRegexpLiteral";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}

fn cli(root: &Path, args: &[&str], input: Option<&[u8]>, code: i32) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(code), "{args:?}: {out:?}");
    assert!(out.stderr.is_empty(), "{args:?}: {out:?}");
    out
}

fn unchanged(path: &Path, source: &[u8], old: &fs::Metadata) {
    let new = fs::metadata(path).unwrap();
    assert_eq!(fs::read(path).unwrap(), source);
    assert_eq!(new.modified().unwrap(), old.modified().unwrap());
    assert_eq!(new.permissions(), old.permissions());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(new.ino(), old.ino());
    }
}

struct Canary(AtomicUsize);
impl Cop for Canary {
    fn name(&self) -> &'static str {
        "Lint/DiagnosticCanary"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn check_parse(&self, _: &ruby_prism::ParseResult<'_>, _: &SourceFile, _: &mut Vec<Offense>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn check_source(&self, _: &SourceFile, _: &mut Vec<Offense>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn check_node<'pr>(
        &self,
        _: &ruby_prism::Node<'pr>,
        _: &rustocop::cops::NodeContext<'_, 'pr>,
        _: &SourceFile,
        _: &mut Vec<Offense>,
    ) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn syntax_and_resource_errors_never_dispatch_ordinary_lifecycles() {
    let cop = Canary(AtomicUsize::new(0));
    for source in [
        "(".to_owned(),
        "foo *some_array\n(\n".to_owned(),
        "if true\n  puts 'x'\n".to_owned(),
        format!("root{}", ".foo".repeat(5000)),
        format!("x = {}1{}", "[".repeat(10_000), "]".repeat(10_000)),
    ] {
        let report = lint_one(
            Path::new("supplied.rb"),
            Path::new("display.rb"),
            Some(source.as_bytes()),
            &[&cop],
            &ResolvedConfig::default(),
        );
        assert!(!report.errors.is_empty());
        assert!(report.result.offenses.is_empty());
        assert!(report.internal_errors.is_empty());
        assert!(
            report
                .errors
                .iter()
                .all(|e| e.path == Path::new("display.rb"))
        );
        assert_eq!(cop.0.load(Ordering::SeqCst), 0);
    }
    let report = lint_one(
        Path::new("supplied.rb"),
        Path::new("display.rb"),
        Some(b"foo *some_array\n"),
        &[&cop],
        &ResolvedConfig::default(),
    );
    assert!(report.errors.is_empty());
    assert!(cop.0.load(Ordering::SeqCst) > 2);
    assert!(Registry::new().has_cop("Lint/Syntax"));
    // A buffer error is not a Prism error, but still forbids every ordinary
    // hook and fix, whether the named fatal consumer is selected or not.
    for active in [
        vec![&cop as &dyn Cop],
        vec![&cop as &dyn Cop, &rustocop::cops::lint::Syntax],
    ] {
        cop.0.store(0, Ordering::SeqCst);
        let report = lint_one(
            Path::new("supplied.rb"),
            Path::new("display.rb"),
            Some(b"# \xf9\nfail\n"),
            &active,
            &ResolvedConfig::default(),
        );
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].position, None);
        assert!(
            report.errors[0]
                .message
                .starts_with("Invalid byte sequence in utf-8.")
        );
        assert!(report.result.offenses.is_empty());
        assert_eq!(cop.0.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn fatal_errors_ignore_cop_selection_configuration_and_directives_without_writes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for prefix in ["rubocop", "rustocop"] {
        let source = format!(
            "# {prefix}:disable all\n# {prefix}:push\n# {prefix}:disable-next Lint\nfoo *args\nfail\n(\n"
        );
        fs::write(&path, &source).unwrap();
        let old = fs::metadata(&path).unwrap();
        for config in [
            "",
            "[cops.Lint]\nenabled = false\n[cops.Style]\nenabled = false\n",
            "[cops.\"Lint/AmbiguousOperator\"]\nenabled = false\nexclude = ['lib/**']\n",
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            for select in [
                vec![],
                vec!["--only", WARNINGS],
                vec!["--only", "Style/SignalException"],
                vec!["--except", "Lint,Style,Layout,Naming,Security"],
                vec!["--only", "Lint/UselessElseWithoutRescue"],
            ] {
                for fix in [false, true] {
                    let mut args = select.clone();
                    if fix {
                        args.push("--fix");
                    }
                    args.push("lib/a.rb");
                    let out = cli(dir.path(), &args, None, 2);
                    let text = String::from_utf8(out.stdout).unwrap();
                    assert!(
                        text.contains("lib/a.rb: error: syntax error on line 6:"),
                        "{text}"
                    );
                    assert!(
                        !text.contains("(Lint/Ambiguous") && !text.contains("(Style/"),
                        "{text}"
                    );
                    unchanged(&path, source.as_bytes(), &old);
                }
            }
            let out = cli(
                dir.path(),
                &["--only", WARNINGS, "--stdin", "virtual/supplied.rb"],
                Some(source.as_bytes()),
                2,
            );
            assert!(
                String::from_utf8(out.stdout)
                    .unwrap()
                    .contains("virtual/supplied.rb: error: syntax error on line 6:")
            );
        }
    }
}

#[test]
fn syntax_named_fatal_selection_and_global_buffer_errors_never_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for source in [b"(".as_slice(), b"# \xf9\nfail\n"] {
        fs::write(&path, source).unwrap();
        let old = fs::metadata(&path).unwrap();
        let mut held = fs::File::open(&path).unwrap();
        for (config, selection, named) in [
            ("", vec![], true),
            ("", vec!["--only", "Lint/Syntax"], true),
            ("", vec!["--only", "Lint/Syntax", "--except", "Lint"], false),
            ("", vec!["--only", "Style/SignalException"], false),
            ("[cops.Lint]\nenabled = false\n", vec![], false),
            (
                "[cops.Lint]\nenabled = false\n[cops.\"Lint/Syntax\"]\nenabled = true\n",
                vec![],
                true,
            ),
            (
                "[cops.Lint]\nenabled = true\n[cops.\"Lint/Syntax\"]\nenabled = false\n",
                vec![],
                false,
            ),
            (
                "[cops.\"Lint/Syntax\"]\nenabled = false\n",
                vec!["--only", "Lint/Syntax"],
                true,
            ),
            (
                "[cops.\"Lint/Syntax\"]\nexclude = ['lib/**']\n",
                vec!["--only", "Lint/Syntax"],
                false,
            ),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            for fix in [false, true] {
                let mut args = selection.clone();
                if fix {
                    args.push("--fix");
                }
                args.push("lib/a.rb");
                let out = cli(dir.path(), &args, None, 2);
                let text = String::from_utf8(out.stdout).unwrap();
                assert_eq!(text.contains("(Lint/Syntax)"), named, "{config}: {text}");
                assert!(
                    text.contains(if fix {
                        "0 offenses remaining, 1 error"
                    } else {
                        "no offenses detected, 1 error"
                    }),
                    "{text}"
                );
                unchanged(&path, source, &old);
            }
            let mut args = selection.clone();
            args.extend(["--format", "github", "--stdin", "lib/a.rb"]);
            let out = cli(dir.path(), &args, Some(source), 2);
            let text = String::from_utf8(out.stdout).unwrap();
            assert_eq!(text.contains("(Lint/Syntax)"), named, "{text}");
            if source == b"(" {
                let col = if named { 1 } else { 2 };
                assert!(
                    text.contains(&format!("file=lib/a.rb,line=1,col={col},")),
                    "{text}"
                );
            } else {
                assert!(text.contains("file=lib/a.rb,title=rustocop::"), "{text}");
                assert!(!text.contains(",line="), "{text}");
            }
        }
        let mut observed = Vec::new();
        held.read_to_end(&mut observed).unwrap();
        assert_eq!(observed, source);
    }
}

#[test]
fn syntax_directive_scopes_never_suppress_fatal_delivery() {
    let dir = project();
    for prefix in ["rustocop", "rubocop"] {
        for scope in [
            format!("# {prefix}:disable Lint/Syntax\n"),
            format!("# {prefix}:disable Lint/Syntax,\n#   Style/SignalException\n"),
            format!("# {prefix}:disable-next Lint/Syntax\n"),
            format!("# {prefix}:push\n# {prefix}:disable all\n"),
        ] {
            for tail in [b"(".as_slice(), b"# \xf9\nfail\n"] {
                let source = [scope.as_bytes(), tail].concat();
                let out = cli(
                    dir.path(),
                    &["--only", "Lint/Syntax", "--stdin", "virtual/input.rb"],
                    Some(&source),
                    2,
                );
                let text = String::from_utf8(out.stdout).unwrap();
                assert!(text.contains("virtual/input.rb: error:"), "{text}");
                assert!(text.contains("(Lint/Syntax)"), "{text}");
            }
        }
    }
}

#[test]
fn syntax_expanded_positions_keep_encoded_original_bytes_and_observers() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (prefix, column) in [
        (b"\xef\xbb\xbf\"\xc3\xa9\"; ".as_slice(), 6),
        (b"# coding: Shift_JIS\r\n\"\x82\xa0\"; ", 6),
        (b"# coding: ISO-8859-1\r\n\"\xe9\"; ", 6),
        (b"# coding: ascii-8bit\r\n\"\xc3\xa9\"; ", 7),
    ] {
        let mut source = prefix.to_vec();
        source.extend_from_slice(b"puts <<~TEXT\r\n#{__LINE__}\r\nTEXT\r\nFile.binread(__FILE__)\r\nRubyVM::InstructionSequence.of(method(:foo))\r\nObjectSpace.each_object(String)\r\n(\r\n__END__\r\n(\r\n");
        // An EOF in a separate source keeps an actually zero-length range;
        // the observer/heredoc/DATA case instead retains raw Prism locations.
        for bytes in [source, [prefix, b"foo("].concat()] {
            fs::write(&path, &bytes).unwrap();
            let old = fs::metadata(&path).unwrap();
            let out = cli(
                dir.path(),
                &[
                    "--only",
                    "Lint/Syntax",
                    "--fix",
                    "--format",
                    "github",
                    "lib/a.rb",
                ],
                None,
                2,
            );
            let text = String::from_utf8(out.stdout).unwrap();
            assert!(text.contains("(Lint/Syntax)"), "{text}");
            if bytes.ends_with(b"foo(") {
                let line = if prefix.starts_with(b"#") { 2 } else { 1 };
                assert!(
                    text.contains(&format!("line={line},col={},", column + 3)),
                    "{text}"
                );
            }
            unchanged(&path, &bytes, &old);
        }
    }
}

#[test]
fn syntax_admission_keeps_minimal_strict_configuration() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, b"(").unwrap();
    let old = fs::metadata(&path).unwrap();
    for setting in [
        "max = 1",
        "TargetRubyVersion = '3.3'",
        "ParserEngine = 'parser_whitequark'",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"Lint/Syntax\"]\n{setting}\n"),
        )
        .unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", "Lint/Syntax", "--fix", "lib/a.rb"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(!out.stderr.is_empty());
        assert!(
            !String::from_utf8(out.stdout)
                .unwrap()
                .contains("(Lint/Syntax)")
        );
        unchanged(&path, b"(", &old);
    }
}

#[test]
fn empty_fatal_hooks_retain_rejection_and_never_run_on_valid_input() {
    struct NoFindings(AtomicUsize);
    impl Cop for NoFindings {
        fn name(&self) -> &'static str {
            "Lint/NoFatalFindings"
        }
        fn wants_fatal_diagnostics(&self) -> bool {
            true
        }
        fn check_fatal(&self, _: &SourceFile, _: &mut Vec<rustocop::source::FatalDiagnostic>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let empty = NoFindings(AtomicUsize::new(0));
    let ordinary = Canary(AtomicUsize::new(0));
    let syntax = rustocop::cops::lint::Syntax;
    for source in [b"(".as_slice(), b"# \xf9\n(", b"foo *args\n("] {
        for active in [
            vec![&empty as &dyn Cop, &ordinary],
            vec![&empty as &dyn Cop, &syntax, &ordinary],
            vec![&syntax as &dyn Cop, &empty, &ordinary],
        ] {
            empty.0.store(0, Ordering::SeqCst);
            let named = active.iter().any(|c| c.name() == "Lint/Syntax");
            let report = lint_one(
                Path::new("file.rb"),
                Path::new("display.rb"),
                Some(source),
                &active,
                &ResolvedConfig::default(),
            );
            assert_eq!(empty.0.load(Ordering::SeqCst), 1);
            assert_eq!(ordinary.0.load(Ordering::SeqCst), 0);
            assert_eq!(report.errors.len(), 1);
            assert_eq!(report.errors[0].path, Path::new("display.rb"));
            assert_eq!(report.errors[0].message.contains("(Lint/Syntax)"), named);
            if source.starts_with(b"#") {
                assert_eq!(report.errors[0].position, None);
                assert!(
                    report.errors[0]
                        .message
                        .starts_with("Invalid byte sequence in utf-8.")
                );
            } else {
                assert!(report.errors[0].position.is_some());
            }
            assert!(report.result.offenses.is_empty());
            assert!(report.internal_errors.is_empty());
            assert_eq!(report.corrections_applied, 0);
        }
    }
    empty.0.store(0, Ordering::SeqCst);
    for source in [b"".as_slice(), b"foo *args\n", b"foo(*args)\n"] {
        let report = lint_one(
            Path::new("file.rb"),
            Path::new("display.rb"),
            Some(source),
            &[&empty, &syntax],
            &ResolvedConfig::default(),
        );
        assert!(report.errors.is_empty());
        assert!(report.result.offenses.is_empty());
        assert_eq!(empty.0.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn fatal_lifecycle_is_opt_in_isolated_and_cannot_bypass_rejection() {
    struct FatalCanary(bool);
    impl Cop for FatalCanary {
        fn name(&self) -> &'static str {
            "Lint/FatalCanary"
        }
        fn wants_fatal_diagnostics(&self) -> bool {
            self.0
        }
        fn check_fatal(&self, _: &SourceFile, _: &mut Vec<rustocop::source::FatalDiagnostic>) {
            panic!("fatal hook isolated");
        }
    }
    for opted_in in [false, true] {
        let cop = FatalCanary(opted_in);
        let report = lint_one(
            Path::new("file.rb"),
            Path::new("file.rb"),
            Some(b"("),
            &[&cop],
            &ResolvedConfig::default(),
        );
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.internal_errors.len(), usize::from(opted_in));
        assert!(report.result.offenses.is_empty());
        assert_eq!(report.corrections_applied, 0);
    }
}

#[test]
fn both_prefixes_multiline_next_and_push_scopes_filter_only_warning_findings() {
    let dir = project();
    for prefix in ["rubocop", "rustocop"] {
        let source = format!(
            "# {prefix}:disable Lint/AmbiguousOperator\nfoo *args\n# {prefix}:enable Lint/AmbiguousOperator\n# {prefix}:disable-next Lint/AmbiguousRegexpLiteral\np /pattern/\n# {prefix}:push\n# {prefix}:disable Lint\nfoo *args\np /pattern/\n# {prefix}:pop\nfoo *args\np /pattern/\n"
        );
        let out = cli(
            dir.path(),
            &["--only", WARNINGS, "--stdin", "lib/a.rb"],
            Some(source.as_bytes()),
            1,
        );
        let text = String::from_utf8(out.stdout).unwrap();
        let findings: Vec<_> = text
            .lines()
            .filter(|l| l.contains("(Lint/Ambiguous"))
            .collect();
        assert_eq!(findings.len(), 2, "{text}");
        assert!(findings[0].starts_with("lib/a.rb:11:5: E:"), "{text}");
        assert!(findings[1].starts_with("lib/a.rb:12:3: E:"), "{text}");
    }
}

#[cfg(unix)]
#[test]
fn rejected_input_preserves_readonly_symlink_target_and_other_files_can_fix() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let alias = dir.path().join("lib/link.rb");
    let source = b"foo *args\nfail\n(\n";
    fs::write(&path, source).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    symlink("a.rb", &alias).unwrap();
    fs::write(dir.path().join("lib/good.rb"), "fail\n").unwrap();
    let old = fs::metadata(&path).unwrap();
    cli(
        dir.path(),
        &[
            "--only",
            "Style/SignalException,Lint/Syntax",
            "--fix",
            "lib/a.rb",
            "lib/link.rb",
            "lib/good.rb",
        ],
        None,
        2,
    );
    unchanged(&path, source, &old);
    assert_eq!(fs::read_link(alias).unwrap(), Path::new("a.rb"));
    assert_eq!(
        fs::read(dir.path().join("lib/good.rb")).unwrap(),
        b"raise\n"
    );
    // The shared stdin+fix rejection happens before parsing or source reading.
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--stdin", "virtual.rb", "--fix"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8(out.stderr)
            .unwrap()
            .contains("cannot be used with")
    );
}
