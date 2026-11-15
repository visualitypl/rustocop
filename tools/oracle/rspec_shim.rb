# frozen_string_literal: true

# A minimal stand-in for RSpec, just enough to evaluate RuboCop's cop specs
# and record every `expect_offense` / `expect_no_offenses` call instead of
# asserting on it.
#
# Example groups are classes and examples run on fresh instances, as in RSpec:
# `let` defines memoized methods, nested groups subclass their parent,
# `before` hooks run parent-first, shared examples are class_exec'd into a
# nested group. The `:config` shared context, CopHelper's `ruby_version` and
# the `:rubyXX` contexts are emulated as RuboCop's spec suite runs them with
# `PARSER_ENGINE=parser_prism`: target Rubies below 3.3 read as 3.3, and
# examples tagged `unsupported_on: :prism` or `broken_on: :prism` are skipped.
#
# The annotation parsing (`format_offense`, `AnnotatedSource.parse`) is ported
# from RuboCop's lib/rubocop/rspec/expect_offense.rb (MIT License,
# Copyright (c) 2012-26 Bozhidar Batsov).

require 'stringio'
require_relative "../fixed_policy"
require_relative "empty_file_lifecycle"
require_relative "script_permission_lifecycle"
require_relative "syntax_lifecycle"
require_relative "redundant_disable_lifecycle"

module Oracle
  # The lowest target Ruby RuboCop's Prism engine parses.
  PRISM_MINIMUM_RUBY = 3.3

  # One `expect_offense` or `expect_no_offenses` call. `correction` is the
  # source that a following `expect_correction` expects, `:none` after
  # `expect_no_corrections`; `correction_loop` is its `loop:`.
  Expectation = Struct.new(
    :cop, :spec, :spec_line, :description, :source, :file, :offenses,
    :config, :ruby_version, :skip, :lifecycle, :correction, :correction_loop, keyword_init: true
  )

  # An expected offense: 1-based line, 1-based column, caret count, message.
  ExpectedOffense = Struct.new(:line, :col, :carets, :message)

  # An example that could not be evaluated to its end.
  ExampleError = Struct.new(:cop, :spec, :spec_line, :description, :error, :recorded, :config,
                            keyword_init: true)

  # Raised by spec helpers the shim does not emulate.
  class Unsupported < StandardError; end

  # Raised by `skip` inside an example.
  class SkipExample < StandardError; end

  # Accepts any message: stands in for RSpec matchers and `expect(...)`.
  class Blackhole < BasicObject
    def method_missing(*_args, **_kwargs)
      self
    end

    def respond_to_missing?(*)
      true
    end
  end

  # The annotated-source format of `expect_offense`, ported from RuboCop.
  module Annotations
    ANNOTATION_PATTERN = /\A\s*((?<!\\)\^+|\^{}) ?/.freeze
    ANNOTATION_PARTS = /\A(?<indent>\s*)(?<carets>\^{}|\^+) ?(?<message>.*)\z/m.freeze

    module_function

    # Apply `%{x}`, `^{x}` and `_{x}` replacements.
    def format_offense(source, **replacements)
      replacements.each do |keyword, value|
        value = value.to_s
        source = source.gsub("%{#{keyword}}", value)
                       .gsub("^{#{keyword}}", "^" * value.size)
                       .gsub("_{#{keyword}}", " " * value.size)
      end
      source
    end

    # Split annotated source into plain source and expected offenses.
    def parse(annotated)
      lines = []
      annotations = []
      annotated.each_line do |line|
        if ANNOTATION_PATTERN.match?(line)
          annotations << [lines.size, line]
        else
          lines << line
        end
      end
      annotations.each { |a| a[0] = 1 } if lines.empty?

      offenses = annotations.map do |line_number, text|
        parts = ANNOTATION_PARTS.match(text.chomp)
        carets = parts[:carets] == "^{}" ? 0 : parts[:carets].size
        ExpectedOffense.new(line_number, parts[:indent].size + 1, carets, parts[:message])
      end
      [lines.join, offenses.sort_by { |o| [o.line, o.col, o.carets] }]
    end
  end

  # Collects what the evaluated specs recorded.
  class Recorder
    attr_reader :expectations, :errors, :skipped

    def initialize
      @expectations = []
      @errors = []
      @skipped = []
    end
  end

  class << self
    attr_accessor :recorder, :current_spec, :current_cop

    # Load one spec file and run its examples, recording into `recorder`.
    def evaluate_spec(path, cop_name, recorder)
      self.recorder = recorder
      self.current_spec = path
      self.current_cop = cop_name
      @top_groups = []
      load(path)
      @top_groups.each { |group| group.each_example(&:run) }
    ensure
      self.current_spec = nil
    end

    def register_top_group(group)
      @top_groups << group
    end

    # A deep copy, so that `before` hooks mutating a nested config hash
    # cannot leak into the default configuration.
    def deep_dup(object)
      case object
      when Hash then object.to_h { |k, v| [deep_dup(k), deep_dup(v)] }
      when Array then object.map { |v| deep_dup(v) }
      when String then object.dup
      else object
      end
    end

    # `describe`/`it` arguments: a leading description, then symbol tags
    # and metadata hashes.
    def split_args(args)
      args = args.compact
      description = args.first.is_a?(Hash) || args.first.is_a?(Symbol) ? nil : args.shift
      metadata = {}
      args.each do |arg|
        case arg
        when Symbol then metadata[arg] = true
        when Hash then metadata.merge!(arg)
        end
      end
      [description, metadata]
    end

    # The target Ruby a `:rubyXY` tag selects, as the Prism engine runs it.
    def tagged_ruby_version(metadata)
      versions = metadata.keys.filter_map do |key|
        match = /\Aruby(\d)(\d)\z/.match(key.to_s)
        "#{match[1]}.#{match[2]}".to_f if match
      end
      return if versions.empty?

      [versions.last, PRISM_MINIMUM_RUBY].max
    end

    # A one-line description of an exception raised by an example.
    def describe_error(error)
      message = error.is_a?(NameError) ? "undefined `#{error.name}'" : error.message
      "#{error.class}: #{message.lines.first.to_s.chomp[0, 160]}"
    end

    # Whether the Prism-engine spec run excludes examples with this metadata.
    def prism_excluded?(metadata)
      metadata[:unsupported_on] == :prism || metadata[:broken_on] == :prism
    end
  end

  # RuboCop's `config` shared context and CopHelper's lets.
  CONFIG_CONTEXT = proc do
    let(:cop_class) { described_class }
    let(:cop_config) { {} }
    let(:other_cops) { {} }
    let(:cop_options) { {} }
    let(:gem_versions) { {} }
    let(:path_sourced_gems) { [] }
    let(:all_cops_config) do
      rails = { "TargetRubyVersion" => ruby_version }
      rails["TargetRailsVersion"] = rails_version if rails_version
      rails
    end
    let(:cur_cop_config) do
      defaults = RuboCop::ConfigLoader.default_configuration.for_cop(cop_class)
      Oracle.deep_dup(defaults.to_h)
            .merge("Enabled" => true, "AutoCorrect" => "always")
            .merge(cop_config)
    end
    let(:config) do
      hash = { "AllCops" => all_cops_config, cop_class.cop_name => cur_cop_config }
             .merge!(other_cops)
      RuboCop::Config.new(hash, "#{Dir.pwd}/.rubocop.yml")
    end
    let(:cop) { cop_class.new(config, cop_options) }
  end

  # An example group: a class whose instances run its examples.
  class ExampleGroup
    class << self
      attr_reader :description, :metadata, :examples, :children, :hooks, :around_hooks, :shared

      def setup(description, metadata, parent)
        @description = description
        @metadata = (parent ? parent.metadata : {}).merge(metadata)
        if description.is_a?(Module) && !@metadata.key?(:described_class)
          @metadata[:described_class] = description
        end
        @examples = []
        @children = []
        @hooks = []
        @around_hooks = []
        @shared = {}
        class_exec(&CONFIG_CONTEXT) if metadata[:config]
        version = Oracle.tagged_ruby_version(metadata)
        let(:ruby_version) { version } if version
      end

      def group_parent
        superclass if superclass < ExampleGroup
      end

      def subgroup(description, metadata, &block)
        group = Class.new(self)
        group.setup(description, metadata, self)
        group.class_exec(&block) if block
        @children << group
        group
      end

      def describe(*args, &block)
        description, metadata = Oracle.split_args(args)
        subgroup(description, metadata, &block)
      end
      alias_method :context, :describe
      alias_method :feature, :describe

      def xdescribe(*args, &block)
        description, metadata = Oracle.split_args(args)
        subgroup(description, metadata.merge(skip: "xdescribe"), &block)
      end
      alias_method :xcontext, :xdescribe

      def it(*args, &block)
        description, metadata = Oracle.split_args(args)
        metadata[:skip] = "pending" unless block
        line = caller_locations(1, 1).first.lineno
        @examples << Example.new(self, description, metadata, block, line)
      end
      alias_method :specify, :it
      alias_method :example, :it

      def xit(*args, &block)
        description, metadata = Oracle.split_args(args)
        it(description, metadata.merge(skip: "xit"), &block)
      end
      alias_method :xspecify, :xit

      def pending(*args, &block)
        description, metadata = Oracle.split_args(args)
        it(description, metadata.merge(skip: "pending"), &block)
      end
      alias_method :skip, :pending

      # As in RSpec: the body is a method of a module the group includes,
      # so that `super()` in a nested group's `let` reaches the outer
      # definition; the group's own method memoizes it.
      def let(name, &block)
        let_module.__send__(:define_method, name, &block)
        define_method(name) do
          @__memo.fetch(name) { @__memo[name] = super(&nil) }
        end
      end

      def let_module
        @let_module ||= Module.new.tap { |mod| include mod }
      end

      def let!(name, &block)
        let(name, &block)
        before { __send__(name) }
      end

      def subject(name = nil, &block)
        let(:subject, &block)
        let(name) { subject } if name
      end

      def before(scope = :each, &block)
        @hooks << block if %i[each example].include?(scope)
      end

      def after(*); end

      def around(scope = :each, &block)
        @around_hooks << block if %i[each example].include?(scope)
      end

      def all_around_hooks
        (group_parent ? group_parent.all_around_hooks : []) + @around_hooks
      end

      def shared_examples(name, *_metadata, &block)
        @shared[name] = block
      end
      alias_method :shared_examples_for, :shared_examples
      alias_method :shared_context, :shared_examples

      def find_shared(name)
        @shared.fetch(name) do
          parent = group_parent
          next parent.find_shared(name) if parent

          RSpec.find_shared(name)
        end
      end

      def it_behaves_like(name, *args, **kwargs, &block)
        shared = find_shared(name)
        subgroup("behaves like #{name}", {}) do
          class_exec(*args, **kwargs, &shared)
          class_exec(&block) if block
        end
      end
      alias_method :it_should_behave_like, :it_behaves_like

      def include_examples(name, *args, &block)
        # RuboCop's shared contexts that do not affect offenses.
        return if name == "with exclude limit tracking"
        if name == 'mock console output'
          # Capture only; never skip the examples or mock their investigation.
          @metadata[:oracle_console_output] = true
          return
        end

        class_exec(*args, &find_shared(name))
        class_exec(&block) if block
      end
      alias_method :include_context, :include_examples

      def described_class
        metadata[:described_class]
      end

      def full_description
        parts = []
        group = self
        while group
          parts.unshift(group.description.to_s) if group.description
          group = group.group_parent
        end
        parts.join(" ")
      end

      # Hooks in run order: outermost group first.
      def all_hooks
        (group_parent ? group_parent.all_hooks : []) + @hooks
      end

      def each_example(&block)
        @examples.each(&block)
        @children.each { |child| child.each_example(&block) }
      end
    end

    # CopHelper's defaults, Prism-engine flavoured.
    def ruby_version
      FixedPolicy::TARGET_RUBY
    end

    def rails_version
      false
    end

    def described_class
      self.class.described_class
    end

    # Older specs construct a named cop subject with a method-local Config,
    # rather than including the :config shared context. Read that actual cop's
    # configuration; never substitute defaults and lose nested fixed styles.
    def oracle_config
      return config if respond_to?(:config)
      return cop.config if respond_to?(:cop) && cop.is_a?(RuboCop::Cop::Base)

      raise Unsupported, 'example supplies neither config nor a real cop subject'
    end

    # RSpec's assertion and mocking API: accepted and ignored. Mocks and
    # stubbed constants can change what a cop sees, so they mark the
    # example as skipped.
    %i[is_expected eq eql equal be be_a be_an be_empty be_nil be_truthy be_falsey
       include match match_array contain_exactly start_with end_with raise_error change
       output satisfy all anything a_kind_of kind_of instance_of have_attributes
       receive have_received an_instance_of a_string_including].each do |name|
      define_method(name) { |*_args, **_kwargs, &_block| Blackhole.new }
    end

    %i[allow allow_any_instance_of expect_any_instance_of stub_const double
       instance_double spy].each do |name|
      define_method(name) do |*_args, **_kwargs, &_block|
        @__example.mocked = name
        Blackhole.new
      end
    end

    def expect_offense(source, file = nil, severity: nil, chomp: false, **replacements)
      _ = severity
      annotated = Annotations.format_offense(source, **replacements)
      plain, offenses = Annotations.parse(annotated)
      plain = plain.chomp if chomp
      @__example.record(self, plain, file, offenses)
    end

    # This owner's specs batch source assertions in RSpec's failure wrapper.
    # Yield every expectation; do not widen historical consumers' applicability.
    def aggregate_failures(*)
      unless cop_class.cop_name == 'Lint/UnmodifiedReduceAccumulator'
        raise Unsupported, '`aggregate_failures` is only emulated for UnmodifiedReduceAccumulator'
      end
      yield
    end

    def expect_no_offenses(source, file = nil)
      @__example.record(self, source, file, [])
    end

    # Source expectations can be nested in `expect { ... }.not_to raise_error`.
    # Do not silently discard that block. Value assertions themselves are helper
    # checks, not evidence that a snippet was linted.
    def expect(*, **, &block)
      @__example.helper_only = "assertion without source expectations"
      instance_exec(&block) if block
      Blackhole.new
    end

    def expect_correction(correction = nil, *, source: nil, loop: true, **)
      @__example.correct(correction, loop) if correction.is_a?(String)
      # SingleLineMethods' shared examples provide their input only here. Run
      # the pinned detector for expected positions, not the correction text;
      # independent Rust tests assert the canonical before/after bytes.
      if source && cop_class.cop_name == 'Style/SingleLineMethods'
        processed = RuboCop::ProcessedSource.new(source, ruby_version, nil, parser_engine: :parser_prism)
        found = RuboCop::Cop::Team.new([cop], config, {}).investigate(processed).offenses
        expected = found.map do |offense|
          ExpectedOffense.new(offense.line, offense.column + 1, offense.location.size, offense.message)
        end
        @__example.record(self, source, nil, expected)
      else
        @__example.helper_only = "correction-only example (detection oracle)"
      end
    end

    # `pending` inside an example: RSpec expects the rest to fail.
    def pending(*)
      @__example.pending = true
    end

    def skip(reason = "explicit skip", *)
      raise SkipExample, "skip: #{reason}"
    end

    # The 'with exclude limit tracking' shared context's helper.
    def read_exclude_limit(*)
      Blackhole.new
    end

    def expect_no_corrections
      @__example.correct(:none, true)
    end

    # The empty-call spec includes a batched performance fixture without caret
    # annotations. Obtain its expected positions from the pinned reference cop;
    # this does not execute the Ruby snippet or emulate RSpec timing assertions.
    def inspect_source(source)
      unless cop_class.cop_name == 'Style/MethodCallWithoutArgsParentheses'
        raise Unsupported, '`inspect_source` is only emulated for empty-call batch fixtures'
      end

      processed = RuboCop::ProcessedSource.new(source, ruby_version, nil, parser_engine: :parser_prism)
      found = RuboCop::Cop::Team.new([cop], config, {}).investigate(processed).offenses
      expected = found.map do |offense|
        ExpectedOffense.new(offense.line, offense.column + 1, offense.location.size, offense.message)
      end
      @__example.record(self, source, nil, expected)
      found
    end

    def parse_source(source)
      raise Unsupported, '`parse_source` is not emulated outside WordArray investigation fixtures' unless cop_class.cop_name == 'Style/WordArray'
      RuboCop::ProcessedSource.new(source, ruby_version, nil, parser_engine: :parser_prism)
    end

    def _investigate(target, processed)
      raise Unsupported, '`_investigate` only supports WordArray investigation fixtures' unless cop_class.cop_name == 'Style/WordArray'
      found = RuboCop::Cop::Team.new([target], config, {}).investigate(processed).offenses
      expected = found.map { |o| ExpectedOffense.new(o.line, o.column + 1, o.location.size, o.message) }
      @__example.record(self, processed.raw_source, nil, expected)
      found
    end

    %i[autocorrect_source autocorrect_source_file
       investigate].each do |name|
      define_method(name) do |*_args, **_kwargs|
        raise Unsupported, "`#{name}` is not emulated"
      end
    end

    def __oracle_start(example)
      @__example = example
      @__memo = {}
    end
  end

  # One `it` block.
  class Example
    attr_reader :group, :description, :metadata, :line
    attr_accessor :mocked, :pending, :helper_only

    def initialize(group, description, metadata, block, line)
      @group = group
      @description = description
      @metadata = group.metadata.merge(metadata)
      @block = block
      @line = line
    end

    def full_description
      [group.full_description, description].compact.join(" ")
    end

    def skip_reason
      return metadata[:skip].to_s if metadata[:skip]
      return "pending" if pending
      return "not run on Prism" if Oracle.prism_excluded?(metadata)
      # RuboCop's project index: analysis across files, which rustocop's
      # per-file cops do not do.
      return "project index" if metadata[:project_index]

      nil
    end

    def run
      stdout, stderr = $stdout, $stderr
      if metadata[:oracle_console_output]
        $stdout = StringIO.new
        $stderr = StringIO.new
      end
      @recorded = 0
      instance = group.new
      instance.__oracle_start(self)
      version = Oracle.tagged_ruby_version(metadata)
      instance.define_singleton_method(:ruby_version) { version } if version
      run = proc do
        group.all_hooks.each { |hook| instance.instance_exec(&hook) }
        instance.instance_exec(&@block) if @block
      end
      group.all_around_hooks.reverse_each do |hook|
        inner = run
        runner = Object.new
        runner.define_singleton_method(:run) { inner.call }
        # RSpec examples also convert to blocks, e.g. silence_warnings(&example).
        runner.define_singleton_method(:to_proc) { inner }
        run = proc { instance.instance_exec(runner, &hook) }
      end
      if Oracle.current_cop == 'Lint/Syntax'
        # The pinned main spec defaults to3.3. This product is fixed at3.4;
        # adapt both its actual parser and its expected advice consistently.
        instance.define_singleton_method(:ruby_version) { 3.4 }
        SyntaxLifecycle.run(metadata, &run)
      else
        run.call
      end
      if @recorded.zero? && !skip_reason && !helper_only
        raise Unsupported, 'no source expectation was evaluated'
      end
      unrecorded_skip = skip_reason || ("helper-only: #{helper_only}" if helper_only)
    rescue SkipExample => e
      unrecorded_skip = e.message
    rescue Exception => e # rubocop:disable Lint/RescueException
      raise if e.is_a?(Interrupt) || e.is_a?(SystemExit)
      # A skipped example need not work: RuboCop's run never evaluates it.
      unrecorded_skip = skip_reason
      return if unrecorded_skip

      config = begin
        Oracle.deep_dup(instance.oracle_config.to_h)
      rescue StandardError
        nil
      end
      Oracle.recorder.errors << ExampleError.new(
        cop: Oracle.current_cop, spec: Oracle.current_spec, spec_line: line,
        description: full_description, error: Oracle.describe_error(e), recorded: @recorded,
        config: config
      )
    ensure
      $stdout, $stderr = stdout, stderr
      if @recorded.zero? && unrecorded_skip
        location = Expectation.new(spec: Oracle.current_spec, spec_line: line,
                                   description: full_description)
        Oracle.recorder.skipped << [location, unrecorded_skip]
      end
    end

    def record(instance, source, file, offenses)
      @recorded += 1
      call_line = caller_locations.find { |loc| loc.path == Oracle.current_spec }&.lineno
      reason = skip_reason
      reason ||= "mocks: uses `#{mocked}`" if mocked
      config = reason ? nil : instance.oracle_config.to_h
      @last = Expectation.new(
        cop: Oracle.current_cop, spec: Oracle.current_spec, spec_line: call_line || line,
        description: full_description, source: source, file: file, offenses: offenses,
        config: config && Oracle.deep_dup(config), ruby_version: instance.ruby_version,
        skip: reason
      )
      Oracle.recorder.expectations << @last
    end

    # What the last expectation of this example corrects to.
    def correct(correction, loop)
      return unless @last

      @last.correction = correction
      @last.correction_loop = loop
    end
  end
end

# The `RSpec` entry points spec files call.
module RSpec
  # Layout/EndOfLine's `iso-8859-15` shared examples assert that a nested
  # `expect_no_offenses` fails (`raise_error(RSpec::Expectations::
  # ExpectationNotMetError)`). Their recorded source keeps its `crlf`/`lf`
  # configuration, so the detection oracle skips it like any custom-config
  # example; they are replayed at the fixed style in Rust unit tests.
  module Expectations
    class ExpectationNotMetError < StandardError; end
  end

  class << self
    def describe(*args, &block)
      description, metadata = Oracle.split_args(args)
      group = Class.new(Oracle::ExampleGroup)
      group.setup(description, metadata, nil)
      group.include(Oracle::EmptyFileLifecycle) if Oracle.current_cop == 'Lint/EmptyFile'
      group.include(Oracle::ScriptPermissionLifecycle) if Oracle.current_cop == 'Lint/ScriptPermission'
      group.include(Oracle::SyntaxLifecycle) if Oracle.current_cop == 'Lint/Syntax'
      if Oracle.current_cop == Oracle::RedundantDisableLifecycle::COP
        require 'rubocop/cop/legacy/corrector'
        require File.join(Oracle.locate_rubocop(nil), 'spec/support/fake_location')
        group.include(Oracle::RedundantDisableLifecycle)
      end
      group.class_exec(&block) if block
      Oracle.register_top_group(group)
      group
    end

    def shared_examples(name, *_metadata, &block)
      (@shared ||= {})[name] = block
    end
    alias_method :shared_examples_for, :shared_examples
    alias_method :shared_context, :shared_examples

    def find_shared(name)
      (@shared || {}).fetch(name) do
        raise Oracle::Unsupported, "unknown shared examples #{name.inspect}"
      end
    end

    def configure(*); end
  end
end
