//! `Lint/RedundantCopDisableDirective` — post-investigation suppression diagnostics.
//! Algorithm ported from RuboCop (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
use crate::{
    cops::{Cop, PostInvestigation, name_similarity},
    directives::{
        comments::{DirectiveComment, hits},
        syntax,
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use std::collections::{BTreeMap, BTreeSet};

pub struct RedundantCopDisableDirective;
const NAME: &str = "Lint/RedundantCopDisableDirective";
impl Cop for RedundantCopDisableDirective {
    fn name(&self) -> &'static str {
        NAME
    }
    fn wants_post_investigation(&self) -> bool {
        true
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn check_post_investigation(
        &self,
        cx: &PostInvestigation<'_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Token indices preserve multiplicity: a duplicate spelling must not
        // cause the necessary surviving suppression to be reported/deleted.
        let mut redundant: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        let known: Vec<_> = cx.cops.iter().map(|c| c.name).collect();
        let mut selectors = BTreeMap::new();
        let mut evaluated_broad_ranges = BTreeSet::new();
        let own = cx.directives.disabled_ranges(NAME);
        for d in cx.directives.records() {
            let mut seen_unknown = BTreeSet::new();
            for (index, n) in d.names.iter().enumerate() {
                // Expand/check each distinct selector once per investigation,
                // never once per expanded cop and attributed range.
                let selector = selectors.entry(n.as_str()).or_insert_with(|| {
                    let all = n == "all";
                    let department = known.iter().any(|c| c.split('/').next() == Some(n));
                    let registered = known.iter().any(|c| hits(n, c));
                    let targets = || {
                        cx.cops.iter().filter(|c| {
                            hits(n, c.name)
                                && (!(all || department) || !matches!(c.name, NAME | "Lint/Syntax"))
                        })
                    };
                    // RuboCop has these cops on and rustocop has them opt-in.
                    // While none of them runs, the comment is no evidence of
                    // anything: the code may be linted by RuboCop too. The pin
                    // treats its pending cops that do not run in the same way.
                    let dormant = registered
                        && targets().all(|c| {
                            crate::cops::policy::opt_in_here(c.name)
                                && !c.configured_enabled
                                && !c.investigated
                        });
                    let complete = !dormant
                        && targets().all(|c| {
                            c.available
                                && ((!c.configured_enabled && !c.investigated) || c.completed)
                        });
                    (all, department, registered, complete)
                });
                // An unregistered-as-written name cannot suppress a detector.
                // Handle it directly; never build source-vocabulary x source
                // directive rescans for arbitrarily many unknown selectors.
                if matches!(d.mode, "disable" | "todo" | "disable-next" | "todo-next")
                    && !selector.2
                    && seen_unknown.insert(n)
                    && !cx
                        .directives
                        .statement_bounds(d)
                        .is_some_and(|(first, last)| {
                            own.iter().any(|r| r.first <= first && r.last >= last)
                        })
                {
                    redundant.entry(d.range.start).or_default().insert(index);
                }
            }
            if matches!(
                d.mode,
                "disable-next" | "todo-next" | "enable-next" | "next"
            ) && !cx.directives.attached(d)
            {
                // The pin collects detached raw names in a Set, but compares
                // its size with the original token count. Identical targets
                // therefore report only the first token, not the whole comment;
                // distinct alias/qualified spellings remain separate identities.
                let mut seen = BTreeSet::new();
                let names: Vec<_> = if d.mode == "next" {
                    d.signed.iter().map(|(_, n)| n).collect()
                } else {
                    d.names.iter().collect()
                };
                redundant.entry(d.range.start).or_default().extend(
                    names
                        .into_iter()
                        .enumerate()
                        .filter_map(|(index, n)| seen.insert(n).then_some(index)),
                );
            }
        }
        for fact in cx.cops {
            let cop = fact.name;
            if cop == "Lint/Syntax" {
                continue;
            }
            let ranges = cx.directives.disabled_ranges(cop);
            for (i, r) in ranges.iter().enumerate() {
                let Some(d) = r.directive else {
                    continue;
                };
                if matches!(d.mode, "push" | "pop" | "next")
                    || own.iter().any(|s| s.first <= r.first && s.last >= r.last)
                {
                    continue;
                }
                let Some(index) = d
                    .names
                    .iter()
                    .position(|n| n == cop)
                    .or_else(|| {
                        // Only a fully qualified target overrides a department;
                        // an alias expanded alongside it keeps department semantics.
                        d.names
                            .iter()
                            .position(|n| Some(n.as_str()) == cop.split('/').next())
                    })
                    .or_else(|| {
                        d.names
                            .iter()
                            .position(|n| Some(n.as_str()) == cop.rsplit('/').next())
                    })
                    .or_else(|| d.names.iter().position(|n| hits(n, cop)))
                else {
                    continue;
                };
                let target = d.names[index].as_str();
                let &(all, department, _, complete) = &selectors[target];
                // A partial/failed investigation is NOT proof of no offenses.
                if !complete {
                    continue;
                }
                let already = i > 0 && ranges[i - 1].last == r.first;
                // The pin keeps an interval for every expanded token. Plain
                // repeated disables close/reopen on the comment line, so their
                // ranges follow. Repeated next targets instead share the whole
                // attached statement interval: end == begin only for one line,
                // not a multiline call or a heredoc. Mirror that adjacency
                // without duplicating the shared suppression ranges.
                let duplicate_ranges_follow =
                    !matches!(d.mode, "disable-next" | "todo-next") || r.last == r.first;
                let duplicated = duplicate_ranges_follow
                    && !all
                    && !department
                    && d.names
                        .iter()
                        .enumerate()
                        .any(|(other, n)| other != index && hits(n, cop));
                // Configured-off rules can be judged only for a repeated disable
                // or a bounded range; an intentional final re-disable is kept.
                if !already && !duplicated && !fact.configured_enabled && r.last == u32::MAX {
                    continue;
                }
                if all && ranges.get(i + 1).is_some_and(|n| n.first == r.last) {
                    continue;
                }
                // Different expanded cops can have different range boundaries
                // (specific enables/next/push scopes), so deduplicate only equal
                // effective ranges after the cop-specific eligibility guards.
                if (all || department)
                    && !evaluated_broad_ranges.insert((d.range.start, index, r.first, r.last))
                {
                    continue;
                }
                let global_present = r.first <= 1
                    && cx.raw_global_offenses.iter().any(|o| {
                        all || if department {
                            o.cop.split('/').next() == Some(target)
                        } else {
                            o.cop == cop
                        }
                    });
                let absent = !global_present
                    && !cx.raw_offenses.iter().any(|o| {
                        (all || if department {
                            o.cop.split('/').next() == Some(target)
                        } else {
                            o.cop == cop
                        }) && o.line <= r.last
                            && o.end_line >= r.first
                    });
                if absent || (!all && !department && (already || duplicated)) {
                    redundant.entry(d.range.start).or_default().insert(index);
                }
            }
        }
        // A deliberately closed, executable-empty source proof. Ordinary
        // comment removals can change __LINE__, DATA, tracing, backtraces and
        // directive attachment. Keep those findings without speculative edits.
        let closed = cx.corrections_requested && comments_only(file);
        for d in cx.directives.records() {
            let Some(names) = redundant.get(&d.range.start) else {
                continue;
            };
            let text = &file.bytes[d.range.clone()];
            let Some(m) = syntax::matches(text).into_iter().next() else {
                continue;
            };
            let raw: Vec<_> = if d.mode == "next" {
                d.signed.iter().map(|(_, n)| n).collect()
            } else {
                d.names.iter().collect()
            };
            let entire = names.len() == raw.len();
            let describe = |name: &str| {
                if name == "all" {
                    return "all cops".to_owned();
                }
                if known.iter().any(|c| c.split('/').next() == Some(name)) {
                    return format!("`{name}` department");
                }
                let resolved = known
                    .iter()
                    .find(|c| **c == name || c.rsplit('/').next() == Some(name));
                if let Some(resolved) = resolved {
                    return format!("`{resolved}`");
                }
                let base = name.rsplit('/').next().unwrap_or(name);
                let similar = known
                    .iter()
                    .find(|c| c.rsplit('/').next() == Some(base))
                    .copied()
                    .or_else(|| name_similarity::find_similar_name(name, &known));
                similar.map_or_else(
                    || format!("`{name}` (unknown cop)"),
                    |s| format!("`{name}` (did you mean `{s}`?)"),
                )
            };
            let mut correction = if closed {
                removal(file, d, names, entire, &known)
            } else {
                None
            };
            let enabling = d.mode == "enable-next";
            let mut report = |start: usize, end: usize, descriptions: Vec<String>| {
                let (line, col) = file.line_col(start);
                out.push(Offense {
                    cop: NAME,
                    severity: self.severity(),
                    line,
                    col,
                    end_line: file.line_of(end.saturating_sub(1).max(start)),
                    message: format!(
                        "Unnecessary {} of {}.",
                        if enabling { "enabling" } else { "disabling" },
                        descriptions.join(", ")
                    ),
                    correction: correction.take(),
                });
            };
            if entire {
                report(
                    d.range.start + m.start,
                    d.range.start + m.end,
                    names
                        .iter()
                        .map(|&i| raw[i].as_str())
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .map(describe)
                        .collect(),
                );
            } else {
                for (index, token) in m.names.into_iter().enumerate() {
                    if !names.contains(&index) {
                        continue;
                    }
                    let name = if m.signed {
                        token.trim_start_matches(['+', '-'])
                    } else {
                        token
                    };
                    let start = name.as_ptr() as usize - text.as_ptr() as usize + d.range.start;
                    report(start, start + name.len(), vec![describe(name)]);
                }
            }
        }
    }
}

/// Prove absence of executable bytes from the owned main-parse comment spans,
/// not by reparsing a candidate or walking the AST from a post callback.
fn comments_only(file: &SourceFile) -> bool {
    let mut end = 0;
    for range in file.comments() {
        if !file.bytes[end..range.start]
            .iter()
            .all(u8::is_ascii_whitespace)
        {
            return false;
        }
        let text = &file.bytes[range.clone()];
        // No magic, shebang, block comment, free prose, encoding or editor key.
        if !text.starts_with(b"#")
            || crate::cops::style::correction::contains_magic_comment_key(text)
        {
            return false;
        }
        let matches = syntax::matches(text);
        if matches.len() != 1 || matches[0].start != 0 {
            return false;
        }
        let tail = text[matches[0].end..].trim_ascii();
        if !tail.is_empty() && !tail.starts_with(b"--") {
            return false;
        }
        end = range.end;
    }
    file.bytes[end..].iter().all(u8::is_ascii_whitespace)
}

fn removal(
    file: &SourceFile,
    d: &DirectiveComment,
    names: &BTreeSet<usize>,
    entire: bool,
    known: &[&str],
) -> Option<Correction> {
    let text = &file.bytes[d.range.clone()];
    let m = syntax::matches(text).into_iter().next()?;
    if d.embedded
        || !d.standalone
        || !names.iter().all(|&i| {
            known
                .iter()
                .any(|c| hits(m.names[i].trim_start_matches(['+', '-']), c))
        })
    {
        return None;
    }
    if entire {
        return Some(Correction {
            range: file.line_start(d.line)..file.line_start(d.line + 1),
            replacement: String::new(),
        });
    }
    if m.signed {
        return None;
    }
    let tokens: Vec<_> = m
        .names
        .iter()
        .enumerate()
        .map(|(index, n)| {
            let start = n.as_ptr() as usize - text.as_ptr() as usize;
            (start, start + n.len(), names.contains(&index))
        })
        .collect();
    let mut edits = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if !tokens[i].2 {
            i += 1;
            continue;
        }
        let first = i;
        while i < tokens.len() && tokens[i].2 {
            i += 1;
        }
        let (start, end) = if first == 0 {
            (tokens[first].0, tokens.get(i)?.0)
        } else {
            let previous = tokens[first - 1].1;
            let comma = text[previous..tokens[first].0]
                .iter()
                .position(|b| *b == b',')?;
            (previous + comma, tokens[i - 1].1)
        };
        edits.push(Correction {
            range: d.range.start + start..d.range.start + end,
            replacement: String::new(),
        });
    }
    crate::cops::style::correction::combine(file, edits)
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's redundant_cop_disable_directive_spec.rb and
    //! comment_config_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{offense::Offense, pipeline::lint_str};

    fn injected(row: &serde_json::Value, names: &[serde_json::Value], fixes: bool) -> Vec<Offense> {
        let source = row["source"].as_str().unwrap();
        let file = SourceFile::new("supplied.rb".into(), source.as_bytes().to_vec());
        let parsed = crate::parser::parse(&file.bytes);
        assert_eq!(parsed.errors().count(), 0, "{row}");
        let disabled: Vec<_> = row["disabled"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        file.record_directive_disabled(
            &disabled.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>(),
        );
        let directives =
            crate::directives::Directives::parse_with_disabled(&parsed, &file, &disabled);
        let cops: Vec<_> = names
            .iter()
            .map(|n| {
                let name: &'static str = Box::leak(n.as_str().unwrap().to_owned().into_boxed_str());
                crate::cops::InvestigatedCop {
                    name,
                    configured_enabled: !disabled.contains(&name),
                    available: row["unavailable"].as_str() != Some(name),
                    investigated: row["unrun"].as_str() != Some(name),
                    completed: row["failed"].as_str() != Some(name)
                        && row["unrun"].as_str() != Some(name),
                }
            })
            .collect();
        let raw: Vec<_> = row["lifecycle"]["raw"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| Offense {
                cop: Box::leak(o["cop"].as_str().unwrap().to_owned().into_boxed_str()),
                line: o["line"].as_u64().unwrap() as u32,
                end_line: o["last"].as_u64().unwrap() as u32,
                col: 1,
                message: String::new(),
                correction: None,
                severity: crate::offense::Severity::Error,
            })
            .collect();
        let cx = PostInvestigation {
            raw_offenses: &raw,
            raw_global_offenses: &[],
            directives: &directives,
            cops: &cops,
            corrections_requested: fixes,
        };
        let mut out = Vec::new();
        crate::cops::run_post_investigation(&[&RedundantCopDisableDirective], &file, &cx, &mut out);
        out.retain(|o| directives.is_enabled_for_lines(o.cop, o.line, o.end_line));
        out.sort_by_key(|o| (o.line, o.col));
        out
    }

    #[test]
    fn pinned_lifecycle() {
        let dynamic = std::env::var("RUSTOCOP_REDUNDANT_DISABLE_REFERENCE").ok();
        let fixture: serde_json::Value = if let Some(path) = &dynamic {
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
        } else {
            serde_json::from_str(include_str!(
                "../../../tests/fixtures/redundant_cop_disable_directive.json"
            ))
            .unwrap()
        };
        let mut results = Vec::new();
        for row in fixture["rows"].as_array().unwrap() {
            let line = row["spec_line"].as_u64().unwrap();
            let found = injected(row, fixture["names"].as_array().unwrap(), false);
            results.push(serde_json::json!({ "positions": found.iter().map(|o| [o.line, o.col]).collect::<Vec<_>>(), "messages": found.iter().map(|o| &o.message).collect::<Vec<_>>() }));
            if dynamic.is_some() {
                continue;
            }
            // No public Preview mode; pending cops are enabled.
            let expected = match line {
                104 | 129 => vec![(1, 1)],
                _ => row["positions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| (p[0].as_u64().unwrap() as u32, p[1].as_u64().unwrap() as u32))
                    .collect(),
            };
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                expected,
                "{row}"
            );
            if !matches!(line, 104 | 129) {
                assert_eq!(
                    found.iter().map(|o| o.message.as_str()).collect::<Vec<_>>(),
                    row["messages"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|m| m.as_str().unwrap())
                        .collect::<Vec<_>>(),
                    "{row}"
                );
            }
        }
        if let Some(path) = dynamic {
            std::fs::write(format!("{path}.out"), serde_json::to_vec(&results).unwrap()).unwrap();
        }
    }
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/RedundantCopDisableDirective")
            .collect()
    }

    #[test]
    fn a_disable_for_a_cop_that_only_rustocop_has_opt_in_is_kept_while_the_cop_is_off() {
        for target in [
            "Metrics/AbcSize",
            "Metrics/AbcSize, Metrics/MethodLength",
            "Metrics",
            "Style/Documentation",
            "Style/FrozenStringLiteralComment",
        ] {
            let source = format!("def x # rubocop:disable {target}\nend\n");
            assert!(offenses(&source).is_empty(), "{target}");
        }
        // A cop that RuboCop has off as well is judged as before.
        let found = offenses("x = 1 # rubocop:disable Style/Copyright\n");
        assert_eq!(found.len(), 1, "{found:?}");
    }
    #[test]
    fn incomplete_universe_retains_directives_and_configured_final_disable() {
        let names = serde_json::json!([NAME, "Lint/Debugger", "Style/For"]);
        for target in ["Lint/Debugger", "Lint", "all"] {
            for field in ["unavailable", "failed", "unrun"] {
                let mut row = serde_json::json!({ "source": format!("# rubocop:disable {target}\n"), "disabled": [], "lifecycle": { "raw": [] } });
                row[field] = "Lint/Debugger".into();
                assert!(
                    injected(&row, names.as_array().unwrap(), true).is_empty(),
                    "{row}"
                );
            }
        }
        for (source, lines) in [
            ("# rubocop:disable Lint/Debugger\n", vec![1]),
            (
                "# rubocop:enable Lint/Debugger\n# rubocop:disable Lint/Debugger\n",
                vec![],
            ),
            (
                "# rubocop:enable Lint/Debugger\n# rubocop:disable Lint/Debugger\n# rubocop:enable Lint/Debugger\n",
                vec![2],
            ),
        ] {
            let row = serde_json::json!({ "source": source, "disabled": ["Lint/Debugger"], "unrun": "Lint/Debugger", "lifecycle": { "raw": [] } });
            assert_eq!(
                injected(&row, names.as_array().unwrap(), false)
                    .iter()
                    .map(|o| o.line)
                    .collect::<Vec<_>>(),
                lines,
                "{source}"
            );
        }
    }

    #[test]
    fn repeated_targets_preserve_occurrences_and_prefer_qualified_names() {
        for (targets, col, remaining) in [
            ("Lint/Debugger, Lint/Debugger", 19, "Lint/Debugger"),
            (
                "Lint/Debugger, Lint/Debugger, Lint/Debugger",
                19,
                "Lint/Debugger, Lint/Debugger",
            ),
            ("Debugger, Lint/Debugger", 29, "Debugger"),
            ("Lint/Debugger, Debugger", 19, "Debugger"),
            ("Debugger, Debugger", 19, "Debugger"),
        ] {
            let source = format!("# rubocop:disable {targets}\nbinding.pry\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
            assert_eq!(
                found[0].message,
                "Unnecessary disabling of `Lint/Debugger`."
            );
            assert!(found[0].correction.is_none());
            // The closed comments-only proof may delete just the selected
            // occurrence, not every identical spelling or the whole comment.
            let before = format!("# rubocop:disable {targets}\n");
            let row =
                serde_json::json!({ "source": before, "disabled": [], "lifecycle": { "raw": [] } });
            let names = serde_json::json!([NAME, "Lint/Debugger"]);
            let found = injected(&row, names.as_array().unwrap(), true);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), (1, col));
            assert_eq!(
                crate::corrections::apply(before.as_bytes(), &found)
                    .unwrap()
                    .bytes,
                format!("# rubocop:disable {remaining}\n").as_bytes()
            );
        }
        assert!(offenses("# rubocop:disable Lint/Debugger\nbinding.pry\n").is_empty());
    }

    #[test]
    fn duplicate_next_targets_respect_effective_statement_boundaries() {
        for prefix in ["rubocop", "rustocop"] {
            for eol in ["\n", "\r\n"] {
                for mode in ["disable-next", "todo-next"] {
                    for (targets, selected) in [
                        ("Lint/Debugger, Lint/Debugger", "Lint/Debugger"),
                        ("Debugger, Debugger", "Debugger"),
                        ("Debugger, Lint/Debugger", "Lint/Debugger"),
                        ("Lint/Debugger, Debugger", "Lint/Debugger"),
                    ] {
                        for body in [
                            "binding.pry\n",
                            "binding.pry(\n value\n)\n",
                            "puts <<~DOC\n#{binding.pry(value)}\nDOC\n",
                        ] {
                            let source = format!(
                                "# {prefix}:{mode} {targets}\n{body}# {prefix}:enable Lint/Debugger, Lint/Void\n"
                            ).replace('\n', eol);
                            let out = crate::pipeline::lint_str_with_corrections(
                                &[
                                    &RedundantCopDisableDirective,
                                    &super::super::Debugger,
                                    &super::super::Void,
                                ],
                                &source,
                            );
                            let found: Vec<_> = out.iter().filter(|o| o.cop == NAME).collect();
                            let single_line = body == "binding.pry\n";
                            assert_eq!(found.len(), usize::from(single_line), "{source}");
                            if single_line {
                                let col = source.find(selected).unwrap() as u32 + 1;
                                assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
                                assert_eq!(
                                    found[0].message,
                                    "Unnecessary disabling of `Lint/Debugger`."
                                );
                            }
                            assert!(
                                out.iter().all(|o| o.cop != "Lint/Debugger"),
                                "necessary suppression: {source}"
                            );
                            assert_eq!(
                                crate::corrections::apply(source.as_bytes(), &out)
                                    .unwrap()
                                    .bytes,
                                source.as_bytes()
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn detached_identical_next_targets_report_first_token_not_entire_comment() {
        for prefix in ["rubocop", "rustocop"] {
            for eol in ["\n", "\r\n"] {
                for mode in ["disable-next", "todo-next"] {
                    for (targets, selected, identical) in [
                        ("Lint/Debugger, Lint/Debugger", "Lint/Debugger", true),
                        ("Debugger, Debugger", "Debugger", true),
                        ("Debugger, Lint/Debugger", "Lint/Debugger", false),
                        ("Lint/Debugger, Debugger", "Lint/Debugger", false),
                    ] {
                        for gap in ["", "\n"] {
                            let source = format!(
                                "# {prefix}:{mode} {targets}\n{gap}puts 1\n# {prefix}:enable Lint/Debugger, Lint/Void\n"
                            ).replace('\n', eol);
                            let found = offenses(&source);
                            let col = if identical || gap.is_empty() {
                                source.find(selected).unwrap() as u32 + 1
                            } else {
                                1
                            };
                            assert_eq!(found.len(), 1, "{source}");
                            assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
                            assert_eq!(
                                found[0].message,
                                if identical || gap.is_empty() {
                                    "Unnecessary disabling of `Lint/Debugger`."
                                } else {
                                    "Unnecessary disabling of `Lint/Debugger`, `Lint/Debugger`."
                                }
                            );
                            assert!(found[0].correction.is_none());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn department_precedes_alias_for_next_directives() {
        let cops: &[&dyn Cop] = &[
            &RedundantCopDisableDirective,
            &super::super::Debugger,
            &super::super::Void,
        ];
        for prefix in ["rubocop", "rustocop"] {
            for eol in ["\n", "\r\n"] {
                for mode in ["disable-next", "todo-next"] {
                    for targets in ["Lint, Debugger", "Debugger, Lint"] {
                        let source = format!(
                            "# {prefix}:{mode} {targets}\nbinding.pry\n# {prefix}:enable Lint/Debugger, Lint/Void\n"
                        ).replace('\n', eol);
                        for fixes in [false, true] {
                            let out = if fixes {
                                crate::pipeline::lint_str_with_corrections(cops, &source)
                            } else {
                                crate::pipeline::lint_str_with(cops, &source)
                            };
                            assert!(out.is_empty(), "{source}: {out:?}");
                            // Also prove department semantics with a completed
                            // universe, not just conservative unrun-cop holds.
                            let row = serde_json::json!({
                                "source": source, "disabled": [],
                                "lifecycle": { "raw": [{ "cop": "Lint/Debugger", "line": 2, "last": 2 }] }
                            });
                            let names = serde_json::json!([NAME, "Lint/Debugger", "Lint/Void"]);
                            assert!(
                                injected(&row, names.as_array().unwrap(), fixes).is_empty(),
                                "{source}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn detached_signed_duplicates_report_unsigned_first_token() {
        for prefix in ["rubocop", "rustocop"] {
            for eol in ["\n", "\r\n"] {
                for targets in [
                    "-Lint/Debugger -Lint/Debugger",
                    "+Lint/Debugger +Lint/Debugger",
                    "-Lint/Debugger +Lint/Debugger",
                    "+Lint/Debugger -Lint/Debugger",
                ] {
                    for tail in ["", "\nputs 1\n"] {
                        let source =
                            format!("# {prefix}:next {targets}\n{tail}").replace('\n', eol);
                        let out = crate::pipeline::lint_str_with_corrections(
                            &[&RedundantCopDisableDirective, &super::super::Debugger],
                            &source,
                        );
                        let found: Vec<_> = out.iter().filter(|o| o.cop == NAME).collect();
                        assert_eq!(found.len(), 1, "{source}");
                        assert_eq!(
                            (found[0].line, found[0].col),
                            (1, source.find("Lint/Debugger").unwrap() as u32 + 1),
                            "{source}"
                        );
                        assert_eq!(
                            found[0].message,
                            "Unnecessary disabling of `Lint/Debugger`."
                        );
                        assert!(found[0].correction.is_none());
                    }
                }
            }
        }
    }

    #[test]
    fn unknown_duplicates_report_first_token_for_ordinary_and_next_modes() {
        for prefix in ["rubocop", "rustocop"] {
            for eol in ["\n", "\r\n"] {
                for mode in ["disable", "todo", "disable-next", "todo-next"] {
                    for tail in ["", "puts 1\n", "\nputs 1\n"] {
                        let source = format!("# {prefix}:{mode} UnknownCop, UnknownCop\n{tail}")
                            .replace('\n', eol);
                        let found = offenses(&source);
                        assert_eq!(found.len(), 1, "{source}");
                        assert_eq!(
                            (found[0].line, found[0].col),
                            (1, source.find("UnknownCop").unwrap() as u32 + 1),
                            "{source}"
                        );
                        assert_eq!(
                            found[0].message,
                            "Unnecessary disabling of `UnknownCop` (unknown cop)."
                        );
                        assert!(found[0].correction.is_none());
                    }
                }
            }
        }
    }

    #[test]
    fn dense_broad_selectors_preserve_incomplete_evidence() {
        for target in ["all", "Lint"] {
            let source = format!("# rubocop:disable {target}\n").repeat(1000);
            assert!(offenses(&source).is_empty());
            let names = serde_json::json!([NAME, "Lint/Void", "Lint/Debugger"]);
            let row =
                serde_json::json!({ "source": source, "disabled": [], "lifecycle": { "raw": [] } });
            let found = injected(&row, names.as_array().unwrap(), false);
            assert_eq!(found.len(), if target == "all" { 1 } else { 1000 });
            // The same selector may have distinct effective ranges after a
            // specific enable. Do not cache one range's absence for another.
            let row = serde_json::json!({
                "source": format!("# rubocop:disable {target}\n# rubocop:enable Lint/Debugger\nbinding.pry\n"),
                "disabled": [], "lifecycle": { "raw": [{ "cop": "Lint/Debugger", "line": 3, "last": 3 }] }
            });
            let found = injected(&row, names.as_array().unwrap(), false);
            assert_eq!(found.len(), 1, "{row}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
        }
    }

    #[test]
    fn dense_unknown_targets_and_deep_scopes_are_bounded() {
        let source = format!(
            "# rubocop:disable {}\n",
            (0..3000)
                .map(|i| format!("Unknown/Cop{i}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert_eq!(offenses(&source).len(), 1);
        let source = format!(
            "{}# rubocop:disable Lint/Debugger\n{}",
            "# rubocop:push\n".repeat(3000),
            "# rubocop:pop\n".repeat(3000)
        );
        assert_eq!(offenses(&source).len(), 1);
    }

    #[test]
    fn canonical_comment_only_corrections_and_all_upstream_holds() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_cop_disable_directive.json"
        ))
        .unwrap();
        let mut corrections = 0;
        let mut holds = 0;
        for row in fixture["rows"].as_array().unwrap() {
            let source = row["source"].as_str().unwrap();
            let out = injected(row, fixture["names"].as_array().unwrap(), true);
            let applied = crate::corrections::apply(source.as_bytes(), &out).unwrap();
            if row["lifecycle"]["hold"] == true {
                holds += 1;
                assert_eq!(applied.bytes, source.as_bytes(), "{row}");
            }
            if let Some(after) = row["lifecycle"]["correction"].as_str() {
                corrections += 1;
                let canonical = matches!(
                    row["spec_line"].as_u64().unwrap(),
                    34 | 43 | 63 | 159 | 168 | 179 | 266 | 287 | 312 | 334 | 571
                );
                assert_eq!(
                    out.iter().any(|o| o.correction.is_some()),
                    canonical,
                    "{row}"
                );
                if canonical {
                    assert_eq!(applied.bytes, after.as_bytes(), "{row}");
                } else {
                    assert_eq!(applied.bytes, source.as_bytes(), "{row}");
                }
            }
        }
        assert_eq!((corrections, holds), (39, 8));
        for (before, after) in [
            ("# rubocop:disable Lint/Debugger\n", ""),
            ("# rustocop:todo Lint/Debugger -- obsolete\r\n", ""),
            ("# rubocop:next -Lint/Debugger +Lint/Void\n", ""),
            (
                "# rubocop:disable Lint/Debugger\n# rubocop:enable Lint/Debugger\n",
                "# rubocop:enable Lint/Debugger\n",
            ),
        ] {
            let row =
                serde_json::json!({ "source": before, "disabled": [], "lifecycle": { "raw": [] } });
            let out = injected(&row, fixture["names"].as_array().unwrap(), true);
            assert_eq!(
                crate::corrections::apply(before.as_bytes(), &out)
                    .unwrap()
                    .bytes,
                after.as_bytes()
            );
            assert!(out.iter().any(|o| o.correction.is_some()));
        }
    }

    #[test]
    fn ordinary_runtime_source_and_encoding_edits_are_withheld() {
        for source in [
            "# rubocop:disable Lint/Debugger\nputs 1\n",
            "# rubocop:disable Lint/Debugger\np __LINE__\n",
            "# rubocop:disable Lint/Debugger\np DATA.read\n__END__\ndata\n",
            "# rubocop:disable Lint/Debugger\nFile.read(__FILE__)\n",
            "# rubocop:disable Lint/Debugger\nRubyVM::InstructionSequence.compile_file(__FILE__)\n",
            "# rubocop:disable Lint/Debugger\np ObjectSpace.count_objects\n",
            "# rubocop:disable Lint/Debugger\np binding.local_variables\n",
            "# rubocop:disable Lint/Debugger\nputs <<~H\nhi\nH\n",
            "\u{feff}# rubocop:disable Lint/Debugger\n",
            "#!/usr/bin/env ruby\n# rubocop:disable Lint/Debugger\n",
            "# coding: ascii\n# rubocop:disable Lint/Debugger\n",
            "# rubocop:disable Lint/Debugger -- coding: ascii\n",
            "# rubocop:disable UnknownCop\n",
        ] {
            let out = crate::pipeline::lint_str_with_corrections(
                &[&RedundantCopDisableDirective, &super::super::Debugger],
                source,
            );
            let found: Vec<_> = out.iter().filter(|o| o.cop == NAME).collect();
            assert!(!found.is_empty(), "{source}");
            assert!(
                found.iter().all(|o| o.correction.is_none()),
                "{source}: {found:?}"
            );
        }
    }

    #[test]
    fn real_next_signed_scopes_and_multiline_highlights() {
        for prefix in ["rubocop", "rustocop"] {
            for (body, lines) in [
                (
                    "# P:disable-next Lint/Debugger\nbinding.pry(value)\n",
                    vec![],
                ),
                ("# P:disable-next Lint/Debugger\n\nbinding.pry\n", vec![1]),
                ("puts 1 # P:disable-next Lint/Debugger\n", vec![1]),
                ("# P:next -Lint/Debugger\nputs 1\n", vec![]),
                ("# P:next -Lint/Debugger +Lint/Void\n", vec![1]),
                ("# P:enable-next Lint/Debugger\n", vec![1]),
                ("# P:push -Lint/Debugger\nputs 1\n# P:pop\n", vec![]),
                (
                    "binding.pry(\n value\n) # P:disable Lint/Debugger\n",
                    vec![],
                ),
                (
                    "binding.pry(\n value\n)\nputs 1 # P:disable Lint/Debugger\n",
                    vec![4],
                ),
                (
                    "# P:disable Lint/Debugger\nbinding.pry\n# P:disable Lint/Debugger\nbinding.pry\n",
                    vec![3],
                ),
                (
                    "# P:disable-next Lint/Debugger\nputs <<~DOC\n#{binding.pry(value)}\nDOC\n",
                    vec![],
                ),
            ] {
                let source = body.replace("P:", &format!("{prefix}:"));
                assert_eq!(
                    offenses(&source).iter().map(|o| o.line).collect::<Vec<_>>(),
                    lines,
                    "{source}"
                );
            }
        }
    }

    #[test]
    fn unknown_next_uses_statement_self_suppression_not_comment_line() {
        assert!(offenses("# rubocop:disable-next Lint/RedundantCopDisableDirective\n# rubocop:disable-next UnknownCop\nfoo\n").is_empty());
    }

    #[test]
    fn no_offense_lexical_lookalikes() {
        for source in [
            "",
            "# # rubocop:disable Lint/Debugger\n",
            "value = '# rubocop:disable Lint/Debugger'\n",
            "value = <<~DOC\n# rubocop:disable Lint/Debugger\nDOC\n",
            "__END__\n# rubocop:disable Lint/Debugger\n",
            "=begin\n# rubocop:disable Lint/Debugger\n=end\n",
            "# rubocop:disable Lint/Syntax\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn actual_empty_and_suppressed_investigations() {
        let found =
            offenses("# rubocop:disable Lint/Debugger\nputs 1\n# rubocop:enable Lint/Debugger\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 1));
        assert_eq!(
            found[0].message,
            "Unnecessary disabling of `Lint/Debugger`."
        );
        assert!(offenses("# rubocop:disable Lint/Debugger\nbinding.pry\n").is_empty());
    }
    #[test]
    fn incomplete_all_is_preserved_and_explicit_self_suppresses() {
        // The live universe includes version-inactive/unrun detectors. Absence
        // of their findings must not justify removing an all/Lint directive.
        assert!(offenses("# rubocop:disable all\n").is_empty());
        assert!(offenses("# rubocop:disable Lint\n").is_empty());
        assert!(
            offenses(
                "# rubocop:disable Lint/RedundantCopDisableDirective\n# rubocop:disable all\n"
            )
            .is_empty()
        );
    }
}
