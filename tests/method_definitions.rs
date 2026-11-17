//! controlled fixtures only; no corpus code is ever executed.
use rustocop::cops::Registry;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const SINGLE: &str = "Style/SingleLineMethods";
const TOP: &str = "Style/TopLevelMethodDefinition";
const MULTI: &str = "Style/MultilineMethodSignature";
const CASES: [(&str, &str, &str); 3] = [
    (SINGLE, "def f; 42; end\n", "def f() = 42\n"),
    (TOP, "def f; end\n", "def f; end\n"),
    (
        MULTI,
        "def f(a,\n b)\n [a,b]\nend\n",
        "def f(a, b)\n [a,b]\nend\n",
    ),
];
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn cli(dir: &Path, args: &[&str], status: Option<i32>) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    if let Some(status) = status {
        assert_eq!(output.status.code(), Some(status), "{args:?}: {output:?}");
    } else {
        assert!(matches!(output.status.code(), Some(0 | 1)), "{output:?}");
    }
    output
}
fn fix(source: &str, only: &str, config: &str) -> String {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    fs::write(dir.path().join("rustocop.toml"), config).unwrap();
    cli(dir.path(), &["--only", only, "--fix", "lib/a.rb"], None);
    fs::read_to_string(path).unwrap()
}
#[test]
fn defaults_opt_ins_selectors_config_and_no_write_are_real() {
    let registry = Registry::new();
    for (name, before, after) in CASES {
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert_eq!(cop.default_enabled(), name == SINGLE);
        assert_eq!(cop.safe_fixes(), name != TOP);
        let dir = project();
        let path = dir.path().join("lib/a.rb");
        fs::write(&path, before).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let default = cli(dir.path(), &["lib/a.rb"], None);
        assert_eq!(
            String::from_utf8_lossy(&default.stdout).contains(name),
            name == SINGLE
        );
        cli(dir.path(), &["--only", name, "lib/a.rb"], Some(1));
        assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{name}\"]\nenabled = false\n"),
        )
        .unwrap();
        cli(
            dir.path(),
            &["--only", name, "--except", name, "--fix", "lib/a.rb"],
            Some(0),
        );
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        cli(
            dir.path(),
            &["--only", name, "--fix", "lib/a.rb"],
            Some(i32::from(name == TOP)),
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
        if name == TOP {
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        } else {
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            cli(dir.path(), &["--only", name, "--fix", "lib/a.rb"], Some(0));
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
        fs::write(&path, before).unwrap();
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{name}\"]\nenabled = true\n"),
        )
        .unwrap();
        let output = cli(dir.path(), &["lib/a.rb"], Some(1));
        assert!(String::from_utf8_lossy(&output.stdout).contains(name));
    }
}
#[test]
fn correction_dependencies_read_config_not_only_or_except() {
    let before = CASES[0].1;
    let multiline = "def f; \n  42; \nend\n";
    assert_eq!(
        fix(
            before,
            SINGLE,
            "[cops.\"Style/EndlessMethod\"]\nenabled = false\n"
        ),
        multiline
    );
    assert_eq!(
        fix(before, SINGLE, "[cops.Style]\nenabled = false\n"),
        multiline
    );
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, before).unwrap();
    cli(
        dir.path(),
        &[
            "--only",
            SINGLE,
            "--except",
            "Style/EndlessMethod",
            "--fix",
            "lib/a.rb",
        ],
        Some(0),
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), CASES[0].2);
    let before = CASES[2].1;
    for (config, count) in [("max = 5", 0), ("max = 25", 1), ("enabled = false", 1)] {
        fs::write(&path, before).unwrap();
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"Layout/LineLength\"]\n{config}\n"),
        )
        .unwrap();
        cli(dir.path(), &["--only", MULTI, "lib/a.rb"], Some(count));
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }
}
#[test]
fn directives_excludes_parse_errors_and_detection_only_never_write() {
    for (name, before, _) in CASES {
        let dir = project();
        let path = dir.path().join("lib/a.rb");
        for config in [
            "exclude = [\"lib/**\"]\n".to_string(),
            format!("[cops.\"{name}\"]\nexclude = [\"lib/**\"]\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            fs::write(&path, before).unwrap();
            cli(
                &dir.path().join("lib"),
                &["--only", name, "--fix", "a.rb"],
                Some(0),
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), before);
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rustocop", "rubocop"] {
            // Source enables also opt in the two default-off rules, without
            // relying on a force-enabling --only selector.
            let enabled = format!("# {prefix}:enable {name}\n{before}");
            fs::write(&path, &enabled).unwrap();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            let output = cli(dir.path(), &["lib/a.rb"], Some(1));
            assert!(String::from_utf8_lossy(&output.stdout).contains(name));
            assert_eq!(fs::read_to_string(&path).unwrap(), enabled);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            for target in [name, "Style", "all"] {
                for directive in ["disable", "disable-next"] {
                    let source = format!("# {prefix}:{directive} {target}\n{before}");
                    fs::write(&path, &source).unwrap();
                    let modified = fs::metadata(&path).unwrap().modified().unwrap();
                    cli(dir.path(), &["--only", name, "--fix", "lib/a.rb"], Some(0));
                    assert_eq!(fs::read_to_string(&path).unwrap(), source);
                    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
                }
            }
        }
        for (source, status) in [("def f(", 2), ("x = 1\n", 0)] {
            fs::write(&path, source).unwrap();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            cli(
                dir.path(),
                &["--only", name, "--fix", "lib/a.rb"],
                Some(status),
            );
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
        }
        let source =
            format!("# rustocop:disable {name}\n{before}# rustocop:enable {name}\n{before}");
        let result = fix(&source, name, "");
        let after = CASES.iter().find(|c| c.0 == name).unwrap().2;
        assert_eq!(
            result,
            format!("# rustocop:disable {name}\n{before}# rustocop:enable {name}\n{after}")
        );
    }
}
#[test]
fn encoding_bom_crlf_data_and_non_utf8_spans_preserve_bytes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (name, before, after) in [
        (
            SINGLE,
            b"\xef\xbb\xbfdef f; 1; 2; end\r\n__END__\r\n\xff\0raw\r\n".as_slice(),
            b"\xef\xbb\xbfdef f; \r\n  1; \r\n  2; \r\nend\r\n__END__\r\n\xff\0raw\r\n".as_slice(),
        ),
        (
            MULTI,
            b"\xef\xbb\xbfdef f(a,\r\n b)\r\nend\r\n__END__\r\n\xff\0raw\r\n",
            b"\xef\xbb\xbfdef f(a, b)\r\nend\r\n__END__\r\n\xff\0raw\r\n",
        ),
        (
            SINGLE,
            b"# coding: ISO-8859-1\r\n# caf\xe9\r\ndef f; 1; end\r\n",
            b"# coding: ISO-8859-1\r\n# caf\xe9\r\ndef f() = 1\r\n",
        ),
        (
            MULTI,
            b"# coding: ISO-8859-1\r\n# caf\xe9\r\ndef f(a,\r\n b)\r\nend\r\n",
            b"# coding: ISO-8859-1\r\n# caf\xe9\r\ndef f(a, b)\r\nend\r\n",
        ),
        (
            SINGLE,
            b"# coding: ISO-8859-1\ndef f; \"caf\xe9\"; end\n",
            b"# coding: ISO-8859-1\ndef f; \"caf\xe9\"; end\n",
        ),
        (
            MULTI,
            b"# coding: ISO-8859-1\ndef f(a = \"caf\xe9\",\n b = 1)\nend\n",
            b"# coding: ISO-8859-1\ndef f(a = \"caf\xe9\",\n b = 1)\nend\n",
        ),
        (
            TOP,
            b"\xef\xbb\xbfdef f; end\r\n__END__\r\n\xff\0raw\r\n",
            b"\xef\xbb\xbfdef f; end\r\n__END__\r\n\xff\0raw\r\n",
        ),
    ] {
        fs::write(&path, before).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(
            dir.path(),
            &["--only", name, "--fix", "lib/a.rb"],
            Some(i32::from(before == after)),
        );
        assert_eq!(fs::read(&path).unwrap(), after);
        if before == after {
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
    }
}
#[test]
fn signature_width_uses_declared_encoding_at_the_line_length_boundary() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let overhead = b"def f(a = \"\", b = 1)".len();
    for (encoding, character) in [
        ("ISO-8859-1", b"\xa3".as_slice()),
        ("ISO-8859-1", b"\xe9".as_slice()),
        ("UTF-8", b"\xc2\xa3".as_slice()),
        ("Shift_JIS", b"\xe0\x40".as_slice()),
    ] {
        for length in [119, 120, 121, overhead + 120] {
            let mut source = format!("# coding: {encoding}\ndef f(a = \"").into_bytes();
            source.extend(character.repeat(length - overhead));
            source.extend_from_slice(b"\",\n b = 1)\nend\n");
            fs::write(&path, &source).unwrap();
            let expected = i32::from(length <= 120);
            let output = cli(dir.path(), &["--only", MULTI, "lib/a.rb"], None);
            assert_eq!(
                output.status.code(),
                Some(expected),
                "{encoding}, collapsed length {length}: {output:?}"
            );
            if expected == 1 {
                let stdout = String::from_utf8(output.stdout).unwrap();
                assert!(stdout.contains("lib/a.rb:2:1:"), "{stdout}");
                assert!(
                    stdout.contains("Avoid multi-line method signatures."),
                    "{stdout}"
                );
            }
            assert_eq!(fs::read(&path).unwrap(), source);
        }
    }
}
#[test]
fn stdin_is_diagnostic_only_for_every_assigned_cop() {
    use std::{io::Write, process::Stdio};
    for (name, before, _) in CASES {
        let dir = project();
        let path = dir.path().join("lib/a.rb");
        fs::write(&path, before).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", name, "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(before.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stdout).contains(name));
        cli(
            dir.path(),
            &["--only", name, "--fix", "--stdin", "lib/a.rb"],
            Some(2),
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
fn ruby(path: &Path, source: &str) -> Vec<u8> {
    fs::write(path, source).unwrap();
    let output = Command::new("ruby").arg(path).output().unwrap();
    assert!(output.status.success(), "{output:?}: {source}");
    assert!(output.stderr.is_empty(), "{output:?}");
    output.stdout
}
fn has_ruby() -> bool {
    Command::new("ruby").arg("--version").output().is_ok()
}
#[test]
fn controlled_ruby_binding_evaluation_control_flow_and_literal_values_survive() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let path = dir.path().join("run.rb");
    for (name, source) in [
        (
            SINGLE,
            "$log = []\nclass C\n private def f(x = ($log << :default; 2)); $log << x; return x + 1; end\n public def run; f; end\n def value=(x); @value = x; end\n def value; @value; end\nend\nx=C.new\np [x.run,x.value=4,x.value,$log,C.private_instance_methods(false).include?(:f)]\n",
        ),
        (
            SINGLE,
            "def f(x); x => Integer; x * 2; end\ndef g(x); x in Integer; end\ndef h; a, b = [1,2]; a + b; end\np [f(3),g(3),g(nil),h]\n",
        ),
        (
            SINGLE,
            "def f(a=\" x   y \"); a; end\ndef g; %q(a;  b); end\ndef h; raise \"oops\"; end\nbegin\n h\nrescue => e\n p [f,g,e.class,e.message]\nend\n",
        ),
        (
            SINGLE,
            "def f; begin 1 / 0 rescue 42 end; end\ndef g; x = 0; begin x += 1 end while x < 3; x; end\np [f,g]\n",
        ),
        (
            SINGLE,
            "class Box\n attr_accessor :value\n def []=(k,v); @value = [k,v]; end\nend\n$obj = Box.new\ndef bar(x); x * 2; end\ndef baz; 3; end\ndef f; $obj.value = bar baz; end\ndef g; $obj[:k] = 1, 2; end\np [f,$obj.value,g,$obj.value]\n",
        ),
        (
            MULTI,
            "$log=[]\ndef f((b,c), a = ($log << :a; \" x   y \"),\n *rest, key: ($log << :key; %q(a;  b)), **kw, &block)\n [a,b,c,rest,key,kw,block.call]\nend\np [f(($log << :bc; [2,3])) { :block },$log]\n",
        ),
        (
            MULTI,
            "class C\n def self.f(a,\n b) = [a,b]\n private def g(a,\n b)\n  [a,b]\n end\n def run\n  g(1,2)\n end\nend\np [C.f(3,4),C.new.run,C.private_instance_methods(false).include?(:g)]\n",
        ),
    ] {
        let after = fix(source, name, "");
        assert_ne!(after, source, "fixture must actually exercise a correction");
        assert_eq!(
            ruby(&path, source),
            ruby(&path, &after),
            "{source} -> {after}"
        );
        assert_eq!(
            fix(&after, name, ""),
            after,
            "single-cop correction must stabilize"
        );
    }
}
#[test]
fn single_line_shebang_relocation_preserves_main_script_output_bytes_and_mtime() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let path = dir.path().join("run.rb");
    for (config, body) in [
        ("", "1; 2"),
        ("[cops.\"Style/EndlessMethod\"]\nenabled = false\n", "2"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        for newline in ["\n", "\r\n"] {
            let source = format!("def f; {body}; end #!/usr/bin/ruby -n{newline}puts f{newline}");
            fs::write(&path, &source).unwrap();
            // Main-script invocation with empty stdin: activating -n skips all code.
            let run = || {
                Command::new("ruby")
                    .arg(&path)
                    .stdin(std::process::Stdio::null())
                    .output()
                    .unwrap()
            };
            let before = run();
            assert!(before.status.success(), "{before:?}");
            assert!(before.stderr.is_empty(), "{before:?}");
            assert_eq!(before.stdout, b"2\n");
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            for args in [
                vec!["--only", SINGLE, "run.rb"],
                vec!["--only", SINGLE, "--fix", "run.rb"],
            ] {
                let output = cli(dir.path(), &args, Some(1));
                let stdout = String::from_utf8(output.stdout).unwrap();
                assert!(stdout.contains("run.rb:1:1:"), "{stdout}");
                assert!(
                    stdout.contains("Avoid single-line method definitions."),
                    "{stdout}"
                );
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            }
            let after = run();
            assert!(after.status.success(), "{after:?}");
            assert_eq!(before.stdout, after.stdout);
            assert_eq!(before.stderr, after.stderr);
            // Controlled naive witness proves this is a runtime safety guard.
            let naive =
                format!("#!/usr/bin/ruby -n{newline}def f; {body}; end{newline}puts f{newline}");
            fs::write(&path, naive).unwrap();
            let activated = run();
            assert!(activated.status.success(), "{activated:?}");
            assert!(activated.stdout.is_empty(), "{activated:?}");
            // Ruby may warn about a CRLF shebang in this deliberately unsafe witness.
        }
    }
}
#[test]
fn single_line_shebang_lookalikes_and_unmoved_comments_still_fix_at_runtime() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let path = dir.path().join("run.rb");
    for (config, source, expected) in [
        (
            "",
            "def f; 2; end #!/usr/bin/ruby -n\nputs f\n",
            "def f() = 2 #!/usr/bin/ruby -n\nputs f\n",
        ),
        (
            "",
            "def f; 1; \"#!\"; end # docs #!/usr/bin/ruby -n\nputs f\n",
            "# docs #!/usr/bin/ruby -n\ndef f; \n  1; \n  \"#!\"; \nend \nputs f\n",
        ),
        (
            "[cops.\"Style/EndlessMethod\"]\nenabled = false\n",
            "def f; \"#!\"; end # docs\nputs f\n",
            "# docs\ndef f; \n  \"#!\"; \nend \nputs f\n",
        ),
    ] {
        let before = ruby(&path, source);
        let after = fix(source, SINGLE, config);
        assert_eq!(after.as_bytes(), expected.as_bytes());
        assert_eq!(ruby(&path, &after), before);
        assert_eq!(fix(&after, SINGLE, config), after);
    }
}
#[test]
fn trailing_comments_on_visibility_wrappers_preserve_following_method_visibility() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let path = dir.path().join("run.rb");
    for visibility in ["private", "protected", "public"] {
        // `public def` must also preserve an existing private default scope.
        let default = if visibility == "public" {
            " private\n"
        } else {
            ""
        };
        for config in ["", "[cops.\"Style/EndlessMethod\"]\nenabled = false\n"] {
            let source = format!(
                "class C\n{default} {visibility} def f; 1; 2; end # docs\n def g\n  3\n end\nend\np [C.public_instance_methods(false).sort,C.protected_instance_methods(false).sort,C.private_instance_methods(false).sort]\n"
            );
            let before = ruby(&path, &source);
            let after = fix(&source, SINGLE, config);
            assert_ne!(after, source, "fixture must exercise a correction");
            assert!(
                after.contains(&format!(" # docs\n {visibility} def f;")),
                "{after}"
            );
            assert_eq!(before, ruby(&path, &after), "{source} -> {after}");
            assert_eq!(fix(&after, SINGLE, config), after);
        }
    }
}
#[test]
fn style_layout_composition_is_order_independent_and_stabilizes_without_hidden_passes() {
    let names = [
        SINGLE,
        MULTI,
        "Style/EndlessMethod",
        "Style/MethodDefParentheses",
        "Style/StringLiterals",
        "Style/HashSyntax",
        "Style/RedundantReturn",
        "Style/Semicolon",
        "Style/SignalException",
        "Layout/TrailingWhitespace",
    ];
    let before = "def f a; return 'x', {:key => a}; end  \ndef g(a,\n b)\n fail 'oops'\nend\n";
    let first = fix(before, &names.join(","), "");
    assert_ne!(first, before);
    let mut reversed = names;
    reversed.reverse();
    assert_eq!(first, fix(before, &reversed.join(","), ""));
    let mut current = first;
    let mut stable = false;
    for _ in 0..5 {
        assert_eq!(ruby_prism::parse(current.as_bytes()).errors().count(), 0);
        let next = fix(&current, &names.join(","), "");
        if next == current {
            stable = true;
            break;
        }
        current = next;
    }
    assert!(
        stable,
        "curated fixture must stabilize within six explicit passes"
    );
    assert!(current.contains("def f(a)"));
    assert!(current.contains("def g(a, b)"));
    assert!(current.contains("raise \"oops\""));
    assert!(!current.contains("return"));
    // The real department selectors keep their normal opt-in behavior and
    // coexist with the now-default SingleLineMethods, without cycling.
    let mut stable = false;
    for _ in 0..4 {
        let next = fix(&current, "Style,Layout", "");
        assert_eq!(ruby_prism::parse(next.as_bytes()).errors().count(), 0);
        if current == next {
            stable = true;
            break;
        }
        current = next;
    }
    assert!(
        stable,
        "department fixture must stabilize within four explicit passes"
    );
    assert_eq!(
        current,
        "def f(a) = [\"x\", { key: a }]\n\ndef g(a, b)\n  raise(\"oops\")\nend\n"
    );
}
