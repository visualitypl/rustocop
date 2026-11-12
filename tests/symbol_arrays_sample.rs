//! Controlled Ruby programs only; no application/corpus execution.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const SYMBOL: &str = "Style/SymbolArray";
const SAMPLE: &str = "Style/Sample";
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
fn unchanged(dir: &Path, s: &[u8], args: &[&str], code: i32) -> Output {
    let p = dir.join("a.rb");
    fs::write(&p, s).unwrap();
    let time = fs::metadata(&p).unwrap().modified().unwrap();
    let o = cli(dir, args, code);
    assert_eq!(fs::read(&p).unwrap(), s);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), time);
    o
}
fn fixed_bytes(s: &[u8], only: &str) -> Vec<u8> {
    let d = project();
    fs::write(d.path().join("a.rb"), s).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
    command.current_dir(d.path());
    if !only.is_empty() {
        command.args(["--only", only]);
    }
    let o = command.args(["--fix", "a.rb"]).output().unwrap();
    assert!(matches!(o.status.code(), Some(0 | 1)), "{o:?}");
    assert!(o.stderr.is_empty(), "{o:?}");
    fs::read(d.path().join("a.rb")).unwrap()
}
fn fixed(s: &str, only: &str) -> String {
    String::from_utf8(fixed_bytes(s.as_bytes(), only)).unwrap()
}
#[test]
fn defaults_capabilities_and_fixed_ruby_api_gate() {
    use rustocop::cops::{
        Registry,
        policy::{FIXED_RUBY_VERSION, RubyVersion},
    };
    let r = Registry::new();
    assert_eq!(FIXED_RUBY_VERSION, RubyVersion(3, 4));
    for name in [SYMBOL, SAMPLE] {
        let c = r.cops().iter().find(|c| c.name() == name).unwrap();
        assert!(c.default_enabled());
        assert!(c.safe_fixes());
        assert!(c.path_policy().is_none());
        assert_eq!(
            c.ruby_versions().minimum,
            if name == SYMBOL {
                Some(RubyVersion(2, 0))
            } else {
                None
            }
        );
        assert_eq!(c.ruby_versions().maximum, None);
    }
}
#[test]
fn selectors_config_excludes_and_no_write_contract() {
    let d = project();
    for (cop, s) in [(SYMBOL, "[:one, :two]\n"), (SAMPLE, "[42].shuffle.first\n")] {
        let o = unchanged(d.path(), s.as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            "[cops.Style]\nenabled = false\n".into(),
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s.as_bytes(), &["--fix", "a.rb"], 0);
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
            format!("[cops.\"{cop}\"]\nEnforcedStyle = \"brackets\"\n"),
        )
        .unwrap();
        unchanged(d.path(), s.as_bytes(), &["--fix", "a.rb"], 2);
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn directives_all_scopes_reenable_clean_and_error_no_write() {
    let d = project();
    for (cop, s) in [(SYMBOL, "[:one, :two]"), (SAMPLE, "[42].shuffle.first")] {
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, cop.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\n{s}\n"),
                    format!("# {prefix}:disable-next {target}\n{s}\n"),
                    format!("{s} # {prefix}:disable {target}\n"),
                ] {
                    unchanged(
                        d.path(),
                        source.as_bytes(),
                        &["--only", cop, "--fix", "a.rb"],
                        0,
                    );
                }
            }
            let source = format!("# {prefix}:disable {cop}\n{s}\n# {prefix}:enable {cop}\n{s}\n");
            let o = unchanged(d.path(), source.as_bytes(), &["--only", cop, "a.rb"], 1);
            assert_eq!(
                String::from_utf8_lossy(&o.stdout)
                    .matches(&format!("({cop})"))
                    .count(),
                1
            );
        }
        for (s, code) in [("[]\n", 0), ("(\n", 2)] {
            unchanged(
                d.path(),
                s.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                code,
            );
        }
    }
}
#[test]
fn stdin_does_not_write_display_file() {
    use std::{io::Write, process::Stdio};
    let d = project();
    fs::write(d.path().join("a.rb"), "unchanged\n").unwrap();
    for (cop, s) in [(SYMBOL, "[:one,:two]\n"), (SAMPLE, "[].shuffle.first\n")] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", cop, "--stdin", "a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
        let o = child.wait_with_output().unwrap();
        assert_eq!(o.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        assert_eq!(
            fs::read_to_string(d.path().join("a.rb")).unwrap(),
            "unchanged\n"
        );
    }
}
#[test]
fn bom_crlf_data_line_and_source_observers() {
    let s = "\u{feff}x = [\r\n:one,\r\n:two,\r\n]\r\np __LINE__\r\n__END__\r\npayload\0\r\n";
    assert_eq!(
        fixed(s, SYMBOL),
        "\u{feff}x = %i[\r\none\r\ntwo\r\n]\r\np __LINE__\r\n__END__\r\npayload\0\r\n"
    );
    for (cop, s) in [
        (SYMBOL, "x = [:one, :two]"),
        (SAMPLE, "x = [42].shuffle.first"),
    ] {
        for observer in [
            "p File.read(__FILE__)",
            "p DATA.pos\n__END__\nraw",
            "ObjectSpace.each_object(Symbol) { |s| p s }",
            "RubyVM::InstructionSequence.of(method(:x))",
            "TracePoint.new(:line) {}",
        ] {
            let source = format!("{s}; {observer}");
            assert_eq!(fixed(&source, cop), source, "{source}");
        }
    }
}
#[test]
fn forced_utf8_balanced_brackets_do_not_abort_sibling_corrections() {
    let source = "x = [:\"[\\u4e2d]\", :two]\ny = [:three, :four]\n";
    let expected = "x = %I[[\\u{4e2d}] \\u{74}wo]\ny = %I[\\u{74}hree \\u{66}our]\n";
    let d = project();
    fs::write(d.path().join("a.rb"), source).unwrap();
    let o = cli(d.path(), &["--only", SYMBOL, "--fix", "a.rb"], 0);
    assert!(String::from_utf8_lossy(&o.stdout).contains("2 corrections"));
    assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), expected);
    unchanged(
        d.path(),
        expected.as_bytes(),
        &["--only", SYMBOL, "--fix", "a.rb"],
        0,
    );
}
const SERIALIZATION_CASES: [&str; 10] = [
    r#"[:"([)",:two]"#,
    r#"[:"(])",:two]"#,
    r#"[:"([[)",:two]"#,
    r#"[:"(][)",:two]"#,
    r#"[:"([x)",:two]"#,
    r#"[:"\u{e9 f1}",:two]"#,
    r#"[:"A\u4e2d\u6587",:two]"#,
    r#"[:"[\u4e2d\u6587]",:two]"#,
    r#"[:"([\u4e2d\u6587)",:two]"#,
    r#"[:"[]\u4e2d\u6587[]",:two]"#,
];
#[test]
fn cross_delimiter_and_multiglyph_edits_commit_independent_lf_crlf_siblings() {
    for encoding in [
        "UTF-8",
        "US-ASCII",
        "binary",
        "Windows-31J",
        "EUC-JP",
        "ISO-8859-1",
    ] {
        for nl in ["\n", "\r\n"] {
            for literal in SERIALIZATION_CASES {
                let source =
                    format!("# encoding: {encoding}{nl}x={literal}{nl}y=[:three,:four]{nl}");
                let d = project();
                fs::write(d.path().join("a.rb"), &source).unwrap();
                let o = cli(d.path(), &["--only", SYMBOL, "--fix", "a.rb"], 0);
                assert!(String::from_utf8_lossy(&o.stdout).contains("2 corrections"));
                let after = fs::read(d.path().join("a.rb")).unwrap();
                assert_ne!(after, source.as_bytes());
                let after_text = String::from_utf8(after.clone()).unwrap();
                assert!(after_text.contains("x=%"), "{after_text}");
                assert!(after_text.contains("y=%"), "{after_text}");
                assert_eq!(after_text.matches(nl).count(), 3);
                if encoding != "UTF-8" {
                    assert!(after.is_ascii(), "{after_text}");
                }
                unchanged(d.path(), &after, &["--only", SYMBOL, "--fix", "a.rb"], 0);
            }
        }
    }
}
#[test]
fn uncertain_edits_do_not_abort_independent_files_or_siblings() {
    let d = project();
    for (cop, s) in [
        (SAMPLE, "ary.shuffle.first\n"),
        (SAMPLE, "[1,2].shuffle.first\n"),
        (SAMPLE, "[].shuffle.first(-1)\n"),
        (SAMPLE, "[].shuffle.slice(0,-1)\n"),
        (SAMPLE, "[].shuffle.at(0,3)\n"),
        (SAMPLE, "[].shuffle.first(&factory())\n"),
        (SAMPLE, "x=<<~DOC; y=[42].shuffle.first\n[:one,:two]\nDOC\n"),
        (SAMPLE, "[].shuffle { effect }.first\n"),
        (SYMBOL, "%I[foo\\ bar#{effect} baz]\n"),
    ] {
        fs::write(d.path().join("safe.rb"), "[:one,:two]\n").unwrap();
        // A literal shuffle block is intentionally not an upstream candidate.
        let code = if s.contains("shuffle {") { 0 } else { 1 };
        unchanged(
            d.path(),
            s.as_bytes(),
            &[
                "--only",
                &format!("{cop},{SYMBOL}"),
                "--fix",
                "a.rb",
                "safe.rb",
            ],
            code,
        );
        assert_eq!(
            fs::read_to_string(d.path().join("safe.rb")).unwrap(),
            "%i[one two]\n"
        );
    }
    for s in [
        "x=1; [effect()].shuffle.first",
        "class Array; def sample; :changed; end; end; [].shuffle.first",
        "[42].shuffle(random: random).first",
        "[42].shuffle.first { effect }",
        "[].shuffle.slice(0, -1)",
    ] {
        assert_eq!(fixed(s, SAMPLE), s);
    }
}
fn runtimes() -> Vec<String> {
    let mut out = Vec::new();
    if Command::new("ruby").arg("--version").output().is_ok() {
        out.push("ruby".into());
    }
    if let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") {
        out.push(ruby);
    }
    out
}
fn observe(ruby: &str, s: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("program.rb");
    fs::write(&p, s).unwrap();
    let o = Command::new(ruby)
        .current_dir(d.path())
        .arg("-W:deprecated")
        .arg("program.rb")
        .output()
        .unwrap();
    assert!(o.status.success(), "{ruby}: {o:?}");
    (o.stdout, o.stderr)
}
#[test]
fn controlled_symbol_values_encodings_intern_identity_and_warnings_all_frozen_modes() {
    for ruby in runtimes() {
        for magic in [
            "",
            "# frozen_string_literal: true\n",
            "# frozen_string_literal: false\n",
        ] {
            for (literal, count) in [
                ("[:one,:two]", 2),
                ("[:\"\u{85}\",:two]", 2),
                ("[:\"one\\\r\ntwo\",:three]", 2),
                ("[:'b[]',:'c[][]']", 2),
                (r#"[:"\t", :"\n", :three]"#, 3),
                (r#"[:"\u4e2d", :"\u0061"]"#, 2),
                (r#"[:"[\u4e2d]", :two]"#, 2),
                (r#"[:"[]\u4e2d[]", :two]"#, 2),
                (r#"[:"\u4e2d", :"[]"]"#, 2),
                (r#"[:"a\\b", :'#{effect}']"#, 2),
                ("[:\"\t\", :\"two\n\", :three]", 3),
                (r"%i[one \[ \] two]", 4),
                (r"%I[one two\ three]", 2),
                ("%i[a\\ b\\\nc d\\ e\\\nf]", 2),
            ] {
                let s = format!(
                    "{magic}x={literal}\np x.map {{ |s| [s.to_s.bytes,s.encoding.name,s.frozen?,s.equal?(s.to_s.to_sym)] }}\np [x.length,x[0].equal?(x[0]),__LINE__]\nx << :extra\np x.length\n"
                );
                let after = fixed(&s, SYMBOL);
                assert_ne!(after, s, "{ruby}: {s}");
                let before_observation = observe(&ruby, s.as_bytes());
                assert_eq!(
                    before_observation,
                    observe(&ruby, after.as_bytes()),
                    "{ruby}: {s} => {after}"
                );
                assert!(
                    String::from_utf8_lossy(&before_observation.0)
                        .contains(&format!("[{count}, true,"))
                );
            }
        }
    }
}
#[test]
fn encoded_symbol_delimiters_use_character_heads_and_uncertain_encoding_edits_are_withheld() {
    // Windows-31J ソ has a backslash trail; Ы has a '[' trail, Ъ a ']'.
    for (encoding, bytes) in [
        ("Windows-31J", vec![0x83, 0x5c]),
        ("Windows-31J", vec![0x84, 0x5b]),
        ("Windows-31J", vec![0x84, 0x5d]),
        ("EUC-JP", vec![0xa4, 0xa2]),
        ("ISO-8859-1", vec![0xe9]),
    ] {
        let mut s = format!("# encoding: {encoding}\nx=[:\"").into_bytes();
        s.extend(&bytes);
        s.extend(
            b"\",:two]\np x.map { |s| [s.to_s.bytes,s.encoding.name,s.equal?(s.to_s.to_sym)] }\n",
        );
        let d = project();
        let o = unchanged(d.path(), &s, &["--only", SYMBOL, "--fix", "a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(SYMBOL));
        assert_eq!(fixed_bytes(&s, SYMBOL), s);
        for (tail, code) in [
            (b" two|\n".as_slice(), 0),
            (b" two\\ three|\n".as_slice(), 1),
        ] {
            let mut percent = format!("# encoding: {encoding}\nx=%i|").into_bytes();
            percent.extend(&bytes);
            percent.extend(tail);
            unchanged(
                d.path(),
                &percent,
                &["--only", SYMBOL, "--fix", "a.rb"],
                code,
            );
        }
        // The optional miniruby has no loadable legacy codecs. Actual legacy
        // encoding observations use the host Ruby; native 3.4 covers UTF8/binary.
        if Command::new("ruby").arg("--version").output().is_ok() {
            assert_eq!(
                observe("ruby", &s),
                observe("ruby", &fixed_bytes(&s, SYMBOL))
            );
        }
    }
    let s = "# encoding: binary\nx=[:\"\\u4e2d\",:two]\np x.map { |s| [s.to_s.bytes,s.encoding.name] }\n";
    let after = fixed(s, SYMBOL);
    assert_ne!(s, after);
    for ruby in runtimes() {
        assert_eq!(
            observe(&ruby, s.as_bytes()),
            observe(&ruby, after.as_bytes())
        );
    }
}
#[test]
fn serialization_regressions_preserve_native_source_runtime_and_converge() {
    // Real native bytes alongside forced-UTF8 symbols ensure the header keeps
    // governing ordinary strings. Windows-31J includes a backslash trail byte.
    let profiles: [(&str, &[u8]); 6] = [
        ("UTF-8", "é".as_bytes()),
        ("US-ASCII", b"native"),
        ("binary", b"\xe9"),
        ("Windows-31J", b"\x83\x5c"),
        ("EUC-JP", b"\xa4\xa2"),
        ("ISO-8859-1", b"\xe9"),
    ];
    let runtimes = runtimes();
    parallel_rows!(for (encoding, native) in profiles => {
        // miniruby lacks loadable legacy codecs; native 3.4.1 covers UTF8,
        // US-ASCII and binary, while host Ruby covers every native profile.
        let rubies: Vec<_> = runtimes
            .iter()
            .filter(|r| r.as_str() == "ruby" || matches!(encoding, "UTF-8" | "US-ASCII" | "binary"))
            .collect();
        for fsl in [
            "",
            "# frozen_string_literal: true\n",
            "# frozen_string_literal: false\n",
        ] {
            for nl in ["\n", "\r\n"] {
                for literal in SERIALIZATION_CASES {
                    let mut source =
                        format!("# encoding: {encoding}\n{fsl}x={literal}\ny=[:three,:four]\nz=\"")
                            .replace('\n', nl)
                            .into_bytes();
                    source.extend_from_slice(native);
                    source.extend_from_slice(
                        format!("\"{nl}v=[42].shuffle.first{nl}line=__LINE__{nl}").as_bytes(),
                    );
                    let observe_source = |bytes: &[u8]| {
                        let start: usize = bytes
                            .split_inclusive(|&b| b == b'\n')
                            .take_while(|line| {
                                line.starts_with(b"#") || line.iter().all(u8::is_ascii_whitespace)
                            })
                            .map(<[u8]>::len)
                            .sum();
                        let mut program = bytes[..start].to_vec();
                        program.extend_from_slice(b"srand(17)\n");
                        program.extend_from_slice(&bytes[start..]);
                        program.extend_from_slice(br#"p [__ENCODING__.name,line,v]
p [x,y].map { |a| [a.frozen?,a.map { |s| [s.to_s.bytes,s.encoding.name,s.frozen?,s.equal?(s.to_s.to_sym)] }] }
x << :extra
p [x.length,y.length]
p [z.bytes,z.encoding.name,z.frozen?]
begin
  z << "?"
rescue => e
  p [e.class.name,e.message.bytes]
end
p [z.bytes,z.frozen?]
p Array.new(4) { rand(2**30) }
"#);
                        program
                    };
                    let before: Vec<_> = rubies
                        .iter()
                        .map(|r| observe(r, &observe_source(&source)))
                        .collect();
                    for only in [
                        "Style/SymbolArray,Style/Sample",
                        "Style/SymbolArray,Style/Sample,Style/QuotedSymbols,Style/SymbolLiteral,Style/PercentLiteralDelimiters,Style/RedundantCapitalW,Style/RedundantPercentQ,Style/RedundantStringEscape,Style/StringLiterals,Style/WordArray,Layout",
                        "",
                    ] {
                        let mut current = source.clone();
                        let mut stable = false;
                        for turn in 0..6 {
                            let after = fixed_bytes(&current, only);
                            if turn == 0 {
                                assert_ne!(after, current, "{encoding}: {literal}");
                                // Both named symbol candidates must actually
                                // correct, not be hidden/held to avoid an abort.
                                let compact: Vec<_> = after
                                    .iter()
                                    .copied()
                                    .filter(|b| !b.is_ascii_whitespace())
                                    .collect();
                                assert!(compact.windows(3).any(|w| w == b"x=%"));
                                assert!(compact.windows(3).any(|w| w == b"y=%"));
                            }
                            if current == after {
                                stable = true;
                                break;
                            }
                            for (ruby, expected) in rubies.iter().zip(&before) {
                                assert_eq!(
                                    *expected,
                                    observe(ruby, &observe_source(&after)),
                                    "{ruby} / {encoding} / {literal} / {only} / pass {turn}"
                                );
                            }
                            current = after;
                        }
                        assert!(stable, "{encoding} / {literal} / {only}");
                    }
                }
            }
        }
    });
}
#[test]
fn controlled_sample_corrections_preserve_default_rng_following_draws_and_identity() {
    for ruby in runtimes() {
        for magic in [
            "",
            "# frozen_string_literal: true\n",
            "# frozen_string_literal: false\n",
        ] {
            for array in ["[]", "[42]", "[:one]", "[\"value\"]", "[[42]]", "%i[one]"] {
                for tail in [
                    ".first",
                    ".last",
                    ".at(0)",
                    ".slice(-1)",
                    "[0]",
                    ".first(0)",
                    ".last(0)",
                ] {
                    let s = format!(
                        "{magic}srand(17)\nx={array}.shuffle{tail}\np [x, x.frozen?, x.equal?(\"value\"), x.equal?(:one), x.equal?([]), rand(100000),rand(100000),__LINE__]\nif x.is_a?(String) && !x.frozen?\n  x << \"!\"\n  p x\nend\n"
                    );
                    // srand is deliberately an unknown completed call to the
                    // production proof. Correct the isolated expression, then
                    // insert it into a controlled runtime harness.
                    let before = format!("{array}.shuffle{tail}");
                    let after = fixed(&before, SAMPLE);
                    assert_ne!(before, after);
                    assert_eq!(
                        observe(&ruby, s.as_bytes()),
                        observe(&ruby, s.replace(&before, &after).as_bytes()),
                        "{ruby}: {before}"
                    );
                }
            }
        }
    }
}
#[test]
fn controlled_rng_dispatch_conversion_error_and_evaluation_order_witnesses() {
    for ruby in runtimes() {
        let s = r#"def outcome
  value=yield
  [value,rand(100000),rand(100000)]
rescue => e
  [e.class.name,e.message,rand(100000)]
end
srand(17); a=outcome { [1,2].shuffle.first }
srand(17); b=outcome { [1,2].sample }
p [a,b,a==b]
srand(17); a=outcome { [1,2,3,4,5,6,7,8].shuffle.first }
srand(17); b=outcome { [1,2,3,4,5,6,7,8].sample }
p [a,b,a==b]
raise 'larger-array default RNG witness lost' if a.drop(1)==b.drop(1)
p outcome { [].shuffle.first(-1) }
p outcome { [].sample(-1) }
p outcome { [].shuffle.slice(0,-1) }
p outcome { [].sample(-1) }
class Generator
  attr_reader :trace
  def initialize; @trace=[]; end
  def rand(n); @trace << n; 0; end
end
g=Generator.new; p [[1,2,3].shuffle(random:g).first,g.trace]
g=Generator.new; p [[1,2,3].sample(random:g),g.trace]
trace=[]
a=Object.new
 a.define_singleton_method(:shuffle) { |**kw,&block| trace << [:shuffle,kw,!!block]; [9,8] }
 a.define_singleton_method(:sample) { |*args| trace << [:sample,args]; 7 }
p [a.shuffle.first,a.sample,trace]
trace=[]
p [([trace << 1,trace << 2].shuffle.first {trace << 3}),trace]
"#;
        let o = observe(&ruby, s.as_bytes());
        let text = String::from_utf8(o.0).unwrap();
        assert!(
            text.lines().next().unwrap().ends_with("false]"),
            "{ruby}: {text}"
        );
        assert!(text.contains("negative array size"));
        assert!(text.contains("negative sample number"));
        assert!(text.contains(":shuffle"));
        assert!(text.contains(":sample"));
        assert!(text.contains("[3, 2"), "{ruby}: {text}");
        assert!(text.contains("[1, [3]]"), "{ruby}: {text}");
        assert!(
            text.contains("[1, 2]"),
            "ignored literal block: {ruby}: {text}"
        );
    }
}
#[test]
fn curated_peer_and_default_composition_converges_without_validation_abort() {
    for source in [
        "x = [:one, :two]; y = [42].shuffle.first\n",
        "x = %i[one two\\ three]\np x\n",
        "x = [\r\n:one,\r\n:two,\r\n]\r\np __LINE__\r\n",
    ] {
        for only in [
            "Style/SymbolArray,Style/Sample",
            "Style/SymbolArray,Style/Sample,Style/StringLiterals,Style/SymbolLiteral,Style/PercentLiteralDelimiters,Style/TrailingCommaInArrayLiteral,Layout",
            "",
        ] {
            let mut s = source.to_owned();
            let mut stable = false;
            for _ in 0..6 {
                let next = fixed(&s, only);
                if next == s {
                    stable = true;
                    break;
                }
                s = next;
            }
            assert!(stable, "{only}: {s}");
            for ruby in runtimes() {
                assert_eq!(
                    observe(&ruby, source.as_bytes()),
                    observe(&ruby, s.as_bytes()),
                    "{only}: {s}"
                );
            }
        }
    }
}
