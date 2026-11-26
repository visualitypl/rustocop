//! unsafe enumeration suggestions never change program behavior.
use crate::support::cli::project;
use crate::support::stdin;
use serde::Deserialize;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
const COPS: [&str; 3] = [
    "Style/MapToSet",
    "Style/PartitionInsteadOfDoubleSelect",
    "Style/ReduceToHash",
];
const SOURCES: [&str; 3] = [
    "array.map { it * 2 }.to_set\n",
    "arr.select(&:ok?)\narr.reject(&:ok?)\n",
    "array.each_with_object({}) { |e, h| h[e] = e.to_s }\n",
];
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
fn unchanged(dir: &Path, s: &[u8], args: &[&str], code: i32) -> Output {
    let p = dir.join("a.rb");
    fs::write(&p, s).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    let o = cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), s);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
    o
}
#[test]
fn defaults_versions_selectors_and_unsafe_capabilities() {
    let registry = rustocop::cops::Registry::new();
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        let c = registry.cops().iter().find(|c| c.name() == *cop).unwrap();
        assert!(c.default_enabled());
        assert!(!c.safe_fixes());
        assert!(c.path_policy().is_none());
        assert_eq!(
            c.ruby_versions().minimum,
            if i == 2 {
                Some(rustocop::cops::policy::RubyVersion(2, 6))
            } else {
                None
            }
        );
        assert_eq!(c.ruby_versions().maximum, None);
        let o = unchanged(d.path(), SOURCES[i].as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        unchanged(
            d.path(),
            SOURCES[i].as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            1,
        );
        unchanged(
            d.path(),
            SOURCES[i].as_bytes(),
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
    }
}
#[test]
fn config_enabling_disabling_excluding_and_strict_settings() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        for config in [
            "[cops.Style]\nenabled=false\n".into(),
            format!("[cops.\"{cop}\"]\nenabled=false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), SOURCES[i].as_bytes(), &["--fix", "a.rb"], 0);
            unchanged(
                d.path(),
                SOURCES[i].as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                1,
            );
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
        )
        .unwrap();
        unchanged(d.path(), SOURCES[i].as_bytes(), &["--fix", "a.rb"], 1);
        for config in [
            "exclude=[\"a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude=[\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                SOURCES[i].as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        for key in [
            "SafeAutoCorrect=true",
            "TargetRubyVersion=3.4",
            "EnforcedStyle=\"other\"",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{key}\n"),
            )
            .unwrap();
            unchanged(d.path(), SOURCES[i].as_bytes(), &["--fix", "a.rb"], 2);
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn directives_and_clean_error_files_do_not_write() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        for prefix in ["rubocop", "rustocop"] {
            for target in [*cop, cop.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for directive in ["disable", "todo"] {
                    let s = format!("# {prefix}:{directive} {target}\n{}", SOURCES[i]);
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
                }
                let s = format!("{} # {prefix}:disable {target}\n", SOURCES[i].trim_end());
                unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
            }
            let s = format!(
                "# {prefix}:disable {cop}\n{}separator\n# {prefix}:enable {cop}\n{}",
                SOURCES[i], SOURCES[i]
            );
            let o = unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
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
}
#[derive(Deserialize)]
struct Case {
    cop: String,
    spec_line: u32,
    source: String,
    positions: Vec<(u32, u32)>,
    correction: Option<String>,
}
#[test]
fn all_100_pinned_calls_and_all_64_unsafe_correction_calls() {
    // Ported from the three assigned RuboCop specs (MIT, (c) 2012-26 Bozhidar Batsov).
    // The native-invalid top-level yield example is wrapped in a def; both
    // Ruby-2.5-only unsupported-Prism negatives are deliberately excluded.
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/enumeration_sets_hashes_pinned.json")).unwrap();
    assert_eq!(cases.len(), 100);
    assert_eq!(cases.iter().filter(|c| c.correction.is_some()).count(), 64);
    let d = project();
    let registry = rustocop::cops::Registry::new();
    let config = rustocop::config::ResolvedConfig::default();
    for c in cases {
        let active =
            rustocop::cops::select_active(&registry, &config, std::slice::from_ref(&c.cop), &[]);
        let report = rustocop::pipeline::lint_one(
            Path::new("a.rb"),
            Path::new("a.rb"),
            Some(c.source.as_bytes()),
            &active,
            &config,
        );
        assert!(
            report.errors.is_empty() && report.internal_errors.is_empty(),
            "{}:{}",
            c.cop,
            c.spec_line
        );
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
        assert!(
            report
                .result
                .offenses
                .iter()
                .all(|o| o.correction.is_none())
        );
        unchanged(
            d.path(),
            c.source.as_bytes(),
            &["--only", &c.cop, "a.rb"],
            i32::from(!c.positions.is_empty()),
        );
        if c.correction.is_some() {
            unchanged(
                d.path(),
                c.source.as_bytes(),
                &["--only", &c.cop, "--fix", "a.rb"],
                1,
            );
        }
    }
    let invalid = "array.each_with_object({}) { |elem,hash| hash[elem.id] = yield elem }\n";
    let o = unchanged(
        d.path(),
        invalid.as_bytes(),
        &["--only", COPS[2], "--fix", "a.rb"],
        2,
    );
    assert!(String::from_utf8_lossy(&o.stdout).contains("Invalid yield"));
}
#[test]
fn source_envelopes_line_data_and_deferred_heredoc_membership() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        for prefix in [
            "",
            "\u{feff}",
            "# coding: UTF-8\n# café\n",
            "# frozen_string_literal: true\n",
        ] {
            let s = format!(
                "{prefix}{}p [__LINE__,__FILE__,DATA.read]\n__END__\nraw payload\n",
                SOURCES[i]
            );
            for s in [s.clone(), s.replace('\n', "\r\n")] {
                unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 1);
            }
        }
        let mut s = b"# coding: ISO-8859-1\n# caf\xe9\n".to_vec();
        s.extend_from_slice(SOURCES[i].as_bytes());
        unchanged(d.path(), &s, &["--only", cop, "--fix", "a.rb"], 1);
    }
    let s = "xs.each_with_object({}) { |x,h| h[x] = <<~TEXT }\n  #{h.size}\nTEXT\n";
    unchanged(
        d.path(),
        s.as_bytes(),
        &["--only", COPS[2], "--fix", "a.rb"],
        0,
    );
    let s = "xs.each_with_object({}) { |x,h| h[x] = <<~TEXT }\n  #{ys.reduce({}) { |a,y| a[y]=y; a }}\nTEXT\n";
    let o = unchanged(
        d.path(),
        s.as_bytes(),
        &["--only", COPS[2], "--fix", "a.rb"],
        1,
    );
    assert_eq!(
        String::from_utf8_lossy(&o.stdout)
            .matches(&format!("({})", COPS[2]))
            .count(),
        1
    );
}
#[test]
fn stdin_selection_and_fix_rejection_leave_disk_unchanged() {
    let d = project();
    for (i, cop) in COPS.iter().enumerate() {
        for fix in [false, true] {
            let p = d.path().join("a.rb");
            fs::write(&p, "disk bytes\n").unwrap();
            let time = fs::metadata(&p).unwrap().modified().unwrap();
            let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
            command
                .current_dir(d.path())
                .args(["--only", cop, "--stdin", "a.rb"]);
            if fix {
                command.arg("--fix");
            }
            let mut child = command
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
            assert_eq!(fs::read(&p).unwrap(), b"disk bytes\n");
            assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
        }
    }
}
#[test]
fn captured_bindings_arity_and_visibility_stay_unchanged() {
    let d = project();
    let sources = [
        (
            COPS[0],
            "require 'set'; class X; def map; [yield(1,2)]; end; def to_set; Set.new([yield(1)]); end; end; p X.new.map { |a,b| [a,b] }.to_set.to_a",
            "require 'set'; class X; def map; [yield(1,2)]; end; def to_set; Set.new([yield(1)]); end; end; p X.new.to_set { |a,b| [a,b] }.to_a",
        ),
        (
            COPS[1],
            "xs=[1,2]; n=0; captured=-> { n+=1 }; a=xs.select { |x| captured.call.odd? }; b=xs.reject { |x| captured.call.odd? }; p [a,b,n]",
            "xs=[1,2]; n=0; captured=-> { n+=1 }; a,b=xs.partition { |x| captured.call.odd? }; p [a,b,n]",
        ),
        (
            COPS[1],
            "class X; def select; [yield(1)]; end; def reject; [yield(1)]; end; private; def partition; end; end; xs=X.new; a=xs.select { |x| x.odd? }; b=xs.reject { |x| x.odd? }; p [a,b]",
            "class X; def select; [yield(1)]; end; def reject; [yield(1)]; end; private; def partition; end; end; xs=X.new; begin; a,b=xs.partition { |x| x.odd? }; rescue NoMethodError; puts :private; end",
        ),
        (
            COPS[2],
            "saved=nil; xs=[1]; result=xs.each_with_object({}) { |e,h| h[e]=(saved=binding; e) }; p saved.local_variable_get(:h).equal?(result)",
            "saved=nil; xs=[1]; result=xs.to_h { |e| [e,(saved=binding; e)] }; p saved.local_variable_defined?(:h)",
        ),
    ];
    for (cop, source, alternative) in sources {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            1,
        );
        for ruby in rubies() {
            assert_ne!(
                observe(&ruby, source),
                observe(&ruby, alternative),
                "{ruby}: {source}"
            );
        }
    }
}
#[test]
fn peer_and_actual_default_passes_preserve_observations_and_stabilize() {
    let source = "require 'set'\nxs = [1,2]\na = xs.map { |x| x * 2 }.to_set\nb = xs.select(&:odd?)\nc = xs.reject(&:odd?)\nd = xs.each_with_object({}) { |e,h| h[e] = e.to_s }\np [a.to_a,b,c,d]\n";
    let selected = format!(
        "{},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout",
        COPS.join(",")
    );
    for only in [Some(selected.as_str()), None] {
        let d = project();
        let path = d.path().join("a.rb");
        fs::write(&path, source).unwrap();
        let expected: Vec<_> = rubies().iter().map(|r| observe(r, source)).collect();
        let mut seen = std::collections::HashSet::new();
        let mut stable = false;
        for _ in 0..6 {
            let before = fs::read(&path).unwrap();
            assert!(seen.insert(before.clone()));
            let args = match only {
                Some(cops) => vec!["--only", cops, "--fix", "a.rb"],
                None => vec!["--fix", "a.rb"],
            };
            let o = cli(d.path(), &args, 1);
            let after = fs::read_to_string(&path).unwrap();
            for (i, ruby) in rubies().iter().enumerate() {
                assert_eq!(observe(ruby, &after), expected[i]);
            }
            for cop in COPS {
                assert!(String::from_utf8_lossy(&o.stdout).contains(cop), "{o:?}");
            }
            if before == after.as_bytes() {
                stable = true;
                break;
            }
        }
        assert!(stable);
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
fn observe(ruby: &str, source: &str) -> Vec<u8> {
    let mut command = Command::new(ruby);
    // miniruby has no installed standard-library path. Set is pure Ruby;
    // use its matching source-tree library when running the native 3.4 gate.
    if let Some(parent) = Path::new(ruby).parent() {
        let lib = parent.join("lib");
        if lib.join("set.rb").is_file() {
            command.arg("-I").arg(lib);
        }
    }
    let o = command.args(["-e", source]).output().unwrap();
    assert!(o.status.success() && o.stderr.is_empty(), "{source}: {o:?}");
    o.stdout
}
#[test]
fn controlled_binding_evaluation_control_flow_arity_lazy_visibility_and_equality_witnesses() {
    // Only these controlled programs execute. Each plausible rewrite changes
    // observations, explaining why a parseable correction is not a safe fix.
    let witnesses = [
        (
            COPS[0],
            "require 'set'; xs=[1,2]; p xs.map { |x| break 7 }.to_set rescue puts :no_set",
            "require 'set'; xs=[1,2]; p xs.to_set { |x| break 7 }",
        ),
        (
            COPS[0],
            "require 'set'; $trace=[]; class K; def hash; $trace << :hash; 0; end; def eql?(x); $trace << :equal; true; end; end; xs=[K.new,K.new]; xs.map { |x| $trace << :map; x }.to_set; p $trace",
            "require 'set'; $trace=[]; class K; def hash; $trace << :hash; 0; end; def eql?(x); $trace << :equal; true; end; end; xs=[K.new,K.new]; xs.to_set { |x| $trace << :map; x }; p $trace",
        ),
        (
            COPS[0],
            "require 'set'; class X; def map; [yield(1,2)]; end; private; def to_set; end; end; p X.new.map { |a,b| [a,b] }.to_set.to_a",
            "require 'set'; class X; def map; [yield(1,2)]; end; private; def to_set; end; end; begin; p X.new.to_set { |a,b| [a,b] }; rescue NoMethodError; puts :private; end",
        ),
        (
            COPS[1],
            "trace=[]; xs=[1,2]; a=xs.select { |x| trace << x; true }; b=xs.reject { |x| trace << x; true }; p trace",
            "trace=[]; xs=[1,2]; a,b=xs.partition { |x| trace << x; true }; p trace",
        ),
        (
            COPS[1],
            "def factory; $calls+=1; [1,2]; end; $calls=0; a=factory.select(&:odd?); b=factory.reject(&:odd?); p $calls",
            "def factory; $calls+=1; [1,2]; end; $calls=0; a,b=factory.partition(&:odd?); p $calls",
        ),
        (
            COPS[1],
            "xs={a:1,b:2}; a=xs.select { |k,v| v.odd? }; b=xs.reject { |k,v| v.odd? }; p [a.class,b.class]",
            "xs={a:1,b:2}; a,b=xs.partition { |k,v| v.odd? }; p [a.class,b.class]",
        ),
        (
            COPS[1],
            "xs=[1,2].lazy; a=xs.select(&:odd?); b=xs.reject(&:odd?); p [a.class,b.class]",
            "xs=[1,2].lazy; a,b=xs.partition(&:odd?); p [a.class,b.class]",
        ),
        (
            COPS[2],
            "class X; def each_with_object(h); yield(1,2,h); h; end; def to_h; [yield(1)].to_h; end; end; p X.new.each_with_object({}) { |e,h| h[e]=true } rescue puts :arity",
            "class X; def each_with_object(h); yield(1,2,h); h; end; def to_h; [yield(1)].to_h; end; end; p X.new.to_h { |e| [e,true] }",
        ),
        (
            COPS[2],
            "trace=[]; xs=[1,2]; p xs.each_with_object({}) { |e,h| h[e]=begin; next if e==1; trace << e; e; end }; p trace",
            "trace=[]; xs=[1,2]; p xs.to_h { |e| [e,begin; next if e==1; trace << e; e; end] } rescue puts :next; p trace",
        ),
        (
            COPS[2],
            "class X; def each_with_object(h); yield(1,h); h; end; private; def to_h; end; end; p X.new.each_with_object({}) { |e,h| h[e]=true }",
            "class X; def each_with_object(h); yield(1,h); h; end; private; def to_h; end; end; begin; p X.new.to_h { |e| [e,true] }; rescue NoMethodError; puts :private; end",
        ),
    ];
    let d = project();
    for (cop, source, alternative) in witnesses {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            1,
        );
        for ruby in rubies() {
            assert_ne!(
                observe(&ruby, source),
                observe(&ruby, alternative),
                "{ruby}: {source}"
            );
        }
    }
}
