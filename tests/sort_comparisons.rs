//! Pinned RuboCop specs (MIT, (c) 2012-26 Bozhidar Batsov).
//! Runtime witnesses execute only controlled fixtures, never corpus code.
use crate::support::stdin;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 3] = [
    "Style/RedundantSort",
    "Style/RedundantSortBy",
    "Style/ComparableBetween",
];
const SOURCES: [&str; 3] = [
    "[3, 1, 2].sort.first\n",
    "[3, 1, 2].sort_by { |x| x }\n",
    "x >= min && x <= max\n",
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
fn all_81_pinned_examples_and_63_correction_calls() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/sort_comparisons_pinned.json")).unwrap();
    assert_eq!(cases.len(), 81);
    assert_eq!(cases.iter().filter(|c| c.correction.is_some()).count(), 63);
    let r = rustocop::cops::Registry::new();
    let d = project();
    let config = rustocop::config::ResolvedConfig::default();
    let mut adapted = 0;
    for c in cases {
        assert!(r.has_cop(&c.cop), "{}", c.cop);
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
            // Unsafe rules never edit; each original safe-rule example has an
            // unproven `array` dispatch. Preserve offenses, do not hide them.
            unchanged(
                d.path(),
                c.source.as_bytes(),
                &["--only", &c.cop, "--fix", "a.rb"],
                1,
            );
            if c.cop == COPS[1] {
                fs::write(d.path().join("a.rb"), c.source.replace("array", "[3,1,2]")).unwrap();
                cli(d.path(), &["--only", &c.cop, "--fix", "a.rb"], 0);
                assert_eq!(
                    fs::read_to_string(d.path().join("a.rb")).unwrap(),
                    after.replace("array", "[3,1,2]")
                );
                adapted += 1;
            }
        }
    }
    assert_eq!(adapted, 8);
}
#[test]
fn fixed_defaults_and_default_cli_detection_without_writes() {
    let r = rustocop::cops::Registry::new();
    let d = project();
    for (i, name) in COPS.iter().enumerate() {
        let c = r.cops().iter().find(|c| c.name() == *name).unwrap();
        assert!(c.default_enabled());
        assert_eq!(c.safe_fixes(), *name == COPS[1]);
        assert_eq!(c.ruby_versions(), Default::default());
        assert!(c.path_policy().is_none());
        let o = unchanged(d.path(), SOURCES[i].as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(name));
    }
}
#[test]
fn selectors_config_excludes_and_strict_settings() {
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
        b"[1].sort_by do |x|\n x # rustocop:disable Style/RedundantSortBy\nend\n",
        &["--only", COPS[1], "--fix", "a.rb"],
        0,
    );
}
#[test]
fn stdin_read_only_and_fix_rejected() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        for fix in [false, true] {
            fs::write(d.path().join("a.rb"), "disk bytes\n").unwrap();
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            cmd.current_dir(d.path())
                .args(["--only", cop, "--stdin", "a.rb"]);
            if fix {
                cmd.arg("--fix");
            }
            let mut child = cmd
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
fn rubies() -> Vec<String> {
    let mut r = vec![];
    if Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        r.push("ruby".into());
    }
    if let Ok(p) = std::env::var("RUSTOCOP_RUBY34") {
        let o = Command::new(&p)
            .args(["-e", "print RUBY_VERSION"])
            .output()
            .unwrap();
        assert!(o.status.success() && o.stdout.starts_with(b"3.4."));
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
    assert!(o.status.success() && o.stderr.is_empty(), "{ruby}: {o:?}");
    o.stdout
}
#[test]
fn canonical_fixes_preserve_values_binding_identity_and_control_flow() {
    let d = project();
    let s = "x=7\na=[3,1,2].sort_by { |x| x }\nb=[3,1,1].sort_by { _1 }\nc=[-1,2,0].sort_by do |item|\n item\nend\np [a,b,c,x,a.first.equal?(1)]\n";
    fs::write(d.path().join("a.rb"), s).unwrap();
    for ruby in rubies() {
        assert_eq!(
            observe(&ruby, d.path()),
            b"[[1, 2, 3], [1, 1, 3], [-1, 0, 2], 7, true]\n"
        );
    }
    cli(d.path(), &["--only", COPS[1], "--fix", "a.rb"], 0);
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert_eq!(
        after,
        "x=7\na=[3,1,2].sort\nb=[3,1,1].sort\nc=[-1,2,0].sort\np [a,b,c,x,a.first.equal?(1)]\n"
    );
    for ruby in rubies() {
        assert_eq!(
            observe(&ruby, d.path()),
            b"[[1, 2, 3], [1, 1, 3], [-1, 0, 2], 7, true]\n"
        );
    }
    unchanged(
        d.path(),
        after.as_bytes(),
        &["--only", COPS[1], "--fix", "a.rb"],
        0,
    );
    // Identity block removal must retain the outer return and script exit.
    fs::write(
        d.path().join("a.rb"),
        "return [3,1].sort_by { |x| x }\nraise 'unreachable'\n",
    )
    .unwrap();
    let observation = |ruby: &str| {
        let o = Command::new(ruby)
            .current_dir(d.path())
            .arg("a.rb")
            .output()
            .unwrap();
        assert!(o.status.success() && o.stdout.is_empty(), "{o:?}");
        o.stderr
    };
    let before: Vec<_> = rubies().iter().map(|r| observation(r)).collect();
    cli(d.path(), &["--only", COPS[1], "--fix", "a.rb"], 0);
    assert_eq!(
        fs::read_to_string(d.path().join("a.rb")).unwrap(),
        "return [3,1].sort\nraise 'unreachable'\n"
    );
    assert_eq!(
        rubies().iter().map(|r| observation(r)).collect::<Vec<_>>(),
        before
    );
}
#[test]
fn comparator_key_evaluation_unordered_values_identity_and_dispatch_never_change() {
    let d = project();
    for (cop, s, expected) in [
        (
            COPS[0],
            "log=[]; xs=[3,1,2]; p xs.sort { |a,b| log << [a,b]; b <=> a }.last; p log.length\n",
            "1\n2\n",
        ),
        (
            COPS[0],
            "log=[]; p [3,1,2].sort_by { |x| log << x; -x }.first; p log\n",
            "3\n[3, 1, 2]\n",
        ),
        (
            COPS[0],
            "class MyString < String; end\nx=[MyString.new('test'),'test']; p [x.sort.last.class,x.max.class]\n",
            "[String, MyString]\n",
        ),
        (
            COPS[0],
            "p %w(dog horse mouse).sort_by { |w| w.length }.last\n",
            "\"mouse\"\n",
        ),
        (
            COPS[0],
            "begin; [Float::NAN,1].sort.first; rescue ArgumentError; puts :unordered; end\n",
            "unordered\n",
        ),
        (
            COPS[1],
            "class Custom; def sort_by; p [:key,yield(2),yield(1)]; :result; end; def sort; raise; end; end\np Custom.new.sort_by { |x| x }\n",
            "[:key, 2, 1]\n:result\n",
        ),
        (
            COPS[1],
            "class Key; def initialize(log); @log=log; end; def <=>(other); @log << :compare; 0; end; end\nlog=[]; xs=[Key.new(log),Key.new(log),Key.new(log)]; result=xs.sort_by { |x| x }; p [result.first.equal?(xs.first),result.last.equal?(xs.last),log.length]\n",
            "[true, true, 2]\n",
        ),
        (
            COPS[1],
            "p [nil].sort_by { it }; begin; [Float::NAN,1].sort_by { it }; rescue ArgumentError; puts :unordered; end\n",
            "[nil]\nunordered\n",
        ),
        (
            COPS[1],
            "class Array; private :sort; end\np [2,1].sort_by { |x| x }\n",
            "[1, 2]\n",
        ),
        (
            COPS[1],
            "class Integer; def <=>(other); 0; end; end\np [3,1].sort_by { |x| x }\n",
            "[3, 1]\n",
        ),
        (
            COPS[2],
            "log=[]; v=-> { log << :value; 2 }; lo=-> { log << :low; 1 }; hi=-> { log << :high; 3 }; p(v.call >= lo.call && v.call <= hi.call); p log\n",
            "true\n[:value, :low, :value, :high]\n",
        ),
        (
            COPS[2],
            "log=[]; v=-> { log << :value; 0 }; lo=-> { log << :low; 1 }; hi=-> { log << :high; 3 }; p(v.call >= lo.call && v.call <= hi.call); p log\n",
            "false\n[:value, :low]\n",
        ),
        (
            COPS[2],
            "class OnlyCompare; def >=(other); true; end; def <=(other); :truthy; end; end\nx=OnlyCompare.new; p(x >= 1 && x <= 3)\n",
            ":truthy\n",
        ),
        (COPS[2], "x=Float::NAN; p(x >= 1 && x <= 3)\n", "false\n"),
    ] {
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
        for ruby in rubies() {
            // Implicit it is Ruby 3.4-only; use the same explicit binding on host.
            if ruby == "ruby" && s.contains("{ it }") {
                fs::write(d.path().join("a.rb"), s.replace("{ it }", "{ |x| x }")).unwrap();
            } else {
                fs::write(d.path().join("a.rb"), s).unwrap();
            }
            assert_eq!(observe(&ruby, d.path()), expected.as_bytes(), "{s}");
        }
    }
}
#[test]
fn source_envelopes_comments_line_data_and_encoding() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        let s = format!(
            "\u{feff}# encoding: UTF-8\r\n{}p [__LINE__, DATA.pos, DATA.read]\r\n__END__\r\nraw\0\r\n",
            SOURCES[i].replace('\n', "\r\n")
        );
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
    }
    for s in [
        "[1].sort_by { |x| x # retained\n}\n",
        "[1].sort_by do |x|\n x\nend\np __LINE__\n",
        "[1].sort_by { it }; File.read(__FILE__)\n",
        "[1].sort_by { it }; binding\n",
    ] {
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", COPS[1], "--fix", "a.rb"],
            1,
        );
    }
    for header in [
        b"\xef\xbb\xbf# encoding: UTF-8\r\n".as_slice(),
        b"# encoding: ISO-8859-1\r\n# caf\xe9\r\n",
    ] {
        let mut s = header.to_vec();
        s.extend(SOURCES[1].replace('\n', "\r\n").as_bytes());
        s.extend(b"p __LINE__\r\n__END__\r\nraw\0\r\n");
        let mut expected = header.to_vec();
        expected.extend(b"[3, 1, 2].sort\r\np __LINE__\r\n__END__\r\nraw\0\r\n");
        fs::write(d.path().join("a.rb"), s).unwrap();
        cli(d.path(), &["--only", COPS[1], "--fix", "a.rb"], 0);
        assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), expected);
    }
}
#[test]
fn mixed_safe_unsafe_siblings_and_peer_layout_composition_stabilize() {
    let d = project();
    let s = "a=[3,1].sort_by { |x| x }; b=[2,1].sort_by { _1 }; p [a,b,'ok']\n";
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
            assert!(
                matches!(o.status.code(), Some(0 | 1)) && o.stderr.is_empty(),
                "{o:?}"
            );
            for ruby in rubies() {
                assert_eq!(observe(&ruby, d.path()), b"[[1, 3], [1, 2], \"ok\"]\n");
            }
            if before == fs::read(d.path().join("a.rb")).unwrap() {
                stable = true;
                break;
            }
        }
        assert!(stable);
        assert!(
            !fs::read_to_string(d.path().join("a.rb"))
                .unwrap()
                .contains("sort_by")
        );
    }
    // The unsafe endpoint finding remains after the independent safe key edit.
    fs::write(d.path().join("a.rb"), "[3,1].sort_by { it }.first\n").unwrap();
    cli(d.path(), &["--only", &COPS.join(","), "--fix", "a.rb"], 1);
    assert_eq!(
        fs::read_to_string(d.path().join("a.rb")).unwrap(),
        "[3,1].sort.first\n"
    );
}
