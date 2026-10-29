//! parallel-assignment semantics and real CLI safety.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/TrailingUnderscoreVariable";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::create_dir(d.path().join("lib")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
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
fn unchanged(dir: &Path, source: &[u8], args: &[&str], status: i32) {
    let path = dir.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir, args, status);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
}
#[test]
fn registration_default_lint_and_single_pass_fix() {
    let r = rustocop::cops::Registry::new();
    let cops: Vec<_> = r.cops().iter().filter(|c| c.name() == COP).collect();
    assert_eq!(cops.len(), 1);
    assert!(cops[0].default_enabled());
    assert!(cops[0].safe_fixes());
    assert!(cops[0].path_policy().is_none());
    let d = project();
    unchanged(d.path(), b"a, _ = foo()\n", &["lib/a.rb"], 1);
    unchanged(d.path(), b"a, _ = foo()\n", &["--only", COP, "lib/a.rb"], 1);
    cli(d.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
    assert_eq!(
        fs::read(d.path().join("lib/a.rb")).unwrap(),
        b"a, = foo()\n"
    );
    unchanged(
        d.path(),
        b"a, = foo()\n",
        &["--only", COP, "--fix", "lib/a.rb"],
        0,
    );
}
#[test]
fn only_except_and_per_cop_department_enabling() {
    let d = project();
    for config in [
        format!("[cops.\"{COP}\"]\nenabled = false\n"),
        "[cops.Style]\nenabled = false\n".into(),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), b"a, _ = values\np a\n", &["--fix", "lib/a.rb"], 0);
        cli(d.path(), &["--only", COP, "lib/a.rb"], 1);
        cli(
            d.path(),
            &["--only", COP, "--except", COP, "--fix", "lib/a.rb"],
            0,
        );
        assert_eq!(
            fs::read(d.path().join("lib/a.rb")).unwrap(),
            b"a, _ = values\np a\n"
        );
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
    )
    .unwrap();
    cli(d.path(), &["--fix", "lib/a.rb"], 0);
    assert_eq!(
        fs::read(d.path().join("lib/a.rb")).unwrap(),
        b"a, = values\np a\n"
    );
}
#[test]
fn directives_and_atomic_disabled_spans() {
    let d = project();
    for prefix in ["rubocop", "rustocop"] {
        for selector in [COP, "TrailingUnderscoreVariable", "Style", "all"] {
            for s in [
                format!("# {prefix}:disable {selector}\na, _ = rhs\n"),
                format!("a, _ = rhs # {prefix}:disable {selector}\n"),
                format!("# {prefix}:disable-next {selector}\na, _ = rhs\n"),
            ] {
                unchanged(
                    d.path(),
                    s.as_bytes(),
                    &["--only", COP, "--fix", "lib/a.rb"],
                    0,
                );
            }
        }
    }
    // The whole removal range is suppressed, not just its first target.
    let s = format!("a, _, # rubocop:disable {COP}\n _ = rhs\n");
    unchanged(
        d.path(),
        s.as_bytes(),
        &["--only", COP, "--fix", "lib/a.rb"],
        0,
    );
}
#[test]
fn config_relative_excludes_and_stdin_are_read_only() {
    let d = project();
    let source = b"a, _ = rhs\n";
    for config in [
        String::new(),
        "exclude = [\"lib/a.rb\"]\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = [\"lib/a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), &config).unwrap();
        let status = i32::from(config.is_empty());
        unchanged(d.path(), source, &["--only", COP, "lib/a.rb"], status);
        cli(&d.path().join("lib"), &["--only", COP, "a.rb"], status);
        if status == 0 {
            cli(d.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
        }
        let mut p = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path().join("lib"))
            .args(["--only", COP, "--stdin", "a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        p.stdin.take().unwrap().write_all(source).unwrap();
        assert_eq!(p.wait_with_output().unwrap().status.code(), Some(status));
        assert_eq!(fs::read(d.path().join("lib/a.rb")).unwrap(), source);
    }
}
#[test]
fn lookalike_uncorrectable_and_error_files_keep_mtime_and_bytes() {
    let d = project();
    for (s, status) in [
        ("a, _name = rhs; a, *rest, _ = rhs\n", 0),
        (
            "f { |a, _| a }; f(a, _); [a, _]; {a: _}; def f(a,_); end\n",
            0,
        ),
        ("_, = rhs\n", 1),
        ("a, (_, _) = rhs\n", 1),
        ("a, _ = rhs; p _\n", 1),
        ("a, _ = rhs; p local_variables\n", 1),
        ("obj.x, _ = rhs\n", 1),
        ("a, _ = rhs; p DATA.pos\n__END__\nraw\n", 1),
        ("a, _ = [\n", 2),
    ] {
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            status,
        );
    }
}
#[test]
fn bom_crlf_encoding_comments_literals_line_and_payload_survive() {
    let d = project();
    let path = d.path().join("lib/a.rb");
    for source in [
        "\u{feff}# encoding: UTF-8\r\na, _ = [\"café\", 2] # keep\r\np a, __LINE__\r\n__END__\r\nraw\r\n".as_bytes().to_vec(),
        b"# encoding: ISO-8859-1\r\na, _ = [\"caf\xe9\", 2] # keep\r\np a, __LINE__\r\n__END__\r\nraw\xff\r\n".to_vec(),
    ] {
        let offset = source.windows(5).position(|b| b == b"a, _ ").unwrap()+3;
        let mut expected=source.clone(); expected.drain(offset..offset+2);
        fs::write(&path, &source).unwrap(); cli(d.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read(&path).unwrap(), expected);
    }
}
fn ruby(path: &Path) -> Option<Output> {
    Command::new("ruby").arg(path).output().ok()
}
fn same_observations(a: Output, b: Output) {
    assert_eq!(
        (a.status.code(), a.stdout, a.stderr),
        (b.status.code(), b.stdout, b.stderr)
    );
}
#[test]
fn ruby_forced_conversion_binding_expression_return_and_effect_counts() {
    let source = r#"$events = []
class Box
  def initialize(tag, values); @tag = tag; @values = values; end
  def to_ary; $events << @tag; @values; end
end
class Values
  def to_a; $events << :to_a; [7, 8, 9]; end
end
box = Box.new(:outer, [1, 2, 3])
value = (a, _ = box)
nested = Box.new(:nested, [4, 5])
b, (c, _) = Box.new(:second, [3, nested])
d, *_, _, _ = [6, 7, 8, 9]
array = (_, _ = *Values.new)
p [a, b, c, d, value.equal?(box), array, $events]
"#;
    let d = project();
    let path = d.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let before = ruby(&path);
    if let Some(b) = &before {
        assert!(b.status.success(), "{b:?}");
        assert_eq!(
            b.stdout,
            b"[1, 3, 4, 6, true, [7, 8, 9], [:outer, :second, :nested, :to_a]]\n"
        );
    }
    cli(d.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
    let after = fs::read_to_string(&path).unwrap();
    assert_ne!(after, source);
    assert!(after.contains("a, = box"));
    if let Some(before) = before {
        same_observations(before, ruby(&path).unwrap());
    }
}
#[test]
fn ruby_withheld_conversion_exception_and_local_observers_have_naive_counterexamples() {
    let d = project();
    let path = d.path().join("lib/a.rb");
    for (source, naive) in [
        (
            "$events=[]\nobj=Object.new\ndef obj.to_ary; $events << :convert; raise TypeError; end\nbegin; _, = obj; rescue TypeError; $events << :raised; end\np $events\n",
            "$events=[]\nobj=Object.new\ndef obj.to_ary; $events << :convert; raise TypeError; end\nbegin; obj; rescue TypeError; $events << :raised; end\np $events\n",
        ),
        (
            "obj=Object.new\ndef obj.to_ary; raise TypeError; end\nbegin; a, (_, _) = [1,obj]; rescue TypeError; puts :converted; end\n",
            "obj=Object.new\ndef obj.to_ary; raise TypeError; end\nbegin; a, _ = [1,obj]; rescue TypeError; puts :converted; end\n",
        ),
        (
            "a, _ = [1,2]; p defined?(_)\n",
            "a, = [1,2]; p defined?(_)\n",
        ),
        (
            "a, _ = [1,2]; p local_variables.include?(:_)\n",
            "a, = [1,2]; p local_variables.include?(:_)\n",
        ),
        (
            "a, _ = [1,2]; p binding.local_variable_get(:_)\n",
            "a, = [1,2]; p binding.local_variable_get(:_)\n",
        ),
        (
            "a, _ = [-> { _ },2]; p a.call\n",
            "a, = [-> { _ },2]; p a.call\n",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let before = ruby(&path);
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            1,
        );
        if let Some(before) = before {
            assert!(before.status.success(), "{source}: {before:?}");
            same_observations(before.clone(), ruby(&path).unwrap());
            fs::write(&path, naive).unwrap();
            let changed = ruby(&path).unwrap();
            assert_ne!(
                (before.status.code(), before.stdout),
                (changed.status.code(), changed.stdout),
                "{source}"
            );
        }
    }
}
#[test]
fn ruby_qualified_vm_observer_keeps_local_table_and_offense_without_writing() {
    let d = project();
    let path = d.path().join("lib/a.rb");
    // The CLI checks always run; InstructionSequence witnesses require MRI.
    let supports_iseq = Command::new("ruby")
        .args(["-e", "exit(defined?(RubyVM::InstructionSequence) ? 0 : 1)"])
        .output()
        .is_ok_and(|out| out.status.success());
    for observer in ["RubyVM", "::RubyVM", "Object::RubyVM", "::Object::RubyVM"] {
        let source =
            format!("p {observer}::InstructionSequence.of(-> {{ a, _ = [1, 2] }}).to_a[10]\n");
        fs::write(&path, &source).unwrap();
        let before = supports_iseq.then(|| ruby(&path).unwrap());
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            1,
        );
        if let Some(before) = before {
            assert!(before.status.success(), "{before:?}");
            assert_eq!(before.stdout, b"[:a, :_]\n");
            same_observations(before.clone(), ruby(&path).unwrap());
            fs::write(&path, source.replace("a, _ =", "a, =")).unwrap();
            let naive = ruby(&path).unwrap();
            assert!(naive.status.success(), "{naive:?}");
            assert_eq!(naive.stdout, b"[:a]\n");
            assert_ne!(before.stdout, naive.stdout);
        }
    }
}
#[test]
fn deferred_singleton_receiver_interpolation_keeps_its_outer_local_binding() {
    let source = "$seen=[]\ndef _; 10; end\ndef build(s); $seen << s; Object.new; end\na, _ = [def (build(<<~DOC)).tag; end, 2]\n#{_}\nDOC\np $seen\n";
    let d = project();
    let path = d.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let before = ruby(&path);
    unchanged(
        d.path(),
        source.as_bytes(),
        &["--only", COP, "--fix", "lib/a.rb"],
        1,
    );
    if let Some(before) = before {
        assert!(before.status.success(), "{before:?}");
        assert_eq!(before.stdout, b"[\"\\n\"]\n");
        same_observations(before.clone(), ruby(&path).unwrap());
        fs::write(&path, source.replace("a, _ =", "a, =")).unwrap();
        assert_ne!(before.stdout, ruby(&path).unwrap().stdout);
    }
}
#[test]
fn partial_corrections_preserve_conversion_exception_counts_and_partial_writes() {
    let source = "$events=[]\nobj=Object.new\ndef obj.to_ary; $events << :convert; :invalid; end\nbegin; a, _ = obj; rescue TypeError; $events << :raised; end\np [a, $events]\n";
    let d = project();
    let path = d.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let before = ruby(&path);
    cli(d.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        source.replace("a, _ =", "a, =")
    );
    if let Some(before) = before {
        assert!(before.status.success(), "{before:?}");
        assert_eq!(before.stdout, b"[nil, [:convert, :raised]]\n");
        same_observations(before, ruby(&path).unwrap());
    }
}
#[test]
fn ruby_data_positions_are_observable_but_unobserved_payloads_can_change_prefix() {
    let d = project();
    let path = d.path().join("lib/a.rb");
    let source = "a, _ = [1,2]; p [DATA.pos, DATA.read]\n__END__\npayload\n";
    fs::write(&path, source).unwrap();
    let before = ruby(&path);
    unchanged(
        d.path(),
        source.as_bytes(),
        &["--only", COP, "--fix", "lib/a.rb"],
        1,
    );
    if let Some(before) = before {
        assert!(before.status.success());
        fs::write(&path, source.replace("_ ", "")).unwrap();
        assert_ne!(before.stdout, ruby(&path).unwrap().stdout);
    }
}
#[test]
fn curated_comma_assignment_style_layout_composition_stabilizes() {
    let d = project();
    let path = d.path().join("lib/a.rb");
    let source = "def sample\n  a, _ = [1,2,]; b, (c, _) = [3,[4,5,]]; p(a,b,c,)\nend\nsample\n";
    for selection in [
        Some(
            "Style/TrailingUnderscoreVariable,Style/TrailingCommaInArrayLiteral,Style/TrailingCommaInArguments,Style/ParallelAssignment,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout",
        ),
        None,
    ] {
        // ParallelAssignment is now a real selected/default peer;
        // its discard-target hold leaves forced destructuring to this cop.
        let registry = rustocop::cops::Registry::new();
        let selected = selection.map(|s| {
            s.split(',')
                .filter(|n| !n.contains('/') || registry.has_cop(n))
                .collect::<Vec<_>>()
                .join(",")
        });
        fs::write(&path, source).unwrap();
        let before = ruby(&path);
        let mut stable = false;
        for _ in 0..6 {
            let old = fs::read(&path).unwrap();
            let mut args = vec!["--fix", "lib/a.rb"];
            if let Some(s) = &selected {
                args.extend(["--only", s.as_str()]);
            }
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(args)
                .output()
                .unwrap();
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            if fs::read(&path).unwrap() == old {
                stable = true;
                break;
            }
        }
        assert!(stable);
        assert!(!fs::read_to_string(&path).unwrap().contains("_"));
        if let Some(before) = before {
            assert!(before.status.success());
            same_observations(before, ruby(&path).unwrap());
        }
    }
}
#[test]
fn lexical_rhs_captures_survive_cli_fixes_with_independent_safe_edits() {
    let d = project();
    let path = d.path().join("lib/a.rb");
    for (body, expected) in [
        ("a, _ = [-> { _ }, 2]; p a.call", "2\n"),
        ("a, _ = [->(x = _) { x }, 2]; p a.call", "2\n"),
        ("a, _ = [-> { _ = 3 }, 2]; p [a.call, _]", "[3, 3]\n"),
        (
            "_, _ = 1, -> { _, _ = 8, 9 }; f = _; p [f.call, _]",
            "[[8, 9], 9]\n",
        ),
        (
            "a, _ = [-> { <<~TXT }, 2]; p a.call\n#{_}\nTXT\n",
            "\"2\\n\"\n",
        ),
        (
            "$seen=[]; def build(s); $seen << s; Object.new; end\na, _ = [-> { def (build(<<~TXT)).tag; end }, 2]\n#{_}\nTXT\na.call; p $seen",
            "[\"2\\n\"]\n",
        ),
    ] {
        let source =
            format!("def _; :method; end\n{body}\ndef safe; x, _ = [1, 2]; x; end\np safe\n");
        fs::write(&path, &source).unwrap();
        let before = ruby(&path);
        if let Some(before) = &before {
            assert!(before.status.success(), "{source}: {before:?}");
            assert_eq!(
                before.stdout,
                format!("{expected}1\n").as_bytes(),
                "{source}"
            );
        }
        cli(d.path(), &["--only", COP, "--fix", "lib/a.rb"], 1);
        let corrected = source.replace("x, _ =", "x, =");
        assert_eq!(fs::read(&path).unwrap(), corrected.as_bytes(), "{source}");
        if let Some(before) = before {
            same_observations(before, ruby(&path).unwrap());
        }
        unchanged(
            d.path(),
            corrected.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            1,
        );
    }
}

#[test]
fn prism_lexical_depth_and_cli_variable_liveness_agree() {
    use ruby_prism::Node;
    use rustocop::cops::walk::{NodeVisitor, walk};
    #[derive(Default)]
    struct Depths(Vec<(&'static str, u32)>);
    impl NodeVisitor<'_> for Depths {
        fn enter(&mut self, node: Node<'_>) {
            if let Some(n) = node.as_local_variable_read_node()
                && n.name().as_slice() == b"b"
            {
                self.0.push(("read", n.depth()));
            }
            if let Some(n) = node.as_local_variable_write_node()
                && n.name().as_slice() == b"b"
            {
                self.0.push(("write", n.depth()));
            }
            if let Some(n) = node.as_local_variable_target_node()
                && n.name().as_slice() == b"b"
            {
                self.0.push(("target", n.depth()));
            }
        }
    }
    let d = project();
    for (source, expected) in [
        (
            "a, b = [-> { b }, 2]; p a.call\n",
            vec![("target", 0), ("read", 1)],
        ),
        (
            "a, b = [-> { b = 3 }, 2]; p a.call, b\n",
            vec![("target", 0), ("write", 1), ("read", 0)],
        ),
        (
            "a, b = [->(x = b) { x }, 2]; p a.call\n",
            vec![("target", 0), ("read", 1)],
        ),
        (
            "a, b = [-> { def (build(<<~TXT)).m; end }, 2]; p a\n#{b}\nTXT\n",
            vec![("target", 0), ("read", 1)],
        ),
    ] {
        let parsed = ruby_prism::parse(source.as_bytes());
        assert_eq!(parsed.errors().count(), 0, "{source}");
        let mut depths = Depths::default();
        walk(&mut depths, &parsed.node());
        assert_eq!(depths.0, expected, "fixed-3.4 Prism: {source}");
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", "Lint/UselessAssignment", "--fix", "lib/a.rb"],
            0,
        );
    }
}

#[test]
fn lexical_capture_and_conversion_exception_order_is_unchanged() {
    let d = project();
    let path = d.path().join("lib/a.rb");
    let source = r#"$events=[]
class Broken
  def to_ary; $events << :convert; raise TypeError, "stop"; end
end
begin
  a, (_, _) = [-> { $events << :call; _ }, Broken.new]
rescue TypeError
  $events << :rescue
  p [a.call, $events]
end
"#;
    fs::write(&path, source).unwrap();
    let before = ruby(&path);
    unchanged(
        d.path(),
        source.as_bytes(),
        &["--only", COP, "--fix", "lib/a.rb"],
        1,
    );
    if let Some(before) = before {
        assert!(before.status.success(), "{before:?}");
        assert_eq!(before.stdout, b"[nil, [:convert, :rescue, :call]]\n");
        same_observations(before, ruby(&path).unwrap());
    }
}

#[test]
fn fixed_34_prism_binding_implicit_it_and_decoded_literals_are_unchanged() {
    use ruby_prism::Node;
    use rustocop::cops::walk::{NodeVisitor, walk};
    struct Observations<'a> {
        source: &'a [u8],
        items: Vec<String>,
    }
    impl NodeVisitor<'_> for Observations<'_> {
        fn enter(&mut self, node: Node<'_>) {
            if let Some(n) = node.as_local_variable_read_node() {
                self.items
                    .push(format!("read:{:?}:{}", n.name().as_slice(), n.depth()));
            }
            if let Some(n) = node.as_string_node() {
                self.items.push(format!("string:{:?}", n.unescaped()));
            }
            if node.as_it_local_variable_read_node().is_some() {
                self.items.push("implicit-it".into());
            }
            if let Some(n) = node.as_source_line_node() {
                let line = self.source[..n.location().start_offset()]
                    .iter()
                    .filter(|b| **b == b'\n')
                    .count()
                    + 1;
                self.items.push(format!("line:{line}"));
            }
        }
    }
    fn observe(source: &[u8]) -> Vec<String> {
        let parse = ruby_prism::parse(source);
        assert_eq!(parse.errors().count(), 0);
        let mut o = Observations {
            source,
            items: Vec::new(),
        };
        walk(&mut o, &parse.node());
        o.items
    }
    let d = project();
    let path = d.path().join("lib/a.rb");
    let source = "a, _ = [-> { it }, \"caf\\u00e9\"]\np a, __LINE__\n";
    fs::write(&path, source).unwrap();
    cli(d.path(), &["--only", COP, "--fix", "lib/a.rb"], 0);
    let after = fs::read(&path).unwrap();
    assert_ne!(source.as_bytes(), after);
    let before = observe(source.as_bytes());
    assert!(before.contains(&"implicit-it".into()));
    assert_eq!(before, observe(&after));
}
