//! real CLI policy and isolated Ruby behavioral observations.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const TIME: &str = "Style/TimeNow";
const RANDOM: &str = "Style/RandomWithOffset";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn run(dir: &Path, args: &[&str], status: i32, counts: [usize; 2]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(status), "{args:?}: {out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    for (cop, count) in [TIME, RANDOM].into_iter().zip(counts) {
        assert_eq!(
            text.lines().filter(|line| line.contains(cop)).count(),
            count,
            "{args:?}: {text}"
        );
    }
    out
}
#[test]
fn defaults_selectors_config_and_no_fix_never_write() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "Time.new\nrand(6) + 1\n";
    fs::write(&path, source).unwrap();
    for (config, args, counts) in [
        (String::new(), vec!["lib/a.rb"], [1, 1]),
        (String::new(), vec!["--only", TIME, "lib/a.rb"], [1, 0]),
        (String::new(), vec!["--only", RANDOM, "lib/a.rb"], [0, 1]),
        (String::new(), vec!["--except", TIME, "lib/a.rb"], [0, 1]),
        (String::new(), vec!["--except", RANDOM, "lib/a.rb"], [1, 0]),
        (
            format!("[cops.\"{TIME}\"]\nenabled = false\n[cops.\"{RANDOM}\"]\nenabled = false\n"),
            vec!["lib/a.rb"],
            [0, 0],
        ),
        (
            "[cops.Style]\nenabled = false\n".into(),
            vec!["--only", "Style/TimeNow,Style/RandomWithOffset", "lib/a.rb"],
            [1, 1],
        ),
        (
            format!(
                "[cops.Style]\nenabled = false\n[cops.\"{TIME}\"]\nenabled = true\n[cops.\"{RANDOM}\"]\nenabled = true\n"
            ),
            vec!["lib/a.rb"],
            [1, 1],
        ),
        (
            String::new(),
            vec![
                "--only",
                "Style/TimeNow,Style/RandomWithOffset",
                "--except",
                "Style/TimeNow,Style/RandomWithOffset",
                "lib/a.rb",
            ],
            [0, 0],
        ),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(dir.path(), &args, i32::from(counts != [0, 0]), counts);
        if counts == [0, 0] {
            let mut fixing = args.clone();
            fixing.insert(0, "--fix");
            run(dir.path(), &fixing, 0, [0, 0]);
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn excludes_and_both_directive_prefixes_suppress_fixes() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, code) in [(TIME, "Time.new"), (RANDOM, "rand(6).succ")] {
        for prefix in ["rustocop", "rubocop"] {
            fs::write(dir.path().join("rustocop.toml"), "").unwrap();
            for source in [
                format!("# {prefix}:disable {cop}\n{code}\n"),
                format!("{code} # {prefix}:disable {cop}\n"),
            ] {
                fs::write(&path, &source).unwrap();
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                run(dir.path(), &["--only", cop, "--fix", "lib/a.rb"], 0, [0, 0]);
                assert_eq!(fs::read_to_string(&path).unwrap(), source);
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            }
        }
        for config in [
            format!("[cops.\"{cop}\"]\nexclude = ['lib/**']\n"),
            "exclude = ['lib/**']\n".into(),
        ] {
            fs::write(dir.path().join("rustocop.toml"), config).unwrap();
            fs::write(&path, code).unwrap();
            run(
                &dir.path().join("lib"),
                &["--only", cop, "--fix", "a.rb"],
                0,
                [0, 0],
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), code);
        }
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let source = "rand(\n 6 # rustocop:disable Style/RandomWithOffset\n).succ\nTime.new(\n # rubocop:disable Style/TimeNow\n)\n";
    fs::write(&path, source).unwrap();
    run(
        dir.path(),
        &[
            "--only",
            "Style/TimeNow,Style/RandomWithOffset",
            "--fix",
            "lib/a.rb",
        ],
        0,
        [0, 0],
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
}
#[test]
fn canonical_fixes_and_clean_error_or_withheld_inputs_keep_no_write_contract() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    fs::write(&path, "Time.new()\n::Random.rand(6).succ\n").unwrap();
    run(
        dir.path(),
        &[
            "--only",
            "Style/TimeNow,Style/RandomWithOffset",
            "--fix",
            "lib/a.rb",
        ],
        0,
        [0, 0],
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "Time.now\n::Random.rand(1..6)\n"
    );
    for (cop, source, status, counts) in [
        (TIME, "Time.now\n", 0, [0, 0]),
        (TIME, "Time.new(\n", 2, [0, 0]),
        (TIME, "Time.new { break :value }\n", 1, [1, 0]),
        (TIME, "Time = Custom\nTime.new\n", 1, [1, 0]),
        (RANDOM, "1 - rand(6)\n", 1, [0, 1]),
        (RANDOM, "rand(0).succ\n", 1, [0, 1]),
        (RANDOM, "rand(-6).succ\n", 1, [0, 1]),
        (RANDOM, "rand(3..1).succ\n", 1, [0, 1]),
        (RANDOM, "def rand(x); 99; end\nrand(6).succ\n", 1, [0, 1]),
    ] {
        fs::write(&path, source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(
            dir.path(),
            &["--only", cop, "--fix", "lib/a.rb"],
            status,
            counts,
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
fn has_ruby() -> bool {
    Command::new("ruby").arg("--version").output().is_ok()
}
fn ruby(dir: &Path) -> Output {
    Command::new("ruby")
        .current_dir(dir)
        .arg("lib/a.rb")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap()
}
fn same(before: &Output, after: &Output) {
    assert!(before.status.success(), "{before:?}");
    assert_eq!(before.status.code(), after.status.code(), "{after:?}");
    assert_eq!(before.stdout, after.stdout);
    assert_eq!(before.stderr, after.stderr);
}
#[test]
fn seeded_translations_preserve_draws_types_bounds_order_defaults_and_control_flow() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let expressions = [
        "rand(6) + 1",
        "1 + Kernel.rand(6)",
        "::Kernel.rand(6) - 1",
        "Random.rand(0...6).succ",
        "::Random.rand(-3..3).pred",
        "rand(1).next",
        "rand(1..1) + 4",
        "rand(0...1).pred",
        "rand(2147483648) + 3",
        "rand(4294967296) - 2",
        "rand(4611686018427387903) - 7",
        "rand(-10..-3) + 2",
    ];
    for seed in [0, 1, 42, 12345, 999999] {
        let source = format!(
            "srand({seed}); Random.srand({seed})\n$events = []\ndef record(x); $events << :record; x; end\ndef f(x = rand(6).succ); $events << :default; x; end\ndef early; return rand(6) - 1; end\nvalues = [{} , record(f), -> {{ rand(6).pred }}.call, early, catch(:done) {{ throw :done, rand(6).next }}]\np [values, values.map(&:class), rand, Random.rand, $events, __LINE__]\n",
            expressions.join(", ")
        );
        let path = dir.path().join("lib/a.rb");
        fs::write(&path, &source).unwrap();
        let before = ruby(dir.path());
        assert!(String::from_utf8_lossy(&before.stdout).contains("[:default, :record]"));
        run(
            dir.path(),
            &["--only", RANDOM, "--fix", "lib/a.rb"],
            0,
            [0, 0],
        );
        let fixed = fs::read_to_string(&path).unwrap();
        assert_ne!(fixed, source);
        assert!(fixed.contains("rand(1..6)"));
        same(&before, &ruby(dir.path()));
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        run(
            dir.path(),
            &["--only", RANDOM, "--fix", "lib/a.rb"],
            0,
            [0, 0],
        );
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
#[test]
fn time_uses_the_real_clock_and_preserves_defaults_chaining_and_ignored_blocks() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let source = "def f(t = ::Time.new()); t; end\na = Time.now\ntimes = [Time.new, Time&.new(), f, -> { Time.new }.call]\nb = Time.now\np [times.map(&:class), times.all? { |t| t >= a && t <= b }, Time.new.year == b.year]\n";
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path());
    run(
        dir.path(),
        &["--only", TIME, "--fix", "lib/a.rb"],
        0,
        [0, 0],
    );
    assert!(!fs::read_to_string(&path).unwrap().contains("Time.new"));
    same(&before, &ruby(dir.path()));
    let source = "$calls = 0; class Callback; def to_proc; $calls += 1; proc { raise 'ignored' }; end; end\np [Time.new(2026, 8, 19).year, Time.new(in: '+09:00').utc_offset, Time.new(&Callback.new).class, $calls]\n";
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path());
    assert_eq!(before.stdout, b"[2026, 32400, Time, 1]\n");
    run(
        dir.path(),
        &["--only", TIME, "--fix", "lib/a.rb"],
        0,
        [0, 0],
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    same(&before, &ruby(dir.path()));
    // Blocks are ignored by the actual APIs; preserve them byte-for-byte
    // rather than risk changing control flow for a nonstandard implementation.
    let source = "$events = []; p Time.new { $events << :block; break :bad }.class; p $events\n";
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path());
    assert_eq!(before.stdout, b"Time\n[]\n");
    run(
        dir.path(),
        &["--only", TIME, "--fix", "lib/a.rb"],
        1,
        [1, 0],
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    same(&before, &ruby(dir.path()));
}
#[test]
fn time_empty_parentheses_preserve_signed_operand_binding_at_runtime() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for receiver in ["Time.", "::Time.", "Time&.", "Time::"] {
        for operand in ["+1", "-1"] {
            let source = format!(
                "a = Time.now\nt = {receiver}new() {operand}\nb = Time.now\np [t.class, t >= (a {operand}) && t <= (b {operand})]\n"
            );
            fs::write(&path, &source).unwrap();
            let before = ruby(dir.path());
            assert_eq!(before.stdout, b"[Time, true]\n");
            run(
                dir.path(),
                &["--only", TIME, "--fix", "lib/a.rb"],
                0,
                [0, 0],
            );
            same(&before, &ruby(dir.path()));
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                source.replace("new()", "now()")
            );
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            run(
                dir.path(),
                &["--only", TIME, "--fix", "lib/a.rb"],
                0,
                [0, 0],
            );
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
    }
}
#[test]
fn visible_api_mutations_reflections_and_bound_type_hazards_are_never_substituted() {
    if !has_ruby() {
        return;
    }
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    for (cop, source) in [
        (
            TIME,
            "Time.define_singleton_method(:now) { :changed }; p Time.new.class\n",
        ),
        (
            TIME,
            "Time.define_method(:initialize) { @marker = :custom; super() }; p Time.new.instance_variable_get(:@marker)\n",
        ),
        (
            TIME,
            "Class.class_eval('def new; :changed; end'); p Time.new\n",
        ),
        (
            TIME,
            "Time.singleton_class.class_eval { alias now object_id }; p Time.new.class\n",
        ),
        (
            TIME,
            "Time.instance_eval('def now; :changed; end'); p Time.new.class\n",
        ),
        (
            TIME,
            "module M; Time = Class.new { def self.new; :shadowed; end }; p Time.new; end\n",
        ),
        (
            TIME,
            "module M; refine Time.singleton_class do; def now; :changed; end; end; end; using M; p Time.new.class\n",
        ),
        (RANDOM, "def rand(n); n * 2; end; p rand(6) + 1\n"),
        (
            RANDOM,
            "Random.define_singleton_method(:rand) { |n| n * 2 }; p Random.rand(6) + 1\n",
        ),
        (
            RANDOM,
            "module M; Random = Class.new { def self.rand(n); n * 2; end }; p Random.rand(6) + 1; end\n",
        ),
        (
            RANDOM,
            "module M; refine Kernel do; def rand(n); n * 2; end; end; end; using M; p rand(6) + 1\n",
        ),
        (
            RANDOM,
            "class Integer; def succ; :changed; end; end; p rand(6).succ\n",
        ),
        (RANDOM, "srand(42); p [1 - rand(6), rand]\n"),
        (RANDOM, "srand(42); p [rand(0) + 1, rand]\n"),
        (RANDOM, "srand(42); p [rand(-6) + 1, rand]\n"),
        (
            RANDOM,
            "Random.srand(42); begin; p Random.rand(0).succ; rescue => e; p e.class; end; p Random.rand\n",
        ),
        (
            RANDOM,
            "Random.srand(42); begin; p Random.rand(-6).succ; rescue => e; p e.class; end; p Random.rand\n",
        ),
        (
            RANDOM,
            "srand(42); begin; p rand(6..2).succ; rescue => e; p e.class; end; p rand\n",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let before = ruby(dir.path());
        run(
            dir.path(),
            &["--only", cop, "--fix", "lib/a.rb"],
            1,
            if cop == TIME { [1, 0] } else { [0, 1] },
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        same(&before, &ruby(dir.path()));
    }
    let source = "$events = []; def bound; $events << :bound; 6.0; end; def offset; $events << :offset; 1; end; srand(42); p [rand(bound) + offset, Random.new(42).rand(6) + 1, $events]\n";
    fs::write(&path, source).unwrap();
    let before = ruby(dir.path());
    run(
        dir.path(),
        &["--only", RANDOM, "--fix", "lib/a.rb"],
        0,
        [0, 0],
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    same(&before, &ruby(dir.path()));
}
#[test]
fn bom_crlf_encoding_data_and_line_survive_edits_and_mixed_fixes_stabilize() {
    let dir = project();
    let path = dir.path().join("lib/a.rb");
    let listed = "\u{feff}# encoding: UTF-8\r\nsrand(42)\r\np [Time.new.class, rand(0x6).succ, __LINE__, 'é'.encoding.name, DATA.read.bytes]\r\n__END__\r\nraw\0\r\n";
    // Reading `DATA` is a listed observer: that file keeps its
    // findings and bytes. The unread data section survives below.
    fs::write(&path, listed).unwrap();
    run(
        dir.path(),
        &[
            "--only",
            "Style/TimeNow,Style/RandomWithOffset",
            "--fix",
            "lib/a.rb",
        ],
        1,
        [1, 1],
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), listed);
    let source = &listed.replace(", DATA.read.bytes]", "]");
    fs::write(&path, source).unwrap();
    let before = has_ruby().then(|| ruby(dir.path()));
    run(
        dir.path(),
        &[
            "--only",
            "Style/TimeNow,Style/RandomWithOffset",
            "--fix",
            "lib/a.rb",
        ],
        0,
        [0, 0],
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        source
            .replace("Time.new", "Time.now")
            .replace("rand(0x6).succ", "rand(1..6)")
    );
    if let Some(before) = before {
        same(&before, &ruby(dir.path()));
    }
    let bytes =
        b"# encoding: ISO-8859-1\r\n# caf\xe9\r\nTime.new\r\nrand(6) + 1\r\n__END__\r\nraw\xff\r\n";
    fs::write(&path, bytes).unwrap();
    run(
        dir.path(),
        &[
            "--only",
            "Style/TimeNow,Style/RandomWithOffset",
            "--fix",
            "lib/a.rb",
        ],
        0,
        [0, 0],
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        b"# encoding: ISO-8859-1\r\n# caf\xe9\r\nTime.now\r\nrand(1..6)\r\n__END__\r\nraw\xff\r\n"
    );
    fs::write(
        &path,
        "def f\n  return [Time.new(), (rand(6).succ), {'x' => 1}];  \nend\n",
    )
    .unwrap();
    let only = "Style/TimeNow,Style/RandomWithOffset,Style/StringLiterals,Style/HashSyntax,Style/RedundantReturn,Style/RedundantParentheses,Style/Semicolon,Style/SignalException,Layout";
    for _ in 0..4 {
        let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
            .current_dir(dir.path())
            .args(["--only", only, "--fix", "lib/a.rb"])
            .output()
            .unwrap();
        assert_ne!(out.status.code(), Some(2), "{out:?}");
    }
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "def f\n  [Time.now, rand(1..6), { \"x\" => 1 }]\nend\n"
    );
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    run(
        dir.path(),
        &["--only", only, "--fix", "lib/a.rb"],
        0,
        [0, 0],
    );
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
