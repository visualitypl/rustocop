//! real CLI contracts and controlled, local filesystem semantics.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
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
