//! DATA exists only for a main script: execute controlled Ruby, never `load`.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

const PRELUDE: &str = "def identity(value = nil); value; end\n";
const CASES: [(&str, &str, &str); 3] = [
    (
        "Style/MethodCallWithArgsParentheses",
        "x = identity 1\n",
        "x = identity(1)\n",
    ),
    (
        "Style/MethodCallWithoutArgsParentheses",
        "x = identity()\n",
        "x = identity\n",
    ),
    (
        "Style/NestedParenthesizedCalls",
        "x = identity(identity 1)\n",
        "x = identity(identity(1))\n",
    ),
];
const REFERENCES: [(&str, &str); 13] = [
    ("", "DATA"),
    ("", "(DATA ||= nil)"),
    ("", "::DATA"),
    ("", "Object::DATA"),
    ("", "::Object::DATA"),
    ("Root = Object\n", "Root::DATA"),
    ("Root = Object\n", "Root::Object::DATA"),
    ("Handle = DATA\n", "Handle"),
    ("handle = ::DATA\n", "handle"),
    ("", "Object.const_get(:DATA)"),
    ("", "::Object.const_get('DATA')"),
    ("Root = Object\n", "Root.const_get(:DATA)"),
    (
        "class << Object; alias_method :lookup_data, :const_get; end\n",
        "Object.lookup_data(:DATA)",
    ),
];
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn cli(dir: &Path, args: &[&str], status: i32) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    out
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], status: i32) -> Output {
    let path = dir.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let out = cli(dir, args, status);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    out
}
fn observed(candidate: &str, setup: &str, reference: &str, selector: &str) -> String {
    format!(
        "{PRELUDE}{candidate}{setup}p [{reference}.{selector}, {reference}.lineno, {reference}.read]\n__END__\nraw\n"
    )
}
fn ruby_available() -> bool {
    Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}
fn execute(path: &Path) -> Vec<u8> {
    let out = Command::new("ruby")
        .args(["-W0"])
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    out.stdout
}
fn expected_observation(source: &str) -> Vec<u8> {
    let start = source.find("__END__\n").unwrap() + "__END__\n".len();
    let line = source[..start].bytes().filter(|b| *b == b'\n').count();
    format!("[{start}, {line}, \"raw\\n\"]\n").into_bytes()
}
#[test]
fn no_data_and_inert_spellings_still_correct_exactly_and_stabilize() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, before, after) in CASES {
        for payload in [false, true] {
            for crlf in [false, true] {
                let decorate = |candidate: &str| {
                    let source = format!(
                        "{PRELUDE}# DATA.pos Object.const_get(:DATA)\n{candidate}p 'DATA.tell'\n{}",
                        if payload { "__END__\nDATA.read\n" } else { "" }
                    );
                    let mut bytes = if crlf {
                        format!("\u{feff}{}", source.replace('\n', "\r\n")).into_bytes()
                    } else {
                        source.into_bytes()
                    };
                    if payload {
                        bytes.extend_from_slice(b"\xff\x00raw");
                    }
                    bytes
                };
                fs::write(&path, decorate(before)).unwrap();
                let original = ruby_available().then(|| execute(&path));
                cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
                assert_eq!(fs::read(&path).unwrap(), decorate(after));
                if let Some(original) = original {
                    assert_eq!(execute(&path), original);
                }
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            }
        }
    }
}

#[test]
fn compound_data_reads_withhold_byte_changes_but_other_constants_still_correct() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, before, after) in CASES {
        for operator in ["||=", "&&=", "+="] {
            // Bare compound targets have no ConstantReadNode child. Keep them
            // unreachable so every operator is valid without mutating the IO.
            for name in ["DATA", "OTHER"] {
                let decorate = |candidate| {
                    format!(
                        "{PRELUDE}{candidate}if false; {name} {operator} nil; end\n__END__\nraw\n"
                    )
                };
                let source = decorate(before);
                if name == "DATA" {
                    let out = unchanged(
                        dir.path(),
                        source.as_bytes(),
                        &["--only", cop, "--fix", "lib/a.rb"],
                        1,
                    );
                    assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
                } else {
                    fs::write(&path, source).unwrap();
                    cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
                    assert_eq!(fs::read(&path).unwrap(), decorate(after).as_bytes());
                }
            }
        }
    }
}

#[test]
fn selectors_configuration_directives_and_excludes_do_not_bypass_safety() {
    let dir = project();
    let config = dir.path().join("rustocop.toml");
    let path = dir.path().join("lib/a.rb");
    let registry = rustocop::cops::Registry::new();
    for (index, (cop, before, after)) in CASES.into_iter().enumerate() {
        let implementation = registry.cops().iter().find(|c| c.name() == cop).unwrap();
        assert!(implementation.safe_fixes());
        assert_eq!(implementation.default_enabled(), index != 0);
        let source = observed(before, "", "DATA", "pos");
        for setting in [
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            String::new(),
        ] {
            // Isolate the consumer; disabling the department must not bypass
            // --only's force-enable or its correction-only safety guard.
            fs::write(
                &config,
                format!("[cops.Lint]\nenabled = false\n[cops.Style]\nenabled = false\n{setting}"),
            )
            .unwrap();
            unchanged(dir.path(), source.as_bytes(), &["--fix", "lib/a.rb"], 0);
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                1,
            );
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--except", cop, "--fix", "lib/a.rb"],
                0,
            );
        }
        fs::write(&config, format!("[cops.Lint]\nenabled = false\n[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n")).unwrap();
        unchanged(dir.path(), source.as_bytes(), &["--fix", "lib/a.rb"], 1);
        fs::write(&path, format!("{PRELUDE}{before}")).unwrap();
        cli(dir.path(), &["--fix", "lib/a.rb"], 0);
        assert_eq!(
            fs::read(&path).unwrap(),
            format!("{PRELUDE}{after}").as_bytes()
        );
        fs::write(&config, "").unwrap();
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", cop, "lib/a.rb"],
            1,
        );
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                for directive in ["disable", "disable-next"] {
                    let suppressed = format!(
                        "{PRELUDE}# {prefix}:{directive} {target}\n{before}p DATA.pos\n__END__\nraw\n"
                    );
                    unchanged(
                        dir.path(),
                        suppressed.as_bytes(),
                        &["--only", cop, "--fix", "lib/a.rb"],
                        0,
                    );
                }
            }
            let suppressed = format!(
                "{PRELUDE}{} # {prefix}:disable {cop}\np DATA.pos\n__END__\nraw\n",
                before.trim_end()
            );
            unchanged(
                dir.path(),
                suppressed.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                0,
            );
        }
        for setting in [
            "exclude = [\"lib/**\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = [\"lib/**\"]\n"),
        ] {
            fs::write(&config, setting).unwrap();
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                0,
            );
        }
        fs::write(&config, "").unwrap();
    }
}

#[test]
fn forwarding_and_super_data_safeguards_cover_the_same_acquisitions() {
    let dir = project();
    for (cop, candidate) in [
        (
            "Style/ArgumentsForwarding",
            "def f(*args, **kwargs, &block); identity(*args, **kwargs, &block); end\n",
        ),
        ("Style/SuperArguments", "def f(a); super(a); end\n"),
        ("Style/SuperWithArgsParentheses", "def f(a); super a; end\n"),
    ] {
        for (setup, reference) in REFERENCES {
            let source = observed(candidate, setup, reference, "method(:tell).call");
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", cop, "--fix", "lib/a.rb"],
                1,
            );
        }
    }
}

#[test]
fn byte_guard_is_conservative_for_data_acquisitions_not_just_pos_selectors() {
    let dir = project();
    for (cop, before, _) in CASES {
        for observer in [
            "p DATA.read",
            "p DATA.lineno",
            "handle = DATA",
            "p :const_get",
            "p Object.const_get(:String)",
        ] {
            // As in the existing super proof, even absent payload or harmless
            // const_get data is conservative. No runtime/value-flow inference.
            for payload in ["", "__END__\nraw\n"] {
                let source = format!("{PRELUDE}{before}{observer}\n{payload}");
                unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    1,
                );
            }
        }
    }
}

#[test]
fn data_paths_withhold_length_changes_with_exact_bytes_and_mtime() {
    let dir = project();
    for (cop, before, _) in CASES {
        for (setup, reference) in REFERENCES {
            for crlf in [false, true] {
                let source = observed(before, setup, reference, "pos");
                let mut bytes = if crlf {
                    format!("\u{feff}{}", source.replace('\n', "\r\n")).into_bytes()
                } else {
                    source.into_bytes()
                };
                bytes.extend_from_slice(b"\xff\x00\r\n");
                let out = unchanged(dir.path(), &bytes, &["--only", cop, "--fix", "lib/a.rb"], 1);
                assert!(String::from_utf8_lossy(&out.stdout).contains(cop));
            }
        }
    }
}
#[test]
fn ruby_main_script_bom_crlf_and_binary_payload_keep_positions_bytes_and_mtime() {
    if !ruby_available() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, before, after) in CASES {
        for header in ["\u{feff}", "# coding: ISO-8859-1\n"] {
            let decorate = |candidate| {
                let source = observed(
                    candidate,
                    "handle = ::DATA\n",
                    "handle",
                    "method(:tell).call",
                );
                let mut bytes = format!("{header}{source}")
                    .replace('\n', "\r\n")
                    .into_bytes();
                bytes.extend_from_slice(b"\xff\x00\r\n");
                bytes
            };
            let source = decorate(before);
            fs::write(&path, &source).unwrap();
            let original = execute(&path);
            let marker = b"__END__\r\n";
            let start = source
                .windows(marker.len())
                .position(|w| w == marker)
                .unwrap()
                + marker.len();
            let line = source[..start].iter().filter(|b| **b == b'\n').count();
            assert!(original.starts_with(format!("[{start}, {line}, ").as_bytes()));
            unchanged(
                dir.path(),
                &source,
                &["--only", cop, "--fix", "lib/a.rb"],
                1,
            );
            assert_eq!(execute(&path), original);
            fs::write(&path, decorate(after)).unwrap();
            let changed = execute(&path);
            assert_ne!(changed, original);
            // Byte position alone changes; the line number and binary payload
            // remain identical even in a naive rewrite of BOM/CRLF source.
            assert_eq!(
                &changed[changed.iter().position(|b| *b == b',').unwrap()..],
                &original[original.iter().position(|b| *b == b',').unwrap()..]
            );
        }
    }
}

#[test]
fn ruby_main_scripts_preserve_positions_where_naive_edits_change_only_pos() {
    if !ruby_available() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, before, after) in CASES {
        for (setup, reference) in REFERENCES {
            for selector in [
                "pos",
                "tell",
                "method(:pos).call",
                "public_send(:tell)",
                "enum_for(:pos).each {}",
                "then(&:tell)",
                "instance_eval { pos }",
            ] {
                let source = observed(before, setup, reference, selector);
                fs::write(&path, &source).unwrap();
                let original = execute(&path);
                assert_eq!(original, expected_observation(&source));
                unchanged(
                    dir.path(),
                    source.as_bytes(),
                    &["--only", cop, "--fix", "lib/a.rb"],
                    1,
                );
                assert_eq!(execute(&path), original);
                let naive = observed(after, setup, reference, selector);
                fs::write(&path, &naive).unwrap();
                let changed = execute(&path);
                assert_eq!(changed, expected_observation(&naive));
                assert_ne!(changed, original);
                assert_eq!(source.lines().count(), naive.lines().count());
                assert_ne!(source.find("__END__"), naive.find("__END__"));
            }
        }
    }
}
