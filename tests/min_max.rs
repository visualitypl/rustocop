//! Only controlled Ruby programs are executed, never corpus code.
use crate::support::stdin;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 3] = [
    "Style/MinMax",
    "Style/MinMaxComparison",
    "Style/RedundantMinMaxBy",
];
const SOURCES: [&str; 3] = [
    "[[3, 1, 2].min, [3, 1, 2].max]\n",
    "a > b ? a : b\n",
    "[3, 1, 2].min_by { |x| x }\n",
];
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(dir: &Path, args: &[&str], code: i32) -> Output {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(code), "{args:?}: {o:?}");
    assert!(o.stderr.is_empty() || code == 2, "{o:?}");
    o
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: i32) -> Output {
    let p = dir.join("a.rb");
    fs::write(&p, source).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    let o = cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), source);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
    o
}
#[test]
fn fixed_defaults_capabilities_and_default_cli_detection() {
    let r = rustocop::cops::Registry::new();
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        let c = r.cops().iter().find(|c| c.name() == *cop).unwrap();
        assert!(c.default_enabled());
        assert_eq!(c.safe_fixes(), i != 1);
        assert_eq!(c.ruby_versions(), Default::default());
        assert!(c.path_policy().is_none());
        let o = unchanged(d.path(), SOURCES[i].as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
    }
}
#[test]
fn selectors_config_exclusions_and_rejected_style_options() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        let s = SOURCES[i].as_bytes();
        unchanged(d.path(), s, &["--only", cop, "a.rb"], 1);
        unchanged(
            d.path(),
            s,
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            "[cops.Style]\nenabled=false\n".into(),
            format!("[cops.\"{cop}\"]\nenabled=false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s, &["--fix", "a.rb"], 0);
            unchanged(d.path(), s, &["--only", cop, "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
        )
        .unwrap();
        assert!(
            String::from_utf8_lossy(&unchanged(d.path(), s, &["a.rb"], 1).stdout).contains(cop)
        );
        for config in [
            "exclude=[\"a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude=[\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s, &["--only", cop, "--fix", "a.rb"], 0);
        }
        for knob in [
            "EnforcedStyle=\"other\"",
            "SafeAutoCorrect=true",
            "TargetRubyVersion=3.4",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{knob}\n"),
            )
            .unwrap();
            unchanged(d.path(), s, &["--fix", "a.rb"], 2);
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn directives_and_clean_error_files_never_write() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        for prefix in ["rubocop", "rustocop"] {
            for target in [*cop, cop.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for s in [
                    format!("# {prefix}:disable {target}\n{}", SOURCES[i]),
                    format!("# {prefix}:disable-next {target}\n{}", SOURCES[i]),
                    format!("{} # {prefix}:disable {target}\n", SOURCES[i].trim_end()),
                ] {
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
                }
            }
            let s = format!(
                "# {prefix}:disable {cop}\n{}# {prefix}:enable {cop}\n{}",
                SOURCES[i], SOURCES[i]
            );
            let o = unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
            assert_eq!(
                String::from_utf8_lossy(&o.stdout)
                    .matches(&format!("({cop})"))
                    .count(),
                1
            );
        }
        for (s, code) in [("p :clean\n", 0), ("(\n", 2)] {
            unchanged(
                d.path(),
                s.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                code,
            );
        }
    }
    unchanged(
        d.path(),
        b"[1].min_by do |x|\n x # rustocop:disable Style/RedundantMinMaxBy\nend\n",
        &["--only", COPS[2], "--fix", "a.rb"],
        0,
    );
}
#[test]
fn stdin_is_read_only_and_fix_is_rejected() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        for fix in [false, true] {
            fs::write(d.path().join("a.rb"), "disk bytes\n").unwrap();
            let mut c = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            c.current_dir(d.path())
                .args(["--only", cop, "--stdin", "a.rb"]);
            if fix {
                c.arg("--fix");
            }
            let mut child = c
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            stdin::write_stdin(child.stdin.take().unwrap(), SOURCES[i].as_bytes(), fix).unwrap();
            let o = child.wait_with_output().unwrap();
            assert_eq!(o.status.code(), Some(if fix { 2 } else { 1 }));
            if fix {
                assert!(String::from_utf8_lossy(&o.stderr).contains("cannot be used with '--fix'"));
            } else {
                assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
            }
            assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), b"disk bytes\n");
        }
    }
}
#[derive(serde::Deserialize)]
struct Case {
    cop: String,
    spec_line: u32,
    source: String,
    positions: Vec<(u32, u32)>,
    messages: Vec<String>,
    correction: Option<String>,
}
#[test]
fn all_64_pinned_examples_and_41_correction_calls() {
    // RuboCop assigned specs, MIT, (c) 2012-26 Bozhidar Batsov.
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/min_max_pinned.json")).unwrap();
    assert_eq!(cases.len(), 64);
    assert_eq!(cases.iter().filter(|c| c.correction.is_some()).count(), 41);
    let d = project();
    let r = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    let mut adapted = 0;
    for c in cases {
        let active = rustocop::cops::select_active(&r, &config, std::slice::from_ref(&c.cop), &[]);
        let report = rustocop::pipeline::lint_one(
            Path::new("a.rb"),
            Path::new("a.rb"),
            Some(c.source.as_bytes()),
            &active,
            &config,
        );
        assert!(report.errors.is_empty() && report.internal_errors.is_empty());
        assert_eq!(
            report
                .result
                .offenses
                .iter()
                .map(|o| (o.line, o.col))
                .collect::<Vec<_>>(),
            c.positions,
            "{}:{}",
            c.cop,
            c.spec_line
        );
        assert_eq!(
            report
                .result
                .offenses
                .iter()
                .map(|o| o.message.clone())
                .collect::<Vec<_>>(),
            c.messages,
            "{}:{}",
            c.cop,
            c.spec_line
        );
        unchanged(
            d.path(),
            c.source.as_bytes(),
            &["--only", &c.cop, "a.rb"],
            i32::from(!c.positions.is_empty()),
        );
        if let Some(after) = c.correction {
            // Every original safe-rule example uses an unknown `foo`/`array`
            // receiver. Preserve dispatch/evaluation; do not hide its offense.
            unchanged(
                d.path(),
                c.source.as_bytes(),
                &["--only", &c.cop, "--fix", "a.rb"],
                1,
            );
            if c.cop != COPS[1] {
                let name = if c.cop == COPS[0] { "foo" } else { "array" };
                fs::write(d.path().join("a.rb"), c.source.replace(name, "[3,1,2]")).unwrap();
                cli(d.path(), &["--only", &c.cop, "--fix", "a.rb"], 0);
                assert_eq!(
                    fs::read_to_string(d.path().join("a.rb")).unwrap(),
                    after.replace(name, "[3,1,2]")
                );
                adapted += 1;
            }
        }
    }
    assert_eq!(adapted, 27);
}
fn rubies() -> Vec<String> {
    let mut r = vec![];
    if Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        r.push("ruby".into());
    }
    if let Ok(p) = crate::support::runtime::ruby() {
        let o = Command::new(&p)
            .args(["-e", "print RUBY_VERSION"])
            .output()
            .unwrap();
        assert!(
            o.status.success()
                && o.stdout
                    .starts_with(crate::support::runtime::version_prefix().as_bytes())
        );
        r.push(p);
    }
    r
}
fn observe(ruby: &str, dir: &Path) -> Vec<u8> {
    let o = Command::new(ruby)
        .current_dir(dir)
        .arg("a.rb")
        .output()
        .unwrap();
    assert!(o.status.success(), "{ruby}: {o:?}");
    assert!(o.stderr.is_empty(), "{o:?}");
    o.stdout
}
#[test]
fn canonical_fixes_preserve_numeric_values_identity_binding_and_control_flow() {
    let d = project();
    let s = "a = [3,1,2].min, [3,1,2].max\nb = [3,1,2].min_by { |x| x }\nc = [3,1,2].max_by { _1 }\nd = [3,1,2].minmax_by do |item|\n item\nend\np [a, b, c, d, b.equal?(1), c.equal?(3)]\n";
    fs::write(d.path().join("a.rb"), s).unwrap();
    let expected = b"[[1, 3], 1, 3, [1, 3], true, true]\n";
    for ruby in rubies() {
        assert_eq!(observe(&ruby, d.path()), expected);
    }
    cli(d.path(), &["--only", &COPS.join(","), "--fix", "a.rb"], 0);
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert!(!after.contains("_by"));
    assert!(after.contains("a = [3,1,2].minmax"));
    for ruby in rubies() {
        assert_eq!(observe(&ruby, d.path()), expected);
    }
    unchanged(
        d.path(),
        after.as_bytes(),
        &["--only", &COPS.join(","), "--fix", "a.rb"],
        0,
    );
    // Keep the return keyword and script exit, not merely a parseable expression.
    let source = "return [3,1].min, [3,1].max\nraise 'unreachable'\n";
    fs::write(d.path().join("a.rb"), source).unwrap();
    let return_output = |ruby: &str| {
        let o = Command::new(ruby)
            .current_dir(d.path())
            .arg("a.rb")
            .output()
            .unwrap();
        assert!(o.status.success() && o.stdout.is_empty(), "{o:?}");
        o.stderr // Ruby 3.4 warns that a top-level return value is ignored.
    };
    let original: Vec<_> = rubies().iter().map(|r| return_output(r)).collect();
    cli(d.path(), &["--only", COPS[0], "--fix", "a.rb"], 0);
    assert_eq!(
        fs::read_to_string(d.path().join("a.rb")).unwrap(),
        "return [3,1].minmax\nraise 'unreachable'\n"
    );
    assert_eq!(
        rubies()
            .iter()
            .map(|r| return_output(r))
            .collect::<Vec<_>>(),
        original
    );
}
#[test]
fn comparator_key_evaluation_unordered_values_and_custom_identity_never_change() {
    let d = project();
    let cases = [
        (
            COPS[0],
            "log=[]; make=-> { log << :receiver; [3,1] }; result=[make.call.min, make.call.max]; p [result,log]\n",
            "[[1, 3], [:receiver, :receiver]]\n",
        ),
        (
            COPS[0],
            "class Custom; def min; :low; end; def max; :high; end; def minmax; raise; end; end\nx=Custom.new; p [x.min,x.max]\n",
            "[:low, :high]\n",
        ),
        (
            COPS[2],
            "class Custom; def min_by; p [:key,yield(2),yield(1)]; :result; end; def min; raise; end; end\np Custom.new.min_by { |x| x }\n",
            "[:key, 2, 1]\n:result\n",
        ),
        (
            COPS[2],
            "class Key; def <=>(other); 0; end; end\na=Key.new; b=Key.new; x=[a,b]; p x.min_by { |v| v }.equal?(a)\n",
            "true\n",
        ),
        (
            COPS[2],
            "p [nil].min_by { |x| x }; begin; [Float::NAN,1].max_by { |x| x }; rescue ArgumentError; puts :unordered; end\n",
            "nil\nunordered\n",
        ),
        (
            COPS[1],
            "log=[]; a=-> { log << :a; 2 }; b=-> { log << :b; 1 }; result=a.call > b.call ? a.call : b.call; p [result,log]\n",
            "[2, [:a, :b, :a]]\n",
        ),
        (
            COPS[1],
            "class OnlyGreater; def >(other); true; end; end\na=OnlyGreater.new; b=OnlyGreater.new; p((a > b ? a : b).equal?(a))\n",
            "true\n",
        ),
        (COPS[1], "a=Float::NAN; b=1; p(a > b ? a : b)\n", "1\n"),
        (
            COPS[2],
            "class Array; private :min; end\np [1,2].min_by { |x| x }\n",
            "1\n",
        ),
        (
            COPS[0],
            "class Key; def initialize(log); @log=log; end; def <=>(other); @log << :compare; 0; end; end\nlog=[]; xs=[Key.new(log),Key.new(log),Key.new(log)]; pair=[xs.min,xs.max]; p [pair.first.equal?(xs.first),pair.last.equal?(xs.first),log.length]\n",
            "[true, true, 4]\n",
        ),
        (
            COPS[0],
            "class Integer; def <=>(other); 0; end; end\np [[3,1].min,[3,1].max]\n",
            "[3, 3]\n",
        ),
    ];
    for (cop, s, expected) in cases {
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
        for ruby in rubies() {
            assert_eq!(observe(&ruby, d.path()), expected.as_bytes(), "{s}");
        }
    }
    let s = "log=[]; p [3,1].min_by { |x| log << x; -x }; p log\n";
    unchanged(
        d.path(),
        s.as_bytes(),
        &["--only", COPS[2], "--fix", "a.rb"],
        0,
    );
    for ruby in rubies() {
        assert_eq!(observe(&ruby, d.path()), b"3\n[3, 1]\n");
    }
}
#[test]
fn source_envelopes_comments_lines_data_and_encoding() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        let s = format!(
            "\u{feff}# encoding: UTF-8\r\n{}p [__LINE__, DATA.pos, DATA.read]\r\n__END__\r\nraw\0\r\n",
            SOURCES[i].replace('\n', "\r\n")
        );
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
    }
    for (cop, s) in [
        (COPS[0], "[[3,1].min, # retained\n [3,1].max]\n"),
        (COPS[0], "[[3,1].min,\n [3,1].max]; p __LINE__\n"),
        (COPS[2], "[1].min_by { |x| x # retained\n}\n"),
        (COPS[2], "[1].min_by do |x|\n x\nend\np __LINE__\n"),
    ] {
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
    }
    for (cop, before, after) in [
        (COPS[0], SOURCES[0], "[3, 1, 2].minmax\n"),
        (COPS[2], SOURCES[2], "[3, 1, 2].min\n"),
    ] {
        for header in [
            b"\xef\xbb\xbf# encoding: UTF-8\r\n".as_slice(),
            b"# encoding: ISO-8859-1\r\n# caf\xe9\r\n",
        ] {
            let mut s = header.to_vec();
            s.extend(before.replace('\n', "\r\n").as_bytes());
            s.extend(b"p __LINE__\r\n__END__\r\nraw\0\r\n");
            let mut expected = header.to_vec();
            expected.extend(after.replace('\n', "\r\n").as_bytes());
            expected.extend(b"p __LINE__\r\n__END__\r\nraw\0\r\n");
            fs::write(d.path().join("a.rb"), s).unwrap();
            cli(d.path(), &["--only", cop, "--fix", "a.rb"], 0);
            assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), expected);
        }
    }
}
#[test]
fn independent_safe_siblings_and_peer_layout_composition_stabilize() {
    let d = project();
    let s = "a = [[3,1].min, [3,1].max]; b = [3,1].min_by { |x| x }; p [a,b,'ok']\n";
    for selector in [
        COPS.join(","),
        format!(
            "{},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException",
            COPS.join(",")
        ),
        "Style,Layout".into(),
        String::new(),
    ] {
        fs::write(d.path().join("a.rb"), s).unwrap();
        let mut stable = false;
        for _ in 0..6 {
            let before = fs::read(d.path().join("a.rb")).unwrap();
            let mut args = vec![];
            if !selector.is_empty() {
                args.extend(["--only", selector.as_str()]);
            }
            args.extend(["--fix", "a.rb"]);
            let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(args)
                .output()
                .unwrap();
            assert!(matches!(o.status.code(), Some(0 | 1)), "{o:?}");
            assert!(o.stderr.is_empty(), "{o:?}");
            for ruby in rubies() {
                assert_eq!(observe(&ruby, d.path()), b"[[1, 3], 1, \"ok\"]\n");
            }
            if before == fs::read(d.path().join("a.rb")).unwrap() {
                stable = true;
                break;
            }
        }
        assert!(stable);
        let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
        assert!(after.contains("minmax"));
        assert!(!after.contains("min_by"));
    }
    let s = "[xs.min,xs.max]\n[[3,1].min,[3,1].max]\n[3,1].min_by { it }\n";
    fs::write(d.path().join("a.rb"), s).unwrap();
    cli(d.path(), &["--only", &COPS.join(","), "--fix", "a.rb"], 1);
    // Unknown receiver calls may mutate core dispatch before the safe siblings;
    // reversing their source order makes the independent edits provable.
    let s = "[[3,1].min,[3,1].max]\n[3,1].min_by { it }\n[xs.min,xs.max]\n";
    fs::write(d.path().join("a.rb"), s).unwrap();
    cli(d.path(), &["--only", &COPS.join(","), "--fix", "a.rb"], 1);
    assert_eq!(
        fs::read_to_string(d.path().join("a.rb")).unwrap(),
        "[3,1].minmax\n[3,1].min\n[xs.min,xs.max]\n"
    );
}
