//! Authored source-only witnesses. Never execute these inputs.
use std::ops::Range;

#[derive(Debug)]
pub struct Case {
    pub source: Vec<u8>,
    pub valid: bool,
    pub keyword: &'static str,
    pub range: Range<usize>,
}

fn case(template: &str, keyword: &'static str, valid: bool) -> Case {
    let start = template.find("<flow>").unwrap();
    Case {
        source: template.replace("<flow>", keyword).into_bytes(),
        valid,
        keyword,
        range: start..start + keyword.len(),
    }
}

pub fn cases() -> Vec<Case> {
    let invalid = [
        "def f; foo { |x = (<flow>; y)| x }; end",
        "foo { |x = (<flow>; y)| x }",
        "foo do |x = (<flow>; y)| x end",
        "foo { |x: (<flow>; y)| x }",
        "foo do |x: (<flow>; y)| x end",
        "foo { |a, x = (<flow>; y), *rest, k: 1; local| x }",
        "foo { |a, x: (<flow>; y), **rest, &block| x }",
        "foo { |x = (if test; <flow>; y; else; z; end)| x }",
        "foo { |x = (begin; <flow>; y; end)| x }",
        "foo { |x = (begin; y; ensure; <flow>; z; end)| x }",
        "foo { |x = (begin; y; rescue; <flow>; z; end)| x }",
        "foo { |x = (\"#{<flow>; y}\")| x }",
        "foo { |x = (foo { |y = (<flow>; z)| y })| x }",
        "foo { |x = ->(y = (<flow>; z)) { y }| x }",
        "foo { |x = (def f; <flow>; y; end)| x }",
        "foo { |x = (def f(y = (<flow>; z)); y; end)| x }",
        "foo { |x = (class A; <flow>; y; end)| x }",
        "foo { |x = (module A; <flow>; y; end)| x }",
        "foo { |x = (class << obj; <flow>; y; end)| x }",
        "foo { |x = (<flow>; y)| next }",
        "foo { |x = (<flow>; y)| while true; break; end }",
        "foo { |x = (<flow>; y)| bar {} }",
        "foo { |x = (<flow>; y)| x }; while true; break; end",
        "foo { |x = (<flow>; y)| x } if test",
        "foo { |x = (<flow>; y)| x } rescue nil",
        "while true; def f; foo { |x = (<flow>; y)| x }; end; end",
        "outer { def f; foo { |x = (<flow>; y)| x }; end }",
        "-> { def f; foo { |x = (<flow>; y)| x }; end }",
        "->(x = (<flow>; y)) { x }",
        "foo { |x = (<<~TEXT)| x }\n#{<flow>; y}\nTEXT\n",
    ];
    let valid = [
        "foo { |x = 1| <flow>; y }",
        "foo do |x: 1| <flow>; y end",
        "while true; foo { |x = (<flow>; y)| x }; end",
        "until true; foo { |x: (<flow>; y)| x }; end",
        "for y in []; foo { |x = (<flow>; y)| x }; end",
        "outer { foo { |x = (<flow>; y)| x } }",
        "outer do; foo { |x: (<flow>; y)| x }; end",
        "-> { foo { |x = (<flow>; y)| x } }",
        "-> do; foo { |x = (<flow>; y)| x }; end",
        "foo { |x = (<flow>; y)| x } while true",
        "foo { |x = (<flow>; y)| x } until true",
        "begin; foo { |x = (<flow>; y)| x }; end while true",
        "foo { |x = (<flow> while true)| x }",
        "foo { |x = (<flow> until true)| x }",
        "foo { |x = (begin; <flow>; y; end while true)| x }",
        "foo { |x = (while true; <flow>; y; end)| x }",
        "foo { |x = (until true; <flow>; y; end)| x }",
        "foo { |x = (for y in []; <flow>; y; end)| x }",
        "foo { |x = bar { <flow>; y }| x }",
        "foo { |x = bar do; <flow>; y; end| x }",
        "foo { |x = -> { <flow>; y }| x }",
        "foo { |x = -> do; <flow>; y; end| x }",
        "foo { |x = (def f; bar { <flow>; y }; end)| x }",
        "foo { |x = defined?(<flow>)| x }",
        "defined?(foo { |x = (<flow>; y)| x })",
        "outer { foo { |x = ->(y = (<flow>; z)) { y }| x } }",
        "foo { |x = (bar { foo { |y = (<flow>; z)| y } })| x }",
        "outer do; y; rescue; foo { |x = (<flow>; y)| x }; end",
        "outer do; y; ensure; foo { |x = (<flow>; y)| x }; end",
        "foo { |x = (\"<flow>\")| x }",
        "foo { |x = (/<flow>/)| x }",
        "# foo { |x = (<flow>; y)| x }\nfoo { |x = 1| x }",
        "foo { |x = (<<'TEXT')| x }\n<flow>\nTEXT\n",
        "foo { |x = 1| x }\n__END__\nfoo { |x = (<flow>; y)| x }",
    ];
    let mut cases = Vec::new();
    for keyword in ["next", "break", "redo"] {
        for (templates, valid) in [(&invalid[..], false), (&valid[..], true)] {
            for template in templates {
                cases.push(case(template, keyword, valid));
            }
        }
        for template in [
            "foo { |x = (<flow> 42; y)| x }",
            "foo { |x: (<flow> :value; y)| x }",
        ] {
            if keyword != "redo" {
                let mut c = case(template, keyword, false);
                c.range.end = c.source.iter().position(|&b| b == b';').unwrap();
                cases.push(c);
            }
        }
        for (prefix, suffix) in [
            (b"\xef\xbb\xbf'\xc3\xa9'; ".as_slice(), b"\r\n".as_slice()),
            (b"# coding: UTF-8\r\n'\xc3\xa9'; ", b"\r\n"),
            (b"# coding: Windows-1252\r\n'\xe9'; ", b"\r\n"),
            (b"# coding: Shift_JIS\r\n'\x82\xa0'; ", b"\r\n"),
            (b"# coding: ASCII-8BIT\r\n'\xff'; ", b"\r\n"),
        ] {
            for (template, valid) in [
                ("def f; foo { |x = (<flow>; y)| x }; end", false),
                ("foo { |x = 1| <flow>; y }", true),
                ("foo { |x = bar { <flow>; y }| x }", true),
            ] {
                let mut c = case(template, keyword, valid);
                c.range = c.range.start + prefix.len()..c.range.end + prefix.len();
                c.source = [prefix, &c.source, suffix].concat();
                cases.push(c);
            }
        }
    }
    cases
}
