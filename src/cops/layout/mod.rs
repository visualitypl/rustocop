//! The `Layout` department: formatting.
//!
//! `LineLength` is detection-only; whitespace cops supply conservative fixes.

mod body_padding;
mod brace_spacing;
mod bracket_spacing;
mod empty_line_between_defs;
mod empty_lines;
mod empty_lines_around_class_body;
mod empty_lines_around_method_body;
mod empty_lines_around_module_body;
mod end_alignment;
mod indentation_style;
mod indentation_width;
pub(crate) mod line_length;
mod operator_spacing;
mod space_after_comma;
mod space_after_semicolon;
mod space_around_equals_in_parameter_default;
mod space_around_operators;
mod space_before_comma;
mod space_inside_array_literal_brackets;
mod space_inside_block_braces;
mod space_inside_hash_literal_braces;
mod space_inside_parens;
mod space_inside_reference_brackets;
mod trailing_empty_lines;
mod trailing_whitespace;

pub use empty_line_between_defs::EmptyLineBetweenDefs;
pub use empty_lines::EmptyLines;
pub use empty_lines_around_class_body::EmptyLinesAroundClassBody;
pub use empty_lines_around_method_body::EmptyLinesAroundMethodBody;
pub use empty_lines_around_module_body::EmptyLinesAroundModuleBody;
pub use end_alignment::EndAlignment;
pub use indentation_style::IndentationStyle;
pub use indentation_width::IndentationWidth;
pub use line_length::LineLength;
pub use space_after_comma::SpaceAfterComma;
pub use space_after_semicolon::SpaceAfterSemicolon;
pub use space_around_equals_in_parameter_default::SpaceAroundEqualsInParameterDefault;
pub use space_around_operators::SpaceAroundOperators;
pub use space_before_comma::SpaceBeforeComma;
pub use space_inside_array_literal_brackets::SpaceInsideArrayLiteralBrackets;
pub use space_inside_block_braces::SpaceInsideBlockBraces;
pub use space_inside_hash_literal_braces::SpaceInsideHashLiteralBraces;
pub use space_inside_parens::SpaceInsideParens;
pub use space_inside_reference_brackets::SpaceInsideReferenceBrackets;
pub use trailing_empty_lines::TrailingEmptyLines;
pub use trailing_whitespace::TrailingWhitespace;
