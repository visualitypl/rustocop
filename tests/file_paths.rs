//! controlled local path semantics, correction composition and no-write.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
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
fn expand_paths_preserve_decoded_values_and_pathname_constructors() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let source = r#"require "pathname"
p [File.expand_path('..', __FILE__), File.expand_path('../..', __FILE__), File.expand_path('.', __FILE__)]
p [File.expand_path('./../../lib', __FILE__), File.expand_path("..\x2f../lib", __FILE__), File.expand_path(%q(../../a'b), __FILE__)]
p [File.expand_path('../~', __FILE__), File.expand_path('../~/lib', __FILE__), File.expand_path('../~root', __FILE__), File.expand_path('./..//./~rustocop_nonexistent_user/lib', __FILE__)]
p [Pathname(__FILE__).parent.expand_path, Pathname.new(__FILE__).parent.expand_path, ::Pathname.new(__FILE__).parent.expand_path].map(&:to_s)
p __LINE__
"#;
    // A dynamic load plus a Symbol#to_proc getter no longer proves unchanged
    // core dispatch. Every target names `__FILE__`, a listed source
    // observer, so both variants keep their findings and are never
    // written.
    for reflective in [true, false] {
        let source = if reflective {
            source.to_owned()
        } else {
            source.replace("map(&:to_s)", "map { |path| path.to_s }")
        };
        let path = dir.path().join("a.rb");
        fs::write(&path, &source).unwrap();
        for script in ["a.rb", path.to_str().unwrap()] {
            let before = ruby(dir.path(), script);
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            fix(dir.path(), "Style/ExpandPathArguments", "a.rb", 1);
            same(&before, &ruby(dir.path(), script), &source);
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            fs::write(&path, &source).unwrap();
        }
    }
}

#[test]
fn require_relative_preserves_feature_identity_interpolation_effects_and_source_data() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    fs::create_dir(dir.path().join("features")).unwrap();
    fs::write(
        dir.path().join("features/feature.rb"),
        "$loaded = ($loaded || 0) + 1\n",
    )
    .unwrap();
    // Reading `DATA` is a listed observer that withholds every
    // edit: that variant keeps its findings and bytes.
    let listed = "\u{feff}# encoding: UTF-8\r\n$count = 0\r\ndef feature; $count += 1; 'feature'; end\r\np require_relative(\".///./features/#{feature}\")\r\np require_relative('./features/feature')\r\np [$count, $loaded, __LINE__, DATA.read.bytes]\r\n__END__\r\nraw\r\n";
    fs::write(dir.path().join("a.rb"), listed).unwrap();
    let before = ruby(dir.path(), "a.rb");
    fix(
        dir.path(),
        "Style/RedundantCurrentDirectoryInPath",
        "a.rb",
        1,
    );
    same(&before, &ruby(dir.path(), "a.rb"), listed);
    assert_eq!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), listed);
    let source = "\u{feff}# encoding: UTF-8\r\n$count = 0\r\ndef feature; $count += 1; 'feature'; end\r\np require_relative(\".///./features/#{feature}\")\r\np require_relative('./features/feature')\r\np [$count, $loaded, __LINE__]\r\n__END__\r\nraw\r\n";
    fs::write(dir.path().join("a.rb"), source).unwrap();
    let before = ruby(dir.path(), "a.rb");
    fix(
        dir.path(),
        "Style/RedundantCurrentDirectoryInPath",
        "a.rb",
        0,
    );
    same(&before, &ruby(dir.path(), "a.rb"), source);
    let fixed = fs::read_to_string(dir.path().join("a.rb")).unwrap();
    assert_eq!(
        fixed,
        source
            .replace(".///./features/", "features/")
            .replace("./features/", "features/")
    );
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

#[cfg(unix)]
#[test]
fn symlink_provenance_and_the_listed_file_observer_withhold_expand_path_edits() {
    use std::os::unix::fs::symlink;
    let dir = project();
    fs::create_dir(dir.path().join("real")).unwrap();
    symlink("real/a.rb", dir.path().join("alias.rb")).unwrap();
    symlink("real", dir.path().join("aliasdir")).unwrap();
    for alias in ["alias.rb", "aliasdir/a.rb"] {
        for source in [
            "p File.expand_path('..', __FILE__)\n",
            "require 'pathname'; p Pathname(__FILE__).parent.expand_path\n",
            "require 'pathname'; p Pathname.new(__FILE__).parent.expand_path\n",
        ] {
            fs::write(dir.path().join("real/a.rb"), source).unwrap();
            let before = has_ruby().then(|| ruby(dir.path(), alias));
            let modified = fs::metadata(dir.path().join("real/a.rb"))
                .unwrap()
                .modified()
                .unwrap();
            fix(dir.path(), "Style/ExpandPathArguments", alias, 1);
            assert_eq!(
                fs::read_to_string(dir.path().join("real/a.rb")).unwrap(),
                source
            );
            assert_eq!(
                fs::metadata(dir.path().join("real/a.rb"))
                    .unwrap()
                    .modified()
                    .unwrap(),
                modified
            );
            if let Some(before) = before {
                same(&before, &ruby(dir.path(), alias), source);
            }
        }
        // The lexical edit is safe through the alias, but the listed
        // `__FILE__` withholds it.
        fs::write(
            dir.path().join("real/a.rb"),
            "p File.expand_path('.', __FILE__)\n",
        )
        .unwrap();
        fix(dir.path(), "Style/ExpandPathArguments", alias, 1);
        assert_eq!(
            fs::read_to_string(dir.path().join("real/a.rb")).unwrap(),
            "p File.expand_path('.', __FILE__)\n"
        );
    }
    assert!(
        fs::symlink_metadata(dir.path().join("alias.rb"))
            .unwrap()
            .is_symlink()
    );
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

#[test]
fn mixed_path_style_layout_fixes_stabilize_within_four_explicit_passes() {
    let dir = project();
    let path = dir.path().join("a.rb");
    let listed = "def f\n  return (File.dirname(File.dirname('a/b/c')));  \nend\nFile.expand_path('../..', __FILE__)\nrequire_relative './feature'\n";
    let cops = "Style/NestedFileDirname,Style/ExpandPathArguments,Style/RedundantCurrentDirectoryInPath,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Layout";
    // A file naming `__FILE__`, a listed observer,
    // keeps every finding and is never written.
    fs::write(&path, listed).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fix(dir.path(), cops, "a.rb", 1);
    assert_eq!(fs::read_to_string(&path).unwrap(), listed);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    // Without it the mixed fixes converge within four explicit passes.
    fs::write(
        &path,
        listed.replace("File.expand_path('../..', __FILE__)\n", ""),
    )
    .unwrap();
    let mut stable = false;
    for _ in 0..4 {
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", cops, "--fix", "a.rb"])
            .output()
            .unwrap();
        assert_ne!(out.status.code(), Some(2), "{out:?}");
        if out.status.code() == Some(0)
            && String::from_utf8_lossy(&out.stdout).contains("0 corrections applied")
        {
            stable = true;
            break;
        }
    }
    assert!(stable, "{}", fs::read_to_string(&path).unwrap());
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "def f\n  File.dirname(\"a/b/c\", 2)\nend\nrequire_relative \"feature\"\n"
    );
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fix(dir.path(), cops, "a.rb", 0);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
