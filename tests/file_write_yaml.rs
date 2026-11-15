//! only controlled temporary Ruby/file/YAML fixtures are executed.
use std::{fs, path::Path, process::Command};

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn fix(dir: &Path, cop: &str, status: i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(["--only", cop, "--fix", "a.rb"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(status), "{output:?}");
}
fn ruby(dir: &Path) -> std::process::Output {
    let output = Command::new("ruby")
        .current_dir(dir)
        .arg("a.rb")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    output
}
fn has_ruby() -> bool {
    Command::new("ruby").arg("--version").output().is_ok()
}
fn unchanged(dir: &Path, cop: &str, source: &[u8], status: i32) {
    fs::write(dir.join("a.rb"), source).unwrap();
    let mtime = fs::metadata(dir.join("a.rb")).unwrap().modified().unwrap();
    fix(dir, cop, status);
    assert_eq!(fs::read(dir.join("a.rb")).unwrap(), source);
    assert_eq!(
        fs::metadata(dir.join("a.rb")).unwrap().modified().unwrap(),
        mtime
    );
}

#[test]
fn writes_preserve_truncation_bytes_encodings_return_counts_and_outer_binding() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    for mode in ["w", "wb"] {
        for form in ["chain", "brace", "do"] {
            for content in ["'hello'", "\"é\\r\\n\\0\\xff\"", "content"] {
                let expression = match form {
                    "chain" => format!("::File.open(\"fi\\x6ce\", '{mode}').write({content})"),
                    "brace" => {
                        format!("::File.open(\"fi\\x6ce\", '{mode}') {{ |f| f.write({content}) }}")
                    }
                    _ => {
                        format!("::File.open(\"fi\\x6ce\", '{mode}') do |f| f.write({content}) end")
                    }
                };
                let source = format!(
                    "f = :outer; content = \"é\\r\\n\\0\\xff\"; value = {expression}; p [value, f, File.binread('file').bytes, File.read('file').encoding.name, __LINE__]\n"
                );
                fs::write(dir.path().join("a.rb"), &source).unwrap();
                fs::write(dir.path().join("file"), "long old contents to truncate").unwrap();
                let before = ruby(dir.path());
                let bytes = fs::read(dir.path().join("file")).unwrap();
                assert!(bytes.len() < 28);
                assert!(String::from_utf8_lossy(&before.stdout).contains(":outer"));
                let method = if mode == "wb" { "binwrite" } else { "write" };
                let status = i32::from(form == "chain");
                fix(dir.path(), "Style/FileWrite", status);
                let expected = if form == "chain" {
                    source.clone()
                } else {
                    source.replace(
                        &expression,
                        &format!("::File.{method}(\"fi\\x6ce\", {content})"),
                    )
                };
                assert_eq!(
                    fs::read_to_string(dir.path().join("a.rb")).unwrap(),
                    expected
                );
                fs::write(dir.path().join("file"), "long old contents to truncate").unwrap();
                let after = ruby(dir.path());
                assert_eq!(before.stdout, after.stdout, "{source}");
                assert_eq!(before.stderr, after.stderr, "{source}");
                assert_eq!(fs::read(dir.path().join("file")).unwrap(), bytes);
                let mtime = fs::metadata(dir.path().join("a.rb"))
                    .unwrap()
                    .modified()
                    .unwrap();
                fix(dir.path(), "Style/FileWrite", status);
                assert_eq!(
                    fs::metadata(dir.path().join("a.rb"))
                        .unwrap()
                        .modified()
                        .unwrap(),
                    mtime
                );
            }
        }
    }
    if Path::new("/proc/self/fd").is_dir() {
        let source = "before = Dir.children('/proc/self/fd').size\n100.times { File.open('file', 'w') { |f| f.write('x') } }\np(Dir.children('/proc/self/fd').size == before)\n";
        fs::write(dir.path().join("a.rb"), source).unwrap();
        assert_eq!(ruby(dir.path()).stdout, b"true\n");
        fix(dir.path(), "Style/FileWrite", 0);
        assert_eq!(ruby(dir.path()).stdout, b"true\n");
    }
}

#[test]
fn uncertain_writes_preserve_order_control_flow_resource_modes_and_api_mutations() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    for source in [
        "File.open('file', 'w') { |f| f.write(File.read('file')) }; p File.binread('file')\n",
        "$events = []; def content; $events << File.size('file'); 'new'; end; File.open('file', 'w') { |f| f.write(content) }; p $events\n",
        "def content; raise 'stop'; end; begin; File.open('file', 'w').write(content); rescue; end; p File.size('file')\n",
        "def helper; yield; end; p File.open('file', 'w') { |f| f.write(helper { break 'new' }) }\n",
        "File.open('file', 'w') { |f| f.write(f.path) }; p File.binread('file')\n",
        "fd = IO.sysopen('file', 'w'); p File.open(fd, 'w').write('data')\n",
        "p File.open('file', 'w+').write('data')\n",
        "p File.open('file', 'wt').write(\"a\\r\\nb\")\n",
        "File.define_singleton_method(:write) { |*args| 123 }; p File.open('file', 'w').write('data')\n",
        "IO.class_eval('def write(*args); 123; end'); p File.open('file', 'w').write('data')\n",
        "File.singleton_class.class_eval { alias binwrite basename }; p File.open('file', 'wb').write('data')\n",
        "p File.open('file', 'w', &:write).write('data') rescue p $!.class.name\n",
        "def run; File.open('file', 'w') { |f| return f.write('data') }; end; p run\n",
        "def run; File.open('file', 'w') { |f| break f.write('data') }; end; p run\n",
    ] {
        fs::write(dir.path().join("file"), "old contents").unwrap();
        fs::write(dir.path().join("a.rb"), source).unwrap();
        let before = ruby(dir.path());
        let bytes = fs::read(dir.path().join("file")).unwrap();
        // Return/break bodies deliberately are not canonical single-write bodies.
        let status = i32::from(!source.starts_with("def run;"));
        unchanged(dir.path(), "Style/FileWrite", source.as_bytes(), status);
        fs::write(dir.path().join("file"), "old contents").unwrap();
        let after = ruby(dir.path());
        assert_eq!(before.stdout, after.stdout, "{source}");
        assert_eq!(before.stderr, after.stderr, "{source}");
        assert_eq!(fs::read(dir.path().join("file")).unwrap(), bytes);
    }
}

#[test]
fn touch_never_changes_timestamps_contents_or_return_values() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let source = "p File.open('file', 'a') {}; p File.binread('file'); p File.mtime('file').to_i\n";
    fs::write(dir.path().join("file"), "old").unwrap();
    fs::write(dir.path().join("a.rb"), source).unwrap();
    let before = ruby(dir.path());
    let mtime = fs::metadata(dir.path().join("file"))
        .unwrap()
        .modified()
        .unwrap();
    unchanged(dir.path(), "Style/FileTouch", source.as_bytes(), 1);
    assert_eq!(before.stdout, ruby(dir.path()).stdout);
    assert_eq!(
        fs::metadata(dir.path().join("file"))
            .unwrap()
            .modified()
            .unwrap(),
        mtime
    );
}

#[test]
fn yaml_null_corrections_preserve_defaults_keywords_and_api_binding() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    for method in ["load", "safe_load", "parse"] {
        for options in [
            "",
            ", fallback: 'empty'",
            ", filename: 'custom', aliases: true",
        ] {
            if method == "parse" && !options.is_empty() {
                continue;
            }
            let source =
                format!("require 'yaml'\np YAML.{method}(File.read(File::NULL){options})\n");
            fs::write(dir.path().join("a.rb"), &source).unwrap();
            let before = ruby(dir.path());
            if options.is_empty() {
                assert_eq!(
                    before.stdout,
                    if method == "parse" {
                        b"false\n".as_slice()
                    } else {
                        b"nil\n"
                    }
                );
            }
            fix(dir.path(), "Style/YAMLFileRead", 0);
            assert_eq!(
                fs::read_to_string(dir.path().join("a.rb")).unwrap(),
                source.replace(
                    &format!("{method}(File.read(File::NULL){options})"),
                    &format!("{method}_file(File::NULL{options})")
                )
            );
            let after = ruby(dir.path());
            assert_eq!(before.stdout, after.stdout);
            assert_eq!(before.stderr, after.stderr);
        }
    }
    for binding in [
        "YAML.define_singleton_method(:load_file) { |*args| 123 }",
        "Psych.singleton_class.class_eval { alias load_file dump }",
        "YAML.instance_eval { def load_file(*args); 123; end }",
        "YAML.singleton_class.module_eval('def load_file(*args); 123; end')",
        "File.define_singleton_method(:read) { |*args| 'data' }",
        "File.const_set(:NULL, 'file.yml')",
        "Encoding.default_external = Encoding::UTF_16LE",
    ] {
        fs::write(dir.path().join("file.yml"), "hello: world\n").unwrap();
        let source = format!(
            "require 'yaml'\n{binding}\np(begin; YAML.load(File.read(File::NULL)); rescue Exception => e; e.class.name; end)\n"
        );
        fs::write(dir.path().join("a.rb"), &source).unwrap();
        let before = ruby(dir.path());
        unchanged(dir.path(), "Style/YAMLFileRead", source.as_bytes(), 1);
        let after = ruby(dir.path());
        assert_eq!(before.stdout, after.stdout, "{source}");
        assert_eq!(before.stderr, after.stderr, "{source}");
    }
}

#[test]
fn yaml_regular_paths_keep_encoding_bom_filename_exception_and_keyword_observations() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    for contents in [
        b"".as_slice(),
        b"hello: world\r\n",
        b"\xef\xbb\xbfhello: world\n",
        b"x: [",
        b"\xff\xfea\0:\0 \0b\0\n\0",
        b"!ruby/object:Object {}\n",
        b"a: &a [1]\nb: *a\n",
    ] {
        fs::write(dir.path().join("file.yml"), contents).unwrap();
        for method in ["load", "safe_load", "parse"] {
            for options in ["", ", filename: nil"] {
                let source = format!(
                    "require 'yaml'\nbegin\n value = YAML.{method}(File.read('file.yml'){options})\n value = value.to_ruby if value.respond_to?(:to_ruby)\n p Marshal.dump(value).bytes\nrescue Exception => e\n p [e.class.name, e.message, (e.file if e.respond_to?(:file))]\nend\n"
                );
                fs::write(dir.path().join("a.rb"), &source).unwrap();
                let before = ruby(dir.path());
                unchanged(dir.path(), "Style/YAMLFileRead", source.as_bytes(), 1);
                let after = ruby(dir.path());
                assert_eq!(before.stdout, after.stdout, "{contents:?}: {source}");
                assert_eq!(before.stderr, after.stderr);
                if contents == b"x: [" {
                    assert!(String::from_utf8_lossy(&before.stdout).contains("<unknown>"));
                }
            }
        }
    }
    // The keyword expression must still observe the file AFTER File.read;
    // helpers evaluate all arguments before opening it.
    fs::write(dir.path().join("file.yml"), "old").unwrap();
    let source = "require 'yaml'\ndef options; File.write('file.yml', 'new'); {}; end\np YAML.load(File.read('file.yml'), **options)\n";
    fs::write(dir.path().join("a.rb"), source).unwrap();
    let before = ruby(dir.path());
    assert_eq!(before.stdout, b"\"old\"\n");
    unchanged(dir.path(), "Style/YAMLFileRead", source.as_bytes(), 1);
    fs::write(dir.path().join("file.yml"), "old").unwrap();
    assert_eq!(before.stdout, ruby(dir.path()).stdout);
}

#[test]
fn ordinary_yaml_helper_rewrites_really_change_filename_metadata_and_bom_errors() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    for contents in [b"x: [".as_slice(), b"\xff\xfea\0:\0 \0b\0\n\0"] {
        fs::write(dir.path().join("file.yml"), contents).unwrap();
        let source = "require 'yaml'\nbegin\n YAML.load(File.read('file.yml'))\nrescue Exception => e\n p [e.class.name, e.message, (e.file if e.respond_to?(:file))]\nend\n";
        fs::write(dir.path().join("a.rb"), source).unwrap();
        let before = ruby(dir.path());
        assert!(String::from_utf8_lossy(&before.stdout).contains("Psych::SyntaxError"));
        unchanged(dir.path(), "Style/YAMLFileRead", source.as_bytes(), 1);
        assert_eq!(before.stdout, ruby(dir.path()).stdout);
        // Execute only this controlled synthetic alternate, not application YAML.
        fs::write(
            dir.path().join("a.rb"),
            source.replace("load(File.read('file.yml'))", "load_file('file.yml')"),
        )
        .unwrap();
        let naive = ruby(dir.path());
        assert_ne!(before.stdout, naive.stdout);
        if contents == b"x: [" {
            assert!(String::from_utf8_lossy(&before.stdout).contains("<unknown>"));
            assert!(String::from_utf8_lossy(&naive.stdout).contains("file.yml"));
        } else {
            assert!(String::from_utf8_lossy(&naive.stdout).contains("ArgumentError"));
        }
    }
}

#[test]
fn local_write_object_conversion_occurs_after_truncation_and_closes_on_error() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    for conversion in ["'data'", "raise 'stop'"] {
        let source = format!(
            "$events = []\ncontent = Object.new\ndef content.to_s; $events << File.binread('file').bytes; {conversion}; end\nbefore = Dir.children('/proc/self/fd').size if File.directory?('/proc/self/fd')\nbegin\n p File.open('file', 'w') {{ |f| f.write(content) }}\nrescue => e; p [e.class.name, e.message]; end\np $events\np(Dir.children('/proc/self/fd').size == before) if before\n"
        );
        fs::write(dir.path().join("a.rb"), &source).unwrap();
        fs::write(dir.path().join("file"), "old").unwrap();
        let before = ruby(dir.path());
        assert!(String::from_utf8_lossy(&before.stdout).contains("[[]]"));
        fix(dir.path(), "Style/FileWrite", 0);
        assert!(
            fs::read_to_string(dir.path().join("a.rb"))
                .unwrap()
                .contains("File.write('file', content)")
        );
        fs::write(dir.path().join("file"), "old").unwrap();
        let after = ruby(dir.path());
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
    }
}

#[test]
fn mixed_safe_cops_and_existing_style_layout_rules_stabilize_in_four_explicit_passes() {
    let dir = project();
    let source = "require 'yaml'\ndef write_file\n  return (File.open('file', 'wb') { |f| f.write('data') });  \nend\ndef load_yaml\n  return (YAML.load(File.read(File::NULL)));  \nend\n";
    fs::write(dir.path().join("a.rb"), source).unwrap();
    let only = "Style/FileWrite,Style/YAMLFileRead,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Layout";
    for _ in 0..4 {
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", only, "--fix", "a.rb"])
            .output()
            .unwrap();
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    let expected = "require \"yaml\"\ndef write_file\n  File.binwrite(\"file\", \"data\")\nend\n\ndef load_yaml\n  YAML.load_file(File::NULL)\nend\n";
    assert_eq!(
        fs::read_to_string(dir.path().join("a.rb")).unwrap(),
        expected
    );
    let mtime = fs::metadata(dir.path().join("a.rb"))
        .unwrap()
        .modified()
        .unwrap();
    fix(dir.path(), only, 0);
    assert_eq!(
        fs::metadata(dir.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap(),
        mtime
    );
}
