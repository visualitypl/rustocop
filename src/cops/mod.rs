//! The cop registry, the `Cop` contract, and the AST walker that dispatches
//! nodes to cops.

use std::{
    cell::RefCell,
    collections::HashSet,
    mem::{Discriminant, discriminant},
};

use ruby_prism::Node;

use crate::{
    config::ResolvedConfig,
    isolation::{CaughtPanic, catch},
    offense::{Department, GlobalOffense, Offense, Severity},
    source::SourceFile,
};
use settings::{Setting, Values};

pub mod ast;
pub mod bundler;
pub mod depth;
pub mod gemspec;
pub mod layout;
pub mod lint;
pub mod metrics;
pub mod migration;
pub mod name_similarity;
pub mod naming;
pub mod policy;
pub(crate) mod reparse;
pub mod security;
pub mod settings;
pub mod style;
#[cfg(test)]
pub(crate) mod testing;
pub mod variables;
pub mod walk;

/// The contract every cop implements.
///
/// Cops are stateless and shared across worker threads: configuration is
/// resolved by the pipeline (which decides whether a cop runs, and filters
/// its offenses), never read ad hoc inside the cop. The settings a cop
/// declares and the configuration of the cops it follows are resolved into
/// configured copies ([`Cop::configure`], [`Cop::with_companions`]) before
/// selection; those copies run instead.
pub trait Cop: Send + Sync {
    /// The full cop name, e.g. `Lint/Debugger`. A department prefix is required.
    fn name(&self) -> &'static str;

    /// Whether every correction this cop emits is safe to apply with `--fix`.
    /// Detection-only and unsafe cops leave this false.
    fn safe_fixes(&self) -> bool {
        false
    }

    /// Whether this cop's corrections are ones RuboCop marks unsafe (`Safe`
    /// or `SafeAutoCorrect` false): only `-A` applies them.
    fn unsafe_fixes(&self) -> bool {
        false
    }

    /// Selected companions that must not correct in the same pass as this cop.
    /// Keep detection and capability metadata intact, but withhold this cop's
    /// edits whenever a named companion is active (independent of input order).
    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &[]
    }

    /// Companions whose edits take precedence over this cop's within one
    /// pass, as upstream's Team skips a cop's corrector once an
    /// autocorrect-incompatible cop's corrections were applied. This cop's
    /// edits in a file are withheld for the pass only when a named companion
    /// still has an edit there after every safety and directive filter; a
    /// merely active companion withholds nothing (unlike
    /// [`Cop::autocorrect_incompatible_with`]), so runs with default-on
    /// companions converge over explicit passes.
    fn autocorrect_deferred_to(&self) -> &'static [&'static str] {
        &[]
    }

    /// Fixed enable policy. Explicit upstream-disabled Style alternatives are
    /// opt-in; pending cops are enabled. This never registers a planned cop.
    fn default_enabled(&self) -> bool {
        policy::default_enabled(self.name())
    }

    /// Whether selecting this cop's department (`--only Layout`, or
    /// `[cops.Layout] enabled = true`) enables it when the cop itself is not
    /// enabled. Accepted cops keep rustocop's rule (a department selection
    /// enables its opt-in cops); a cop answering false keeps upstream's rule
    /// instead (RuboCop's `enabled_cop_name?` and `enable_cop?`): only its
    /// exact name in `--only` or its own `enabled = true` enables it, while a
    /// disabled department still disables it.
    fn department_selection_enables(&self) -> bool {
        true
    }

    /// Fixed Ruby applicability. Selection never bypasses these bounds.
    fn ruby_versions(&self) -> policy::RubyVersions {
        policy::ruby_versions(self.name())
    }

    /// Fixed cop-level Include/Exclude eligibility. User exclusions add to it;
    /// selectors and enabling a cop do not bypass it.
    fn path_policy(&self) -> Option<&policy::PathPolicy> {
        policy::path_policy(self.name())
    }

    /// The cop's severity: RuboCop's default for it.
    fn severity(&self) -> Severity {
        Severity::for_cop(self.name())
    }

    /// The settings this cop takes in `rustocop.toml`, beyond `enabled` and
    /// `exclude`. Config validation and the `.rubocop.yml` import read this
    /// list; a cop that declares a setting also implements
    /// [`Cop::configure`].
    fn settings(&self) -> &'static [Setting] {
        &[]
    }

    /// A copy of this cop with `values` for its settings. Each value is
    /// valid for the [`Setting`] it belongs to, and a setting without a
    /// value keeps what this cop has. `None` for a cop without settings.
    /// Config resolution builds these (see `ResolvedConfig::configured`).
    fn configure(&self, _values: &Values) -> Option<Box<dyn Cop>> {
        None
    }

    /// `"max"` when the cop declares that setting. Read from
    /// [`Cop::settings`]; a cop does not implement this.
    fn max_setting(&self) -> Option<&'static str> {
        let max = self.settings().iter().find(|setting| setting.name == "max");
        max.map(|setting| setting.name)
    }

    /// A copy of this cop for what the configuration says about the cops it
    /// follows (RuboCop's `config.cop_enabled?` and `for_cop`), never their
    /// CLI selection. Config resolution calls it on the copy that
    /// [`Cop::configure`] built, if any. `None` for a cop that follows none.
    fn with_companions(&self, _companions: &Companions) -> Option<Box<dyn Cop>> {
        None
    }

    /// Location-free filename checks (first consumer: Bundler/GemFilename).
    /// Receives only the supplied config-relative name, never file contents or
    /// a resolved target. Runs on opaque gem locks and on valid Ruby inputs;
    /// mandatory Ruby parse/resource failures prevent all ordinary findings.
    fn check_filename(&self, _path: &std::path::Path, _out: &mut Vec<GlobalOffense>) {}

    /// Location-free valid-Ruby findings, including empty gemspecs (first
    /// consumer: Gemspec/RequiredRubyVersion). Shares the owned main parse;
    /// never called on opaque locks or rejected Ruby. No source edits allowed.
    fn check_global(
        &self,
        _result: &ruby_prism::ParseResult<'_>,
        _file: &SourceFile,
        _out: &mut Vec<GlobalOffense>,
    ) {
    }

    /// Whole-source checks for line-oriented cops.
    fn check_source(&self, _file: &SourceFile, _out: &mut Vec<Offense>) {}

    /// Main-parse facts for diagnostic-oriented rules, without reparsing.
    /// This does not bypass the pipeline's fatal-parser gate. Direct internal
    /// callers may inspect a recovered tree (e.g. a version-inactive rule).
    fn check_parse(
        &self,
        _result: &ruby_prism::ParseResult<'_>,
        _file: &SourceFile,
        _out: &mut Vec<Offense>,
    ) {
    }

    /// Opt in to fatal delivery on rejected input. Ordinary parse/source/node
    /// hooks never run there. The mandatory file-error gate is independent of
    /// selection and cannot be suppressed by this bounded lifecycle.
    fn wants_fatal_diagnostics(&self) -> bool {
        false
    }

    /// Fatal findings from the owned main-parse snapshot, without AST access,
    /// candidate parses or source corrections. Only opted-in cops receive this.
    fn check_fatal(&self, _file: &SourceFile, _out: &mut Vec<crate::source::FatalDiagnostic>) {}

    /// Opt in to the pipeline's bounded post-investigation phase. This runs
    /// once after all ordinary callbacks, before directive filtering, only on
    /// valid input and only if this cop completed ordinary investigation.
    fn wants_post_investigation(&self) -> bool {
        false
    }

    /// Inspect immutable ordinary findings (including suppressed ones) and the
    /// effective per-file cop universe. Peers' post findings are not inputs.
    /// No AST, reparsing or configuration engine is exposed. Emitted findings
    /// obey the same self-suppression and correction guards as ordinary ones.
    fn check_post_investigation(
        &self,
        _cx: &PostInvestigation<'_>,
        _file: &SourceFile,
        _out: &mut Vec<Offense>,
    ) {
    }

    /// AST checks. Called exactly once for every node in the tree, parents
    /// before children, including typed children such as `StatementsNode`,
    /// `ArgumentsNode`, `RescueNode`, `EnsureNode` and `ElseNode`. `cx` holds
    /// the node's ancestors: a cop whose rule depends on them asks there
    /// instead of walking the tree itself.
    fn check_node<'pr>(
        &self,
        _node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        _file: &SourceFile,
        _out: &mut Vec<Offense>,
    ) {
    }

    /// Whether [`Cop::check_node`] looks at nodes of `node`'s kind. The answer
    /// must depend on the kind alone: the walker asks once per kind per file
    /// and skips `check_node` for every node of a kind the cop declines.
    /// Declare it when `check_node` starts by returning for other kinds; a
    /// cop that does not is called for every node.
    fn wants_node(&self, _node: &Node<'_>) -> bool {
        true
    }

    /// Whether the walker calls [`Cop::leave_node`] for this cop. Off by
    /// default, so a cop that does not use the hook costs nothing for it.
    fn wants_leave_node(&self) -> bool {
        false
    }

    /// Called once for every node after its whole subtree (every descendant's
    /// `check_node` and `leave_node`), with the context `check_node` had.
    /// Only for cops whose [`Cop::wants_leave_node`] is true.
    fn leave_node<'pr>(
        &self,
        _node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        _file: &SourceFile,
        _out: &mut Vec<Offense>,
    ) {
    }
}

/// What the configuration says about the cops that other cops follow,
/// independent of CLI selection: a cop's `enabled`, else its department's,
/// else its default (Layout/SingleLineBlockChain reads only its own).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Companions {
    /// Layout/LineLength's `max`, `None` when that cop is disabled.
    pub line_length_limit: Option<usize>,
    /// Style/EndlessMethod: SingleLineMethods may correct to endless methods.
    pub endless_method: bool,
    /// Style/FetchEnvVar: HashLookupMethod leaves `ENV` lookups to it.
    pub fetch_env_var: bool,
    /// Style/BitwisePredicate: NumericPredicate leaves bitwise comparisons to it.
    pub bitwise_predicate: bool,
    /// Style/GuardClause and Style/UnlessElse: MissingElse's exemptions.
    pub guard_clause: bool,
    pub unless_else: bool,
    /// Style/MissingElse: EmptyElse keeps the clauses it requires.
    pub missing_else: bool,
    /// Style/FrozenStringLiteralComment, for EmptyLiteral.
    pub frozen_string_literal_comment: bool,
    /// Style/DigChain: SingleArgumentDig leaves chains to it.
    pub dig_chain: bool,
    /// Lint/ConstantResolution: RedundantConstantBase defers to it.
    pub constant_resolution: bool,
    /// Layout/SingleLineBlockChain: RedundantLineBreak defers chains on
    /// single-line blocks to it.
    pub single_line_block_chain: bool,
    /// Layout/SpaceInsideParens: SpaceAroundBlockParameters leaves the inside
    /// of a lambda's parameter parentheses to it.
    pub space_inside_parens: bool,
    /// Style/StringLiterals is disabled or prefers single quotes, for the cops
    /// that write a string literal (RuboCop's `StringLiteralsHelp`).
    pub single_quotes: bool,
}

/// One registered cop's per-file investigation facts. Absence from the universe
/// means unregistered as written, not disabled/deselected/inapplicable. These
/// facts never themselves prove a directive redundant or safe to remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvestigatedCop {
    /// Exact registered name (no namespace correction or fuzzy resolution).
    pub name: &'static str,
    /// Configuration/default enablement, independent of selectors and gates.
    pub configured_enabled: bool,
    /// Selected candidate eligible at fixed Ruby3.4 and this supplied path;
    /// includes disabled candidates that a source enable could mobilize.
    pub available: bool,
    /// Mobilized for ordinary investigation, including scoped source opt-ins.
    pub investigated: bool,
    /// Investigation ran without a cop panic. Incomplete evidence must not be
    /// mistaken for an absence of offenses.
    pub completed: bool,
}

/// Pipeline-only post-investigation inputs, borrowed for one callback. Raw
/// findings are the immutable ordinary batch, before suppression or sorting;
/// corrections may already have been withheld by shared safety guards.
pub struct PostInvestigation<'a> {
    /// All ordinary findings, including enabled and suppressed highlights.
    pub raw_offenses: &'a [Offense],
    /// Location-free ordinary findings, before suppression. The pin uses line1
    /// solely for directive scope, not as a reported source location.
    pub raw_global_offenses: &'a [GlobalOffense],
    /// The exact shared analyzer used for final finding/correction filtering.
    pub directives: &'a crate::directives::Directives,
    /// All registered names with configured/selected/path/run facts.
    pub cops: &'a [InvestigatedCop],
    /// Whether this pass requests corrections (false during validation).
    pub corrections_requested: bool,
}
impl PostInvestigation<'_> {
    /// Ordinary findings hidden by any disabled line in their highlight.
    pub fn suppressed_offenses(&self) -> impl Iterator<Item = &Offense> {
        self.raw_offenses.iter().filter(|o| {
            !self
                .directives
                .is_enabled_for_lines(o.cop, o.line, o.end_line)
        })
    }
}

/// Where a node sits in the tree, for [`Cop::check_node`] and
/// [`Cop::leave_node`]: its ancestors, typed children (`StatementsNode`,
/// `ArgumentsNode`, …) included, as Prism nests them (so a block is a
/// child of its call, unlike in RuboCop's AST).
#[derive(Clone, Copy)]
pub struct NodeContext<'a, 'pr> {
    /// The ancestors, outermost (the `ProgramNode`) first.
    ancestors: &'a [Node<'pr>],
    corrections_requested: bool,
    /// Exact send ranges already reported by NonDeterministicRequireOrder.
    /// Shared only within this walk; no allocation unless that cop reports.
    require_order_reports: &'a RefCell<HashSet<(usize, usize)>>,
}

impl<'a, 'pr> NodeContext<'a, 'pr> {
    /// Whether correction generation is requested. Expensive correction-only
    /// work should be skipped in ordinary lint and post-fix validation runs.
    /// This never controls verification required for offense detection.
    pub fn corrections_requested(&self) -> bool {
        self.corrections_requested
    }

    /// The node's parent; `None` for the root.
    pub fn parent(&self) -> Option<&'a Node<'pr>> {
        self.ancestors.last()
    }

    /// The node's ancestors, innermost (the parent) first, up to the root:
    /// RuboCop's `each_ancestor`.
    pub fn ancestors(
        &self,
    ) -> impl DoubleEndedIterator<Item = &'a Node<'pr>> + ExactSizeIterator + use<'a, 'pr> {
        self.ancestors.iter().rev()
    }
}

/// A cop prints as its name.
impl std::fmt::Debug for dyn Cop + '_ {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// All registered cops.
pub struct Registry {
    cops: Vec<Box<dyn Cop>>,
}

impl Registry {
    /// Build the registry. Extend as cops land.
    pub fn new() -> Self {
        Registry {
            cops: vec![
                Box::new(lint::Debugger),
                Box::new(bundler::GemFilename),
                Box::new(bundler::DuplicatedGem),
                Box::new(bundler::DuplicatedGroup),
                Box::new(bundler::GemComment),
                Box::new(bundler::GemVersion),
                Box::new(bundler::InsecureProtocolSource),
                Box::new(bundler::OrderedGems),
                Box::new(gemspec::AddRuntimeDependency),
                Box::new(gemspec::AttributeAssignment),
                Box::new(gemspec::DependencyVersion),
                Box::new(gemspec::DeprecatedAttributeAssignment),
                Box::new(gemspec::DevelopmentDependencies),
                Box::new(gemspec::DuplicatedAssignment),
                Box::new(gemspec::OrderedDependencies),
                Box::new(gemspec::RequireMFA),
                Box::new(gemspec::RequiredRubyVersion),
                Box::new(gemspec::RubyVersionGlobalsUsage),
                Box::new(metrics::ParameterLists::default()),
                Box::new(metrics::BlockLength::default()),
                Box::new(metrics::ClassLength::default()),
                Box::new(metrics::MethodLength::default()),
                Box::new(metrics::ModuleLength::default()),
                Box::new(metrics::BlockNesting::default()),
                Box::new(metrics::CollectionLiteralLength),
                Box::new(metrics::CyclomaticComplexity::default()),
                Box::new(metrics::PerceivedComplexity::default()),
                Box::new(metrics::AbcSize::default()),
                Box::new(migration::DepartmentName),
                Box::new(lint::ParenthesesAsGroupedExpression),
                Box::new(lint::PercentStringArray),
                Box::new(lint::PercentSymbolArray),
                Box::new(lint::RedundantDirGlobSort),
                Box::new(lint::RedundantSafeNavigation),
                Box::new(lint::SafeNavigationChain),
                Box::new(lint::SafeNavigationConsistency),
                Box::new(lint::SafeNavigationWithEmpty),
                Box::new(lint::SendWithMixinArgument),
                Box::new(lint::ShadowedArgument),
                Box::new(lint::ShadowingOuterLocalVariable),
                Box::new(lint::SuppressedExceptionInNumberConversion),
                Box::new(lint::SymbolConversion),
                Box::new(lint::TopLevelReturnWithArgument),
                Box::new(lint::UnderscorePrefixedVariableName),
                Box::new(lint::UnexpectedBlockArity),
                Box::new(lint::UnreachablePatternBranch),
                Box::new(lint::UnusedBlockArgument),
                Box::new(lint::UnusedMethodArgument),
                Box::new(lint::UnreachableLoop),
                Box::new(lint::ShadowedException),
                Box::new(lint::RedundantSplatExpansion),
                Box::new(lint::RedundantStringCoercion),
                Box::new(lint::RedundantTypeConversion),
                Box::new(lint::RedundantWithIndex),
                Box::new(lint::RedundantWithObject),
                Box::new(lint::RefinementImportMethods),
                Box::new(lint::RegexpAsCondition),
                Box::new(lint::RequireParentheses),
                Box::new(lint::RequireRangeParentheses),
                Box::new(lint::RequireRelativeSelfPath),
                Box::new(lint::RescueType),
                Box::new(lint::ReturnInVoidContext),
                Box::new(lint::NonAtomicFileOperation),
                Box::new(lint::NumberConversion),
                Box::new(lint::ToEnumArguments),
                Box::new(lint::ToJSON),
                Box::new(lint::NumericOperationWithConstantResult),
                Box::new(lint::MissingSuper),
                Box::new(lint::NameTypo),
                Box::new(lint::UnusedPrivateMethod),
                Box::new(lint::UselessDefaultValueArgument),
                Box::new(lint::UselessDefined),
                Box::new(lint::UselessMethodDefinition),
                Box::new(lint::UselessNumericOperation),
                Box::new(lint::UselessOr),
                Box::new(lint::UselessSetterCall),
                Box::new(lint::UselessTimes),
                Box::new(lint::UselessRuby2Keywords),
                Box::new(lint::UselessRescue),
                Box::new(lint::NextWithoutAccumulator),
                Box::new(lint::UnmodifiedReduceAccumulator),
                Box::new(lint::MultipleComparison),
                Box::new(lint::RandOne),
                Box::new(lint::MixedCaseRange),
                Box::new(lint::NumberedParameterAssignment),
                Box::new(lint::LiteralAssignmentInCondition),
                Box::new(lint::Loop),
                Box::new(lint::NoReturnInBeginEndBlocks),
                Box::new(lint::ItWithoutArgumentsInBlock),
                Box::new(lint::UselessElseWithoutRescue),
                Box::new(lint::NonDeterministicRequireOrder),
                Box::new(lint::EmptyBlock),
                Box::new(lint::EmptyClass),
                Box::new(lint::EmptyExpression),
                Box::new(lint::EmptyFile),
                Box::new(lint::ScriptPermission),
                Box::new(lint::Syntax),
                Box::new(lint::EmptyInPattern),
                Box::new(lint::AmbiguousAssignment),
                Box::new(lint::ArgumentMismatch),
                Box::new(lint::SuperArgumentMismatch),
                Box::new(lint::CircularArgumentReference),
                Box::new(lint::ConstantDefinitionInBlock),
                Box::new(lint::ConstantOverwrittenInRescue),
                Box::new(lint::ConstantReassignment),
                Box::new(lint::ConstantResolution),
                Box::new(lint::UselessConstantScoping),
                Box::new(lint::DeprecatedConstants),
                Box::new(lint::DeprecatedOpenSSLConstant),
                Box::new(lint::DeprecatedReference),
                Box::new(lint::BigDecimalNew),
                Box::new(lint::ErbNewArguments),
                Box::new(lint::UnifiedInteger),
                Box::new(lint::UriEscapeUnescape),
                Box::new(lint::UriRegexp),
                Box::new(lint::OrAssignmentToConstant),
                Box::new(lint::DisjunctiveAssignmentInConstructor),
                Box::new(lint::AmbiguousOperatorPrecedence),
                Box::new(lint::CopDirectiveSyntax),
                Box::new(lint::DuplicateBranch),
                Box::new(lint::DuplicateMagicComment),
                Box::new(lint::MisplacedMagicComment),
                Box::new(lint::OrderedMagicComments),
                Box::new(lint::ArrayLiteralInRegexp),
                Box::new(lint::BooleanSymbol),
                Box::new(lint::UnescapedBracketInRegexp),
                Box::new(lint::RedundantRegexpQuantifiers),
                Box::new(lint::EachWithObjectArgument),
                Box::new(lint::FlipFlop),
                Box::new(lint::HashCompareByIdentity),
                Box::new(lint::HashNewWithKeywordArgumentsAsDefault),
                Box::new(lint::HeredocMethodCallPosition),
                Box::new(lint::IdentityComparison),
                Box::new(lint::LambdaWithoutLiteralBlock),
                Box::new(lint::IneffectiveAccessModifier),
                Box::new(lint::UselessAccessModifier),
                Box::new(lint::ImplicitStringConcatenation),
                Box::new(lint::InterpolationCheck),
                Box::new(lint::IncompatibleIoSelectWithFiberScheduler),
                Box::new(lint::TripleQuotes),
                Box::new(lint::ElseLayout),
                Box::new(lint::TrailingCommaInAttributeDeclaration),
                Box::new(lint::FloatComparison),
                Box::new(lint::FloatOutOfRange),
                Box::new(lint::MixedRegexpCaptureTypes),
                Box::new(lint::NestedPercentLiteral),
                Box::new(lint::DuplicateMatchPattern),
                Box::new(lint::DuplicateRegexpCharacterClassElement),
                Box::new(lint::DuplicateRequire),
                Box::new(lint::DuplicateCaseCondition),
                Box::new(lint::DuplicateElsifCondition),
                Box::new(lint::DuplicateHashKey),
                Box::new(lint::DuplicateRescueException),
                Box::new(lint::DuplicateSetElement),
                Box::new(lint::BinaryOperatorWithIdenticalOperands),
                Box::new(lint::SelfAssignment),
                Box::new(lint::DuplicateMethods),
                Box::new(lint::EmptyConditionalBody),
                Box::new(lint::EmptyEnsure),
                Box::new(lint::EmptyInterpolation),
                Box::new(lint::EmptyWhen),
                Box::new(lint::EnsureReturn),
                Box::new(lint::NonLocalExitFromIterator),
                Box::new(lint::UnreachableCode),
                Box::new(lint::Void),
                Box::new(lint::AmbiguousBlockAssociation),
                Box::new(lint::AmbiguousOperator),
                Box::new(lint::AmbiguousRange),
                Box::new(lint::AmbiguousRegexpLiteral),
                Box::new(lint::AssignmentInCondition),
                Box::new(lint::RescueException),
                Box::new(lint::SuppressedException),
                Box::new(lint::RaiseException),
                Box::new(lint::InheritException),
                Box::new(lint::NestedMethodDefinition),
                Box::new(lint::DeprecatedClassMethods),
                Box::new(lint::OutOfRangeRegexpRef),
                Box::new(lint::FormatParameterMismatch),
                Box::new(lint::RedundantRequireStatement),
                Box::new(lint::RedundantCopEnableDirective),
                Box::new(lint::RedundantCopDisableDirective),
                Box::new(lint::MissingCopEnableDirective),
                Box::new(lint::StructNewOverride),
                Box::new(lint::DataDefineOverride),
                Box::new(lint::SharedMutableDefault),
                Box::new(lint::LiteralAsCondition),
                Box::new(lint::LiteralInInterpolation),
                Box::new(lint::UselessAssignment),
                Box::new(security::CompoundHash),
                Box::new(security::Eval),
                Box::new(security::IoMethods),
                Box::new(security::JsonLoad),
                Box::new(security::MarshalLoad),
                Box::new(security::Open),
                Box::new(security::YamlLoad),
                Box::new(style::EmptyLiteral::default()),
                Box::new(style::EmptyClassDefinition),
                Box::new(style::EmptyMethod::default()),
                Box::new(style::RedundantConstantBase::default()),
                Box::new(style::RedundantFreeze),
                Box::new(style::RedundantInitialize),
                Box::new(style::RedundantSelf),
                Box::new(style::RedundantFetchBlock),
                Box::new(style::SafeNavigation),
                Box::new(style::SafeNavigationChainLength),
                Box::new(style::IpAddresses),
                Box::new(style::SingleArgumentDig::default()),
                Box::new(style::DigChain),
                Box::new(style::AndOr),
                Box::new(style::DoubleNegation),
                Box::new(style::CaseEquality),
                Box::new(style::CombinableDefined),
                Box::new(style::CombinableLoops),
                Box::new(style::For),
                Box::new(style::InfiniteLoop),
                Box::new(style::InPatternThen),
                Box::new(style::MultilineInPatternThen),
                Box::new(style::AccessModifierDeclarations),
                Box::new(style::AccessorGrouping),
                Box::new(style::Attr),
                Box::new(style::BisectedAttrAccessor),
                Box::new(style::TrivialAccessors),
                Box::new(style::ArgumentsForwarding),
                Box::new(style::ItAssignment),
                Box::new(style::ItBlockParameter),
                Box::new(style::NumberedParameters),
                Box::new(style::NumberedParametersLimit),
                Box::new(style::NilLambda),
                Box::new(style::SuperArguments),
                Box::new(style::SuperWithArgsParentheses),
                Box::new(style::KeywordParametersOrder),
                Box::new(style::RedundantArgument),
                Box::new(style::KeywordArgumentsMerging),
                Box::new(style::ExplicitBlockArgument),
                Box::new(style::OptionalArguments),
                Box::new(style::OptionalBooleanParameter),
                Box::new(style::OptionHash),
                Box::new(style::AsciiComments),
                Box::new(style::AutoResourceCleanup),
                Box::new(style::GlobalStdStream),
                Box::new(style::GlobalVars),
                Box::new(style::SpecialGlobalVars),
                Box::new(style::PerlBackrefs),
                Box::new(style::StderrPuts),
                Box::new(style::PredicateWithKind),
                Box::new(style::ClassAndModuleChildren),
                Box::new(style::ClassCheck),
                Box::new(style::ClassVars),
                Box::new(style::ColonMethodCall),
                Box::new(style::ColonMethodDefinition),
                Box::new(style::MethodDefParentheses),
                Box::new(style::AmbiguousEndlessMethodDefinition),
                Box::new(style::DefWithParentheses),
                Box::new(style::Alias),
                Box::new(style::OneClassPerFile),
                Box::new(style::StaticClass),
                Box::new(style::OpenStructUse),
                Box::new(style::ClassMethods),
                Box::new(style::ClassMethodsDefinitions),
                Box::new(style::ModuleFunction),
                Box::new(style::MixinUsage),
                Box::new(style::MixinGrouping),
                Box::new(style::ModuleMemberExistenceCheck),
                Box::new(style::ConstantVisibility),
                Box::new(style::MutableConstant),
                Box::new(style::DataInheritance),
                Box::new(style::StructInheritance),
                Box::new(style::RedundantStructKeywordInit),
                Box::new(style::ClassEqualityComparison),
                Box::new(style::MissingRespondToMissing),
                Box::new(style::EvalWithLocation),
                Box::new(style::Send),
                Box::new(style::SendWithLiteralMethodName),
                Box::new(style::BlockComments),
                Box::new(style::CommentAnnotation),
                Box::new(style::CommentedKeyword),
                Box::new(style::Copyright),
                Box::new(style::BlockDelimiters),
                Box::new(style::MultilineBlockChain),
                Box::new(style::SingleLineDoEndBlock),
                Box::new(style::SingleLineBlockParams),
                Box::new(style::Lambda),
                Box::new(style::LambdaCall),
                Box::new(style::Proc),
                Box::new(style::StabbyLambdaParentheses),
                Box::new(style::EmptyBlockParameter),
                Box::new(style::EmptyLambdaParameter),
                Box::new(style::ConditionalAssignment::default()),
                Box::new(style::ExpandPathArguments),
                Box::new(style::NestedFileDirname),
                Box::new(style::RedundantCurrentDirectoryInPath),
                Box::new(style::RedundantFileExtensionInRequire),
                Box::new(style::RequireOrder),
                Box::new(style::EnvHome),
                Box::new(style::DateTime),
                Box::new(style::FetchEnvVar),
                Box::new(style::TimeNow),
                Box::new(style::RandomWithOffset),
                Box::new(style::Dir),
                Box::new(style::DirEmpty),
                Box::new(style::FileEmpty),
                Box::new(style::FileNull),
                Box::new(style::FileOpen),
                Box::new(style::FileRead),
                Box::new(style::FileTouch),
                Box::new(style::FileWrite),
                Box::new(style::YAMLFileRead),
                Box::new(style::DirectiveScope),
                Box::new(style::DisableCopsWithinSourceCodeDirective),
                Box::new(style::DocumentDynamicEvalDefinition),
                Box::new(style::Documentation),
                Box::new(style::DocumentationMethod),
                Box::new(style::Encoding),
                Box::new(style::InlineComment),
                Box::new(style::MagicCommentFormat),
                Box::new(style::EndlessMethod),
                Box::new(style::SingleLineMethods::default()),
                Box::new(style::TrailingMethodEndStatement),
                Box::new(style::TrailingBodyOnClass),
                Box::new(style::TrailingBodyOnModule),
                Box::new(style::TrailingBodyOnMethodDefinition),
                Box::new(style::TrailingUnderscoreVariable),
                Box::new(style::TrailingCommaInArguments),
                Box::new(style::TrailingCommaInArrayLiteral),
                Box::new(style::TrailingCommaInBlockArgs),
                Box::new(style::TrailingCommaInHashLiteral),
                Box::new(style::TopLevelMethodDefinition),
                Box::new(style::MultilineMethodSignature::default()),
                Box::new(style::FrozenStringLiteralComment),
                Box::new(style::SignalException),
                Box::new(style::ImplicitRuntimeError),
                Box::new(style::RaiseArgs),
                Box::new(style::RedundantException),
                Box::new(style::RescueModifier),
                Box::new(style::BeginBlock),
                Box::new(style::EndBlock),
                Box::new(style::RedundantBegin::default()),
                Box::new(style::RescueStandardError),
                Box::new(style::ReturnNil),
                Box::new(style::ReturnNilInPredicateMethodDefinition),
                Box::new(style::SoleNestedConditional),
                Box::new(style::HashSyntax::default()),
                Box::new(style::GuardClause::default()),
                Box::new(style::Next),
                Box::new(style::IfUnlessModifier::default()),
                Box::new(style::MethodCallWithArgsParentheses),
                Box::new(style::MethodCallWithoutArgsParentheses),
                Box::new(style::MethodCalledOnDoEndBlock),
                Box::new(style::OperatorMethodCall),
                Box::new(style::NestedParenthesizedCalls),
                Box::new(style::PercentLiteralDelimiters),
                Box::new(style::PercentQLiterals),
                Box::new(style::BarePercentLiterals),
                Box::new(style::CharacterLiteral),
                Box::new(style::CommandLiteral),
                Box::new(style::FormatString),
                Box::new(style::FormatStringToken),
                Box::new(style::RedundantFormat),
                Box::new(style::StringConcatenation),
                Box::new(style::StringChars),
                Box::new(style::StringMethods),
                Box::new(style::Strip),
                Box::new(style::UnpackFirst),
                Box::new(style::HashAsLastArrayItem),
                Box::new(style::HashConversion),
                Box::new(style::HashEachMethods),
                Box::new(style::EachForSimpleLoop),
                Box::new(style::EachWithObject),
                Box::new(style::MapCompactWithConditionalBlock),
                Box::new(style::MapJoin),
                Box::new(style::MapToHash),
                Box::new(style::MapToSet),
                Box::new(style::PartitionInsteadOfDoubleSelect),
                Box::new(style::ReduceToHash),
                Box::new(style::RedundantEach),
                Box::new(style::RedundantFilterChain),
                Box::new(style::SelectByKind),
                Box::new(style::SelectByRange),
                Box::new(style::SelectByRegexp),
                Box::new(style::ExactRegexpMatch),
                Box::new(style::RegexpLiteral),
                Box::new(style::RedundantRegexpArgument::default()),
                Box::new(style::RedundantRegexpCharacterClass),
                Box::new(style::RedundantRegexpConstructor),
                Box::new(style::RedundantRegexpEscape),
                Box::new(style::MapIntoArray),
                Box::new(style::HashExcept),
                Box::new(style::HashSlice),
                Box::new(style::RedundantDoubleSplatHashBraces),
                Box::new(style::StringHashKeys),
                Box::new(style::HashTransformKeys),
                Box::new(style::HashTransformValues),
                Box::new(style::PreferredHashMethods),
                Box::new(style::CollectionCompact),
                Box::new(style::CollectionMethods),
                Box::new(style::ObjectThen),
                Box::new(style::ComparableClamp),
                Box::new(style::BitwisePredicate),
                Box::new(style::EvenOdd),
                Box::new(style::ExponentialNotation),
                Box::new(style::FloatDivision),
                Box::new(style::NumericLiteralPrefix),
                Box::new(style::NumericLiterals),
                Box::new(style::NumericPredicate),
                Box::new(style::UnlessLogicalOperators),
                Box::new(style::YodaCondition),
                Box::new(style::YodaExpression),
                Box::new(style::ZeroLengthPredicate),
                Box::new(style::MinMax),
                Box::new(style::MinMaxComparison),
                Box::new(style::InverseMethods),
                Box::new(style::NilComparison),
                Box::new(style::NonNilCheck),
                Box::new(style::Not),
                Box::new(style::MultipleComparison),
                Box::new(style::RedundantMinMaxBy),
                Box::new(style::RedundantSort),
                Box::new(style::RedundantSortBy),
                Box::new(style::ComparableBetween),
                Box::new(style::ReverseFind),
                Box::new(style::CollectionQuerying),
                Box::new(style::HashFetchChain),
                Box::new(style::HashLikeCase),
                Box::new(style::CaseLikeIf),
                Box::new(style::IdenticalConditionalBranches),
                Box::new(style::IfInsideElse),
                Box::new(style::MultilineMemoization),
                Box::new(style::OrAssignment),
                Box::new(style::SelfAssignment),
                Box::new(style::SwapValues),
                Box::new(style::ParallelAssignment),
                Box::new(style::RedundantAssignment),
                Box::new(style::RedundantSelfAssignment),
                Box::new(style::RedundantSelfAssignmentBranch),
                Box::new(style::IfUnlessModifierOfIfUnless),
                Box::new(style::IfWithBooleanLiteralBranches),
                Box::new(style::IfWithSemicolon),
                Box::new(style::MissingElse::default()),
                Box::new(style::EmptyElse::default()),
                Box::new(style::MultilineIfModifier),
                Box::new(style::MultilineIfThen),
                Box::new(style::MultilineTernaryOperator),
                Box::new(style::MultilineWhenThen),
                Box::new(style::UnlessElse),
                Box::new(style::WhenThen),
                Box::new(style::EmptyCaseCondition),
                Box::new(style::OneLineConditional),
                Box::new(style::ParenthesesAroundCondition),
                Box::new(style::NestedModifier),
                Box::new(style::NestedTernaryOperator),
                Box::new(style::TernaryParentheses),
                Box::new(style::NegatedIf),
                Box::new(style::NegatedUnless),
                Box::new(style::NegatedWhile),
                Box::new(style::WhileUntilDo),
                Box::new(style::WhileUntilModifier::default()),
                Box::new(style::NegatedIfElseCondition),
                Box::new(style::InvertibleUnlessCondition),
                Box::new(style::HashLookupMethod::default()),
                Box::new(style::ArrayCoercion),
                Box::new(style::ArrayFirstLast),
                Box::new(style::ArrayIntersect),
                Box::new(style::ArrayIntersectWithSingleElement),
                Box::new(style::ArrayJoin),
                Box::new(style::ConcatArrayLiterals),
                Box::new(style::NegativeArrayIndex),
                Box::new(style::RedundantArrayConstructor),
                Box::new(style::RedundantArrayFlatten),
                Box::new(style::SlicingWithRange),
                Box::new(style::WordArray),
                Box::new(style::SymbolArray),
                Box::new(style::SymbolProc),
                Box::new(style::TallyMethod),
                Box::new(style::Sample),
                Box::new(style::LineEndConcatenation),
                Box::new(style::RedundantLineContinuation),
                Box::new(style::RedundantPercentQ),
                Box::new(style::RedundantCapitalW),
                Box::new(style::RedundantStringEscape),
                Box::new(style::RedundantHeredocDelimiterQuotes),
                Box::new(style::EmptyHeredoc::default()),
                Box::new(style::QuotedSymbols),
                Box::new(style::SymbolLiteral),
                Box::new(style::VariableInterpolation),
                Box::new(style::EmptyStringInsideInterpolation),
                Box::new(style::RedundantInterpolation),
                Box::new(style::RedundantInterpolationUnfreeze),
                Box::new(style::StringLiterals),
                Box::new(style::StringLiteralsInInterpolation),
                Box::new(style::RedundantCondition),
                Box::new(style::RedundantConditional),
                Box::new(style::RedundantReturn),
                Box::new(style::Semicolon),
                Box::new(style::RedundantParentheses),
                Box::new(naming::ClassAndModuleCamelCase),
                Box::new(naming::ConstantName),
                Box::new(naming::MethodName),
                Box::new(naming::AsciiIdentifiers),
                Box::new(naming::BlockParameterName),
                Box::new(naming::VariableNumber),
                Box::new(naming::AccessorMethodName),
                Box::new(naming::PredicatePrefix),
                Box::new(naming::BinaryOperatorParameterName),
                Box::new(naming::BlockForwarding),
                Box::new(naming::InclusiveLanguage),
                Box::new(naming::FileName),
                Box::new(naming::HeredocDelimiterCase),
                Box::new(naming::HeredocDelimiterNaming),
                Box::new(naming::MemoizedInstanceVariableName),
                Box::new(naming::PredicateMethod),
                Box::new(naming::RescuedExceptionsVariableName),
                Box::new(naming::MethodParameterName),
                Box::new(naming::VariableName),
                Box::new(layout::LineLength::default()),
                Box::new(layout::TrailingWhitespace),
                Box::new(layout::TrailingEmptyLines),
                Box::new(layout::EmptyLines),
                Box::new(layout::EmptyLineBetweenDefs),
                Box::new(layout::EmptyLinesAroundClassBody),
                Box::new(layout::EmptyLinesAroundModuleBody),
                Box::new(layout::EmptyLinesAroundMethodBody),
                Box::new(layout::SpaceAfterComma),
                Box::new(layout::SpaceAfterSemicolon),
                Box::new(layout::SpaceBeforeComma),
                Box::new(layout::SpaceAroundEqualsInParameterDefault),
                Box::new(layout::SpaceAroundOperators),
                Box::new(layout::SpaceInsideParens),
                Box::new(layout::SpaceInsideHashLiteralBraces),
                Box::new(layout::SpaceInsideBlockBraces),
                Box::new(layout::SpaceInsideArrayLiteralBrackets),
                Box::new(layout::SpaceInsideReferenceBrackets),
                Box::new(layout::IndentationStyle),
                Box::new(layout::IndentationWidth),
                Box::new(layout::EndAlignment),
                Box::new(layout::AccessModifierIndentation),
                Box::new(layout::AssignmentIndentation),
                Box::new(layout::ArgumentAlignment),
                Box::new(layout::ArrayAlignment),
                Box::new(layout::HashAlignment),
                Box::new(layout::ParameterAlignment),
                Box::new(layout::FirstParameterIndentation),
                Box::new(layout::BeginEndAlignment),
                Box::new(layout::DefEndAlignment),
                Box::new(layout::BlockAlignment),
                Box::new(layout::ClosingParenthesisIndentation),
                Box::new(layout::FirstArgumentIndentation),
                Box::new(layout::FirstArrayElementIndentation),
                Box::new(layout::FirstHashElementIndentation),
                Box::new(layout::CaseIndentation),
                Box::new(layout::ElseAlignment),
                Box::new(layout::RescueEnsureAlignment),
                Box::new(layout::HeredocIndentation),
                Box::new(layout::ClosingHeredocIndentation),
                Box::new(layout::CommentIndentation),
                Box::new(layout::InitialIndentation),
                Box::new(layout::HeredocArgumentClosingParenthesis),
                Box::new(layout::IndentationConsistency),
                Box::new(layout::MultilineMethodCallIndentation),
                Box::new(layout::MultilineOperationIndentation),
                Box::new(layout::DotPosition),
                Box::new(layout::ConditionPosition),
                Box::new(layout::MultilineAssignmentLayout),
                Box::new(layout::SpaceBeforeComment),
                Box::new(layout::LeadingCommentSpace),
                Box::new(layout::EmptyComment),
                Box::new(layout::LeadingEmptyLines),
                Box::new(layout::EndOfLine),
                Box::new(layout::EmptyLineAfterMagicComment),
                Box::new(layout::EmptyLineAfterGuardClause),
                Box::new(layout::EmptyLineAfterMultilineCondition),
                Box::new(layout::EmptyLinesAfterModuleInclusion),
                Box::new(layout::EmptyLinesAroundAttributeAccessor),
                Box::new(layout::EmptyLinesAroundAccessModifier),
                Box::new(layout::EmptyLinesAroundBeginBody),
                Box::new(layout::EmptyLinesAroundBlockBody),
                Box::new(layout::EmptyLinesAroundExceptionHandlingKeywords),
                Box::new(layout::EmptyLinesAroundArguments),
                Box::new(layout::ExtraSpacing),
                Box::new(layout::SpaceBeforeFirstArg),
                Box::new(layout::SpaceBeforeSemicolon),
                Box::new(layout::FirstArrayElementLineBreak),
                Box::new(layout::FirstHashElementLineBreak),
                Box::new(layout::FirstMethodArgumentLineBreak),
                Box::new(layout::FirstMethodParameterLineBreak),
                Box::new(layout::MultilineArrayLineBreaks),
                Box::new(layout::MultilineHashKeyLineBreaks),
                Box::new(layout::MultilineMethodArgumentLineBreaks),
                Box::new(layout::MultilineMethodParameterLineBreaks),
                Box::new(layout::MultilineArrayBraceLayout),
                Box::new(layout::MultilineHashBraceLayout),
                Box::new(layout::MultilineMethodCallBraceLayout),
                Box::new(layout::MultilineMethodDefinitionBraceLayout),
                Box::new(layout::MultilineBlockLayout::default()),
                Box::new(layout::BlockEndNewline),
                Box::new(layout::SingleLineBlockChain),
                Box::new(layout::RedundantLineBreak::default()),
                Box::new(layout::LineContinuationLeadingSpace),
                Box::new(layout::LineContinuationSpacing),
                Box::new(layout::LineEndStringConcatenationIndentation),
                Box::new(layout::SpaceAfterColon),
                Box::new(layout::SpaceAfterMethodName),
                Box::new(layout::SpaceAfterNot),
                Box::new(layout::SpaceAroundKeyword),
                Box::new(layout::SpaceAroundMethodCallOperator),
                Box::new(layout::SpaceAroundBlockParameters::default()),
                Box::new(layout::SpaceInLambdaLiteral),
                Box::new(layout::SpaceBeforeBlockBraces),
                Box::new(layout::SpaceBeforeBrackets),
                Box::new(layout::SpaceInsideArrayPercentLiteral),
                Box::new(layout::SpaceInsidePercentLiteralDelimiters),
                Box::new(layout::SpaceInsideRangeLiteral),
                Box::new(layout::SpaceInsideStringInterpolation),
                Box::new(layout::ClassStructure),
            ],
        }
    }

    /// A registry of exactly these cops, for tests that need cops the real
    /// registry does not ship.
    #[cfg(test)]
    pub(crate) fn from_cops(cops: Vec<Box<dyn Cop>>) -> Self {
        Registry { cops }
    }

    /// The registered cops.
    pub fn cops(&self) -> &[Box<dyn Cop>] {
        &self.cops
    }

    /// Whether a cop with this exact name is registered.
    pub fn has_cop(&self, name: &str) -> bool {
        self.cops.iter().any(|cop| cop.name() == name)
    }

    /// A plausible correction for an unknown cop or department name.
    pub fn suggest_name(&self, name: &str) -> Option<&'static str> {
        let mut names: Vec<_> = self.cops.iter().map(|cop| cop.name()).collect();
        names.extend(Department::ALL.iter().map(|department| department.as_str()));
        // Stable tie-breaking independent of registration order.
        names.sort_unstable();
        name_similarity::find_similar_name(name, &names)
    }

    /// The cop with this exact name.
    pub fn cop(&self, name: &str) -> Option<&dyn Cop> {
        let cop = self.cops.iter().find(|cop| cop.name() == name);
        cop.map(|cop| &**cop)
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

/// Select the cops that run, resolving CLI flags over config over defaults.
///
/// Precedence: `--except` wins over everything; `--only` (when non-empty) is
/// an exclusive whitelist that also force-enables listed cops; then per-cop
/// config; then department config; then the cop's default.
///
/// A cop the config gives a setting runs as the config's copy of it.
pub fn select_active<'r>(
    registry: &'r Registry,
    config: &'r ResolvedConfig,
    only: &[String],
    except: &[String],
) -> Vec<&'r dyn Cop> {
    select_active_at_version(registry, config, only, except, config.ruby_version)
}

/// Feature tests can exercise a future-only real matcher without changing the
/// production target. No CLI/TOML/environment setting can call this seam.
#[cfg(test)]
pub(crate) fn select_active_for_version<'r>(
    registry: &'r Registry,
    config: &'r ResolvedConfig,
    only: &[String],
    except: &[String],
    version: policy::RubyVersion,
) -> Vec<&'r dyn Cop> {
    select_active_at_version(registry, config, only, except, version)
}

/// Configuration state only: independent of runtime applicability and CLI selection.
/// Directive pairing needs this fact even for version-inactive registered cops.
pub(crate) fn configured_enabled(cop: &dyn Cop, config: &ResolvedConfig) -> bool {
    let name = cop.name();
    let department = name.split('/').next().unwrap_or(name);
    // Department first, then per-cop: the more specific key wins.
    config
        .cop_settings
        .get(name)
        .and_then(|settings| settings.enabled)
        .or_else(|| {
            config
                .cop_settings
                .get(department)
                .and_then(|settings| settings.enabled)
                .filter(|&on| !on || cop.default_enabled() || cop.department_selection_enables())
        })
        .unwrap_or_else(|| cop.default_enabled())
}

fn select_active_at_version<'r>(
    registry: &'r Registry,
    config: &'r ResolvedConfig,
    only: &[String],
    except: &[String],
    version: policy::RubyVersion,
) -> Vec<&'r dyn Cop> {
    registry
        .cops()
        .iter()
        .filter(|cop| {
            let name = cop.name();
            let department = name.split('/').next().unwrap_or(name);
            let mut enabled = configured_enabled(cop.as_ref(), config);
            if !only.is_empty() {
                enabled = only.iter().any(|entry| entry == name)
                    || (only.iter().any(|entry| entry == department)
                        && (enabled || cop.department_selection_enables()));
            }
            if matches_selection(except, name, department) {
                enabled = false;
            }
            enabled && cop.ruby_versions().supports(version)
        })
        .map(|cop| {
            config
                .configured
                .get(cop.name())
                .map_or(cop.as_ref(), |configured| configured.as_ref())
        })
        .collect()
}

fn matches_selection(list: &[String], cop: &str, department: &str) -> bool {
    list.iter().any(|entry| entry == cop || entry == department)
}

/// A cop running on a file, numbered by its position in the file's initial
/// list of live cops (two entries may be the same cop).
#[derive(Clone, Copy)]
struct Running<'b> {
    id: usize,
    cop: &'b dyn Cop,
}

/// Walks the AST, dispatching every node to the active AST cops exactly once,
/// parents before children, typed children included (see [`walk`]), with the
/// node's ancestors; and, after its subtree, to the cops that want
/// `leave_node`.
///
/// Each cop call runs under [`catch`]: a cop that panics is moved from
/// `cops` (and `leaving`) to `panics` and gets no further nodes.
struct CopWalker<'a, 'b, 'pr> {
    /// The cops still running on this file, in registry order.
    cops: Vec<Running<'b>>,
    /// The ones among them that want `leave_node`, in the same order.
    leaving: Vec<Running<'b>>,
    /// Per node kind met so far, the cops whose [`Cop::wants_node`] takes it,
    /// in the same order. A file has few kinds, so a scan finds one.
    by_kind: Vec<(Discriminant<Node<'pr>>, Vec<Running<'b>>)>,
    /// The ancestors of the next node, outermost first: every branch node
    /// entered and not yet left.
    ancestors: Vec<Node<'pr>>,
    file: &'a SourceFile,
    out: &'a mut Vec<Offense>,
    panics: Vec<CopPanic>,
    corrections_requested: bool,
    require_order_reports: RefCell<HashSet<(usize, usize)>>,
}

impl<'pr> CopWalker<'_, '_, 'pr> {
    /// Call `check_node` (or, when `LEAVE`, `leave_node`) on `node` for
    /// every cop that takes it.
    fn dispatch<const LEAVE: bool>(&mut self, node: &Node<'pr>) {
        let kind = if LEAVE { 0 } else { self.kind(node) };
        let cx = NodeContext {
            ancestors: &self.ancestors,
            corrections_requested: self.corrections_requested,
            require_order_reports: &self.require_order_reports,
        };
        let file = self.file;
        // Tests hold every cop to its `wants_node`: a cop that declines a
        // kind must report nothing for a node of it.
        #[cfg(test)]
        if !LEAVE {
            for running in &self.cops {
                if !running.cop.wants_node(node) {
                    let before = self.out.len();
                    running.cop.check_node(node, &cx, file, self.out);
                    assert_eq!(
                        before,
                        self.out.len(),
                        "{} reports a node kind its wants_node declines",
                        running.cop.name()
                    );
                }
            }
        }
        let mut index = 0;
        loop {
            let cops = if LEAVE {
                &self.leaving
            } else {
                &self.by_kind[kind].1
            };
            let Some(&Running { id, cop }) = cops.get(index) else {
                return;
            };
            let out = &mut *self.out;
            let called = catch(|| {
                if LEAVE {
                    cop.leave_node(node, &cx, file, out);
                } else {
                    cop.check_node(node, &cx, file, out);
                }
            });
            match called {
                Ok(()) => index += 1,
                Err(panic) => {
                    let position = file.line_col(node.location().start_offset());
                    let cops = [&mut self.cops, &mut self.leaving]
                        .into_iter()
                        .chain(self.by_kind.iter_mut().map(|(_, cops)| cops));
                    self.panics.push(drop_cop(cops, id, position, panic));
                }
            }
        }
    }

    /// The index in `by_kind` of `node`'s kind, added on its first node.
    fn kind(&mut self, node: &Node<'pr>) -> usize {
        let kind = discriminant(node);
        if let Some(index) = self.by_kind.iter().position(|(known, _)| *known == kind) {
            return index;
        }
        let wanting = self
            .cops
            .iter()
            .copied()
            .filter(|running| running.cop.wants_node(node))
            .collect();
        self.by_kind.push((kind, wanting));
        self.by_kind.len() - 1
    }
}

/// Remove the cop numbered `id` from each of `lists`, and report its panic
/// at `position`.
#[cold]
fn drop_cop<'l, 'b: 'l>(
    lists: impl Iterator<Item = &'l mut Vec<Running<'b>>>,
    id: usize,
    position: (u32, u32),
    panic: CaughtPanic,
) -> CopPanic {
    let mut cop = "";
    for list in lists {
        if let Some(index) = list.iter().position(|running| running.id == id) {
            cop = list.remove(index).cop.name();
        }
    }
    CopPanic {
        cop,
        position: Some(position),
        panic,
    }
}

impl<'pr> walk::NodeVisitor<'pr> for CopWalker<'_, '_, 'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        self.dispatch::<false>(&node);
        self.ancestors.push(node);
    }

    fn leave(&mut self) {
        let node = self.ancestors.pop().expect("leave follows enter");
        if !self.leaving.is_empty() {
            self.dispatch::<true>(&node);
        }
    }

    // A leaf is never an ancestor: no push and pop.
    fn leaf(&mut self, node: Node<'pr>) {
        self.dispatch::<false>(&node);
        if !self.leaving.is_empty() {
            self.dispatch::<true>(&node);
        }
    }
}

/// A cop that panicked while checking a file.
#[derive(Debug)]
pub struct CopPanic {
    /// The cop's name.
    pub cop: &'static str,
    /// The 1-based (line, column) of the node it was checking; `None` when it
    /// panicked in `check_source`.
    pub position: Option<(u32, u32)>,
    /// What it panicked with.
    pub panic: CaughtPanic,
}

/// Run the active cops over a parsed file: every `check_source`/`check_parse`,
/// then one AST walk dispatching each node to every `check_node` (and, after the
/// node's subtree, to every `leave_node` a cop wants).
///
/// Each cop call is isolated. A cop that panics is reported in the returned
/// list and sits out the rest of the file, while the other cops carry on;
/// the offenses it pushed before panicking stay in `out` (RuboCop keeps a
/// failing cop's offenses too). Call it inside an
/// [`isolation::QuietPanics`](crate::isolation::QuietPanics) scope so the
/// panic's location is recorded and the default panic message stays quiet.
pub fn run_cops(
    cops: &[&dyn Cop],
    result: &ruby_prism::ParseResult<'_>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) -> Vec<CopPanic> {
    run_cops_with_corrections(cops, result, file, out, true)
}

/// Pipeline entry point: ordinary linting does not request correction-only work.
pub(crate) fn run_cops_with_corrections(
    cops: &[&dyn Cop],
    result: &ruby_prism::ParseResult<'_>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
    corrections_requested: bool,
) -> Vec<CopPanic> {
    let first_offense = out.len();
    let mut live = Vec::with_capacity(cops.len());
    let mut panics = Vec::new();
    for &cop in cops {
        match catch(|| {
            cop.check_source(file, out);
            cop.check_parse(result, file, out);
        }) {
            Ok(()) => live.push(Running {
                id: live.len(),
                cop,
            }),
            Err(panic) => panics.push(CopPanic {
                cop: cop.name(),
                position: None,
                panic,
            }),
        }
    }
    let leaving = live
        .iter()
        .copied()
        .filter(|running| running.cop.wants_leave_node())
        .collect();
    let mut walker = CopWalker {
        cops: live,
        leaving,
        by_kind: Vec::new(),
        ancestors: Vec::new(),
        file,
        out,
        panics,
        corrections_requested,
        require_order_reports: RefCell::default(),
    };
    walk::walk(&mut walker, &result.node());
    let panics = walker.panics;
    guard_corrections(cops, &mut out[first_offense..], corrections_requested);
    panics
}

/// What ordinary and post-investigation batches share: the corrections that
/// incompatible cops withhold. As `rubocop -a` does, an edit is applied
/// whatever the program could observe of it.
fn guard_corrections(cops: &[&dyn Cop], emitted: &mut [Offense], corrections_requested: bool) {
    if corrections_requested {
        for cop in cops {
            if cop
                .autocorrect_incompatible_with()
                .iter()
                .any(|name| cops.iter().any(|other| other.name() == *name))
            {
                for offense in emitted
                    .iter_mut()
                    .filter(|offense| offense.cop == cop.name())
                {
                    offense.correction = None;
                }
            }
        }
    }
    if !corrections_requested {
        for offense in emitted {
            offense.correction = None;
        }
    }
}

/// The pipeline owns lifecycle ordering and immutable evidence. Keep post
/// outputs separate until all callbacks finish, so registration order cannot
/// make a peer's post finding appear in another callback's raw inputs.
pub(crate) fn run_post_investigation(
    cops: &[&dyn Cop],
    file: &SourceFile,
    cx: &PostInvestigation<'_>,
    out: &mut Vec<Offense>,
) -> Vec<CopPanic> {
    let first = out.len();
    let mut panics = Vec::new();
    for cop in cops.iter().filter(|cop| cop.wants_post_investigation()) {
        if !cx.cops.iter().any(|c| c.name == cop.name() && c.completed) {
            continue;
        }
        if let Err(panic) = catch(|| cop.check_post_investigation(cx, file, out)) {
            panics.push(CopPanic {
                cop: cop.name(),
                position: None,
                panic,
            });
        }
    }
    guard_corrections(cops, &mut out[first..], cx.corrections_requested);
    panics
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cops::testing::preorder_kinds;

    #[test]
    fn registry_contains_lint_debugger() {
        let registry = Registry::new();
        assert!(registry.has_cop("Lint/Debugger"));
        assert!(registry.has_cop("Style/Documentation"));
        assert!(!registry.has_cop("Lint"));
    }

    #[test]
    fn registry_contains_duplicate_and_identity_cops() {
        let registry = Registry::new();
        for name in [
            "Lint/BinaryOperatorWithIdenticalOperands",
            "Lint/DuplicateCaseCondition",
            "Lint/DuplicateElsifCondition",
            "Lint/DuplicateHashKey",
            "Lint/DuplicateMethods",
            "Lint/DuplicateRescueException",
            "Lint/DuplicateSetElement",
            "Lint/SelfAssignment",
        ] {
            assert!(registry.has_cop(name), "{name} is registered");
        }
    }

    #[test]
    fn registry_contains_empty_body_void_and_reachability_cops() {
        let registry = Registry::new();
        for name in [
            "Lint/EmptyConditionalBody",
            "Lint/EmptyEnsure",
            "Lint/EmptyInterpolation",
            "Lint/EmptyWhen",
            "Lint/EnsureReturn",
            "Lint/NonLocalExitFromIterator",
            "Lint/UnreachableCode",
            "Lint/Void",
        ] {
            assert!(registry.has_cop(name), "{name} is registered");
        }
    }

    #[test]
    fn registry_contains_ambiguity_cops() {
        let registry = Registry::new();
        for name in [
            "Lint/AmbiguousBlockAssociation",
            "Lint/AmbiguousOperator",
            "Lint/AmbiguousRange",
            "Lint/AmbiguousRegexpLiteral",
            "Lint/AssignmentInCondition",
        ] {
            assert!(registry.has_cop(name), "{name} is registered");
        }
    }

    #[test]
    fn registry_contains_exception_and_misc_cops() {
        let registry = Registry::new();
        for name in [
            "Lint/DeprecatedClassMethods",
            "Lint/FormatParameterMismatch",
            "Lint/InheritException",
            "Lint/NestedMethodDefinition",
            "Lint/OutOfRangeRegexpRef",
            "Lint/RaiseException",
            "Lint/RescueException",
            "Lint/SuppressedException",
        ] {
            assert!(registry.has_cop(name), "{name} is registered");
        }
    }

    #[test]
    fn registry_contains_literal_require_and_assignment_cops() {
        let registry = Registry::new();
        for name in [
            "Lint/LiteralAsCondition",
            "Lint/LiteralInInterpolation",
            "Lint/RedundantRequireStatement",
            "Lint/StructNewOverride",
            "Lint/UselessAssignment",
        ] {
            assert!(registry.has_cop(name), "{name} is registered");
        }
    }

    #[test]
    fn registry_contains_security_cops() {
        let registry = Registry::new();
        for name in [
            "Security/CompoundHash",
            "Security/Eval",
            "Security/IoMethods",
            "Security/JSONLoad",
            "Security/MarshalLoad",
            "Security/Open",
            "Security/YAMLLoad",
        ] {
            assert!(registry.has_cop(name), "{name} is registered");
        }
        let config = ResolvedConfig::default();
        let active = select_active(&registry, &config, &["Security".into()], &[]);
        assert_eq!(
            active.len(),
            6,
            "YAMLLoad is registered but inactive at Ruby 3.4"
        );
        assert!(active.iter().all(|cop| cop.name() != "Security/YAMLLoad"));
    }

    #[test]
    fn registry_contains_style_cops() {
        let registry = Registry::new();
        let style = [
            "Style/EmptyMethod",
            "Style/RedundantConstantBase",
            "Style/RedundantFreeze",
            "Style/RedundantInitialize",
            "Style/RedundantSelf",
            "Style/RedundantFetchBlock",
            "Style/SafeNavigation",
            "Style/SafeNavigationChainLength",
            "Style/IpAddresses",
            "Style/SingleArgumentDig",
            "Style/DigChain",
            "Style/EmptyLiteral",
            "Style/EmptyClassDefinition",
            "Style/AndOr",
            "Style/DoubleNegation",
            "Style/CaseEquality",
            "Style/CombinableDefined",
            "Style/CombinableLoops",
            "Style/For",
            "Style/InfiniteLoop",
            "Style/InPatternThen",
            "Style/MultilineInPatternThen",
            "Style/TrailingCommaInArrayLiteral",
            "Style/TrailingCommaInHashLiteral",
            "Style/TrailingCommaInBlockArgs",
            "Style/TrailingCommaInArguments",
            "Style/ItAssignment",
            "Style/ItBlockParameter",
            "Style/NumberedParameters",
            "Style/NumberedParametersLimit",
            "Style/NilLambda",
            "Style/AccessModifierDeclarations",
            "Style/AccessorGrouping",
            "Style/Attr",
            "Style/BisectedAttrAccessor",
            "Style/TrivialAccessors",
            "Style/ArgumentsForwarding",
            "Style/SuperArguments",
            "Style/SuperWithArgsParentheses",
            "Style/KeywordParametersOrder",
            "Style/KeywordArgumentsMerging",
            "Style/ExplicitBlockArgument",
            "Style/RedundantArgument",
            "Style/OptionalArguments",
            "Style/OptionalBooleanParameter",
            "Style/OptionHash",
            "Style/AsciiComments",
            "Style/AutoResourceCleanup",
            "Style/GlobalStdStream",
            "Style/GlobalVars",
            "Style/SpecialGlobalVars",
            "Style/PerlBackrefs",
            "Style/PredicateWithKind",
            "Style/StderrPuts",
            "Style/ClassAndModuleChildren",
            "Style/ClassCheck",
            "Style/ClassVars",
            "Style/ColonMethodCall",
            "Style/ColonMethodDefinition",
            "Style/DefWithParentheses",
            "Style/MethodDefParentheses",
            "Style/AmbiguousEndlessMethodDefinition",
            "Style/Alias",
            "Style/OneClassPerFile",
            "Style/StaticClass",
            "Style/OpenStructUse",
            "Style/ClassMethods",
            "Style/ClassMethodsDefinitions",
            "Style/ModuleFunction",
            "Style/MixinUsage",
            "Style/MixinGrouping",
            "Style/ModuleMemberExistenceCheck",
            "Style/ConstantVisibility",
            "Style/MutableConstant",
            "Style/DataInheritance",
            "Style/StructInheritance",
            "Style/RedundantStructKeywordInit",
            "Style/ClassEqualityComparison",
            "Style/MissingRespondToMissing",
            "Style/EvalWithLocation",
            "Style/Send",
            "Style/SendWithLiteralMethodName",
            "Style/BlockComments",
            "Style/CommentAnnotation",
            "Style/CommentedKeyword",
            "Style/Copyright",
            "Style/BlockDelimiters",
            "Style/MultilineBlockChain",
            "Style/SingleLineDoEndBlock",
            "Style/SingleLineBlockParams",
            "Style/Lambda",
            "Style/LambdaCall",
            "Style/Proc",
            "Style/StabbyLambdaParentheses",
            "Style/EmptyBlockParameter",
            "Style/EmptyLambdaParameter",
            "Style/Dir",
            "Style/DirEmpty",
            "Style/NestedFileDirname",
            "Style/ExpandPathArguments",
            "Style/RedundantCurrentDirectoryInPath",
            "Style/RedundantFileExtensionInRequire",
            "Style/RequireOrder",
            "Style/EnvHome",
            "Style/DateTime",
            "Style/FetchEnvVar",
            "Style/TimeNow",
            "Style/RandomWithOffset",
            "Style/FileEmpty",
            "Style/FileNull",
            "Style/FileOpen",
            "Style/FileRead",
            "Style/FileTouch",
            "Style/FileWrite",
            "Style/YAMLFileRead",
            "Style/DirectiveScope",
            "Style/DisableCopsWithinSourceCodeDirective",
            "Style/Documentation",
            "Style/DocumentationMethod",
            "Style/DocumentDynamicEvalDefinition",
            "Style/Encoding",
            "Style/InlineComment",
            "Style/MagicCommentFormat",
            "Style/EndlessMethod",
            "Style/SingleLineMethods",
            "Style/TrailingUnderscoreVariable",
            "Style/TrailingMethodEndStatement",
            "Style/TrailingBodyOnClass",
            "Style/TrailingBodyOnModule",
            "Style/TrailingBodyOnMethodDefinition",
            "Style/TopLevelMethodDefinition",
            "Style/MultilineMethodSignature",
            "Style/FrozenStringLiteralComment",
            "Style/GuardClause",
            "Style/Next",
            "Style/ConditionalAssignment",
            "Style/HashSyntax",
            "Style/IfUnlessModifier",
            "Style/MethodCallWithArgsParentheses",
            "Style/MethodCallWithoutArgsParentheses",
            "Style/MethodCalledOnDoEndBlock",
            "Style/OperatorMethodCall",
            "Style/NestedParenthesizedCalls",
            "Style/RedundantCondition",
            "Style/RedundantConditional",
            "Style/RedundantParentheses",
            "Style/RedundantReturn",
            "Style/Semicolon",
            "Style/SignalException",
            "Style/ImplicitRuntimeError",
            "Style/RaiseArgs",
            "Style/RedundantException",
            "Style/RescueModifier",
            "Style/BeginBlock",
            "Style/EndBlock",
            "Style/RedundantBegin",
            "Style/RescueStandardError",
            "Style/ReturnNil",
            "Style/ReturnNilInPredicateMethodDefinition",
            "Style/SoleNestedConditional",
            "Style/LineEndConcatenation",
            "Style/RedundantLineContinuation",
            "Style/RedundantFormat",
            "Style/StringConcatenation",
            "Style/FormatString",
            "Style/FormatStringToken",
            "Style/CommandLiteral",
            "Style/PercentLiteralDelimiters",
            "Style/PercentQLiterals",
            "Style/RedundantPercentQ",
            "Style/RedundantCapitalW",
            "Style/RedundantStringEscape",
            "Style/RedundantHeredocDelimiterQuotes",
            "Style/EmptyHeredoc",
            "Style/CharacterLiteral",
            "Style/BarePercentLiterals",
            "Style/QuotedSymbols",
            "Style/SymbolLiteral",
            "Style/VariableInterpolation",
            "Style/EmptyStringInsideInterpolation",
            "Style/RedundantInterpolation",
            "Style/RedundantInterpolationUnfreeze",
            "Style/StringLiterals",
            "Style/StringLiteralsInInterpolation",
            "Style/StringChars",
            "Style/StringMethods",
            "Style/Strip",
            "Style/UnpackFirst",
            "Style/ArrayCoercion",
            "Style/ArrayFirstLast",
            "Style/ArrayIntersect",
            "Style/ArrayIntersectWithSingleElement",
            "Style/ArrayJoin",
            "Style/ConcatArrayLiterals",
            "Style/NegativeArrayIndex",
            "Style/RedundantArrayConstructor",
            "Style/RedundantArrayFlatten",
            "Style/SlicingWithRange",
            "Style/WordArray",
            "Style/SymbolArray",
            "Style/Sample",
            "Style/HashAsLastArrayItem",
            "Style/HashConversion",
            "Style/HashEachMethods",
            "Style/HashExcept",
            "Style/HashSlice",
            "Style/RedundantDoubleSplatHashBraces",
            "Style/StringHashKeys",
            "Style/HashTransformKeys",
            "Style/HashTransformValues",
            "Style/PreferredHashMethods",
            "Style/HashFetchChain",
            "Style/HashLikeCase",
            "Style/CaseLikeIf",
            "Style/IdenticalConditionalBranches",
            "Style/IfInsideElse",
            "Style/MultilineMemoization",
            "Style/OrAssignment",
            "Style/SelfAssignment",
            "Style/SwapValues",
            "Style/ParallelAssignment",
            "Style/RedundantAssignment",
            "Style/RedundantSelfAssignment",
            "Style/RedundantSelfAssignmentBranch",
            "Style/IfUnlessModifierOfIfUnless",
            "Style/IfWithBooleanLiteralBranches",
            "Style/IfWithSemicolon",
            "Style/MissingElse",
            "Style/EmptyElse",
            "Style/MultilineIfModifier",
            "Style/MultilineIfThen",
            "Style/MultilineTernaryOperator",
            "Style/MultilineWhenThen",
            "Style/UnlessElse",
            "Style/WhenThen",
            "Style/EmptyCaseCondition",
            "Style/OneLineConditional",
            "Style/ParenthesesAroundCondition",
            "Style/NestedModifier",
            "Style/NestedTernaryOperator",
            "Style/TernaryParentheses",
            "Style/NegatedIf",
            "Style/NegatedUnless",
            "Style/NegatedWhile",
            "Style/WhileUntilDo",
            "Style/WhileUntilModifier",
            "Style/NegatedIfElseCondition",
            "Style/InvertibleUnlessCondition",
            "Style/HashLookupMethod",
            "Style/CollectionCompact",
            "Style/CollectionMethods",
            "Style/CollectionQuerying",
            "Style/EachForSimpleLoop",
            "Style/EachWithObject",
            "Style/MapCompactWithConditionalBlock",
            "Style/MapIntoArray",
            "Style/MapJoin",
            "Style/MapToHash",
            "Style/MapToSet",
            "Style/PartitionInsteadOfDoubleSelect",
            "Style/ReduceToHash",
            "Style/RedundantEach",
            "Style/RedundantFilterChain",
            "Style/SelectByKind",
            "Style/SelectByRange",
            "Style/SelectByRegexp",
            "Style/ExactRegexpMatch",
            "Style/RegexpLiteral",
            "Style/RedundantRegexpEscape",
            "Style/RedundantRegexpArgument",
            "Style/RedundantRegexpCharacterClass",
            "Style/RedundantRegexpConstructor",
            "Style/SymbolProc",
            "Style/TallyMethod",
            "Style/ObjectThen",
            "Style/ComparableClamp",
            "Style/BitwisePredicate",
            "Style/EvenOdd",
            "Style/ExponentialNotation",
            "Style/FloatDivision",
            "Style/NumericLiteralPrefix",
            "Style/NumericLiterals",
            "Style/NumericPredicate",
            "Style/ZeroLengthPredicate",
            "Style/MinMax",
            "Style/MinMaxComparison",
            "Style/InverseMethods",
            "Style/NilComparison",
            "Style/NonNilCheck",
            "Style/Not",
            "Style/UnlessLogicalOperators",
            "Style/YodaCondition",
            "Style/YodaExpression",
            "Style/MultipleComparison",
            "Style/RedundantMinMaxBy",
            "Style/RedundantSort",
            "Style/RedundantSortBy",
            "Style/ComparableBetween",
            "Style/ReverseFind",
        ];
        for name in style {
            assert!(registry.has_cop(name), "{name} is registered");
        }
        // No style parameters: only `Layout/LineLength` takes a setting.
        for name in style {
            assert_eq!(
                registry.cop(name).and_then(Cop::max_setting),
                None,
                "{name}"
            );
        }
        let config = ResolvedConfig::default();
        let active = select_active(&registry, &config, &["Style".into()], &[]);
        assert_eq!(
            active.len(),
            style.len() - 1,
            "--only Style force-enables all applicable cops"
        );
        assert!(!active.iter().any(|c| c.name() == "Style/ReverseFind"));
    }

    #[test]
    fn registry_contains_naming_cops() {
        let registry = Registry::new();
        let naming = [
            "Naming/ClassAndModuleCamelCase",
            "Naming/ConstantName",
            "Naming/MethodName",
            "Naming/AsciiIdentifiers",
            "Naming/BlockParameterName",
            "Naming/VariableNumber",
            "Naming/AccessorMethodName",
            "Naming/PredicatePrefix",
            "Naming/BinaryOperatorParameterName",
            "Naming/BlockForwarding",
            "Naming/HeredocDelimiterCase",
            "Naming/HeredocDelimiterNaming",
            "Naming/MemoizedInstanceVariableName",
            "Naming/PredicateMethod",
            "Naming/InclusiveLanguage",
            "Naming/FileName",
            "Naming/RescuedExceptionsVariableName",
            "Naming/MethodParameterName",
            "Naming/VariableName",
        ];
        for name in naming {
            assert!(registry.has_cop(name), "{name} is registered");
            assert_eq!(
                registry.cop(name).and_then(Cop::max_setting),
                None,
                "{name}"
            );
        }
        let config = ResolvedConfig::default();
        let defaults = select_active(&registry, &config, &[], &[]);
        let enabled: Vec<&str> = defaults
            .iter()
            .map(|cop| cop.name())
            .filter(|name| name.starts_with("Naming/"))
            .collect();
        assert_eq!(
            enabled.len(),
            18,
            "eighteen are on by default (InclusiveLanguage opt-in): {enabled:?}"
        );
        let active = select_active(&registry, &config, &["Naming".into()], &[]);
        assert_eq!(active.len(), 19, "--only Naming selects all nineteen");
    }

    #[test]
    fn registry_contains_the_ten_cops_with_max() {
        let registry = Registry::new();
        assert!(registry.has_cop("Layout/LineLength"));
        assert!(registry.has_cop("Layout/TrailingWhitespace"));
        for cop in registry.cops() {
            let name = cop.name();
            // Metrics/CollectionLiteralLength has a `LengthThreshold`, no `Max`.
            let limited = name == "Layout/LineLength"
                || (name.starts_with("Metrics/") && name != "Metrics/CollectionLiteralLength");
            let expected = limited.then_some("max");
            assert_eq!(cop.max_setting(), expected, "{name}");
            // A cop declares settings exactly when it can be built with them.
            let values = Values::default().with("max", settings::Value::Count(100));
            let configured = cop.configure(&values);
            assert_eq!(configured.is_some(), !cop.settings().is_empty(), "{name}");
            if let Some(configured) = configured {
                assert_eq!(configured.name(), name);
            }
        }
        let config = ResolvedConfig::default();
        let defaults = select_active(&registry, &config, &[], &[]);
        assert!(defaults.iter().any(|cop| cop.name() == "Layout/LineLength"));
        let active = select_active(&registry, &config, &["Layout".into()], &[]);
        assert!(
            defaults
                .iter()
                .any(|cop| cop.name() == "Layout/TrailingWhitespace")
        );
        // Opt-in MultilineAssignmentLayout and
        // EmptyLineAfterMultilineCondition need their exact names.
        assert_eq!(active.len(), 87);
        for name in [
            "Layout/TrailingEmptyLines",
            "Layout/EmptyLines",
            "Layout/EmptyLineBetweenDefs",
            "Layout/EmptyLinesAroundClassBody",
            "Layout/EmptyLinesAroundModuleBody",
            "Layout/EmptyLinesAroundMethodBody",
            "Layout/SpaceAfterComma",
            "Layout/SpaceBeforeComma",
            "Layout/SpaceAroundOperators",
            "Layout/SpaceAroundEqualsInParameterDefault",
            "Layout/SpaceAfterSemicolon",
            "Layout/SpaceInsideParens",
            "Layout/SpaceInsideArrayLiteralBrackets",
            "Layout/SpaceInsideReferenceBrackets",
            "Layout/SpaceInsideHashLiteralBraces",
            "Layout/SpaceInsideBlockBraces",
            "Layout/IndentationStyle",
            "Layout/IndentationWidth",
            "Layout/EndAlignment",
            "Layout/AccessModifierIndentation",
            "Layout/AssignmentIndentation",
            "Layout/ArgumentAlignment",
            "Layout/ArrayAlignment",
            "Layout/HashAlignment",
            "Layout/ParameterAlignment",
            "Layout/FirstParameterIndentation",
            "Layout/BeginEndAlignment",
            "Layout/DefEndAlignment",
            "Layout/BlockAlignment",
            "Layout/ClosingParenthesisIndentation",
            "Layout/FirstArgumentIndentation",
            "Layout/FirstArrayElementIndentation",
            "Layout/FirstHashElementIndentation",
            "Layout/CaseIndentation",
            "Layout/ElseAlignment",
            "Layout/RescueEnsureAlignment",
            "Layout/HeredocIndentation",
            "Layout/ClosingHeredocIndentation",
            "Layout/CommentIndentation",
            "Layout/InitialIndentation",
            "Layout/IndentationConsistency",
            "Layout/MultilineMethodCallIndentation",
            "Layout/MultilineOperationIndentation",
            "Layout/DotPosition",
            "Layout/ConditionPosition",
            "Layout/SpaceBeforeComment",
            "Layout/LeadingCommentSpace",
            "Layout/EmptyComment",
            "Layout/LeadingEmptyLines",
            "Layout/EmptyLineAfterMagicComment",
            "Layout/EmptyLineAfterGuardClause",
            "Layout/EmptyLinesAfterModuleInclusion",
            "Layout/EmptyLinesAroundAttributeAccessor",
            "Layout/EmptyLinesAroundAccessModifier",
            "Layout/EmptyLinesAroundBeginBody",
            "Layout/EmptyLinesAroundBlockBody",
            "Layout/EmptyLinesAroundExceptionHandlingKeywords",
            "Layout/EmptyLinesAroundArguments",
            "Layout/ExtraSpacing",
            "Layout/SpaceBeforeFirstArg",
            "Layout/SpaceBeforeSemicolon",
        ] {
            let cop = defaults.iter().find(|cop| cop.name() == name).unwrap();
            assert!(cop.safe_fixes(), "{name}");
            assert!(active.iter().any(|cop| cop.name() == name));
        }
    }

    #[test]
    fn frozen_string_literal_comment_is_off_unless_enabled() {
        let registry = Registry::new();
        let names = |config: &ResolvedConfig| -> Vec<&'static str> {
            select_active(&registry, config, &[], &[])
                .iter()
                .map(|cop| cop.name())
                .filter(|name| name.starts_with("Style/"))
                .collect()
        };
        let defaults = names(&ResolvedConfig::default());
        assert_eq!(defaults.len(), 263, "{defaults:?}");
        assert!(defaults.contains(&"Style/SafeNavigation"));
        assert!(defaults.contains(&"Style/SafeNavigationChainLength"));
        assert!(defaults.contains(&"Style/SingleArgumentDig"));
        assert!(defaults.contains(&"Style/DigChain"));
        assert!(defaults.contains(&"Style/EmptyMethod"));
        assert!(defaults.contains(&"Style/RedundantConstantBase"));
        assert!(defaults.contains(&"Style/RedundantFetchBlock"));
        assert!(defaults.contains(&"Style/EmptyLiteral"));
        assert!(defaults.contains(&"Style/EmptyClassDefinition"));
        assert!(defaults.contains(&"Style/BeginBlock"));
        assert!(defaults.contains(&"Style/EndBlock"));
        assert!(defaults.contains(&"Style/RedundantBegin"));
        assert!(defaults.contains(&"Style/RescueModifier"));
        assert!(defaults.contains(&"Style/RescueStandardError"));
        assert!(!defaults.contains(&"Style/ReturnNil"));
        assert!(defaults.contains(&"Style/ReturnNilInPredicateMethodDefinition"));
        assert!(defaults.contains(&"Style/RaiseArgs"));
        assert!(defaults.contains(&"Style/RedundantException"));
        assert!(!defaults.contains(&"Style/ImplicitRuntimeError"));
        assert!(defaults.contains(&"Style/SelfAssignment"));
        assert!(defaults.contains(&"Style/SwapValues"));
        assert!(defaults.contains(&"Style/RedundantAssignment"));
        assert!(defaults.contains(&"Style/RedundantSelfAssignment"));
        assert!(defaults.contains(&"Style/RedundantSelfAssignmentBranch"));
        assert!(defaults.contains(&"Style/MultilineMemoization"));
        assert!(defaults.contains(&"Style/OrAssignment"));
        assert!(defaults.contains(&"Style/CombinableLoops"));
        assert!(defaults.contains(&"Style/For"));
        assert!(defaults.contains(&"Style/InfiniteLoop"));
        assert!(defaults.contains(&"Style/InPatternThen"));
        assert!(defaults.contains(&"Style/MultilineInPatternThen"));
        for name in [
            "Style/NegatedWhile",
            "Style/WhileUntilDo",
            "Style/WhileUntilModifier",
        ] {
            assert!(defaults.contains(&name));
        }
        assert!(defaults.contains(&"Style/EmptyElse"));
        for name in [
            "Style/UnlessElse",
            "Style/WhenThen",
            "Style/EmptyCaseCondition",
        ] {
            assert!(defaults.contains(&name));
        }
        assert!(defaults.contains(&"Style/TernaryParentheses"));
        assert!(defaults.contains(&"Style/OneLineConditional"));
        assert!(defaults.contains(&"Style/ParenthesesAroundCondition"));
        assert!(defaults.contains(&"Style/NestedModifier"));
        assert!(defaults.contains(&"Style/NestedTernaryOperator"));
        assert!(defaults.contains(&"Style/MultilineIfModifier"));
        assert!(!defaults.contains(&"Style/MissingElse"));
        assert!(defaults.contains(&"Style/IfWithSemicolon"));
        assert!(!defaults.contains(&"Style/InvertibleUnlessCondition"));
        assert!(defaults.contains(&"Style/IfUnlessModifierOfIfUnless"));
        assert!(defaults.contains(&"Style/IfWithBooleanLiteralBranches"));
        assert!(defaults.contains(&"Style/IdenticalConditionalBranches"));
        assert!(defaults.contains(&"Style/CaseLikeIf"));
        for cop in [
            "Style/RedundantRegexpArgument",
            "Style/RedundantRegexpCharacterClass",
            "Style/RedundantRegexpConstructor",
        ] {
            assert!(defaults.contains(&cop));
        }
        assert!(defaults.contains(&"Style/RedundantRegexpEscape"));
        assert!(defaults.contains(&"Style/RegexpLiteral"));
        assert!(defaults.contains(&"Style/ExactRegexpMatch"));
        assert!(defaults.contains(&"Style/YodaCondition"));
        assert!(!defaults.contains(&"Style/YodaExpression"));
        assert!(!defaults.contains(&"Style/UnlessLogicalOperators"));
        assert!(defaults.contains(&"Style/ZeroLengthPredicate"));
        for cop in [
            "Style/FloatDivision",
            "Style/NumericLiteralPrefix",
            "Style/NumericLiterals",
            "Style/ComparableClamp",
            "Style/MinMax",
            "Style/MinMaxComparison",
            "Style/RedundantMinMaxBy",
        ] {
            assert!(defaults.contains(&cop));
        }
        assert!(defaults.contains(&"Style/SelectByRange"));
        assert!(defaults.contains(&"Style/SelectByRegexp"));
        assert!(defaults.contains(&"Style/SymbolProc"));
        assert!(defaults.contains(&"Style/TallyMethod"));
        for name in ["Style/MapIntoArray", "Style/MapJoin", "Style/MapToHash"] {
            assert!(defaults.contains(&name));
        }
        assert!(defaults.contains(&"Style/CollectionCompact"));
        assert!(!defaults.contains(&"Style/CollectionMethods"));
        assert!(defaults.contains(&"Style/CollectionQuerying"));
        assert!(defaults.contains(&"Style/RedundantDoubleSplatHashBraces"));
        assert!(!defaults.contains(&"Style/StringHashKeys"));
        for name in [
            "Style/HashTransformKeys",
            "Style/HashTransformValues",
            "Style/PreferredHashMethods",
        ] {
            assert!(defaults.contains(&name));
        }
        assert!(defaults.contains(&"Style/HashFetchChain"));
        assert!(defaults.contains(&"Style/HashLikeCase"));
        assert!(!defaults.contains(&"Style/HashLookupMethod"));
        assert!(defaults.contains(&"Style/HashExcept"));
        assert!(defaults.contains(&"Style/HashSlice"));
        assert!(defaults.contains(&"Style/HashEachMethods"));
        assert!(defaults.contains(&"Style/SymbolArray"));
        assert!(defaults.contains(&"Style/Sample"));
        assert!(defaults.contains(&"Style/NegativeArrayIndex"));
        assert!(defaults.contains(&"Style/RedundantArrayConstructor"));
        assert!(defaults.contains(&"Style/RedundantArrayFlatten"));
        assert!(defaults.contains(&"Style/ArrayIntersect"));
        assert!(!defaults.contains(&"Style/ArrayCoercion"));
        assert!(!defaults.contains(&"Style/ArrayFirstLast"));
        assert!(defaults.contains(&"Style/UnpackFirst"));
        assert!(defaults.contains(&"Style/StringChars"));
        assert!(defaults.contains(&"Style/Strip"));
        assert!(!defaults.contains(&"Style/StringMethods"));
        assert!(defaults.contains(&"Style/TrailingCommaInArrayLiteral"));
        assert!(defaults.contains(&"Style/TrailingCommaInHashLiteral"));
        assert!(!defaults.contains(&"Style/TrailingCommaInBlockArgs"));
        assert!(!defaults.contains(&"Style/SingleLineBlockParams"));
        assert!(defaults.contains(&"Style/SingleLineDoEndBlock"));
        assert!(defaults.contains(&"Style/MultilineBlockChain"));
        for name in [
            "Style/LineEndConcatenation",
            "Style/RedundantLineContinuation",
            "Style/RedundantFormat",
            "Style/StringConcatenation",
            "Style/FormatString",
            "Style/FormatStringToken",
            "Style/CommandLiteral",
            "Style/BarePercentLiterals",
            "Style/PercentLiteralDelimiters",
            "Style/PercentQLiterals",
            "Style/RedundantPercentQ",
            "Style/RedundantCapitalW",
            "Style/RedundantStringEscape",
            "Style/RedundantHeredocDelimiterQuotes",
            "Style/EmptyHeredoc",
            "Style/CharacterLiteral",
            "Style/QuotedSymbols",
            "Style/SymbolLiteral",
            "Style/VariableInterpolation",
            "Style/EmptyStringInsideInterpolation",
            "Style/RedundantInterpolation",
            "Style/RedundantInterpolationUnfreeze",
            "Style/TrailingCommaInArguments",
            "Style/Lambda",
            "Style/LambdaCall",
            "Style/Proc",
            "Style/StabbyLambdaParentheses",
            "Style/EmptyBlockParameter",
            "Style/EmptyLambdaParameter",
            "Style/KeywordArgumentsMerging",
            "Style/ExplicitBlockArgument",
            "Style/RedundantArgument",
            "Style/NilLambda",
            "Style/NumberedParametersLimit",
        ] {
            assert!(defaults.contains(&name));
        }
        assert!(defaults.contains(&"Style/OptionalArguments"));
        assert!(defaults.contains(&"Style/OptionalBooleanParameter"));
        assert!(!defaults.contains(&"Style/OptionHash"));
        assert!(defaults.contains(&"Style/SuperArguments"));
        assert!(defaults.contains(&"Style/SuperWithArgsParentheses"));
        assert!(defaults.contains(&"Style/KeywordParametersOrder"));
        assert!(defaults.contains(&"Style/TrailingBodyOnMethodDefinition"));
        assert!(defaults.contains(&"Style/SingleLineMethods"));
        assert!(!defaults.contains(&"Style/TopLevelMethodDefinition"));
        assert!(!defaults.contains(&"Style/MultilineMethodSignature"));
        assert!(defaults.contains(&"Style/MethodCallWithoutArgsParentheses"));
        assert!(defaults.contains(&"Style/OperatorMethodCall"));
        assert!(defaults.contains(&"Style/NestedParenthesizedCalls"));
        assert!(defaults.contains(&"Style/ColonMethodCall"));
        assert!(defaults.contains(&"Style/Alias"));
        assert!(defaults.contains(&"Style/ColonMethodDefinition"));
        assert!(defaults.contains(&"Style/DefWithParentheses"));
        assert!(defaults.contains(&"Style/MethodDefParentheses"));
        assert!(defaults.contains(&"Style/AmbiguousEndlessMethodDefinition"));
        assert!(defaults.contains(&"Style/AccessorGrouping"));
        assert!(defaults.contains(&"Style/Attr"));
        assert!(defaults.contains(&"Style/BisectedAttrAccessor"));
        assert!(defaults.contains(&"Style/TrivialAccessors"));
        assert!(defaults.contains(&"Style/AccessModifierDeclarations"));
        assert!(defaults.contains(&"Style/OpenStructUse"));
        assert!(defaults.contains(&"Style/ClassMethods"));
        assert!(!defaults.contains(&"Style/ClassMethodsDefinitions"));
        assert!(defaults.contains(&"Style/DataInheritance"));
        assert!(defaults.contains(&"Style/StructInheritance"));
        assert!(!defaults.contains(&"Style/RedundantStructKeywordInit"));
        assert!(defaults.contains(&"Style/MutableConstant"));
        assert!(defaults.contains(&"Style/ModuleMemberExistenceCheck"));
        assert!(!defaults.contains(&"Style/ConstantVisibility"));
        assert!(defaults.contains(&"Style/ModuleFunction"));
        assert!(defaults.contains(&"Style/MixinUsage"));
        assert!(defaults.contains(&"Style/MixinGrouping"));
        assert!(defaults.contains(&"Style/PredicateWithKind"));
        assert!(defaults.contains(&"Style/ClassAndModuleChildren"));
        assert!(defaults.contains(&"Style/ClassCheck"));
        assert!(defaults.contains(&"Style/ClassVars"));
        assert!(defaults.contains(&"Style/OneClassPerFile"));
        assert!(!defaults.contains(&"Style/StaticClass"));
        assert!(defaults.contains(&"Style/ClassEqualityComparison"));
        assert!(defaults.contains(&"Style/MissingRespondToMissing"));
        assert!(defaults.contains(&"Style/EvalWithLocation"));
        assert!(defaults.contains(&"Style/SendWithLiteralMethodName"));
        assert!(!defaults.contains(&"Style/Send"));
        assert!(!defaults.contains(&"Style/AutoResourceCleanup"));
        assert!(defaults.contains(&"Style/GlobalStdStream"));
        assert!(defaults.contains(&"Style/StderrPuts"));
        assert!(defaults.contains(&"Style/EnvHome"));
        assert!(defaults.contains(&"Style/FetchEnvVar"));
        assert!(defaults.contains(&"Style/TimeNow"));
        assert!(defaults.contains(&"Style/RandomWithOffset"));
        assert!(!defaults.contains(&"Style/DateTime"));
        for name in ["Style/FileNull", "Style/FileOpen", "Style/FileRead"] {
            assert!(defaults.contains(&name));
        }
        assert!(!defaults.contains(&"Style/FrozenStringLiteralComment"));

        let raw: crate::config::RawConfig =
            toml::from_str("[cops.\"Style/FrozenStringLiteralComment\"]\nenabled = true\n")
                .unwrap();
        let config = ResolvedConfig::resolve(raw, None, &registry).unwrap();
        assert!(names(&config).contains(&"Style/FrozenStringLiteralComment"));

        let raw: crate::config::RawConfig =
            toml::from_str("[cops.\"Style\"]\nenabled = true\n").unwrap();
        let config = ResolvedConfig::resolve(raw, None, &registry).unwrap();
        assert!(names(&config).contains(&"Style/FrozenStringLiteralComment"));
    }

    #[test]
    fn no_cops_selected_with_unknown_only() {
        let registry = Registry::new();
        let config = ResolvedConfig::default();
        let active = select_active(&registry, &config, &["Style/Unknown".into()], &[]);
        assert!(active.is_empty());
    }

    #[test]
    fn only_force_enables_and_except_wins() {
        let registry = Registry::new();
        let config = ResolvedConfig::default();
        let active = select_active(
            &registry,
            &config,
            &["Lint/Debugger".into()],
            &["Lint/Debugger".into()],
        );
        assert!(active.is_empty());
    }

    #[test]
    fn department_toggle_in_config_disables_cop() {
        let registry = Registry::new();
        let raw: crate::config::RawConfig =
            toml::from_str("[cops.\"Lint\"]\nenabled = false").unwrap();
        let config = ResolvedConfig::resolve(raw, None, &registry).unwrap();
        let active = select_active(&registry, &config, &[], &[]);
        assert!(!active.is_empty(), "other departments still run");
        assert!(active.iter().all(|cop| !cop.name().starts_with("Lint/")));
    }

    /// A cop that reports every node it is given, at the node's start, with
    /// the message `Kind start..end`.
    struct Probe;

    impl Cop for Probe {
        fn name(&self) -> &'static str {
            "Lint/Probe"
        }

        fn check_node<'pr>(
            &self,
            node: &Node<'pr>,
            _cx: &NodeContext<'_, 'pr>,
            file: &SourceFile,
            out: &mut Vec<Offense>,
        ) {
            let location = node.location();
            let (line, col) = file.line_col(location.start_offset());
            out.push(Offense {
                correction: None,
                cop: self.name(),
                severity: Severity::Error,
                message: format!(
                    "{} {}..{}",
                    kind_name(node),
                    location.start_offset(),
                    location.end_offset()
                ),
                line,
                end_line: file.last_line_of(location),
                col,
            });
        }
    }

    /// A node's kind (`CallNode`), from ruby-prism's `Debug` dump.
    fn kind_name(node: &Node<'_>) -> String {
        let dump = format!("{node:?}");
        dump[..dump.find('(').unwrap_or(dump.len())].to_string()
    }

    /// Run the walker with only the probe; one offense per dispatch, in order.
    fn probe(source: &str) -> Vec<Offense> {
        let result = ruby_prism::parse(source.as_bytes());
        assert!(
            result.errors().next().is_none(),
            "fixture must parse: {source}"
        );
        probe_tree(source)
    }

    /// Like `probe`, but also walks the tree Prism recovers from a source
    /// with syntax errors.
    fn probe_tree(source: &str) -> Vec<Offense> {
        let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
        let result = ruby_prism::parse(&file.bytes);
        let mut out = Vec::new();
        let panics = run_cops(&[&Probe], &result, &file, &mut out);
        assert!(panics.is_empty(), "{panics:?}");
        out
    }

    /// Run `cops` over `source` as the pipeline does, in a quiet scope.
    fn run_isolated(
        cops: &[&dyn Cop],
        source: &str,
    ) -> (Vec<(&'static str, u32, u32)>, Vec<CopPanic>) {
        let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
        let result = ruby_prism::parse(&file.bytes);
        let mut out = Vec::new();
        let _quiet = crate::isolation::QuietPanics::enter();
        let panics = run_cops(cops, &result, &file, &mut out);
        let offenses = out.iter().map(|o| (o.cop, o.line, o.col)).collect();
        (offenses, panics)
    }

    #[test]
    fn a_cop_panicking_on_a_node_does_not_stop_the_other_cops() {
        let source = "binding.pry\nboom\ndebugger\n";
        let orders: [&[&dyn Cop]; 2] = [
            &[&testing::PanicOnCall, &lint::Debugger],
            &[&lint::Debugger, &testing::PanicOnCall],
        ];
        for cops in orders {
            let (offenses, panics) = run_isolated(cops, source);
            assert_eq!(offenses, [("Lint/Debugger", 1, 1), ("Lint/Debugger", 3, 1)]);
            assert_eq!(panics.len(), 1, "{panics:?}");
            assert_eq!(panics[0].cop, "Lint/PanicOnCall");
            assert_eq!(panics[0].position, Some((2, 1)));
            let panic = &panics[0].panic;
            assert!(panic.message.contains("index out of bounds"), "{panic:?}");
            let location = panic.location.as_deref().unwrap_or_default();
            assert!(location.starts_with("src/cops/testing.rs:"), "{panic:?}");
        }
    }

    #[test]
    fn a_panicking_cop_keeps_earlier_offenses_and_sits_out_the_rest_of_the_file() {
        let (offenses, panics) = run_isolated(&[&testing::PanicOnCall], "mark\nboom\nmark\nboom\n");
        assert_eq!(offenses, [("Lint/PanicOnCall", 1, 1)]);
        assert_eq!(panics.len(), 1, "one report per cop and file: {panics:?}");
        assert_eq!(panics[0].position, Some((2, 1)));
    }

    #[test]
    fn several_cops_can_panic_on_the_same_node() {
        let (offenses, panics) = run_isolated(
            &[
                &testing::PanicOnCall,
                &testing::PanicOnCall,
                &lint::Debugger,
            ],
            "boom\nbinding.pry\n",
        );
        assert_eq!(offenses, [("Lint/Debugger", 2, 1)]);
        assert_eq!(panics.len(), 2, "{panics:?}");
    }

    #[test]
    fn a_cop_panicking_in_check_source_is_isolated_and_skips_the_walk() {
        let (offenses, panics) = run_isolated(
            &[&testing::PanicOnSource, &lint::Debugger],
            "mark\nboom\nbinding.pry\n",
        );
        assert_eq!(offenses, [("Lint/Debugger", 3, 1)], "no `mark` offense");
        assert_eq!(panics.len(), 1, "{panics:?}");
        assert_eq!(panics[0].cop, "Lint/PanicOnSource");
        assert_eq!(panics[0].position, None);
        assert_eq!(panics[0].panic.message, "PanicOnSource saw `boom`");

        let (offenses, panics) = run_isolated(&[&testing::PanicOnSource], "mark\n");
        assert_eq!(offenses, [("Lint/PanicOnSource", 1, 1)]);
        assert!(panics.is_empty());
    }

    #[test]
    fn a_cop_panicking_in_check_parse_is_isolated_and_skips_the_walk() {
        struct ParsePanic;
        impl Cop for ParsePanic {
            fn name(&self) -> &'static str {
                "Lint/ParsePanic"
            }
            fn check_parse(
                &self,
                _: &ruby_prism::ParseResult<'_>,
                _: &SourceFile,
                _: &mut Vec<Offense>,
            ) {
                panic!("parse hook failed");
            }
            fn check_node<'pr>(
                &self,
                _: &Node<'pr>,
                _: &NodeContext<'_, 'pr>,
                _: &SourceFile,
                _: &mut Vec<Offense>,
            ) {
                panic!("must not walk a failed cop");
            }
        }
        let (offenses, panics) = run_isolated(&[&ParsePanic, &lint::Debugger], "binding.pry");
        assert_eq!(offenses, [("Lint/Debugger", 1, 1)]);
        assert_eq!(panics.len(), 1);
        assert_eq!(panics[0].cop, "Lint/ParsePanic");
        assert_eq!(panics[0].position, None);
        assert_eq!(panics[0].panic.message, "parse hook failed");
    }

    /// Every dispatch, in order, as `Kind start..end`.
    fn dispatched(source: &str) -> Vec<String> {
        probe(source).into_iter().map(|o| o.message).collect()
    }

    /// The kinds the walker dispatches, in order.
    fn dispatched_kinds(source: &str) -> Vec<String> {
        dispatched(source)
            .into_iter()
            .map(|entry| entry.split(' ').next().unwrap().to_string())
            .collect()
    }

    /// Every dispatch of `kind`, as (line, col) of the node's start.
    fn dispatched_at(source: &str, kind: &str) -> Vec<(u32, u32)> {
        probe(source)
            .into_iter()
            .filter(|o| o.message.split(' ').next() == Some(kind))
            .map(|o| (o.line, o.col))
            .collect()
    }

    #[test]
    fn rescue_ensure_statements_and_arguments_reach_check_node() {
        let source = "\
begin
  foo(1)
rescue A
  bar
rescue B
  baz
else
  qux
ensure
  quux
end
";
        assert_eq!(dispatched_at(source, "RescueNode"), [(3, 1), (5, 1)]);
        assert_eq!(dispatched_at(source, "ElseNode"), [(7, 1)]);
        assert_eq!(dispatched_at(source, "EnsureNode"), [(9, 1)]);
        assert_eq!(dispatched_at(source, "ArgumentsNode"), [(2, 7)]);
        assert_eq!(
            dispatched_at(source, "StatementsNode"),
            [(1, 1), (2, 3), (4, 3), (6, 3), (8, 3), (10, 3)]
        );
    }

    #[test]
    fn every_typed_child_kind_reaches_check_node() {
        for (source, kind) in testing::TYPED_CHILD_CASES {
            let seen = dispatched_kinds(source);
            let expected = preorder_kinds(source);
            assert!(
                seen.iter().any(|seen| seen == kind),
                "{kind} not dispatched for {source:?}: {seen:?}"
            );
            assert_eq!(seen, expected, "dispatch order for {source:?}");
        }
    }

    #[test]
    fn block_argument_of_index_write_reaches_check_node() {
        // Prism rejects block arguments in index assignment, so the pipeline
        // never lints these, but it still builds the tree with the
        // `BlockArgumentNode` reached only as a typed child.
        for source in testing::ERROR_TREE_TYPED_CHILDREN {
            let seen: Vec<String> = probe_tree(source)
                .into_iter()
                .map(|o| o.message.split(' ').next().unwrap().to_string())
                .collect();
            let count = seen.iter().filter(|k| *k == "BlockArgumentNode").count();
            assert_eq!(count, 1, "BlockArgumentNode once for {source:?}: {seen:?}");
            assert_eq!(
                seen,
                preorder_kinds(source),
                "dispatch order for {source:?}"
            );
        }
    }

    #[test]
    fn every_node_reaches_check_node_exactly_once_in_preorder() {
        let source = testing::FULL_TREE;
        let seen = dispatched(source);
        let mut unique = seen.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), seen.len(), "no node is dispatched twice");
        assert_eq!(dispatched_kinds(source), preorder_kinds(source));
    }

    /// `Kind < Parent < … < ProgramNode`: a node's kind and its ancestors'
    /// kinds, innermost first.
    fn ancestry(kind: &str, ancestors: &[String]) -> String {
        let mut line = kind.to_string();
        for ancestor in ancestors.iter().rev() {
            line.push_str(" < ");
            line.push_str(ancestor);
        }
        line
    }

    /// A node's ancestry as the context reports it, checking that `parent`
    /// agrees with the first ancestor.
    fn context_ancestry(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> String {
        assert_eq!(
            cx.parent().map(kind_name),
            cx.ancestors().next().map(kind_name),
            "parent is the innermost ancestor"
        );
        let mut outermost_first: Vec<String> = cx.ancestors().map(kind_name).collect();
        outermost_first.reverse();
        assert_eq!(outermost_first.len(), cx.ancestors().len());
        ancestry(&kind_name(node), &outermost_first)
    }

    /// The walk as the oracle sees it ([`testing::tree_steps`]): `enter` and
    /// `leave` with each node's ancestry, or only the `enter`s.
    fn oracle_ancestry(source: &str, with_leaves: bool) -> Vec<String> {
        let mut open: Vec<String> = Vec::new();
        let mut lines = Vec::new();
        for step in testing::tree_steps(source) {
            match step {
                testing::Step::Enter(kind) => {
                    let line = ancestry(&kind, &open);
                    lines.push(if with_leaves {
                        format!("enter {line}")
                    } else {
                        line
                    });
                    open.push(kind);
                }
                testing::Step::Leave(kind) => {
                    open.pop();
                    if with_leaves {
                        lines.push(format!("leave {}", ancestry(&kind, &open)));
                    }
                }
            }
        }
        lines
    }

    /// Reports every node with its ancestry (`Kind < Parent < …`).
    struct AncestryProbe;

    impl Cop for AncestryProbe {
        fn name(&self) -> &'static str {
            "Lint/AncestryProbe"
        }

        fn check_node<'pr>(
            &self,
            node: &Node<'pr>,
            cx: &NodeContext<'_, 'pr>,
            _file: &SourceFile,
            out: &mut Vec<Offense>,
        ) {
            out.push(probe_offense(self.name(), context_ancestry(node, cx)));
        }

        fn leave_node<'pr>(
            &self,
            _node: &Node<'pr>,
            _cx: &NodeContext<'_, 'pr>,
            _file: &SourceFile,
            _out: &mut Vec<Offense>,
        ) {
            panic!("leave_node called on a cop that does not want it");
        }
    }

    /// Reports every `check_node` and `leave_node` call with the node's
    /// ancestry (`enter Kind < Parent < …`, `leave Kind < Parent < …`).
    struct LeaveProbe;

    impl Cop for LeaveProbe {
        fn name(&self) -> &'static str {
            "Lint/LeaveProbe"
        }

        fn wants_leave_node(&self) -> bool {
            true
        }

        fn check_node<'pr>(
            &self,
            node: &Node<'pr>,
            cx: &NodeContext<'_, 'pr>,
            _file: &SourceFile,
            out: &mut Vec<Offense>,
        ) {
            let line = format!("enter {}", context_ancestry(node, cx));
            out.push(probe_offense(self.name(), line));
        }

        fn leave_node<'pr>(
            &self,
            node: &Node<'pr>,
            cx: &NodeContext<'_, 'pr>,
            _file: &SourceFile,
            out: &mut Vec<Offense>,
        ) {
            let line = format!("leave {}", context_ancestry(node, cx));
            out.push(probe_offense(self.name(), line));
        }
    }

    /// An offense carrying only a message, at 1:1.
    fn probe_offense(cop: &'static str, message: String) -> Offense {
        Offense {
            correction: None,
            cop,
            severity: Severity::Error,
            message,
            line: 1,
            end_line: 1,
            col: 1,
        }
    }

    /// Run `cops` over `source`, Prism errors or not; the messages in order.
    fn probe_messages(cops: &[&dyn Cop], source: &str) -> Vec<String> {
        let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
        let result = ruby_prism::parse(&file.bytes);
        let mut out = Vec::new();
        let panics = run_cops(cops, &result, &file, &mut out);
        assert!(panics.is_empty(), "{panics:?}");
        out.into_iter().map(|o| o.message).collect()
    }

    /// The full tree, every typed-child snippet and the error-recovered trees.
    fn ancestry_fixtures() -> Vec<&'static str> {
        let mut sources = vec![testing::FULL_TREE];
        sources.extend(testing::TYPED_CHILD_CASES.iter().map(|(source, _)| *source));
        sources.extend(testing::ERROR_TREE_TYPED_CHILDREN);
        sources
    }

    #[test]
    fn check_node_sees_every_ancestor_typed_children_included() {
        for source in ancestry_fixtures() {
            assert_eq!(
                probe_messages(&[&AncestryProbe], source),
                oracle_ancestry(source, false),
                "ancestry for {source:?}"
            );
        }
    }

    #[test]
    fn root_has_no_parent() {
        let seen = probe_messages(&[&AncestryProbe], "foo(1)\n");
        assert_eq!(
            seen,
            [
                "ProgramNode",
                "StatementsNode < ProgramNode",
                "CallNode < StatementsNode < ProgramNode",
                "ArgumentsNode < CallNode < StatementsNode < ProgramNode",
                "IntegerNode < ArgumentsNode < CallNode < StatementsNode < ProgramNode",
            ]
        );
    }

    #[test]
    fn leave_node_follows_each_subtree_with_the_same_ancestors() {
        for source in ancestry_fixtures() {
            assert_eq!(
                probe_messages(&[&LeaveProbe], source),
                oracle_ancestry(source, true),
                "enter and leave for {source:?}"
            );
        }
    }

    #[test]
    fn leave_node_runs_only_for_cops_that_want_it() {
        // `AncestryProbe::leave_node` panics; the walk never calls it, also
        // when another cop does take leave calls.
        let source = "def m(a)\n  foo(a) { |b| b }\nend\n";
        let (leave_probe, ancestry_probe): (Vec<String>, Vec<String>) =
            probe_messages(&[&AncestryProbe, &LeaveProbe], source)
                .into_iter()
                .partition(|m| m.starts_with("enter ") || m.starts_with("leave "));
        assert_eq!(ancestry_probe, oracle_ancestry(source, false));
        assert_eq!(leave_probe, oracle_ancestry(source, true));
    }

    #[test]
    fn context_keeps_the_actual_ancestors_not_same_kind_siblings() {
        struct CallIdentityProbe;

        impl Cop for CallIdentityProbe {
            fn name(&self) -> &'static str {
                "Lint/CallIdentityProbe"
            }

            fn wants_leave_node(&self) -> bool {
                true
            }

            fn check_node<'pr>(
                &self,
                node: &Node<'pr>,
                cx: &NodeContext<'_, 'pr>,
                _file: &SourceFile,
                out: &mut Vec<Offense>,
            ) {
                let Some(call) = node.as_call_node() else {
                    return;
                };
                let parents: Vec<_> = cx
                    .ancestors()
                    .filter_map(Node::as_call_node)
                    .map(|call| String::from_utf8_lossy(call.name().as_slice()).into_owned())
                    .collect();
                let parent = cx.parent().unwrap().location();
                out.push(probe_offense(
                    self.name(),
                    format!(
                        "{} < {parents:?}; parent={}",
                        String::from_utf8_lossy(call.name().as_slice()),
                        String::from_utf8_lossy(parent.as_slice()),
                    ),
                ));
            }

            fn leave_node<'pr>(
                &self,
                node: &Node<'pr>,
                cx: &NodeContext<'_, 'pr>,
                file: &SourceFile,
                out: &mut Vec<Offense>,
            ) {
                self.check_node(node, cx, file, out);
            }
        }

        // Nested calls and adjacent siblings have identical node kinds, but
        // different identities. Both hooks must see the enclosing call, never
        // a previous sibling, and the actual typed Arguments/Statements parent.
        let source = "outer(inner(deep(1)), sibling(2))\nafter(3)";
        assert_eq!(
            probe_messages(&[&CallIdentityProbe], source),
            [
                "outer < []; parent=outer(inner(deep(1)), sibling(2))\nafter(3)",
                "inner < [\"outer\"]; parent=inner(deep(1)), sibling(2)",
                "deep < [\"inner\", \"outer\"]; parent=deep(1)",
                "deep < [\"inner\", \"outer\"]; parent=deep(1)",
                "inner < [\"outer\"]; parent=inner(deep(1)), sibling(2)",
                "sibling < [\"outer\"]; parent=inner(deep(1)), sibling(2)",
                "sibling < [\"outer\"]; parent=inner(deep(1)), sibling(2)",
                "outer < []; parent=outer(inner(deep(1)), sibling(2))\nafter(3)",
                "after < []; parent=outer(inner(deep(1)), sibling(2))\nafter(3)",
                "after < []; parent=outer(inner(deep(1)), sibling(2))\nafter(3)",
            ],
        );
    }

    #[test]
    fn duplicate_cops_panicking_in_leave_do_not_skip_other_leave_callbacks() {
        let source = "mark\nbang\nmark\nbinding.pry\n";
        // Exercise removals at the beginning, middle and end of the leave
        // list. Duplicate entries must each panic once, not remove each other.
        let orders: [&[&dyn Cop]; 3] = [
            &[&testing::PanicOnLeave, &testing::PanicOnLeave, &LeaveProbe],
            &[&testing::PanicOnLeave, &LeaveProbe, &testing::PanicOnLeave],
            &[&LeaveProbe, &testing::PanicOnLeave, &testing::PanicOnLeave],
        ];
        for cops in orders {
            let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
            let result = ruby_prism::parse(&file.bytes);
            let mut out = Vec::new();
            let panics = {
                let _quiet = crate::isolation::QuietPanics::enter();
                run_cops(cops, &result, &file, &mut out)
            };
            assert_eq!(panics.len(), 2, "{panics:?}");
            for panic in panics {
                assert_eq!(panic.cop, "Lint/PanicOnLeave");
                assert_eq!(panic.position, Some((2, 1)));
                assert_eq!(panic.panic.message, "PanicOnLeave left `bang`");
            }
            let marks: Vec<_> = out
                .iter()
                .filter(|o| o.cop == "Lint/PanicOnLeave")
                .map(|o| (o.line, o.col))
                .collect();
            assert_eq!(marks, [(1, 1), (1, 1)]);
            let seen: Vec<_> = out
                .into_iter()
                .filter(|o| o.cop == "Lint/LeaveProbe")
                .map(|o| o.message)
                .collect();
            assert_eq!(seen, oracle_ancestry(source, true));
        }
    }

    #[test]
    fn a_cop_panicking_in_leave_node_is_isolated() {
        let (offenses, panics) = run_isolated(
            &[&testing::PanicOnLeave, &lint::Debugger],
            "mark\nbang\nmark\nbinding.pry\n",
        );
        assert_eq!(
            offenses,
            [("Lint/PanicOnLeave", 1, 1), ("Lint/Debugger", 4, 1)],
            "no `mark` offense after the panic"
        );
        assert_eq!(panics.len(), 1, "{panics:?}");
        assert_eq!(panics[0].cop, "Lint/PanicOnLeave");
        assert_eq!(panics[0].position, Some((2, 1)));
        assert_eq!(panics[0].panic.message, "PanicOnLeave left `bang`");
    }

    #[test]
    fn a_cop_panicking_in_check_node_gets_no_further_leave_calls() {
        // `mark(boom)`: the panic on `boom` drops the cop before it leaves
        // `boom` or `mark`, so `mark` is never reported.
        let (offenses, panics) = run_isolated(
            &[&testing::PanicOnLeave, &lint::Debugger],
            "mark(boom)\nmark\nbinding.pry\n",
        );
        assert_eq!(offenses, [("Lint/Debugger", 3, 1)]);
        assert_eq!(panics.len(), 1, "{panics:?}");
        assert_eq!(panics[0].position, Some((1, 6)));
        assert!(
            panics[0].panic.message.contains("index out of bounds"),
            "{panics:?}"
        );
    }

    #[test]
    fn per_cop_config_overrides_department() {
        let registry = Registry::new();
        let raw: crate::config::RawConfig = toml::from_str(
            "[cops.\"Lint\"]\nenabled = false\n\n[cops.\"Lint/Debugger\"]\nenabled = true\n",
        )
        .unwrap();
        let config = ResolvedConfig::resolve(raw, None, &registry).unwrap();
        let active = select_active(&registry, &config, &[], &[]);
        let lint: Vec<&str> = active
            .iter()
            .map(|cop| cop.name())
            .filter(|name| name.starts_with("Lint/"))
            .collect();
        assert_eq!(lint, ["Lint/Debugger"]);
    }
}
