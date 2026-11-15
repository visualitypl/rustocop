//! structural binding equivalence and actual peer composition.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const ALL: &str = "Style/ItAssignment,Style/ItBlockParameter,Style/NumberedParameters";
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
    out
}
#[test]
fn actual_lambda_stabby_numbered_and_default_style_layout_composition_is_bounded() {
    let d = project();
    let source = "a = lambda { _1 + 1 }\nb = ->x { x }\nc = -> do\n  _1 + 2\nend\nfoo do use(_1) end\np [a.call(2), b.call(3), c.call(4)]\n";
    for selection in [
        Some(
            "Style/ItAssignment,Style/ItBlockParameter,Style/NumberedParameters,Style/Lambda,Style/StabbyLambdaParentheses",
        ),
        Some("Style"),
        Some("Style,Layout"),
        None,
    ] {
        fs::write(d.path().join("x.rb"), source).unwrap();
        let mut seen = vec![source.as_bytes().to_vec()];
        let mut stable = false;
        for _ in 0..6 {
            let mut args = vec!["--fix", "x.rb"];
            if let Some(cops) = selection {
                args.extend(["--only", cops]);
            }
            let out = cli(d.path(), &args, 1);
            assert!(!String::from_utf8_lossy(&out.stderr).contains("error"));
            let next = fs::read(d.path().join("x.rb")).unwrap();
            if next == *seen.last().unwrap() {
                stable = true;
                break;
            }
            assert!(!seen.contains(&next), "cycle: {selection:?}");
            seen.push(next);
        }
        assert!(stable, "{selection:?}");
        let final_source = String::from_utf8(seen.last().unwrap().clone()).unwrap();
        assert!(
            final_source.contains("-> { it + 1 }"),
            "{selection:?}: {final_source}"
        );
        assert!(final_source.contains("->(x) { x }"), "{final_source}");
        assert!(
            final_source.contains("lambda do\n  it + 2\nend"),
            "{final_source}"
        );
        assert!(!final_source.contains("_1"));
        // It stays enabled: the unavoidable default-style multiline finding
        // remains visible rather than being silently disabled to stabilize.
        let out = cli(d.path(), &["--only", ALL, "x.rb"], 1);
        assert!(
            String::from_utf8_lossy(&out.stdout)
                .contains("Avoid using `it` block parameter for multi-line blocks.")
        );
    }
}
