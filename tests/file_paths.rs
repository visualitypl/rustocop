//! controlled local path semantics, correction composition and no-write.
use crate::support::cli::project;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn fix(dir: &Path, cop: &str, path: &str, status: i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(["--only", cop, "--fix", path])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{out:?}");
}
fn ruby(dir: &Path, path: &str) -> Output {
    Command::new("ruby")
        .current_dir(dir)
        .arg(path)
        .output()
        .unwrap()
}
fn has_ruby() -> bool {
    Command::new("ruby").arg("--version").output().is_ok()
}
fn same(before: &Output, after: &Output, source: &str) {
    assert!(before.status.success(), "{source}: {before:?}");
    assert_eq!(
        before.status.code(),
        after.status.code(),
        "{source}: {after:?}"
    );
    assert_eq!(before.stdout, after.stdout, "{source}");
    assert_eq!(before.stderr, after.stderr, "{source}");
}

#[test]
fn nested_dirname_preserves_values_encodings_coercion_counts_and_control_flow() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let source = r#"paths = ["a/b/c", "/a/b/c", "//a/b/c", "///a/b", ".", "..", "", "a//b/../c", "a/é/c".encode("ISO-8859-1")]
paths.each do |path|
  value = File.dirname(File.dirname(File.dirname(path)))
  p [value.bytes, value.encoding.name]
end
class Path
  attr_reader :calls
  def initialize; @calls = 0; end
  def to_path; @calls += 1; "a/b/c"; end
end
path = Path.new
p File.dirname(File.dirname(path))
p path.calls
$count = 0
def make_path(value); $count += 1; value; end
p File.dirname(File.dirname(make_path "a/b/c"))
p $count
def run
  File.dirname(File.dirname(begin; return :early; "a"; end))
end
p run
p __LINE__
"#;
    fs::write(dir.path().join("a.rb"), source).unwrap();
    let before = ruby(dir.path(), "a.rb");
    fix(dir.path(), "Style/NestedFileDirname", "a.rb", 0);
    same(&before, &ruby(dir.path(), "a.rb"), source);
    let fixed = fs::read_to_string(dir.path().join("a.rb")).unwrap();
    assert!(fixed.contains("File.dirname(path, 3)"));
    assert!(fixed.contains("File.dirname((make_path \"a/b/c\"), 2)"));
}

#[test]
fn uncertain_runtime_cases_retain_offenses_bytes_and_mtime() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    for (cop, source) in [
        (
            "Style/NestedFileDirname",
            "p File.dirname(File.dirname('a/b/c/d', 2))\n",
        ),
        (
            "Style/NestedFileDirname",
            "File.define_singleton_method(:dirname) { |p| p + '/x' }; p File.dirname(File.dirname('a'))\n",
        ),
        (
            "Style/NestedFileDirname",
            "module Paths; refine File.singleton_class do; def dirname(path); path + '/x'; end; end; end; using Paths; p File.dirname(File.dirname('a'))\n",
        ),
        (
            "Style/ExpandPathArguments",
            "module Paths; refine File.singleton_class do; def expand_path(path, base); [path, base]; end; end; end; using Paths; p File.expand_path('..', __FILE__)\n",
        ),
        (
            "Style/ExpandPathArguments",
            "p File.expand_path('lib', __FILE__)\n",
        ),
        (
            "Style/ExpandPathArguments",
            "__dir__ = '/wrong'; p File.expand_path('..', __FILE__)\n",
        ),
        (
            "Style/ExpandPathArguments",
            "require 'pathname'; def Pathname(path); ::Pathname.new(path).join('extra'); end; p Pathname(__FILE__).parent.expand_path\n",
        ),
        (
            "Style/RedundantCurrentDirectoryInPath",
            "def require_relative(path); p path; end; require_relative './feature'\n",
        ),
        (
            "Style/RedundantCurrentDirectoryInPath",
            "path = '/missing'; begin; require_relative \"./#{path}\"; rescue LoadError => e; p e.path; end\n",
        ),
    ] {
        let file = dir.path().join("a.rb");
        fs::write(&file, source).unwrap();
        let modified = fs::metadata(&file).unwrap().modified().unwrap();
        let before = ruby(dir.path(), "a.rb");
        fix(dir.path(), cop, "a.rb", 1);
        same(&before, &ruby(dir.path(), "a.rb"), source);
        assert_eq!(fs::read_to_string(&file).unwrap(), source);
        assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn legacy_encoding_clean_error_and_disabled_files_do_not_write() {
    let dir = project();
    for (cop, source, status) in [
        ("Style/NestedFileDirname", b"# encoding: ISO-8859-1\r\nFile.dirname(File.dirname('caf\xe9/a'))\r\n__END__\r\nraw\xff\r\n".as_slice(), 1),
        ("Style/ExpandPathArguments", b"# encoding: ISO-8859-1\r\nFile.expand_path('../caf\xe9', __FILE__)\r\n".as_slice(), 1),
        ("Style/ExpandPathArguments", b"# encoding: ISO-8859-1\nFile.expand_path(\"../\\u00e9\", __FILE__)\n".as_slice(), 1),
        ("Style/RedundantCurrentDirectoryInPath", b"# encoding: ISO-8859-1\r\nrequire_relative './caf\xe9'\r\n".as_slice(), 1),
        ("Style/NestedFileDirname", b"File.dirname(path, 2)\n".as_slice(), 0),
        ("Style/ExpandPathArguments", b"File.expand_path(\n".as_slice(), 2),
        ("Style/RedundantCurrentDirectoryInPath", b"require_relative('./feature') # rustocop:disable Style/RedundantCurrentDirectoryInPath\n".as_slice(), 0),
    ] {
        let path = dir.path().join("a.rb");
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fix(dir.path(), cop, "a.rb", status);
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn directives_outside_selector_highlights_still_protect_whole_expression_edits() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for (cop, source) in [
        (
            "Style/ExpandPathArguments",
            "File.expand_path(\n '..', # rubocop:disable Style/ExpandPathArguments\n __FILE__)\n",
        ),
        (
            "Style/RedundantCurrentDirectoryInPath",
            "require_relative \"./dir/#{\n name # rustocop:disable Style/RedundantCurrentDirectoryInPath\n}\"\n",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fix(dir.path(), cop, "a.rb", 1);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
