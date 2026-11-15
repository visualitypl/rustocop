//! The `#to_s` counterpart of a fixture that reads `#source`, for the runtime
//! comparisons of canonical edits.
//! deliberately does not infer receiver types through locals/parameters, even
//! in test instrumentation: they may hold a pre-entry retained AST in real code.
use std::path::Path;

pub fn pattern_sibling(_dir: &Path, _file: &str, source: &str, _cops: &str) -> String {
    // Regexp#to_s still exposes the decoded pattern and inline options; the
    // paired runtime executions compare it, encoding, flags and match behavior.
    source.replace(".source", ".to_s")
}
