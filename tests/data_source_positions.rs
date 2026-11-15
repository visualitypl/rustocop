//! only controlled Ruby main scripts are executed (DATA is not set by load).
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

const CASES: [(&str, &str, &str); 2] = [
    (
        "Style/AmbiguousEndlessMethodDefinition",
        "def f = true if true\n",
        "def f\n  true\nend if true\n",
    ),
    (
        "Style/ClassMethodsDefinitions",
        "class A\n  class << self\n    def one; :one; end\n  end\nend\n",
        "class A\n  def self.one; :one; end\nend\n",
    ),
];

// Setup follows the candidate: ClassMethodsDefinitions' independent closed-
// declaration prefix guard must not be the reason these edits are withheld.
const OBSERVATIONS: [(&str, &str); 12] = [
    ("", "DATA"),
    ("", "::DATA"),
    ("", "Object::DATA"),
    ("", "::Object::DATA"),
    ("Root = Object\n", "Root::DATA"),
    ("Root = Object\n", "Root::Object::DATA"),
    ("Handle = DATA\n", "Handle"),
    ("handle = ::DATA\n", "handle"),
    ("", "Object.const_get(:DATA)"),
    ("", "::Object.const_get('DATA')"),
    ("Root = Object\n", "Root.const_get(:DATA)"),
    (
        "class << Object; alias_method :lookup_data, :const_get; end\n",
        "Object.lookup_data(:DATA)",
    ),
];

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}

fn cli(dir: &Path, args: &[&str], status: i32) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(status), "{args:?}: {output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    output
}

fn unchanged(dir: &Path, source: &[u8], args: &[&str], status: i32) -> Output {
    let path = dir.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let output = cli(dir, args, status);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    output
}

fn observed(candidate: &str, setup: &str, reference: &str) -> String {
    format!(
        "{candidate}{setup}p [{reference}.pos, {reference}.lineno, {reference}.read]\n__END__\nraw\n"
    )
}

#[test]
fn data_paths_withhold_only_edits_with_exact_bytes_and_mtime() {
    let dir = project();
    for (cop, before, _) in CASES {
        for (setup, reference) in OBSERVATIONS {
            for crlf in [false, true] {
                let source = observed(before, setup, reference);
                let bytes = if crlf {
                    // Raw payload is not necessarily UTF-8; code retains BOM/CRLF.
                    let mut bytes =
                        format!("\u{feff}{}", source.replace('\n', "\r\n")).into_bytes();
                    bytes.extend_from_slice(b"\xff\x00\r\n");
                    bytes
                } else {
                    source.into_bytes()
                };
                let out = unchanged(dir.path(), &bytes, &["--only", cop, "--fix", "lib/a.rb"], 1);
                assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
            }
        }
    }
}

#[test]
fn individual_pos_lineno_and_read_observations_all_withhold() {
    let dir = project();
    for (cop, before, _) in CASES {
        for selector in ["pos", "lineno", "read"] {
            let source = format!("{before}p DATA.{selector}\n__END__\nraw\n");
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                1,
            );
        }
    }
}

#[test]
fn ordinary_payload_and_inert_data_spellings_still_correct_exactly() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, before, after) in CASES {
        for crlf in [false, true] {
            let decorate = |candidate: &str| {
                let source = format!(
                    "# DATA.pos Object.const_get(:DATA)\n{candidate}p 'DATA.lineno'\n__END__\nDATA.read\n"
                );
                let mut bytes = if crlf {
                    format!("\u{feff}{}", source.replace('\n', "\r\n")).into_bytes()
                } else {
                    source.into_bytes()
                };
                bytes.extend_from_slice(b"\xff\x00raw");
                bytes
            };
            fs::write(&path, decorate(before)).unwrap();
            cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
            assert_eq!(fs::read(&path).unwrap(), decorate(after));
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
    }
}

#[test]
fn real_selectors_config_directives_and_excludes_keep_the_guard() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let config = dir.path().join("rustocop.toml");
    let registry = rustocop::cops::Registry::new();
    for (cop, before, after) in CASES {
        let implementation = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(implementation.safe_fixes());
        assert_eq!(implementation.default_enabled(), cop == CASES[0].0);
        let source = observed(before, "", "DATA");
        for setting in [
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            fs::write(
                &config,
                // Isolate the older consumers; SingleLineMethods independently
                // reports these one-line bodies while sharing the same DATA guard.
                // PredicateMethod now independently reports the boolean tail.
                format!("[cops.\"Naming/PredicateMethod\"]\nenabled = false\n[cops.Lint]\nenabled = false\n[cops.\"Style/Documentation\"]\nenabled = false\n[cops.\"Style/SingleLineMethods\"]\nenabled = false\n{setting}"),
            ).unwrap();
            unchanged(dir.path(), source.as_bytes(), &["--fix", "lib/a.rb"], 0);
            // --only force-enables but must not bypass DATA safety.
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                1,
            );
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--except", cop, "--fix", "lib/a.rb"],
                0,
            );
        }
        fs::write(
            &config,
            format!("[cops.\"Naming/PredicateMethod\"]\nenabled = false\n[cops.Lint]\nenabled = false\n[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        unchanged(dir.path(), source.as_bytes(), &["--fix", "lib/a.rb"], 1);
        // Prove configuration really activated the consumer, independently of DATA.
        fs::write(&path, before).unwrap();
        cli(dir.path(), &["--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
        fs::write(&config, "").unwrap();
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", cop, "lib/a.rb"],
            1,
        );
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                for directive in ["disable", "disable-next"] {
                    let suppressed = format!("# {prefix}:{directive} {target}\n{source}");
                    let out = unchanged(
                        dir.path(),
                        suppressed.as_bytes(),
                        &["--only", cop, "--fix", "lib/a.rb"],
                        0,
                    );
                    assert!(!String::from_utf8_lossy(&out.stdout).contains(cop));
                }
            }
        }
        for setting in [
            "exclude = [\"lib/**\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = [\"lib/**\"]\n"),
        ] {
            fs::write(&config, setting).unwrap();
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                0,
            );
        }
        fs::write(&config, "").unwrap();
    }
}

fn ruby_available() -> bool {
    Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn execute(path: &Path) -> Vec<u8> {
    let output = Command::new("ruby").arg("-W0").arg(path).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    output.stdout
}

fn expected_observation(source: &str) -> Vec<u8> {
    let start = source.find("__END__\n").unwrap() + "__END__\n".len();
    let line = source[..start].bytes().filter(|b| *b == b'\n').count();
    format!("[{start}, {line}, \"raw\\n\"]\n").into_bytes()
}

#[test]
fn ruby_main_scripts_preserve_data_positions_where_naive_edits_do_not() {
    if !ruby_available() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, before, after) in CASES {
        for (setup, reference) in OBSERVATIONS {
            let source = observed(before, setup, reference);
            fs::write(&path, &source).unwrap();
            let original = execute(&path);
            assert_eq!(original, expected_observation(&source));
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 1);
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            assert_eq!(execute(&path), original);
            // A real witness, not just an unchanged-file tautology: applying the
            // formerly offered edit keeps the payload but changes pos AND lineno.
            let naive = observed(after, setup, reference);
            fs::write(&path, &naive).unwrap();
            let changed = execute(&path);
            assert_eq!(changed, expected_observation(&naive));
            assert_ne!(changed, original);
            assert_ne!(source.find("__END__"), naive.find("__END__"));
            assert_ne!(source.lines().count(), naive.lines().count());
        }
    }
}

#[test]
fn ruby_main_scripts_with_unobserved_payload_still_correct() {
    if !ruby_available() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, before, after) in CASES {
        let decorate = |candidate| format!("{candidate}p :ok\n__END__\nraw\n");
        fs::write(&path, decorate(before)).unwrap();
        let original = execute(&path);
        assert_eq!(original, b":ok\n");
        cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read(&path).unwrap(), decorate(after).as_bytes());
        assert_eq!(execute(&path), original);
    }
}
