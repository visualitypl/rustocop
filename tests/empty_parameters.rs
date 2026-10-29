//! actual CLI contracts and controlled Ruby observations (never corpus code).
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const BLOCK: &str = "Style/EmptyBlockParameter";
const LAMBDA: &str = "Style/EmptyLambdaParameter";
const BOTH: &str = "Style/EmptyBlockParameter,Style/EmptyLambdaParameter";
const CASES: [(&str, &str, &str); 2] = [
    (BLOCK, "proc { || 42 }\n", "proc { 42 }\n"),
    (LAMBDA, "-> () { 42 }\n", "-> { 42 }\n"),
];
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
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
fn unchanged(dir: &Path, source: &[u8], args: &[&str], status: i32) -> Output {
    let path = dir.join("x.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let out = cli(dir, args, status);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    out
}
#[test]
fn defaults_selectors_config_and_no_write() {
    let dir = project();
    for (cop, before, after) in CASES {
        let registry = rustocop::cops::Registry::new();
        let registered: Vec<_> = registry.cops().iter().filter(|c| c.name() == cop).collect();
        assert_eq!(registered.len(), 1);
        assert!(registered[0].default_enabled() && registered[0].safe_fixes());
        assert!(registered[0].path_policy().is_none());
        assert!(registered[0].ruby_versions().minimum.is_none());
        assert!(registered[0].ruby_versions().maximum.is_none());
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        let out = unchanged(dir.path(), before.as_bytes(), &["x.rb"], 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
        unchanged(dir.path(), before.as_bytes(), &["--only", cop, "x.rb"], 1);
        for config in [
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            unchanged(dir.path(), before.as_bytes(), &["--fix", "x.rb"], 0);
            unchanged(dir.path(), before.as_bytes(), &["--only", cop, "x.rb"], 1);
            unchanged(
                dir.path(),
                before.as_bytes(),
                &["--only", cop, "--except", cop, "--fix", "x.rb"],
                0,
            );
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        cli(dir.path(), &["--fix", "x.rb"], 0);
        assert_eq!(fs::read_to_string(dir.path().join("x.rb")).unwrap(), after);
        unchanged(
            dir.path(),
            after.as_bytes(),
            &["--only", cop, "--fix", "x.rb"],
            0,
        );
    }
}
#[test]
fn excludes_directives_and_source_only_enabling() {
    let dir = project();
    for (cop, before, after) in CASES {
        for config in [
            "exclude = [\"*.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = [\"*.rb\"]\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                dir.path(),
                before.as_bytes(),
                &["--only", cop, "--fix", "x.rb"],
                0,
            );
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rubocop", "rustocop"] {
            for target in [cop, "Style", "all"] {
                for directive in ["disable", "disable-next"] {
                    let source = format!("# {prefix}:{directive} {target}\n{before}");
                    unchanged(
                        dir.path(),
                        source.as_bytes(),
                        &["--only", cop, "--fix", "x.rb"],
                        0,
                    );
                }
                let source = format!("{} # {prefix}:disable {target}\n", before.trim_end());
                unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", cop, "--fix", "x.rb"],
                    0,
                );
            }
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
        )
        .unwrap();
        let header = format!("# rustocop:enable {cop}\n");
        fs::write(dir.path().join("x.rb"), format!("{header}{before}")).unwrap();
        cli(dir.path(), &["--fix", "x.rb"], 0);
        assert_eq!(
            fs::read_to_string(dir.path().join("x.rb")).unwrap(),
            format!("{header}{after}")
        );
    }
}
#[test]
fn stdin_is_report_only_and_fix_is_rejected() {
    use std::{io::Write, process::Stdio};
    let dir = project();
    for (cop, source, _) in CASES {
        let path = dir.path().join("x.rb");
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", cop, "--stdin", "x.rb"])
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
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
        cli(dir.path(), &["--only", cop, "--fix", "--stdin", "x.rb"], 2);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn byte_exact_bom_crlf_encoding_comments_heredocs_data_and_line_values() {
    let dir = project();
    for (before, after) in [
        ("\u{feff}# encoding: UTF-8\r\np = proc { || [\"café\", __LINE__] }\r\nl = ->() { __LINE__ } # keep\r\np __LINE__\r\n__END__\r\nrawÿ\r\n".as_bytes(), "\u{feff}# encoding: UTF-8\r\np = proc { [\"café\", __LINE__] }\r\nl = -> { __LINE__ } # keep\r\np __LINE__\r\n__END__\r\nrawÿ\r\n".as_bytes()),
        (b"# encoding: ISO-8859-1\r\np = proc { || \"caf\xe9\" }\r\nl = ->() { \"caf\xe9\" }\r\n__END__\r\nraw\xff\r\n", b"# encoding: ISO-8859-1\r\np = proc { \"caf\xe9\" }\r\nl = -> { \"caf\xe9\" }\r\n__END__\r\nraw\xff\r\n"),
        (b"p = proc { || <<~TEXT\n  #{__LINE__} it || ()\nTEXT\n}\nl = ->() { <<~TEXT }\n  #{__LINE__} it || ()\nTEXT\np __LINE__\n", b"p = proc { <<~TEXT\n  #{__LINE__} it || ()\nTEXT\n}\nl = -> { <<~TEXT }\n  #{__LINE__} it || ()\nTEXT\np __LINE__\n"),
    ] {
        fs::write(dir.path().join("x.rb"), before).unwrap();
        cli(dir.path(), &["--only", BOTH, "--fix", "x.rb"], 0);
        assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), after);
    }
}
#[test]
fn clean_uncorrectable_and_invalid_files_keep_bytes_and_mtime() {
    let dir = project();
    for (source, status) in [
        ("proc { 1 }; -> { 2 }\n", 0),
        ("proc { | # retain\n| 1 }; ->( # retain\n) { 2 }\n", 1),
        ("proc { || 1 }; ->() { 2 }; p DATA.pos\n__END__\nraw\n", 1),
        ("proc { || it }\n", 2),
        ("->() { it }\n", 2),
        ("proc { || _1 }\n", 2),
        ("->() { _1 }\n", 2),
        ("proc { ||\n", 2),
    ] {
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", BOTH, "--fix", "x.rb"],
            status,
        );
    }
}
fn ruby_available() -> bool {
    Command::new("ruby").arg("--version").output().is_ok()
}
fn ruby(dir: &Path) -> Output {
    Command::new("ruby")
        .current_dir(dir)
        .arg("x.rb")
        .output()
        .unwrap()
}
fn runtime_case(source: &str, expected: &str) {
    let dir = project();
    let path = dir.path().join("x.rb");
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path());
    assert!(before.status.success(), "{source}: {before:?}");
    assert_eq!(String::from_utf8_lossy(&before.stdout), expected);
    cli(dir.path(), &["--only", BOTH, "--fix", "x.rb"], 0);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        source.replace(" ||", "").replace("->()", "->")
    );
    let after = ruby(dir.path());
    assert!(after.status.success(), "{after:?}");
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
}
#[test]
fn runtime_proc_lambda_arity_arguments_and_nested_captures() {
    if !ruby_available() {
        return;
    }
    runtime_case(
        "it = 7\nps = [proc { || it }, lambda { || it }, Proc.new { || it }, ->() { it }]\np ps.map { |pr| [pr.lambda?, pr.arity, pr.parameters, pr.call] }\np ps.map { |pr| begin; pr.call(99); rescue ArgumentError; :arity_error; end }\n",
        "[[false, 0, [], 7], [true, 0, [], 7], [false, 0, [], 7], [true, 0, [], 7]]\n[7, :arity_error, 7, :arity_error]\n",
    );
    runtime_case(
        "def it; 9; end\nx = 4\npr = proc { || [it(), {it:}, x, [2].map { |y| x + y }] }\nl = ->() { x += 1; x }\np [pr.call, l.call, x]\n",
        "[[9, {:it=>9}, 4, [6]], 5, 5]\n",
    );
}
#[test]
fn runtime_return_break_next_ensure_and_block_association() {
    if !ruby_available() {
        return;
    }
    runtime_case(
        "def f\n  log = []\n  result = [1].each { || begin; break 3; ensure; log << :ensure; end }\n  p [result, log]\n  [1].map { || next 4 }.tap { |xs| p xs }\n  [1].each { || return :returned }\nend\np f\np (->() { return 5 }).call\np (lambda { || next 6 }).call\n",
        "[3, [:ensure]]\n[4]\n:returned\n5\n6\n",
    );
    runtime_case(
        "def inner; block_given? ? [:inner, yield] : :no_block; end\ndef outer(x); [x, block_given?]; end\np outer inner { || :brace }\nresult = outer inner do || :keyword end\np result\n",
        "[[:inner, :brace], false]\n[:no_block, true]\n",
    );
}
#[test]
fn runtime_literal_heredoc_line_and_data_payload_values() {
    if !ruby_available() {
        return;
    }
    runtime_case(
        "pr = proc { || <<~TEXT\n  café #{__LINE__}\nTEXT\n}\nl = ->() { [__LINE__, \"é\"] }\np [pr.call, l.call, __LINE__]\n__END__\nraw\n",
        "[\"café 2\\n\", [5, \"é\"], 6]\n",
    );
}
#[test]
fn runtime_data_position_naive_edit_witnesses() {
    if !ruby_available() {
        return;
    }
    let dir = project();
    for (cop, before, after) in CASES {
        for observer in [
            "DATA.pos",
            "Object.const_get(:DATA).tell",
            "[DATA].map(&:pos)",
        ] {
            let source = format!("{before}p {observer}\n__END__\nraw\n");
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "x.rb"],
                1,
            );
            let original = ruby(dir.path());
            assert!(original.status.success(), "{original:?}");
            fs::write(dir.path().join("x.rb"), source.replace(before, after)).unwrap();
            let naive = ruby(dir.path());
            assert!(naive.status.success(), "{naive:?}");
            assert_ne!(original.stdout, naive.stdout);
        }
    }
}
#[test]
fn explicit_fixed_34_prism_values_and_parameter_kinds() {
    // Host Ruby is 3.2 here: use Prism's explicit 3.4 parser for modern binding
    // evidence, without pretending it is a Ruby 3.4 runtime execution.
    let probe = Command::new("ruby")
        .args(["-rprism", "-e", "Prism.parse('nil', version: '3.4.0')"])
        .output();
    if !probe.is_ok_and(|o| o.status.success()) {
        eprintln!("skip: Ruby/Prism 3.4 parser unavailable");
        return;
    }
    let script = r#"
      def parse(s)
        Prism.parse(s, version: '3.4.0')
      end
      def kinds(n)
        [n.class.name, *n.compact_child_nodes.flat_map { |c| kinds(c) }]
      end
      ['proc { || it }', '->() { it }', 'proc { || _1 }', '->() { _1 }'].each do |s|
        raise s if parse(s).errors.empty?
      end
      ['proc { it }', '-> { it }'].each do |s|
        r = parse(s); raise s unless r.errors.empty?
        raise s unless kinds(r.value).include?('Prism::ItParametersNode') && kinds(r.value).include?('Prism::ItLocalVariableReadNode')
      end
      ['it = 7; proc { || it }', 'it = 7; ->() { it }', 'proc { || {it:} }', '->() { {it:} }'].each do |s|
        a = parse(s); b = parse(s.gsub(' ||', '').gsub('->()', '->'))
        raise s unless a.errors.empty? && b.errors.empty?
        # Removing explicit empty syntax does NOT introduce implicit parameters.
        raise s if kinds(b.value).any? { |k| ['Prism::ItParametersNode', 'Prism::NumberedParametersNode'].include?(k) }
        left = kinds(a.value).reject { |k| k == 'Prism::BlockParametersNode' }
        raise s unless left == kinds(b.value)
      end
      puts '3.4 AST binding/parameter evidence passed'
    "#;
    let out = Command::new("ruby")
        .args(["-rprism", "-e", script])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(out.stdout, b"3.4 AST binding/parameter evidence passed\n");
}
#[test]
fn all_defaults_with_layout_preserve_runtime_and_stabilize() {
    let dir = project();
    let path = dir.path().join("x.rb");
    let source = "result=[1].map{||\n  7\n}\nf=->(){8}\np [result,f.call]\n";
    fs::write(&path, source).unwrap();
    let original = ruby_available().then(|| ruby(dir.path()));
    if let Some(out) = &original {
        assert!(out.status.success(), "{out:?}");
        assert_eq!(out.stdout, b"[[7], 8]\n");
    }
    for pass in 0..5 {
        let before = fs::read(&path).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--fix", "x.rb"])
            .output()
            .unwrap();
        assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
        let after = fs::read(&path).unwrap();
        if let Some(original) = &original {
            let fixed = ruby(dir.path());
            assert!(fixed.status.success(), "{fixed:?}");
            assert_eq!(fixed.stdout, original.stdout);
            assert_eq!(fixed.stderr, original.stderr);
        }
        if after == before {
            assert_eq!(out.status.code(), Some(0));
            let text = String::from_utf8(after).unwrap();
            assert!(!text.contains("||") && !text.contains("->()"));
            assert!(text.contains("map do"));
            assert!(pass > 0);
            unchanged(dir.path(), text.as_bytes(), &["--fix", "x.rb"], 0);
            return;
        }
    }
    panic!("default fixture did not stabilize in five explicit passes");
}

#[test]
fn block_arguments_layout_and_original_style_composition_stabilizes() {
    let dir = project();
    let only = format!(
        "{BOTH},Style/BlockDelimiters,Style/MethodCallWithArgsParentheses,Layout/SpaceInsideBlockBraces,Layout/SpaceAroundOperators,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/SignalException"
    );
    let source = "def f\n  outer inner{ ||\n    return {'key' => 'value'}\n  }\n  consume ->() { 1+2 }\nend\n";
    fs::write(dir.path().join("x.rb"), source).unwrap();
    let mut snapshots = vec![source.to_string()];
    for _ in 0..5 {
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", &only, "--fix", "x.rb"])
            .output()
            .unwrap();
        assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
        let current = fs::read_to_string(dir.path().join("x.rb")).unwrap();
        if current == *snapshots.last().unwrap() {
            assert_eq!(out.status.code(), Some(0));
            assert!(!current.contains("||") && !current.contains("->()"));
            assert!(current.contains("outer(inner"));
            assert!(current.contains("1 + 2"));
            unchanged(
                dir.path(),
                current.as_bytes(),
                &["--only", &only, "--fix", "x.rb"],
                0,
            );
            return;
        }
        assert!(!snapshots.contains(&current), "correction cycle");
        snapshots.push(current);
    }
    panic!("fixture did not stabilize in five explicit passes: {snapshots:?}");
}
