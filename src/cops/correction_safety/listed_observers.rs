//! The listed-observer rule for all 613 registered cops. The closed
//! list of source observers withholds every correction, whichever cop emits
//! it (all nine departments) and whichever callback emits it; findings stay.
//! Lookalikes outside the list keep the edit.
use super::*;

/// A same-length edit of the last line's `1`, emitted from the source, node
/// and leave callbacks under a real registered cop's name.
struct Named(&'static str);
impl Named {
    fn emit(&self, file: &SourceFile, out: &mut Vec<Offense>, message: &str) {
        let at = file
            .bytes
            .windows(6)
            .rposition(|w| w == b"x = 1\n")
            .unwrap()
            + 4;
        out.push(Offense {
            cop: self.0,
            severity: crate::offense::Severity::Convention,
            line: 1,
            col: 1,
            end_line: 1,
            message: message.into(),
            correction: Some(crate::offense::Correction {
                range: at..at + 1,
                replacement: "2".into(),
            }),
        });
    }
}
impl super::Cop for Named {
    fn name(&self) -> &'static str {
        self.0
    }
    fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
        self.emit(file, out, "source");
    }
    fn wants_leave_node(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &ruby_prism::Node<'pr>,
        _: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_program_node().is_some() {
            self.emit(file, out, "node");
        }
    }
    fn leave_node<'pr>(
        &self,
        node: &ruby_prism::Node<'pr>,
        _: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_program_node().is_some() {
            self.emit(file, out, "leave");
        }
    }
}

/// `(prefix, data section, held)`: the whole closed list, rooted, reflected
/// and nested spellings, and lookalikes that name none of them.
const OBSERVERS: [(&str, bool, bool); 31] = [
    ("p __FILE__\n", false, true),
    ("def m = __FILE__\n", false, true),
    ("p __dir__\n", false, true),
    ("p DATA.read\n", true, true),
    ("p ::DATA.pos\n", true, true),
    ("p $0\n", false, true),
    ("$0 = 'name'\n", false, true),
    ("p $PROGRAM_NAME\n", false, true),
    ("$PROGRAM_NAME = 'name'\n", false, true),
    ("p caller.first\n", false, true),
    ("p caller_locations(0).first.lineno\n", false, true),
    ("def m = 1\np method(:m).source_location\n", false, true),
    ("p proc {}.source_location\n", false, true),
    ("SCRIPT_LINES__ = {}\n", false, true),
    ("p SCRIPT_LINES__\n", false, true),
    ("p ::SCRIPT_LINES__\n", false, true),
    ("p Object.const_get(:SCRIPT_LINES__)\n", false, true),
    ("if false\n  p SCRIPT_LINES__\nend\n", false, true),
    // Rooted and namespaced parallel-assignment targets.
    ("::SCRIPT_LINES__, unused = {}, nil\n", false, true),
    ("Object::SCRIPT_LINES__, unused = {}, nil\n", false, true),
    ("::DATA, unused = DATA, nil\n", true, true),
    ("::DATA, unused = 1, 2\n", true, true),
    // Lookalikes: no listed observer, the edit stays.
    ("p 1\n", false, false),
    ("p SCRIPT_LINES\n", false, false),
    ("p '__FILE__ $0 caller'\n", false, false),
    ("# __dir__ SCRIPT_LINES__ $PROGRAM_NAME\n", false, false),
    ("p :caller_name, $PROGRAM\n", false, false),
    ("p DATA_DIR\n", false, false),
    // `DATA` without a data section is an ordinary (missing) constant.
    ("p DATA\n", false, false),
    ("p __method__, __LINE__\n", false, false),
    ("p $1, $PROGRAM_FILE\n", false, false),
];

#[test]
fn every_registered_cop_withholds_corrections_for_listed_observers() {
    let registry = Registry::new();
    let names: Vec<&'static str> = registry.cops().iter().map(|cop| cop.name()).collect();
    assert_eq!(names.len(), 613);
    let departments: std::collections::BTreeSet<_> = names
        .iter()
        .map(|name| name.split('/').next().unwrap())
        .collect();
    assert_eq!(departments.len(), 9, "{departments:?}");
    for (prefix, data, held) in OBSERVERS {
        let tail = if data { "__END__\ndata\n" } else { "" };
        let source = format!("{prefix}x = 1\n{tail}");
        for &name in &names {
            let found = collect(&Named(name), &source);
            let mut messages: Vec<_> = found.iter().map(|o| o.message.as_str()).collect();
            messages.sort_unstable();
            assert_eq!(messages, ["leave", "node", "source"], "{name}: {prefix:?}");
            assert!(
                found.iter().all(|o| o.correction.is_some() != held),
                "{name}: {prefix:?}: held={held}: {found:?}"
            );
        }
    }
}
