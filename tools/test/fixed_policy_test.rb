# frozen_string_literal: true

require "minitest/autorun"
require "minitest/mock"
require_relative "../fixed_policy"

class FixedPolicyTest < Minitest::Test
  def test_all_style_enables_and_pending_states
    records = FixedPolicy::STYLE.values
    assert_equal 299, records.size
    records.each do |record|
      name = record.fetch("name")
      expected = record.fetch("upstream_enabled") != false &&
                 !%w[Style/Documentation Style/FrozenStringLiteralComment].include?(name)
      assert_equal expected, FixedPolicy.default_enabled(name), name
    end
    assert_equal 264, records.count { |r| FixedPolicy.default_enabled(r.fetch("name")) }
    refute FixedPolicy.default_enabled('Metrics/AbcSize')
    refute FixedPolicy.default_enabled('Naming/InclusiveLanguage')
    assert FixedPolicy.default_enabled('Naming/PredicateMethod')
  end

  def test_reference_config_is_limited_to_real_cops_and_keeps_all_eight_choices
    names = FixedPolicy.implemented_cops
    config = FixedPolicy.reference_config(names)
    style = names.grep(/^Style\//)
    assert_equal (style + names.grep(/^Lint\//)).sort, config.keys.sort
    existing = %w[Style/StringLiterals Style/StringLiteralsInInterpolation Style/HashSyntax
                  Style/RedundantReturn Style/Semicolon Style/RedundantParentheses
                  Style/SignalException Style/FrozenStringLiteralComment]
    assert_empty existing - style
    assert_equal "double_quotes", config.dig("Style/StringLiterals", "EnforcedStyle")
    assert_equal "double_quotes", config.dig("Style/StringLiteralsInInterpolation", "EnforcedStyle")
    assert_equal "ruby19", config.dig("Style/HashSyntax", "EnforcedStyle")
    assert_equal "either", config.dig("Style/HashSyntax", "EnforcedShorthandSyntax")
    assert_equal "only_raise", config.dig("Style/SignalException", "EnforcedStyle")
    assert_equal "always", config.dig("Style/FrozenStringLiteralComment", "EnforcedStyle")
    assert_equal false, config.dig("Style/FrozenStringLiteralComment", "Enabled")
    assert_equal false, config.dig("Style/RedundantReturn", "AllowMultipleReturnValues")
    assert_equal false, config.dig("Style/Semicolon", "AllowAsExpressionSeparator")
    assert_equal true, config.dig("Style/RedundantParentheses", "Enabled")
    assert_equal true, config.dig("Style/ReverseFind", "Enabled")
    assert_equal true, config.dig("Style/ObjectThen", "Enabled")
    assert_equal "then", FixedPolicy::STYLE.fetch("Style/ObjectThen").fetch("upstream_defaults").fetch("EnforcedStyle")
    assert_equal true, config.dig("Style/InverseMethods", "Enabled")
    assert_equal true, config.dig("Style/MultipleComparison", "Enabled")
    %w[Style/NilComparison Style/NonNilCheck Style/Not].each do |cop|
      assert_equal true, config.dig(cop, "Enabled")
    end
    assert_equal "predicate", FixedPolicy::STYLE.fetch("Style/NilComparison").fetch("upstream_defaults").fetch("EnforcedStyle")
    assert_equal false, FixedPolicy::STYLE.fetch("Style/NonNilCheck").fetch("upstream_defaults").fetch("IncludeSemanticChanges")
    %w[Style/NegatedIf Style/NegatedUnless Style/NegatedIfElseCondition].each do |cop|
      assert_equal true, config.dig(cop, "Enabled")
    end
    %w[Style/NegatedIf Style/NegatedUnless].each do |cop|
      assert_equal "both", FixedPolicy::STYLE.fetch(cop).fetch("upstream_defaults").fetch("EnforcedStyle")
    end
    assert_equal true, config.dig("Style/NestedModifier", "Enabled")
    assert_equal true, config.dig("Style/NestedTernaryOperator", "Enabled")
    %w[Style/NegatedWhile Style/WhileUntilDo Style/WhileUntilModifier].each do |cop|
      assert_equal true, config.dig(cop, "Enabled")
    end
    assert_equal false, config.dig("Style/ArrayCoercion", "Enabled")
    assert_equal false, config.dig("Style/ArrayFirstLast", "Enabled")
    assert_equal true, config.dig("Style/ArrayIntersect", "Enabled")
  end

  def test_landed_top_level_return_mandatory_jb_path_exclusion
    %w[a.jb lib/a.jb lib/.hidden.jb].each { |p| refute FixedPolicy.path_allowed?('Lint/TopLevelReturnWithArgument',p),p }
    %w[a.rb lib/a.rb a.jb.rb Gemfile].each { |p| assert FixedPolicy.path_allowed?('Lint/TopLevelReturnWithArgument',p),p }
  end

  def test_only_landed_lint_names_get_explicit_pinned_defaults
    names = FixedPolicy.implemented_cops
    assert_equal 613, names.size
    assert_equal 160, names.grep(/^Lint\//).size
    %w[Lint/ScriptPermission Lint/AmbiguousAssignment Lint/AmbiguousOperatorPrecedence Lint/CopDirectiveSyntax Lint/RedundantCopEnableDirective Lint/RedundantCopDisableDirective Lint/MissingCopEnableDirective Lint/EmptyBlock Lint/EmptyClass Lint/EmptyExpression Lint/EmptyFile Lint/EmptyInPattern Lint/ArgumentMismatch Lint/SuperArgumentMismatch Lint/CircularArgumentReference Lint/DataDefineOverride Lint/SharedMutableDefault Lint/ConstantDefinitionInBlock Lint/ConstantOverwrittenInRescue Lint/ConstantReassignment Lint/UselessConstantScoping Lint/OrAssignmentToConstant Lint/DisjunctiveAssignmentInConstructor Lint/DeprecatedConstants Lint/DeprecatedOpenSSLConstant Lint/DeprecatedReference Lint/BigDecimalNew Lint/ErbNewArguments Lint/UnifiedInteger Lint/UriEscapeUnescape Lint/UriRegexp Lint/DuplicateBranch Lint/DuplicateMatchPattern Lint/DuplicateRegexpCharacterClassElement Lint/DuplicateRequire Lint/DuplicateMagicComment Lint/MisplacedMagicComment Lint/OrderedMagicComments Lint/ArrayLiteralInRegexp Lint/BooleanSymbol Lint/MixedRegexpCaptureTypes Lint/NestedPercentLiteral Lint/UnescapedBracketInRegexp Lint/RedundantRegexpQuantifiers Lint/FloatComparison Lint/FloatOutOfRange Lint/EachWithObjectArgument Lint/FlipFlop Lint/HashCompareByIdentity Lint/HashNewWithKeywordArgumentsAsDefault Lint/ElseLayout Lint/TrailingCommaInAttributeDeclaration Lint/IdentityComparison Lint/ImplicitStringConcatenation Lint/InterpolationCheck Lint/TripleQuotes Lint/IncompatibleIoSelectWithFiberScheduler Lint/IneffectiveAccessModifier Lint/UselessAccessModifier Lint/LambdaWithoutLiteralBlock Lint/LiteralAssignmentInCondition Lint/Loop Lint/NoReturnInBeginEndBlocks Lint/MissingSuper Lint/MixedCaseRange Lint/NumberedParameterAssignment Lint/NameTypo Lint/NextWithoutAccumulator Lint/UnmodifiedReduceAccumulator Lint/NonAtomicFileOperation Lint/NumericOperationWithConstantResult Lint/ParenthesesAsGroupedExpression Lint/PercentStringArray Lint/PercentSymbolArray Lint/RedundantDirGlobSort Lint/RedundantSafeNavigation Lint/RedundantSplatExpansion Lint/RedundantStringCoercion Lint/RedundantTypeConversion Lint/RedundantWithIndex Lint/RedundantWithObject Lint/RefinementImportMethods Lint/RegexpAsCondition Lint/RequireParentheses Lint/RequireRangeParentheses Lint/RequireRelativeSelfPath Lint/RescueType Lint/ReturnInVoidContext Lint/SafeNavigationChain Lint/SafeNavigationConsistency Lint/SafeNavigationWithEmpty Lint/SendWithMixinArgument Lint/ShadowedArgument Lint/ShadowedException Lint/SuppressedExceptionInNumberConversion Lint/SymbolConversion Lint/TopLevelReturnWithArgument Lint/UnderscorePrefixedVariableName Lint/UnexpectedBlockArity Lint/UnreachableLoop Lint/UnreachablePatternBranch Lint/UnusedBlockArgument Lint/UnusedMethodArgument Lint/UselessDefaultValueArgument Lint/UselessDefined Lint/UselessRescue Lint/UselessMethodDefinition Lint/UselessRuby2Keywords Lint/UselessNumericOperation Lint/UselessOr Lint/UselessSetterCall Lint/UselessTimes].each do |name|
      assert FixedPolicy.default_enabled(name)
      assert_equal({ 'Enabled' => true }, FixedPolicy.reference_config(names).fetch(name))
    end
    refute FixedPolicy.default_enabled('Lint/ShadowingOuterLocalVariable')
    assert_equal({ 'Enabled' => false }, FixedPolicy.reference_config(names).fetch('Lint/ShadowingOuterLocalVariable'))
    refute FixedPolicy.default_enabled('Lint/NumberConversion')
    assert_equal({ 'Enabled' => false }, FixedPolicy.reference_config(names).fetch('Lint/NumberConversion'))
    refute FixedPolicy.default_enabled('Lint/HeredocMethodCallPosition')
    assert_equal({ 'Enabled' => false }, FixedPolicy.reference_config(names).fetch('Lint/HeredocMethodCallPosition'))
    refute FixedPolicy.default_enabled('Lint/ConstantResolution')
    assert_equal({ 'Enabled' => false }, FixedPolicy.reference_config(names).fetch('Lint/ConstantResolution'))
    refute FixedPolicy.default_enabled('Lint/UnusedPrivateMethod')
    assert_equal({ 'Enabled' => false }, FixedPolicy.reference_config(names).fetch('Lint/UnusedPrivateMethod'))
    %w[Lint/ToEnumArguments Lint/ToJSON].each do |name|
      assert FixedPolicy.default_enabled(name)
      assert_equal({ 'Enabled' => true }, FixedPolicy.reference_config(names).fetch(name))
    end
    unimplemented = %w[Lint/NotInventoried]
    assert_empty FixedPolicy.reference_config(names + unimplemented).keys & unimplemented
  end

  def test_syntax_defaults_are_read_only_once_implemented
    names = FixedPolicy.implemented_cops
    FixedPolicy.stub(:implemented_cops, names - ['Lint/Syntax']) do
      refute FixedPolicy.reference_config(['Lint/Syntax']).key?('Lint/Syntax')
    end
    FixedPolicy.stub(:implemented_cops, names) do
      assert_equal({ 'Enabled' => true }, FixedPolicy.reference_config(['Lint/Syntax']).fetch('Lint/Syntax'))
      assert FixedPolicy.default_enabled('Lint/Syntax')
      # Unrequested names are never added, even when live.
      assert_empty FixedPolicy.reference_config([])
    end
    assert_includes FixedPolicy.implemented_cops, 'Lint/Syntax'
    assert FixedPolicy.reference_config(['Lint/Syntax']).key?('Lint/Syntax')
  end

  def test_live_lint_metadata_must_exist_rather_than_silently_inventing_defaults
    names = FixedPolicy.implemented_cops
    FixedPolicy.stub(:implemented_cops, names + ['Lint/NotInventoried']) do
      assert_raises(KeyError) { FixedPolicy.reference_config(['Lint/NotInventoried']) }
    end
  end

  def test_baseline_lint_default_policies_are_unchanged
    inventory = JSON.parse(File.read(File.join(FixedPolicy::ROOT, 'tools/oracle/lint_reference.json')))
    records = inventory.fetch('cops').select { |record| record.fetch('existing') }
    assert_equal 35, records.size
    records.each do |record|
      name = record.fetch('name')
      assert_equal record.fetch('default_enabled'), FixedPolicy.default_enabled(name), name
      assert_equal({ 'Enabled' => true }, FixedPolicy.reference_config([name]).fetch(name))
    end
  end

  def test_fixed_version_and_version_mismatch_reasons
    assert_equal 3.4, FixedPolicy::TARGET_RUBY
    assert_nil FixedPolicy.ruby_mismatch("Style/ItBlockParameter", 3.4)
    assert_match(/Ruby applicability/, FixedPolicy.ruby_mismatch("Style/ItBlockParameter", 3.3))
    assert_equal [3.4, 4.0], FixedPolicy::SUPPORTED_RUBY
    assert_equal 4.0, FixedPolicy.run_ruby(4.0)
    assert_equal 3.4, FixedPolicy.run_ruby(3.3)
    assert_equal 3.4, FixedPolicy.run_ruby(4.1)
    assert_nil FixedPolicy.ruby_mismatch("Style/ReverseFind", 4.0)
    assert_match(/requires Ruby >= 4.0/, FixedPolicy.ruby_mismatch("Style/ReverseFind", 3.4))
    assert_nil FixedPolicy.ruby_mismatch("Style/HashSlice", 3.3)
    assert_nil FixedPolicy.ruby_mismatch("Lint/Debugger", 3.3)
    assert_nil FixedPolicy.ruby_mismatch('Lint/SafeNavigationChain', 3.4)
    assert_nil FixedPolicy.ruby_mismatch('Lint/SafeNavigationChain', 2.3)
    assert_match(/spec Ruby 2.2/, FixedPolicy.ruby_mismatch('Lint/SafeNavigationChain', 2.2))
    assert_nil FixedPolicy.ruby_mismatch('Lint/RefinementImportMethods', 3.1)
    assert_match(/spec Ruby 3.0/, FixedPolicy.ruby_mismatch('Lint/RefinementImportMethods', 3.0))
    assert_nil FixedPolicy.ruby_mismatch('Lint/RedundantDirGlobSort', 3.0)
    assert_match(/spec Ruby 2.7/, FixedPolicy.ruby_mismatch('Lint/RedundantDirGlobSort', 2.7))
  end

  def test_only_landed_lint_version_consumers_have_pinned_applicability
    {
      'Lint/ItWithoutArgumentsInBlock' => 3.3,
      'Lint/UselessElseWithoutRescue' => 2.5,
      'Lint/NonDeterministicRequireOrder' => 2.7
    }.each do |name, max|
      assert_match(/requires Ruby <= #{max}/, FixedPolicy.ruby_mismatch(name, max))
      assert_match(/runs this example at 3.4/, FixedPolicy.ruby_mismatch(name, 3.4))
    end
    names = FixedPolicy.implemented_cops
    FixedPolicy.stub(:implemented_cops, names - ['Lint/ItWithoutArgumentsInBlock']) do
      assert_nil FixedPolicy.ruby_mismatch('Lint/ItWithoutArgumentsInBlock', 3.3)
    end
  end

  def test_filename_exclusion_is_exact_and_root_relative_not_a_basename_glob
    assert FixedPolicy.default_enabled('Naming/FileName')
    refute FixedPolicy.path_allowed?('Naming/FileName', 'Rakefile.rb')
    refute FixedPolicy.path_allowed?('Naming/FileName', './Rakefile.rb')
    %w[lib/Rakefile.rb Rakefile.rb.extra rakeFile.rb lib/../Rakefile.rb].each do |path|
      assert FixedPolicy.path_allowed?('Naming/FileName', path), path
    end
  end

  def test_meaningful_pinned_path_eligibility_is_root_relative
    assert FixedPolicy.path_allowed?("Style/Documentation", "lib/a.rb")
    refute FixedPolicy.path_allowed?("Style/Documentation", "spec/a.rb")
    refute FixedPolicy.path_allowed?("Style/Documentation", "test/deep/a.rb")
    assert FixedPolicy.path_allowed?("Style/Documentation", "specification/a.rb")
    refute FixedPolicy.path_allowed?("Style/FrozenStringLiteralComment", "a.arb")
    refute FixedPolicy.path_allowed?("Style/FrozenStringLiteralComment", "lib/a.arb")
    assert FixedPolicy.path_allowed?("Lint/Debugger", "spec/a.rb")
  end
end
