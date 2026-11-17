//! DATA exists only for a main script: execute controlled Ruby, never `load`.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

const PRELUDE: &str = "def identity(value = nil); value; end\n";
const CASES: [(&str, &str, &str); 3] = [
    (
        "Style/MethodCallWithArgsParentheses",
        "x = identity 1\n",
        "x = identity(1)\n",
    ),
    (
        "Style/MethodCallWithoutArgsParentheses",
        "x = identity()\n",
        "x = identity\n",
    ),
    (
        "Style/NestedParenthesizedCalls",
        "x = identity(identity 1)\n",
        "x = identity(identity(1))\n",
    ),
];
const REFERENCES: [(&str, &str); 13] = [
    ("", "DATA"),
    ("", "(DATA ||= nil)"),
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
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    out
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], status: i32) -> Output {
    let path = dir.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let out = cli(dir, args, status);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    out
}
fn observed(candidate: &str, setup: &str, reference: &str, selector: &str) -> String {
    format!(
        "{PRELUDE}{candidate}{setup}p [{reference}.{selector}, {reference}.lineno, {reference}.read]\n__END__\nraw\n"
    )
}
fn ruby_available() -> bool {
    Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}
fn execute(path: &Path) -> Vec<u8> {
    let out = Command::new("ruby")
        .args(["-W0"])
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    out.stdout
}
#[test]
fn no_data_and_inert_spellings_still_correct_exactly_and_stabilize() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, before, after) in CASES {
        for payload in [false, true] {
            for crlf in [false, true] {
                let decorate = |candidate: &str| {
                    let source = format!(
                        "{PRELUDE}# DATA.pos Object.const_get(:DATA)\n{candidate}p 'DATA.tell'\n{}",
                        if payload { "__END__\nDATA.read\n" } else { "" }
                    );
                    let mut bytes = if crlf {
                        format!("\u{feff}{}", source.replace('\n', "\r\n")).into_bytes()
                    } else {
                        source.into_bytes()
                    };
                    if payload {
                        bytes.extend_from_slice(b"\xff\x00raw");
                    }
                    bytes
                };
                fs::write(&path, decorate(before)).unwrap();
                let original = ruby_available().then(|| execute(&path));
                cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
                assert_eq!(fs::read(&path).unwrap(), decorate(after));
                if let Some(original) = original {
                    assert_eq!(execute(&path), original);
                }
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            }
        }
    }
}

#[test]
fn forwarding_and_super_data_safeguards_cover_the_same_acquisitions() {
    let dir = project();
    for (cop, candidate) in [
        (
            "Style/ArgumentsForwarding",
            "def f(*args, **kwargs, &block); identity(*args, **kwargs, &block); end\n",
        ),
        ("Style/SuperArguments", "def f(a); super(a); end\n"),
        ("Style/SuperWithArgsParentheses", "def f(a); super a; end\n"),
    ] {
        for (setup, reference) in REFERENCES {
            let source = observed(candidate, setup, reference, "method(:tell).call");
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                1,
            );
        }
    }
}
