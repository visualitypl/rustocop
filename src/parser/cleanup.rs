//! Stack-bounded cleanup for rejected Prism trees. ParseResult's normal Drop
//! recursively destroys the AST, including trees rejected by our depth guard.

use std::{alloc::Layout, ffi::c_void, mem::size_of, ptr};

use ruby_prism::{Node, ParseResult};
use ruby_prism_sys::{self as sys, pm_node_t};

unsafe extern "C" {
    // The bundled Prism uses the C allocator for nodes (and frees them in
    // pm_node_destroy), so Rust's allocator must not be used for these copies.
    fn malloc(size: usize) -> *mut c_void;
    fn pm_visit_child_nodes(
        node: *const pm_node_t,
        visitor: unsafe extern "C" fn(*const pm_node_t, *mut c_void) -> bool,
        data: *mut c_void,
    );
}

/// Consume a rejected parse, with no recursive descent in its destructor.
/// No Node/Location/diagnostic handles may be used after consuming the result.
///
/// Move each node's contents to a C-allocated shallow copy and turn its original
/// allocation into a resource-free MissingNode shell. Before destroying a copy,
/// do the same for its immediate children, placing their copies on a heap work
/// list. Prism's destructor then frees only that parent, its own resources and
/// its children's shells, never their subtrees. Finally ParseResult frees the
/// root shell and parser normally. Each node's resources are destroyed exactly
/// once, with constant stack depth and linear work; no tree/parser is leaked.
pub(super) fn drop_rejected(result: ParseResult<'_>) {
    let Node::ProgramNode {
        parser, pointer, ..
    } = result.node()
    else {
        unreachable!("Prism parse roots are programs")
    };
    let mut pending = Vec::<*mut pm_node_t>::new();
    // SAFETY: consuming result gives exclusive ownership of the live tree.
    // No Rust node handles survive this statement. Keep result (and thus its
    // parser/source) alive until all shallow copies have been destroyed.
    unsafe {
        pending.push(take_node(pointer.cast()));
        while let Some(node) = pending.pop() {
            pm_visit_child_nodes(node, take_child, (&mut pending as *mut Vec<_>).cast());
            // Every immediate child is now a MissingNode shell. Prism still
            // handles strings, integers, constant/node lists etc. for us.
            sys::pm_node_destroy(parser.as_ptr(), node);
        }
    }
    drop(result);
}

unsafe extern "C" fn take_child(node: *const pm_node_t, data: *mut c_void) -> bool {
    // SAFETY: only drop_rejected supplies this live work list. The child
    // enumerator is synchronous and does not recurse when we return false.
    unsafe {
        let pending = &mut *data.cast::<Vec<*mut pm_node_t>>();
        pending.push(take_node(node.cast_mut()));
    }
    false
}

/// Move ownership of all fields out of node, leaving only a freeable shell.
unsafe fn take_node(node: *mut pm_node_t) -> *mut pm_node_t {
    // SAFETY: callers own this node and have not previously moved its fields.
    let size = node_size(unsafe { (*node).type_ });
    // SAFETY: malloc returns suitably aligned storage, compatible with Prism's
    // free. Each copy has exactly the size of its actual C node type, not just
    // the base header (and we never read beyond the original allocation).
    let copy = unsafe { malloc(size) }.cast::<pm_node_t>();
    if copy.is_null() {
        std::alloc::handle_alloc_error(
            Layout::from_size_align(size, align_of::<pm_node_t>()).unwrap(),
        );
    }
    // SAFETY: both allocations are live, nonoverlapping and size bytes long.
    // The original shell owns no resources after this move. MissingNode's
    // destructor reads only its base header and then frees the allocation.
    unsafe {
        ptr::copy_nonoverlapping(node.cast::<u8>(), copy.cast::<u8>(), size);
        (*node).type_ = sys::pm_node_type::PM_MISSING_NODE as u16;
    }
    copy
}

// Prism 1.9's generated AST layout. The size comes from the actual bindings,
// not hardcoded byte counts. ScopeNode is a compilation-only sentinel, never
// a parsed AST node. It follows all AST types; fail compilation on additions so
// a dependency upgrade must audit this table and the teardown assumptions.
macro_rules! node_sizes {
    ($($kind:ident => $ty:ident),* $(,)?) => {
        const _: () = {
            let kinds = [$(sys::pm_node_type::$kind as usize),*];
            assert!(kinds.len() + 1 == sys::pm_node_type::PM_SCOPE_NODE as usize);
            let mut i = 0;
            while i < kinds.len() {
                assert!(kinds[i] == i + 1);
                i += 1;
            }
        };

        fn node_size(kind: u16) -> usize {
            $(const $kind: u16 = sys::pm_node_type::$kind as u16;)*
            match kind {
                $($kind => size_of::<sys::$ty>(),)*
                _ => unreachable!("not a parsed Prism AST node: {kind}"),
            }
        }
    };
}

node_sizes! {
    PM_ALIAS_GLOBAL_VARIABLE_NODE => pm_alias_global_variable_node_t,
    PM_ALIAS_METHOD_NODE => pm_alias_method_node_t,
    PM_ALTERNATION_PATTERN_NODE => pm_alternation_pattern_node_t,
    PM_AND_NODE => pm_and_node_t,
    PM_ARGUMENTS_NODE => pm_arguments_node_t,
    PM_ARRAY_NODE => pm_array_node_t,
    PM_ARRAY_PATTERN_NODE => pm_array_pattern_node_t,
    PM_ASSOC_NODE => pm_assoc_node_t,
    PM_ASSOC_SPLAT_NODE => pm_assoc_splat_node_t,
    PM_BACK_REFERENCE_READ_NODE => pm_back_reference_read_node_t,
    PM_BEGIN_NODE => pm_begin_node_t,
    PM_BLOCK_ARGUMENT_NODE => pm_block_argument_node_t,
    PM_BLOCK_LOCAL_VARIABLE_NODE => pm_block_local_variable_node_t,
    PM_BLOCK_NODE => pm_block_node_t,
    PM_BLOCK_PARAMETER_NODE => pm_block_parameter_node_t,
    PM_BLOCK_PARAMETERS_NODE => pm_block_parameters_node_t,
    PM_BREAK_NODE => pm_break_node_t,
    PM_CALL_AND_WRITE_NODE => pm_call_and_write_node_t,
    PM_CALL_NODE => pm_call_node_t,
    PM_CALL_OPERATOR_WRITE_NODE => pm_call_operator_write_node_t,
    PM_CALL_OR_WRITE_NODE => pm_call_or_write_node_t,
    PM_CALL_TARGET_NODE => pm_call_target_node_t,
    PM_CAPTURE_PATTERN_NODE => pm_capture_pattern_node_t,
    PM_CASE_MATCH_NODE => pm_case_match_node_t,
    PM_CASE_NODE => pm_case_node_t,
    PM_CLASS_NODE => pm_class_node_t,
    PM_CLASS_VARIABLE_AND_WRITE_NODE => pm_class_variable_and_write_node_t,
    PM_CLASS_VARIABLE_OPERATOR_WRITE_NODE => pm_class_variable_operator_write_node_t,
    PM_CLASS_VARIABLE_OR_WRITE_NODE => pm_class_variable_or_write_node_t,
    PM_CLASS_VARIABLE_READ_NODE => pm_class_variable_read_node_t,
    PM_CLASS_VARIABLE_TARGET_NODE => pm_class_variable_target_node_t,
    PM_CLASS_VARIABLE_WRITE_NODE => pm_class_variable_write_node_t,
    PM_CONSTANT_AND_WRITE_NODE => pm_constant_and_write_node_t,
    PM_CONSTANT_OPERATOR_WRITE_NODE => pm_constant_operator_write_node_t,
    PM_CONSTANT_OR_WRITE_NODE => pm_constant_or_write_node_t,
    PM_CONSTANT_PATH_AND_WRITE_NODE => pm_constant_path_and_write_node_t,
    PM_CONSTANT_PATH_NODE => pm_constant_path_node_t,
    PM_CONSTANT_PATH_OPERATOR_WRITE_NODE => pm_constant_path_operator_write_node_t,
    PM_CONSTANT_PATH_OR_WRITE_NODE => pm_constant_path_or_write_node_t,
    PM_CONSTANT_PATH_TARGET_NODE => pm_constant_path_target_node_t,
    PM_CONSTANT_PATH_WRITE_NODE => pm_constant_path_write_node_t,
    PM_CONSTANT_READ_NODE => pm_constant_read_node_t,
    PM_CONSTANT_TARGET_NODE => pm_constant_target_node_t,
    PM_CONSTANT_WRITE_NODE => pm_constant_write_node_t,
    PM_DEF_NODE => pm_def_node_t,
    PM_DEFINED_NODE => pm_defined_node_t,
    PM_ELSE_NODE => pm_else_node_t,
    PM_EMBEDDED_STATEMENTS_NODE => pm_embedded_statements_node_t,
    PM_EMBEDDED_VARIABLE_NODE => pm_embedded_variable_node_t,
    PM_ENSURE_NODE => pm_ensure_node_t,
    PM_FALSE_NODE => pm_false_node_t,
    PM_FIND_PATTERN_NODE => pm_find_pattern_node_t,
    PM_FLIP_FLOP_NODE => pm_flip_flop_node_t,
    PM_FLOAT_NODE => pm_float_node_t,
    PM_FOR_NODE => pm_for_node_t,
    PM_FORWARDING_ARGUMENTS_NODE => pm_forwarding_arguments_node_t,
    PM_FORWARDING_PARAMETER_NODE => pm_forwarding_parameter_node_t,
    PM_FORWARDING_SUPER_NODE => pm_forwarding_super_node_t,
    PM_GLOBAL_VARIABLE_AND_WRITE_NODE => pm_global_variable_and_write_node_t,
    PM_GLOBAL_VARIABLE_OPERATOR_WRITE_NODE => pm_global_variable_operator_write_node_t,
    PM_GLOBAL_VARIABLE_OR_WRITE_NODE => pm_global_variable_or_write_node_t,
    PM_GLOBAL_VARIABLE_READ_NODE => pm_global_variable_read_node_t,
    PM_GLOBAL_VARIABLE_TARGET_NODE => pm_global_variable_target_node_t,
    PM_GLOBAL_VARIABLE_WRITE_NODE => pm_global_variable_write_node_t,
    PM_HASH_NODE => pm_hash_node_t,
    PM_HASH_PATTERN_NODE => pm_hash_pattern_node_t,
    PM_IF_NODE => pm_if_node_t,
    PM_IMAGINARY_NODE => pm_imaginary_node_t,
    PM_IMPLICIT_NODE => pm_implicit_node_t,
    PM_IMPLICIT_REST_NODE => pm_implicit_rest_node_t,
    PM_IN_NODE => pm_in_node_t,
    PM_INDEX_AND_WRITE_NODE => pm_index_and_write_node_t,
    PM_INDEX_OPERATOR_WRITE_NODE => pm_index_operator_write_node_t,
    PM_INDEX_OR_WRITE_NODE => pm_index_or_write_node_t,
    PM_INDEX_TARGET_NODE => pm_index_target_node_t,
    PM_INSTANCE_VARIABLE_AND_WRITE_NODE => pm_instance_variable_and_write_node_t,
    PM_INSTANCE_VARIABLE_OPERATOR_WRITE_NODE => pm_instance_variable_operator_write_node_t,
    PM_INSTANCE_VARIABLE_OR_WRITE_NODE => pm_instance_variable_or_write_node_t,
    PM_INSTANCE_VARIABLE_READ_NODE => pm_instance_variable_read_node_t,
    PM_INSTANCE_VARIABLE_TARGET_NODE => pm_instance_variable_target_node_t,
    PM_INSTANCE_VARIABLE_WRITE_NODE => pm_instance_variable_write_node_t,
    PM_INTEGER_NODE => pm_integer_node_t,
    PM_INTERPOLATED_MATCH_LAST_LINE_NODE => pm_interpolated_match_last_line_node_t,
    PM_INTERPOLATED_REGULAR_EXPRESSION_NODE => pm_interpolated_regular_expression_node_t,
    PM_INTERPOLATED_STRING_NODE => pm_interpolated_string_node_t,
    PM_INTERPOLATED_SYMBOL_NODE => pm_interpolated_symbol_node_t,
    PM_INTERPOLATED_X_STRING_NODE => pm_interpolated_x_string_node_t,
    PM_IT_LOCAL_VARIABLE_READ_NODE => pm_it_local_variable_read_node_t,
    PM_IT_PARAMETERS_NODE => pm_it_parameters_node_t,
    PM_KEYWORD_HASH_NODE => pm_keyword_hash_node_t,
    PM_KEYWORD_REST_PARAMETER_NODE => pm_keyword_rest_parameter_node_t,
    PM_LAMBDA_NODE => pm_lambda_node_t,
    PM_LOCAL_VARIABLE_AND_WRITE_NODE => pm_local_variable_and_write_node_t,
    PM_LOCAL_VARIABLE_OPERATOR_WRITE_NODE => pm_local_variable_operator_write_node_t,
    PM_LOCAL_VARIABLE_OR_WRITE_NODE => pm_local_variable_or_write_node_t,
    PM_LOCAL_VARIABLE_READ_NODE => pm_local_variable_read_node_t,
    PM_LOCAL_VARIABLE_TARGET_NODE => pm_local_variable_target_node_t,
    PM_LOCAL_VARIABLE_WRITE_NODE => pm_local_variable_write_node_t,
    PM_MATCH_LAST_LINE_NODE => pm_match_last_line_node_t,
    PM_MATCH_PREDICATE_NODE => pm_match_predicate_node_t,
    PM_MATCH_REQUIRED_NODE => pm_match_required_node_t,
    PM_MATCH_WRITE_NODE => pm_match_write_node_t,
    PM_MISSING_NODE => pm_missing_node_t,
    PM_MODULE_NODE => pm_module_node_t,
    PM_MULTI_TARGET_NODE => pm_multi_target_node_t,
    PM_MULTI_WRITE_NODE => pm_multi_write_node_t,
    PM_NEXT_NODE => pm_next_node_t,
    PM_NIL_NODE => pm_nil_node_t,
    PM_NO_KEYWORDS_PARAMETER_NODE => pm_no_keywords_parameter_node_t,
    PM_NUMBERED_PARAMETERS_NODE => pm_numbered_parameters_node_t,
    PM_NUMBERED_REFERENCE_READ_NODE => pm_numbered_reference_read_node_t,
    PM_OPTIONAL_KEYWORD_PARAMETER_NODE => pm_optional_keyword_parameter_node_t,
    PM_OPTIONAL_PARAMETER_NODE => pm_optional_parameter_node_t,
    PM_OR_NODE => pm_or_node_t,
    PM_PARAMETERS_NODE => pm_parameters_node_t,
    PM_PARENTHESES_NODE => pm_parentheses_node_t,
    PM_PINNED_EXPRESSION_NODE => pm_pinned_expression_node_t,
    PM_PINNED_VARIABLE_NODE => pm_pinned_variable_node_t,
    PM_POST_EXECUTION_NODE => pm_post_execution_node_t,
    PM_PRE_EXECUTION_NODE => pm_pre_execution_node_t,
    PM_PROGRAM_NODE => pm_program_node_t,
    PM_RANGE_NODE => pm_range_node_t,
    PM_RATIONAL_NODE => pm_rational_node_t,
    PM_REDO_NODE => pm_redo_node_t,
    PM_REGULAR_EXPRESSION_NODE => pm_regular_expression_node_t,
    PM_REQUIRED_KEYWORD_PARAMETER_NODE => pm_required_keyword_parameter_node_t,
    PM_REQUIRED_PARAMETER_NODE => pm_required_parameter_node_t,
    PM_RESCUE_MODIFIER_NODE => pm_rescue_modifier_node_t,
    PM_RESCUE_NODE => pm_rescue_node_t,
    PM_REST_PARAMETER_NODE => pm_rest_parameter_node_t,
    PM_RETRY_NODE => pm_retry_node_t,
    PM_RETURN_NODE => pm_return_node_t,
    PM_SELF_NODE => pm_self_node_t,
    PM_SHAREABLE_CONSTANT_NODE => pm_shareable_constant_node_t,
    PM_SINGLETON_CLASS_NODE => pm_singleton_class_node_t,
    PM_SOURCE_ENCODING_NODE => pm_source_encoding_node_t,
    PM_SOURCE_FILE_NODE => pm_source_file_node_t,
    PM_SOURCE_LINE_NODE => pm_source_line_node_t,
    PM_SPLAT_NODE => pm_splat_node_t,
    PM_STATEMENTS_NODE => pm_statements_node_t,
    PM_STRING_NODE => pm_string_node_t,
    PM_SUPER_NODE => pm_super_node_t,
    PM_SYMBOL_NODE => pm_symbol_node_t,
    PM_TRUE_NODE => pm_true_node_t,
    PM_UNDEF_NODE => pm_undef_node_t,
    PM_UNLESS_NODE => pm_unless_node_t,
    PM_UNTIL_NODE => pm_until_node_t,
    PM_WHEN_NODE => pm_when_node_t,
    PM_WHILE_NODE => pm_while_node_t,
    PM_X_STRING_NODE => pm_x_string_node_t,
    PM_YIELD_NODE => pm_yield_node_t,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cops::testing::{ERROR_TREE_TYPED_CHILDREN, FULL_TREE, TYPED_CHILD_CASES};

    #[test]
    fn destroys_full_typed_and_recovered_trees() {
        for source in std::iter::once(FULL_TREE)
            .chain(TYPED_CHILD_CASES.iter().map(|(source, _)| *source))
            .chain(ERROR_TREE_TYPED_CHILDREN.iter().copied())
            .chain([
                // Owned strings, large integers, and locals/constant lists.
                "def foo(a = 999999999999999999999999999999999999); \"a\\nb\"; /a\\tb/; end",
                "# coding: utf-8\nfoo(\"a\\nb\"); __FILE__\n__END__\ndata\n",
                "",
            ])
        {
            drop_rejected(ruby_prism::parse(source.as_bytes()));
        }
    }
}
