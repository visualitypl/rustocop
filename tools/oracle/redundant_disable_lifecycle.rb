# frozen_string_literal: true
# RuboCop lifecycle examples (MIT), Copyright (c) 2012-26 Bozhidar Batsov.
# Capture injected raw evidence separately from ordinary source investigation.
module Oracle
  module RedundantDisableLifecycle
    COP = 'Lint/RedundantCopDisableDirective'
    def self.check(result, recorder, tmp)
      raise 'missing lifecycle examples' unless recorder.expectations.size == 67 && recorder.errors.empty? && recorder.skipped.empty?
      rows = recorder.expectations.map do |e|
        # Preview is not a project setting; pending is always enabled. Preserve
        # original source and explicitly adapt only these two expected results.
        if [104, 129].include?(e.spec_line)
          e.offenses = [ExpectedOffense.new(1, 1, 0, 'fixed policy: preview absent / pending enabled')]
        end
        { source: e.source, spec_line: e.spec_line,
          disabled: e.config.filter_map { |n, c| n if c.is_a?(Hash) && c['Enabled'] == false },
          lifecycle: e.lifecycle }
      end
      path = File.join(tmp, 'redundant-disable-lifecycle.json')
      File.write(path, JSON.generate(names: RuboCop::Cop::Registry.global.names, rows: rows))
      # The raw-offense injection seam is cfg(test)-private. Do not pretend
      # `--only` runs ordinary detectors or expose a public injection setting.
      stdout, stderr, status = Open3.capture3({ 'RUSTOCOP_REDUNDANT_DISABLE_REFERENCE' => path },
        'cargo', '+1.99.0', 'test', '--locked', '--lib',
        'cops::lint::redundant_cop_disable_directive::tests::pinned_lifecycle', '--', '--exact', chdir: ROOT)
      raise "lifecycle consumer failed: #{stdout}#{stderr}" unless status.success?
      actual = JSON.parse(File.read("#{path}.out"))
      recorder.expectations.zip(actual).each do |e, row|
        result[:cases] << Case.new(id: Oracle.case_id(COP, e, {}), expectation: e, toml: {},
          actual: row.fetch('positions').zip(row.fetch('messages')).map { |p, m| { line: p[0], col: p[1], message: m } }, errors: [])
      end
      result
    end
    def expect_offense(*)
      super
      capture_disable_lifecycle
    end
    def expect_no_offenses(*)
      super
      capture_disable_lifecycle
    end
    def capture_disable_lifecycle
      @__disable_row = Oracle.recorder.expectations.last
      @__disable_row.lifecycle = {
        raw: offenses.map { |o| { cop: o.cop_name, line: o.line, last: o.location.last_line } },
        correction: nil, hold: false
      }
    end
    def expect_correction(source, **)
      @__disable_row.lifecycle[:correction] = source
      super
    end
    def expect_no_corrections
      @__disable_row.lifecycle[:hold] = true
      super
    end
  end
end
