//! real CLI policy, pinned corrections and controlled Ruby witnesses.
//! RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
const SELF: &str = "Style/SelfAssignment";
const SWAP: &str = "Style/SwapValues";
const BOTH: &str = "Style/SelfAssignment,Style/SwapValues";
const CASES: [(&str, &str); 2] = [(SELF, "x = x + 1\n"), (SWAP, "tmp=x\nx=y\ny=tmp\n")];
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), "").unwrap();
    d
}
fn cli(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn unchanged(root: &Path, cop: &str, source: &[u8], args: &[&str], count: usize) {
    let path = root.join("a.rb");
    fs::write(&path, source).unwrap();
    let time = fs::metadata(&path).unwrap().modified().unwrap();
    let o = cli(root, args);
    assert_ne!(o.status.code(), Some(2), "{o:?}");
    assert_eq!(
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| l.contains(cop))
            .count(),
        count,
        "{o:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), source, "{o:?}");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
}
fn fix(root: &Path, selected: &str, before: &str, after: &str) {
    fs::write(root.join("a.rb"), before).unwrap();
    let o = cli(root, &["--only", selected, "--fix", "a.rb"]);
    assert_ne!(o.status.code(), Some(2), "{o:?}");
    assert_eq!(
        fs::read_to_string(root.join("a.rb")).unwrap(),
        after,
        "{o:?}"
    );
}
#[test]
fn default_selectors_and_config_are_real_and_never_implicitly_write() {
    let d = project();
    let registry = rustocop::cops::Registry::new();
    for (cop, source) in CASES {
        let rule = registry.cops().iter().find(|r| r.name() == cop).unwrap();
        assert!(rule.default_enabled());
        assert_eq!(rule.safe_fixes(), cop == SELF);
        assert_eq!(rule.ruby_versions(), Default::default());
        assert!(rule.path_policy().is_none() && rule.max_setting().is_none());
        for (config, selectors, count) in [
            (String::new(), vec![], 1),
            (String::new(), vec!["--only", cop], 1),
            (String::new(), vec!["--only", "Style", "--except", cop], 0),
            (String::new(), vec!["--only", cop, "--except", cop], 0),
            (format!("[cops.\"{cop}\"]\nenabled=false\n"), vec![], 0),
            (
                format!("[cops.\"{cop}\"]\nenabled=false\n"),
                vec!["--only", cop],
                1,
            ),
            (
                format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
                vec![],
                1,
            ),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            let mut args = selectors;
            args.push("a.rb");
            unchanged(d.path(), cop, source.as_bytes(), &args, count);
        }
    }
}
#[test]
fn excludes_and_both_directive_prefixes_protect_files() {
    let d = project();
    for (cop, source) in CASES {
        for config in [
            "exclude=['a.rb']\n".to_owned(),
            format!("[cops.\"{cop}\"]\nexclude=['a.rb']\n"),
        ] {
            fs::write(d.path().join("rustocop.toml"), config).unwrap();
            unchanged(
                d.path(),
                cop,
                source.as_bytes(),
                &["--only", cop, "--fix", "a.rb"],
                0,
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                let before = format!("# {prefix}:disable {target}\n{source}");
                unchanged(
                    d.path(),
                    cop,
                    before.as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    0,
                );
            }
            let before =
                format!("# {prefix}:disable {cop}\n{source}# {prefix}:enable {cop}\n{source}");
            let after = if cop == SELF {
                format!("# {prefix}:disable {cop}\n{source}# {prefix}:enable {cop}\nx += 1\n")
            } else {
                before.clone()
            };
            fix(d.path(), cop, &before, &after);
        }
    }
}
#[test]
fn every_pinned_correction_is_canonical_or_an_individual_no_write_hold() {
    let d = project();
    let (mut canonical, mut boolean_holds, mut unsafe_holds) = (0, 0, 0);
    for (cop, fixture) in [
        (SELF, include_str!("fixtures/self_assignment_pinned.json")),
        (SWAP, include_str!("fixtures/swap_values_pinned.json")),
    ] {
        let cases: Vec<serde_json::Value> = serde_json::from_str(fixture).unwrap();
        for c in cases {
            let Some(after) = c["correction"].as_str() else {
                continue;
            };
            let before = c["source"].as_str().unwrap();
            if cop == SWAP
                || before.starts_with('@') && (before.contains("||") || before.contains("&&"))
            {
                unchanged(
                    d.path(),
                    cop,
                    before.as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    1,
                );
                if cop == SWAP {
                    unsafe_holds += 1;
                } else {
                    boolean_holds += 1;
                }
            } else {
                fix(d.path(), cop, before, after);
                unchanged(
                    d.path(),
                    cop,
                    after.as_bytes(),
                    &["--only", cop, "--fix", "a.rb"],
                    0,
                );
                canonical += 1;
            }
        }
    }
    assert_eq!((canonical, boolean_holds, unsafe_holds), (76, 6, 8));
}
#[test]
fn bom_crlf_encoding_line_values_and_data_payloads_are_preserved() {
    let d = project();
    let before = "\u{feff}# coding: UTF-8\r\nx = x + 'é'\r\np __LINE__\r\n__END__\r\npayload é\r\n";
    let after = before.replace("x = x +", "x +=");
    fix(d.path(), SELF, before, &after);
    fix(
        d.path(),
        SELF,
        "x = x + __LINE__\np __LINE__\n",
        "x += __LINE__\np __LINE__\n",
    );
    for source in [
        b"# coding: ISO-8859-1\nx = x + '\xe9'\n".as_slice(),
        b"x = (x +\n __LINE__)\np __LINE__\n".as_slice(),
        b"x = x + DATA.pos\n__END__\npayload\n".as_slice(),
        b"x = x + 1 # rustocop:disable Lint/Debugger\n".as_slice(),
        b"x = (x + # comment\n 1)\n".as_slice(),
    ] {
        unchanged(
            d.path(),
            SELF,
            source,
            &["--only", SELF, "--fix", "a.rb"],
            1,
        );
    }
    let before = "\u{feff}# coding: UTF-8\r\ntmp=x\r\nx=y\r\ny=tmp\r\n__END__\r\npayload\r\n";
    unchanged(
        d.path(),
        SWAP,
        before.as_bytes(),
        &["--only", SWAP, "--fix", "a.rb"],
        1,
    );
}
#[test]
fn non_utf8_swap_identifiers_are_compared_as_bytes_and_never_rewritten() {
    let d = project();
    for (tmp, x, y) in [
        ("tmp", "café", "thé"),
        ("tèmp", "café", "thé"),
        ("@tèmp", "@café", "@thé"),
        ("@@Tèmp", "@@Café", "@@Thé"),
        ("$tèmp", "$café", "$thé"),
        ("Tèmp", "Café", "Thé"),
        ("::Tèmp", "::Café", "::Thé"),
        ("Café::Tèmp", "Café::X", "Thé::Y"),
        ("::Café::Tèmp", "::Café::X", "::Thé::Y"),
    ] {
        let source = format!("# coding: ISO-8859-1\n{tmp} = {x}\n{x} = {y}\n{y} = {tmp}\n");
        let bytes: Vec<u8> = source
            .chars()
            .map(|c| u8::try_from(c as u32).unwrap())
            .collect();
        unchanged(
            d.path(),
            SWAP,
            &bytes,
            &["--only", SWAP, "--fix", "a.rb"],
            1,
        );
        let o = cli(d.path(), &["--only", SWAP, "a.rb"]);
        let output = String::from_utf8_lossy(&o.stdout);
        assert!(output.contains("a.rb:2:1:"), "{o:?}");
        let message =
            format!("Replace this and assignments at lines 3 and 4 with `{x}, {y} = {y}, {x}`.");
        let message_bytes: Vec<u8> = message
            .chars()
            .map(|c| u8::try_from(c as u32).unwrap())
            .collect();
        assert!(
            output.contains(String::from_utf8_lossy(&message_bytes).as_ref()),
            "{o:?}"
        );
    }
    // These different names have identical lossy UTF-8 renderings.
    unchanged(
        d.path(),
        SWAP,
        b"# coding: ISO-8859-1\ntmp = caf\xe9\ncaf\xe8 = th\xe9\nth\xe9 = tmp\n",
        &["--only", SWAP, "--fix", "a.rb"],
        0,
    );
}
#[test]
fn incompatible_layout_keeps_detection_and_safe_capability_but_withholds_edits() {
    let d = project();
    let selected = format!("{SELF},Layout/SpaceAroundOperators");
    unchanged(
        d.path(),
        SELF,
        b"x = x + 1\n",
        &["--only", &selected, "--fix", "a.rb"],
        1,
    );
    fix(d.path(), SELF, "x = x + 1\n", "x += 1\n");
    // The default path selects the incompatible layout peer as well.
    unchanged(d.path(), SELF, b"x = x + 1\n", &["--fix", "a.rb"], 1);
}
#[test]
fn stdin_policy_positions_and_error_clean_uncorrectable_no_write_contract() {
    let d = project();
    for (cop, source) in CASES {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(d.path())
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
        let o = child.wait_with_output().unwrap();
        assert_eq!(o.status.code(), Some(1), "{o:?}");
        let text = String::from_utf8_lossy(&o.stdout);
        assert!(text.contains("a.rb:1:1:") && text.contains(cop), "{text}");
        assert!(!d.path().join("a.rb").exists());
        for setting in [
            "EnforcedStyle='anything'",
            "max=80",
            "TargetRubyVersion=3.4",
        ] {
            fs::write(
                d.path().join("rustocop.toml"),
                format!("[cops.\"{cop}\"]\n{setting}\n"),
            )
            .unwrap();
            assert_eq!(
                cli(d.path(), &["--only", cop, "--stdin", "a.rb"])
                    .status
                    .code(),
                Some(2)
            );
        }
        fs::write(d.path().join("rustocop.toml"), "").unwrap();
    }
    for (cop, source, count) in [
        (SELF, "x += 1\n", 0),
        (SWAP, "x,y=y,x\n", 0),
        (SELF, "@x = @x || 1\n", 1),
        (SWAP, "tmp=x; x=y; y=tmp; consume(tmp)\n", 1),
    ] {
        unchanged(
            d.path(),
            cop,
            source.as_bytes(),
            &["--only", cop, "--fix", "a.rb"],
            count,
        );
    }
    fs::write(d.path().join("a.rb"), "x = (\n").unwrap();
    let time = fs::metadata(d.path().join("a.rb"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        cli(d.path(), &["--only", BOTH, "--fix", "a.rb"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(fs::read(d.path().join("a.rb")).unwrap(), b"x = (\n");
    assert_eq!(
        fs::metadata(d.path().join("a.rb"))
            .unwrap()
            .modified()
            .unwrap(),
        time
    );
}
#[test]
fn controlled_dispatch_truthiness_bindings_results_setters_and_flow_witnesses() {
    let source = r#"$trace = []
class Value
  attr_reader :n
  def initialize(n); @n = n; end
  def +(other)
    $trace << [:plus, @n, other]
    Value.new(@n + other)
  end
end
def rhs
  $trace << :rhs
  2
end
x = Value.new(3)
p((x = x.+(rhs)).n)
p x.n
x = 1
f = -> { x = x + 2 }
p [f.call, x]
p [5].map { |x| x = x * 2 }
p [nil].map { |; x| x = x || :local }
def choose(x)
  [x = x || ($trace << :default; :value), x = x && :yes]
end
p [nil, false, true].map { |x| choose(x) }
class FrozenInstance
  def initialize; @x = :set; freeze; end
  def read; @x = @x || :other; end
end
begin
  FrozenInstance.new.read
rescue FrozenError
  $trace << :frozen_instance
end
class UndefinedClassVariable
  def self.read; @@x = @@x || :other; end
end
begin
  UndefinedClassVariable.read
rescue NameError
  $trace << :undefined_class_variable
end
class Numbers
  @@x = Value.new(1)
  def self.add; @@x = @@x + rhs; end
end
p Numbers.add.n
class Setter
  def value; $trace << :get; @value || 1; end
  def value=(x); $trace << [:set,x]; @value=x; :different_return; end
end
obj = Setter.new
p(obj.value = obj.value + rhs)
a,b = 1,2
a,b = a+1,b+1
p [a,b]
tmp = a
a = b
b = tmp
p [a,b,tmp]
def flow(stop)
  x=1
  x = x + (return :early if stop; raise 'failure')
rescue RuntimeError
  :rescued
ensure
  $trace << :ensure
end
p [flow(true),flow(false)]
p $trace
"#;
    let d = project();
    fs::write(d.path().join("before.rb"), source).unwrap();
    fs::write(d.path().join("a.rb"), source).unwrap();
    let o = cli(d.path(), &["--only", BOTH, "--fix", "a.rb"]);
    assert_ne!(o.status.code(), Some(2), "{o:?}");
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert_ne!(after, source);
    assert!(
        after.contains("x += rhs") && after.contains("x *= 2") && after.contains("@@x += rhs"),
        "{after}"
    );
    assert!(
        after.contains("@x = @x ||")
            && after.contains("@@x = @@x ||")
            && after.contains("obj.value = obj.value + rhs")
            && after.contains("tmp = a\na = b\nb = tmp")
    );
    let mut rubies = vec!["ruby".to_owned()];
    if let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") {
        rubies.push(ruby);
    }
    for ruby in rubies {
        if Command::new(&ruby).arg("--version").output().is_err() {
            continue;
        }
        let before = Command::new(&ruby)
            .arg(d.path().join("before.rb"))
            .output()
            .unwrap();
        let after = Command::new(&ruby)
            .arg(d.path().join("a.rb"))
            .output()
            .unwrap();
        assert!(
            before.status.success() && after.status.success(),
            "{ruby}: {before:?} {after:?}"
        );
        assert_eq!(before.stdout, after.stdout, "{ruby}");
        assert_eq!(before.stderr, after.stderr, "{ruby}");
        let observed = String::from_utf8_lossy(&after.stdout);
        assert!(
            observed.contains("[:early, :rescued]")
                && observed.contains(":frozen_instance")
                && observed.contains(":undefined_class_variable")
                && observed.contains("[:plus, 3, 2]"),
            "{observed}"
        );
    }
}
#[test]
fn mixed_style_layout_edits_stabilize_without_reordering_or_removing_swaps() {
    let d = project();
    let before =
        "def f(x)\n  x = x + ({ :key => 'value' });\n  tmp=x\n  x=y\n  y=tmp\n  return x\nend\n";
    // Exclude the explicitly incompatible spacing peer when composing fixes.
    let selected = format!(
        "{BOTH},Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/Semicolon,Style/RedundantParentheses,Style/SignalException,Layout/EndAlignment,Layout/IndentationWidth"
    );
    fs::write(d.path().join("a.rb"), before).unwrap();
    let mut stable = false;
    for _ in 0..6 {
        let old = fs::read(d.path().join("a.rb")).unwrap();
        let o = cli(d.path(), &["--only", &selected, "--fix", "a.rb"]);
        assert_ne!(o.status.code(), Some(2), "{o:?}");
        if fs::read(d.path().join("a.rb")).unwrap() == old {
            stable = true;
            break;
        }
    }
    assert!(stable);
    let after = fs::read_to_string(d.path().join("a.rb")).unwrap();
    assert!(
        after.contains("x +=")
            && after.contains("key: \"value\"")
            && after.contains("tmp=x\n  x=y\n  y=tmp"),
        "{after}"
    );
}
