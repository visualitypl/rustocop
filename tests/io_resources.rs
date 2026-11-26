//! stream/resource policy through the real CLI and controlled local Ruby.
use crate::support::cli::project_with_lib;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const CASES: &[(&str, &str, &str, bool)] = &[
    (
        "Style/AutoResourceCleanup",
        "File.open(\"file\")\n",
        "File.open(\"file\")\n",
        false,
    ),
    (
        "Style/GlobalStdStream",
        "STDOUT.puts(\"hello\")\n",
        "STDOUT.puts(\"hello\")\n",
        true,
    ),
    (
        "Style/StderrPuts",
        "$VERBOSE = false\n$stderr.puts(\"hello\")\n",
        "$VERBOSE = false\nwarn(\"hello\")\n",
        true,
    ),
];
fn cli(cwd: &Path, args: &[&str], status: i32) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    out
}
#[test]
fn defaults_opt_in_selectors_config_and_safe_fix_policy() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    let registry = rustocop::cops::Registry::new();
    for (cop, before, after, enabled) in CASES {
        let registered: Vec<_> = registry
            .cops()
            .iter()
            .filter(|c| c.name() == *cop)
            .collect();
        assert_eq!(registered.len(), 1);
        assert_eq!(registered[0].default_enabled(), *enabled);
        assert_eq!(registered[0].safe_fixes(), before != after);
        assert!(registered[0].path_policy().is_none());
        fs::write(&path, before).unwrap();
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        // Other default cops may report, so inspect this cop rather than total exit status.
        let out = cli(dir.path(), &["lib/a.rb"], 1);
        assert_eq!(String::from_utf8_lossy(&out.stdout).contains(cop), *enabled);
        cli(dir.path(), &["--only", cop, "lib/a.rb"], 1);
        assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        for config in [
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            cli(
                dir.path(),
                &["--only", cop, "--except", cop, "--fix", "lib/a.rb"],
                0,
            );
            let out = cli(dir.path(), &["--only", cop, "lib/a.rb"], 1);
            let line = if *cop == "Style/StderrPuts" { 2 } else { 1 };
            assert!(
                String::from_utf8_lossy(&out.stdout).contains(&format!("lib/a.rb:{line}:1: C:"))
            );
            assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        let status = i32::from(before == after);
        cli(dir.path(), &["--fix", "lib/a.rb"], status);
        assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], status);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn directives_excludes_and_stdin_do_not_write() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    for (cop, before, _, _) in CASES {
        for prefix in ["rubocop", "rustocop"] {
            for target in [*cop, "Style", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\n{before}"),
                    format!("{} # {prefix}:disable {target}\n", before.trim_end()),
                    {
                        let (preamble, offense) = before
                            .trim_end()
                            .rsplit_once('\n')
                            .unwrap_or(("", before.trim_end()));
                        format!("{preamble}\n# {prefix}:disable-next {target}\n{offense}\n")
                    },
                ] {
                    fs::write(&path, &source).unwrap();
                    cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0);
                    assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
                }
            }
        }
        fs::write(&path, before).unwrap();
        for config in [
            "exclude = [\"lib/a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = [\"lib/a.rb\"]\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            cli(
                &dir.path().join("lib"),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
            assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        for (source, status) in [
            (before.to_string(), 1),
            (format!("# rubocop:disable {cop}\n{before}"), 0),
        ] {
            let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path().join("lib"))
                .args(["--only", cop, "--stdin", "a.rb"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(source.as_bytes())
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(out.status.code(), Some(status), "{out:?}");
            assert_eq!(fs::read(&path).unwrap(), before.as_bytes());
        }
    }
}
#[test]
fn byte_local_fixes_preserve_legacy_encoding_bom_crlf_data_and_line() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    for source in [b"# encoding: ISO-8859-1\r\n$VERBOSE = false\r\n$stderr.puts(\"caf\xe9\")\r\np __LINE__\r\n__END__\r\nraw\xff\r\n".to_vec(), "\u{feff}# encoding: UTF-8\r\n$VERBOSE = true\r\n$stderr.puts(\"café\")\r\np __LINE__\r\n__END__\r\nraw\r\n".as_bytes().to_vec()] {
        let start = source.windows(12).position(|w| w == b"$stderr.puts").unwrap();
        let mut expected = source[..start].to_vec();
        expected.extend_from_slice(b"warn");
        expected.extend_from_slice(&source[start + 12..]);
        fs::write(&path, &source).unwrap();
        cli(dir.path(), &["--only", "Style/StderrPuts", "--fix", "lib/a.rb"], 0);
        assert_eq!(fs::read(&path).unwrap(), crate::support::lf::lf(&expected));
    }
}
fn ruby(source: &str) -> Option<Output> {
    match Command::new("ruby")
        .args(["--disable-gems", "-e", source])
        .output()
    {
        Ok(out) => {
            assert!(out.status.success(), "{source}: {out:?}");
            Some(out)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping runtime probe: Ruby unavailable");
            None
        }
        Err(e) => panic!("ruby: {e}"),
    }
}
fn corrected(dir: &Path, source: &str, status: i32) -> String {
    fs::write(dir.join("lib/a.rb"), source).unwrap();
    cli(
        dir,
        &["--only", "Style/StderrPuts", "--fix", "lib/a.rb"],
        status,
    );
    fs::read_to_string(dir.join("lib/a.rb")).unwrap()
}
#[test]
fn runtime_warning_policy_output_return_control_flow_and_verbosity() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    for receiver in ["$stderr", "STDERR", "::STDERR"] {
        for verbose in ["false", "true"] {
            let source = format!(
                "$VERBOSE = {verbose}\nresult = {receiver}.puts(\"hello\", \"world\\n\")\nbegin\n  p result\n  2.times {{ puts :loop }}\nensure\n  p :ensure\nend\np __LINE__\n"
            );
            for level in ["-W0", "-W1", "-W2"] {
                fs::write(&path, &source).unwrap();
                let Some(before) = ruby_file(&path, level) else {
                    return;
                };
                let after = corrected(dir.path(), &source, 0);
                assert_eq!(after, source.replace(&format!("{receiver}.puts"), "warn"));
                let out = ruby_file(&path, level).unwrap();
                assert_eq!(before.stdout, b"nil\nloop\nloop\n:ensure\n9\n");
                assert_eq!(before.stderr, b"hello\nworld\n");
                assert_eq!(before.status.code(), out.status.code());
                assert_eq!(before.stdout, out.stdout);
                assert_eq!(before.stderr, out.stderr);
            }
        }
    }
}
fn ruby_file(path: &Path, level: &str) -> Option<Output> {
    match Command::new("ruby").arg(level).arg(path).output() {
        Ok(out) => {
            assert!(out.status.success(), "{path:?} {level}: {out:?}");
            Some(out)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => panic!("ruby: {e}"),
    }
}
#[test]
fn runtime_detection_only_rules_preserve_stream_identity_and_resource_lifetime() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    let data = dir.path().join("resource.txt");
    fs::write(&data, "payload").unwrap();
    let resource = format!(
        "f = File.open({:?})\nbegin\n  p f.closed?\n  p f.read\nensure\n  f.close\nend\np f.closed?\n",
        data.to_str().unwrap()
    );
    let stream = "class Sink; def write(x); STDERR.write(\"redirected:#{x}\"); x.bytesize; end; end\n$stdout = Sink.new\nSTDOUT.puts('original')\n";
    for (cop, source, stdout, stderr) in [
        (
            "Style/AutoResourceCleanup",
            resource.as_str(),
            "false\n\"payload\"\ntrue\n",
            "",
        ),
        ("Style/GlobalStdStream", stream, "original\n", ""),
    ] {
        fs::write(&path, source).unwrap();
        cli(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 1);
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        let Some(out) = ruby(source) else { return };
        assert_eq!(out.stdout, stdout.as_bytes());
        assert_eq!(out.stderr, stderr.as_bytes());
    }
}

#[test]
fn style_layout_interactions_stabilize_in_bounded_explicit_passes() {
    let dir = project_with_lib();
    let path = dir.path().join("lib/a.rb");
    fs::write(
        &path,
        "$VERBOSE = false\n$stderr.puts('x');  \ndef f\n  return (1);\nend\n",
    )
    .unwrap();
    let only = "Style/StderrPuts,Style/GlobalStdStream,Style/StringLiterals,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Style/HashSyntax,Layout";
    for _ in 0..4 {
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", only, "--fix", "lib/a.rb"])
            .output()
            .unwrap();
        assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
    }
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "$VERBOSE = false\nwarn(\"x\")\ndef f\n  1\nend\n"
    );
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    cli(dir.path(), &["--only", only, "--fix", "lib/a.rb"], 0);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
