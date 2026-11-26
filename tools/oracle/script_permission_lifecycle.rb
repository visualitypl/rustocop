# frozen_string_literal: true
# RuboCop spec fixtures (MIT), Copyright (c) 2012-26 Bozhidar Batsov.
require 'tempfile'
require 'fileutils'

module Oracle
  # Metadata examples cannot be flattened into ordinary 0644 disk snippets:
  # preserve each real spec's mode/stdin/virtual provenance before its teardown.
  module ScriptPermissionLifecycle
    COP = 'Lint/ScriptPermission'
    class << self
      attr_accessor :rows, :files, :requests, :permission_assertions

      def reset
        self.rows, self.files, self.requests, self.permission_assertions = [], [], [], []
      end

      def check(result, recorder, binary, tmp)
        raise 'incomplete ScriptPermission lifecycle' unless recorder.errors.empty? && recorder.skipped.empty? && recorder.expectations.size == 8
        recorder.expectations.each_with_index do |e, index|
          facts = e.lifecycle
          root = File.join(tmp, "script-permission-#{index}")
          FileUtils.mkdir_p(File.join(root, '.git'))
          File.write(File.join(root, 'rustocop.toml'), '')
          name = e.file || 'virtual.rb'
          path = File.join(root, name)
          File.binwrite(path, e.source) if facts.fetch(:disk)
          File.chmod(facts.fetch(:mode), path) if facts.fetch(:disk) # fixture only
          before = File.stat(path) if facts.fetch(:disk)
          actual = nil
          # Virtual buffers use stdin, never a disk-backed ordinary oracle case.
          modes = facts.fetch(:disk) && !facts.fetch(:stdin) ? [[], ['--fix']] : [['--stdin']]
          modes.each do |flags|
            stdout, stderr, status = Open3.capture3(binary, '--only', COP, *flags, name,
              chdir: root, stdin_data: e.source)
            raise "ScriptPermission CLI failed: #{stdout}#{stderr}" unless [0, 1].include?(status.exitstatus) && stderr.empty?
            found = stdout.lines.filter_map do |line|
              match = /\A#{Regexp.escape(name)}:(\d+):(\d+): W: (.*) \(Lint\/ScriptPermission\)\s*\z/.match(line)
              { line: match[1].to_i, col: match[2].to_i, message: match[3] } if match
            end
            wanted = e.offenses.map { |o| [o.line, o.col, o.message] }
            raise "ScriptPermission CLI mismatch #{wanted.inspect} != #{found.inspect}" unless wanted == found.map { |o| o.values_at(:line, :col, :message) }
            actual ||= found
            if before
              after = File.stat(path)
              raise 'ScriptPermission wrote to input' unless File.binread(path) == e.source &&
                [after.dev, after.ino, after.mode, after.mtime] == [before.dev, before.ino, before.mode, before.mtime]
            end
          end
          result[:cases] << Case.new(id: Oracle.case_id(COP, e, {}), expectation: e,
            toml: {}, path: name, actual: actual, errors: [])
        end
        result
      end
    end
    reset

    def expect_offense(source, file = nil, **)
      plain, expected = Annotations.parse(source)
      investigate_supplied(plain, file, expected)
    end

    def expect_no_offenses(source, file = nil)
      investigate_supplied(source, file, [])
    end

    def investigate_supplied(source, file, expected)
      ScriptPermissionLifecycle.files << file if file
      processed = RuboCop::ProcessedSource.new(source, 3.4, file&.path, parser_engine: :parser_prism)
      raise 'invalid reference source' unless processed.valid_syntax?
      processed.config = config
      processed.registry = RuboCop::Cop::Registry.global
      @script_source = source
      @script_requests_before = ScriptPermissionLifecycle.requests.size
      target = cop
      target.instance_variable_get(:@options)[:autocorrect] = true
      found = _investigate(target, processed)
      actual = found.reject(&:disabled?).map { |o| [o.line, o.column + 1, o.message] }
      wanted = expected.map { |o| [o.line, o.col, o.message] }
      raise "metadata lifecycle mismatch #{wanted.inspect} != #{actual.inspect}" unless wanted == actual
      @__example.record(self, source, file && File.basename(file.path), expected)
      Oracle.recorder.expectations.last.lifecycle = {
        disk: !file.nil?, mode: file && file.stat.mode & 0o777, stdin: (options || {}).key?(:stdin)
      }
      ScriptPermissionLifecycle.rows << { source: source, expected: wanted, file: file&.path,
        options: options, requests: ScriptPermissionLifecycle.requests.size - @script_requests_before }
    end

    def _investigate(target, processed)
      team = RuboCop::Cop::Team.new([target], config, raise_error: true)
      team.defer_corrections = true
      # Run the real pinned correction, intercepting ONLY its non-source effect.
      Thread.current[:script_permission_requests] = ScriptPermissionLifecycle.requests
      report = team.investigate(processed)
      raise team.errors.inspect unless team.errors.empty?
      raise 'unexpected source correction' if team.updated_source && team.updated_source != @script_source
      report.offenses
    ensure
      Thread.current[:script_permission_requests] = nil
    end

    def expect_no_corrections
      raise 'no metadata investigation' unless @script_source
    end

    def be_executable
      ->(stat) { stat.executable? }
    end

    def expect(actual, **, &block)
      raise 'unexpected assertion type' unless actual.is_a?(File::Stat) && !block
      PermissionAssertion.new(actual, ScriptPermissionLifecycle.requests.size - @script_requests_before)
    end

    class PermissionAssertion
      def initialize(stat, requests)
        @stat, @requests = stat, requests
      end
      def to(matcher)
        # The original first example expects the chmod side effect. Account for
        # its actual +x request, explicitly withheld by policy, not a skip.
        raise 'missing dry-run +x request' unless @requests == 1
        raise 'reference mutated permissions' if matcher.call(@stat)
        ScriptPermissionLifecycle.permission_assertions << :non_source_chmod_withheld
      end
      def not_to(matcher)
        raise 'unexpected chmod request' unless @requests.zero?
        raise 'reference mutated permissions' if matcher.call(@stat)
        ScriptPermissionLifecycle.permission_assertions << :unchanged
      end
    end
  end
end

module CaptureScriptPermissionCorrection
  def chmod(mode, path, *)
    requests = Thread.current[:script_permission_requests]
    return super unless requests && mode.is_a?(String)
    raise 'unexpected permission correction' unless mode == '+x'
    requests << [mode, path]
  end
end
FileUtils.singleton_class.prepend(CaptureScriptPermissionCorrection)
