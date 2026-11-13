//! controlled synthetic programs only; command literals are never run.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const BRACES: &str = "Style/HashAsLastArrayItem";
const CONVERSION: &str = "Style/HashConversion";
const BOTH: &str = "Style/HashAsLastArrayItem,Style/HashConversion";
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
fn fixed(s: &[u8], only: &str) -> Vec<u8> {
    let d = project();
    let p = d.path().join("a.rb");
    fs::write(&p, s).unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_rustocop"));
    c.current_dir(d.path());
    if !only.is_empty() {
        c.args(["--only", only]);
    }
    let o = c.args(["--fix", "a.rb"]).output().unwrap();
    assert!(matches!(o.status.code(), Some(0 | 1)), "{only}: {o:?}");
    assert!(o.stderr.is_empty(), "{o:?}");
    fs::read(p).unwrap()
}
#[test]
fn registration_defaults_capabilities_and_plain_lint_are_read_only() {
    let d = project();
    let r = rustocop::cops::Registry::new();
    for (cop, s, safe) in [
        (BRACES, "p [a: 1]\n", true),
        (CONVERSION, "p Hash[ary]\n", false),
    ] {
        let cops: Vec<_> = r.cops().iter().filter(|c| c.name() == cop).collect();
        assert_eq!(cops.len(), 1);
        let c = cops[0];
        assert!(c.default_enabled());
        assert_eq!(c.safe_fixes(), safe);
        assert!(c.path_policy().is_none());
        assert!(c.ruby_versions().minimum.is_none());
        assert!(c.ruby_versions().maximum.is_none());
        assert!(c.max_setting().is_none());
        let o = unchanged(d.path(), s.as_bytes(), &["a.rb"], 1);
        assert!(String::from_utf8_lossy(&o.stdout).contains(cop));
        unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
    }
}
#[test]
fn selectors_config_fixed_options_and_relative_excludes() {
    let d = project();
    fs::create_dir(d.path().join("lib")).unwrap();
    for (cop, s, after, safe) in [
        (BRACES, "p [a: 1]\n", "p [{a: 1}]\n", true),
        (CONVERSION, "p Hash[ary]\n", "p Hash[ary]\n", false),
    ] {
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", cop, "--except", cop, "--fix", "a.rb"],
            0,
        );
        for config in [
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(d.path(), s.as_bytes(), &["--fix", "a.rb"], 0);
            unchanged(d.path(), s.as_bytes(), &["--only", cop, "a.rb"], 1);
        }
        fs::write(d.path().join("rustocop.toml"),format!("[cops.Style]\nenabled = false\n[cops.Layout]\nenabled = false\n[cops.\"{cop}\"]\nenabled = true\n")).unwrap();
        cli(d.path(), &["--fix", "a.rb"], i32::from(!safe));
        assert_eq!(fs::read_to_string(d.path().join("a.rb")).unwrap(), after);
        unchanged(
            d.path(),
            after.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            i32::from(!safe),
        );
        for config in [
            "exclude = [\"lib/a.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude = [\"lib/a.rb\"]\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                &d.path().join("lib"),
                s.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        let option = if safe {
            "EnforcedStyle = \"no_braces\""
        } else {
            "AllowSplatArgument = false"
        };
        fs::write(
            d.path().join("rustocop.toml"),
            format!("[cops.\"{cop}\"]\n{option}\n"),
        )
        .unwrap();
        unchanged(d.path(), s.as_bytes(), &["--fix", "a.rb"], 2);
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn directives_all_forms_and_atomic_interior_disabled_spans() {
    let d = project();
    for (cop, s) in [(BRACES, "p [a: 1]"), (CONVERSION, "p Hash[ary]")] {
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, cop.strip_prefix("Style/").unwrap(), "Style", "all"] {
                for s in [
                    format!("# {prefix}:disable {target}\n{s}\n"),
                    format!("# {prefix}:disable-next {target}\n{s}\n"),
                    format!("{s} # {prefix}:disable {target}\n"),
                ] {
                    unchanged(d.path(), s.as_bytes(), &["--only", cop, "--fix", "a.rb"], 0);
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
    }
    let s = "p [a: 1,\n# rubocop:disable Style/HashAsLastArrayItem\nb: 2\n# rubocop:enable Style/HashAsLastArrayItem\n]\np [c: 3]\n";
    let a = String::from_utf8(fixed(s.as_bytes(), BRACES)).unwrap();
    assert_eq!(a, s.replace("[c: 3]", "[{c: 3}]"));
}
#[test]
fn stdin_never_writes_and_clean_unsafe_withheld_error_files_hold_mtime() {
    use std::{io::Write, process::Stdio};
    let d = project();
    for (s, code) in [
        ("p [{a: 1}]; p [**nil]\n", 0),
        ("p Hash[ary]\n", 1),
        ("p [a: 1]; p DATA.pos\n__END__\npayload\n", 1),
        ("p [a: 1\n", 2),
    ] {
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", BOTH, "--fix", "a.rb"],
            code,
        );
    }
    for (cop, s) in [(BRACES, "[a: 1]\n"), (CONVERSION, "Hash[ary]\n")] {
        let mut c = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
            .args(["--only", cop, "--stdin", "a.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        c.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
        assert_eq!(c.wait_with_output().unwrap().status.code(), Some(1));
        assert_eq!(
            fs::read_to_string(d.path().join("a.rb")).unwrap(),
            "p [a: 1\n"
        );
    }
}
#[test]
fn envelopes_physical_lines_data_payload_comments_and_heredoc_ownership() {
    for s in [
        "\u{feff}# encoding: UTF-8\r\np [\r\n a: __LINE__, # before\r\n \"é\" => __LINE__ # after\r\n]\r\np __LINE__\r\n__END__\r\npayload\0\r\n".as_bytes().to_vec(),
        b"# encoding: ISO-8859-1\r\np [a: 1]\r\np \"caf\xe9\"\r\np __LINE__\r\n__END__\r\nraw\xff\r\n".to_vec(),
        b"x=<<~A; y=[a: <<~B]\r\nfirst\r\nA\r\nsecond\r\nB\r\np [\r\n a: <<~C,\r\nthird\r\nC\r\n b: __LINE__\r\n]\r\n".to_vec(),
        b"p [a: <<~DOC]\n#{[inner: __LINE__]}\nDOC\np [sibling: __LINE__]\n".to_vec(),
    ] {
        let a=fixed(&s,BRACES); assert_ne!(a,s);
        assert_eq!(s.iter().filter(|&&b|b==b'\n').count(),a.iter().filter(|&&b|b==b'\n').count());
        assert_eq!(s.iter().filter(|&&b|b==b'\r').count(),a.iter().filter(|&&b|b==b'\r').count());
        // Wrappers insert only braces, never alter any original operand/trivia.
        let mut i=0;
        for &b in &a { if s.get(i)==Some(&b) {i+=1;} else {assert!(b==b'{' || b==b'}');} }
        assert_eq!(i,s.len()); assert_eq!(fixed(&a,BRACES),a);
    }
}
#[test]
fn individual_encoding_withhold_does_not_abort_safe_siblings_or_files() {
    let d = project();
    let s = b"# encoding: ISO-8859-1\np [\"caf\xe9\" => 1]\np [safe: 2]\n";
    fs::write(d.path().join("safe.rb"), "p [safe: 3]\n").unwrap();
    let o = unchanged(
        d.path(),
        b"# encoding: ISO-8859-1\np [\"caf\xe9\" => 1]\n",
        &["--only", BRACES, "--fix", "a.rb", "safe.rb"],
        1,
    );
    assert!(String::from_utf8_lossy(&o.stdout).contains("1 correction"));
    assert_eq!(
        fs::read_to_string(d.path().join("safe.rb")).unwrap(),
        "p [{safe: 3}]\n"
    );
    assert_eq!(
        fixed(s, BRACES),
        b"# encoding: ISO-8859-1\np [\"caf\xe9\" => 1]\np [{safe: 2}]\n"
    );
}
#[test]
fn shareable_assignment_withholds_hold_bytes_mtime_and_never_abort_safe_siblings() {
    let d = project();
    for mode in ["literal", "experimental_copy", "experimental_everything"] {
        for nl in ["\n", "\r\n"] {
            for value in ["1", "\"x\"", "<<~DOC]\nbody\nDOC\n#"] {
                let source = format!("# shareable_constant_value: {mode}\nC=[a:{value}]\n")
                    .replace('\n', nl);
                fs::write(d.path().join("safe.rb"), "x=[b:2]\n").unwrap();
                let out = unchanged(
                    d.path(),
                    source.as_bytes(),
                    &["--only", BRACES, "--fix", "a.rb", "safe.rb"],
                    1,
                );
                let stdout = String::from_utf8(out.stdout).unwrap();
                assert!(stdout.contains(BRACES), "{stdout}");
                assert!(stdout.contains("1 correction"), "{stdout}");
                assert_eq!(fs::read(d.path().join("safe.rb")).unwrap(), b"x=[{b:2}]\n");
                let with_sibling = format!("{source}x=[b:2]{nl}");
                assert_eq!(
                    fixed(with_sibling.as_bytes(), BRACES),
                    format!("{source}x=[{{b:2}}]{nl}").as_bytes()
                );
            }
        }
    }
}
#[test]
fn source_metadata_heap_and_data_observers_retain_offenses_without_edits() {
    let d = project();
    for observer in [
        "p File.read(__FILE__)",
        "p DATA.pos\n__END__\nraw",
        "ObjectSpace.each_object(Hash) { |x| p x }",
        "RubyVM::InstructionSequence.of(method(:x))",
        "TracePoint.new(:line) {}",
    ] {
        let s = format!("p [a: 1]\n{observer}\n");
        unchanged(
            d.path(),
            s.as_bytes(),
            &["--only", BRACES, "--fix", "a.rb"],
            1,
        );
    }
}
fn rubies() -> Vec<String> {
    let mut r = Vec::new();
    if Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        r.push("ruby".to_owned());
    } else {
        eprintln!("host Ruby runtime gate skipped: ruby is unavailable");
    }
    if let Ok(path) = crate::support::runtime::ruby() {
        let o = Command::new(&path)
            .args(["-e", "print RUBY_VERSION"])
            .output()
            .unwrap();
        assert!(o.status.success());
        assert!(
            o.stdout
                .starts_with(crate::support::runtime::version_prefix().as_bytes()),
            "{o:?}"
        );
        r.push(path);
    } else {
        eprintln!("native Ruby 3.4 gate skipped: set RUSTOCOP_RUBY34");
    }
    r
}
fn observe(ruby: &str, source: &[u8]) -> Output {
    let d = project();
    let p = d.path().join("program.rb");
    fs::write(&p, source).unwrap();
    let o = Command::new(ruby)
        .current_dir(d.path())
        .args(["-W:deprecated", "program.rb"])
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "{ruby}: {o:?}\n{}",
        String::from_utf8_lossy(source)
    );
    o
}
fn equal_runtime(ruby: &str, before: &[u8], after: &[u8]) {
    let b = observe(ruby, before);
    let a = observe(ruby, after);
    assert_eq!(
        String::from_utf8_lossy(&a.stdout),
        String::from_utf8_lossy(&b.stdout),
        "{ruby}: {}",
        String::from_utf8_lossy(after)
    );
    // No file/line-sensitive warnings should be introduced by wrappers.
    assert_eq!(
        String::from_utf8_lossy(&a.stderr),
        String::from_utf8_lossy(&b.stderr),
        "{ruby}"
    );
}
const RUNTIME: &str = r#"
$trace=[]
def mark(x); $trace << x; x; end
class Key
  attr_reader :name
  def initialize(name); @name=name; end
  def hash; $trace << [:hash,@name]; 7; end
  def eql?(other); $trace << [:eql,@name,other.class.name]; other.is_a?(Key) && @name==other.name; end
end
class Options
  def initialize(hash); @hash=hash; end
  def to_hash; $trace << :to_hash; @hash; end
end
def snapshot(x)
  case x
  when String; [x.bytes,x.encoding.name,x.frozen?]
  when Array; [:array,x.length,x.frozen?,x.map { |v| snapshot(v) }]
  when Hash
    [:hash,x.length,x.frozen?,x.compare_by_identity?,Hash.ruby2_keywords_hash?(x),x.default,x.default_proc.nil?,x.keys.map { |k| snapshot(k) },x.values.map { |v| snapshot(v) },x[x.keys.first.is_a?(Key) ? Key.new(:missing) : :missing]]
  when Key; [:key,x.name]
  else; x
  end
end
def probe
  $trace=[]
  begin
    x=yield
    p [snapshot(x),$trace]
    if x.last.is_a?(Hash)
      y=x.last
      p [y.equal?(x.last),y.keys.first.equal?(y.keys.first)]
      y[:new]=:mutated
      p snapshot(x)
    end
  rescue => e
    p [e.class.name,e.message,$trace]
  end
end
def forward(*a,**k); [a,k]; end
seed={a: 99, "string" => [1]}; seed.default=:default
options=Options.new(seed)
empty=Options.new({})
key=Key.new(:key)
other=Key.new(:other)
probe { [mark(:first), a: mark(:a), "string" => mark(:v), mark(:dynamic) => mark(:last)] }
probe { [a: 1, **options, b: 2, a: 3] }
probe { [a: 1, **empty] }
probe { [a: 1, **{}] }
probe { [a: 1, **nil] }
probe { [key => mark(:one), other => mark(:two), key => mark(:three)] }
probe { ["é" => %q( value\ ), :é => "value", a: __LINE__, b: __LINE__] }
probe { [a: <<~DOC, b: __LINE__] }
heredoc #{mark(:interpolation)}
DOC
probe { [mark(:first),
 a: mark(:one),
 b: __LINE__
] }
probe { [**nil] }
probe { [**{}] }
probe { [**empty] }
probe { [**options, a: 1] }
identity={}.compare_by_identity; identity[key]=[1]; identity.default_proc=proc { |h,k| [:missing,k] }
probe { [other => 1, **identity] }
probe { [a: 1, **Object.new] }
a=7
probe { [a:] }
probe { [a: [nested: mark(:nested)]] }
x=[a: 1]; p snapshot(forward(*x)); p snapshot(forward(**x.last))
srand(123); probe { [a: rand(100), b: rand(100)] }; p rand(100)
value=[]; string_key="mutable"; x=[string_key => value, a: value, key => value]
p [x.last.keys.first.equal?(string_key), x.last.keys.last.equal?(key), x.last[:a].equal?(value), x.last[string_key].equal?(x.last[key])]
x.last[:a] << :mutation
p [value, x.last[string_key], x.last[key], string_key.frozen?]
def flagged(*args); args; end
ruby2_keywords(:flagged)
kw=flagged(a: 1).last
p Hash.ruby2_keywords_hash?(kw)
probe { [a: 0, **kw] }
"#;
#[test]
fn controlled_runtime_preserves_graph_flags_defaults_callbacks_binding_and_order() {
    let rubies = rubies();
    for mode in [
        "",
        "# frozen_string_literal: true\n",
        "# frozen_string_literal: false\n",
    ] {
        for nl in ["\n", "\r\n"] {
            let s = format!("{mode}{RUNTIME}").replace('\n', nl);
            let a = fixed(s.as_bytes(), BRACES);
            assert_ne!(a, s.as_bytes());
            let second = fixed(&a, BRACES);
            assert_eq!(fixed(&second, BRACES), second);
            for ruby in &rubies {
                equal_runtime(ruby, &a, &second);
            }
            assert!(String::from_utf8_lossy(&a).contains("[**nil]"));
            assert!(String::from_utf8_lossy(&a).contains("[**{}]"));
            for ruby in &rubies {
                equal_runtime(ruby, s.as_bytes(), &a);
            }
        }
    }
}
#[test]
fn native_shareable_literal_exception_is_not_replaced_by_successful_deep_freeze() {
    for ruby in rubies().into_iter().filter(|ruby| ruby != "ruby") {
        for value in ["1", "\"x\""] {
            let source = format!(
                "# shareable_constant_value: literal\nC=[a:{value}]\nx=[b:2]\np [C.frozen?, C[0].frozen?, C[0][:a].frozen?]\n"
            );
            let after = fixed(source.as_bytes(), BRACES);
            assert_eq!(after, source.replace("x=[b:2]", "x=[{b:2}]").as_bytes());
            let run = |s: &[u8]| {
                Command::new(&ruby)
                    .args(["-W:deprecated", "-e"])
                    .arg(std::str::from_utf8(s).unwrap())
                    .output()
                    .unwrap()
            };
            let before = run(source.as_bytes());
            let actual = run(&after);
            assert_eq!(before.status.code(), Some(1));
            assert!(before.stdout.is_empty());
            assert!(
                String::from_utf8_lossy(&before.stderr)
                    .contains("cannot assign unshareable object to C (Ractor::IsolationError)")
            );
            assert_eq!(actual.status.code(), before.status.code());
            assert_eq!(actual.stdout, before.stdout);
            assert_eq!(actual.stderr, before.stderr);
            // Counterexample: syntax-only validation accepts a behavior change.
            let unsafe_edit =
                source.replace(&format!("C=[a:{value}]"), &format!("C=[{{a:{value}}}]"));
            let witness = run(unsafe_edit.as_bytes());
            assert!(witness.status.success(), "{witness:?}");
            assert_eq!(witness.stdout, b"[true, true, true]\n");
        }
    }
}
#[test]
fn shareable_runtime_modes_constant_forms_errors_state_and_unprotected_controls() {
    if crate::support::runtime::skips(
        "Ruby 4.0 stops with a segmentation fault on this program (Ruby Bug #22543)",
    ) {
        return;
    }
    let rubies = rubies();
    for mode in [
        "literal",
        "experimental_copy",
        "experimental_everything",
        "none",
    ] {
        for lhs in ["C", "::C", "M::C", "::M::C"] {
            for op in ["=", "||=", "&&=", "+="] {
                let seed = match op {
                    "||=" => format!("{lhs}=nil\n"),
                    "&&=" | "+=" => format!("{lhs}=[].freeze\n"),
                    _ => String::new(),
                };
                let s = format!(
                    "# shareable_constant_value: none\nmodule M; end\n{seed}# shareable_constant_value: {mode}\n$trace=[]\nbegin\n {lhs}{op}[a: ($trace << :value; \"x\"), line: __LINE__]\nrescue => e\n p [e.class.name,e.message]\nend\nx=[b: ($trace << :sibling; 2)]\np [$trace, defined?({lhs}), x, x.frozen?, x[0].frozen?]\nif defined?({lhs})\n v={lhs}\n p [v, Ractor.shareable?(v), v.frozen?, v.is_a?(Array) && v.last.is_a?(Hash) && v.last.frozen?]\nend\n"
                );
                let a = fixed(s.as_bytes(), BRACES);
                assert_ne!(a, s.as_bytes()); // The ordinary sibling always fixes.
                assert_eq!(fixed(&a, BRACES), a);
                for ruby in &rubies {
                    equal_runtime(ruby, s.as_bytes(), &a);
                }
            }
        }
        let s = format!(
            "# shareable_constant_value: {mode}\nmodule M; end\nA,M::B=[a:1],[b:\"x\"]\nx=[c:2]\np [A,M::B,x,Ractor.shareable?(A),Ractor.shareable?(M::B)]\n"
        );
        let a = fixed(s.as_bytes(), BRACES);
        assert_ne!(a, s.as_bytes());
        for ruby in &rubies {
            equal_runtime(ruby, s.as_bytes(), &a);
        }
    }
    let s = b"module M\n# shareable_constant_value: literal\nbegin; C=[a:1]; rescue => e; p [e.class.name,e.message]; end\n# shareable_constant_value: none\nD=[b:2]\nend\nE=[c:3]\nclass K\n# shareable_constant_value: literal\nbegin; C=[a:\"x\"]; rescue => e; p [e.class.name,e.message]; end\nend\nF=[d:4]\np [M::D,E,F,Ractor.shareable?(E),Ractor.shareable?(F)]\n";
    let a = fixed(s, BRACES);
    assert_ne!(a, s);
    for ruby in &rubies {
        equal_runtime(ruby, s, &a);
    }
}
#[test]
fn native_first_kwsplat_exemption_and_unsafe_conversion_witnesses() {
    let d = project();
    let s = r#"begin; p [**nil]; rescue TypeError; p :nil_error; end
begin; p [{**nil}]; rescue TypeError; p :nil_error; end
p [**{}]; p [{**{}}]
p Hash[[[1,2],[3]]]
begin; p [[1,2],[3]].to_h; rescue => e; p [e.class.name,e.message]; end
class Custom
  def to_hash; {a: 1}; end
end
p Hash[Custom.new]
begin; p Custom.new.to_h; rescue NoMethodError; p :no_to_h; end
"#;
    for ruby in rubies() {
        let o = observe(&ruby, s.as_bytes());
        let out = String::from_utf8(o.stdout).unwrap();
        if ruby != "ruby" {
            assert!(out.starts_with("[]\n[{}]\n[]\n[{}]\n"), "{ruby}: {out}");
        }
        assert!(out.contains("[]\n[{}]\n"), "{ruby}: {out}");
        assert!(out.contains("ArgumentError"));
        assert!(out.contains("no_to_h"));
    }
    unchanged(
        d.path(),
        s.as_bytes(),
        &["--only", BOTH, "--fix", "a.rb"],
        1,
    );
}
#[test]
fn controlled_legacy_encodings_bom_data_and_line_values() {
    for ruby in rubies() {
        for s in [
            "\u{feff}# encoding: UTF-8\r\na=[\r\n \"é\" => [\"é\".encoding.name,__LINE__],\r\n line: __LINE__]\r\np a\r\np [__LINE__]\r\n__END__\r\nraw\0\r\n".as_bytes().to_vec(),
            b"# encoding: ISO-8859-1\r\nv=\"caf\xe9\"; a=[key: v.bytes, line: __LINE__]\r\np a\r\np [__LINE__]\r\n__END__\r\nraw\xff\r\n".to_vec(),
        ] {
            let a=fixed(&s,BRACES); assert_ne!(a,s); equal_runtime(&ruby,&s,&a);
        }
    }
}
#[test]
fn selected_peers_and_actual_defaults_stabilize_within_six_explicit_passes() {
    let s =
        "x = [1, :a => 2, b: 3,]\np(x)\nx = [a: __LINE__, b: \"value\"]\np(x)\nHash[[[1,2],[3]]]\n";
    for selector in [
        BOTH,
        "Style/HashAsLastArrayItem,Style/HashConversion,Style/TrailingCommaInArrayLiteral,Style/TrailingCommaInHashLiteral,Style/MethodCallWithArgsParentheses,Style/HashSyntax,Style/StringLiterals,Style/RedundantReturn,Style/Semicolon,Style/SignalException,Style/MethodCallWithoutArgsParentheses,Style/WordArray,Style/SymbolArray,Style/Sample,Layout",
        "Style,Layout",
        "",
    ] {
        let mut current = s.as_bytes().to_vec();
        let mut stable = false;
        for _ in 0..6 {
            let a = fixed(&current, selector);
            for ruby in rubies() {
                equal_runtime(&ruby, &current, &a);
            }
            if a == current {
                stable = true;
                break;
            }
            current = a;
        }
        assert!(stable, "{selector}: {}", String::from_utf8_lossy(&current));
        assert!(String::from_utf8_lossy(&current).contains("Hash["));
        assert!(!String::from_utf8_lossy(&current).contains("x = [a:"));
    }
}
#[test]
fn bounded_many_arrays_arguments_and_unsafe_siblings() {
    let source = "[a: 1, b: 2]\nHash[Hash[ary]]\n".repeat(6000);
    let a = fixed(source.as_bytes(), BOTH);
    assert_eq!(
        a,
        "[{a: 1, b: 2}]\nHash[Hash[ary]]\n".repeat(6000).as_bytes()
    );
    assert_eq!(fixed(&a, BOTH), a);
    let s = format!("Hash[{}]\n", vec!["Hash[ary]"; 6000].join(","));
    let d = project();
    let o = unchanged(
        d.path(),
        s.as_bytes(),
        &["--only", CONVERSION, "--fix", "a.rb"],
        1,
    );
    assert_eq!(
        String::from_utf8_lossy(&o.stdout)
            .matches(&format!("({CONVERSION})"))
            .count(),
        1
    );
}
