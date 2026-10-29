//! Emission audit: no unsafe corrections even before the CLI safety filter.
use super::*;
use serde_json::Value;

fn evidence() -> Vec<Value> {
    serde_json::from_str(include_str!("../../../tests/fixtures/style_evidence.json")).unwrap()
}
fn canonical() -> Vec<Value> {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/style_corrections.json"
    ))
    .unwrap()
}
fn collect_selected(cops: &[&dyn Cop], source: &[u8]) -> Vec<Offense> {
    let file = SourceFile::new("evidence.rb".into(), source.to_vec());
    let parsed = crate::parser::parse(&file.bytes);
    assert_eq!(
        parsed.errors().count(),
        0,
        "{}",
        String::from_utf8_lossy(source)
    );
    file.record_comments(&parsed);
    file.record_continuations(&parsed.continuations);
    file.record_data_start(&parsed);
    file.record_warnings(&parsed);
    file.record_encoding(&parsed);
    let mut out = Vec::new();
    assert!(run_cops(cops, &parsed, &file, &mut out).is_empty());
    out
}
#[test]
fn all_124_unsafe_or_noncorrectable_style_rules_never_emit_edits_on_detection_evidence() {
    let registry = Registry::new();
    let mut checked = 0;
    for record in evidence() {
        let cop = registry
            .cops()
            .iter()
            .find(|c| c.name() == record["name"].as_str().unwrap())
            .unwrap();
        if cop.safe_fixes() {
            continue;
        }
        checked += 1;
        for case in record["cases"].as_array().unwrap() {
            let found =
                collect_selected(&[cop.as_ref()], case["source"].as_str().unwrap().as_bytes());
            // ReverseFind's matcher is directly audited here; real selection's
            // 3.4 version gate is independently exercised by style_coverage.
            assert_eq!(
                found.len(),
                case["expected"].as_array().unwrap().len(),
                "{}",
                cop.name()
            );
            assert!(
                found.iter().all(|o| o.correction.is_none()),
                "{}",
                cop.name()
            );
        }
    }
    assert_eq!(checked, 124);
}
#[test]
fn selected_incompatible_companions_withhold_edits_not_diagnostics_in_either_order() {
    let registry = Registry::new();
    let canonical = canonical();
    let mut checked = 0;
    for cop in registry
        .cops()
        .iter()
        .filter(|c| c.name().starts_with("Style/"))
    {
        for name in cop.autocorrect_incompatible_with() {
            let Some(peer) = registry.cops().iter().find(|p| p.name() == *name) else {
                continue;
            };
            let case = canonical.iter().find(|c| c["name"] == cop.name()).unwrap();
            let source = case["before"].as_str().unwrap().as_bytes();
            let original = collect_selected(&[cop.as_ref()], source);
            assert!(
                original.iter().any(|o| o.correction.is_some()),
                "{}",
                cop.name()
            );
            for selected in [[cop.as_ref(), peer.as_ref()], [peer.as_ref(), cop.as_ref()]] {
                let combined = collect_selected(&selected, source);
                let own: Vec<_> = combined.iter().filter(|o| o.cop == cop.name()).collect();
                assert_eq!(
                    own.iter()
                        .map(|o| (o.line, o.col, o.message.as_str()))
                        .collect::<Vec<_>>(),
                    original
                        .iter()
                        .map(|o| (o.line, o.col, o.message.as_str()))
                        .collect::<Vec<_>>(),
                    "{} / {name}",
                    cop.name()
                );
                assert!(
                    own.iter().all(|o| o.correction.is_none()),
                    "{} / {name}",
                    cop.name()
                );
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 15);
}
#[test]
fn every_emitting_safe_style_capability_honors_source_code_object_and_heap_barriers() {
    let registry = Registry::new();
    let mut checked = 0;
    for case in canonical() {
        if case.get("withheld").is_some() {
            continue;
        }
        let name = case["name"].as_str().unwrap();
        let cop = registry.cops().iter().find(|c| c.name() == name).unwrap();
        let source = case["before"].as_str().unwrap();
        let original = collect_selected(&[cop.as_ref()], source.as_bytes());
        assert!(original.iter().any(|o| o.correction.is_some()), "{name}");
        checked += 1;
        for observer in [
            "p File.binread(__FILE__)",
            "p RubyVM::InstructionSequence.of(-> {})",
            "p ObjectSpace.count_objects",
        ] {
            let source = format!("{source}\n{observer}\n");
            let guarded = collect_selected(&[cop.as_ref()], source.as_bytes());
            assert!(guarded.len() >= original.len(), "{name}: {source}");
            assert!(
                guarded.iter().all(|o| o.correction.is_none()),
                "{name}: {source}"
            );
        }
    }
    assert_eq!(checked, 173);
}
