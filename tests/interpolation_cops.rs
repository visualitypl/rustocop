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
fn bom_crlf_line_data_and_legacy_encoding_survive_real_safe_edits() {
    let d = project();
    for source in [
        "\u{feff}# encoding: UTF-8\r\np +\"#{1} café\"\r\np __LINE__\r\n__END__\r\nraw\\payload\r\n".as_bytes(),
        b"# encoding: ISO-8859-1\np +\"#{1} caf\xe9\"\np __LINE__\n__END__\nraw\n",
    ] {
        fs::write(d.path().join("a.rb"),source).unwrap(); let before = has_ruby().then(|| ruby(d.path()));
        cli(d.path(), &["--only",U,"--fix","a.rb"],0); let after = fs::read(d.path().join("a.rb")).unwrap();
        let plus = source.iter().position(|&b| b == b'+').unwrap(); let mut expected = source.to_vec(); expected.remove(plus); assert_eq!(after,expected);
        if let Some(before) = before { same(&before,&ruby(d.path())); }
    }
    for encoding in ["UTF-8", "ISO-8859-1", "US-ASCII", "ASCII-8BIT"] {
        let source = format!(
            "# encoding: {encoding}\n# frozen_string_literal: false\nv = \"#{{nil}}#{{true ? '' : 'yes'}}\"\np [v.bytes, v.encoding.name, v.frozen?]\nv << 'tail'\np v\n"
        );
        fs::write(d.path().join("a.rb"), &source).unwrap();
        let before = has_ruby().then(|| ruby(d.path()));
        unchanged(d.path(), E, source.as_bytes(), 1);
        if let Some(before) = before {
            same(&before, &ruby(d.path()));
        }
    }
    for name in [E, U] {
        let source = if name == E {
            "p \"#{true ? 'x' : nil}\"\np DATA.pos\n__END__\npayload\n"
        } else {
            "p +\"#{1}\"\np DATA.pos\n__END__\npayload\n"
        };
        unchanged(d.path(), name, source.as_bytes(), 1);
    }
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
fn hostile_to_s_and_copy_hooks_have_parseable_runtime_counterexamples() {
    if !has_ruby() {
        return;
    }
    let d = project();
    for (cop, before, naive) in [
        (
            E,
            "class NilClass; def to_s; 'N'; end; end\np \"#{false ? 'x' : ''}\"\n",
            "class NilClass; def to_s; 'N'; end; end\np \"#{'x' if false}\"\n",
        ),
        (
            E,
            "def c; class << nil; def to_s; 'N'; end; end; false; end\np \"#{c ? 'x' : ''}\"\n",
            "def c; class << nil; def to_s; 'N'; end; end; false; end\np \"#{'x' if c}\"\n",
        ),
        (
            U,
            "class String; def initialize_copy(x); super; self << 'copy'; end; end\np \"#{1}\".dup\n",
            "class String; def initialize_copy(x); super; self << 'copy'; end; end\np \"#{1}\"\n",
        ),
        (
            U,
            "class String; def +@; 'plus'; end; end\np +\"#{1}\"\n",
            "class String; def +@; 'plus'; end; end\np \"#{1}\"\n",
        ),
        (
            U,
            "class String; def initialize(x); replace(x); self << 'new'; end; end\np String.new(\"#{1}\")\n",
            "class String; def initialize(x); replace(x); self << 'new'; end; end\np \"#{1}\"\n",
        ),
    ] {
        fs::write(d.path().join("a.rb"), before).unwrap();
        let original = ruby(d.path());
        assert!(original.status.success(), "{original:?}");
        unchanged(d.path(), cop, before.as_bytes(), 1);
        same(&original, &ruby(d.path()));
        assert_eq!(ruby_prism::parse(naive.as_bytes()).errors().count(), 0);
        fs::write(d.path().join("a.rb"), naive).unwrap();
        let changed = ruby(d.path());
        assert!(changed.status.success(), "{changed:?}");
        assert_ne!(original.stdout, changed.stdout);
    }
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
fn known_source_observers_and_binding_risks_keep_reported_files_unchanged() {
    let d = project();
    for (name, source, _) in CASES {
        for observer in [
            "p File.binread($0)",
            "p method(:p).source_location",
            "p RubyVM::AbstractSyntaxTree",
        ] {
            unchanged(
                d.path(),
                name,
                format!("{source}{observer}\n").as_bytes(),
                1,
            );
        }
    }
    for source in [
        "x = nil; p \"#{(x = true) ? x : nil}\"\n",
        "p \"#{x ? (x = 1) : nil}\"\n",
    ] {
        unchanged(d.path(), E, source.as_bytes(), 1);
    }
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
// A deliberately closed program: all candidates run before observations, with
// no definitions, dynamic predicates, or earlier calls to mask the folding bug.
fn constant_branch_matrix(header: &str) -> String {
    let mut assignments = String::from(header);
    let mut observations = String::new();
    let mut i = 0;
    for predicate in ["true", "false", "nil", "0"] {
        for expr in [
            format!("{predicate} ? '' : 'yes'"),
            format!("{predicate} ? 'yes' : ''"),
        ] {
            for body in [
                format!("\"#{{{expr}}}\""),
                format!("\"pré#{{{expr}}}suffix\""),
                format!("\"#{{\"#{{{expr}}}\"}}\""),
                format!("\"#{{1}}#{{{expr}}}#{{''}}\""),
                format!("\"#{{''}}#{{{expr}}}#{{1}}\""),
                format!("\"#{{1}}#{{{expr}}}\""),
                format!("\"#{{{expr}}}#{{1}}\""),
            ] {
                for wrapped in [
                    body.clone(),
                    format!("+{body}"),
                    format!("{body}.dup"),
                    format!("String.new({body})"),
                ] {
                    assignments.push_str(&format!("v{i} = {wrapped}\nw{i} = {wrapped}\n"));
                    observations.push_str(&format!(
                        "p [v{i}.bytes, v{i}.encoding.name, v{i}.frozen?, v{i}.equal?(w{i})]\nv{i} << '!' unless v{i}.frozen?\np [v{i}, w{i}]\n"
                    ));
                    i += 1;
                }
            }
        }
    }
    assignments + &observations
}
fn constant_branch_passes(executable: Option<&str>) {
    let d = project();
    let peers = format!(
        "{ALL},Style/StringLiterals,Style/StringLiteralsInInterpolation,Style/RedundantParentheses,Style/Semicolon,Style/HashSyntax,Style/RedundantReturn,Style/SignalException,Style/BarePercentLiterals,Style/PercentLiteralDelimiters,Style/PercentQLiterals,Style/RedundantPercentQ,Style/QuotedSymbols,Style/SymbolLiteral,Style/VariableInterpolation,Layout"
    );
    let run = || {
        executable.map(|exe| {
            Command::new(exe)
                .args(["--disable-gems", "-W:deprecated", "a.rb"])
                .current_dir(d.path())
                .output()
                .unwrap()
        })
    };
    for header in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
    ] {
        let source = constant_branch_matrix(header);
        for selection in [
            Some(E),
            Some(U),
            Some(R),
            Some(ALL),
            Some(peers.as_str()),
            None,
        ] {
            fs::write(d.path().join("a.rb"), &source).unwrap();
            let before = run();
            let mut stable = false;
            for _ in 0..6 {
                let old = fs::read(d.path().join("a.rb")).unwrap();
                let mtime = fs::metadata(d.path().join("a.rb"))
                    .unwrap()
                    .modified()
                    .unwrap();
                let mut args = vec!["--fix", "a.rb"];
                if let Some(cops) = selection {
                    args.extend(["--only", cops]);
                }
                let o = cli(d.path(), &args, 1); // Retained unsafe/uncertain offenses.
                let after = fs::read(d.path().join("a.rb")).unwrap();
                if matches!(selection, Some(E) | Some(U) | Some(R) | Some(ALL)) {
                    assert_eq!(after, source.as_bytes(), "{selection:?}: {o:?}");
                    assert_eq!(
                        fs::metadata(d.path().join("a.rb"))
                            .unwrap()
                            .modified()
                            .unwrap(),
                        mtime
                    );
                }
                if let Some(before) = &before {
                    let observed = run().unwrap();
                    assert!(before.status.success() && observed.status.success());
                    let expected = String::from_utf8_lossy(&before.stdout);
                    let actual = String::from_utf8_lossy(&observed.stdout);
                    for (i, (a, b)) in expected.lines().zip(actual.lines()).enumerate() {
                        assert_eq!(a, b, "{selection:?} {header:?} observation {i}");
                    }
                    assert_eq!(expected.lines().count(), actual.lines().count());
                    assert_eq!(before.stderr, observed.stderr);
                }
                if old == after {
                    stable = true;
                    break;
                }
            }
            assert!(stable, "{selection:?} {header}");
        }
    }
}
fn proven_primitive_passes(executable: Option<&str>) {
    let d = project();
    let peers = format!(
        "{ALL},Style/StringLiterals,Style/StringLiteralsInInterpolation,Style/RedundantParentheses,Style/Semicolon,Style/HashSyntax,Style/RedundantReturn,Style/SignalException,Layout"
    );
    for header in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
    ] {
        let mut source = header.to_owned();
        let mut expressions = Vec::new();
        for predicate in ["true", "false", "nil", "0"] {
            expressions.push(format!("\"#{{{predicate} ? 'yes' : nil}}\""));
            expressions.push(format!("\"#{{{predicate} ? nil : 'yes'}}\""));
        }
        for value in [
            "1",
            "0",
            "1.25",
            "true",
            "false",
            "nil",
            "1}#{nil",
            "nil}#{false",
            "nil}#{nil",
        ] {
            let body = format!("\"#{{{value}}}\"");
            expressions.extend([
                format!("+{body}"),
                format!("{body}.dup"),
                format!("String.new({body})"),
            ]);
        }
        for (i, expr) in expressions.iter().enumerate() {
            source.push_str(&format!("v{i} = {expr}\nw{i} = {expr}\n"));
        }
        for i in 0..expressions.len() {
            source.push_str(&format!("p [v{i}.bytes, v{i}.encoding.name, v{i}.frozen?, v{i}.equal?(w{i})]\nv{i} << '!' unless v{i}.frozen?\np [v{i}, w{i}]\n"));
        }
        let run = || {
            executable.map(|exe| {
                Command::new(exe)
                    .args(["--disable-gems", "-W:deprecated", "a.rb"])
                    .current_dir(d.path())
                    .output()
                    .unwrap()
            })
        };
        for selection in [Some(E), Some(U), Some(ALL), Some(peers.as_str()), None] {
            fs::write(d.path().join("a.rb"), &source).unwrap();
            let before = run();
            let mut stable = false;
            for pass in 0..6 {
                let old = fs::read(d.path().join("a.rb")).unwrap();
                let mut args = vec!["--fix", "a.rb"];
                if let Some(cops) = selection {
                    args.extend(["--only", cops]);
                }
                let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                    .current_dir(d.path())
                    .args(args)
                    .output()
                    .unwrap();
                assert!(matches!(o.status.code(), Some(0 | 1)), "{o:?}");
                let after = fs::read(d.path().join("a.rb")).unwrap();
                if pass == 0 {
                    assert_ne!(old, after, "canonical corrections withheld: {selection:?}");
                }
                if let Some(before) = &before {
                    same(before, &run().unwrap());
                }
                if old == after {
                    stable = true;
                    break;
                }
            }
            assert!(stable);
        }
    }
}
#[test]
fn proven_primitives_still_correct_and_preserve_runtime_values_identity_and_mutation() {
    proven_primitive_passes(has_ruby().then_some("ruby"));
}
#[test]
fn constant_branches_preserve_cli_bytes_and_host_runtime_across_individual_peer_default_passes() {
    // Local host is Ruby 3.2.3, not evidence of 3.4 compiler/chilled semantics.
    constant_branch_passes(has_ruby().then_some("ruby"));
}
#[test]
fn gated_ruby34_constant_branch_runtime() {
    // Set an existing local executable; never install/fetch a runtime in tests.
    let exe = crate::support::runtime::ruby().unwrap_or_else(|_| "ruby".into());
    let version = Command::new(&exe)
        .args(["--disable-gems", "-e", "print RUBY_VERSION"])
        .output();
    if !version.as_ref().is_ok_and(|o| {
        o.status.success()
            && o.stdout
                .starts_with(crate::support::runtime::version_prefix().as_bytes())
    }) {
        eprintln!(
            "SKIP Ruby 3.4 runtime (set RUSTOCOP_RUBY34); host Ruby/Prism checks are separate"
        );
        return;
    }
    constant_branch_passes(Some(&exe));
    proven_primitive_passes(Some(&exe));
}
#[test]
fn primitive_sibling_runtime_counterexamples_retain_bytes_and_mtime() {
    let d = project();
    for header in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
    ] {
        let mut cases = Vec::new();
        for body in [r##""#{1}#{''}""##, r##""#{''}#{1}""##] {
            for wrapped in [
                format!("+{body}"),
                format!("{body}.+@"),
                format!("{body}.dup"),
                format!("String.new({body})"),
            ] {
                cases.push((U, wrapped, body.to_owned()));
            }
        }
        for (expr, modifier) in [
            ("true ? '' : 'yes'", "'yes' unless true"),
            ("false ? 'yes' : ''", "'yes' if false"),
        ] {
            cases.push((
                E,
                format!("\"#{{1}}#{{{expr}}}#{{''}}\""),
                format!("\"#{{1}}#{{{modifier}}}#{{''}}\""),
            ));
        }
        for (cop, before, naive) in cases {
            let observe = "\np [v.bytes, v.encoding.name, v.frozen?]\nv << '!'\np v\n";
            let source = format!("{header}v = {before}{observe}");
            fs::write(d.path().join("a.rb"), &source).unwrap();
            let original = has_ruby().then(|| ruby(d.path()));
            unchanged(d.path(), cop, source.as_bytes(), 1);
            if let Some(original) = original {
                same(&original, &ruby(d.path()));
                fs::write(
                    d.path().join("a.rb"),
                    format!("{header}v = {naive}{observe}"),
                )
                .unwrap();
                let changed = ruby(d.path());
                // Both findings have concrete witnesses on the installed host.
                // No inference about the 3.4 compiler is made from this check.
                let version = Command::new("ruby")
                    .args(["-e", "print RUBY_VERSION"])
                    .output()
                    .unwrap();
                if version.stdout.starts_with(b"3.2.") && before.contains("#{1}#{") {
                    assert!(
                        original.stdout != changed.stdout || original.status != changed.status,
                        "{source}"
                    );
                }
            }
        }
    }
}
#[test]
fn controlled_host_frozen_branch_counterexamples_are_not_parse_proofs() {
    if !has_ruby() {
        return;
    }
    let d = project();
    for predicate in ["true", "false", "nil", "0"] {
        // Always select the immutable empty branch, without evaluating general Ruby.
        let truthy = matches!(predicate, "true" | "0");
        let expr = if truthy {
            format!("{predicate} ? '' : 'yes'")
        } else {
            format!("{predicate} ? 'yes' : ''")
        };
        let naive = if truthy {
            format!("'yes' unless {predicate}")
        } else {
            format!("'yes' if {predicate}")
        };
        let body = format!("\"#{{{expr}}}\"");
        for (cop, before, after) in [
            (E, body.clone(), format!("\"#{{{naive}}}\"")),
            (U, format!("+{body}"), body.clone()),
            (U, format!("{body}.dup"), body.clone()),
            (U, format!("String.new({body})"), body.clone()),
        ] {
            let source = format!(
                "# frozen_string_literal: true\nv = {before}\np [v.bytes, v.encoding.name, v.frozen?]\n"
            );
            fs::write(d.path().join("a.rb"), &source).unwrap();
            let original = ruby(d.path());
            unchanged(d.path(), cop, source.as_bytes(), 1);
            same(&original, &ruby(d.path()));
            let rewritten = source.replace(&format!("v = {before}"), &format!("v = {after}"));
            assert_eq!(ruby_prism::parse(rewritten.as_bytes()).errors().count(), 0);
            fs::write(d.path().join("a.rb"), rewritten).unwrap();
            let changed = ruby(d.path());
            assert!(changed.status.success());
            // This is a local runtime witness, NOT a target-version deduction.
            let version = Command::new("ruby")
                .args(["--disable-gems", "-e", "print RUBY_VERSION"])
                .output()
                .unwrap();
            if version.stdout.starts_with(b"3.2.") {
                assert_ne!(original.stdout, changed.stdout);
            }
        }
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
