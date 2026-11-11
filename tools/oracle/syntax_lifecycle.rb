# frozen_string_literal: true

module Oracle
  # Syntax's eight pinned examples assert real Commissioner/global-error facts,
  # not expect_offense annotations. Assert them, then compare the rebuilt CLI's
  # fatal file-error channel separately. Never execute the supplied Ruby source.
  module SyntaxLifecycle
    def parser_engine
      :parser_prism
    end

    def processed_source
      @__syntax_processed_source ||= begin
        processed = RuboCop::ProcessedSource.new(source, 3.4, 'supplied.rb', parser_engine: :parser_prism)
        processed.config = config
        processed.registry = RuboCop::Cop::Registry.global
        processed
      end
    end

    def eq(expected)
      # Four pinned messages literally embed3.3 (not ruby_version). Adapt only
      # that advice to the fixed3.4 parser used for the actual investigation.
      expected = expected.sub('(Using Ruby 3.3 parser;', '(Using Ruby 3.4 parser;') if expected.is_a?(String)
      ->(actual) { actual == expected }
    end

    def contain_exactly(*expected)
      ->(actual) { actual.sort == expected.sort }
    end

    def be_empty
      ->(actual) { actual.empty? }
    end

    def expect(actual)
      unless @__syntax_recorded
        # Explicit fixed3.4/native-Prism source accounting. The translated
        # EOF fixture has one diagnostic; native Prism additionally reports
        # its opening keyword. Both must survive, not become baseline masks.
        positions = case source.b
                    when '('.b then [[1, 1]]
                    when "# warning: ambiguous `*` has been interpreted as an argument prefix\nfoo *some_array\n(\n".b then [[3, 2]]
                    when "if true\n  puts 'x'\n".b then [[2, 11], [1, 1]]
                    when "# \xf9\n".b then [[1, 1]] # Global exception: CLI must omit line/col.
                    else raise "Syntax lifecycle: unaccounted source #{source.inspect}"
                    end
        @__example.record(self, source, nil, positions.map { |line, col| ExpectedOffense.new(line, col, 0, '') })
        @__syntax_recorded = true
      end
      Assertion.new(actual)
    end

    class Assertion
      def initialize(actual)
        @actual = actual
      end

      def to(matcher)
        raise "Syntax lifecycle: assertion failed for #{@actual.inspect}" unless matcher.call(@actual)
      end

      def not_to(matcher)
        raise "Syntax lifecycle: negative assertion failed for #{@actual.inspect}" if matcher.call(@actual)
      end
    end

    def self.run(metadata)
      original = RuboCop::LSP.enabled?
      metadata[:lsp] ? RuboCop::LSP.enable : RuboCop::LSP.disable
      yield
    ensure
      original ? RuboCop::LSP.enable : RuboCop::LSP.disable
    end

    def self.lint(cases, binary, root)
      by_path = cases.to_h { |kase| [kase.path, kase] }
      stdout, stderr, status = Open3.capture3(binary, '--only', 'Lint/Syntax', '--format', 'github', *by_path.keys, chdir: root)
      raise "Syntax lifecycle CLI failed: #{stdout}#{stderr}" unless status.exitstatus == 2 && stderr.empty?

      stdout.each_line do |line|
        next unless line.start_with?('::error ')

        match = /\A::error file=(?<path>[^,]+)(?:,line=(?<line>\d+),col=(?<col>\d+))?,title=rustocop::(?<message>.*)/.match(line)
        raise "Syntax lifecycle: unexpected annotation #{line}" unless match

        kase = by_path.fetch(match[:path])
        unless match[:message].end_with?(' (Lint/Syntax)')
          kase.errors << "unnamed fatal error: #{line}"
          next
        end
        global = kase.expectation.source.b == "# \xf9\n".b
        if global != match[:line].nil?
          kase.errors << "wrong global/ranged fatal location: #{line}"
        end
        kase.actual << { line: match[:line]&.to_i || 1, col: match[:col]&.to_i || 1, message: match[:message] }
      end
    end
  end
end
