//! A fixed depth budget for recursive AST consumers. Check the parsed program
//! before directives or cops run: stack overflow is an abort, not a catchable
//! cop panic. This preflight uses Prism's C child enumerator so it sees typed
//! children too, without the large frames of the Rust scope analyzer.

use std::{ffi::c_void, fmt};

use ruby_prism::{Node, ProgramNode};
use ruby_prism_sys::pm_node_t;

/// Counts every AST node on a path, including Program/Statements/Arguments.
/// Each file is checked and linted on a [`STACK_SIZE`] segment; release
/// builds were measured at under 1.5 KB of stack per level for the deepest
/// shapes (nested blocks, definitions, calls), which leaves a wide margin.
/// This is not a configurable Ruby/style limit.
pub const MAX_AST_DEPTH: usize = 4096;

/// The stack a file's parse and lint run on, independent of the calling
/// thread's (a Rayon worker's is 2 MiB). Only the pages used are committed.
pub const STACK_SIZE: usize = 64 * 1024 * 1024;

/// The first node (in Prism field order) beyond the supported depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepthError {
    /// Byte offset in the original source, for the file error's position.
    pub offset: usize,
}

impl fmt::Display for DepthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AST exceeds maximum depth of {MAX_AST_DEPTH}")
    }
}

impl std::error::Error for DepthError {}

// ruby-prism-sys does not currently expose this public Prism API.
unsafe extern "C" {
    fn pm_visit_child_nodes(
        node: *const pm_node_t,
        visitor: unsafe extern "C" fn(*const pm_node_t, *mut c_void) -> bool,
        data: *mut c_void,
    );
}

/// Reject excessive depth before invoking any recursive Rust analysis.
/// The preflight itself recurses at most MAX_AST_DEPTH + 1 times, with small
/// frames, and never descends into the excessive subtree.
pub fn check(program: &ProgramNode<'_>) -> Result<(), DepthError> {
    check_at_limit(program, MAX_AST_DEPTH)
}

fn check_at_limit(program: &ProgramNode<'_>, limit: usize) -> Result<(), DepthError> {
    let Node::ProgramNode { pointer, .. } = program.as_node() else {
        unreachable!()
    };
    let location = program.location();
    let mut state = State {
        limit,
        depth: 0,
        start: location.as_slice().as_ptr(),
        start_offset: location.start_offset(),
        error: None,
    };
    // SAFETY: the program's ParseResult keeps its tree/source live throughout
    // this synchronous traversal. The callback only reads nodes and mutates
    // this stack-local state; neither pointer escapes the call.
    unsafe { visit(pointer.cast(), (&mut state as *mut State).cast()) };
    state.error.map_or(Ok(()), Err)
}

struct State {
    limit: usize,
    depth: usize,
    start: *const u8,
    start_offset: usize,
    error: Option<DepthError>,
}

unsafe extern "C" fn visit(node: *const pm_node_t, data: *mut c_void) -> bool {
    // SAFETY: only check/this callback supply data, pointing to a live State.
    // Child enumeration is synchronous. End the mutable borrow before entering
    // C again, since it will reborrow this same state in the next callback.
    let state = unsafe { &mut *data.cast::<State>() };
    if state.error.is_some() {
        return false;
    }
    if state.depth == state.limit {
        // SAFETY: Prism supplies a live child node; its location and the root
        // location point into the same source allocation (kept alive by check).
        let offset = unsafe { (*node).location.start.offset_from(state.start) };
        state.error = Some(DepthError {
            offset: state.start_offset.saturating_add_signed(offset),
        });
        return false;
    }
    state.depth += 1;
    // SAFETY: node/data retain the validity described above. Returning false
    // below disables Prism's automatic recursion: only this bounded call recurses.
    unsafe { pm_visit_child_nodes(node, visit, data) };
    unsafe { &mut *data.cast::<State>() }.depth -= 1;
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cops::{
        testing::{ERROR_TREE_TYPED_CHILDREN, FULL_TREE, TYPED_CHILD_CASES},
        walk::{NodeVisitor, walk},
    };

    #[derive(Default)]
    struct Depths {
        depth: usize,
        nodes: Vec<(usize, usize)>,
    }

    impl<'pr> NodeVisitor<'pr> for Depths {
        fn enter(&mut self, node: Node<'pr>) {
            self.depth += 1;
            self.nodes
                .push((node.location().start_offset(), self.depth));
        }

        fn leave(&mut self) {
            self.depth -= 1;
        }
    }

    #[test]
    fn preflight_counts_all_nodes_including_typed_and_recovered_children() {
        for source in std::iter::once(FULL_TREE)
            .chain(TYPED_CHILD_CASES.iter().map(|(source, _)| *source))
            .chain(ERROR_TREE_TYPED_CHILDREN.iter().copied())
        {
            let parsed = ruby_prism::parse(source.as_bytes());
            let root = parsed.node();
            let program = root.as_program_node().unwrap();
            let mut depths = Depths::default();
            walk(&mut depths, &root);
            let max = depths.nodes.iter().map(|(_, depth)| *depth).max().unwrap();
            for limit in 1..=max + 1 {
                let expected = depths
                    .nodes
                    .iter()
                    .find(|(_, depth)| *depth > limit)
                    .map(|(offset, _)| DepthError { offset: *offset });
                assert_eq!(
                    check_at_limit(&program, limit),
                    expected.map_or(Ok(()), Err),
                    "limit {limit}: {source}"
                );
            }
        }
    }

    #[test]
    fn accepts_exact_limit_and_rejects_the_next_node_deterministically() {
        // Program > Statements > 4093 Arrays > Integer = 4096 nodes.
        let arrays = MAX_AST_DEPTH - 3;
        for (count, expected) in [
            (arrays, Ok(())),
            (arrays + 1, Err(DepthError { offset: arrays + 1 })),
        ] {
            let source = format!("{}1{}", "[".repeat(count), "]".repeat(count));
            // A raw Prism parse and teardown recurse too: give them the
            // stack the pipeline gives a file.
            stacker::grow(STACK_SIZE, || {
                let parsed = ruby_prism::parse(source.as_bytes());
                assert_eq!(parsed.errors().count(), 0);
                let program = parsed.node().as_program_node().unwrap();
                assert_eq!(check(&program), expected);
                assert_eq!(check(&program), expected);
            });
        }
    }

    #[test]
    fn wide_shallow_files_do_not_consume_the_depth_budget() {
        let source = "foo(1)\n".repeat(1000);
        let parsed = ruby_prism::parse(source.as_bytes());
        assert_eq!(check(&parsed.node().as_program_node().unwrap()), Ok(()));
    }
}
