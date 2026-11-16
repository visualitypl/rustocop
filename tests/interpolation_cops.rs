//! public CLI, byte preservation and controlled Ruby semantic witnesses.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const E: &str = "Style/EmptyStringInsideInterpolation";
const R: &str = "Style/RedundantInterpolation";
const U: &str = "Style/RedundantInterpolationUnfreeze";
const ALL: &str = "Style/EmptyStringInsideInterpolation,Style/RedundantInterpolation,Style/RedundantInterpolationUnfreeze";
const CASES: [(&str, &str, &str); 3] = [
    (E, "p \"#{true ? 'x' : nil}\"\n", "p \"#{'x' if true}\"\n"),
    (R, "p \"#{value}\"\n", "p \"#{value}\"\n"),
    (U, "p +\"#{1} x\"\n", "p \"#{1} x\"\n"),
];
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(d: &Path, args: &[&str], status: i32) -> Output {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(status), "{args:?}: {o:?}");
    o
}
fn unchanged(d: &Path, cop: &str, source: &[u8], status: i32) {
    let p = d.join("a.rb");
    fs::write(&p, source).unwrap();
    let m = fs::metadata(&p).unwrap().modified().unwrap();
    cli(d, &["--only", cop, "--fix", "a.rb"], status);
    assert_eq!(fs::read(&p).unwrap(), source);
    assert_eq!(fs::metadata(p).unwrap().modified().unwrap(), m);
}
fn has_ruby() -> bool {
    Command::new("ruby").arg("--version").output().is_ok()
}
fn ruby(d: &Path) -> Output {
    Command::new("ruby")
        .arg("a.rb")
        .current_dir(d)
        .output()
        .unwrap()
}
fn same(a: &Output, b: &Output) {
    assert!(a.status.success(), "{a:?}");
    assert_eq!(a.status, b.status, "{b:?}");
    assert_eq!(a.stdout, b.stdout);
    assert_eq!(a.stderr, b.stderr);
}
#[test]
fn registry_fixed_default_version_safety_and_no_write_without_fix() {
    let d = project();
    let r = rustocop::cops::Registry::new();
    for (name, source, _) in CASES {
        let c: Vec<_> = r.cops().iter().filter(|c| c.name() == name).collect();
        assert_eq!(c.len(), 1);
        assert!(c[0].default_enabled());
        assert_eq!(c[0].safe_fixes(), name != R);
        assert_eq!(
            c[0].ruby_versions().minimum,
            if name == U {
                Some(rustocop::cops::policy::RubyVersion(3, 0))
            } else {
                None
            }
        );
        fs::write(d.path().join("a.rb"), source).unwrap();
        let m = fs::metadata(d.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap();
        for args in [vec!["a.rb"], vec!["--only", name, "a.rb"]] {
            let o = cli(d.path(), &args, 1);
            assert!(String::from_utf8_lossy(&o.stdout).contains(name));
        }
        assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), source);
        assert_eq!(
            fs::metadata(d.path().join("a.rb"))
                .unwrap()
                .modified()
                .unwrap(),
            m
        );
    }
}
#[test]
fn selectors_config_force_enabling_and_one_pass_corrections() {
    let d = project();
    for (name, source, after) in CASES {
        for config in [
            "[cops.Style]\nenabled = false\n".to_string(),
            format!("[cops.\"{name}\"]\nenabled = false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            fs::write(d.path().join("a.rb"), source).unwrap();
            cli(
                d.path(),
                &["--only", name, "--except", name, "--fix", "a.rb"],
                0,
            );
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), source);
            cli(
                d.path(),
                &["--only", name, "--fix", "a.rb"],
                i32::from(name == R),
            );
            assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
        }
        fs::write(d.path().join("rustocop.toml"),format!("[cops.Style]\nenabled = false\n[cops.Lint]\nenabled = false\n[cops.\"{name}\"]\nenabled = true\n")).unwrap();
        fs::write(d.path().join("a.rb"), source).unwrap();
        cli(d.path(), &["--fix", "a.rb"], i32::from(name == R));
        assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
    }
}
#[test]
fn directives_excludes_clean_error_and_unsafe_files_preserve_bytes_and_mtime() {
    let d = project();
    for (name, source, _) in CASES {
        for prefix in ["rubocop", "rustocop"] {
            for target in [name, "Style", "all"] {
                for text in [
                    format!("# {prefix}:disable {target}\n{source}"),
                    format!("{} # {prefix}:disable {target}\n", source.trim_end()),
                    format!("# {prefix}:disable-next {target}\n{source}"),
                ] {
                    unchanged(d.path(), name, text.as_bytes(), 0);
                }
            }
        }
        for config in [
            "exclude = [\"a.rb\"]\n".to_string(),
            format!("[cops.\"{name}\"]\nexclude = [\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), name, source.as_bytes(), 0);
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        unchanged(d.path(), name, b"p 'clean'\n", 0);
        unchanged(d.path(), name, b"p \"unterminated\n", 2);
    }
    unchanged(d.path(), R, b"p \"#{value}\"\n", 1);
}
#[test]
fn controlled_runtime_nil_branch_preserves_effect_order_truthiness_and_assignment_value() {
    if !has_ruby() {
        return;
    }
    let d = project();
    // NilClass dispatch remains IDENTICAL on the discarded nil branch, even
    // when overridden. No empty-string-to-nil conversion is allowed here.
    let source = r##"$events = []
class NilClass
  def to_s
    $events << :nil
    'N'
  end
end
def condition
  $events << :condition
  false
end
result = "#{condition ? 'x' : nil}"
p [result.bytes, result.encoding.name, result.frozen?, $events, __LINE__]
"##;
    fs::write(d.path().join("a.rb"), source).unwrap();
    let before = ruby(d.path());
    cli(d.path(), &["--only", E, "--fix", "a.rb"], 0);
    assert!(
        fs::read_to_string(d.path().join("a.rb"))
            .unwrap()
            .contains("'x' if condition")
    );
    same(&before, &ruby(d.path()));
}
#[test]
fn controlled_frozen_mutable_encoding_and_identity_observations_survive_unfreeze() {
    if !has_ruby() {
        return;
    }
    let d = project();
    for frozen in ["true", "false"] {
        for expr in ["+\"#{1}x\"", "\"#{1}x\".dup", "String.new(\"#{1}x\")"] {
            let src = format!(
                "# frozen_string_literal: {frozen}\na = {expr}\np [a.bytes, a.encoding.name, a.frozen?, a.equal?(a)]\na << 'tail'\np a\n"
            );
            fs::write(d.path().join("a.rb"), &src).unwrap();
            let before = ruby(d.path());
            cli(d.path(), &["--only", U, "--fix", "a.rb"], 0);
            assert_ne!(fs::read_to_string(d.path().join("a.rb")).unwrap(), src);
            same(&before, &ruby(d.path()));
        }
    }
}
#[test]
fn heredoc_indent_and_boundary_ownership_are_preserved_without_command_execution() {
    let d = project();
    for (source, after) in [
        (
            "p(+<<~X, <<~Y)\n  #{1}\nX\n  sibling\nY\n",
            "p(<<~X, <<~Y)\n  #{1}\nX\n  sibling\nY\n",
        ),
        (
            "p <<~X\n  #{true ? 'x' : nil}\nX\n",
            "p <<~X\n  #{'x' if true}\nX\n",
        ),
    ] {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let before = has_ruby().then(|| ruby(d.path()));
        cli(d.path(), &["--only", ALL, "--fix", "a.rb"], 0);
        assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
        if let Some(before) = before {
            same(&before, &ruby(d.path()));
        }
    }
    // Commands are source/AST-only. The xstring remains a command, not a String.
    let source = "`not_executed #{true ? 'x' : nil}`\n";
    fs::write(d.path().join("a.rb"), source).unwrap();
    cli(d.path(), &["--only", E, "--fix", "a.rb"], 0);
    let after = fs::read(d.path().join("a.rb")).unwrap();
    assert_eq!(after, b"`not_executed #{'x' if true}`\n");
    let parsed = ruby_prism::parse(&after);
    assert!(
        parsed
            .node()
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .iter()
            .next()
            .unwrap()
            .as_interpolated_x_string_node()
            .is_some()
    );
}
#[test]
fn strict_options_and_stdin_remain_detection_only_and_read_only() {
    use std::io::Write;
    use std::process::Stdio;
    let d = project();
    for (name, source, _) in CASES {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", name, "--stdin", "a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        assert_eq!(child.wait_with_output().unwrap().status.code(), Some(1));
        assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), source);
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{name}\"]\nEnforcedStyle = \"other\"\n"),
        )
        .unwrap();
        unchanged(d.path(), name, source.as_bytes(), 2);
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn fixed_34_prism_keeps_literal_flags_payloads_and_local_binding_depths() {
    use rustocop::cops::walk::{NodeVisitor, walk};
    #[derive(Default)]
    struct Shape(Vec<String>);
    impl<'pr> NodeVisitor<'pr> for Shape {
        fn enter(&mut self, n: ruby_prism::Node<'pr>) {
            if let Some(s) = n.as_string_node() {
                self.0.push(format!(
                    "str:{:?}:{}:{}:{}:{}",
                    s.unescaped(),
                    s.is_forced_utf8_encoding(),
                    s.is_forced_binary_encoding(),
                    s.is_frozen(),
                    s.is_mutable()
                ));
            }
            if let Some(s) = n.as_local_variable_read_node() {
                self.0
                    .push(format!("local:{:?}:{}", s.name().as_slice(), s.depth()));
            }
            if let Some(s) = n.as_interpolated_string_node() {
                self.0
                    .push(format!("interp:{}:{}", s.is_frozen(), s.is_mutable()));
            }
        }
    }
    let d = project();
    for header in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
    ] {
        let src = format!("{header}a = +\"#{{1}}x\"\np [a]\n");
        fs::write(d.path().join("a.rb"), &src).unwrap();
        cli(d.path(), &["--only", U, "--fix", "a.rb"], 0);
        let corrected = fs::read(d.path().join("a.rb")).unwrap();
        assert_ne!(corrected, src.as_bytes());
        let a = ruby_prism::parse(src.as_bytes());
        let b = ruby_prism::parse(&corrected);
        assert_eq!(a.errors().count(), 0);
        assert_eq!(b.errors().count(), 0);
        let mut before = Shape::default();
        let mut after = Shape::default();
        walk(&mut before, &a.node());
        walk(&mut after, &b.node());
        assert_eq!(before.0, after.0);
    }
}
#[test]
fn controlled_nil_rewrites_preserve_interpolation_order_and_regexp_once() {
    if !has_ruby() {
        return;
    }
    let d = project();
    let src = r##"$events = []
def c
  $events << $events.length
  $events.length.odd?
end
2.times do
  values = ["#{c ? 'x' : nil}", :"#{c ? nil : 'y'}", %W[#{c ? 'x' : nil}], %I[#{c ? nil : 'y'}], /#{c ? 'x' : nil}/o]
  p values.map { |v| v.inspect }
end
p $events
"##;
    fs::write(d.path().join("a.rb"), src).unwrap();
    let original = ruby(d.path());
    cli(d.path(), &["--only", E, "--fix", "a.rb"], 0);
    let corrected = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert_ne!(corrected, src);
    assert!(corrected.contains("/o"));
    same(&original, &ruby(d.path()));
}
#[test]
fn main_literal_flags_agree_with_explicit_prism34_but_do_not_predict_vm_folding() {
    use rustocop::cops::walk::{NodeVisitor, walk};
    #[derive(Default)]
    struct Flags(Vec<String>);
    impl<'pr> NodeVisitor<'pr> for Flags {
        fn enter(&mut self, n: ruby_prism::Node<'pr>) {
            if let Some(s) = n.as_string_node().filter(|s| s.unescaped().is_empty()) {
                self.0
                    .push(format!("empty:{}:{}", s.is_frozen(), s.is_mutable()));
            }
            if let Some(s) = n.as_interpolated_string_node() {
                self.0
                    .push(format!("interpolated:{}:{}", s.is_frozen(), s.is_mutable()));
            }
        }
    }
    let d = project();
    let prism34 = Command::new("ruby")
        .args(["-rprism", "-e", "Prism.parse('nil', version: '3.4')"])
        .output()
        .is_ok_and(|o| o.status.success());
    for (header, empty_flags) in [
        ("", "empty:false:false"),
        ("# frozen_string_literal: true\n", "empty:true:false"),
        ("# frozen_string_literal: false\n", "empty:false:true"),
    ] {
        let source = format!("{header}\"#{{true ? '' : 'yes'}}\"");
        let parsed = ruby_prism::parse(source.as_bytes());
        let mut flags = Flags::default();
        walk(&mut flags, &parsed.node());
        assert_eq!(flags.0, ["interpolated:false:false", empty_flags]);
        if prism34 {
            fs::write(d.path().join("a.rb"), &source).unwrap();
            let o = Command::new("ruby")
                .args([
                    "-rprism",
                    "-e",
                    r#"
r = Prism.parse(File.binread('a.rb'), version: '3.4')
abort r.errors.inspect unless r.success?
w = ->(n) do
  puts "empty:#{n.frozen?}:#{n.mutable?}" if n.is_a?(Prism::StringNode) && n.unescaped.empty?
  puts "interpolated:#{n.frozen?}:#{n.mutable?}" if n.is_a?(Prism::InterpolatedStringNode)
  n.compact_child_nodes.each { |c| w.call(c) }
end
w.call(r.value)
"#,
                ])
                .current_dir(d.path())
                .output()
                .unwrap();
            assert!(o.status.success(), "{o:?}");
            assert_eq!(
                String::from_utf8(o.stdout).unwrap(),
                flags.0.join("\n") + "\n"
            );
        }
    }
    if !prism34 {
        eprintln!("SKIP explicit Prism 3.4 cross-check (gem unavailable), main-AST flags checked");
    }
}
#[test]
fn curated_default_and_literal_peer_composition_stabilizes_within_six_passes() {
    if !has_ruby() {
        return;
    }
    let d = project();
    let src =
        "p +\"#{1} x\"\np \"#{true ? 'x' : nil}\"\np [%Q(hi), %W(one two), :'a-b', :\"word\"]\n";
    let peers = format!(
        "{ALL},Style/BarePercentLiterals,Style/PercentLiteralDelimiters,Style/PercentQLiterals,Style/RedundantPercentQ,Style/RedundantCapitalW,Style/CharacterLiteral,Style/QuotedSymbols,Style/SymbolLiteral,Style/VariableInterpolation,Style/StringLiterals,Style/StringLiteralsInInterpolation,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Layout"
    );
    for select in [Some(peers.as_str()), None] {
        fs::write(d.path().join("a.rb"), src).unwrap();
        let before = ruby(d.path());
        let mut stable = false;
        for _ in 0..6 {
            let old = fs::read(d.path().join("a.rb")).unwrap();
            let mut args = vec!["--fix", "a.rb"];
            if let Some(names) = select {
                args.extend(["--only", names]);
            }
            let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(args)
                .output()
                .unwrap();
            assert_ne!(o.status.code(), Some(2), "{o:?}");
            same(&before, &ruby(d.path()));
            if old == fs::read(d.path().join("a.rb")).unwrap() {
                stable = true;
                break;
            }
        }
        assert!(stable);
        assert_ne!(fs::read_to_string(d.path().join("a.rb")).unwrap(), src);
    }
}
