//! real CLI controls, byte preservation and controlled Ruby semantics.
use crate::support::cli::project_with_lib;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const COP: &str = "Style/ColonMethodCall";
const BEFORE: &str = "object::method_name(arg)\n";
const AFTER: &str = "object.method_name(arg)\n";

fn cli(dir: &Path, args: &[&str], status: i32) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(status), "{args:?}: {output:?}");
    output
}

#[test]
fn defaults_selectors_config_and_stable_single_pass_fix() {
    let dir = project_with_lib();
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
    assert!(
        String::from_utf8_lossy(&out.stdout)
            .contains("lib/a.rb:1:7: C: Do not use `::` for method calls. (Style/ColonMethodCall)")
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), BEFORE);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
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
fn directives_suppress_the_dispatch_token_not_unrelated_argument_lines() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    for prefix in ["rubocop", "rustocop"] {
        for target in [COP, "ColonMethodCall", "Style", "all"] {
            for source in [
                format!("# {prefix}:disable {target}\n{BEFORE}"),
                format!("object::method_name(arg) # {prefix}:disable {target}\n"),
                format!("# {prefix}:disable-next {target}\nobject::method_name(\n arg)\n"),
            ] {
                fs::write(&path, &source).unwrap();
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
                assert_eq!(fs::read_to_string(&path).unwrap(), source);
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            }
        }
    }
    let source =
        "object:: # receiver dispatch\n method_name(arg) # rubocop:disable Style/ColonMethodCall\n";
    fs::write(&path, source).unwrap();
    cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        source.replace("::", ".")
    );
}

#[test]
fn config_relative_exclusions_and_stdin_do_not_write() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    for config in [
        String::new(),
        "exclude = [\"lib/a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = [\"lib/a.rb\"]\n"),
    ] {
        let status = i32::from(config.is_empty());
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        fs::write(&path, BEFORE).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
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
            assert!(String::from_utf8_lossy(&out.stdout).contains("a.rb:1:7:"));
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), BEFORE);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn clean_exempt_and_error_files_are_not_rewritten() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    for (source, status) in [
        (AFTER, 0),
        ("Tip::Top(arg)\nJava::com::method_name\n", 0),
        ("object&.method_name(arg)\n", 0),
        ("object::method_name(\n", 2),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], status);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn token_edits_preserve_bom_crlf_encoding_literals_data_and_line_numbers() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    for source in [
        "\u{feff}# encoding: UTF-8\r\np File::basename(\"café\")\r\np __LINE__\r\n__END__\r\nraw::data\r\n".as_bytes(),
        b"# encoding: ISO-8859-1\r\np File::basename(\"caf\xe9\")\r\np __LINE__\r\n__END__\r\nraw\xff::data\r\n",
    ] {
        fs::write(&path, source).unwrap();
        let start = source.windows(2).position(|w| w == b"::").unwrap();
        let mut expected = source.to_vec();
        expected.splice(start..start + 2, *b".");
        cli(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read(&path).unwrap(), crate::support::lf::lf(&expected));
    }
}

#[test]
fn upstream_incompatible_redundant_self_pair_when_registered() {
    if !rustocop::cops::Registry::new().has_cop("Style/RedundantSelf") {
        eprintln!("RedundantSelf pair pending its implementation; ColonMethodCall remains enabled");
        return;
    }
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    let source = "class C\n  def value; 1; end\n  def run; self::value; end\nend\np C.new.run\n";
    fs::write(&path, source).unwrap();
    let only = "Style/ColonMethodCall,Style/RedundantSelf";
    let out = cli(dir.path(), &["--only", only, "lib/a.rb"], 1);
    let report = String::from_utf8_lossy(&out.stdout);
    assert!(report.contains(COP));
    assert!(report.contains("Style/RedundantSelf"));
    for _ in 0..3 {
        let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", only, "--fix", "lib/a.rb"])
            .output()
            .unwrap();
        assert_ne!(output.status.code(), Some(2), "{output:?}");
    }
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        source.replace("self::value", "self.value")
    );
    // The selected incompatible peer withholds receiver removal, even once
    // ColonMethodCall is clean. The class-body call also lacks the stronger
    // main-object dispatch proof required by RedundantSelf's safe edits.
    cli(dir.path(), &["--only", only, "--fix", "lib/a.rb"], 1);
    fs::write(&path, "def value; 1; end\nself::value\n").unwrap();
    cli(dir.path(), &["--only", only, "--fix", "lib/a.rb"], 1);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "def value; 1; end\nself.value\n"
    );
    cli(
        dir.path(),
        &["--only", "Style/RedundantSelf", "--fix", "lib/a.rb"],
        0,
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "def value; 1; end\nvalue\n"
    );
}

#[test]
fn mixed_style_layout_fixes_compose_without_disabling_requested_rules() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    let source = "def f\n  return (self::take('x',{:key => 1}));  \nend\n";
    fs::write(&path, source).unwrap();
    let only = "Style/ColonMethodCall,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Layout";
    // The upstream incompatible RedundantSelf is not selected here. We do
    // not silently disable this cop or a selected peer; single-pass overlaps
    // defer deterministically, with a finite explicit bound for this fixture.
    for _ in 0..4 {
        let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", only, "--fix", "lib/a.rb"])
            .output()
            .unwrap();
        assert_ne!(output.status.code(), Some(2), "{output:?}");
    }
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "def f\n  self.take(\"x\", { key: 1 })\nend\n"
    );
    cli(dir.path(), &["--only", only, "--fix", "lib/a.rb"], 0);
}
