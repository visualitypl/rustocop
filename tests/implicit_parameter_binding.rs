//! structural binding equivalence and actual peer composition.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const IT: &str = "Style/ItBlockParameter";
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
fn successful_fixes_preserve_binding_edges_dispatch_control_flow_and_literal_values_at_34() {
    use ruby_prism::Node;
    use rustocop::cops::walk::{NodeVisitor, walk};
    #[derive(Default)]
    struct Shape(Vec<(String, usize, usize, Vec<u8>)>);
    impl<'pr> NodeVisitor<'pr> for Shape {
        fn enter(&mut self, n: Node<'pr>) {
            let mut kind = format!("{:?}", std::mem::discriminant(&n));
            let mut payload = Vec::new();
            if let Some(p) = n.as_numbered_parameters_node() {
                kind = "implicit-params".into();
                payload.push(p.maximum());
            } else if n.as_it_parameters_node().is_some() {
                kind = "implicit-params".into();
                payload.push(1);
            } else if n.as_it_local_variable_read_node().is_some()
                || n.as_local_variable_read_node()
                    .is_some_and(|r| r.name().as_slice() == b"_1")
            {
                kind = "implicit-read".into();
            } else if let Some(r) = n.as_local_variable_read_node() {
                payload.extend(r.name().as_slice());
                payload.extend(r.depth().to_le_bytes());
            } else if let Some(c) = n.as_call_node() {
                payload.extend(c.name().as_slice());
            } else if let Some(s) = n.as_string_node() {
                payload.extend(s.unescaped());
            } else if let Some(s) = n.as_symbol_node() {
                payload.extend(s.unescaped());
            }
            // Implicit parameter declarations have synthetic source locations.
            let (start, end) = if kind == "implicit-params" {
                (0, 0)
            } else {
                (n.location().start_offset(), n.location().end_offset())
            };
            self.0.push((kind, start, end, payload));
        }
    }
    let d = project();
    // `DATA` with a data section is a listed observer that holds
    // every edit.
    let listed = "foo { [\"café\", _1, __LINE__] }; p DATA.pos\n__END__\nraw\n";
    fs::write(d.path().join("x.rb"), listed).unwrap();
    cli(d.path(), &["--only", IT, "--fix", "x.rb"], 1);
    assert_eq!(fs::read(d.path().join("x.rb")).unwrap(), listed.as_bytes());
    for source in [
        "def f(v); xs.map { _1 + v }; end\n",
        "proc { use(_1); inner { |it| use(it) }; next _1 }\n",
        "-> { use(_1); inner { |(x,y)| use(x,y) }; _1.foo { |x=1| x } }\n",
        "foo { use(_1); def (build(<<~TXT)).tag(x=1); end }\n#{_1} #{__LINE__} _1 literal\nTXT\n",
        "foo { [\"café\", _1, __LINE__] }; p :data\n__END__\nraw\n",
        "f = -> { return _1 if _1; next _1 }; g = proc { break _1 }\n",
    ] {
        fs::write(d.path().join("x.rb"), source).unwrap();
        cli(d.path(), &["--only", IT, "--fix", "x.rb"], 0);
        let after = fs::read(d.path().join("x.rb")).unwrap();
        assert_ne!(after, source.as_bytes());
        let before = ruby_prism::parse(source.as_bytes());
        let after = ruby_prism::parse(&after);
        assert_eq!(before.errors().count(), 0);
        assert_eq!(after.errors().count(), 0);
        let mut a = Shape::default();
        let mut b = Shape::default();
        walk(&mut a, &before.node());
        walk(&mut b, &after.node());
        assert_eq!(a.0, b.0, "{source}");
        // VariableForce confirms that each implicit input still owns the same
        // references in its same structural scope, not a captured outer local.
        let bindings = |root: Node<'_>| {
            let a = rustocop::cops::variables::ScopeAnalysis::of(&root.as_program_node().unwrap());
            let mut refs: Vec<_> = a
                .scopes_in_exit_order()
                .flat_map(|s| a.scope(s).variables())
                .map(|v| a.variable(v))
                .filter(|v| v.is_parameter() && [b"_1".as_slice(), b"it"].contains(&v.name))
                .map(|v| {
                    (
                        a.scope(v.scope).node.location().start_offset(),
                        v.references
                            .iter()
                            .map(|&r| a.reference(r).range.clone())
                            .collect::<Vec<_>>(),
                    )
                })
                .collect();
            refs.sort_by_key(|r| r.0);
            refs
        };
        assert_eq!(bindings(before.node()), bindings(after.node()));
    }
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
