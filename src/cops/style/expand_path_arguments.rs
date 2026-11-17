//! `Style/ExpandPathArguments` — canonical current-file expand_path arguments.
use super::file_api::{argument, no_arguments, replacement, send};
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ExpandPathArguments;
impl Cop for ExpandPathArguments {
    fn name(&self) -> &'static str {
        "Style/ExpandPathArguments"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node
            .as_call_node()
            .filter(|c| !c.is_safe_navigation() && c.name().as_slice() == b"expand_path")
        else {
            return;
        };
        let args = ast::call_arguments(&call);
        if call
            .receiver()
            .is_some_and(|r| ast::is_global_const(&r, &["File"]))
        {
            let [path, base] = args.as_slice() else {
                return;
            };
            if base.as_source_file_node().is_none() {
                return;
            }
            let Some(string) = path.as_string_node() else {
                return;
            };
            let current = String::from_utf8_lossy(string.unescaped());
            let mut parts: Vec<_> = current
                .split('/')
                .filter(|p| !p.is_empty() && *p != ".")
                .collect();
            let depth = parts.len();
            let canonical = std::str::from_utf8(string.unescaped()).is_ok()
                && std::str::from_utf8(path.location().as_slice()).is_ok()
                && (string.unescaped().is_ascii() || file.is_utf8())
                && !current.starts_with('/')
                && (parts.is_empty() || parts.first() == Some(&".."))
                && !current.bytes().any(|b| b.is_ascii_control());
            if let Some(index) = parts.iter().position(|p| *p == "..") {
                parts.remove(index);
            }
            let parent = parts.join("/");
            // Removing the parent component must not expose Ruby's leading-~
            // home expansion: ../~user is an ordinary relative path.
            let parent = if parent.starts_with('~') {
                format!("./{parent}")
            } else {
                parent
            };
            let new_args = if depth == 0 {
                "__FILE__".into()
            } else if depth == 1 {
                "__dir__".into()
            } else {
                format!("{}, __dir__", quote(&parent))
            };
            let start = call.message_loc().unwrap().start_offset();
            let prefix = &file.bytes[node.location().start_offset()..start];
            let text = std::str::from_utf8(prefix)
                .ok()
                .map(|p| format!("{p}expand_path({new_args})"));
            let correction = (cx.corrections_requested()
                && canonical
                && (depth == 0 || !file.noncanonical_load_path()))
            .then(|| {
                replacement(
                    node,
                    cx,
                    file,
                    text?,
                    if depth == 0 { "FilePath" } else { "File" },
                )
            })
            .flatten();
            let (line, col) = file.line_col(start);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: format!(
                    "Use `expand_path({new_args})` instead of `expand_path({}, __FILE__)`.",
                    quote(&current)
                ),
                line,
                col,
                end_line: file.line_of(call.message_loc().unwrap().end_offset() - 1),
                correction,
            });
            return;
        }
        if !no_arguments(&call) {
            return;
        }
        let Some(parent) = call
            .receiver()
            .and_then(|n| send(&n))
            .filter(|c| c.name().as_slice() == b"parent" && no_arguments(c))
        else {
            return;
        };
        let Some(constructor) = parent.receiver().and_then(|n| send(&n)) else {
            return;
        };
        let factory =
            constructor.name().as_slice() == b"Pathname" && constructor.receiver().is_none();
        let new = constructor.name().as_slice() == b"new"
            && constructor
                .receiver()
                .is_some_and(|n| ast::is_global_const(&n, &["Pathname"]));
        if !factory && !new {
            return;
        }
        let Some(base) = argument(&constructor).filter(|n| n.as_source_file_node().is_some())
        else {
            return;
        };
        let start = node.location().start_offset();
        let end = ast::source_end(node);
        let (line, col) = file.line_col(start);
        let name = if factory { "Pathname" } else { "Pathname.new" };
        let correction = (cx.corrections_requested() && !file.noncanonical_load_path())
            .then(|| {
                let before =
                    std::str::from_utf8(&file.bytes[start..base.location().start_offset()]).ok()?;
                let middle = std::str::from_utf8(
                    &file.bytes[base.location().end_offset()..constructor.location().end_offset()],
                )
                .ok()?;
                let after =
                    std::str::from_utf8(&file.bytes[parent.location().end_offset()..end]).ok()?;
                replacement(
                    node,
                    cx,
                    file,
                    format!("{before}__dir__{middle}{after}"),
                    "Pathname",
                )
            })
            .flatten();
        let message = format!(
            "Use `{name}(__dir__).expand_path` instead of `{name}(__FILE__).parent.expand_path`."
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            correction,
        });
    }
}
fn quote(path: &str) -> String {
    format!("'{}'", path.replace('\\', "\\\\").replace('\'', "\\'"))
}
