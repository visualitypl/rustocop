//! Correction-only position metadata, distinct from raw-source consumption.
//!
//! Folded into the existing single lazy main-AST observer walk. No receiver
//! flow is assumed: acquisitions and selectors can meet through aliases or
//! deferred bodies. Deltas are checked individually, never balanced against
//! another correction that conflict resolution or directives might discard.
use crate::{offense::Correction, source::SourceFile};
use ruby_prism::Node;

fn test_flag(node: &Node<'_>) -> Option<u8> {
    if let Some(string) = node.as_string_node() {
        return match string.unescaped() {
            [flag] => Some(*flag),
            _ => None,
        };
    }
    let integer = node.as_integer_node()?;
    let value = integer.value();
    let (negative, digits) = value.to_u32_digits();
    match digits {
        [flag] if !negative => u8::try_from(*flag).ok(),
        _ => None,
    }
}

fn input_line_number(name: &[u8]) -> bool {
    matches!(name, b"$." | b"$INPUT_LINE_NUMBER")
}

enum MetadataTarget {
    Unknown,
    // A known filesystem API with an unresolved path can address this source
    // without any separate __FILE__/$0 acquisition in the AST.
    PossibleFile,
    NonFile,
    Literals(Vec<Vec<u8>>),
}

/// Resolve only literal forwarding dispatches, consuming each argument once.
/// Method objects/aliases with later arguments require flow and remain unknown.
fn dispatched_argument<'pr>(
    call: &ruby_prism::CallNode<'pr>,
    selector: fn(&[u8]) -> bool,
    index: usize,
) -> Option<Node<'pr>> {
    let list = call.arguments()?.arguments();
    let mut arguments = list.iter();
    if selector(call.name().as_slice()) {
        return arguments.nth(index);
    }
    if !matches!(
        call.name().as_slice(),
        b"send" | b"__send__" | b"public_send"
    ) {
        return None;
    }
    loop {
        let node = arguments.next()?;
        let symbol = node.as_symbol_node();
        let string = node.as_string_node();
        let name = symbol
            .as_ref()
            .map(|s| s.unescaped())
            .or_else(|| string.as_ref().map(|s| s.unescaped()))?;
        if selector(name) {
            return arguments.nth(index);
        }
        if !matches!(name, b"send" | b"__send__" | b"public_send") {
            return None;
        }
    }
}

fn literal_metadata_target(node: Option<Node<'_>>) -> MetadataTarget {
    let Some(path) = node.and_then(|n| n.as_string_node()) else {
        return MetadataTarget::Unknown;
    };
    let Ok(path) = std::str::from_utf8(path.unescaped()) else {
        return MetadataTarget::Unknown;
    };
    // Literal unrelated files must not be connected to an incidental __FILE__.
    // Same basenames are uncertain (relative/absolute/aliased loads), so retain
    // the guard for them. No filesystem probing or path/receiver flow analysis.
    std::path::Path::new(path)
        .file_name()
        .map_or(MetadataTarget::Unknown, |name| {
            MetadataTarget::Literals(vec![name.as_encoded_bytes().to_vec()])
        })
}

fn metadata_paths(first: Option<Node<'_>>, second: Option<Node<'_>>) -> MetadataTarget {
    let first = literal_metadata_target(first);
    let Some(second) = second else { return first };
    match (first, literal_metadata_target(Some(second))) {
        (MetadataTarget::Literals(mut first), MetadataTarget::Literals(second)) => {
            first.extend(second);
            MetadataTarget::Literals(first)
        }
        _ => MetadataTarget::Unknown,
    }
}

fn metadata_target(call: &ruby_prism::CallNode<'_>) -> MetadataTarget {
    use super::ast;
    // Kernel.test is also callable on self or through an unresolved receiver.
    // No receiver flow is needed: classify all literal test dispatches by their
    // paths, retaining unrelated-literal exemptions and uncertain computed paths.
    if let Some(path) = dispatched_argument(call, |s| s == b"test", 1) {
        let second = dispatched_argument(call, |s| s == b"test", 2);
        return match metadata_paths(Some(path), second) {
            MetadataTarget::Unknown => MetadataTarget::PossibleFile,
            target => target,
        };
    }
    // Zero/one fixed argument cannot observe metadata. Splat/forwarding
    // can supply a complete flag/path tuple without spelling either here.
    if literal_dispatch(call, |s| s == b"test", false)
        && call.arguments().is_some_and(|a| {
            a.arguments().iter().any(|n| {
                matches!(
                    n,
                    Node::SplatNode { .. } | Node::ForwardingArgumentsNode { .. }
                )
            })
        })
    {
        return MetadataTarget::PossibleFile;
    }
    let Some(receiver) = call.receiver() else {
        return MetadataTarget::Unknown;
    };
    if matches!(
        receiver,
        Node::ArrayNode { .. } | Node::HashNode { .. } | Node::StringNode { .. }
    ) {
        return MetadataTarget::NonFile;
    }
    let inner = receiver.as_call_node();
    let target = if let Some(inner) = &inner {
        if matches!(inner.name().as_slice(), b"children" | b"entries")
            && inner
                .receiver()
                .is_some_and(|r| ast::is_global_const(&r, &["Dir"]))
        {
            return MetadataTarget::NonFile;
        }
        if stat_invocation(inner) {
            inner
        } else {
            return MetadataTarget::Unknown;
        }
    } else {
        call
    };
    if !target.receiver().is_some_and(|r| {
        ast::is_global_const(&r, &["File"])
            || ast::is_global_const(&r, &["FileTest"])
            || direct_stat_class(&r)
    }) {
        return MetadataTarget::Unknown;
    }
    let second = dispatched_argument(target, |s| s == b"identical?", 1);
    match metadata_paths(
        dispatched_argument(
            target,
            |s| {
                matches!(
                    s,
                    b"new"
                        | b"initialize"
                        | b"size"
                        | b"size?"
                        | b"stat"
                        | b"lstat"
                        | b"atime"
                        | b"mtime"
                        | b"ctime"
                        | b"birthtime"
                        | b"ino"
                        | b"nlink"
                        | b"identical?"
                )
            },
            0,
        ),
        second,
    ) {
        MetadataTarget::Unknown => MetadataTarget::PossibleFile,
        target => target,
    }
}

/// Resolve actual direct/literal-forwarded calls, not Method/proc acquisition.
/// The projection caller additionally requires no arguments and no block.
fn literal_dispatch(
    call: &ruby_prism::CallNode<'_>,
    selector: fn(&[u8]) -> bool,
    zero_arguments: bool,
) -> bool {
    let list = call.arguments().map(|a| a.arguments());
    let mut arguments = list.as_ref().into_iter().flat_map(|a| a.iter());
    let name = call.name().as_slice();
    if selector(name) {
        return !zero_arguments || arguments.next().is_none();
    }
    if !matches!(name, b"send" | b"__send__" | b"public_send") {
        return false;
    }
    loop {
        let Some(node) = arguments.next() else {
            return false;
        };
        let symbol = node.as_symbol_node();
        let string = node.as_string_node();
        let Some(name) = symbol
            .as_ref()
            .map(|s| s.unescaped())
            .or_else(|| string.as_ref().map(|s| s.unescaped()))
        else {
            return false;
        };
        if selector(name) {
            return !zero_arguments || arguments.next().is_none();
        }
        if !matches!(name, b"send" | b"__send__" | b"public_send") {
            return false;
        }
    }
}

fn stat_projection(name: &[u8]) -> bool {
    // Atomic replacement preserves file type (and follows input symlinks).
    // Other scalar getters are deliberately not treated as invariant: inode,
    // ownership, allocation and timestamps can change without a byte delta.
    matches!(
        name,
        b"size"
            | b"size?"
            | b"file?"
            | b"directory?"
            | b"blockdev?"
            | b"chardev?"
            | b"pipe?"
            | b"socket?"
            | b"symlink?"
            | b"ftype"
    )
}

fn stat_invocation(call: &ruby_prism::CallNode<'_>) -> bool {
    literal_dispatch(call, |s| matches!(s, b"stat" | b"lstat"), false)
        || call.receiver().is_some_and(|r| direct_stat_class(&r))
            && literal_dispatch(call, |s| matches!(s, b"new" | b"initialize"), false)
}

fn filesystem_name(name: &[u8]) -> bool {
    matches!(
        name,
        b"File" | b"FileTest" | b"IO" | b"Pathname" | b"File::Stat" | b"Kernel"
    ) || [
        b"::File".as_slice(),
        b"::FileTest",
        b"::IO",
        b"::Pathname",
        b"::File::Stat",
        b"::Kernel",
    ]
    .iter()
    .any(|suffix| name.ends_with(suffix))
}

fn filesystem_api(node: &Node<'_>) -> bool {
    node.as_constant_read_node()
        .is_some_and(|c| filesystem_name(c.name().as_slice()))
        || node
            .as_constant_path_node()
            .is_some_and(|c| c.name().is_some_and(|n| filesystem_name(n.as_slice())))
        || node.as_call_node().is_some_and(|c| {
            dispatched_argument(&c, |s| s == b"const_get", 0).is_some_and(|n| {
                n.as_symbol_node()
                    .is_some_and(|s| filesystem_name(s.unescaped()))
                    || n.as_string_node()
                        .is_some_and(|s| filesystem_name(s.unescaped()))
            })
        })
}

fn direct_stat_class(node: &Node<'_>) -> bool {
    node.as_constant_path_node().is_some_and(|c| {
        c.name().is_some_and(|n| n.as_slice() == b"Stat")
            && c.parent().is_some_and(|p| file_constant(&p))
    })
}

fn file_constant(node: &Node<'_>) -> bool {
    node.as_constant_read_node()
        .is_some_and(|c| c.name().as_slice() == b"File")
        || node
            .as_constant_path_node()
            .is_some_and(|c| c.name().is_some_and(|n| n.as_slice() == b"File"))
}

fn stat_class(node: &Node<'_>) -> bool {
    direct_stat_class(node)
        || node.as_call_node().is_some_and(|c| {
            dispatched_argument(&c, |s| s == b"const_get", 0).is_some_and(|n| {
                let symbol = n.as_symbol_node();
                let string = n.as_string_node();
                symbol
                    .as_ref()
                    .map(|s| s.unescaped())
                    .or_else(|| string.as_ref().map(|s| s.unescaped()))
                    .is_some_and(|name| {
                        name == b"File::Stat"
                            || name.ends_with(b"::File::Stat")
                            || name == b"Stat" && c.receiver().is_some_and(|r| file_constant(&r))
                    })
            })
        })
}

fn filesystem_alias(name: &[u8]) -> bool {
    // File, Pathname and FileUtils spellings; aliases/reflection use the shared
    // acquisition classifier. After visible alias creation no literal basename
    // mismatch proves independence. Do not probe the host filesystem.
    matches!(
        name,
        b"symlink"
            | b"link"
            | b"rename"
            | b"mv"
            | b"move"
            | b"make_symlink"
            | b"make_link"
            | b"ln"
            | b"ln_s"
            | b"ln_sf"
            | b"ln_sr"
            | b"link_entry"
    )
}

fn indirect_physical_observer(call: &ruby_prism::CallNode<'_>) -> bool {
    let Some(node) = dispatched_argument(call, |s| s == b"const_get", 0) else {
        return false;
    };
    let symbol = node.as_symbol_node();
    let string = node.as_string_node();
    symbol
        .as_ref()
        .map(|s| s.unescaped())
        .or_else(|| string.as_ref().map(|s| s.unescaped()))
        .is_some_and(|name| {
            matches!(name, b"TracePoint" | b"RubyVM")
                || name.ends_with(b"::TracePoint")
                || name.ends_with(b"::RubyVM")
        })
}

#[derive(Default)]
pub(crate) struct Facts {
    data_bytes: bool,
    data_lines: bool,
    physical_lines: bool,
    line_values: Vec<usize>,
    size: bool,
    timestamp: bool,
    possible_file_size: bool,
    possible_file_timestamp: bool,
    filesystem_api: bool,
    stat_class: bool,
    unknown_stat_constructor: bool,
    aliases_paths: bool,
    // Parents are entered before children by the existing walk. A receiver has
    // exactly one parent, so these offsets need no value flow or second pass.
    // true = size projection; false = invariant file-type projection.
    projected_stats: std::collections::HashMap<(usize, usize), bool>,
    literal_sizes: std::collections::HashSet<Vec<u8>>,
    literal_timestamps: std::collections::HashSet<Vec<u8>>,
}

impl Facts {
    pub(crate) fn observe(&mut self, node: &Node<'_>) {
        use super::style::class_methods::position_acquisition_observes as selector;
        self.data_bytes |= selector(node, |s| matches!(s, b"pos" | b"tell"));
        self.data_lines |= selector(node, |s| s == b"lineno")
            || node
                .as_global_variable_read_node()
                .is_some_and(|g| input_line_number(g.name().as_slice()))
            || node
                .as_global_variable_or_write_node()
                .is_some_and(|g| input_line_number(g.name().as_slice()))
            || node
                .as_global_variable_and_write_node()
                .is_some_and(|g| input_line_number(g.name().as_slice()))
            || node
                .as_global_variable_operator_write_node()
                .is_some_and(|g| input_line_number(g.name().as_slice()));
        // Constants can escape into aliases/deferred calls. This is bounded
        // acquisition evidence, not receiver or path value flow.
        self.filesystem_api |= filesystem_api(node);
        self.stat_class |= stat_class(node);
        self.aliases_paths |= selector(node, filesystem_alias);
        let constructor = selector(node, |s| matches!(s, b"new" | b"initialize" | b"allocate"));
        let constructs_stat = node
            .as_call_node()
            .and_then(|c| c.receiver())
            .is_some_and(|r| direct_stat_class(&r));
        self.unknown_stat_constructor |= constructor && !constructs_stat;
        let test = selector(node, |s| s == b"test");
        // Acquiring Kernel's test via a Method/proc/alias leaves later path and
        // flag arguments unknown. Direct zero/one-argument lookalikes cannot
        // observe metadata, but unknown flag/path tuples are not external proofs.
        if test
            && !node
                .as_call_node()
                .is_some_and(|c| literal_dispatch(&c, |s| s == b"test", false))
        {
            self.possible_file_timestamp = true;
        }
        let flag = node
            .as_call_node()
            .and_then(|c| dispatched_argument(&c, |s| s == b"test", 0))
            .and_then(|arg| test_flag(&arg));
        if let Some(call) = node.as_call_node()
            && call.block().is_none()
            && literal_dispatch(&call, stat_projection, true)
            && let Some(receiver) = call.receiver().and_then(|r| r.as_call_node())
            && stat_invocation(&receiver)
        {
            let loc = receiver.location();
            self.projected_stats.insert(
                (loc.start_offset(), loc.end_offset()),
                literal_dispatch(&call, |s| matches!(s, b"size" | b"size?"), true),
            );
        }
        let stat =
            selector(node, |s| matches!(s, b"stat" | b"lstat")) || constructor && constructs_stat;
        let loc = node.location();
        let projection = self
            .projected_stats
            .get(&(loc.start_offset(), loc.end_offset()));
        let size = selector(node, |s| matches!(s, b"size" | b"size?"))
            || stat && projection == Some(&true)
            || (test && flag.is_none_or(|flag| flag == b's'));
        // Atomic replacement changes inode timestamps even for equal-size edits.
        let timestamp = selector(node, |s| {
            matches!(
                s,
                b"atime" | b"mtime" | b"ctime" | b"birthtime" | b"ino" | b"nlink" | b"identical?"
            )
        }) || (stat && projection.is_none())
            || (test
                && flag.is_none_or(|flag| {
                    matches!(flag, b'A' | b'M' | b'C' | b'=' | b'<' | b'>' | b'-')
                }));
        match node.as_call_node().map(|c| metadata_target(&c)) {
            Some(MetadataTarget::Literals(paths)) => {
                for path in paths {
                    if size {
                        self.literal_sizes.insert(path.clone());
                    }
                    if timestamp {
                        self.literal_timestamps.insert(path);
                    }
                }
            }
            Some(MetadataTarget::PossibleFile) => {
                self.possible_file_size |= size;
                self.possible_file_timestamp |= timestamp;
            }
            Some(MetadataTarget::NonFile) => {}
            _ => {
                self.size |= size;
                self.timestamp |= timestamp;
            }
        }
        self.physical_lines |= selector(node, |s| {
            matches!(
                s,
                b"caller"
                    | b"caller_locations"
                    | b"backtrace"
                    | b"backtrace_locations"
                    | b"source_location"
                    | b"const_source_location"
                    | b"set_trace_func"
            )
        }) || node
            .as_constant_read_node()
            .is_some_and(|c| matches!(c.name().as_slice(), b"TracePoint" | b"RubyVM"))
            || node.as_constant_path_node().is_some_and(|c| {
                c.name()
                    .is_some_and(|n| matches!(n.as_slice(), b"TracePoint" | b"RubyVM"))
            })
            || node
                .as_call_node()
                .is_some_and(|c| indirect_physical_observer(&c));
        if matches!(node, Node::SourceLineNode { .. }) {
            self.line_values.push(node.location().start_offset());
        }
    }

    pub(crate) fn finish(&mut self) {
        // Heredoc children need not be visited in physical source order.
        self.line_values.sort_unstable();
    }

    pub(crate) fn preserves(
        &self,
        correction: &Correction,
        file: &SourceFile,
        data_start: Option<usize>,
        acquires_data: bool,
        acquires_path: bool,
    ) -> bool {
        let range = &correction.range;
        if range.start > range.end || range.end > file.bytes.len() {
            // Leave malformed corrections to the application layer's error path.
            return true;
        }
        let basename = file.path.file_name().map(|name| name.as_encoded_bytes());
        let may_address_self = acquires_path || self.filesystem_api;
        let timestamp = self.possible_file_timestamp
            || self.stat_class && self.unknown_stat_constructor
            || self.timestamp && may_address_self
            || self.aliases_paths && !self.literal_timestamps.is_empty()
            || basename.is_some_and(|name| self.literal_timestamps.contains(name));
        let size = self.possible_file_size
            || self.size && may_address_self
            || self.aliases_paths && !self.literal_sizes.is_empty()
            || basename.is_some_and(|name| self.literal_sizes.contains(name));
        if timestamp {
            return false;
        }
        let replacement = correction.replacement.as_bytes();
        let same_size = range.len() == replacement.len();
        if !same_size && size {
            return false;
        }
        let before_data = acquires_data && data_start.is_some_and(|end| range.start <= end);
        if before_data && self.data_bytes && !same_size {
            return false;
        }
        let first = self
            .line_values
            .partition_point(|&offset| offset < range.start);
        if !(before_data && self.data_lines)
            && !self.physical_lines
            && first == self.line_values.len()
        {
            return true;
        }
        // The existing line index answers old-span counts in logarithmic work;
        // only newly emitted replacement bytes are traversed, not source spans.
        let old_lines = file.line_of(range.end) - file.line_of(range.start);
        let new_lines = memchr::memchr_iter(b'\n', replacement).count();
        if before_data && self.data_lines && old_lines as usize != new_lines {
            return false;
        }
        // Equal totals cannot prove locations/events *inside* a replacement.
        // Global stack/trace/method observers conservatively require single-line
        // spans; byte-only edits still fix. __LINE__ has exact parsed locations,
        // so multiline wrappers with unchanged observed line ordinals can fix.
        if self.physical_lines && (old_lines != 0 || new_lines != 0) {
            return false;
        }
        if first == self.line_values.len() {
            return true;
        }
        if old_lines as usize != new_lines {
            return false;
        }
        let last = self
            .line_values
            .partition_point(|&offset| offset < range.end);
        let markers = &self.line_values[first..last];
        if markers.is_empty() {
            return true;
        }
        let mut markers = markers.iter();
        let mut previous = 0;
        let mut line = file.line_of(range.start);
        for offset in memchr::memmem::find_iter(replacement, b"__LINE__") {
            line += memchr::memchr_iter(b'\n', &replacement[previous..offset]).count() as u32;
            if markers
                .next()
                .is_none_or(|&original| file.line_of(original) != line)
            {
                return false;
            }
            previous = offset;
        }
        markers.next().is_none()
    }
}
