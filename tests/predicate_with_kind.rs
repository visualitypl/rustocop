//! real CLI policy and detection-only semantic/no-write contracts.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const COP: &str = "Style/PredicateWithKind";
const SOURCE: &str = "array.any? { |x| x.is_a?(Integer) }\n";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}

fn run(dir: &Path, args: &[&str], status: i32, count: usize) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(status), "{args:?}: {output:?}");
    let text = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        text.lines().filter(|l| l.contains(COP)).count(),
        count,
        "{args:?}: {text}"
    );
    output
}

fn unchanged(dir: &Path, source: &[u8], args: &[&str], status: i32, count: usize) {
    let path = dir.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    run(dir, args, status, count);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn default_selectors_config_disable_and_explicit_reenable() {
    let dir = project();
    for (config, args, count) in [
        ("", vec!["lib/a.rb"], 1),
        ("", vec!["--only", COP, "lib/a.rb"], 1),
        ("", vec!["--only", "Style", "lib/a.rb"], 1),
        ("", vec!["--except", COP, "lib/a.rb"], 0),
        ("", vec!["--only", COP, "--except", COP, "lib/a.rb"], 0),
        ("[cops.Style]\nenabled = false\n", vec!["lib/a.rb"], 0),
        (
            "[cops.\"Style/PredicateWithKind\"]\nenabled = false\n",
            vec!["lib/a.rb"],
            0,
        ),
        (
            "[cops.\"Style/PredicateWithKind\"]\nenabled = false\n",
            vec!["--only", COP, "lib/a.rb"],
            1,
        ),
        (
            "[cops.Style]\nenabled = false\n[cops.\"Style/PredicateWithKind\"]\nenabled = true\n",
            vec!["lib/a.rb"],
            1,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            dir.path(),
            SOURCE.as_bytes(),
            &args,
            i32::from(count != 0),
            count,
        );
        // This source is otherwise clean, so default/Style --fix is also inert.
        let mut fixing = args;
        fixing.insert(0, "--fix");
        unchanged(
            dir.path(),
            SOURCE.as_bytes(),
            &fixing,
            i32::from(count != 0),
            count,
        );
    }
}

#[test]
fn both_directive_prefixes_and_full_block_suppression_ranges() {
    let dir = project();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "Style", "all"] {
            for source in [
                format!("# {prefix}:disable {target}\n{SOURCE}"),
                format!("{} # {prefix}:disable {target}\n", SOURCE.trim()),
                format!("# {prefix}:disable-next {target}\n{SOURCE}"),
                format!("array.any? do |x|\n  x.is_a?(Integer) # {prefix}:disable {target}\nend\n"),
            ] {
                unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "lib/a.rb"],
                    0,
                    0,
                );
            }
        }
        let source = format!("# {prefix}:disable {COP}\n{SOURCE}# {prefix}:enable {COP}\n{SOURCE}");
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            1,
            1,
        );
        fs::write(
            dir.path().join("rustocop.toml"),
            "[cops.Style]\nenabled = false\n",
        )
        .unwrap();
        let source = format!("# {prefix}:enable {COP}\n{SOURCE}# {prefix}:disable {COP}\n{SOURCE}");
        unchanged(dir.path(), source.as_bytes(), &["--fix", "lib/a.rb"], 1, 1);
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}

#[test]
fn config_relative_and_global_excludes_win_over_only() {
    let dir = project();
    for config in [
        "[cops.\"Style/PredicateWithKind\"]\nexclude = ['lib/**']\n",
        "exclude = ['lib/**']\n",
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            dir.path(),
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            0,
            0,
        );
        run(
            &dir.path().join("lib"),
            &["--only", COP, "--fix", "a.rb"],
            0,
            0,
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            SOURCE
        );
    }
}

#[test]
fn unsafe_clean_and_error_files_keep_bytes_and_mtime_even_with_fix() {
    let dir = project();
    for (source, status, count) in [
        (SOURCE.as_bytes(), 1, 1),
        (b"array.any?(Integer)\n".as_slice(), 0, 0),
        (b"array.any? { |x|\n".as_slice(), 2, 0),
        (b"\xef\xbb\xbf# coding: utf-8\r\n# caf\xc3\xa9\r\narray.any? { |x| x.is_a?(Integer) }\r\np __LINE__\r\n__END__\r\n\xffarray.any?\r\n".as_slice(), 1, 1),
        (b"# coding: ISO-8859-1\r\n# \xe9\r\narray.any? { |x| x.is_a?(Integer) }\r\n__END__\r\n\xe9\r\n".as_slice(), 1, 1),
    ] {
        for args in [vec!["--only", COP, "lib/a.rb"], vec!["--only", COP, "--fix", "lib/a.rb"]] {
            unchanged(dir.path(), source, &args, status, count);
        }
    }
}

#[test]
fn stdin_reports_exact_location_without_touching_display_path() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "disk sentinel").unwrap();
    for (config, count) in [
        ("", 1),
        (
            "[cops.\"Style/PredicateWithKind\"]\nexclude = ['lib/**']\n",
            0,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(format!("# heading\n  {SOURCE}").as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(count));
        let text = String::from_utf8_lossy(&output.stdout);
        assert_eq!(text.matches(COP).count(), count as usize);
        if count == 1 {
            assert!(text.contains("lib/a.rb:2:3:"), "{text}");
            assert!(text.contains("Prefer `any?(Integer)` to `any? { ... }` with a kind check."));
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "disk sentinel");
    }
    run(
        dir.path(),
        &["--only", COP, "--stdin", "lib/a.rb", "--fix"],
        2,
        0,
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

#[test]
fn ruby_custom_semantics_visibility_evaluation_context_and_control_flow_remain_intact() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, count, expected) in [
        // Exact classes differ from === on subclasses, even without overrides.
        (
            "class Base; end; class Child < Base; end\np [[Child.new].any? { |x| x.instance_of?(Base) }, [Child.new].any?(Base)]\n",
            1,
            "[false, true]\n",
        ),
        // A shadowed constant is a legitimate argument with custom === dispatch.
        (
            "Integer = Object.new\ndef Integer.===(x); false; end\no = Object.new; o.define_singleton_method(:is_a?) { |klass| klass.equal?(Integer) }\np [[o].any? { |x| x.is_a?(Integer) }, [o].any?(Integer)]\n",
            1,
            "[true, false]\n",
        ),
        // Do not turn a private reflective kind check into a public pattern call.
        (
            "o = Object.new\no.singleton_class.method(:private).call(:is_a?)\nbegin\n [o].any? { |x| x.is_a?(Object) }\nrescue NoMethodError\n puts :private\nend\np [o].any?(Object)\n",
            1,
            "private\ntrue\n",
        ),
        // Argument evaluation belongs in the block: empty/short-circuit enumerables,
        // block binding, source locations, match state, encoding and nonlocal return.
        (
            "$events = []\ndef klass(x); $events << x; Integer; end\ndef f\n [1, 2].all? { |x| x.is_a?(klass(x)) }\nend\ndef g\n [1].any? { |x| return x.is_a?(Integer) }\nend\n/(ca)(fé)/ =~ 'café'\np [f, [].none? { |x| x.is_a?(klass(x)) }, [1, 'x'].any? { |x| x.is_a?(klass(x)) }, g, $events, $1, $2.bytes, __LINE__, method(:f).source_location.last, 'é'.encoding.name]\n",
            3,
            "[true, true, true, true, [1, 2, 1], \"ca\", [102, 195, 169], 10, 3, \"UTF-8\"]\n",
        ),
        // Same selector names on a custom enumerable still match, never rewrite.
        (
            "o = Object.new\ndef o.any?(*args); [args, yield(1)]; end\np o.any?(:pattern) { |x| x.is_a?(Integer) }\n",
            1,
            "[[:pattern], true]\n",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let Some(before) = ruby(dir.path()) else {
            return;
        };
        assert!(before.status.success(), "{before:?}");
        assert_eq!(before.stdout, expected.as_bytes(), "{source}");
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(dir.path(), &["--only", COP, "--fix", "lib/a.rb"], 1, count);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        let after = ruby(dir.path()).unwrap();
        assert_eq!(before.status.code(), after.status.code());
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
    }
}

#[test]
fn composition_with_class_check_and_layout_never_applies_the_unsafe_predicate_edit() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "array.any? { |x| x.kind_of?(Integer) }\n").unwrap();
    let only = "Style/PredicateWithKind,Style/ClassCheck,Layout";
    run(dir.path(), &["--only", only, "--fix", "lib/a.rb"], 1, 1);
    assert_eq!(fs::read_to_string(&path).unwrap(), SOURCE);
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    run(dir.path(), &["--only", only, "--fix", "lib/a.rb"], 1, 1);
    assert_eq!(fs::read_to_string(&path).unwrap(), SOURCE);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
