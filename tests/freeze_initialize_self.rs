//! real CLI policy, pinned corrections and controlled Ruby semantics.
//! Fixtures port RuboCop specs (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
use crate::support::cli::project;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const FREEZE: &str = "Style/RedundantFreeze";
const INIT: &str = "Style/RedundantInitialize";
const SELF: &str = "Style/RedundantSelf";
const ALL: &str = "Style/RedundantFreeze,Style/RedundantInitialize,Style/RedundantSelf";
const CASES: [(&str, &str); 3] = [
    (FREEZE, "1.freeze\n"),
    (INIT, "def initialize; end\n"),
    (SELF, "self.object_id\n"),
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
fn unchanged(dir: &Path, source: &[u8], args: &[&str], code: i32) -> Output {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let o = cli(dir, args, code);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), mtime);
    o
}
fn fixed(dir: &Path, cop: &str, before: &[u8], after: &[u8]) {
    unchanged(dir, before, &["--only", cop, "a.rb"], 1);
    cli(dir, &["--only", cop, "--fix", "a.rb"], 0);
    assert_eq!(
        fs::read(dir.join("a.rb")).unwrap(),
        crate::support::lf::lf(after)
    );
    unchanged(dir, after, &["--only", cop, "--fix", "a.rb"], 0);
}
#[test]
fn exact_registry_defaults_and_safe_capabilities() {
    let registry = rustocop::cops::Registry::new();
    for (name, _) in CASES {
        let cops: Vec<_> = registry
            .cops()
            .iter()
            .filter(|c| c.name() == name)
            .collect();
        assert_eq!(cops.len(), 1);
        assert!(cops[0].default_enabled());
        assert_eq!(cops[0].safe_fixes(), name != INIT);
        assert!(cops[0].path_policy().is_none());
        assert_eq!(cops[0].ruby_versions(), Default::default());
    }
}
#[test]
fn defaults_selectors_config_excludes_and_no_write() {
    let d = project();
    for (cop, source) in CASES {
        let o = unchanged(d.path(), source.as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            "[cops.Style]\nenabled = false\n".into(),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), source.as_bytes(), &["--fix", "a.rb"], 0);
            unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        let o = unchanged(d.path(), source.as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            "exclude = [\"*.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = [\"*.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        for setting in [
            "EnforcedStyle = \"expanded\"",
            "AllowComments = false",
            "TargetRubyVersion = 4.0",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{setting}\n"),
            )
            .unwrap();
            unchanged(d.path(), source.as_bytes(), &["--fix", "a.rb"], 2);
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn both_directive_prefixes_scopes_source_enabling_clean_and_error_files() {
    let d = project();
    for (cop, source) in CASES {
        for prefix in ["rubocop", "rustocop"] {
            for target in [cop, "Style", "all"] {
                for kind in ["disable", "disable-next"] {
                    let source = format!("# {prefix}:{kind} {target}\n{source}");
                    unchanged(
                        d.path(),
                        source.as_bytes(),
                        &["--only", cop, "--fix", "a.rb"],
                        0,
                    );
                }
                let inline = format!("{} # {prefix}:disable {target}\n", source.trim_end());
                unchanged(
                    d.path(),
                    inline.as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    0,
                );
            }
            let source =
                format!("# {prefix}:disable {cop}\n{source}# {prefix}:enable {cop}\n{source}");
            let o = unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
            assert_eq!(
                String::from_utf8_lossy(&o.stdout)
                    .matches(&format!("({cop})"))
                    .count(),
                1
            );
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
        )
        .unwrap();
        let enabled = format!("# rustocop:enable {cop}\n{source}");
        unchanged(d.path(), enabled.as_bytes(), &["a.rb"], 1);
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        unchanged(d.path(), b"nil\n", &["--only", cop, "--fix", "a.rb"], 0);
        unchanged(d.path(), b"(\n", &["--only", cop, "--fix", "a.rb"], 2);
    }
}
#[test]
fn stdin_is_report_only_and_fix_is_rejected() {
    let d = project();
    for (cop, source) in CASES {
        fs::write(d.path().join("a.rb"), source).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", cop, "--stdin", "a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let o = child.wait_with_output().unwrap();
        assert_eq!(o.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        cli(d.path(), &["--only", cop, "--fix", "--stdin", "a.rb"], 2);
        assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), source);
    }
}
#[test]
fn source_envelopes_preserve_encoding_bom_crlf_literals_line_and_data_bytes() {
    let d = project();
    for (cop,before,after) in [
        (FREEZE,&b"\xef\xbb\xbf# frozen_string_literal: true\r\n\"a\\x82\".freeze\r\n__LINE__\r\n__END__\r\nraw\xff\x00.freeze\r\n"[..],&b"\xef\xbb\xbf# frozen_string_literal: true\r\n\"a\\x82\"\r\n__LINE__\r\n__END__\r\nraw\xff\x00.freeze\r\n"[..]),
        (FREEZE,&b"# coding: ISO-8859-1\n# frozen_string_literal: true\n'\xe9'.freeze\n"[..],&b"# coding: ISO-8859-1\n# frozen_string_literal: true\n'\xe9'\n"[..]),
        (FREEZE,&b"# frozen_string_literal: true\n<<~'TEXT'.freeze\n  literal \\x82\nTEXT\n__LINE__\n__END__\nraw\n"[..],&b"# frozen_string_literal: true\n<<~'TEXT'\n  literal \\x82\nTEXT\n__LINE__\n__END__\nraw\n"[..]),
        (SELF,&b"\xef\xbb\xbfdef b; false; end\r\na = self.b\r\n__LINE__\r\n__END__\r\nself.b\xff\x00\r\n"[..],&b"\xef\xbb\xbfdef b; false; end\r\na = b\r\n__LINE__\r\n__END__\r\nself.b\xff\x00\r\n"[..]),
    ] { fixed(d.path(),cop,before,after); }
}
#[test]
fn incompatible_colon_and_layout_peers_hold_self_edits_without_hiding_offenses() {
    let d = project();
    {
        let only = format!("{SELF},Style/ColonMethodCall");
        let out = unchanged(
            d.path(),
            b"self.object_id\n",
            &["--only", &only, "--fix", "a.rb"],
            1,
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains(SELF));
    }
    fixed(d.path(), FREEZE, b"(1+2).freeze\n", b"(1+2)\n");
    fs::write(d.path().join("a.rb"), "(1+2).freeze\n").unwrap();
    cli(
        d.path(),
        &[
            "--only",
            &format!("{FREEZE},Layout/SpaceAroundOperators"),
            "--fix",
            "a.rb",
        ],
        0,
    );
    assert_eq!(
        fs::read_to_string(d.path().join("a.rb")).unwrap(),
        "(1 + 2)\n"
    );
    // Selection of the unsafe rule never makes its edits eligible.
    unchanged(
        d.path(),
        b"def initialize; end\n",
        &["--only", ALL, "--fix", "a.rb"],
        1,
    );
}
fn ruby(binary: &str, source: &str) -> Option<Output> {
    let mut child = match Command::new(binary)
        .args(["--disable-gems", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => panic!("{e}"),
    };
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    Some(child.wait_with_output().unwrap())
}
#[test]
fn native_witnesses_explain_visibility_missing_dispatch_it_and_safe_navigation_holds() {
    for binary in ["ruby", "/opt/ruby34/miniruby"] {
        let Some(o) = ruby(
            binary,
            "def missing(call); eval(call); rescue NameError => e; e.class.name; end\np [missing('self.no_such_method'),missing('no_such_method')]\nclass C; private def b; 1; end; def check; [defined?(self.b),defined?(b),self.b,b]; end; end\np C.new.check\n[false,nil].each { |v| p v.instance_eval { self&.object_id } }\n",
        ) else {
            continue;
        };
        assert!(o.status.success(), "{o:?}");
        let text = String::from_utf8_lossy(&o.stdout);
        assert!(
            text.contains("[\"NoMethodError\", \"NameError\"]"),
            "{text}"
        );
        assert!(text.contains("[nil, \"method\", 1, 1]"), "{text}");
        assert!(text.lines().last().unwrap() == "nil");
    }
    if let Some(o) = ruby("/opt/ruby34/miniruby", "0.times { || it }\n") {
        assert!(!o.status.success());
        assert!(String::from_utf8_lossy(&o.stderr).contains("not allowed"));
    }
}
