//! selectors/no-write policy and controlled regexp-state observations.
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
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
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
    let dir = project();
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
    let dir = project();
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
#[test]
fn safe_fix_is_canonical_while_unsafe_clean_and_error_files_keep_bytes_and_mtime() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "$custom = 1\nputs $$\nputs $1\n").unwrap();
    run(
        dir.path(),
        &["--only", ALL, "--fix", "lib/a.rb"],
        1,
        [1, 1, 0],
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "$custom = 1\nputs $$\nputs Regexp.last_match(1)\n"
    );
    for (source, status, counts) in [
        ("$custom = 1\nputs $$\n", 1, [1, 1, 0]),
        (
            "puts [$`, $', $MATCH, $PREMATCH, $POSTMATCH]\n",
            1,
            [0, 0, 5],
        ),
        (
            "puts [Regexp.last_match(1), $+, $LAST_PAREN_MATCH]\n",
            0,
            [0; 3],
        ),
        ("puts $1(\n", 2, [0; 3]),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(
            dir.path(),
            &["--only", ALL, "--fix", "lib/a.rb"],
            status,
            counts,
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
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
    let dir = project();
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
#[test]
fn runtime_nil_and_english_alias_hazards_are_reported_without_rewriting() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for source in [
        "p [$`, $', $MATCH, $PREMATCH, $POSTMATCH]\n",
        "/(a)/ =~ 'xa!'; p [$`, $', $MATCH, $PREMATCH, $POSTMATCH]\n",
        "require 'English'; /(a)/ =~ 'xa!'; p [$`, $', $MATCH, $PREMATCH, $POSTMATCH]\n",
        "$MATCH = 'changed'; /(a)/ =~ 'xa!'; p [$MATCH, $PREMATCH, $POSTMATCH, $`, $']\n",
    ] {
        fs::write(&path, source).unwrap();
        let before = ruby(dir.path());
        run(
            dir.path(),
            &["--only", PERL, "--fix", "lib/a.rb"],
            1,
            [0, 0, 5],
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        same(&before, &ruby(dir.path()));
    }
}
#[test]
fn runtime_visible_api_binding_visibility_refinement_eval_and_defined_hazards_remain_unedited() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (source, count) in [
        (
            "Regexp.define_singleton_method(:last_match) { |*| :custom }; /(a)/ =~ 'a'; p $1\n",
            1,
        ),
        (
            "Regexp.private_class_method(:last_match); /(a)/ =~ 'a'; p $1\n",
            1,
        ),
        (
            "Regexp.singleton_class.class_eval { alias last_match object_id }; /(a)/ =~ 'a'; p $1\n",
            1,
        ),
        (
            "Regexp.instance_eval('def last_match(*); :custom; end'); /(a)/ =~ 'a'; p $1\n",
            1,
        ),
        (
            "module M; refine Regexp.singleton_class do; def last_match(*); :custom; end; end; end; using M; /(a)/ =~ 'a'; p $1\n",
            1,
        ),
        (
            "Object.send(:remove_const, :Regexp); Regexp = Object.new; /(a)/ =~ 'a'; p $1\n",
            1,
        ),
        ("p defined?($1); /(a)/ =~ 'a'; p defined?($1)\n", 2),
        ("/(a)/ =~ 'a'; p $2147483648\n", 1),
        (
            "module M; Regexp = Object.new; end; M.module_eval { /(a)/ =~ 'a'; p $1 }\n",
            1,
        ),
    ] {
        fs::write(&path, source).unwrap();
        let before = ruby(dir.path());
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(
            dir.path(),
            &["--only", PERL, "--fix", "lib/a.rb"],
            1,
            [0, 0, count],
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        same(&before, &ruby(dir.path()));
    }
}
#[test]
fn runtime_object_namespace_and_method_object_mutations_remain_unedited() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for binding in [
        "class Object; Regexp = Object.new; end",
        "class ::Object; Regexp = Object.new; end",
        "class Object; Regexp &&= Object.new; end",
        "class ::Object; Regexp, x = [Object.new, nil]; end",
        "Regexp.method(:define_singleton_method).call(:last_match) { |*| :changed }",
        "Regexp.public_method('define_singleton_method')[:last_match] { |*| :changed }",
        "binder = Regexp.method(:define_singleton_method); binder.call(:last_match) { |*| :changed }",
        "Module.instance_method(:define_method).bind_call(Regexp.singleton_class, :last_match) { |*| :changed }",
        "Object.method(:const_set).call(:Regexp, Object.new)",
        "Regexp.singleton_class.method(:private).call(:last_match)",
    ] {
        let source = format!("{binding}; /(a)/ =~ 'a'; p $1\n");
        fs::write(&path, &source).unwrap();
        let before = ruby(dir.path());
        assert!(before.status.success(), "{source}: {before:?}");
        assert_eq!(before.stdout, b"\"a\"\n", "{source}");
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(
            dir.path(),
            &["--only", PERL, "--fix", "lib/a.rb"],
            1,
            [0, 0, 1],
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        same(&before, &ruby(dir.path()));
    }
}
#[test]
fn source_bytes_line_data_and_mixed_style_layout_composition_are_preserved() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let listed = "\u{feff}# encoding: UTF-8\r\n/(é)/ =~ 'é'\r\np [$1, \"#$1\", __LINE__, 'é'.encoding.name, DATA.read.bytes]\r\n__END__\r\nraw\0\r\n";
    // Reading `DATA` is a listed observer: that file keeps its
    // findings and bytes. The unread data section survives below.
    fs::write(&path, listed).unwrap();
    run(
        dir.path(),
        &["--only", PERL, "--fix", "lib/a.rb"],
        1,
        [0, 0, 2],
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), listed);
    let source = &listed.replace(", DATA.read.bytes]", "]");
    fs::write(&path, source).unwrap();
    let before = has_ruby().then(|| ruby(dir.path()));
    run(
        dir.path(),
        &["--only", PERL, "--fix", "lib/a.rb"],
        0,
        [0; 3],
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        source.replace(
            "[$1, \"#$1\"",
            "[Regexp.last_match(1), \"#{Regexp.last_match(1)}\""
        )
    );
    if let Some(before) = before {
        same(&before, &ruby(dir.path()));
    }
    let source = b"# encoding: ISO-8859-1\r\n# caf\xe9\r\nputs $1\r\n__END__\r\nraw\xff\r\n";
    fs::write(&path, source).unwrap();
    run(
        dir.path(),
        &["--only", PERL, "--fix", "lib/a.rb"],
        0,
        [0; 3],
    );
    assert_eq!(fs::read(&path).unwrap(), b"# encoding: ISO-8859-1\r\n# caf\xe9\r\nputs Regexp.last_match(1)\r\n__END__\r\nraw\xff\r\n");
    fs::write(&path, "def f\n  return [($1), {'x' => 1}];  \nend\n").unwrap();
    let only = "Style/PerlBackrefs,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Layout";
    for _ in 0..4 {
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", only, "--fix", "lib/a.rb"])
            .output()
            .unwrap();
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "def f\n  [Regexp.last_match(1), { \"x\" => 1 }]\nend\n"
    );
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    run(
        dir.path(),
        &["--only", only, "--fix", "lib/a.rb"],
        0,
        [0; 3],
    );
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
