//! Controlled synthetic Ruby witnesses only. Never execute corpus/command inputs.
use std::{fs, path::Path, process::Command};
const K: &str = "Style/KeywordArgumentsMerging";
const U: &str = "Style/RedundantInterpolationUnfreeze";
const UNION: &str = "Style/KeywordArgumentsMerging,Style/RedundantInterpolationUnfreeze,Style/StringLiterals,Layout";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn fix(dir: &Path, only: Option<&str>, files: &[&str]) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rustocop"));
    cmd.current_dir(dir).arg("--fix");
    if let Some(only) = only {
        cmd.args(["--only", only]);
    }
    let out = cmd.args(files).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    String::from_utf8(out.stdout).unwrap()
}
fn count(dir: &Path) -> usize {
    count_with_args(dir, &[])
}
fn count_with_args(dir: &Path, args: &[&str]) -> usize {
    let out = Command::new("ruby")
        .arg("--disable-gems")
        .args(args)
        .arg("a.rb")
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    String::from_utf8(out.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

#[test]
fn real_hash_and_string_counters_reject_naive_rewrites_and_preserve_repeated_values() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("a.rb");
    for (cop, candidate, canonical, kind) in [
        (
            K,
            "def f(**kw); end\nf(**{a:1}.merge(b:2))\n",
            "def f(**kw); end\nf(**{a:1}, b:2)\n",
            "T_HASH",
        ),
        (U, "a = \"#{12}\".dup\n", "a = \"#{12}\"\n", "T_STRING"),
        (
            U,
            "a = String.new(\"#{12}\")\n",
            "a = \"#{12}\"\n",
            "T_STRING",
        ),
    ] {
        for observer in [
            format!("p ObjectSpace.count_objects.fetch(\"{kind}\".to_sym)\n"),
            format!("p ::Object::ObjectSpace.count_objects.fetch(\"{kind}\".to_sym)\n"),
            format!("space = ObjectSpace; p space.count_objects.fetch(\"{kind}\".to_sym)\n"),
            format!(
                "getter = ObjectSpace.method(:count_objects); p getter.call.fetch(\"{kind}\".to_sym)\n"
            ),
            format!(
                "def read_heap(reader); reader.count_objects.fetch(\"{kind}\".to_sym); end; p read_heap(ObjectSpace)\n"
            ),
            "p GC.stat(:total_allocated_objects)\n".into(),
            "p ::GC.public_send(:stat, :total_allocated_objects)\n".into(),
            "getter = :stat.to_proc; p getter.call(GC, :total_allocated_objects)\n".into(),
            "def read_heap(reader); reader.stat(:total_allocated_objects); end; p read_heap(GC)\n"
                .into(),
        ] {
            let source = format!("{candidate}{observer}");
            let naive = format!("{canonical}{observer}");
            // Fixed3.4 Prism acceptance is separate from the local host runtime.
            assert_eq!(ruby_prism::parse(source.as_bytes()).errors().count(), 0);
            assert_eq!(ruby_prism::parse(naive.as_bytes()).errors().count(), 0);
            let mut originals = Vec::new();
            let mut rewrites = Vec::new();
            for _ in 0..3 {
                fs::write(&path, &source).unwrap();
                originals.push(count(dir.path()));
                fs::write(&path, &naive).unwrap();
                rewrites.push(count(dir.path()));
            }
            assert!(originals.windows(2).all(|w| w[0] == w[1]), "{originals:?}");
            assert!(rewrites.windows(2).all(|w| w[0] == w[1]), "{rewrites:?}");
            assert!(
                originals[0] > rewrites[0],
                "{source}: {originals:?} vs {rewrites:?}"
            );
            fs::write(&path, &source).unwrap();
            let mtime = fs::metadata(&path).unwrap().modified().unwrap();
            for only in [Some(cop), Some(UNION), None] {
                for _ in 0..2 {
                    let output = fix(dir.path(), only, &["a.rb"]);
                    assert!(output.contains(cop), "{output}");
                    assert!(output.contains("0 corrections"), "{output}");
                    assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
                    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
                    assert_eq!(count(dir.path()), originals[0]);
                }
            }
        }
    }
}

#[test]
fn symbol_proc_mutations_and_compound_getters_preserve_runtime_counts_without_writes() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("a.rb");
    fs::write(
        dir.path().join("heap_helper.rb"),
        "Trigger = nil; def File.stat(*); GC.stat(:total_allocated_objects); end\n",
    )
    .unwrap();
    // Runtime setup is deliberately outside the main AST: the visible evidence
    // must be the acquired mutation/getter, not the hidden GC constant or def.
    fs::write(
        dir.path().join("prelude.rb"),
        "def reader; GC; end\ndef heap_snapshot; GC.stat; end\ndef GC.stat=(_); end\ndef GC.total_allocated_objects; stat(:total_allocated_objects); end\ndef GC.total_allocated_objects=(_); end\n",
    )
    .unwrap();
    let mut witnesses = Vec::new();
    for mutation in [
        ":autoload.to_proc.call(Object, 'Trigger', './heap_helper.rb')",
        "def invoke; yield Object, 'Trigger', './heap_helper.rb'; end\ninvoke(&:autoload)",
    ] {
        witnesses.push((
            "Style/DirEmpty",
            "a = Dir.children('.').empty?\n".to_owned(),
            "a = Dir.empty?('.')\n".to_owned(),
            format!("{mutation}\nTrigger\np File.stat('ignored')\n"),
        ));
    }
    for getter in [
        "reader.stat ||= nil",
        "reader.stat &&= heap_snapshot",
        "reader.total_allocated_objects += 0",
    ] {
        let observer = if getter.contains("total_allocated_objects") {
            // A numeric getter needs no Hash conversion after the assignment.
            format!("p({getter})\n")
        } else {
            format!("p(({getter}).fetch(:total_allocated_objects))\n")
        };
        witnesses.push((
            K,
            "def f(**kw); end\nf(**{a:1}.merge(b:2))\n".to_owned(),
            "def f(**kw); end\nf(**{a:1}, b:2)\n".to_owned(),
            observer,
        ));
    }
    for (cop, candidate, canonical, observer) in witnesses {
        let source = format!("{candidate}{observer}");
        let naive = format!("{canonical}{observer}");
        assert_eq!(ruby_prism::parse(source.as_bytes()).errors().count(), 0);
        assert_eq!(ruby_prism::parse(naive.as_bytes()).errors().count(), 0);
        let runtime = || count_with_args(dir.path(), &["-r", "./prelude.rb"]);
        let mut originals = Vec::new();
        let mut rewrites = Vec::new();
        for _ in 0..3 {
            fs::write(&path, &source).unwrap();
            originals.push(runtime());
            fs::write(&path, &naive).unwrap();
            rewrites.push(runtime());
        }
        assert!(originals.windows(2).all(|w| w[0] == w[1]), "{originals:?}");
        assert!(rewrites.windows(2).all(|w| w[0] == w[1]), "{rewrites:?}");
        assert!(
            originals[0] > rewrites[0],
            "{source}: {originals:?} vs {rewrites:?}"
        );
        fs::write(&path, &source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        for only in [Some(cop), Some("Style,Layout"), None] {
            let output = fix(dir.path(), only, &["a.rb"]);
            assert!(output.contains(cop), "{output}");
            assert!(output.contains("0 corrections"), "{output}");
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
            assert_eq!(runtime(), originals[0]);
        }
    }
}

#[test]
fn wrapped_composed_deferred_and_reflective_mutations_preserve_runtime_without_writes() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("a.rb");
    let helper = dir.path().join("helper.rb");
    fs::write(
        &helper,
        "Trigger = nil\ndef File.stat(*)\n GC.stat(:total_allocated_objects)\nend\n",
    )
    .unwrap();
    let helper = format!("\"{}\"", helper.display());
    for mutation in [
        format!(
            "def setup(path); yield Object, \"Trigger\", path; end; setup({helper}, &((:autoload)))"
        ),
        format!("((:autoload)).to_proc.call(Object, \"Trigger\", {helper})"),
        format!("callback = ((:autoload)).to_proc; callback.call(Object, \"Trigger\", {helper})"),
        format!(
            "def later(path); ((:autoload)).to_proc.call(Object, \"Trigger\", path); end; later({helper})"
        ),
        format!("((:autoload)).public_send((:to_proc)).call(Object, \"Trigger\", {helper})"),
        format!("((:autoload)).method((:to_proc)).call.call(Object, \"Trigger\", {helper})"),
        format!("Object.public_send((:public_send), ((:autoload)), \"Trigger\", {helper})"),
        format!(
            "selector = (\"auto\" + \"load\").to_sym; selector.to_proc.call(Object, \"Trigger\", {helper})"
        ),
        format!(
            "def setup(path); yield Object, \"Trigger\", path; end; selector = (\"auto\" + \"load\").to_sym; setup({helper}, &selector)"
        ),
        format!("((:public_send)).to_proc.call(Object, :autoload, \"Trigger\", {helper})"),
    ] {
        let observer = format!("{mutation}\nTrigger\np File.stat(\"ignored\")\n");
        let source = format!("a = Dir.children(\".\").empty?\n{observer}");
        let naive = format!("a = Dir.empty?(\".\")\n{observer}");
        for code in [&source, &naive] {
            assert_eq!(ruby_prism::parse(code.as_bytes()).errors().count(), 0);
        }
        let mut originals = Vec::new();
        let mut rewrites = Vec::new();
        for _ in 0..3 {
            fs::write(&path, &source).unwrap();
            originals.push(count(dir.path()));
            fs::write(&path, &naive).unwrap();
            rewrites.push(count(dir.path()));
        }
        assert!(originals.windows(2).all(|w| w[0] == w[1]), "{originals:?}");
        assert!(rewrites.windows(2).all(|w| w[0] == w[1]), "{rewrites:?}");
        assert!(
            originals[0] > rewrites[0],
            "{source}: {originals:?} vs {rewrites:?}"
        );
        fs::write(&path, &source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        for only in [Some("Style/DirEmpty"), Some("Style,Layout"), None] {
            for _ in 0..2 {
                let output = fix(dir.path(), only, &["a.rb"]);
                assert!(output.contains("Style/DirEmpty"), "{output}");
                assert!(output.contains("0 corrections"), "{output}");
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
                assert_eq!(count(dir.path()), originals[0]);
            }
        }
    }
}

#[test]
fn reflected_aliased_deferred_and_unknown_readers_keep_cli_bytes_and_mtime() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for observer in [
        "p ::Object::GC.stat",
        "Space = ::ObjectSpace; p Space.count_objects",
        "def later(reader); reader.stat; end; p later(acquire)",
        "getter = reader.method(:stat); escape(getter)",
        "getter = :count_objects.to_proc; escape(getter)",
        "class << reader; alias counts stat; end; p reader.counts",
        "p reader.public_send(:public_send, 'stat')",
        "p reader.send(selector)",
        "p Object.const_get(namespace).stat",
        ":autoload.to_proc.call(Object, 'Trigger', './heap_helper.rb')\np File.stat('ignored')",
        "invoke(&:autoload)\np File.stat('ignored')",
        "((:autoload)).to_proc.call(Object, 'Trigger', './heap_helper.rb')\np File.stat('ignored')",
        "invoke(&((:autoload)))\np File.stat('ignored')",
        "((:autoload)).public_send((:to_proc))\np File.stat('ignored')",
        "selector.to_proc\np File.stat('ignored')",
        "invoke(&selector)\np File.stat('ignored')",
        "reader.stat ||= nil",
        "reader.stat &&= heap_snapshot",
        "reader.total_allocated_objects += 0",
    ] {
        let source = format!(
            "a = \"#{{12}}\".dup  \ndef f(**kw); end\nf(**{{a:1}}.merge(b:2))\n{observer}\n"
        );
        fs::write(&path, &source).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        for only in [Some(K), Some(U), Some(UNION), None] {
            fix(dir.path(), only, &["a.rb"]);
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes(), "{source}");
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        }
    }
}

#[test]
fn proved_non_heap_metadata_and_no_observer_canonicals_remain_eligible() {
    let dir = project();
    fs::write(dir.path().join("other.rb"), "# metadata only\n").unwrap();
    let path = dir.path().join("a.rb");
    for observer in [
        "p File.stat(\"other.rb\").file?",
        "p ::Object::GC.name",
        "p ObjectSpace.class",
        "p defined?(GC)",
        "# GC.stat; ObjectSpace.count_objects",
    ] {
        let source = format!("x = 'hi'\n{observer}\n");
        fs::write(&path, &source).unwrap();
        let before = Command::new("ruby")
            .arg("--version")
            .output()
            .is_ok()
            .then(|| {
                Command::new("ruby")
                    .args(["--disable-gems", "a.rb"])
                    .current_dir(dir.path())
                    .output()
                    .unwrap()
            });
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", "Style/StringLiterals", "--fix", "a.rb"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            source.replace("'hi'", "\"hi\"")
        );
        if let Some(before) = before {
            let after = Command::new("ruby")
                .args(["--disable-gems", "a.rb"])
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(before.status.success() && after.status.success());
            assert_eq!(before.stdout, after.stdout);
            assert_eq!(before.stderr, after.stderr);
        }
    }
    // No-write includes noncanonical envelopes. Commands are never executed.
    let source = b"\xef\xbb\xbfx = 'hi'  \r\np ::GC.stat\r\n__END__\r\npayload\r\n";
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    fix(dir.path(), None, &["a.rb"]);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
}

#[cfg(unix)]
#[test]
fn alias_union_cannot_reintroduce_heap_changing_edits_from_disjoint_selections() {
    let dir = project();
    let path = dir.path().join("a.rb");
    let alias = dir.path().join("b.rb");
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    let string = "Style/StringLiterals";
    fs::write(
        dir.path().join("helper.rb"),
        "Trigger = nil; def File.stat(*); GC.stat(:total_allocated_objects); end\n",
    )
    .unwrap();
    for (cop, candidate, canonical) in [
        (
            "Style/DirEmpty",
            "a = Dir.children(\".\").empty?\n",
            "a = Dir.empty?(\".\")\n",
        ),
        (
            K,
            "def f(**kw); end\nf(**{a:1}.merge(b:2))\n",
            "def f(**kw); end\nf(**{a:1}, b:2)\n",
        ),
        (U, "a = \"#{12}\".dup\n", "a = \"#{12}\"\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), format!("[cops.\"{cop}\"]\nexclude = [\"a.rb\"]\n[cops.\"{string}\"]\nexclude = [\"b.rb\"]\n")).unwrap();
        for observer in [
            "p GC.stat(:total_allocated_objects)\n",
            "((:autoload)).to_proc.call(Object, \"Trigger\", \"./helper.rb\")\nTrigger\np File.stat(\"ignored\")\n",
            "def setup; yield Object, \"Trigger\", \"./helper.rb\"; end; setup(&((:autoload)))\nTrigger\np File.stat(\"ignored\")\n",
        ] {
            let source = format!("{candidate}b = 'hi'\n{observer}");
            fs::write(&path, &source).unwrap();
            let mtime = fs::metadata(&path).unwrap().modified().unwrap();
            let before = Command::new("ruby")
                .arg("--version")
                .output()
                .is_ok()
                .then(|| count(dir.path()));
            for _ in 0..3 {
                let output = fix(
                    dir.path(),
                    Some(&format!("{cop},{string}")),
                    &["a.rb", "b.rb"],
                );
                assert!(output.contains(cop) && output.contains(string), "{output}");
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
                assert!(
                    fs::symlink_metadata(&alias)
                        .unwrap()
                        .file_type()
                        .is_symlink()
                );
                if let Some(before) = before {
                    assert_eq!(count(dir.path()), before);
                }
            }
        }
        // Same disjoint selection without observer still supplies both fixes.
        fs::write(&path, format!("{candidate}b = 'hi'\n")).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args([
                "--fix",
                "--only",
                &format!("{cop},{string}"),
                "a.rb",
                "b.rb",
            ])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("{canonical}b = \"hi\"\n")
        );
    }
}
