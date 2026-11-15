//! real selectors and controlled subprocess ENV; no ambient values used.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const HOME: &str = "Style/EnvHome";
const FETCH: &str = "Style/FetchEnvVar";
const DATE: &str = "Style/DateTime";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn run(dir: &Path, args: &[&str], status: i32, counts: [usize; 3]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    for (cop, count) in [HOME, FETCH, DATE].into_iter().zip(counts) {
        assert_eq!(
            text.lines().filter(|line| line.contains(cop)).count(),
            count,
            "{args:?}: {text}"
        );
    }
    out
}
#[test]
fn defaults_opt_in_only_except_and_config_are_executable() {
    let dir = project();
    let source = "ENV[\"HOME\"]\nENV[\"TASK70_X\"]\nDateTime.now\n";
    fs::write(dir.path().join("a.rb"), source).unwrap();
    for (config, args, counts) in [
        (String::new(), vec!["a.rb"], [1, 2, 0]),
        (String::new(), vec!["--only", HOME, "a.rb"], [1, 0, 0]),
        (String::new(), vec!["--only", FETCH, "a.rb"], [0, 2, 0]),
        (String::new(), vec!["--only", DATE, "a.rb"], [0, 0, 1]),
        (
            format!("[cops.\"{DATE}\"]\nenabled = true\n"),
            vec!["a.rb"],
            [1, 2, 1],
        ),
        (
            "[cops.Style]\nenabled = false\n".into(),
            vec!["--only", DATE, "a.rb"],
            [0, 0, 1],
        ),
        (
            format!("[cops.Style]\nenabled = false\n[cops.\"{DATE}\"]\nenabled = true\n"),
            vec!["a.rb"],
            [0, 0, 1],
        ),
        (
            format!("[cops.\"{FETCH}\"]\nenabled = false\n"),
            vec!["a.rb"],
            [1, 0, 0],
        ),
        (
            String::new(),
            vec!["--only", DATE, "--except", DATE, "a.rb"],
            [0, 0, 0],
        ),
        (
            String::new(),
            vec!["--only", FETCH, "--except", FETCH, "a.rb"],
            [0, 0, 0],
        ),
        (
            String::new(),
            vec!["--only", HOME, "--except", HOME, "a.rb"],
            [0, 0, 0],
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let modified = fs::metadata(dir.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap();
        run(dir.path(), &args, i32::from(counts != [0, 0, 0]), counts);
        assert_eq!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), source);
        assert_eq!(
            fs::metadata(dir.path().join("a.rb"))
                .unwrap()
                .modified()
                .unwrap(),
            modified
        );
    }
}
#[test]
fn config_relative_excludes_and_both_directive_prefixes_prevent_writes() {
    let dir = project();
    fs::create_dir(dir.path().join("lib")).unwrap();
    let path = dir.path().join("lib/a.rb");
    for (cop, code) in [
        (HOME, "ENV['HOME']\n"),
        (FETCH, "ENV['TASK70_X']\n"),
        (DATE, "DateTime.now\n"),
    ] {
        for prefix in ["rustocop", "rubocop"] {
            fs::write(dir.path().join("rustocop.toml"), "").unwrap();
            for source in [
                format!("# {prefix}:disable {cop}\n{code}"),
                format!("{} # {prefix}:disable {cop}\n", code.trim()),
            ] {
                fs::write(&path, &source).unwrap();
                run(
                    dir.path(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    0,
                    [0, 0, 0],
                );
                assert_eq!(fs::read_to_string(&path).unwrap(), source);
            }
        }
        fs::write(&path, code).unwrap();
        for config in [
            format!("[cops.\"{cop}\"]\nexclude = ['lib/**']\n"),
            "exclude = ['lib/**']\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            run(
                &dir.path().join("lib"),
                &["--only", cop, "--fix", "a.rb"],
                0,
                [0, 0, 0],
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), code);
        }
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        let source = format!("# {prefix}:enable {DATE}\nDateTime.now\n");
        fs::write(&path, &source).unwrap();
        run(dir.path(), &["--fix", "lib/a.rb"], 1, [0, 0, 1]);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
    // An interior disabled line prevents an atomic multiline Fetch edit.
    let source = "ENV[\n 'TASK70_X' # rustocop:disable Style/FetchEnvVar\n]\n";
    fs::write(&path, source).unwrap();
    run(
        dir.path(),
        &["--only", FETCH, "--fix", "lib/a.rb"],
        0,
        [0, 0, 0],
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
}
#[test]
fn clean_error_unsafe_shadowed_and_non_utf8_files_keep_bytes_and_mtime() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for (cop, source, status, counts) in [
        (FETCH, b"ENV.fetch('X', nil)\n".as_slice(), 0, [0, 0, 0]),
        (FETCH, b"ENV[\n".as_slice(), 2, [0, 0, 0]),
        (
            FETCH,
            b"# encoding: ISO-8859-1\r\nENV['caf\xe9']\r\n__END__\r\nraw\xff\r\n".as_slice(),
            1,
            [0, 1, 0],
        ),
        (FETCH, b"ENV = Custom\nENV['X']\n".as_slice(), 1, [0, 1, 0]),
        (HOME, b"ENV['HOME']\n".as_slice(), 1, [1, 0, 0]),
        (
            DATE,
            b"DateTime.now\nthing.to_datetime\n".as_slice(),
            1,
            [0, 0, 2],
        ),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(
            dir.path(),
            &["--only", cop, "--fix", "a.rb"],
            status,
            counts,
        );
        assert_eq!(fs::read(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
fn has_ruby() -> bool {
    Command::new("ruby").arg("--version").output().is_ok()
}
fn ruby(dir: &Path, value: Option<&str>) -> Output {
    let mut command = Command::new("ruby");
    command
        .current_dir(dir)
        .arg("a.rb")
        .env_clear()
        .env("PATH", "/usr/bin:/bin");
    if let Some(value) = value {
        command.env("TASK70_X", value);
    }
    command.output().unwrap()
}
fn same(before: &Output, after: &Output) {
    assert!(before.status.success(), "{before:?}");
    assert_eq!(before.status.code(), after.status.code(), "{after:?}");
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
}
#[test]
fn nil_defaults_decoded_keys_coercion_evaluation_counts_scopes_and_control_flow_survive() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let source = "$events = []\ndef key; $events << :key; 'TASK70_X'; end\nclass Key\n def to_str; $events << :coerce; 'TASK70_X'; end\nend\ndef read_default(value = ENV['TASK70_X']); value; end\ndef escape; catch(:done) { ENV[throw(:done, :escaped)] }; end\ndef early; return ENV[key()]; end\nk = Key.new\nvalues = [ENV['TASK70_X'], ENV[\"TASK70_\\x58\"], ENV[%q(TASK70_X)], ENV[\"TASK70_#{'X'}\"], ENV[key()], ENV[k], read_default, -> { ENV['TASK70_X'] }.call, early, escape]\np [values, $events, k.class.name, __LINE__]\n";
    for value in [None, Some(""), Some("hello"), Some("é")] {
        fs::write(dir.path().join("a.rb"), source).unwrap();
        let before = ruby(dir.path(), value);
        let text = String::from_utf8_lossy(&before.stdout);
        assert!(text.contains("[:key, :coerce, :key]"), "{text}");
        if value.is_none() {
            assert!(
                text.starts_with("[[nil, nil, nil, nil, nil, nil, nil, nil, nil, :escaped]"),
                "{text}"
            );
        }
        run(
            dir.path(),
            &["--only", FETCH, "--fix", "a.rb"],
            0,
            [0, 0, 0],
        );
        let fixed = fs::read_to_string(dir.path().join("a.rb")).unwrap();
        assert_eq!(fixed.matches("ENV.fetch(").count(), 10);
        assert!(!fixed.contains("ENV["));
        same(&before, &ruby(dir.path(), value));
        let modified = fs::metadata(dir.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap();
        run(
            dir.path(),
            &["--only", FETCH, "--fix", "a.rb"],
            0,
            [0, 0, 0],
        );
        assert_eq!(
            fs::metadata(dir.path().join("a.rb"))
                .unwrap()
                .modified()
                .unwrap(),
            modified
        );
    }
}
#[test]
fn visible_method_and_constant_bindings_and_fallbacks_are_never_substituted() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    for source in [
        "ENV.define_singleton_method(:fetch) { |*args| :changed }; p ENV['TASK70_X']\n",
        "ENV.instance_eval('def fetch(*args); :changed; end'); p ENV['TASK70_X']\n",
        "ENV.singleton_class.class_eval { alias fetch object_id }; p ENV['TASK70_X']\n",
        "module M; ENV = {'TASK70_X' => :shadowed}; p ENV['TASK70_X']; end\n",
        "module M; refine ENV.singleton_class do; def fetch(*args); :changed; end; end; end; using M; p ENV['TASK70_X']\n",
        "$events = []; def helper; yield; end; p ENV[helper { $events << :block; break 'TASK70_X' }]; p $events\n",
        "def key(x); x; end; p ENV[key 'TASK70_X']\n",
    ] {
        fs::write(dir.path().join("a.rb"), source).unwrap();
        let before = ruby(dir.path(), None);
        run(
            dir.path(),
            &["--only", FETCH, "--fix", "a.rb"],
            1,
            [0, 1, 0],
        );
        assert_eq!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), source);
        same(&before, &ruby(dir.path(), None));
    }
    let source = "require 'date'\n$events = []; def fallback; $events << :fallback; :fallback; end\np ENV.fetch('HOME', fallback)\np ENV.fetch('HOME') { $events << :block; :block }\np ENV.fetch('HOME', nil) { $events << :block_with_default; :block }\np ENV['HOME']\np DateTime.now.class\np $events\n";
    fs::write(dir.path().join("a.rb"), source).unwrap();
    let before = ruby(dir.path(), None);
    assert!(
        String::from_utf8_lossy(&before.stdout)
            .contains("[:fallback, :block, :block_with_default]")
    );
    run(
        dir.path(),
        &["--only", "Style/EnvHome,Style/DateTime", "--fix", "a.rb"],
        1,
        [1, 0, 1],
    );
    assert_eq!(fs::read_to_string(dir.path().join("a.rb")).unwrap(), source);
    same(&before, &ruby(dir.path(), None));
}
#[test]
fn mixed_style_layout_and_home_detection_stabilize_without_unsafe_fixes() {
    let dir = project();
    let path = dir.path().join("a.rb");
    fs::write(&path, "def f\n  return (ENV['HOME']);  \nend\n").unwrap();
    let only = "Style/EnvHome,Style/FetchEnvVar,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Layout";
    for _ in 0..4 {
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", only, "--fix", "a.rb"])
            .output()
            .unwrap();
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "def f\n  ENV.fetch(\"HOME\", nil)\nend\n"
    );
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    run(dir.path(), &["--only", only, "--fix", "a.rb"], 1, [1, 0, 0]);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
