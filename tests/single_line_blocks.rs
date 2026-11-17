//! selectors, atomic byte safety, runtime observations and composition.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const PARAMS: &str = "Style/SingleLineBlockParams";
const DO_END: &str = "Style/SingleLineDoEndBlock";
const CHAIN: &str = "Style/MultilineBlockChain";
const ONLY: &str =
    "Style/SingleLineBlockParams,Style/SingleLineDoEndBlock,Style/MultilineBlockChain";
const CASES: [(&str, &str, &str); 3] = [
    (
        PARAMS,
        "xs.reduce { |memo, item| memo + item }\n",
        "xs.reduce { |acc, elem| acc + elem }\n",
    ),
    (DO_END, "foo do bar end\n", "foo do\n bar \nend\n"),
    (
        CHAIN,
        "foo do\n  bar\nend.map { it }\n",
        "foo do\n  bar\nend.map { it }\n",
    ),
];
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn cli(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], status: i32) {
    let path = dir.join("x.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let out = cli(dir, args);
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
}
#[test]
fn default_opt_in_only_except_and_config_precedence() {
    let dir = project();
    for (cop, before, after) in CASES {
        unchanged(dir.path(), before.as_bytes(), &["--only", cop, "x.rb"], 1);
        let default = cli(dir.path(), &["x.rb"]);
        assert_eq!(
            String::from_utf8_lossy(&default.stdout).contains(cop),
            cop != PARAMS,
            "{default:?}"
        );
        unchanged(
            dir.path(),
            before.as_bytes(),
            &[
                "--only",
                cop,
                "--except",
                cop,
                "--fix",
                "--fix-passes",
                "1",
                "x.rb",
            ],
            0,
        );
        for (config, args, expected) in [
            (
                "[cops.Style]\nenabled=false\n".into(),
                vec!["--fix", "--fix-passes", "1", "x.rb"],
                before,
            ),
            (
                format!("[cops.\"{cop}\"]\nenabled=false\n"),
                vec!["--only", cop, "--fix", "--fix-passes", "1", "x.rb"],
                after,
            ),
            (
                format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
                vec!["--fix", "--fix-passes", "1", "x.rb"],
                after,
            ),
        ] {
            // `{ it }` opens and closes on one line: the default-on
            // Layout/MultilineBlockLayout and BlockEndNewline leave it alone
            // although the chained call starts at `foo`, as the pinned Team
            // does (rubocop-ast `BlockNode#single_line?`).
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            fs::write(dir.path().join("x.rb"), before).unwrap();
            let out = cli(dir.path(), &args);
            assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
            assert_eq!(
                fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                expected,
                "{cop}"
            );
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn directives_excludes_and_noncorrectable_offenses_do_not_write() {
    let dir = project();
    for (cop, before, _) in CASES {
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\n{before}"),
                    format!("# {prefix}:disable-next {target}\n{before}"),
                ] {
                    unchanged(
                        dir.path(),
                        source.as_bytes(),
                        &["--only", cop, "--fix", "x.rb"],
                        0,
                    );
                }
            }
        }
        for config in [
            format!("[cops.\"{cop}\"]\nexclude=[\"x.rb\"]\n"),
            "exclude=[\"x.rb\"]\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                dir.path(),
                before.as_bytes(),
                &["--only", cop, "--fix", "x.rb"],
                0,
            );
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
    unchanged(
        dir.path(),
        CASES[2].1.as_bytes(),
        &["--only", CHAIN, "--fix", "x.rb"],
        1,
    );
}
#[test]
fn disabled_interior_spans_and_inline_directives_withhold_whole_edits() {
    let dir = project();
    for (cop, source, status) in [
        (
            PARAMS,
            "xs.reduce { |memo,item| <<~TEXT }\n#{memo+item # rubocop:disable Style/SingleLineBlockParams\n}\nTEXT\n",
            1,
        ),
        (
            PARAMS,
            "xs.reduce { |memo,item| memo+item } # rustocop:disable Style/SingleLineBlockParams\n",
            0,
        ),
        (
            DO_END,
            "foo do <<~TEXT end\n#{1 # rubocop:disable Style/SingleLineDoEndBlock\n}\nTEXT\n",
            1,
        ),
        (
            DO_END,
            "foo do bar end # rustocop:disable Style/SingleLineDoEndBlock\n",
            0,
        ),
    ] {
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "x.rb"],
            status,
        );
    }
}
#[test]
fn stdin_detection_and_fix_rejection_never_write() {
    let dir = project();
    let path = dir.path().join("x.rb");
    fs::write(&path, "on disk\n").unwrap();
    for (cop, before, _) in CASES {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", cop, "--stdin", "x.rb"])
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
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
        assert_eq!(
            cli(dir.path(), &["--only", cop, "--fix", "--stdin", "x.rb"])
                .status
                .code(),
            Some(2)
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "on disk\n");
    }
}
fn ruby(dir: &Path) -> Output {
    Command::new("ruby")
        .current_dir(dir)
        .arg("x.rb")
        .output()
        .unwrap()
}
#[test]
fn controlled_ruby_binding_evaluation_control_flow_arity_and_literal_values() {
    if Command::new("ruby").arg("--version").output().is_err() {
        eprintln!("Ruby unavailable: runtime check skipped");
        return;
    }
    let dir = project();
    for (cop, source) in [
        (
            PARAMS,
            "events=[]\np [1,2,3].reduce(0) { |memo,item| events << [memo,item]; memo += item; inner=->(memo) { memo*2 }; events << inner.call(item); memo }\np events\n",
        ),
        (
            PARAMS,
            "def f; [1,2].reduce(0) { |memo,item| return memo+item if item==2; next memo+item }; end\np f\np [1,2].reduce(0) { |elem,acc| break elem+acc if acc==2; elem+acc }\n",
        ),
        (
            PARAMS,
            "p [1,2].reduce(0) { |memo,item| capture=-> { memo += item }; capture.call; shadow=->(memo) { memo }; shadow.call(8); memo }\n",
        ),
        (
            DO_END,
            "events=[]\nf=->(a,b) do events << [a,b]; a+b end\ng=proc do |a,b| events << [a,b]; next a end\np [f.lambda?,f.arity,f.call(1,2),g.lambda?,g.arity,g.call([3,4])]\np events\n",
        ),
        (
            DO_END,
            "def f; [1,2].each do |x| return x+1 if x==2 end; end\np f\np [1,2].each do |x| break x*3 if x==2; next x end\n",
        ),
        (
            DO_END,
            "def outer(x); [x,block_given? ? yield : :none]; end\ndef inner; block_given? ? yield : 1; end\np outer inner do 9 end\n",
        ),
        (
            DO_END,
            "def foo; yield; end\npayload = foo do [<<~ONE, <<~TWO] end\n  one\nONE\n  two\nTWO\np payload\n",
        ),
        (
            DO_END,
            "p [1,2].map do |x| begin; raise 'x' if x==1; x; rescue StandardError; :rescued; ensure; $count=($count||0)+1; end end\np $count\n",
        ),
    ] {
        let path = dir.path().join("x.rb");
        fs::write(&path, source).unwrap();
        let before = ruby(dir.path());
        assert!(before.status.success(), "{source}: {before:?}");
        let fix = cli(dir.path(), &["--only", cop, "--fix", "x.rb"]);
        assert!(matches!(fix.status.code(), Some(0 | 1)), "{fix:?}");
        let changed = fs::read_to_string(&path).unwrap();
        assert_ne!(changed, source, "{cop}: {fix:?}");
        let after = ruby(dir.path());
        assert_eq!(
            (after.status.code(), after.stdout, after.stderr),
            (before.status.code(), before.stdout, before.stderr),
            "{source} => {changed}"
        );
    }
}
#[test]
fn pending_parameter_heredocs_preserve_literal_contents_and_block_return_values() {
    if Command::new("ruby").arg("--version").output().is_err() {
        eprintln!("Ruby unavailable: runtime check skipped");
        return;
    }
    let dir = project();
    for source in [
        "def foo; yield; end\nvalue = foo do |x=<<~TEXT| x end\n  hi\nTEXT\np value\n",
        "def foo; yield; end\nliterals=[]\nvalue = foo do |x=<<~TEXT| literals << x; x end\n  hi #{1 + 2}\nTEXT\np [literals, value]\n",
    ] {
        fs::write(dir.path().join("x.rb"), source).unwrap();
        let before = ruby(dir.path());
        assert!(before.status.success(), "{before:?}");
        let expected = if source.contains("literals=[]") {
            "[[\"hi 3\\n\"], \"hi 3\\n\"]\n"
        } else {
            "\"hi\\n\"\n"
        };
        assert_eq!(before.stdout, expected.as_bytes());
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", DO_END, "--fix", "x.rb"],
            1,
        );
        let after = ruby(dir.path());
        assert_eq!(
            (after.status.code(), after.stdout, after.stderr),
            (before.status.code(), before.stdout, before.stderr)
        );
    }
}
#[test]
fn deferred_def_receiver_collision_keeps_exact_bytes_mtime_and_ruby_effects() {
    let source = "$seen=[]\ndef acc;10;end\ndef build(s);$seen<<s;Object.new;end\nvalue=[1,2].reduce { |memo,item| def (build(<<~TXT)).tag;end; memo+item }\n#{acc}\nTXT\np [value,$seen]\n";
    let dir = project();
    let path = dir.path().join("x.rb");
    fs::write(&path, source).unwrap();
    let before = Command::new("ruby")
        .current_dir(dir.path())
        .arg("x.rb")
        .output()
        .ok();
    if let Some(before) = &before {
        assert!(before.status.success(), "{before:?}");
        assert_eq!(before.stdout, b"[3, [\"10\\n\"]]\n");
    }
    unchanged(
        dir.path(),
        source.as_bytes(),
        &["--only", PARAMS, "x.rb"],
        1,
    );
    unchanged(
        dir.path(),
        source.as_bytes(),
        &["--only", PARAMS, "--fix", "x.rb"],
        1,
    );
    if let Some(before) = before {
        let after = ruby(dir.path());
        assert_eq!(
            (after.status.code(), after.stdout, after.stderr),
            (before.status.code(), before.stdout.clone(), before.stderr)
        );
        // Show that the former correction really changes a lexical binding,
        // rather than relying only on parse success or a conservative no-op.
        let naive = source.replace("memo", "acc").replace("item", "elem");
        fs::write(&path, naive).unwrap();
        let naive = ruby(dir.path());
        assert!(naive.status.success(), "{naive:?}");
        assert_eq!(naive.stdout, b"[3, [\"1\\n\"]]\n");
        assert_ne!(naive.stdout, before.stdout);
    }
}
#[test]
fn all_header_kinds_with_deferred_interpolation_preserve_binding_and_effect_order() {
    let dir = project();
    let path = dir.path().join("x.rb");
    for (header, target) in [
        ("def (build(<<~TXT)).tag; end", "Object.new"),
        ("def (build(<<~TXT)).tag(acc); end", "Object.new"),
        ("def (build(<<~TXT)).tag = :body", "Object.new"),
        ("class << build(<<~TXT); end", "Object.new"),
        ("class C < build(<<~TXT); end", "Object"),
        ("class build(<<~TXT)::C; end", "Module.new"),
        ("module build(<<~TXT)::M; end", "Module.new"),
    ] {
        for (interpolation, value, event) in [
            ("acc", "10", "acc"),
            ("[0].map { elem }.first", "20", "elem"),
            ("-> { acc }.call", "10", "acc"),
            ("\"#{acc}\"", "10", "acc"),
        ] {
            let source = format!(
                "$seen=[]\n$events=[]\n$target={target}\ndef acc; $events<<:acc; 10; end\ndef elem; $events<<:elem; 20; end\ndef build(s); $events<<:build; $seen<<s; $target; end\nvalue=[1,2].reduce {{ |memo,item| {header}; memo+item }}\n#{{{interpolation}}}\nTXT\np [value,$seen,$events]\n"
            );
            fs::write(&path, &source).unwrap();
            let before = Command::new("ruby")
                .current_dir(dir.path())
                .arg("x.rb")
                .output()
                .ok();
            if let Some(before) = &before {
                assert!(before.status.success(), "{source}: {before:?}");
                assert_eq!(
                    before.stdout,
                    format!("[3, [\"{value}\\n\"], [:{event}, :build]]\n").as_bytes()
                );
            }
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", PARAMS, "--fix", "x.rb"],
                1,
            );
            if let Some(before) = before {
                let after = ruby(dir.path());
                assert_eq!(
                    (after.status.code(), after.stdout, after.stderr),
                    (before.status.code(), before.stdout.clone(), before.stderr)
                );
                fs::write(&path, source.replace("memo", "acc").replace("item", "elem")).unwrap();
                let naive = ruby(dir.path());
                assert!(naive.status.success(), "{source}: {naive:?}");
                assert_ne!(naive.stdout, before.stdout, "{source}");
            }
        }
    }
}
#[test]
fn captured_header_uses_and_independent_heredoc_siblings_still_correct() {
    let dir = project();
    let path = dir.path().join("x.rb");
    for (header, target) in [
        ("def (build(<<~TXT)).tag(acc); end", "Object.new"),
        ("class << build(<<~TXT); end", "Object.new"),
        ("class C < build(<<~TXT); end", "Object"),
        ("class build(<<~TXT)::C; end", "Module.new"),
        ("module build(<<~TXT)::M; end", "Module.new"),
    ] {
        for collision in [false, true] {
            let interpolation = if collision {
                "#{acc}"
            } else {
                "#{memo} #{[0].map { item }.first} #{->(memo) { memo }.call(item)}"
            };
            let source = format!(
                "$seen=[]\n$target={target}\ndef acc; 10; end\ndef build(s); $seen<<s; $target; end\nvalue=[1,2].reduce {{ |memo,item| {header}; memo+item }}\n{interpolation}\nTXT\np [value,$seen]\nsafe=[1,2].inject {{ |total,value| <<~SAFE }}\n#{{total+value}}\nSAFE\np safe\n"
            );
            let after = if collision {
                source.clone()
            } else {
                source
                    .replace("|memo,item|", "|acc, elem|")
                    .replace("memo+item", "acc+elem")
                    .replace("#{memo}", "#{acc}")
                    .replace("map { item }", "map { elem }")
                    .replace("call(item)", "call(elem)")
            }
            .replace("|total,value|", "|acc, elem|")
            .replace("total+value", "acc+elem");
            fs::write(&path, &source).unwrap();
            let before = Command::new("ruby")
                .current_dir(dir.path())
                .arg("x.rb")
                .output()
                .ok();
            if let Some(before) = &before {
                assert!(before.status.success(), "{source}: {before:?}");
                assert_eq!(
                    before.stdout,
                    if collision {
                        b"[3, [\"10\\n\"]]\n\"3\\n\"\n".as_slice()
                    } else {
                        b"[3, [\"1 2 2\\n\"]]\n\"3\\n\"\n".as_slice()
                    }
                );
            }
            let out = cli(dir.path(), &["--only", PARAMS, "--fix", "x.rb"]);
            assert_eq!(out.status.code(), Some(i32::from(collision)), "{out:?}");
            assert_eq!(fs::read(&path).unwrap(), after.as_bytes(), "{source}");
            let mtime = fs::metadata(&path).unwrap().modified().unwrap();
            let out = cli(dir.path(), &["--only", PARAMS, "--fix", "x.rb"]);
            assert_eq!(out.status.code(), Some(i32::from(collision)), "{out:?}");
            assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
            if let Some(before) = before {
                let after = ruby(dir.path());
                assert_eq!(
                    (after.status.code(), after.stdout, after.stderr),
                    (before.status.code(), before.stdout, before.stderr)
                );
            }
        }
    }
}
#[test]
fn definition_body_and_parameter_heredocs_are_closed_scopes_at_runtime() {
    let dir = project();
    let path = dir.path().join("x.rb");
    for (inner, invoke) in [
        ("def tag(x=<<~TXT); $seen<<x; end", "tag"),
        ("def tag; $seen<< <<~TXT; end", "tag"),
        ("def tag = $seen<< <<~TXT", "tag"),
        ("class C; $seen<< <<~TXT; end", ""),
        ("module M; $seen<< <<~TXT; end", ""),
        ("class << self; $seen<< <<~TXT; end", ""),
    ] {
        let source = format!(
            "$seen=[]\ndef acc; 10; end\ndef elem; 20; end\nvalue=[1,2].reduce {{ |memo,item| {inner}; memo+item }}\n#{{acc}} #{{[0].map {{ elem }}.first}}\nTXT\n{invoke}\np [value,$seen]\n"
        );
        let after = source
            .replace("|memo,item|", "|acc, elem|")
            .replace("memo+item", "acc+elem");
        fs::write(&path, &source).unwrap();
        let before = Command::new("ruby")
            .current_dir(dir.path())
            .arg("x.rb")
            .output()
            .ok();
        if let Some(before) = &before {
            assert!(before.status.success(), "{source}: {before:?}");
            assert_eq!(before.stdout, b"[3, [\"10 20\\n\"]]\n");
        }
        let out = cli(dir.path(), &["--only", PARAMS, "--fix", "x.rb"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
        if let Some(before) = before {
            let after = ruby(dir.path());
            assert_eq!(
                (after.status.code(), after.stdout, after.stderr),
                (before.status.code(), before.stdout, before.stderr)
            );
        }
    }
}
#[test]
fn simultaneous_header_and_parameter_heredocs_keep_separate_runtime_bindings() {
    let dir = project();
    let path = dir.path().join("x.rb");
    for nested in [false, true] {
        for collision in [false, true] {
            let value = if collision { "acc" } else { "memo+item" };
            let header = if nested {
                format!("#{{<<~INNER}}\n#{{{value}}}\nINNER")
            } else {
                format!("#{{{value}}}")
            };
            let source = format!(
                "$seen=[]\ndef acc; 10; end\ndef elem; 20; end\ndef build(s); $seen<<s; Object.new; end\nvalue=[1,2].reduce {{ |memo,item| def (build(<<~HEADER)).tag; end; def body(x=<<~PARAM); $seen<<x; end; memo+item }}\n{header}\nHEADER\n#{{-> {{ elem }}.call}}\nPARAM\nbody\np [value,$seen]\n"
            );
            let after = if collision {
                source.clone()
            } else {
                source
                    .replace("|memo,item|", "|acc, elem|")
                    .replace("memo+item", "acc+elem")
            };
            fs::write(&path, &source).unwrap();
            let before = Command::new("ruby")
                .current_dir(dir.path())
                .arg("x.rb")
                .output()
                .ok();
            if let Some(before) = &before {
                assert!(before.status.success(), "{source}: {before:?}");
                let value = if collision { "10" } else { "3" };
                let newline = if nested { "\\n\\n" } else { "\\n" };
                assert_eq!(
                    before.stdout,
                    format!("[3, [\"{value}{newline}\", \"20\\n\"]]\n").as_bytes()
                );
            }
            let out = cli(dir.path(), &["--only", PARAMS, "--fix", "x.rb"]);
            assert_eq!(out.status.code(), Some(i32::from(collision)), "{out:?}");
            assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
            if let Some(before) = before {
                let after = ruby(dir.path());
                assert_eq!(
                    (after.status.code(), after.stdout, after.stderr),
                    (before.status.code(), before.stdout, before.stderr)
                );
            }
        }
    }
}
#[test]
fn rename_collision_and_callback_metadata_guards_have_runtime_witnesses() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    for source in [
        "def acc; 40; end\np [1].reduce(0) { |memo,item| memo+item+acc }\n",
        "acc=40\np [1].reduce(0) { |memo,item| memo+item+acc }\n",
        "class X; def reduce(&callback); callback.parameters; end; end\np X.new.reduce { |memo,item| memo+item }\n",
        "p [1].reduce(0) { |memo,item| binding.local_variables }\n",
        "def acc; $target; end\n$target=Object.new\nmemo_obj=Object.new\n[1].reduce(memo_obj) { |memo,item| def acc.mark; :marked; end; memo }\np [$target.respond_to?(:mark),memo_obj.respond_to?(:mark)]\n",
    ] {
        fs::write(dir.path().join("x.rb"), source).unwrap();
        let before = ruby(dir.path());
        assert!(before.status.success());
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", PARAMS, "--fix", "x.rb"],
            1,
        );
        assert_eq!(ruby(dir.path()).stdout, before.stdout);
        let naive = source.replace("memo", "acc").replace("item", "elem");
        fs::write(dir.path().join("x.rb"), naive).unwrap();
        let after = ruby(dir.path());
        assert!(after.status.success());
        assert_ne!(after.stdout, before.stdout);
    }
}
#[test]
fn selected_and_default_style_layout_composition_stabilizes_without_cycles() {
    let dir = project();
    for only in [ONLY, "Style,Layout", "defaults"] {
        for source in [
            "f = lambda do |x| x+1 end\nxs = [1,2].reduce(0) do |memo,item| memo+item end\np xs\n",
            "def target(*args, **kwargs, &block); block.call(*args, **kwargs); end\np(target(1) do |x| x+1 end)\n",
        ] {
            fs::write(dir.path().join("x.rb"), source).unwrap();
            let before = Command::new("ruby")
                .current_dir(dir.path())
                .arg("x.rb")
                .output()
                .ok();
            let mut seen = vec![source.as_bytes().to_vec()];
            let mut stable = false;
            for _ in 0..6 {
                let out = if only == "defaults" {
                    cli(
                        dir.path(),
                        &["--except", "Lint,Naming,Security", "--fix", "x.rb"],
                    )
                } else {
                    cli(dir.path(), &["--only", only, "--fix", "x.rb"])
                };
                assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
                let bytes = fs::read(dir.path().join("x.rb")).unwrap();
                if bytes == *seen.last().unwrap() {
                    stable = true;
                    break;
                }
                assert!(!seen.contains(&bytes), "correction cycle: {seen:?}");
                seen.push(bytes);
            }
            assert!(stable, "{only}: {seen:?}");
            if let Some(before) = before {
                let after = ruby(dir.path());
                assert!(before.status.success(), "{before:?}");
                assert_eq!(
                    (after.status.code(), after.stdout, after.stderr),
                    (before.status.code(), before.stdout, before.stderr)
                );
            }
        }
    }
}
