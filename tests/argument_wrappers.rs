//! public pipeline, source bytes and controlled Ruby runtime evidence.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const MERGE: &str = "Style/KeywordArgumentsMerging";
const BLOCK: &str = "Style/ExplicitBlockArgument";
const REDUNDANT: &str = "Style/RedundantArgument";
const VISIBILITY_REPRO: &str = "class Hash\n  private \"merge\"\nend\ndef take(**kwargs);kwargs;end\nbegin\n  p take(**{}.merge(a: 1))\nrescue NoMethodError\n  p :missing\nend\n";
const CASES: [(&str, &str, &str); 3] = [
    (MERGE, "foo(**{}.merge(y: 1))\n", "foo(**{}, y: 1)\n"),
    (
        BLOCK,
        "def f; if block_given?; [1].each { |x| yield x }; end; end\n",
        "def f(&block); if block_given?; [1].each(&block); end; end\n",
    ),
    (REDUNDANT, "foo.sum(0)\n", "foo.sum(0)\n"),
];
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
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
    out
}
#[test]
fn defaults_selectors_config_and_detection_only_never_write() {
    let dir = project();
    let path = dir.path().join("x.rb");
    for (cop, before, after) in CASES {
        fs::write(&path, before).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        assert!(String::from_utf8_lossy(&cli(dir.path(), &["x.rb"], 1).stdout).contains(cop));
        cli(dir.path(), &["--only", cop, "x.rb"], 1);
        cli(
            dir.path(),
            &["--only", cop, "--except", cop, "--fix", "x.rb"],
            0,
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        for config in [
            format!("[cops.\"{cop}\"]\nenabled = false\n"),
            "[cops.Style]\nenabled = false\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .arg("x.rb")
                .output()
                .unwrap();
            assert!(matches!(out.status.code(), Some(0 | 1)));
            assert!(!String::from_utf8_lossy(&out.stdout).contains(cop));
            let status = i32::from(before == after);
            cli(dir.path(), &["--only", cop, "--fix", "x.rb"], status);
            assert_eq!(fs::read_to_string(&path).unwrap(), after);
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            cli(dir.path(), &["--only", cop, "--fix", "x.rb"], status);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            fs::write(&path, before).unwrap();
        }
        fs::write(
            dir.path().join("rustocop.toml"),
            format!("[cops.Style]\nenabled=false\n[cops.\"{cop}\"]\nenabled=true\n"),
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&cli(dir.path(), &["x.rb"], 1).stdout).contains(cop));
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    }
}
#[test]
fn directives_excludes_errors_clean_and_withheld_keep_bytes_and_mtime() {
    let dir = project();
    let path = dir.path().join("x.rb");
    for (cop, before, _) in CASES {
        for prefix in ["rustocop", "rubocop"] {
            for target in [cop, "Style", "all"] {
                let source = format!("# {prefix}:disable {target}\n{before}");
                fs::write(&path, &source).unwrap();
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                cli(dir.path(), &["--only", cop, "--fix", "x.rb"], 0);
                assert_eq!(fs::read_to_string(&path).unwrap(), source);
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            }
        }
        for config in [
            "exclude=[\"x.rb\"]\n".into(),
            format!("[cops.\"{cop}\"]\nexclude=[\"x.rb\"]\n"),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            fs::write(&path, before).unwrap();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            cli(dir.path(), &["--only", cop, "--fix", "x.rb"], 0);
            assert_eq!(fs::read_to_string(&path).unwrap(), before);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
        fs::write(dir.path().join("rustocop.toml"), "").unwrap();
        for (source, status) in [("def f(\n", 2), ("value\n", 0)] {
            fs::write(&path, source).unwrap();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            cli(dir.path(), &["--only", cop, "--fix", "x.rb"], status);
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
    }
    for (cop, source) in [
        (MERGE, "foo(**options.merge(y: 1))\n"),
        (BLOCK, "def f; 3.times { yield }; end\n"),
        (REDUNDANT, "foo.sum(0)\n"),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), &["--only", cop, "--fix", "x.rb"], 1);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn stdin_is_diagnostic_only_and_fix_never_writes_its_filename_hint() {
    use std::{io::Write, process::Stdio};
    let dir = project();
    let path = dir.path().join("x.rb");
    for (cop, before, _) in CASES {
        fs::write(&path, before).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", cop, "--stdin", "x.rb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(before.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains(cop));
        cli(dir.path(), &["--only", cop, "--fix", "--stdin", "x.rb"], 2);
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn encoding_bom_crlf_payload_and_explicit_line_values_survive() {
    let dir = project();
    let path = dir.path().join("x.rb");
    for (cop, before, after) in CASES {
        for header in [
            b"\xef\xbb\xbf# coding: UTF-8\r\n".as_slice(),
            b"# coding: ISO-8859-1\r\n# caf\xe9\r\n",
        ] {
            let suffix = b"p __LINE__\r\n__END__\r\nraw\xff\r\n";
            let before = [header, before.replace('\n', "\r\n").as_bytes(), suffix].concat();
            let after = [header, after.replace('\n', "\r\n").as_bytes(), suffix].concat();
            fs::write(&path, &before).unwrap();
            cli(
                dir.path(),
                &["--only", cop, "--fix", "x.rb"],
                i32::from(cop == REDUNDANT),
            );
            assert_eq!(fs::read(&path).unwrap(), after);
        }
    }
}
#[test]
fn atomic_signature_envelope_respects_disabled_lines_and_keeps_safe_siblings() {
    let dir = project();
    let path = dir.path().join("x.rb");
    for prefix in ["rubocop", "rustocop"] {
        let before = format!(
            "def f # {prefix}:disable {BLOCK}\n  if block_given?; [1].each {{ |x| yield x }}; end\nend\nfoo(**{{}}.merge(y: 1))\n"
        );
        let after = before.replace("{}.merge(y: 1)", "{}, y: 1");
        fs::write(&path, &before).unwrap();
        cli(
            dir.path(),
            &["--only", &format!("{BLOCK},{MERGE}"), "--fix", "x.rb"],
            1,
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
    }
    let before = b"# coding: ISO-8859-1\r\ndef f\r\n # caf\xe9\r\n if block_given?; [1].each { |x| yield x }; end\r\nend\r\nfoo(**{}.merge(y: 1))\r\n";
    let after = b"# coding: ISO-8859-1\r\ndef f\r\n # caf\xe9\r\n if block_given?; [1].each { |x| yield x }; end\r\nend\r\nfoo(**{}, y: 1)\r\n";
    fs::write(&path, before).unwrap();
    cli(
        dir.path(),
        &["--only", &format!("{BLOCK},{MERGE}"), "--fix", "x.rb"],
        1,
    );
    assert_eq!(fs::read(&path).unwrap(), after);
}
fn ruby_available() -> bool {
    Command::new("ruby")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}
fn ruby(path: &Path) -> Output {
    Command::new("ruby").arg("-W0").arg(path).output().unwrap()
}
fn observed_fix(cop: &str, before: &str, after: &str, status: i32) -> Vec<u8> {
    observed_fix_with_driver(cop, before, after, status, "")
}
// Drivers are controlled Ruby observations, not part of the edited source.
// In particular forwarding an unknown driver Proc intentionally ends the shared
// whole-file source-observer proof if it is placed in the implementation file.
fn observed_fix_with_driver(
    cop: &str,
    before: &str,
    after: &str,
    status: i32,
    driver: &str,
) -> Vec<u8> {
    let dir = project();
    let path = dir.path().join("x.rb");
    fs::write(&path, before).unwrap();
    let runner = if driver.is_empty() {
        path.clone()
    } else {
        let runner = dir.path().join("driver.rb");
        fs::write(
            &runner,
            format!("load File.expand_path('x.rb', __dir__)\n{driver}"),
        )
        .unwrap();
        runner
    };
    let original = ruby(&runner);
    assert!(original.status.success(), "{original:?}\n{before}");
    cli(dir.path(), &["--only", cop, "--fix", "x.rb"], status);
    assert_eq!(fs::read_to_string(&path).unwrap(), after);
    let fixed = ruby(&runner);
    assert_eq!(original.status.code(), fixed.status.code());
    assert_eq!(original.stdout, fixed.stdout, "{before}");
    assert_eq!(original.stderr, fixed.stderr);
    original.stdout
}
#[test]
fn runtime_literal_merge_preserves_keyword_insertion_overwrites_and_positional_channel() {
    if !ruby_available() {
        eprintln!("Ruby unavailable; runtime probe skipped");
        return;
    }
    let before = "def take(*a, **k); p [a,k.to_a]; end\ntake(:pos, **{a: 1, b: 2}.merge(a: 3, c: 4))\ntake({}, **{}.merge({z: 1}, {a: 2}))\n";
    let after = before
        .replace("{a: 1, b: 2}.merge(a: 3, c: 4)", "{a: 1, b: 2}, a: 3, c: 4")
        .replace("{}.merge({z: 1}, {a: 2})", "{}, z: 1, a: 2");
    assert_eq!(
        observed_fix(MERGE, before, &after, 0),
        b"[[:pos], [[:a, 3], [:b, 2], [:c, 4]]]\n[[{}], [[:z, 1], [:a, 2]]]\n"
    );
}
#[test]
fn runtime_method_removal_preserves_missing_core_dispatch() {
    if !ruby_available() {
        eprintln!("Ruby unavailable; runtime probe skipped");
        return;
    }
    for removal in ["undef_method", "remove_method"] {
        for target in ["\"merge\"", ":merge", "('mer' + 'ge')"] {
            let before = format!(
                "class Hash; {removal} {target}; end\ndef take(**k); p k; end\nbegin; take(**{{}}.merge(a: 1)); rescue NoMethodError; puts 'missing merge'; end\n"
            );
            assert_eq!(observed_fix(MERGE, &before, &before, 1), b"missing merge\n");
            // A naive rewrite would silently bypass the removed method.
            let dir = project();
            let path = dir.path().join("naive.rb");
            fs::write(&path, before.replace("{}.merge(a: 1)", "{}, a: 1")).unwrap();
            let naive = ruby(&path);
            assert!(naive.status.success(), "{naive:?}");
            assert_eq!(naive.stdout, b"{:a=>1}\n");
        }
        let before = format!(
            "class Array; {removal} \"each\"; end\ndef f; if block_given?; [1].each {{ |x| yield x }}; end; end\nbegin; f {{ |x| p x }}; rescue NoMethodError; puts 'missing each'; end\n"
        );
        assert_eq!(observed_fix(BLOCK, &before, &before, 1), b"missing each\n");
    }
}
#[test]
fn attribute_and_undef_mutations_retain_offenses_bytes_and_mtime() {
    let dir = project();
    let path = dir.path().join("x.rb");
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    let mut prefixes = vec![
        "class Hash; undef :\"#{'mer'}ge\"; end".to_owned(),
        "class Array; undef :\"#{'ea'}ch\"; end".to_owned(),
    ];
    for operation in ["attr_reader", "attr_accessor", "attr", "attr_writer"] {
        for (class, target) in [("Hash", "merge"), ("Array", "each")] {
            for name in [format!("\"{target}\""), format!("\"#{{'{target}'}}\"")] {
                prefixes.push(format!("class {class}; {operation} {name}; end"));
            }
        }
    }
    for prefix in prefixes {
        for (cop, candidate, _) in &CASES[..2] {
            let source = format!("{prefix}\n{candidate}");
            fs::write(&path, &source).unwrap();
            fs::File::open(&path).unwrap().set_modified(old).unwrap();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            for args in [
                ["--only", cop, "x.rb"].as_slice(),
                &["--only", cop, "--fix", "x.rb"],
            ] {
                let out = cli(dir.path(), args, 1);
                assert!(
                    String::from_utf8_lossy(&out.stdout).contains(cop),
                    "{source}"
                );
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes(), "{source}");
                assert_eq!(
                    fs::metadata(&path).unwrap().modified().unwrap(),
                    modified,
                    "{source}"
                );
            }
        }
    }
}
#[test]
fn runtime_attribute_readers_preserve_merge_argument_errors_against_naive_rewrites() {
    if !ruby_available() {
        eprintln!("Ruby unavailable; runtime probe skipped");
        return;
    }
    let dir = project();
    let path = dir.path().join("naive.rb");
    for operation in ["attr_reader", "attr_accessor", "attr"] {
        for target in ["\"merge\"", "\"#{'mer'}ge\""] {
            let before = format!(
                "class Hash; {operation} {target}; end\ndef take(**k); p k; end\nbegin; take(**{{}}.merge(a: 1)); rescue ArgumentError; puts 'argument error'; end\n"
            );
            assert_eq!(
                observed_fix(MERGE, &before, &before, 1),
                b"argument error\n"
            );
            fs::write(&path, before.replace("{}.merge(a: 1)", "{}, a: 1")).unwrap();
            let naive = ruby(&path);
            assert!(naive.status.success(), "{naive:?}");
            assert!(naive.stderr.is_empty(), "{naive:?}");
            assert_eq!(naive.stdout, b"{:a=>1}\n");
        }
    }
}
#[test]
fn runtime_computed_undef_preserves_missing_merge_against_naive_rewrites() {
    if !ruby_available() {
        eprintln!("Ruby unavailable; runtime probe skipped");
        return;
    }
    let before = "class Hash; undef :\"#{'mer'}ge\"; end\ndef take(**k); p k; end\nbegin; take(**{}.merge(a: 1)); rescue NoMethodError; puts 'missing merge'; end\n";
    assert_eq!(observed_fix(MERGE, before, before, 1), b"missing merge\n");
    let dir = project();
    let path = dir.path().join("naive.rb");
    fs::write(&path, before.replace("{}.merge(a: 1)", "{}, a: 1")).unwrap();
    let naive = ruby(&path);
    assert!(naive.status.success(), "{naive:?}");
    assert!(naive.stderr.is_empty(), "{naive:?}");
    assert_eq!(naive.stdout, b"{:a=>1}\n");
    let before = "class Array; undef :\"#{'ea'}ch\"; end\ndef f; if block_given?; [1].each { |x| yield x }; end; end\nbegin; f { |x| p x }; rescue NoMethodError; puts 'missing each'; end\n";
    assert_eq!(observed_fix(BLOCK, before, before, 1), b"missing each\n");
}
#[test]
fn visibility_export_and_selector_calls_retain_offenses_bytes_and_mtime() {
    let dir = project();
    let path = dir.path().join("x.rb");
    // A deliberately old mtime makes an atomic rewrite observable even on a
    // filesystem whose timestamp resolution is coarser than the CLI invocation.
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    let mut cases = vec![(MERGE, VISIBILITY_REPRO.to_owned())];
    for operation in [
        "private",
        "protected",
        "public",
        "private_class_method",
        "public_class_method",
        "module_function",
        "ruby2_keywords",
    ] {
        for (cop, candidate, _) in &CASES[..2] {
            for prefix in [
                format!("class Hash; {operation} \"merge\"; end"),
                format!("class Array; {operation} \"\\x65ach\"; end"),
                format!("Hash.{operation}(target)"),
                format!("Array&.{operation} *targets"),
                format!(
                    "targets.map(&:\"\\x{:02x}{}\")",
                    operation.as_bytes()[0],
                    &operation[1..]
                ),
                format!("targets.map(&\"{operation}\".to_sym)"),
                "targets.map(&selector)".into(),
            ] {
                cases.push((cop, format!("{prefix}\n{candidate}")));
            }
        }
    }
    for (cop, source) in cases {
        fs::write(&path, &source).unwrap();
        fs::File::open(&path).unwrap().set_modified(old).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        for args in [
            ["--only", cop, "x.rb"].as_slice(),
            &["--only", cop, "--fix", "x.rb"],
        ] {
            let out = cli(dir.path(), args, 1);
            assert!(
                String::from_utf8_lossy(&out.stdout).contains(cop),
                "{source}"
            );
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes(), "{source}");
            assert_eq!(
                fs::metadata(&path).unwrap().modified().unwrap(),
                modified,
                "{source}"
            );
        }
    }
    // Ordinary canonical literal merging and guarded forwarding still write
    // their exact expected bytes and stabilize on the next explicit pass.
    for (cop, before, after) in &CASES[..2] {
        fs::write(&path, before).unwrap();
        fs::File::open(&path).unwrap().set_modified(old).unwrap();
        cli(dir.path(), &["--only", cop, "--fix", "x.rb"], 0);
        assert_eq!(fs::read(&path).unwrap(), after.as_bytes());
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        assert_ne!(modified, old);
        cli(dir.path(), &["--only", cop, "--fix", "x.rb"], 0);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn runtime_visibility_changes_preserve_core_dispatch_exceptions_against_naive_rewrites() {
    if !ruby_available() {
        eprintln!("Ruby unavailable; runtime probe skipped");
        return;
    }
    let mut cases = vec![VISIBILITY_REPRO.to_owned()];
    for visibility in ["private", "protected"] {
        for target in [
            ":merge",
            "\"\\x6derge\"",
            "('mer' + 'ge')",
            "\"#{'mer'}ge\"",
        ] {
            for receiver in ["", "self."] {
                cases.push(format!(
                    "class Hash; {receiver}{visibility} {target}; end\ndef take(**kwargs);kwargs;end\nbegin\n  p take(**{{}}.merge(a: 1))\nrescue NoMethodError\n  p :missing\nend\n"
                ));
            }
        }
    }
    let dir = project();
    let path = dir.path().join("naive.rb");
    for before in cases {
        assert_eq!(observed_fix(MERGE, &before, &before, 1), b":missing\n");
        fs::write(&path, before.replace("{}.merge(a: 1)", "{}, a: 1")).unwrap();
        let naive = ruby(&path);
        assert!(naive.status.success(), "{naive:?}\n{before}");
        assert!(naive.stderr.is_empty(), "{naive:?}");
        assert_eq!(naive.stdout, b"{:a=>1}\n", "{before}");
    }
    // The shared proof also conservatively retains guarded Array#each edits
    // when its visibility is changed, including a computed target spelling.
    for visibility in ["private", "protected"] {
        let before = format!(
            "class Array; {visibility} ('ea' + 'ch'); end\ndef f; if block_given?; [1].each {{ |x| yield x }}; end; end\np f\nbegin; f {{ |x| p x }}; rescue NoMethodError; p :missing; end\n"
        );
        assert_eq!(observed_fix(BLOCK, &before, &before, 1), b"nil\n:missing\n");
    }
}
#[test]
fn runtime_guarded_each_matches_block_absence_proc_lambda_arity_break_return_and_exceptions() {
    if !ruby_available() {
        eprintln!("Ruby unavailable; runtime probe skipped");
        return;
    }
    let before = r#"def f
  if block_given?; [1,2].each { |x| yield x }; end
end
p f
[nil, ->(x) { x*2 }, -> { :zero }, ->(x,y) { [x,y] }, ->((a,b)) { [a,b] }, ->(**k) { k }, ->(x:, **k) { [x,k] }, proc { |a,b| [a,b] }, proc { |*a, **k| [a,k] }].each do |callback|
  begin
    p f(&callback)
  rescue ArgumentError => e
    p [e.class, e.message]
  end
end
p(f { |x| break [:break,x] })
def returning
  f { |x| return [:return,x] }
end
p returning
p catch(:done) { f { |x| throw :done, x } }
begin
  f { |x| raise 'expected' }
rescue RuntimeError => e
  p [e.class, e.message]
end
p __LINE__
"#;
    let (before, driver) = before.split_once("p f\n").unwrap();
    let after = before
        .replace("def f\n", "def f(&block)\n")
        .replace(".each { |x| yield x }", ".each(&block)");
    let out = observed_fix_with_driver(BLOCK, before, &after, 0, &format!("p f\n{driver}"));
    assert!(String::from_utf8_lossy(&out).contains("wrong number of arguments"));
    assert!(String::from_utf8_lossy(&out).contains("[:return, 1]"));
    assert!(String::from_utf8_lossy(&out).contains("[:break, 1]"));
}
#[test]
fn runtime_unknown_merge_and_forwarding_witnesses_explain_individual_withholding() {
    if !ruby_available() {
        eprintln!("Ruby unavailable; runtime probe skipped");
        return;
    }
    let merge = "def take(**k); p k.to_a; end\ndef update(h); h[:a]=9; 2; end\nh={a: 1}\ntake(**h.merge(b: update(h)))\n";
    assert_eq!(
        observed_fix(MERGE, merge, merge, 1),
        b"[[:a, 9], [:b, 2]]\n"
    );
    let dir = project();
    let path = dir.path().join("x.rb");
    fs::write(
        &path,
        merge.replace("h.merge(b: update(h))", "h, b: update(h)"),
    )
    .unwrap();
    assert_eq!(ruby(&path).stdout, b"[[:a, 1], [:b, 2]]\n");
    let wrappers = "def f; 1.times { yield }; end\n[-> { :zero }, ->(x) { x }].each { |b| begin; p f(&b); rescue => e; p [e.class,e.message]; end }\nbegin; p f; rescue => e; p e.class; end\n";
    let original = observed_fix(BLOCK, wrappers, wrappers, 1);
    fs::write(
        &path,
        wrappers
            .replace("def f;", "def f(&block);")
            .replace("1.times { yield }", "1.times(&block)"),
    )
    .unwrap();
    assert_ne!(ruby(&path).stdout, original);
    let identity = "def use(&callback); p [callback.lambda?,callback.arity]; callback.call(1,2); end\ndef f; use { |x| yield x }; end\nf { |*a| p a }\n";
    observed_fix(BLOCK, identity, identity, 1);
}
#[test]
fn runtime_custom_hashes_keys_defaults_and_unsafe_redundant_arguments_stay_unchanged() {
    if !ruby_available() {
        eprintln!("Ruby unavailable; runtime probe skipped");
        return;
    }
    let before = r#"$events=[]
class H < Hash
  def merge(*a); $events << :merge; super; end
end
class Coerce
  def to_hash; $events << :to_hash; {b: 2}; end
end
class Key
  def hash; $events << :hash; 1; end
  def eql?(x); $events << :eql; true; end
end
h=H.new { |h,k| $events << :default; h[k]=4 }
h[Key.new]=1
h[:missing]
def take(**k); p k.values; end
take(**h.merge(Coerce.new))
p $events
"#;
    observed_fix(MERGE, before, before, 1);
    let redundant = "$, = ':'\np [1,2].join('')\nclass Custom; def sum(x=4); x; end; end\np Custom.new.sum(0)\n";
    assert_eq!(
        observed_fix(REDUNDANT, redundant, redundant, 1),
        b"\"12\"\n0\n"
    );
}
#[test]
fn runtime_inherited_super_defaults_rest_keywords_and_block_channel_are_not_rewritten() {
    if !ruby_available() {
        eprintln!("Ruby unavailable; runtime probe skipped");
        return;
    }
    let before = r#"$events=[]
class Parent
  def f(*a, **k, &callback)
    p [a, k.to_a, callback.lambda?, callback.arity]
    yield 1,2,key: 3
  end
end
class Child < Parent
  def f(x=($events << :default; 4), *rest, required:, optional: ($events << :keyword; 5), **keys)
    super { yield }
  end
end
p Child.new.f(required: 6, extra: 7) { |*a, **k| [a,k.to_a] }
p $events
"#;
    let out = observed_fix(BLOCK, before, before, 1);
    assert!(String::from_utf8_lossy(&out).contains("[[], []]"));
    assert!(String::from_utf8_lossy(&out).contains("[:default, :keyword]"));
}
#[test]
fn runtime_data_positions_and_source_reflection_retain_original_bytes() {
    if !ruby_available() {
        eprintln!("Ruby unavailable; runtime probe skipped");
        return;
    }
    for (cop, source) in [
        (
            MERGE,
            "def take(**k); k; end\np take(**{}.merge(y: 1))\np [DATA.pos, DATA.lineno, __LINE__]\n__END__\nraw\n",
        ),
        (
            BLOCK,
            "def f; if block_given?; [1].each { |x| yield x }; end; end\np f { |x| x }\np [method(:f).parameters, method(:f).source_location, DATA.pos, DATA.lineno, __LINE__]\n__END__\nraw\n",
        ),
    ] {
        observed_fix(cop, source, source, 1);
    }
}
#[test]
fn source_data_observers_and_directives_withhold_only_affected_work() {
    let dir = project();
    let path = dir.path().join("x.rb");
    for (cop, before, _) in &CASES[..2] {
        for observer in [
            "p __FILE__",
            "p DATA.pos",
            "p method(:f).source_location",
            "p caller_locations",
            "p binding.local_variables",
            "class Array; def each; yield 1,2; end; end",
            "class Hash; def merge(*a); {}; end; end",
            "class Hash; undef_method \"merge\"; end",
            "class Hash; remove_method \"merge\"; end",
            "class Hash; undef_method :merge; end",
            "class Hash; remove_method ('mer' + 'ge'); end",
            "class Array; undef_method \"each\"; end",
            "class Array; remove_method \"each\"; end",
        ] {
            let source = format!("{observer}\n{before}__END__\nraw\n");
            fs::write(&path, &source).unwrap();
            cli(dir.path(), &["--only", cop, "--fix", "x.rb"], 1);
            assert_eq!(fs::read_to_string(&path).unwrap(), source, "{observer}");
        }
    }
    let before = "foo(**{\n a: 1\n}.merge(b: 2))\ndef f\n if block_given?\n  [1].each do |x|\n   yield x\n  end\n end\nend\np __LINE__\n";
    fs::write(&path, before).unwrap();
    cli(
        dir.path(),
        &["--only", &format!("{MERGE},{BLOCK}"), "--fix", "x.rb"],
        1,
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
}
#[test]
fn current_style_layout_and_available_incompatible_peer_stabilize_without_cycles() {
    let dir = project();
    let path = dir.path().join("x.rb");
    let before =
        "def f\n  if block_given?; [1].each { |x| yield x }; end\nend\nfoo(**{}.merge(y: 1))\n";
    let registry = rustocop::cops::Registry::new();
    let mut peers = format!("{BLOCK},{MERGE}");
    for peer in [
        "Lint/UnusedMethodArgument",
        "Style/LambdaCall",
        "Style/TrailingCommaInArguments",
    ] {
        if registry.has_cop(peer) {
            peers.push(',');
            peers.push_str(peer);
        } else {
            eprintln!("{peer} unavailable; composition activates automatically when registered");
        }
    }
    let selections = [peers, "Style,Layout".into()];
    for only in selections {
        fs::write(&path, before).unwrap();
        let mut seen = std::collections::HashSet::new();
        for pass in 0..8 {
            let bytes = fs::read(&path).unwrap();
            assert!(
                seen.insert(bytes.clone()),
                "cycle after {pass} passes: {only}"
            );
            let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
                .current_dir(dir.path())
                .args(["--only", &only, "--fix", "x.rb"])
                .output()
                .unwrap();
            assert!(matches!(out.status.code(), Some(0 | 1)), "{out:?}");
            if fs::read(&path).unwrap() == bytes {
                break;
            }
            assert!(pass < 7, "did not stabilize in eight explicit passes");
        }
        let fixed = fs::read_to_string(&path).unwrap();
        assert!(!fixed.contains("yield"), "{only}: {fixed}");
        assert!(!fixed.contains("merge"), "{only}: {fixed}");
    }
    // No explicit call or trailing comma is eligible; signature/use edits form
    // one envelope, so any future incompatible overlap defers them together.
}
