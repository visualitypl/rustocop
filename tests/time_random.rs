//! real CLI policy and isolated Ruby behavioral observations.
use crate::support::cli::project_with_lib;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const TIME: &str = "Style/TimeNow";
const RANDOM: &str = "Style/RandomWithOffset";
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
    let dir = project_with_lib();
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
    let dir = project_with_lib();
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
    let dir = project_with_lib();
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
    let dir = project_with_lib();
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
    let dir = project_with_lib();
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
