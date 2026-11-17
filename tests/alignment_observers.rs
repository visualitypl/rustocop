//! Real CLI proofs for every shared alignment correction consumer.
use std::{fs, path::Path, process::Command};

fn unchanged(path: &Path, bytes: &[u8], old: &fs::Metadata) {
    assert_eq!(fs::read(path).unwrap(), bytes);
    let new = fs::metadata(path).unwrap();
    assert_eq!(old.permissions(), new.permissions());
    assert_eq!(old.modified().unwrap(), new.modified().unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(old.ino(), new.ino());
    }
}

#[test]
fn path_alias_stream_process_readers_and_lookalikes_really_correct() {
    check(
        &[
            "p File.binread('example.rb')",
            "p IO.read('./example.rb')",
            "F = File; path = 'example.rb'; p F.read(path)",
            "p File.method(:binread).call('example.rb')",
            "File.open('example.rb', &:read)",
            "File.foreach('example.rb') { |line| print line }",
            "p STDIN.read",
            "input = $stdin; input.each { |line| print line }",
            "p STDIN.tell",
            "p STDIN.eof?",
            "STDIN.seek(0, 2)",
            "for line in STDIN; print line; end",
            "p [*STDIN]",
            "p Array(STDIN)",
            "system('/usr/bin/cat', 'example.rb')",
            "p `/usr/bin/cat example.rb`",
            "Process.spawn('cat example.rb')",
            "IO.popen('cat example.rb', &:read)",
            "exec('cat example.rb')",
            "C.new",
            "p DATA",
            "p 'Object.const_get(:DATA)'\n__END__\npayload",
            "# __FILE__ __dir__ DATA $0 $PROGRAM_NAME caller caller_locations source_location SCRIPT_LINES__\np 'caller SCRIPT_LINES__ __FILE__'",
        ],
        false,
    );
}

fn check(tails: &[&str], hold: bool) {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    let path = d.path().join("example.rb");
    for (cop, stem, fixed, line) in [
        (
            "Layout/AssignmentIndentation",
            "value =\n1\n",
            "value =\n  1\n",
            2,
        ),
        (
            "Layout/AccessModifierIndentation",
            "class C\nprivate\n  def f; end\nend\n",
            "class C\n  private\n  def f; end\nend\n",
            2,
        ),
        (
            "Layout/ArgumentAlignment",
            "def f(*v); v; end\nf(1,\n2)\n",
            "def f(*v); v; end\nf(1,\n  2)\n",
            3,
        ),
        (
            "Layout/ArrayAlignment",
            "x = [1,\n2]\np x\n",
            "x = [1,\n     2]\np x\n",
            2,
        ),
        // HashAlignment's local edits reuse the same closed-observer fact.
        (
            "Layout/HashAlignment",
            "h = {\n  a: 1,\nb: 2\n}\np h\n",
            "h = {\n  a: 1,\n  b: 2\n}\np h\n",
            3,
        ),
        (
            "Layout/ParameterAlignment",
            "def g(a,\nb); end\n",
            "def g(a,\n      b); end\n",
            2,
        ),
        (
            "Layout/FirstParameterIndentation",
            "def g(\na); end\n",
            "def g(\n  a); end\n",
            2,
        ),
        // End-indentation edits share the same closed-observer fact.
        (
            "Layout/BeginEndAlignment",
            "  x = begin\nend\n",
            "  x = begin\n  end\n",
            2,
        ),
        (
            "Layout/DefEndAlignment",
            "x = def f\nend\n",
            "x = def f\n    end\n",
            2,
        ),
        (
            "Layout/BlockAlignment",
            "  foo do\nend\n",
            "  foo do\n  end\n",
            2,
        ),
        // First-element and closing-parenthesis edits.
        (
            "Layout/ClosingParenthesisIndentation",
            "  foo(\n    a\n)\n",
            "  foo(\n    a\n  )\n",
            3,
        ),
        (
            "Layout/FirstArgumentIndentation",
            "foo(\na)\n",
            "foo(\n  a)\n",
            2,
        ),
        (
            "Layout/FirstArrayElementIndentation",
            "x = [\n1]\np x\n",
            "x = [\n  1]\np x\n",
            2,
        ),
        (
            "Layout/FirstHashElementIndentation",
            "h = {\na: 1}\np h\n",
            "h = {\n  a: 1}\np h\n",
            2,
        ),
        // Clause and keyword indentation edits.
        (
            "Layout/CaseIndentation",
            "  case x\nwhen 1\n  end\n",
            "  case x\n  when 1\n  end\n",
            2,
        ),
        (
            "Layout/ElseAlignment",
            "  if x\n    1\nelse\n  end\n",
            "  if x\n    1\n  else\n  end\n",
            3,
        ),
        (
            "Layout/RescueEnsureAlignment",
            "  begin\n    1\nrescue\n  end\n",
            "  begin\n    1\n  rescue\n  end\n",
            3,
        ),
        // Heredoc body and closing indentation edits.
        (
            "Layout/HeredocIndentation",
            "x = <<~A\nfoo\nA\n",
            "x = <<~A\n  foo\nA\n",
            2,
        ),
        (
            "Layout/ClosingHeredocIndentation",
            "  x = <<~A\n    y\nA\n",
            "  x = <<~A\n    y\n  A\n",
            3,
        ),
        // Own-line comment indentation edits.
        (
            "Layout/CommentIndentation",
            "def f\n# a\n  x\nend\n",
            "def f\n  # a\n  x\nend\n",
            2,
        ),
        // Inconsistent statement indentation edits.
        (
            "Layout/IndentationConsistency",
            "def f\n  a\nb\nend\n",
            "def f\n  a\n  b\nend\n",
            3,
        ),
        // Multi-line method call indentation edits.
        (
            "Layout/MultilineMethodCallIndentation",
            "a\n.b\n",
            "a\n  .b\n",
            2,
        ),
        // Multi-line operation indentation edits.
        (
            "Layout/MultilineOperationIndentation",
            "a +\nb\n",
            "a +\n  b\n",
            2,
        ),
    ] {
        for tail in tails {
            let source = format!("{stem}{tail}\n");
            let expected = if hold {
                source.clone()
            } else {
                format!("{fixed}{tail}\n")
            };
            fs::write(&path, &source).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
            }
            let old = fs::metadata(&path).unwrap();
            for fix in [false, true] {
                let mut cmd = Command::new(env!("CARGO_BIN_EXE_rustocop"));
                cmd.current_dir(d.path());
                if fix {
                    cmd.arg("--fix");
                }
                let out = cmd.args(["--only", cop, "example.rb"]).output().unwrap();
                assert_eq!(
                    out.status.code(),
                    Some(i32::from(!fix || hold)),
                    "{cop}: {source}: {out:?}"
                );
                if !fix || hold {
                    let text = String::from_utf8_lossy(&out.stdout);
                    assert_eq!(text.matches(&format!("({cop})")).count(), 1, "{out:?}");
                    assert!(text.contains(&format!("example.rb:{line}:1:")), "{out:?}");
                    unchanged(&path, source.as_bytes(), &old);
                }
            }
            assert_eq!(
                fs::read(&path).unwrap(),
                expected.as_bytes(),
                "{cop}: {source}"
            );
            assert_eq!(
                old.permissions(),
                fs::metadata(&path).unwrap().permissions()
            );
            let old = fs::metadata(&path).unwrap();
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(["--fix", "--only", cop, "example.rb"])
                .output()
                .unwrap();
            assert_eq!(out.status.code(), Some(i32::from(hold)), "{out:?}");
            unchanged(&path, expected.as_bytes(), &old);
        }
    }
}
