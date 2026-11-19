//! real CLI, bytes, binding and controlled-runtime evidence.
use crate::support::source_getter;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/Lambda";
const BEFORE: &str = "f = lambda { |x| x }\n";
const AFTER: &str = "f = ->(x) { x }\n";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn cli(dir: &Path, args: &[&str]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        matches!(out.status.code(), Some(0 | 1)),
        "{args:?}: {out:?}"
    );
    out
}
fn fix(dir: &Path) {
    cli(dir, &["--only", COP, "--fix", "x.rb"]);
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], status: i32) {
    let path = dir.join("x.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let out = cli(dir, args);
    assert_eq!(out.status.code(), Some(status), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), mtime);
}
#[test]
fn directives_excludes_and_atomic_disabled_delimiters() {
    let dir = project();
    for prefix in ["rustocop", "rubocop"] {
        for source in [
            format!("# {prefix}:disable {COP}\n{BEFORE}"),
            format!("# {prefix}:disable-next {COP}\n{BEFORE}"),
            format!("{} # {prefix}:disable {COP}\n", BEFORE.trim_end()),
        ] {
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "x.rb"],
                0,
            );
        }
        // As in RuboCop, the offense on line 1 is corrected, delimiters on
        // the disabled line included.
        fs::write(
            dir.path().join("x.rb"),
            format!("sink -> do\n 1\nend # {prefix}:disable {COP}\n"),
        )
        .unwrap();
        fix(dir.path());
        assert_eq!(
            fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            format!("sink lambda {{\n 1\n}} # {prefix}:disable {COP}\n")
        );
        let source = format!("# {prefix}:disable {COP}\n{BEFORE}# {prefix}:enable {COP}\n{BEFORE}");
        fs::write(dir.path().join("x.rb"), &source).unwrap();
        fix(dir.path());
        assert_eq!(
            fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            source.replacen(
                &format!("# {prefix}:enable {COP}\n{BEFORE}"),
                &format!("# {prefix}:enable {COP}\n{AFTER}"),
                1
            )
        );
    }
    for config in [
        format!("[cops.\"{COP}\"]\nexclude=[\"x.rb\"]\n"),
        "exclude=[\"x.rb\"]\n".into(),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            dir.path(),
            BEFORE.as_bytes(),
            &["--only", COP, "--fix", "x.rb"],
            0,
        );
    }
}
#[test]
fn stdin_reports_and_rejects_fix_without_disk_writes() {
    let dir = project();
    fs::write(dir.path().join("x.rb"), "on disk\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", COP, "--stdin", "x.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(BEFORE.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains(COP));
    let rejected = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir.path())
        .args(["--only", COP, "--fix", "--stdin", "x.rb"])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(2));
    assert_eq!(
        fs::read_to_string(dir.path().join("x.rb")).unwrap(),
        "on disk\n"
    );
}
#[test]
fn byte_fidelity_bom_crlf_legacy_heredocs_line_and_data() {
    let dir = project();
    for (before, after) in [
        (
            b"\xef\xbb\xbff = lambda { |x| [x, __LINE__] }\r\np __LINE__\r\n__END__\r\nraw\xff\r\n"
                .as_slice(),
            b"\xef\xbb\xbff = ->(x) { [x, __LINE__] }\r\np __LINE__\r\n__END__\r\nraw\xff\r\n"
                .as_slice(),
        ),
        (
            b"# encoding: ISO-8859-1\r\nf = lambda { 'caf\xe9' }\r\n".as_slice(),
            b"# encoding: ISO-8859-1\r\nf = -> { 'caf\xe9' }\r\n".as_slice(),
        ),
        (
            b"# encoding: ISO-8859-1\r\nf = lambda { |x = 'caf\xe9'| x }\r\n".as_slice(),
            b"# encoding: ISO-8859-1\r\nf = lambda { |x = 'caf\xe9'| x }\r\n".as_slice(),
        ),
        (
            b"f = -> do # comment\r\n <<~TEXT\r\n  } end | # payload\r\nTEXT\r\nend\r\n".as_slice(),
            b"f = lambda do # comment\r\n <<~TEXT\r\n  } end | # payload\r\nTEXT\r\nend\r\n"
                .as_slice(),
        ),
    ] {
        fs::write(dir.path().join("x.rb"), before).unwrap();
        fix(dir.path());
        assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), after);
    }
}
fn ruby(dir: &Path) -> Output {
    Command::new("ruby")
        .current_dir(dir)
        .arg("x.rb")
        .output()
        .unwrap()
}
#[test]
fn controlled_runtime_preserves_strict_arity_flow_captures_defaults_and_binding() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    for (source, expected) in [
        (
            "f = lambda { |x| return x+1 }; p [f.lambda?, f.arity, f.call(2)]; begin; f.call; rescue ArgumentError; p :arity; end\n",
            "[true, 1, 3]\n:arity\n",
        ),
        (
            "f = ->(x) do\n break x+1\nend; p [f.lambda?, f.arity, f.call(2)]; begin; f.call(1,2); rescue ArgumentError; p :arity; end\n",
            "[true, 1, 3]\n:arity\n",
        ),
        ("f = lambda { |x| next x*2 }; p f.call(3)\n", "6\n"),
        (
            "x = 10; f = lambda { |y; shadow| shadow=y; [x, -> { shadow }.call] }; p f.call(2)\n",
            "[10, 2]\n",
        ),
        (
            "f = lambda { |(a,b), *xs, key: 2, **kw, &blk| [a,b,xs,key,kw,blk.call] }; p f.call([1,3],4,key:5,other:6) { 7 }\n",
            "[1, 3, [4], 5, {:other=>6}, 7]\n",
        ),
        (
            "events=[]; f = ->(x = (events << :default; 3)) do\n [x, events.dup]\nend; p events; p f.call; p f.call(4)\n",
            "[]\n[3, [:default]]\n[4, [:default]]\n",
        ),
        (
            "xs=[]; xs.push ->(x = 3) do\n x*2\nend; p xs.map(&:call)\n",
            "[6]\n",
        ),
        (
            "f = ->(x = __LINE__) do\n [x, __LINE__, <<~TEXT]\n  } end | # value\nTEXT\nend; p f.call; p __LINE__\n",
            "[1, 2, \"} end | # value\\n\"]\n5\n",
        ),
        ("f = ->(x = 1 | 2) do\n x\nend; p f.call\n", "3\n"),
        (
            "f = lambda { _1 + 2 }; p [f.lambda?, f.arity, f.call(3)]\n",
            "[true, 1, 5]\n",
        ),
        (
            "f = lambda { |x = 'a,b;|', r: /[|,]/| [x, r.source] }; p f.call\n",
            "[\"a,b;|\", \"(?-mix:[|,])\"]\n",
        ),
    ] {
        let sibling;
        let source = if source.contains("r.source") {
            sibling = source_getter::pattern_sibling(dir.path(), "x.rb", source, COP);
            sibling.as_str()
        } else {
            source
        };
        let path = dir.path().join("x.rb");
        fs::write(&path, source).unwrap();
        let before = ruby(dir.path());
        assert!(before.status.success(), "{source}: {before:?}");
        assert_eq!(
            String::from_utf8_lossy(&before.stdout),
            expected,
            "{source}"
        );
        fix(dir.path());
        assert_ne!(
            fs::read_to_string(&path).unwrap(),
            source,
            "must really fix {source}"
        );
        let after = ruby(dir.path());
        assert_eq!(
            (after.status.code(), after.stdout, after.stderr),
            (before.status.code(), before.stdout, before.stderr),
            "{source}"
        );
        let stable = fs::read(&path).unwrap();
        fix(dir.path());
        assert_eq!(fs::read(path).unwrap(), stable);
    }
}
#[test]
fn custom_proc_witness_is_not_rewritten_to_strict_lambda() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let source =
        "def lambda(&b); b; end\nf = lambda { |x,y| [x,y] }; p [f.lambda?, f.arity, f.call(1)]\n";
    fs::write(dir.path().join("x.rb"), source).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&ruby(dir.path()).stdout),
        "[false, 2, [1, nil]]\n"
    );
    unchanged(
        dir.path(),
        source.as_bytes(),
        &["--only", COP, "--fix", "x.rb"],
        1,
    );
    // Witness: the syntactically valid naive edit changes strict arity.
    fs::write(
        dir.path().join("x.rb"),
        source.replace("f = lambda { |x,y|", "f = ->(x,y) {"),
    )
    .unwrap();
    assert!(!ruby(dir.path()).status.success());
}
#[test]
fn incompatible_symbol_proc_composition_gate_when_that_peer_is_available() {
    let registry = rustocop::cops::Registry::new();
    let only = if registry.has_cop("Style/SymbolProc") {
        "Style/Lambda,Style/SymbolProc"
    } else {
        COP
    };
    let dir = project();
    let path = dir.path().join("x.rb");
    fs::write(&path, "f = lambda { |x| x.to_s }\n").unwrap();
    for _ in 0..3 {
        cli(dir.path(), &["--only", only, "--fix", "x.rb"]);
        let bytes = fs::read(&path).unwrap();
        assert_eq!(ruby_prism::parse(&bytes).errors().count(), 0);
        assert!(!String::from_utf8_lossy(&bytes).contains("->(x)(&"));
    }
    let stable = fs::read(&path).unwrap();
    cli(dir.path(), &["--only", only, "--fix", "x.rb"]);
    assert_eq!(fs::read(path).unwrap(), stable);
}
#[test]
fn ruby34_it_has_real_prism_parameter_shape_without_claiming_runtime32_execution() {
    for source in [
        "f = lambda { it }; p f.call(3)\n",
        "f = -> {\n it\n}; p f.call(3)\n",
    ] {
        let parsed = ruby_prism::parse(source.as_bytes());
        assert_eq!(parsed.errors().count(), 0);
        assert!(format!("{:?}", parsed.node()).contains("ItParametersNode"));
        let dir = project();
        fs::write(dir.path().join("x.rb"), source).unwrap();
        fix(dir.path());
        let changed = fs::read(dir.path().join("x.rb")).unwrap();
        assert_ne!(changed, source.as_bytes());
        let parsed = ruby_prism::parse(&changed);
        assert!(format!("{:?}", parsed.node()).contains("ItParametersNode"));
    }
}
#[test]
fn default_style_and_layout_peer_composition_stabilizes() {
    let dir = project();
    let source = "# Copyright (c) 2026 Example\n# frozen_string_literal: true\nf = lambda { || 1 }; g = ->()do\n  [1].map { |x|\n    x\n  }\nend\ndef delegate(*args)\n  target(*args)\nend\np([f[], g[]])\n";
    let path = dir.path().join("x.rb");
    fs::write(&path, source).unwrap();
    let before_runtime = Command::new("ruby")
        .arg("--version")
        .output()
        .ok()
        .map(|_| ruby(dir.path()));
    for _ in 0..4 {
        cli(dir.path(), &["--fix", "x.rb"]);
    }
    let stable = fs::read(&path).unwrap();
    assert_ne!(stable, source.as_bytes());
    let text = String::from_utf8_lossy(&stable);
    assert!(text.contains("map do"), "{text}");
    assert!(text.contains("def delegate(*)"), "{text}");
    assert!(!text.contains("->()"), "{text}");
    if let Some(before) = before_runtime {
        let after = ruby(dir.path());
        assert!(before.status.success(), "{before:?}");
        assert_eq!(
            (after.status.code(), after.stdout, after.stderr),
            (before.status.code(), before.stdout, before.stderr)
        );
    }
    let out = cli(dir.path(), &["--fix", "x.rb"]);
    assert_eq!(fs::read(&path).unwrap(), stable);
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains(COP),
        "{out:?}"
    );
    // Includes EmptyParameter, BlockDelimiters, RedundantReturn, Semicolon,
    // ArgumentsForwarding, and Layout in the real default registry. SymbolProc
    // is not implemented yet; Lambda never emits SymbolProc syntax itself.
}
