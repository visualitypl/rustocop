//! exact safe corrections, CLI controls and controlled semantic witnesses.
use std::{fs, path::Path, process::Command};
const COP: &str = "Style/RedundantCondition";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}

fn run(root: &Path, args: &[&str], status: Option<i32>, count: usize) {
    let output = cli(root, args);
    assert_eq!(output.status.code(), status, "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| l.contains(COP))
            .count(),
        count,
        "{output:?}"
    );
}

fn cli(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

fn unchanged(root: &Path, source: &[u8], count: usize) {
    let path = root.join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    run(
        root,
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(i32::from(count > 0)),
        count,
    );
    assert_eq!(fs::read(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn safe_fixes_obey_defaults_selectors_and_per_cop_enabling() {
    let dir = project();
    let source = "@b ? @b : fallback\n";
    let fixed = "@b || fallback\n";
    for (config, selectors, corrected) in [
        (String::new(), vec![], true),
        (String::new(), vec!["--only", COP], true),
        (String::new(), vec!["--only", COP, "--except", COP], false),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec![],
            false,
        ),
        (
            format!("[cops.\"{COP}\"]\nenabled = false\n"),
            vec!["--only", COP],
            true,
        ),
        (
            format!("[cops.Style]\nenabled = false\n[cops.\"{COP}\"]\nenabled = true\n"),
            vec![],
            true,
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        fs::write(dir.path().join("lib/a.rb"), source).unwrap();
        let mut args = selectors;
        args.extend(["--fix", "lib/a.rb"]);
        let output = cli(dir.path(), &args);
        assert_ne!(output.status.code(), Some(2), "{output:?}");
        assert_eq!(
            fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
            if corrected { fixed } else { source },
            "{output:?}"
        );
    }
}

#[test]
fn safe_candidates_honor_excludes_disabled_spans_and_plain_lint_mtimes() {
    let dir = project();
    let source = "@b ? @b : fallback\n";
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, source).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    run(dir.path(), &["--only", COP, "lib/a.rb"], Some(1), 1);
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    for config in [
        "exclude = [\"lib/a.rb\"]\n".to_owned(),
        format!("[cops.\"{COP}\"]\nexclude = [\"lib/a.rb\"]\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(dir.path(), source.as_bytes(), 0);
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for prefix in ["rustocop", "rubocop"] {
        for target in [COP, "Style", "all"] {
            unchanged(
                dir.path(),
                format!("# {prefix}:disable {target}\n{source}").as_bytes(),
                0,
            );
        }
        unchanged(
            dir.path(),
            format!("# {prefix}:disable-next {COP}\n{source}").as_bytes(),
            0,
        );
        unchanged(
            dir.path(),
            format!("@b ? @b : fallback # {prefix}:disable {COP}\n").as_bytes(),
            0,
        );
        unchanged(
            dir.path(),
            format!("if @b\n  @b # {prefix}:disable {COP}\nelse\n  fallback\nend\n").as_bytes(),
            0,
        );
        let before = format!("# {prefix}:disable {COP}\n{source}# {prefix}:enable {COP}\n{source}");
        fs::write(&path, &before).unwrap();
        run(
            dir.path(),
            &["--only", COP, "--fix", "lib/a.rb"],
            Some(0),
            0,
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("# {prefix}:disable {COP}\n{source}# {prefix}:enable {COP}\n@b || fallback\n")
        );
    }
}

#[test]
fn every_valid_pinned_correction_is_exact_or_a_reasoned_no_write() {
    // RuboCop specs: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    let dir = project();
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/redundant_condition_pinned.json")).unwrap();
    let mut counts = (0, 0, 0);
    for c in cases.into_iter().filter(|c| c["correction"].is_string()) {
        let source = c["source"].as_str().unwrap();
        if ruby_prism::parse(source.as_bytes()).errors().count() > 0 {
            counts.2 += 1;
            fs::write(dir.path().join("lib/a.rb"), source).unwrap();
            run(
                dir.path(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(2),
                0,
            );
            assert_eq!(
                fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
                source
            );
        } else if source == "@b ? @b : c\n" {
            counts.0 += 1;
            fs::write(dir.path().join("lib/a.rb"), source).unwrap();
            run(
                dir.path(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
                0,
            );
            let expected = c["correction"].as_str().unwrap();
            assert_eq!(
                fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
                expected
            );
            unchanged(dir.path(), expected.as_bytes(), 0);
        } else {
            // Repeated sends and custom predicates have no repeatability or
            // Boolean-return proof; these must not be presented as safe edits.
            counts.1 += 1;
            unchanged(dir.path(), source.as_bytes(), 1);
        }
    }
    assert_eq!(counts, (1, 62, 4));
}

fn observations_equal(root: &Path) {
    let mut rubies = vec!["ruby".to_owned()];
    if let Ok(ruby) = crate::support::runtime::ruby() {
        rubies.push(ruby);
    }
    for ruby in rubies {
        let version = Command::new(&ruby).arg("--version").output();
        if ruby == "ruby" && version.is_err() {
            continue;
        }
        let version = version.expect("explicit RUSTOCOP_RUBY34 must be executable");
        assert!(version.status.success(), "{ruby}: {version:?}");
        if ruby != "ruby" {
            assert!(
                String::from_utf8_lossy(&version.stdout)
                    .starts_with(crate::support::runtime::banner_prefix())
            );
        }
        let before = Command::new(&ruby)
            .arg(root.join("before.rb"))
            .output()
            .unwrap();
        let after = Command::new(&ruby)
            .arg(root.join("lib/a.rb"))
            .output()
            .unwrap();
        assert!(
            before.status.success() && after.status.success(),
            "{ruby}: {before:?} {after:?}"
        );
        assert_eq!(before.stdout, after.stdout, "{ruby}");
        assert_eq!(before.stderr, after.stderr, "{ruby}");
    }
}

#[test]
fn runtime_values_binding_branch_effects_guards_and_rescue_ensure() {
    let source = r#"$trace = []
def emit(x); $trace << x; x; end
def consume(x); $trace << [:consume, x]; x; end
def store=(x); $trace << [:store, x]; :ignored; end
def value; :method; end
def example(v)
  @b = v
  plain = if @b then @b else emit(:fallback) end
  absent = if @b then @b end
  assigned = if @b then value = @b else value = emit(:assigned) end
  method = if @b then consume(@b) else consume(emit(:method)) end
  @stack = []
  operator = if @b then @stack << @b else @stack << emit(:operator) end
  setter = if @b then self.store = @b else self.store = emit(:setter) end
  nested = if v
    if @b then @b else :nested end
  elsif v.nil?
    :nil
  else
    :false
  end
  protected_value = begin
    if @b then @b else begin; emit(:raising); raise 'x'; rescue; emit(:rescued); ensure; emit(:ensured); end end
  rescue
    emit(:outer_rescue)
  ensure
    emit(:outer_ensure)
  end
  [plain, absent, assigned, value, method, operator, setter, nested, protected_value]
end
[true, false, nil, :truthy].each { |v| p example(v) }
p $trace
"#;
    let dir = project();
    fs::write(dir.path().join("before.rb"), source).unwrap();
    fs::write(dir.path().join("lib/a.rb"), source).unwrap();
    run(
        dir.path(),
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(0),
        0,
    );
    let fixed = fs::read_to_string(dir.path().join("lib/a.rb")).unwrap();
    assert!(fixed.contains("absent = @b || nil"));
    assert!(fixed.contains("consume(@b || emit(:method))"));
    assert!(fixed.contains("@stack << (@b || emit(:operator))"));
    assert!(fixed.contains("self.store = @b || emit(:setter)"));
    observations_equal(dir.path());
}

#[test]
fn runtime_repeated_sends_and_nonboolean_predicates_prove_withholding() {
    let source = r#"$trace = []
def guard; $trace << :guard; $trace.count(:guard) == 1 ? :first : :second; end
def unusual?; :not_boolean; end
a = if guard then guard else :fallback end
b = unusual? ? true : :fallback
p [a, b, $trace]
"#;
    let dir = project();
    fs::write(dir.path().join("before.rb"), source).unwrap();
    unchanged(dir.path(), source.as_bytes(), 2);
    observations_equal(dir.path());
}

#[test]
fn runtime_modifier_scope_return_break_next_yield_and_block_association() {
    let source = r#"$trace = []
def collect(x); $trace << [:collect, block_given?]; x; end
def work(x); $trace << [:work, block_given?]; yield if block_given?; x; end
def returns(v)
  @b = v
  if @b then @b else return :early end
end
def yields(v)
  @b = v
  if @b then @b else yield 4 end
end
def modifiers(v)
  @b = v
  result = if @b then @b else (x = :bound if true) end
  [result, x]
end
def blocks(v)
  @b = v
  collect(if @b then @b else work(1) do; $trace << :body; end end)
end
[true, false, nil].each do |v|
  @b = v
  p returns(v)
  p yields(v) { |x| x * 2 }
  p modifiers(v)
  p blocks(v)
  p [1, 2].map { |x| if @b then @b else next x * 2 end }
  p([1, 2].each { |x| if @b then @b else break x end })
end
p $trace
"#;
    let dir = project();
    fs::write(dir.path().join("before.rb"), source).unwrap();
    fs::write(dir.path().join("lib/a.rb"), source).unwrap();
    run(
        dir.path(),
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(0),
        0,
    );
    observations_equal(dir.path());
}

#[test]
fn predicate_sensitive_fallbacks_remain_values_and_are_not_rewritten() {
    let dir = project();
    for fallback in [
        "/yes/",
        "/#{'yes'}/",
        "1..2",
        "1...2",
        "(1..2)",
        "begin; /yes/; end",
        "(nil || /yes/)",
    ] {
        for context in [
            "if VALUE; puts 'yes'; else; puts 'no'; end",
            "unless VALUE; puts 'no'; else; puts 'yes'; end",
            "p(VALUE ? :yes : :no)",
            "p(!VALUE)",
            "p((not VALUE))",
            "while VALUE; puts 'yes'; break; end",
            "until VALUE; puts 'no'; break; end",
            "case; when VALUE; puts 'yes'; else; puts 'no'; end",
        ] {
            let source = format!(
                "@b = false; $_ = 'no'; {}\n",
                context.replace("VALUE", &format!("(if @b then @b else {fallback} end)"))
            );
            fs::write(dir.path().join("before.rb"), &source).unwrap();
            unchanged(dir.path(), source.as_bytes(), 1);
            observations_equal(dir.path());
        }
    }
    // Assignments and call arguments keep literal value semantics even inside
    // a predicate; ordinary body/value contexts must remain correctable too.
    let source = "$_ = 'no'; @b = false\ndef use(x); p x; x; end\nvalue = if @b then @b else /yes/ end\np value\nif true\n  p(if @b then @b else 1..2 end)\nend\nif (if @b then @v = @b else @v = /yes/ end); p @v; end\nif (if @b then use(@b) else use(1..2) end); puts 'yes'; end\n";
    fs::write(dir.path().join("before.rb"), source).unwrap();
    fs::write(dir.path().join("lib/a.rb"), source).unwrap();
    run(
        dir.path(),
        &["--only", COP, "--fix", "lib/a.rb"],
        Some(0),
        0,
    );
    observations_equal(dir.path());
}

#[test]
fn extracted_command_calls_do_not_capture_siblings_or_outer_blocks() {
    let dir = project();
    let conditional = "if @b then bar @b else bar(:fallback) end";
    for (before, after) in [
        (
            format!("outer({conditional}, :tail)"),
            "outer((bar @b || :fallback), :tail)",
        ),
        (
            format!("p [{conditional}, :tail]"),
            "p [(bar @b || :fallback), :tail]",
        ),
        (
            format!("outer({conditional}, :tail) do; :outer_block; end"),
            "outer((bar @b || :fallback), :tail) do; :outer_block; end",
        ),
        (
            format!("outer({conditional}, :tail) {{ :outer_block }}"),
            "outer((bar @b || :fallback), :tail) { :outer_block }",
        ),
    ] {
        let setup = "def bar(*xs); p [:bar, block_given?]; xs; end\ndef outer(*xs); p [xs, block_given?]; end\n";
        for guard in ["false", "nil", ":truthy"] {
            let source = format!("{setup}@b = {guard}; {before}\n");
            let expected = format!("{setup}@b = {guard}; {after}\n");
            fs::write(dir.path().join("before.rb"), &source).unwrap();
            fs::write(dir.path().join("lib/a.rb"), &source).unwrap();
            run(
                dir.path(),
                &["--only", COP, "--fix", "lib/a.rb"],
                Some(0),
                0,
            );
            assert_eq!(
                fs::read_to_string(dir.path().join("lib/a.rb")).unwrap(),
                expected
            );
            observations_equal(dir.path());
            unchanged(dir.path(), expected.as_bytes(), 0);
        }
    }
}

#[test]
fn mixed_style_layout_overlaps_are_deterministic_and_stabilize() {
    let dir = project();
    let source = "def example\n  if @b\n    @b\n  else\n    return 'fallback';\n  end\nend\n";
    let only = "Style/RedundantCondition,Style/RedundantReturn,Style/StringLiterals,Style/Semicolon,Style/MultilineTernaryOperator,Style/ConditionalAssignment,Style/IfUnlessModifier,Style/Next,Style/SoleNestedConditional,Layout/TrailingWhitespace,Layout/SpaceAroundOperators";
    let path = dir.path().join("lib/a.rb");
    let mut first = None;
    for _ in 0..2 {
        fs::write(&path, source).unwrap();
        let o = cli(
            dir.path(),
            &["--only", only, "--fix", "--fix-passes", "1", "lib/a.rb"],
        );
        assert_ne!(o.status.code(), Some(2), "{o:?}");
        let bytes = fs::read(&path).unwrap();
        assert_eq!(bytes, b"def example\n  @b || (return 'fallback')\nend\n");
        if let Some(expected) = &first {
            assert_eq!(&bytes, expected);
        }
        first = Some(bytes);
    }
    let mut stable = false;
    for _ in 0..3 {
        let before = fs::read(&path).unwrap();
        let o = cli(
            dir.path(),
            &["--only", only, "--fix", "--fix-passes", "1", "lib/a.rb"],
        );
        assert_ne!(o.status.code(), Some(2), "{o:?}");
        if fs::read(&path).unwrap() == before {
            stable = true;
            break;
        }
    }
    assert!(
        stable,
        "curated mix must stabilize within four explicit passes"
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        b"def example\n  @b || (return \"fallback\")\nend\n"
    );
}
