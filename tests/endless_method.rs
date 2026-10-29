//! controlled fixtures only; never execute corpus code.
use rustocop::cops::Registry;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const COP: &str = "Style/EndlessMethod";
const BEFORE: &str = "def f = [\n  1]\n";
const AFTER: &str = "def f\n  [\n  1]\nend\n";
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
    output
}
fn fix(source: &str, only: &[&str]) -> Vec<u8> {
    let registry = Registry::new();
    assert_eq!(
        registry
            .cops()
            .iter()
            .filter(|c| only.contains(&c.name()))
            .count(),
        only.len()
    );
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", &only.join(","), "--fix", "lib/a.rb"])
        .output()
        .unwrap();
    assert!(matches!(output.status.code(), Some(0 | 1)), "{output:?}");
    fs::read(path).unwrap()
}
#[test]
fn default_selectors_config_no_write_and_canonical_fix_are_real() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let registry = Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled() && cop.safe_fixes());
    fs::write(&path, BEFORE).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    assert!(String::from_utf8_lossy(&cli(dir.path(), &["lib/a.rb"], 1).stdout).contains(COP));
    cli(dir.path(), &["--only", COP, "lib/a.rb"], 1);
    assert_eq!(fs::read(&path).unwrap(), BEFORE.as_bytes());
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    for config in [
        format!("[cops.\"{COP}\"]\nenabled = false\n"),
        "[cops.Style]\nenabled = false\n".to_string(),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        cli(dir.path(), &["--fix", "lib/a.rb"], 0);
        cli(
            dir.path(),
            &["--only", COP, "--except", COP, "--fix", "lib/a.rb"],
            0,
        );
        assert_eq!(fs::read(&path).unwrap(), BEFORE.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
    // --only force-enables a disabled cop; no new public configuration knobs.
    cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
    assert_eq!(fs::read(&path).unwrap(), AFTER.as_bytes());
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    fs::write(&path, BEFORE).unwrap();
    fs::write(
        dir.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
    )
    .unwrap();
    cli(dir.path(), &["--fix", "lib/a.rb"], 0);
    assert_eq!(fs::read(&path).unwrap(), AFTER.as_bytes());
}
#[test]
fn directives_excludes_errors_and_uncertain_edits_do_not_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for config in [
        "exclude = [\"lib/**\"]\n".to_string(),
        format!("[cops.\"{COP}\"]\nexclude = [\"lib/**\"]\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        fs::write(&path, BEFORE).unwrap();
        cli(
            &dir.path().join("lib"),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), BEFORE);
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let mut cases = vec![
        ("def f = [\n  __LINE__]\n".to_string(), 1),
        ("def f = [\n  1]\np __LINE__\n".to_string(), 1),
        (
            "def f = [\n  1]\np errors.map(&:full_message)\n".to_string(),
            1,
        ),
        ("def f = [\n  1".to_string(), 2),
        ("def f=(x) = [\n  x]\n".to_string(), 2),
        ("def []=(x, y) = [\n  y]\n".to_string(), 2),
        ("def f = 1\n".to_string(), 0),
    ];
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "Style", "all"] {
            for directive in ["disable", "disable-next"] {
                cases.push((format!("# {prefix}:{directive} {target}\n{BEFORE}"), 0));
            }
        }
        cases.push((format!("def f = [\n  1] # {prefix}:disable {COP}\n"), 0));
        // A directive for a different rule still prevents line-scope changes.
        cases.push((format!("def f = [ # {prefix}:disable Layout\n  1]\n"), 1));
    }
    for (source, status) in cases {
        fs::write(&path, &source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], status);
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
    // Disabled and enabled definitions in the same file remain independent.
    let source = format!("# rustocop:disable {COP}\n{BEFORE}# rustocop:enable {COP}\n{BEFORE}");
    fs::write(&path, &source).unwrap();
    cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        format!("# rustocop:disable {COP}\n{BEFORE}# rustocop:enable {COP}\n{AFTER}")
    );
}
#[test]
fn encoding_bom_crlf_data_and_stdin_contract() {
    use std::{io::Write, process::Stdio};
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (before, after) in [
        (
            b"\xef\xbb\xbfdef f() = [\r\n  1]\r\n__END__\r\n\xff\x00raw\r\n".as_slice(),
            b"\xef\xbb\xbfdef f\r\n  [\r\n  1]\r\nend\r\n__END__\r\n\xff\x00raw\r\n".as_slice(),
        ),
        (
            b"# coding: ISO-8859-1\r\n# caf\xe9\r\ndef f() = [\r\n  1]\r\n",
            b"# coding: ISO-8859-1\r\n# caf\xe9\r\ndef f\r\n  [\r\n  1]\r\nend\r\n",
        ),
        // A non-UTF-8 body cannot be represented in Correction's String.
        (
            b"# coding: ISO-8859-1\ndef f = \"caf\xe9\nvalue\"\n",
            b"# coding: ISO-8859-1\ndef f = \"caf\xe9\nvalue\"\n",
        ),
    ] {
        fs::write(&path, before).unwrap();
        cli(
            dir.path(),
            &["--only", COP, "--fix", "lib/a.rb"],
            if before == after { 1 } else { 0 },
        );
        assert_eq!(fs::read(&path).unwrap(), after);
    }
    fs::write(&path, BEFORE).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", COP, "--stdin", "lib/a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(BEFORE.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains(COP));
    cli(
        dir.path(),
        &["--only", COP, "--fix", "--stdin", "lib/a.rb"],
        2,
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), BEFORE);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
#[test]
fn incompatible_peers_and_layout_use_deterministic_atomic_single_pass_edits() {
    let only = [
        COP,
        "Style/GuardClause",
        "Style/MethodCallWithArgsParentheses",
        "Style/MethodDefParentheses",
        "Style/StringLiterals",
        "Style/HashSyntax",
        "Style/RedundantReturn",
        "Style/Semicolon",
        "Layout/TrailingWhitespace",
    ];
    let before =
        "def f() = if ready\n    consume 'x', {:key => 1};  \n  end\ndef g arg; return arg; end\n";
    let first =
        "def f\n  if ready\n    consume 'x', {:key => 1};  \n  end\nend\ndef g(arg); arg; end\n";
    assert_eq!(fix(before, &only), first.as_bytes());
    let mut reverse = only;
    reverse.reverse();
    assert_eq!(fix(before, &reverse), first.as_bytes());
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, before).unwrap();
    let selector = only.join(",");
    let report = cli(dir.path(), &["--only", &selector, "lib/a.rb"], 1);
    let report = String::from_utf8_lossy(&report.stdout);
    for name in [
        COP,
        "Style/MethodCallWithArgsParentheses",
        "Style/MethodDefParentheses",
        "Layout/TrailingWhitespace",
    ] {
        assert!(report.contains(name), "{name}: {report}");
    }
    cli(dir.path(), &["--only", &selector, "--fix", "lib/a.rb"], 1);
    assert_eq!(fs::read_to_string(&path).unwrap(), first);
    // GuardClause deliberately exempts endless definitions; it becomes active
    // on the expanded ordinary definition and corrects on the next explicit pass.
    let report = cli(dir.path(), &["--only", &selector, "lib/a.rb"], 1);
    assert!(String::from_utf8_lossy(&report.stdout).contains("Style/GuardClause"));
    let mut current = first.as_bytes().to_vec();
    let mut stabilized = false;
    for _ in 0..4 {
        let next = fix(std::str::from_utf8(&current).unwrap(), &only);
        assert_eq!(ruby_prism::parse(&next).errors().count(), 0);
        if next == current {
            stabilized = true;
            break;
        }
        current = next;
    }
    assert!(
        stabilized,
        "curated fixture must stabilize within five explicit passes"
    );
    assert_eq!(
        current,
        b"def f\n  return unless ready\n    consume(\"x\", {key: 1})\n\nend\ndef g(arg); arg; end\n"
    );
}
fn ruby_available() -> bool {
    Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}
fn runtime_case(before: &str, after: &str, driver: &str) -> Vec<u8> {
    let fixed = fix(before, &[COP]);
    assert_eq!(fixed, after.as_bytes());
    assert_eq!(
        fix(after, &[COP]),
        fixed,
        "canonical corrections stabilize in one pass"
    );
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let execute = || {
        let mut command = Command::new("ruby");
        command.arg("-W0");
        if !driver.is_empty() {
            command.args(["-e", driver]);
        }
        let out = command.arg(&path).output().unwrap();
        assert!(out.status.success(), "{out:?}");
        (out.stdout, out.stderr)
    };
    fs::write(&path, before).unwrap();
    let original = execute();
    fs::write(&path, after).unwrap();
    let corrected = execute();
    assert_eq!(original, corrected);
    original.0
}
#[test]
fn ruby_preserves_literal_values_binding_defaults_visibility_and_definition_timing() {
    if !ruby_available() {
        return;
    }
    let before = r##"$events = []
$object = Object.new
def receiver; $events << :receiver; $object; end
class C
  private def value(a = ($events << :default; 4), *rest, key: 2, **kw, &block) = [
    a, rest, key, kw, block.call]
  def text = ["first
second", /first
second/.source, %w[first
second], "#{1}
second"]
end
def (receiver).value(x) = [
  ($events << :body; x)]
"##;
    let after = before
        .replace(") = [", ")\n    [")
        .replace("def text = [", "def text\n    [")
        .replace("kw, block.call]\n", "kw, block.call]\n  end\n")
        .replace("second\"]\n", "second\"]\n  end\n")
        .replace(
            "def (receiver).value(x)\n    [",
            "def (receiver).value(x)\n  [",
        )
        .replace("($events << :body; x)]\n", "($events << :body; x)]\nend\n");
    let observed = runtime_case(
        before,
        &after,
        "result = eval(File.read(ARGV[0]), TOPLEVEL_BINDING, ARGV[0]); p [result, $events.dup]; p [C.new.send(:value, key: 3, extra: 5) { :block }, C.new.send(:value, 8, 9) { :explicit }, C.private_instance_methods(false), C.new.text, $object.value(7), $events]",
    );
    assert!(String::from_utf8_lossy(&observed).contains(":receiver"));
    assert!(String::from_utf8_lossy(&observed).contains(":default"));
}
#[test]
fn ruby_preserves_control_flow_definition_result_and_exception_timing() {
    if !ruby_available() {
        return;
    }
    for (op, predicate) in [
        ("and", ":rhs"),
        ("or", ":rhs"),
        ("if", "true"),
        ("unless", "false"),
        ("while", "($n += 1) < 3"),
        ("until", "($n += 1) >= 3"),
    ] {
        let before = format!(
            "$events = []; $n = 0\ndef subject(a = ($events << :default; 4)) = (\n  $events << [:body, a]; a) {op} ($events << :outside; {predicate})\n"
        );
        let after = before
            .replace(" = (\n", "\n  (\n")
            .replace("a) ", "a)\nend ");
        runtime_case(
            &before,
            &after,
            "p eval(File.read(ARGV[0]), TOPLEVEL_BINDING, ARGV[0]); p [$events.dup, $n]; p [subject, subject(9), $events]",
        );
    }
    let before = "$events = []\ndef subject = begin\n  $events << :body\n  raise 'later'\nrescue\n  $events << :rescued\n  :fallback\nensure\n  $events << :ensured\nend and ($events << :outside; raise 'now') rescue ($events << :caught; :recovered)\n";
    let after = before
        .replace(" = begin\n", "\n  begin\n")
        .replace("end and ", "end\nend and ");
    let observed = runtime_case(
        before,
        &after,
        "p eval(File.read(ARGV[0]), TOPLEVEL_BINDING, ARGV[0]); p $events.dup; p [subject, $events]",
    );
    assert_eq!(observed, b":recovered\n[:outside, :caught]\n[:fallback, [:outside, :caught, :body, :rescued, :ensured]]\n");
}
#[test]
fn ruby_return_yield_rescue_and_ensure_keep_block_control_flow() {
    if !ruby_available() {
        return;
    }
    let before = "$events = []\ndef subject(value) = begin\n  return :early if value == :early\n  yield value\nrescue => e\n  $events << e.message\n  :fallback\nensure\n  $events << :ensure\nend\n";
    let after = before.replace(" = begin\n", "\n  begin\n") + "end\n";
    let output = runtime_case(
        before,
        &after,
        "load ARGV[0]; p [subject(:early) { raise 'not reached' }, subject(2) { |x| next x * 3 }, subject(3) { break :stopped }, subject(4) { raise 'caught' }, $events]",
    );
    assert_eq!(
        output,
        b"[:early, 6, :stopped, :fallback, [:ensure, :ensure, :ensure, \"caught\", :ensure]]\n"
    );
}

#[test]
fn ruby_qualified_data_references_preserve_bytes_position_and_runtime() {
    if !ruby_available() {
        return;
    }
    for (prefix, reference) in [
        ("", "Object::DATA"),
        ("", "::Object::DATA"),
        ("Root = Object\n", "Root::DATA"),
        ("Root = Object\n", "Root::Object::DATA"),
    ] {
        let source = format!(
            "{prefix}{BEFORE}p [{reference}.pos, {reference}.lineno, {reference}.read]\n__END__\nraw\n"
        );
        let data_start = source.find("__END__\n").unwrap() + "__END__\n".len();
        let data_line = source[..data_start].bytes().filter(|b| *b == b'\n').count();
        let expected = format!("[{data_start}, {data_line}, \"raw\\n\"]\n");
        assert_eq!(runtime_case(&source, &source, ""), expected.as_bytes());
    }
}

#[test]
fn ruby_observed_source_metadata_and_lines_withhold_edits_and_preserve_runtime() {
    if !ruby_available() {
        return;
    }
    for observer in [
        "p e.backtrace",
        "p e.full_message(highlight: false)",
        "p [e].flat_map(&:backtrace_locations).map(&:lineno)",
    ] {
        let before =
            format!("def f = begin\n  raise 'boom'\nend\nbegin; f; rescue => e; {observer}; end\n");
        runtime_case(&before, &before, "load ARGV[0]");
    }
    for before in [
        "def f = [\n  __LINE__]\np [f, __LINE__]\n",
        "def f = [\n  1]\ndef later; end\np method(:later).source_location\n",
        "def f = [\n  1]\np File.readlines(__FILE__)\n",
    ] {
        runtime_case(before, before, "load ARGV[0]");
    }
    // DATA exists only for the main script, not a file loaded from -e.
    let source = "def f = [\n  1]\np [DATA.pos, DATA.lineno, DATA.read]\n__END__\nraw\n";
    assert_eq!(runtime_case(source, source, ""), b"[60, 4, \"raw\\n\"]\n");
    let indirect = source.replace("DATA", "Object.const_get(:DATA)");
    runtime_case(&indirect, &indirect, "");
}
