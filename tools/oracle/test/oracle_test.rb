# frozen_string_literal: true

# Tests for the oracle's own machinery: ruby tools/oracle/test/oracle_test.rb
# Needs what the oracle needs (a RuboCop checkout and its runtime gems), and
# exits with Oracle::EXIT_UNAVAILABLE without them.

require_relative "../oracle"
Oracle.load_rubocop(Oracle.locate_rubocop(nil))

require "minitest/autorun"
require "tmpdir"

class OrdinaryLintPolicyTest < Minitest::Test
  def test_landed_and_next_consumer_source_main_spec_and_config_match_the_pin
    inventory = JSON.parse(File.read(File.join(Oracle::ROOT, 'tools/oracle/lint_reference.json')))
    src = Oracle.locate_rubocop(nil)
    assert_equal inventory.fetch('reference').fetch('revision'), Oracle.rubocop_revision(src)
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    %w[Lint/AmbiguousAssignment Lint/AmbiguousOperatorPrecedence Lint/CopDirectiveSyntax Lint/Syntax].each do |name|
      record = inventory.fetch('cops').find { |entry| entry.fetch('name') == name }
      assert_equal record.fetch('upstream_defaults'), defaults.fetch(name)
      mapping = Oracle.source_and_specs(src, name)
      paths = [mapping.fetch(:source), *mapping.fetch(:specs)].map { |path| path.delete_prefix("#{src}/") }
      paths.each do |path|
        assert_equal inventory.fetch('reference_files').fetch(path), Digest::SHA256.file(File.join(src, path)).hexdigest, path
      end
      assert_empty record.fetch('path_policy').fetch('include')
      assert_empty record.fetch('path_policy').fetch('exclude')
      assert_nil record.fetch('minimum_ruby')
      assert_nil record.fetch('maximum_ruby')
    end
    assert_equal 'pending', defaults.fetch('Lint/AmbiguousAssignment').fetch('Enabled')
    assert_equal 'pending', defaults.fetch('Lint/AmbiguousOperatorPrecedence').fetch('Enabled')
    assert_equal true, defaults.fetch('Lint/Syntax').fetch('Enabled')
    names = FixedPolicy.implemented_cops
    FixedPolicy.stub(:implemented_cops, names + ['Lint/Syntax']) do
      assert_equal({ 'Lint/Syntax' => { 'Enabled' => true } }, FixedPolicy.reference_config(['Lint/Syntax']))
    end
  end
end

class AccessorNamingSettingsTest < Minitest::Test
  def test_source_irrelevant_prefix_and_macro_lists_do_not_exclude_applicable_cases
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    cop = 'Naming/PredicatePrefix'
    config = {cop => defaults.fetch(cop).merge('NamePrefix' => %w[is_ has_], 'ForbiddenPrefixes' => %w[is_ has_])}
    ['def is_attr; end', 'def is_2d?; end', 'def is_hello=; end', 'def other; end'].each do |source|
      assert_nil Oracle.config_mismatch(cop, config, defaults, source).first, source
    end
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'def have_attr; end').first
    config = {cop => defaults.fetch(cop).merge('MethodDefinitionMacros' => %w[define_method def_node_matcher])}
    assert_nil Oracle.config_mismatch(cop, config, defaults, 'define_method(:is_hello) {}').first
    assert_nil Oracle.config_mismatch(cop, config, defaults, 'self.def_node_matcher(:is_hello)').first
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'def_node_matcher(:is_hello)').first
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'define_singleton_method(:is_hello) {}').first
    config = {cop => defaults.fetch(cop).merge('ForbiddenPrefixes' => [])}
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'def is_attr; end').first
    config = {cop => defaults.fetch(cop).merge('UseSorbetSigs' => true)}
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'def is_attr; end').first
  end
end

class IdentifierNamingSettingsTest < Minitest::Test
  def test_all_pinned_calls_and_each_exclusion_are_accounted_for
    # RuboCop specs: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    calls = []; excluded = []
    %w[Naming/AsciiIdentifiers Naming/BlockParameterName Naming/VariableNumber].each do |cop|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      recorder.expectations.each do |e|
        reasons, toml = e.skip ? [nil, {}] : Oracle.config_mismatch(cop, e.config, defaults, e.source)
        row = {'cop' => cop, 'spec_line' => e.spec_line, 'description' => e.description, 'source' => e.source}
        if e.skip || reasons
          excluded << row.merge('reason' => e.skip || reasons)
        else
          assert_empty toml
          calls << row.merge('positions' => e.offenses.map { |o| [o.line, o.col] }, 'messages' => e.offenses.map(&:message))
        end
      end
    end
    assert_equal [12, 7, 19], calls.group_by { |r| r['cop'] }.values.map(&:size)
    assert_equal [6, 99], excluded.group_by { |r| r['cop'] }.values.map(&:size)
    fixture = JSON.parse(File.read(File.expand_path('../../../tests/fixtures/ns02_pinned.json', __dir__)))
    assert_equal({'calls' => calls, 'excluded' => excluded}, fixture)
  end

  def test_only_source_irrelevant_block_name_and_ascii_settings_are_projected
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    cop = 'Naming/BlockParameterName'
    config = {cop => defaults[cop].merge('MinNameLength' => 2, 'AllowNamesEndingInNumbers' => false)}
    ['foo { |number_One| }', 'foo { |foo, bar| }', 'foo { work }'].each do |source|
      assert_nil Oracle.config_mismatch(cop, config, defaults, source).first
    end
    ['foo { |x| }', 'foo { |foo1| }', 'foo { |; __| }'].each do |source|
      refute_nil Oracle.config_mismatch(cop, config, defaults, source).first
    end
    refute_nil Oracle.config_mismatch(cop, {cop => {}}, defaults, 'foo { |a| }').first
    config = {cop => defaults[cop].merge('ForbiddenNames' => ['arg'])}
    assert_nil Oracle.config_mismatch(cop, config, defaults, 'foo { |foo_arg| }').first
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'foo { |arg| }').first
    cop = 'Naming/AsciiIdentifiers'
    config = {cop => defaults[cop].merge('AsciiConstants' => false)}
    assert_nil Oracle.config_mismatch(cop, config, defaults, 'älg = 1').first
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'class Foö; end').first
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'class É; end').first
    assert_nil Oracle.config_mismatch(cop, config, defaults, ':Foö; "É"; # Bär').first
  end
end

class SafeNavigationSettingsTest < Minitest::Test
  def test_every_pinned_safe_navigation_call_correction_and_skip_is_accounted_for
    # RuboCop specs: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    cop = 'Style/SafeNavigation'
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    assert_empty recorder.skipped
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
    assert_equal 707, matching.size
    assert_equal 493, matching.count { |e| corrections.key?(e.object_id) }
    assert_equal 1, skipped.count { |e| e.skip == 'not run on Prism' }
    config_skips = skipped.reject(&:skip)
    assert_equal 90, config_skips.size
    assert_equal 36, config_skips.count { |e| e.config.dig(cop, 'MaxChainLength') == 1 }
    assert_equal 18, config_skips.count { |e| e.config.dig(cop, 'MaxChainLength') == 3 }
    assert_equal 12, config_skips.count { |e| e.config.dig('Lint/SafeNavigationChain', 'Enabled') == false }
    assert_equal 24, config_skips.count { |e| Oracle.config_mismatch(cop, e.config, defaults, e.source).first == ["#{cop} ConvertCodeThatCanStartToReturnNil=true"] }
    expected = matching.map do |e|
      {'spec_line' => e.spec_line, 'source' => e.source,
       'positions' => e.offenses.map { |o| [o.line, o.col] },
       'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/safe_navigation_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end

  def test_convert_nil_setting_is_irrelevant_without_negated_nil_conjunction
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    config = {'Style/SafeNavigation' => defaults.fetch('Style/SafeNavigation').merge('ConvertCodeThatCanStartToReturnNil' => true)}
    ['x && x.foo', 'x.bar && x.bar.baz', 'x.bar if !x.nil?', 'x && x.foo { !y.nil? }'].each do |source|
      mismatch, toml = Oracle.config_mismatch('Style/SafeNavigation', config, defaults, source)
      assert_nil mismatch, source
      assert_empty toml
    end
    ['!x.nil? && x.foo', 'a && (!x.nil? && x.foo)', 'x && (!y.nil? && y.foo)'].each do |source|
      mismatch, = Oracle.config_mismatch('Style/SafeNavigation', config, defaults, source)
      refute_nil mismatch, source
    end
    mismatch, = Oracle.config_mismatch('Style/SafeNavigation',
      {'Style/SafeNavigation' => {'MaxChainLength' => 3}}, defaults, 'x && x.a.b.c')
    refute_nil mismatch
  end
end

class DigCompanionSettingsTest < Minitest::Test
  def test_every_pinned_dig_call_and_unsafe_correction_is_retained
    # RuboCop specs: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [['SingleArgumentDig', 'single_argument_dig', 15, 3],
     ['DigChain', 'dig_chain', 24, 13]].each do |name, snake, calls, fixes|
      cop = "Style/#{name}"
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      assert_equal calls, recorder.expectations.size
      assert_equal fixes, recorder.expectations.count { |e| corrections.key?(e.object_id) }
      expected = recorder.expectations.map do |e|
        assert_nil e.skip
        mismatch, toml = Oracle.config_mismatch(cop, e.config, defaults, e.source)
        assert_nil mismatch
        {'cop' => cop, 'spec_line' => e.spec_line, 'description' => e.description, 'source' => e.source,
         'positions' => e.offenses.map { |o| [o.line, o.col] },
         'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id], 'toml' => toml}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{snake}_pinned.json", __dir__)))
    end
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end

  def test_dig_chain_boolean_config_is_executed_not_skipped
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [true, false].each do |enabled|
      mismatch, toml = Oracle.config_mismatch('Style/SingleArgumentDig',
        {'Style/DigChain' => {'Enabled' => enabled}}, defaults, 'x.dig(a).dig(b)')
      assert_nil mismatch
      assert_equal({'Style/DigChain' => {'enabled' => enabled}}, toml)
    end
    mismatch, = Oracle.config_mismatch('Style/DigChain',
      {'Style/SingleArgumentDig' => {'Enabled' => false}}, defaults, 'x.dig(a).dig(b)')
    refute_nil mismatch # Narrow projection: not a generic other-cop override.
  end
end

class FreezeInitializeSelfFixtureTest < Minitest::Test
  def test_every_pinned_call_correction_and_justified_skip_is_retained
    # RuboCop specs: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [['RedundantFreeze', 'redundant_freeze', 50, 18, 3, 15],
     ['RedundantInitialize', 'redundant_initialize', 22, 5, 1, 0],
     ['RedundantSelf', 'redundant_self', 65, 12, 0, 0]].each do |name, snake, calls, fixes, config_skips, prism_skips|
      cop = "Style/#{name}"
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal calls, matching.size
      assert_equal fixes, matching.count { |e| corrections.key?(e.object_id) }
      assert_equal prism_skips, skipped.count { |e| e.skip == 'not run on Prism' }
      assert_equal config_skips, skipped.count { |e| !e.skip }
      skipped.each do |e|
        if e.skip
          assert_equal 'not run on Prism', e.skip
        else
          reasons = Oracle.config_mismatch(cop, e.config, defaults, e.source).first
          assert_equal [name == 'RedundantFreeze' ? 'AllCops StringLiteralsFrozenByDefault=true' : 'Style/RedundantInitialize AllowComments=false'], reasons
        end
      end
      expected = matching.map do |e|
        _, toml = Oracle.config_mismatch(cop, e.config, defaults, e.source)
        {'cop' => cop, 'spec_line' => e.spec_line, 'description' => e.description, 'source' => e.source,
         'positions' => e.offenses.map { |o| [o.line, o.col] },
         'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id], 'toml' => toml}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{snake}_pinned.json", __dir__)))
    end
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class RemainingRedundancyPolicyTest < Minitest::Test
  def test_all_pinned_calls_and_corrections_are_retained_with_exact_skip_reasons
    # RuboCop specs: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [['EmptyMethod', 'empty_method', 19, 9, 16, 0],
     ['RedundantConstantBase', 'redundant_constant_base', 7, 5, 1, 3],
     ['RedundantFetchBlock', 'redundant_fetch_block', 21, 9, 1, 0]].each do |name, snake, calls, fixes, skips, groups|
      cop = "Style/#{name}"
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_equal groups, recorder.skipped.size
      recorder.skipped.each { |_, reason| assert_equal 'project index', reason }
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal calls, matching.size
      assert_equal fixes, matching.count { |e| corrections[e.object_id] }
      assert_equal skips, skipped.size
      skipped.each do |e|
        assert_nil e.skip
        reasons = Oracle.config_mismatch(cop, e.config, defaults, e.source).first
        assert reasons.all? { |r| r.include?('EnforcedStyle="expanded"') || r == 'Style/SingleLineMethods AllowIfMethodIsEmpty=false' || r == 'Lint/ConstantResolution Enabled=true' || r == 'Style/RedundantFetchBlock SafeForConstants=true' }, reasons.inspect
      end
      expected = matching.map do |e|
        _, toml = Oracle.config_mismatch(cop, e.config, defaults, e.source)
        {'cop' => cop, 'spec_line' => e.spec_line, 'source' => e.source,
         'positions' => e.offenses.map { |o| [o.line, o.col] },
         'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id], 'toml' => toml}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{snake}_pinned.json", __dir__)))
    end
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end

  def test_fetch_constants_setting_is_projected_only_for_nonconstant_bodies_and_empty_method_reads_line_length
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    cop = 'Style/RedundantFetchBlock'
    config = {cop => defaults.fetch(cop).merge('SafeForConstants' => true)}
    ["hash.fetch(:key) { 5 }", "hash.fetch(:key) {}", "hash.fetch(:key) { 'x' }"].each do |source|
      assert_nil Oracle.config_mismatch(cop, config, defaults, source).first
    end
    ["hash.fetch(:key) { VALUE }", "hash.fetch(:key) { ::X::VALUE }", "hash.fetch(:key) { __ENCODING__ }"].each do |source|
      assert_equal ['Style/RedundantFetchBlock SafeForConstants=true'], Oracle.config_mismatch(cop, config, defaults, source).first
    end
    refute_nil Oracle.config_mismatch(cop, config, defaults, nil).first
    reason, toml = Oracle.config_mismatch('Style/EmptyMethod', {'Style/EmptyMethod' => defaults.fetch('Style/EmptyMethod'), 'Layout/LineLength' => {'Enabled' => false, 'Max' => 20}}, defaults, "def foo\nend\n")
    assert_nil reason
    assert_equal({'Layout/LineLength' => {'enabled' => false, 'max' => 20}}, toml)
  end
end

class EmptyConstructorsFixtureTest < Minitest::Test
  def test_all_pinned_calls_corrections_and_representable_frozen_comment_settings
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [['EmptyLiteral', 'empty_literal', 51, 28, 3],
     ['EmptyClassDefinition', 'empty_class_definition', 33, 6, 20]].each do |name, snake, calls, fixes, skips|
      cop = "Style/#{name}"
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal calls, matching.size
      assert_equal fixes, matching.count { |e| corrections[e.object_id] }
      assert_equal skips, skipped.size
      skipped.each do |e|
        assert_nil e.skip
        reasons = Oracle.config_mismatch(cop, e.config, defaults, e.source).first
        if name == 'EmptyLiteral'
          assert_equal ['AllCops StringLiteralsFrozenByDefault=true'], reasons
        else
          assert reasons.all? { |r| r.include?('AllowedParentClasses=') || r.include?('EnforcedStyle="class_new"') }, reasons.inspect
        end
      end
      expected = matching.map do |e|
        _, toml = Oracle.config_mismatch(cop, e.config, defaults, e.source)
        {'spec_line' => e.spec_line, 'source' => e.source,
         'positions' => e.offenses.map { |o| [o.line, o.col] },
         'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id],
         'frozen_comment' => toml.dig('Style/FrozenStringLiteralComment', 'enabled') || false}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{snake}_pinned.json", __dir__)))
    end
    [true, false].each do |enabled|
      reason, toml = Oracle.config_mismatch('Style/EmptyLiteral', {'Style/FrozenStringLiteralComment' => {'Enabled' => enabled}}, defaults, 'String.new')
      assert_nil reason
      assert_equal({'Style/FrozenStringLiteralComment' => {'enabled' => enabled}}, toml)
    end
    assert_nil Oracle.config_mismatch('Style/EmptyClassDefinition', {'Style/EmptyClassDefinition' => {'EnforcedStyle' => 'class_definition'}}, defaults, 'Foo = Class.new(StandardError)').first
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class RedundantBeginFixtureTest < Minitest::Test
  def test_every_pinned_call_correction_and_line_length_dependency_is_retained
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    cop = 'Style/RedundantBegin'
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    assert_empty recorder.errors
    assert_empty recorder.skipped
    matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
    assert_equal 72, matching.size
    assert_equal 38, matching.count { |e| corrections[e.object_id] }
    assert_equal 1, skipped.size
    assert_equal 735, skipped.first.spec_line
    assert_equal 'not run on Prism', skipped.first.skip
    configured = matching.select { |e| e.config.dig('Layout/LineLength', 'Max') == 30 }
    assert_equal 3, configured.size
    configured.each do |e|
      reason, toml = Oracle.config_mismatch(cop, e.config, defaults, e.source)
      assert_nil reason
      assert_equal({'Layout/LineLength' => {'max' => 30}}, toml)
    end
    reason, toml = Oracle.config_mismatch(cop, {'Layout/LineLength' => {'Enabled' => false}}, defaults, 'x = begin foo end')
    assert_nil reason
    assert_equal({'Layout/LineLength' => {'enabled' => false}}, toml)
    expected = matching.map do |e|
      {'spec_line' => e.spec_line, 'source' => e.source,
       'positions' => e.offenses.map { |o| [o.line, o.col] },
       'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id],
       'max' => e.config.dig('Layout/LineLength', 'Max') || 120}
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/redundant_begin_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class RescueStylesFixtureTest < Minitest::Test
  def test_all_applicable_calls_corrections_and_excluded_file_projection
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [['RescueModifier', 'rescue_modifier', 25, 15, 1],
     ['RescueStandardError', 'rescue_standard_error', 17, 4, 20]].each do |name, snake, calls, fixes, skips|
      cop = "Style/#{name}"
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal calls, matching.size
      assert_equal fixes, matching.count { |e| corrections[e.object_id] }
      assert_equal skips, skipped.size
      skipped.each do |e|
        if name == 'RescueModifier'
          assert_equal 170, e.spec_line
          assert_equal 'not run on Prism', e.skip
        else
          assert_nil e.skip
          assert_equal ['Style/RescueStandardError EnforcedStyle="implicit"'], Oracle.config_mismatch(cop, e.config, defaults, e.source).first
        end
      end
      expected = matching.map do |e|
        {'spec_line' => e.spec_line, 'source' => e.source,
         'positions' => e.offenses.map { |o| [o.line, o.col] },
         'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{snake}_pinned.json", __dir__)))
      if name == 'RescueModifier'
        excluded = matching.find { |e| e.spec_line == 404 }
        mismatch, toml = Oracle.config_mismatch(cop, excluded.config, defaults, excluded.source)
        assert_nil mismatch
        assert_equal({cop => {'exclude' => ['**/**']}}, toml)
      end
    end
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class ReturnNilFixtureTest < Minitest::Test
  def test_every_matching_call_and_correction_is_retained
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [['ReturnNil', 'return_nil', 3, 1, 6],
     ['ReturnNilInPredicateMethodDefinition', 'return_nil_in_predicate_method_definition', 32, 17, 12]].each do |name, snake, calls, fixes, skips|
      cop = "Style/#{name}"
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal calls, matching.size
      assert_equal fixes, matching.count { |e| corrections[e.object_id] }
      assert_equal skips, skipped.size
      skipped.each do |e|
        assert_nil e.skip
        mismatch = Oracle.config_mismatch(cop, e.config, defaults, e.source).first
        if name == 'ReturnNil'
          assert_equal ['Style/ReturnNil EnforcedStyle="return_nil"'], mismatch
        else
          assert_includes [['Style/ReturnNilInPredicateMethodDefinition AllowedMethods=["foo?"]'],
                           ['Style/ReturnNilInPredicateMethodDefinition AllowedPatterns=[/foo/]']], mismatch
        end
      end
      expected = matching.map do |e|
        {'spec_line' => e.spec_line, 'source' => e.source,
         'positions' => e.offenses.map { |o| [o.line, o.col] },
         'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{snake}_pinned.json", __dir__)))
    end
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class ExceptionPoliciesFixtureTest < Minitest::Test
  def test_every_applicable_exception_call_and_correction_is_retained
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [['ImplicitRuntimeError', 'implicit_runtime_error', 8, 0],
     ['RaiseArgs', 'raise_args', 29, 17],
     ['RedundantException', 'redundant_exception', 37, 27]].each do |name, snake, calls, fixes|
      cop = "Style/#{name}"
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal calls, matching.size
      assert_equal fixes, matching.count { |e| corrections[e.object_id] }
      assert_equal(name == 'RaiseArgs' ? 15 : 0, skipped.size)
      skipped.each do |e|
        if e.skip
          assert_equal 'mocks: uses `stub_const`', e.skip
          assert_includes e.description, 'when exception type is in AllowedCompactTypes'
          assert_includes [414, 419, 425], e.spec_line
        else
          assert_equal ['Style/RaiseArgs EnforcedStyle="compact"'], Oracle.config_mismatch(cop, e.config, defaults, e.source).first
        end
      end
      expected = matching.map do |e|
        {'spec_line' => e.spec_line, 'source' => e.source,
         'positions' => e.offenses.map { |o| [o.line, o.col] },
         'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{snake}_pinned.json", __dir__)))
    end
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class SelfAssignmentSwapValuesFixtureTest < Minitest::Test
  def test_every_matching_call_and_correction_is_retained
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    [['SelfAssignment', 'self_assignment', 118, 82],
     ['SwapValues', 'swap_values', 11, 8]].each do |name, snake, calls, fixes|
      cop = "Style/#{name}"
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      defaults = RuboCop::ConfigLoader.default_configuration.to_h
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_empty skipped
      assert_equal calls, matching.size
      assert_equal fixes, matching.count { |e| corrections[e.object_id] }
      expected = matching.map do |e|
        {'spec_line' => e.spec_line, 'source' => e.source,
         'positions' => e.offenses.map { |o| [o.line, o.col] },
         'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{snake}_pinned.json", __dir__)))
    end
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class RedundantAssignmentsFixtureTest < Minitest::Test
  def test_every_matching_call_and_correction_is_retained
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    [['RedundantAssignment', 'redundant_assignment', 12, 6],
     ['RedundantSelfAssignment', 'redundant_self_assignment', 14, 9],
     ['RedundantSelfAssignmentBranch', 'redundant_self_assignment_branch', 34, 17]].each do |name, snake, calls, fixes|
      cop = "Style/#{name}"
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      defaults = RuboCop::ConfigLoader.default_configuration.to_h
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_empty skipped
      assert_equal calls, matching.size
      assert_equal fixes, matching.count { |e| corrections[e.object_id] }
      expected = matching.map do |e|
        {'spec_line' => e.spec_line, 'source' => e.source,
         'positions' => e.offenses.map { |o| [o.line, o.col] },
         'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{snake}_pinned.json", __dir__)))
    end
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class ParallelAssignmentFixtureTest < Minitest::Test
  def test_every_matching_call_and_correction_is_retained
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    cop = 'Style/ParallelAssignment'
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    assert_empty recorder.skipped
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
    assert_equal 86, matching.size
    assert_equal 48, matching.count { |e| corrections[e.object_id] }
    assert_equal [807, 820, 834, 850, 866], skipped.map(&:spec_line)
    skipped.each do |e|
      assert_nil e.skip
      assert_equal ['Layout/IndentationWidth Width=3'], Oracle.config_mismatch(cop, e.config, defaults, e.source).first
    end
    expected = matching.map do |e|
      {'spec_line' => e.spec_line, 'source' => e.source,
       'positions' => e.offenses.map { |o| [o.line, o.col] },
       'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/parallel_assignment_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class AssignmentMemoizationFixtureTest < Minitest::Test
  def test_every_matching_call_and_correction_is_retained
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [['Style/MultilineMemoization', 'multiline_memoization', 9, 3, 10],
     ['Style/OrAssignment', 'or_assignment', 25, 17, 0]].each do |cop, snake, count, fixes, skips|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal count, matching.size
      assert_equal skips, skipped.size
      skipped.each do |e|
        assert_nil e.skip
        assert_equal ['Style/MultilineMemoization EnforcedStyle="braces"'], Oracle.config_mismatch(cop, e.config, defaults, e.source).first
      end
      assert_equal fixes, matching.count { |e| corrections[e.object_id] }
      expected = matching.map do |e|
        {'spec_line' => e.spec_line, 'source' => e.source,
         'positions' => e.offenses.map { |o| [o.line, o.col] },
         'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{snake}_pinned.json", __dir__)))
    end
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class InPatternThenFixtureTest < Minitest::Test
  def test_every_applicable_pattern_call_and_safe_correction_is_retained
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [['Style/InPatternThen', 'in_pattern_then', 7],
     ['Style/MultilineInPatternThen', 'multiline_in_pattern_then', 14]].each do |cop, snake, count|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      assert_equal count, recorder.expectations.size
      recorder.expectations.each do |e|
        assert_nil e.skip
        assert_nil Oracle.config_mismatch(cop, e.config, defaults, e.source).first
      end
      assert_equal 5, recorder.expectations.count { |e| corrections[e.object_id] }
      expected = recorder.expectations.map do |e|
        {'spec_line' => e.spec_line, 'source' => e.source,
         'positions' => e.offenses.map { |o| [o.line, o.col] },
         'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{snake}_pinned.json", __dir__)))
    end
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class InfiniteLoopFixtureTest < Minitest::Test
  def test_every_applicable_call_and_unsafe_correction_is_retained
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    cop = 'Style/InfiniteLoop'
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    assert_empty recorder.skipped
    assert_equal 28, recorder.expectations.size
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
    assert_equal 26, matching.size
    assert_equal 2, skipped.size
    skipped.each do |e|
      assert_nil e.skip
      assert_equal ['Layout/IndentationWidth Width=4'], Oracle.config_mismatch(cop, e.config, defaults, e.source).first
    end
    assert_equal 23, matching.count { |e| corrections[e.object_id] }
    expected = matching.map do |e|
      {'spec_line' => e.spec_line, 'source' => e.source,
       'positions' => e.offenses.map { |o| [o.line, o.col] },
       'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/infinite_loop_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class ForFixtureTest < Minitest::Test
  def test_every_fixed_each_example_and_unsafe_correction_call_is_retained
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    no_corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    original_no = Oracle::ExampleGroup.instance_method(:expect_no_corrections)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    Oracle::ExampleGroup.define_method(:expect_no_corrections) do |**|
      no_corrections[Oracle.recorder.expectations.last.object_id] = true
    end
    cop = 'Style/For'
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    assert_empty recorder.skipped
    assert_equal 60, recorder.expectations.size
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
    assert_equal 29, matching.size
    assert_equal 31, skipped.size
    assert skipped.none?(&:skip)
    skipped.each do |e|
      assert_equal ['Style/For EnforcedStyle="for"'], Oracle.config_mismatch(cop, e.config, defaults, e.source).first
    end
    assert_equal 20, matching.count { |e| corrections[e.object_id] }
    assert_equal 6, matching.count { |e| no_corrections[e.object_id] }
    expected = matching.map do |e|
      {'spec_line' => e.spec_line, 'source' => e.source,
       'positions' => e.offenses.map { |o| [o.line, o.col] },
       'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id],
       'no_correction' => !!no_corrections[e.object_id]}
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/for_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
    Oracle::ExampleGroup.define_method(:expect_no_corrections, original_no) if original_no
  end
end

class CombinableLoopsFixtureTest < Minitest::Test
  def test_all_pinned_calls_and_unsafe_corrections_are_retained
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    cop = 'Style/CombinableLoops'
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    assert_empty recorder.skipped
    assert_equal 46, recorder.expectations.size
    assert_equal 26, corrections.size
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    recorder.expectations.each do |e|
      assert_nil e.skip
      assert_nil Oracle.config_mismatch(cop, e.config, defaults, e.source).first
    end
    expected = recorder.expectations.map do |e|
      {'spec_line' => e.spec_line, 'source' => e.source,
       'positions' => e.offenses.map { |o| [o.line, o.col] },
       'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/combinable_loops_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class LoopStyleFixtureTest < Minitest::Test
  def test_every_fixed_policy_call_correction_and_line_length_projection_is_retained
    # RuboCop specs: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [['Style/NegatedWhile', 'negated_while', 10, 6, 0],
     ['Style/WhileUntilDo', 'while_until_do', 10, 3, 0],
     ['Style/WhileUntilModifier', 'while_until_modifier', 52, 19, 1]].each do |cop, snake, calls, fixes, skips|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal calls, matching.size
      assert_equal fixes, matching.count { |e| corrections[e.object_id] }
      assert_equal skips, skipped.size
      skipped.each do |e|
        assert_equal ['Layout/LineLength AllowURI=false'], Oracle.config_mismatch(cop, e.config, defaults, e.source).first
      end
      expected = matching.map do |e|
        _, toml = Oracle.config_mismatch(cop, e.config, defaults, e.source)
        {'spec_line' => e.spec_line, 'source' => e.source,
         'positions' => e.offenses.map { |o| [o.line, o.col] },
         'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id], 'toml' => toml}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{snake}_pinned.json", __dir__)))
      next unless cop == 'Style/WhileUntilModifier'

      disabled = matching.select { |e| e.config.dig('Layout/LineLength', 'Enabled') == false }
      assert_equal 2, disabled.size
      disabled.each do |e|
        assert_equal({'enabled' => false}, Oracle.config_mismatch(cop, e.config, defaults, e.source).last['Layout/LineLength'])
      end
    end
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class EmptyElseFixtureTest < Minitest::Test
  def test_all_fixed_calls_and_missing_else_enablement_are_preserved
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    cop = 'Style/EmptyElse'
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    assert_empty recorder.skipped
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
    assert_equal 20, matching.size
    assert_equal 104, skipped.size
    assert skipped.none?(&:skip)
    reasons = skipped.map do |e|
      Oracle.config_mismatch(cop, e.config, defaults, e.source).first.first.split(' ').first(2).join(' ')
    end
    assert_equal({'Style/EmptyElse EnforcedStyle="empty"' => 30,
                  'Style/EmptyElse EnforcedStyle="nil"' => 41,
                  'Style/MissingElse EnforcedStyle="if"' => 7,
                  'Style/MissingElse EnforcedStyle="case"' => 7,
                  'Style/EmptyElse AllowComments=true' => 19}, reasons.tally)
    matching.select { |e| e.config.dig('Style/MissingElse', 'Enabled') == true }.each do |e|
      _, toml = Oracle.config_mismatch(cop, e.config, defaults, e.source)
      assert_equal({'enabled' => true}, toml['Style/MissingElse'])
    end
    assert_equal 7, matching.count { |e| corrections[e.object_id] }
    expected = matching.map do |e|
      {'spec_line' => e.spec_line, 'source' => e.source,
       'positions' => e.offenses.map { |o| [o.line, o.col] },
       'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id],
       'missing_else' => e.config.dig('Style/MissingElse', 'Enabled') == true}
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/empty_else_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class UnlessWhenEmptyCaseFixtureTest < Minitest::Test
  def test_all_applicable_calls_and_corrections_are_preserved
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [
      ['Style/UnlessElse', 'unless_else', 5, 4, 0],
      ['Style/WhenThen', 'when_then', 4, 2, 0],
      ['Style/EmptyCaseCondition', 'empty_case_condition', 30, 11, 2]
    ].each do |cop, snake, calls, fixes, prism_skips|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal calls, matching.size
      assert_equal prism_skips, skipped.size
      assert skipped.all? { |e| e.skip == 'not run on Prism' }
      assert_equal fixes, matching.count { |e| corrections[e.object_id] }
      expected = matching.map do |e|
        {'spec_line' => e.spec_line, 'source' => e.source,
         'positions' => e.offenses.map { |o| [o.line, o.col] },
         'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{snake}_pinned.json", __dir__)))
    end
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class RedundantConditionalFixtureTest < Minitest::Test
  def test_every_pinned_call_and_correction_is_preserved
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    cop = 'Style/RedundantConditional'
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    assert_empty recorder.skipped
    assert_equal 11, recorder.expectations.size
    assert_equal 8, recorder.expectations.count { |e| corrections[e.object_id] }
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    expected = recorder.expectations.map do |e|
      refute e.skip
      assert_nil Oracle.config_mismatch(cop, e.config, defaults, e.source).first
      offenses = e.offenses.sort_by { |o| [o.line, o.col] }
      {'spec_line' => e.spec_line, 'source' => e.source,
       'positions' => offenses.map { |o| [o.line, o.col] },
       'messages' => offenses.map(&:message), 'correction' => corrections[e.object_id]}
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/redundant_conditional_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class NestedConditionalFixtureTest < Minitest::Test
  def test_every_pinned_call_and_correction_is_preserved
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [['NestedModifier', 'nested_modifier', 23, 19],
     ['NestedTernaryOperator', 'nested_ternary_operator', 15, 13]].each do |name, filename, calls, edits|
      cop = "Style/#{name}"
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      assert_equal calls, recorder.expectations.size
      assert_equal edits, recorder.expectations.count { |e| corrections[e.object_id] }
      expected = recorder.expectations.map do |e|
        refute e.skip
        assert_nil Oracle.config_mismatch(cop, e.config, defaults, e.source).first
        offenses = e.offenses.sort_by { |o| [o.line, o.col] }
        {'spec_line' => e.spec_line, 'source' => e.source,
         'positions' => offenses.map { |o| [o.line, o.col] },
         'messages' => offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{filename}_pinned.json", __dir__)))
    end
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class NegatedConditionalSubjectTest < Minitest::Test
  def test_all_applicable_calls_and_corrections_survive_in_credited_rust_fixtures
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [['NegatedIf', 'negated_if', 11, 3, 4],
     ['NegatedUnless', 'negated_unless', 10, 3, 4],
     ['NegatedIfElseCondition', 'negated_if_else_condition', 35, 27, 0]].each do |name, filename, calls, edits, skips|
      cop = "Style/#{name}"
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal calls, matching.size
      assert_equal edits, matching.count { |e| corrections[e.object_id] }
      assert_equal skips, skipped.size
      skipped.each do |e|
        assert Oracle.config_mismatch(cop, e.config, defaults, e.source).first.all? { |r| r.start_with?("#{cop} EnforcedStyle=") }
      end
      expected = matching.map do |e|
        offenses = e.offenses.sort_by { |o| [o.line, o.col] }
        {'spec_line' => e.spec_line, 'source' => e.source,
         'positions' => offenses.map { |o| [o.line, o.col] },
         'messages' => offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{filename}_pinned.json", __dir__)))
    end
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end

  def test_named_cop_subject_supplies_its_local_config_without_config_context
    %w[NegatedIf NegatedUnless].each do |name|
      cop = "Style/#{name}"
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      assert_equal %w[both postfix prefix], recorder.expectations.map { |e| e.config.fetch(cop).fetch('EnforcedStyle') }.uniq.sort
      assert_equal %w[both prefix postfix], recorder.expectations.first.config.fetch(cop).fetch('SupportedStyles')
    end
  end
end

class MissingElseMultilineFixtureTest < Minitest::Test
  def test_all_applicable_pinned_calls_and_corrections_are_retained
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [['MissingElse', 'missing_else', 13, 0, 74],
     ['MultilineIfModifier', 'multiline_if_modifier', 17, 13, 0]].each do |name, filename, calls, edits, skips|
      cop = "Style/#{name}"
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal calls, matching.size
      assert_equal edits, matching.count { |e| corrections[e.object_id] }
      assert_equal skips, skipped.size
      skipped.each do |e|
        refute e.skip
        reasons = Oracle.config_mismatch(cop, e.config, defaults, e.source).first
        assert reasons.all? { |r| r.start_with?('Style/UnlessElse Enabled=false', 'Style/EmptyElse EnforcedStyle=', 'Style/MissingElse EnforcedStyle=') }, reasons.inspect
      end
      expected = matching.map do |e|
        offenses = e.offenses.sort_by { |o| [o.line, o.col] }
        {'spec_line' => e.spec_line, 'source' => e.source,
         'positions' => offenses.map { |o| [o.line, o.col] },
         'messages' => offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{filename}_pinned.json", __dir__)))
    end
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class SemicolonUnlessFixtureTest < Minitest::Test
  def test_every_applicable_pinned_call_and_correction_is_preserved
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [['IfWithSemicolon', 'if_with_semicolon', 45, 44, []],
     ['InvertibleUnlessCondition', 'invertible_unless_condition', 14, 7, [58, 117, 128]]].each do |name, filename, calls, edits, skips|
      cop = "Style/#{name}"
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal calls, matching.size
      assert_equal edits, matching.count { |e| corrections[e.object_id] }
      assert_equal skips, skipped.map(&:spec_line).sort
      expected = matching.map do |e|
        offenses = e.offenses.sort_by { |o| [o.line, o.col] }
        {'spec_line' => e.spec_line, 'source' => e.source,
         'positions' => offenses.map { |o| [o.line, o.col] },
         'messages' => offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{filename}_pinned.json", __dir__)))
    end
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class InvertibleUnlessSettingsTest < Minitest::Test
  def test_only_source_relevant_inverse_pairs_are_projected
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    cop = 'Style/InvertibleUnlessCondition'
    config = {cop => {'InverseMethods' => {'!=' => '==', 'include?' => 'exclude?'}}}
    assert_nil Oracle.config_mismatch(cop, config, defaults, 'foo unless x != y').first
    assert_nil Oracle.config_mismatch(cop, config, defaults, 'foo if x.odd?').first
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'foo unless x.odd?').first
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'foo unless array.include?(x)').first
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'foo unless x != y || array.include?(x)').first
  end
end

class ConditionalModifiersBooleanFixtureTest < Minitest::Test
  def test_assigned_pinned_calls_and_corrections_are_complete
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [
      ['Style/IfUnlessModifierOfIfUnless', 'if_unless_modifier_of_if_unless', 7, 5, 0],
      ['Style/IfWithBooleanLiteralBranches', 'if_with_boolean_literal_branches', 93, 68, 1]
    ].each do |cop, filename, calls, edits, skips|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal calls, matching.size
      assert_equal edits, matching.count { |e| corrections[e.object_id] }
      assert_equal skips, skipped.size
      if skips.positive?
        assert_equal [626], skipped.map(&:spec_line) # Alternate AllowedMethods list.
        assert_equal "num.nonzero? ? true : false\n", skipped.first.source
      end
      expected = matching.map do |e|
        offenses = e.offenses.sort_by { |o| [o.line, o.col] }
        {'spec_line' => e.spec_line, 'source' => e.source,
         'positions' => offenses.map { |o| [o.line, o.col] },
         'messages' => offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{filename}_pinned.json", __dir__)))
    end
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class IdenticalConditionalBranchesFixtureTest < Minitest::Test
  def test_every_pinned_call_and_unsafe_correction_is_preserved
    cop = 'Style/IdenticalConditionalBranches'
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    assert_empty recorder.skipped
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    expected = recorder.expectations.map do |e|
      assert_nil e.skip
      assert_nil Oracle.config_mismatch(cop, e.config, defaults, e.source).first
      offenses = e.offenses.sort_by { |o| [o.line, o.col] }
      {'spec_line' => e.spec_line, 'source' => e.source,
       'positions' => offenses.map { |o| [o.line, o.col] },
       'messages' => offenses.map(&:message), 'correction' => corrections[e.object_id]}
    end
    assert_equal 53, expected.size
    assert_equal 23, corrections.size
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/identical_conditional_branches_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class CaseLikeIfFixtureTest < Minitest::Test
  def test_all_pinned_calls_and_unsafe_corrections_are_preserved
    cop = 'Style/CaseLikeIf'
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    assert_empty recorder.skipped
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    fixture = JSON.parse(File.read(File.expand_path('../../../tests/fixtures/case_like_if_pinned.json', __dir__)))
    assert_equal 42, recorder.expectations.size
    assert_equal 18, corrections.size
    assert_equal 17, fixture.count { |e| e['adapted'] }
    expected = recorder.expectations.map do |e|
      mismatch = Oracle.config_mismatch(cop, e.config, defaults, e.source).first
      adapted = nil
      if mismatch
        assert_equal 1, e.offenses.size
        assert_equal 2, e.config[cop]['MinBranchesCount']
        ast = RuboCop::ProcessedSource.new(e.source, 3.4, '(fixture)', parser_engine: :parser_prism).ast
        tail = ast
        tail = tail.else_branch while tail.else_branch&.if_type? && tail.else_branch.elsif?
        offset = tail.loc.else ? tail.loc.else.begin_pos : ast.loc.end.begin_pos
        adapted = e.source.dup.insert(offset, "elsif #{ast.condition.source}\n")
      end
      {'spec_line' => e.spec_line, 'source' => e.source, 'positions' => e.offenses.map { |o| [o.line, o.col] },
       'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id], 'adapted' => adapted}
    end
    assert_equal expected, fixture
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end

  def test_minimum_projection_retains_unaffected_matchers_only
    cop = 'Style/CaseLikeIf'
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    mismatch = ->(source, value = 2) {
      Oracle.config_mismatch(cop, {cop => defaults.fetch(cop).merge('MinBranchesCount' => value)}, defaults, source).first
    }
    short = 'if x == 1; elsif x == 2; end'
    long = 'if x == 1; elsif x == 2; elsif x == 3; end'
    assert mismatch.call(short)
    refute mismatch.call(long)
    refute mismatch.call('if x == Foo; elsif x == Bar; end')
    refute mismatch.call('if x == 1; elsif y == 2; end')
    refute mismatch.call('if /(?<name>.*)/ =~ x; elsif x == 1; end')
    refute mismatch.call('if x == 1; elsif x == 2; end', 4)
    assert mismatch.call(long, 4)
    assert mismatch.call(long + "; #{short}")
    assert mismatch.call(long, 0)
  end
end

class NextFixtureTest < Minitest::Test
  def test_all_applicable_calls_and_corrections_are_preserved
    cop = 'Style/Next'
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    assert_empty recorder.skipped
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
    assert_equal 31, matching.size # 29 unique sources, two repeated calls.
    assert_equal 45, skipped.size # Genuine fixed-setting differences only.
    assert_equal 10, matching.count { |e| corrections[e.object_id] }
    expected = matching.map do |e|
      {'spec_line' => e.spec_line, 'source' => e.source,
       'positions' => e.offenses.map { |o| [o.line, o.col] },
       'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/next_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end

  def test_minimum_projection_keeps_only_unaffected_sources
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    mismatch = lambda do |source, options|
      Oracle.config_mismatch('Style/Next', {'Style/Next' => defaults.fetch('Style/Next').merge(options)}, defaults, source).first
    end
    short = "each do\n if ready?\n  work\n end\nend\n"
    long = "each do\n if ready?\n  a\n  b\n  c\n end\nend\n"
    assert mismatch.call(short, 'MinBodyLength' => 1)
    refute mismatch.call(long, 'MinBodyLength' => 1)
    refute mismatch.call(short.sub('each do', 'custom do'), 'MinBodyLength' => 1)
    refute mismatch.call("each { work if ready? }", 'MinBodyLength' => 1)
    # An inner short loop must prevent projection even if its outer loop is long.
    assert mismatch.call("loop do\n if a\n #{short}\n end\nend\n", 'MinBodyLength' => 1)
    assert mismatch.call(long, 'MinBodyLength' => -2)
    assert mismatch.call(long, 'MinBodyLength' => 1, 'EnforcedStyle' => 'always')
    assert mismatch.call(long, 'MinBodyLength' => 1, 'AllowConsecutiveConditionals' => true)
  end
end

class GuardClauseFixtureTest < Minitest::Test
  def test_all_applicable_pinned_calls_and_corrections_are_preserved
    cop = 'Style/GuardClause'
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    assert_empty recorder.skipped
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
    assert_equal 77, matching.size # 76 unique sources; retain the repeated call too.
    assert_equal 21, skipped.size # 5 alternate configs, 16 upstream Prism exclusions.
    assert_equal 46, matching.count { |e| corrections[e.object_id] }
    expected = matching.map do |e|
      _, toml = Oracle.config_mismatch(cop, e.config, defaults, e.source)
      {'spec_line' => e.spec_line, 'source' => e.source,
       'positions' => e.offenses.map { |o| [o.line, o.col] },
       'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id],
       'toml' => Oracle.toml_text(toml)}
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/guard_clause_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class YodaOperatorSettingsTest < Minitest::Test
  def test_all_applicable_calls_and_unsafe_corrections_are_preserved
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    expected = []
    {'Style/UnlessLogicalOperators' => [20, 8, 0], 'Style/YodaCondition' => [39, 39, 13], 'Style/YodaExpression' => [14, 1, 7]}.each do |cop, (calls, skips, fixes)|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal calls, matching.size
      assert_equal skips, skipped.size
      assert_equal fixes, matching.count { |e| corrections[e.object_id] }
      matching.each do |e|
        expected << {'cop' => cop, 'spec_line' => e.spec_line, 'source' => e.source,
                     'positions' => e.offenses.map { |o| [o.line, o.col] },
                     'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/unless_yoda_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end

  def test_supported_operators_project_only_unaffected_sends
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    cop = 'Style/YodaExpression'
    config = {cop => defaults.fetch(cop).merge('SupportedOperators' => ['*', '+'])}
    ['1 + x', 'CONST.+(*ary)', '"1 | x"', '# 1 | x', '1 - x'].each do |source|
      assert_nil Oracle.config_mismatch(cop, config, defaults, source).first, source
    end
    ['1 | x', '1 & x', '1 ^ x', '1&.|(x)', 'f { 1 | x }'].each do |source|
      refute_nil Oracle.config_mismatch(cop, config, defaults, source).first, source
    end
  end
end

class NilNotFixtureTest < Minitest::Test
  def test_all_applicable_calls_and_corrections_are_preserved
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    expected = []
    {'Style/NilComparison' => [6, 10, 6], 'Style/NonNilCheck' => [12, 11, 4], 'Style/Not' => [13, 0, 12]}.each do |cop, (calls, skips, fixes)|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal calls, matching.size
      assert_equal skips, skipped.size
      assert_equal fixes, matching.count { |e| corrections[e.object_id] }
      matching.each do |e|
        expected << {'cop' => cop, 'spec_line' => e.spec_line, 'source' => e.source,
                     'positions' => e.offenses.map { |o| [o.line, o.col] },
                     'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/nil_not_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class DoubleNegationFixtureTest < Minitest::Test
  def test_all_fixed_policy_calls_and_unsafe_corrections_are_preserved
    cop = 'Style/DoubleNegation'
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    assert_empty recorder.skipped
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
    assert_equal 42, matching.size
    assert_equal 15, skipped.size
    assert skipped.all? { |e| e.config.fetch(cop).fetch('EnforcedStyle') == 'forbidden' }
    assert_equal 18, matching.count { |e| corrections[e.object_id] }
    expected = matching.map do |e|
      {'spec_line' => e.spec_line, 'source' => e.source,
       'positions' => e.offenses.map { |o| [o.line, o.col] },
       'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/double_negation_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class InverseMultipleComparisonFixtureTest < Minitest::Test
  def test_all_applicable_calls_and_corrections_are_preserved
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    expected = []
    {'Style/InverseMethods' => [111, 4, 96], 'Style/MultipleComparison' => [27, 9, 15]}.each do |cop, (calls, skips, fixes)|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal calls, matching.size
      assert_equal skips, skipped.size
      assert_equal fixes, matching.count { |e| corrections[e.object_id] }
      matching.each do |e|
        expected << {'cop' => cop, 'spec_line' => e.spec_line, 'source' => e.source,
                     'positions' => e.offenses.map { |o| [o.line, o.col] },
                     'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/inverse_multiple_comparison_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end

  def test_source_relevant_settings_never_skip_fixed_map_or_threshold_examples
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    cop = 'Style/InverseMethods'
    options = defaults.fetch(cop).merge('InverseMethods' => defaults.fetch(cop).fetch('InverseMethods').merge('present?' => 'blank?'))
    mismatch = ->(source) { Oracle.config_mismatch(cop, {cop => options}, defaults, source).first }
    assert_nil mismatch.call('!foo.none?')
    assert_nil mismatch.call('foo.present?')
    refute_nil mismatch.call('!foo.present?')
    refute_nil mismatch.call('not foo.blank?')
    options = defaults.fetch(cop).merge('InverseBlocks' => {'select' => 'different'})
    refute_nil mismatch.call('foo.select { |e| !e }')
    assert_nil mismatch.call('foo.select { |e| e }')
    cop = 'Style/MultipleComparison'
    options = defaults.fetch(cop).merge('ComparisonsThreshold' => 3)
    assert_nil mismatch.call('a=1; a == 1 || a == 2 || a == 3')
    assert_nil mismatch.call('a=1; a == 1')
    refute_nil mismatch.call('a=1; a == 1 || a == 2')
    refute_nil mismatch.call('a=1; a == foo || a == 1 || a == 2')
    options = defaults.fetch(cop).merge('AllowMethodComparison' => false)
    refute_nil mismatch.call('a[:key] == 1 || a[:key] == 2')
  end
end

class BooleanPredicateFixtureTest < Minitest::Test
  def test_every_applicable_call_and_correction_is_preserved
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    expected = []
    {'Style/AndOr' => [38, 44, 22], 'Style/CaseEquality' => [25, 2, 8], 'Style/CombinableDefined' => [39, 0, 22]}.each do |cop, (calls, skips, fixes)|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal calls, matching.size
      assert_equal skips, skipped.size
      assert_equal fixes, matching.count { |e| corrections[e.object_id] }
      matching.each do |e|
        expected << {'cop' => cop, 'spec_line' => e.spec_line, 'source' => e.source,
                     'positions' => e.offenses.map { |o| [o.line, o.col] },
                     'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/boolean_predicates_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end

  def test_settings_only_project_candidates_that_do_not_change
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    mismatch = lambda do |cop, options, source|
      Oracle.config_mismatch(cop, {cop => defaults.fetch(cop).merge(options)}, defaults, source).first
    end
    cop = 'Style/AndOr'
    options = {'EnforcedStyle' => 'always'}
    assert_nil mismatch.call(cop, options, 'work if a and b')
    assert_nil mismatch.call(cop, options, 'x = a && b')
    refute_nil mismatch.call(cop, options, 'x = a and b')
    refute_nil mismatch.call(cop, options, 'if a; b and c; end')
    refute_nil mismatch.call(cop, options, 'case x; in y if (a or b); end')
    cop = 'Style/CaseEquality'
    assert_nil mismatch.call(cop, {'AllowOnConstant' => true}, 'REGEXP_CONSTANT === x')
    assert_nil mismatch.call(cop, {'AllowOnConstant' => true}, 'foo === x')
    refute_nil mismatch.call(cop, {'AllowOnConstant' => true}, 'Foo === x')
    assert_nil mismatch.call(cop, {'AllowOnSelfClass' => true}, 'self.klass === x')
    refute_nil mismatch.call(cop, {'AllowOnSelfClass' => true}, 'self.class === x')
  end
end

class RedundantRegexpEscapeFixtureTest < Minitest::Test
  def test_all_pinned_calls_and_corrections_remain_accounted_for
    cop = 'Style/RedundantRegexpEscape'
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    assert_empty recorder.skipped
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    assert recorder.expectations.none? { |e| e.skip || Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
    cases = recorder.expectations.uniq { |e| Oracle.case_id(cop, e, {}) }
    assert_equal 218, cases.size
    assert_equal 31, cases.count { |e| corrections[e.object_id] }
    expected = cases.map do |e|
      {'spec_line' => e.spec_line, 'source' => e.source,
       'positions' => e.offenses.map { |o| [o.line, o.col] },
       'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/redundant_regexp_escape_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class RedundantRegexpFixtureTest < Minitest::Test
  def test_all_pinned_calls_and_corrections_remain_accounted_for
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    expected = []
    {'Style/RedundantRegexpArgument' => [51, 0, 43],
     'Style/RedundantRegexpCharacterClass' => [49, 0, 26],
     'Style/RedundantRegexpConstructor' => [17, 0, 9]}.each do |cop, (count, skip_count, correction_count)|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      matching, skipped = recorder.expectations.partition do |e|
        !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first
      end
      matching = matching.uniq { |e| Oracle.case_id(cop, e, {}) }
      assert_equal count, matching.size
      assert_equal skip_count, skipped.size
      assert_equal correction_count, matching.count { |e| corrections[e.object_id] }
      assert skipped.all? { |e| Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      expected.concat(matching.map do |e|
        {'cop' => cop, 'spec_line' => e.spec_line, 'source' => e.source,
         'positions' => e.offenses.map { |o| [o.line, o.col] },
         'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end)
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/redundant_regexps_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end
end

class ExactRegexpMatchShimTest < Minitest::Test
  def test_all_pinned_examples_including_silenced_invalid_regexps_execute
    cop = 'Style/ExactRegexpMatch'
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    assert_empty recorder.skipped
    assert_equal 25, recorder.expectations.size
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    assert recorder.expectations.all? { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
  end
end

class SymbolProcSettingsTest < Minitest::Test
  def test_all_assigned_examples_are_evaluated_with_only_fixed_option_and_version_skips
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    {'Style/SymbolProc' => [66, 21], 'Style/TallyMethod' => [30, 2]}.each do |cop, (matches, skips)|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      applicable, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal matches, applicable.uniq { |e| Oracle.case_id(cop, e, {}) }.size
      assert_equal skips, skipped.size
    end
  end

  def test_absent_or_custom_allowlists_only_skip_source_relevant_differences
    cop = 'Style/SymbolProc'
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    fixed = defaults.fetch(cop)
    [nil, [], ['unrelated']].each do |allowed|
      config = {cop => fixed.merge('AllowedMethods' => allowed)}
      ['->(x) { x.foo }', 'proc { _1.foo }', 'xs.map { it.foo }'].each do |source|
        assert_nil Oracle.config_mismatch(cop, config, defaults, source).first
      end
      refute_nil Oracle.config_mismatch(cop, config, defaults, 'define_method(:x) { |x| x.foo }').first
    end
    ['AllowedMethods', 'AllowedPatterns'].each do |key|
      config = {cop => fixed.merge(key => ['respond_'])}
      source = 'respond_to { |format| format.xml }'
      if key == 'AllowedPatterns'
        refute_nil Oracle.config_mismatch(cop, config, defaults, source).first
      else
        assert_nil Oracle.config_mismatch(cop, config, defaults, source).first
      end
    end
    config = {'AllCops' => {'ActiveSupportExtensionsEnabled' => false}}
    assert_nil Oracle.config_mismatch(cop, config, defaults, '-> { it.foo }').first
    config['AllCops']['ActiveSupportExtensionsEnabled'] = true
    refute_nil Oracle.config_mismatch(cop, config, defaults, '-> { it.foo }').first
  end

  def test_argument_and_comment_options_only_skip_affected_sources
    cop = 'Style/SymbolProc'
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    fixed = defaults.fetch(cop)
    config = {cop => fixed.merge('AllowMethodsWithArguments' => true)}
    assert_nil Oracle.config_mismatch(cop, config, defaults, 'xs.map { |x| x.foo }').first
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'xs.map(arg) { |x| x.foo }').first
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'super(arg) { |x| x.foo }').first
    config = {cop => fixed.merge('AllowComments' => true)}
    assert_nil Oracle.config_mismatch(cop, config, defaults, "# outside\nxs.map do |x|\n x.foo\nend # outside").first
    refute_nil Oracle.config_mismatch(cop, config, defaults, "xs.map do |x| # inside\n x.foo\nend").first
  end
end

class SymbolArraySettingsTest < Minitest::Test
  def test_symbol_threshold_is_source_relevant_and_delimiters_are_detection_inert
    cop = 'Style/SymbolArray'
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    config = { cop => { 'MinSize' => 0, 'EnforcedStyle' => 'percent' }, 'Style/PercentLiteralDelimiters' => { 'PreferredDelimiters' => { 'default' => '()' } } }
    ['[:one,:two]', '%i[foo\\ bar]', '[]', '[:one,*other]', '["one"]'].each do |source|
      assert_nil Oracle.config_mismatch(cop, config, defaults, source).first
    end
    refute_nil Oracle.config_mismatch(cop, config, defaults, '[:one]').first
    refute_nil Oracle.config_mismatch(cop, config.merge(cop => { 'MinSize' => 3 }), defaults, '[:one,:two]').first
    refute_nil Oracle.config_mismatch(cop, config.merge(cop => { 'EnforcedStyle' => 'brackets' }), defaults, '[:one,:two]').first
  end

  def test_all_assigned_examples_execute_with_only_real_fixed_setting_skips
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    { 'Style/SymbolArray' => [26, 7], 'Style/Sample' => [82, 0] }.each do |cop, (matches, skips)|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      applicable, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal matches, applicable.uniq { |e| Oracle.case_id(cop, e, {}) }.size
      assert_equal skips, skipped.size
    end
  end
end

class WordArraySettingsTest < Minitest::Test
  def test_nested_encoding_around_hooks_enclose_before_and_restore_even_on_errors
    original = [Encoding.default_external, Encoding.default_internal]
    Dir.mktmpdir do |dir|
      path = File.join(dir, 'word_array_spec.rb')
      File.write(path, <<~'SPEC')
        RSpec.describe RuboCop::Cop::Style::WordArray, :config do
          include EncodingHelper
          around { |example| with_default_external_encoding(Encoding::US_ASCII) { example.run } }
          context 'nested' do
            around { |example| with_default_internal_encoding(Encoding::UTF_8) { example.run } }
            before do
              raise 'external hook ignored' unless Encoding.default_external == Encoding::US_ASCII
              raise 'internal hook ignored' unless Encoding.default_internal == Encoding::UTF_8
            end
            it('records') { expect_no_offenses('[]') }
            it('restores on failure') { raise 'controlled failure' }
          end
        end
      SPEC
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(path, 'Style/WordArray', recorder)
      assert_equal ['[]'], recorder.expectations.map(&:source)
      assert_equal ['RuntimeError: controlled failure'], recorder.errors.map(&:error)
      assert_empty recorder.skipped
    end
    assert_equal original, [Encoding.default_external, Encoding.default_internal]
  end

  def test_minimum_is_source_relevant_but_styles_and_regexes_are_fixed
    cop = 'Style/WordArray'
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    config = { cop => defaults.fetch(cop).merge('MinSize' => 0), 'Style/PercentLiteralDelimiters' => { 'PreferredDelimiters' => { 'default' => '()' } } }
    ["['one','two']", '%w[one\\ two]', '[]', '[1]'].each do |source|
      assert_nil Oracle.config_mismatch(cop, config, defaults, source).first
    end
    refute_nil Oracle.config_mismatch(cop, config, defaults, "['one']").first
    refute_nil Oracle.config_mismatch(cop, config.merge(cop => config[cop].merge('MinSize' => 4)), defaults, "['one','two','three']").first
    refute_nil Oracle.config_mismatch(cop, config.merge(cop => config[cop].merge('EnforcedStyle' => 'brackets')), defaults, "['one','two']").first
    refute_nil Oracle.config_mismatch(cop, config.merge(cop => config[cop].merge('WordRegex' => /x/)), defaults, "['one','two']").first
  end

  def test_all_assigned_examples_including_encoding_around_and_large_matrix_execute
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    { 'Style/WordArray' => 34, 'Style/SlicingWithRange' => 23 }.each do |cop, count|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      matching = recorder.expectations.reject { |e| e.skip || Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal count, matching.uniq { |e| Oracle.case_id(cop, e, {}) }.size
      if cop == 'Style/WordArray'
        assert_equal 2, matching.count { |e| e.description.include?('arrays of unicode') }
        assert matching.any? { |e| e.source.include?("'998'") }
        assert matching.any? { |e| e.source == "['g', 'h']" }
      end
    end
  end
end

class ArrayIntersectionSettingsTest < Minitest::Test
  def test_pre_34_implicit_it_examples_are_version_skips_not_detector_deviations
    cop = 'Style/ArrayIntersect'
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    config = { cop => defaults.fetch(cop), 'AllCops' => { 'TargetRubyVersion' => 3.3 } }
    ['a.any? { b.member?(it) }', 'a.none? { b.member?(it) }'].each do |source|
      assert_match(/Ruby behavior:.*implicit it/, Oracle.config_mismatch(cop, config, defaults, source).first.join)
      assert_nil Oracle.config_mismatch(cop, config.merge('AllCops' => { 'TargetRubyVersion' => 3.4 }), defaults, source).first
    end
    ['a.any? { |it| b.member?(it) }', 'it = 1; a.any? { b.member?(it) }',
     'a.any? { b.member?(_1) }', '"it" # it',
     '(a & b).any?; other { it }', 'a.any? { b.include?(it) }',
     'a&.none? { b.member?(it) }'].each do |source|
      assert_nil Oracle.config_mismatch(cop, config, defaults, source).first, source
    end
  end

  def test_all_applicable_array_examples_are_evaluated
    { 'Style/ArrayCoercion' => 5, 'Style/ArrayFirstLast' => 18, 'Style/ArrayIntersect' => 85 }.each do |cop, count|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      defaults = RuboCop::ConfigLoader.default_configuration.to_h
      matching = recorder.expectations.reject { |e| e.skip || Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal count, matching.uniq { |e| Oracle.case_id(cop, e, {}) }.size
    end
  end
end

class QuotedSymbolSettingsTest < Minitest::Test
  def mismatch(style, strings = {})
    cop = 'Style/QuotedSymbols'
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    Oracle.config_mismatch(cop, { cop => defaults.fetch(cop).merge('EnforcedStyle' => style),
                                 'Style/StringLiterals' => strings }, defaults, ":'a'").first
  end

  def test_every_style_runs_and_so_does_a_disabled_string_literals_cop
    assert_nil mismatch('double_quotes')
    assert_nil mismatch('single_quotes')
    assert_nil mismatch('same_as_string_literals', 'EnforcedStyle' => 'double_quotes')
    assert_nil mismatch('same_as_string_literals', 'EnforcedStyle' => 'single_quotes')
    assert_nil mismatch('same_as_string_literals', 'Enabled' => false)
  end

  def test_all_applicable_assigned_examples_are_evaluated
    { 'Style/QuotedSymbols' => 28, 'Style/SymbolLiteral' => 4, 'Style/VariableInterpolation' => 9 }.each do |cop, count|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      defaults = RuboCop::ConfigLoader.default_configuration.to_h
      matching = recorder.expectations.reject { |e| e.skip || Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal count, matching.uniq { |e| Oracle.case_id(cop, e, {}) }.size
    end
  end
end

class CommandDelimiterSettingsTest < Minitest::Test
  def test_only_the_effective_command_delimiter_matters
    cop = 'Style/CommandLiteral'
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [{ '%x' => '()' }, { 'default' => '()' }, { '%x' => '()', 'default' => '[]' }].each do |map|
      assert_nil Oracle.config_mismatch(cop, { cop => defaults.fetch(cop), 'Style/PercentLiteralDelimiters' => { 'PreferredDelimiters' => map } }, defaults, '%x(ls)').first
    end
    refute_nil Oracle.config_mismatch(cop, { cop => defaults.fetch(cop), 'Style/PercentLiteralDelimiters' => { 'PreferredDelimiters' => { '%x' => '[]' } } }, defaults, '%x(ls)').first
  end

  def test_all_applicable_command_examples_are_evaluated
    cop = 'Style/CommandLiteral'
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    matching = recorder.expectations.reject { |e| e.skip || Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
    assert_equal 10, matching.uniq { |e| Oracle.case_id(cop, e, {}) }.size
  end
end

class RegexpDelimiterSettingsTest < Minitest::Test
  def test_all_pinned_calls_and_corrections_are_preserved_in_the_rust_fixture
    cop = 'Style/RegexpLiteral'
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    matching, skipped = recorder.expectations.partition do |e|
      !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first
    end
    assert_equal 40, skipped.size
    assert skipped.all? { |e| !e.skip && Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
    matching = matching.uniq { |e| Oracle.case_id(cop, e, {}) }
    assert_equal 37, matching.size
    assert_equal 24, matching.count { |e| corrections[e.object_id] }
    expected = matching.map do |e|
      { 'cop' => cop, 'spec_line' => e.spec_line, 'source' => e.source,
        'positions' => e.offenses.map { |o| [o.line, o.col] },
        'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id] }
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/regexp_literal_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end

  def test_only_the_effective_regexp_delimiter_matters
    cop = 'Style/RegexpLiteral'
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [{ '%r' => '{}' }, { '%r' => '{}', 'default' => '[]' }].each do |map|
      assert_nil Oracle.config_mismatch(cop, { cop => defaults.fetch(cop), 'Style/PercentLiteralDelimiters' => { 'PreferredDelimiters' => map } }, defaults, '/a/').first
    end
    [{ '%r' => '[]' }, { 'default' => '{}' }].each do |map|
      refute_nil Oracle.config_mismatch(cop, { cop => defaults.fetch(cop), 'Style/PercentLiteralDelimiters' => { 'PreferredDelimiters' => map } }, defaults, '/a/').first
    end
  end
end

class PercentDelimiterSettingsTest < Minitest::Test
  def mismatch(map, source)
    cop = 'Style/PercentLiteralDelimiters'
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    Oracle.config_mismatch(cop, { cop => defaults.fetch(cop).merge('PreferredDelimiters' => map) }, defaults, source).first
  end

  def test_global_override_retains_matching_types_but_not_changed_scalar_policies
    ['%w(a b)', '%W(#{[1].first})', '%i[a b]', '%I(#{x})', '"%q(x)" # %r(x)'].each do |source|
      assert_nil mismatch({ 'default' => '[]' }, source), source
    end
    ['%(string)', '%q[string]', '%r(regexp)', '%x[command]', '%s[symbol]', '%W(#{%q[inner]})'].each do |source|
      refute_nil mismatch({ 'default' => '[]' }, source), source
    end
    assert_nil mismatch({ 'default' => '[]', '%' => '()' }, '%w[string] + [%(string)]')
    refute_nil mismatch({ 'default' => '[]' }, nil)
    refute_nil mismatch({ 'foobar' => '()' }, '%w[string]')
  end

  def test_all_applicable_percent_examples_are_evaluated
    { 'Style/BarePercentLiterals' => 11, 'Style/PercentLiteralDelimiters' => 35, 'Style/PercentQLiterals' => 8 }.each do |cop, count|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      defaults = RuboCop::ConfigLoader.default_configuration.to_h
      matching = recorder.expectations.reject { |e| e.skip || Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal count, matching.uniq { |e| Oracle.case_id(cop, e, {}) }.size
    end
  end
end

class NumberedLimitSettingsTest < Minitest::Test
  def mismatch(max, source)
    cop = 'Style/NumberedParametersLimit'
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    Oracle.config_mismatch(cop, { cop => defaults.fetch(cop).merge('Max' => max) }, defaults, source).first
  end

  def test_alternate_max_only_skips_different_offense_positions
    ['foo { |a,b| [a,b] }', 'foo { _9 }', 'foo { [_1,_1] }',
     'foo { [_1,_2,_3] }', 'foo { |x| inner { [_1,_2,_3,x] } }',
     '-> { [_1,_2,_3] }', 'foo { "_1 _2" } # _3'].each do |source|
      assert_nil mismatch(2, source), source
    end
    refute_nil mismatch(2, 'foo { [_1,_2] }')
    refute_nil mismatch(5, 'foo { [_1,_2,_3,_4,_5] }')
    refute_nil mismatch(2, nil)
  end

  def test_all_applicable_pinned_examples_are_evaluated
    { 'Style/NumberedParametersLimit' => 7, 'Style/NilLambda' => 31 }.each do |cop, count|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      defaults = RuboCop::ConfigLoader.default_configuration.to_h
      matching = recorder.expectations.reject { |e| e.skip || Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal count, matching.uniq { |e| Oracle.case_id(cop, e, {}) }.size
    end
  end
end

class BlockPolicySettingsTest < Minitest::Test
  def mismatch(source)
    cop = 'Style/SingleLineBlockParams'
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    config = { cop => defaults.fetch(cop).merge('Methods' => [{ 'reduce' => %w[a e] }, { 'test' => %w[x y] }]) }
    Oracle.config_mismatch(cop, config, defaults, source).first
  end

  def test_parameter_map_only_skips_examples_with_different_detection
    ['xs.reduce { |c,d| c+d }', 'xs.reduce { |_x,_y| }', 'xs.reduce { |a,b| a+b }',
     'xs.reduce { |e,a| e+a }', 'xs.reduce { |b| b }', 'xs.reduce { |b,| b }', 'xs.reduce { it }',
     'xs.reduce { |a,(id,_)| a+id }', "xs.reduce do |c,d|\n c+d\nend"].each do |source|
      assert_nil mismatch(source), source
    end
    ['xs.reduce { |a,e| a+e }', 'xs.reduce { |acc,elem| acc+elem }',
     'xs.reduce { |a| a }', 'ala.test { |x,z| z }', 'xs.inject { |a,b| a+b }'].each do |source|
      refute_nil mismatch(source), source
    end
    refute_nil mismatch(nil)
  end

  def test_suitability_only_filters_actual_single_line_candidates_that_fit
    assert Oracle::DoEndBlockSuitability.unchanged?("foo do\nrescue CustomError\nend\n", 20)
    assert Oracle::DoEndBlockSuitability.unchanged?("foo do\nensure\nend\n", 20)
    assert Oracle::DoEndBlockSuitability.unchanged?('aaaaaaaaaaaa do bbbbbbbbbbbbb end', 20)
    assert Oracle::DoEndBlockSuitability.unchanged?('foo { it }', 20)
    refute Oracle::DoEndBlockSuitability.unchanged?('a do b end', 20)
    refute Oracle::DoEndBlockSuitability.unchanged?('-> do 1 end', 20)
    refute Oracle::DoEndBlockSuitability.unchanged?('def f; super do x end; end', 120)
    refute Oracle::DoEndBlockSuitability.unchanged?(nil, 20)
  end

  def test_line_length_disable_is_representable_for_do_end
    cop = 'Style/SingleLineDoEndBlock'
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    reason, toml = Oracle.config_mismatch(cop, { cop => defaults.fetch(cop), 'Layout/LineLength' => { 'Enabled' => false } }, defaults, 'foo do bar end')
    assert_nil reason
    assert_equal false, toml.fetch('Layout/LineLength').fetch('enabled')
  end
end

class OptionalParameterSettingsTest < Minitest::Test
  def mismatch(cop, options, source)
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    Oracle.config_mismatch(cop, { cop => defaults.fetch(cop).merge(options) }, defaults, source).first
  end

  def test_boolean_allowlist_only_matters_for_actual_method_boolean_defaults
    cop = 'Style/OptionalBooleanParameter'
    ['def some_method(flag=false); end', 'def respond_to_missing?(name, flag: false); end',
     'proc { |flag=false| }', '"def respond_to_missing?(x=false); end"',
     'def respond_to_missing?(name, flag=(false)); end'].each do |source|
      assert_nil mismatch(cop, { 'AllowedMethods' => [] }, source), source
    end
    refute_nil mismatch(cop, { 'AllowedMethods' => [] }, 'def self.respond_to_missing?(name, flag=false); end')
    refute_nil mismatch(cop, { 'AllowedMethods' => ['custom'] }, 'def custom(flag=true); end')
    refute_nil mismatch(cop, { 'AllowedMethods' => [] }, nil)
  end

  def test_option_hash_names_and_allowlist_compare_only_final_unforwarded_empty_hashes
    cop = 'Style/OptionHash'
    ['def f(options={}); end', 'def f(config={a: 1}); end', 'def f(config={}, *rest); end',
     'def f(opts={}); super; end', 'def f(opts={}); def g; super; end; end',
     'proc { |options={}| }', 'proc { |opts={}; local| }', 'foo(super) { |opts={}| }',
     '"def f(opts={}); end"'].each do |source|
      assert_nil mismatch(cop, { 'SuspiciousParamNames' => ['options'] }, source), source
    end
    ['def f(opts={}); end', '->(args={}) {}', 'def self.f(parameters={}); end'].each do |source|
      refute_nil mismatch(cop, { 'SuspiciousParamNames' => ['options'] }, source), source
    end
    refute_nil mismatch(cop, { 'SuspiciousParamNames' => %w[options config] }, 'def f(config={}); end')
    refute_nil mismatch(cop, { 'Allowlist' => ['to_json'] }, 'def to_json(options={}); end')
    assert_nil mismatch(cop, { 'Allowlist' => ['to_json'] }, 'def to_json(options={}); super; end')
    refute_nil mismatch(cop, { 'SuspiciousParamNames' => ['options'] }, nil)
  end

  def test_all_applicable_pinned_optional_parameter_examples_are_evaluated
    { 'Style/OptionalArguments' => 13, 'Style/OptionalBooleanParameter' => 8, 'Style/OptionHash' => 8 }.each do |cop, count|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      defaults = RuboCop::ConfigLoader.default_configuration.to_h
      matching = recorder.expectations.reject { |e| e.skip || Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal count, matching.uniq { |e| Oracle.case_id(cop, e, {}) }.size
    end
  end
end

class HashLookupSettingsTest < Minitest::Test
  def defaults
    RuboCop::ConfigLoader.default_configuration.to_h
  end

  def test_env_enable_and_pending_states_project_to_boolean_dependency
    cop = 'Style/HashLookupMethod'
    [[true, nil, true], [false, nil, false], ['pending', nil, false], ['pending', 'enable', true]].each do |enabled, new_cops, expected|
      config = { cop => defaults.fetch(cop), 'Style/FetchEnvVar' => { 'Enabled' => enabled } }
      config['AllCops'] = { 'NewCops' => new_cops } if new_cops
      reason, toml = Oracle.config_mismatch(cop, config, defaults, 'ENV.fetch(key)')
      assert_nil reason
      assert_equal expected, toml.fetch('Style/FetchEnvVar').fetch('enabled')
    end
    config = { cop => defaults.fetch(cop).merge('EnforcedStyle' => 'fetch') }
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'hash[key]').first
  end

  def test_case_threshold_only_skips_changed_eligible_mappings
    cop = 'Style/HashLikeCase'
    config = { cop => defaults.fetch(cop).merge('MinBranchesCount' => 2) }
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'case x; when :a; 1; when :b; 2; end').first
    ['case x; when :a; 1; when :b; effect(); end', 'case x; when :a; 1; when "b"; 2; end',
     'case x; when :a; 1; when :b; 2; when :c; 3; end'].each do |source|
      assert_nil Oracle.config_mismatch(cop, config, defaults, source).first
    end
  end

  def test_all_assigned_pinned_examples_are_evaluated
    { 'Style/HashFetchChain' => 34, 'Style/HashLikeCase' => 6, 'Style/HashLookupMethod' => 15 }.each do |cop, count|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      matching = recorder.expectations.reject { |e| e.skip || Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal count, matching.uniq { |e| Oracle.case_id(cop, e, Oracle.config_mismatch(cop, e.config, defaults, e.source).last) }.size
    end
  end
end

class NumericPredicateSettingsTest < Minitest::Test
  def test_bitwise_enable_dependency_is_representable_and_not_a_config_skip
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    cop = 'Style/NumericPredicate'
    [true, false].each do |enabled|
      config = { cop => defaults.fetch(cop), 'Style/BitwisePredicate' => { 'Enabled' => enabled } }
      reason, toml = Oracle.config_mismatch(cop, config, defaults, 'stat.mode & 0o077 == 0')
      assert_nil reason
      assert_equal enabled, toml.fetch('Style/BitwisePredicate').fetch('enabled')
    end
    config = { cop => defaults.fetch(cop).merge('EnforcedStyle' => 'comparison') }
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'number.zero?').first
    config = { cop => defaults.fetch(cop).merge('AllowedMethods' => ['where']) }
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'where { number > 0 }').first
  end
end

class MethodDefinitionSettingsTest < Minitest::Test
  def defaults
    RuboCop::ConfigLoader.default_configuration.to_h
  end

  def test_single_line_enable_dependency_is_representable_but_alternative_styles_are_not
    cop = 'Style/SingleLineMethods'
    [true, false].each do |enabled|
      config = { cop => defaults.fetch(cop), 'Style/EndlessMethod' => { 'Enabled' => enabled } }
      reason, toml = Oracle.config_mismatch(cop, config, defaults, 'def f; body; end')
      assert_nil reason
      assert_equal enabled, toml.fetch('Style/EndlessMethod').fetch('enabled')
    end
    config = { cop => defaults.fetch(cop), 'Style/EndlessMethod' => { 'Enabled' => true, 'EnforcedStyle' => 'allow_always' } }
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'def f; body; end').first
    config = { cop => defaults.fetch(cop).merge('AllowIfMethodIsEmpty' => false) }
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'def f; end').first
  end

  def test_multiline_signature_reads_representable_line_length_disable_and_max
    cop = 'Style/MultilineMethodSignature'
    config = { cop => defaults.fetch(cop), 'Layout/LineLength' => { 'Enabled' => false, 'Max' => 25 } }
    reason, toml = Oracle.config_mismatch(cop, config, defaults, "def f(a,\n b)\nend")
    assert_nil reason
    assert_equal({ 'enabled' => false, 'max' => 25 }, toml.fetch('Layout/LineLength'))
  end

  def test_all_applicable_method_definition_examples_including_correction_only_sources_are_recorded
    { 'Style/SingleLineMethods' => 56, 'Style/TopLevelMethodDefinition' => 14, 'Style/MultilineMethodSignature' => 25 }.each do |cop, count|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      matching = recorder.expectations.reject { |e| e.skip || Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal count, matching.uniq { |e| Oracle.case_id(cop, e, Oracle.config_mismatch(cop, e.config, defaults, e.source).last) }.size
      if cop == 'Style/SingleLineMethods'
        supplied = matching.find { |e| e.source == 'def some_method; body end' && e.config.dig('Style/EndlessMethod', 'Enabled') }
        refute_nil supplied
        assert_equal [[1, 1]], supplied.offenses.map { |o| [o.line, o.col] }
        refute recorder.skipped.any? { |_, reason| reason.include?('helper-only') }
      end
    end
  end
end

class CallSyntaxSettingsTest < Minitest::Test
  def mismatch(names, source)
    cop = 'Style/NestedParenthesizedCalls'
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    config = { cop => defaults.fetch(cop).merge('AllowedMethods' => names) }
    Oracle.config_mismatch(cop, config, defaults, source).first
  end

  def test_nested_allowlist_is_compared_only_for_actual_eligible_children
    ['puts 1', 'puts(compute something)', 'expect(obj).to(be true)',
     'puts(eq(a))', 'puts(eq)', 'puts(eq a, b)', 'puts([eq a])',
     'puts(block_taker { eq 1 })', '"puts(eq a)"'].each do |source|
      assert_nil mismatch(['be'], source), source
    end
    refute_nil mismatch(['be'], 'puts(eq a)')
    refute_nil mismatch(['custom'], 'puts(custom a)')
    assert_nil mismatch(['custom'], 'puts(custom a, b)')
    refute_nil mismatch(['be'], nil)
  end

  def test_all_pinned_empty_call_and_nested_examples_are_evaluated
    ['Style/MethodCallWithoutArgsParentheses', 'Style/NestedParenthesizedCalls'].each do |cop|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      defaults = RuboCop::ConfigLoader.default_configuration.to_h
      matching = recorder.expectations.reject { |e| e.skip || Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal(cop.include?('WithoutArgs') ? 43 : 12, matching.uniq { |e| Oracle.case_id(cop, e, {}) }.size)
      if cop.include?('WithoutArgs')
        performance = matching.find { |e| e.source.start_with?("foo0 = 1\n") }
        assert_equal 63, performance.offenses.size
        assert_equal [3, 5], [performance.offenses.first.line, performance.offenses.first.col]
      end
    end
  end
end

class ClassEqualitySettingsTest < Minitest::Test
  COP = 'Style/ClassEqualityComparison'

  def mismatch(options, source)
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    config = { COP => defaults.fetch(COP).merge(options) }
    Oracle.config_mismatch(COP, config, defaults, source).first
  end

  def test_pinned_global_empty_list_keeps_top_level_and_unrelated_method_cases
    ['var.class == Date', 'def check; var.class.name == "Date"; end',
     'def ==(x); x == y; end', '"def ==; var.class == Date; end"',
     '# def ==; var.class == Date; end',
     'def ==(x); def check; var.class == Date; end; end'].each do |source|
      assert_nil mismatch({ 'AllowedMethods' => [] }, source), source
    end
    ['==', 'equal?', 'eql?'].each do |name|
      refute_nil mismatch({ 'AllowedMethods' => [] }, "def self.#{name}(x); var.class == Date; end")
    end
    refute_nil mismatch({ 'AllowedMethods' => [] }, nil)
  end

  def test_combined_methods_and_patterns_compare_effective_nearest_def_exemption
    assert_nil mismatch({ 'AllowedMethods' => ['=='] }, 'def ==(x); self.class == x.class; end')
    assert_nil mismatch({ 'AllowedMethods' => [], 'AllowedPatterns' => ['^equal\\?$'] },
                        'def equal?(x); self.class == x.class; end')
    refute_nil mismatch({ 'AllowedMethods' => [], 'AllowedPatterns' => ['^equal\\?$'] },
                       'def eql?(x); self.class == x.class; end')
    refute_nil mismatch({ 'AllowedPatterns' => ['check'] }, 'def check; x.class == Date; end')
    assert_nil mismatch({ 'AllowedPatterns' => ['check'] }, 'def check; x&.class == Date; end')
    assert_nil mismatch({ 'AllowedMethods' => [] }, 'def ==(x); x.class == "#{name}"; end')
    refute_nil mismatch({ 'AllowedMethods' => [] }, 'def check; def ==(x); x.class == Date; end; end')
  end
end

class GlobalVariableSettingsTest < Minitest::Test
  def mismatch(cop, options, source)
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    config = { cop => defaults.fetch(cop).merge(options) }
    Oracle.config_mismatch(cop, config, defaults, source).first
  end

  def test_allowed_variables_only_skip_changed_actual_reads_and_writes
    cop = 'Style/GlobalVars'
    options = { 'AllowedVariables' => ['$allowed', '$stdin'] }
    ['$custom', '$custom = 1', '$stdin', '$1', "'$allowed' # $allowed"].each do |source|
      assert_nil mismatch(cop, options, source), source
    end
    ['$allowed', '$allowed = 1', '$allowed ||= 1', '$allowed, x = 1, 2', '"#$allowed"', 'defined?($allowed)', 'alias $allowed $custom'].each do |source|
      refute_nil mismatch(cop, options, source), source
    end
    refute_nil mismatch(cop, options, nil)
  end

  def test_require_english_is_detection_inert_but_alternative_styles_still_skip
    cop = 'Style/SpecialGlobalVars'
    assert_nil mismatch(cop, { 'RequireEnglish' => false }, 'puts $$')
    refute_nil mismatch(cop, { 'EnforcedStyle' => 'use_perl_names' }, 'puts $$')
  end
end

class BlockDelimitersSettingsTest < Minitest::Test
  COP = 'Style/BlockDelimiters'

  def mismatch(options, source)
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    config = { COP => defaults.fetch(COP).merge(options) }
    Oracle.config_mismatch(COP, config, defaults, source).first
  end

  def test_pinned_custom_lists_do_not_discard_unrelated_block_examples
    options = { 'AllowedMethods' => ['proc'], 'AllowedPatterns' => ['test'] }
    ["each {\n x\n}", 'proc do x end', 'puts lambda', '# test_method {',
     '"lambda { x }"', 'object.test_method'].each do |source|
      assert_nil mismatch(options, source), source
    end
  end

  def test_actual_changed_allowances_still_skip_including_arrow_lambdas_and_receivers
    options = { 'AllowedMethods' => ['proc'], 'AllowedPatterns' => ['test'] }
    ['lambda do x end', '-> { x }', 'obj&.it do x end', "test_method {\n x\n}"].each do |source|
      refute_nil mismatch(options, source), source
    end
    refute_nil mismatch(options, nil), 'source-less helpers must not be assumed unrelated'
  end

  def test_braces_required_names_are_relevant_only_for_matching_blocks
    options = { 'BracesRequiredMethods' => ['sig'] }
    assert_nil mismatch(options, "each {\n x\n}")
    refute_nil mismatch(options, "sig {\n x\n}")
    refute_nil mismatch(options, "sig do\n x\nend")
  end

  def test_arbitrary_list_differences_and_regexes_are_compared_not_just_pinned_test_names
    refute_nil mismatch({ 'AllowedMethods' => ['custom'] }, 'custom do x end')
    assert_nil mismatch({ 'AllowedMethods' => ['custom'] }, 'other do x end')
    refute_nil mismatch({ 'AllowedPatterns' => ['^work_'] }, 'work_now do x end')
    assert_nil mismatch({ 'AllowedPatterns' => ['^work_'] }, 'now_work do x end')
    refute_nil mismatch({ 'EnforcedStyle' => 'semantic' }, 'each do x end')
  end
end

class CommentSettingsTest < Minitest::Test
  def mismatch(cop, options, source)
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    config = { cop => defaults.fetch(cop).merge(options) }
    Oracle.config_mismatch(cop, config, defaults, source).first
  end

  def test_keyword_lists_only_skip_relevant_comment_annotations
    cop = 'Style/CommentAnnotation'
    options = { 'Keywords' => %w[TODO FIXME OPTIMIZE HACK REVIEW] }
    assert_nil mismatch(cop, options, '# TODO make better')
    refute_nil mismatch(cop, options, '# NOTE make better')
    assert_nil mismatch(cop, options, '"# NOTE text"')
    assert_nil mismatch(cop, options, "__END__\n# NOTE text\n")
    refute_nil mismatch(cop, options, nil)
    refute_nil mismatch(cop, { 'Keywords' => ['ISSUE'] }, '# ISSUE wrong order')
    refute_nil mismatch(cop, { 'Keywords' => ['TODO LATER'] }, '# TODO LATER text')
    assert_nil mismatch(cop, { 'Keywords' => ['TODO', 'DO SOMETHING'] }, '# TODO text')
    refute_nil mismatch(cop, { 'RequireColon' => false }, '# TODO: text')
  end

  def test_allowed_char_lists_only_skip_changed_comment_eligibility
    cop = 'Style/AsciiComments'
    options = { 'AllowedChars' => ['∂'] }
    assert_nil mismatch(cop, options, '# 这是什么？')
    refute_nil mismatch(cop, options, '# ∂')
    refute_nil mismatch(cop, options, '# ©')
    assert_nil mismatch(cop, options, '"# ∂"')
    assert_nil mismatch(cop, options, '# ∂ 日本語')
    refute_nil mismatch(cop, options, nil)
  end
end

class CopyrightSettingsTest < Minitest::Test
  COP = 'Style/Copyright'

  def mismatch(options, source)
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    config = { COP => defaults.fetch(COP).merge(options) }
    Oracle.config_mismatch(COP, config, defaults, source).first
  end

  def test_custom_notice_patterns_run_only_when_detection_is_equivalent
    options = { 'Notice' => 'Copyright (\\(c\\) )?2015 Acme Inc',
                'AutocorrectNotice' => '# Copyright (c) 2015 Acme Inc.' }
    ["# Copyright 2015 Acme Inc.\nnames = []\n", "names = []\n",
     "names = []\n# Copyright 2015 Acme Inc.\n",
     "=begin\nCopyright 2015 Acme Inc.\n=end\n"].each do |source|
      assert_nil mismatch(options, source), source
    end
    refute_nil mismatch(options, '# Copyright 2026 Someone Else')
    refute_nil mismatch(options, '# prefix Copyright 2015 Acme Inc.')
    refute_nil mismatch(options, nil), 'source-less helpers must not be assumed equivalent'
  end

  def test_normalized_hash_patterns_and_multiline_notices
    source = "# Copyright (c) 2026 My Name\nputs 1\n"
    ['^# Copyright', '\\A# Copyright', '^#\\s+Copyright', '^#  Copyright'].each do |prefix|
      assert_nil mismatch({ 'Notice' => "#{prefix} (\\(c\\) )?2[0-9]{3} .+" }, source)
    end
    options = { 'Notice' => "Copyright 2026 X\n\nLicense details" }
    assert_nil mismatch(options, "# Copyright 2026 X\n#\n# License details\n")
    refute_nil mismatch(options, "# Copyright 2026 X\n")
    assert_nil mismatch(options, "x = '# Copyright 2026 X'\n")
    assert_nil mismatch(options, "__END__\n# Copyright 2026 X\n")
    refute_nil mismatch({ 'Notice' => '' }, 'x = 1')
  end
end

class AnnotationsTest < Minitest::Test
  def parse(source)
    Oracle::Annotations.parse(source)
  end

  def test_carets_mark_the_line_above_by_column_and_length
    plain, offenses = parse(<<~RUBY)
      a do
        b
      end.c
      ^^^^^ Avoid chaining a method call on a do...end block.
    RUBY
    assert_equal "a do\n  b\nend.c\n", plain
    assert_equal [[3, 1, 5, "Avoid chaining a method call on a do...end block."]],
                 offenses.map(&:to_a)
  end

  def test_several_annotations_on_one_line_sort_by_column
    _, offenses = parse(<<~RUBY)
      foo(bar, baz)
              ^^^ Second.
          ^^^ First.
    RUBY
    assert_equal([[1, 5], [1, 9]], offenses.map { |o| [o.line, o.col] })
  end

  def test_empty_marker_is_a_zero_length_offense
    _, offenses = parse("\n^{} Missing magic comment.\nputs 1\n")
    assert_equal([[1, 1, 0]], offenses.map { |o| [o.line, o.col, o.carets] })
  end

  def test_escaped_caret_is_source
    plain, offenses = parse("x = 1\n\\^ not an annotation\n")
    assert_equal "x = 1\n\\^ not an annotation\n", plain
    assert_empty offenses
  end

  def test_replacements_fill_source_carets_and_padding
    source = Oracle::Annotations.format_offense(<<~RUBY, keyword: "raise")
      %{keyword}(x); %{keyword}(y)
      ^{keyword}^^^ First.
      _{keyword}     ^{keyword}^^^ Second.
    RUBY
    plain, offenses = parse(source)
    assert_equal "raise(x); raise(y)\n", plain
    assert_equal([[1, 1, 8], [1, 11, 8]], offenses.map { |o| [o.line, o.col, o.carets] })
  end
end

class SpecEvaluationTest < Minitest::Test
  SPEC = <<~'SPEC'
    RSpec.describe RuboCop::Cop::Lint::Debugger, :config do
      let(:cop_config) { { 'DebuggerMethods' => %w[custom] } }

      it 'registers an offense' do
        expect_offense(<<~RUBY)
          custom
          ^^^^^^ Remove debugger entry point `custom`.
        RUBY
      end

      context 'when nested', :ruby27 do
        let(:cop_config) { super().merge('Extra' => 1) }

        before { cop_config['FromHook'] = 2 }

        it 'sees the merged configuration' do
          expect_no_offenses('x')
        end
      end

      shared_examples 'accepts' do |code|
        it "accepts #{code}" do
          expect_no_offenses(code)
        end
      end

      it_behaves_like 'accepts', 'foo'
      shared_examples 'keyword arguments' do |prefix, suffix:|
        it('keeps keywords') { expect_no_offenses(prefix + suffix) }
      end
      it_behaves_like 'keyword arguments', 'foo', suffix: 'bar'

      it 'is not run on Prism', unsupported_on: :prism do
        expect_no_offenses('y')
      end

      it 'is pending' do
        pending 'not yet'
        expect_no_offenses('z')
      end

      it 'mocks' do
        allow(Object).to receive(:new)
        expect_no_offenses('w')
      end

      it 'uses the project index', :project_index do
        expect_no_offenses('v')
      end

      it 'uses an unknown helper' do
        unknown_helper
      end
    end
  SPEC

  def setup
    @recorder = Oracle::Recorder.new
    Dir.mktmpdir do |dir|
      path = File.join(dir, "debugger_spec.rb")
      File.write(path, SPEC)
      Oracle.evaluate_spec(path, "Lint/Debugger", @recorder)
    end
  end

  def expectation(description)
    @recorder.expectations.find { |e| e.description.end_with?(description) } ||
      flunk("no expectation for #{description.inspect}")
  end

  def test_records_source_offenses_and_configuration
    found = expectation("registers an offense")
    assert_equal "custom\n", found.source
    assert_equal([[1, 1, 6]], found.offenses.map { |o| [o.line, o.col, o.carets] })
    assert_equal %w[custom], found.config["Lint/Debugger"]["DebuggerMethods"]
    assert_nil found.skip
    assert_equal "RuboCop::Cop::Lint::Debugger registers an offense", found.description
  end

  def test_nested_lets_reach_outer_definitions_and_hooks_run
    found = expectation("sees the merged configuration")
    section = found.config["Lint/Debugger"]
    assert_equal [%w[custom], 1, 2], section.values_at("DebuggerMethods", "Extra", "FromHook")
  end

  def test_ruby_tags_below_prism_minimum_read_as_3_3
    assert_in_delta 3.3, expectation("sees the merged configuration").ruby_version
    assert_in_delta 3.4, expectation("registers an offense").ruby_version
  end

  def test_shared_examples_take_arguments
    assert_equal "foo", expectation("behaves like accepts accepts foo").source
  end

  def test_shared_examples_forward_keyword_arguments
    assert_equal 'foobar', expectation('keeps keywords').source
  end

  def test_map_into_array_reads_only_the_map_replacement_entry
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    cop = 'Style/MapIntoArray'
    source = 'dest=[]; src.each { |e| dest << e }'
    [{}, {'reject' => 'filter'}, {'map' => 'map', 'collect' => 'foo'}].each do |methods|
      config = {cop => defaults.fetch(cop), 'Style/CollectionMethods' => {'PreferredMethods' => methods}}
      assert_nil Oracle.config_mismatch(cop, config, defaults, source).first
    end
    config = {cop => defaults.fetch(cop), 'Style/CollectionMethods' => {'PreferredMethods' => {'map' => 'collect'}}}
    refute_nil Oracle.config_mismatch(cop, config, defaults, source).first
  end

  def test_collection_settings_compare_only_actual_eligible_calls
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    cop = 'Style/CollectionMethods'
    config = {cop => defaults.fetch(cop).merge('PreferredMethods' => {'collect' => 'map', 'inject' => 'reduce'})}
    ['x.collect { it }', 'x.inject(:+)', '# x.detect { it }', 'x.detect', '"x.detect { it }"'].each do |source|
      assert_nil Oracle.config_mismatch(cop, config, defaults, source).first, source
    end
    ['x.detect { it }', 'x&.detect(&:foo)', 'detect { _1 }'].each do |source|
      refute_nil Oracle.config_mismatch(cop, config, defaults, source).first, source
    end
    refute_nil Oracle.config_mismatch(cop, config, defaults, nil).first
    cop = 'Style/CollectionCompact'
    config = {cop => defaults.fetch(cop).merge('AllowedReceivers' => ['params'])}
    assert_nil Oracle.config_mismatch(cop, config, defaults, 'foo.reject { |e| e.nil? }').first
    assert_nil Oracle.config_mismatch(cop, config, defaults, 'params.reject { |e| e.odd? }').first
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'params.reject(&:nil?)').first
    refute_nil Oracle.config_mismatch(cop, config, defaults, 'params.merge(x: y).reject { |k,v| v.nil? }').first
  end

  def test_collection_assigned_specs_have_no_blanket_mapping_skips
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    {'Style/CollectionCompact' => 26, 'Style/CollectionMethods' => 68, 'Style/CollectionQuerying' => 28}.each do |cop, count|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      applicable = recorder.expectations.select { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal count, applicable.size, cop
    end
  end

  def test_string_methods_legacy_spec_key_retains_both_pinned_examples
    cop = 'Style/StringMethods'
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    assert_equal 2, recorder.expectations.size
    recorder.expectations.each do |e|
      assert_nil e.skip
      assert_nil Oracle.config_mismatch(cop, e.config, defaults, e.source).first
    end
    config = { cop => defaults.fetch(cop).merge('intern' => 'to_sym', 'PreferredMethods' => { 'intern' => 'length' }) }
    refute_nil Oracle.config_mismatch(cop, config, defaults, "'x'.intern").first
  end

  def test_format_token_threshold_only_skips_behavior_changing_limits
    helper = Oracle::FormatTokenThreshold
    assert helper.equivalent?(0, 'format("%{x}", x: 1)')
    assert helper.equivalent?(0, 'format("%s %s", 1, 2)')
    refute helper.equivalent?(0, 'format("%s", 1)')
    refute helper.equivalent?(2, 'format("%s %s", 1, 2)')
    assert helper.equivalent?(2, 'format("%s", 1)')
    assert helper.equivalent?(0, 'foo("%s")')
  end

  def test_all_applicable_format_token_examples_are_evaluated
    recorder = Oracle::Recorder.new
    cop = 'Style/FormatStringToken'
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    applicable = recorder.expectations.select { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
    assert_equal 154, applicable.size
    assert_equal 101, applicable.map { |e| Oracle.case_id(cop, e, {}) }.uniq.size
    assert applicable.any? { |e| e.source.include?("sprintf('%{foo}'") }
    assert applicable.any? { |e| e.source.include?('%-20s %-30s') }
    assert applicable.any? { |e| e.source.include?("format('%A %s'") }
  end

  def test_if_unless_modifier_loads_both_pinned_shared_keyword_groups
    recorder = Oracle::Recorder.new
    cop = 'Style/IfUnlessModifier'
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    %w[if unless].each do |keyword|
      assert recorder.expectations.any? { |e| e.description.include?("for a multiline '#{keyword}'") && e.source.include?("#{keyword} condition") }
    end
  end

  def test_global_shared_examples_load_pinned_body_padding_support
    src = Oracle.locate_rubocop(nil)
    %w[Class Module].each do |kind|
      recorder = Oracle::Recorder.new
      cop = "Layout/EmptyLinesAround#{kind}Body"
      Oracle.evaluate_spec(Oracle.spec_path(src, cop), cop, recorder)
      assert_empty recorder.errors
      assert recorder.expectations.any? { |e| e.description.include?('behaves like empty_lines_around_class_or_module_body') }
      defaults = RuboCop::ConfigLoader.default_configuration.to_h
      matching = recorder.expectations.reject do |e|
        e.skip || Oracle.config_mismatch(cop, e.config, defaults, e.source).first
      end
      assert_equal(kind == 'Class' ? 5 : 3, matching.size)
    end
  end

  def test_end_alignment_loads_pinned_shared_examples_and_keyword_assignments
    src = Oracle.locate_rubocop(nil)
    recorder = Oracle::Recorder.new
    cop = 'Layout/EndAlignment'
    Oracle.evaluate_spec(Oracle.spec_path(src, cop), cop, recorder)
    assert_empty recorder.errors
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    matching = recorder.expectations.reject do |e|
      e.skip || Oracle.config_mismatch(cop, e.config, defaults, e.source).first
    end
    assert_equal 73, matching.uniq { |e| Oracle.case_id(cop, e, {}) }.size
    assert matching.any? { |e| e.description.include?('behaves like aligned') }
    assert matching.any? { |e| e.description.include?('behaves like misaligned') }
    assert matching.any? { |e| e.source.start_with?('var = if') && !e.offenses.empty? }
    refute matching.any? { |e| e.source.start_with?('var = if') && e.config[cop]['EnforcedStyleAlignWith'] != 'keyword' }
    refute matching.any? { |e| e.source.start_with?('puts 1; class') && e.config[cop]['EnforcedStyleAlignWith'] == 'start_of_line' }
    assert matching.any? { |e| e.source.start_with?('puts 1; class') && e.config[cop]['EnforcedStyleAlignWith'] == 'keyword' }
  end

  def test_operator_specs_run_under_equivalent_unset_no_space_styles
    src = Oracle.locate_rubocop(nil)
    recorder = Oracle::Recorder.new
    cop = 'Layout/SpaceAroundOperators'
    Oracle.evaluate_spec(Oracle.spec_path(src, cop), cop, recorder)
    assert_empty recorder.errors
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    matching = recorder.expectations.reject do |e|
      e.skip || Oracle.config_mismatch(cop, e.config, defaults, e.source).first
    end
    assert_operator matching.size, :>=, 80
    assert matching.any? { |e| e.description.end_with?('accepts rational') }
    refute matching.any? { |e| e.description.include?('when EnforcedStyleForExponentOperator is space') }
    refute matching.any? { |e| e.description.include?('when EnforcedStyleForRationalLiterals is space') }
    refute matching.any? { |e| e.description.include?('when does not allowed for alignment') }
    refute matching.any? { |e| e.description.include?('configured to true') }
  end

  def test_method_call_parentheses_evaluates_all_fixed_style_examples
    src = Oracle.locate_rubocop(nil)
    recorder = Oracle::Recorder.new
    cop = 'Style/MethodCallWithArgsParentheses'
    Oracle.evaluate_spec(Oracle.spec_path(src, cop), cop, recorder)
    assert_empty recorder.errors
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    matching = recorder.expectations.reject do |e|
      e.skip || Oracle.config_mismatch(cop, e.config, defaults, e.source).first
    end
    assert_equal 42, matching.uniq { |e| Oracle.case_id(cop, e, {}) }.size
    assert matching.any? { |e| e.source.include?('class Foo') && e.config[cop]['IgnoreMacros'] == 'true' }
    assert matching.any? { |e| e.source.include?('module Foo') && e.config[cop]['IgnoreMacros'] == 'true' }
    refute matching.any? { |e| e.config[cop]['EnforcedStyle'] == 'omit_parentheses' }
    refute matching.any? { |e| !e.config[cop]['IncludedMacros'].empty? || !e.config[cop]['IncludedMacroPatterns'].empty? }
  end

  def test_prism_exclusions_pending_mocks_and_project_index_are_skipped
    assert_equal "not run on Prism", expectation("is not run on Prism").skip
    assert_equal "pending", expectation("is pending").skip
    assert_equal "mocks: uses `allow`", expectation("mocks").skip
    assert_equal "project index", expectation("uses the project index").skip
  end

  def test_examples_that_raise_are_reported
    assert_equal 1, @recorder.errors.size
    assert_equal "NameError: undefined `unknown_helper'", @recorder.errors.first.error
  end

  def test_skip_before_any_source_is_recorded_is_counted_with_its_reason
    Dir.mktmpdir do |dir|
      path = File.join(dir, 'debugger_spec.rb')
      File.write(path, <<~SPEC)
        RSpec.describe RuboCop::Cop::Lint::Debugger, :config do
          it('explicit skip') { skip 'requires a newer Ruby' }
          it('metadata skip', :ruby40, unsupported_on: :prism) { unknown_helper }
          it('pending')
        end
      SPEC
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(path, 'Lint/Debugger', recorder)
      assert_empty recorder.errors
      assert_equal ['skip: requires a newer Ruby', 'not run on Prism', 'pending'],
                   recorder.skipped.map { |_, reason| reason }
      assert recorder.skipped.all? { |example, _| example.spec == path }
    end
  end

  def test_expect_blocks_do_not_hide_source_examples
    Dir.mktmpdir do |dir|
      path = File.join(dir, 'debugger_spec.rb')
      File.write(path, <<~SPEC)
        RSpec.describe RuboCop::Cop::Lint::Debugger, :config do
          it('wrapped') { expect { expect_no_offenses('wrapped') }.not_to raise_error }
          it('empty') {}
          it('metadata') { expect(described_class.cop_name).to eq('Lint/Debugger') }
          it('unknown inside expect') { expect { unknown_helper }.not_to raise_error }
        end
      SPEC
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(path, 'Lint/Debugger', recorder)
      assert_equal ['wrapped'], recorder.expectations.map(&:source)
      assert_equal 2, recorder.errors.size
      assert recorder.errors.any? { |error| error.error.include?('no source expectation') }
      assert recorder.errors.any? { |error| error.error.include?('unknown_helper') }
      assert_equal ['helper-only: assertion without source expectations'],
                   recorder.skipped.map { |_, reason| reason }
    end
  end

  def test_console_context_captures_output_without_skipping_or_leaking_even_on_error
    stdout, stderr = $stdout, $stderr
    Dir.mktmpdir do |dir|
      path = File.join(dir, 'copyright_spec.rb')
      File.write(path, <<~'SPEC')
        RSpec.describe RuboCop::Cop::Style::Copyright, :config do
          include_context 'mock console output'
          it 'still records sources' do
            warn 'captured warning'
            puts 'captured output'
            raise 'no capture' unless $stderr.string.include?('captured warning')
            expect_no_offenses('# Copyright 2026 X')
          end
          it('unsupported helper remains an error') { parse_source('x') }
        end
      SPEC
      recorder = Oracle::Recorder.new
      out, err = capture_io { Oracle.evaluate_spec(path, 'Style/Copyright', recorder) }
      assert_empty out
      assert_empty err
      assert_equal ['# Copyright 2026 X'], recorder.expectations.map(&:source)
      assert_equal 1, recorder.errors.size
      assert_includes recorder.errors.first.error, '`parse_source` is not emulated'
      assert_empty recorder.skipped
    end
    assert_same stdout, $stdout
    assert_same stderr, $stderr
  end

  def test_pinned_copyright_examples_are_all_accounted_for
    src = Oracle.locate_rubocop(nil)
    cop = 'Style/Copyright'
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(src, cop), cop, recorder)
    assert_empty recorder.skipped
    assert_equal 23, recorder.expectations.size
    applicable = recorder.expectations.reject do |e|
      Oracle.config_mismatch(cop, e.config, defaults, e.source).first
    end
    assert_equal recorder.expectations.size, applicable.size
    assert_equal 14, applicable.uniq { |e| Oracle.case_id(cop, e, {}) }.size
    assert_equal 2, recorder.errors.size
    assert recorder.errors.all? { |error|
      error.recorded.zero? && Oracle.config_mismatch(cop, error.config, defaults, nil).first
    }
  end

  def test_hooks_do_not_leak_into_the_default_configuration
    defaults = RuboCop::ConfigLoader.default_configuration.for_cop("Lint/Debugger")
    refute defaults.key?("FromHook")
  end
end

class SplitSpecEvaluationTest < Minitest::Test
  def test_one_line_condition_fixtures_retain_all_fixed_setting_calls_and_corrections
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **_options|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [
      ['Style/OneLineConditional', 'one_line_conditional', 61, 56, 51, 4],
      ['Style/ParenthesesAroundCondition', 'parentheses_around_condition', 26, 7, 6, 0]
    ].each do |cop, snake, calls, fixes, config_skips, version_skips|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal calls, matching.size
      assert_equal config_skips, skipped.count { |e| !e.skip }
      assert_equal version_skips, skipped.count { |e| e.skip == 'not run on Prism' }
      assert_equal fixes, matching.count { |e| corrections[e.object_id] }
      expected = matching.map do |e|
        {'spec_line' => e.spec_line, 'source' => e.source,
         'positions' => e.offenses.map { |o| [o.line, o.col] },
         'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{snake}_pinned.json", __dir__)))
    end
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end

  def test_multiline_conditional_fixtures_retain_every_applicable_call_and_correction
    # RuboCop examples: MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **_options|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [
      ['Style/MultilineIfThen', 'multiline_if_then', 14, 3, 0],
      ['Style/MultilineTernaryOperator', 'multiline_ternary_operator', 19, 17, 2],
      ['Style/MultilineWhenThen', 'multiline_when_then', 14, 5, 0]
    ].each do |cop, snake, calls, fixes, prism_skips|
      recorder = Oracle::Recorder.new
      Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
      assert_empty recorder.errors
      assert_empty recorder.skipped
      matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
      assert_equal calls, matching.size
      assert_equal prism_skips, skipped.size
      assert skipped.all? { |e| e.skip == 'not run on Prism' }
      assert_equal fixes, matching.count { |e| corrections[e.object_id] }
      expected = matching.map do |e|
        {'spec_line' => e.spec_line, 'source' => e.source,
         'positions' => e.offenses.map { |o| [o.line, o.col] },
         'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
      end
      assert_equal expected, JSON.parse(File.read(File.expand_path("../../../tests/fixtures/#{snake}_pinned.json", __dir__)))
    end
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end

  def test_if_unless_modifier_fixture_retains_all_applicable_correction_calls
    # RuboCop examples are MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    cop = 'Style/IfUnlessModifier'
    corrections, no_corrections = {}, {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    original_no = Oracle::ExampleGroup.instance_method(:expect_no_corrections)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    Oracle::ExampleGroup.define_method(:expect_no_corrections) do
      no_corrections[Oracle.recorder.expectations.last.object_id] = true
    end
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_path(Oracle.locate_rubocop(nil), cop), cop, recorder)
    assert_empty recorder.errors
    assert_empty recorder.skipped
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
    assert_equal 162, matching.size
    assert_equal 157, matching.uniq { |e| Oracle.case_id(cop, e, {}) }.size
    assert_equal 6, skipped.size # Unsupported URI/pattern/directive/tab-width options.
    assert_equal 83, matching.count { |e| corrections[e.object_id] }
    assert_equal 7, matching.count { |e| no_corrections[e.object_id] }
    expected = matching.map do |e|
      {'spec_line' => e.spec_line, 'source' => e.source,
       'positions' => e.offenses.map { |o| [o.line, o.col] },
       'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id],
       'no_correction' => no_corrections.fetch(e.object_id, false),
       'toml' => Oracle.toml_text(Oracle.config_mismatch(cop, e.config, defaults, e.source).last)}
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/if_unless_modifier_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
    Oracle::ExampleGroup.define_method(:expect_no_corrections, original_no) if original_no
  end

  def test_ternary_parentheses_fixture_retains_every_fixed_policy_call_and_correction
    # RuboCop examples are MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    cop = 'Style/TernaryParentheses'
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **_options|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_paths(Oracle.locate_rubocop(nil), cop).first, cop, recorder)
    assert_empty recorder.errors
    assert_empty recorder.skipped
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
    assert_equal 45, matching.size # 44 unique sources; range lookalike is repeated.
    assert_equal 62, skipped.size # 61 alternate configs, one upstream Prism exclusion.
    assert_equal 27, matching.count { |e| corrections[e.object_id] }
    expected = matching.map do |e|
      {'spec_line' => e.spec_line, 'source' => e.source,
       'positions' => e.offenses.map { |o| [o.line, o.col] },
       'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/ternary_parentheses_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end

  def test_sole_nested_conditional_fixture_retains_every_fixed_policy_call_and_correction
    # RuboCop examples are MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    cop = 'Style/SoleNestedConditional'
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **_options|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    recorder = Oracle::Recorder.new
    Oracle.evaluate_spec(Oracle.spec_paths(Oracle.locate_rubocop(nil), cop).first, cop, recorder)
    assert_empty recorder.errors
    assert_empty recorder.skipped
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
    assert_equal 71, matching.size
    # Two duplicate comment cases disable IfUnlessModifier; one AllowModifier=true.
    assert_equal 3, skipped.size
    assert_equal 63, matching.count { |e| corrections[e.object_id] }
    expected = matching.map do |e|
      {'spec_line' => e.spec_line, 'source' => e.source,
       'positions' => e.offenses.map { |o| [o.line, o.col] },
       'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/sole_nested_conditional_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end

  def test_if_inside_else_fixture_retains_every_fixed_policy_call_and_correction
    # RuboCop examples are MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    cop = 'Style/IfInsideElse'
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **_options|
      corrections[Oracle.recorder.expectations.last.object_id] = source
    end
    recorder = Oracle::Recorder.new
    paths = Oracle.spec_paths(Oracle.locate_rubocop(nil), cop)
    assert_equal 1, paths.size
    Oracle.evaluate_spec(paths.first, cop, recorder)
    assert_empty recorder.errors
    assert_empty recorder.skipped
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
    assert_equal 29, matching.size
    assert_equal 1, skipped.size # AllowIfModifier=true is not our fixed policy.
    assert_equal 13, matching.count { |e| corrections[e.object_id] }
    expected = matching.map do |e|
      {'spec_line' => e.spec_line, 'source' => e.source,
       'positions' => e.offenses.map { |o| [o.line, o.col] },
       'messages' => e.offenses.map(&:message), 'correction' => corrections[e.object_id]}
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/if_inside_else_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end

  def test_fixed_conditional_assignment_fixture_retains_every_call_and_correction
    cop = 'Style/ConditionalAssignment'
    corrections = {}
    original = Oracle::ExampleGroup.instance_method(:expect_correction)
    Oracle::ExampleGroup.define_method(:expect_correction) do |source, **options|
      corrections[Oracle.recorder.expectations.last.object_id] = [source, options]
    end
    recorder = Oracle::Recorder.new
    paths = Oracle.spec_paths(Oracle.locate_rubocop(nil), cop)
    paths.each { |path| Oracle.evaluate_spec(path, cop, recorder) }
    assert_empty recorder.errors
    assert_empty recorder.skipped
    assert_equal paths, recorder.expectations.map(&:spec).uniq
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    matching, skipped = recorder.expectations.partition { |e| !e.skip && !Oracle.config_mismatch(cop, e.config, defaults, e.source).first }
    assert_equal 573, matching.size # 378 unique; repeated calls retained.
    assert_equal 654, skipped.size # Alternate style/branch/ternary/end alignment settings.
    assert_equal 514, matching.count { |e| corrections[e.object_id] }
    assert_equal 612, skipped.count { |e| e.spec == paths.first }
    expected = matching.map do |e|
      _, toml = Oracle.config_mismatch(cop, e.config, defaults, e.source)
      source, options = corrections[e.object_id]
      {'spec' => File.basename(e.spec), 'spec_line' => e.spec_line, 'source' => e.source,
       'positions' => e.offenses.map { |o| [o.line, o.col] },
       'messages' => e.offenses.map(&:message), 'correction' => source,
       'loop' => options && options[:loop], 'toml' => Oracle.toml_text(toml)}
    end
    assert_equal expected, JSON.parse(File.read(File.expand_path('../../../tests/fixtures/conditional_assignment_pinned.json', __dir__)))
  ensure
    Oracle::ExampleGroup.define_method(:expect_correction, original) if original
  end

  def test_evaluates_each_exact_file_once_and_deduplicates_identical_cases
    Dir.mktmpdir do |dir|
      cop = "Style/ConditionalAssignment"
      mapping = Oracle.source_and_specs(dir, cop)
      [mapping[:source], *mapping[:specs]].each do |path|
        FileUtils.mkdir_p(File.dirname(path))
        File.write(path, "")
      end
      mapping[:specs].each_with_index do |path, index|
        File.write(path, <<~SPEC)
          RSpec.describe RuboCop::Cop::Style::ConditionalAssignment, :config do
            it('unique') { expect_no_offenses('unique#{index}') }
            it('duplicate') { expect_no_offenses('same') }
          end
        SPEC
      end
      nearby = mapping[:specs].first.sub('_assign_in_condition_spec', '_other_cop_spec')
      File.write(nearby, "raise 'must not load a nearby prefix'")
      result = Oracle.check_cop(cop, dir, RuboCop::ConfigLoader.default_configuration.to_h,
                                "/bin/true", dir)
      assert_nil result[:fatal]
      assert_empty result[:errors]
      assert_equal %w[unique0 same unique1], result[:cases].map { |c| c.expectation.source }
      assert_equal mapping[:specs], result[:cases].map { |c| c.expectation.spec }.uniq
      second = result[:cases].last
      assert_includes Oracle.case_comment(result, second), File.basename(mapping[:specs].last)
      out, = capture_io { Oracle.print_case(result, second, 'test') }
      assert_includes out, File.basename(mapping[:specs].last)
    end
  end

  def test_pinned_conditional_assignment_examples_from_both_files_are_evaluated
    cop = 'Style/ConditionalAssignment'
    recorder = Oracle::Recorder.new
    paths = Oracle.spec_paths(Oracle.locate_rubocop(nil), cop)
    paths.each { |path| Oracle.evaluate_spec(path, cop, recorder) }
    assert_empty recorder.errors
    assert_equal paths, recorder.expectations.map(&:spec).uniq
    assert_equal 1227, recorder.expectations.size
  end

  def test_partial_missing_mapping_does_not_silently_run_only_one_file
    Dir.mktmpdir do |dir|
      mapping = Oracle.source_and_specs(dir, "Style/ConditionalAssignment")
      [mapping[:source], mapping[:specs].first].each do |path|
        FileUtils.mkdir_p(File.dirname(path))
        File.write(path, "raise 'discovery must fail before evaluation'")
      end
      result = Oracle.check_cop("Style/ConditionalAssignment", dir, {}, "unused", dir)
      assert result[:no_spec]
      assert_includes result[:fatal], File.basename(mapping[:specs].last)
      refute_includes result[:fatal], 'could not evaluate'
    end
  end
end

class EvaluationAccountingTest < Minitest::Test
  def test_only_demonstrably_inapplicable_source_less_errors_are_config_skips
    Dir.mktmpdir do |dir|
      mapping = Oracle.source_and_specs(dir, 'Lint/Debugger')
      [mapping[:source], *mapping[:specs]].each do |path|
        FileUtils.mkdir_p(File.dirname(path))
        File.write(path, '')
      end
      File.write(mapping[:specs].first, <<~SPEC)
        RSpec.describe RuboCop::Cop::Lint::Debugger, :config do
          it('applicable unknown helper') { unknown_helper }
          it 'partial example' do
            expect_no_offenses('ordinary_call')
            unknown_helper
          end
          context 'another configuration' do
            let(:cop_config) { { 'DebuggerMethods' => ['custom'] } }
            it('inapplicable helper') { unknown_helper }
            it('inapplicable source') { expect_no_offenses('custom') }
          end
        end
      SPEC
      result = Oracle.check_cop('Lint/Debugger', dir, RuboCop::ConfigLoader.default_configuration.to_h,
                                '/bin/true', dir)
      assert_equal 2, result[:errors].size
      assert_equal [0, 1], result[:errors].map(&:recorded)
      assert_equal 2, result[:skipped].size
      assert result[:skipped].all? { |_, reason| reason.start_with?('config:') }
      capture_io do
        assert_equal 1, Oracle.report([result], { entries: {} }, Oracle::Options.new(quiet: true))
      end
    end
  end
end

class ConfigMismatchTest < Minitest::Test
  DEFAULTS = RuboCop::ConfigLoader.default_configuration.to_h

  def mismatch(cop, config, source = "x\n")
    Oracle.config_mismatch(cop, config, DEFAULTS, source)
  end

  def test_missing_source_does_not_make_source_sensitive_settings_irrelevant
    config = { 'Style/HashSyntax' => DEFAULTS['Style/HashSyntax'].merge('EnforcedShorthandSyntax' => 'always') }
    assert_equal ['Style/HashSyntax EnforcedShorthandSyntax="always"'],
                 mismatch('Style/HashSyntax', config, nil).first
  end

  def test_default_configuration_matches
    config = { "Lint/Debugger" => Oracle.deep_dup(DEFAULTS["Lint/Debugger"]) }
    assert_equal [nil, {}], mismatch("Lint/Debugger", config)
  end

  def test_a_declared_setting_goes_to_the_toml_of_the_example
    defaults = Oracle.deep_dup(DEFAULTS["Style/StringLiterals"])
    assert_equal [nil, { "Style/StringLiterals" => { "enforced_style" => "single_quotes" } }],
                 mismatch("Style/StringLiterals", { "Style/StringLiterals" => defaults })
    # A value that the setting does not take is a mismatch.
    backticks = defaults.merge("EnforcedStyle" => "backticks")
    assert_equal ['Style/StringLiterals EnforcedStyle="backticks"'],
                 mismatch("Style/StringLiterals", { "Style/StringLiterals" => backticks }).first
    line_length = ->(patterns) { { "Layout/LineLength" => DEFAULTS["Layout/LineLength"].merge("Max" => 80, "AllowedPatterns" => patterns) } }
    toml = { "Layout/LineLength" => { "max" => 80, "allowed_patterns" => ['^\s*test\s', '(?i)^def\s+test_'] } }
    assert_equal [nil, toml], mismatch("Layout/LineLength", line_length.call(['^\s*test\s', /^def\s+test_/i]))
    # No patterns need no key, and a pattern that Rust's regex crate cannot
    # read is a mismatch.
    assert_equal [nil, { "Layout/LineLength" => { "max" => 80 } }], mismatch("Layout/LineLength", line_length.call(nil))
    assert_equal ['Layout/LineLength AllowedPatterns=[/(?<=a)b/]'],
                 mismatch("Layout/LineLength", line_length.call([/(?<=a)b/])).first
    # A setting of another cop goes to the toml only for a cop that reads it here.
    single = { "Style/StringLiterals" => { "EnforcedStyle" => "single_quotes" } }
    symbols = single.merge("Style/QuotedSymbols" => DEFAULTS["Style/QuotedSymbols"])
    assert_equal [nil, { "Style/QuotedSymbols" => { "enforced_style" => "same_as_string_literals" },
                         "Style/StringLiterals" => { "enforced_style" => "single_quotes" } }],
                 mismatch("Style/QuotedSymbols", symbols)
    assert_equal ['Style/StringLiterals EnforcedStyle="single_quotes"'],
                 mismatch("Lint/Debugger", single.merge("Lint/Debugger" => DEFAULTS["Lint/Debugger"])).first
    assert_equal '[cops."Layout/LineLength"]' "\n" 'allowed_patterns = ["^\\\\s*#"]' "\n",
                 Oracle.toml_text("Layout/LineLength" => { "allowed_patterns" => ['^\s*#'] })
  end

  def test_method_call_parentheses_accepts_pinned_truthy_macro_setting_only
    cop = 'Style/MethodCallWithArgsParentheses'
    [true, 'true'].each do |ignore|
      config = { cop => DEFAULTS[cop].merge('IgnoreMacros' => ignore) }
      assert_nil mismatch(cop, config, 'class Foo; bar :baz; end').first
    end
    config = { cop => DEFAULTS[cop].merge('IgnoreMacros' => false) }
    assert_equal ["#{cop} IgnoreMacros=false"], mismatch(cop, config).first
    config = { cop => DEFAULTS[cop].merge('IncludedMacros' => ['bar']) }
    assert_equal ["#{cop} IncludedMacros=[\"bar\"]"], mismatch(cop, config).first
  end

  def test_fixed_ruby_applicability_is_not_an_inert_config_key
    [3.3, 3.4].each do |target|
      config = { 'AllCops' => { 'TargetRubyVersion' => target },
                 'Style/ReverseFind' => DEFAULTS['Style/ReverseFind'] }
      assert_match(/requires Ruby >= 4.0/, mismatch('Style/ReverseFind', config).first.join)
    end
    config = { 'AllCops' => { 'TargetRubyVersion' => 4.0 },
               'Style/ReverseFind' => DEFAULTS['Style/ReverseFind'] }
    assert_equal [nil, { 'ruby_version' => '4.0' }], mismatch('Style/ReverseFind', config)
    config = { 'AllCops' => { 'TargetRubyVersion' => 3.3 },
               'Style/ItBlockParameter' => DEFAULTS['Style/ItBlockParameter'] }
    assert_match(/Ruby applicability/, mismatch('Style/ItBlockParameter', config).first.join)
    config['AllCops']['TargetRubyVersion'] = 3.4
    assert_nil mismatch('Style/ItBlockParameter', config).first
  end

  def test_arguments_forwarding_ruby_33_block_bug_is_not_the_fixed_34_contract
    cop = 'Style/ArgumentsForwarding'
    config = { 'AllCops' => { 'TargetRubyVersion' => 3.3 }, cop => DEFAULTS[cop] }
    [
      'def foo(*args); items.each { bar(*args) }; end',
      'def foo(**kwargs); -> { bar(**kwargs) }; end',
      'def foo(&block); items.each { bar(&block) }; end'
    ].each do |source|
      assert_match(/Ruby behavior:.*3.4/, mismatch(cop, config, source).first.join)
      assert_nil mismatch(cop, config.merge('AllCops' => { 'TargetRubyVersion' => 3.4 }), source).first
    end
    [
      'def foo(*args); bar(*args); end',
      'def foo(*args); bar(*args) { ordinary_call }; end',
      'def foo(*args); items.each { |*args| bar(*args) }; end',
      'def foo(*args); bar(*args); items.each { ordinary_call }; end',
      'def foo(*, **, &); items.each { bar(*, **, &) }; end',
      '# items.each { bar(*args) }'
    ].each { |source| assert_nil mismatch(cop, config, source).first, source }
    assert_nil mismatch('Style/EndlessMethod', { 'Style/EndlessMethod' => DEFAULTS['Style/EndlessMethod'], 'AllCops' => { 'TargetRubyVersion' => 3.3 } }, 'def foo = items.each { bar(*args) }').first
  end

  def test_other_cop_enables_use_fixed_pending_and_opt_in_defaults
    base = { 'Style/RedundantParentheses' => DEFAULTS['Style/RedundantParentheses'] }
    { 'Style/ReverseFind' => true, 'Style/FrozenStringLiteralComment' => false,
      'Style/ArrayCoercion' => false }.each do |other, enabled|
      assert_nil mismatch('Style/RedundantParentheses', base.merge(other => { 'Enabled' => enabled })).first
      assert_equal ["#{other} Enabled=#{!enabled}"],
                   mismatch('Style/RedundantParentheses', base.merge(other => { 'Enabled' => !enabled })).first
    end
  end

  def test_keys_a_custom_config_leaves_out_read_as_nil
    config = { "Lint/AssignmentInCondition" => {} }
    assert_equal ["Lint/AssignmentInCondition AllowSafeAssignment=nil"],
                 mismatch("Lint/AssignmentInCondition", config).first
  end

  def test_nil_false_and_empty_are_the_same_setting
    config = { "Lint/NestedMethodDefinition" => { "AllowedMethods" => nil,
                                                  "AllowedPatterns" => false } }
    assert_nil mismatch("Lint/NestedMethodDefinition", config).first
  end

  def test_fallbacks_stand_in_for_missing_keys
    config = { "Style/HashSyntax" => DEFAULTS["Style/HashSyntax"].reject { |k, _| k == "EnforcedShorthandSyntax" } }
    assert_equal ['Style/HashSyntax EnforcedShorthandSyntax="always"'],
                 mismatch("Style/HashSyntax", config, "{ a: a }\n").first
  end

  def test_shorthand_syntax_only_counts_for_hashes_with_omitted_or_omittable_values
    config = { "Style/HashSyntax" => DEFAULTS["Style/HashSyntax"] }
    %w[always never consistent either_consistent].each do |style|
      config["Style/HashSyntax"] = DEFAULTS["Style/HashSyntax"].merge("EnforcedShorthandSyntax" => style)
      ["x = { :a => 0, :b => 2 }\n", "{ a: b, \"c\": c }\n", "f(a: a.b, c: c?)\n",
       "x = {a: 1\n"].each do |source|
        assert_nil mismatch("Style/HashSyntax", config, source).first, "#{style}: #{source}"
      end
      ["{ a: a }\n", "f(a:)\n", "f a:\n", "{ a:a, b: 1 }\n", "a = 1\nf(a: a)\n"].each do |source|
        assert_equal ["Style/HashSyntax EnforcedShorthandSyntax=#{style.inspect}"],
                     mismatch("Style/HashSyntax", config, source).first, "#{style}: #{source}"
      end
    end
  end

  def test_allow_names_ending_in_numbers_only_counts_for_such_names
    config = { "Naming/MethodParameterName" =>
                 DEFAULTS["Naming/MethodParameterName"].merge("AllowNamesEndingInNumbers" => false) }
    assert_nil mismatch("Naming/MethodParameterName", config, "def foo(bar); end\n").first
    assert_equal ["Naming/MethodParameterName AllowNamesEndingInNumbers=false"],
                 mismatch("Naming/MethodParameterName", config, "def foo(bar1); end\n").first
  end

  def test_if_unless_modifier_passes_disabled_line_length_dependency_to_toml
    defaults = RuboCop::ConfigLoader.default_configuration.to_h
    [80, nil].each do |max|
      dependency = { 'Enabled' => false }
      dependency['Max'] = max if max
      config = { 'Layout/LineLength' => dependency }
      mismatch, toml = Oracle.config_mismatch('Style/IfUnlessModifier', config, defaults, 'if a; b; end')
      assert_nil mismatch
      assert_equal false, toml.fetch('Layout/LineLength').fetch('enabled')
      assert_includes Oracle.toml_text(toml), 'enabled = false'
    end
    mismatch, = Oracle.config_mismatch('Style/IfUnlessModifier', { 'Layout/LineLength' => { 'AllowURI' => false } }, defaults, 'work if url == "https://example.com/long"')
    refute_nil mismatch, 'fixed URI policy is not configurable'
  end

  def test_toml_keys_accept_any_value_and_are_written_out
    config = { "Layout/LineLength" => DEFAULTS["Layout/LineLength"].merge("Max" => 80) }
    assert_equal [nil, { "Layout/LineLength" => { "max" => 80 } }],
                 mismatch("Layout/LineLength", config)
  end

  def test_relevant_settings_only_count_for_matching_source
    config = { "Layout/LineLength" => { "Max" => 80, "URISchemes" => %w[http https] } }
    assert_nil mismatch("Layout/LineLength", config, "#{"#" * 81}\n").first
    assert_equal ["Layout/LineLength AllowHeredoc=nil"],
                 mismatch("Layout/LineLength", config, "x = <<~TEXT\n  y\nTEXT\n").first
  end

  def test_end_alignment_style_is_irrelevant_without_inline_conditionals
    config = { "Layout/IndentationWidth" => DEFAULTS["Layout/IndentationWidth"],
               "Layout/EndAlignment" => { "EnforcedStyleAlignWith" => "variable" } }
    ["if x\n a\nend\n", "class A\n   def x\n   end\nend\n",
     "x =\n  if y\n    z\n  end\n"].each do |source|
      assert_nil mismatch("Layout/IndentationWidth", config, source).first
    end
    ["x = if y\n a\nend\n", "foo.bar = while y\n a\nend\n"].each do |source|
      assert_equal ['Layout/EndAlignment EnforcedStyleAlignWith="variable"'],
                   mismatch("Layout/IndentationWidth", config, source).first
    end
  end

  def test_one_line_definition_allowance_only_matters_for_consecutive_one_liners
    cop = "Layout/EmptyLineBetweenDefs"
    config = { cop => DEFAULTS[cop].merge("AllowAdjacentOneLineDefs" => false) }
    ["def a\nend\ndef b\nend\n", "class K\ndef a; end\ndef b\nend\nend\n",
     "def a; end\nprivate\ndef b; end\n", "def a; end\n\ndef b; end\n"].each do |source|
      assert_nil mismatch(cop, config, source).first, source
    end
    ["def a; end\ndef b; end\n", "class K\ndef a = x\ndef b = y\nend\n",
     "class A; end; module B; end\n", "def a; end\n\n\ndef b; end\n"].each do |source|
      assert_equal ["#{cop} AllowAdjacentOneLineDefs=false"], mismatch(cop, config, source).first, source
    end
  end

  def test_magic_directive_case_is_source_sensitive_not_a_blanket_nil_skip
    cop = 'Style/MagicCommentFormat'
    config = { cop => DEFAULTS[cop].merge('DirectiveCapitalization' => nil) }
    ["# frozen-string-literal: true\nputs 1\n", "# -*- encoding: ASCII-8BIT; frozen-string-literal: true -*-\nputs 1\n",
     "x = '# FROZEN_STRING_LITERAL: true'\n", "puts 1\n# FROZEN_STRING_LITERAL: true\n",
     "# ordinary FROZEN_STRING_LITERAL\nputs 1\n", ""].each do |source|
      assert_nil mismatch(cop, config, source).first, source
    end
    ["# eNcOdInG: utf-8\nputs 1\n", "# typed: FROZEN-STRING-LITERAL\nputs 1\n",
     "# -*- encoding: UTF-8; FROZEN_STRING_LITERAL: true -*-\nputs 1\n"].each do |source|
      assert_equal ["#{cop} DirectiveCapitalization=nil"], mismatch(cop, config, source).first, source
    end
    assert_equal ["#{cop} DirectiveCapitalization=nil"], mismatch(cop, config, nil).first
    config[cop]['DirectiveCapitalization'] = 'foobar'
    assert_equal ["#{cop} DirectiveCapitalization=\"foobar\""], mismatch(cop, config, "puts 1\n").first
    config[cop]['DirectiveCapitalization'] = 'uppercase'
    assert_equal ["#{cop} DirectiveCapitalization=\"uppercase\""], mismatch(cop, config, "# encoding: utf-8\nputs 1\n").first
  end

  def test_other_cops_count_only_the_keys_given_enabled_included
    config = { "Style/RedundantParentheses" => {},
               "Style/TernaryParentheses" => { "Enabled" => false } }
    assert_equal ["Style/TernaryParentheses Enabled=false"],
                 mismatch("Style/RedundantParentheses", config).first
  end

  def test_any_of_accepts_each_listed_value
    [nil, 2].each do |width|
      config = { "Layout/LineLength" => DEFAULTS["Layout/LineLength"],
                 "Layout/IndentationStyle" => { "IndentationWidth" => width } }
      assert_nil mismatch("Layout/LineLength", config).first
    end
  end
end

class CaseTest < Minitest::Test
  def kase(expected, actual, errors = [])
    offenses = expected.map do |line, col, message = "m"|
      Oracle::ExpectedOffense.new(line, col, 1, message)
    end
    expectation = Oracle::Expectation.new(offenses: offenses, source: "x\n")
    Oracle::Case.new(id: "abc", expectation: expectation, toml: {},
                     actual: actual.map { |line, col| { line: line, col: col } },
                     errors: errors)
  end

  def test_match_has_no_signature
    assert_nil kase([[1, 1]], [[1, 1]]).signature
  end

  def test_misses_and_false_positives_are_multiset_differences
    found = kase([[1, 1, "a"], [1, 1, "b"], [2, 3]], [[1, 1], [2, 4]])
    assert_equal [[1, 1], [2, 3]], found.misses
    assert_equal [[2, 4]], found.false_positives
    assert_equal "miss 1:1 2:3; fp 2:4", found.signature
  end

  def test_identical_expected_offenses_count_once_like_the_rubocop_runner
    assert_nil kase([[1, 1], [1, 1]], [[1, 1]]).signature
    assert_equal "fp 1:1", kase([[1, 1], [1, 1]], [[1, 1], [1, 1]]).signature
  end

  def test_parse_errors_are_part_of_the_signature
    assert_equal "miss 1:1; error", kase([[1, 1]], [], ["error: syntax error"]).signature
  end
end

class BaselineTest < Minitest::Test
  def result(cases)
    { cop: "Lint/Debugger", spec: "/src/spec/rubocop/cop/lint/debugger_spec.rb", cases: cases,
      skipped: [], errors: [] }
  end

  def kase(id, signature_misses)
    offenses = signature_misses.map { |line, col| Oracle::ExpectedOffense.new(line, col, 1, "m") }
    expectation = Oracle::Expectation.new(offenses: offenses, source: "x\n", spec_line: 7,
                                          description: "Lint::Debugger flags   it")
    Oracle::Case.new(id: id, expectation: expectation, toml: {}, actual: [], errors: [])
  end

  def entry(id, signature, cop: "Lint/Debugger")
    Oracle::BaselineEntry.new(id: id, cop: cop, signature: signature, reason: "why")
  end

  def test_classifies_new_changed_stale_and_known_deviations
    cases = [kase("a" * 12, [[1, 1]]), kase("b" * 12, [[2, 1]]), kase("c" * 12, [[3, 1]]),
             kase("d" * 12, [])]
    entries = [entry("b" * 12, "miss 9:9"), entry("c" * 12, "miss 3:1"),
               entry("d" * 12, "miss 4:1"), entry("e" * 12, "miss 5:1"),
               entry("f" * 12, "miss 6:1", cop: "Style/Semicolon")].to_h { |e| [e.id, e] }
    verdict = Oracle.classify(result(cases), entries)
    kinds = verdict[:problems].map { |kind, kase, entry| [kind, (kase || entry).id] }
    assert_equal [[:new, "a" * 12], [:changed, "b" * 12], [:stale, "d" * 12],
                  [:stale, "e" * 12]], kinds
    assert_equal(["c" * 12], verdict[:known].map { |kase, _| kase.id })
    assert_equal 1, verdict[:counts][:match]
  end

  def test_write_then_read_keeps_reasons_and_marks_new_entries
    Dir.mktmpdir do |dir|
      path = File.join(dir, "baseline.txt")
      old = { entries: { "a" * 12 => entry("a" * 12, "miss 1:1") }, revision: nil }
      results = [result([kase("a" * 12, [[1, 1]]), kase("b" * 12, [[2, 1]])])]
      Oracle.write_baseline(old, results, dir, path)
      written = Oracle.read_baseline(path)[:entries]
      assert_equal([["a" * 12, "miss 1:1", "why"], ["b" * 12, "miss 2:1", "TODO: triage"]],
                   written.values.map { |e| [e.id, e.signature, e.reason] })
      assert_equal "lint/debugger_spec.rb:7 Lint::Debugger flags it",
                   written["a" * 12].comment.delete_prefix("spec/rubocop/cop/")
    end
  end
end

class ReportTest < Minitest::Test
  def result(**overrides)
    { cop: "Lint/Debugger", spec: "/src/spec/rubocop/cop/lint/debugger_spec.rb", cases: [],
      skipped: [], errors: [], fatal: nil, no_spec: false }.merge(overrides)
  end

  def report(results)
    options = Oracle::Options.new(quiet: true, verbose: false)
    status = nil
    out, = capture_io { status = Oracle.report(results, { entries: {} }, options) }
    [status, out]
  end

  def test_a_clean_run_passes_quietly
    status, out = report([result])
    assert_equal 0, status
    refute_match(/Lint\/Debugger/, out)
  end

  def test_a_cop_without_a_spec_fails_even_when_quiet
    status, out = report([result(cop: "Lint/Debuger", no_spec: true)])
    assert_equal 1, status
    assert_match %r{Lint/Debuger: no RuboCop spec}, out
  end

  def test_skips_are_counted_by_reason_and_verbose_output_uses_each_specs_location
    skips = ['config: EnforcedStyle', 'skip: requires Ruby 4.0', 'helper-only: metadata'].map do |reason|
      [Oracle::Expectation.new(spec: '/src/spec/rubocop/cop/style/second_spec.rb', spec_line: 9), reason]
    end
    options = Oracle::Options.new(verbose: true)
    out, = capture_io do
      assert_equal 0, Oracle.report([result(skipped: skips)], { entries: {} }, options)
    end
    assert_includes out, '3 skipped (config 1, helper-only 1, skip 1)'
    assert_includes out, 'second_spec.rb:9 skip: requires Ruby 4.0'
  end

  def test_baseline_refresh_cannot_accept_missing_or_unevaluated_specs
    error = Oracle::ExampleError.new(spec_line: 3, description: 'helper', error: 'unknown', recorded: 0)
    [result(no_spec: true), result(errors: [error])].each do |broken|
      Oracle.stub(:locate_rubocop, '/unused') do
        Oracle.stub(:load_rubocop, nil) do
          Oracle.stub(:note_rubocop_revision, nil) do
            Oracle.stub(:check_cop, broken) do
              Oracle.stub(:write_baseline, ->(*) { flunk 'must not rewrite the baseline' }) do
                capture_io do
                  assert_equal 1, Oracle.main(['--write-baseline', '--binary', '/bin/true', 'Lint/Debugger'])
                end
              end
            end
          end
        end
      end
    end
  end

  def test_examples_that_could_not_be_evaluated_fail_even_when_quiet
    error = Oracle::ExampleError.new(spec_line: 3, description: "it raises",
                                        error: "NameError: boom", recorded: 0)
    status, out = report([result(errors: [error])])
    assert_equal 1, status
    assert_match(/not evaluated .*debugger_spec.rb:3 it raises: NameError: boom/, out)
  end
end

class LintOutputTest < Minitest::Test
  def test_offenses_and_errors_are_attributed_to_their_snippets
    Dir.mktmpdir do |dir|
      binary = File.join(dir, "fake-rustocop")
      File.write(binary, <<~SH)
        #!/bin/sh
        echo 'e0/example.rb:2:3: E: Remove debugger entry point `pry`. (Lint/Debugger)'
        echo 'e0/example.rb:1:1: C: Other cop. (Style/Semicolon)'
        echo 'e1/lib/a b.rb: error: syntax error on line 1: unexpected end'
        echo 'e1/lib/a b.rb:1:1: internal error: Lint/Debugger panicked: boom' >&2
        exit 2
      SH
      File.chmod(0o755, binary)
      cases = [nil, "lib/a b.rb"].map do |file|
        expectation = Oracle::Expectation.new(offenses: [], source: "x\n", file: file)
        Oracle::Case.new(id: "x", expectation: expectation, toml: {}, actual: [], errors: [])
      end
      Oracle.run_cases("Lint/Debugger", cases, binary, File.join(dir, "snippets"))

      assert_equal [[2, 3]], cases[0].actual_positions
      assert_empty cases[0].errors
      assert_equal ["error: syntax error on line 1: unexpected end",
                    "internal error: Lint/Debugger panicked: boom"], cases[1].errors
      assert File.exist?(File.join(dir, "snippets", "config0", "e1", "lib", "a b.rb"))
    end
  end

  def test_a_failed_run_is_not_silence
    Dir.mktmpdir do |dir|
      binary = File.join(dir, "fake-rustocop")
      File.write(binary, "#!/bin/sh\necho 'error: rustocop.toml: unknown key' >&2\nexit 2\n")
      File.chmod(0o755, binary)
      expectation = Oracle::Expectation.new(offenses: [], source: "x\n")
      cases = [Oracle::Case.new(id: "x", expectation: expectation, toml: {}, actual: [],
                                errors: [])]
      _, stderr = capture_subprocess_io do
        assert_raises(SystemExit) do
          Oracle.run_cases("Lint/Debugger", cases, binary, File.join(dir, "snippets"))
        end
      end
      assert_match(/unknown key/, stderr)
    end
  end
end
