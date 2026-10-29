//! Controlled Ruby 3.4.1 Prism witnesses, not parse.y witnesses: the two native
//! parsers disagree on English `not` in argument/value positions. Keep these
//! tests tied to the default Ruby 3.4 parser, explicitly selected with --parser.

use super::parse;

// Context, accepts ordinary command, accepts bare not, accepts not ... and ... .
// A `not` operand may itself contain a command even when the context forbids a
// direct command. Enabling commands at those call sites would be too broad.
const CONTEXTS: &[(&str, bool, bool, bool)] = &[
    ("<operand>", true, true, true),
    ("if <operand>; :yes; end", true, true, true),
    ("unless <operand>; :yes; end", true, true, true),
    ("while <operand>; break; end", true, true, true),
    ("until <operand>; break; end", true, true, true),
    (":yes if <operand>", true, true, true),
    ("case; when <operand>; :yes; end", false, true, false),
    (
        "case :x; when false, <operand>; :yes; end",
        false,
        true,
        false,
    ),
    ("case; when *<operand>; :yes; end", false, true, false),
    ("case; when [<operand>]; :yes; end", false, true, false),
    ("[<operand>]", false, true, false),
    ("[false, <operand>]", false, true, false),
    ("[*<operand>]", false, true, false),
    ("{k: <operand>}", false, true, false),
    ("{<operand> => :v}", false, false, false),
    ("{:k => <operand>}", false, true, false),
    ("{**<operand>}", false, true, false),
    ("probe(<operand>)", true, true, false),
    ("probe <operand>", true, true, true),
    ("probe(false, <operand>)", false, true, false),
    ("probe(k: <operand>)", false, true, false),
    ("probe(*<operand>)", false, true, false),
    ("probe(**<operand>)", false, true, false),
    ("probe(&<operand>)", false, true, false),
    ("obj[<operand>]", true, true, false),
    ("obj[*<operand>]", false, true, false),
    ("obj.value = <operand>", true, true, true),
    ("value = <operand>", true, true, true),
    ("value += <operand>", true, true, true),
    ("value ||= <operand>", true, true, true),
    ("def m(x = <operand>); end", false, true, false),
    ("def m(x: <operand>); end", false, true, false),
    ("probe { |x = <operand>| }", false, false, false),
    ("->(x = <operand>) {}", false, true, false),
    ("false ? <operand> : true", false, true, false),
    ("false ? true : <operand>", false, true, true),
    ("false && <operand>", false, true, true),
    ("false || <operand>", false, true, true),
    ("false and <operand>", true, true, true),
    ("false or <operand>", true, true, true),
    ("false + <operand>", false, true, true),
    ("!<operand>", true, true, true),
    ("(<operand>)", true, true, true),
    ("defined?(<operand>)", true, true, true),
    ("defined? <operand>", false, true, true),
    ("def m; return <operand>; end", true, true, false),
    ("loop { break <operand>; }", true, true, false),
    ("loop { next <operand>; }", true, true, false),
    ("def m; yield <operand>; end", true, true, true),
    ("def m; super(<operand>); end", true, true, false),
    ("def m; super <operand>; end", true, true, true),
    ("begin; rescue <operand>; end", false, true, false),
    ("for x in <operand>; end", true, true, true),
    ("\"#{<operand>}\"", true, true, true),
    ("case 1; in <operand> then :yes; end", false, false, false),
    ("case 1; in x if <operand>; :yes; end", true, true, true),
];

const EDGE_CASES: &[(&str, bool)] = &[
    ("if not false\n :yes\nend", true),
    ("case\nwhen [not false]\n :yes\nend", true),
    ("[not # keep\n false]", true),
    ("[not\n false]", true),
    ("[not not false]", true),
    ("[not(false)]", true),
    ("[not()]", true),
    ("[not false, true]", true),
    ("probe(false, not false, true)", true),
    ("case; when not obj.probe false; :yes; end", true),
    ("case; when obj.probe false; :yes; end", false),
    ("case; when not false or true; :yes; end", false),
    ("[not false or true]", false),
    ("[not]", false),
    ("[not; false]", false),
];

fn cases() -> impl Iterator<Item = (String, bool)> {
    CONTEXTS
        .iter()
        .flat_map(|&(context, command, not, chain)| {
            [
                ("probe false", command),
                ("not false", not),
                ("not probe false", not),
                ("not (false)", not),
                ("not false and true", chain),
            ]
            .map(|(operand, valid)| (context.replace("<operand>", operand), valid))
        })
        .chain(
            EDGE_CASES
                .iter()
                .map(|&(source, valid)| (source.into(), valid)),
        )
}

#[test]
fn newly_accepted_not_keeps_binding_and_original_byte_locations() {
    let source = b"case\nwhen [not false && true]\n :yes\nend";
    let parsed = parse(source);
    assert_eq!(parsed.errors().count(), 0);
    let case = parsed
        .node()
        .as_program_node()
        .unwrap()
        .statements()
        .body()
        .first()
        .unwrap()
        .as_case_node()
        .unwrap();
    let array = case
        .conditions()
        .first()
        .unwrap()
        .as_when_node()
        .unwrap()
        .conditions()
        .first()
        .unwrap()
        .as_array_node()
        .unwrap();
    let not = array.elements().first().unwrap().as_call_node().unwrap();
    assert_eq!(not.name().as_slice(), b"!");
    assert_eq!(not.message_loc().unwrap().as_slice(), b"not");
    assert_eq!(not.message_loc().unwrap().start_offset(), 11);
    assert_eq!(not.location().as_slice(), b"not false && true");
    let receiver = not.receiver().unwrap().as_and_node().unwrap();
    assert!(receiver.left().as_false_node().is_some());
    assert!(receiver.right().as_true_node().is_some());
}

#[test]
fn compatibility_does_not_change_latest_default_api() {
    for source in [
        "case; when [not false]; :yes; end",
        "[not false]",
        "{k: not false}",
        "probe(false, not false)",
        "def m(x = not false); end",
        "true && not false",
    ] {
        assert_eq!(parse(source.as_bytes()).errors().count(), 0, "{source}");
        assert!(
            ruby_prism::parse(source.as_bytes()).errors().count() > 0,
            "{source}"
        );
    }
}

#[test]
fn fixed_grammar_matches_ruby34_english_not_contexts() {
    for (source, valid) in cases() {
        let parsed = parse(source.as_bytes());
        let errors: Vec<_> = parsed.errors().map(|e| e.message().into_owned()).collect();
        assert_eq!(errors.is_empty(), valid, "{source}: {errors:?}");
    }
}

#[test]
fn native_ruby34_prism_witnesses() {
    use std::process::Command;

    let Some(ruby) = std::env::var_os("RUSTOCOP_RUBY34") else {
        eprintln!("SKIP native Ruby 3.4 witnesses: set RUSTOCOP_RUBY34");
        return;
    };
    let run = |args: &[&str]| {
        Command::new(&ruby)
            .env_remove("RUBYOPT")
            .env_remove("RUBYLIB")
            .args(args)
            .output()
            .expect("RUSTOCOP_RUBY34 must be executable")
    };
    let version = run(&["--parser=prism", "-e", "print RUBY_VERSION"]);
    assert!(version.status.success());
    assert!(version.stdout.starts_with(b"3.4."), "{version:?}");
    for (source, valid) in cases() {
        let output = run(&["--parser=prism", "-c", "-e", &source]);
        assert_eq!(output.status.success(), valid, "{source}: {output:?}");
    }
    // Distinguish not(false && false) from (!false && false), exercise a
    // command inside not, and check ternary/default-argument nesting.
    let source = "def probe(x); $seen << x; x; end; $seen=[]; p [[not probe false], {v: not false && false}, (false ? true : not true), (->(x = not false) {x}).call, $seen]";
    assert_eq!(parse(source.as_bytes()).errors().count(), 0);
    let output = run(&["--parser=prism", "-e", source]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        output.stdout,
        b"[[true], {v: true}, false, true, [false]]\n"
    );
}
