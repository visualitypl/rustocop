//! opt-in detection, unchanged files and controlled Ruby binding probes.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/MethodCalledOnDoEndBlock";
const SOURCE: &str = "# Copyright (c) 2026 Example\n# frozen_string_literal: true\nitems.each do\n  work\nend.size\n";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn run(dir: &Path, args: &[&str], status: i32, count: usize) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(status), "{args:?}: {output:?}");
    let text = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        text.lines()
            .filter(|line| line.contains(&format!("({COP})")))
            .count(),
        count,
        "{args:?}: {text}"
    );
    output
}
fn unchanged(dir: &Path, path: &Path, source: &[u8], args: &[&str], status: i32, count: usize) {
    fs::write(path, source).unwrap();
    let modified = fs::metadata(path).unwrap().modified().unwrap();
    run(dir, args, status, count);
    assert_eq!(fs::read(path).unwrap(), source);
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), modified);
}

#[test]
fn default_opt_in_only_except_and_config_never_write() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for (config, args, count) in [
        (String::new(), vec!["a.rb"], 0),
        (String::new(), vec!["--only", COP, "a.rb"], 1),
        (String::new(), vec!["--only", COP, "--fix", "a.rb"], 1),
        (
            String::new(),
            vec!["--only", COP, "--except", COP, "--fix", "a.rb"],
            0,
        ),
        (
            String::new(),
            vec!["--only", "Style", "--except", COP, "a.rb"],
            0,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled = true\n"),
            vec!["a.rb"],
            1,
        ),
        (
            format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
            vec!["--fix", "a.rb"],
            1,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["--only", COP, "--fix", "a.rb"],
            1,
        ),
        (
            "[cops.Style]\nenabled = false\n".into(),
            vec!["--only", COP, "a.rb"],
            1,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            dir.path(),
            &path,
            SOURCE.as_bytes(),
            &args,
            i32::from(count != 0),
            count,
        );
    }
}

#[test]
fn both_directives_source_opt_in_and_highlight_boundaries() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for prefix in ["rustocop", "rubocop"] {
        for (source, count) in [
            (format!("# {prefix}:disable {COP}\n{SOURCE}"), 0),
            (format!("# {prefix}:disable-next {COP}\n{SOURCE}"), 0),
            (format!("a do b end.c # {prefix}:disable {COP}\n"), 0),
            (format!("a do\n  b # {prefix}:disable {COP}\nend.c\n"), 1),
            (
                format!("a do b end.c(\n  1 # {prefix}:disable {COP}\n)\n"),
                0,
            ),
            (
                format!("a do b end.c ||=\n  1 # {prefix}:disable {COP}\n"),
                1,
            ),
        ] {
            unchanged(
                dir.path(),
                &path,
                source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                i32::from(count != 0),
                count,
            );
        }
        let source = format!("# {prefix}:enable {COP}\n{SOURCE}");
        unchanged(dir.path(), &path, source.as_bytes(), &["a.rb"], 1, 1);
        unchanged(
            dir.path(),
            &path,
            source.as_bytes(),
            &["--except", COP, "a.rb"],
            0,
            0,
        );
        unchanged(
            dir.path(),
            &path,
            source.as_bytes(),
            &["--only", "Lint/Debugger", "a.rb"],
            0,
            0,
        );
    }
}

#[test]
fn config_relative_excludes_stdin_and_strict_fixed_policy() {
    let dir = project();
    fs::create_dir(dir.path().join("lib")).unwrap();
    let path = dir.path().join("lib/a.rb");
    for config in [
        String::new(),
        format!("[cops.\"{COP}\"]\nexclude = ['lib/**']\n"),
        "exclude = ['lib/**']\n".into(),
    ] {
        let count = usize::from(config.is_empty());
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            &dir.path().join("lib"),
            &path,
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            count as i32,
            count,
        );
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", COP, "--stdin", "lib/a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(SOURCE.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(count as i32), "{output:?}");
        assert_eq!(fs::read(&path).unwrap(), SOURCE.as_bytes());
    }
    for option in [
        "enforced_style = 'braces'",
        "target_ruby_version = '3.4'",
        "max = 99",
    ] {
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.\"{COP}\"]\n{option}\n"),
        )
        .unwrap();
        unchanged(
            dir.path(),
            &path,
            SOURCE.as_bytes(),
            &["--only", COP, "--fix", "lib/a.rb"],
            2,
            0,
        );
    }
}

#[test]
fn offenses_clean_errors_encodings_bom_crlf_and_data_keep_bytes_and_mtime() {
    let dir = project();
    let path = dir.path().join("a.rb");
    for (source, status, count) in [
        (b"\xef\xbb\xbf# encoding: UTF-8\r\na do\r\n b\r\nend.c\r\n__END__\r\nraw\xff\r\n".as_slice(), 1, 1),
        (b"# encoding: ISO-8859-1\r\na do b end.c # caf\xe9\r\n".as_slice(), 1, 1),
        (b"a do\n b(<<~TXT)\n end.c is literal content\nTXT\nend.c <<~OTHER\n#{__LINE__}\nOTHER\n".as_slice(), 1, 1),
        (b"a { b }.c\n".as_slice(), 0, 0),
        (b"a do b end.c do d end\n".as_slice(), 0, 0),
        (b"a do\n".as_slice(), 2, 0),
    ] {
        for fix in [false, true] {
            let mut args = vec!["--only", COP, "a.rb"];
            if fix { args.push("--fix"); }
            unchanged(dir.path(), &path, source, &args, status, count);
        }
    }
}

#[test]
fn mixed_safe_corrections_never_replace_block_delimiters_or_hide_the_offense() {
    let dir = project();
    let path = dir.path().join("a.rb");
    let source = "a do\n  b\nend::c('x')\n";
    fs::write(&path, source).unwrap();
    let only = format!("{COP},Style/ColonMethodCall,Style/StringLiterals");
    run(dir.path(), &["--only", &only, "--fix", "a.rb"], 1, 1);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "a do\n  b\nend.c(\"x\")\n"
    );
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    run(dir.path(), &["--only", &only, "--fix", "a.rb"], 1, 1);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn fix_preserves_observed_command_binding_evaluation_values_and_control_flow() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = project();
    let path = dir.path().join("a.rb");
    // Only these controlled synthetic programs are executed, never corpus code.
    for (source, expected) in [
        (
            "events = []\ndef inner(e); e << :inner; :argument; end\ndef outer(value, e); e << value; yield; end\nresult = outer inner(events), events do\n  events << :block\n  7\nend.to_s\np [events, result]\n",
            "[[:inner, :argument, :block], \"7\"]\n",
        ),
        (
            "events = []\ndef outer(e); e << :outer; yield; end\nresult = outer(events) do\n  events << :block\n  7\nend + (events << :rhs; 2)\np [events, result]\n",
            "[[:outer, :block, :rhs], 9]\n",
        ),
        (
            "events = []\ndef early(e)\n  [1].each do\n    e << :block\n    return :early\n  end.to_s\n  :late\nend\np [events, early(events), __LINE__]\n",
            "[[:block], :early, 9]\n",
        ),
        (
            "events = []\narray = [0]\nresult = array.tap do\n  events << :block\nend[0] = (events << :rhs; 7)\np [events, result, array]\n",
            "[[:block, :rhs], 7, [7]]\n",
        ),
        (
            "events = []\ndef outer(e); e << :outer; yield; end\nresult = outer(events) do\n  events << :block\n  \"value\"\nend.concat(<<~TEXT)\n  #{events << :heredoc; 'suffix'}\nTEXT\np [events, result]\n",
            "[[:outer, :block, :heredoc], \"valuesuffix\\n\"]\n",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let observe = || {
            Command::new("ruby")
                .current_dir(dir.path())
                .arg("a.rb")
                .output()
                .unwrap()
        };
        let before = observe();
        assert!(before.status.success(), "{before:?}");
        assert_eq!(String::from_utf8_lossy(&before.stdout), expected);
        assert!(before.stderr.is_empty(), "{before:?}");
        unchanged(
            dir.path(),
            &path,
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
            1,
        );
        let after = observe();
        assert_eq!(before.status.code(), after.status.code());
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
    }
}
