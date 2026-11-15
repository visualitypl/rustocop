//! controlled fixtures only. Command literals are parsed, NEVER run.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const COP: &str = "Style/RedundantLineContinuation";
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(dir: &Path, args: &[&str], status: i32) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    if status != 2 {
        assert!(out.stderr.is_empty(), "{out:?}");
    }
    out
}
fn unchanged(dir: &Path, source: &[u8], args: &[&str], status: i32) -> Output {
    let path = dir.join("a.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let out = cli(dir, args, status);
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    out
}
fn fix(dir: &Path, source: &[u8], status: i32) -> Vec<u8> {
    fs::write(dir.join("a.rb"), source).unwrap();
    cli(dir, &["--only", COP, "--fix", "a.rb"], status);
    fs::read(dir.join("a.rb")).unwrap()
}
#[test]
fn default_registration_selected_canonical_fix_and_stdin_no_write() {
    let registry = rustocop::cops::Registry::new();
    let cop = registry.cops().iter().find(|c| c.name() == COP).unwrap();
    assert!(cop.default_enabled());
    assert!(cop.safe_fixes());
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none());
    let d = project();
    let source = b"[1, \\\n 2]\n";
    for args in [vec!["a.rb"], vec!["--only", COP, "a.rb"]] {
        let out = unchanged(d.path(), source, &args, 1);
        assert!(String::from_utf8_lossy(&out.stdout).contains(
            "a.rb:1:5: C: Redundant line continuation. (Style/RedundantLineContinuation)"
        ));
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", COP, "--stdin", "a.rb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    assert_eq!(child.wait_with_output().unwrap().status.code(), Some(1));
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), source);
    let fixed = fix(d.path(), source, 0);
    assert_eq!(fixed, b"[1, \n 2]\n");
    unchanged(d.path(), &fixed, &["--only", COP, "--fix", "a.rb"], 0);
}
#[test]
fn selectors_config_enabling_excludes_and_strict_fixed_options() {
    let d = project();
    let source = b"[1, \\\n 2]\n";
    unchanged(
        d.path(),
        source,
        &[
            "--only",
            COP,
            "--except",
            COP,
            "--fix",
            "--fix-passes",
            "1",
            "a.rb",
        ],
        0,
    );
    for config in [
        "[cops.Style]\nenabled = false\n".to_string(),
        format!("[cops.\"{COP}\"]\nenabled = false\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(d.path(), source, &["--fix", "--fix-passes", "1", "a.rb"], 0);
        assert_ne!(fix(d.path(), source, 0), source);
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
    )
    .unwrap();
    fs::write(d.path().join("a.rb"), source).unwrap();
    // Layout sees newly exposed trailing whitespace only on a later pass.
    cli(d.path(), &["--fix", "--fix-passes", "1", "a.rb"], 1);
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), b"[1, \n 2]\n");
    for config in [
        "exclude = [\"a.rb\"]\n".to_string(),
        format!("[cops.\"{COP}\"]\nexclude = [\"a.rb\"]\n"),
    ] {
        fs::write(d.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            d.path(),
            source,
            &["--only", COP, "--fix", "--fix-passes", "1", "a.rb"],
            0,
        );
    }
    fs::write(
        d.path().join("rustocop.toml"),
        format!("[cops.\"{COP}\"]\nEnforcedStyle = \"never\"\n"),
    )
    .unwrap();
    let path = d.path().join("a.rb");
    fs::write(&path, source).unwrap();
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(d.path())
        .args(["--only", COP, "--fix", "--fix-passes", "1", "a.rb"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), mtime);
}
#[test]
fn both_directive_prefixes_ranges_and_clean_error_no_write() {
    let d = project();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "RedundantLineContinuation", "Style", "all"] {
            for source in [
                format!("# {prefix}:disable {target}\n[1, \\\n 2]\n"),
                format!("# {prefix}:disable-next {target}\n[1, \\\n 2]\n"),
            ] {
                unchanged(
                    d.path(),
                    source.as_bytes(),
                    &["--only", COP, "--fix", "a.rb"],
                    0,
                );
            }
        }
        let source = format!(
            "# {prefix}:disable {COP}\n[1, \\\n 2]\n# {prefix}:enable {COP}\n[3, \\\n 4]\n"
        );
        let expected = source.replacen("[3, \\", "[3, ", 1);
        assert_eq!(fix(d.path(), source.as_bytes(), 0), expected.as_bytes());
        // The slash's own physical line, not the following line, is reported.
        let source = format!("[1, \\\n 2] # {prefix}:disable {COP}\n");
        assert_eq!(
            fix(d.path(), source.as_bytes(), 0),
            source.replacen('\\', "", 1).as_bytes()
        );
    }
    for (source, status) in [
        ("foo \\\n + 2\n", 0),
        ("foo +\\\n bar\n", 0),
        ("[1, \\\n", 2),
        ("foo \\", 2),
        ("'literal \\\n text'\n", 0),
    ] {
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            status,
        );
    }
}
#[test]
fn bom_crlf_legacy_encoding_frozen_line_data_and_command_contents() {
    let d = project();
    for source in [
        b"\xef\xbb\xbf[1, \\\r\n 2]\r\np __LINE__\r\n__END__\r\ndata \\\r\n".as_slice(),
        b"# encoding: ISO-8859-1\n['\xe9', \\\n '\xe8']\n",
        b"# encoding: Windows-31J\n['\x83\\', \\\n '\x83\\']\n",
        b"# frozen_string_literal: true\n['a' \\\n 'b', \\\n 'c']\n",
        b"# frozen_string_literal: false\n['a', \\\n 'b']\n",
        b"# frozen_string_literal: chilled\n['a', \\\n 'b']\n",
        b"[`never \\\n executed`, \\\n %x(also never executed)]\n",
        b"[<<~TXT, \\\nbody \\\ntext\nTXT\n 'value']\n",
    ] {
        let fixed = fix(d.path(), source, 0);
        assert_eq!(
            fixed.iter().filter(|&&b| b == b'\n').count(),
            source.iter().filter(|&&b| b == b'\n').count()
        );
        assert_eq!(fixed.len() + 1, source.len(), "{source:?}");
        let mut expected = source.to_vec();
        // Each fixture has exactly one redundant code escape following a comma.
        let comma = source
            .windows(4)
            .position(|w| w.starts_with(b", \\"))
            .unwrap();
        expected.remove(comma + 2);
        assert_eq!(fixed, expected);
    }
}
// Parse-only independent Prism 3.4 comparison: all node fields/flags, scopes,
// decoded bytes and encodings. No command literals are evaluated.
const PRISM_SHAPE: &str = r#"
require 'prism'
def shape(x)
  case x
  when Prism::Node
    fields = x.deconstruct_keys(nil).reject { |k,v| k == :node_id || k == :location || k.to_s.end_with?('_loc') }
    [x.type, x.send(:flags), fields.map { |k,v| [k, shape(v)] }]
  when String then [x.bytes, x.encoding.name]
  when Array then x.map { |v| shape(v) }
  else x
  end
end
parsed = Prism.parse(File.binread(ARGV[0]), version: '3.4')
abort parsed.errors.map(&:message).join('\n') unless parsed.success?
p shape(parsed.value)
"#;
fn shape(path: &Path) -> Option<Vec<u8>> {
    let out = Command::new("ruby")
        .args(["-e", PRISM_SHAPE])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success()
        && String::from_utf8_lossy(&out.stderr).contains("cannot load such file -- prism")
    {
        return None;
    }
    assert!(out.status.success(), "{out:?}");
    Some(out.stdout)
}
#[test]
fn independent_fixed_34_ast_binding_payload_encoding_flags_and_scope_equality() {
    let d = project();
    let path = d.path().join("a.rb");
    for source in [
        b"# frozen_string_literal: true\nfoo('a' \\\n 'b', \\\n %W[x #{mark(1)}], {key: \\\n 1})\nfoo +\\\n bar\n".as_slice(),
        b"\xef\xbb\xbfclass A \\\r\n def foo(...); target(\\\r\n...); end\r\nend\r\n",
        b"# encoding: Windows-31J\n['\x83\\', \\\n '\x83\\']\n",
        b"[<<~TXT, \\\nbody #{foo(1, \\\n 2)} \\\ntext\nTXT\n %x(never executed)]\n",
        b"def foo\n 'a' \\\n 'b' \\\nend\nfoo { |x| [x, \\\n 1] }\n",
        b"case x; in [a,b]\\\n then a+b; end\nx ? y\\\n : z\na,b\\\n = [1,2]\n",
        b"foo { |a, b \\\n| [a,b] }\nfoo { |a; b \\\n| a }\n->(a,b \\\n) { a }\ndefined?(foo \\\n)\ndef foo; super(1,2 \\\n); end\nobj[1,2 \\\n] ||= value\nobj[1,2 \\\n] &&= value\nobj[1,2 \\\n] += value\ncase x; in [a,b \\\n]; end\ncase x; in [*,a,* \\\n]; end\ncase x; in {a:, b: \\\n}; end\ncase x; in Point(a,b \\\n); end\na,(b,c \\\n) = values\n",
        b"if foo \\\n; bar; end\nunless foo \\\n\n bar; end\nwhile foo \\\n\n bar; end\nuntil foo \\\n; bar; end\nfor x in values \\\n; bar; end\ncase x; when 1,2 \\\n; bar; end\ncase x; in [a,b] \\\n\n a; end\nif foo \\\r\n # comment\r\n bar; end\r\n",
        // These MLHS newline rules are fixed-3.4 parse evidence, not a claim
        // that the Ruby 3.2 host accepts every corrected spelling.
        b"(x,(y,z)) \\\n= a,b\n*x \\\n= values\nx,(y,z) \\\n= a,b\n(x,y),z \\\n= a,b\n(x,y \\\n) = a,b\n(x, \\\n) = values\n",
        b"(x,(y,z)) \\\r\n= a,b\r\n*x \\\r\n= values\r\nx,(y,z) \\\r\n= a,b\r\n(x,y),z \\\r\n= a,b\r\n(x,y \\\r\n) = a,b\r\n(x, \\\r\n) = values\r\n",
    ] {
        fs::write(&path, source).unwrap();
        let Some(before) = shape(&path) else { eprintln!("SKIP: optional Ruby/Prism 3.4 unavailable"); return; };
        assert_ne!(fix(d.path(), source, 0), source);
        assert_eq!(Some(before), shape(&path), "{source:?}");
    }
}
fn runtime(path: &Path) -> Option<Vec<u8>> {
    let out = Command::new("ruby").arg(path).output().ok()?;
    assert!(out.status.success(), "{out:?}");
    Some(out.stdout)
}
#[test]
fn controlled_runtime_preserves_binding_evaluation_order_literal_values_lines_and_control_flow() {
    let d = project();
    let path = d.path().join("a.rb");
    let source = br#"# frozen_string_literal: true
$events = []
def mark(x); $events << x; x; end
def foo(x); [:argument, x]; end
def bar; 3; end
argument = foo +\
 bar
values = [mark(1), \
 mark(2), {key: \
 mark(3)}]
text = 'a' \
 'b'
heredoc = <<~TXT

body #{mark(4)}
TXT
result = values. \
 map { |x| x }
result << text
result << [heredoc, __LINE__, text.encoding.name, text.frozen?, argument]
begin
  result << mark(5) \
    if true
  raise 'expected' \
    if false
rescue
  result << :unexpected
end
p [result, $events]
"#;
    fs::write(&path, source).unwrap();
    let Some(before) = runtime(&path) else {
        eprintln!("SKIP: Ruby runtime unavailable");
        return;
    };
    let fixed = fix(d.path(), source, 0);
    assert_ne!(fixed, source);
    assert!(fixed.windows(b"foo +\\\n".len()).any(|w| w == b"foo +\\\n"));
    assert_eq!(Some(before), runtime(&path));
}
#[test]
fn review_delimiters_and_header_separators_preserve_controlled_runtime() {
    let d = project();
    let path = d.path().join("a.rb");
    let source = br#"events = []
class ReviewParent
  def value(a, b); [a, b]; end
end
class ReviewChild < ReviewParent
  def value(a, b); super(a, b \
); end
end
events << ReviewChild.new.value(1, 2)
events << [3, 4].then { |a, b \
| [a, b] }
f = ->(a, b \
) { [a, b] }
events << [defined?(f \
), f.call(5, 6)]
values = [7, nil]
values[1 \
] ||= 8
values[0 \
] &&= 9
values[1 \
] += 2
events << values
case [11, 12]; in [a, b \
]; events << [a, b]; end
case {a: 13, b: 14}; in {a:, b: \
}; events << [a, b]; end
if true \
; events << :if; end
unless false \

 events << :unless
end
n = 0
while n < 2 \

 events << n
 n += 1
end
until n == 4 \
; events << n; n += 1; end
for x in [15, 16] \
; events << x; end
case n; when 4 \
; events << :when; end
case [17, 18]; in [a, b] \

 events << [a, b]; end
p [events, __LINE__]
"#;
    fs::write(&path, source).unwrap();
    let before = runtime(&path);
    assert_eq!(
        fix(d.path(), source, 0),
        source
            .iter()
            .copied()
            .filter(|&b| b != b'\\')
            .collect::<Vec<_>>()
    );
    if let Some(before) = before {
        assert_eq!(Some(before), runtime(&path));
    }
    unchanged(
        d.path(),
        &fs::read(&path).unwrap(),
        &["--only", COP, "--fix", "a.rb"],
        0,
    );
    for source in [
        b"while foo \\\ndo bar; end\n".as_slice(),
        b"until foo \\\ndo bar; end\n",
        b"for x in values \\\ndo bar; end\n",
        b"defined?(foo \\\n + bar)\n",
    ] {
        unchanged(d.path(), source, &["--only", COP, "--fix", "a.rb"], 0);
    }
}
#[test]
fn required_mlhs_escape_never_aborts_same_file_or_sibling_corrections() {
    let d = project();
    let path = d.path().join("a.rb");
    for nl in ["\n", "\r\n"] {
        for target in [
            "(x, (y, z))",
            "(x, y)",
            "((x, y))",
            "(x,)",
            "(*x)",
            "*x",
            "x",
            "(obj.x, ary[0])",
        ] {
            let source = format!("{target} \\{nl}= a, b{nl}");
            unchanged(
                d.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                0,
            );
            let before = shape(&path);
            let safe = format!("safe = [1, \\{nl} 2]{nl}");
            let mixed = format!("{source}{safe}");
            let expected = format!("{source}{}", safe.replace('\\', ""));
            assert_eq!(fix(d.path(), mixed.as_bytes(), 0), expected.as_bytes());
            unchanged(
                d.path(),
                expected.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                0,
            );
            fs::write(&path, &source).unwrap();
            let mtime = fs::metadata(&path).unwrap().modified().unwrap();
            fs::write(d.path().join("sibling.rb"), &safe).unwrap();
            cli(d.path(), &["--only", COP, "--fix", "a.rb", "sibling.rb"], 0);
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
            assert_eq!(
                fs::read(d.path().join("sibling.rb")).unwrap(),
                safe.replace('\\', "").as_bytes()
            );
            if let Some(before) = before {
                assert_eq!(Some(before), shape(&path));
            }
        }
    }
}
#[test]
fn completed_headers_preserve_runtime_errors_bindings_and_fixed_34_shape() {
    let d = project();
    let path = d.path().join("a.rb");
    // Only this controlled, ordinary Ruby is executed; no command literals.
    let template = br#"# frozen_string_literal: true
$events = []
def mark(x); $events << x; x; end
obj = Object.new
class << obj \
; def label; :singleton; end; end
class << obj \
# separator
 def other; :other; end
end
$events << [obj.label, obj.other]
case mark(1) \
when 1 then $events << :when; end
case mark([2, 3]) \
;in [a, b]; $events << [a, b]; end
case mark({a: 4}) \

in {a:}; $events << a; end
begin; raise ArgumentError, "first"; rescue ArgumentError \
then $events << :rescued; end
begin; raise TypeError, "second"; rescue TypeError => e \
# separator
 $events << [e.class.name, e.message]; end
begin; missing_local; rescue NameError => e \
; $events << [e.class.name, e.name]; end
(x, (y, z)) \
= mark([5, [6, 7]])
$events << [x, y, z]
x, (y, z), \
= mark([8, [9, 10]])
$events << [x, y, z]
*x \
= mark([11, 12])
$events << x
case mark(1) \
 + 1; when 2; $events << :operator; end
$events << ["caf\u00e9".bytes, "caf\u00e9".encoding.name, "caf\u00e9".frozen?, __LINE__]
p $events
"#;
    for crlf in [false, true] {
        let source = if crlf {
            template
                .iter()
                .flat_map(|&b| {
                    if b == b'\n' {
                        vec![b'\r', b'\n']
                    } else {
                        vec![b]
                    }
                })
                .collect()
        } else {
            template.to_vec()
        };
        fs::write(&path, &source).unwrap();
        let before_shape = shape(&path);
        let before_runtime = runtime(&path);
        let out = unchanged(d.path(), &source, &["--only", COP, "a.rb"], 1);
        assert_eq!(
            String::from_utf8_lossy(&out.stdout)
                .matches("Redundant line continuation.")
                .count(),
            9
        );
        let fixed = fix(d.path(), &source, 0);
        let expected = String::from_utf8(source.clone())
            .unwrap()
            .replace("obj \\", "obj ")
            .replace("mark(1) \\\nwhen", "mark(1) \nwhen")
            .replace("mark(1) \\\r\nwhen", "mark(1) \r\nwhen")
            .replace("mark([2, 3]) \\", "mark([2, 3]) ")
            .replace("mark({a: 4}) \\", "mark({a: 4}) ")
            .replace("ArgumentError \\", "ArgumentError ")
            .replace("=> e \\", "=> e ")
            .replace("x, (y, z), \\", "x, (y, z), ");
        assert_eq!(fixed, expected.as_bytes());
        if let Some(before) = before_shape {
            assert_eq!(Some(before), shape(&path));
        }
        if let Some(before) = before_runtime {
            assert_eq!(Some(before), runtime(&path));
        }
        unchanged(d.path(), &fixed, &["--only", COP, "--fix", "a.rb"], 0);
    }
}
#[test]
fn review_continuation_trivia_exact_diagnostics_and_single_pass_cli_fixes() {
    let d = project();
    let path = d.path().join("a.rb");
    for nl in ["\n", "\r\n"] {
        for (source, positions) in [
            ("case x \\\n\\\nwhen 1;foo;end\n", vec![(1, 8), (2, 1)]),
            ("class << obj \\\n\\\n;foo;end\n", vec![(1, 14), (2, 1)]),
            (
                "begin;foo;rescue E \\\n\\\nthen bar;end\n",
                vec![(1, 20), (2, 1)],
            ),
            (
                "case x \\\n=begin\ncomment\n=end\nwhen 1;foo;end\n",
                vec![(1, 8)],
            ),
            (
                "class << obj \\\n=begin\ncomment\n=end\n;foo;end\n",
                vec![(1, 14)],
            ),
            (
                "begin;foo;rescue E => e \\\n=begin\ncomment\n=end\n;bar;end\n",
                vec![(1, 25)],
            ),
        ] {
            let source = source.replace('\n', nl);
            let out = unchanged(d.path(), source.as_bytes(), &["--only", COP, "a.rb"], 1);
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert_eq!(
                stdout.matches("Redundant line continuation.").count(),
                positions.len()
            );
            for (line, col) in positions {
                assert!(
                    stdout.contains(&format!(
                        "a.rb:{line}:{col}: C: Redundant line continuation. ({COP})"
                    )),
                    "{stdout}"
                );
            }
            let before_shape = shape(&path);
            let fixed = fix(d.path(), source.as_bytes(), 0);
            assert_eq!(fixed, source.replace('\\', "").as_bytes());
            if let Some(before) = before_shape {
                assert_eq!(Some(before), shape(&path));
            }
            unchanged(d.path(), &fixed, &["--only", COP, "--fix", "a.rb"], 0);
        }
    }
}
#[test]
fn review_continuation_trivia_preserves_controlled_header_runtime() {
    let d = project();
    let path = d.path().join("a.rb");
    // Controlled ordinary Ruby only: preserve effects, exception bindings and
    // physical __LINE__ positions across both kinds of newly proved trivia.
    let source = "# frozen_string_literal: true\n$events = []\ndef mark(x); $events << x; x; end\nobj = Object.new\nclass << obj \\\n\\\n;def label; :singleton; end;end\n$events << obj.label\ncase mark(1) \\\n\\\nwhen 1;$events << :when;end\ncase mark([2]) \\\n\\\n=begin\ncomment \\\n=end\n;in [a];$events << a;end\nbegin;raise ArgumentError, 'message';rescue ArgumentError => e \\\n\\\n=begin\ncomment\n=end\n\\\n; $events << [e.class.name, e.message];end\n$events << ['value'.frozen?, 'value'.encoding.name, __LINE__]\np $events\n";
    for nl in ["\n", "\r\n"] {
        let source = source.replace('\n', nl);
        fs::write(&path, &source).unwrap();
        let before_runtime = runtime(&path);
        let before_shape = shape(&path);
        let fixed = fix(d.path(), source.as_bytes(), 0);
        // Keep the slash inside the embedded comment untouched.
        let expected = source
            .replace('\\', "")
            .replace(&format!("comment {nl}"), &format!("comment \\{nl}"));
        assert_eq!(fixed, expected.as_bytes());
        if let Some(before) = before_runtime {
            assert_eq!(Some(before), runtime(&path));
        }
        if let Some(before) = before_shape {
            assert_eq!(Some(before), shape(&path));
        }
        unchanged(d.path(), &fixed, &["--only", COP, "--fix", "a.rb"], 0);
    }
}
#[test]
fn necessary_heredoc_header_continuation_preserves_runtime_and_allows_safe_sibling_fix() {
    let d = project();
    let path = d.path().join("a.rb");
    for space in ["", " "] {
        let source = format!("x = <<~TXT{space}\\\n# body\n.foo\nTXT\n + 'tail'\np x\n");
        fs::write(&path, &source).unwrap();
        let before = runtime(&path);
        unchanged(
            d.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
        if let Some(before) = before {
            assert_eq!(Some(before), runtime(&path));
        }
        fs::write(d.path().join("sibling.rb"), b"[1, \\\n 2]\n").unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        cli(d.path(), &["--only", COP, "--fix", "a.rb", "sibling.rb"], 0);
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        assert_eq!(
            fs::read(d.path().join("sibling.rb")).unwrap(),
            b"[1, \n 2]\n"
        );
    }
}

#[test]
fn selected_peer_and_actual_default_composition_stabilizes_within_six_explicit_passes() {
    let d = project();
    let path = d.path().join("a.rb");
    let source = b"values = [ %q(a), \\\n %Q(b), \\\n 'c' ]; puts values.join\n";
    let peers = format!(
        "{COP},Style/BarePercentLiterals,Style/PercentLiteralDelimiters,Style/PercentQLiterals,Style/RedundantPercentQ,Style/RedundantCapitalW,Style/CharacterLiteral,Style/QuotedSymbols,Style/SymbolLiteral,Style/VariableInterpolation,Style/EmptyStringInsideInterpolation,Style/RedundantInterpolation,Style/RedundantInterpolationUnfreeze,Style/RedundantStringEscape,Style/RedundantHeredocDelimiterQuotes,Style/EmptyHeredoc,Style/CommandLiteral,Style/FormatString,Style/FormatStringToken,Style/RedundantFormat,Style/StringConcatenation,Style/LineEndConcatenation,Style/StringLiterals,Style/StringLiteralsInInterpolation,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout"
    );
    for selected in [Some(COP), Some(peers.as_str()), None] {
        fs::write(&path, source).unwrap();
        let before = runtime(&path);
        let mut seen = vec![source.to_vec()];
        let mut stable = false;
        for _ in 0..6 {
            let old = fs::read(&path).unwrap();
            let mut args = vec!["--fix", "a.rb"];
            if let Some(selected) = selected {
                args.extend(["--only", selected]);
            }
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(d.path())
                .args(&args)
                .output()
                .unwrap();
            assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
            assert!(out.stderr.is_empty(), "{out:?}");
            let new = fs::read(&path).unwrap();
            if let Some(before) = &before {
                assert_eq!(Some(before.clone()), runtime(&path));
            }
            if old == new {
                stable = true;
                break;
            }
            assert!(!seen.contains(&new), "correction cycle: {selected:?}");
            seen.push(new);
        }
        assert!(stable, "{selected:?}");
        assert!(!fs::read(&path).unwrap().contains(&b'\\'));
    }
}
