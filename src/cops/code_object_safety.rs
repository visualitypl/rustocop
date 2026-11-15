//! The methods that can change constants, lookup, visibility or the code a
//! method reaches.

pub(super) fn mutation(name: &[u8]) -> bool {
    matches!(
        name,
        b"define_method"
            | b"define_singleton_method"
            | b"alias_method"
            | b"attr"
            | b"attr_reader"
            | b"attr_writer"
            | b"attr_accessor"
            | b"public"
            | b"private"
            | b"protected"
            | b"module_function"
            | b"public_class_method"
            | b"private_class_method"
            | b"public_constant"
            | b"private_constant"
            | b"remove_method"
            | b"undef_method"
            | b"include"
            | b"prepend"
            | b"extend"
            | b"append_features"
            | b"prepend_features"
            | b"extend_object"
            | b"refine"
            | b"using"
            | b"const_set"
            | b"remove_const"
            | b"autoload"
            | b"load"
            | b"require"
            | b"require_relative"
            | b"class_exec"
            | b"module_exec"
            | b"instance_exec"
    )
}
