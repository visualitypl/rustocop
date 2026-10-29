//! real CLI policy/bytes and controlled synthetic Ruby observations.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const EVAL: &str = "Style/EvalWithLocation";
const SEND: &str = "Style/Send";
const LITERAL: &str = "Style/SendWithLiteralMethodName";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn run(dir: &Path, args: &[&str], status: i32, counts: [usize; 3]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    for (cop, count) in [EVAL, SEND, LITERAL].into_iter().zip(counts) {
        assert_eq!(
            text.lines()
                .filter(|line| line.contains(&format!("({cop})")))
                .count(),
            count,
            "{args:?}: {text}"
        );
    }
    out
}
#[test]
fn default_opt_in_selectors_config_and_no_write_contract() {
    let dir = project();
    let path = dir.path().join("a.rb");
    let source = "eval(\"42\", binding)\nobj.send(:secret)\nobj.public_send(:foo)\n";
    fs::write(&path, source).unwrap();
    for (config, args, counts) in [
        (String::new(), vec!["a.rb"], [1, 0, 1]),
        (String::new(), vec!["--only", EVAL, "a.rb"], [1, 0, 0]),
        (String::new(), vec!["--only", SEND, "a.rb"], [0, 1, 0]),
        (String::new(), vec!["--only", LITERAL, "a.rb"], [0, 0, 1]),
        (
            format!("[cops.\"{SEND}\"]\nenabled = true\n"),
            vec!["a.rb"],
            [1, 1, 1],
        ),
        (
            format!("[cops.Style]\nenabled = false\n[cops.\"{SEND}\"]\nenabled = true\n"),
            vec!["a.rb"],
            [0, 1, 0],
        ),
        (
            "[cops.Style]\nenabled = false\n".into(),
            vec!["--only", LITERAL, "a.rb"],
            [0, 0, 1],
        ),
        (
            format!("[cops.\"{EVAL}\"]\nenabled = false\n[cops.\"{LITERAL}\"]\nenabled = false\n"),
            vec!["a.rb"],
            [0, 0, 0],
        ),
        (
            String::new(),
            vec!["--only", EVAL, "--except", EVAL, "--fix", "a.rb"],
            [0, 0, 0],
        ),
        (
            String::new(),
            vec!["--only", SEND, "--except", SEND, "--fix", "a.rb"],
            [0, 0, 0],
        ),
        (
            String::new(),
            vec!["--only", LITERAL, "--except", LITERAL, "--fix", "a.rb"],
            [0, 0, 0],
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(dir.path(), &args, i32::from(counts != [0, 0, 0]), counts);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn both_directives_and_config_relative_excludes_block_edits() {
    let dir = project();
    fs::create_dir(dir.path().join("lib")).unwrap();
    let path = dir.path().join("lib/a.rb");
    for (cop, code) in [
        (EVAL, "eval('42', binding)"),
        (SEND, "obj.send(:secret)"),
        (LITERAL, "obj.public_send(:foo)"),
    ] {
        for prefix in ["rustocop", "rubocop"] {
            for source in [
                format!("# {prefix}:disable {cop}\n{code}\n"),
                format!("{code} # {prefix}:disable {cop}\n"),
            ] {
                fs::write(&path, &source).unwrap();
                run(
                    dir.path(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    0,
                    [0, 0, 0],
                );
                assert_eq!(fs::read_to_string(&path).unwrap(), source);
            }
        }
        for config in [
            format!("[cops.\"{cop}\"]\nexclude = ['lib/**']\n"),
            "exclude = ['lib/**']\n".into(),
        ] {
            fs::write(&path, code).unwrap();
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            run(
                &dir.path().join("lib"),
                &["--only", cop, "--fix", "a.rb"],
                0,
                [0, 0, 0],
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), code);
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
    for prefix in ["rustocop", "rubocop"] {
        let source = format!("# {prefix}:enable {SEND}\nobj.send(:secret)\n");
        fs::write(&path, &source).unwrap();
        run(dir.path(), &["--fix", "lib/a.rb"], 1, [0, 1, 0]);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        // An interior disabled argument prevents a separately anchored edit.
        let source = format!("eval('42', binding, 'wrong',\n 99 # {prefix}:disable {EVAL}\n)\n");
        fs::write(&path, &source).unwrap();
        run(
            dir.path(),
            &["--only", EVAL, "--fix", "lib/a.rb"],
            0,
            [0, 0, 0],
        );
        let fixed = fs::read_to_string(&path).unwrap();
        assert!(fixed.contains("__FILE__,\n 99"), "{fixed}");
    }
}
#[test]
fn canonical_fixes_preserve_bom_headers_crlf_data_and_mixed_safe_fixes_stabilize() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for (source, expected, selector) in [
        (b"\xef\xbb\xbf# encoding: UTF-8\r\neval('42', binding)\r\n__END__\r\nraw\xff\r\n".as_slice(), b"\xef\xbb\xbf# encoding: UTF-8\r\neval('42', binding, __FILE__, __LINE__)\r\n__END__\r\nraw\xff\r\n".as_slice(), EVAL),
        (b"# encoding: ISO-8859-1\r\neval('42', binding) # caf\xe9\r\n".as_slice(), b"# encoding: ISO-8859-1\r\neval('42', binding, __FILE__, __LINE__) # caf\xe9\r\n".as_slice(), EVAL),
        (b"Object.class_eval(<<~CODE)\r\n  42\r\nCODE\r\n".as_slice(), b"Object.class_eval(<<~CODE, __FILE__, __LINE__ + 1)\r\n  42\r\nCODE\r\n".as_slice(), EVAL),
        (b"eval('42',binding)\n".as_slice(), b"eval(\"42\", binding, __FILE__, __LINE__)\n".as_slice(), "Style/EvalWithLocation,Style/StringLiterals,Layout/SpaceAfterComma"),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(dir.path(), &["--only", selector, "a.rb"], 1, [1, 0, 0]);
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        run(dir.path(), &["--only", selector, "--fix", "a.rb"], 0, [0, 0, 0]);
        assert_eq!(fs::read(&path).unwrap(), expected);
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(dir.path(), &["--only", selector, "--fix", "a.rb"], 0, [0, 0, 0]);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn clean_error_unsafe_uncertain_non_utf8_files_are_never_written() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for (cop, source, status, counts) in [
        (
            EVAL,
            b"eval('42', binding, __FILE__, __LINE__)\n".as_slice(),
            0,
            [0, 0, 0],
        ),
        (EVAL, b"eval(\n".as_slice(), 2, [0, 0, 0]),
        (
            EVAL,
            b"# encoding: ISO-8859-1\r\neval('caf\xe9', binding)\r\n__END__\r\n\xff".as_slice(),
            1,
            [1, 0, 0],
        ),
        (
            EVAL,
            b"eval('42', binding, \"nul\\0\")\n".as_slice(),
            1,
            [2, 0, 0],
        ),
        (
            EVAL,
            b"Kernel = Custom; Kernel.eval('42', binding)\n".as_slice(),
            1,
            [1, 0, 0],
        ),
        (
            SEND,
            b"obj.send(:private_method)\n".as_slice(),
            1,
            [0, 1, 0],
        ),
        (LITERAL, b"obj.public_send(:foo)\n".as_slice(), 1, [0, 0, 1]),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(
            dir.path(),
            &["--only", cop, "--fix", "a.rb"],
            status,
            counts,
        );
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn stdin_selection_excludes_directives_and_strict_fixed_options_never_write() {
    let dir = project();
    let path = dir.path().join("a.rb");
    let original = b"sentinel bytes\xff\n";
    fs::write(&path, original).unwrap();
    for (cop, source) in [
        (EVAL, "eval('42', binding)"),
        (SEND, "obj.send(:secret)"),
        (LITERAL, "obj.public_send(:foo)"),
    ] {
        for (config, source, expected) in [
            (String::new(), source.to_owned(), 1),
            (
                format!("[cops.\"{cop}\"]\nexclude = ['a.rb']\n"),
                source.to_owned(),
                0,
            ),
            (
                String::new(),
                format!("# rubocop:disable {cop}\n{source}"),
                0,
            ),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", cop, "--stdin", "a.rb"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(source.as_bytes())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert_eq!(output.status.code(), Some(expected), "{output:?}");
            assert_eq!(fs::read(&path).unwrap(), original);
        }
        for option in [
            "allow_send = false",
            "target_ruby_version = '3.4'",
            "max = 99",
        ] {
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{option}\n"),
            )
            .unwrap();
            run(dir.path(), &["--only", cop, "--fix", "a.rb"], 2, [0, 0, 0]);
            assert_eq!(fs::read(&path).unwrap(), original);
        }
    }
}
fn has_ruby() -> bool {
    Command::new("ruby").arg("--version").output().is_ok()
}
fn observe(dir: &Path) -> Output {
    // Only our fixed synthetic fixtures are evaluated, never arbitrary input.
    Command::new("ruby").current_dir(dir).args(["-e", "/(a)/ =~ 'a'; original = $~; value = eval(File.binread('a.rb'), TOPLEVEL_BINDING, 'a.rb', 1); p [value, $1, $~.equal?(original)]"]).output().unwrap()
}
fn same(before: &Output, after: &Output) {
    assert!(before.status.success(), "{before:?}");
    assert_eq!(before.status.code(), after.status.code(), "{after:?}");
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
}
#[test]
fn inert_corrections_preserve_values_binding_context_and_regexp_state_at_runtime() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    for source in [
        "eval('42', binding)",
        "Kernel.eval('true', binding, 'old.rb', 99)",
        "::Kernel.eval(\"\\x34\\x32\", binding, __FILE__, __LINE__ + 1)",
        "Object.class_eval <<~CODE\n42\nCODE\n",
        "Module.module_eval('nil')",
        "instance_eval('false')",
        "eval('42',\n binding,\n __FILE__,\n __LINE__)",
    ] {
        fs::write(dir.path().join("a.rb"), source).unwrap();
        let before = observe(dir.path());
        assert!(
            String::from_utf8_lossy(&before.stdout).contains(", \"a\", true]"),
            "{before:?}"
        );
        run(dir.path(), &["--only", EVAL, "--fix", "a.rb"], 0, [0, 0, 0]);
        assert_ne!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), source);
        same(&before, &observe(dir.path()));
    }
}
#[test]
fn decoded_carriage_return_warnings_preserve_stderr_and_source_bytes() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let path = dir.path().join("a.rb");
    for (source, count) in [
        ("eval(\"42\\r\", binding)", 1),
        ("Kernel.eval(\"\\r42\", binding, 'old.rb', 99)", 2),
        ("Object.class_eval(\"42\\x0d\")", 1),
        ("Module.module_eval(\"42\\r\")", 1),
        ("instance_eval(\"false\\r\")", 1),
    ] {
        fs::write(&path, source).unwrap();
        let before = observe(dir.path());
        assert!(
            String::from_utf8_lossy(&before.stderr).contains("warning:"),
            "{source}: {before:?}"
        );
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(
            dir.path(),
            &["--only", EVAL, "--fix", "a.rb"],
            1,
            [count, 0, 0],
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        same(&before, &observe(dir.path()));
    }
}
#[test]
fn uncertain_source_locations_evaluation_order_visibility_and_overrides_keep_runtime_behavior() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    for (cop, source, counts) in [
        (
            EVAL,
            "eval('[__FILE__, __LINE__, caller_locations.first.path]', binding)\n",
            [1, 0, 0],
        ),
        (
            EVAL,
            "eval('proc {}.source_location', binding, 'explicit.rb', 99)\n",
            [2, 0, 0],
        ),
        (
            EVAL,
            "eval('42', binding, <<~NAME, 99)\nside_effect()\nNAME\n",
            [2, 0, 0],
        ),
        (
            EVAL,
            "def outer; eval('return 42', binding); :unreachable; end; outer\n",
            [1, 0, 0],
        ),
        (
            EVAL,
            "events = []; eval('events << :code; 42', binding, (events << :file; 'x'), (events << :line; 1)); events\n",
            [2, 0, 0],
        ),
        (EVAL, "eval(\"/(b)/ =~ 'b'; $1\", binding)\n", [1, 0, 0]),
        (
            EVAL,
            "events = []; def record(events); events << :file; 'explicit.rb'; end; eval('42', binding, record(events)); [events, __LINE__]\n",
            [2, 0, 0],
        ),
        (
            EVAL,
            "def eval(*args); args.length; end; eval('42', binding)\n",
            [1, 0, 0],
        ),
        (
            EVAL,
            "module M; Kernel = Object.new; def Kernel.eval(*args); args.length; end; VALUE = Kernel.eval('42', binding); end; M::VALUE\n",
            [1, 0, 0],
        ),
        (
            EVAL,
            "object = Object.new; def object.instance_eval(*args); args; end; object.instance_eval('42')\n",
            [1, 0, 0],
        ),
        (
            EVAL,
            "eval('42', binding, 'explicit.rb', (events = []; events << :line; 1)); events\n",
            [2, 0, 0],
        ),
        (
            SEND,
            "class X; private; def secret; 42; end; end; X.new.send(:secret)\n",
            [0, 1, 0],
        ),
        (
            LITERAL,
            "class X; def public_send(*args); [:custom, args]; end; end; X.new.public_send(:foo, 42)\n",
            [0, 0, 1],
        ),
        (
            LITERAL,
            "class X; def value=(value); 42; end; end; X.new.public_send(:value=, 1)\n",
            [0, 0, 0],
        ),
        (
            LITERAL,
            "class X; def foo(x); yield x; end; end; events = []; result = X.new.public_send(:foo, (events << :arg; 42)) { |x| events << :block; x }; [result, events]\n",
            [0, 0, 1],
        ),
        (
            LITERAL,
            "class X; private; def secret; 42; end; end; begin; X.new.public_send(:secret); rescue NoMethodError; :private; end\n",
            [0, 0, 1],
        ),
    ] {
        fs::write(dir.path().join("a.rb"), source).unwrap();
        let before = observe(dir.path());
        run(
            dir.path(),
            &["--only", cop, "--fix", "a.rb"],
            i32::from(counts != [0, 0, 0]),
            counts,
        );
        assert_eq!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), source);
        same(&before, &observe(dir.path()));
    }
}
