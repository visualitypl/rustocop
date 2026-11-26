//! literal separator ownership, selectors, bytes and Ruby observations.
use crate::support::cli::project_with_lib;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const ARRAY: &str = "Style/TrailingCommaInArrayLiteral";
const HASH: &str = "Style/TrailingCommaInHashLiteral";
const BLOCK: &str = "Style/TrailingCommaInBlockArgs";
const ALL: &str = "Style/TrailingCommaInArrayLiteral,Style/TrailingCommaInHashLiteral,Style/TrailingCommaInBlockArgs";
const CASES: [(&str, &str, &str, bool); 3] = [
    (ARRAY, "p [1,]\n", "p [1]\n", true),
    (HASH, "p({ a: 1, })\n", "p({ a: 1 })\n", true),
    (
        BLOCK,
        "test { |a, b,| a + b }\n",
        "test { |a, b,| a + b }\n",
        false,
    ),
];

fn cli(dir: &Path, args: &[&str], status: i32) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    out
}

#[test]
fn exact_registration_defaults_and_plain_lint_are_read_only() {
    let registry = rustocop::cops::Registry::new();
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    for (name, before, _, safe) in CASES {
        let cops: Vec<_> = registry
            .cops()
            .iter()
            .filter(|cop| cop.name() == name)
            .collect();
        assert_eq!(cops.len(), 1);
        assert_eq!(cops[0].default_enabled(), safe);
        assert_eq!(cops[0].safe_fixes(), safe);
        assert!(cops[0].path_policy().is_none());
        fs::write(&path, before).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .arg("lib/a.rb")
            .output()
            .unwrap();
        assert_ne!(out.status.code(), Some(2), "{out:?}");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).contains(name),
            safe,
            "{out:?}"
        );
        cli(dir.path(), &["--only", name, "lib/a.rb"], 1);
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn selectors_and_config_enable_disable_with_stable_safe_only_fixes() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    for (name, before, after, safe) in CASES {
        for config in [
            format!("[cops.\"{name}\"]\nenabled = false\n"),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            fs::write(&path, before).unwrap();
            let mtime = fs::metadata(&path).unwrap().modified().unwrap();
            cli(dir.path(), &["--fix", "lib/a.rb"], 0);
            cli(dir.path(), &["--only", name, "lib/a.rb"], 1);
            cli(
                dir.path(),
                &["--only", name, "--except", name, "--fix", "lib/a.rb"],
                0,
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), before);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{name}\"]\nenabled = true\n"),
        )
        .unwrap();
        cli(dir.path(), &["--fix", "lib/a.rb"], i32::from(!safe));
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        cli(
            dir.path(),
            &["--only", name, "--fix", "lib/a.rb"],
            i32::from(!safe),
        );
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn directives_support_both_prefixes_and_all_selector_forms() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    for (name, before, _, _) in CASES {
        for prefix in ["rustocop", "rubocop"] {
            for target in [name, name.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\n{before}"),
                    format!("{} # {prefix}:disable {target}\n", before.trim_end()),
                    format!("# {prefix}:disable-next {target}\n{before}"),
                ] {
                    fs::write(&path, &source).unwrap();
                    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
                    cli(dir.path(), &["--only", name, "--fix", "lib/a.rb"], 0);
                    assert_eq!(fs::read_to_string(&path).unwrap(), source);
                    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
                }
            }
        }
    }
    // Source enables can opt in the unsafe detector without enabling fixes.
    fs::write(&path, format!("# rustocop:enable {BLOCK}\n{}", CASES[2].1)).unwrap();
    let out = cli(dir.path(), &["--fix", "lib/a.rb"], 1);
    assert!(String::from_utf8_lossy(&out.stdout).contains(BLOCK));
    assert!(fs::read_to_string(&path).unwrap().contains("a, b,|"));
}

#[test]
fn literal_directives_anchor_the_comma_not_the_open_or_close() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    for (name, open, item, close) in [(ARRAY, "[", "1", "]"), (HASH, "{", "a: 1", "}")] {
        let source = format!(
            "{open} # rubocop:disable {name}\n{item},\n{close} # rustocop:disable {name}\n"
        );
        fs::write(&path, &source).unwrap();
        cli(dir.path(), &["--only", name, "--fix", "lib/a.rb"], 0);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            source.replace(&format!("{item},"), item)
        );
        let source = format!("{open}\n{item}, # rubocop:disable {name}\n{close}\n");
        fs::write(&path, &source).unwrap();
        cli(dir.path(), &["--only", name, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
}

#[test]
fn config_relative_excludes_and_stdin_never_write() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    for (name, before, _, _) in CASES {
        for config in [
            String::new(),
            "exclude = [\"lib/a.rb\"]\n".into(),
            format!("[cops.\"{name}\"]\nexclude = [\"lib/a.rb\"]\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), &config).unwrap();
            fs::write(&path, before).unwrap();
            let mtime = fs::metadata(&path).unwrap().modified().unwrap();
            let status = i32::from(config.is_empty());
            cli(&dir.path().join("lib"), &["--only", name, "a.rb"], status);
            if status == 0 {
                cli(
                    &dir.path().join("lib"),
                    &["--only", name, "--fix", "a.rb"],
                    0,
                );
            }
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path().join("lib"))
                .args(["--only", name, "--stdin", "a.rb"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(before.as_bytes())
                .unwrap();
            assert_eq!(
                child.wait_with_output().unwrap().status.code(),
                Some(status)
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), before);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        }
    }
}

#[test]
fn deletion_preserves_bom_crlf_legacy_encoding_comments_literals_data_and_line() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    for source in [
        "\u{feff}# encoding: UTF-8\r\np [\"café\",]\r\np({a: \"café\",}) # comment,\r\np __LINE__\r\n__END__\r\nraw,\r\n".as_bytes().to_vec(),
        b"# encoding: ISO-8859-1\r\np [\"caf\xe9\",]\r\np({a: \"caf\xe9\",}) # comment,\r\np __LINE__\r\n__END__\r\nraw\xff,\r\n".to_vec(),
    ] {
        fs::write(&path, &source).unwrap();
        let mut expected = source.clone();
        for offset in (0..expected.len()-1).rev() {
            if expected[offset] == b',' && matches!(expected[offset+1], b']' | b'}') { expected.remove(offset); }
        }
        cli(dir.path(), &["--only", ALL, "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read(&path).unwrap(), crate::support::lf::lf(&expected));
    }
}

fn ruby(dir: &Path) -> Output {
    Command::new("ruby")
        .current_dir(dir)
        .arg("lib/a.rb")
        .output()
        .unwrap()
}

#[test]
fn controlled_ruby_values_insertion_order_evaluation_and_block_destructuring() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    let source = r#"$events = []
def effect(x); $events << x; x; end
def take(*args, **kw); [args, kw]; end
xs = [2,3]
kw = {b: 4, a: 5}
a = :omitted
p [effect(1), *effect(xs),]
p({a: effect(:first), **effect(kw), c: effect(:last),})
p take([a:,], {a:,}, a: [effect(:keyword),])
p [<<~A, <<~B.chomp,]
  café, #{effect(:first_heredoc)}
A
  café, #{ {c: effect(:interpolation),} }
B
p({a: <<~C.chomp,})
  literal, #{[effect(:nested),]}
C
p [[1,2]].map { |(x,y),| [x,y,] }
p [[1,2]].map { |x,| x }
p [1,2].map { |x,y,| [x,y,] }
p lambda { |x,y,| [x,y,] }.call(1,2)
p catch(:done) { [effect(:before_jump), throw(:done, :jump), effect(:not_run),] }
p catch(:done) { {a: effect(:hash_before_jump), b: throw(:done, :hash_jump), c: effect(:not_run),} }
p [false && effect(:not_run), true || effect(:not_run),]
p ({a: (effect(:exception); raise "stop"),} rescue :rescued)
p __LINE__
p $events
__END__
payload,
"#;
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path());
    assert!(before.status.success(), "{before:?}");
    // Unsafe block findings remain, but all eligible literal edits apply.
    cli(dir.path(), &["--only", ALL, "--fix", "lib/a.rb"], 1);
    let fixed = fs::read_to_string(&path).unwrap();
    assert!(fixed.contains("|(x,y),|"));
    assert!(fixed.contains("|x,|"));
    assert!(fixed.contains("|x,y,|"));
    assert!(fixed.contains("literal, #{[effect(:nested)]}"));
    assert_eq!(fixed.lines().count(), source.lines().count());
    assert!(fixed.ends_with("payload,\n"));
    cli(
        dir.path(),
        &["--only", &format!("{ARRAY},{HASH}"), "lib/a.rb"],
        0,
    );
    let after = ruby(dir.path());
    assert!(after.status.success(), "{after:?}");
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
    assert!(!String::from_utf8_lossy(&after.stdout).contains(":not_run"));
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir.path(), &["--only", ALL, "--fix", "lib/a.rb"], 1);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
}

#[test]
fn actual_default_and_selected_style_layout_peers_stabilize_and_preserve_runtime() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    let source = "def sink(x)\n  x\nend\nf = lambda { |x| sink([x,],) }\np f.(({'a' => 'x',}),)\np [[1,2]].map { |(a,b),| [a,b,] }\n";
    let peers = format!(
        "{ALL},Style/TrailingCommaInArguments,Style/HashSyntax,Style/Lambda,Style/LambdaCall,Style/Proc,Style/StabbyLambdaParentheses,Style/BlockDelimiters,Style/StringLiterals,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Layout"
    );
    for only in [Some(peers.as_str()), None] {
        fs::write(&path, source).unwrap();
        let before = Command::new("ruby")
            .current_dir(dir.path())
            .arg("lib/a.rb")
            .output()
            .ok();
        let mut stable = false;
        for _ in 0..6 {
            let previous = fs::read(&path).unwrap();
            let mut args = vec!["--fix", "lib/a.rb"];
            if let Some(only) = only {
                args.extend(["--only", only]);
            }
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert_ne!(out.status.code(), Some(2), "{out:?}");
            if previous == fs::read(&path).unwrap() {
                stable = true;
                break;
            }
        }
        assert!(
            stable,
            "curated fixture must stabilize within six explicit passes"
        );
        let fixed = fs::read_to_string(&path).unwrap();
        assert!(!fixed.contains(",]"));
        assert!(!fixed.contains(",}"));
        assert!(!fixed.contains(",)"));
        assert!(
            fixed.contains("|(a,b),|") || fixed.contains("|(a, b),|"),
            "{fixed}"
        );
        cli(
            dir.path(),
            &["--only", &format!("{ARRAY},{HASH}"), "lib/a.rb"],
            0,
        );
        if let Some(before) = before {
            let after = ruby(dir.path());
            assert!(
                before.status.success() && after.status.success(),
                "{before:?} {after:?}"
            );
            assert_eq!(before.stdout, after.stdout);
            assert_eq!(before.stderr, after.stderr);
        }
    }
}
