# frozen_string_literal: true
# RuboCop MIT, Copyright (c) 2012-26 Bozhidar Batsov.
module Oracle
  # The 15 main assertions use _investigate and mocked buffer names, not source
  # annotations. Account for each shim error, then replay declared AND fixed
  # defaults using the real Team lifecycle and location-free project CLI.
  module GemFilenameLifecycle
    COP = 'Bundler/GemFilename'
    def self.check(result, recorder, binary, tmp)
      record = JSON.parse(File.read(File.join(__dir__, 'gem_filename_reference.json')))
      actual_errors = recorder.errors.map do |e|
        { 'spec' => Oracle.relative_spec(e.spec),
          'line' => e.spec_line, 'description' => e.description, 'error' => e.error, 'recorded' => e.recorded }
      end
      raise 'changed GemFilename main/shared applicability' unless recorder.expectations.empty? && recorder.skipped.empty? &&
        actual_errors == record.fetch('spec_accounting').fetch('errors') && actual_errors.size == 15
      rows = record.fetch('lifecycle_evidence')
      recorder.errors.each_with_index do |error, index|
        style = error.config.fetch(COP).fetch('EnforcedStyle')
        name = error.description.match(/with (non-root )?(\S+) file path/)
        raise 'unaccounted filename assertion' unless name
        path = (name[1] ? 'spec/' : '') + name[2]
        row = rows.find { |r| r['path'] == path && r['declared_style'] == style }
        raise 'missing lifecycle row' unless row
        # Identity, not formatter fallback 1:1. Sources are parsed, NEVER run.
        data = Marshal.load(Marshal.dump(RuboCop::ConfigLoader.default_configuration.to_h))
        data['AllCops'].merge!('TargetRubyVersion' => 3.4, 'ParserEngine' => 'parser_prism', 'UseProjectIndex' => false)
        config = RuboCop::Config.new(data)
        source = row.fetch('source')
        processed = RuboCop::ProcessedSource.new(source, 3.4, path, parser_engine: :parser_prism)
        processed.config = config
        processed.registry = RuboCop::Cop::Registry.global
        klass = RuboCop::Cop::Registry.global.find_by_cop_name(COP)
        raise 'unexpected source correction support' if klass.support_autocorrect?
        team = RuboCop::Cop::Team.new([klass.new(config)], config, raise_error: true)
        findings = team.investigate(processed).offenses.reject(&:disabled?)
        raise 'non-global reference finding' unless findings.all? { |o| o.location.equal?(RuboCop::Cop::Offense::NO_LOCATION) }
        expected = row.fetch('fixed_result').fetch('offenses').map { |o| o.fetch('message').delete_prefix(COP + ': ') }
        raise 'fixed lifecycle mismatch' unless findings.map { |o| o.message.delete_prefix(COP + ': ') } == expected && team.errors.empty?
        root = File.join(tmp, "gem-filename-#{index}")
        FileUtils.mkdir_p(File.join(root, '.git'))
        FileUtils.mkdir_p(File.dirname(File.join(root, path)))
        File.write(File.join(root, 'rustocop.toml'), '')
        File.binwrite(File.join(root, path), source)
        before = File.stat(File.join(root, path))
        found = []
        [[], ['--fix']].each do |flags|
          stdout, stderr, status = Open3.capture3(binary, '--only', COP, *flags, path, chdir: root)
          wanted = expected.map { |message| "#{path}: C: #{message} (#{COP})\n" }
          lines = stdout.lines.grep(/\(Bundler\/GemFilename\)/)
          raise "GemFilename CLI mismatch: #{stdout}#{stderr}" unless lines == wanted && stderr.empty? && status.exitstatus == (expected.empty? ? 0 : 1)
          after = File.stat(File.join(root, path))
          raise 'GemFilename wrote source/metadata' unless File.binread(File.join(root, path)) == source &&
            [before.ino, before.mode, before.mtime] == [after.ino, after.mode, after.mtime]
          found = expected.map { |message| { line: nil, col: nil, message: message } }
        end
        e = Expectation.new(cop: COP, source: source, file: path, description: error.description,
          spec: error.spec, spec_line: error.spec_line,
          offenses: expected.map { |m| ExpectedOffense.new(nil, nil, nil, m) })
        result[:cases] << Case.new(id: Digest::SHA256.hexdigest([COP, error.description, source].join("\0"))[0, 12],
          expectation: e, toml: {}, path: path, actual: found, errors: [])
      end
      result
    end
  end
end
