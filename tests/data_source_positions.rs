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
            assert_eq!(
                fs::read(&path).unwrap(),
                crate::support::lf::lf(decorate(after))
            );
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
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
