//! Runtime helpers execute ONLY controlled ordinary Ruby, never
//! command literals, corpus/application code or the source-only probes below.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const CHARS: &str = "Style/StringChars";
const METHODS: &str = "Style/StringMethods";
const STRIP: &str = "Style/Strip";
const ALL: &str = "Style/StringChars,Style/StringMethods,Style/Strip";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(dir: &Path, args: &[&str], status: i32) -> Output {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(status), "{args:?}: {o:?}");
    o
}
fn unchanged(dir: &Path, s: &[u8], args: &[&str], status: i32) -> Output {
    let p = dir.join("a.rb");
    fs::write(&p, s).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    let o = cli(dir, args, status);
    assert_eq!(fs::read(&p).unwrap(), s);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
    o
}
fn fix(dir: &Path, cop: &str, s: &[u8], status: i32) -> Vec<u8> {
    fs::write(dir.join("a.rb"), s).unwrap();
    cli(dir, &["--only", cop, "--fix", "a.rb"], status);
    fs::read(dir.join("a.rb")).unwrap()
}
#[test]
fn real_defaults_opt_in_exact_diagnostics_and_no_write_without_fix() {
    let registry = rustocop::cops::Registry::new();
    for (name, enabled, safe) in [
        (CHARS, true, false),
        (METHODS, false, true),
        (STRIP, true, true),
    ] {
        let c = registry.cops().iter().find(|c| c.name() == name).unwrap();
        assert_eq!((c.default_enabled(), c.safe_fixes()), (enabled, safe));
        assert!(c.path_policy().is_none());
        assert_eq!(c.ruby_versions(), Default::default());
    }
    let d = project();
    let s = b"\"x\".intern\n\" x \".lstrip.rstrip\nstring.split('')\n";
    let out = unchanged(d.path(), s, &["a.rb"], 1);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(!text.contains(METHODS));
    assert!(text.contains(STRIP));
    assert!(text.contains(CHARS));
    let out = unchanged(d.path(), s, &["--only", ALL, "a.rb"], 1);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("a.rb:1:5: C: Prefer `to_sym` over `intern`. (Style/StringMethods)"));
    assert!(text.contains("a.rb:2:7: C: Use `strip` instead of `lstrip.rstrip`. (Style/Strip)"));
    assert!(text.contains("a.rb:3:8: C: Use `chars` instead of `split('')`. (Style/StringChars)"));
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", ALL, "--stdin", "a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(s).unwrap();
    assert_eq!(child.wait_with_output().unwrap().status.code(), Some(1));
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), s);
    assert_eq!(
        fix(d.path(), ALL, s, 1),
        b"\"x\".to_sym\n\" x \".strip\nstring.split('')\n"
    );
}
#[test]
fn selectors_config_excludes_and_strict_fixed_options() {
    let d = project();
    for (cop, s) in [
        (CHARS, "s.split(\"\")\n"),
        (METHODS, "\"x\".intern\n"),
        (STRIP, "\"x\".lstrip.rstrip\n"),
    ] {
        for config in [
            "[cops.Style]\nenabled = false\n".into(),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s.as_bytes(), &["--fix", "a.rb"], 0);
            unchanged(
                d.path(),
                s.as_bytes(),
                &["--only", cop, "--except", cop, "--fix", "a.rb"],
                0,
            );
            unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n"),
        )
        .unwrap();
        unchanged(d.path(), s.as_bytes(), &["a.rb"], 1);
        for config in [
            "exclude = [\"a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = [\"a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
        }
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\nPreferredMethods = {{ intern = \"length\" }}\n"),
        )
        .unwrap();
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 2);
    }
}
#[test]
fn directives_clean_error_and_unsafe_files_keep_bytes_and_mtime() {
    let d = project();
    for (cop, s) in [
        (CHARS, "s.split('')"),
        (METHODS, "\"x\".intern"),
        (STRIP, "\"x\".lstrip.rstrip"),
    ] {
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, cop.split('/').nth(1).unwrap(), "Style", "all"] {
                for s in [
                    format!("# {prefix}:disable {target}\n{s}\n"),
                    format!("{s} # {prefix}:disable {target}\n"),
                    format!("# {prefix}:disable-next {target}\n{s}\n"),
                ] {
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
                }
            }
        }
    }
    for (s, status) in [
        ("'x'.to_sym\n'x'.strip\ns.chars\n", 0),
        ("broken(\n", 2),
        ("s.split('')\n", 1),
    ] {
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", ALL, "--fix", "a.rb"],
            status,
        );
    }
}
#[test]
fn unknown_dispatch_refinements_alias_loading_blocks_and_source_observers_withhold() {
    let d = project();
    for body in ["\"x\".intern", "\" x \".lstrip.rstrip"] {
        for prefix in [
            "def unknown; end\n",
            "class String; def intern; :changed; end; end\n",
            "class String; alias strip size; end\n",
            "class Symbol; undef intern; end\n",
            "module M; refine String do; def strip; :changed; end; end; end\nusing M\n",
            "require 'other'\n",
            "load 'other'\n",
            "effect\n",
        ] {
            let s = format!("{prefix}{body}\n");
            unchanged(d.path(), s.as_bytes(), &["--only", ALL, "--fix", "a.rb"], 1);
        }
        for observer in [
            "DATA.pos",
            "File.binread(__FILE__)",
            "RubyVM::InstructionSequence.compile_file(__FILE__)",
            "GC.stat",
            "ObjectSpace.each_object(String).count",
            "TracePoint.new {}",
        ] {
            let s = format!("{body}\n{observer}\n__END__\npayload\n");
            unchanged(d.path(), s.as_bytes(), &["--only", ALL, "--fix", "a.rb"], 1);
        }
    }
    for s in [
        "object&.intern",
        "'x'.intern(&block)",
        "'x'.intern { effect }",
        "'x'.intern(1)",
        "nil&.lstrip.rstrip",
        "value.to_s.lstrip.rstrip",
        "'x'.lstrip.rstrip { effect }",
    ] {
        unchanged(d.path(), s.as_bytes(), &["--only", ALL, "--fix", "a.rb"], 1);
    }
    // No whole-file disable just because an independent later send is uncertain.
    assert_eq!(
        fix(d.path(), ALL, b"\"x\".intern\nunknown.intern\n", 1),
        b"\"x\".to_sym\nunknown.intern\n"
    );
}
#[test]
fn bom_crlf_legacy_encodings_invalid_payload_line_and_unobserved_data() {
    let d = project();
    for (s,after) in [
        (b"\xef\xbb\xbf\"x\".intern\r\n\" x \".lstrip.rstrip\r\np __LINE__\r\n__END__\r\npayload\r\n".as_slice(),b"\xef\xbb\xbf\"x\".to_sym\r\n\" x \".strip\r\np __LINE__\r\n__END__\r\npayload\r\n".as_slice()),
        (b"# encoding: ISO-8859-1\n'\xe9'.intern\n' \xe9 '.rstrip.lstrip\n",b"# encoding: ISO-8859-1\n'\xe9'.to_sym\n' \xe9 '.strip\n"),
        (b"# encoding: Windows-31J\n'\x83\\'.intern\n' \x83\\ '.lstrip.rstrip\n",b"# encoding: Windows-31J\n'\x83\\'.to_sym\n' \x83\\ '.strip\n"),

    ] { assert_eq!(fix(d.path(),ALL,s,0),after); }
}
#[test]
fn invalid_trim_encodings_keep_exception_class_while_safe_siblings_still_fix() {
    let d = project();
    for header in ["", "# encoding: Windows-31J\n", "# encoding: UTF-8\n"] {
        let payload = if header.contains("31J") {
            "\\x83"
        } else {
            "\\xFF"
        };
        for chain in ["lstrip.rstrip", "rstrip.lstrip", "lstrip&.rstrip"] {
            let source = format!("{header}\" x \".lstrip.rstrip\n\" {payload} \".{chain}\n");
            let after = format!("{header}\" x \".strip\n\" {payload} \".{chain}\n");
            assert_eq!(fix(d.path(), STRIP, source.as_bytes(), 1), after.as_bytes());
        }
    }
    // Explicit binary strings have no invalid byte sequences; forced UTF-8
    // escapes in a legacy source must instead be validated as UTF-8.
    for (before, after) in [
        (
            "# encoding: ASCII-8BIT\n\" \\xFF \".rstrip.lstrip\n",
            "# encoding: ASCII-8BIT\n\" \\xFF \".strip\n",
        ),
        (
            "# encoding: US-ASCII\n\" \\xFF \".lstrip.rstrip\n",
            "# encoding: US-ASCII\n\" \\xFF \".strip\n",
        ),
        (
            "# encoding: Windows-31J\n\" \\u00e9 \".lstrip.rstrip\n",
            "# encoding: Windows-31J\n\" \\u00e9 \".strip\n",
        ),
    ] {
        assert_eq!(fix(d.path(), STRIP, before.as_bytes(), 0), after.as_bytes());
    }
}

#[test]
fn continuations_comments_queued_heredocs_and_commands_are_source_only() {
    let d = project();
    for s in [
        "' x '.lstrip. # keep\n rstrip\n",
        "'x'.lstrip \\\n .rstrip\n",
        "(<<~S).intern\nx\nS\n",
        "consume(<<~S, 'x'.intern)\n#{effect}\nS\n",
        "consume(<<~S)\n#{'x'.intern}\nS\n",
        "`never execute`.intern\n",
        "`never execute`.lstrip.rstrip\n",
        "(<<~`CMD`).lstrip.rstrip\nnever execute\nCMD\n",
    ] {
        unchanged(d.path(), s.as_bytes(), &["--only", ALL, "--fix", "a.rb"], 1);
    }
    // Selector-only alias edits preserve comment, continuation and block ownership.
    assert_eq!(
        fix(d.path(), METHODS, b"'x' \\\n .intern( # keep\n )\n", 0),
        b"'x' \\\n .to_sym( # keep\n )\n"
    );
}
fn ruby(executable: &str, source: &[u8]) -> Output {
    let mut child = Command::new(executable)
        .args(["-W0", "--disable-gems", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}
fn runtime_matrix(executable: &str) {
    let d = project();
    let mut changed = 0;
    for mode in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
    ] {
        for expr in [
            "'name'.intern",
            ":name.intern",
            "\"é\\0name\"&.intern()",
            "\"\\xFF\".intern",
            "\" \\0é\\t \\n\".lstrip.rstrip",
            "\" \\0é\\t \\n\"&.rstrip&.lstrip",
            "\"\\0\\0\".lstrip.rstrip",
            "\" \\xFF \".rstrip.lstrip",
            "''.lstrip.rstrip",
            "\"\\u00a0é\\u00a0\".rstrip.lstrip",
        ] {
            let source = format!("{mode}{expr}\n");
            let invalid_trim = expr.contains("\\xFF") && expr.contains("strip");
            let after = fix(d.path(), ALL, source.as_bytes(), i32::from(invalid_trim));
            if invalid_trim {
                assert_eq!(source.as_bytes(), after);
            } else {
                assert_ne!(source.as_bytes(), after);
                changed += 1;
            }
            let observe = |s: &[u8]| {
                let text = String::from_utf8_lossy(s);
                // Only this controlled expression's body is placed inside begin.
                let body = text
                    .lines()
                    .filter(|l| !l.starts_with('#'))
                    .collect::<Vec<_>>()
                    .join("\n");
                let program = format!(
                    "{mode}begin\nvalue = {body}\nif value.is_a?(String)\n p [value.bytes, value.encoding.name, value.frozen?, value.concat('!').bytes]\nelse\n p [value.class.name, value.to_s.bytes, value.encoding.name, value.frozen?, value.equal?(value.intern)]\nend\nrescue => e\n p [e.class.name, e.message]\nend\n"
                );
                ruby(executable, program.as_bytes())
            };
            let before = observe(source.as_bytes());
            let after = observe(&after);
            assert!(
                before.status.success() && after.status.success(),
                "{source}: {before:?} {after:?}"
            );
            assert_eq!(
                (before.stdout, before.stderr),
                (after.stdout, after.stderr),
                "{source}"
            );
        }
    }
    assert_eq!(changed, 27);
}
#[test]
fn controlled_host_runtime_values_encodings_errors_mutability_and_frozen_modes() {
    if Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        runtime_matrix("ruby");
    }
}
#[test]
fn controlled_host_legacy_encoded_literal_values_and_mutability_are_preserved() {
    if !Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return;
    }
    let d = project();
    for (header, body) in [
        (
            b"# encoding: ISO-8859-1\n".as_slice(),
            b"['\xe9'.intern, ' \xe9 '.rstrip.lstrip]\n".as_slice(),
        ),
        (
            b"# encoding: Windows-31J\n".as_slice(),
            b"['\x83\\'.intern, ' \x83\\ '.lstrip.rstrip]\n".as_slice(),
        ),
        (
            b"# encoding: ASCII-8BIT\n".as_slice(),
            b"[\"\\xFF\".intern, \" \\xFF \".rstrip.lstrip]\n".as_slice(),
        ),
    ] {
        let before = [header, body].concat();
        let after = fix(d.path(), ALL, &before, 0);
        assert_ne!(after, before);
        let observe = |s: &[u8]| {
            ruby("ruby", &[header, b"values = ", &s[header.len()..], b"p [values[0].to_s.bytes, values[0].encoding.name, values[1].bytes, values[1].encoding.name, values[1].frozen?, values[1].concat('!').bytes]\n"].concat())
        };
        let a = observe(&before);
        let b = observe(&after);
        assert!(a.status.success() && b.status.success(), "{a:?} {b:?}");
        assert_eq!((a.stdout, a.stderr), (b.stdout, b.stderr));
    }
}

#[test]
fn explicit_ruby34_runtime_gate() {
    let Ok(exe) = std::env::var("RUSTOCOP_RUBY34") else {
        eprintln!("SKIP: set RUSTOCOP_RUBY34 for native 3.4 runtime");
        return;
    };
    let version = Command::new(&exe).arg("--version").output().unwrap();
    assert!(String::from_utf8_lossy(&version.stdout).starts_with("ruby 3.4."));
    runtime_matrix(&exe);
}
#[test]
fn controlled_counterexamples_preserve_evaluation_and_short_circuit_exceptions() {
    if !Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return;
    }
    for (before, naive) in [
        ("nil&.lstrip.rstrip", "nil&.strip"),
        ("Object.new.intern", "Object.new.to_sym"),
        (
            "class String; def strip; :custom; end; end; ' x '.lstrip.rstrip",
            "class String; def strip; :custom; end; end; ' x '.strip",
        ),
        (
            "class String; def intern; :custom; end; end; 'x'.intern",
            "class String; def intern; :custom; end; end; 'x'.to_sym",
        ),
    ] {
        let observe = |s: &str| {
            ruby(
                "ruby",
                format!("begin; p(begin; {s}; end); rescue => e; p [e.class.name, (e.name if e.respond_to?(:name))]; end").as_bytes(),
            )
        };
        assert_ne!(observe(before).stdout, observe(naive).stdout);
    }
}
#[test]
fn controlled_unknown_receiver_effect_order_count_and_block_proc_are_preserved() {
    if !Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return;
    }
    let d = project();
    let source = br#"$events = []
class Probe
  def intern; $events << :intern; :old; end
  def to_sym; $events << :to_sym; :new; end
  def lstrip; $events << :left; self; end
  def rstrip; $events << :right; self; end
  def strip; $events << :strip; self; end
end
def value; $events << :receiver; Probe.new; end
value.intern
value.lstrip.rstrip
value.rstrip.lstrip
"x".intern(&(begin; $events << :block; nil; end))
p $events
"#;
    let after = fix(d.path(), ALL, source, 1);
    assert_eq!(after, source);
    let before = ruby("ruby", source);
    let after = ruby("ruby", &after);
    assert!(
        before.status.success() && after.status.success(),
        "{before:?} {after:?}"
    );
    assert_eq!(
        before.stdout,
        b"[:receiver, :intern, :receiver, :left, :right, :receiver, :right, :left, :block]\n"
    );
    assert_eq!((before.stdout, before.stderr), (after.stdout, after.stderr));
}

#[test]
fn actual_default_and_selected_peer_composition_stabilizes_within_six_passes() {
    let d = project();
    let source=b"[ 'name'.intern, ' x '.lstrip.rstrip, '%s' % 1, format('%{x}', x: 1), %q[word], +\"#{1}\", \"a\" \\\n \"b\" ]\n";
    let peers = format!(
        "{ALL},Style/FormatString,Style/FormatStringToken,Style/RedundantFormat,Style/StringConcatenation,Style/LineEndConcatenation,Style/RedundantLineContinuation,Style/BarePercentLiterals,Style/PercentLiteralDelimiters,Style/PercentQLiterals,Style/RedundantPercentQ,Style/RedundantCapitalW,Style/CharacterLiteral,Style/QuotedSymbols,Style/SymbolLiteral,Style/VariableInterpolation,Style/EmptyStringInsideInterpolation,Style/RedundantInterpolation,Style/RedundantInterpolationUnfreeze,Style/RedundantStringEscape,Style/RedundantHeredocDelimiterQuotes,Style/EmptyHeredoc,Style/CommandLiteral,Style/StringLiterals,Style/StringLiteralsInInterpolation,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout"
    );
    for selected in [Some(ALL), Some(peers.as_str()), None] {
        let mut old = source.to_vec();
        let mut stable = false;
        for _ in 0..6 {
            fs::write(d.path().join("a.rb"), &old).unwrap();
            let mut args = vec!["--fix", "a.rb"];
            if let Some(sel) = selected {
                args.extend(["--only", sel]);
            }
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(args)
                .output()
                .unwrap();
            assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
            let new = fs::read(d.path().join("a.rb")).unwrap();
            if Command::new("ruby")
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success())
            {
                let obs = |s: &[u8]| {
                    ruby(
                        "ruby",
                        format!("p(begin; {}; end)", String::from_utf8_lossy(s)).as_bytes(),
                    )
                };
                let before = obs(source);
                let after = obs(&new);
                assert!(
                    before.status.success() && after.status.success(),
                    "{before:?} {after:?}"
                );
                assert!(!before.stdout.is_empty());
                assert_eq!(
                    (before.stdout, before.stderr),
                    (after.stdout, after.stderr),
                    "{}",
                    String::from_utf8_lossy(&new)
                );
            }
            stable = new == old;
            old = new;
        }
        assert!(stable, "{}", String::from_utf8_lossy(&old));
        assert!(String::from_utf8_lossy(&old).contains(".strip"));
        if selected.is_some() {
            assert!(String::from_utf8_lossy(&old).contains(".to_sym"));
        }
    }
}
