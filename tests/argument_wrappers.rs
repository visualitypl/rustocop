//! public pipeline, source bytes and controlled Ruby runtime evidence.
use crate::support::cli::project;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const MERGE: &str = "Style/KeywordArgumentsMerging";
const BLOCK: &str = "Style/ExplicitBlockArgument";
const REDUNDANT: &str = "Style/RedundantArgument";
const CASES: [(&str, &str, &str); 3] = [
    (MERGE, "foo(**{}.merge(y: 1))\n", "foo(**{}, y: 1)\n"),
    (
        BLOCK,
        "def f; if block_given?; [1].each { |x| yield x }; end; end\n",
        "def f(&block); if block_given?; [1].each(&block); end; end\n",
    ),
    (REDUNDANT, "foo.sum(0)\n", "foo.sum(0)\n"),
];
fn cli(dir: &Path, args: &[&str], status: i32) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    out
}
#[test]
fn defaults_selectors_config_and_detection_only_never_write() {
    let dir = project();
    let path = dir.path().join("x.rb");
    for (cop, before, after) in CASES {
        fs::write(&path, before).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        assert!(String::from_utf8_lossy(&cli(dir.path(), &["x.rb"], 1).stdout).contains(cop));
        cli(dir.path(), &["--only", cop, "x.rb"], 1);
        cli(
            dir.path(),
            &["--only", cop, "--except", cop, "--fix", "x.rb"],
            0,
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        for config in [
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .arg("x.rb")
                .output()
                .unwrap();
            assert!(matches!(out.status.code(), Some(0 | 1)));
            assert!(!String::from_utf8_lossy(&out.stdout).contains(cop));
            let status = i32::from(before == after);
            cli(dir.path(), &["--only", cop, "--fix", "x.rb"], status);
            assert_eq!(fs::read_to_string(&path).unwrap(), after);
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            cli(dir.path(), &["--only", cop, "--fix", "x.rb"], status);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            fs::write(&path, before).unwrap();
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&cli(dir.path(), &["x.rb"], 1).stdout).contains(cop));
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn stdin_is_diagnostic_only_and_fix_never_writes_its_filename_hint() {
    use std::{io::Write, process::Stdio};
    let dir = project();
    let path = dir.path().join("x.rb");
    for (cop, before, _) in CASES {
        fs::write(&path, before).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", cop, "--stdin", "x.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(before.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains(cop));
        cli(dir.path(), &["--only", cop, "--fix", "--stdin", "x.rb"], 2);
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn encoding_bom_crlf_payload_and_explicit_line_values_survive() {
    let dir = project();
    let path = dir.path().join("x.rb");
    for (cop, before, after) in CASES {
        for header in [
            b"\xef\xbb\xbf# coding: UTF-8\r\n".as_slice(),
            b"# coding: ISO-8859-1\r\n# caf\xe9\r\n",
        ] {
            let suffix = b"p __LINE__\r\n__END__\r\nraw\xff\r\n";
            let before = [header, before.replace('\n', "\r\n").as_bytes(), suffix].concat();
            let after = [header, after.replace('\n', "\r\n").as_bytes(), suffix].concat();
            fs::write(&path, &before).unwrap();
            cli(
                dir.path(),
                &["--only", cop, "--fix", "x.rb"],
                i32::from(cop == REDUNDANT),
            );
            assert_eq!(
                fs::read(&path).unwrap(),
                crate::support::lf::fixed(&before, &after)
            );
        }
    }
}
#[test]
fn signature_edit_reaches_a_disabled_line_as_in_rubocop() {
    let dir = project();
    let path = dir.path().join("x.rb");
    for prefix in ["rubocop", "rustocop"] {
        // The offense (line 2) is enabled, so, as in RuboCop, its correction
        // also edits the signature on the disabled line 1.
        let before = format!(
            "def f # {prefix}:disable {BLOCK}\n  if block_given?; [1].each {{ |x| yield x }}; end\nend\nfoo(**{{}}.merge(y: 1))\n"
        );
        let after = format!(
            "def f(&block) # {prefix}:disable {BLOCK}\n  if block_given?; [1].each(&block); end\nend\nfoo(**{{}}, y: 1)\n"
        );
        fs::write(&path, &before).unwrap();
        cli(
            dir.path(),
            &["--only", &format!("{BLOCK},{MERGE}"), "--fix", "x.rb"],
            0,
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
    }
    let before = b"# coding: ISO-8859-1\r\ndef f\r\n # caf\xe9\r\n if block_given?; [1].each { |x| yield x }; end\r\nend\r\nfoo(**{}.merge(y: 1))\r\n";
    let after = b"# coding: ISO-8859-1\r\ndef f\r\n # caf\xe9\r\n if block_given?; [1].each { |x| yield x }; end\r\nend\r\nfoo(**{}, y: 1)\r\n";
    fs::write(&path, before).unwrap();
    cli(
        dir.path(),
        &["--only", &format!("{BLOCK},{MERGE}"), "--fix", "x.rb"],
        1,
    );
    assert_eq!(fs::read(&path).unwrap(), crate::support::lf::lf(after));
}
fn ruby_available() -> bool {
    Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}
fn ruby(path: &Path) -> Output {
    Command::new("ruby").arg("-W0").arg(path).output().unwrap()
}
fn observed_fix(cop: &str, before: &str, after: &str, status: i32) -> Vec<u8> {
    observed_fix_with_driver(cop, before, after, status, "")
}
// Drivers are controlled Ruby observations, not part of the edited source.
// In particular forwarding an unknown driver Proc intentionally ends the shared
// whole-file source-observer proof if it is placed in the implementation file.
fn observed_fix_with_driver(
    cop: &str,
    before: &str,
    after: &str,
    status: i32,
    driver: &str,
) -> Vec<u8> {
    let dir = project();
    let path = dir.path().join("x.rb");
    fs::write(&path, before).unwrap();
    let runner = if driver.is_empty() {
        path.clone()
    } else {
        let runner = dir.path().join("driver.rb");
        fs::write(
            &runner,
            format!("load File.expand_path('x.rb', __dir__)\n{driver}"),
        )
        .unwrap();
        runner
    };
    let original = ruby(&runner);
    assert!(original.status.success(), "{original:?}\n{before}");
    cli(dir.path(), &["--only", cop, "--fix", "x.rb"], status);
    assert_eq!(fs::read_to_string(&path).unwrap(), after);
    let fixed = ruby(&runner);
    assert_eq!(original.status.code(), fixed.status.code());
    assert_eq!(original.stdout, fixed.stdout, "{before}");
    assert_eq!(original.stderr, fixed.stderr);
    original.stdout
}
#[test]
fn runtime_literal_merge_preserves_keyword_insertion_overwrites_and_positional_channel() {
    if !ruby_available() {
        eprintln!("Ruby unavailable; runtime probe skipped");
        return;
    }
    let before = "def take(*a, **k); p [a,k.to_a]; end\ntake(:pos, **{a: 1, b: 2}.merge(a: 3, c: 4))\ntake({}, **{}.merge({z: 1}, {a: 2}))\n";
    let after = before
        .replace("{a: 1, b: 2}.merge(a: 3, c: 4)", "{a: 1, b: 2}, a: 3, c: 4")
        .replace("{}.merge({z: 1}, {a: 2})", "{}, z: 1, a: 2");
    assert_eq!(
        observed_fix(MERGE, before, &after, 0),
        b"[[:pos], [[:a, 3], [:b, 2], [:c, 4]]]\n[[{}], [[:z, 1], [:a, 2]]]\n"
    );
}
#[test]
fn runtime_guarded_each_matches_block_absence_proc_lambda_arity_break_return_and_exceptions() {
    if !ruby_available() {
        eprintln!("Ruby unavailable; runtime probe skipped");
        return;
    }
    let before = r#"def f
  if block_given?; [1,2].each { |x| yield x }; end
end
p f
[nil, ->(x) { x*2 }, -> { :zero }, ->(x,y) { [x,y] }, ->((a,b)) { [a,b] }, ->(**k) { k }, ->(x:, **k) { [x,k] }, proc { |a,b| [a,b] }, proc { |*a, **k| [a,k] }].each do |callback|
  begin
    p f(&callback)
  rescue ArgumentError => e
    p [e.class, e.message]
  end
end
p(f { |x| break [:break,x] })
def returning
  f { |x| return [:return,x] }
end
p returning
p catch(:done) { f { |x| throw :done, x } }
begin
  f { |x| raise 'expected' }
rescue RuntimeError => e
  p [e.class, e.message]
end
p __LINE__
"#;
    let (before, driver) = before.split_once("p f\n").unwrap();
    let after = before
        .replace("def f\n", "def f(&block)\n")
        .replace(".each { |x| yield x }", ".each(&block)");
    let out = observed_fix_with_driver(BLOCK, before, &after, 0, &format!("p f\n{driver}"));
    assert!(String::from_utf8_lossy(&out).contains("wrong number of arguments"));
    assert!(String::from_utf8_lossy(&out).contains("[:return, 1]"));
    assert!(String::from_utf8_lossy(&out).contains("[:break, 1]"));
}
#[test]
fn current_style_layout_and_available_incompatible_peer_stabilize_without_cycles() {
    let dir = project();
    let path = dir.path().join("x.rb");
    let before =
        "def f\n  if block_given?; [1].each { |x| yield x }; end\nend\nfoo(**{}.merge(y: 1))\n";
    let registry = rustocop::cops::Registry::new();
    let mut peers = format!("{BLOCK},{MERGE}");
    for peer in [
        "Lint/UnusedMethodArgument",
        "Style/LambdaCall",
        "Style/TrailingCommaInArguments",
    ] {
        if registry.has_cop(peer) {
            peers.push(',');
            peers.push_str(peer);
        } else {
            eprintln!("{peer} unavailable; composition activates automatically when registered");
        }
    }
    let selections = [peers, "Style,Layout".into()];
    for only in selections {
        fs::write(&path, before).unwrap();
        let mut seen = std::collections::HashSet::new();
        for pass in 0..8 {
            let bytes = fs::read(&path).unwrap();
            assert!(
                seen.insert(bytes.clone()),
                "cycle after {pass} passes: {only}"
            );
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", &only, "--fix", "x.rb"])
                .output()
                .unwrap();
            assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
            if fs::read(&path).unwrap() == bytes {
                break;
            }
            assert!(pass < 7, "did not stabilize in eight explicit passes");
        }
        let fixed = fs::read_to_string(&path).unwrap();
        assert!(!fixed.contains("yield"), "{only}: {fixed}");
        assert!(!fixed.contains("merge"), "{only}: {fixed}");
    }
    // No explicit call or trailing comma is eligible; signature/use edits form
    // one envelope, so any future incompatible overlap defers them together.
}
