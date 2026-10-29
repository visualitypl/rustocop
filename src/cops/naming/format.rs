//! RuboCop's naming formats, shared by the `Naming` cops.
//!
//! RuboCop matches names with Ruby regexps anchored by `^` and `$`, which
//! are line anchors: a name passes when any of its lines matches in full.
//! Only string arguments (`define_method "a\nb"`) can hold a newline, but
//! the helpers keep the semantics exact. POSIX classes are Unicode-aware in
//! Ruby, as `char::is_lowercase` and `char::is_uppercase` are; `\d` is ASCII.

/// RuboCop's `snake_case` format (`ConfigurableNaming::FORMATS`):
/// `/^@{0,2}[\d[[:lower:]]_]+[!?=]?$/`. The optional sigils let it check
/// instance and class variable names too.
pub(super) fn is_snake_case(name: &str) -> bool {
    name.split('\n').any(|line| {
        let rest = line
            .strip_prefix("@@")
            .or_else(|| line.strip_prefix('@'))
            .unwrap_or(line);
        let body = rest.strip_suffix(['!', '?', '=']).unwrap_or(rest);
        !body.is_empty()
            && body
                .chars()
                .all(|c| c.is_ascii_digit() || c.is_lowercase() || c == '_')
    })
}

/// `Naming/ConstantName`'s `SCREAMING_SNAKE_CASE`:
/// `/^[[:digit:][:upper:]_]+$/`.
///
/// Ruby's `[[:digit:]]` is Unicode's decimal digits (`Nd`); the standard
/// library has no such test, so other non-ASCII numerals (`²`, `½`) pass
/// here too. Constant names holding them are vanishingly rare.
pub(super) fn is_screaming_snake_case(name: &str) -> bool {
    name.split('\n').any(|line| {
        !line.is_empty()
            && line.chars().all(|c| {
                c.is_ascii_digit()
                    || (!c.is_ascii() && c.is_numeric())
                    || c.is_uppercase()
                    || c == '_'
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_names() {
        for name in [
            "foo",
            "foo_bar",
            "_",
            "_foo",
            "__foo__",
            "foo2",
            "2",
            "foo?",
            "foo!",
            "foo=",
            "@foo",
            "@@foo_bar",
            "última_vista",
            "léo",
            "straße",
            // Line anchors: one matching line is enough.
            "fooBar\nfoo",
            "foo\n",
        ] {
            assert!(is_snake_case(name), "{name:?} is snake_case");
        }
        for name in [
            "fooBar",
            "FooBar",
            "Foo",
            "_myLocal",
            "foo??",
            "foo?!",
            "?",
            "@",
            "@@",
            "@@@foo",
            "$foo",
            "foo-bar",
            "日本語",
            "",
            "Foo\nBar",
        ] {
            assert!(!is_snake_case(name), "{name:?} is not snake_case");
        }
    }

    #[test]
    fn screaming_snake_case_names() {
        for name in ["TOP_TEST", "TEST_2", "_", "A", "TÖP_TEST", "X١"] {
            assert!(is_screaming_snake_case(name), "{name:?}");
        }
        for name in ["TopCase", "TOP_test", "Nö", "Inch_In_Cm", "INCHinCM", ""] {
            assert!(!is_screaming_snake_case(name), "{name:?}");
        }
    }
}
