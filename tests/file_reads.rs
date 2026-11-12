//! controlled Ruby semantics and real CLI correction boundaries.
use std::{fs, path::Path, process::Command};

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    fs::write(dir.path().join("file"), b"a\r\nb\0\xff\xc3\xa9").unwrap();
    dir
}
fn fix(dir: &Path, only: &str, status: i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(["--only", only, "--fix", "a.rb"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{out:?}");
}
fn ruby(dir: &Path) -> std::process::Output {
    let out = Command::new("ruby")
        .current_dir(dir)
        .arg("a.rb")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    out
}
fn has_ruby() -> bool {
    Command::new("ruby").arg("--version").output().is_ok()
}

#[test]
fn canonical_reads_preserve_decoded_bytes_encodings_and_block_local_binding() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    for mode in ["", "r", "rt", "rb"] {
        let mode_arg = if mode.is_empty() {
            String::new()
        } else {
            format!(", '{mode}'")
        };
        for form in ["chain", "proc", "brace", "do"] {
            let open = format!("::File.open(\"fi\\x6ce\"{mode_arg}");
            let expression = match form {
                "chain" => format!("{open}).read"),
                "proc" => format!("{open}, &:read)"),
                "brace" => format!("{open}) {{ |f| f.read }}"),
                _ => format!("{open}) do |f| f.read end"),
            };
            let source = format!(
                "f = :outer; value = {expression}; p [value.bytes, value.encoding.name, value.valid_encoding?, f, __LINE__]\n"
            );
            fs::write(dir.path().join("a.rb"), &source).unwrap();
            let before = ruby(dir.path());
            let expected_bytes = if mode == "rt" {
                "[[97, 10, 98, 0, 255, 195, 169]"
            } else {
                "[[97, 13, 10, 98, 0, 255, 195, 169]"
            };
            assert!(
                String::from_utf8_lossy(&before.stdout).starts_with(expected_bytes),
                "{source}: {before:?}"
            );
            let status = i32::from(mode == "rt");
            fix(dir.path(), "Style/FileRead", status);
            let method = if mode == "rb" { "binread" } else { "read" };
            let expected = if mode == "rt" {
                source.clone()
            } else {
                source.replace(&expression, &format!("::File.{method}(\"fi\\x6ce\")"))
            };
            assert_eq!(
                fs::read_to_string(dir.path().join("a.rb")).unwrap(),
                expected
            );
            let after = ruby(dir.path());
            assert_eq!(before.stdout, after.stdout, "{source}");
            assert_eq!(before.stderr, after.stderr, "{source}");
            let modified = fs::metadata(dir.path().join("a.rb"))
                .unwrap()
                .modified()
                .unwrap();
            fix(dir.path(), "Style/FileRead", status);
            assert_eq!(
                fs::metadata(dir.path().join("a.rb"))
                    .unwrap()
                    .modified()
                    .unwrap(),
                modified
            );
        }
    }
    // On Linux, both canonical block forms and direct read release the
    // descriptor immediately. Chained opens intentionally improve that leak.
    if Path::new("/proc/self/fd").is_dir() {
        let source = "before = Dir.children('/proc/self/fd').size\n100.times { File.open('file') { |f| f.read } }\np(Dir.children('/proc/self/fd').size == before)\n";
        fs::write(dir.path().join("a.rb"), source).unwrap();
        assert_eq!(ruby(dir.path()).stdout, b"true\n");
        fix(dir.path(), "Style/FileRead", 0);
        assert_eq!(ruby(dir.path()).stdout, b"true\n");
    }
}

#[test]
fn uncertain_rewrites_retain_runtime_observations_offenses_and_file_bytes() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    for source in [
        "fd = IO.sysopen('file'); value = File.open(fd).read; p value.bytes\n",
        "p File.open('file', 'r+').read.bytes\n",
        "p File.open('file', 'r+b', &:read).bytes\n",
        "$calls = []; def path; $calls << :path; 'file'; end; p File.open(path).read.bytes; p $calls\n",
        "def helper; yield; end; p File.open(helper { break 'file' }).read.bytes\n",
        "File.define_singleton_method(:read) { |*args| 'changed' }; p File.open('file').read.bytes\n",
        "File.class_eval('def self.read(path); 123; end'); p File.open('file').read.bytes\n",
        "IO.class_eval('def read; gets; end'); p File.open('file').read.bytes\n",
        "Symbol.class_eval(\"def to_proc; proc { |f| f.read.b + 'changed' }; end\"); p File.open('file', &:read).bytes\n",
        "File.define_singleton_method(:open) { |*args| StringIO.new('changed') }; require 'stringio'; p File.open('file').read\n",
        "File.singleton_class.class_eval { alias read basename }; p File.open('file').read.bytes\n",
        "File.instance_eval { Dir.class_exec(private_class_method(:read)) {} }; p File.open('file').read.bytes\n",
        "class IO; alias read gets; end; p File.open('file').read.bytes\n",
        "class Symbol; def to_proc; proc { |f| f.read.b + 'changed' }; end; end; p File.open('file', &:read).bytes\n",
        "C = Struct.new(:read); p File.open('file').read.bytes\n",
        "C = Data.define(:binread); p File.open('file', 'rb').read.bytes\n",
        "class C; attr_reader :read; end; p File.open('file').read.bytes\n",
        "p File.open('file') do |f| f.read end\n", // block belongs to p: no FileRead offense
    ] {
        fs::write(dir.path().join("a.rb"), source).unwrap();
        let before = ruby(dir.path());
        let modified = fs::metadata(dir.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap();
        let status = i32::from(!source.starts_with("p File.open('file') do"));
        fix(dir.path(), "Style/FileRead", status);
        assert_eq!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), source);
        assert_eq!(
            fs::metadata(dir.path().join("a.rb"))
                .unwrap()
                .modified()
                .unwrap(),
            modified
        );
        let after = ruby(dir.path());
        assert_eq!(before.stdout, after.stdout, "{source}");
        assert_eq!(before.stderr, after.stderr, "{source}");
    }
}

#[test]
fn bom_crlf_data_and_line_values_survive_and_legacy_bytes_are_not_lossily_fixed() {
    let dir = project();
    let listed = "\u{feff}# encoding: UTF-8\r\np File.open(\"fi\\x6ce\", 'rb', &:read).bytes # keep\r\np __LINE__\r\np DATA.read.bytes\r\n__END__\r\npayload\0\r\n";
    // Reading `DATA` is a listed observer: that file keeps its
    // finding and bytes. The unread data section survives below.
    fs::write(dir.path().join("a.rb"), listed).unwrap();
    fix(dir.path(), "Style/FileRead", 1);
    assert_eq!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), listed);
    let source = &listed.replace("p DATA.read.bytes\r\n", "p :data\r\n");
    fs::write(dir.path().join("a.rb"), source).unwrap();
    let before = has_ruby().then(|| ruby(dir.path()));
    fix(dir.path(), "Style/FileRead", 0);
    assert_eq!(
        fs::read_to_string(dir.path().join("a.rb")).unwrap(),
        source.replace("open(\"fi\\x6ce\", 'rb', &:read)", "binread(\"fi\\x6ce\")")
    );
    if let Some(before) = before {
        assert_eq!(before.stdout, ruby(dir.path()).stdout);
    }
    for source in [
        b"# encoding: ISO-8859-1\r\nFile.open(\"caf\xe9\").read\r\n__END__\r\nraw\xff\r\n"
            .as_slice(),
        b"File.open('file') do |f|\r\n f.read\r\nend\r\np __LINE__\r\n__END__\r\nraw\r\n",
    ] {
        fs::write(dir.path().join("a.rb"), source).unwrap();
        fix(dir.path(), "Style/FileRead", 1);
        assert_eq!(fs::read(dir.path().join("a.rb")).unwrap(), source);
    }
}

#[test]
fn disabled_multiline_spans_never_apply_a_partial_read_rewrite() {
    let dir = project();
    for prefix in ["rustocop", "rubocop"] {
        for source in [
            format!("File.open('file') do |f|\n f.read # {prefix}:disable Style/FileRead\nend\n"),
            format!("File.open('file') do |f|\n # {prefix}:disable Style/FileRead\n f.read\nend\n"),
            format!("File.open(\n 'file' # {prefix}:disable Style/FileRead\n).read\n"),
        ] {
            fs::write(dir.path().join("a.rb"), &source).unwrap();
            fix(dir.path(), "Style/FileRead", 0);
            assert_eq!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), source);
        }
    }
}

#[test]
fn mixed_read_string_parentheses_return_semicolon_and_layout_edits_converge() {
    let dir = project();
    fs::write(
        dir.path().join("a.rb"),
        "def f\n  return (File.open('file', 'rb', &:read));  \nend\n",
    )
    .unwrap();
    let only = "Style/FileRead,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Layout";
    for _ in 0..4 {
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", only, "--fix", "a.rb"])
            .output()
            .unwrap();
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(
        fs::read_to_string(dir.path().join("a.rb")).unwrap(),
        "def f\n  File.binread(\"file\")\nend\n"
    );
    fix(dir.path(), only, 0);
}
