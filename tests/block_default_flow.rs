//! Authored source-only parser, lifecycle and real CLI witnesses.
use crate::support::flow_defaults as witnesses;

use rustocop::{
    config::ResolvedConfig,
    cops::{Cop, NodeContext, PostInvestigation},
    offense::Offense,
    pipeline::lint_one,
    source::SourceFile,
};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

#[test]
fn native_ruby34_source_only_flow_default_boundaries() {
    if crate::support::runtime::skips("asserts the Ruby 3.4 grammar, which Ruby 4.0 changed") {
        return;
    }
    let Some(ruby) = crate::support::runtime::ruby_os() else {
        eprintln!("SKIP native Ruby 3.4: set RUSTOCOP_RUBY34");
        return;
    };
    let version = Command::new(&ruby).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(
        version
            .stdout
            .starts_with(crate::support::runtime::banner_prefix().as_bytes()),
        "{version:?}"
    );
    let cases = witnesses::cases();
    for case in &cases {
        let mut child = Command::new(&ruby)
            .env_remove("RUBYOPT")
            .env_remove("RUBYLIB")
            .args(["--parser=prism", "-c", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&case.source).unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.success(), case.valid, "{case:?}: {output:?}");
        if !case.valid {
            assert_eq!(output.status.code(), Some(1));
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains(&format!("Invalid {}", case.keyword)),
                "{case:?}: {stderr}"
            );
            // Native pretty diagnostics expose exact node widths. For ASCII
            // inputs the caret's source column is also directly byte-addressed.
            if case.source.is_ascii() {
                let line_start = case.source[..case.range.start]
                    .iter()
                    .rposition(|&b| b == b'\n')
                    .map_or(0, |i| i + 1);
                let marker = format!(
                    "^{} Invalid {}",
                    "~".repeat(case.range.len() - 1),
                    case.keyword
                );
                assert!(stderr.contains(&marker), "{case:?}: {stderr}");
                // Ruby abbreviates long source lines with `...`; only compare
                // an absolute column when the displayed line is unabridged.
                if !stderr.contains("| ...") {
                    let caret = format!("| {}{marker}", " ".repeat(case.range.start - line_start));
                    assert!(stderr.contains(&caret), "{case:?}: {stderr}");
                }
            }
        }
    }
    eprintln!("native source-only: {} cases", cases.len());
}

struct Canary([AtomicUsize; 5]);
impl Cop for Canary {
    fn name(&self) -> &'static str {
        "Lint/FlowCanary"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn check_parse(&self, _: &ruby_prism::ParseResult<'_>, _: &SourceFile, _: &mut Vec<Offense>) {
        self.0[0].fetch_add(1, Ordering::SeqCst);
    }
    fn check_source(&self, _: &SourceFile, _: &mut Vec<Offense>) {
        self.0[1].fetch_add(1, Ordering::SeqCst);
    }
    fn check_node<'pr>(
        &self,
        _: &ruby_prism::Node<'pr>,
        _: &NodeContext<'_, 'pr>,
        _: &SourceFile,
        _: &mut Vec<Offense>,
    ) {
        self.0[2].fetch_add(1, Ordering::SeqCst);
    }
    fn wants_leave_node(&self) -> bool {
        true
    }
    fn leave_node<'pr>(
        &self,
        _: &ruby_prism::Node<'pr>,
        _: &NodeContext<'_, 'pr>,
        _: &SourceFile,
        _: &mut Vec<Offense>,
    ) {
        self.0[3].fetch_add(1, Ordering::SeqCst);
    }
    fn wants_post_investigation(&self) -> bool {
        true
    }
    fn check_post_investigation(
        &self,
        _: &PostInvestigation<'_>,
        _: &SourceFile,
        _: &mut Vec<Offense>,
    ) {
        self.0[4].fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn flow_defaults_fatal_gate_prevents_every_ordinary_callback() {
    let cop = Canary(std::array::from_fn(|_| AtomicUsize::new(0)));
    for case in witnesses::cases() {
        for named in [false, true] {
            for count in &cop.0 {
                count.store(0, Ordering::SeqCst);
            }
            let mut active: Vec<&dyn Cop> = vec![&cop];
            if named {
                active.push(&rustocop::cops::lint::Syntax);
            }
            let report = lint_one(
                Path::new("raw.rb"),
                Path::new("display.rb"),
                Some(&case.source),
                &active,
                &ResolvedConfig::default(),
            );
            assert!(report.internal_errors.is_empty(), "{case:?}");
            assert_eq!(report.corrections_applied, 0);
            assert_eq!(
                report.errors.is_empty(),
                case.valid,
                "{case:?}: {:?}",
                report.errors
            );
            if case.valid {
                assert!(
                    cop.0.iter().all(|c| c.load(Ordering::SeqCst) > 0),
                    "{case:?}"
                );
            } else {
                assert!(report.result.offenses.is_empty(), "{case:?}");
                assert!(
                    cop.0.iter().all(|c| c.load(Ordering::SeqCst) == 0),
                    "{case:?}"
                );
                assert_eq!(report.errors.len(), 1, "{case:?}");
                assert_eq!(report.errors[0].path, Path::new("display.rb"));
                assert!(
                    report.errors[0]
                        .message
                        .contains(&format!("Invalid {}", case.keyword))
                );
                assert_eq!(report.errors[0].message.contains("(Lint/Syntax)"), named);
                let file = SourceFile::new("raw.rb".into(), case.source.clone());
                let (line, col) = file.line_col(case.range.start);
                let position = report.errors[0].position.as_ref().unwrap();
                assert_eq!(*position, (line, col));
                assert_eq!(file.bytes, case.source);
            }
        }
    }
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}

fn cli(root: &Path, args: &[&str], input: Option<&[u8]>, code: i32) -> String {
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
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(code), "{args:?}: {output:?}");
    assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
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

#[test]
fn flow_defaults_cli_original_byte_matrix_is_fatal_or_accepted_without_writes() {
    let dir = project();
    let path = dir.path().join("input.rb");
    for case in witnesses::cases() {
        fs::write(&path, &case.source).unwrap();
        let old = fs::metadata(&path).unwrap();
        for fix in [false, true] {
            let mut args = vec!["--only", "Lint/Syntax,Lint/UnreachableCode"];
            if fix {
                args.push("--fix");
            }
            args.push("input.rb");
            // Valid flow bodies can have ordinary UnreachableCode findings;
            // use Syntax alone so acceptance has an unambiguous exit0 control.
            if case.valid {
                args[1] = "Lint/Syntax";
            }
            let text = cli(dir.path(), &args, None, if case.valid { 0 } else { 2 });
            assert_eq!(text.contains("Invalid "), !case.valid, "{case:?}: {text}");
            assert!(!text.contains("(Lint/UnreachableCode)"), "{case:?}: {text}");
            unchanged(&path, &case.source, &old);
        }
    }
}

#[test]
fn flow_defaults_selectors_strict_config_directives_stdin_and_observers() {
    let dir = project();
    let path = dir.path().join("input.rb");
    for keyword in ["next", "break", "redo"] {
        for prefix in ["rustocop", "rubocop"] {
            let source = format!(
                "# {prefix}:disable all\n# {prefix}:push\n# {prefix}:disable-next Lint/Syntax\ndef f; foo {{ |x = ({keyword}; y)| x }}; end\nfail\nFile.binread(__FILE__)\nRubyVM::InstructionSequence.of(method(:f))\nObjectSpace.each_object(String)\n__LINE__\n__END__\nraw\n"
            );
            fs::write(&path, &source).unwrap();
            let old = fs::metadata(&path).unwrap();
            let mut held = fs::File::open(&path).unwrap();
            for config in [
                "",
                "[cops.Lint]\nenabled = false\n",
                "[cops.\"Lint/Syntax\"]\nenabled = false\n",
                "[cops.\"Lint/Syntax\"]\nexclude = ['*.rb']\n",
            ] {
                fs::write(dir.path().join("rustocop.toml"), config).unwrap();
                for select in [
                    vec![],
                    vec!["--only", "Lint/Syntax"],
                    vec!["--only", "Lint/UnreachableCode,Style/SignalException"],
                    vec!["--except", "Lint/Syntax"],
                    vec!["--except", "Lint,Style,Layout,Naming,Security"],
                ] {
                    for fix in [false, true] {
                        let mut args = select.clone();
                        if fix {
                            args.push("--fix");
                        }
                        args.push("input.rb");
                        let text = cli(dir.path(), &args, None, 2);
                        assert!(text.contains(&format!("Invalid {keyword}")), "{text}");
                        assert!(
                            !text.contains("(Lint/UnreachableCode)") && !text.contains("(Style/"),
                            "{text}"
                        );
                        unchanged(&path, source.as_bytes(), &old);
                    }
                    let mut args = select;
                    args.extend(["--format", "github", "--stdin", "virtual.rb"]);
                    let text = cli(dir.path(), &args, Some(source.as_bytes()), 2);
                    assert!(text.contains("file=virtual.rb,line=4,col=20,"), "{text}");
                    assert!(text.contains(&format!("Invalid {keyword}")), "{text}");
                }
            }
            let mut observed = Vec::new();
            held.read_to_end(&mut observed).unwrap();
            assert_eq!(observed, source.as_bytes());
            // Unsupported grammar/style knobs remain strict config errors,
            // not a way to select another parser or obtain source writes.
            for setting in [
                "TargetRubyVersion = '3.3'",
                "ParserEngine = 'parse.y'",
                "max = 1",
            ] {
                fs::write(
                    dir.path().join("rustocop.toml"),
                    format!("[cops.\"Lint/Syntax\"]\n{setting}\n"),
                )
                .unwrap();
                let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                    .current_dir(dir.path())
                    .args(["--fix", "input.rb"])
                    .output()
                    .unwrap();
                assert_eq!(output.status.code(), Some(2));
                assert!(!output.stderr.is_empty());
                unchanged(&path, source.as_bytes(), &old);
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn flow_defaults_readonly_symlink_rejection_keeps_atomic_peer_correction() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = project();
    let path = dir.path().join("bad.rb");
    for keyword in ["next", "break", "redo"] {
        let source = format!("def f; foo {{ |x = ({keyword}; y)| x }}; end\nfail\n");
        fs::write(&path, &source).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let alias = dir.path().join("alias.rb");
        symlink("bad.rb", &alias).unwrap();
        let peer = dir.path().join("peer.rb");
        fs::write(&peer, "fail\n").unwrap();
        let old = fs::metadata(&path).unwrap();
        let mut held = fs::File::open(&path).unwrap();
        let text = cli(
            dir.path(),
            &[
                "--only",
                "Style/SignalException",
                "--fix",
                "bad.rb",
                "alias.rb",
                "peer.rb",
            ],
            None,
            2,
        );
        assert!(text.contains(&format!("Invalid {keyword}")), "{text}");
        unchanged(&path, source.as_bytes(), &old);
        assert_eq!(fs::read_link(&alias).unwrap(), Path::new("bad.rb"));
        assert_eq!(fs::read(&peer).unwrap(), b"raise\n");
        let mut observed = Vec::new();
        held.read_to_end(&mut observed).unwrap();
        assert_eq!(observed, source.as_bytes());
        fs::remove_file(alias).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    }
}
