//! real selectors, opt-in policy, no-write and controlled Ruby semantics.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const MEMBER: &str = "Style/ModuleMemberExistenceCheck";
const VISIBILITY: &str = "Style/ConstantVisibility";
const BOTH: &str = "Style/ModuleMemberExistenceCheck,Style/ConstantVisibility";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn run(dir: &Path, args: &[&str], status: i32, counts: [usize; 2]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    for (cop, count) in [MEMBER, VISIBILITY].into_iter().zip(counts) {
        assert_eq!(
            text.lines().filter(|l| l.contains(cop)).count(),
            count,
            "{args:?}: {text}"
        );
    }
    assert!(out.stderr.is_empty(), "{out:?}");
    out
}
#[test]
fn defaults_opt_in_selectors_config_and_no_fix_never_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "Array.instance_methods.include?(:size)\nmodule M\n  X = 1\nend\n";
    // The module declaration prevents a closed-file correction proof, but does
    // not disable detection. Neither cop has a path or Ruby-version gate.
    for (config, args, counts) in [
        (String::new(), vec!["lib/a.rb"], [1, 0]),
        (String::new(), vec!["--only", MEMBER, "lib/a.rb"], [1, 0]),
        (
            String::new(),
            vec!["--only", VISIBILITY, "lib/a.rb"],
            [0, 1],
        ),
        (String::new(), vec!["--only", BOTH, "lib/a.rb"], [1, 1]),
        (
            String::new(),
            vec!["--only", BOTH, "--except", VISIBILITY, "lib/a.rb"],
            [1, 0],
        ),
        (
            String::new(),
            vec!["--only", BOTH, "--except", BOTH, "lib/a.rb"],
            [0, 0],
        ),
        (
            "[cops.Style]\nenabled = false\n".into(),
            vec!["--only", BOTH, "lib/a.rb"],
            [1, 1],
        ),
        (
            format!(
                "[cops.\"{MEMBER}\"]\nenabled = false\n[cops.\"{VISIBILITY}\"]\nenabled = true\n"
            ),
            vec!["lib/a.rb"],
            [0, 1],
        ),
        (
            format!(
                "[cops.Style]\nenabled = false\n[cops.\"{MEMBER}\"]\nenabled = true\n[cops.\"{VISIBILITY}\"]\nenabled = true\n"
            ),
            vec!["lib/a.rb"],
            [1, 1],
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(dir.path(), &args, i32::from(counts != [0, 0]), counts);
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn fixed_settings_are_not_public_style_or_ruby_parameters() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "module M; X = 1; end\n";
    fs::write(&path, source).unwrap();
    for (cop, setting) in [
        (VISIBILITY, "IgnoreModules = true"),
        (MEMBER, "TargetRubyVersion = '3.4'"),
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\n{setting}\n"),
        )
        .unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", BOTH, "--fix", "lib/a.rb"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("is not a valid setting"),
            "{out:?}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
}

#[test]
fn directives_and_config_excludes_suppress_reports_and_fixes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, code) in [
        (MEMBER, "Array.instance_methods.include?(:size)"),
        (VISIBILITY, "module M; X = 1; end"),
    ] {
        for prefix in ["rustocop", "rubocop"] {
            for source in [
                format!("# {prefix}:disable {cop}\n{code}\n"),
                format!("{code} # {prefix}:disable {cop}\n"),
                format!("# {prefix}:disable-next {cop}\n{code}\n"),
            ] {
                fs::write(&path, &source).unwrap();
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                run(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0, [0, 0]);
                assert_eq!(fs::read_to_string(&path).unwrap(), source);
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            }
        }
        for config in [
            format!("[cops.\"{cop}\"]\nexclude = ['lib/**']\n"),
            "exclude = ['lib/**']\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            fs::write(&path, code).unwrap();
            run(
                &dir.path().join("lib"),
                &["--only", cop, "--fix", "a.rb"],
                0,
                [0, 0],
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), code);
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
    for prefix in ["rustocop", "rubocop"] {
        // Source opt-in of ConstantVisibility works without --only. Closed literal
        // member calls can still correct around suppression comments.
        let source = format!(
            "# {prefix}:enable {VISIBILITY}\nmodule M; X = 1; end\n# {prefix}:disable {VISIBILITY}\nmodule N; X = 1; end\n"
        );
        fs::write(&path, &source).unwrap();
        run(dir.path(), &["--fix", "lib/a.rb"], 1, [0, 1]);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        let source = format!(
            "# {prefix}:disable {MEMBER}\nArray.instance_methods.include?(:size)\n# {prefix}:enable {MEMBER}\nArray.instance_methods.include?(:size)\n"
        );
        fs::write(&path, &source).unwrap();
        run(
            dir.path(),
            &["--only", MEMBER, "--fix", "lib/a.rb"],
            0,
            [0, 0],
        );
        let at = source
            .rfind("Array.instance_methods.include?(:size)")
            .unwrap();
        let expected = format!("{}Array.method_defined?(:size)\n", &source[..at]);
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    }
}
#[test]
fn canonical_fixes_and_uncorrectable_clean_error_files_keep_metadata() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "Array.instance_methods.include?(:size)\n").unwrap();
    run(
        dir.path(),
        &["--only", MEMBER, "--fix", "lib/a.rb"],
        0,
        [0, 0],
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "Array.method_defined?(:size)\n"
    );
    for (source, status, counts) in [
        ("Array.method_defined?(:size)\n", 0, [0, 0]),
        ("Array.instance_methods.include?(\n", 2, [0, 0]),
        ("x.instance_methods.include?(:size)\n", 1, [1, 0]),
        ("module M; X = 1; end\n", 1, [0, 1]),
        ("module M; X = 1; private_constant :X; end\n", 0, [0, 0]),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", BOTH, "--fix", "lib/a.rb"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(status), "{output:?}");
        for (cop, count) in [MEMBER, VISIBILITY].into_iter().zip(counts) {
            assert_eq!(
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .filter(|l| l.contains(cop))
                    .count(),
                count,
                "{output:?}"
            );
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
fn ruby(dir: &Path) -> Option<Output> {
    Command::new("ruby")
        .current_dir(dir)
        .arg("lib/a.rb")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .ok()
}
fn same(before: &Output, after: &Output) {
    assert!(before.status.success(), "{before:?}");
    assert_eq!(before.status.code(), after.status.code());
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
}
#[test]
fn ruby_core_visibility_inheritance_and_identity_survive_canonical_fixes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "p [Array.instance_methods.include?(:size), Array.public_instance_methods(true).member?(:size), Array.private_instance_methods.include?(:initialize), Array.protected_instance_methods.include?(:size), Array.instance_methods(false).include?(:tap), Array.instance_methods(true).include?(:tap), Array.class_variables.include?(:@@foo), Array&.instance_methods&.include?(:size), __LINE__]\n";
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path());
    run(
        dir.path(),
        &["--only", MEMBER, "--fix", "lib/a.rb"],
        0,
        [0, 0],
    );
    let fixed = fs::read_to_string(&path).unwrap();
    assert!(!fixed.contains(".include?"));
    assert!(!fixed.contains(".member?"));
    if let Some(before) = before {
        assert_eq!(
            before.stdout,
            b"[true, true, true, false, false, true, false, true, 1]\n"
        );
        same(&before, &ruby(dir.path()).unwrap());
    }
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    run(
        dir.path(),
        &["--only", MEMBER, "--fix", "lib/a.rb"],
        0,
        [0, 0],
    );
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
#[test]
fn ruby_dynamic_names_binding_eval_control_flow_and_flags_are_not_rewritten() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, count) in [
        (
            "p [Array.instance_methods.include?('size'), Array.class_variables.include?(:invalid)]\n",
            2,
        ),
        (
            "$events = []; def receiver; $events << :receiver; Array; end; def flag; $events << :flag; false; end; def name; $events << :name; :size; end\np [receiver.instance_methods(flag).include?(name), $events]\n",
            1,
        ),
        (
            "x = Object.new; def x.instance_methods; [:size]; end; def x.method_defined?(name); false; end; p x.instance_methods.include?(:size)\n",
            1,
        ),
        (
            "class Array; def self.method_defined?(name); false; end; end; p Array.instance_methods.include?(:size)\n",
            1,
        ),
        (
            "module R; refine Module do; def method_defined?(name); false; end; end; end; using R; p Array.instance_methods.include?(:size)\n",
            1,
        ),
        (
            "eval('class Array; def self.method_defined?(name); false; end; end'); p Array.instance_methods.include?(:size)\n",
            1,
        ),
        (
            "p [nil&.instance_methods&.include?(:size), catch(:done) { Array.instance_methods.include?(throw(:done, :exit)) }]\n",
            2,
        ),
        (
            "def f(value = Array.instance_methods.include?(:size)); value; end; p f\n",
            1,
        ),
        (
            "p Array.instance_methods.include?(:size) { break :other }\n",
            1,
        ),
        (
            "class Parent; def foo; end; protected def bar; end; private def baz; end; end\nmodule Mix; def mixed; end; end\nclass Child < Parent; include Mix; extend Mix; end\np [Child.instance_methods.include?(:foo), Child.instance_methods(false).include?(:foo), Child.protected_instance_methods.include?(:bar), Child.private_instance_methods.include?(:baz), Child.instance_methods.include?(:mixed)]\n",
            5,
        ),
    ] {
        fs::write(&path, source).unwrap();
        let before = ruby(dir.path());
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(
            dir.path(),
            &["--only", MEMBER, "--fix", "lib/a.rb"],
            1,
            [count, 0],
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        if let Some(before) = before {
            same(&before, &ruby(dir.path()).unwrap());
        }
    }
    fs::write(dir.path().join("lib/load.rb"), "$events << :autoload; module Loaded; def self.instance_methods; [:size]; end; def self.method_defined?(*); false; end; end\n").unwrap();
    let source = "$events = []; autoload :Loaded, './lib/load.rb'; p [Loaded.instance_methods.include?(:size), $events]\n";
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path());
    run(
        dir.path(),
        &["--only", MEMBER, "--fix", "lib/a.rb"],
        1,
        [1, 0],
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    if let Some(before) = before {
        assert_eq!(before.stdout, b"[true, [:autoload]]\n");
        same(&before, &ruby(dir.path()).unwrap());
    }
}
#[test]
fn ruby_constant_nesting_qualified_reopen_and_singleton_visibility_remain_unchanged() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "module Outer\n X = :outer\n private_constant :X\n module Inner\n  Y = X\n  def self.value; [Y, Module.nesting]; end\n end\nend\nclass C; X = :public; class << self; X = :singleton; end; end\nmodule Outer::Inner; Z = :qualified; end\nmodule Outer::Inner; private_constant :Z; end\np [Outer::Inner.value, C::X, C.singleton_class::X, Outer.constants(false), Outer::Inner.constants(false)]\n";
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path());
    run(
        dir.path(),
        &["--only", BOTH, "--fix", "lib/a.rb"],
        1,
        [0, 3],
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    if let Some(before) = before {
        same(&before, &ruby(dir.path()).unwrap());
    }
}
#[test]
fn canonical_edits_preserve_bom_crlf_encodings_comments_line_values_and_data() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for source in [
        b"\xef\xbb\xbf# coding: utf-8\r\np ['caf\xc3\xa9', __LINE__, Array.instance_methods(true).include?(:size)] # keep\r\n__END__\r\nArray.instance_methods.include?(:size)\xff\r\n".to_vec(),
        b"# coding: ISO-8859-1\r\np ['\xe9', __LINE__, Array.instance_methods(true).include?(:size)] # keep\r\n__END__\r\n\xe9.instance_methods\r\n".to_vec(),
    ] {
        fs::write(&path, &source).unwrap();
        let before = ruby(dir.path());
        run(dir.path(), &["--only", MEMBER, "--fix", "lib/a.rb"], 0, [0, 0]);
        let old = b"instance_methods(true).include?(:size)";
        let at = source.windows(old.len()).position(|w| w == old).unwrap();
        let mut expected = source[..at].to_vec(); expected.extend(b"method_defined?(:size)"); expected.extend(&source[at + old.len()..]);
        assert_eq!(fs::read(&path).unwrap(), expected);
        if let Some(before) = before { same(&before, &ruby(dir.path()).unwrap()); }
    }
}
#[test]
fn curated_style_layout_composition_stabilizes_and_preserves_observations() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "p( [ 'value',Array.instance_methods(true).include?(:size) ] );\n";
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path());
    let only = "Style/ModuleMemberExistenceCheck,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout";
    let mut stable = false;
    for _ in 0..4 {
        let old = fs::read(&path).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", only, "--fix", "lib/a.rb"])
            .output()
            .unwrap();
        assert!(matches!(output.status.code(), Some(0 | 1)), "{output:?}");
        if fs::read(&path).unwrap() == old {
            stable = true;
            break;
        }
    }
    assert!(
        stable,
        "at most three changing passes and one no-write check"
    );
    let fixed = fs::read_to_string(&path).unwrap();
    assert!(fixed.contains("method_defined?(:size)"), "{fixed}");
    assert!(!fixed.contains("instance_methods"));
    if let Some(before) = before {
        same(&before, &ruby(dir.path()).unwrap());
    }
}

#[test]
fn stdin_is_lint_only_and_obeys_display_path_excludes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "disk bytes").unwrap();
    for (config, count) in [("", 1), ("exclude = ['lib/**']\n", 0)] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", BOTH, "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"Array.instance_methods.include?(:size)\nmodule M; X = 1; end\n")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(count));
        for cop in [MEMBER, VISIBILITY] {
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).matches(cop).count(),
                count as usize
            );
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "disk bytes");
    }
}
