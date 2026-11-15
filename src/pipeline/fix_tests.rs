//! Safe-correction integration tests, with production Style cops and synthetic
//! cops for unsafe/conflicting/crashing infrastructure cases.
//!
//! Style fixture examples ported from RuboCop's spec/rubocop/cop/style specs
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

use super::*;
use crate::{
    cops::{Cop, NodeContext},
    offense::Correction,
};

struct Fixer {
    name: &'static str,
    method: &'static [u8],
    replacement: Option<&'static str>,
    safe: bool,
}

impl Cop for Fixer {
    fn name(&self) -> &'static str {
        self.name
    }
    fn safe_fixes(&self) -> bool {
        self.safe
    }
    fn check_node<'pr>(
        &self,
        node: &ruby_prism::Node<'pr>,
        _: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.name().as_slice() != self.method {
            return;
        }
        let loc = call.message_loc().unwrap();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "test fix".into(),
            line,
            end_line: line,
            col,
            correction: self.replacement.map(|text| Correction {
                range: loc.start_offset()..loc.end_offset(),
                replacement: text.into(),
            }),
        });
    }
}

fn registry() -> Registry {
    Registry::from_cops(vec![
        Box::new(Fixer {
            name: "Style/SafeFix",
            method: b"before",
            replacement: Some("after"),
            safe: true,
        }),
        Box::new(Fixer {
            name: "Style/UnsafeFix",
            method: b"risky",
            replacement: Some("changed"),
            safe: false,
        }),
        Box::new(Fixer {
            name: "Lint/Unfixable",
            method: b"unfixable",
            replacement: None,
            safe: true,
        }),
    ])
}

fn project(source: &str) -> (tempfile::TempDir, ResolvedConfig) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("x.rb"), source).unwrap();
    let config = ResolvedConfig {
        root: dir.path().to_path_buf(),
        ..ResolvedConfig::default()
    };
    (dir, config)
}

fn run_single_pass(dir: &Path, config: &ResolvedConfig, registry: &Registry) -> RunReport {
    run(
        &[dir.to_path_buf()],
        &RunOptions {
            fix: true,
            fix_passes: Some(1),
            ..RunOptions::default()
        },
        config,
        registry,
    )
    .unwrap()
}

fn run_project(dir: &Path, config: &ResolvedConfig, registry: &Registry, fix: bool) -> RunReport {
    run(
        &[dir.to_path_buf()],
        &RunOptions {
            fix,
            ..RunOptions::default()
        },
        config,
        registry,
    )
    .unwrap()
}

#[test]
fn correction_validation_rejects_ruby_4_logical_chains_without_writing() {
    for replacement in ["foo\n  && bar", "foo\n  || bar"] {
        let registry = Registry::from_cops(vec![Box::new(Fixer {
            name: "Style/VersionFix",
            method: b"before",
            replacement: Some(replacement),
            safe: true,
        })]);
        let (dir, config) = project("before\n");
        let path = dir.path().join("x.rb");
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let report = run_project(dir.path(), &config, &registry, true);
        assert_eq!(report.exit_code(), EXIT_ERRORS);
        assert_eq!(report.corrections_applied, 0);
        assert_eq!(report.offense_count(), 1);
        assert!(report.internal_errors.is_empty());
        assert_eq!(report.errors.len(), 1);
        assert_eq!(
            report.errors[0].message,
            "corrections failed validation; file left unchanged"
        );
        assert_eq!(fs::read(&path).unwrap(), b"before\n");
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

#[test]
fn a_run_at_ruby_4_parses_and_validates_with_the_ruby_4_grammar() {
    use crate::cops::policy::{DEFAULT_RUBY_VERSION, RubyVersion};
    for replacement in ["foo\n  && bar", "foo\n  || bar"] {
        let registry = Registry::from_cops(vec![Box::new(Fixer {
            name: "Style/VersionFix",
            method: b"before",
            replacement: Some(replacement),
            safe: true,
        })]);
        let (dir, mut config) = project("before\n");
        config.ruby_version = RubyVersion(4, 0);
        let path = dir.path().join("x.rb");
        let report = run_project(dir.path(), &config, &registry, true);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(report.internal_errors.is_empty());
        assert_eq!(report.corrections_applied, 1);
        assert_eq!(
            fs::read(&path).unwrap(),
            format!("{replacement}\n").as_bytes()
        );

        let accepted = run_project(dir.path(), &config, &registry, false);
        assert!(accepted.errors.is_empty(), "{:?}", accepted.errors);
        config.ruby_version = DEFAULT_RUBY_VERSION;
        let rejected = run_project(dir.path(), &config, &registry, false);
        assert!(
            rejected.errors[0]
                .message
                .starts_with("syntax error on line 2"),
            "{:?}",
            rejected.errors
        );
    }
}

#[test]
fn corrections_that_exceed_the_ast_depth_budget_are_not_written() {
    let source = format!("{}before{}\n", "[".repeat(61), "]".repeat(61));
    let registry = Registry::from_cops(vec![Box::new(Fixer {
        name: "Style/DeepFix",
        method: b"before",
        replacement: Some("[[after]]"),
        safe: true,
    })]);
    let (dir, config) = project(&source);
    let report = run_project(dir.path(), &config, &registry, true);
    assert_eq!(report.exit_code(), EXIT_ERRORS);
    assert_eq!(report.corrections_applied, 0);
    assert_eq!(report.offense_count(), 1);
    assert!(report.internal_errors.is_empty());
    assert_eq!(report.errors.len(), 1);
    assert_eq!(
        report.errors[0].message,
        "corrections failed validation; file left unchanged"
    );
    assert_eq!(fs::read_to_string(dir.path().join("x.rb")).unwrap(), source);
}

#[test]
fn corrections_that_exhaust_parser_budgets_are_not_written() {
    struct ParserBudgetFix(String);

    impl Cop for ParserBudgetFix {
        fn name(&self) -> &'static str {
            "Style/ParserBudgetFix"
        }
        fn safe_fixes(&self) -> bool {
            true
        }
        fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
            if file.bytes != b"before\n" {
                return;
            }
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "test parser validation".into(),
                line: 1,
                end_line: 1,
                col: 1,
                correction: Some(Correction {
                    range: 0..file.bytes.len(),
                    replacement: self.0.clone(),
                }),
            });
        }
    }

    for replacement in [
        format!(
            "{}{}",
            "def foo(*args)\nbar(*args)\n".repeat(1000),
            "end\n".repeat(1000)
        ),
        format!(
            "value in {}capture{}\n",
            "[".repeat(12_000),
            "]".repeat(12_000)
        ),
    ] {
        let registry = Registry::from_cops(vec![Box::new(ParserBudgetFix(replacement))]);
        let (dir, config) = project("before\n");
        let report = run_project(dir.path(), &config, &registry, true);
        assert_eq!(report.exit_code(), EXIT_ERRORS);
        assert_eq!(report.corrections_applied, 0);
        assert_eq!(report.offense_count(), 1);
        assert!(report.internal_errors.is_empty());
        assert_eq!(report.errors.len(), 1);
        assert_eq!(
            report.errors[0].message,
            "corrections failed validation; file left unchanged"
        );
        assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), b"before\n");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

#[test]
fn control_flow_hash_fixes_do_not_block_unrelated_corrections() {
    let registry = Registry::from_cops(vec![
        Box::new(crate::cops::style::HashSyntax::default()),
        Box::new(crate::cops::style::SignalException),
    ]);
    for (before, after) in [
        (
            "p [1].map { next :a => 1 }\nfail\n",
            "p [1].map { next {a: 1} }\nraise\n",
        ),
        (
            "loop { break :a => 1, :b => 2 }\nfail\n",
            "loop { break {a: 1, b: 2} }\nraise\n",
        ),
    ] {
        let (dir, config) = project(before);
        let report = run_project(dir.path(), &config, &registry, true);
        assert!(!report.has_errors(), "{report:?}");
        assert_eq!(report.corrections_applied, 2);
        assert_eq!(report.offense_count(), 0);
        assert_eq!(fs::read_to_string(dir.path().join("x.rb")).unwrap(), after);
    }
}

#[test]
fn fixes_preserve_magic_lines_after_multiline_method_edits() {
    let source = "def foo\n  (\n    1\n  )\nend\ndef bar\n  return(\n  )\nend\np __LINE__\nfail\n";
    let registry = Registry::from_cops(vec![
        Box::new(crate::cops::style::RedundantParentheses),
        Box::new(crate::cops::style::RedundantReturn),
        Box::new(crate::cops::style::SignalException),
    ]);
    let (dir, config) = project(source);
    let report = run_project(dir.path(), &config, &registry, true);
    assert!(!report.has_errors(), "{report:?}");
    assert_eq!(report.corrections_applied, 1);
    assert_eq!(report.offense_count(), 2);
    assert_eq!(
        fs::read_to_string(dir.path().join("x.rb")).unwrap(),
        source.replace("fail", "raise")
    );
}

#[test]
fn fixture_pairs_apply_safe_fixes_and_report_remaining_offenses() {
    for (before, after, count, remaining) in [
        (
            include_str!("../../tests/fixtures/fix/safe.before.rb"),
            include_str!("../../tests/fixtures/fix/safe.after.rb"),
            2,
            0,
        ),
        (
            include_str!("../../tests/fixtures/fix/mixed.before.rb"),
            include_str!("../../tests/fixtures/fix/mixed.after.rb"),
            2,
            2,
        ),
        (
            include_str!("../../tests/fixtures/fix/directives.before.rb"),
            include_str!("../../tests/fixtures/fix/directives.after.rb"),
            1,
            0,
        ),
    ] {
        let (dir, config) = project(before);
        let report = run_project(dir.path(), &config, &registry(), true);
        assert!(!report.has_errors(), "{report:?}");
        assert_eq!(fs::read_to_string(dir.path().join("x.rb")).unwrap(), after);
        assert_eq!(report.corrections_applied, count);
        assert_eq!(report.offense_count(), remaining);
        assert_eq!(
            report.exit_code(),
            if remaining == 0 {
                EXIT_CLEAN
            } else {
                EXIT_OFFENSES
            }
        );
        assert!(crate::output::render_text(&report).contains(&format!(
            "{count} correction{} applied, {remaining} offense{} remaining",
            if count == 1 { "" } else { "s" },
            if remaining == 1 { "" } else { "s" }
        )));
    }
}

#[test]
fn detection_only_never_rewrites_even_when_cops_supply_safe_fixes() {
    let (dir, config) = project("before\nrisky\n");
    let report = run_project(dir.path(), &config, &registry(), false);
    assert_eq!(report.offense_count(), 2);
    assert_eq!(report.corrections_applied, 0);
    assert_eq!(
        fs::read(dir.path().join("x.rb")).unwrap(),
        b"before\nrisky\n"
    );
    assert_eq!(
        crate::output::render_text(&report),
        "x.rb:1:1: C: test fix (Style/SafeFix)\nx.rb:2:1: C: test fix (Style/UnsafeFix)\n\n1 file inspected, 2 offenses detected\n"
    );
}

#[test]
fn only_the_safe_style_layout_and_landed_naming_lint_cops_opt_in() {
    let registry = Registry::new();
    let mut safe: Vec<_> = registry
        .cops()
        .iter()
        .filter(|cop| cop.safe_fixes())
        .map(|cop| cop.name())
        .collect();
    safe.sort_unstable();
    assert_eq!(
        safe,
        [
            "Bundler/InsecureProtocolSource",
            "Bundler/OrderedGems",
            "Gemspec/AddRuntimeDependency",
            "Gemspec/DeprecatedAttributeAssignment",
            "Gemspec/OrderedDependencies",
            "Gemspec/RequireMFA",
            "Layout/AccessModifierIndentation",
            "Layout/ArgumentAlignment",
            "Layout/ArrayAlignment",
            "Layout/AssignmentIndentation",
            "Layout/BeginEndAlignment",
            "Layout/BlockAlignment",
            "Layout/BlockEndNewline",
            "Layout/CaseIndentation",
            "Layout/ClosingHeredocIndentation",
            "Layout/ClosingParenthesisIndentation",
            "Layout/CommentIndentation",
            "Layout/ConditionPosition",
            "Layout/DefEndAlignment",
            "Layout/DotPosition",
            "Layout/ElseAlignment",
            "Layout/EmptyComment",
            "Layout/EmptyLineAfterGuardClause",
            "Layout/EmptyLineAfterMagicComment",
            "Layout/EmptyLineAfterMultilineCondition",
            "Layout/EmptyLineBetweenDefs",
            "Layout/EmptyLines",
            "Layout/EmptyLinesAfterModuleInclusion",
            "Layout/EmptyLinesAroundAccessModifier",
            "Layout/EmptyLinesAroundArguments",
            "Layout/EmptyLinesAroundAttributeAccessor",
            "Layout/EmptyLinesAroundBeginBody",
            "Layout/EmptyLinesAroundBlockBody",
            "Layout/EmptyLinesAroundClassBody",
            "Layout/EmptyLinesAroundExceptionHandlingKeywords",
            "Layout/EmptyLinesAroundMethodBody",
            "Layout/EmptyLinesAroundModuleBody",
            "Layout/EndAlignment",
            "Layout/ExtraSpacing",
            "Layout/FirstArgumentIndentation",
            "Layout/FirstArrayElementIndentation",
            "Layout/FirstArrayElementLineBreak",
            "Layout/FirstHashElementIndentation",
            "Layout/FirstHashElementLineBreak",
            "Layout/FirstMethodArgumentLineBreak",
            "Layout/FirstMethodParameterLineBreak",
            "Layout/FirstParameterIndentation",
            "Layout/HashAlignment",
            "Layout/HeredocArgumentClosingParenthesis",
            "Layout/HeredocIndentation",
            "Layout/IndentationConsistency",
            "Layout/IndentationStyle",
            "Layout/IndentationWidth",
            "Layout/InitialIndentation",
            "Layout/LeadingCommentSpace",
            "Layout/LeadingEmptyLines",
            "Layout/LineContinuationLeadingSpace",
            "Layout/LineContinuationSpacing",
            "Layout/LineEndStringConcatenationIndentation",
            "Layout/MultilineArrayBraceLayout",
            "Layout/MultilineArrayLineBreaks",
            "Layout/MultilineAssignmentLayout",
            "Layout/MultilineBlockLayout",
            "Layout/MultilineHashBraceLayout",
            "Layout/MultilineHashKeyLineBreaks",
            "Layout/MultilineMethodArgumentLineBreaks",
            "Layout/MultilineMethodCallBraceLayout",
            "Layout/MultilineMethodCallIndentation",
            "Layout/MultilineMethodDefinitionBraceLayout",
            "Layout/MultilineMethodParameterLineBreaks",
            "Layout/MultilineOperationIndentation",
            "Layout/ParameterAlignment",
            "Layout/RedundantLineBreak",
            "Layout/RescueEnsureAlignment",
            "Layout/SingleLineBlockChain",
            "Layout/SpaceAfterColon",
            "Layout/SpaceAfterComma",
            "Layout/SpaceAfterMethodName",
            "Layout/SpaceAfterNot",
            "Layout/SpaceAfterSemicolon",
            "Layout/SpaceAroundBlockParameters",
            "Layout/SpaceAroundEqualsInParameterDefault",
            "Layout/SpaceAroundKeyword",
            "Layout/SpaceAroundMethodCallOperator",
            "Layout/SpaceAroundOperators",
            "Layout/SpaceBeforeBlockBraces",
            "Layout/SpaceBeforeBrackets",
            "Layout/SpaceBeforeComma",
            "Layout/SpaceBeforeComment",
            "Layout/SpaceBeforeFirstArg",
            "Layout/SpaceBeforeSemicolon",
            "Layout/SpaceInLambdaLiteral",
            "Layout/SpaceInsideArrayLiteralBrackets",
            "Layout/SpaceInsideArrayPercentLiteral",
            "Layout/SpaceInsideBlockBraces",
            "Layout/SpaceInsideHashLiteralBraces",
            "Layout/SpaceInsideParens",
            "Layout/SpaceInsidePercentLiteralDelimiters",
            "Layout/SpaceInsideRangeLiteral",
            "Layout/SpaceInsideReferenceBrackets",
            "Layout/SpaceInsideStringInterpolation",
            "Layout/TrailingEmptyLines",
            "Layout/TrailingWhitespace",
            "Lint/AmbiguousOperatorPrecedence",
            "Lint/BigDecimalNew",
            "Lint/ConstantOverwrittenInRescue",
            "Lint/CopDirectiveSyntax",
            "Lint/DeprecatedConstants",
            "Lint/DeprecatedOpenSSLConstant",
            "Lint/DuplicateMagicComment",
            "Lint/DuplicateRegexpCharacterClassElement",
            "Lint/ElseLayout",
            "Lint/ErbNewArguments",
            "Lint/HashNewWithKeywordArgumentsAsDefault",
            "Lint/HeredocMethodCallPosition",
            "Lint/IdentityComparison",
            "Lint/ImplicitStringConcatenation",
            "Lint/LambdaWithoutLiteralBlock",
            "Lint/ParenthesesAsGroupedExpression",
            "Lint/RedundantCopDisableDirective",
            "Lint/RedundantCopEnableDirective",
            "Lint/RedundantRegexpQuantifiers",
            "Lint/RedundantSplatExpansion",
            "Lint/RedundantStringCoercion",
            "Lint/RedundantTypeConversion",
            "Lint/RedundantWithIndex",
            "Lint/RegexpAsCondition",
            "Lint/RequireRelativeSelfPath",
            "Lint/RescueType",
            "Lint/SafeNavigationWithEmpty",
            "Lint/SendWithMixinArgument",
            "Lint/SymbolConversion",
            "Lint/ToJSON",
            "Lint/TrailingCommaInAttributeDeclaration",
            "Lint/TripleQuotes",
            "Lint/UnescapedBracketInRegexp",
            "Lint/UnifiedInteger",
            "Lint/UnusedBlockArgument",
            "Lint/UnusedMethodArgument",
            "Lint/UriRegexp",
            "Lint/UselessAccessModifier",
            "Lint/UselessNumericOperation",
            "Migration/DepartmentName",
            "Naming/BinaryOperatorParameterName",
            "Naming/BlockForwarding",
            "Naming/HeredocDelimiterCase",
            "Naming/RescuedExceptionsVariableName",
            "Style/AccessorGrouping",
            "Style/Alias",
            "Style/AmbiguousEndlessMethodDefinition",
            "Style/ArgumentsForwarding",
            "Style/ArrayJoin",
            "Style/Attr",
            "Style/BarePercentLiterals",
            "Style/BisectedAttrAccessor",
            "Style/BlockComments",
            "Style/BlockDelimiters",
            "Style/CaseEquality",
            "Style/CharacterLiteral",
            "Style/ClassCheck",
            "Style/ClassMethods",
            "Style/ClassMethodsDefinitions",
            "Style/ColonMethodCall",
            "Style/ColonMethodDefinition",
            "Style/CombinableDefined",
            "Style/CommandLiteral",
            "Style/CommentAnnotation",
            "Style/ComparableClamp",
            "Style/ConditionalAssignment",
            // Safe-correctable inventory entry, but the fixed empty notice
            // withholds every edit rather than inventing copyright text.
            "Style/Copyright",
            "Style/DefWithParentheses",
            "Style/Dir",
            "Style/DirEmpty",
            "Style/DisableCopsWithinSourceCodeDirective",
            "Style/EachForSimpleLoop",
            "Style/EachWithObject",
            "Style/EmptyBlockParameter",
            "Style/EmptyCaseCondition",
            "Style/EmptyClassDefinition",
            "Style/EmptyElse",
            "Style/EmptyHeredoc",
            "Style/EmptyLambdaParameter",
            "Style/EmptyLiteral",
            "Style/EmptyMethod",
            "Style/EmptyStringInsideInterpolation",
            "Style/Encoding",
            "Style/EndBlock",
            "Style/EndlessMethod",
            "Style/EvalWithLocation",
            "Style/EvenOdd",
            "Style/ExactRegexpMatch",
            "Style/ExpandPathArguments",
            "Style/ExplicitBlockArgument",
            "Style/FetchEnvVar",
            "Style/FileRead",
            "Style/FileWrite",
            "Style/FormatString",
            "Style/FormatStringToken",
            "Style/GuardClause",
            "Style/HashAsLastArrayItem",
            "Style/HashSyntax",
            "Style/IfInsideElse",
            "Style/IfUnlessModifier",
            "Style/IfUnlessModifierOfIfUnless",
            "Style/IfWithSemicolon",
            "Style/InPatternThen",
            "Style/ItBlockParameter",
            "Style/KeywordArgumentsMerging",
            "Style/KeywordParametersOrder",
            "Style/Lambda",
            "Style/LambdaCall",
            "Style/MagicCommentFormat",
            "Style/MethodCallWithArgsParentheses",
            "Style/MethodCallWithoutArgsParentheses",
            "Style/MethodDefParentheses",
            "Style/MinMax",
            "Style/MissingElse",
            "Style/MixinGrouping",
            "Style/ModuleMemberExistenceCheck",
            "Style/MultilineIfModifier",
            "Style/MultilineIfThen",
            "Style/MultilineInPatternThen",
            "Style/MultilineMemoization",
            "Style/MultilineMethodSignature",
            "Style/MultilineTernaryOperator",
            "Style/MultilineWhenThen",
            "Style/MultipleComparison",
            "Style/NegatedIf",
            "Style/NegatedIfElseCondition",
            "Style/NegatedUnless",
            "Style/NegatedWhile",
            "Style/NestedFileDirname",
            "Style/NestedModifier",
            "Style/NestedParenthesizedCalls",
            "Style/NestedTernaryOperator",
            "Style/Next",
            "Style/NilComparison",
            "Style/NilLambda",
            "Style/NonNilCheck",
            "Style/Not",
            "Style/NumericLiteralPrefix",
            "Style/NumericLiterals",
            "Style/ObjectThen",
            "Style/OneLineConditional",
            "Style/OperatorMethodCall",
            "Style/OrAssignment",
            "Style/ParallelAssignment",
            "Style/ParenthesesAroundCondition",
            "Style/PercentLiteralDelimiters",
            "Style/PercentQLiterals",
            "Style/PerlBackrefs",
            "Style/Proc",
            "Style/QuotedSymbols",
            "Style/RandomWithOffset",
            "Style/RedundantArrayConstructor",
            "Style/RedundantAssignment",
            "Style/RedundantBegin",
            "Style/RedundantCapitalW",
            "Style/RedundantCondition",
            "Style/RedundantConditional",
            "Style/RedundantConstantBase",
            "Style/RedundantCurrentDirectoryInPath",
            "Style/RedundantDoubleSplatHashBraces",
            "Style/RedundantException",
            "Style/RedundantFileExtensionInRequire",
            "Style/RedundantFreeze",
            "Style/RedundantHeredocDelimiterQuotes",
            "Style/RedundantInterpolationUnfreeze",
            "Style/RedundantLineContinuation",
            "Style/RedundantMinMaxBy",
            "Style/RedundantParentheses",
            "Style/RedundantPercentQ",
            "Style/RedundantRegexpArgument",
            "Style/RedundantRegexpCharacterClass",
            "Style/RedundantRegexpConstructor",
            "Style/RedundantRegexpEscape",
            "Style/RedundantReturn",
            "Style/RedundantSelf",
            "Style/RedundantSelfAssignmentBranch",
            "Style/RedundantSortBy",
            "Style/RedundantStringEscape",
            "Style/RegexpLiteral",
            "Style/RescueModifier",
            "Style/RescueStandardError",
            "Style/ReturnNil",
            "Style/Sample",
            "Style/SelfAssignment",
            "Style/Semicolon",
            "Style/SignalException",
            "Style/SingleLineBlockParams",
            "Style/SingleLineDoEndBlock",
            "Style/SingleLineMethods",
            "Style/SoleNestedConditional",
            "Style/StabbyLambdaParentheses",
            "Style/StderrPuts",
            "Style/StringLiterals",
            "Style/StringLiteralsInInterpolation",
            "Style/StringMethods",
            "Style/Strip",
            "Style/SuperArguments",
            "Style/SuperWithArgsParentheses",
            "Style/SymbolArray",
            "Style/SymbolLiteral",
            "Style/TernaryParentheses",
            "Style/TimeNow",
            "Style/TrailingBodyOnClass",
            "Style/TrailingBodyOnMethodDefinition",
            "Style/TrailingBodyOnModule",
            "Style/TrailingCommaInArguments",
            "Style/TrailingCommaInArrayLiteral",
            "Style/TrailingCommaInHashLiteral",
            "Style/TrailingMethodEndStatement",
            "Style/TrailingUnderscoreVariable",
            "Style/TrivialAccessors",
            "Style/UnlessElse",
            "Style/UnpackFirst",
            "Style/VariableInterpolation",
            "Style/WhenThen",
            "Style/WhileUntilDo",
            "Style/WhileUntilModifier",
            "Style/WordArray",
            "Style/YAMLFileRead",
        ]
    );
}

#[test]
fn production_style_fixture_pairs() {
    macro_rules! fixture {
        ($name:literal, $cop:literal, $remaining:literal) => {
            (
                $cop,
                include_str!(concat!("../../tests/fixtures/fix/", $name, ".before.rb")),
                include_str!(concat!("../../tests/fixtures/fix/", $name, ".after.rb")),
                $remaining,
            )
        };
    }
    let registry = Registry::new();
    for (cop, before, after, remaining) in [
        fixture!("string_literals", "Style/StringLiterals", 0),
        fixture!(
            "string_literals_in_interpolation",
            "Style/StringLiteralsInInterpolation",
            0
        ),
        fixture!("hash_syntax", "Style/HashSyntax", 0),
        fixture!("redundant_return", "Style/RedundantReturn", 0),
        fixture!("semicolon", "Style/Semicolon", 1),
        fixture!("redundant_parentheses", "Style/RedundantParentheses", 1),
        fixture!("signal_exception", "Style/SignalException", 0),
    ] {
        let (dir, config) = project(before);
        for fix in [false, true] {
            let report = run(
                &[dir.path().to_path_buf()],
                &RunOptions {
                    fix,
                    fix_passes: fix.then_some(1),
                    only: vec![cop.into()],
                    ..RunOptions::default()
                },
                &config,
                &registry,
            )
            .unwrap();
            assert!(!report.has_errors(), "{cop}: {report:?}");
            assert_eq!(
                fs::read_to_string(dir.path().join("x.rb")).unwrap(),
                if fix { after } else { before },
                "{cop}"
            );
            if fix {
                assert!(report.corrections_applied > 0, "{cop}");
                assert_eq!(report.offense_count(), remaining, "{cop}");
            } else {
                assert_eq!(report.corrections_applied, 0);
            }
        }
    }
}

#[test]
fn production_atomic_corrections_respect_disabled_interior_lines() {
    for (cop, source) in [
        (
            "Style/RedundantParentheses",
            "(\n  1 # rubocop:disable Style/RedundantParentheses\n)\n",
        ),
        (
            "Style/HashSyntax",
            "return :a => 1,\n  :b => 2 # rustocop:disable Style/HashSyntax\n",
        ),
        (
            "Style/RedundantReturn",
            "def func\n  return 1,\n    2 # rubocop:disable Style/RedundantReturn\nend\n",
        ),
    ] {
        let (dir, config) = project(source);
        let report = run(
            &[dir.path().to_path_buf()],
            &RunOptions {
                fix: true,
                only: vec![cop.into()],
                ..RunOptions::default()
            },
            &config,
            &Registry::new(),
        )
        .unwrap();
        assert!(!report.has_errors(), "{report:?}");
        assert_eq!(report.corrections_applied, 0, "{cop}");
        assert_eq!(fs::read_to_string(dir.path().join("x.rb")).unwrap(), source);
    }
}

#[test]
fn production_overlaps_defer_whole_corrections_and_converge() {
    let (dir, config) = project("def func\n  return ({ :a => 'x' });\nend\n");
    let registry = Registry::new();
    let first = run_single_pass(dir.path(), &config, &registry);
    assert!(!first.has_errors(), "{first:?}");
    assert_eq!(
        fs::read_to_string(dir.path().join("x.rb")).unwrap(),
        "def func\n  { :a => 'x' }\nend\n"
    );
    assert_eq!(first.offense_count(), 2);
    for _ in 0..2 {
        let report = run_single_pass(dir.path(), &config, &registry);
        assert!(!report.has_errors(), "{report:?}");
    }
    assert_eq!(
        fs::read_to_string(dir.path().join("x.rb")).unwrap(),
        "def func\n  { a: \"x\" }\nend\n"
    );
    let final_report = run_single_pass(dir.path(), &config, &registry);
    assert_eq!(final_report.corrections_applied, 0);
    assert_eq!(final_report.offense_count(), 0);
}

#[test]
fn production_fixes_preserve_non_utf8_bytes_and_detection() {
    let (dir, config) = project("");
    let source = b"# encoding: ISO-8859-1\nx = '\xff'\nfail\n";
    fs::write(dir.path().join("x.rb"), source).unwrap();
    let report = run(
        &[dir.path().to_path_buf()],
        &RunOptions {
            fix: true,
            only: vec![
                "Style/StringLiterals".into(),
                "Style/SignalException".into(),
            ],
            ..RunOptions::default()
        },
        &config,
        &Registry::new(),
    )
    .unwrap();
    assert!(!report.has_errors(), "{report:?}");
    assert_eq!(report.corrections_applied, 1);
    assert_eq!(report.offense_count(), 1);
    assert_eq!(report.results[0].offenses[0].cop, "Style/StringLiterals");
    assert_eq!(
        fs::read(dir.path().join("x.rb")).unwrap(),
        b"# encoding: ISO-8859-1\nx = '\xff'\nraise\n"
    );
}

#[test]
fn encoded_columns_do_not_change_correction_byte_ranges_or_output() {
    use crate::cops::{layout::TrailingWhitespace, lint::Debugger, style::SignalException};

    let registry = Registry::from_cops(vec![
        Box::new(TrailingWhitespace),
        Box::new(Debugger),
        Box::new(SignalException),
    ]);
    for (prefix, col) in super::tests::encoded_position_prefixes() {
        let mut source = prefix.to_vec();
        source.extend_from_slice(b"fail; binding.pry  \n");
        let (dir, config) = project("");
        let path = dir.path().join("x.rb");
        fs::write(&path, &source).unwrap();
        let collected = lint_one_in_mode(
            &path,
            Path::new("x.rb"),
            SourceInput::Memory(&source),
            &CopSelection::explicit(&[&TrailingWhitespace, &Debugger, &SignalException], &config),
            &config,
            CorrectionMode::Collect,
        );
        assert!(collected.errors.is_empty(), "{collected:?}");
        assert!(collected.internal_errors.is_empty(), "{collected:?}");
        let fail = collected
            .result
            .offenses
            .iter()
            .find(|o| o.cop == "Style/SignalException")
            .unwrap();
        assert_eq!((fail.line, fail.col), (2, col));
        let correction = fail.correction.as_ref().unwrap();
        assert_eq!(correction.range, prefix.len()..prefix.len() + 4);
        assert_eq!(&source[correction.range.clone()], b"fail");
        let whitespace = collected
            .result
            .offenses
            .iter()
            .find(|o| o.cop == "Layout/TrailingWhitespace")
            .unwrap();
        assert_eq!((whitespace.line, whitespace.col), (2, col + 17));
        assert_eq!(
            whitespace.correction.as_ref().unwrap().range,
            source.len() - 3..source.len() - 1
        );
        assert_eq!(fs::read(&path).unwrap(), source, "collect must not write");

        let report = run_project(dir.path(), &config, &registry, true);
        assert!(!report.has_errors(), "{report:?}");
        assert_eq!(report.corrections_applied, 2);
        let mut expected = prefix.to_vec();
        expected.extend_from_slice(b"raise; binding.pry\n");
        assert_eq!(fs::read(&path).unwrap(), expected);
        assert_eq!(report.offense_count(), 1);
        let remaining = &report.results[0].offenses[0];
        assert_eq!(remaining.cop, "Lint/Debugger");
        assert_eq!((remaining.line, remaining.col), (2, col + 7));
    }
}

#[test]
fn config_excludes_and_selectors_also_prevent_fixes() {
    let (dir, mut config) = project("before\n");
    config.cop_settings.insert(
        "Style/SafeFix".into(),
        crate::config::CopSettings {
            excludes: crate::config::compile_globs(&["x.rb"], "test").unwrap(),
            ..Default::default()
        },
    );
    let report = run_project(dir.path(), &config, &registry(), true);
    assert_eq!(report.corrections_applied, 0);
    assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), b"before\n");
    config.cop_settings.clear();
    let report = run(
        &[dir.path().to_path_buf()],
        &RunOptions {
            fix: true,
            except: vec!["Style/SafeFix".into()],
            ..Default::default()
        },
        &config,
        &registry(),
    )
    .unwrap();
    assert_eq!(report.corrections_applied, 0);
    assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), b"before\n");
}

#[cfg(unix)]
#[test]
fn excludes_hold_for_absolute_paths_through_a_symlink_to_the_root() {
    let tmp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(tmp.path()).unwrap();
    let root = base.join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("x.rb"), "before\n").unwrap();
    std::os::unix::fs::symlink(&root, base.join("alias")).unwrap();
    let supplied = [base.join("alias/x.rb")];
    let fix = RunOptions {
        fix: true,
        ..RunOptions::default()
    };
    let mut config = ResolvedConfig {
        root: root.clone(),
        ..ResolvedConfig::default()
    };

    let report = run(&supplied, &RunOptions::default(), &config, &registry()).unwrap();
    assert_eq!(report.offense_count(), 1);
    assert_eq!(report.results[0].path, Path::new("x.rb"));

    config.cop_settings.insert(
        "Style/SafeFix".into(),
        crate::config::CopSettings {
            excludes: crate::config::compile_globs(&["x.rb"], "test").unwrap(),
            ..Default::default()
        },
    );
    let report = run(&supplied, &fix, &config, &registry()).unwrap();
    assert_eq!(report.files_inspected, 1);
    assert_eq!(report.offense_count(), 0);
    assert_eq!(report.corrections_applied, 0);
    assert_eq!(fs::read(root.join("x.rb")).unwrap(), b"before\n");

    config.cop_settings.clear();
    config.global_excludes = crate::config::compile_globs(&["x.rb"], "test").unwrap();
    let report = run(&supplied, &fix, &config, &registry()).unwrap();
    assert_eq!(report.files_inspected, 0);
    assert_eq!(fs::read(root.join("x.rb")).unwrap(), b"before\n");

    config.global_excludes.clear();
    let report = run(&supplied, &fix, &config, &registry()).unwrap();
    assert_eq!(report.corrections_applied, 1);
    assert_eq!(fs::read(root.join("x.rb")).unwrap(), b"after\n");
}

#[test]
fn remaining_positions_are_recomputed_after_length_changing_fixes() {
    let (dir, config) = project("before; risky\n");
    let report = run_project(dir.path(), &config, &registry(), true);
    assert_eq!(report.corrections_applied, 1);
    assert_eq!(report.offense_count(), 1);
    assert_eq!(report.results[0].offenses[0].col, 8);
    assert!(
        crate::output::render_github(&report).contains("1 correction applied, 1 offense remaining")
    );
}

fn rewriting(steps: &[(&'static str, &'static [u8], &'static str)]) -> Registry {
    Registry::from_cops(
        steps
            .iter()
            .map(|&(name, method, replacement)| {
                Box::new(Fixer {
                    name,
                    method,
                    replacement: Some(replacement),
                    safe: true,
                }) as Box<dyn Cop>
            })
            .collect(),
    )
}

#[test]
fn a_fix_run_repeats_passes_until_nothing_changes() {
    let registry = rewriting(&[
        ("Style/First", b"first", "second"),
        ("Style/Second", b"second", "third"),
        ("Style/Third", b"third", "done"),
    ]);
    let (dir, config) = project("first\nsecond\n");
    let report = run_project(dir.path(), &config, &registry, true);
    assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), b"done\ndone\n");
    assert_eq!(report.corrections_applied, 5);
    assert_eq!(report.offense_count(), 0);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
}

#[test]
fn corrections_that_undo_each_other_stop_with_an_error() {
    let registry = rewriting(&[
        ("Style/Ping", b"ping", "pong"),
        ("Style/Pong", b"pong", "ping"),
    ]);
    let (dir, config) = project("ping\n");
    let report = run_project(dir.path(), &config, &registry, true);
    assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), b"pong\n");
    assert_eq!(report.corrections_applied, 1);
    assert_eq!(report.offense_count(), 1);
    assert_eq!(report.errors.len(), 1);
    assert!(
        report.errors[0].message.contains("earlier version"),
        "{:?}",
        report.errors
    );
    assert_eq!(report.exit_code(), EXIT_ERRORS);
}

#[test]
fn corrections_that_never_settle_stop_at_the_pass_limit() {
    let registry = rewriting(&[("Style/Grow", b"grow", "grow; 1")]);
    let (dir, config) = project("grow\n");
    let report = run_project(dir.path(), &config, &registry, true);
    let expected = format!("grow{}\n", "; 1".repeat(MAX_CORRECTION_PASSES));
    assert_eq!(
        fs::read(dir.path().join("x.rb")).unwrap(),
        expected.as_bytes()
    );
    assert_eq!(report.corrections_applied, MAX_CORRECTION_PASSES);
    assert_eq!(report.errors.len(), 1);
    assert!(
        report.errors[0].message.contains("did not settle"),
        "{:?}",
        report.errors
    );
}

#[test]
fn a_later_pass_that_fails_validation_keeps_the_earlier_passes() {
    let registry = rewriting(&[("Style/Deep", b"deep", "deep.more")]);
    let (dir, config) = project("deep\n");
    let report = run_project(dir.path(), &config, &registry, true);
    let applied = report.corrections_applied;
    assert!(applied > 1 && applied < MAX_CORRECTION_PASSES, "{applied}");
    let expected = format!("deep{}\n", ".more".repeat(applied));
    assert_eq!(
        fs::read(dir.path().join("x.rb")).unwrap(),
        expected.as_bytes()
    );
    assert!(
        crate::parser::parse(expected.as_bytes())
            .depth_error()
            .is_none()
    );
    assert_eq!(report.errors.len(), 1);
    assert!(
        report.errors[0].message.contains("failed validation after"),
        "{:?}",
        report.errors
    );
}

#[test]
fn overlapping_cops_have_a_deterministic_winner_and_are_relinted() {
    for reverse in [false, true] {
        let mut cops: Vec<Box<dyn Cop>> = vec![
            Box::new(Fixer {
                name: "Style/Z",
                method: b"before",
                replacement: Some("z"),
                safe: true,
            }),
            Box::new(Fixer {
                name: "Style/A",
                method: b"before",
                replacement: Some("a"),
                safe: true,
            }),
        ];
        if reverse {
            cops.reverse();
        }
        let (dir, config) = project("before\n");
        let report = run_project(dir.path(), &config, &Registry::from_cops(cops), true);
        assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), b"a\n");
        assert_eq!(report.corrections_applied, 1);
        assert_eq!(report.offense_count(), 0);
    }
}

#[test]
fn syntax_errors_and_cop_panics_prevent_rewrites() {
    for source in ["before(\n", "before\nboom\n"] {
        let mut registry = registry();
        // A separate registry includes a cop which panics on `boom`.
        if source.contains("boom") {
            registry = Registry::from_cops(vec![
                Box::new(Fixer {
                    name: "Style/SafeFix",
                    method: b"before",
                    replacement: Some("after"),
                    safe: true,
                }),
                Box::new(crate::cops::testing::PanicOnCall),
            ]);
        }
        let (dir, config) = project(source);
        let report = run_project(dir.path(), &config, &registry, true);
        assert!(report.has_errors());
        assert_eq!(report.corrections_applied, 0);
        assert_eq!(fs::read_to_string(dir.path().join("x.rb")).unwrap(), source);
    }
}

#[test]
fn corrections_producing_invalid_ruby_are_not_written() {
    let registry = Registry::from_cops(vec![Box::new(Fixer {
        name: "Style/Broken",
        method: b"before",
        replacement: Some("("),
        safe: true,
    })]);
    let (dir, config) = project("before\n");
    let report = run_project(dir.path(), &config, &registry, true);
    assert!(report.has_errors());
    assert_eq!(report.corrections_applied, 0);
    assert_eq!(report.offense_count(), 1);
    assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), b"before\n");
}

#[test]
fn correction_span_cannot_edit_a_disabled_line_outside_the_highlight() {
    struct WideFix;
    impl Cop for WideFix {
        fn name(&self) -> &'static str {
            "Style/WideFix"
        }
        fn safe_fixes(&self) -> bool {
            true
        }
        fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "wide fix".into(),
                line: 1,
                end_line: 1,
                col: 1,
                correction: Some(Correction {
                    range: 0..file.bytes.len(),
                    replacement: "after\n".into(),
                }),
            });
        }
    }
    for prefix in ["rubocop", "rustocop"] {
        for tail in [
            format!("before # {prefix}:disable Style/WideFix\n"),
            format!("# {prefix}:disable Style/WideFix\nbefore\n# {prefix}:enable Style/WideFix\n"),
            format!("# {prefix}:todo Style/WideFix\nbefore\n"),
            format!("# {prefix}:disable-next Style/WideFix\nbefore\n"),
            format!("# {prefix}:todo-next Style/WideFix\nbefore\n"),
            format!("# {prefix}:next -Style/WideFix\nbefore\n"),
            format!("# {prefix}:push -Style/WideFix\nbefore\n# {prefix}:pop\n"),
            format!("# {prefix}:push\n# {prefix}:disable Style/WideFix\nbefore\n# {prefix}:pop\n"),
            format!(
                "# {prefix}:disable Style/WideFix\n# {prefix}:enable-next Style/WideFix\nbefore\nbefore\n"
            ),
            format!(
                "# {prefix}:disable Style/WideFix\n# {prefix}:next +Style/WideFix\nbefore\nbefore\n"
            ),
            format!(
                "# {prefix}:disable Style/WideFix\n# {prefix}:push +Style/WideFix\nbefore\n# {prefix}:pop\nbefore\n"
            ),
        ] {
            let source = format!("before\n{tail}");
            let (dir, config) = project(&source);
            let report = run_project(
                dir.path(),
                &config,
                &Registry::from_cops(vec![Box::new(WideFix)]),
                true,
            );
            assert_eq!(report.corrections_applied, 0, "{source}");
            assert_eq!(report.offense_count(), 1, "{source}");
            assert!(
                report.results[0].offenses[0].correction.is_none(),
                "{source}"
            );
            assert_eq!(fs::read_to_string(dir.path().join("x.rb")).unwrap(), source);
        }
    }
}

#[test]
fn several_files_and_deferred_fixes_use_a_single_pass() {
    let (dir, config) = project("before\n");
    fs::write(dir.path().join("a.rb"), "before\n").unwrap();
    let registry = Registry::from_cops(vec![
        Box::new(Fixer {
            name: "Style/First",
            method: b"before",
            replacement: Some("after"),
            safe: true,
        }),
        Box::new(Fixer {
            name: "Style/Second",
            method: b"after",
            replacement: Some("done"),
            safe: true,
        }),
    ]);
    let report = run_single_pass(dir.path(), &config, &registry);
    assert_eq!(report.files_inspected, 2);
    assert_eq!(report.corrections_applied, 2);
    assert_eq!(report.offense_count(), 2);
    assert_eq!(report.results[0].path, PathBuf::from("a.rb"));
    for path in ["a.rb", "x.rb"] {
        assert_eq!(fs::read(dir.path().join(path)).unwrap(), b"after\n");
    }
}

#[test]
fn write_failure_reports_an_error_and_leaves_original_offenses() {
    let (dir, config) = project("before\n");
    // Use a directory as the write destination, but supply source bytes to
    // exercise the same write-error path without depending on user privileges.
    let report = lint_one_with_fix(
        dir.path(),
        Path::new("x.rb"),
        Some(b"before\n"),
        &CopSelection::new(&registry(), &config, &[], &[]),
        &config,
        Some(Passes {
            max: MAX_CORRECTION_PASSES,
            explicit: false,
        }),
    );
    assert_eq!(report.corrections_applied, 0);
    assert_eq!(report.result.offenses.len(), 1);
    assert!(
        report.errors[0]
            .message
            .starts_with("could not write corrections:")
    );
    assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), b"before\n");
}

#[test]
fn atomic_writer_rejects_stale_sources() {
    let (dir, _) = project("newer source\n");
    assert!(write_corrected(&dir.path().join("x.rb"), b"before\n", b"after\n").is_err());
    assert_eq!(
        fs::read(dir.path().join("x.rb")).unwrap(),
        b"newer source\n"
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn preserves_permissions_and_symlinks_and_coordinates_alias_writes() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (dir, config) = project("before\n");
    let path = dir.path().join("x.rb");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
    let link = dir.path().join("link.rb");
    symlink(&path, &link).unwrap();
    let report = run(
        &[path.clone(), link.clone()],
        &RunOptions {
            fix: true,
            ..Default::default()
        },
        &config,
        &registry(),
    )
    .unwrap();
    assert_eq!(report.files_inspected, 2);
    assert_eq!(report.corrections_applied, 1);
    assert_eq!(fs::read(&path).unwrap(), b"after\n");
    assert!(fs::symlink_metadata(link).unwrap().is_symlink());
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o750
    );
}

#[cfg(unix)]
#[test]
fn alias_fix_collection_requests_semicolon_corrections_without_writing_early() {
    let (dir, config) = project("x = 1;\n");
    std::os::unix::fs::symlink("x.rb", dir.path().join("alias.rb")).unwrap();
    let registry = Registry::from_cops(vec![Box::new(crate::cops::style::Semicolon)]);
    let report = run(
        &[dir.path().join("x.rb"), dir.path().join("alias.rb")],
        &RunOptions {
            fix: true,
            ..Default::default()
        },
        &config,
        &registry,
    )
    .unwrap();
    assert!(!report.has_errors(), "{report:?}");
    assert_eq!(report.files_inspected, 2);
    assert_eq!(report.corrections_applied, 1);
    assert_eq!(report.offense_count(), 0);
    assert_eq!(fs::read(dir.path().join("x.rb")).unwrap(), b"x = 1\n");
}

#[cfg(unix)]
#[test]
fn alias_fixes_merge_path_eligibility_in_one_pass_and_relint_every_path() {
    // Neither alias alone permits both fixes. A second pass through aliases
    // would also apply Deferred, contrary to the single-pass contract.
    let registry = Registry::from_cops(vec![
        Box::new(Fixer {
            name: "Style/First",
            method: b"before",
            replacement: Some("after"),
            safe: true,
        }),
        Box::new(Fixer {
            name: "Style/Other",
            method: b"old",
            replacement: Some("longer"),
            safe: true,
        }),
        Box::new(Fixer {
            name: "Style/Deferred",
            method: b"after",
            replacement: Some("done"),
            safe: true,
        }),
        Box::new(Fixer {
            name: "Lint/Unfixable",
            method: b"unfixable",
            replacement: None,
            safe: false,
        }),
    ]);
    for reverse in [false, true] {
        let (dir, mut config) = project("before; old; unfixable\n");
        let alias = dir.path().join("a.rb");
        std::os::unix::fs::symlink("x.rb", &alias).unwrap();
        for (cop, excluded) in [
            ("Style/First", "a.rb"),
            ("Style/Other", "x.rb"),
            ("Lint/Unfixable", "a.rb"),
        ] {
            config.cop_settings.insert(
                cop.into(),
                crate::config::CopSettings {
                    excludes: crate::config::compile_globs(&[excluded], "test").unwrap(),
                    ..Default::default()
                },
            );
        }
        let mut paths = vec![alias, dir.path().join("x.rb")];
        if reverse {
            paths.reverse();
        }
        let report = run(
            &paths,
            &RunOptions {
                fix: true,
                fix_passes: Some(1),
                ..Default::default()
            },
            &config,
            &registry,
        )
        .unwrap();
        assert!(!report.has_errors(), "{report:?}");
        assert_eq!(report.files_inspected, 2);
        assert_eq!(report.corrections_applied, 2);
        assert_eq!(report.offense_count(), 3);
        assert_eq!(report.exit_code(), EXIT_OFFENSES);
        assert_eq!(
            fs::read(dir.path().join("x.rb")).unwrap(),
            b"after; longer; unfixable\n"
        );
        assert_eq!(report.results[0].path, Path::new("a.rb"));
        assert_eq!(report.results[0].offenses.len(), 1);
        assert_eq!(report.results[0].offenses[0].cop, "Style/Deferred");
        assert_eq!(report.results[1].path, Path::new("x.rb"));
        assert_eq!(report.results[1].offenses[1].cop, "Lint/Unfixable");
        assert_eq!(report.results[1].offenses[1].col, 16);
    }
}

#[cfg(unix)]
#[test]
fn alias_union_reuses_main_parse_boundaries_before_selecting_conflicts() {
    let registry = Registry::from_cops(vec![
        Box::new(Fixer {
            name: "Style/First",
            method: b"before",
            replacement: Some("after"),
            safe: true,
        }),
        Box::new(Fixer {
            name: "Style/Other",
            method: b"old",
            replacement: Some("longer"),
            safe: true,
        }),
    ]);
    let source = "p <<~\"#{after+longer}\"\n#{before+old}\n#{after+longer}\n";
    for cop in registry.cops() {
        let individual = lint_str_with_corrections(&[cop.as_ref()], source);
        assert_eq!(individual.len(), 1);
        assert!(individual[0].correction.is_some());
    }
    for reverse in [false, true] {
        let (dir, mut config) = project(source);
        let path = dir.path().join("x.rb");
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let alias = dir.path().join("a.rb");
        std::os::unix::fs::symlink("x.rb", &alias).unwrap();
        for (cop, excluded) in [("Style/First", "a.rb"), ("Style/Other", "x.rb")] {
            config.cop_settings.insert(
                cop.into(),
                crate::config::CopSettings {
                    excludes: crate::config::compile_globs(&[excluded], "test").unwrap(),
                    ..Default::default()
                },
            );
        }
        let mut paths = vec![path.clone(), alias];
        if reverse {
            paths.reverse();
        }
        let report = run(
            &paths,
            &RunOptions {
                fix: true,
                ..Default::default()
            },
            &config,
            &registry,
        )
        .unwrap();
        assert!(!report.has_errors(), "{report:?}");
        assert_eq!(report.corrections_applied, 0);
        assert_eq!(report.offense_count(), 2);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn opening_boundary_changes_never_write_through_pipeline_entries() {
    struct Extension {
        stage: u8,
        removal: bool,
        atomic: bool,
        leading: bool,
    }
    impl Extension {
        fn emit(&self, file: &SourceFile, out: &mut Vec<Offense>) {
            let opening = file.bytes.windows(3).position(|w| w == b"<<A").unwrap();
            let at = opening + 3;
            let (range, replacement) = if self.leading {
                if self.atomic {
                    let replacement = format!(
                        "{}{}",
                        String::from_utf8_lossy(&file.bytes[..opening - 1]),
                        String::from_utf8_lossy(&file.bytes[opening..])
                    );
                    (0..file.bytes.len(), replacement)
                } else {
                    (opening - 1..opening, String::new())
                }
            } else if self.atomic {
                let end = file.bytes.iter().position(|&b| b == b'\n').unwrap();
                // Keep the uniquely matched main-parse opener in the envelope.
                let replacement = format!("{}é", String::from_utf8_lossy(&file.bytes[..at]));
                (0..end, replacement)
            } else if self.removal {
                (at..at + 2, String::new())
            } else {
                (at..at, "é".into())
            };
            let (line, col) = file.line_col(range.start);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                col,
                end_line: line,
                message: "name extension".into(),
                correction: Some(Correction { range, replacement }),
            });
        }
    }
    impl Cop for Extension {
        fn name(&self) -> &'static str {
            "Style/Extension"
        }
        fn safe_fixes(&self) -> bool {
            true
        }
        fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
            if self.stage == 0 {
                self.emit(file, out);
            }
        }
        fn check_node<'pr>(
            &self,
            node: &ruby_prism::Node<'pr>,
            _: &NodeContext<'_, 'pr>,
            file: &SourceFile,
            out: &mut Vec<Offense>,
        ) {
            if self.stage == 1 && node.as_program_node().is_some() {
                self.emit(file, out);
            }
        }
        fn wants_leave_node(&self) -> bool {
            self.stage == 2
        }
        fn leave_node<'pr>(
            &self,
            node: &ruby_prism::Node<'pr>,
            _: &NodeContext<'_, 'pr>,
            file: &SourceFile,
            out: &mut Vec<Offense>,
        ) {
            if node.as_program_node().is_some() {
                self.emit(file, out);
            }
        }
    }
    for stage in 0..3 {
        for (source, removal, leading) in [
            ("p <<A\ntext\nAé\nA\n", false, false),
            ("p <<A +é\ntext\nAé\nA\n", true, false),
            (
                "def p(x = [])\n  Kernel.p(x)\n  []\nend\nA = 1\np <<A\n1\nA\n",
                true,
                true,
            ),
        ] {
            for atomic in [false, true] {
                let registry = Registry::from_cops(vec![Box::new(Extension {
                    stage,
                    removal,
                    atomic,
                    leading,
                })]);
                let (dir, config) = project(source);
                let path = dir.path().join("x.rb");
                let mtime = fs::metadata(&path).unwrap().modified().unwrap();
                let report = run_project(dir.path(), &config, &registry, true);
                assert!(!report.has_errors(), "{report:?}");
                assert_eq!(report.corrections_applied, 0);
                assert_eq!(report.offense_count(), 1);
                assert!(report.results[0].offenses[0].correction.is_none());
                assert_eq!(fs::read_to_string(&path).unwrap(), source);
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn alias_union_cannot_destroy_heredoc_opening_separators() {
    struct SuffixRemoval {
        second: bool,
        leading: bool,
        atomic: bool,
    }
    impl Cop for SuffixRemoval {
        fn name(&self) -> &'static str {
            if self.second {
                "Style/Other"
            } else {
                "Style/First"
            }
        }
        fn safe_fixes(&self) -> bool {
            true
        }
        fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
            let marker = if self.leading { b"p  <" } else { b"A + " };
            let Some(at) = file.bytes.windows(4).position(|w| w == marker) else {
                return;
            };
            let range = if self.leading {
                let start = at + 1 + usize::from(self.second);
                start..start + 1
            } else if self.second {
                at + 3..at + 4
            } else {
                at + 1..at + 3
            };
            let (range, replacement) = if self.atomic {
                if self.second {
                    let end = range.end
                        + file.bytes[range.end..]
                            .iter()
                            .position(|&b| b == b'\n')
                            .unwrap();
                    (
                        range.start..end,
                        String::from_utf8(file.bytes[range.end..end].to_vec()).unwrap(),
                    )
                } else {
                    (
                        0..range.end,
                        String::from_utf8(file.bytes[..range.start].to_vec()).unwrap(),
                    )
                }
            } else {
                (range, String::new())
            };
            let (line, col) = file.line_col(range.start);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "separator removal".into(),
                line,
                col,
                end_line: line,
                correction: Some(Correction { range, replacement }),
            });
        }
    }
    for leading in [false, true] {
        for atomic in [false, true] {
            let registry = Registry::from_cops(vec![
                Box::new(SuffixRemoval {
                    second: false,
                    leading,
                    atomic,
                }),
                Box::new(SuffixRemoval {
                    second: true,
                    leading,
                    atomic,
                }),
            ]);
            let source = if leading {
                "def p(x = [])\n  Kernel.p(x)\n  []\nend\nA = 1\np  <<A\n1\nA\n"
            } else {
                "A = 1\né = \"\"\np <<A + é\ntext\nAé\nA\n"
            };
            for cop in registry.cops() {
                assert!(
                    lint_str_with_corrections(&[cop.as_ref()], source)[0]
                        .correction
                        .is_some()
                );
            }
            for reverse in [false, true] {
                let (dir, mut config) = project(source);
                let path = dir.path().join("x.rb");
                let alias = dir.path().join("a.rb");
                std::os::unix::fs::symlink("x.rb", &alias).unwrap();
                for (cop, excluded) in [("Style/First", "a.rb"), ("Style/Other", "x.rb")] {
                    config.cop_settings.insert(
                        cop.into(),
                        crate::config::CopSettings {
                            excludes: crate::config::compile_globs(&[excluded], "test").unwrap(),
                            ..Default::default()
                        },
                    );
                }
                let mut paths = vec![path.clone(), alias];
                if reverse {
                    paths.reverse();
                }
                let report = run(
                    &paths,
                    &RunOptions {
                        fix: true,
                        ..Default::default()
                    },
                    &config,
                    &registry,
                )
                .unwrap();
                assert!(!report.has_errors(), "{report:?}");
                assert_eq!(
                    report.corrections_applied, 1,
                    "must retain the opener separator"
                );
                assert_eq!(
                    fs::read_to_string(&path).unwrap(),
                    if leading {
                        source.replace("p  <<", "p <<")
                    } else {
                        source.replace("A + é", "A +é")
                    }
                );
            }
        }
    }
}

#[test]
fn fix_with_stdin_is_rejected_before_reading_or_writing() {
    let options = RunOptions {
        fix: true,
        stdin: Some("x.rb".into()),
        ..Default::default()
    };
    let err = run(&[], &options, &ResolvedConfig::default(), &registry()).unwrap_err();
    assert!(
        err.to_string()
            .contains("--fix cannot be used with --stdin")
    );
}
