//! real CLI contracts and controlled, local filesystem semantics.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const CASES: &[(&str, &str, &str)] = &[
    (
        "Style/NestedFileDirname",
        "File.dirname(File.dirname(path))\n",
        "File.dirname(path, 2)\n",
    ),
    (
        "Style/ExpandPathArguments",
        "File.expand_path(\"..\", __FILE__)\n",
        "File.expand_path(__dir__)\n",
    ),
    (
        "Style/RedundantCurrentDirectoryInPath",
        "require_relative \"./feature\"\n",
        "require_relative \"feature\"\n",
    ),
    (
        "Style/FileTouch",
        "File.open(\"file\", \"a\") {}\n",
        "File.open(\"file\", \"a\") {}\n",
    ),
    (
        "Style/FileWrite",
        "File.open(\"file\", \"w\") { |f| f.write(\"data\") }\n",
        "File.write(\"file\", \"data\")\n",
    ),
    (
        "Style/YAMLFileRead",
        "YAML.safe_load(File.read(File::NULL))\n",
        "YAML.safe_load_file(File::NULL)\n",
    ),
    ("Style/FileNull", "\"/dev/null\"\n", "\"/dev/null\"\n"),
    (
        "Style/FileOpen",
        "File.open(\"file\")\n",
        "File.open(\"file\")\n",
    ),
    (
        "Style/FileRead",
        "File.open(\"file\", &:read)\n",
        "File.read(\"file\")\n",
    ),
    (
        "Style/Dir",
        "File.expand_path(File.dirname(__FILE__))\n",
        "__dir__\n",
    ),
    (
        "Style/DirEmpty",
        "Dir.children(\".\").empty?\n",
        "Dir.empty?(\".\")\n",
    ),
    (
        "Style/FileEmpty",
        "File.size(\"path/to/file\") == 0\n",
        "File.size(\"path/to/file\") == 0\n",
    ),
];
/// Every target of these cops names the
/// listed `__FILE__`, so their edits are always withheld; findings stay.
fn listed_subject(cop: &str) -> bool {
    matches!(cop, "Style/Dir" | "Style/ExpandPathArguments")
}
fn offense_column(cop: &str) -> usize {
    match cop {
        "Style/YAMLFileRead" | "Style/NestedFileDirname" | "Style/ExpandPathArguments" => 6,
        "Style/RedundantCurrentDirectoryInPath" => 19,
        _ => 1,
    }
}
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn cli(cwd: &Path, args: &[&str], status: i32) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(status), "{args:?}: {output:?}");
    output
}

#[test]
fn defaults_exact_names_selectors_config_and_safe_fix_policy() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let registry = rustocop::cops::Registry::new();
    for (cop, before, after) in CASES {
        let rules: Vec<_> = registry
            .cops()
            .iter()
            .filter(|c| c.name() == *cop)
            .collect();
        assert_eq!(rules.len(), 1);
        assert!(rules[0].default_enabled());
        assert_eq!(rules[0].safe_fixes(), before != after);
        assert!(rules[0].path_policy().is_none());
        let after = if listed_subject(cop) { before } else { after };
        fs::write(&path, before).unwrap();
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let out = cli(dir.path(), &["lib/a.rb"], 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
        assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        for config in [
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            // Isolate this Style correction gate from detection-only EmptyBlock.
            fs::write(
                dir.path().join("rustocop.toml"),
                format!("{config}\n[cops.\"Lint/EmptyBlock\"]\nenabled = false\n"),
            )
            .unwrap();
            // Disabling FileEmpty alone still leaves the independent default
            // NumericPredicate hint for `File.size(...) == 0` (detection only).
            let numeric_remains = *cop == "Style/FileEmpty" && !config.contains("[cops.Style]");
            let out = cli(
                dir.path(),
                &["--fix", "lib/a.rb"],
                i32::from(numeric_remains),
            );
            if numeric_remains {
                assert!(String::from_utf8_lossy(&out.stdout).contains("Style/NumericPredicate"));
            }
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            let out = cli(dir.path(), &["--only", cop, "lib/a.rb"], 1);
            let col = offense_column(cop);
            assert!(
                String::from_utf8_lossy(&out.stdout).contains(&format!("lib/a.rb:1:{col}: C:"))
            );
            cli(
                dir.path(),
                &["--only", cop, "--except", cop, "--fix", "lib/a.rb"],
                0,
            );
            assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n[cops.\"Lint/EmptyBlock\"]\nenabled = false\n"),
        )
        .unwrap();
        let status = i32::from(before == after);
        cli(dir.path(), &["--fix", "lib/a.rb"], status);
        assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], status);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn directives_excludes_and_disabled_spans_preserve_bytes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, before, _) in CASES {
        for prefix in ["rubocop", "rustocop"] {
            for target in [*cop, "Style", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\n{before}"),
                    format!("{} # {prefix}:disable {target}\n", before.trim_end()),
                    format!("# {prefix}:disable-next {target}\n{before}"),
                ] {
                    fs::write(&path, &source).unwrap();
                    cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
                    assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
                }
            }
        }
        fs::write(&path, before).unwrap();
        for config in [
            "exclude = [\"lib/a.rb\"]\n".to_string(),
            format!("[cops.\"{cop}\"]\nexclude = [\"lib/a.rb\"]\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            cli(
                &dir.path().join("lib"),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
            assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
    let source = "Dir.\n children(path).empty? # rubocop:disable Style/DirEmpty\n";
    fs::write(&path, source).unwrap();
    cli(
        dir.path(),
        &["--only", "Style/DirEmpty", "--fix", "lib/a.rb"],
        0,
    );
    assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
}

#[test]
fn stdin_and_uncorrectable_clean_error_files_never_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "sentinel\n").unwrap();
    for (cop, source, _) in CASES {
        for (config, input, status) in [
            (String::new(), source.to_string(), 1),
            (
                format!("[cops.\"{cop}\"]\nexclude = [\"lib/a.rb\"]\n"),
                source.to_string(),
                0,
            ),
            (
                String::new(),
                format!("# rustocop:disable {cop}\n{source}"),
                0,
            ),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path().join("lib"))
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
                .write_all(input.as_bytes())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert_eq!(output.status.code(), Some(status), "{output:?}");
            if status == 1 {
                let col = offense_column(cop);
                assert!(
                    String::from_utf8_lossy(&output.stdout).contains(&format!("a.rb:1:{col}:"))
                );
            }
            assert_eq!(fs::read(&path).unwrap(), b"sentinel\n");
        }
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for (cop, source, status) in [
        (
            "Style/Dir",
            "__dir__ = 1; File.dirname(File.realpath(__FILE__))\n",
            1,
        ),
        (
            "Style/DirEmpty",
            "Dir.children(<<~PATH).empty?\n  path\nPATH\n",
            1,
        ),
        (
            "Style/FileEmpty",
            "\u{feff}# encoding: UTF-8\r\nFile.read(path).empty?\r\np __LINE__\r\n__END__\r\nraw\r\n",
            1,
        ),
        ("Style/FileRead", "File.open( # comment\n 'file').read\n", 1),
        ("Style/FileRead", "File.open('file', 'r+').read\n", 1),
        ("Style/FileRead", "File.open(filename).read\n", 1),
        ("Style/FileRead", "File.open(\n", 2),
        ("Style/FileRead", "File.read('file')\n", 0),
        (
            "Style/FileNull",
            "\u{feff}# encoding: UTF-8\r\n\"/dev/null\"\r\n__END__\r\nraw\r\n",
            1,
        ),
        ("Style/DirEmpty", "Dir.empty?(path)\n", 0),
        ("Style/DirEmpty", "Dir.children(\n", 2),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], status);
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn legacy_encoding_path_bytes_are_reported_but_not_lossily_rewritten() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let bytes =
        b"# encoding: ISO-8859-1\r\nDir.children(\"caf\xe9\").empty?\r\n__END__\r\nraw\xff\r\n";
    fs::write(&path, bytes).unwrap();
    let output = cli(
        dir.path(),
        &["--only", "Style/DirEmpty", "--fix", "lib/a.rb"],
        1,
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("lib/a.rb:2:1:"));
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn mixed_style_and_layout_fixes_stabilize_in_bounded_explicit_passes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(
        &path,
        "def f\n  return (Dir.children('.').empty?);  \nend\n",
    )
    .unwrap();
    let only = "Style/DirEmpty,Style/StringLiterals,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Layout";
    // Deliberate single-pass overlap deferrals, never an automatic fix loop.
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
        "def f\n  Dir.empty?(\".\")\nend\n"
    );
    cli(dir.path(), &["--only", only, "--fix", "lib/a.rb"], 0);
}

#[cfg(unix)]
#[test]
fn symlink_loading_and_the_listed_file_observer_withhold_dir_edits() {
    use std::os::unix::fs::symlink;
    let dir = project();
    let real = dir.path().join("lib/a.rb");
    let alias = dir.path().join("alias.rb");
    symlink("lib/a.rb", &alias).unwrap();
    let before = "p File.expand_path(File.dirname(__FILE__))\n";
    fs::write(&real, before).unwrap();
    cli(dir.path(), &["--only", "Style/Dir", "--fix", "alias.rb"], 1);
    assert_eq!(fs::read_to_string(&real).unwrap(), before);
    // The realpath form is canonical through the alias, but the listed
    // `__FILE__` withholds it too.
    fs::write(&real, "p File.dirname(File.realpath(__FILE__))\n").unwrap();
    cli(dir.path(), &["--only", "Style/Dir", "--fix", "alias.rb"], 1);
    assert_eq!(
        fs::read_to_string(&real).unwrap(),
        "p File.dirname(File.realpath(__FILE__))\n"
    );
    assert!(fs::symlink_metadata(&alias).unwrap().is_symlink());
    // A symlinked parent directory has the same noncanonical loading risk.
    symlink("lib", dir.path().join("aliasdir")).unwrap();
    fs::write(&real, before).unwrap();
    cli(
        dir.path(),
        &["--only", "Style/Dir", "--fix", "aliasdir/a.rb"],
        1,
    );
    assert_eq!(fs::read_to_string(&real).unwrap(), before);
}

#[test]
fn review_runtime_regressions_preserve_indexing_alias_bindings_and_path_errors() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let script = dir.path().join("a.rb");
    fs::write(dir.path().join("regular"), "x").unwrap();
    for (cop, source) in [
        ("Style/Dir", "p File.dirname(File.realpath(__FILE__)) [0]\n"),
        (
            "Style/Dir",
            "p File.dirname(File.realpath(__FILE__)) +\"suffix\"\n",
        ),
        (
            "Style/Dir",
            "class C; alias __dir__ object_id; def run; File.dirname(File.realpath(__FILE__)); end; end; p C.new.run\n",
        ),
        (
            "Style/Dir",
            "class C; alias :__dir__ :object_id; def run; File.dirname(File.realpath(__FILE__)); end; end; p C.new.run\n",
        ),
        (
            "Style/DirEmpty",
            "[:entries, :children, :each_child].each do |method|\n  [\"regular\", \"missing\"].each do |path|\n    p(begin\n      case method\n      when :entries then Dir.entries(path).size == 2\n      when :children then Dir.children(path).empty?\n      else Dir.each_child(path).none?\n      end\n    rescue SystemCallError => e; e.class.name; end)\n  end\nend\n",
        ),
    ] {
        fs::write(&script, source).unwrap();
        let run = || {
            Command::new("ruby")
                .current_dir(dir.path())
                .arg(&script)
                .output()
                .unwrap()
        };
        let before = run();
        assert!(before.status.success(), "{before:?}");
        let fixed = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", cop, "--fix", "a.rb"])
            .output()
            .unwrap();
        assert_ne!(fixed.status.code(), Some(2), "{fixed:?}");
        let after = run();
        assert!(after.status.success(), "{after:?}");
        assert_eq!(before.stdout, after.stdout, "{source}");
    }
}

#[test]
fn literal_method_binding_runtime_regressions_retain_offenses_and_do_not_write() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let script = dir.path().join("a.rb");
    for source in [
        // Exact reproductions: these used to become object_id / 123.
        "class C\n alias_method :__dir__, :object_id\n def run\n  File.dirname(File.realpath(__FILE__))\n end\nend\np C.new.run\n",
        "class C\n define_method(:__dir__) { 123 }\n def run\n  File.dirname(File.realpath(__FILE__))\n end\nend\np C.new.run\n",
        "class C\n self.alias_method '__dir__', :object_id\n def run\n  File.dirname(File.realpath(__FILE__))\n end\nend\np C.new.run\n",
        "class C; end\nC.define_method(:__dir__) { 123 }\nclass C\n def run\n  File.dirname(File.realpath(__FILE__))\n end\nend\np C.new.run\n",
        "object = Object.new\nobject.define_singleton_method(:__dir__) { 123 }\nobject.define_singleton_method(:run) do\n File.dirname(File.realpath(__FILE__))\nend\np object.run\n",
        "class C\n attr_reader :other, :__dir__\n def run\n  File.dirname(File.realpath(__FILE__))\n end\nend\np C.new.run\n",
        "class C\n attr_accessor :other, :__dir__\n def run\n  File.dirname(File.realpath(__FILE__))\n end\nend\np C.new.run\n",
        "class C\n attr :other, :__dir__\n def run\n  File.dirname(File.realpath(__FILE__))\n end\nend\np C.new.run\n",
        "class C\n attr :__dir__, true\n def run\n  File.dirname(File.realpath(__FILE__))\n end\nend\np C.new.run\n",
        "class C\n send(:alias_method, :__dir__, :object_id)\n def run\n  File.dirname(File.realpath(__FILE__))\n end\nend\np C.new.run\n",
        "class C; end\nC.__send__('define_method', '__dir__') { 123 }\nclass C\n def run\n  File.dirname(File.realpath(__FILE__))\n end\nend\np C.new.run\n",
        "object = Object.new\nobject.public_send(:define_singleton_method, '__dir__') { 123 }\np object.instance_exec { File.dirname(File.realpath(__FILE__)) }\n",
        "class C\n undef_method :__dir__\n def run\n  File.dirname(File.realpath(__FILE__))\n end\nend\np C.new.run\n",
        "class C\n undef :__dir__\n def run\n  File.dirname(File.realpath(__FILE__))\n end\nend\np C.new.run\n",
    ] {
        fs::write(&script, source).unwrap();
        let run = || {
            Command::new("ruby")
                .current_dir(dir.path())
                .arg(&script)
                .output()
                .unwrap()
        };
        let before = run();
        assert!(before.status.success(), "{source}: {before:?}");
        assert_eq!(
            String::from_utf8_lossy(&before.stdout).trim(),
            format!("{:?}", dir.path().to_str().unwrap()),
            "the uncorrected result must actually be the directory"
        );
        let modified = fs::metadata(&script).unwrap().modified().unwrap();
        let output = cli(dir.path(), &["--only", "Style/Dir", "--fix", "a.rb"], 1);
        assert!(String::from_utf8_lossy(&output.stdout).contains("Style/Dir"));
        let after = run();
        assert_eq!(before.status.code(), after.status.code(), "{source}");
        assert_eq!(before.stdout, after.stdout, "{source}");
        assert_eq!(before.stderr, after.stderr, "{source}");
        assert_eq!(fs::read_to_string(&script).unwrap(), source);
        assert_eq!(fs::metadata(&script).unwrap().modified().unwrap(), modified);
    }
}

// Outside the narrow closed default-constructor grammar, user constructors can
// still be IO aliases. Keep the original custom-initializer/metaprogramming
// fixtures as exact no-write controls, with constructor-free canonical siblings.
fn assert_custom_constructor_withholds(dir: &Path, script: &Path, source: &str, cops: &str) {
    fs::write(script, source).unwrap();
    let run = || {
        Command::new("ruby")
            .current_dir(dir)
            .arg(script)
            .output()
            .unwrap()
    };
    let before = run();
    assert!(before.status.success(), "{source}: {before:?}");
    let modified = fs::metadata(script).unwrap().modified().unwrap();
    for _ in 0..2 {
        cli(dir, &["--only", cops, "--fix", "a.rb"], 1);
        assert_eq!(fs::read_to_string(script).unwrap(), source);
        assert_eq!(fs::metadata(script).unwrap().modified().unwrap(), modified);
    }
    let after = run();
    assert_eq!(before.status.code(), after.status.code());
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
}

#[test]
fn closed_local_default_constructors_recover_runtime_equivalent_canonical_fixes() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let script = dir.path().join("a.rb");
    for (cop, expression, replacement) in [
        (
            "Style/Dir",
            "File.dirname(File.realpath(__FILE__))",
            "__dir__",
        ),
        (
            "Style/Dir",
            "File.expand_path(File.dirname(__FILE__))",
            "__dir__",
        ),
        (
            "Style/StringLiterals",
            "'filename only'",
            "\"filename only\"",
        ),
        (
            "Layout/SpaceInsideParens",
            "File.dirname( __FILE__ )",
            "File.dirname(__FILE__)",
        ),
    ] {
        for crlf in [false, true] {
            // `__FILE__` is a listed observer that withholds every
            // edit; the StringLiterals sibling prints no path.
            let trailer = if cop == "Style/StringLiterals" {
                ":done"
            } else {
                "__FILE__"
            };
            let source = format!(
                "class Other; end\nclass C\n def run; {expression}; end\nend\nOther.new\np C.new.run\np {trailer}\n"
            );
            let source = if crlf {
                format!("\u{feff}{}", source.replace('\n', "\r\n"))
            } else {
                source
            };
            fs::write(&script, &source).unwrap();
            let run = || {
                Command::new("ruby")
                    .current_dir(dir.path())
                    .arg(&script)
                    .output()
                    .unwrap()
            };
            let before = run();
            assert!(before.status.success(), "{source}: {before:?}");
            if source.contains("__FILE__") {
                // The listed `__FILE__` withholds every
                // cop's edit; the finding stays.
                assert_ne!(expression, replacement);
                cli(dir.path(), &["--only", cop, "--fix", "a.rb"], 1);
                assert_eq!(fs::read_to_string(&script).unwrap(), source);
                assert_eq!(before.stdout, run().stdout, "{source}");
                continue;
            }
            cli(dir.path(), &["--only", cop, "--fix", "a.rb"], 0);
            assert_eq!(
                fs::read_to_string(&script).unwrap(),
                source.replace(expression, replacement)
            );
            let after = run();
            assert_eq!(before.status.code(), after.status.code(), "{source}");
            assert_eq!(before.stdout, after.stdout, "{source}");
            assert_eq!(before.stderr, after.stderr, "{source}");
            let modified = fs::metadata(&script).unwrap().modified().unwrap();
            cli(dir.path(), &["--only", cop, "--fix", "a.rb"], 0);
            assert_eq!(fs::metadata(&script).unwrap().modified().unwrap(), modified);
        }
    }
}

#[test]
fn uncertain_local_and_aliased_io_constructors_still_do_not_write() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let script = dir.path().join("a.rb");
    let class = "class C\n def run; File.dirname(File.realpath(__FILE__)); end\nend\n";
    for source in [
        format!("{}p C.new.run\n", class.replace("class C", "class C < Object")),
        format!("{}p C.new.run\n", class.replace(" def run;", " def initialize; end\n def run;")),
        format!("{class}class C; end\np C.new.run\n"),
        format!("{class}Saved = C\np Saved.new.run\n"),
        format!("{class}C.class_eval {{ def initialize; end }}\np C.new.run\n"),
        format!("{class}later = -> {{ C.define_method(:initialize) {{}} }}\nlater.call\np C.new.run\n"),
        "p File.dirname(File.realpath(__FILE__))\nC = File\nhandle = C.new(__FILE__)\np handle.path\n".into(),
        "p File.dirname(File.realpath(__FILE__))\nconstructor = File\nhandle = constructor.new(__FILE__)\np handle.path\n".into(),
    ] {
        assert_custom_constructor_withholds(dir.path(), &script, &source, "Style/Dir");
    }
}

#[test]
fn unrelated_literal_method_bindings_keep_findings_under_the_listed_hold() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let script = dir.path().join("a.rb");
    for binding in [
        "alias_method :other, :__dir__",
        "define_method(:other) { 123 }",
        "define_singleton_method('__dir__=') { |value| value }",
        "attr_writer :__dir__",
        "attr :other, true",
        "__send__('define_method', :other) { 123 }",
    ] {
        let source = format!(
            "class C\n {binding}\n def run\n  File.dirname(File.realpath(__FILE__))\n end\nend\np C.new.run\n"
        );
        assert_custom_constructor_withholds(dir.path(), &script, &source, "Style/Dir");
        let source = source.replace("C.new.run", "C.allocate.run");
        fs::write(&script, &source).unwrap();
        let run = || {
            Command::new("ruby")
                .current_dir(dir.path())
                .arg(&script)
                .output()
                .unwrap()
        };
        let before = run();
        assert!(before.status.success(), "{source}: {before:?}");
        // Without a constructor the binding is harmless, but the listed
        // `__FILE__` withholds the edit.
        let modified = fs::metadata(&script).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", "Style/Dir", "--fix", "a.rb"], 1);
        let after = run();
        assert!(after.status.success(), "{source}: {after:?}");
        assert_eq!(before.stdout, after.stdout, "{source}");
        assert_eq!(before.stderr, after.stderr, "{source}");
        assert_eq!(fs::read_to_string(&script).unwrap(), source);
        assert_eq!(fs::metadata(&script).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn literal_api_mutations_retain_offenses_and_preserve_ruby_behavior_without_writes() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let script = dir.path().join("a.rb");
    for (cop, binding, expression) in [
        (
            "Style/Dir",
            "File.define_singleton_method(:dirname) { |path| 123 }",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "::File&.define_singleton_method(\"dir\\x6eame\") { |path| 123 }",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.define_singleton_method(:realpath) { |path| '/' }",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.define_singleton_method(:expand_path) { |path| 123 }",
            "File.expand_path(File.dirname(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.singleton_class.alias_method(:dirname, :basename)",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.singleton_class.define_method('dirname') { |path| 123 }",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.send(:define_singleton_method, :dirname) { |path| 123 }",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.singleton_class.__send__('alias_method', :dirname, :basename)",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.public_send(:define_singleton_method, :dirname) { |path| 123 }",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.singleton_class.attr_reader(:dirname)",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.singleton_class.undef_method(:dirname)",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.singleton_class.remove_method(:dirname)",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.private_class_method(:dirname)",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/DirEmpty",
            "Dir.define_singleton_method(:children) { |path| [] }",
            "Dir.children('.').empty?",
        ),
        (
            "Style/DirEmpty",
            "::Dir.define_singleton_method('entries') { |path| ['.', '..'] }",
            "Dir.entries('.').size == 2",
        ),
        (
            "Style/DirEmpty",
            "Dir&.define_singleton_method(:each_child) { |path| [].each }",
            "Dir.each_child('.').none?",
        ),
        (
            "Style/DirEmpty",
            "Dir.define_singleton_method(:empty?) { |path| 123 }",
            "Dir.children('.').empty?",
        ),
        (
            "Style/DirEmpty",
            "Dir.singleton_class.alias_method(:children, :entries)",
            "Dir.children('.').empty?",
        ),
        (
            "Style/DirEmpty",
            "Dir.singleton_class.define_method(:empty?) { |path| 123 }",
            "Dir.children('.').empty?",
        ),
        (
            "Style/DirEmpty",
            "Dir.__send__('define_singleton_method', 'empty?') { |path| 123 }",
            "Dir.children('.').empty?",
        ),
        (
            "Style/DirEmpty",
            "Dir.singleton_class.undef_method(:children)",
            "Dir.children('.').empty?",
        ),
        (
            "Style/DirEmpty",
            "Dir.private_class_method(:empty?)",
            "Dir.children('.').empty?",
        ),
        (
            "Style/Dir",
            "(File).define_singleton_method(:dirname) { |path| 123 }",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.send(:singleton_class).define_method(:dirname) { |path| 123 }",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.instance_eval { define_singleton_method(:dirname) { |path| 123 } }",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.instance_eval { self.singleton_class.define_method(:dirname) { |path| 123 } }",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.instance_eval { singleton_class.alias_method(:dirname, :basename) }",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.instance_eval { def dirname(path); 123; end }",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.instance_eval { alias dirname basename }",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.singleton_class.class_eval { alias_method :dirname, :basename }",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.singleton_class.module_exec { attr_reader :dirname }",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.singleton_class.class_eval { private :dirname }",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/Dir",
            "File.singleton_class.class_eval { undef dirname }",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/DirEmpty",
            "Dir.instance_eval { define_singleton_method(:empty?) { |path| 123 } }",
            "Dir.children('.').empty?",
        ),
        (
            "Style/DirEmpty",
            "Dir.singleton_class.class_exec(:empty?) { |name| define_method(name) { |path| 123 } }",
            "Dir.children('.').empty?",
        ),
        (
            "Style/DirEmpty",
            "Dir.send(:instance_exec, :empty?) { |name| define_singleton_method(name) { |path| 123 } }",
            "Dir.children('.').empty?",
        ),
        (
            "Style/Dir",
            "File.instance_eval { Dir.class_exec(define_singleton_method(:dirname) { |path| 123 }) {} }",
            "File.dirname(File.realpath(__FILE__))",
        ),
        (
            "Style/DirEmpty",
            "Dir.instance_eval { File.class_exec(define_singleton_method(:empty?) { |path| 123 }) {} }",
            "Dir.children('.').empty?",
        ),
    ] {
        let source = format!(
            "{binding}\np(begin; {expression}; rescue NoMethodError, ArgumentError => e; e.class.name; end)\n"
        );
        fs::write(&script, &source).unwrap();
        let run = || {
            Command::new("ruby")
                .current_dir(dir.path())
                .arg(&script)
                .output()
                .unwrap()
        };
        let before = run();
        assert!(before.status.success(), "{source}: {before:?}");
        if binding == "File.define_singleton_method(:dirname) { |path| 123 }"
            || binding.starts_with("File.instance_eval { Dir.class_exec(")
        {
            assert_eq!(before.stdout, b"123\n", "exact reproduction");
        }
        if binding.starts_with("Dir.instance_eval { File.class_exec(") {
            assert_eq!(
                before.stdout, b"false\n",
                "directory is nonempty before fixing"
            );
        }
        let modified = fs::metadata(&script).unwrap().modified().unwrap();
        let output = cli(dir.path(), &["--only", cop, "--fix", "a.rb"], 1);
        assert!(String::from_utf8_lossy(&output.stdout).contains(cop));
        let after = run();
        assert_eq!(before.status.code(), after.status.code(), "{source}");
        assert_eq!(before.stdout, after.stdout, "{source}");
        assert_eq!(before.stderr, after.stderr, "{source}");
        assert_eq!(fs::read_to_string(&script).unwrap(), source);
        assert_eq!(fs::metadata(&script).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn unrelated_literal_api_mutations_keep_canonical_runtime_equivalent_dir_empty_fixes() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let script = dir.path().join("a.rb");
    for binding in [
        "File.define_singleton_method(:other) { 123 }",
        "::Dir&.define_singleton_method('other') { 123 }",
        "File.singleton_class.alias_method(:other, :dirname)",
        "Dir.singleton_class.alias_method(:other, :children)",
        "File.singleton_class.attr_writer(:dirname)",
        "File.singleton_class.send(:define_method, :other) { 123 }",
        "Dir.__send__('define_singleton_method', :other) { 123 }",
        "File.instance_eval { define_singleton_method(:other) { 123 } }",
        "File.instance_eval { alias other dirname }",
        "Dir.singleton_class.class_eval { alias_method :other, :empty? }",
        "File.singleton_class.class_exec(:other) { |name| define_method(:other) { name } }",
        "Other = Object.new; Other.define_singleton_method(:dirname) { 123 }",
    ] {
        // With Style/Dir's target the listed `__FILE__` withholds both edits
        //; the findings stay and nothing is written.
        let listed = format!(
            "{binding}\np File.dirname(File.realpath(__FILE__))\np Dir.children('.').empty?\n"
        );
        fs::write(&script, &listed).unwrap();
        let modified = fs::metadata(&script).unwrap().modified().unwrap();
        let output = cli(
            dir.path(),
            &["--only", "Style/Dir,Style/DirEmpty", "--fix", "a.rb"],
            1,
        );
        let report = String::from_utf8_lossy(&output.stdout);
        assert!(report.contains("Style/Dir)") && report.contains("Style/DirEmpty)"));
        assert_eq!(fs::read_to_string(&script).unwrap(), listed);
        assert_eq!(fs::metadata(&script).unwrap().modified().unwrap(), modified);
        let source = format!("{binding}\np Dir.children('.').empty?\n");
        fs::write(&script, &source).unwrap();
        let run = || {
            Command::new("ruby")
                .current_dir(dir.path())
                .arg(&script)
                .output()
                .unwrap()
        };
        let before = run();
        assert!(before.status.success(), "{source}: {before:?}");
        cli(
            dir.path(),
            &["--only", "Style/Dir,Style/DirEmpty", "--fix", "a.rb"],
            0,
        );
        let after = run();
        assert_eq!(before.status.code(), after.status.code(), "{source}");
        assert_eq!(before.stdout, after.stdout, "{source}");
        assert_eq!(before.stderr, after.stderr, "{source}");
        assert_eq!(
            fs::read_to_string(&script).unwrap(),
            source.replace("Dir.children('.').empty?", "Dir.empty?('.')")
        );
        let modified = fs::metadata(&script).unwrap().modified().unwrap();
        cli(
            dir.path(),
            &["--only", "Style/Dir,Style/DirEmpty", "--fix", "a.rb"],
            0,
        );
        assert_eq!(fs::metadata(&script).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn struct_and_data_readers_preserve_runtime_bindings_and_unrelated_fixes() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let script = dir.path().join("a.rb");
    for constructor in [
        "Struct.new({member})",
        "::Struct.new({member})",
        "Struct.new('Named', {member})",
        "Data.define({member})",
        "::Data.define({member})",
    ] {
        // Every member holds: the reader names `__dir__`, and the listed
        // `__FILE__` withholds the `:other` edit too.
        for (member, withheld) in [
            (":__dir__", true),
            (":\"__di\\x72__\"", true),
            (":other", true),
        ] {
            let constructor = constructor.replace("{member}", member);
            let source = format!(
                "C = {constructor} {{ def run; File.dirname(File.realpath(__FILE__)); end }}\np C.new(123).run\n"
            );
            assert_custom_constructor_withholds(dir.path(), &script, &source, "Style/Dir");
            let source = source.replace("C.new(123)", "C[123]");
            fs::write(&script, &source).unwrap();
            let run = || {
                Command::new("ruby")
                    .current_dir(dir.path())
                    .arg(&script)
                    .output()
                    .unwrap()
            };
            let before = run();
            assert!(before.status.success(), "{source}: {before:?}");
            assert_eq!(
                String::from_utf8_lossy(&before.stdout).trim(),
                format!("{:?}", dir.path().to_str().unwrap())
            );
            let modified = fs::metadata(&script).unwrap().modified().unwrap();
            let output = cli(
                dir.path(),
                &["--only", "Style/Dir", "--fix", "a.rb"],
                i32::from(withheld),
            );
            let report = String::from_utf8_lossy(&output.stdout);
            assert!(
                report.contains(if withheld {
                    "Style/Dir"
                } else {
                    "1 correction"
                }),
                "{report}"
            );
            let after = run();
            assert_eq!(before.status.code(), after.status.code(), "{source}");
            assert_eq!(before.stdout, after.stdout, "{source}");
            assert_eq!(before.stderr, after.stderr, "{source}");
            if withheld {
                assert_eq!(fs::read_to_string(&script).unwrap(), source);
                assert_eq!(fs::metadata(&script).unwrap().modified().unwrap(), modified);
            } else {
                assert_eq!(
                    fs::read_to_string(&script).unwrap(),
                    source.replace("File.dirname(File.realpath(__FILE__))", "__dir__")
                );
            }
            let modified = fs::metadata(&script).unwrap().modified().unwrap();
            cli(
                dir.path(),
                &["--only", "Style/Dir", "--fix", "a.rb"],
                i32::from(withheld),
            );
            assert_eq!(fs::metadata(&script).unwrap().modified().unwrap(), modified);
        }
    }
}

#[cfg(unix)]
#[test]
fn grouped_aliases_withhold_load_sensitive_edits_even_when_an_alias_is_excluded() {
    use std::os::unix::fs::symlink;
    let dir = project();
    let real = dir.path().join("lib/a.rb");
    symlink("lib/a.rb", dir.path().join("alias.rb")).unwrap();
    let source = "p File.expand_path(File.dirname(__FILE__))\n";
    for excluded in ["", "lib/a.rb", "alias.rb"] {
        fs::write(&real, source).unwrap();
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"Style/Dir\"]\nexclude = [\"{excluded}\"]\n"),
        )
        .unwrap();
        for paths in [["alias.rb", "lib/a.rb"], ["lib/a.rb", "alias.rb"]] {
            let modified = fs::metadata(&real).unwrap().modified().unwrap();
            cli(
                dir.path(),
                &["--only", "Style/Dir", "--fix", paths[0], paths[1]],
                1,
            );
            assert_eq!(fs::read_to_string(&real).unwrap(), source);
            assert_eq!(fs::metadata(&real).unwrap().modified().unwrap(), modified);
        }
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    // The listed `__FILE__` withholds the realpath edit.
    fs::write(&real, "p File.dirname(File.realpath(__FILE__))\n").unwrap();
    cli(
        dir.path(),
        &["--only", "Style/Dir", "--fix", "alias.rb", "lib/a.rb"],
        1,
    );
    assert_eq!(
        fs::read_to_string(&real).unwrap(),
        "p File.dirname(File.realpath(__FILE__))\n"
    );
}

#[test]
fn controlled_runtime_preserves_paths_evaluation_counts_binding_and_control_flow() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let script = dir.path().join("a.rb");
    fs::create_dir(dir.path().join("empty")).unwrap();
    fs::create_dir(dir.path().join("full")).unwrap();
    fs::write(dir.path().join("full/child"), "x").unwrap();
    fs::write(dir.path().join("zero"), "").unwrap();
    let source = r#"require "json"
$calls = []
def path(value)
  $calls << value
  value
end
class PathValue
  def initialize(value)
    @value = value
  end
  def to_path
    $calls << "coerce:#{@value}"
    @value
  end
end
values = []
["empty", "full"].each do |name|
  values << (Dir.entries(path(name)).size == 2)
  values << (Dir.entries(path(name)).size != 2)
  values << (Dir.children(path(name)).size > 0)
  values << Dir.each_child(path(name)).none?
  values << Dir.children(path(name)).size.>(0).to_s
  values << (Dir.children(path(name)).empty? ? :empty : :full)
  values << Dir.children(PathValue.new(name)).empty?
  Dir.chdir(name) do
    values << Dir.children(".").size.>(0).to_s
    values << (Dir.children(".").empty? ? :empty : :full)
  end
end
values << Dir.children("em\x70ty").empty?
values << File.expand_path(File.dirname(__FILE__))
values << File.dirname(File.realpath(__FILE__))
values << (File.size("zero") == 0)
values << (begin File.size("missing") == 0; rescue Errno::ENOENT; :enoent; end)
values << File.empty?("missing")
puts JSON.generate([values, $calls, __LINE__])
"#;
    assert_custom_constructor_withholds(
        dir.path(),
        &script,
        source,
        "Style/Dir,Style/DirEmpty,Style/FileEmpty",
    );
    let allocated = source.replace(
        "PathValue.new(name)",
        "PathValue.allocate.tap { |value| value.send(:initialize, name) }",
    );
    // Explicit initialization is an uncertain IO acquisition too. Preserve
    // this original coercion fixture as a runtime no-write control, then use a
    // genuinely constructor-free sibling for the independent API corrections.
    assert_custom_constructor_withholds(
        dir.path(),
        &script,
        &allocated,
        "Style/Dir,Style/DirEmpty,Style/FileEmpty",
    );
    // The listed `__FILE__` lines would withhold every edit: the corrected sibling drops them and Style/Dir's targets.
    let source = source
        .replace("  values << Dir.children(PathValue.new(name)).empty?\n", "")
        .replace("values << File.expand_path(File.dirname(__FILE__))\n", "")
        .replace("values << File.dirname(File.realpath(__FILE__))\n", "");
    assert!(!source.contains("__FILE__"));
    fs::write(&script, &source).unwrap();
    let run = || {
        let output = Command::new("ruby")
            .current_dir(dir.path())
            .arg(&script)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        output.stdout
    };
    let before = run();
    cli(
        dir.path(),
        &[
            "--only",
            "Style/Dir,Style/DirEmpty,Style/FileEmpty",
            "--fix",
            "a.rb",
        ],
        1,
    );
    let fixed = fs::read_to_string(&script).unwrap();
    assert!(fixed.contains("(!Dir.empty?(\".\")).to_s"));
    assert!(!fixed.contains("PathValue.new(name)"));
    assert!(fixed.contains("File.size(\"missing\") == 0"));
    assert_eq!(before, run());
    let observed = String::from_utf8(before).unwrap();
    assert!(observed.contains("enoent"));
    assert!(observed.contains("false"));
    assert_eq!(
        fixed.matches("path(name)").count(),
        6,
        "each expression evaluates its argument once"
    );
    assert_eq!(source.lines().count(), fixed.lines().count());
}
