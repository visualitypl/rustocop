//! Examples ported from RuboCop's `spec/rubocop/cop/layout/heredoc_indentation_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). Every
//! example, including the `squish`, `AllowHeredoc: false`, disabled
//! `Layout/LineLength` and Ruby 2.2 contexts, is replayed at the fixed
//! defaults; findings, messages and single-pass corrected bytes match the
//! fixed replay recorded in the pinned RuboCop reference run.
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    offense::Severity,
    pipeline::lint_str,
};

const COP: &str = "Layout/HeredocIndentation";

fn offenses(source: &str) -> Vec<Offense> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source}"
    );
    lint_str(source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .collect()
}

fn positions(source: &str) -> Vec<(u32, u32, String)> {
    offenses(source)
        .into_iter()
        .map(|o| (o.line, o.col, o.message))
        .collect()
}

/// The correction a source would get, without asserting it exists.
fn corrected(source: &str) -> Option<String> {
    let offenses = correction_offenses(&HeredocIndentation, source);
    offenses.iter().any(|o| o.correction.is_some()).then(|| {
        let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
        String::from_utf8(applied.bytes).unwrap()
    })
}

/// Every pinned example without an offense at the fixed defaults.
const ACCEPTED: &[&str] = &[
    // 10 when Ruby >= 2.3 behaves like all heredoc type quoted by  does not register an offense whe
    "          def foo\n            <<-RUBY2\n            something\n            RUBY2\n          end\n",
    // 20 when Ruby >= 2.3 behaves like all heredoc type quoted by  accepts for indented, but with `
    "def foo\n  <<-RUBY2\n    something\n  RUBY2\nend\n",
    // 30 when Ruby >= 2.3 behaves like all heredoc type quoted by  accepts for not indented but wit
    "def foo\n  <<RUBY2\n  something\nRUBY2\nend\n",
    // 40 when Ruby >= 2.3 behaves like all heredoc type quoted by  accepts for indented, but withou
    "def foo\n  <<RUBY2\n    something\nRUBY2\nend\n",
    // 50 when Ruby >= 2.3 behaves like all heredoc type quoted by  accepts for an empty line
    "<<-RUBY2\n\nRUBY2\n",
    // 168 when Ruby >= 2.3 behaves like all heredoc type quoted by  accepts for indented, with `~`
    "<<~RUBY2\n  something\nRUBY2\n",
    // 176 when Ruby >= 2.3 behaves like all heredoc type quoted by  accepts for include empty lines
    "<<~MSG\n\n  foo\n\n    bar\n\nMSG\n",
    // 290 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "          def foo\n            <<-RUBY2.squish\n            something\n            RUBY2\n          end\n",
    // 309 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "          def foo\n            <<-RUBY2.squish\n              something\n            RUBY2\n          end\n",
    // 328 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "          def foo\n            <<-RUBY2.squish\n                something\n            RUBY2\n          end\n",
    // 347 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "          def foo\n            <<-RUBY2.squish!\n            something\n            RUBY2\n          end\n",
    // 372 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "def foo\n  <<-RUBY2.squish\n  something\n  RUBY2\nend\n",
    // 10 when Ruby >= 2.3 behaves like all heredoc type quoted by ' does not register an offense wh
    "          def foo\n            <<-'RUBY2'\n            something\n            RUBY2\n          end\n",
    // 20 when Ruby >= 2.3 behaves like all heredoc type quoted by ' accepts for indented, but with
    "def foo\n  <<-'RUBY2'\n    something\n  RUBY2\nend\n",
    // 30 when Ruby >= 2.3 behaves like all heredoc type quoted by ' accepts for not indented but wi
    "def foo\n  <<'RUBY2'\n  something\nRUBY2\nend\n",
    // 40 when Ruby >= 2.3 behaves like all heredoc type quoted by ' accepts for indented, but witho
    "def foo\n  <<'RUBY2'\n    something\nRUBY2\nend\n",
    // 50 when Ruby >= 2.3 behaves like all heredoc type quoted by ' accepts for an empty line
    "<<-'RUBY2'\n\nRUBY2\n",
    // 168 when Ruby >= 2.3 behaves like all heredoc type quoted by ' accepts for indented, with `~`
    "<<~'RUBY2'\n  something\nRUBY2\n",
    // 176 when Ruby >= 2.3 behaves like all heredoc type quoted by ' accepts for include empty lines
    "<<~'MSG'\n\n  foo\n\n    bar\n\nMSG\n",
    // 290 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "          def foo\n            <<-'RUBY2'.squish\n            something\n            RUBY2\n          end\n",
    // 309 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "          def foo\n            <<-'RUBY2'.squish\n              something\n            RUBY2\n          end\n",
    // 328 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "          def foo\n            <<-'RUBY2'.squish\n                something\n            RUBY2\n          end\n",
    // 347 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "          def foo\n            <<-'RUBY2'.squish!\n            something\n            RUBY2\n          end\n",
    // 372 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "def foo\n  <<-'RUBY2'.squish\n  something\n  RUBY2\nend\n",
    // 10 when Ruby >= 2.3 behaves like all heredoc type quoted by " does not register an offense wh
    "          def foo\n            <<-\"RUBY2\"\n            something\n            RUBY2\n          end\n",
    // 20 when Ruby >= 2.3 behaves like all heredoc type quoted by " accepts for indented, but with
    "def foo\n  <<-\"RUBY2\"\n    something\n  RUBY2\nend\n",
    // 30 when Ruby >= 2.3 behaves like all heredoc type quoted by " accepts for not indented but wi
    "def foo\n  <<\"RUBY2\"\n  something\nRUBY2\nend\n",
    // 40 when Ruby >= 2.3 behaves like all heredoc type quoted by " accepts for indented, but witho
    "def foo\n  <<\"RUBY2\"\n    something\nRUBY2\nend\n",
    // 50 when Ruby >= 2.3 behaves like all heredoc type quoted by " accepts for an empty line
    "<<-\"RUBY2\"\n\nRUBY2\n",
    // 168 when Ruby >= 2.3 behaves like all heredoc type quoted by " accepts for indented, with `~`
    "<<~\"RUBY2\"\n  something\nRUBY2\n",
    // 176 when Ruby >= 2.3 behaves like all heredoc type quoted by " accepts for include empty lines
    "<<~\"MSG\"\n\n  foo\n\n    bar\n\nMSG\n",
    // 290 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "          def foo\n            <<-\"RUBY2\".squish\n            something\n            RUBY2\n          end\n",
    // 309 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "          def foo\n            <<-\"RUBY2\".squish\n              something\n            RUBY2\n          end\n",
    // 328 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "          def foo\n            <<-\"RUBY2\".squish\n                something\n            RUBY2\n          end\n",
    // 347 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "          def foo\n            <<-\"RUBY2\".squish!\n            something\n            RUBY2\n          end\n",
    // 372 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "def foo\n  <<-\"RUBY2\".squish\n  something\n  RUBY2\nend\n",
    // 10 when Ruby >= 2.3 behaves like all heredoc type quoted by ` does not register an offense wh
    "          def foo\n            <<-`RUBY2`\n            something\n            RUBY2\n          end\n",
    // 20 when Ruby >= 2.3 behaves like all heredoc type quoted by ` accepts for indented, but with
    "def foo\n  <<-`RUBY2`\n    something\n  RUBY2\nend\n",
    // 30 when Ruby >= 2.3 behaves like all heredoc type quoted by ` accepts for not indented but wi
    "def foo\n  <<`RUBY2`\n  something\nRUBY2\nend\n",
    // 40 when Ruby >= 2.3 behaves like all heredoc type quoted by ` accepts for indented, but witho
    "def foo\n  <<`RUBY2`\n    something\nRUBY2\nend\n",
    // 50 when Ruby >= 2.3 behaves like all heredoc type quoted by ` accepts for an empty line
    "<<-`RUBY2`\n\nRUBY2\n",
    // 168 when Ruby >= 2.3 behaves like all heredoc type quoted by ` accepts for indented, with `~`
    "<<~`RUBY2`\n  something\nRUBY2\n",
    // 176 when Ruby >= 2.3 behaves like all heredoc type quoted by ` accepts for include empty lines
    "<<~`MSG`\n\n  foo\n\n    bar\n\nMSG\n",
    // 290 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "          def foo\n            <<-`RUBY2`.squish\n            something\n            RUBY2\n          end\n",
    // 309 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "          def foo\n            <<-`RUBY2`.squish\n              something\n            RUBY2\n          end\n",
    // 328 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "          def foo\n            <<-`RUBY2`.squish\n                something\n            RUBY2\n          end\n",
    // 347 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "          def foo\n            <<-`RUBY2`.squish!\n            something\n            RUBY2\n          end\n",
    // 372 when Ruby >= 2.3 behaves like all heredoc type when `AllCops/ActiveSupportExtensionsEnable
    "def foo\n  <<-`RUBY2`.squish\n  something\n  RUBY2\nend\n",
];

type Reported = (
    &'static str,
    &'static [(u32, u32, &'static str)],
    &'static str,
);

/// Every pinned example with offenses at the fixed defaults, and its
/// single-pass correction.
const REPORTED: &[Reported] = &[
    // 387 when Ruby >= 2.3 registers an offense for a heredoc nested in the interpolation of another
    (
        "def foo\n  <<~A\n  #{y ? <<~B : \"\"}\n  b\n  B\n  a\n  A\nend\n",
        &[(3, 1, "Use 2 spaces for indentation in a heredoc.")],
        "def foo\n  <<~A\n    #{y ? <<~B : \"\"}\n    b\n    B\n    a\n  A\nend\n",
    ),
    // 70 when Ruby >= 2.3 behaves like all heredoc type quoted by  registers an offense for not ind
    (
        "<<~RUBY2\nsomething\nRUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "<<~RUBY2\n  something\nRUBY2\n",
    ),
    // 85 when Ruby >= 2.3 behaves like all heredoc type quoted by  registers an offense for minus l
    (
        "def foo\n  <<~RUBY2\nsomething\n  RUBY2\nend\n",
        &[(3, 1, "Use 2 spaces for indentation in a heredoc.")],
        "def foo\n  <<~RUBY2\n    something\n  RUBY2\nend\n",
    ),
    // 104 when Ruby >= 2.3 behaves like all heredoc type quoted by  registers an offense for too dee
    (
        "<<~RUBY2\n    something\nRUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "<<~RUBY2\n  something\nRUBY2\n",
    ),
    // 119 when Ruby >= 2.3 behaves like all heredoc type quoted by  registers an offense for not ind
    (
        "<<RUBY2\nfoo\nRUBY2\n",
        &[(
            2,
            1,
            "Use 2 spaces for indentation in a heredoc by using `<<~` instead of `<<`.",
        )],
        "<<~RUBY2\nfoo\nRUBY2\n",
    ),
    // 134 when Ruby >= 2.3 behaves like all heredoc type quoted by  registers an offense for not ind
    (
        "<<~RUBY2\nfoo\nRUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "<<~RUBY2\n  foo\nRUBY2\n",
    ),
    // 149 when Ruby >= 2.3 behaves like all heredoc type quoted by  registers an offense for first l
    (
        "        puts <<-RUBY2\ndef foo\n  bar\nend\nRUBY2\n",
        &[(
            2,
            1,
            "Use 2 spaces for indentation in a heredoc by using `<<~` instead of `<<-`.",
        )],
        "        puts <<~RUBY2\ndef foo\n  bar\nend\nRUBY2\n",
    ),
    // 191 when Ruby >= 2.3 behaves like all heredoc type quoted by  registers an offense for not ind
    (
        "            def baz\n              <<~MSG\n              foo\n\n                bar\n              MSG\n            end\n",
        &[(3, 1, "Use 2 spaces for indentation in a heredoc.")],
        "            def baz\n              <<~MSG\n                foo\n\n                  bar\n              MSG\n            end\n",
    ),
    // 214 when Ruby >= 2.3 behaves like all heredoc type quoted by  registers an offense for too dee
    (
        "            <<~RUBY2\n                  foo\n\n                bar\n            RUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "            <<~RUBY2\n                foo\n\n              bar\n            RUBY2\n",
    ),
    // 191 when Ruby >= 2.3 behaves like all heredoc type quoted by  registers an offense for not ind
    (
        "            def baz\n              <<~MSG\n              foo\n    \n                bar\n              MSG\n            end\n",
        &[(3, 1, "Use 2 spaces for indentation in a heredoc.")],
        "            def baz\n              <<~MSG\n                foo\n    \n                  bar\n              MSG\n            end\n",
    ),
    // 214 when Ruby >= 2.3 behaves like all heredoc type quoted by  registers an offense for too dee
    (
        "            <<~RUBY2\n                  foo\n    \n                bar\n            RUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "            <<~RUBY2\n                foo\n    \n              bar\n            RUBY2\n",
    ),
    // 250 when Ruby >= 2.3 behaves like all heredoc type quoted by  displays message to use `<<~` in
    (
        "<<-RUBY2\nfoo\nRUBY2\n",
        &[(
            2,
            1,
            "Use 2 spaces for indentation in a heredoc by using `<<~` instead of `<<-`.",
        )],
        "<<~RUBY2\nfoo\nRUBY2\n",
    ),
    // 61 when Ruby >= 2.3 behaves like all heredoc type quoted by  when Layout/LineLength is config
    (
        "<<RUBY2\n12345678\nRUBY2\n",
        &[(
            2,
            1,
            "Use 2 spaces for indentation in a heredoc by using `<<~` instead of `<<`.",
        )],
        "<<~RUBY2\n12345678\nRUBY2\n",
    ),
    // 70 when Ruby >= 2.3 behaves like all heredoc type quoted by ' registers an offense for not in
    (
        "<<~'RUBY2'\nsomething\nRUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "<<~'RUBY2'\n  something\nRUBY2\n",
    ),
    // 85 when Ruby >= 2.3 behaves like all heredoc type quoted by ' registers an offense for minus
    (
        "def foo\n  <<~'RUBY2'\nsomething\n  RUBY2\nend\n",
        &[(3, 1, "Use 2 spaces for indentation in a heredoc.")],
        "def foo\n  <<~'RUBY2'\n    something\n  RUBY2\nend\n",
    ),
    // 104 when Ruby >= 2.3 behaves like all heredoc type quoted by ' registers an offense for too de
    (
        "<<~'RUBY2'\n    something\nRUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "<<~'RUBY2'\n  something\nRUBY2\n",
    ),
    // 119 when Ruby >= 2.3 behaves like all heredoc type quoted by ' registers an offense for not in
    (
        "<<'RUBY2'\nfoo\nRUBY2\n",
        &[(
            2,
            1,
            "Use 2 spaces for indentation in a heredoc by using `<<~` instead of `<<`.",
        )],
        "<<~'RUBY2'\nfoo\nRUBY2\n",
    ),
    // 134 when Ruby >= 2.3 behaves like all heredoc type quoted by ' registers an offense for not in
    (
        "<<~'RUBY2'\nfoo\nRUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "<<~'RUBY2'\n  foo\nRUBY2\n",
    ),
    // 149 when Ruby >= 2.3 behaves like all heredoc type quoted by ' registers an offense for first
    (
        "        puts <<-'RUBY2'\ndef foo\n  bar\nend\nRUBY2\n",
        &[(
            2,
            1,
            "Use 2 spaces for indentation in a heredoc by using `<<~` instead of `<<-`.",
        )],
        "        puts <<~'RUBY2'\ndef foo\n  bar\nend\nRUBY2\n",
    ),
    // 191 when Ruby >= 2.3 behaves like all heredoc type quoted by ' registers an offense for not in
    (
        "            def baz\n              <<~'MSG'\n              foo\n\n                bar\n              MSG\n            end\n",
        &[(3, 1, "Use 2 spaces for indentation in a heredoc.")],
        "            def baz\n              <<~'MSG'\n                foo\n\n                  bar\n              MSG\n            end\n",
    ),
    // 214 when Ruby >= 2.3 behaves like all heredoc type quoted by ' registers an offense for too de
    (
        "            <<~'RUBY2'\n                  foo\n\n                bar\n            RUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "            <<~'RUBY2'\n                foo\n\n              bar\n            RUBY2\n",
    ),
    // 191 when Ruby >= 2.3 behaves like all heredoc type quoted by ' registers an offense for not in
    (
        "            def baz\n              <<~'MSG'\n              foo\n    \n                bar\n              MSG\n            end\n",
        &[(3, 1, "Use 2 spaces for indentation in a heredoc.")],
        "            def baz\n              <<~'MSG'\n                foo\n    \n                  bar\n              MSG\n            end\n",
    ),
    // 214 when Ruby >= 2.3 behaves like all heredoc type quoted by ' registers an offense for too de
    (
        "            <<~'RUBY2'\n                  foo\n    \n                bar\n            RUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "            <<~'RUBY2'\n                foo\n    \n              bar\n            RUBY2\n",
    ),
    // 61 when Ruby >= 2.3 behaves like all heredoc type quoted by ' when Layout/LineLength is confi
    (
        "<<'RUBY2'\n12345678\nRUBY2\n",
        &[(
            2,
            1,
            "Use 2 spaces for indentation in a heredoc by using `<<~` instead of `<<`.",
        )],
        "<<~'RUBY2'\n12345678\nRUBY2\n",
    ),
    // 70 when Ruby >= 2.3 behaves like all heredoc type quoted by " registers an offense for not in
    (
        "<<~\"RUBY2\"\nsomething\nRUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "<<~\"RUBY2\"\n  something\nRUBY2\n",
    ),
    // 85 when Ruby >= 2.3 behaves like all heredoc type quoted by " registers an offense for minus
    (
        "def foo\n  <<~\"RUBY2\"\nsomething\n  RUBY2\nend\n",
        &[(3, 1, "Use 2 spaces for indentation in a heredoc.")],
        "def foo\n  <<~\"RUBY2\"\n    something\n  RUBY2\nend\n",
    ),
    // 104 when Ruby >= 2.3 behaves like all heredoc type quoted by " registers an offense for too de
    (
        "<<~\"RUBY2\"\n    something\nRUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "<<~\"RUBY2\"\n  something\nRUBY2\n",
    ),
    // 119 when Ruby >= 2.3 behaves like all heredoc type quoted by " registers an offense for not in
    (
        "<<\"RUBY2\"\nfoo\nRUBY2\n",
        &[(
            2,
            1,
            "Use 2 spaces for indentation in a heredoc by using `<<~` instead of `<<`.",
        )],
        "<<~\"RUBY2\"\nfoo\nRUBY2\n",
    ),
    // 134 when Ruby >= 2.3 behaves like all heredoc type quoted by " registers an offense for not in
    (
        "<<~\"RUBY2\"\nfoo\nRUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "<<~\"RUBY2\"\n  foo\nRUBY2\n",
    ),
    // 149 when Ruby >= 2.3 behaves like all heredoc type quoted by " registers an offense for first
    (
        "        puts <<-\"RUBY2\"\ndef foo\n  bar\nend\nRUBY2\n",
        &[(
            2,
            1,
            "Use 2 spaces for indentation in a heredoc by using `<<~` instead of `<<-`.",
        )],
        "        puts <<~\"RUBY2\"\ndef foo\n  bar\nend\nRUBY2\n",
    ),
    // 191 when Ruby >= 2.3 behaves like all heredoc type quoted by " registers an offense for not in
    (
        "            def baz\n              <<~\"MSG\"\n              foo\n\n                bar\n              MSG\n            end\n",
        &[(3, 1, "Use 2 spaces for indentation in a heredoc.")],
        "            def baz\n              <<~\"MSG\"\n                foo\n\n                  bar\n              MSG\n            end\n",
    ),
    // 214 when Ruby >= 2.3 behaves like all heredoc type quoted by " registers an offense for too de
    (
        "            <<~\"RUBY2\"\n                  foo\n\n                bar\n            RUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "            <<~\"RUBY2\"\n                foo\n\n              bar\n            RUBY2\n",
    ),
    // 191 when Ruby >= 2.3 behaves like all heredoc type quoted by " registers an offense for not in
    (
        "            def baz\n              <<~\"MSG\"\n              foo\n    \n                bar\n              MSG\n            end\n",
        &[(3, 1, "Use 2 spaces for indentation in a heredoc.")],
        "            def baz\n              <<~\"MSG\"\n                foo\n    \n                  bar\n              MSG\n            end\n",
    ),
    // 214 when Ruby >= 2.3 behaves like all heredoc type quoted by " registers an offense for too de
    (
        "            <<~\"RUBY2\"\n                  foo\n    \n                bar\n            RUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "            <<~\"RUBY2\"\n                foo\n    \n              bar\n            RUBY2\n",
    ),
    // 61 when Ruby >= 2.3 behaves like all heredoc type quoted by " when Layout/LineLength is confi
    (
        "<<\"RUBY2\"\n12345678\nRUBY2\n",
        &[(
            2,
            1,
            "Use 2 spaces for indentation in a heredoc by using `<<~` instead of `<<`.",
        )],
        "<<~\"RUBY2\"\n12345678\nRUBY2\n",
    ),
    // 70 when Ruby >= 2.3 behaves like all heredoc type quoted by ` registers an offense for not in
    (
        "<<~`RUBY2`\nsomething\nRUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "<<~`RUBY2`\n  something\nRUBY2\n",
    ),
    // 85 when Ruby >= 2.3 behaves like all heredoc type quoted by ` registers an offense for minus
    (
        "def foo\n  <<~`RUBY2`\nsomething\n  RUBY2\nend\n",
        &[(3, 1, "Use 2 spaces for indentation in a heredoc.")],
        "def foo\n  <<~`RUBY2`\n    something\n  RUBY2\nend\n",
    ),
    // 104 when Ruby >= 2.3 behaves like all heredoc type quoted by ` registers an offense for too de
    (
        "<<~`RUBY2`\n    something\nRUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "<<~`RUBY2`\n  something\nRUBY2\n",
    ),
    // 119 when Ruby >= 2.3 behaves like all heredoc type quoted by ` registers an offense for not in
    (
        "<<`RUBY2`\nfoo\nRUBY2\n",
        &[(
            2,
            1,
            "Use 2 spaces for indentation in a heredoc by using `<<~` instead of `<<`.",
        )],
        "<<~`RUBY2`\nfoo\nRUBY2\n",
    ),
    // 134 when Ruby >= 2.3 behaves like all heredoc type quoted by ` registers an offense for not in
    (
        "<<~`RUBY2`\nfoo\nRUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "<<~`RUBY2`\n  foo\nRUBY2\n",
    ),
    // 149 when Ruby >= 2.3 behaves like all heredoc type quoted by ` registers an offense for first
    (
        "        puts <<-`RUBY2`\ndef foo\n  bar\nend\nRUBY2\n",
        &[(
            2,
            1,
            "Use 2 spaces for indentation in a heredoc by using `<<~` instead of `<<-`.",
        )],
        "        puts <<~`RUBY2`\ndef foo\n  bar\nend\nRUBY2\n",
    ),
    // 191 when Ruby >= 2.3 behaves like all heredoc type quoted by ` registers an offense for not in
    (
        "            def baz\n              <<~`MSG`\n              foo\n\n                bar\n              MSG\n            end\n",
        &[(3, 1, "Use 2 spaces for indentation in a heredoc.")],
        "            def baz\n              <<~`MSG`\n                foo\n\n                  bar\n              MSG\n            end\n",
    ),
    // 214 when Ruby >= 2.3 behaves like all heredoc type quoted by ` registers an offense for too de
    (
        "            <<~`RUBY2`\n                  foo\n\n                bar\n            RUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "            <<~`RUBY2`\n                foo\n\n              bar\n            RUBY2\n",
    ),
    // 191 when Ruby >= 2.3 behaves like all heredoc type quoted by ` registers an offense for not in
    (
        "            def baz\n              <<~`MSG`\n              foo\n    \n                bar\n              MSG\n            end\n",
        &[(3, 1, "Use 2 spaces for indentation in a heredoc.")],
        "            def baz\n              <<~`MSG`\n                foo\n    \n                  bar\n              MSG\n            end\n",
    ),
    // 214 when Ruby >= 2.3 behaves like all heredoc type quoted by ` registers an offense for too de
    (
        "            <<~`RUBY2`\n                  foo\n    \n                bar\n            RUBY2\n",
        &[(2, 1, "Use 2 spaces for indentation in a heredoc.")],
        "            <<~`RUBY2`\n                foo\n    \n              bar\n            RUBY2\n",
    ),
    // 61 when Ruby >= 2.3 behaves like all heredoc type quoted by ` when Layout/LineLength is confi
    (
        "<<`RUBY2`\n12345678\nRUBY2\n",
        &[(
            2,
            1,
            "Use 2 spaces for indentation in a heredoc by using `<<~` instead of `<<`.",
        )],
        "<<~`RUBY2`\n12345678\nRUBY2\n",
    ),
];

#[test]
fn pinned_accepted_examples() {
    for source in ACCEPTED {
        assert_eq!(positions(source), vec![], "{source}");
    }
}

#[test]
fn pinned_reported_examples_and_single_pass_corrections() {
    for (source, expected, after) in REPORTED {
        let expected: Vec<_> = expected
            .iter()
            .map(|&(l, c, m)| (l, c, m.to_owned()))
            .collect();
        assert_eq!(positions(source), expected, "{source}");
        assert_correction(&HeredocIndentation, source, after);
    }
}

#[test]
fn metadata_is_fixed() {
    let cop = HeredocIndentation;
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert_eq!(cop.severity(), Severity::Convention);
    assert_eq!(
        cop.ruby_versions().minimum,
        Some(crate::cops::policy::RubyVersion(2, 3))
    );
    assert!(cop.max_setting().is_none() && cop.autocorrect_incompatible_with().is_empty());
}

#[test]
fn lookalikes_and_fixed_policy_boundaries() {
    for source in [
        // Not heredocs.
        "x = \"\nfoo\n\"\n",
        "x = 'a' \\\n  'b'\n",
        "x = %(\nfoo\n)\n",
        "x = [1 << 2]\n",
        // Blank bodies (`body.strip.empty?`), including NUL.
        "x = <<~A\nA\n",
        "x = <<~A\n\n   \nA\n",
        "x = <<-A\n\0\nA\n",
        // Correct squiggly bodies, interpolation and xstrings.
        "def f\n  <<~A\n    #{x}\n      y\n  A\nend\n",
        "x = <<~`A`\n  ls\nA\n",
        // `<<-`/`<<` only need some indentation; `squish` is never special.
        "x = <<-A\n  foo\nfoo2\n  A\n"
            .replace("foo2", " foo2")
            .as_str(),
        "def f\n  <<-A.squish\n  foo\n  A\nend\n",
        // Ruby `\\s` includes VT and FF.
        "x = <<-A\n\x0bfoo\nA\n",
        // A long body is not exempt (AllowHeredoc: true) but this one is fine.
        &format!("x = <<~A\n  {}\nA\n", "a".repeat(200)),
    ] {
        assert_eq!(positions(source), vec![], "{source:?}");
    }
    // `AllowHeredoc: true` fixed: a long misindented body is still reported.
    let long = format!("x = <<~A\n{}\nA\n", "a".repeat(200));
    assert_eq!(positions(&long).len(), 1);
    // Tabs count as one character; the opener's line sets the base.
    assert_eq!(
        positions("\tx = <<~A\n\t foo\n\tA\n"),
        vec![(2, 1, "Use 2 spaces for indentation in a heredoc.".into())]
    );
    assert_eq!(positions("x = <<-A\n\tfoo\nA\n"), vec![]);
}

#[test]
fn nested_heredocs_follow_the_stateful_within_offending_rule() {
    // The outer body is fine; the inner one is reported on its own.
    let source = "def f\n  <<~A\n    #{<<~B}\n    b\n    B\n  A\nend\n";
    assert_eq!(
        positions(source),
        vec![(4, 1, "Use 2 spaces for indentation in a heredoc.".into())]
    );
    // Both misindented: only the enclosing body is reported, as upstream.
    let source = "x = <<~A\n#{<<-B}\nb\nB\nA\n";
    assert_eq!(
        positions(source),
        vec![(2, 1, "Use 2 spaces for indentation in a heredoc.".into())]
    );
    // Two heredocs on one line are independent.
    let source = "f(<<~A, <<-B)\na\nA\nb\nB\n";
    assert_eq!(
        positions(source),
        vec![
            (2, 1, "Use 2 spaces for indentation in a heredoc.".into()),
            (
                4,
                1,
                "Use 2 spaces for indentation in a heredoc by using `<<~` instead of `<<-`.".into()
            ),
        ]
    );
    assert_correction(
        &HeredocIndentation,
        "f(<<~A, <<-B)\na\nA\nb\nB\n",
        "f(<<~A, <<~B)\n  a\nA\nb\nB\n",
    );
}

#[test]
fn canonical_edits_keep_values_crlf_and_other_bytes() {
    for (before, after) in [
        (
            "def f\n  <<~A\n      x\n        y\n\n  A\nend\n",
            "def f\n  <<~A\n    x\n      y\n\n  A\nend\n",
        ),
        // Blank lines at least as deep as the body shift with it.
        (
            "x = <<~A\n    a\n      \n    b\nA\n",
            "x = <<~A\n  a\n    \n  b\nA\n",
        ),
        // A short blank line not deeper than the target keeps its value.
        (
            "x = <<~A\n    a\n \n    b\nA\n",
            "x = <<~A\n  a\n \n  b\nA\n",
        ),
        // The closing line is raised to the opening line's indentation.
        ("  x = <<~A\nfoo\nA\n", "  x = <<~A\n    foo\n  A\n"),
        (
            "x = <<~A\r\nfoo\r\n\r\nA\r\n",
            "x = <<~A\r\n  foo\r\n  \r\nA\r\n",
        ),
        ("x = <<-'A'\nfoo #{x}\n  A\n", "x = <<~'A'\nfoo #{x}\n  A\n"),
        ("x = <<`A`\nls\nA\n", "x = <<~`A`\nls\nA\n"),
        (
            "# encoding: utf-8\nx = <<~A\n  \u{e9}t\u{e9}\n  A\n",
            "# encoding: utf-8\nx = <<~A\n  \u{e9}t\u{e9}\n  A\n",
        ),
    ] {
        if before == after {
            assert_eq!(positions(before), vec![], "{before:?}");
            continue;
        }
        assert_correction(&HeredocIndentation, before, after);
    }
}

#[test]
fn value_changing_or_uncertain_edits_keep_the_finding_without_an_edit() {
    for source in [
        // Tabs are dedented by columns, not characters.
        "x = <<~A\n\tfoo\n        bar\nA\n",
        "x = <<~A\n    foo\n\t\nA\n",
        // A blank line shorter than the old level but deeper than the target.
        "x = <<~A\n      a\n   \n      b\nA\n",
        // Line continuations are not dedent line starts.
        "x = <<~A\n      foo \\\n    bar\nA\n",
        "x = <<-A\nfoo \\\n  bar\nA\n",
        // Interpolated code spanning lines is not string text.
        "x = <<~A\n      #{[\n    1].join}\nA\n",
        // A non-squiggly nested body would change its value when shifted.
        "x = <<~A\n#{<<-B}\n  b\n  B\nA\n",
        // Converting a plain `<<` whose body holds an indented label line
        // would end the heredoc early.
        "x = <<A\nfoo\n  A\nA\n",
        "x = <<-A\n#{<<~B}\nb\nB\nA\n",
        // Tab indentation on the opening or closing line.
        "\tx = <<~A\nfoo\n\tA\n",
        "x = <<~A\nfoo\n\tA\n",
    ] {
        let found = correction_offenses(&HeredocIndentation, source);
        assert_eq!(found.len(), 1, "{source:?}");
        assert!(found[0].correction.is_none(), "{source:?}");
        assert_eq!(corrected(source), None);
    }
}

#[test]
fn listed_source_observers_hold_the_edit() {
    for observer in [
        "p __FILE__",
        "p __dir__",
        "p $0",
        "p $PROGRAM_NAME",
        "p caller",
        "p caller_locations",
        "p method(:f).source_location",
        "p SCRIPT_LINES__",
    ] {
        let source = format!("{observer}\nx = <<~A\nfoo\nA\n");
        let found = correction_offenses(&HeredocIndentation, &source);
        assert_eq!(found.len(), 1, "{source}");
        assert!(found[0].correction.is_none(), "{source}");
    }
    let data = "p DATA.read\nx = <<~A\nfoo\nA\n__END__\nx\n";
    assert!(
        correction_offenses(&HeredocIndentation, data)[0]
            .correction
            .is_none()
    );
    // A program reading its own path is out of scope.
    assert_correction(
        &HeredocIndentation,
        "p File.read('example.rb')\nx = <<~A\nfoo\nA\n",
        "p File.read('example.rb')\nx = <<~A\n  foo\nA\n",
    );
}

#[test]
fn deferred_nested_bodies_follow_the_translation_body() {
    // The translation's body ends with the line holding the nested opener,
    // so only that line is measured and shifted.
    let source = "def f\n  <<~A\n  #{<<~B}\n      b\n    B\n  A\nend\n";
    assert_eq!(
        positions(source),
        vec![(3, 1, "Use 2 spaces for indentation in a heredoc.".into())]
    );
    assert_correction(
        &HeredocIndentation,
        source,
        "def f\n  <<~A\n    #{<<~B}\n      b\n    B\n  A\nend\n",
    );
}
