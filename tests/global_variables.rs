//! selectors/no-write policy and controlled regexp-state observations.
use crate::support::cli::project_with_lib;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const COPS: [&str; 3] = [
    "Style/GlobalVars",
    "Style/SpecialGlobalVars",
    "Style/PerlBackrefs",
];
const ALL: &str = "Style/GlobalVars,Style/SpecialGlobalVars,Style/PerlBackrefs";
const PERL: &str = "Style/PerlBackrefs";
fn run(dir: &Path, args: &[&str], status: i32, counts: [usize; 3]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    for (cop, count) in COPS.into_iter().zip(counts) {
        assert_eq!(
            text.lines().filter(|line| line.contains(cop)).count(),
            count,
            "{args:?}: {text}"
        );
    }
    out
}
#[test]
fn defaults_only_except_config_and_lint_without_fix_never_write() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    let source = "/(a)/ =~ \"a\"\n$custom = 1\nputs $:\nputs $1\n";
    fs::write(&path, source).unwrap();
    run(dir.path(), &["lib/a.rb"], 1, [1, 1, 1]);
    for (index, cop) in COPS.into_iter().enumerate() {
        let mut only = [0; 3];
        only[index] = 1;
        let mut except = [1; 3];
        except[index] = 0;
        run(dir.path(), &["--only", cop, "lib/a.rb"], 1, only);
        run(dir.path(), &["--except", cop, "lib/a.rb"], 1, except);
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
        )
        .unwrap();
        run(dir.path(), &["lib/a.rb"], 1, except);
        run(dir.path(), &["--only", cop, "lib/a.rb"], 1, only);
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        run(dir.path(), &["lib/a.rb"], 1, only);
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
    fs::write(
        dir.path().join("rustocop.toml"),
        "[cops.Style]\nenabled = false\n",
    )
    .unwrap();
    run(dir.path(), &["lib/a.rb"], 0, [0; 3]);
    run(dir.path(), &["--only", ALL, "lib/a.rb"], 1, [1; 3]);
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    run(
        dir.path(),
        &["--only", ALL, "--except", ALL, "--fix", "lib/a.rb"],
        0,
        [0; 3],
    );
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
}
#[test]
fn directives_and_config_relative_excludes_suppress_findings_and_writes() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    for (cop, code) in COPS.into_iter().zip(["$custom = 1", "puts $$", "puts $1"]) {
        for prefix in ["rustocop", "rubocop"] {
            fs::write(dir.path().join("rustocop.toml"), "").unwrap();
            for source in [
                format!("# {prefix}:disable {cop}\n{code}\n"),
                format!("{code} # {prefix}:disable {cop}\n"),
                format!("# {prefix}:disable-next {cop}\n{code}\n"),
            ] {
                fs::write(&path, &source).unwrap();
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                run(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0, [0; 3]);
                assert_eq!(fs::read_to_string(&path).unwrap(), source);
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            }
        }
        for config in [
            format!("[cops.\"{cop}\"]\nexclude = ['lib/**']\n"),
            "exclude = ['lib/**']\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            fs::write(&path, code).unwrap();
            run(
                &dir.path().join("lib"),
                &["--only", cop, "--fix", "a.rb"],
                0,
                [0; 3],
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), code);
        }
    }
}
fn has_ruby() -> bool {
    Command::new("ruby").arg("--version").output().is_ok()
}
fn ruby(dir: &Path) -> Output {
    Command::new("ruby")
        .current_dir(dir)
        .arg("lib/a.rb")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap()
}
fn same(before: &Output, after: &Output) {
    assert!(before.status.success(), "{before:?}");
    assert_eq!(before.status.code(), after.status.code(), "{after:?}");
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
}
#[test]
fn runtime_nil_failure_optional_captures_method_block_thread_and_interpolation_state() {
    if !has_ruby() {
        return;
    }
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    for source in [
        "p [$1, $9, $&]\n",
        "/(a)(b)?/ =~ 'xa'; p [$1, $2, $9, $&, $+, __LINE__]; /z/ =~ 'xa'; p [$1, $&]\n",
        "def f(x = $1); p [x, $1]; /(b)/ =~ 'b'; p $1; end; /(a)/ =~ 'a'; f; p $1\n",
        "/(a)/ =~ 'a'; p [[1].map { $1 }, -> { $1 }.call, Thread.new { p [$1, $&]; /(b)/ =~ 'b'; [$1, $&] }.value, [$1, $&]]\n",
        "def `(x); x; end; /(a)/ =~ 'a'; p [\"#$1:#{$&}\", `#$1`, /#$1/.source, %W[#$1], %I[#$1]]\n",
        "module M; class Regexp; end; /(a)/ =~ 'a'; p [$1, $&]; end\n",
        "$events = []; def observe(x); $events << :observe; x; end; /(a)/ =~ 'a'; p [observe($1), catch(:done) { throw :done, $& }, -> { return $1 }.call, $events, __LINE__]\n",
    ] {
        fs::write(&path, source).unwrap();
        let before = ruby(dir.path());
        run(
            dir.path(),
            &["--only", PERL, "--fix", "lib/a.rb"],
            0,
            [0; 3],
        );
        let fixed = fs::read_to_string(&path).unwrap();
        assert_ne!(fixed, source);
        same(&before, &ruby(dir.path()));
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(
            dir.path(),
            &["--only", PERL, "--fix", "lib/a.rb"],
            0,
            [0; 3],
        );
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
