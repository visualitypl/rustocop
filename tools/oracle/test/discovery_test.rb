# frozen_string_literal: true

# Offline: no RuboCop checkout or runtime gems needed.
require "minitest/autorun"
require "minitest/mock"
require_relative "../oracle"

class DiscoveryTest < Minitest::Test
  def test_inventory_uses_every_exact_source_and_spec_mapping
    inventory = JSON.parse(File.read(Oracle::STYLE_INVENTORY))
    inventory.fetch("cops").each do |record|
      mapping = Oracle.source_and_specs("/checkout", record.fetch("name"))
      assert_equal File.join("/checkout", record.fetch("source")), mapping.fetch(:source)
      assert_equal record.fetch("specs").map { |p| File.join("/checkout", p) }, mapping.fetch(:specs)
    end
  end

  def test_conditional_assignment_has_both_split_specs_and_no_prefix_glob
    Dir.mktmpdir do |dir|
      paths = Oracle.spec_paths(dir, "Style/ConditionalAssignment")
      assert_equal %w[conditional_assignment_assign_in_condition_spec.rb conditional_assignment_assign_to_condition_spec.rb],
                   paths.map { |path| File.basename(path) }
      FileUtils.mkdir_p(File.dirname(paths.first))
      File.write(File.join(File.dirname(paths.first), "conditional_assignment_spec.rb"), "")
      File.write(File.join(File.dirname(paths.first), "conditional_assignment_other_cop_spec.rb"), "")
      assert_equal paths, Oracle.spec_paths(dir, "Style/ConditionalAssignment")
      result = Oracle.check_cop("Style/ConditionalAssignment", dir, {}, "unused", dir)
      assert result[:no_spec]
      assert_includes result[:fatal], File.basename(paths.first)
      assert_includes result[:fatal], File.basename(paths.last)
    end
  end

  def test_selection_rejects_unimplemented_cops_before_prerequisites_or_baseline_writes
    _out, err = capture_io do
      assert_equal 2, Oracle.main(['--rubocop', '/nonexistent', '--write-baseline', 'Style/NotImplemented'])
    end
    assert_includes err, 'unknown or unimplemented cops: Style/NotImplemented'
  end

  def test_duplicate_spec_paths_are_returned_only_once
    record = { 'name' => 'Style/Example', 'source' => 'lib/example.rb',
               'specs' => ['spec/example_spec.rb', 'spec/example_spec.rb'] }
    JSON.stub(:parse, { 'cops' => [record] }) do
      assert_equal ['/checkout/spec/example_spec.rb'], Oracle.spec_paths('/checkout', 'Style/Example')
    end
  end

  def test_conventional_non_style_mapping_does_not_pick_nearby_prefixes
    assert_equal ["/checkout/spec/rubocop/cop/lint/debugger_spec.rb"],
                 Oracle.spec_paths("/checkout", "Lint/Debugger")
  end

  def test_syntax_keeps_conventional_main_spec_not_integration_context
    inventory = JSON.parse(File.read(File.join(Oracle::ROOT, 'tools/oracle/lint_reference.json')))
    record = inventory.fetch('cops').find { |entry| entry.fetch('name') == 'Lint/Syntax' }
    mapping = Oracle.source_and_specs('/checkout', 'Lint/Syntax')
    assert_equal File.join('/checkout', record.fetch('source')), mapping.fetch(:source)
    main = record.fetch('spec_roles').select { |_, role| role == 'main_cop' }.keys
    assert_equal main.map { |path| File.join('/checkout', path) }, mapping.fetch(:specs)
    assert_equal ['spec/rubocop/comment_config_spec.rb'], record.fetch('specs') - main
    # This proves discovery only: Syntax has lifecycle assertions, not ordinary
    # expect_offense fixtures; those lifecycle checks run explicitly elsewhere.
    Dir.mktmpdir do |dir|
      result = Oracle.check_cop('Lint/Syntax', dir, {}, 'must-not-run', dir)
      assert result[:no_spec]
      assert_empty result[:cases]
      assert_includes result[:fatal], 'lib/rubocop/cop/lint/syntax.rb'
      assert_includes result[:fatal], 'spec/rubocop/cop/lint/syntax_spec.rb'
    end
  end

  def test_unimplemented_lint_is_rejected_before_reference_lookup_or_baseline_writes
    _out, err = capture_io do
      assert_equal 2, Oracle.main(['--rubocop', '/nonexistent', '--write-baseline', 'Lint/NotImplemented'])
    end
    assert_includes err, 'unknown or unimplemented cops: Lint/NotImplemented'
  end

  def test_no_match_is_an_error_without_running_a_binary
    Dir.mktmpdir do |dir|
      result = Oracle.check_cop("Lint/NoSuchCop", dir, {}, "unused", dir)
      assert result[:no_spec]
      assert_empty result[:cases]
      options = Oracle::Options.new(quiet: true)
      capture_io { assert_equal 1, Oracle.report([result], { entries: {} }, options) }
    end
  end
end
