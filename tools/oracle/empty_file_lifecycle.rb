# frozen_string_literal: true

module Oracle
  # Only EmptyFile's two global-offense examples need this lifecycle. Unlike
  # ordinary annotation fixtures, they investigate `source` through a real
  # Commissioner and assert count/message/severity. Do not blackhole those
  # assertions or infer expected findings from the detector being checked.
  module EmptyFileLifecycle
    def processed_source
      @__empty_file_processed_source ||= RuboCop::ProcessedSource.new(
        source, ruby_version, 'test', parser_engine: :parser_prism)
    end

    def eq(expected)
      expected
    end

    def expect(actual)
      unless @__empty_file_recorded
        # Both pinned lifecycle examples explicitly expect one global offense.
        # Record at the actual assertion, not at the lazy `offenses` let.
        @__example.record(self, source, nil,
                          [ExpectedOffense.new(1, 1, 0, 'Empty file detected.')])
        @__empty_file_recorded = true
      end
      EqualityAssertion.new(actual)
    end

    class EqualityAssertion
      def initialize(actual)
        @actual = actual
      end

      def to(expected)
        return if @actual == expected

        raise "EmptyFile lifecycle: expected #{expected.inspect}, got #{@actual.inspect}"
      end
    end
  end
end
