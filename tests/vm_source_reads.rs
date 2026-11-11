//! Execute only these controlled fixtures.
//! Host compile_file retains lines; native 3.4.1 compile_file returns nil, while
//! compile_file_prism and AST.parse_file retain lines. Do not conflate evidence.
#[macro_use]
#[path = "support/parallel_rows.rs"]
mod parallel_rows;
use std::{fs, path::Path, process::Command};

const CASES: &[(&str, &str, &str)] = &[
    ("Style/StringLiterals", "x = 'hi'\n", "x = \"hi\"\n"),
    ("Layout/TrailingWhitespace", "x = 1  \n", "x = 1\n"),
    (
        "Style/ClassMethods",
        "class A\n  def A.f; end\nend\n",
        "class A\n  def self.f; end\nend\n",
    ),
];

fn runtimes() -> Vec<String> {
    let mut runtimes = vec!["ruby".into()];
    if let Ok(native) = std::env::var("RUSTOCOP_RUBY34") {
        let version = Command::new(&native).arg("-v").output().unwrap();
        assert!(version.status.success());
        assert!(String::from_utf8_lossy(&version.stdout).starts_with("ruby 3.4."));
        runtimes.push(native);
    }
    runtimes
}

fn run(runtime: &str, path: &Path) -> Vec<u8> {
    let output = Command::new(runtime)
        .arg("--disable-gems")
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{runtime}: {output:?}");
    assert!(output.stderr.is_empty(), "{runtime}: {output:?}");
    output.stdout
}

fn cli(dir: &Path, cop: &str, fixing: bool, path: &str, status: i32) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
    command.current_dir(dir).args(["--only", cop]);
    if fixing {
        command.arg("--fix");
    }
    let output = command.arg(path).output().unwrap();
    assert_eq!(output.status.code(), Some(status), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn retained_vm_lines_preserve_runtime_bytes_mtime_and_converging_siblings() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    for runtime in runtimes() {
        let native = runtime != "ruby";
        let witnesses = AtomicUsize::new(0);
        let nil_controls = AtomicUsize::new(0);
        for (factory, selector, args) in [
            ("RubyVM::InstructionSequence", "compile_file", "__FILE__"),
            (
                "RubyVM::InstructionSequence",
                "compile_file_prism",
                "__FILE__",
            ),
            (
                "RubyVM::AbstractSyntaxTree",
                "parse_file",
                "__FILE__, keep_script_lines: true",
            ),
        ] {
            if !native && selector == "compile_file_prism" {
                continue; // Ruby 3.2 has no Prism compilation API.
            }
            parallel_rows!(for acquisition in [
                format!("{factory}.{selector}({args})"),
                format!("(VMFactory = ::{factory}; VMFactory.{selector}({args}))"),
                format!("{factory}.public_send(:public_send, '{selector}', {args})"),
                format!("{factory}.method(:{selector}).call({args})"),
                format!(
                    "(class << {factory}; alias acquire {selector}; end; {factory}.acquire({args}))"
                ),
                format!("(def acquire; {factory}.{selector}({args}); end; acquire)"),
                format!(":{selector}.to_proc.call({factory}, {args})"),
            ] => {
                let dir = tempfile::tempdir().unwrap();
                fs::create_dir(dir.path().join(".git")).unwrap();
                fs::write(dir.path().join("rustocop.toml"), "").unwrap();
                let path = dir.path().join("case.rb");
                let sibling = dir.path().join("safe.rb");
                // Each acquisition is paired with direct, reflective and
                // escaped/aliased retained getters, not just compiler spelling.
                for getter in [
                    "p retained.script_lines",
                    "p retained.public_send(:public_send, 'script_lines')",
                    "reader = retained.method(:script_lines); p reader.call",
                    "p :\"script\\x5flines\".to_proc.call(retained)",
                    "class << retained; alias lines script_lines; end; p retained.lines",
                    "def later(object); object.script_lines; end; p later(retained)",
                ] {
                    for &(cop, prefix, canonical) in CASES {
                        for crlf in [false, true] {
                            let observer = format!(
                                "RubyVM.keep_script_lines = true\nretained = {acquisition}\n{getter}\n"
                            );
                            let encode = |s: String| {
                                if crlf {
                                    format!("\u{feff}{}", s.replace('\n', "\r\n"))
                                } else {
                                    s
                                }
                            };
                            let source = encode(format!("{prefix}{observer}"));
                            fs::write(&path, &source).unwrap();
                            let before = run(&runtime, &path);
                            fs::write(&path, encode(format!("{canonical}{observer}"))).unwrap();
                            let naive = run(&runtime, &path);
                            if native && selector == "compile_file" {
                                assert_eq!(before, b"nil\n");
                                assert_eq!(naive, before);
                                nil_controls.fetch_add(1, Ordering::Relaxed);
                            } else {
                                assert_ne!(naive, before, "{runtime}: {source}");
                                assert!(before.starts_with(b"["));
                                witnesses.fetch_add(1, Ordering::Relaxed);
                            }
                            fs::write(&path, &source).unwrap();
                            fs::write(&sibling, prefix).unwrap();
                            let mtime = fs::metadata(&path).unwrap().modified().unwrap();
                            for fixing in [false, true, true] {
                                let report = cli(dir.path(), cop, fixing, ".", 1);
                                assert!(report.contains(cop), "{report}");
                                assert_eq!(fs::read(&path).unwrap(), source.as_bytes(), "{source}");
                                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
                            }
                            assert_eq!(fs::read_to_string(&sibling).unwrap(), canonical);
                            assert_eq!(run(&runtime, &path), before);
                            cli(dir.path(), cop, true, "safe.rb", 0);
                        }
                    }
                }
            });
        }
        let witnesses = witnesses.load(Ordering::Relaxed);
        let nil_controls = nil_controls.load(Ordering::Relaxed);
        eprintln!(
            "{runtime}: {witnesses} raw VM witnesses; {nil_controls} compile_file nil controls"
        );
        assert_eq!(witnesses, 504);
        assert_eq!(nil_controls, if native { 252 } else { 0 });
    }
}

#[test]
fn pre_entry_retained_ast_getters_preserve_runtime_bytes_mtime_and_safe_siblings() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("case.rb");
    let sibling = dir.path().join("safe.rb");
    let launcher = dir.path().join("launcher.rb");
    // Only this controlled launcher creates the object. The linted script has
    // neither a VM constant nor a path/retention acquisition to mask the getter.
    fs::write(
        &launcher,
        "$retained = RubyVM::AbstractSyntaxTree.parse_file(ARGV[0], keep_script_lines: true)\n$raw = $retained.source\nload ARGV[0]\n",
    )
    .unwrap();
    for runtime in runtimes() {
        let observe = || {
            let output = Command::new(&runtime)
                .arg("--disable-gems")
                .arg(&launcher)
                .arg(&path)
                .output()
                .unwrap();
            assert!(output.status.success(), "{runtime}: {output:?}");
            assert!(output.stderr.is_empty(), "{runtime}: {output:?}");
            assert_ne!(output.stdout, b"nil\n");
            output.stdout
        };
        let mut witnesses = 0;
        for getter in ["source", "script_lines"] {
            let mut observers = vec![
                format!("p $retained.{getter}"),
                format!("p $retained.public_send(:public_send, '{getter}')"),
                format!("reader = $retained.method(:{getter}); p reader.call"),
                format!("p :{getter}.to_proc.call($retained)"),
                format!("p [$retained].map(&:{getter})"),
                format!("class << $retained; alias raw {getter}; end; p $retained.raw"),
                format!("class << $retained; alias_method :raw, '{getter}'; end; p $retained.raw"),
                format!("def later(object); object.{getter}; end; p later($retained)"),
                format!("p ($retained.{getter} ||= nil)"),
                format!("p $retained.method(\"{getter}\".sub(/./) {{ |c| c }}).call"),
            ];
            if getter == "source" {
                observers.extend([
                    "class Regexp; def source; $raw; end; end; p /x/.source".into(),
                    "class Regexp; def hidden; $raw; end; alias source hidden; end; p /x/.source"
                        .into(),
                ]);
            }
            for observer in observers {
                for &(cop, prefix, canonical) in CASES {
                    for crlf in [false, true] {
                        let encode = |s: String| {
                            if crlf {
                                format!("\u{feff}{}", s.replace('\n', "\r\n"))
                            } else {
                                s
                            }
                        };
                        let source = encode(format!("{prefix}{observer}\n"));
                        fs::write(&path, &source).unwrap();
                        let before = observe();
                        fs::write(&path, encode(format!("{canonical}{observer}\n"))).unwrap();
                        assert_ne!(observe(), before, "{runtime}: {source}");
                        witnesses += 1;
                        fs::write(&path, &source).unwrap();
                        fs::write(&sibling, prefix).unwrap();
                        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
                        // Limit selection to subjects: never correct the launcher.
                        for fixing in [false, true, true] {
                            let report = cli(dir.path(), cop, fixing, "case.rb", 1);
                            assert!(report.contains(cop), "{report}");
                            assert_eq!(fs::read(&path).unwrap(), source.as_bytes(), "{source}");
                            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
                        }
                        cli(dir.path(), cop, true, "safe.rb", 0);
                        assert_eq!(fs::read_to_string(&sibling).unwrap(), canonical);
                        let sibling_mtime = fs::metadata(&sibling).unwrap().modified().unwrap();
                        cli(dir.path(), cop, true, "safe.rb", 0);
                        assert_eq!(
                            fs::metadata(&sibling).unwrap().modified().unwrap(),
                            sibling_mtime
                        );
                        assert_eq!(observe(), before);
                    }
                }
            }
        }
        assert_eq!(witnesses, 132);
        eprintln!("{runtime}: {witnesses} pre-entry retained AST raw witnesses");
    }
}

#[test]
fn vm_metadata_and_original_payload_controls_keep_canonical_runtime_fixes() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("case.rb");
    for runtime in runtimes() {
        for observer in [
            "p RubyVM::InstructionSequence.name",
            "p RubyVM::AbstractSyntaxTree.class",
            "p [:script_lines, :compile_file, :parse_file].map(&:to_s)",
            "p [:source, :script_lines].map(&:to_s)",
            "p /retained.source/.source",
            "p ((%r(retained.source))).source",
            "p \"retained.source\"",
            "# p retained.source\np :ok",
            "p \"RubyVM.keep_script_lines = true; retained.script_lines\"",
            "# RubyVM::InstructionSequence.compile_file(__FILE__).script_lines\np :ok",
            "p File.stat(__FILE__).size",
            "p [__FILE__].map(&:itself)",
            "p DATA.each.to_a\n__END__\npayload",
            "DATA.rewind; p DATA.closed?\n__END__\npayload",
            "ARGV << __FILE__; p ARGF.argv",
        ] {
            let source = format!("x = 'hi'\n{observer}\n");
            fs::write(&path, &source).unwrap();
            let before = run(&runtime, &path);
            cli(dir.path(), "Style/StringLiterals", true, "case.rb", 0);
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                source.replace("'hi'", "\"hi\"")
            );
            assert_eq!(run(&runtime, &path), before, "{runtime}: {source}");
            cli(dir.path(), "Style/StringLiterals", true, "case.rb", 0);
        }
    }
}
