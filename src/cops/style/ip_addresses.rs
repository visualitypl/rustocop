//! `Style/IpAddresses` — opt-in detection of hardcoded source IP literals.

use std::sync::LazyLock;

use regex::bytes::Regex;
use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct IpAddresses;

impl Cop for IpAddresses {
    fn name(&self) -> &'static str {
        "Style/IpAddresses"
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_string_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(string) = node.as_string_node() else {
            return;
        };
        // StringHelp checks strings with a begin token but slices only one
        // source character from either end. Percent literals/heredocs therefore
        // never match; unquoted word-array elements and synthetic dstr parts
        // have no begin token. Do not inspect decoded values or dynamic chunks.
        if !string
            .opening_loc()
            .is_some_and(|loc| matches!(loc.as_slice(), b"'" | b"\""))
        {
            return;
        }
        let location = string.location();
        let source = location.as_slice();
        let Some(contents) = source.get(1..source.len().saturating_sub(1)) else {
            return;
        };
        if !address_literal(contents) {
            return;
        }
        // StringHelp ignores the entire regexp subtree, even nested expressions
        // inside interpolation. Other interpolations still check quoted strings.
        if cx.ancestors().any(|ancestor| {
            matches!(
                ancestor,
                Node::RegularExpressionNode { .. }
                    | Node::InterpolatedRegularExpressionNode { .. }
                    | Node::MatchLastLineNode { .. }
                    | Node::InterpolatedMatchLastLineNode { .. }
            )
        }) {
            return;
        }
        let (line, col) = file.line_col(location.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Do not hardcode IP addresses.".into(),
            line,
            end_line: file.line_of(location.end_offset() - 1),
            col,
            correction: None,
        });
    }
}

/// Match the pinned Resolv regex language, not address resolution or IPAddr.
/// In particular, Resolv's compressed alternatives do not count hextets and its
/// embedded IPv4 alternative does not range-check decimal components. Keep these
/// upstream quirks rather than silently substituting std::net::Ipv6Addr.
fn address_literal(contents: &[u8]) -> bool {
    if contents.is_empty()
        || contents.len() > 45
        || contents == b"::"
        || !matches!(contents[0], b'0'..=b'9' | b':' | b'A'..=b'F' | b'a'..=b'f')
    {
        return false;
    }
    static IPV6: LazyLock<Regex> = LazyLock::new(|| {
        let hex = "[0-9a-fA-F]{1,4}";
        let list = format!("(?:{hex}(?::{hex})*)?");
        let decimal = "[0-9]+(?:\\.[0-9]+){3}";
        let zone = "[-0-9A-Za-z._~]+";
        Regex::new(&format!(
            "\\A(?:\
             (?:{hex}:){{7}}{hex}|\
             {list}::{list}|\
             (?:{hex}:){{6}}{decimal}|\
             {list}::(?:{hex}:)*{decimal}|\
             [fF][eE]80(?::{hex}){{7}}%{zone}|\
             [fF][eE]80:(?:{list}::{list}|:{list})?:{hex}%{zone}\
             )\\z"
        ))
        .expect("fixed Resolv IPv6 grammar")
    });
    std::str::from_utf8(contents).is_ok_and(|text| text.parse::<std::net::Ipv4Addr>().is_ok())
        || IPV6.is_match(contents)
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/style/ip_addresses_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str_with;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str_with(&[&IpAddresses], source)
    }

    fn assert_offense(source: &str, line: u32, col: u32) {
        let found = offenses(source);
        assert_eq!(found.len(), 1, "{source}");
        assert_eq!((found[0].line, found[0].col), (line, col), "{source}");
        assert_eq!(found[0].end_line, line, "{source}");
        assert_eq!(found[0].message, "Do not hardcode IP addresses.");
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn pinned_ipv4_and_ipv6_offenses() {
        for address in [
            "255.255.255.255",
            "2001:0db8:85a3:0000:0000:8a2e:0370:7334",
            "2001:db8:85a3::8a2e:370:7334",
            "2001:db8::1",
            "1::",
            "::1",
        ] {
            assert_offense(&format!("ip =\n  '{address}'\n"), 2, 3);
        }
    }

    #[test]
    fn raw_spelling_not_decoded_values_is_checked() {
        for source in [
            r#""192.168.\x31.1""#,
            r#""\u0031.2.3.4""#,
            "%q(1.2.3.4)",
            "%Q(1.2.3.4)",
            "%(1.2.3.4)",
            "%w[1.2.3.4]",
            "%W[1.2.3.4]",
            ":'1.2.3.4'",
            "/1.2.3.4/",
            "`1.2.3.4`",
            "__FILE__",
            "ip = <<~IP\n1.2.3.4\nIP\n",
            "# '1.2.3.4'\n",
            "\"1.2.3.4#{suffix}\"",
            "\"1.2.3.4\n\"",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_offense("\"1.2.3.4\"", 1, 1);
    }

    #[test]
    fn nested_strings_and_adjacent_literals_keep_their_locations() {
        for (source, col) in [
            (r##""#{'1.2.3.4'}""##, 4),
            (":\"#{'1.2.3.4'}\"", 5),
            ("`echo #{'1.2.3.4'}`", 9),
            ("consume('1.2.3.4')", 9),
        ] {
            assert_offense(source, 1, col);
        }
        let found = offenses("'1.2.3.4' '5.6.7.8'");
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 1), (1, 11)]
        );
        for source in [
            r#"/#{'1.2.3.4'}/"#,
            r#"%r{#{consume('1.2.3.4')}}"#,
            r#"/#{'1.2.3.4'}/ =~ value"#,
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn address_boundaries_and_resolv_ipv6_regex_compatibility() {
        for address in [
            "0.0.0.0",
            "127.0.0.1",
            "a::b",
            "A::B",
            "1:2:3:4:5:6:7:8",
            "::ffff:192.0.2.128",
            "ffff:ffff:ffff:ffff:ffff:ffff:255.255.255.255",
            "fe80::1%eth0",
            "FE80:0:0:0:0:0:0:1%en-0._~",
            // Resolv::IPv6::Regex (not IPAddr) accepts these spellings.
            "1:2:3:4:5:6:7::8",
            "::ffff:999.888.777.666",
            "::ffff:001.2.3.4",
        ] {
            assert_offense(&format!("'{address}'"), 1, 1);
        }
        for address in [
            "01.2.3.4",
            "256.0.0.1",
            "1.2.3",
            "1.2.3.4:80",
            "1.2.3.4/24",
            "[::1]",
            "::1/128",
            "2001::1%eth0",
            "fe80::%eth0",
            "fe80::1%",
            "1:2:3:4:5:6:7",
            "1:2:3:4:5:6:7:8:9",
            "1:::2",
            "::g",
            "::12345",
            " localhost",
            "1.2.3.4 ",
            "https://1.2.3.4",
            "é::1",
        ] {
            assert!(offenses(&format!("'{address}'")).is_empty(), "{address}");
        }
    }

    #[test]
    fn candidates_are_bounded_to_45_source_bytes() {
        let longest = format!("::{}111", "1:".repeat(20));
        assert_eq!(longest.len(), 45);
        assert!(address_literal(longest.as_bytes()));
        assert!(!address_literal(format!("{longest}1").as_bytes()));
        assert!(!address_literal(&vec![b'1'; 1_000_000]));
        let source = format!("'{}'\n{}", "1".repeat(1_000_000), "'::1'\n".repeat(1000));
        let found = offenses(&source);
        assert_eq!(found.len(), 1000);
        assert_eq!((found[999].line, found[999].col), (1001, 1));
    }

    #[test]
    fn noncorrectable_even_when_corrections_are_requested() {
        assert!(!IpAddresses.default_enabled());
        assert!(!IpAddresses.safe_fixes());
        assert!(
            crate::pipeline::lint_str("'::1'\n")
                .iter()
                .all(|o| o.cop != IpAddresses.name())
        );
        let found = crate::cops::testing::correction_offenses(&IpAddresses, "'::1'\n'1.2.3.4'\n");
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|o| o.correction.is_none()));
    }

    #[test]
    fn pinned_no_offense_examples_and_fixed_allowlist() {
        for source in [
            "''",
            "\"578.194.591.059\"",
            "\"My IP is 192.168.1.1\"",
            "\"2001:db8::1xyz\"",
            "\"::\"",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        // The upstream custom allowlist examples are alternate policy, not defaults.
        assert_offense("\"a::b\"", 1, 1);
        assert_offense("\"A::B\"", 1, 1);
    }
}
