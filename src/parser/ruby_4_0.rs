//! The Ruby 4.0 grammar, checked against a real Ruby 4.0 when `RUSTOCOP_RUBY40`
//! names its executable. The inputs are compiled, never run.

use std::{
    io::Write,
    process::{Command, Stdio},
};

use super::{english_not, flow_defaults, parse_as};
use crate::cops::policy::RubyVersion;

const RUBY_4_0: RubyVersion = RubyVersion(4, 0);

const PINNED: &[(&str, bool)] = &[
    ("foo\n  && bar\n", true),
    ("foo\n  || bar\n", true),
    ("foo\r\n  && bar\r\n", true),
    ("foo\n  # note\n  && bar\n", true),
    ("foo\n  and bar\n", true),
    ("foo\n  or bar\n", true),
    ("private def foo = bar baz\n", true),
    ("private def foo(x) = bar :baz, x\n", true),
    ("qux 1, def foo(x) = bar :baz, x\n", false),
    ("def foo = raise Error, self\n", true),
    ("case; when not false; 1; end\n", false),
    ("[not false]\n", false),
    ("items.map { it }\n", true),
    ("def foo(*) = bar(*)\n", true),
    ("def foo(&) = bar(&)\n", true),
    ("x = 1 in Integer\n", true),
];

const VERDICTS: &str = r"
$VERBOSE = nil
abort 'expected Ruby 4.0' unless RUBY_VERSION.start_with?('4.0.')
STDIN.each_line do |line|
  source = [line.chomp].pack('H*').force_encoding('UTF-8')
  valid = begin
    RubyVM::InstructionSequence.compile(source)
    true
  rescue SyntaxError
    false
  end
  print valid ? '1' : '0'
end
";

fn accepts(source: &[u8]) -> bool {
    let parsed = parse_as(source, RUBY_4_0);
    parsed.errors().next().is_none() && parsed.depth_error().is_none()
}

fn sources() -> Vec<Vec<u8>> {
    PINNED
        .iter()
        .map(|(source, _)| source.as_bytes().to_vec())
        .chain(english_not::cases().map(|(source, _)| source.into_bytes()))
        .chain(
            flow_defaults::witnesses::cases()
                .into_iter()
                .map(|case| case.source),
        )
        .collect()
}

#[test]
fn pinned_ruby_4_0_syntax_is_accepted_or_rejected() {
    for &(source, valid) in PINNED {
        assert_eq!(accepts(source.as_bytes()), valid, "{source:?}");
    }
}

#[test]
fn the_ruby_4_0_grammar_agrees_with_native_ruby_4_0() {
    let Some(ruby) = std::env::var_os("RUSTOCOP_RUBY40") else {
        eprintln!("SKIP native Ruby 4.0 witnesses: set RUSTOCOP_RUBY40");
        return;
    };
    let sources = sources();
    let mut child = Command::new(&ruby)
        .env_remove("RUBYOPT")
        .env_remove("RUBYLIB")
        .args(["--parser=prism", "-e", VERDICTS])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("RUSTOCOP_RUBY40 must be executable");
    let mut input = Vec::new();
    for source in &sources {
        for byte in source {
            write!(input, "{byte:02x}").unwrap();
        }
        input.push(b'\n');
    }
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout.len(), sources.len(), "{output:?}");
    let disagreements: Vec<_> = sources
        .iter()
        .zip(&output.stdout)
        .filter(|&(source, &verdict)| accepts(source) != (verdict == b'1'))
        .map(|(source, &verdict)| {
            format!(
                "Ruby 4.0 {} {:?}",
                if verdict == b'1' {
                    "accepts"
                } else {
                    "rejects"
                },
                String::from_utf8_lossy(source)
            )
        })
        .collect();
    assert!(disagreements.is_empty(), "{}", disagreements.join("\n"));
}
