//! Controlled synthetic main scripts only; no corpus input is ever executed.
use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, SystemTime},
};

fn ruby(path: &Path) -> Vec<Vec<u8>> {
    std::iter::once("ruby".to_string())
        .chain(crate::support::runtime::ruby())
        .map(|runtime| {
            let mut command = Command::new(&runtime);
            if runtime != "ruby" {
                // Native miniruby has no installed stdlib load path. Use its
                // own source-tree library, not the host Ruby's FileUtils.
                command
                    .arg("-I")
                    .arg(Path::new(&runtime).parent().unwrap().join("lib"));
            }
            let output = command
                .current_dir(path.parent().unwrap())
                .args(["--disable-gems", "-W0"])
                .arg(path)
                .output()
                .unwrap();
            assert!(output.status.success(), "{runtime}: {output:?}");
            assert!(output.stderr.is_empty(), "{runtime}: {output:?}");
            output.stdout
        })
        .collect()
}

fn fix(dir: &Path, cop: &str, status: i32) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(["--only", cop, "--fix", "."])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(status), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

/// The closed list, as it appears in these fixtures.
fn listed_observer(source: &str) -> bool {
    [
        "__FILE__",
        "__dir__",
        "__END__",
        "caller",
        "source_location",
        "$0",
        "$PROGRAM_NAME",
    ]
    .iter()
    .any(|name| source.contains(name))
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}

fn witness(cop: &str, prefix: &str, canonical: &str, observers: &[&str]) {
    eprintln!(
        "Metadata witness batch: {} source/envelope fixtures",
        observers.len() * 2
    );
    let runtime = Command::new("ruby").arg("--version").output().is_ok();
    for observer in observers {
        for crlf in [false, true] {
            let dir = project();
            let path = dir.path().join("case.rb");
            let sibling = dir.path().join("safe.rb");
            let encode = |s: String| {
                if crlf {
                    format!("\u{feff}{}", s.replace('\n', "\r\n"))
                } else {
                    s
                }
            };
            let source = encode(format!("{prefix}{observer}\n"));
            let naive = encode(format!("{canonical}{observer}\n"));
            fs::write(&path, &source).unwrap();
            // Make timestamp witnesses independent of host timestamp resolution.
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000))
                .unwrap();
            let before = runtime.then(|| ruby(&path));
            let mtime = fs::metadata(&path).unwrap().modified().unwrap();
            fs::write(&sibling, prefix).unwrap();
            for _ in 0..2 {
                let report = fix(dir.path(), cop, 1);
                assert!(report.contains(cop), "{report}");
                assert_eq!(
                    fs::read(&path).unwrap(),
                    source.as_bytes(),
                    "{cop}: {source}"
                );
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
            }
            assert_eq!(fs::read_to_string(&sibling).unwrap(), canonical);
            if let Some(before) = before {
                assert_eq!(ruby(&path), before, "{source}");
                // Match the CLI's atomic replacement: truncating the same
                // inode would hide identity/hard-link and access-time changes.
                let replacement = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
                fs::write(replacement.path(), naive).unwrap();
                replacement.persist(&path).unwrap();
                for (runtime, (actual, original)) in ruby(&path).iter().zip(&before).enumerate() {
                    if runtime == 1
                        && matches!(
                            *observer,
                            "p DATA.lineno\n__END__\npayload"
                                | "DATA.gets; p $.\n__END__\npayload"
                                | "require 'English'; DATA.gets; p $INPUT_LINE_NUMBER\n__END__\npayload"
                        )
                    {
                        // Ruby 3.4 Prism starts DATA's line count at zero,
                        // unlike the host 3.2 parser's physical-line count.
                        // Keep the conservative hold; this is a native
                        // preserving control, not a native changed witness.
                        assert_eq!(actual, original, "native DATA line control: {source}");
                    } else {
                        assert_ne!(
                            actual, original,
                            "not a runtime witness (runtime {runtime}): {source}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn independent_preserving_siblings_fix_within_the_same_guarded_file() {
    let dir = project();
    let path = dir.path().join("case.rb");
    let runtime = Command::new("ruby").arg("--version").output().is_ok();
    for (cop, source, expected) in [
        // A listed observer (`DATA` with a data
        // section, `caller_locations`, `__FILE__`) withholds every edit,
        // the preserving sibling's too.
        (
            "Layout/TrailingWhitespace,Style/StringLiterals",
            "x = 'hi'  \np DATA.pos\n__END__\npayload\n",
            "x = 'hi'  \np DATA.pos\n__END__\npayload\n",
        ),
        (
            "Layout/EmptyLines,Layout/TrailingWhitespace,Style/StringLiterals",
            "x = 'hi'  \n\n\np caller_locations(0).first.lineno\n",
            "x = 'hi'  \n\n\np caller_locations(0).first.lineno\n",
        ),
        (
            "Layout/TrailingWhitespace,Style/StringLiterals",
            "x = 'hi'  \nrequire \"fileutils\"; FileUtils.mv(__FILE__, \"alias.rb\"); p File.size(\"alias.rb\"); FileUtils.mv(\"alias.rb\", __FILE__)\n",
            "x = 'hi'  \nrequire \"fileutils\"; FileUtils.mv(__FILE__, \"alias.rb\"); p File.size(\"alias.rb\"); FileUtils.mv(\"alias.rb\", __FILE__)\n",
        ),
        // An unlisted size read of the file: only the equal-length sibling
        // is corrected.
        (
            "Layout/TrailingWhitespace,Style/StringLiterals",
            "x = 'hi'  \np File.size(\"case.rb\")\n",
            "x = \"hi\"  \np File.size(\"case.rb\")\n",
        ),
    ] {
        for crlf in [false, true] {
            let encode = |s: &str| {
                if crlf {
                    format!("\u{feff}{}", s.replace('\n', "\r\n"))
                } else {
                    s.into()
                }
            };
            let source = encode(source);
            let expected = encode(expected);
            fs::write(&path, &source).unwrap();
            let original = runtime.then(|| ruby(&path));
            fix(dir.path(), cop, 1);
            assert_eq!(fs::read(&path).unwrap(), expected.as_bytes());
            if let Some(original) = original {
                assert_eq!(ruby(&path), original);
            }
            let mtime = fs::metadata(&path).unwrap().modified().unwrap();
            fix(dir.path(), cop, 1);
            assert_eq!(fs::read(&path).unwrap(), expected.as_bytes());
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        }
    }
}

#[test]
fn data_byte_position_nonconsumer_witnesses() {
    witness(
        "Layout/TrailingWhitespace",
        "x = 1  \n",
        "x = 1\n",
        &[
            "p DATA.pos\n__END__\npayload",
            "p DATA.tell\n__END__\npayload",
            "h = Object::DATA; p h.pos\n__END__\npayload",
            "p Object.const_get(:DATA).tell\n__END__\npayload",
            "getter = DATA.method(:pos); p getter.call\n__END__\npayload",
            "p DATA.public_send(:public_send, 'tell')\n__END__\npayload",
            "class << DATA; alias position pos; end; p DATA.position\n__END__\npayload",
            "def position(h); h.tell; end; p position(DATA)\n__END__\npayload",
            "reader = :pos.to_proc; p reader.call(DATA)\n__END__\npayload",
        ],
    );
}

#[test]
fn physical_line_nonconsumer_witnesses() {
    witness(
        "Layout/EmptyLines",
        "x = 1\n\n\nx = 2\n",
        "x = 1\n\nx = 2\n",
        &[
            "p __LINE__",
            "p DATA.lineno\n__END__\npayload",
            "DATA.gets; p $.\n__END__\npayload",
            "require 'English'; DATA.gets; p $INPUT_LINE_NUMBER\n__END__\npayload",
            "p caller_locations(0).first.lineno",
            "p caller(0)",
            "def marker; end; p method(:marker).source_location",
            "A = 1; p Object.const_source_location(:A)",
            "begin; raise; rescue => e; p e.backtrace; end",
            "tp = TracePoint.new(:line) { |t| p t.lineno; t.disable }; tp.enable\nx = 2",
            "tp = Object.const_get(:TracePoint).new(:line) { |t| p t.lineno; t.disable }; tp.enable\nx = 2",
            "tp = Object.const_get('::TracePoint').new(:line) { |t| p t.lineno; t.disable }; tp.enable\nx = 2",
            "tp = Object.public_send(:const_get, :TracePoint).new(:line) { |t| p t.lineno; t.disable }; tp.enable\nx = 2",
        ],
    );
}

#[test]
fn explicit_current_file_metadata_witnesses() {
    witness(
        "Layout/TrailingWhitespace",
        "x = 1  \n",
        "x = 1\n",
        &[
            "p File.stat(__FILE__).size",
            "p File.size(__FILE__)",
            "p test(?s, __FILE__)",
            "p Kernel.test(115, __FILE__)",
            "p test(?M, __FILE__)",
            "p DATA.stat.size\n__END__\npayload",
            "p File.mtime(__FILE__)",
            "p File.stat(__FILE__).mtime",
            "p DATA.stat.mtime\n__END__\npayload",
            "p File.public_send(:mtime, __FILE__)",
        ],
    );
    witness(
        "Style/StringLiterals",
        "x = 'hi'\n",
        "x = \"hi\"\n",
        &["p File.mtime(__FILE__)"],
    );
}

#[test]
fn literal_current_file_metadata_witnesses() {
    witness(
        "Layout/TrailingWhitespace",
        "x = 1  \n",
        "x = 1\n",
        &[
            "p test(?s, 'case.rb')",
            "p Kernel.test(115, './case.rb')",
            "p Kernel.public_send(:test, ?s, 'case.rb')",
            "p File.send(:size, 'case.rb')",
            "p File.public_send(:send, 'size', './case.rb')",
            "p FileTest.__send__(:size?, 'case.rb')",
            "p File.send(:stat, 'case.rb').size",
        ],
    );
    witness(
        "Style/StringLiterals",
        "x = 'hi'\n",
        "x = \"hi\"\n",
        &[
            "p test(?M, \"case.rb\")",
            "p File.public_send(:mtime, \"case.rb\")",
            "p File.send(:stat, \"case.rb\").mtime",
        ],
    );
}

#[test]
fn explicit_and_unresolved_kernel_test_runtime_witnesses() {
    witness(
        "Layout/TrailingWhitespace",
        "x = 1  \n",
        "x = 1\n",
        &[
            "p self.test(?s, 'case.rb')",
            "p self.send(:test, ?s, 'case.rb')",
            "p self.public_send(:send, :test, ?s, 'case.rb')",
            "p self.test(?s, 'case' + '.rb')",
            "receiver = self; p receiver.__send__(:test, ?s, 'case.rb')",
            "receiver = self; class << receiver; public :test; end; p receiver.test(?s, 'case.rb')",
        ],
    );
    witness(
        "Style/StringLiterals",
        "x = 'hi'\n",
        "x = \"hi\"\n",
        &[
            "p self.test(?M, \"case.rb\")",
            "receiver = self; p receiver.send(:test, ?M, \"case.rb\")",
        ],
    );
}

#[test]
fn relative_symlink_runtime_witnesses() {
    witness(
        "Layout/TrailingWhitespace",
        "x = 1  \n",
        "x = 1\n",
        &[
            "require 'fileutils'; FileUtils.ln_sr(__FILE__, 'alias.rb', force: true); p File.size('alias.rb')",
            "require 'fileutils'; FileUtils.send(:ln_sr, __FILE__, 'alias.rb', force: true); p File.size('alias.rb')",
            "require 'fileutils'; FileUtils.public_send(:send, :ln_sr, __FILE__, 'alias.rb', force: true); p File.size('alias.rb')",
            "require 'fileutils'; getter = FileUtils.method(:ln_sr); getter.call(__FILE__, 'alias.rb', force: true); p File.size('alias.rb')",
            "require 'fileutils'; class << FileUtils; alias make_alias ln_sr; end; FileUtils.make_alias(__FILE__, 'alias.rb', force: true); p File.size('alias.rb')",
        ],
    );
}

#[test]
fn fileutils_move_current_file_alias_runtime_witnesses() {
    for name in ["mv", "move"] {
        let acquisitions = [
            format!("FileUtils.{name}(__FILE__, \"alias.rb\")"),
            format!("FileUtils.send(:{name}, __FILE__, \"alias.rb\")"),
            format!("FileUtils.public_send(:send, :{name}, __FILE__, \"alias.rb\")"),
            format!("getter = FileUtils.method(:{name}); getter.call(__FILE__, \"alias.rb\")"),
            format!("getter = :{name}.to_proc; getter.call(FileUtils, __FILE__, \"alias.rb\")"),
            format!(
                "class << FileUtils; alias relocate {name}; end; FileUtils.relocate(__FILE__, \"alias.rb\")"
            ),
            format!(
                "FileUtils.singleton_class.alias_method(:relocate, :{name}); FileUtils.relocate(__FILE__, \"alias.rb\")"
            ),
            format!("include FileUtils; {name}(__FILE__, \"alias.rb\")"),
            format!("def later; FileUtils.{name}(__FILE__, \"alias.rb\"); end; later"),
        ];
        for (cop, prefix, canonical, observation) in [
            ("Layout/TrailingWhitespace", "x = 1  \n", "x = 1\n", "size"),
            (
                "Style/StringLiterals",
                "x = 'hi'\n",
                "x = \"hi\"\n",
                "mtime",
            ),
        ] {
            let observers: Vec<_> = acquisitions.iter().map(|acquisition| {
                format!("require \"fileutils\"; {acquisition}; p File.{observation}(\"alias.rb\"); FileUtils.{name}(\"alias.rb\", __FILE__)")
            }).collect();
            witness(
                cop,
                prefix,
                canonical,
                &observers.iter().map(String::as_str).collect::<Vec<_>>(),
            );
        }
    }
}

#[test]
fn computed_filesystem_paths_remain_uncertain_without_path_acquisition() {
    witness(
        "Layout/TrailingWhitespace",
        "x = 1  \n",
        "x = 1\n",
        &[
            "p File.size(File.join(__dir__, \"case.rb\"))",
            "p File.size(\"case\" + \".rb\")",
            "p FileTest.size?(\"case\" + \".rb\")",
            "p test(?s, \"case\" + \".rb\")",
            "p File.public_send(:size, \"case\" + \".rb\")",
            "f = File; p f.size(\"case\" + \".rb\")",
            "getter = File.method(:size); p getter.call(\"case\" + \".rb\")",
            "class << File; alias bytes size; end; p File.bytes(\"case\" + \".rb\")",
            "getter = method(:test); p getter.call(?s, \"case\" + \".rb\")",
            "alias file_test test; p file_test(?s, \"case\" + \".rb\")",
            "k = Kernel; p k.test(?s, \"case\" + \".rb\")",
            "flag = 115; p test(flag, \"case\" + \".rb\")",
            "args = [115, \"case\" + \".rb\"]; p test(*args)",
            "def metadata(...); test(...); end; p metadata(115, \"case\" + \".rb\")",
            "def bytes(f, path); f.size(path); end; p bytes(File, \"case\" + \".rb\")",
            "f = Object.const_get(\"Object::File\"); p f.size(\"case\" + \".rb\")",
        ],
    );
}

#[test]
fn escaping_stat_objects_protect_implicit_and_saved_fields() {
    for (cop, prefix, canonical) in [
        ("Layout/TrailingWhitespace", "x = 1  \n", "x = 1\n"),
        ("Style/StringLiterals", "x = 'hi'\n", "x = \"hi\"\n"),
    ] {
        witness(
            cop,
            prefix,
            canonical,
            &[
                "p File.stat(__FILE__)",
                "p File.lstat(__FILE__)",
                "p DATA.stat\n__END__\npayload",
                "saved = File.stat(__FILE__); p saved.inspect",
                "p File.public_send(:stat, __FILE__)",
                "getter = File.method(:stat); p getter.call(__FILE__)",
                "p File.stat(\"case\" + \".rb\")",
                "p File::Stat.new(\"case\" + \".rb\")",
                "s = File::Stat; p s.new(\"case\" + \".rb\")",
                "saved = DATA.stat; p saved.inspect\n__END__\npayload",
                "p File.stat(__FILE__).method(:size).receiver",
                "p Object::File::Stat.new(\"case\" + \".rb\")",
                "s = Object.const_get(\"Object::File::Stat\"); p s.new(\"case\" + \".rb\")",
            ],
        );
    }
}

#[test]
fn visible_filesystem_aliases_invalidate_unrelated_literal_exemptions() {
    witness(
        "Layout/TrailingWhitespace",
        "x = 1  \n",
        "x = 1\n",
        &[
            "File.symlink(__FILE__, \"alias.rb\") unless File.symlink?(\"alias.rb\"); p File.size(\"alias.rb\")",
            "File.public_send(:symlink, __FILE__, \"alias.rb\") unless File.symlink?(\"alias.rb\"); p test(?s, \"alias.rb\")",
            "class << File; alias make_alias symlink; end; File.make_alias(__FILE__, \"alias.rb\") unless File.symlink?(\"alias.rb\"); p File.stat(\"alias.rb\").size",
        ],
    );
}

#[test]
fn access_time_metadata_withholds_even_equal_byte_edits() {
    for (cop, prefix, canonical) in [
        ("Layout/TrailingWhitespace", "x = 1  \n", "x = 1\n"),
        ("Style/StringLiterals", "x = 'hi'\n", "x = \"hi\"\n"),
    ] {
        witness(
            cop,
            prefix,
            canonical,
            &[
                "p File.atime(__FILE__)",
                "p File.stat(__FILE__).atime",
                "p test(?A, __FILE__)",
                "p Kernel.test(65, __FILE__)",
                "p DATA.stat.atime\n__END__\npayload",
            ],
        );
    }
}

#[test]
fn inode_identity_and_timestamp_comparisons_are_write_sensitive() {
    witness(
        "Layout/TrailingWhitespace",
        "x = 1  \n",
        "x = 1\n",
        &[
            "File.link(__FILE__, \"hard.alias\") unless File.exist?(\"hard.alias\"); p File.identical?(__FILE__, \"hard.alias\")",
            "File.link(__FILE__, \"hard.alias\") unless File.exist?(\"hard.alias\"); p test(?-, __FILE__, \"hard.alias\")",
            "File.write(\"other\", \"\"); File.utime(Time.at(1700000000), Time.at(1700000000), \"other\"); p test(?<, \"case.rb\", \"other\")",
            "File.write(\"other\", \"\"); File.utime(Time.at(1700000000), Time.at(1700000000), \"other\"); p test(?<, \"other\", \"case.rb\")",
        ],
    );
}

#[test]
fn canonical_metadata_preserving_edits_still_fix_and_converge() {
    let dir = project();
    let path = dir.path().join("case.rb");
    let runtime = Command::new("ruby").arg("--version").output().is_ok();
    for (cop, prefix, canonical, observer) in [
        (
            "Style/StringLiterals",
            "x = 'hi'\n",
            "x = \"hi\"\n",
            "p [DATA.pos, DATA.tell, DATA.lineno]\n__END__\npayload",
        ),
        (
            "Style/StringLiterals",
            "x = 'hi'\n",
            "x = \"hi\"\n",
            "p File.stat(__FILE__).size",
        ),
        (
            "Style/StringLiterals",
            "x = 'hi'\n",
            "x = \"hi\"\n",
            "p test(?s, __FILE__)",
        ),
        (
            "Layout/TrailingWhitespace",
            "x = 1  \n",
            "x = 1\n",
            "p __LINE__",
        ),
        (
            "Layout/TrailingWhitespace",
            "x = 1  \n",
            "x = 1\n",
            "p [__FILE__, File.size('/dev/null'), Dir.children('.').size]",
        ),
        (
            "Layout/TrailingWhitespace",
            "x = 1  \n",
            "x = 1\n",
            "p DATA.lineno\n__END__\npayload",
        ),
        (
            "Layout/TrailingWhitespace",
            "x = 1  \n",
            "x = 1\n",
            "p caller_locations(0).first.lineno",
        ),
        (
            "Layout/TrailingWhitespace",
            "x = 1  \n",
            "x = 1\n",
            "def marker; end; p method(:marker).source_location",
        ),
        (
            "Style/StringLiterals",
            "x = 'hi'\n",
            "x = \"hi\"\n",
            "p [test(?s, \"case.rb\"), File.send(:size, \"case.rb\")]",
        ),
        (
            "Layout/TrailingWhitespace",
            "x = 1  \n",
            "x = 1\n",
            "p [__FILE__, test(?s, '/dev/null'), File.send(:size, '/dev/null')]",
        ),
        (
            "Layout/TrailingWhitespace",
            "x = 1  \n",
            "x = 1\n",
            "tp = Object.const_get(:TracePoint).new(:line) { |t| p t.lineno; t.disable }; tp.enable\nx = 2",
        ),
        (
            "Style/StringLiterals",
            "x = 'hi'\n",
            "x = \"hi\"\n",
            "p File.stat(File.join(__dir__, \"case.rb\")).size",
        ),
        (
            "Style/StringLiterals",
            "x = 'hi'\n",
            "x = \"hi\"\n",
            "p File::Stat.new(\"case\" + \".rb\").size",
        ),
        (
            "Style/StringLiterals",
            "x = 'hi'\n",
            "x = \"hi\"\n",
            "f = File; p f.size(\"case\" + \".rb\")",
        ),
        (
            "Style/StringLiterals",
            "x = 'hi'\n",
            "x = \"hi\"\n",
            "p File.send(:stat, \"case.rb\").public_send(:size)",
        ),
        (
            "Style/StringLiterals",
            "x = 'hi'\n",
            "x = \"hi\"\n",
            "p test(?s, \"case\" + \".rb\")",
        ),
        (
            "Style/StringLiterals",
            "x = 'hi'\n",
            "x = \"hi\"\n",
            "File.symlink(__FILE__, \"alias\") unless File.symlink?(\"alias\"); p File.size(\"alias\")",
        ),
        (
            "Layout/TrailingWhitespace",
            "x = 1  \n",
            "x = 1\n",
            "p [__FILE__, File.stat('/dev/null'), File.atime('/dev/null'), File.identical?('/dev/null', '/dev/null'), test(?=, '/dev/null', '/dev/null')]",
        ),
        (
            "Style/StringLiterals",
            "x = 'hi'\n",
            "x = \"hi\"\n",
            "p self.test(?s, \"case.rb\")",
        ),
        (
            "Style/StringLiterals",
            "x = 'hi'\n",
            "x = \"hi\"\n",
            "receiver = self; p receiver.send(:test, ?s, \"case.rb\")",
        ),
        (
            "Layout/TrailingWhitespace",
            "x = 1  \n",
            "x = 1\n",
            "p [self.test(?s, '/dev/null'), self.send(:test, ?M, '/dev/null')]",
        ),
        (
            "Style/StringLiterals",
            "x = 'hi'\n",
            "x = \"hi\"\n",
            "require \"fileutils\"; FileUtils.ln_sr(__FILE__, \"alias\", force: true); p File.size(\"alias\")",
        ),
        (
            "Style/StringLiterals",
            "x = 'hi'\n",
            "x = \"hi\"\n",
            "require \"fileutils\"; FileUtils.mv(__FILE__, \"alias.rb\"); p File.size(\"alias.rb\"); FileUtils.mv(\"alias.rb\", __FILE__)",
        ),
        (
            "Style/StringLiterals",
            "x = 'hi'\n",
            "x = \"hi\"\n",
            "require \"fileutils\"; FileUtils.move(__FILE__, \"alias.rb\"); p File.size(\"alias.rb\"); FileUtils.move(\"alias.rb\", __FILE__)",
        ),
        (
            "Layout/TrailingWhitespace",
            "x = 1  \n",
            "x = 1\n",
            "require \"fileutils\"; FileUtils.move(__FILE__, \"alias.rb\"); p __LINE__; FileUtils.move(\"alias.rb\", __FILE__)",
        ),
        (
            "Layout/EmptyLines",
            "p __LINE__\n\n\nx = 1\n",
            "p __LINE__\n\nx = 1\n",
            "p :ok",
        ),
    ] {
        let source = format!("{prefix}{observer}\n");
        let expected = format!("{canonical}{observer}\n");
        fs::write(&path, &source).unwrap();
        // A bare listed observer withholds every
        // cop's edit; the finding stays.
        if listed_observer(observer) {
            let mtime = fs::metadata(&path).unwrap().modified().unwrap();
            fix(dir.path(), cop, 1);
            assert_eq!(fs::read_to_string(&path).unwrap(), source, "{observer}");
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
            continue;
        }
        let before = runtime.then(|| ruby(&path));
        fix(dir.path(), cop, 0);
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
        if let Some(before) = before {
            assert_eq!(ruby(&path), before);
        }
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        fix(dir.path(), cop, 0);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}
