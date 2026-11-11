//! Bounded post-pass delivery tests.
//! Fixtures adapted from RuboCop's redundant_cop_disable_directive_spec.rb and
//! comment_config_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
use super::*;
use crate::cops::{Cop, PostInvestigation};
use std::sync::Mutex;

#[derive(Debug)]
struct Snapshot {
    raw: Vec<(&'static str, u32, u32)>,
    suppressed: Vec<(&'static str, u32, u32)>,
    cops: Vec<crate::cops::InvestigatedCop>,
    corrections: bool,
}

#[derive(Default, Clone)]
struct Probe(std::sync::Arc<Mutex<Vec<Snapshot>>>);
impl Cop for Probe {
    fn name(&self) -> &'static str {
        "Lint/PostProbe"
    }
    fn wants_post_investigation(&self) -> bool {
        true
    }
    fn check_post_investigation(
        &self,
        cx: &PostInvestigation<'_>,
        _: &SourceFile,
        _: &mut Vec<Offense>,
    ) {
        let positions = |o: &Offense| (o.cop, o.line, o.end_line);
        self.0.lock().unwrap().push(Snapshot {
            raw: cx.raw_offenses.iter().map(positions).collect(),
            suppressed: cx.suppressed_offenses().map(positions).collect(),
            cops: cx.cops.to_vec(),
            corrections: cx.corrections_requested,
        });
    }
}

#[test]
fn post_receives_raw_suppressed_multiline_findings_once_after_investigation() {
    let probe = Probe::default();
    let debugger = crate::cops::lint::Debugger;
    for prefix in ["rubocop", "rustocop"] {
        let source =
            format!("binding.pry(\n value\n) # {prefix}:disable Lint/Debugger\nbinding.pry\n");
        let found = lint_str_with(&[&debugger, &probe], &source);
        assert_eq!(
            found
                .iter()
                .filter(|o| o.cop == debugger.name())
                .map(|o| o.line)
                .collect::<Vec<_>>(),
            [4]
        );
        let snapshots = probe.0.lock().unwrap();
        let snapshot = snapshots.last().unwrap();
        assert_eq!(
            snapshot.raw,
            [(debugger.name(), 1, 3), (debugger.name(), 4, 4)]
        );
        assert_eq!(snapshot.suppressed, [(debugger.name(), 1, 3)]);
        assert!(!snapshot.corrections);
    }
    assert_eq!(probe.0.lock().unwrap().len(), 2);
}

#[test]
fn post_distinguishes_unknown_names_from_registered_but_unavailable_cops() {
    let probe = Probe::default();
    lint_str_with(
        &[&probe],
        "# rubocop:disable Lint/Debugger, Style/NotRegistered\n",
    );
    let snapshots = probe.0.lock().unwrap();
    let snapshot = &snapshots[0];
    let debugger = snapshot
        .cops
        .iter()
        .find(|c| c.name == "Lint/Debugger")
        .unwrap();
    assert!(debugger.configured_enabled);
    assert!(!debugger.available);
    assert!(!debugger.investigated);
    assert!(!debugger.completed);
    assert!(
        !snapshot
            .cops
            .iter()
            .any(|c| c.name == "Style/NotRegistered")
    );
}

#[test]
fn post_uses_shared_next_signed_push_pop_and_lookalike_scopes() {
    for prefix in ["rubocop", "rustocop"] {
        for (body, suppressed) in [
            (
                "# PREFIX:disable-next Debugger\nbinding.pry(\n  value\n)\nbinding.pry\n",
                vec![2],
            ),
            ("# PREFIX:next -Lint\nbinding.pry\nbinding.pry\n", vec![2]),
            (
                "# PREFIX:push -Debugger\nbinding.pry\n# PREFIX:next +Debugger\nbinding.pry\nbinding.pry\n# PREFIX:pop\nbinding.pry\n",
                vec![2, 5],
            ),
            (
                "# PREFIX:disable Debugger\n# PREFIX:enable-next Lint\nbinding.pry\nbinding.pry\n",
                vec![4],
            ),
            ("# PREFIX:disable-next Debugger\n\nbinding.pry\n", vec![]),
            ("# # PREFIX:disable Debugger\nbinding.pry\n", vec![]),
            ("value = '# PREFIX:disable Debugger'\nbinding.pry\n", vec![]),
            (
                "# PREFIX:todo-next Debugger\nputs <<~DOC\n#{binding.pry(value)}\nDOC\nbinding.pry\n",
                vec![3],
            ),
        ] {
            let probe = Probe::default();
            let source = body.replace("PREFIX", prefix);
            lint_str_with(&[&crate::cops::lint::Debugger, &probe], &source);
            assert_eq!(
                probe.0.lock().unwrap()[0]
                    .suppressed
                    .iter()
                    .map(|o| o.1)
                    .collect::<Vec<_>>(),
                suppressed,
                "{source}"
            );
        }
    }
}

fn selected_snapshot(
    source: &str,
    toml: &str,
    only: &[&str],
    except: &[&str],
    path: &str,
) -> Snapshot {
    let probe = Probe::default();
    let registry = Registry::from_cops(vec![
        Box::new(probe.clone()),
        Box::new(crate::cops::lint::Debugger),
        Box::new(crate::cops::lint::ItWithoutArgumentsInBlock),
        Box::new(crate::cops::lint::TopLevelReturnWithArgument),
    ]);
    let config = ResolvedConfig::resolve(toml::from_str(toml).unwrap(), None, &registry).unwrap();
    let active = CopSelection::new(
        &registry,
        &config,
        &only.iter().map(|s| (*s).into()).collect::<Vec<_>>(),
        &except.iter().map(|s| (*s).into()).collect::<Vec<_>>(),
    );
    let report = lint_one_in_mode(
        Path::new(path),
        Path::new(path),
        SourceInput::Memory(source.as_bytes()),
        &active,
        &config,
        CorrectionMode::None,
    );
    assert!(report.errors.is_empty());
    assert!(report.internal_errors.is_empty());
    probe.0.lock().unwrap().pop().unwrap()
}

#[test]
fn universe_separates_configuration_selection_version_path_and_source_opt_in() {
    let settings = "[cops.Lint]\nenabled = false\n[cops.\"Lint/PostProbe\"]\nenabled = true\n";
    for (source, only, except, available, ran) in [
        ("binding.pry\n", vec![], vec![], true, false),
        (
            "# rubocop:enable Debugger\nbinding.pry\n",
            vec![],
            vec![],
            true,
            true,
        ),
        (
            "# rustocop:enable-next Lint\nbinding.pry\n",
            vec![],
            vec![],
            true,
            true,
        ),
        (
            "# rustocop:push +Debugger\nbinding.pry\n# rustocop:pop\nbinding.pry\n",
            vec![],
            vec![],
            true,
            true,
        ),
        (
            "# rubocop:enable Debugger\nbinding.pry\n",
            vec![],
            vec!["Lint/Debugger"],
            false,
            false,
        ),
        (
            "# rubocop:enable Debugger\nbinding.pry\n",
            vec!["Lint/PostProbe"],
            vec![],
            false,
            false,
        ),
        ("binding.pry\n", vec!["Lint"], vec![], true, true),
    ] {
        let snapshot = selected_snapshot(source, settings, &only, &except, "supplied.rb");
        let cop = snapshot
            .cops
            .iter()
            .find(|c| c.name == "Lint/Debugger")
            .unwrap();
        assert!(!cop.configured_enabled);
        assert_eq!(
            (cop.available, cop.investigated, cop.completed),
            (available, ran, ran),
            "{source} {only:?} {except:?}"
        );
        if ran {
            assert_eq!(
                snapshot.raw.len(),
                if source.contains("pop") { 2 } else { 1 }
            );
        }
        let inactive = snapshot
            .cops
            .iter()
            .find(|c| c.name == "Lint/ItWithoutArgumentsInBlock")
            .unwrap();
        assert!(!inactive.available && !inactive.investigated && !inactive.completed);
    }
    let snapshot = selected_snapshot(
        "binding.pry\n",
        "[cops.Lint]\nenabled=false\n[cops.\"Lint/Debugger\"]\nenabled=true\n[cops.\"Lint/PostProbe\"]\nenabled=true\n",
        &[],
        &[],
        "supplied.rb",
    );
    assert!(
        snapshot
            .cops
            .iter()
            .find(|c| c.name == "Lint/Debugger")
            .unwrap()
            .configured_enabled
    );
    for (path, settings) in [
        ("view.jb", ""),
        (
            "supplied.rb",
            "[cops.\"Lint/TopLevelReturnWithArgument\"]\nexclude=[\"supplied.rb\"]\n",
        ),
    ] {
        let snapshot = selected_snapshot("return 1\n", settings, &["Lint"], &[], path);
        let cop = snapshot
            .cops
            .iter()
            .find(|c| c.name == "Lint/TopLevelReturnWithArgument")
            .unwrap();
        assert!(cop.configured_enabled);
        assert!(!cop.available && !cop.investigated && !cop.completed);
    }
}

struct PostFinding {
    panic: bool,
    fix: bool,
}
impl Cop for PostFinding {
    fn name(&self) -> &'static str {
        "Lint/PostFinding"
    }
    fn safe_fixes(&self) -> bool {
        self.fix
    }
    fn wants_post_investigation(&self) -> bool {
        true
    }
    fn check_post_investigation(
        &self,
        _: &PostInvestigation<'_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let start = memchr::memmem::find(&file.bytes, b"before").unwrap_or(0);
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "post finding".into(),
            line,
            end_line: line,
            col,
            correction: self.fix.then(|| crate::offense::Correction {
                range: start..start + 6,
                replacement: "after".into(),
            }),
        });
        assert!(!self.panic, "post failure");
    }
}

#[test]
fn post_outputs_are_not_inputs_to_peers_and_obey_self_suppression() {
    for prefix in ["rubocop", "rustocop"] {
        for scope in ["disable", "disable-next", "push -"] {
            let separator = if scope.ends_with('-') { "" } else { " " };
            let source = format!("# {prefix}:{scope}{separator}Lint/PostFinding\nbefore\n");
            let probe = Probe::default();
            let found = lint_str_with(
                &[
                    &PostFinding {
                        panic: false,
                        fix: false,
                    },
                    &probe,
                ],
                &source,
            );
            assert!(found.is_empty(), "{source}");
            assert!(probe.0.lock().unwrap()[0].raw.is_empty());
        }
    }
    let probe = Probe::default();
    let found = lint_str_with(
        &[
            &PostFinding {
                panic: false,
                fix: false,
            },
            &probe,
        ],
        "before\n",
    );
    assert_eq!(
        (found[0].line, found[0].col, found[0].message.as_str()),
        (1, 1, "post finding")
    );
    assert!(probe.0.lock().unwrap()[0].raw.is_empty());
}

#[test]
fn ordinary_failures_are_explicit_incomplete_evidence_and_skip_own_post() {
    struct Failure;
    impl Cop for Failure {
        fn name(&self) -> &'static str {
            "Lint/Failure"
        }
        fn check_source(&self, _: &SourceFile, _: &mut Vec<Offense>) {
            panic!("ordinary failure");
        }
        fn wants_post_investigation(&self) -> bool {
            true
        }
        fn check_post_investigation(
            &self,
            _: &PostInvestigation<'_>,
            _: &SourceFile,
            _: &mut Vec<Offense>,
        ) {
            panic!("must not run");
        }
    }
    let probe = Probe::default();
    let report = lint_one(
        Path::new("x.rb"),
        Path::new("x.rb"),
        Some(b"before\n"),
        &[&Failure, &probe],
        &ResolvedConfig::default(),
    );
    assert_eq!(report.internal_errors.len(), 1);
    assert_eq!(report.internal_errors[0].panic.message, "ordinary failure");
    let snapshots = probe.0.lock().unwrap();
    let failed = snapshots[0]
        .cops
        .iter()
        .find(|c| c.name == "Lint/Failure")
        .unwrap();
    assert!(failed.investigated && failed.available);
    assert!(!failed.completed);
}

#[test]
fn post_follows_all_ordinary_phases_and_requires_explicit_opt_in() {
    struct Phases;
    fn emit(out: &mut Vec<Offense>, message: &str) {
        out.push(Offense {
            cop: "Lint/Phases",
            severity: crate::offense::Severity::Error,
            message: message.into(),
            line: 1,
            end_line: 1,
            col: 1,
            correction: None,
        });
    }
    impl Cop for Phases {
        fn name(&self) -> &'static str {
            "Lint/Phases"
        }
        fn check_source(&self, _: &SourceFile, out: &mut Vec<Offense>) {
            emit(out, "source");
        }
        fn check_parse(
            &self,
            _: &ruby_prism::ParseResult<'_>,
            _: &SourceFile,
            out: &mut Vec<Offense>,
        ) {
            emit(out, "parse");
        }
        fn check_node<'pr>(
            &self,
            node: &ruby_prism::Node<'pr>,
            _: &crate::cops::NodeContext<'_, 'pr>,
            _: &SourceFile,
            out: &mut Vec<Offense>,
        ) {
            if node.as_program_node().is_some() {
                emit(out, "enter");
            }
        }
        fn wants_leave_node(&self) -> bool {
            true
        }
        fn leave_node<'pr>(
            &self,
            node: &ruby_prism::Node<'pr>,
            _: &crate::cops::NodeContext<'_, 'pr>,
            _: &SourceFile,
            out: &mut Vec<Offense>,
        ) {
            if node.as_program_node().is_some() {
                emit(out, "leave");
            }
        }
        fn wants_post_investigation(&self) -> bool {
            true
        }
        fn check_post_investigation(
            &self,
            cx: &PostInvestigation<'_>,
            _: &SourceFile,
            _: &mut Vec<Offense>,
        ) {
            assert_eq!(
                cx.raw_offenses
                    .iter()
                    .map(|o| o.message.as_str())
                    .collect::<Vec<_>>(),
                ["source", "parse", "enter", "leave"]
            );
        }
    }
    struct NotOptedIn;
    impl Cop for NotOptedIn {
        fn name(&self) -> &'static str {
            "Lint/NotOptedIn"
        }
        fn check_post_investigation(
            &self,
            _: &PostInvestigation<'_>,
            _: &SourceFile,
            _: &mut Vec<Offense>,
        ) {
            panic!("not opted in");
        }
    }
    assert_eq!(lint_str_with(&[&Phases, &NotOptedIn], "before\n").len(), 4);
}

#[test]
fn invalid_or_resource_rejected_input_never_reaches_post() {
    for source in [
        "(\n".to_owned(),
        format!("{}1{}", "[".repeat(300), "]".repeat(300)),
    ] {
        let probe = Probe::default();
        let report = lint_one(
            Path::new("virtual.rb"),
            Path::new("virtual.rb"),
            Some(source.as_bytes()),
            &[&probe],
            &ResolvedConfig::default(),
        );
        assert!(!report.errors.is_empty());
        assert!(probe.0.lock().unwrap().is_empty());
    }
}

#[test]
fn post_panics_are_isolated_keep_findings_and_prevent_writes() {
    let probe = Probe::default();
    let registry = Registry::from_cops(vec![
        Box::new(PostFinding {
            panic: true,
            fix: true,
        }),
        Box::new(probe.clone()),
    ]);
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("x.rb"), "before\n").unwrap();
    let config = ResolvedConfig {
        root: dir.path().to_path_buf(),
        ..ResolvedConfig::default()
    };
    let path = dir.path().join("x.rb");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let report = run(
        std::slice::from_ref(&path),
        &RunOptions {
            fix: true,
            ..RunOptions::default()
        },
        &config,
        &registry,
    )
    .unwrap();
    assert_eq!(report.exit_code(), EXIT_ERRORS);
    assert_eq!(report.offense_count(), 1);
    assert_eq!(report.internal_errors.len(), 1);
    assert_eq!(report.internal_errors[0].cop, Some("Lint/PostFinding"));
    assert_eq!(report.internal_errors[0].position, None);
    assert_eq!(probe.0.lock().unwrap().len(), 1);
    assert_eq!(fs::read(&path).unwrap(), b"before\n");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}

#[test]
fn post_corrections_cannot_reach_beyond_highlights_into_disabled_lines() {
    struct Wide;
    impl Cop for Wide {
        fn name(&self) -> &'static str {
            "Lint/Wide"
        }
        fn safe_fixes(&self) -> bool {
            true
        }
        fn wants_post_investigation(&self) -> bool {
            true
        }
        fn check_post_investigation(
            &self,
            _: &PostInvestigation<'_>,
            file: &SourceFile,
            out: &mut Vec<Offense>,
        ) {
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "wide".into(),
                line: 1,
                end_line: 1,
                col: 1,
                correction: Some(crate::offense::Correction {
                    range: 0..file.bytes.len(),
                    replacement: "after\n".into(),
                }),
            });
        }
    }
    for prefix in ["rubocop", "rustocop"] {
        let source = format!("before\n# {prefix}:disable Lint/Wide\nafter\n");
        let found = lint_str_with_corrections(&[&Wide], &source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }
    assert!(
        lint_str_with_corrections(&[&Wide], "before\n")[0]
            .correction
            .is_some()
    );
}

#[test]
fn post_corrections_use_atomic_validation_and_preserve_symlink_permissions() {
    for replacement_is_invalid in [false, true] {
        struct Fix(bool);
        impl Cop for Fix {
            fn name(&self) -> &'static str {
                "Lint/PostFix"
            }
            fn safe_fixes(&self) -> bool {
                true
            }
            fn wants_post_investigation(&self) -> bool {
                true
            }
            fn check_post_investigation(
                &self,
                _: &PostInvestigation<'_>,
                file: &SourceFile,
                out: &mut Vec<Offense>,
            ) {
                if file.bytes != b"before\n" {
                    return;
                }
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    message: "fix".into(),
                    line: 1,
                    end_line: 1,
                    col: 1,
                    correction: Some(crate::offense::Correction {
                        range: 0..6,
                        replacement: if self.0 { "(" } else { "after" }.into(),
                    }),
                });
            }
        }
        let registry = Registry::from_cops(vec![Box::new(Fix(replacement_is_invalid))]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.rb");
        fs::write(&path, "before\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
            symlink(&path, dir.path().join("alias.rb")).unwrap();
        }
        let config = ResolvedConfig {
            root: dir.path().to_path_buf(),
            ..ResolvedConfig::default()
        };
        let original_time = fs::metadata(&path).unwrap().modified().unwrap();
        let run_mode = |fix| {
            run(
                &[dir.path().to_path_buf()],
                &RunOptions {
                    fix,
                    ..RunOptions::default()
                },
                &config,
                &registry,
            )
            .unwrap()
        };
        let report = run_mode(false);
        assert_eq!(report.corrections_applied, 0);
        assert_eq!(fs::read(&path).unwrap(), b"before\n");
        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            original_time
        );
        let report = run_mode(true);
        assert_eq!(report.has_errors(), replacement_is_invalid);
        assert_eq!(
            report.corrections_applied,
            usize::from(!replacement_is_invalid)
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            if replacement_is_invalid {
                b"before\n".as_slice()
            } else {
                b"after\n".as_slice()
            }
        );
        if replacement_is_invalid {
            assert_eq!(
                fs::metadata(&path).unwrap().modified().unwrap(),
                original_time
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o751
            );
            assert!(
                fs::symlink_metadata(dir.path().join("alias.rb"))
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        }
    }
}

#[test]
fn post_corrections_obey_intent_shared_source_code_heap_and_position_barriers() {
    let post = PostFinding {
        panic: false,
        fix: true,
    };
    assert!(lint_str_with(&[&post], "before\n")[0].correction.is_none());
    assert!(
        lint_str_with_corrections(&[&post], "before\n")[0]
            .correction
            .is_some()
    );
    for observer in [
        "p File.binread(__FILE__)",
        "p RubyVM::InstructionSequence.of(-> {}).to_a",
        "p ObjectSpace.count_objects",
        "p GC.stat",
        "DATA.rewind; p DATA.read\n__END__\npayload",
    ] {
        let source = format!("before\n{observer}\n");
        let found = lint_str_with_corrections(&[&post], &source);
        assert_eq!(found.len(), 1, "{source}");
        assert!(found[0].correction.is_none(), "{source}");
    }
}
