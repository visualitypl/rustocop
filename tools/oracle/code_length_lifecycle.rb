# frozen_string_literal: true
# RuboCop MIT, Copyright (c) 2012-26 Bozhidar Batsov.
module Oracle
  # The metrics-length specs end nine examples with
  # `offenses.first.location.last_line` (five whole-node end lines, four
  # `--lsp` header ranges), as do four metrics-complexity examples
  # (CyclomaticComplexity and AbcSize, one whole-method end line
  # and one `--lsp` header range each): the shim's `expect_offense` returns no
  # offense objects, so each errors right AFTER its one source expectation was
  # recorded. That expectation is evaluated or config-skipped like any other.
  # Account every such error individually against the recorded
  # shim errors and its declared custom Max (never a blanket exclusion); the
  # fixed-default end line of each shape is asserted by the Rust ports in
  # src/cops/metrics/*/tests.rs.
  module CodeLengthLifecycle
    COPS = %w[Metrics/MethodLength Metrics/BlockLength Metrics/ClassLength Metrics/ModuleLength
              Metrics/CyclomaticComplexity Metrics/AbcSize].freeze
    ERROR = "NoMethodError: undefined `location'"

    def self.account(cop, recorder, defaults)
      record = JSON.parse(File.read(File.join(__dir__, 'standard_reference.json'))).fetch('cops').find { |r| r['name'] == cop }
      actual = recorder.errors.map do |e|
        { 'spec' => Oracle.relative_spec(e.spec), 'line' => e.spec_line, 'description' => e.description,
          'error' => e.error, 'recorded' => e.recorded }
      end
      raise "changed #{cop} shim errors" unless actual == record.fetch('spec_accounting').fetch('errors')

      accounted = recorder.errors.map do |error|
        max = error.config&.dig(cop, 'Max')
        unless error.recorded == 1 && error.error == ERROR && max.is_a?(Integer) && max != defaults.fetch(cop).fetch('Max')
          raise "unaccounted #{cop} shim error at line #{error.spec_line}"
        end
        [error, "config: Max #{max} (offense-range assertion after its recorded expectation; " \
                'fixed-default range asserted in src/cops/metrics)']
      end
      recorder.errors.clear
      accounted
    end
  end
end
