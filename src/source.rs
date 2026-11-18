//! Source files and byte-offset to line/column mapping.

use std::{ops::Range, path::PathBuf, sync::OnceLock};

use ruby_prism::{Node, ParseResult};

mod columns;
mod diagnostics;
mod encoding;
mod input_metadata;
mod literal_ranges;
pub(crate) use literal_ranges::Heredoc;
pub(crate) use literal_ranges::literal_interior;
mod punctuation;
pub(crate) mod tokens;

pub use diagnostics::{FatalDiagnostic, ParseDiagnostic, ParseDiagnostics, ParseWarning};
pub use input_metadata::{InputIdentity, SuppliedInputMetadata};

pub(crate) use encoding::{
    SourceEncoding, projected_alphanumeric, unicode_lowercase, unicode_uppercase,
};

/// A Ruby source file: its display path, raw bytes, and a line index.
pub struct SourceFile {
    /// The path as reported in output (relative to the config root when possible).
    pub path: PathBuf,
    /// The raw file bytes.
    pub bytes: Vec<u8>,
    ruby_version: crate::cops::policy::RubyVersion,
    /// Actual disk input, distinct from its display/config path. Absent for stdin
    /// and in-memory fixtures; consulted lazily only by load-path-sensitive fixes.
    read_path: Option<PathBuf>,
    /// Captured from the opened disk input, never looked up through a filename.
    supplied_input_metadata: Option<SuppliedInputMetadata>,
    /// Supplied parser filename, including stdin's virtual name. Unlike `path`,
    /// this is not rebased for display; unlike `read_path`, it grants no I/O.
    source_path: Option<PathBuf>,
    noncanonical_load_path: OnceLock<bool>,
    interpolation_facts: OnceLock<crate::cops::style::interpolation_safety::Facts>,
    negative_array_index_facts: OnceLock<crate::cops::style::negative_array_index::Facts>,
    dig_calls: OnceLock<crate::cops::style::dig_chain::Facts>,
    word_array_matrices: OnceLock<std::sync::Mutex<std::collections::HashSet<usize>>>,
    visible_locals: OnceLock<crate::cops::variables::VisibleLocals>,
    /// Detection-only UTF8-MAC aliases; built once, only for numeric candidates.
    pub(crate) numeric_operation_local_aliases:
        OnceLock<std::collections::HashMap<Vec<u8>, Vec<Vec<u8>>>>,
    local_rename_facts: OnceLock<crate::cops::naming::local_rename::Facts>,
    forwarding_safety: OnceLock<crate::cops::style::arguments_forwarding::Safety>,
    rescue_preferred_reads:
        OnceLock<crate::cops::naming::rescued_exceptions_variable_name::PreferredReads>,
    class_emitter_siblings: OnceLock<crate::cops::naming::class_emitters::Siblings>,
    self_facts: OnceLock<crate::cops::style::redundant_self::Facts>,
    initialize_comments: OnceLock<crate::cops::style::redundant_initialize::Comments>,
    redundant_assignment_safety: OnceLock<crate::cops::style::redundant_assignment::Safety>,
    hash_each_facts: OnceLock<crate::cops::style::hash_each_methods::Facts>,
    each_with_object_facts: OnceLock<crate::cops::style::each_with_object::Facts>,
    map_into_array_facts: OnceLock<crate::cops::style::map_into_array::Facts>,
    hash_transform_facts: OnceLock<crate::cops::style::hash_transform::Facts>,
    reduce_to_hash_facts: OnceLock<crate::cops::style::reduce_to_hash::Facts>,
    redundant_double_splats: OnceLock<std::collections::HashSet<usize>>,
    hash_fetch_facts: OnceLock<crate::cops::style::hash_fetch_chain::Facts>,
    hash_lookup_facts: [OnceLock<crate::cops::style::hash_lookup_method::Facts>; 2],
    super_facts: OnceLock<crate::cops::style::super_arguments::Facts>,
    option_hash_supers: OnceLock<crate::cops::style::option_hash::SuperIndex>,
    inverse_method_nexts: OnceLock<crate::cops::style::inverse_methods::NextIndex>,
    /// Correction-only lexical declaration/read evidence for branch swaps.
    unless_else_bindings: OnceLock<crate::cops::style::unless_else::Bindings>,
    /// Detection-only bottom-up return facts for subjectless-case branch guards.
    empty_case_return_bodies: OnceLock<std::collections::HashSet<usize>>,
    /// Correction-only implicit-it hazards for both empty-parameter cops.
    empty_parameter_facts: OnceLock<crate::cops::style::empty_parameters::Facts>,
    do_end_facts: OnceLock<crate::cops::style::single_line_do_end_block::Facts>,
    block_chain_facts: OnceLock<crate::cops::style::multiline_block_chain::Facts>,
    block_param_facts: OnceLock<crate::cops::style::single_line_block_params::Facts>,
    it_parameter_facts: OnceLock<crate::cops::style::it_block_parameter::Facts>,
    /// Correction-only closed eager Kernel.lambda lookup proof.
    lambda_facts: OnceLock<crate::cops::style::lambda::Facts>,
    /// Correction-only closed eager Proc constant and Kernel.proc lookup proof.
    proc_facts: OnceLock<crate::cops::style::proc::Facts>,
    /// Correction-only ownership of nested sugar sends, indexed in linear work.
    lambda_call_facts: OnceLock<crate::cops::style::lambda_call::Facts>,
    /// Distinct numbered-name masks folded once from the main AST.
    numbered_limit_facts: OnceLock<crate::cops::style::numbered_parameters_limit::Facts>,
    /// Detection-only descendant symbol-call ranges for lexical visibility.
    ineffective_private_names:
        OnceLock<crate::cops::lint::ineffective_access_modifier::PrivateNames>,
    useless_access_modifiers: OnceLock<crate::cops::lint::useless_access_modifier::Facts>,
    access_modifier_scopes:
        OnceLock<crate::cops::layout::empty_lines_around_access_modifier::Scopes>,
    missing_super_facts: OnceLock<crate::cops::lint::missing_super::Facts>,
    useless_rescue_facts: OnceLock<crate::cops::lint::useless_rescue::Facts>,
    ruby2_keywords_facts: OnceLock<crate::cops::lint::useless_ruby2_keywords::Facts>,
    /// Conditional-edge equality, only for non-bare method modifiers.
    pub(crate) method_definition_macro_branches:
        std::sync::Mutex<std::collections::HashMap<(usize, usize, usize), bool>>,
    /// Memoized syntactic Time/DateTime roots, only for the opt-in detector.
    pub(crate) number_conversion_roots:
        std::sync::Mutex<std::collections::HashMap<(usize, usize), bool>>,
    /// Byte offsets at which each line starts (line 1 starts at 0).
    line_starts: std::sync::Arc<Vec<usize>>,
    /// Byte ranges of the file's comments, in source order. Recorded by the
    /// pipeline from its parse; computed on first use otherwise.
    comments: OnceLock<Vec<Range<usize>>>,
    /// RuboCop's parser-gem token projection of Prism's own lexer stream.
    parser_tokens: OnceLock<Vec<tokens::Token>>,
    preceding_following: OnceLock<crate::cops::layout::preceding_following::Lines>,
    /// Shared constant-definition locations for Struct/Data superclass checks.
    inheritance_constants: OnceLock<crate::cops::style::struct_inheritance::ConstantDefinitions>,
    custom_attrs: OnceLock<crate::cops::style::accessors::CustomAttrs>,
    accessor_annotations: OnceLock<crate::cops::style::accessors::Annotations>,
    /// MutableConstant's fixed magic policy, indexed once rather than per write.
    mutable_constant_magic: OnceLock<crate::cops::style::mutable_constant::MagicState>,
    /// Fixed documentation/annotation/nodoc facts, classified once per comment.
    documentation_comments: OnceLock<crate::cops::style::documentation::DocumentationComments>,
    /// Physical-line # markers for eval docs, including markers in literals.
    non_interpolation_hash_lines: OnceLock<Vec<bool>>,
    /// Per-container method visibility, indexed lazily from NodeContext parents.
    method_visibility:
        OnceLock<std::sync::Mutex<crate::cops::style::documentation_method::MethodVisibility>>,
    /// Lazy source-order facts for modifier eligibility, using the main AST.
    modifier_index: OnceLock<crate::cops::style::if_unless_modifier::ModifierIndex>,
    /// Correction-only file API binding evidence, indexed once from the main AST.
    file_api_bindings: OnceLock<crate::cops::style::file_api::Bindings>,
    /// Main-AST lexical companion-method presence, indexed at most once.
    respond_to_missing_index:
        OnceLock<crate::cops::style::missing_respond_to_missing::CompanionIndex>,
    /// Correction-only binding evidence for class-check selector replacements.
    class_check_bindings: OnceLock<crate::cops::style::class_check::Bindings>,
    /// Correction-only binding/visibility proof for ObjectThen selector edits.
    object_then_bindings: OnceLock<crate::cops::style::object_then::Bindings>,
    /// Correction-only closed eager core proof for literal conditional clamps.
    comparable_clamp_safety: OnceLock<crate::cops::style::comparable_clamp::Safety>,
    /// Detection-only conservative method return facts, built once on demand.
    predicate_method_returns: OnceLock<crate::cops::naming::predicate_method::Returns>,
    /// Lazy structural tail equality for Naming memoization detection only.
    memoized_name_fingerprints: OnceLock<
        std::sync::Mutex<crate::cops::naming::memoized_instance_variable_name::Fingerprints>,
    >,
    /// LiteralAssignmentInCondition's bounded, lazy structural scope-edge facts.
    literal_assignment_scope_edges:
        OnceLock<std::sync::Mutex<crate::cops::lint::literal_assignment_in_condition::ScopeEdges>>,
    /// Direct conditional guard keys, indexed once per condition for FetchEnvVar.
    fetch_env_conditions: OnceLock<std::sync::Mutex<crate::cops::style::fetch_env_var::Conditions>>,
    /// FileNull's detection evidence, distinct from correction-only bindings.
    contains_dev_null: OnceLock<crate::cops::style::file_null::NullDevices>,
    /// RedundantCondition matches already visited in the main preorder walk.
    /// Descendants query these facts instead of re-fingerprinting ancestors.
    redundant_conditions: OnceLock<std::sync::Mutex<std::collections::HashSet<(usize, usize)>>>,
    /// IdenticalConditionalBranches expression ranges already reported by an
    /// enclosing chain; also deduplicates an expression that is both edges.
    identical_branch_edges: OnceLock<std::sync::Mutex<std::collections::HashSet<(usize, usize)>>>,
    /// Correction-only unbraced variable ends, recorded in the existing main
    /// preorder cop dispatch. Exact offsets avoid fragment/sibling rescans.
    string_escape_variable_ends: OnceLock<std::sync::Mutex<std::collections::HashSet<usize>>>,
    /// Literal-token boundaries recorded by the main LineEndConcatenation walk.
    line_end_boundaries:
        OnceLock<std::sync::Mutex<crate::cops::style::line_end_concatenation::Boundaries>>,
    /// Narrow continuation facts from the main lexer, completed by cop dispatch.
    raw_continuations: OnceLock<Vec<crate::parser::Continuation>>,
    line_continuations:
        OnceLock<std::sync::Mutex<crate::cops::style::redundant_line_continuation::Facts>>,
    /// Raw backtick character starts, indexed once for nested command bodies.
    command_backticks: OnceLock<Vec<usize>>,
    /// Pipeline directive state, also read by cross-cop length-policy checks.
    directives: OnceLock<crate::directives::Directives>,
    /// Configured-off cop universe, before CLI force-selection/exclusion.
    directive_disabled: OnceLock<Vec<String>>,
    /// Parsed directive tokens, shared by the two directive Style policies.
    directive_records: OnceLock<Vec<crate::directives::comments::DirectiveComment>>,
    /// Comments containing directive spellings, cached for newline-edit guards.
    directive_comments: OnceLock<Vec<Range<usize>>>,
    /// Last textual `__LINE__` boundary, shared by newline-changing fixes.
    line_value_boundary: OnceLock<usize>,
    /// Errors and warnings from the main parse, including rejected input.
    diagnostics: OnceLock<ParseDiagnostics>,
    /// The byte offset of the `__END__` line that starts the data section,
    /// if any. Recorded by the pipeline from its parse; computed on first
    /// use otherwise.
    data_start: OnceLock<Option<usize>>,
    /// Merged literal-text ranges, lazily built from the main parse's AST.
    literal_ranges: OnceLock<literal_ranges::SafetyRanges>,
    /// Literal token exclusions shared by spacing and empty-line detection.
    token_ranges: OnceLock<literal_ranges::PunctuationRanges>,
    /// Actual separators and extraneous parenthesis spacing, built once on demand.
    punctuation: OnceLock<punctuation::PunctuationData>,
    /// Encoding-dependent character widths and blanks, from the main parse.
    encoding: OnceLock<encoding::ParsedEncoding>,
    /// Lazy sparse scalar-column savings, shared by every cop.
    columns: OnceLock<columns::ColumnIndex>,
}

impl SourceFile {
    /// Build a source file from its bytes, computing the line index.
    pub fn new(path: PathBuf, bytes: Vec<u8>) -> Self {
        let mut line_starts = vec![0];
        for (i, byte) in bytes.iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push(i + 1);
            }
        }
        SourceFile {
            path,
            bytes,
            ruby_version: crate::cops::policy::DEFAULT_RUBY_VERSION,
            read_path: None,
            supplied_input_metadata: None,
            source_path: None,
            noncanonical_load_path: OnceLock::new(),
            ineffective_private_names: OnceLock::new(),
            useless_access_modifiers: OnceLock::new(),
            access_modifier_scopes: OnceLock::new(),
            missing_super_facts: OnceLock::new(),
            useless_rescue_facts: OnceLock::new(),
            ruby2_keywords_facts: OnceLock::new(),
            method_definition_macro_branches: std::sync::Mutex::new(
                std::collections::HashMap::new(),
            ),
            number_conversion_roots: Default::default(),
            interpolation_facts: OnceLock::new(),
            negative_array_index_facts: OnceLock::new(),
            dig_calls: OnceLock::new(),
            word_array_matrices: OnceLock::new(),
            visible_locals: OnceLock::new(),
            numeric_operation_local_aliases: OnceLock::new(),
            local_rename_facts: OnceLock::new(),
            forwarding_safety: OnceLock::new(),
            rescue_preferred_reads: OnceLock::new(),
            class_emitter_siblings: OnceLock::new(),
            self_facts: OnceLock::new(),
            initialize_comments: OnceLock::new(),
            redundant_assignment_safety: OnceLock::new(),
            hash_each_facts: OnceLock::new(),
            each_with_object_facts: OnceLock::new(),
            map_into_array_facts: OnceLock::new(),
            hash_transform_facts: OnceLock::new(),
            reduce_to_hash_facts: OnceLock::new(),
            redundant_double_splats: OnceLock::new(),
            hash_fetch_facts: OnceLock::new(),
            hash_lookup_facts: std::array::from_fn(|_| OnceLock::new()),
            super_facts: OnceLock::new(),
            option_hash_supers: OnceLock::new(),
            inverse_method_nexts: OnceLock::new(),
            unless_else_bindings: OnceLock::new(),
            empty_case_return_bodies: OnceLock::new(),
            line_starts: std::sync::Arc::new(line_starts),
            comments: OnceLock::new(),
            parser_tokens: OnceLock::new(),
            preceding_following: OnceLock::new(),
            inheritance_constants: OnceLock::new(),
            custom_attrs: OnceLock::new(),
            accessor_annotations: OnceLock::new(),
            mutable_constant_magic: OnceLock::new(),
            documentation_comments: OnceLock::new(),
            non_interpolation_hash_lines: OnceLock::new(),
            method_visibility: OnceLock::new(),
            modifier_index: OnceLock::new(),
            file_api_bindings: OnceLock::new(),
            respond_to_missing_index: OnceLock::new(),
            class_check_bindings: OnceLock::new(),
            object_then_bindings: OnceLock::new(),
            comparable_clamp_safety: OnceLock::new(),
            predicate_method_returns: OnceLock::new(),
            memoized_name_fingerprints: OnceLock::new(),
            fetch_env_conditions: OnceLock::new(),
            literal_assignment_scope_edges: OnceLock::new(),
            contains_dev_null: OnceLock::new(),
            redundant_conditions: OnceLock::new(),
            identical_branch_edges: OnceLock::new(),
            string_escape_variable_ends: OnceLock::new(),
            line_end_boundaries: OnceLock::new(),
            command_backticks: OnceLock::new(),
            raw_continuations: OnceLock::new(),
            line_continuations: OnceLock::new(),
            directives: OnceLock::new(),
            directive_disabled: OnceLock::new(),
            directive_records: OnceLock::new(),
            directive_comments: OnceLock::new(),
            line_value_boundary: OnceLock::new(),
            diagnostics: OnceLock::new(),
            data_start: OnceLock::new(),
            empty_parameter_facts: OnceLock::new(),
            do_end_facts: OnceLock::new(),
            block_chain_facts: OnceLock::new(),
            block_param_facts: OnceLock::new(),
            it_parameter_facts: OnceLock::new(),
            lambda_facts: OnceLock::new(),
            proc_facts: OnceLock::new(),
            lambda_call_facts: OnceLock::new(),
            numbered_limit_facts: OnceLock::new(),
            literal_ranges: OnceLock::new(),
            token_ranges: OnceLock::new(),
            punctuation: OnceLock::new(),
            encoding: OnceLock::new(),
            columns: OnceLock::new(),
        }
    }

    pub(crate) fn record_continuations(&self, records: &[crate::parser::Continuation]) {
        let _ = self.raw_continuations.set(records.to_vec());
    }

    pub(crate) fn line_continuations(
        &self,
    ) -> Option<&std::sync::Mutex<crate::cops::style::redundant_line_continuation::Facts>> {
        let records = self.raw_continuations.get()?;
        if records.is_empty() {
            return None;
        }
        Some(self.line_continuations.get_or_init(|| {
            std::sync::Mutex::new(crate::cops::style::redundant_line_continuation::Facts::new(
                self, records,
            ))
        }))
    }

    pub(crate) fn line_end_boundaries(
        &self,
    ) -> &std::sync::Mutex<crate::cops::style::line_end_concatenation::Boundaries> {
        self.line_end_boundaries.get_or_init(Default::default)
    }

    /// Raw policy includes interpolation code, not just decoded static parts.
    /// One encoding-aware index avoids rescanning recursively nested bodies.
    pub(crate) fn command_body_has_backtick(&self, range: Range<usize>) -> bool {
        let offsets = self.command_backticks.get_or_init(|| {
            self.character_offsets(0..self.bytes.len())
                .filter(|&i| self.bytes[i] == b'`')
                .collect()
        });
        offsets
            .get(offsets.partition_point(|&i| i < range.start))
            .is_some_and(|&i| i < range.end)
    }

    pub(crate) fn useless_access_modifiers(
        &self,
        root: &ruby_prism::Node<'_>,
    ) -> &crate::cops::lint::useless_access_modifier::Facts {
        self.useless_access_modifiers
            .get_or_init(|| crate::cops::lint::useless_access_modifier::Facts::build(root))
    }

    /// Layout/EmptyLinesAroundAccessModifier's whole-investigation state at
    /// each candidate modifier, from one walk of the file.
    pub(crate) fn access_modifier_scopes(
        &self,
        root: &ruby_prism::Node<'_>,
    ) -> &crate::cops::layout::empty_lines_around_access_modifier::Scopes {
        self.access_modifier_scopes.get_or_init(|| {
            crate::cops::layout::empty_lines_around_access_modifier::Scopes::build(root, self)
        })
    }

    pub(crate) fn ineffective_private_names(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::lint::ineffective_access_modifier::PrivateNames {
        self.ineffective_private_names.get_or_init(|| {
            crate::cops::lint::ineffective_access_modifier::PrivateNames::build(root, self)
        })
    }

    pub(crate) fn dig_calls(&self, root: &Node<'_>) -> &crate::cops::style::dig_chain::Facts {
        self.dig_calls
            .get_or_init(|| crate::cops::style::dig_chain::Facts::build(root))
    }

    pub(crate) fn negative_array_index_facts(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::negative_array_index::Facts {
        self.negative_array_index_facts
            .get_or_init(|| crate::cops::style::negative_array_index::Facts::build(root))
    }

    pub(crate) fn interpolation_facts(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::interpolation_safety::Facts {
        self.interpolation_facts
            .get_or_init(|| crate::cops::style::interpolation_safety::Facts::build(root))
    }

    pub(crate) fn numbered_limit_facts(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::numbered_parameters_limit::Facts {
        self.numbered_limit_facts
            .get_or_init(|| crate::cops::style::numbered_parameters_limit::Facts::build(root))
    }

    pub(crate) fn lambda_call_facts(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::lambda_call::Facts {
        self.lambda_call_facts
            .get_or_init(|| crate::cops::style::lambda_call::Facts::build(root))
    }

    pub(crate) fn proc_facts(&self, root: &Node<'_>) -> &crate::cops::style::proc::Facts {
        self.proc_facts
            .get_or_init(|| crate::cops::style::proc::Facts::build(root))
    }

    pub(crate) fn ruby2_keywords_facts(
        &self,
        root: &ruby_prism::Node<'_>,
    ) -> &crate::cops::lint::useless_ruby2_keywords::Facts {
        self.ruby2_keywords_facts
            .get_or_init(|| crate::cops::lint::useless_ruby2_keywords::Facts::build(root, self))
    }

    pub(crate) fn useless_rescue_facts(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::lint::useless_rescue::Facts {
        self.useless_rescue_facts
            .get_or_init(|| crate::cops::lint::useless_rescue::Facts::build(root, self))
    }

    pub(crate) fn missing_super_facts(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::lint::missing_super::Facts {
        self.missing_super_facts
            .get_or_init(|| crate::cops::lint::missing_super::Facts::build(root))
    }

    pub(crate) fn lambda_facts(&self, root: &Node<'_>) -> &crate::cops::style::lambda::Facts {
        self.lambda_facts
            .get_or_init(|| crate::cops::style::lambda::Facts::build(root))
    }

    pub(crate) fn inheritance_constants(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::struct_inheritance::ConstantDefinitions {
        self.inheritance_constants.get_or_init(|| {
            crate::cops::style::struct_inheritance::ConstantDefinitions::build(root)
        })
    }

    pub(crate) fn accessor_annotations(&self) -> &crate::cops::style::accessors::Annotations {
        self.accessor_annotations
            .get_or_init(|| crate::cops::style::accessors::Annotations::build(self))
    }

    pub(crate) fn custom_attrs(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::accessors::CustomAttrs {
        self.custom_attrs
            .get_or_init(|| crate::cops::style::accessors::CustomAttrs::build(root))
    }

    pub(crate) fn mutable_constant_magic(
        &self,
    ) -> &crate::cops::style::mutable_constant::MagicState {
        self.mutable_constant_magic
            .get_or_init(|| crate::cops::style::mutable_constant::MagicState::build(self))
    }

    pub(crate) fn hash_lookup_facts(
        &self,
        root: &ruby_prism::Node<'_>,
        fetch_env_enabled: bool,
    ) -> &crate::cops::style::hash_lookup_method::Facts {
        self.hash_lookup_facts[usize::from(fetch_env_enabled)].get_or_init(|| {
            crate::cops::style::hash_lookup_method::Facts::build(root, self, fetch_env_enabled)
        })
    }

    pub(crate) fn hash_fetch_facts(
        &self,
        root: &ruby_prism::Node<'_>,
    ) -> &crate::cops::style::hash_fetch_chain::Facts {
        self.hash_fetch_facts
            .get_or_init(|| crate::cops::style::hash_fetch_chain::Facts::build(root))
    }

    pub(crate) fn reduce_to_hash_facts(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::reduce_to_hash::Facts {
        self.reduce_to_hash_facts
            .get_or_init(|| crate::cops::style::reduce_to_hash::Facts::build(root))
    }

    pub(crate) fn hash_transform_facts(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::hash_transform::Facts {
        self.hash_transform_facts
            .get_or_init(|| crate::cops::style::hash_transform::Facts::build(root))
    }

    pub(crate) fn each_with_object_facts(
        &self,
        root: &ruby_prism::Node<'_>,
    ) -> &crate::cops::style::each_with_object::Facts {
        self.each_with_object_facts
            .get_or_init(|| crate::cops::style::each_with_object::Facts::build(root))
    }

    pub(crate) fn map_into_array_facts(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::map_into_array::Facts {
        self.map_into_array_facts
            .get_or_init(|| crate::cops::style::map_into_array::Facts::build(root))
    }

    pub(crate) fn hash_each_facts(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::hash_each_methods::Facts {
        self.hash_each_facts
            .get_or_init(|| crate::cops::style::hash_each_methods::Facts::build(root))
    }

    pub(crate) fn forwarding_safety(
        &self,
        root: &ruby_prism::Node<'_>,
        analysis: &crate::cops::variables::ScopeAnalysis<'_>,
    ) -> &crate::cops::style::arguments_forwarding::Safety {
        self.forwarding_safety
            .get_or_init(|| crate::cops::style::arguments_forwarding::Safety::build(root, analysis))
    }

    pub(crate) fn local_rename_facts(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::naming::local_rename::Facts {
        self.local_rename_facts
            .get_or_init(|| crate::cops::naming::local_rename::Facts::build(root))
    }

    pub(crate) fn rescue_preferred_reads(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::naming::rescued_exceptions_variable_name::PreferredReads {
        self.rescue_preferred_reads.get_or_init(|| {
            crate::cops::naming::rescued_exceptions_variable_name::PreferredReads::build(root)
        })
    }

    pub(crate) fn with_ruby_version(mut self, version: crate::cops::policy::RubyVersion) -> Self {
        self.ruby_version = version;
        self
    }

    pub fn ruby_version(&self) -> crate::cops::policy::RubyVersion {
        self.ruby_version
    }

    pub(crate) fn with_source_path(mut self, path: &std::path::Path) -> Self {
        self.source_path = Some(path.to_path_buf());
        self
    }

    pub(crate) fn source_path(&self) -> &std::path::Path {
        self.source_path.as_deref().unwrap_or(&self.path)
    }

    pub(crate) fn with_read_path(mut self, path: &std::path::Path) -> Self {
        self.read_path = Some(path.to_path_buf());
        self
    }

    /// Read-only metadata captured with the disk bytes. Virtual/stdin filenames
    /// (even existing paths) grant no I/O. Alias and correction-validation byte
    /// snapshots retain the original facts; this never re-stats a mutable path.
    pub fn supplied_input_metadata(&self) -> Option<&SuppliedInputMetadata> {
        self.supplied_input_metadata.as_ref()
    }

    pub(crate) fn with_supplied_input_metadata(
        mut self,
        metadata: Option<SuppliedInputMetadata>,
    ) -> Self {
        self.supplied_input_metadata = metadata;
        self
    }

    /// `expand_path(dirname(__FILE__))` retains a symlink's directory, whereas
    /// `__dir__` canonicalizes it. Protect known aliased disk inputs. Unknown
    /// future invocation paths and external monkey patches cannot be inferred.
    pub(crate) fn noncanonical_load_path(&self) -> bool {
        *self.noncanonical_load_path.get_or_init(|| {
            let Some(path) = &self.read_path else {
                return false;
            };
            Self::path_is_noncanonical(path)
        })
    }

    pub(crate) fn path_is_noncanonical(path: &std::path::Path) -> bool {
        let Ok(absolute) = std::path::absolute(path) else {
            return true;
        };
        let Ok(canonical) = std::fs::canonicalize(path) else {
            return true;
        };
        absolute != canonical
    }

    pub(crate) fn file_api_bindings(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::file_api::Bindings {
        self.file_api_bindings
            .get_or_init(|| crate::cops::style::file_api::Bindings::build(root))
    }

    pub(crate) fn respond_to_missing_index(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::missing_respond_to_missing::CompanionIndex {
        self.respond_to_missing_index.get_or_init(|| {
            crate::cops::style::missing_respond_to_missing::CompanionIndex::build(root)
        })
    }

    pub(crate) fn comparable_clamp_safety(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::comparable_clamp::Safety {
        self.comparable_clamp_safety
            .get_or_init(|| crate::cops::style::comparable_clamp::Safety::build(root))
    }

    pub(crate) fn empty_case_return_bodies(
        &self,
        root: &Node<'_>,
    ) -> &std::collections::HashSet<usize> {
        self.empty_case_return_bodies
            .get_or_init(|| crate::cops::style::empty_case_condition::return_bodies(root))
    }

    pub(crate) fn unless_else_bindings(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::unless_else::Bindings {
        self.unless_else_bindings
            .get_or_init(|| crate::cops::style::unless_else::Bindings::build(root))
    }

    pub(crate) fn object_then_bindings(
        &self,
        root: &ruby_prism::Node<'_>,
    ) -> &crate::cops::style::object_then::Bindings {
        self.object_then_bindings
            .get_or_init(|| crate::cops::style::object_then::Bindings::build(root))
    }

    pub(crate) fn class_check_bindings(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::class_check::Bindings {
        self.class_check_bindings
            .get_or_init(|| crate::cops::style::class_check::Bindings::build(root))
    }

    pub(crate) fn predicate_method_returns(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::naming::predicate_method::Returns {
        self.predicate_method_returns
            .get_or_init(|| crate::cops::naming::predicate_method::Returns::build(root))
    }

    pub(crate) fn memoized_name_fingerprints(
        &self,
    ) -> &std::sync::Mutex<crate::cops::naming::memoized_instance_variable_name::Fingerprints> {
        self.memoized_name_fingerprints
            .get_or_init(Default::default)
    }

    pub(crate) fn literal_assignment_scope_edges(
        &self,
    ) -> &std::sync::Mutex<crate::cops::lint::literal_assignment_in_condition::ScopeEdges> {
        self.literal_assignment_scope_edges
            .get_or_init(Default::default)
    }

    pub(crate) fn fetch_env_conditions(
        &self,
    ) -> &std::sync::Mutex<crate::cops::style::fetch_env_var::Conditions> {
        self.fetch_env_conditions.get_or_init(Default::default)
    }

    pub(crate) fn contains_dev_null(&self, root: &Node<'_>) -> bool {
        self.contains_dev_null
            .get_or_init(|| crate::cops::style::file_null::NullDevices::build(root))
            .contains_dev_null
    }

    pub(crate) fn documentation_comments(
        &self,
    ) -> &crate::cops::style::documentation::DocumentationComments {
        self.documentation_comments
            .get_or_init(|| crate::cops::style::documentation::DocumentationComments::build(self))
    }

    /// Eval documentation uses physical source lines, not parser comments.
    /// Index once: many eval calls/interpolations on one line must not rescan it.
    pub(crate) fn line_has_non_interpolation_hash(&self, line: u32) -> bool {
        let lines = self.non_interpolation_hash_lines.get_or_init(|| {
            self.bytes
                .split(|&b| b == b'\n')
                .map(|line| {
                    line.iter()
                        .enumerate()
                        .any(|(i, &b)| b == b'#' && line.get(i + 1) != Some(&b'{'))
                })
                .collect()
        });
        line.checked_sub(1)
            .and_then(|i| lines.get(i as usize))
            .copied()
            .unwrap_or(false)
    }

    pub(crate) fn method_visibility(
        &self,
    ) -> &std::sync::Mutex<crate::cops::style::documentation_method::MethodVisibility> {
        self.method_visibility.get_or_init(Default::default)
    }

    pub(crate) fn modifier_index(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::if_unless_modifier::ModifierIndex {
        self.modifier_index.get_or_init(|| {
            crate::cops::style::if_unless_modifier::ModifierIndex::build(root, self)
        })
    }

    pub(crate) fn redundant_double_splats(
        &self,
        root: &ruby_prism::Node<'_>,
    ) -> &std::collections::HashSet<usize> {
        self.redundant_double_splats
            .get_or_init(|| crate::cops::style::redundant_double_splat_hash_braces::index(root))
    }

    pub(crate) fn redundant_conditions(
        &self,
    ) -> &std::sync::Mutex<std::collections::HashSet<(usize, usize)>> {
        self.redundant_conditions.get_or_init(Default::default)
    }

    pub(crate) fn identical_branch_edges(
        &self,
    ) -> &std::sync::Mutex<std::collections::HashSet<(usize, usize)>> {
        self.identical_branch_edges.get_or_init(Default::default)
    }

    /// Parse once before cop dispatch; filtering and dependent cops share it.
    pub(crate) fn directives_with_disabled(
        &self,
        result: &ParseResult<'_>,
        disabled: &[&str],
    ) -> &crate::directives::Directives {
        self.directives.get_or_init(|| {
            crate::directives::Directives::parse_with_disabled(result, self, disabled)
        })
    }

    /// The main-parse directive cache is shared with branch comment policy.
    /// Callers initialize it in check_parse, never reparse from check_node.
    pub(crate) fn comments_contain_disables(&self, cop: &str, lines: Range<u32>) -> bool {
        self.directives
            .get()
            .is_some_and(|d| d.contains_disables(cop, lines))
    }

    pub(crate) fn record_directive_disabled(&self, names: &[String]) {
        let _ = self.directive_disabled.set(names.to_vec());
    }

    pub(crate) fn directive_disabled(&self) -> &[String] {
        self.directive_disabled.get_or_init(Vec::new)
    }

    /// Whether no directive disables `cop` on any line of `first..=last`
    /// (RuboCop's `enabled_lines?`). Cops that, like upstream, only ignore
    /// the subtree of an offense whose block ran ask this during dispatch.
    pub(crate) fn cop_enabled_for_lines(&self, cop: &str, first: u32, last: u32) -> bool {
        self.directives
            .get()
            .is_none_or(|d| d.is_enabled_for_lines(cop, first, last))
    }

    pub(crate) fn line_length_enabled(&self, line: u32) -> bool {
        self.directives
            .get()
            .is_none_or(|d| d.is_enabled("Layout/LineLength", line))
    }

    /// Record encoding and frozen-string metadata from the main parse, before
    /// mapping either diagnostics or cop offenses. A no-op once known.
    pub(crate) fn record_encoding(&self, result: &ParseResult<'_>) {
        let _ = self
            .encoding
            .set(encoding::ParsedEncoding::from_parse(result));
    }

    fn parsed_encoding(&self) -> &encoding::ParsedEncoding {
        self.encoding.get_or_init(|| {
            encoding::ParsedEncoding::from_parse(&crate::parser::parse_as(
                &self.bytes,
                self.ruby_version,
            ))
        })
    }

    /// Validate decoded literal bytes in the main parse's source encoding.
    /// Callers handle forced UTF-8/binary literal flags separately.
    pub(crate) fn valid_source_encoded_bytes(&self, bytes: &[u8]) -> bool {
        self.parsed_encoding().valid(bytes)
    }

    /// Width in the main parse encoding, also for decoded symbol payloads.
    pub(crate) fn encoded_character_width(&self, bytes: &[u8]) -> usize {
        self.parsed_encoding().width(bytes)
    }

    /// Raw width and projected character count, honoring UTF8-MAC composition.
    pub(crate) fn composed_character_chunk(&self, bytes: &[u8]) -> (usize, usize) {
        self.parsed_encoding().composed_character_chunk(bytes)
    }

    pub(crate) fn word_array_matrices(
        &self,
    ) -> &std::sync::Mutex<std::collections::HashSet<usize>> {
        self.word_array_matrices.get_or_init(Default::default)
    }

    pub(crate) fn word_projection_required(&self) -> bool {
        self.parsed_encoding().word_projection_required()
    }

    pub(crate) fn word_prefix(&self, bytes: &[u8]) -> Option<(usize, usize)> {
        self.parsed_encoding().word_prefix(bytes)
    }

    pub(crate) fn word_character(&self, bytes: &[u8], projected: bool) -> bool {
        self.parsed_encoding().word(bytes, projected)
    }

    /// Bounded reference-buffer projection for DuplicateMatchPattern identities,
    /// DuplicateMagicComment header tokens and column counts, not diagnostic
    /// text or a global decoded source view.
    pub(crate) fn match_pattern_source<'a>(&self, bytes: &'a [u8]) -> std::borrow::Cow<'a, [u8]> {
        self.parsed_encoding().match_pattern_source(bytes)
    }

    /// Only this reference-buffer profile composes distinct identifier spellings.
    pub(crate) fn match_pattern_composes_identifiers(&self) -> bool {
        self.parsed_encoding().match_pattern_composes_identifiers()
    }

    pub(crate) fn match_pattern_binary(&self) -> bool {
        self.parsed_encoding().match_pattern_binary()
    }

    /// Unicode scalar escaped by Ruby inspection of already projected source.
    pub(crate) fn inspection_nonprint(&self, character: &[u8]) -> Option<u32> {
        encoding::inspection_nonprint(character)
    }

    pub(crate) fn binary_encoding(&self) -> bool {
        self.parsed_encoding().binary
    }

    pub(crate) fn frozen_string_setting(&self) -> Option<bool> {
        self.parsed_encoding().frozen_strings
    }

    pub(crate) fn encoding(&self) -> SourceEncoding {
        self.parsed_encoding().blanks
    }

    /// Non-ASCII case folds in the three fixed InclusiveLanguage terms.
    pub(crate) fn inclusive_case_fold(&self, bytes: &[u8]) -> [u8; 2] {
        self.parsed_encoding().inclusive_fold(bytes)
    }

    /// Canonical encoding identity for UTF-8 literal comparisons. Some Ruby
    /// encodings share UTF-8's character widths/blanks but not string equality.
    pub(crate) fn is_utf8(&self) -> bool {
        self.parsed_encoding().utf8
    }

    /// Whether Ruby upcase changes an identifier in the main parse's encoding.
    pub(crate) fn identifier_uppercase_changes(&self, bytes: &[u8]) -> bool {
        self.parsed_encoding().uppercase_changes(bytes)
    }

    pub(crate) fn heredoc_normalized<'a>(&self, bytes: &'a [u8]) -> std::borrow::Cow<'a, [u8]> {
        self.parsed_encoding().heredoc_normalized(bytes)
    }

    pub(crate) fn heredoc_ascii_letter(&self, bytes: &[u8]) -> bool {
        self.parsed_encoding().heredoc_ascii_letter(bytes)
    }

    pub(crate) fn heredoc_has_ascii_word(&self, bytes: &[u8]) -> bool {
        self.parsed_encoding().heredoc_has_ascii_word(bytes)
    }

    /// Source-encoded uppercase for exact heredoc body collision detection.
    pub(crate) fn heredoc_uppercase(&self, bytes: &[u8]) -> Option<Vec<u8>> {
        self.parsed_encoding().heredoc_uppercase(bytes)
    }

    /// POSIX uppercase in names, using the pinned source-buffer projection.
    pub(crate) fn identifier_has_uppercase(&self, bytes: &[u8]) -> bool {
        self.parsed_encoding().contains_uppercase(bytes)
    }

    /// POSIX lowercase in constant identifiers, using the main parse's encoding.
    pub(crate) fn identifier_has_lowercase(&self, bytes: &[u8]) -> bool {
        self.parsed_encoding().contains_lowercase(bytes)
    }

    /// POSIX alnum of the first source-buffer-projected character. Callers need
    /// at most six bytes (a CESU-8 surrogate pair); source widths stay unchanged.
    pub(crate) fn character_alphanumeric(&self, bytes: &[u8]) -> bool {
        self.parsed_encoding().alphanumeric_prefix(bytes)
    }

    pub(crate) fn record_string_escape_variable_end(&self, end: usize) {
        self.string_escape_variable_ends
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .insert(end);
    }

    #[cfg(test)]
    pub(crate) fn string_escape_variable_count(&self) -> Option<usize> {
        self.string_escape_variable_ends
            .get()
            .map(|ends| ends.lock().unwrap().len())
    }

    pub(crate) fn follows_string_escape_variable(&self, start: usize) -> bool {
        self.string_escape_variable_ends
            .get()
            .is_some_and(|ends| ends.lock().unwrap().contains(&start))
    }

    /// Absolute byte offsets of character starts within a token/line range.
    /// Spacing alignment uses these once per line instead of repeatedly
    /// counting prefixes, including for Ruby's declared legacy encodings.
    pub(crate) fn character_offsets(
        &self,
        range: Range<usize>,
    ) -> impl Iterator<Item = usize> + '_ {
        let encoding = *self.parsed_encoding();
        let mut offset = range.start;
        std::iter::from_fn(move || {
            if offset >= range.end {
                return None;
            }
            let start = offset;
            offset += encoding.width(&self.bytes[offset..range.end]);
            Some(start)
        })
    }

    pub(crate) fn inverse_method_nexts(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::inverse_methods::NextIndex {
        self.inverse_method_nexts
            .get_or_init(|| crate::cops::style::inverse_methods::NextIndex::build(root))
    }

    pub(crate) fn option_hash_supers(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::option_hash::SuperIndex {
        self.option_hash_supers
            .get_or_init(|| crate::cops::style::option_hash::SuperIndex::of(root))
    }

    pub(crate) fn super_facts(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::super_arguments::Facts {
        self.super_facts
            .get_or_init(|| crate::cops::style::super_arguments::Facts::of(root))
    }

    pub(crate) fn redundant_assignment_safety(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::redundant_assignment::Safety {
        self.redundant_assignment_safety
            .get_or_init(|| crate::cops::style::redundant_assignment::Safety::of(root))
    }

    pub(crate) fn initialize_comments(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::redundant_initialize::Comments {
        self.initialize_comments
            .get_or_init(|| crate::cops::style::redundant_initialize::Comments::build(root))
    }

    pub(crate) fn self_facts(&self, root: &Node<'_>) -> &crate::cops::style::redundant_self::Facts {
        self.self_facts
            .get_or_init(|| crate::cops::style::redundant_self::Facts::build(root))
    }

    pub(crate) fn class_emitter_siblings(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::naming::class_emitters::Siblings {
        self.class_emitter_siblings
            .get_or_init(|| crate::cops::naming::class_emitters::Siblings::build(root))
    }

    pub(crate) fn visible_locals(&self, root: &Node<'_>) -> &crate::cops::variables::VisibleLocals {
        self.visible_locals
            .get_or_init(|| crate::cops::variables::VisibleLocals::of(root))
    }

    /// Whether a byte range overlaps literal text (not interpolation code).
    /// `root` must be the main parse's root for this file. Collection is lazy
    /// and cached; repeated queries use binary search, never another AST walk.
    /// Percent arrays are conservatively protected in their entirety.
    pub fn overlaps_literal(&self, root: &Node<'_>, range: &Range<usize>) -> bool {
        let ranges = self
            .literal_ranges
            .get_or_init(|| literal_ranges::collect(self, root));
        let first = ranges.partition_point(|literal| literal.end <= range.start);
        ranges
            .get(first)
            .is_some_and(|literal| literal.start < range.end)
    }

    /// Whole literal interiors for subtree alignment, built in the same single
    /// shared safety-index walk as ordinary literal-text exclusions. The point
    /// query protects even empty indentation at an interior's start boundary.
    pub(crate) fn literal_interior_at(&self, root: &Node<'_>, offset: usize) -> bool {
        let ranges = &self
            .literal_ranges
            .get_or_init(|| literal_ranges::collect(self, root))
            .interiors;
        let index = ranges.partition_point(|range| range.end <= offset);
        ranges.get(index).is_some_and(|range| range.start <= offset)
    }

    fn token_ranges(&self, root: &Node<'_>) -> &literal_ranges::PunctuationRanges {
        self.token_ranges
            .get_or_init(|| literal_ranges::punctuation_ranges(self, root))
    }

    /// Literal token bytes, exposing interpolation code and percent-array
    /// separators. Unlike correction safety, these do not protect whole arrays.
    pub(crate) fn literal_token_ranges(&self, root: &Node<'_>) -> &[Range<usize>] {
        &self.token_ranges(root).protected
    }

    /// Original-byte heredoc excursions from the existing main-parse index.
    pub(crate) fn heredocs(&self, root: &Node<'_>) -> &[Heredoc] {
        &self.token_ranges(root).heredocs
    }

    /// Percent-array separator token starts, collected in the shared AST walk.
    pub(crate) fn percent_array_separators(&self, root: &Node<'_>) -> &[usize] {
        &self.token_ranges(root).percent_array_separators
    }

    /// A definition's physical last line, including a trailing heredoc body.
    /// Reuse the token collector's bottom-up extents: no per-definition walks,
    /// even for deeply nested definitions.
    pub(crate) fn definition_end_line(&self, root: &Node<'_>, node: &Node<'_>) -> u32 {
        let end = self
            .token_ranges(root)
            .definition_ends
            .get(&node.location().start_offset())
            .copied()
            .unwrap_or(node.location().end_offset());
        self.line_of(end.saturating_sub(1).max(node.location().start_offset()))
    }

    /// Whether a subtree contains a real heredoc opening (not a `<<` string
    /// lookalike). The shared collector propagates presence through actual AST
    /// parents once; each query is an exact cached lookup, not byte containment
    /// (deferred heredoc text may physically overlap an unrelated subtree).
    pub(crate) fn contains_heredoc(&self, root: &Node<'_>, node: &Node<'_>) -> bool {
        let location = node.location();
        self.token_ranges(root)
            .heredoc_subtrees
            .contains(&(location.start_offset(), location.end_offset()))
    }

    /// An earlier real heredoc opener on this physical line makes a new line
    /// unsafe even when the heredoc belongs to a sibling, not the edited node.
    /// Share the lazy token collection and binary-search it per correction.
    pub(crate) fn heredoc_pending_at(&self, root: &Node<'_>, offset: usize) -> bool {
        let openings = &self.token_ranges(root).heredoc_openings;
        let line_start = self.line_start(self.line_of(offset));
        let first = openings.partition_point(|&start| start < line_start);
        openings.get(first).is_some_and(|&start| start < offset)
    }

    /// Whether a real heredoc opener starts inside `range`, from the same
    /// shared sorted openings (one binary search, no subtree walk).
    pub(crate) fn heredoc_opening_in(&self, root: &Node<'_>, range: Range<usize>) -> bool {
        let openings = &self.token_ranges(root).heredoc_openings;
        let first = openings.partition_point(|&start| start < range.start);
        openings.get(first).is_some_and(|&start| start < range.end)
    }

    pub(crate) fn block_chain_facts(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::multiline_block_chain::Facts {
        self.block_chain_facts
            .get_or_init(|| crate::cops::style::multiline_block_chain::Facts::build(root, self))
    }

    pub(crate) fn it_parameter_facts(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::it_block_parameter::Facts {
        self.it_parameter_facts
            .get_or_init(|| crate::cops::style::it_block_parameter::Facts::build(root, self))
    }

    pub(crate) fn block_param_facts(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::single_line_block_params::Facts {
        self.block_param_facts
            .get_or_init(|| crate::cops::style::single_line_block_params::Facts::build(root, self))
    }

    pub(crate) fn do_end_facts(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::single_line_do_end_block::Facts {
        self.do_end_facts
            .get_or_init(|| crate::cops::style::single_line_do_end_block::Facts::build(root, self))
    }

    pub(crate) fn empty_parameter_facts(
        &self,
        root: &Node<'_>,
    ) -> &crate::cops::style::empty_parameters::Facts {
        self.empty_parameter_facts
            .get_or_init(|| crate::cops::style::empty_parameters::Facts::build(root))
    }

    /// Actual punctuation spacing outside literal text, comments and raw data.
    /// Cached across spacing cops, using the existing parse (never reparsing).
    pub(crate) fn punctuation(&self, root: &Node<'_>) -> &[punctuation::Punctuation] {
        &self
            .punctuation
            .get_or_init(|| punctuation::collect(self, root))
            .tokens
    }

    /// Real square brackets in source order, sharing the punctuation scan.
    pub(crate) fn square_brackets(&self, root: &Node<'_>) -> &[usize] {
        &self
            .punctuation
            .get_or_init(|| punctuation::collect(self, root))
            .square_brackets
    }

    /// Record the comments of `result`, a parse of this file's bytes, so
    /// [`SourceFile::comments`] needs no second parse. A no-op once known.
    pub fn record_comments(&self, result: &ParseResult<'_>) {
        let _ = self.comments.set(comment_ranges(result));
    }

    /// RuboCop's `processed_source.tokens` (stream order, not sorted): Prism's
    /// own lexer tokens projected as the pinned Prism-to-parser translator
    /// does. Computed once, by a token-only parse with the main options.
    pub(crate) fn parser_tokens(&self) -> &[tokens::Token] {
        self.parser_tokens.get_or_init(|| tokens::collect(self))
    }

    /// Line facts of RuboCop's PrecedingFollowingAlignment, shared by the
    /// spacing cops that allow alignment.
    pub(crate) fn preceding_following(&self) -> &crate::cops::layout::preceding_following::Lines {
        self.preceding_following
            .get_or_init(|| crate::cops::layout::preceding_following::Lines::build(self))
    }

    /// Byte ranges of every comment (`# …` and `=begin`/`=end`), in source
    /// order.
    pub fn comments(&self) -> &[Range<usize>] {
        self.comments.get_or_init(|| {
            comment_ranges(&crate::parser::parse_as(&self.bytes, self.ruby_version))
        })
    }

    pub(crate) fn directive_records(&self) -> &[crate::directives::comments::DirectiveComment] {
        self.directive_records
            .get_or_init(|| crate::directives::comments::collect(self))
    }

    /// Directive-like comments for conservative newline correction guards.
    /// Scan each comment's bytes once, even for many edits inside a large
    /// block comment. Detection/normal lint never needs this cache.
    pub(crate) fn directive_comments(&self) -> &[Range<usize>] {
        self.directive_comments.get_or_init(|| {
            self.comments()
                .iter()
                .filter(|comment| {
                    let text = &self.bytes[(*comment).clone()];
                    memchr::memmem::find(text, b"rubocop:").is_some()
                        || memchr::memmem::find(text, b"rustocop:").is_some()
                })
                .cloned()
                .collect()
        })
    }

    /// Cache the newline-edit guard once per file, not once per definition.
    pub(crate) fn line_value_boundary(&self) -> usize {
        *self.line_value_boundary.get_or_init(|| {
            memchr::memmem::rfind(&self.bytes, b"__LINE__").map_or(0, |offset| offset + 1)
        })
    }

    /// Record where the data section of `result`, a parse of this file's
    /// bytes, starts, so [`SourceFile::data_start`] needs no second parse. A
    /// no-op once known.
    pub fn record_data_start(&self, result: &ParseResult<'_>) {
        let _ = self.data_start.set(data_start(result));
    }

    /// The byte offset of the `__END__` marker line, if the file has a data
    /// section: everything from it on is data, not code.
    pub fn data_start(&self) -> Option<usize> {
        *self
            .data_start
            .get_or_init(|| data_start(&crate::parser::parse_as(&self.bytes, self.ruby_version)))
    }

    /// The number of lines before the data section (all of them when there
    /// is none): the lines RuboCop's line-oriented cops check.
    pub fn code_line_count(&self) -> usize {
        match self.data_start() {
            Some(offset) => self.line_of(offset) as usize - 1,
            None => self.line_count(),
        }
    }

    /// Whether a comment starts on one of `lines` (1-based, end-exclusive),
    /// like RuboCop's `each_comment_in_lines`.
    pub fn has_comment_in_lines(&self, lines: Range<u32>) -> bool {
        if lines.start >= lines.end {
            return false;
        }
        let start = self.line_start(lines.start);
        let end = self.line_start(lines.end);
        let comments = self.comments();
        let first = comments.partition_point(|comment| comment.start < start);
        comments
            .get(first)
            .is_some_and(|comment| comment.start < end)
    }

    /// Whether the source contains `needle` anywhere: a cheap prefilter for
    /// cops that can only fire when some token is spelled out.
    pub fn contains(&self, needle: &[u8]) -> bool {
        memchr::memmem::find(&self.bytes, needle).is_some()
    }

    /// The text of a 1-based line, without its `\n` (or `\r\n`) terminator;
    /// empty for lines past the end of the file.
    pub fn line_text(&self, line: u32) -> &[u8] {
        if line == 0 || line as usize > self.line_starts.len() {
            return &[];
        }
        let start = self.line_start(line);
        let end = self.line_start(line + 1);
        let text = &self.bytes[start..end];
        let text = text.strip_suffix(b"\n").unwrap_or(text);
        text.strip_suffix(b"\r").unwrap_or(text)
    }

    /// The byte offset at which a 1-based line starts; the end of the file
    /// for lines past it.
    pub fn line_start(&self, line: u32) -> usize {
        let index = (line as usize).saturating_sub(1);
        self.line_starts
            .get(index)
            .copied()
            .unwrap_or(self.bytes.len())
    }

    /// The 1-based line of a byte offset: [`SourceFile::line_col`] without
    /// the column count.
    pub fn line_of(&self, offset: usize) -> u32 {
        let offset = offset.min(self.bytes.len());
        let line_index = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        line_index as u32 + 1
    }

    /// The inclusive last line of an end-exclusive Prism source location.
    /// Empty locations stay on their starting line; a location ending at
    /// the start of a line does not include that line.
    pub fn last_line_of(&self, location: ruby_prism::Location<'_>) -> u32 {
        self.line_of(
            location
                .end_offset()
                .saturating_sub(1)
                .max(location.start_offset()),
        )
    }

    /// Map a byte offset to a 1-based (line, column) pair.
    ///
    /// Columns count characters in Ruby's declared source encoding (UTF-8 by
    /// default), excluding a leading UTF-8 BOM. Offsets within that BOM map to
    /// column 1. Invalid/incomplete characters in the prefix count one per byte.
    /// Raw source offsets are never transcoded or adjusted.
    pub fn line_col(&self, offset: usize) -> (u32, u32) {
        let offset = offset.min(self.bytes.len());
        let line_index = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let line_start = if line_index == 0 && self.bytes.starts_with(b"\xef\xbb\xbf") {
            // Keep raw offsets intact; only the displayed column ignores the BOM.
            3.min(offset)
        } else {
            self.line_starts[line_index]
        };
        let encoding = *self.parsed_encoding();
        let columns = self
            .columns
            .get_or_init(|| columns::ColumnIndex::build(&self.bytes, &self.line_starts, encoding));
        let col = columns.count_chars(&self.bytes, line_start, offset, encoding) as u32 + 1;
        (line_index as u32 + 1, col)
    }

    /// Map many offsets with the same semantics as `line_col`, retaining input
    /// order and duplicates. Sorting queries lets us scan each queried line's
    /// prefix only once, rather than once per diagnostic on a long rejected line.
    pub(crate) fn line_cols(&self, offsets: impl IntoIterator<Item = usize>) -> Vec<(u32, u32)> {
        let mut queries: Vec<_> = offsets
            .into_iter()
            .enumerate()
            .map(|(index, offset)| (offset.min(self.bytes.len()), index))
            .collect();
        queries.sort_unstable();
        let mut positions = vec![(1, 1); queries.len()];
        if queries.is_empty() {
            return positions;
        }
        let encoding = *self.parsed_encoding();
        let mut current_line = 0;
        let mut cursor = 0;
        let mut line_end = 0;
        let mut characters = 0;
        for (offset, index) in queries {
            let line = self.line_of(offset);
            if line != current_line {
                current_line = line;
                cursor = self.line_start(line);
                if line == 1 && self.bytes.starts_with(b"\xef\xbb\xbf") {
                    cursor = 3;
                }
                line_end = self.line_start(line + 1);
                characters = 0;
            }
            while cursor < offset {
                let width = encoding.width(&self.bytes[cursor..line_end]);
                if cursor + width > offset {
                    break;
                }
                cursor += width;
                characters += 1;
            }
            // A query can split a multibyte character. Count that incomplete
            // prefix exactly as the scalar mapper does, but keep the cursor at
            // the full character boundary for subsequent queries. BOM bytes
            // precede the cursor and always map to column 1.
            let partial = if cursor < offset {
                encoding.count_chars(&self.bytes[cursor..offset])
            } else {
                0
            };
            positions[index] = (line, (characters + partial) as u32 + 1);
        }
        positions
    }

    /// The number of lines in the file.
    pub fn line_count(&self) -> usize {
        let newlines = self.bytes.iter().filter(|b| **b == b'\n').count();
        if newlines > 0 && self.bytes.last() == Some(&b'\n') {
            newlines
        } else {
            newlines + usize::from(!self.bytes.is_empty())
        }
    }
}

/// The byte ranges of a parse's comments.
fn comment_ranges(result: &ParseResult<'_>) -> Vec<Range<usize>> {
    result
        .comments()
        .map(|comment| {
            let location = comment.location();
            location.start_offset()..location.end_offset()
        })
        .collect()
}

/// Where a parse's data section (`__END__` and after) starts.
fn data_start(result: &ParseResult<'_>) -> Option<usize> {
    result.data_loc().map(|location| location.start_offset())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_modifier_branch_comparisons_are_cached_by_real_edge_and_file() {
        use crate::cops::{Cop, lint::UselessMethodDefinition, run_cops_with_corrections};
        let cops: [&dyn Cop; 1] = [&UselessMethodDefinition];
        let body: String = (0..100)
            .map(|i| format!("private def f{i}; super; end;\n"))
            .collect();
        for (other, expected) in [(&body[..], 200), ("nil", 0)] {
            let text = format!("if ({body}); ({other});end");
            let file = SourceFile::new("x.rb".into(), text.into_bytes());
            let parsed = crate::parser::parse(&file.bytes);
            assert!(
                file.method_definition_macro_branches
                    .lock()
                    .unwrap()
                    .is_empty()
            );
            for corrections in [false, true] {
                let mut out = Vec::new();
                assert!(
                    run_cops_with_corrections(&cops, &parsed, &file, &mut out, corrections)
                        .is_empty()
                );
                assert_eq!(out.len(), expected);
                assert_eq!(
                    file.method_definition_macro_branches.lock().unwrap().len(),
                    if expected == 200 { 2 } else { 1 }
                );
                assert!(file.ruby2_keywords_facts.get().is_none());
            }
        }
    }

    #[test]
    fn ruby2_keywords_direct_children_are_lazy_shared_and_source_local() {
        use crate::cops::{Cop, lint::UselessRuby2Keywords, run_cops_with_corrections};
        let cops: [&dyn Cop; 1] = [&UselessRuby2Keywords];
        for text in [
            "nil",
            "ruby2_keywords",
            "ruby2_keywords def foo;end",
            "obj&.ruby2_keywords :foo",
        ] {
            let file = SourceFile::new("x.rb".into(), text.as_bytes().to_vec());
            let parsed = crate::parser::parse(&file.bytes);
            let mut out = Vec::new();
            assert!(run_cops_with_corrections(&cops, &parsed, &file, &mut out, false).is_empty());
            assert!(file.ruby2_keywords_facts.get().is_none());
        }
        let mut text = String::new();
        for i in 0..200 {
            text.push_str(&format!("ruby2_keywords :f{i}\ndef f{i};end\n"));
        }
        let file = SourceFile::new("x.rb".into(), text.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        for corrections in [false, true] {
            let mut out = Vec::new();
            assert!(
                run_cops_with_corrections(&cops, &parsed, &file, &mut out, corrections).is_empty()
            );
            assert_eq!(out.len(), 200);
            let facts = file.ruby2_keywords_facts.get().unwrap();
            assert!(std::ptr::eq(
                facts,
                file.ruby2_keywords_facts(&parsed.node())
            ));
            assert!(file.visible_locals.get().is_none());
            assert!(file.useless_rescue_facts.get().is_none());
        }
        let file = SourceFile::new(
            "x.rb".into(),
            b"ruby2_keywords :f199; def f199(*args);end".to_vec(),
        );
        let parsed = crate::parser::parse(&file.bytes);
        let mut out = Vec::new();
        assert!(run_cops_with_corrections(&cops, &parsed, &file, &mut out, true).is_empty());
        assert!(out.is_empty());
    }

    #[test]
    fn useless_rescue_facts_are_lazy_shared_and_source_local() {
        use crate::cops::{Cop, lint::UselessRescue, run_cops_with_corrections};
        let cops: [&dyn Cop; 1] = [&UselessRescue];
        for text in [
            "nil",
            "work rescue raise",
            "begin; work; rescue => e; raise; end",
            "begin; work; rescue; raise; ensure; use(e); end",
        ] {
            let file = SourceFile::new("x.rb".into(), text.as_bytes().to_vec());
            let parsed = crate::parser::parse(&file.bytes);
            for corrections in [false, true] {
                let mut out = Vec::new();
                assert!(
                    run_cops_with_corrections(&cops, &parsed, &file, &mut out, corrections)
                        .is_empty()
                );
                assert!(file.useless_rescue_facts.get().is_none());
            }
        }
        let mut text = "begin\n".to_string();
        for i in 0..200 {
            text.push_str(&format!("begin; work; rescue => e{i}; raise; end\n"));
        }
        text.push_str("ensure\n use(e199)\nend\n");
        let file = SourceFile::new("x.rb".into(), text.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        for corrections in [false, true] {
            let mut out = Vec::new();
            assert!(
                run_cops_with_corrections(&cops, &parsed, &file, &mut out, corrections).is_empty()
            );
            assert_eq!(out.len(), 199);
            let facts = file.useless_rescue_facts.get().unwrap();
            assert!(std::ptr::eq(
                facts,
                file.useless_rescue_facts(&parsed.node())
            ));
            assert!(file.visible_locals.get().is_none());
            assert!(file.missing_super_facts.get().is_none());
        }
        let file = SourceFile::new(
            "x.rb".into(),
            b"begin; work; rescue => e199; raise; ensure; nil; end".to_vec(),
        );
        let parsed = crate::parser::parse(&file.bytes);
        let mut out = Vec::new();
        assert!(run_cops_with_corrections(&cops, &parsed, &file, &mut out, false).is_empty());
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn missing_super_facts_are_lazy_cached_and_source_local_in_both_modes() {
        use crate::cops::{Cop, lint::MissingSuper, run_cops_with_corrections};
        let cops: [&dyn Cop; 1] = [&MissingSuper];
        for source in [
            "nil",
            "def ordinary; super; end",
            "class C < Object; def initialize; end; end",
            "Class.new(Parent) { def inherited; end }",
        ] {
            let file = SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
            let parsed = crate::parser::parse(&file.bytes);
            for corrections in [false, true] {
                let mut out = Vec::new();
                assert!(
                    run_cops_with_corrections(&cops, &parsed, &file, &mut out, corrections)
                        .is_empty()
                );
                assert!(out.is_empty());
                assert!(file.missing_super_facts.get().is_none());
            }
        }
        // Nested overlapping def subtrees must share one postorder fact pass.
        let mut source = "class C < Parent; def initialize;".to_string();
        for i in 0..20 {
            source.push_str(&format!("def nested_{i};"));
        }
        source.push_str("super;");
        source.push_str(&"end;".repeat(22));
        let file = SourceFile::new("x.rb".into(), source.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        for corrections in [false, true] {
            let mut out = Vec::new();
            assert!(
                run_cops_with_corrections(&cops, &parsed, &file, &mut out, corrections).is_empty()
            );
            assert!(out.is_empty());
            let facts = file.missing_super_facts.get().unwrap();
            assert!(std::ptr::eq(
                facts,
                file.missing_super_facts(&parsed.node())
            ));
            assert!(file.useless_access_modifiers.get().is_none());
            assert!(file.literal_assignment_scope_edges.get().is_none());
        }
        let other = SourceFile::new(
            "x.rb".into(),
            b"class C < Parent; def initialize; end; end".to_vec(),
        );
        let parsed = crate::parser::parse(&other.bytes);
        let mut out = Vec::new();
        assert!(run_cops_with_corrections(&cops, &parsed, &other, &mut out, false).is_empty());
        assert_eq!(out.len(), 1);
        assert!(out[0].correction.is_none());
    }

    #[test]
    fn ineffective_visibility_names_are_lazy_in_lint_and_fix_modes() {
        use crate::cops::{Cop, lint::IneffectiveAccessModifier, run_cops_with_corrections};
        let cops: [&dyn Cop; 1] = [&IneffectiveAccessModifier];
        for source in [
            "class C;public;def self.f;end;end",
            "class C;private;def f;end;end",
            "class C;begin;private;def self.f;end;end;end",
        ] {
            let file = SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
            let parsed = crate::parser::parse(&file.bytes);
            for corrections in [false, true] {
                let mut out = Vec::new();
                assert!(
                    run_cops_with_corrections(&cops, &parsed, &file, &mut out, corrections)
                        .is_empty()
                );
                assert!(out.is_empty());
                assert!(file.ineffective_private_names.get().is_none());
            }
        }
        let file = SourceFile::new(
            "x.rb".into(),
            b"class C;private;def self.f;end;end".to_vec(),
        );
        let parsed = crate::parser::parse(&file.bytes);
        for corrections in [false, true] {
            let mut out = Vec::new();
            assert!(
                run_cops_with_corrections(&cops, &parsed, &file, &mut out, corrections).is_empty()
            );
            assert_eq!(out.len(), 1);
            assert!(out[0].correction.is_none());
            assert!(file.ineffective_private_names.get().is_some());
        }
    }

    #[test]
    fn decoded_literal_validity_uses_real_encoding_widths_not_column_fallbacks() {
        for (header, valid, invalid) in [
            ("UTF-8", b"\xc3\xa9".as_slice(), Some(b"\xff".as_slice())),
            ("US-ASCII", b"ascii".as_slice(), Some(b"\xff".as_slice())),
            (
                "Windows-31J",
                b"\x83\\".as_slice(),
                Some(b"\x83".as_slice()),
            ),
            ("ISO-8859-1", b"\xff".as_slice(), None),
            ("ASCII-8BIT", b"\xff".as_slice(), None),
        ] {
            let file = SourceFile::new(
                "x.rb".into(),
                format!("# encoding: {header}\n'x'\n").into_bytes(),
            );
            let parsed = crate::parser::parse(&file.bytes);
            assert_eq!(parsed.errors().count(), 0);
            file.record_encoding(&parsed);
            assert!(file.valid_source_encoded_bytes(valid), "{header}");
            assert!(file.valid_source_encoded_bytes(b""), "{header}");
            if let Some(invalid) = invalid {
                assert!(!file.valid_source_encoded_bytes(invalid), "{header}");
            }
        }
    }

    #[test]
    fn hash_fetch_and_lookup_facts_are_lazy_linear_and_source_local() {
        use crate::cops::{
            Cop, run_cops,
            style::{HashFetchChain, HashLookupMethod},
        };
        let lookup = HashLookupMethod::default();
        let cops: [&dyn Cop; 2] = [&HashFetchChain, &lookup];
        let clean = SourceFile::new("x.rb".into(), b"h[key]\n".to_vec());
        let parsed = crate::parser::parse(&clean.bytes);
        let mut out = vec![];
        assert!(run_cops(&cops, &parsed, &clean, &mut out).is_empty());
        assert!(clean.hash_fetch_facts.get().is_none());
        assert!(clean.hash_lookup_facts.iter().all(|c| c.get().is_none()));
        let chain = format!("h{}.fetch(:last,nil)\n", ".fetch(:a,{})".repeat(48));
        let source = format!("{chain}h.fetch(h.fetch(h.fetch(:key)))\n").repeat(6000);
        let file = SourceFile::new("x.rb".into(), source.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        assert_eq!(parsed.errors().count(), 0);
        out.clear();
        assert!(run_cops(&cops, &parsed, &file, &mut out).is_empty());
        assert_eq!(out.len(), 12000);
        // HashFetchChain corrects its 6000 chains; HashLookupMethod does not.
        assert_eq!(out.iter().filter(|o| o.correction.is_some()).count(), 6000);
        let fetch = file.hash_fetch_facts.get().unwrap();
        assert_eq!(fetch.links, 49 * 6000);
        assert!(fetch.visits <= 6000 * 330 + 2, "{}", fetch.visits);
        let lookup = file.hash_lookup_facts[1].get().unwrap();
        assert_eq!(fetch.visits, lookup.visits);
        assert!(file.hash_lookup_facts[0].get().is_none());
        for _ in 0..6000 {
            assert!(std::ptr::eq(fetch, file.hash_fetch_facts(&parsed.node())));
            assert!(std::ptr::eq(
                lookup,
                file.hash_lookup_facts(&parsed.node(), true)
            ));
        }
        assert!(clean.hash_fetch_facts.get().is_none());
    }

    #[test]
    fn set_partition_and_reduce_have_bounded_main_ast_work() {
        use crate::cops::{
            Cop, run_cops,
            style::{MapToSet, PartitionInsteadOfDoubleSelect, ReduceToHash},
        };
        let cops: [&dyn Cop; 3] = [&MapToSet, &PartitionInsteadOfDoubleSelect, &ReduceToHash];
        for s in [
            "xs.map.to_set",
            "xs.reduce(0) { |a,e| a+e }",
            "xs.select(&:ok?)",
        ] {
            let f = SourceFile::new("x.rb".into(), s.as_bytes().to_vec());
            let parsed = crate::parser::parse(&f.bytes);
            let mut out = vec![];
            assert!(run_cops(&cops, &parsed, &f, &mut out).is_empty());
            assert!(f.reduce_to_hash_facts.get().is_none());
        }
        let s="xs.map { it*2 }.to_set\nxs.select(&:ok?)\nxs.reject(&:ok?)\nxs.each_with_object({}) { |e,h| h[e]=e }\n".repeat(6000);
        let f = SourceFile::new("x.rb".into(), s.into_bytes());
        let parsed = crate::parser::parse(&f.bytes);
        let mut out = vec![];
        assert!(run_cops(&cops, &parsed, &f, &mut out).is_empty());
        assert_eq!(out.len(), 18000);
        // MapToSet corrects its 6000 calls.
        assert_eq!(out.iter().filter(|o| o.correction.is_some()).count(), 6000);
        let facts = f.reduce_to_hash_facts.get().unwrap();
        assert!(facts.visits <= 6000 * 50 + 2, "{}", facts.visits);
        for _ in 0..6000 {
            assert!(std::ptr::eq(facts, f.reduce_to_hash_facts(&parsed.node())));
        }
        let other = SourceFile::new("other.rb".into(), vec![]);
        assert!(other.reduce_to_hash_facts.get().is_none());
    }

    #[test]
    fn map_destination_index_is_lazy_shared_bounded_and_source_local() {
        use crate::cops::{Cop, run_cops, style::MapIntoArray};
        let cops: [&dyn Cop; 1] = [&MapIntoArray];
        for s in [
            "src.each {}",
            "src.each { |x| x << 1 }",
            "dest=[]; src.map { |x| dest << x }",
        ] {
            let file = SourceFile::new("x.rb".into(), s.as_bytes().to_vec());
            let parsed = crate::parser::parse(&file.bytes);
            let mut out = vec![];
            assert!(run_cops(&cops, &parsed, &file, &mut out).is_empty());
            // A syntactic local push candidate may require the index even if
            // its lexical identity turns out to be a parameter, not a mapping.
            if !s.contains("x <<") {
                assert!(file.map_into_array_facts.get().is_none());
            }
        }
        let s = "dest=[]; src.each { |e| dest << e }; dest\n".repeat(6000);
        let file = SourceFile::new("x.rb".into(), s.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        let mut out = vec![];
        assert!(run_cops(&cops, &parsed, &file, &mut out).is_empty());
        assert_eq!(out.len(), 6000);
        assert!(out.iter().all(|o| o.correction.is_none()));
        let facts = file.map_into_array_facts.get().unwrap();
        assert!(facts.visits <= 6000 * 18 + 2, "{}", facts.visits);
        for _ in 0..6000 {
            assert!(std::ptr::eq(
                facts,
                file.map_into_array_facts(&parsed.node())
            ));
        }
        let other = SourceFile::new("other.rb".into(), b"dest=[]".to_vec());
        assert!(other.map_into_array_facts.get().is_none());
    }

    #[test]
    fn enumeration_indexes_are_lazy_shared_bounded_and_source_local() {
        use crate::cops::{
            Cop, run_cops, run_cops_with_corrections,
            style::{EachForSimpleLoop, EachWithObject},
        };
        let cops: [&dyn Cop; 2] = [&EachForSimpleLoop, &EachWithObject];
        for s in [
            "[].reduce(0) { |a,e| a }",
            "[].reduce({}) { |a| a }",
            "(0..3).each {}",
        ] {
            let file = SourceFile::new("x.rb".into(), s.as_bytes().to_vec());
            let parsed = crate::parser::parse(&file.bytes);
            let mut out = vec![];
            assert!(run_cops_with_corrections(&cops, &parsed, &file, &mut out, false).is_empty());
            assert!(file.each_with_object_facts.get().is_none(), "{s}");
        }
        let s = "[].reduce({}) { |a,e| nested { a[e]=e }; a }\n[].reduce({}) { |a,e| a=e; a }\n"
            .repeat(6000);
        let file = SourceFile::new("x.rb".into(), s.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        let mut out = vec![];
        assert!(run_cops(&cops, &parsed, &file, &mut out).is_empty());
        assert_eq!(out.len(), 6000);
        let facts = file.each_with_object_facts.get().unwrap();
        assert!(facts.visits <= 6000 * 65 + 2, "{}", facts.visits);
        for _ in 0..6000 {
            assert!(std::ptr::eq(
                facts,
                file.each_with_object_facts(&parsed.node())
            ));
        }
        let other = SourceFile::new("other.rb".into(), b"[].reduce(0)".to_vec());
        assert!(other.each_with_object_facts.get().is_none());
    }

    #[test]
    fn hash_each_indexes_are_lazy_once_bounded_and_source_local() {
        use crate::cops::{Cop, run_cops, style::HashEachMethods};
        let cops: [&dyn Cop; 1] = [&HashEachMethods];
        for source in [
            "foo.each {}",
            "foo.each { p it }",
            "foo.each { |k,v,w| p k }",
            "foo.keys.each(&:bar)",
            "foo.each_pair { |k,v| p k }",
        ] {
            let file = SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
            let parsed = crate::parser::parse(&file.bytes);
            let mut out = vec![];
            assert!(run_cops(&cops, &parsed, &file, &mut out).is_empty());
            assert!(file.hash_each_facts.get().is_none(), "{source}");
        }
        let source = "h.chain.more.keys.each { |k| other[k] = k }\nh.each { |k,v| nested { |k| p k } }\nh.keys.each { |k| h[k],x = values }\n".repeat(6000);
        let file = SourceFile::new("x.rb".into(), source.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        assert!(file.hash_each_facts.get().is_none());
        let mut out = vec![];
        assert!(run_cops(&cops, &parsed, &file, &mut out).is_empty());
        assert_eq!(out.len(), 12000);
        assert!(out.iter().all(|o| o.correction.is_some()));
        let facts = file.hash_each_facts.get().unwrap();
        assert!(facts.visits <= 6000 * 65 + 2, "{}", facts.visits);
        for _ in 0..6000 {
            assert!(std::ptr::eq(facts, file.hash_each_facts(&parsed.node())));
        }
        let other = SourceFile::new("other.rb".into(), b"h.each {}".to_vec());
        assert!(other.hash_each_facts.get().is_none());
    }

    #[test]
    fn hash_transform_indexes_are_lazy_shared_and_bounded_for_wide_and_nested_bodies() {
        use crate::cops::{
            Cop, run_cops,
            style::{HashTransformKeys, HashTransformValues},
        };
        let cops: [&dyn Cop; 2] = [&HashTransformKeys, &HashTransformValues];
        for s in [
            "{}.map { |k,v| [k,v] }.to_h",
            "unknown.map { |k,v| [k.to_s,v] }.to_h",
            "{}.to_h { it }",
            "{}.map { |k,v| [k.to_s,v.to_s] }.to_h",
        ] {
            let file = SourceFile::new("x.rb".into(), s.as_bytes().to_vec());
            let mut out = vec![];
            assert!(
                run_cops(&cops, &crate::parser::parse(&file.bytes), &file, &mut out).is_empty()
            );
            assert!(file.hash_transform_facts.get().is_none());
        }
        let nested = format!(
            "{}k.to_s{}",
            "{}.to_h { |k,v| [".repeat(12),
            ",v] }".repeat(12)
        );
        let s =
            format!("{{}}.map {{ |k,v| [{nested},v] }}.to_h\n{{}}.to_h {{ |k,v| [k,v.to_s] }}\n")
                .repeat(6000);
        let file = SourceFile::new("x.rb".into(), s.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        let mut out = vec![];
        assert!(run_cops(&cops, &parsed, &file, &mut out).is_empty());
        // Only the innermost key transform: nested lexical v reads exclude
        // its outer candidates. Each row also has one value transform.
        assert_eq!(out.len(), 2 * 6000);
        assert!(out.iter().all(|o| o.correction.is_none()));
        let facts = file.hash_transform_facts.get().unwrap();
        assert!(facts.visits <= 6000 * 240 + 2, "{}", facts.visits);
        for _ in 0..6000 {
            assert!(std::ptr::eq(
                facts,
                file.hash_transform_facts(&parsed.node())
            ));
        }
        assert!(file.visible_locals.get().is_none());
        let other = SourceFile::new("other.rb".into(), b"{}.to_h {}".to_vec());
        assert!(other.hash_transform_facts.get().is_none());
    }

    #[test]
    fn double_splat_index_is_lazy_shared_and_bounded_for_wide_merge_arguments() {
        use crate::cops::{
            Cop, run_cops_with_corrections,
            style::{RedundantDoubleSplatHashBraces, StringHashKeys},
        };
        let cops: [&dyn Cop; 2] = [&RedundantDoubleSplatHashBraces, &StringHashKeys];
        let clean = SourceFile::new("x.rb".into(), b"f(a: 1); {'a'=>1}".to_vec());
        let mut out = vec![];
        run_cops_with_corrections(
            &cops,
            &crate::parser::parse(&clean.bytes),
            &clean,
            &mut out,
            false,
        );
        assert!(clean.redundant_double_splats.get().is_none());
        let s = "f(**{a: 1, **{b: 2}}); {'a'=>1}\n".repeat(6000);
        let file = SourceFile::new("x.rb".into(), s.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        let mut out = vec![];
        assert!(run_cops_with_corrections(&cops, &parsed, &file, &mut out, false).is_empty());
        assert_eq!(out.len(), 18000);
        assert!(out.iter().all(|o| o.correction.is_none()));
        let facts = file.redundant_double_splats.get().unwrap();
        assert_eq!(facts.len(), 12000);
        for _ in 0..6000 {
            assert!(std::ptr::eq(
                facts,
                file.redundant_double_splats(&parsed.node())
            ));
        }
        let wide = format!("f(**{{a:1}}.merge({}))", vec!["{b:2}"; 6000].join(","));
        let file = SourceFile::new("x.rb".into(), wide.into_bytes());
        let mut out = vec![];
        assert!(
            run_cops_with_corrections(
                &cops,
                &crate::parser::parse(&file.bytes),
                &file,
                &mut out,
                false
            )
            .is_empty()
        );
        assert_eq!(out.len(), 1);
        assert_eq!(file.redundant_double_splats.get().unwrap().len(), 1);
    }

    #[test]
    fn dig_index_is_lazy_shared_and_receiver_chain_work_is_bounded() {
        use crate::cops::{
            Cop,
            ast::CALL_ARGUMENT_VISITS,
            run_cops_with_corrections,
            style::{DigChain, SingleArgumentDig},
        };
        let cops: [&dyn Cop; 2] = [&DigChain, &SingleArgumentDig::default()];
        let clean = SourceFile::new("x.rb".into(), b"x.other(:a)\n".to_vec());
        let parsed = crate::parser::parse(&clean.bytes);
        let mut out = vec![];
        assert!(run_cops_with_corrections(&cops, &parsed, &clean, &mut out, true).is_empty());
        assert!(clean.dig_calls.get().is_none());
        let source = format!("x{}\n", ".dig(:a)".repeat(30)).repeat(1000);
        let file = SourceFile::new("x.rb".into(), source.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        assert!(file.dig_calls.get().is_none());
        CALL_ARGUMENT_VISITS.set(0);
        assert!(run_cops_with_corrections(&cops, &parsed, &file, &mut out, true).is_empty());
        assert_eq!(out.len(), 1000);
        assert!(out.iter().all(|o| o.correction.is_none()));
        assert!(
            CALL_ARGUMENT_VISITS.get() < 150_000,
            "{}",
            CALL_ARGUMENT_VISITS.get()
        );
        let facts = file.dig_calls.get().unwrap();
        for _ in 0..1000 {
            assert!(std::ptr::eq(facts, file.dig_calls(&parsed.node())));
        }
        assert!(file.token_ranges.get().is_none());
    }

    fn file(bytes: &[u8]) -> SourceFile {
        SourceFile::new(PathBuf::from("test.rb"), bytes.to_vec())
    }

    #[test]
    fn eval_documentation_line_facts_are_lazy_cached_and_isolated() {
        let f = file(b"eval(\"#{x}\")\n# docs\neval(\"#@x #{y}\")\n#\r\n");
        assert!(f.non_interpolation_hash_lines.get().is_none());
        for _ in 0..10000 {
            assert!(!f.line_has_non_interpolation_hash(1));
            assert!(f.line_has_non_interpolation_hash(2));
            assert!(f.line_has_non_interpolation_hash(3));
            assert!(f.line_has_non_interpolation_hash(4));
            assert!(!f.line_has_non_interpolation_hash(5));
        }
        let facts = f.non_interpolation_hash_lines.get().unwrap();
        assert_eq!(facts.len(), f.line_starts.len());
        assert!(std::ptr::eq(
            facts,
            f.non_interpolation_hash_lines.get().unwrap()
        ));
        let other = file(b"#{not_a_comment}\n");
        assert!(!other.line_has_non_interpolation_hash(1));
        assert!(!other.line_has_non_interpolation_hash(100));
    }

    #[test]
    fn last_lines_exclude_the_end_offset_and_handle_empty_locations() {
        let empty = file(b"");
        let result = ruby_prism::parse(&empty.bytes);
        assert_eq!(empty.last_line_of(result.node().location()), 1);

        for source in ["<<~TEXT\n  é\nTEXT\n", "<<~TEXT\r\n  é\r\nTEXT\r\n"] {
            let f = file(source.as_bytes());
            let result = ruby_prism::parse(&f.bytes);
            let string = result
                .node()
                .as_program_node()
                .unwrap()
                .statements()
                .body()
                .iter()
                .next()
                .unwrap()
                .as_string_node()
                .unwrap();
            // A heredoc's closing location includes the terminator's newline.
            let closing = string.closing_loc().unwrap();
            assert_eq!(f.line_of(closing.end_offset()), 4);
            assert_eq!(f.last_line_of(closing), 3);
            // The literal's own source location is just its opener.
            assert_eq!(f.last_line_of(string.location()), 1);
        }
    }

    #[test]
    fn separator_and_paren_cops_share_one_cached_scan_of_the_main_parse() {
        use crate::cops::{
            Cop,
            layout::{SpaceAfterComma, SpaceAfterSemicolon, SpaceBeforeComma, SpaceInsideParens},
            run_cops_with_corrections,
        };
        // Thousands of nodes/ranges/queries, but a single AST collection and
        // one separator scan; subsequent cop invocations reuse the allocation.
        let source = "f( \"text,;#{g( 1 ,2 );3}\" ,4 );5\n".repeat(2000);
        let f = file(source.as_bytes());
        let result = ruby_prism::parse(&f.bytes);
        assert_eq!(result.errors().count(), 0);
        f.record_comments(&result);
        f.record_warnings(&result);
        f.record_data_start(&result);
        f.record_encoding(&result);
        let cops: &[&dyn Cop] = &[
            &SpaceAfterComma,
            &SpaceBeforeComma,
            &SpaceAfterSemicolon,
            &SpaceInsideParens,
        ];
        let mut found = Vec::new();
        assert!(f.punctuation.get().is_none());
        assert!(run_cops_with_corrections(cops, &result, &f, &mut found, false).is_empty());
        assert_eq!(found.len(), 20000);
        assert!(found.iter().all(|o| o.correction.is_none()));
        assert!(f.literal_ranges.get().is_none());
        let cached = &f.punctuation.get().unwrap().tokens;
        assert_eq!(cached.len(), 16000);
        let pointer = cached.as_ptr();
        for _ in 0..2000 {
            assert_eq!(f.punctuation(&result.node()).as_ptr(), pointer);
        }
        found.clear();
        assert!(run_cops_with_corrections(cops, &result, &f, &mut found, true).is_empty());
        assert_eq!(found.len(), 20000);
        assert!(found.iter().all(|o| o.correction.is_some()));
        assert_eq!(f.punctuation.get().unwrap().tokens.as_ptr(), pointer);
    }

    #[test]
    fn bracket_cops_share_cached_bracket_tokens_for_many_constant_patterns() {
        use crate::cops::{
            Cop,
            layout::{SpaceInsideArrayLiteralBrackets, SpaceInsideReferenceBrackets},
            run_cops_with_corrections,
        };
        let source = "x in A( [ a, b ] )\na = [ 1, [ 2 ] ]\na[ 0 ]\n".repeat(2000);
        let f = file(source.as_bytes());
        let result = ruby_prism::parse(&f.bytes);
        assert_eq!(result.errors().count(), 0);
        f.record_comments(&result);
        f.record_data_start(&result);
        f.record_encoding(&result);
        let cops: &[&dyn Cop] = &[
            &SpaceInsideArrayLiteralBrackets,
            &SpaceInsideReferenceBrackets,
        ];
        let mut found = Vec::new();
        assert!(run_cops_with_corrections(cops, &result, &f, &mut found, false).is_empty());
        assert_eq!(found.len(), 16000);
        assert!(found.iter().all(|o| o.correction.is_none()));
        assert!(f.literal_ranges.get().is_none());
        let brackets = f.square_brackets(&result.node());
        assert_eq!(brackets.len(), 16000);
        let pointer = brackets.as_ptr();
        for _ in 0..2000 {
            assert_eq!(f.square_brackets(&result.node()).as_ptr(), pointer);
        }
        found.clear();
        assert!(run_cops_with_corrections(cops, &result, &f, &mut found, true).is_empty());
        assert_eq!(found.len(), 16000);
        assert!(found.iter().all(|o| o.correction.is_some()));
        assert_eq!(f.square_brackets(&result.node()).as_ptr(), pointer);
        assert!(f.literal_ranges.get().is_none());
    }

    #[test]
    fn empty_lines_reuse_cached_token_ranges_with_bounded_many_offense_work() {
        use crate::cops::{
            layout::{EmptyLines, SpaceAfterComma},
            run_cops_with_corrections,
        };
        let source = "x = \"a\n\n\nb\"\nx\n\n\ny\nf(1,2)\n%w(a\\\n\n\nb)\n".repeat(2000);
        let f = file(source.as_bytes());
        let result = ruby_prism::parse(&f.bytes);
        assert_eq!(result.errors().count(), 0);
        f.record_comments(&result);
        f.record_data_start(&result);
        f.record_encoding(&result);
        let mut found = Vec::new();
        assert!(
            run_cops_with_corrections(
                &[&EmptyLines, &SpaceAfterComma],
                &result,
                &f,
                &mut found,
                false
            )
            .is_empty()
        );
        assert_eq!(found.len(), 4000);
        assert!(found.iter().all(|o| o.correction.is_none()));
        assert!(
            f.literal_ranges.get().is_none(),
            "no correction-only protection walk"
        );
        let pointer = f.token_ranges.get().unwrap().protected.as_ptr();
        let separators = f.percent_array_separators(&result.node());
        assert_eq!(separators.len(), 4000);
        let separator_pointer = separators.as_ptr();
        for _ in 0..2000 {
            assert_eq!(f.literal_token_ranges(&result.node()).as_ptr(), pointer);
            assert_eq!(
                f.percent_array_separators(&result.node()).as_ptr(),
                separator_pointer
            );
        }
        found.clear();
        assert!(
            run_cops_with_corrections(
                &[&EmptyLines, &SpaceAfterComma],
                &result,
                &f,
                &mut found,
                true
            )
            .is_empty()
        );
        assert_eq!(
            found.iter().filter(|o| o.correction.is_some()).count(),
            4000
        );
        assert_eq!(f.token_ranges.get().unwrap().protected.as_ptr(), pointer);
    }

    #[test]
    fn null_devices_index_is_lazy_reused_and_visits_each_node_once() {
        use crate::cops::{style::FileNull, testing::correction_offenses};
        let source = format!("devices = ['/dev/null']\n{}", "path = 'NUL'\n".repeat(2000));
        let file = SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        assert!(file.contains_dev_null.get().is_none());
        for _ in 0..2000 {
            assert!(file.contains_dev_null(&parsed.node()));
        }
        let index = file.contains_dev_null.get().unwrap();
        assert_eq!(
            index.visited,
            crate::cops::testing::preorder_kinds(&source).len()
        );
        assert!(index.visited < 10_000);
        assert!(file.file_api_bindings.get().is_none());
        let found = correction_offenses(&FileNull, &source);
        assert_eq!(found.len(), 2000);
        assert!(found.iter().all(|o| o.correction.is_none()));
    }

    #[test]
    fn comparable_clamp_uses_one_lazy_bounded_safety_walk() {
        use crate::cops::{run_cops_with_corrections, style::ComparableClamp};
        let source = "if 2 < 1; 1; elsif 3 < 2; 3; else; 2; end\n".repeat(6000);
        let file = SourceFile::new("x.rb".into(), source.into_bytes());
        let result = ruby_prism::parse(&file.bytes);
        for corrections in [false, true, true] {
            let mut found = vec![];
            assert!(
                run_cops_with_corrections(
                    &[&ComparableClamp],
                    &result,
                    &file,
                    &mut found,
                    corrections
                )
                .is_empty()
            );
            assert_eq!(found.len(), 6000);
            assert_eq!(
                found.iter().filter(|o| o.correction.is_some()).count(),
                if corrections { 6000 } else { 0 }
            );
            if !corrections {
                assert!(file.comparable_clamp_safety.get().is_none());
            }
        }
        let index = file.comparable_clamp_safety.get().unwrap();
        assert_eq!(
            index.visits,
            crate::cops::testing::preorder_kinds(std::str::from_utf8(&file.bytes).unwrap()).len()
        );
        assert!(index.visits < 150000);
        assert!(std::ptr::eq(
            index,
            file.comparable_clamp_safety(&result.node())
        ));
        for s in [
            "if x < low; low; elsif high < x; high; else; x; end",
            "[[2,1].max,3].min",
        ] {
            let file = SourceFile::new("x.rb".into(), s.as_bytes().to_vec());
            let result = ruby_prism::parse(&file.bytes);
            let mut found = vec![];
            assert!(
                run_cops_with_corrections(&[&ComparableClamp], &result, &file, &mut found, true)
                    .is_empty()
            );
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none());
            assert!(file.comparable_clamp_safety.get().is_none());
        }
    }

    #[test]
    fn object_then_uses_one_lazy_bounded_binding_walk() {
        use crate::cops::{run_cops_with_corrections, style::ObjectThen};
        let file = SourceFile::new(
            "x.rb".into(),
            "x.yield_self { |e| e }\n".repeat(3000).into_bytes(),
        );
        let result = ruby_prism::parse(&file.bytes);
        for corrections in [false, true, true] {
            let mut found = Vec::new();
            assert!(
                run_cops_with_corrections(&[&ObjectThen], &result, &file, &mut found, corrections)
                    .is_empty()
            );
            assert_eq!(found.len(), 3000);
            assert_eq!(
                found.iter().filter(|o| o.correction.is_some()).count(),
                if corrections { 3000 } else { 0 }
            );
            if !corrections {
                assert!(file.object_then_bindings.get().is_none());
            }
        }
        let index = file.object_then_bindings.get().unwrap();
        assert_eq!(
            index.visited,
            crate::cops::testing::preorder_kinds(std::str::from_utf8(&file.bytes).unwrap()).len()
        );
        assert!(index.visited < 30000);
        assert!(std::ptr::eq(
            index,
            file.object_then_bindings(&result.node())
        ));
    }

    #[test]
    fn class_checks_use_one_lazy_binding_walk_not_one_per_offense() {
        use crate::cops::{run_cops_with_corrections, style::ClassCheck};
        let source = "x.kind_of?(String)\n".repeat(3000);
        let file = SourceFile::new("x.rb".into(), source.into_bytes());
        let result = ruby_prism::parse(&file.bytes);
        let mut found = Vec::new();
        for corrections in [false, true, true] {
            found.clear();
            assert!(
                run_cops_with_corrections(&[&ClassCheck], &result, &file, &mut found, corrections)
                    .is_empty()
            );
            assert_eq!(found.len(), 3000);
            assert_eq!(
                found.iter().filter(|o| o.correction.is_some()).count(),
                if corrections { 3000 } else { 0 }
            );
            if !corrections {
                assert!(file.class_check_bindings.get().is_none());
            }
        }
        let index = file.class_check_bindings.get().unwrap();
        assert_eq!(
            index.visited,
            crate::cops::testing::preorder_kinds(std::str::from_utf8(&file.bytes).unwrap()).len()
        );
        assert!(index.visited < 16000);
        assert!(std::ptr::eq(
            index,
            file.class_check_bindings(&result.node())
        ));
    }

    #[test]
    fn method_object_class_check_guards_share_one_lazy_bounded_walk() {
        use crate::cops::{run_cops_with_corrections, style::ClassCheck};
        for (selector, correctable) in [
            ("object_id", true),
            ("define_singleton_method", false),
            ("new", false),
            ("define", false),
        ] {
            let source = format!(
                "{}{}",
                "obj.kind_of?(Object)\n".repeat(3000),
                format!("binder = obj.method(:{selector})\n").repeat(1000)
            );
            let file = SourceFile::new("x.rb".into(), source.into_bytes());
            let result = ruby_prism::parse(&file.bytes);
            for corrections in [false, true, true] {
                let mut found = Vec::new();
                assert!(
                    run_cops_with_corrections(
                        &[&ClassCheck],
                        &result,
                        &file,
                        &mut found,
                        corrections
                    )
                    .is_empty()
                );
                assert_eq!(found.len(), 3000);
                assert_eq!(
                    found.iter().filter(|o| o.correction.is_some()).count(),
                    if corrections && correctable { 3000 } else { 0 }
                );
                if !corrections {
                    assert!(file.class_check_bindings.get().is_none());
                }
            }
            let index = file.class_check_bindings.get().unwrap();
            assert_eq!(
                index.visited,
                crate::cops::testing::preorder_kinds(std::str::from_utf8(&file.bytes).unwrap())
                    .len()
            );
            assert!(index.visited < 22000);
            assert!(std::ptr::eq(
                index,
                file.class_check_bindings(&result.node())
            ));
        }
    }

    #[test]
    fn empty_call_binding_index_is_lazy_built_once_and_file_local() {
        use crate::cops::{
            run_cops_with_corrections, style::MethodCallWithoutArgsParentheses,
            variables::VISIBLE_LOCAL_BUILDS,
        };
        VISIBLE_LOCAL_BUILDS.with(|n| n.set(0));
        let source = format!(
            "foo0 = 1\n{}",
            (0..5000).map(|i| format!("foo{i}()\n")).collect::<String>()
        );
        let file = SourceFile::new("x.rb".into(), source.into_bytes());
        let result = ruby_prism::parse(&file.bytes);
        assert!(file.visible_locals.get().is_none());
        for corrections in [false, true, false] {
            let mut found = Vec::new();
            assert!(
                run_cops_with_corrections(
                    &[&MethodCallWithoutArgsParentheses],
                    &result,
                    &file,
                    &mut found,
                    corrections
                )
                .is_empty()
            );
            assert_eq!(found.len(), 4999);
            assert_eq!(
                found.iter().filter(|o| o.correction.is_some()).count(),
                if corrections { 4999 } else { 0 }
            );
            VISIBLE_LOCAL_BUILDS.with(|n| assert_eq!(n.get(), 1));
        }
        let other = SourceFile::new("x.rb".into(), b"foo0()\n".to_vec());
        let parsed = ruby_prism::parse(&other.bytes);
        let mut found = Vec::new();
        assert!(
            run_cops_with_corrections(
                &[&MethodCallWithoutArgsParentheses],
                &parsed,
                &other,
                &mut found,
                false
            )
            .is_empty()
        );
        assert_eq!(found.len(), 1);
        VISIBLE_LOCAL_BUILDS.with(|n| assert_eq!(n.get(), 2));
    }

    #[test]
    fn heredoc_local_ownership_is_lazy_shared_and_bounded_by_ancestor_depth() {
        use crate::cops::{
            run_cops_with_corrections,
            style::MethodCallWithoutArgsParentheses,
            variables::{VISIBLE_LOCAL_BUILDS, VISIBLE_LOCAL_EDGES},
        };
        VISIBLE_LOCAL_BUILDS.with(|n| n.set(0));
        let inert = SourceFile::new(
            "x.rb".into(),
            b"class C; X=<<~TXT\n#{obj.foo()}\nTXT\nend\n".to_vec(),
        );
        let parsed = ruby_prism::parse(&inert.bytes);
        let mut found = Vec::new();
        assert!(
            run_cops_with_corrections(
                &[&MethodCallWithoutArgsParentheses],
                &parsed,
                &inert,
                &mut found,
                true
            )
            .is_empty()
        );
        assert_eq!(found.len(), 1);
        assert!(
            inert.visible_locals.get().is_none(),
            "qualified calls need no local analysis"
        );

        let source = "class C; foo=1; X=<<~TXT\n#{foo()} #{safe()}\nTXT\nend\n".repeat(3000);
        let file = SourceFile::new("x.rb".into(), source.into_bytes());
        let parsed = ruby_prism::parse(&file.bytes);
        assert!(file.visible_locals.get().is_none());
        for corrections in [false, true, false] {
            VISIBLE_LOCAL_EDGES.with(|n| n.set(0));
            let mut found = Vec::new();
            assert!(
                run_cops_with_corrections(
                    &[&MethodCallWithoutArgsParentheses],
                    &parsed,
                    &file,
                    &mut found,
                    corrections
                )
                .is_empty()
            );
            assert_eq!(found.len(), 3000);
            assert_eq!(
                found.iter().filter(|o| o.correction.is_some()).count(),
                if corrections { 3000 } else { 0 }
            );
            VISIBLE_LOCAL_BUILDS.with(|n| assert_eq!(n.get(), 1));
            // Each of 6000 queries only walks its existing ancestor path;
            // no body rescan, ownership index, or per-candidate parse is added.
            VISIBLE_LOCAL_EDGES.with(|n| assert_eq!(n.get(), 6000 * 6));
        }
    }

    #[test]
    fn companion_presence_is_cached_linear_and_file_local() {
        use crate::cops::{run_cops, style::MissingRespondToMissing};
        let source = format!(
            "class C\n{}\ndef respond_to_missing?; end\nend\n{}",
            "def method_missing; end\n".repeat(3000),
            "module M; def method_missing; end; end\n".repeat(1000)
        );
        let file = SourceFile::new("x.rb".into(), source.into_bytes());
        let result = ruby_prism::parse(&file.bytes);
        assert!(file.respond_to_missing_index.get().is_none());
        for _ in 0..2 {
            let mut found = Vec::new();
            assert!(run_cops(&[&MissingRespondToMissing], &result, &file, &mut found).is_empty());
            assert_eq!(found.len(), 1000);
        }
        let index = file.respond_to_missing_index.get().unwrap();
        assert_eq!(
            index.visited,
            crate::cops::testing::preorder_kinds(std::str::from_utf8(&file.bytes).unwrap()).len()
        );
        assert!(index.visited < 16000);
        assert!(std::ptr::eq(
            index,
            file.respond_to_missing_index(&result.node())
        ));
        assert_eq!(
            crate::pipeline::lint_str("class C; def method_missing; end; end")
                .iter()
                .filter(|o| o.cop == "Style/MissingRespondToMissing")
                .count(),
            1
        );
    }

    #[test]
    fn initialize_comment_windows_are_lazy_cached_and_linear_in_many_siblings() {
        use crate::cops::{run_cops_with_corrections, style::RedundantInitialize};
        let source = "class C\ndef initialize;end # deliberate override\nend\n".repeat(2000);
        let f = file(source.as_bytes());
        let parsed = crate::parser::parse(&f.bytes);
        f.record_comments(&parsed);
        f.record_encoding(&parsed);
        f.record_data_start(&parsed);
        assert!(f.initialize_comments.get().is_none());
        let mut found = Vec::new();
        assert!(
            run_cops_with_corrections(&[&RedundantInitialize], &parsed, &f, &mut found, false)
                .is_empty()
        );
        assert!(found.is_empty());
        let facts = f.initialize_comments.get().unwrap();
        assert!(facts.visits < 20020);
        let pointer = std::ptr::from_ref(facts);
        assert!(
            run_cops_with_corrections(&[&RedundantInitialize], &parsed, &f, &mut found, true)
                .is_empty()
        );
        assert!(found.is_empty());
        assert_eq!(
            pointer,
            std::ptr::from_ref(f.initialize_comments.get().unwrap())
        );
        let inert = file(b"def other;end\n");
        let parsed = crate::parser::parse(&inert.bytes);
        assert!(
            run_cops_with_corrections(&[&RedundantInitialize], &parsed, &inert, &mut found, true)
                .is_empty()
        );
        assert!(inert.initialize_comments.get().is_none());
    }
    #[test]
    fn definition_spacing_reuses_bottom_up_extents_and_lazy_safety_caches() {
        use crate::cops::{layout::EmptyLineBetweenDefs, run_cops_with_corrections};
        // Nested definitions must not trigger repeated descendant walks. Each
        // of these 2,000 small scopes also has an endless heredoc boundary.
        let source = "p __LINE__\n".to_owned()
            + &"class C\ndef a = <<~H\n  hello\nH\ndef b\nend\nend\n".repeat(2000);
        let f = file(source.as_bytes());
        let result = ruby_prism::parse(&f.bytes);
        assert_eq!(result.errors().count(), 0);
        f.record_comments(&result);
        f.record_data_start(&result);
        f.record_encoding(&result);
        let mut found = Vec::new();
        assert!(
            run_cops_with_corrections(&[&EmptyLineBetweenDefs], &result, &f, &mut found, false)
                .is_empty()
        );
        assert_eq!(found.len(), 3999);
        assert!(f.literal_ranges.get().is_none());
        assert!(f.line_value_boundary.get().is_none());
        assert!(f.directive_comments.get().is_none());
        let cache = f.token_ranges.get().unwrap();
        assert_eq!(cache.definition_ends.len(), 6000);
        let pointer = cache.protected.as_ptr();
        found.clear();
        assert!(
            run_cops_with_corrections(&[&EmptyLineBetweenDefs], &result, &f, &mut found, true)
                .is_empty()
        );
        assert_eq!(found.len(), 3999);
        assert!(found.iter().all(|o| o.correction.is_some()));
        assert_eq!(f.token_ranges.get().unwrap().protected.as_ptr(), pointer);
        assert_eq!(f.line_value_boundary.get(), Some(&3));
    }

    #[test]
    fn body_padding_guards_are_lazy_and_shared_across_many_definitions() {
        use crate::cops::{
            layout::{
                EmptyLinesAroundClassBody, EmptyLinesAroundMethodBody, EmptyLinesAroundModuleBody,
            },
            run_cops_with_corrections,
        };
        let source = "p __LINE__\n".to_string() + &"module M\n\n  class C\n\n    def foo\n\n      'literal'\n\n    end\n\n  end\n\nend\n".repeat(2000);
        let f = file(source.as_bytes());
        let result = ruby_prism::parse(&f.bytes);
        assert_eq!(result.errors().count(), 0);
        f.record_comments(&result);
        f.record_data_start(&result);
        f.record_encoding(&result);
        let cops: &[&dyn crate::cops::Cop] = &[
            &EmptyLinesAroundClassBody,
            &EmptyLinesAroundModuleBody,
            &EmptyLinesAroundMethodBody,
        ];
        let mut found = Vec::new();
        assert!(run_cops_with_corrections(cops, &result, &f, &mut found, false).is_empty());
        assert_eq!(found.len(), 12000);
        assert!(found.iter().all(|o| o.correction.is_none()));
        assert!(f.line_value_boundary.get().is_none());
        assert!(f.literal_ranges.get().is_none());
        assert!(f.directive_comments.get().is_none());
        found.clear();
        assert!(run_cops_with_corrections(cops, &result, &f, &mut found, true).is_empty());
        assert_eq!(found.len(), 12000);
        assert!(found.iter().all(|o| o.correction.is_some()));
        assert_eq!(f.line_value_boundary.get(), Some(&3));
        let pointer = f.literal_ranges.get().unwrap().as_ptr();
        for _ in 0..2000 {
            assert_eq!(f.line_value_boundary(), 3);
            assert_eq!(f.literal_ranges.get().unwrap().as_ptr(), pointer);
        }
    }

    #[test]
    fn many_empty_lines_in_a_directive_block_comment_share_one_guard_scan() {
        use crate::cops::{layout::EmptyLines, run_cops_with_corrections};
        let source = "x\n=begin\n".to_string()
            + &"\n".repeat(2000)
            + "rubocop:enable Lint/Debugger\n=end\ny\n";
        let f = file(source.as_bytes());
        let result = ruby_prism::parse(&f.bytes);
        assert_eq!(result.errors().count(), 0);
        f.record_comments(&result);
        f.record_data_start(&result);
        f.record_encoding(&result);
        let mut found = Vec::new();
        assert!(
            run_cops_with_corrections(&[&EmptyLines], &result, &f, &mut found, false).is_empty()
        );
        assert_eq!(found.len(), 1999);
        assert!(f.directive_comments.get().is_none());
        found.clear();
        assert!(
            run_cops_with_corrections(&[&EmptyLines], &result, &f, &mut found, true).is_empty()
        );
        assert_eq!(found.len(), 1999);
        assert!(found.iter().all(|o| o.correction.is_none()));
        assert_eq!(f.directive_comments.get().unwrap().len(), 1);
        let pointer = f.directive_comments().as_ptr();
        for _ in 0..2000 {
            assert_eq!(f.directive_comments().as_ptr(), pointer);
        }
    }

    #[test]
    fn naming_and_style_forwarding_share_lazy_bounded_correction_safety() {
        use crate::cops::{
            naming::BlockForwarding,
            run_cops_with_corrections,
            style::ArgumentsForwarding,
            variables::ScopeAnalysis,
            walk::{NodeVisitor, walk},
        };
        struct Count(usize);
        impl<'pr> NodeVisitor<'pr> for Count {
            fn enter(&mut self, _: ruby_prism::Node<'pr>) {
                self.0 += 1;
            }
        }
        let source = "def foo(&callback); items.each { bar(&callback) }; end\ndef bar(*args, **kwargs, &block); baz(*args, **kwargs, &block); end\n".repeat(1000);
        let f = file(source.as_bytes());
        let parsed = crate::parser::parse(&f.bytes);
        f.record_comments(&parsed);
        let cops: &[&dyn crate::cops::Cop] = &[&BlockForwarding, &ArgumentsForwarding];
        let mut found = Vec::new();
        assert!(run_cops_with_corrections(cops, &parsed, &f, &mut found, false).is_empty());
        assert!(found.len() >= 6000);
        assert!(f.forwarding_safety.get().is_none());
        found.clear();
        assert!(run_cops_with_corrections(cops, &parsed, &f, &mut found, true).is_empty());
        let proof = f.forwarding_safety.get().unwrap();
        let mut count = Count(0);
        walk(&mut count, &parsed.node());
        assert_eq!(proof.visits, count.0);
        assert!(count.0 < 1000 * 50);
        let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
        for _ in 0..1000 {
            assert!(std::ptr::eq(
                proof,
                f.forwarding_safety(&parsed.node(), &analysis)
            ));
        }
    }

    #[test]
    fn pending_sibling_heredoc_queries_reuse_the_lazy_sorted_token_index() {
        use crate::cops::{run_cops_with_corrections, style::TrailingBodyOnMethodDefinition};
        let source = "x = <<TXT;def f; 1\npayload\nTXT\nend\n".repeat(2000);
        let f = file(source.as_bytes());
        let result = ruby_prism::parse(&f.bytes);
        assert_eq!(result.errors().count(), 0);
        f.record_comments(&result);
        let mut found = Vec::new();
        let cops: &[&dyn crate::cops::Cop] = &[&TrailingBodyOnMethodDefinition];
        assert!(run_cops_with_corrections(cops, &result, &f, &mut found, false).is_empty());
        assert_eq!(found.len(), 2000);
        assert!(f.token_ranges.get().is_none());
        found.clear();
        assert!(run_cops_with_corrections(cops, &result, &f, &mut found, true).is_empty());
        assert_eq!(found.len(), 2000);
        assert!(found.iter().all(|o| o.correction.is_none()));
        let cache = f.token_ranges.get().unwrap();
        assert_eq!(cache.heredoc_openings.len(), 2000);
        assert!(cache.heredoc_openings.windows(2).all(|w| w[0] < w[1]));
        for _ in 0..5 {
            for &opening in &cache.heredoc_openings {
                assert!(!f.heredoc_pending_at(&result.node(), opening));
                assert!(f.heredoc_pending_at(&result.node(), opening + 6));
                assert!(std::ptr::eq(cache, f.token_ranges(&result.node())));
            }
        }
    }

    #[test]
    fn option_hash_super_index_is_lazy_linear_and_uses_only_the_main_parse() {
        use crate::cops::{
            run_cops_with_corrections,
            style::OptionHash,
            walk::{NodeVisitor, walk},
        };
        struct Count(usize);
        impl<'pr> NodeVisitor<'pr> for Count {
            fn enter(&mut self, _: Node<'pr>) {
                self.0 += 1;
            }
        }
        for (source, expected, builds_index) in [
            ("def f(config = {}); super; end\n".repeat(1000), 0, false),
            (
                format!(
                    "{}{}{}",
                    "def f(options = {}); end\n".repeat(1000),
                    "def f(options = {}); proc { super }; end\n".repeat(1000),
                    "foo(super) { |options = {}| }\n".repeat(1000)
                ),
                1000,
                true,
            ),
        ] {
            let f = file(source.as_bytes());
            let result = ruby_prism::parse(&f.bytes);
            assert_eq!(result.errors().count(), 0);
            f.record_comments(&result);
            f.record_data_start(&result);
            let mut found = Vec::new();
            assert!(
                run_cops_with_corrections(&[&OptionHash], &result, &f, &mut found, false)
                    .is_empty()
            );
            assert_eq!(found.len(), expected);
            assert_eq!(f.option_hash_supers.get().is_some(), builds_index);
            if builds_index {
                let mut count = Count(0);
                walk(&mut count, &result.node());
                let index = f.option_hash_supers.get().unwrap();
                assert_eq!(index.visits, count.0);
                assert!(count.0 < 3000 * 20);
                for _ in 0..3000 {
                    assert!(std::ptr::eq(index, f.option_hash_supers(&result.node())));
                }
            }
            assert!(f.super_facts.get().is_none());
            assert!(f.token_ranges.get().is_none());
            assert!(f.literal_ranges.get().is_none());
        }
    }

    #[test]
    fn command_backtick_index_is_lazy_encoding_aware_and_queries_do_not_rescan() {
        for n in [100, 1000] {
            let f = file(
                format!("# encoding: Windows-31J\n{}", "p %x(no); `ok`\n".repeat(n)).as_bytes(),
            );
            let parsed = ruby_prism::parse(&f.bytes);
            f.record_encoding(&parsed);
            assert!(f.command_backticks.get().is_none());
            encoding::WIDTH_CALLS.set(0);
            assert!(f.command_body_has_backtick(0..f.bytes.len()));
            assert!(encoding::WIDTH_CALLS.get() <= f.bytes.len());
            assert_eq!(f.command_backticks.get().unwrap().len(), n * 2);
            encoding::WIDTH_CALLS.set(0);
            for offset in 0..f.bytes.len() {
                let _ = f.command_body_has_backtick(offset..f.bytes.len());
            }
            assert_eq!(encoding::WIDTH_CALLS.get(), 0);
        }
        let f = file(b"# encoding: Windows-31J\n%x(\x83\x60)\n");
        assert!(!f.command_body_has_backtick(0..f.bytes.len()));
    }

    #[test]
    fn block_chain_index_is_one_main_ast_pass_even_with_large_argument_lists() {
        use crate::cops::{
            run_cops_with_corrections,
            style::MultilineBlockChain,
            walk::{NodeVisitor, walk},
        };
        struct Count(usize);
        impl<'pr> NodeVisitor<'pr> for Count {
            fn enter(&mut self, _: Node<'pr>) {
                self.0 += 1;
            }
        }
        for n in [100, 1000] {
            let args = std::iter::repeat_n("a do\n b\nend.c { d }", n)
                .collect::<Vec<_>>()
                .join(",\n");
            let source = format!("outer({args}) do z end");
            let f = file(source.as_bytes());
            let parsed = ruby_prism::parse(&f.bytes);
            assert_eq!(parsed.errors().count(), 0);
            assert!(f.block_chain_facts.get().is_none());
            let mut out = Vec::new();
            assert!(
                run_cops_with_corrections(&[&MultilineBlockChain], &parsed, &f, &mut out, false)
                    .is_empty()
            );
            assert_eq!(out.len(), n);
            assert!(out.iter().all(|o| o.correction.is_none()));
            let mut count = Count(0);
            walk(&mut count, &parsed.node());
            assert_eq!(f.block_chain_facts.get().unwrap().visits, count.0);
            assert!(count.0 < n * 20);
            assert!(std::ptr::eq(
                f.block_chain_facts.get().unwrap(),
                f.block_chain_facts(&parsed.node())
            ));
        }
    }

    #[test]
    fn deferred_header_parameter_index_visits_every_main_ast_node_once() {
        use crate::cops::{
            run_cops_with_corrections,
            style::SingleLineBlockParams,
            walk::{NodeVisitor, walk},
        };
        struct Count(usize);
        impl<'pr> NodeVisitor<'pr> for Count {
            fn enter(&mut self, _: Node<'pr>) {
                self.0 += 1;
            }
        }
        let fragment = "xs.reduce { |memo,item| def (build(<<~HEADER)).tag; end; memo+item }\n#{[0].map { acc }.first}\nHEADER\nxs.inject { |total,value| <<~SAFE }\n#{-> { total+value }.call}\nSAFE\n";
        let mut counts = Vec::new();
        for n in [100, 1000] {
            let f = file(fragment.repeat(n).as_bytes());
            let parsed = ruby_prism::parse(&f.bytes);
            assert_eq!(parsed.errors().count(), 0);
            for collect in [false, true] {
                let mut out = Vec::new();
                assert!(
                    run_cops_with_corrections(
                        &[&SingleLineBlockParams],
                        &parsed,
                        &f,
                        &mut out,
                        collect
                    )
                    .is_empty()
                );
                assert_eq!(out.len(), 2 * n);
                for pair in out.as_chunks::<2>().0 {
                    assert!(pair[0].correction.is_none());
                    assert_eq!(pair[1].correction.is_some(), collect);
                }
                if !collect {
                    assert!(f.block_param_facts.get().is_none());
                }
            }
            let mut count = Count(0);
            walk(&mut count, &parsed.node());
            let facts = f.block_param_facts.get().unwrap();
            assert_eq!(facts.visits, count.0);
            assert!(std::ptr::eq(facts, f.block_param_facts(&parsed.node())));
            counts.push(count.0);
        }
        assert_eq!(counts[1] - 2, 10 * (counts[0] - 2));
    }

    #[test]
    fn nested_sugar_ownership_visits_each_main_node_once() {
        use crate::cops::{run_cops_with_corrections, style::LambdaCall};
        for n in [10, 50] {
            let source = format!("x{}", ".()".repeat(n));
            let f = file(source.as_bytes());
            let parsed = ruby_prism::parse(&f.bytes);
            assert_eq!(parsed.errors().count(), 0);
            let mut out = Vec::new();
            assert!(
                run_cops_with_corrections(&[&LambdaCall], &parsed, &f, &mut out, true).is_empty()
            );
            assert_eq!(out.len(), n);
            assert_eq!(out.iter().filter(|o| o.correction.is_some()).count(), 1);
            assert!(f.lambda_call_facts.get().unwrap().visits <= n * 2 + 3);
            let applied = crate::corrections::apply(&f.bytes, &out).unwrap();
            assert_eq!(
                applied.bytes,
                format!("x{}.call", ".()".repeat(n - 1)).as_bytes()
            );
        }
    }

    #[test]
    fn predicate_return_facts_are_lazy_source_local_complete_and_shared() {
        use crate::cops::{naming::PredicateMethod, run_cops_with_corrections};
        for (source, expected, indexed) in [
            ("def initialize; return true; end\ndef call; true; end\ndef ==(other); true; end\ndef empty?; end\n".to_owned(), 0, false),
            ("def foo; log; return true; end\n".repeat(2000), 2000, true),
            ("def foo?; def nested(x = -> { return true }); 5; end; 5; end\n".repeat(2000), 0, true),
        ] {
            let f = file(source.as_bytes());
            let parsed = crate::parser::parse(&f.bytes);
            assert_eq!(parsed.errors().count(), 0);
            for fixes in [false, true] {
                let mut found = Vec::new();
                assert!(run_cops_with_corrections(&[&PredicateMethod], &parsed, &f, &mut found, fixes).is_empty());
                assert_eq!(found.len(), expected);
                assert!(found.iter().all(|o| o.correction.is_none()));
                assert_eq!(f.predicate_method_returns.get().is_some(), indexed);
                if indexed {
                    let facts = f.predicate_method_returns.get().unwrap();
                    assert_eq!(facts.visited, crate::cops::testing::preorder_kinds(&source).len());
                    assert!(facts.visited < 2000 * 25);
                    for _ in 0..2000 {
                        assert!(std::ptr::eq(facts, f.predicate_method_returns(&parsed.node())));
                    }
                }
                assert!(f.token_ranges.get().is_none());
            }
        }
    }

    // Independent pre-index implementation, not another cache-based path.
    fn prefix_position(f: &SourceFile, offset: usize) -> (u32, u32) {
        let offset = offset.min(f.bytes.len());
        let line = f.line_of(offset);
        let start = if line == 1 && f.bytes.starts_with(b"\xef\xbb\xbf") {
            3.min(offset)
        } else {
            f.line_start(line)
        };
        (
            line,
            f.parsed_encoding().count_chars(&f.bytes[start..offset]) as u32 + 1,
        )
    }

    #[test]
    fn dense_valid_scalar_cop_positions_have_bounded_shared_width_work() {
        use crate::cops::{
            run_cops_with_corrections,
            style::{AmbiguousEndlessMethodDefinition, MethodDefParentheses},
        };
        for fragment in [
            "def f arg; arg; end; def g = true if condition; ",
            "\"é😀\"; def f arg; arg; end; def g = true if condition; ",
        ] {
            let f = file(fragment.repeat(1000).as_bytes());
            let parsed = crate::parser::parse(&f.bytes);
            assert_eq!(parsed.errors().count(), 0);
            f.record_encoding(&parsed);
            encoding::WIDTH_CALLS.set(0);
            let mut found = Vec::new();
            assert!(
                run_cops_with_corrections(
                    &[&MethodDefParentheses, &AmbiguousEndlessMethodDefinition],
                    &parsed,
                    &f,
                    &mut found,
                    false,
                )
                .is_empty()
            );
            assert_eq!(found.len(), 2000);
            let work = encoding::WIDTH_CALLS.get();
            assert!(
                work <= f.bytes.len() + 256 * found.len(),
                "{work} width calls for {} bytes",
                f.bytes.len()
            );
            // Reverse, duplicate and EOF queries cannot restart a line scan.
            encoding::WIDTH_CALLS.set(0);
            for offset in (0..=f.bytes.len())
                .rev()
                .step_by(17)
                .chain([usize::MAX, 0, 0])
            {
                let actual = f.line_col(offset);
                let before_reference = encoding::WIDTH_CALLS.get();
                assert_eq!(actual, prefix_position(&f, offset));
                encoding::WIDTH_CALLS.set(before_reference);
            }
            assert!(encoding::WIDTH_CALLS.get() <= 256 * (f.bytes.len() / 17 + 5));
        }
    }

    #[test]
    fn scalar_column_index_is_lazy_sparse_linear_and_reused() {
        for (pragma, fragment, has_blocks) in [
            ("", "binding.pry; ", false),
            ("# coding: ASCII-8BIT\n", "\"é\"; binding.pry; ", false),
            ("", "\"é😀\"; binding.pry; ", true),
        ] {
            for repeats in [100, 1000, 10000] {
                let f = file(format!("{pragma}{}", fragment.repeat(repeats)).as_bytes());
                let parsed = crate::parser::parse(&f.bytes);
                assert_eq!(parsed.errors().count(), 0);
                f.record_encoding(&parsed);
                assert!(f.columns.get().is_none());
                encoding::WIDTH_CALLS.set(0);
                assert_eq!(f.line_col(0), (1, 1));
                let construction = encoding::WIDTH_CALLS.get();
                assert!(construction <= f.bytes.len() + f.bytes.len() / 250 + 4);
                let index = f.columns.get().unwrap();
                let (blocks, capacity) = index.allocated_blocks();
                if has_blocks {
                    assert!(blocks > 0);
                    assert!(blocks <= f.bytes.len() / 250 + 1);
                    assert!(capacity <= 2 * blocks.max(2));
                } else {
                    assert_eq!((blocks, capacity), (0, 0));
                }
                for offset in [f.bytes.len(), f.bytes.len() / 2, 0, 999, 999] {
                    encoding::WIDTH_CALLS.set(0);
                    let position = f.line_col(offset);
                    assert!(encoding::WIDTH_CALLS.get() <= 256);
                    assert_eq!(position, prefix_position(&f, offset));
                    assert!(std::ptr::eq(index, f.columns.get().unwrap()));
                    assert_eq!(index.allocated_blocks(), (blocks, capacity));
                }
            }
        }
    }

    #[test]
    fn sparse_block_boundaries_preserve_all_prefix_positions() {
        for (name, text) in [
            ("UTF-8", b"\xc3\xa9\xf0\x9f\x98\x80\xff\xc3".as_slice()),
            ("Shift_JIS", b"\x82\xa0\xa6\xff\x82".as_slice()),
            ("EUC-JP", b"\xa4\xa2\x8f\xa2\xaf\xff\x8f\xa2".as_slice()),
            ("Big5", b"\xa4\x40\xff\xa4".as_slice()),
            ("GB18030", b"\x81\x30\x81\x30\xff\x81\x30\x81".as_slice()),
            ("CESU-8", b"\xed\xa0\x80\xed\xb0\x80\xff".as_slice()),
            ("Windows-1252", b"\xc3\xa9\xff".as_slice()),
            ("not-an-encoding", b"\xc3\xa9\xff".as_slice()),
        ] {
            let mut source = format!("# coding: {name}\r\n").into_bytes();
            // Straddle sparse blocks and physical lines at varied boundaries.
            for padding in [0, 1, 249, 254, 255, 256, 257] {
                source.extend(std::iter::repeat_n(b'x', padding));
                source.extend(text.repeat(80));
                source.extend_from_slice(b"\r\n");
            }
            source.extend_from_slice(text); // incomplete prefix at EOF too
            let f = file(&source);
            let parsed = crate::parser::parse(&f.bytes);
            f.record_encoding(&parsed);
            for offset in (0..=source.len()).rev().chain([usize::MAX, 0, 0]) {
                assert_eq!(
                    f.line_col(offset),
                    prefix_position(&f, offset),
                    "{name}: {offset}"
                );
            }
        }
    }

    #[test]
    fn maps_offsets_to_line_and_col() {
        let f = file(b"abc\ndef\n");
        assert_eq!(f.line_col(0), (1, 1));
        assert_eq!(f.line_col(2), (1, 3));
        assert_eq!(f.line_col(4), (2, 1));
        assert_eq!(f.line_col(5), (2, 2));
    }

    #[test]
    fn batched_positions_match_scalar_mapping_for_every_byte_and_query_order() {
        for (name, text) in [
            ("UTF-8", b"\xc3\xa9\xe3\x80\x80\xff\xc3".as_slice()),
            ("ASCII-8BIT", b"\xc3\xa9\xff".as_slice()),
            ("Windows-1252", b"\xc3\xa9\xff".as_slice()),
            ("Shift_JIS", b"\x82\xa0\xa6\xff\x82".as_slice()),
            ("EUC-JP", b"\xa4\xa2\x8f\xa2\xaf\xff\x8f\xa2".as_slice()),
            ("Big5", b"\xa4\x40\xff\xa4".as_slice()),
            ("GB18030", b"\x81\x30\x81\x30\xff\x81\x30\x81".as_slice()),
            ("CESU-8", b"\xed\xa0\x80\xed\xb0\x80\xff".as_slice()),
            ("not-an-encoding", b"\xc3\xa9\xff".as_slice()),
        ] {
            let mut source = format!("# coding: {name}\n").into_bytes();
            source.extend_from_slice(text);
            source.extend_from_slice(b"\r\n\n");
            source.extend_from_slice(text);
            let f = file(&source);
            let parsed = crate::parser::parse(&f.bytes);
            f.record_encoding(&parsed);
            drop(parsed);
            let offsets: Vec<_> = (0..=source.len())
                .rev()
                .chain([usize::MAX, 0, 1, 1])
                .collect();
            let expected: Vec<_> = offsets
                .iter()
                .map(|&offset| prefix_position(&f, offset))
                .collect();
            for &offset in &offsets {
                assert_eq!(
                    f.line_col(offset),
                    prefix_position(&f, offset),
                    "{name}: {offset}"
                );
            }
            assert_eq!(f.line_cols(offsets), expected, "{name}");
            assert!(f.line_cols([]).is_empty());
        }
    }

    #[test]
    fn batched_positions_preserve_bom_empty_and_eof_semantics() {
        for source in [
            b"".as_slice(),
            b"\n",
            b"\xef\xbb",
            b"\xef\xbb\xbf",
            "\u{feff}éx\n\u{feff}z\n".as_bytes(),
            b"\xef\xbb\xbf# coding: ascii-8bit\n\xc3\xa9\xff",
        ] {
            let f = file(source);
            let offsets: Vec<_> = (0..=source.len()).rev().chain([usize::MAX, 0]).collect();
            let expected: Vec<_> = offsets
                .iter()
                .map(|&offset| prefix_position(&f, offset))
                .collect();
            for &offset in &offsets {
                assert_eq!(
                    f.line_col(offset),
                    prefix_position(&f, offset),
                    "{source:?}: {offset}"
                );
            }
            assert_eq!(f.line_cols(offsets), expected, "{source:?}");
        }
    }

    #[test]
    fn counts_columns_in_characters_not_bytes() {
        // "é" is two bytes; one character.
        let f = file("\u{e9}x\n".as_bytes());
        assert_eq!(f.line_col(3), (1, 3));
    }

    #[test]
    fn declared_single_byte_encodings_count_bytes_even_when_valid_utf8() {
        for name in ["ASCII-8BIT", "binary", "ISO-8859-1", "Windows-1252"] {
            let mut source = format!("# coding: {name}\n").into_bytes();
            source.extend_from_slice(b"\"\xe3\x80\x80\"; binding.pry\n");
            let f = file(&source);
            let offset = f.line_start(2) + 7;
            assert_eq!(f.line_col(offset), (2, 8), "{name}");
            assert_eq!(f.bytes, source);
            assert_eq!(f.line_text(2), b"\"\xe3\x80\x80\"; binding.pry");
        }
    }

    #[test]
    fn declared_multibyte_encodings_count_ruby_characters() {
        for (name, text, characters) in [
            ("Shift_JIS", b"\x82\xa0\xa6".as_slice(), 2),
            ("CP932", b"\x82\xa0\xa6".as_slice(), 2),
            ("Shift_JIS", b"\xc3\xa9".as_slice(), 2),
            ("EUC-JP", b"\xa4\xa2\x8f\xa2\xaf".as_slice(), 2),
            ("Big5", b"\xa4\x40".as_slice(), 1),
            ("GB18030", b"\x81\x30\x81\x30".as_slice(), 1),
        ] {
            let mut source = format!("# coding: {name}\n\"").into_bytes();
            source.extend_from_slice(text);
            source.extend_from_slice(b"\"; binding.pry\n");
            let f = file(&source);
            let parsed = ruby_prism::parse(&f.bytes);
            assert_eq!(parsed.errors().count(), 0, "{name}");
            f.record_encoding(&parsed);
            let offset = f.line_start(2) + text.len() + 4;
            assert_eq!(f.line_col(offset), (2, characters + 5), "{name}");
            assert_eq!(f.bytes, source);
        }
    }

    #[test]
    fn column_encoding_obeys_magic_comment_placement_and_aliases() {
        for (prefix, col) in [
            ("# coding = BINARY\n", 8),
            ("# free-form coding: ISO8859-1\n", 8),
            ("# -*- coding: Windows-1252 -*-\n", 8),
            ("#!/usr/bin/ruby\n  # encoding: ascii-8bit\n", 8),
            ("# comment\n# coding: ascii-8bit\n", 6),
            ("x = 0 # coding: ascii-8bit\n", 6),
            ("#!/bin/bash\n# coding: ascii-8bit\n", 6),
            ("# foo: coding:ascii-8bit\n", 6),
            ("# coding: not-an-encoding\n", 6),
        ] {
            let mut source = prefix.as_bytes().to_vec();
            source.extend_from_slice(b"\"\xe3\x80\x80\"; binding.pry\n");
            let f = file(&source);
            let parsed = ruby_prism::parse(&f.bytes);
            f.record_encoding(&parsed);
            // Cached metadata has no dependency on the parse's lifetime.
            drop(parsed);
            assert!(f.encoding.get().is_some());
            let line = prefix.bytes().filter(|&b| b == b'\n').count() as u32 + 1;
            for _ in 0..3 {
                assert_eq!(f.line_col(prefix.len() + 7), (line, col), "{prefix:?}");
            }
        }
    }

    #[test]
    fn invalid_or_incomplete_characters_do_not_erase_prior_character_boundaries() {
        for (name, text, col) in [
            ("UTF-8", b"\xc3\xa9\xffx".as_slice(), 4),
            ("UTF-8", b"\xc3\xa9\xc3".as_slice(), 3),
            ("Shift_JIS", b"\x82\xa0\xffx".as_slice(), 4),
            ("Shift_JIS", b"\x82\xa0\x82".as_slice(), 3),
            ("EUC-JP", b"\xa4\xa2\xffx".as_slice(), 4),
            ("EUC-JP", b"\xa4\xa2\x8f\xa2".as_slice(), 4),
        ] {
            let mut source = format!("# coding: {name}\n").into_bytes();
            source.extend_from_slice(text);
            let f = file(&source);
            assert!(f.encoding.get().is_none());
            assert_eq!(f.line_col(usize::MAX), (2, col), "{name}: {text:?}");
            assert!(f.encoding.get().is_some(), "lazy mapping caches metadata");
            assert_eq!(f.bytes, source);
        }
    }

    #[test]
    fn excludes_leading_utf8_bom_from_columns() {
        let f = file("\u{feff}éx\n  y\n".as_bytes());
        for offset in 0..=3 {
            assert_eq!(f.line_col(offset), (1, 1));
        }
        assert_eq!(f.line_col(5), (1, 2));
        assert_eq!(f.line_col(6), (1, 3));
        assert_eq!(f.line_col(7), (2, 1));
        assert_eq!(f.line_col(9), (2, 3));
        assert_eq!(f.line_col(999), (3, 1));
        // Raw byte offsets and line contents must remain unchanged.
        assert_eq!(f.line_start(1), 0);
        assert_eq!(f.line_start(2), 7);
        assert_eq!(f.line_text(1), "\u{feff}éx".as_bytes());
        assert_eq!(file("\u{feff}".as_bytes()).line_col(999), (1, 1));
    }

    #[test]
    fn counts_nonleading_bom_as_a_character() {
        let f = file("x\u{feff}y\n\u{feff}z".as_bytes());
        assert_eq!(f.line_col(4), (1, 3));
        assert_eq!(f.line_col(9), (2, 2));
    }

    #[test]
    fn excludes_bom_from_invalid_utf8_column_fallback() {
        let f = file(b"\xef\xbb\xbf\xffx");
        assert_eq!(f.line_col(4), (1, 2));
        assert_eq!(f.line_col(5), (1, 3));
        // An incomplete BOM is ordinary invalid UTF-8, not a signature.
        assert_eq!(file(b"\xef\xbbx").line_col(3), (1, 4));
    }

    #[test]
    fn maps_offsets_to_lines() {
        let f = file("\u{e9}b\ncd\n".as_bytes());
        assert_eq!(f.line_of(0), 1);
        assert_eq!(f.line_of(3), 1);
        assert_eq!(f.line_of(4), 2);
        assert_eq!(f.line_of(999), 3);
        assert_eq!(f.line_start(2), 4);
        assert_eq!(f.line_start(9), 7);
    }

    #[test]
    fn clamps_offsets_past_eof() {
        let f = file(b"a\n");
        assert_eq!(f.line_col(999), (2, 1));
    }

    #[test]
    fn indexes_comments_lazily() {
        let f = file(b"a # one\n\"# not a comment\"\n=begin\ndoc\n=end\nb\n");
        assert_eq!(f.comments().len(), 2);
        assert!(f.has_comment_in_lines(1..2));
        assert!(!f.has_comment_in_lines(2..3));
        // Block comments count on the line they start.
        assert!(f.has_comment_in_lines(3..4));
        assert!(!f.has_comment_in_lines(4..7));
        assert!(!f.has_comment_in_lines(1..1));
    }

    #[test]
    fn records_comments_from_an_existing_parse() {
        let f = file(b"x = 1\n# two\n");
        let result = ruby_prism::parse(&f.bytes);
        f.record_comments(&result);
        assert_eq!(f.comments().len(), 1);
        assert_eq!(f.comments()[0], 6..11);
        assert!(f.has_comment_in_lines(2..3));
        assert!(!f.has_comment_in_lines(1..2));
    }

    #[test]
    fn indexes_warnings_lazily() {
        let f = file(b"p 1\np /a/\n");
        assert_eq!(f.warnings().len(), 1);
        assert_eq!(f.warnings()[0].range, 6..7);
        assert!(f.warnings()[0].message.starts_with("ambiguous `/`"));
    }

    #[test]
    fn records_warnings_from_an_existing_parse() {
        let f = file(b"puts *a\nputs(a * b)\n");
        let result = ruby_prism::parse(&f.bytes);
        f.record_warnings(&result);
        assert_eq!(f.warnings().len(), 1);
        assert_eq!(f.warnings()[0].range, 5..6);
        assert!(file(b"puts(*a)\n").warnings().is_empty());
    }

    #[test]
    fn finds_the_data_section_lazily() {
        let f = file(b"x = 1\n__END__\ndata\n");
        assert_eq!(f.data_start(), Some(6));
        assert_eq!(f.code_line_count(), 1);
        // `__END__` inside a heredoc body is text, not the marker.
        let f = file(b"x = <<~A\n__END__\nA\ny\n");
        assert_eq!(f.data_start(), None);
        assert_eq!(f.code_line_count(), 4);
        assert_eq!(file(b"").code_line_count(), 0);
    }

    #[test]
    fn records_the_data_section_from_an_existing_parse() {
        let f = file(b"# c\n__END__\n");
        let result = ruby_prism::parse(&f.bytes);
        f.record_data_start(&result);
        assert_eq!(f.data_start(), Some(4));
        assert_eq!(f.code_line_count(), 1);
        let f = file(b"__END__");
        f.record_data_start(&ruby_prism::parse(&f.bytes));
        assert_eq!(f.data_start(), Some(0));
        assert_eq!(f.code_line_count(), 0);
    }

    #[test]
    fn comment_line_ranges_clamp_past_eof() {
        let f = file(b"# a");
        assert!(f.has_comment_in_lines(1..99));
        assert!(!f.has_comment_in_lines(2..99));
    }

    #[test]
    fn empty_hash_command_arguments_do_not_rescan_the_full_sibling_list_per_literal() {
        use crate::cops::{
            ast::CALL_ARGUMENT_VISITS, run_cops_with_corrections, style::EmptyLiteral,
        };
        for count in [1, 100, 1000] {
            let source = format!("consume {}\n", vec!["Hash.new"; count].join(", "));
            let f = file(source.as_bytes());
            let parsed = crate::parser::parse(&f.bytes);
            let mut found = Vec::new();
            CALL_ARGUMENT_VISITS.set(0);
            run_cops_with_corrections(&[&EmptyLiteral::default()], &parsed, &f, &mut found, true);
            assert_eq!(found.len(), count);
            assert!(
                CALL_ARGUMENT_VISITS.get() <= count * 64,
                "{} visits for {count}",
                CALL_ARGUMENT_VISITS.get()
            );
        }
    }

    #[test]
    fn numbered_class_emitter_siblings_are_lazy_and_cached_across_many_definitions() {
        use crate::cops::{naming::VariableNumber, run_cops_with_corrections};
        let ordinary = SourceFile::new("x.rb".into(), b"def self.foo1; end\n".to_vec());
        let parsed = crate::parser::parse(&ordinary.bytes);
        let mut found = Vec::new();
        run_cops_with_corrections(&[&VariableNumber], &parsed, &ordinary, &mut found, false);
        assert!(ordinary.class_emitter_siblings.get().is_none());
        let mut source = String::from("class A\n");
        for i in 0..2000 {
            source.push_str(&format!("class Foo_{i}; end; def self.Foo_{i}; end\n"));
        }
        source.push_str("end\n");
        let file = SourceFile::new("x.rb".into(), source.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        assert_eq!(parsed.errors().count(), 0);
        assert!(file.class_emitter_siblings.get().is_none());
        run_cops_with_corrections(&[&VariableNumber], &parsed, &file, &mut found, false);
        assert!(found.is_empty());
        let pointer = file.class_emitter_siblings.get().unwrap() as *const _;
        run_cops_with_corrections(&[&VariableNumber], &parsed, &file, &mut found, true);
        assert!(found.is_empty());
        assert_eq!(
            file.class_emitter_siblings.get().unwrap() as *const _,
            pointer
        );
    }

    #[test]
    fn finds_substrings() {
        let f = file(b"raise Exception\n");
        assert!(f.contains(b"Exception"));
        assert!(f.contains(b"raise"));
        assert!(!f.contains(b"StandardError"));
        assert!(!file(b"").contains(b"x"));
    }

    #[test]
    fn returns_line_text_without_terminator() {
        let f = file(b"ab\r\n  # c\nlast");
        assert_eq!(f.line_text(1), b"ab");
        assert_eq!(f.line_text(2), b"  # c");
        assert_eq!(f.line_text(3), b"last");
        assert_eq!(f.line_text(4), b"");
        assert_eq!(f.line_text(0), b"");
        assert_eq!(file(b"a\n").line_text(2), b"");
    }

    #[test]
    fn counts_lines() {
        assert_eq!(file(b"").line_count(), 0);
        assert_eq!(file(b"a").line_count(), 1);
        assert_eq!(file(b"a\n").line_count(), 1);
        assert_eq!(file(b"a\nb\n").line_count(), 2);
        assert_eq!(file(b"a\nb").line_count(), 2);
    }
}
