//! real CLI policy, no-write contracts and isolated Ruby observations.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const CHECK: &str = "Style/ClassCheck";
const EQUALITY: &str = "Style/ClassEqualityComparison";
const MISSING: &str = "Style/MissingRespondToMissing";
const ALL: &str = "Style/ClassCheck,Style/ClassEqualityComparison,Style/MissingRespondToMissing";
const NAMES: [&str; 3] = [CHECK, EQUALITY, MISSING];
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
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
    for (cop, count) in NAMES.into_iter().zip(counts) {
        assert_eq!(
            text.lines().filter(|line| line.contains(cop)).count(),
            count,
            "{args:?}: {text}"
        );
    }
    out
}
#[test]
fn default_selectors_config_reenabling_and_no_fix_never_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source =
        "p x.kind_of?(String)\np x.class == Date\ndef method_missing(name)\n  super\nend\n";
    fs::write(&path, source).unwrap();
    for (config, args, counts) in [
        (String::new(), vec!["lib/a.rb"], [1, 1, 1]),
        (String::new(), vec!["--only", CHECK, "lib/a.rb"], [1, 0, 0]),
        (
            String::new(),
            vec!["--only", EQUALITY, "lib/a.rb"],
            [0, 1, 0],
        ),
        (
            String::new(),
            vec!["--only", MISSING, "lib/a.rb"],
            [0, 0, 1],
        ),
        (String::new(), vec!["--except", ALL, "lib/a.rb"], [0, 0, 0]),
        (
            String::new(),
            vec!["--only", ALL, "--except", CHECK, "lib/a.rb"],
            [0, 1, 1],
        ),
        (
            "[cops.Style]\nenabled = false\n".into(),
            vec!["--only", ALL, "lib/a.rb"],
            [1, 1, 1],
        ),
        (
            NAMES
                .map(|cop| format!("[cops.\"{cop}\"]\nenabled = false\n"))
                .join(""),
            vec!["lib/a.rb"],
            [0, 0, 0],
        ),
        (
            format!(
                "[cops.Style]\nenabled = false\n{}",
                NAMES
                    .map(|cop| format!("[cops.\"{cop}\"]\nenabled = true\n"))
                    .join("")
            ),
            vec!["lib/a.rb"],
            [1, 1, 1],
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        // Another cop now independently reports method_missing's sole super. These
        // peer-specific counts remain unchanged; ordinary Lint still runs
        // unless the test uses exclusive Style selection.
        let delegation = !args.contains(&"--only");
        let status = i32::from(counts != [0, 0, 0] || delegation);
        let out = run(dir.path(), &args, status, counts);
        assert_eq!(
            String::from_utf8_lossy(&out.stdout)
                .matches("(Lint/UselessMethodDefinition)")
                .count(),
            usize::from(delegation)
        );
        if counts == [0, 0, 0] {
            let mut fixing = args.clone();
            fixing.insert(0, "--fix");
            run(dir.path(), &fixing, status, counts);
        }
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn directives_and_config_relative_excludes_suppress_reports_and_fixes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, code) in [
        (CHECK, "x.kind_of?(String)"),
        (EQUALITY, "x.class == Date"),
        (MISSING, "class C; def method_missing; end; end"),
    ] {
        for prefix in ["rustocop", "rubocop"] {
            fs::write(dir.path().join("rustocop.toml"), "").unwrap();
            for source in [
                format!("# {prefix}:disable {cop}\n{code}\n"),
                format!("{code} # {prefix}:disable {cop}\n"),
                format!("# {prefix}:disable-next {cop}\n{code}\n"),
            ] {
                fs::write(&path, &source).unwrap();
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                run(
                    dir.path(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    0,
                    [0, 0, 0],
                );
                assert_eq!(fs::read_to_string(&path).unwrap(), source);
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            }
        }
        for prefix in ["rustocop", "rubocop"] {
            fs::write(dir.path().join("rustocop.toml"), "[cops.Style]\nenabled = false\n[cops.Lint]\nenabled = false\n[cops.Layout]\nenabled = false\n").unwrap();
            let source =
                format!("# {prefix}:enable {cop}\n{code}\n# {prefix}:disable {cop}\n{code}\n");
            fs::write(&path, &source).unwrap();
            let counts = match cop {
                EQUALITY => [0, 1, 0],
                MISSING => [0, 0, 1],
                _ => [0, 0, 0],
            };
            run(
                dir.path(),
                &["--fix", "lib/a.rb"],
                i32::from(cop != CHECK),
                counts,
            );
            let expected = if cop == CHECK {
                source.replacen("kind_of?", "is_a?", 1)
            } else {
                source
            };
            assert_eq!(fs::read_to_string(&path).unwrap(), expected);
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
                [0, 0, 0],
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), code);
        }
    }
    // Definition highlights include the entire body; selector highlights don't.
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for source in [
        "class C\n def method_missing\n  42 # rubocop:disable Style/MissingRespondToMissing\n end\nend\n",
        "x.kind_of?(\n String # rustocop:disable Style/ClassCheck\n)\n",
    ] {
        fs::write(&path, source).unwrap();
        run(
            dir.path(),
            &["--only", ALL, "--fix", "lib/a.rb"],
            0,
            [0, 0, 0],
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            source.replace("kind_of?", "is_a?")
        );
    }
}
#[test]
fn canonical_fixes_detection_only_and_clean_error_withheld_no_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "x.kind_of? y\nx&.kind_of? y\nx.class.name == 'Date'\n";
    fs::write(&path, source).unwrap();
    run(
        dir.path(),
        &["--only", ALL, "--fix", "lib/a.rb"],
        1,
        [0, 1, 0],
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        source.replace("kind_of?", "is_a?")
    );
    for (source, status, counts) in [
        ("x.is_a?(String)\n", 0, [0, 0, 0]),
        ("x.kind_of?(\n", 2, [0, 0, 0]),
        ("x.class == Date\ndef method_missing; end\n", 1, [0, 1, 1]),
        (
            "def is_a?(x); false; end\nx.kind_of?(String)\n",
            1,
            [1, 0, 0],
        ),
        (
            "module R; refine String do; def is_a?(x); false; end; end; end\nusing R\n''.kind_of?(String)\n",
            1,
            [1, 0, 0],
        ),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(
            dir.path(),
            &["--only", ALL, "--fix", "lib/a.rb"],
            status,
            counts,
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn selector_edits_preserve_bom_crlf_encodings_data_comments_and_line_values() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for source in [
        b"\xef\xbb\xbf# coding: utf-8\r\ns = 'caf\xc3\xa9'; p [s.kind_of?(String), s.encoding.name, s.bytes, __LINE__, DATA.read.bytes]\r\n# keep\r\n__END__\r\nraw.kind_of?\xff\r\n".to_vec(),
        b"# coding: ISO-8859-1\r\ns = '\xe9'; p [s.kind_of?(String), s.encoding.name, s.bytes, __LINE__, DATA.read.bytes]\r\n__END__\r\n\xe9.kind_of?\r\n".to_vec(),
    ] {
        // Reading `DATA` is a listed observer: that file keeps
        // its finding and bytes. Without the read, the data
        // section's bytes still survive the canonical edit.
        fs::write(&path, &source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(dir.path(), &["--only", CHECK, "--fix", "lib/a.rb"], 1, [1, 0, 0]);
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        let at = source.windows(18).position(|w| w == b", DATA.read.bytes]").unwrap();
        let source = [&source[..at], &source[at + 17..]].concat();
        fs::write(&path, &source).unwrap();
        let before = ruby(dir.path());
        run(dir.path(), &["--only", CHECK, "--fix", "lib/a.rb"], 0, [0, 0, 0]);
        let at = source.windows(8).position(|w| w == b"kind_of?").unwrap();
        let mut expected = source[..at].to_vec(); expected.extend(b"is_a?"); expected.extend(&source[at + 8..]);
        assert_eq!(fs::read(&path).unwrap(), expected);
        if let Some(before) = before { same(&before, &ruby(dir.path()).unwrap()); }
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(dir.path(), &["--only", CHECK, "--fix", "lib/a.rb"], 0, [0, 0, 0]);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn stdin_is_lint_only_and_uses_the_display_path_for_excludes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "unchanged disk bytes").unwrap();
    for (config, count) in [
        ("", 1),
        ("[cops.\"Style/ClassCheck\"]\nexclude = ['lib/**']\n", 0),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", ALL, "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"x.kind_of?(String)\n")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(count));
        assert_eq!(
            String::from_utf8_lossy(&output.stdout)
                .matches(CHECK)
                .count(),
            count as usize
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "unchanged disk bytes");
    }
    run(
        dir.path(),
        &["--only", ALL, "--stdin", "lib/a.rb", "--fix"],
        2,
        [0, 0, 0],
    );
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
fn ruby_canonical_aliases_preserve_evaluation_binding_control_flow_match_state_and_locations() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    // `method(:f).source_location` is a listed observer that
    // would withhold every edit; `__LINE__` keeps locations.
    let source = "# coding: UTF-8\n$events = []\ndef receiver; $events << :receiver; 'café'; end\ndef argument; $events << :argument; String; end\ndef callback; $events << :callback; proc { raise 'ignored' }; end\ndef f(value = receiver.kind_of?(argument, &callback)); return value; end\ndef g; receiver.kind_of?(argument) { break :wrong }; end\n/(ca)(fé)/ =~ 'café'\nvalues = [f, g, nil&.kind_of?(argument), catch(:done) { throw :done, 'x'.kind_of?(String) }, -> { kind_of?(Object) }.call]\np [values, $events, $~, $1, $2, __LINE__, 'é'.encoding.name]\n";
    let listed = source.replace("__LINE__,", "__LINE__, method(:f).source_location,");
    fs::write(&path, &listed).unwrap();
    run(
        dir.path(),
        &["--only", CHECK, "--fix", "lib/a.rb"],
        1,
        [5, 0, 0],
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), listed);
    // An arbitrary callback factory no longer proves harmless proc acquisition,
    // even when its local definition happens to return a Proc. No return flow
    // analysis is attempted; retain diagnostics without writing this variant.
    for (source, eligible) in [
        (source.to_owned(), false),
        (source.replace("&callback", "&-> { raise 'ignored' }"), true),
    ] {
        fs::write(&path, &source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let Some(before) = ruby(dir.path()) else {
            return;
        };
        run(
            dir.path(),
            &["--only", CHECK, "--fix", "lib/a.rb"],
            if eligible { 0 } else { 1 },
            [if eligible { 0 } else { 5 }, 0, 0],
        );
        if eligible {
            assert!(!fs::read_to_string(&path).unwrap().contains("kind_of?"));
        } else {
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        }
        same(&before, &ruby(dir.path()).unwrap());
    }
}
#[test]
fn ruby_custom_selectors_visibility_refinements_and_unsafe_comparisons_stay_unchanged() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for source in [
        "o = Object.new\ndef o.kind_of?(x); [:custom, x]; end\np o.kind_of?(String)\n",
        "o = Object.new\nclass << o; private :is_a?; end\np [o.kind_of?(Object), o.public_send(:kind_of?, Object)]\n",
        "module R; refine String do; def is_a?(x); false; end; end; end\nusing R\np ''.kind_of?(String)\n",
        "class C; def custom(x); :alias; end; alias kind_of? custom; end\np C.new.kind_of?(String)\n",
        "class DSL; def self.kind_of?(*args, &block); [args, block.call]; end; end\np DSL.kind_of?(:model) { :dsl }\n",
        "Object.new.tap { |o| o.define_singleton_method(:is_a?) { |x| false }; p o.kind_of?(Object) }\n",
    ] {
        fs::write(&path, source).unwrap();
        let Some(before) = ruby(dir.path()) else {
            return;
        };
        run(
            dir.path(),
            &["--only", CHECK, "--fix", "lib/a.rb"],
            1,
            [1, 0, 0],
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        same(&before, &ruby(dir.path()).unwrap());
    }
    let source = "class Delegate\n def method_missing(name, *args); name == :upcase ? 'FOO' : super; end\nend\nd = Delegate.new\np [d.upcase, d.respond_to?(:upcase), d.respond_to?(:upcase, true), d.class.name == 'Delegate', d.class == Delegate]\nclass VisibleDelegate\n private def respond_to_missing?(name, include_private = false); name == :upcase || super; end\n def method_missing(name, *args); name == :upcase ? 'FOO' : super; end\nend\nv = VisibleDelegate.new\np [v.upcase, v.respond_to?(:upcase), v.respond_to?(:upcase, true), v.public_methods.include?(:respond_to_missing?)]\n";
    fs::write(&path, source).unwrap();
    let Some(before) = ruby(dir.path()) else {
        return;
    };
    run(
        dir.path(),
        &["--only", ALL, "--fix", "lib/a.rb"],
        1,
        [0, 2, 1],
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert_eq!(
        before.stdout,
        b"[\"FOO\", false, false, true, true]\n[\"FOO\", true, true, false]\n"
    );
    same(&before, &ruby(dir.path()).unwrap());
    // A class-name string need not denote a constant; custom .class/.==
    // dispatch and a shadowed constant are likewise not safe to replace.
    let source = "fake = Object.new\ndef fake.name; 'DoesNotExist'; end\ndef fake.==(other); :custom_equality; end\nobj = Object.new\nobj.define_singleton_method(:class) { fake }\nDate = :shadow\np [obj.class.name == 'DoesNotExist', obj.class == Date]\n";
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path()).unwrap();
    run(
        dir.path(),
        &["--only", ALL, "--fix", "lib/a.rb"],
        1,
        [0, 2, 0],
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert_eq!(before.stdout, b"[true, :custom_equality]\n");
    same(&before, &ruby(dir.path()).unwrap());
}
#[test]
fn ruby_module_function_visibility_is_not_changed_by_fixes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for visibility in [
        "module_function :is_a?",
        "send(:module_function, :is_a?)",
        "__send__('module_function', 'is_a?')",
        "name = :is_a?; send(:module_function, name)",
    ] {
        let source = format!(
            "module M; {visibility}; end\nclass C; include M; end\np [C.new.kind_of?(C), C.private_instance_methods.include?(:is_a?)]\n"
        );
        fs::write(&path, &source).unwrap();
        let before = ruby(dir.path());
        run(
            dir.path(),
            &["--only", CHECK, "--fix", "lib/a.rb"],
            1,
            [1, 0, 0],
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        if let Some(before) = before {
            assert_eq!(before.stdout, b"[true, true]\n");
            same(&before, &ruby(dir.path()).unwrap());
        }
    }
}
#[test]
fn ruby_method_object_binders_and_visibility_are_reported_without_writes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, expected) in [
        // Struct/Data member DSLs also bind the tested selectors, including
        // through stored or reflectively acquired Method objects.
        (
            "klass = Struct.method(:new).call(:is_a?); obj = klass.new(false); p obj.kind_of?(Object)\n",
            "true\n",
        ),
        (
            "binder = Struct.public_method('new'); klass = binder[:is_a?]; obj = klass.new(false); p obj.kind_of?(Object)\n",
            "true\n",
        ),
        (
            "klass = Struct.send(:method, :new).call(:is_a?); obj = klass.new(false); p obj.kind_of?(Object)\n",
            "true\n",
        ),
        (
            "klass = Data.method(:define).call(:is_a?); obj = klass.new(false); p obj.kind_of?(Object)\n",
            "true\n",
        ),
        (
            "binder = Data.public_method('define'); klass = binder[:is_a?]; obj = klass.new(false); p obj.kind_of?(Object)\n",
            "true\n",
        ),
        (
            "klass = Data.public_send(:method, :define).call(:is_a?); obj = klass.new(false); p obj.kind_of?(Object)\n",
            "true\n",
        ),
        // Exact independent repro.
        (
            "obj = Object.new; obj.method(:define_singleton_method).call(:is_a?) { |_| false }; p obj.kind_of?(Object)\n",
            "true\n",
        ),
        (
            "obj = Object.new; obj.public_method('define_singleton_method')[:is_a?] { |_| false }; p obj.kind_of?(Object)\n",
            "true\n",
        ),
        (
            "obj = Object.new; binder = obj.method(:define_singleton_method); binder.call(:is_a?) { |_| false }; p obj.kind_of?(Object)\n",
            "true\n",
        ),
        (
            "obj = Object.new; Module.instance_method(:define_method).bind_call(obj.singleton_class, :is_a?) { |_| false }; p obj.kind_of?(Object)\n",
            "true\n",
        ),
        (
            "obj = Object.new; Module.public_instance_method(:define_method).bind(obj.singleton_class)[:is_a?] { |_| false }; p obj.kind_of?(Object)\n",
            "true\n",
        ),
        (
            "obj = Object.new; obj.singleton_class.method(:private).call(:is_a?); p [obj.kind_of?(Object), obj.respond_to?(:is_a?)]\n",
            "[true, false]\n",
        ),
        (
            "module M; method(:module_function).call(:is_a?); end; class C; include M; end; p [C.new.kind_of?(C), C.private_instance_methods.include?(:is_a?)]\n",
            "[true, true]\n",
        ),
        (
            "module M; binder = method(:module_function); binder[:is_a?]; end; class C; include M; end; p [C.new.kind_of?(C), C.private_instance_methods.include?(:is_a?)]\n",
            "[true, true]\n",
        ),
        (
            "obj = Object.new; selector = :define_singleton_method; obj.method(selector).call(:is_a?) { |_| false }; p obj.kind_of?(Object)\n",
            "true\n",
        ),
        (
            "obj = Object.new; obj.send(:method, :define_singleton_method).call(:is_a?) { |_| false }; p obj.kind_of?(Object)\n",
            "true\n",
        ),
        (
            "obj = Object.new; obj.method(:method).call(:define_singleton_method).call(:is_a?) { |_| false }; p obj.kind_of?(Object)\n",
            "true\n",
        ),
        (
            "obj = Object.new; obj.method(:send).call(:define_singleton_method, :is_a?) { |_| false }; p obj.kind_of?(Object)\n",
            "true\n",
        ),
        // singleton_method only retrieves singleton-owned methods: copy the
        // inherited binder without defining either tested selector.
        (
            "obj = Object.new; class << obj; alias define_singleton_method define_singleton_method; end; obj.singleton_method(:define_singleton_method).call(:is_a?) { |_| false }; p obj.kind_of?(Object)\n",
            "true\n",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let before = ruby(dir.path());
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        for args in [
            vec!["--only", CHECK, "lib/a.rb"],
            vec!["--only", CHECK, "--fix", "lib/a.rb"],
        ] {
            let output = run(dir.path(), &args, 1, [1, 0, 0]);
            assert!(output.stderr.is_empty(), "{output:?}");
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
        if let Some(before) = before {
            assert_eq!(before.stdout, expected.as_bytes(), "{source}: {before:?}");
            same(&before, &ruby(dir.path()).unwrap());
        }
    }
}
#[test]
fn ruby_benign_method_reflection_still_allows_canonical_fixes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for source in [
        "obj = Object.new; p [obj.method(:is_a?).call(Object), obj.kind_of?(Object)]\n",
        "obj = Object.new; p [obj.public_method('kind_of?')[Object], obj.kind_of?(Object)]\n",
        "obj = Object.new; p [Object.instance_method(:is_a?).bind_call(obj, Object), obj.kind_of?(Object)]\n",
        "obj = Object.new; p [Object.public_instance_method(:is_a?).bind(obj).call(Object), obj.kind_of?(Object)]\n",
    ] {
        fs::write(&path, source).unwrap();
        let before = ruby(dir.path());
        run(
            dir.path(),
            &["--only", CHECK, "--fix", "lib/a.rb"],
            0,
            [0, 0, 0],
        );
        let at = source.rfind(".kind_of?").unwrap();
        let expected = format!("{}.is_a?{}", &source[..at], &source[at + 9..]);
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
        if let Some(before) = before {
            assert_eq!(before.stdout, b"[true, true]\n");
            same(&before, &ruby(dir.path()).unwrap());
        }
    }
}
#[test]
fn curated_style_layout_composition_converges_without_runtime_changes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "def f\n  return ({:value => 'x'.kind_of?(String)});\nend\np f\n";
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path());
    let only = "Style/ClassCheck,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout";
    let mut stable = false;
    for _ in 0..4 {
        let old = fs::read(&path).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", only, "--fix", "lib/a.rb"])
            .output()
            .unwrap();
        assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
        if fs::read(&path).unwrap() == old {
            stable = true;
            break;
        }
    }
    assert!(
        stable,
        "curated fixture must stabilize within three changing passes plus one no-write check"
    );
    let fixed = fs::read_to_string(&path).unwrap();
    assert!(fixed.contains("is_a?"));
    assert!(!fixed.contains("kind_of?"));
    if let Some(before) = before {
        same(&before, &ruby(dir.path()).unwrap());
    }
}
