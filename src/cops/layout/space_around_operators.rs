//! `Layout/SpaceAroundOperators` — pinned spaces and alignment exemptions.
use super::operator_spacing::{self, Kind};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct SpaceAroundOperators;
impl Cop for SpaceAroundOperators {
    fn name(&self) -> &'static str {
        "Layout/SpaceAroundOperators"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_program_node().is_none() {
            return;
        }
        let operators = operator_spacing::collect(node);
        let alignment = operator_spacing::Alignment::new(file, &operators);
        for op in &operators {
            if op.kind == Kind::Ignored {
                continue;
            }
            let around = operator_spacing::surrounding(file, &op.range);
            let text = &file.bytes[around.clone()];
            if text.starts_with(b"\n") || text.starts_with(b"\r\n") {
                continue;
            }
            let (line, col) = file.line_col(op.range.start);
            let comments = file.comments();
            let comment = comments.partition_point(|c| c.start < op.range.end);
            if comments
                .get(comment)
                .is_some_and(|c| c.start == around.end && file.line_of(c.start) == line)
            {
                continue;
            }
            let operator = String::from_utf8_lossy(&file.bytes[op.range.clone()]);
            let message = if op.no_space {
                if around == op.range {
                    continue;
                }
                format!("Space around operator `{operator}` detected.")
            } else if around.start == op.range.start || around.end == op.range.end {
                format!("Surrounding space missing for operator `{operator}`.")
            } else if (text.starts_with(b"  ") && !alignment.leading(op))
                || (text.ends_with(b"  ") && !alignment.trailing(op))
            {
                format!("Operator `{operator}` should be surrounded by a single space.")
            } else {
                continue;
            };
            out.push(Offense {
                correction: cx
                    .corrections_requested()
                    .then(|| operator_spacing::correction(file, node, op, around))
                    .flatten(),
                cop: self.name(),
                severity: self.severity(),
                message,
                line,
                end_line: line,
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's `space_around_operators_spec.rb` at 97f30c4
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == SpaceAroundOperators.name())
            .collect()
    }
    #[test]
    fn pinned_exact_offenses() {
        for (source, line, col, message) in [
            ("x=0", 1, 2, "Surrounding space missing for operator `=`."),
            (
                "x = !a&&!b",
                1,
                7,
                "Surrounding space missing for operator `&&`.",
            ),
            (
                "x = a * b ** 2",
                1,
                11,
                "Space around operator `**` detected.",
            ),
            (
                "x = a * b / 42r",
                1,
                11,
                "Space around operator `/` detected.",
            ),
            (
                "puts 1 +  2",
                1,
                8,
                "Operator `+` should be surrounded by a single space.",
            ),
            (
                "\u{feff}é=1\r\n",
                1,
                2,
                "Surrounding space missing for operator `=`.",
            ),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].end_line, found[0].col),
                (line, line, col)
            );
            assert_eq!(found[0].message, message);
            assert_eq!(found[0].severity, crate::offense::Severity::Convention);
        }
    }
    #[test]
    fn pinned_before_after_fixtures() {
        for (before, after) in [
            ("x = !a&&!b", "x = !a && !b"),
            (
                "x = a * b ** 2\ny = a * b** 2",
                "x = a * b**2\ny = a * b**2",
            ),
            (
                "x = a * b / 42r\ny = a * b/ 42r",
                "x = a * b/42r\ny = a * b/42r",
            ),
            ("base**=exp\nval/=2r", "base **= exp\nval /= 2r"),
            (
                "x+= a+b-c*d/e%f^g|h&i||j\ny -=k&&l",
                "x += a + b - c * d / e % f ^ g | h & i || j\ny -= k && l",
            ),
            (
                "x||=0\ny&&=0\nz*=2\n@a=0\n@@a=0\na,b=0\nA=0\nx[3]=0\n$A=0\nA||=0",
                "x ||= 0\ny &&= 0\nz *= 2\n@a = 0\n@@a = 0\na,b = 0\nA = 0\nx[3] = 0\n$A = 0\nA ||= 0",
            ),
            ("x.y=2,3", "x.y = 2,3"),
            ("x==0\ny!=0\nHash===z", "x == 0\ny != 0\nHash === z"),
            (
                "a-3\nx&0xff\nz+0\n-1-arg",
                "a - 3\nx & 0xff\nz + 0\n-1 - arg",
            ),
            ("{ 1=>2, a: b }", "{ 1 => 2, a: b }"),
            (
                "begin\nrescue Exception=>e\nend",
                "begin\nrescue Exception => e\nend",
            ),
            ("class<<self\nend", "class << self\nend"),
            ("class Foo<Bar\nend", "class Foo < Bar\nend"),
            ("x=~/abc/\ny !~/abc/", "x =~ /abc/\ny !~ /abc/"),
            ("x == 0?1:2", "x == 0 ? 1 : 2"),
            ("x == 0  ? 1 :  2", "x == 0 ? 1 : 2"),
            ("def foo(a, b)=a + b", "def foo(a, b) = a + b"),
            ("def self.foo =1", "def self.foo = 1"),
            ("case foo\nin 0|1|2\nend", "case foo\nin 0 | 1 | 2\nend"),
            ("foo in bar=>baz", "foo in bar => baz"),
            ("\"\"=>foo", "\"\" => foo"),
            (
                "'Here is a'+\r\n'joined string'+\r\n'across three lines'",
                "'Here is a' +\r\n'joined string' +\r\n'across three lines'",
            ),
            ("x   = 0\ny +=   0\nz[0]  =  0", "x = 0\ny += 0\nz[0] = 0"),
            (
                "x +=  a  + b -  c  * d /  e  % f  ^ g   | h &  i  ||  j\ny  -=  k   &&        l",
                "x += a + b - c * d / e % f ^ g | h & i || j\ny -= k && l",
            ),
        ] {
            assert!(!offenses(before).is_empty(), "{before}");
            assert_correction(&SpaceAroundOperators, before, after);
            assert_eq!(
                crate::cops::ast::fingerprint_ignoring_grouping(
                    &ruby_prism::parse(before.as_bytes()).node(),
                    before.as_bytes()
                ),
                crate::cops::ast::fingerprint_ignoring_grouping(
                    &ruby_prism::parse(after.as_bytes()).node(),
                    after.as_bytes()
                ),
                "{before}"
            );
            assert!(offenses(after).is_empty(), "{after}");
        }
    }
    #[test]
    fn newline_scan_stops_at_prior_line_blanks_and_nonstandard_whitespace() {
        assert!(offenses("def foo()  \n  =1").is_empty());
        assert_eq!(offenses("a\u{b}+\u{c}b").len(), 1);
        assert_correction(&SpaceAroundOperators, "a\u{b}+\u{c}b", "a\u{b} + \u{c}b");
    }

    #[test]
    fn additional_pinned_corrections_and_modifiers() {
        for (before, after) in [
            ("def foo =1", "def foo = 1"),
            ("def foo()=1", "def foo() = 1"),
            ("def foo() =1", "def foo() = 1"),
            ("def foo()= 1", "def foo() = 1"),
            ("x == 0? 1: 2", "x == 0 ? 1 : 2"),
            ("x == 0 ?1 :2", "x == 0 ? 1 : 2"),
            ("a -  3\nx &   0xff\nz +  0", "a - 3\nx & 0xff\nz + 0"),
            ("x.y  =  2", "x.y = 2"),
            ("{ 1  =>   2, a: b }", "{ 1 => 2, a: b }"),
            ("{\n  1 =>  2\n}", "{\n  1 => 2\n}"),
            ("x  =~ /abc/\ny !~   /abc/", "x =~ /abc/\ny !~ /abc/"),
            (
                "x  ==  0\ny   != 0\nHash   ===   z",
                "x == 0\ny != 0\nHash === z",
            ),
            ("-1  - arg", "-1 - arg"),
            ("class Foo  <  Bar\nend", "class Foo < Bar\nend"),
            (
                "begin\nrescue Exception   =>      e\nend",
                "begin\nrescue Exception => e\nend",
            ),
        ] {
            assert_correction(&SpaceAroundOperators, before, after);
        }
        for keyword in ["if", "unless", "while", "until"] {
            for before in [
                format!("a=1 {keyword} condition\nc=2"),
                format!("a =  1 {keyword} condition\nc =   2"),
            ] {
                assert_correction(
                    &SpaceAroundOperators,
                    &before,
                    &format!("a = 1 {keyword} condition\nc = 2"),
                );
            }
        }
    }

    #[test]
    fn alignment_columns_follow_source_characters_and_ignore_the_bom() {
        for source in [
            "é  += x\naa += y",
            "\u{feff}x  += y\nzz += z",
            "{\n  é =>  2,\n  aa => 3\n}",
        ] {
            assert!(
                offenses(source).is_empty(),
                "{source}: {:?}",
                offenses(source)
            );
        }
    }

    #[test]
    fn alignment_uses_declared_legacy_character_widths() {
        let source = b"# coding: shift_jis\n\x82\xa0  += x\naa += y\n";
        let file = SourceFile::new("x.rb".into(), source.to_vec());
        let parsed = ruby_prism::parse(source);
        assert_eq!(parsed.errors().count(), 0);
        file.record_encoding(&parsed);
        file.record_comments(&parsed);
        let operators = operator_spacing::collect(&parsed.node());
        let alignment = operator_spacing::Alignment::new(&file, &operators);
        assert_eq!(operators.len(), 2);
        assert!(alignment.leading(&operators[0]));
    }

    #[test]
    fn large_assignment_groups_and_nested_content_are_indexed_once() {
        let mut source = String::new();
        for i in 0..4_000 {
            source.push_str(&format!("a{i:04}  = 1\nconsume\n"));
        }
        source.push_str("longer = 2\n");
        let found = crate::pipeline::lint_str_with(&[&SpaceAroundOperators], &source);
        // Each padded assignment aligns with its predecessor/successor;
        // the final unpadded assignment doesn't force earlier rescans.
        assert!(found.is_empty(), "{:?}", &found[..found.len().min(3)]);
    }

    #[test]
    fn keyword_logical_operator_boundaries_and_compound_assignment_targets() {
        for (before, after) in [
            ("\"a\"and true", "\"a\" and true"),
            ("\"a\"or false", "\"a\" or false"),
            ("obj&.x+=1", "obj&.x += 1"),
            ("obj[0]**=2", "obj[0] **= 2"),
            ("A::B/=2r", "A::B /= 2r"),
            (
                "@a&&=1\n@@a||=2\n$x<<=3\nA::B+=4",
                "@a &&= 1\n@@a ||= 2\n$x <<= 3\nA::B += 4",
            ),
        ] {
            assert_correction(&SpaceAroundOperators, before, after);
        }
    }
    #[test]
    fn pinned_alignment_exemptions() {
        for source in [
            "@integer_message = 12345\n@output  = StringIO.new\n@logger  = Logger.new(@output)",
            "expected = posts(:welcome)\n\ntagging  = Tagging.all\nassert_no_queries { assert_equal expected, tagging }",
            "down? && !migrated.include?(migration.version.to_i)\nup?   &&  migrated.include?(migration.version.to_i)",
            "var.foo       = a\nvar.bar      != b\nvar.quux     <= c\nvar.garply   >= d\nvar.corge    == e\nvar.fred     += f\nvar.baz     === g",
            "{\n  1 =>  2,\n  11 => 3\n}",
            "x  << foo\nyz += bar",
            "x  += foo\nyz << bar",
            "@areas   = params.map do\n             var_1      = 123\n             variable_2 = 456\n           end\n@another = params.map do\n             char_1 = begin\n                        variable_1  = 'a'\n                        variable_20 = 'b'\n\n                        variable_300  = 'c'\n                        # comment\n                        variable_4000 = 'd'\n                      end\n             var_2  = 456\n           end",
        ] {
            assert!(
                offenses(source).is_empty(),
                "{source}: {:?}",
                offenses(source)
            );
        }
    }
    #[test]
    fn nested_blank_lines_preserve_outer_assignment_alignment() {
        // Checked against pinned RuboCop's relevant_lines in both directions.
        for (before, after, line, col) in [
            (
                "a   = begin\n  x\n\nend\nlong = 2\n",
                "a = begin\n  x\n\nend\nlong = 2\n",
                1,
                5,
            ),
            (
                "long = begin\n  x\n\nend\na    = 1\nz = 2\n",
                "long = begin\n  x\n\nend\na = 1\nz = 2\n",
                5,
                6,
            ),
            (
                "long = 1\n\na    = 1\nz = 2\n",
                "long = 1\n\na = 1\nz = 2\n",
                3,
                6,
            ),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!((found[0].line, found[0].col), (line, col));
            assert_eq!(
                found[0].message,
                "Operator `=` should be surrounded by a single space."
            );
            assert_correction(&SpaceAroundOperators, before, after);
        }
        for source in [
            "long = 1\nbegin\n\n  x\nend\na    = 1\nz = 2\n",
            "a   = 1\n\nlong = 2\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn indexed_setter_alignment_uses_the_first_argument() {
        for source in [
            "rows[0] =  10\nrows[0, 100000] = 20\n",
            "rows[0, 100000] = 20\nrows[0] =  10\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
            assert!(correction_offenses(&SpaceAroundOperators, source).is_empty());
        }
        let before = "rows[0] =  10\n";
        let found = offenses(before);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 9));
        assert_eq!(
            found[0].message,
            "Operator `=` should be surrounded by a single space."
        );
        assert_correction(&SpaceAroundOperators, before, "rows[0] = 10\n");
    }

    #[test]
    fn unary_splats_keywords_ranges_and_method_syntax_are_not_binary_operators() {
        for source in [
            "",
            "a\t+\tb",
            "func(:-)",
            "a, b = (1..2), (1...3)",
            "x = 2/3r",
            "x = 2 / 3r",
            "x = a * b**2",
            "x = a * b/42r",
            "return *list if options",
            "def +(other); end\ndef self.===(other); end\ndef !; !__getobj__; end",
            "[].map(&:size)\na.(b)\n-3\nfoo::~\narr.collect { |e| -e }\nx = +2",
            "files[2]\nfiles[]\nfiles[:key], files[:another] = method",
            "def init(name=nil, other: 1, **kw); end",
            "f(*args, **kw, &block)",
            "Date.today.+(1).to_s",
            "a = b \\\n    && c",
            "x =\n0",
            "def foo()\n  =1",
            "foo +  # comment\n  bar",
            "\"\"in foo",
            "a and b or c",
            "f(a:1)",
        ] {
            assert!(
                offenses(source).is_empty(),
                "{source}: {:?}",
                offenses(source)
            );
        }
    }
    #[test]
    fn literal_and_comment_bytes_are_untouched_but_interpolation_is_code() {
        for source in [
            "# a+b\n",
            "'a+b'",
            "\"a+b\"",
            "/a+b/",
            "%r(a+b)",
            "%q(a+b)",
            "%Q(a+b)",
            "%w(a+b)",
            "%W(a+b)",
            "%i(a+b)",
            "%I(a+b)",
            "%x(a+b)",
            "%s(a+b)",
            "x = <<~H\na+b\nH\n",
            "__END__\na+b\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for (before, after) in [
            ("\"a+b#{1+2}\"", "\"a+b#{1 + 2}\""),
            ("%W(a+b#{1+2})", "%W(a+b#{1 + 2})"),
            ("%I(a+b#{1+2})", "%I(a+b#{1 + 2})"),
            ("%r(a+b#{1+2})", "%r(a+b#{1 + 2})"),
            ("%x(a+b#{1+2})", "%x(a+b#{1 + 2})"),
            (
                "x = <<~OUT\n#{<<~IN}\na+b#{1+2}\nIN\nOUT\n",
                "x = <<~OUT\n#{<<~IN}\na+b#{1 + 2}\nIN\nOUT\n",
            ),
            ("x = <<~H\na+b#{1+2}\nH\n", "x = <<~H\na+b#{1 + 2}\nH\n"),
            ("x = 1+2\n__END__\na+b\n", "x = 1 + 2\n__END__\na+b\n"),
        ] {
            assert_correction(&SpaceAroundOperators, before, after);
        }
    }
    #[test]
    fn alignment_does_not_use_assignment_operators_inside_strings() {
        for (before, after) in [
            ("x   += foo\n'yz << bar'", "x += foo\n'yz << bar'"),
            ("'yz << bar'\nx   += foo", "'yz << bar'\nx += foo"),
        ] {
            assert_eq!(offenses(before).len(), 1);
            assert_correction(&SpaceAroundOperators, before, after);
        }
    }
    #[test]
    fn no_space_fixes_withhold_line_joining_and_literal_delimiter_changes() {
        for source in [
            "x = 2 **\n  3\np __LINE__\n",
            "x = 2.0 /\n  3r\np __LINE__\n",
        ] {
            let found = correction_offenses(&SpaceAroundOperators, source);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none(), "{source}");
        }
        let found = correction_offenses(&SpaceAroundOperators, "%q a +1");
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }
}
