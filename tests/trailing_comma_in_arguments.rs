//! real selectors, safe deletion, byte and runtime observations.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const COP: &str = "Style/TrailingCommaInArguments";
const BEFORE: &str = "f(1,)\n";
const AFTER: &str = "f(1)\n";

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
    out
}

#[test]
fn defaults_selectors_config_and_stable_one_pass_fix() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let registry = rustocop::cops::Registry::new();
    let cops: Vec<_> = registry.cops().iter().filter(|c| c.name() == COP).collect();
    assert_eq!(cops.len(), 1);
    assert!(cops[0].default_enabled());
    assert!(cops[0].safe_fixes());
    assert!(cops[0].path_policy().is_none());
    fs::write(&path, BEFORE).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let out = cli(dir.path(), &["lib/a.rb"], 1);
    assert!(String::from_utf8_lossy(&out.stdout).contains("lib/a.rb:1:4: C: Avoid comma after the last parameter of a method call. (Style/TrailingCommaInArguments)"));
    for config in [
        format!("[cops.\"{COP}\"]\nenabled = false\n"),
        "[cops.Style]\nenabled = false\n".into(),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        cli(dir.path(), &["--fix", "lib/a.rb"], 0);
        cli(dir.path(), &["--only", COP, "lib/a.rb"], 1);
        cli(
            dir.path(),
            &["--only", COP, "--except", COP, "--fix", "lib/a.rb"],
            0,
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), BEFORE);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
    fs::write(
        dir.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
    )
    .unwrap();
    cli(dir.path(), &["--fix", "lib/a.rb"], 0);
    assert_eq!(fs::read_to_string(&path).unwrap(), AFTER);
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn directives_anchor_the_comma_not_the_call_or_closing_delimiter() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "TrailingCommaInArguments", "Style", "all"] {
            for source in [
                format!("# {prefix}:disable {target}\n{BEFORE}"),
                format!("f(1,) # {prefix}:disable {target}\n"),
                format!("f(\n# {prefix}:disable-next {target}\n1,\n)\n"),
                format!("f(\n1, # {prefix}:disable {target}\n)\n"),
            ] {
                fs::write(&path, &source).unwrap();
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
                assert_eq!(fs::read_to_string(&path).unwrap(), source);
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            }
        }
    }
    let source = "f( # rubocop:disable Style/TrailingCommaInArguments\n1,\n) # rustocop:disable Style/TrailingCommaInArguments\n";
    fs::write(&path, source).unwrap();
    cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        source.replace("1,", "1")
    );
}

#[test]
fn config_relative_excludes_and_stdin_have_no_write_side_effects() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for config in [
        String::new(),
        "exclude = [\"lib/a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = [\"lib/a.rb\"]\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), &config).unwrap();
        fs::write(&path, BEFORE).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let status = i32::from(config.is_empty());
        cli(&dir.path().join("lib"), &["--only", COP, "a.rb"], status);
        if status == 0 {
            cli(
                &dir.path().join("lib"),
                &["--only", COP, "--fix", "a.rb"],
                0,
            );
        }
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path().join("lib"))
            .args(["--only", COP, "--stdin", "a.rb"])
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
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(status));
        if status == 1 {
            assert!(String::from_utf8_lossy(&out.stdout).contains("a.rb:1:4:"));
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), BEFORE);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn clean_exempt_withheld_and_error_files_keep_bytes_and_mtime() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, status) in [
        (AFTER, 0),
        ("f([1,], {a: 1,})\n", 0),
        ("obj[1,] = value\n", 0),
        ("f(1, &block)\n", 0),
        ("f(\n", 2),
        ("f(1,)\np DATA.pos\n__END__\nraw,\n", 1),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], status);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn comma_deletion_retains_bom_crlf_encoding_literals_comments_data_and_line() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for source in [
        "\u{feff}# encoding: UTF-8\r\np(\"café\",) # comment,\r\np __LINE__\r\n__END__\r\nraw,\r\n".as_bytes().to_vec(),
        b"# encoding: ISO-8859-1\r\np(\"caf\xe9\",) # comment,\r\np __LINE__\r\n__END__\r\nraw\xff,\r\n".to_vec(),
    ] {
        fs::write(&path, &source).unwrap();
        let mut expected = source.clone();
        let offset = source.windows(2).position(|w| w == b",)").unwrap();
        expected.remove(offset);
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read(&path).unwrap(), expected);
    }
}

fn ruby(dir: &Path) -> Output {
    Command::new("ruby")
        .current_dir(dir)
        .arg("lib/a.rb")
        .output()
        .unwrap()
}

#[test]
fn controlled_ruby_argument_count_order_keywords_blocks_and_destructuring() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = r#"$events = []
def effect(value); $events << value; value; end
class Base
  def run(*args, **kw); [args, kw, yield]; end
end
class Receiver < Base
  def take(*args, **kw); $events << [args, kw, block_given? ? yield : :no_block]; self; end
  def [](*keys); $events << [:read, keys]; @value || 0; end
  def []=(*args); $events << [:write, args]; @value = args.last; end
  def +(value); $events << [:plus, value]; self; end
  def run(*args, **kw, &block); super(*args, **kw, &block); end
end
r = Receiver.new
xs = [2, 3]
kw = {b: 4}
a = :omitted
r.take(effect(1), *xs, a:, **kw,) { effect(:block) }
r.take({a: 1,},)
r.take([1, 2,],)
r&.take(effect(:safe),)
nil&.take(effect(:skipped),)
r.+(effect(:operator),)
r[effect(:index),] += effect(1)
r[effect(:and),] &&= effect(2)
r[effect(:or),] ||= effect(:not_run)
r[effect(:assignment),] = effect(7)
p r.run(effect(:super), a: effect(:kw)) { effect(:forwarded_block) }
f = ->(x) { [x, :lambda] }
p f.(effect(:lambda_arg),)
r.take(<<~TEXT.chomp,)
  café, #{effect(:interpolation)}
TEXT
r.take(<<~TEXT)
  text #{effect(:nested,)}
TEXT
p [1,2].map { |(x,y),| [effect(x), y] }
p __LINE__
p $events
__END__
raw payload,
"#;
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path());
    assert!(before.status.success(), "{before:?}");
    cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
    let fixed = fs::read_to_string(&path).unwrap();
    assert!(
        fixed.contains("r[effect(:assignment),] ="),
        "plain assignment stays exempt"
    );
    assert!(
        fixed.contains("{ |(x,y),|"),
        "block destructuring stays intact"
    );
    assert!(fixed.contains("r.take([1, 2,])"));
    assert!(fixed.contains("r.take({a: 1,})"));
    assert!(fixed.contains("text #{effect(:nested)}"));
    assert_eq!(source.lines().count(), fixed.lines().count());
    assert!(fixed.ends_with("raw payload,\n"));
    let after = ruby(dir.path());
    assert!(after.status.success(), "{after:?}");
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
    assert!(!String::from_utf8_lossy(&before.stdout).contains(":skipped"));
    cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
}

#[test]
fn data_initial_byte_position_is_observably_protected() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "p(1,)\np [DATA.pos, DATA.read]\n__END__\nraw,\n";
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path());
    assert!(before.status.success());
    cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 1);
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert_eq!(ruby(dir.path()).stdout, before.stdout);
    fs::write(&path, source.replace("p(1,)", "p(1)")).unwrap();
    assert_ne!(
        ruby(dir.path()).stdout,
        before.stdout,
        "naive deletion changes DATA.pos"
    );
}

#[test]
fn available_call_forwarding_lambda_and_layout_peers_compose() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "def sink(x)\n  x\nend\nf = lambda { |x| sink(x,) }\np f.(('x'),)\n";
    let peers = format!(
        "{COP},Style/Lambda,Style/LambdaCall,Style/Proc,Style/StabbyLambdaParentheses,Style/BlockDelimiters,Style/ArgumentsForwarding,Style/SuperArguments,Style/SuperWithArgsParentheses,Style/MethodCallWithArgsParentheses,Style/MethodCallWithoutArgsParentheses,Style/NestedParenthesizedCalls,Style/RedundantParentheses,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/SignalException,Layout"
    );
    for only in [Some(peers.as_str()), None] {
        fs::write(&path, source).unwrap();
        let before = Command::new("ruby")
            .current_dir(dir.path())
            .arg("lib/a.rb")
            .output()
            .ok();
        let mut stable = false;
        for _ in 0..6 {
            let bytes = fs::read(&path).unwrap();
            let mut args = vec!["--fix", "lib/a.rb"];
            if let Some(only) = only {
                args.extend(["--only", only]);
            }
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            if fs::read(&path).unwrap() == bytes {
                stable = true;
                break;
            }
        }
        assert!(
            stable,
            "curated fixture must stabilize within six explicit passes"
        );
        let fixed = fs::read_to_string(&path).unwrap();
        assert!(!fixed.contains("x,)"));
        assert!(!fixed.contains("'x'"));
        assert!(fixed.contains("f.call"), "{fixed}");
        cli(dir.path(), &["--only", COP, "lib/a.rb"], 0);
        if let Some(before) = before {
            let after = ruby(dir.path());
            assert!(before.status.success() && after.status.success());
            assert_eq!(before.stdout, after.stdout);
            assert_eq!(before.stderr, after.stderr);
        }
    }
}

#[test]
fn forwarding_peer_atomic_rewrite_and_comma_deletion_do_not_partially_apply() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "def target(*args, **kw)\n  [args, kw, yield]\nend\ndef relay(*args, **kwargs, &block)\n  target(*args, **kwargs, &block)\nend\np relay(1, k: 2,) { 3 }\n";
    fs::write(&path, source).unwrap();
    let before = Command::new("ruby")
        .current_dir(dir.path())
        .arg("lib/a.rb")
        .output()
        .ok();
    let only = format!("{COP},Style/ArgumentsForwarding,Style/BlockDelimiters,Layout");
    for _ in 0..4 {
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", &only, "--fix", "lib/a.rb"])
            .output()
            .unwrap();
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    let fixed = fs::read_to_string(&path).unwrap();
    assert!(fixed.contains("def relay(...)"), "{fixed}");
    assert!(fixed.contains("target(...)"), "{fixed}");
    assert!(fixed.contains("relay(1, k: 2)"), "{fixed}");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir.path(), &["--only", &only, "--fix", "lib/a.rb"], 0);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    if let Some(before) = before {
        let after = ruby(dir.path());
        assert!(before.status.success() && after.status.success());
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
    }
}

#[test]
fn incompatible_heredoc_layout_peer_is_exercised_when_available() {
    let peer = "Layout/HeredocArgumentClosingParenthesis";
    if !rustocop::cops::Registry::new().has_cop(peer) {
        eprintln!("{peer} is not implemented; conditional pairing remains pending");
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "p(<<~TEXT,)\n  literal,\nTEXT\n").unwrap();
    let only = format!("{COP},{peer}");
    for _ in 0..4 {
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", &only, "--fix", "lib/a.rb"])
            .output()
            .unwrap();
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    cli(dir.path(), &["--only", &only, "lib/a.rb"], 0);
    assert!(
        fs::read_to_string(path)
            .unwrap()
            .contains("  literal,\nTEXT\n")
    );
}
