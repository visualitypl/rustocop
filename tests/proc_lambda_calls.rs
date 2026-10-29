//! real selectors/config/bytes and controlled Ruby semantics.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const CASES: [(&str, &str, &str); 3] = [
    ("Style/LambdaCall", "x.(a, b)\n", "x.call(a, b)\n"),
    (
        "Style/Proc",
        "f = Proc.new { |x| x }\n",
        "f = proc { |x| x }\n",
    ),
    (
        "Style/StabbyLambdaParentheses",
        "f = ->a, b { [a, b] }\n",
        "f = ->(a, b) { [a, b] }\n",
    ),
];
const ONLY: &str = "Style/LambdaCall,Style/Proc,Style/StabbyLambdaParentheses";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn cli(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}
fn fix(dir: &Path, cop: &str) {
    let out = cli(dir, &["--only", cop, "--fix", "x.rb"]);
    assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], status: i32) {
    let path = dir.join("x.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let out = cli(dir, args);
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), mtime);
}
#[test]
fn default_on_selectors_and_config_precedence_with_no_write_without_fix() {
    let dir = project();
    for (cop, before, after) in CASES {
        for args in [vec!["x.rb"], vec!["--only", cop, "x.rb"]] {
            unchanged(dir.path(), before.as_bytes(), &args, 1);
            assert!(String::from_utf8_lossy(&cli(dir.path(), &args).stdout).contains(cop));
        }
        unchanged(
            dir.path(),
            before.as_bytes(),
            &["--only", cop, "--except", cop, "--fix", "x.rb"],
            0,
        );
        for (config, args, expected) in [
            (
                "[cops.Style]\nenabled=false\n".to_string(),
                vec!["--fix", "x.rb"],
                before,
            ),
            (
                format!("[cops.\"{cop}\"]\nenabled=false\n"),
                vec!["--only", cop, "--fix", "x.rb"],
                after,
            ),
            (
                format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
                vec!["--fix", "x.rb"],
                after,
            ),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            fs::write(dir.path().join("x.rb"), before).unwrap();
            let out = cli(dir.path(), &args);
            assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
            assert_eq!(
                fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                expected,
                "{cop}"
            );
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn both_directive_prefixes_and_config_excludes_preserve_bytes_and_mtime() {
    let dir = project();
    for (cop, before, _) in CASES {
        for prefix in ["rubocop", "rustocop"] {
            for target in [cop, "Style", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\n{before}"),
                    format!("# {prefix}:disable-next {target}\n{before}"),
                    format!("{} # {prefix}:disable {target}\n", before.trim_end()),
                ] {
                    unchanged(
                        dir.path(),
                        source.as_bytes(),
                        &["--only", cop, "--fix", "x.rb"],
                        0,
                    );
                }
            }
        }
        for config in [
            format!("[cops.\"{cop}\"]\nexclude=[\"x.rb\"]\n"),
            "exclude=[\"x.rb\"]\n".into(),
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
    }
}
#[test]
fn wrapping_and_chained_edits_are_atomic_across_disabled_lines() {
    let dir = project();
    for (cop, source) in [
        (
            "Style/StabbyLambdaParentheses",
            "f = ->a, # rubocop:disable Style/StabbyLambdaParentheses\n b { [a,b] }\n",
        ),
        (
            "Style/LambdaCall",
            "x.(\n a # rustocop:disable Style/LambdaCall\n)\n",
        ),
    ] {
        // Comment-bearing sugar is an upstream detection exemption; a wrapped
        // parameter span is indivisible even if only its first line is disabled.
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "x.rb"],
            0,
        );
    }
    let source = "f = ->a,\n b do # rubocop:disable Style/StabbyLambdaParentheses\n [a,b]\nend\n";
    unchanged(
        dir.path(),
        source.as_bytes(),
        &["--only", "Style/StabbyLambdaParentheses", "--fix", "x.rb"],
        0,
    );
}
#[test]
fn stdin_selection_and_fix_rejection_never_write_to_disk() {
    let dir = project();
    let path = dir.path().join("x.rb");
    fs::write(&path, "on disk\n").unwrap();
    for (cop, before, _) in CASES {
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
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
        let out = cli(dir.path(), &["--only", cop, "--fix", "--stdin", "x.rb"]);
        assert_eq!(out.status.code(), Some(2));
        assert_eq!(fs::read_to_string(&path).unwrap(), "on disk\n");
    }
}
#[test]
fn clean_error_unknown_binding_and_source_observer_files_never_write() {
    let dir = project();
    for (cop, source, status) in [
        (
            "Style/Proc",
            "def proc(&b); lambda(&b); end\nf = Proc.new { |x| x }\n",
            1,
        ),
        ("Style/Proc", "f = Proc.new { 1 }; p f.source_location\n", 1),
        ("Style/LambdaCall", "x.(); p caller_locations\n", 1),
        (
            "Style/StabbyLambdaParentheses",
            "f = ->a { a }; p f.source_location\n",
            1,
        ),
        (ONLY, "f = ->(x) { x }; f.call(2)\n", 0),
        (
            ONLY,
            "'Proc.new {}'; 'x.()'; '->a {}'\n__END__\nProc.new {}\nx.()\n->a {}\n",
            0,
        ),
        (ONLY, "Proc.new {\n", 2),
    ] {
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "x.rb"],
            status,
        );
    }
}
#[test]
fn byte_fidelity_for_bom_crlf_legacy_literals_and_data_payloads() {
    let dir = project();
    for (cop, before, after) in [
        (
            "Style/Proc",
            b"\xef\xbb\xbff = Proc.new { |x| [x, __LINE__] }\r\n__END__\r\nraw\xff\r\n".as_slice(),
            b"\xef\xbb\xbff = proc { |x| [x, __LINE__] }\r\n__END__\r\nraw\xff\r\n".as_slice(),
        ),
        (
            "Style/Proc",
            b"# encoding: ISO-8859-1\r\nf = Proc.new { 'caf\xe9' }\r\n".as_slice(),
            b"# encoding: ISO-8859-1\r\nf = proc { 'caf\xe9' }\r\n".as_slice(),
        ),
        (
            "Style/StabbyLambdaParentheses",
            b"\xef\xbb\xbff = ->a,\r\n b { [a,b,__LINE__] }\r\n__END__\r\nraw\xff\r\n".as_slice(),
            b"\xef\xbb\xbff = ->(a,\r\n b) { [a,b,__LINE__] }\r\n__END__\r\nraw\xff\r\n".as_slice(),
        ),
        (
            "Style/StabbyLambdaParentheses",
            b"# encoding: ISO-8859-1\r\nf = ->a='caf\xe9' { a }\r\n".as_slice(),
            b"# encoding: ISO-8859-1\r\nf = ->a='caf\xe9' { a }\r\n".as_slice(),
        ),
        (
            "Style/LambdaCall",
            b"\xef\xbb\xbfx.(\r\n __LINE__\r\n)\r\n__END__\r\nraw\xff\r\n".as_slice(),
            b"\xef\xbb\xbfx.call(\r\n __LINE__\r\n)\r\n__END__\r\nraw\xff\r\n".as_slice(),
        ),
        (
            "Style/LambdaCall",
            b"# encoding: ISO-8859-1\r\nx.('caf\xe9')\r\n".as_slice(),
            b"# encoding: ISO-8859-1\r\nx.('caf\xe9')\r\n".as_slice(),
        ),
    ] {
        fs::write(dir.path().join("x.rb"), before).unwrap();
        fix(dir.path(), cop);
        assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), after, "{cop}");
    }
}
fn ruby(dir: &Path) -> Output {
    Command::new("ruby")
        .current_dir(dir)
        .arg("x.rb")
        .output()
        .unwrap()
}
fn has_ruby() -> bool {
    Command::new("ruby").arg("--version").output().is_ok()
}
fn observed_fix(cop: &str, source: &str, expected: &str) {
    let dir = project();
    let path = dir.path().join("x.rb");
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path());
    assert!(before.status.success(), "{source}: {before:?}");
    assert_eq!(
        String::from_utf8_lossy(&before.stdout),
        expected,
        "{source}"
    );
    fix(dir.path(), cop);
    assert_ne!(
        fs::read(&path).unwrap(),
        source.as_bytes(),
        "must really fix {cop}: {source}"
    );
    let after = ruby(dir.path());
    assert_eq!(
        (after.status.code(), after.stdout, after.stderr),
        (before.status.code(), before.stdout, before.stderr),
        "{source}"
    );
    let stable = fs::read(&path).unwrap();
    fix(dir.path(), cop);
    assert_eq!(fs::read(path).unwrap(), stable);
}
#[test]
fn runtime_proc_keeps_lenient_arity_identity_return_break_next_and_captures() {
    if !has_ruby() {
        return;
    }
    for (source, expected) in [
        (
            "f = Proc.new { |x,y| [x,y] }; p [f.lambda?, f.arity, f.call(1), f.equal?(f)]\n",
            "[false, 2, [1, nil], true]\n",
        ),
        ("f = Proc.new { |x| next x*2 }; p f.call(3)\n", "6\n"),
        (
            "f = Proc.new { break 7 }; begin; f.call; rescue LocalJumpError => e; p [e.reason, e.exit_value]; end\n",
            "[:break, 7]\n",
        ),
        ("f = Proc.new { return 7 }; p f.call\n", ""),
        (
            "x=10; f = Proc.new { |y; z| z=y; [x, -> { z }.call] }; p f.call(2)\n",
            "[10, 2]\n",
        ),
        (
            "events=[]; f = Proc.new { |x=(events << :default; 2), key: 3, **kw, &blk| [x,key,kw,blk.call,events.dup] }; p events; p f.call(other:4) { 5 }\n",
            "[]\n[2, 3, {:other=>4}, 5, [:default]]\n",
        ),
        (
            "a = Proc.new { 1 }; b = Proc.new { 1 }; p [a.equal?(b), a.class == Proc, b.class == Proc]\n",
            "[false, true, true]\n",
        ),
        (
            "f = Proc.new { _1 + 2 }; p [f.lambda?, f.arity, f.call(3)]\n",
            "[false, 1, 5]\n",
        ),
        (
            "f = Proc.new do |x = __LINE__|\n [x, __LINE__, <<~TEXT]\n literal } | # value\nTEXT\nend; p f.call; p __LINE__\n",
            "[1, 2, \"literal } | # value\\n\"]\n5\n",
        ),
    ] {
        observed_fix("Style/Proc", source, expected);
    }
}
#[test]
fn runtime_sugar_uses_call_not_overridden_brackets_and_evaluates_once_in_order() {
    if !has_ruby() {
        return;
    }
    observed_fix(
        "Style/LambdaCall",
        "events=[]; obj=Object.new; obj.define_singleton_method(:call) { |*xs, **kw, &blk| events << :call; [xs,kw,blk.call] }; obj.define_singleton_method(:[]) { :wrong }; make=-> { events << :receiver; obj }; arg=-> { events << :arg; 2 }; p make.call.(arg.call, key:3) { events << :block; 4 }; p events; p obj[]\n",
        "[[2], {:key=>3}, 4]\n[:receiver, :arg, :call, :block]\n:wrong\n",
    );
    observed_fix(
        "Style/LambdaCall",
        "events=[]; obj=nil; p obj&.(events << :arg); p events\n",
        "nil\n[]\n",
    );
    observed_fix(
        "Style/LambdaCall",
        "f=->(x) { return x+1 }; p f.(2); begin; f.(); rescue ArgumentError; p :arity; end\n",
        "3\n:arity\n",
    );
    observed_fix(
        "Style/LambdaCall",
        "f=proc { |x,y| next [x,y] }; p f.(1)\n",
        "[1, nil]\n",
    );
    observed_fix("Style/LambdaCall", "f=-> { break 7 }; p f.()\n", "7\n");
    observed_fix(
        "Style/LambdaCall",
        "p (->(*xs) { xs }).(\n __LINE__,\n 'a,b)', /[)]/.source\n); p __LINE__\n",
        "[2, \"a,b)\", \"[)]\"]\n4\n",
    );
}
#[test]
fn empty_sugar_delimiters_keep_operator_index_and_keyword_binding_at_runtime() {
    if !has_ruby() {
        return;
    }
    for (source, expected) in [
        ("f = -> { 3 }; p(f.() +2)\n", "5\n"),
        ("f = -> { [3] }; p(f.() [0])\n", "3\n"),
        (
            "f = -> { true }; result = f.()and true; p result\n",
            "true\n",
        ),
    ] {
        observed_fix("Style/LambdaCall", source, expected);
        let dir = project();
        fs::write(dir.path().join("x.rb"), source.replace("f.()", "f.call")).unwrap();
        let naive = ruby(dir.path());
        assert!(
            !naive.status.success(),
            "missing unsafe-omission witness: {source}: {naive:?}"
        );
    }
    observed_fix("Style/Proc", "f = Proc.new()do 7 end; p f.call\n", "7\n");
}

#[test]
fn runtime_wrapping_preserves_strict_arity_defaults_destructuring_and_block_binding() {
    if !has_ruby() {
        return;
    }
    for (source, expected) in [
        (
            "f = ->x,y { return [x,y] }; p [f.lambda?, f.arity, f.call(1,2)]; begin; f.call(1); rescue ArgumentError; p :arity; end\n",
            "[true, 2, [1, 2]]\n:arity\n",
        ),
        (
            "x=10; f = ->y { next [x, -> { y }.call] }; p f.call(2)\n",
            "[10, 2]\n",
        ),
        (
            "events=[]; f = ->x=(events << :default; 2), key: 3, **kw, &blk do\n break [x,key,kw,blk.call,events.dup]\nend; p events; p f.call(other:4) { 5 }\n",
            "[]\n[2, 3, {:other=>4}, 5, [:default]]\n",
        ),
        (
            "f = ->x, (a,b), *xs { [x,a,b,xs] }; p f.call(0,[1,2],3,4)\n",
            "[0, 1, 2, [3, 4]]\n",
        ),
        (
            "xs=[]; xs.push ->x do\n x+1\nend; p xs.first.call(2)\n",
            "3\n",
        ),
        (
            "f = ->x = <<~TEXT do\n literal ) { #\nTEXT\n [x, __LINE__]\nend; p f.call; p __LINE__\n",
            "[\"literal ) { #\\n\", 4]\n5\n",
        ),
    ] {
        observed_fix("Style/StabbyLambdaParentheses", source, expected);
    }
}
#[test]
fn overridden_kernel_proc_and_shadowed_proc_constant_have_real_naive_edit_witnesses() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    for (source, expected) in [
        (
            "def proc(&b); :custom; end\nf = Proc.new { |x,y| [x,y] }; p [f.lambda?, f.call(1)]\n",
            "[false, [1, nil]]\n",
        ),
        (
            "class Custom; def self.new(&b); :custom; end; end; Proc = Custom\nf = Proc.new { 1 }; p f\n",
            ":custom\n",
        ),
    ] {
        fs::write(dir.path().join("x.rb"), source).unwrap();
        let before = ruby(dir.path());
        assert!(before.status.success());
        assert_eq!(String::from_utf8_lossy(&before.stdout), expected);
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", "Style/Proc", "--fix", "x.rb"],
            1,
        );
        fs::write(
            dir.path().join("x.rb"),
            source.replace("f = Proc.new", "f = proc"),
        )
        .unwrap();
        let naive = ruby(dir.path());
        assert_ne!(
            (naive.status.code(), naive.stdout),
            (before.status.code(), before.stdout)
        );
    }
}
#[test]
fn data_initial_byte_positions_are_guarded_for_all_three_with_naive_witnesses() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    for (cop, before, after) in CASES {
        let (before, after) = if cop == "Style/LambdaCall" {
            ("f = -> { 1 }; f.()\n", "f = -> { 1 }; f.call\n")
        } else {
            (before, after)
        };
        let source = format!("{before}p DATA.pos\n__END__\nraw\n");
        fs::write(dir.path().join("x.rb"), &source).unwrap();
        let observed = ruby(dir.path());
        assert!(observed.status.success());
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "x.rb"],
            1,
        );
        fs::write(dir.path().join("x.rb"), source.replace(before, after)).unwrap();
        let naive = ruby(dir.path());
        assert!(naive.status.success());
        assert_ne!(naive.stdout, observed.stdout, "{cop}");
    }
}
#[test]
fn prism34_it_numbered_explicit_and_nested_scopes_are_not_interchanged() {
    let dir = project();
    for (cop, source, shape) in [
        (
            "Style/Proc",
            "f=Proc.new { it }; g=Proc.new { _1 }; h=Proc.new { || 1 }\n",
            "ItParametersNode",
        ),
        (
            "Style/StabbyLambdaParentheses",
            "f=->x { -> { it + x } }; g=-> { _1 }; h=-> { it }\n",
            "ItParametersNode",
        ),
        (
            "Style/LambdaCall",
            "f=-> { it }; p f.(3); g=-> { _1 }; p g.(4)\n",
            "NumberedParametersNode",
        ),
    ] {
        let before = ruby_prism::parse(source.as_bytes());
        assert_eq!(before.errors().count(), 0);
        let dump = format!("{:?}", before.node());
        assert!(dump.contains(shape));
        fs::write(dir.path().join("x.rb"), source).unwrap();
        fix(dir.path(), cop);
        let changed = fs::read(dir.path().join("x.rb")).unwrap();
        assert_ne!(changed, source.as_bytes());
        let after = ruby_prism::parse(&changed);
        assert_eq!(after.errors().count(), 0);
        let after_dump = format!("{:?}", after.node());
        for name in [
            "ItParametersNode",
            "NumberedParametersNode",
            "BlockParametersNode",
        ] {
            assert_eq!(
                dump.matches(name).count(),
                after_dump.matches(name).count(),
                "{cop}: {name}"
            );
        }
    }
}
#[test]
fn explicit_ruby34_prism_parameter_scope_evidence_without_executing_it_on_ruby32() {
    let script = r#"
require 'prism'
parsed = Prism.parse(File.binread(ARGV[0]), version: '3.4')
abort parsed.errors.inspect unless parsed.success?
counts = Hash.new(0)
walk = ->(node) do
  name = node.class.name.split('::').last
  counts[name] += 1 if %w[ItParametersNode NumberedParametersNode BlockParametersNode].include?(name)
  node.compact_child_nodes.each { |child| walk.call(child) }
end
walk.call(parsed.value)
p counts.sort
"#;
    if !Command::new("ruby")
        .args(["-rprism", "-e", "exit"])
        .output()
        .is_ok_and(|out| out.status.success())
    {
        return;
    }
    let dir = project();
    let path = dir.path().join("x.rb");
    for (cop, source) in [
        (
            "Style/Proc",
            "f = Proc.new { it }; g = Proc.new { _1 }; h = Proc.new { |x| -> { it + x } }\n",
        ),
        (
            "Style/StabbyLambdaParentheses",
            "f = ->x { -> { it + x } }; g = -> { _1 }; h = -> { it }\n",
        ),
        (
            "Style/LambdaCall",
            "f = -> { it }; g = -> { _1 }; p f.(3); p g.(4)\n",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let run = || {
            Command::new("ruby")
                .args(["-e", script])
                .arg(&path)
                .output()
                .unwrap()
        };
        let before = run();
        assert!(before.status.success(), "{before:?}");
        fix(dir.path(), cop);
        assert_ne!(fs::read(&path).unwrap(), source.as_bytes());
        let after = run();
        assert!(after.status.success(), "{after:?}");
        assert_eq!(after.stdout, before.stdout, "{cop}");
    }
}

#[test]
fn forwarding_without_unknown_call_observers_really_composes() {
    let dir = project();
    let source = "f = Proc.new { || 2 }; g = ->x { x+1 }\ndef delegate(*args)\n target(*args)\nend\np([f[],g[3]])\n";
    let path = dir.path().join("x.rb");
    fs::write(&path, source).unwrap();
    let before = has_ruby().then(|| ruby(dir.path()));
    let only = "Style/Proc,Style/LambdaCall,Style/StabbyLambdaParentheses,Style/EmptyBlockParameter,Style/ArgumentsForwarding,Layout";
    for _ in 0..5 {
        fix(dir.path(), only);
    }
    let stable = fs::read(&path).unwrap();
    let text = String::from_utf8_lossy(&stable);
    assert!(text.contains("f = proc { 2 }"), "{text}");
    assert!(text.contains("g = ->(x)"), "{text}");
    assert!(text.contains("def delegate(*)"), "{text}");
    if let Some(before) = before {
        let after = ruby(dir.path());
        assert!(before.status.success());
        assert_eq!(
            (after.status.code(), after.stdout, after.stderr),
            (before.status.code(), before.stdout, before.stderr)
        );
    }
    fix(dir.path(), only);
    assert_eq!(fs::read(path).unwrap(), stable);
}

#[test]
fn actual_default_and_selected_peer_composition_stabilizes_with_runtime_equivalence() {
    let dir = project();
    let source = "# Copyright (c) 2026 Example\n# frozen_string_literal: true\nf = Proc.new { || 2 }; g = ->x do\n  [x].map { |v|\n    v + 1\n  }\nend\ndef delegate(*args)\n  target(*args)\nend\np([f.(), g.(3)])\n";
    for selected in [
        None,
        Some(
            "Style/Lambda,Style/LambdaCall,Style/Proc,Style/StabbyLambdaParentheses,Style/EmptyBlockParameter,Style/EmptyLambdaParameter,Style/BlockDelimiters,Style/ArgumentsForwarding,Layout",
        ),
    ] {
        let path = dir.path().join("x.rb");
        fs::write(&path, source).unwrap();
        let before = has_ruby().then(|| ruby(dir.path()));
        let args = selected.map_or_else(
            || vec!["--fix", "x.rb"],
            |only| vec!["--only", only, "--fix", "x.rb"],
        );
        for _ in 0..5 {
            let out = cli(dir.path(), &args);
            assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
        }
        let stable = fs::read(&path).unwrap();
        let text = String::from_utf8_lossy(&stable);
        assert!(text.contains("f = proc { 2 }"), "{text}");
        assert!(text.contains("g.call(3)"), "{text}");
        assert!(text.contains("map do"), "{text}");
        // Its existing source-binding proof treats unknown `call` dispatch as
        // an observer, so forwarding stays reported but unedited in this case.
        assert!(text.contains("def delegate(*args)"), "{text}");
        if let Some(before) = before {
            let after = ruby(dir.path());
            assert!(before.status.success(), "{before:?}");
            assert_eq!(
                (after.status.code(), after.stdout, after.stderr),
                (before.status.code(), before.stdout, before.stderr)
            );
        }
        cli(dir.path(), &args);
        assert_eq!(fs::read(path).unwrap(), stable);
    }
}
