# frozen_string_literal: true

# Which RuboCop spec examples apply to rustocop.
#
# An example applies when rustocop can run under its configuration. A
# setting that the cop declares (`Cop::settings` in Rust) takes any value
# that is valid for it, and the value is written to the rustocop.toml the
# example runs under. So is a declared setting of another cop that the cop
# reads here too (READ_BY_OTHER_COPS). Every other setting has to match
# rustocop's fixed one: RuboCop's config/default.yml, except where SETTINGS
# says otherwise.
# Anything else (a style rustocop does not offer, a disabled option) is
# skipped and counted by reason.
require_relative "../fixed_policy"

module Oracle
  # A setting where several spec values mean the same as rustocop's.
  AnyOf = Struct.new(:choices)

  # rustocop's `value` for a setting that only matters for source matching
  # `pattern` (anything with `match?(source)`): other sources run whatever
  # the spec sets.
  Relevant = Struct.new(:value, :pattern)

  # The assigned Naming spec uses ForbiddenDelimiters=['END']. Admit each
  # unchanged example only when that list and the fixed exact pattern agree
  # on every real heredoc label (not comments, strings or a substring guess).
  module HeredocForbiddenSettings
    def self.equivalent?(wanted, fixed, source)
      return false unless source && wanted.is_a?(Array)
      processed = RuboCop::ProcessedSource.new(source, 3.4, '(oracle)', parser_engine: :parser_prism)
      return false unless processed.valid_syntax?
      lists = [wanted, fixed].map { |list| list.map { |pattern| Regexp.new(pattern) } }
      !processed.ast || processed.ast.each_node(:str, :dstr, :xstr).all? do |node|
        next true unless node.heredoc?
        label = node.source.match(RuboCop::Cop::Heredoc::OPENING_DELIMITER)&.captures&.[](1) || ''
        lists.map { |patterns| !/\w/.match?(label) || patterns.any? { |pattern| pattern.match?(label) } }.uniq.size == 1
      end
    end
  end

  module NonAsciiConstantToken
    def self.match?(source)
      processed = RuboCop::ProcessedSource.new(source, 3.4, '(oracle)', parser_engine: :parser_prism)
      processed.tokens.any? { |token| token.type == :tCONSTANT && !token.text.ascii_only? }
    end
  end

  # Project only source-irrelevant BlockParameterName settings. Compare the
  # first reported issue per parameter (RuboCop deduplicates its range), not
  # the complete config or a changed source/expectation. Real short/numeric/
  # forbidden-name examples remain counted fixed-policy exclusions.
  module BlockNameSettings
    KEYS = %w[MinNameLength AllowNamesEndingInNumbers AllowedNames ForbiddenNames].freeze
    def self.equivalent?(wanted, fixed, source)
      return false unless source

      ast = RuboCop::ProcessedSource.new(source, 3.4, '(oracle)', parser_engine: :parser_prism).ast
      !ast || ast.each_node(:block).all? do |block|
        block.arguments.all? do |arg|
          full = arg.children.first.to_s
          issue(full, wanted) == issue(full, fixed)
        end
      end
    end

    def self.issue(full, options)
      return if full == '_' || full.empty?

      name = full.sub(/\A_+/, '')
      return if Array(options['AllowedNames']).include?(name)
      return :forbidden if Array(options['ForbiddenNames']).include?(name)
      return :uppercase if /[[:upper:]]/.match?(name)
      min = options['MinNameLength']
      return [:length, min] if min && name.size < min
      return :number if !options['AllowNamesEndingInNumbers'] && /\d\z/.match?(name)
    end
  end

  # Only project list differences that cannot affect an actual definition or
  # bare static-symbol macro call. Sorbet gating is deliberately not projected.
  module PredicatePrefixSettings
    KEYS = %w[NamePrefix ForbiddenPrefixes AllowedMethods MethodDefinitionMacros].freeze
    def self.equivalent?(wanted, fixed, source)
      return false unless source && KEYS.all? { |key| wanted[key].is_a?(Array) }
      ast = RuboCop::ProcessedSource.new(source, 3.4, '(oracle)', parser_engine: :parser_prism).ast
      !ast || ast.each_node(:def, :defs, :send).all? do |node|
        if node.type?(:def, :defs)
          suggestion(node.method_name.to_s, wanted) == suggestion(node.method_name.to_s, fixed)
        elsif node.receiver.nil? && node.first_argument&.sym_type?
          name = node.first_argument.value.to_s
          results = [wanted, fixed].map do |options|
            options['MethodDefinitionMacros'].include?(node.method_name.to_s) ? suggestion(name, options) : []
          end
          results[0] == results[1]
        else
          true
        end
      end
    end
    def self.suggestion(name, options)
      return [] if name.end_with?('=') || options['AllowedMethods'].include?(name)
      options['NamePrefix'].filter_map do |prefix|
        next unless name.start_with?(prefix) && name.match?(/^#{Regexp.escape(prefix)}[^0-9]/)
        expected = options['ForbiddenPrefixes'].include?(prefix) ? name.delete_prefix(prefix) : name.dup
        expected += '?' unless name.end_with?('?')
        expected unless name == expected
      end
    end
  end

  # Source with a hash pair whose value is omitted (`{ a: }`) or could be
  # (`{ a: a }`): the only pairs Style/HashSyntax's EnforcedShorthandSyntax
  # styles disagree on (RuboCop's HashShorthandSyntax mixin). Parsed with
  # Prism, which RuboCop's runtime gems include; a snippet with syntax errors
  # is judged by what Prism recovers.
  module HashShorthandCandidate
    class << self
      def match?(source)
        require "prism"
        candidate?(Prism.parse(source).value)
      end

      private

      def candidate?(node)
        return true if node.is_a?(Prism::AssocNode) && shorthand_pair?(node)

        node.compact_child_nodes.any? { |child| candidate?(child) }
      end

      def shorthand_pair?(pair)
        key = pair.key
        value = pair.value
        return false unless key.is_a?(Prism::SymbolNode) && key.opening_loc.nil?
        return true if value.is_a?(Prism::ImplicitNode)

        identifier = value.is_a?(Prism::LocalVariableReadNode) ||
                     (value.is_a?(Prism::CallNode) && value.receiver.nil? &&
                      value.arguments.nil? && value.block.nil? && value.opening_loc.nil?)
        identifier && value.slice == key.unescaped
      end
    end
  end

  # YodaExpression's specs use only * and +. Run every source that does not
  # invoke an operator from the symmetric difference; retain the one genuine
  # alternate-operator example as a counted config skip. Strings/comments do not
  # count as sends. This projects settings, never snippets or expectations.
  module YodaOperatorSettings
    def self.equivalent?(wanted, fixed, source)
      return false unless wanted.is_a?(Array) && source

      changed = ((wanted - fixed) + (fixed - wanted)).map(&:to_sym)
      ast = RuboCop::ProcessedSource.new(source, 3.4, '(oracle)', parser_engine: :parser_prism).ast
      !ast || ast.each_node(:send, :csend).none? { |node| changed.include?(node.method_name) }
    end
  end

  # InvertibleUnlessCondition installs a partial map plus Rails pairs globally.
  # Project only when every send in an unless condition has the same mapping;
  # keep actual include?/exclude? examples as counted alternate-config skips.
  module UnlessInverseMapSettings
    def self.equivalent?(wanted, fixed, source)
      return false unless wanted.is_a?(Hash) && source

      wanted = wanted.transform_keys(&:to_sym).transform_values(&:to_sym)
      fixed = fixed.transform_keys(&:to_sym).transform_values(&:to_sym)
      ast = RuboCop::ProcessedSource.new(source, 3.4, '(oracle)', parser_engine: :parser_prism).ast
      !ast || ast.each_node(:if).all? do |node|
        !node.unless? || node.condition.each_node(:send).all? do |send|
          wanted[send.method_name] == fixed[send.method_name]
        end
      end
    end
  end

  # InverseMethods' spec installs ActiveSupport pairs globally. Only project
  # the map when every syntactic inversion candidate keeps the same mapping;
  # never translate the four actual present?/blank?/include?/exclude? examples.
  module InverseMapSettings
    def self.equivalent?(options, fixed, source)
      return false unless source

      ast = RuboCop::ProcessedSource.new(source, 3.4, '(oracle)', parser_engine: :parser_prism).ast
      return true unless ast

      matcher = RuboCop::Cop::Style::InverseMethods.new
      %w[InverseMethods InverseBlocks].all? do |key|
        wanted = options.fetch(key, {}).transform_keys(&:to_sym).transform_values(&:to_sym)
        pinned = fixed.fetch(key, {}).transform_keys(&:to_sym).transform_values(&:to_sym)
        wanted = wanted.merge(wanted.invert)
        pinned = pinned.merge(pinned.invert)
        ast.each_node.all? do |node|
          captures = key == 'InverseMethods' ? matcher.inverse_candidate?(node) : matcher.inverse_block?(node)
          method = captures && captures[key == 'InverseMethods' ? 2 : 1]
          !method || wanted[method] == pinned[method]
        end
      end
    end
  end

  # FetchBlock's SafeForConstants changes only constant block bodies. Its
  # pinned spec groups all numeric/string/negative examples under true; retain
  # those unchanged sources instead of silently skipping the whole group.
  module FetchBlockConstants
    def self.irrelevant?(source)
      return false unless source

      ast = RuboCop::ProcessedSource.new(source, 3.4, '(oracle)', parser_engine: :parser_prism).ast
      !ast || ast.each_node(:block).none? do |node|
        send = node.send_node
        node.body&.const_type? && node.arguments.empty? && send.call_type? &&
          send.method_name == :fetch && send.arguments.one?
      end
    end
  end

  # Next's specs mostly lower MinBodyLength to 1. Retain every source whose
  # candidate tails cross both thresholds (or neither); do not rewrite sources,
  # expected offenses, modifier style or consecutive-conditional policy.
  module NextMinimum
    def self.equivalent?(value, source, defaults)
      return false unless value.is_a?(Integer) && value.positive? && source

      ast = RuboCop::ProcessedSource.new(source, 3.4, '(oracle)', parser_engine: :parser_prism).ast
      return true unless ast

      matchers = [value, 3].map do |length|
        RuboCop::Cop::Style::Next.new(RuboCop::Config.new(
          'Style/Next' => defaults.merge('MinBodyLength' => length)
        ))
      end
      ast.each_node(:block, :numblock, :itblock, :while, :until, :for).all? do |node|
        next true if node.any_block_type? && !(node.send_node.call_type? && node.send_node.enumerator_method?)
        next true unless node.body

        matchers.map { |matcher| matcher.send(:ends_with_condition?, node.body) }.uniq.size == 1
      end
    end
  end

  # Threshold-only MultipleComparison examples still apply when the collected
  # first-variable group crosses both thresholds (or neither).
  module MultipleComparisonMinimum
    def self.equivalent?(limit, source)
      return false unless limit.is_a?(Integer) && limit.positive? && source

      ast = RuboCop::ProcessedSource.new(source, 3.4, '(oracle)', parser_engine: :parser_prism).ast
      return true unless ast

      matcher = RuboCop::Cop::Style::MultipleComparison.new(RuboCop::ConfigLoader.default_configuration)
      ast.each_node(:or).all? do |node|
        next true if node.parent&.or_type? || !matcher.send(:nested_comparison?, node)

        result = matcher.send(:find_offending_var, node)
        count = result ? result[1].size : 0
        (count >= limit) == (count >= 2)
      end
    end
  end

  # CaseLikeIf's threshold matters only for fully convertible elsif chains.
  # Use the pinned matcher rather than skipping all its MinBranchesCount=2
  # negative examples (or rewriting their sources/expected positions).
  module CaseLikeIfMinimum
    def self.equivalent?(value, source)
      return false unless value.is_a?(Integer) && value.positive? && source

      ast = RuboCop::ProcessedSource.new(source, 3.4, '(oracle)', parser_engine: :parser_prism).ast
      return true unless ast

      matcher = RuboCop::Cop::Style::CaseLikeIf.new(RuboCop::Config.new(
        'Style/CaseLikeIf' => { 'MinBranchesCount' => 1 }
      ))
      ast.each_node(:if).all? do |node|
        next true unless matcher.send(:should_check?, node)

        target = matcher.send(:find_target, node.condition)
        next true unless target

        branches = matcher.send(:branch_conditions, node)
        next true unless branches.all? { |condition|
          !matcher.send(:regexp_with_working_captures?, condition) &&
            matcher.send(:collect_conditions, condition, target, [])
        }

        (branches.size >= value) == (branches.size >= 3)
      end
    end
  end

  # HashLikeCase's threshold affects only otherwise-eligible mappings. Retain
  # the pinned negative syntax examples under MinBranchesCount=2 as well.
  module HashLikeCaseMinimum
    def self.equivalent?(value, source)
      return false unless value.is_a?(Integer) && value.positive? && source

      ast = RuboCop::ProcessedSource.new(source, 3.4, '(oracle)', parser_engine: :parser_prism).ast
      return true unless ast

      matcher = RuboCop::Cop::Style::HashLikeCase.new
      ast.each_node(:case).all? do |node|
        captures = matcher.hash_like_case?(node)
        next true unless captures

        conditions, bodies = captures
        next true unless conditions.map(&:type).uniq.one? && bodies.map(&:type).uniq.one?

        (node.when_branches.size >= value) == (node.when_branches.size >= 3)
      end
    end
  end

  # ArrayIntersect has pre-3.4 examples where `it` is a method call, not an
  # implicit local. Skip only source that actually changes binding at fixed 3.4;
  # explicit/shadowed `it` and unrelated older-version examples still execute.
  module IntersectionImplicitItVersion
    def self.match?(source)
      return false unless source

      ast = RuboCop::ProcessedSource.new(source, 3.4, '(oracle)', parser_engine: :parser_prism).ast
      return false unless ast

      matcher = RuboCop::Cop::Style::ArrayIntersect.new
      ast.each_node(:itblock).any? do |node|
        captures = matcher.any_none_block_intersection(node)
        next false unless captures

        receiver, method, argument, membership = captures
        receiver && !(method == :none? && node.send_node.safe_navigation?) &&
          (membership != :include? || argument.array_type?)
      end
    end
  end

  # WordArray/SymbolArray MinSize only gates nonempty bracket arrays of static
  # strings/symbols; delimiter preferences affect spelling, never detection.
  module WordArrayMinimum
    def self.equivalent?(value, source, element_type = :str)
      return false unless value.is_a?(Integer) && value >= 0 && source
      ast = RuboCop::ProcessedSource.new(source, 3.4, '(oracle)', parser_engine: :parser_prism).ast
      !ast || ast.each_node(:array).all? do |node|
        next true unless node.square_brackets? && !node.values.empty? && node.values.all? { |value| value.type == element_type }
        (node.values.size < value) == (node.values.size < 2)
      end
    end
  end

  # PercentLiteralDelimiters specs override the default for all literal types.
  # Keep examples whose actual types retain the fixed delimiter policy, including
  # nested literals in interpolation; never let comment/string text select types.
  module PercentDelimiterSettings
    def self.equivalent?(options, fixed, source)
      return false unless source

      wanted = options.fetch('PreferredDelimiters', {})
      pinned = fixed.fetch('PreferredDelimiters', {})
      return false unless (wanted.keys - %w[default % %q %Q %w %W %i %I %r %s %x]).empty?

      ast = RuboCop::ProcessedSource.new(source, 3.4, '(oracle)', parser_engine: :parser_prism).ast
      return true unless ast

      ast.each_node(:str, :dstr, :sym, :array, :regexp, :xstr).all? do |node|
        opening = node.loc.begin.source if node.loc.respond_to?(:begin) && node.loc.begin
        next true unless opening&.start_with?('%')

        type = opening[0...-1]
        wanted.fetch(type, wanted.fetch('default', '()')) == pinned.fetch(type, pinned.fetch('default', '()'))
      end
    end
  end

  # NumberedParametersLimit specs set Max=2 around unrelated syntax examples.
  # Compare offense eligibility, not messages or maximum implicit arity: _9
  # alone uses one distinct name. Retain every unaffected pinned example.
  module NumberedLimitSettings
    def self.equivalent?(options, fixed, source)
      return false unless source

      parsed = RuboCop::ProcessedSource.new(source, 3.4, '(oracle)', parser_engine: :parser_prism)
      return true unless parsed.ast

      parsed.ast.each_node(:numblock).all? do |node|
        count = node.each_descendant(:lvar).map(&:source).grep(/\A_[1-9]\z/).uniq.size
        (count > [options.fetch('Max', 1), 9].min) == (count > [fixed.fetch('Max', 1), 9].min)
      end
    end
  end

  # Parameter-map specs install alternate names globally. Execute every source
  # whose offense positions agree with the fixed map (messages are not compared).
  # Do not translate snippets/expectations or skip an unaffected syntax example.
  module BlockParameterSettings
    def self.equivalent?(options, fixed, source)
      return false unless source

      require 'prism'
      positions = lambda do |settings|
        methods = Array(settings['Methods']).reduce({}) { |map, entry| map.merge(entry) }
        found = []
        visit = lambda do |node|
          if node.is_a?(Prism::CallNode) && node.receiver && node.block.is_a?(Prism::BlockNode) &&
             node.block.opening_loc.start_line == node.block.closing_loc.start_line
            block = node.block
            parameters = block.parameters
            args = parameters.is_a?(Prism::BlockParametersNode) ? parameters.parameters : nil
            target = methods[node.name.to_s]
            if target && args && parameters.locals.empty? && args.optionals.empty? && (!args.rest || args.rest.is_a?(Prism::ImplicitRestNode)) &&
               args.posts.empty? && args.keywords.empty? && !args.keyword_rest && !args.block &&
               !args.requireds.empty? && args.requireds.all? { |arg| arg.is_a?(Prism::RequiredParameterNode) }
              names = args.requireds.map { |arg| arg.name.to_s.sub(/^_+/, '') }
              found << parameters.location.start_offset unless names == target.first(names.size)
            end
          end
          node.compact_child_nodes.each { |child| visit.call(child) }
        end
        visit.call(Prism.parse(source).value)
        found
      end
      positions.call(options) == positions.call(fixed)
    end
  end

  # InspectBlocks cannot suppress a multiline keyword block (already clean),
  # or a single-line block whose complete physical line exceeds the configured
  # length. Other suitability cases remain conservative configuration skips.
  module DoEndBlockSuitability
    def self.unchanged?(source, max)
      return false unless source && max

      require 'prism'
      lines = source.lines
      visit = lambda do |node|
        owner = node.respond_to?(:block) && node.block.is_a?(Prism::BlockNode) ? node : nil
        block = owner&.block
        if node.is_a?(Prism::LambdaNode)
          owner = block = node
        end
        if block && block.opening_loc.slice == 'do' && block.opening_loc.start_line == block.closing_loc.start_line &&
           (owner.location.start_line != block.closing_loc.start_line || lines.fetch(owner.location.start_line - 1).chomp.length <= max)
          return false
        end
        node.compact_child_nodes.all? { |child| visit.call(child) }
      end
      visit.call(Prism.parse(source).value)
    end
  end

  # EndAlignment's styles coincide when the keyword is first on its line.
  # IndentationWidth's specs install `variable` globally; do not discard all
  # the unrelated class/block/rescue fixtures for that. Include namespaces:
  # start_of_line differs from keyword for `puts 1; class C`, too.
  module InlineConditional
    def self.match?(source)
      require "prism"
      visit = lambda do |node|
        loc = case node
              when Prism::IfNode then node.if_keyword_loc
              when Prism::UnlessNode, Prism::WhileNode, Prism::UntilNode then node.keyword_loc
              when Prism::CaseNode, Prism::CaseMatchNode then node.case_keyword_loc
              when Prism::ForNode then node.for_keyword_loc
              when Prism::ClassNode, Prism::SingletonClassNode then node.class_keyword_loc
              when Prism::ModuleNode then node.module_keyword_loc
              end
        if loc
          prefix = source.byteslice(0, loc.start_offset).split("\n", -1).last.to_s
          return true unless prefix.strip.empty?
        end
        node.compact_child_nodes.any? { |child| visit.call(child) }
      end
      visit.call(Prism.parse(source).value)
    end
  end

  # The definition-spacing spec globally disallows adjacent one-liners. That
  # option cannot affect pairs containing a multiline definition (or pairs
  # already separated by exactly one blank). Keep those matching examples.
  module AdjacentOneLineDefs
    def self.match?(source)
      require "prism"
      lines = source.lines
      candidate = ->(n) { n.is_a?(Prism::DefNode) || n.is_a?(Prism::ClassNode) || n.is_a?(Prism::ModuleNode) }
      visit = lambda do |node|
        if node.is_a?(Prism::StatementsNode)
          relevant = node.body.each_cons(2).any? do |a, b|
            next false unless [a, b].all? { |n| candidate.call(n) && n.location.start_line == n.location.end_line }

            gap = lines[a.location.end_line...b.location.start_line - 1] || []
            blanks = gap.map { |line| line.match?(/\A[[:space:]]*\z/) }
            nonblank = blanks.index(false)
            blanks.count(true) != 1 && !(nonblank && blanks.rindex(true) && blanks.rindex(true) > nonblank)
          end
          return true if relevant
        end
        node.compact_child_nodes.any? { |child| visit.call(child) }
      end
      visit.call(Prism.parse(source).value)
    end
  end

  # Ruby <=3.3 suppresses anonymous forwarding of captured method parameters
  # because of ruby-lang bug #20090; fixed Ruby 3.4 deliberately reports it.
  # Do not discard the entire older-version group: ordinary forwarding still
  # applies. Parse bindings/depth so comments and block-local shadows do not skip.
  module ArgumentsForwardingBlockVersion
    NAMES = [%w[args arguments], %w[kwargs options opts], %w[blk block proc]].freeze

    def self.match?(source, block_only: false)
      return false unless source

      require 'prism'
      visit = lambda do |node, names, depth|
        if node.is_a?(Prism::DefNode)
          parameters = node.parameters
          names = if block_only
                    [parameters&.block&.name].compact
                  else
                    parameters ? [parameters.rest, parameters.keyword_rest, parameters.block].each_with_index.filter_map do |param, kind|
                      param.name if param.respond_to?(:name) && NAMES[kind].include?(param.name.to_s)
                    end : []
                  end
          depth = 0
        elsif node.is_a?(Prism::BlockNode) || node.is_a?(Prism::LambdaNode)
          depth += 1
        end
        value = case node
                when Prism::SplatNode, Prism::BlockArgumentNode then node.expression
                when Prism::AssocSplatNode then node.value
                end
        return true if (!block_only || node.is_a?(Prism::BlockArgumentNode)) &&
                       depth.positive? && value.is_a?(Prism::LocalVariableReadNode) &&
                       value.depth == depth && names.include?(value.name)

        node.compact_child_nodes.any? { |child| visit.call(child, names, depth) }
      end
      visit.call(Prism.parse(source).value, [], 0)
    end
  end

  # BlockDelimiters' specs customize allow/require lists for whole style
  # groups. Compare their effect on actual literal-block method names, not
  # comments, strings or ordinary sends, so unrelated examples still run.
  BlockMethodList = Struct.new(:fixed, :patterns) do
    def matches?(spec, source)
      return true if spec == fixed
      return false unless source

      require 'prism'
      allowed = lambda do |list, name|
        Array(list).any? { |entry| patterns ? Regexp.new(entry).match?(name) : entry == name }
      end
      visit = lambda do |node|
        name = if node.is_a?(Prism::LambdaNode)
                 'lambda'
               elsif node.is_a?(Prism::CallNode) && node.block.is_a?(Prism::BlockNode)
                 node.name.to_s
               elsif (node.is_a?(Prism::SuperNode) || node.is_a?(Prism::ForwardingSuperNode)) &&
                     node.block.is_a?(Prism::BlockNode)
                 'super'
               end
        return false if name && allowed.call(spec, name) != allowed.call(fixed, name)

        node.compact_child_nodes.all? { |child| visit.call(child) }
      end
      visit.call(Prism.parse(source).value)
    end
  end

  # These SymbolProc boolean overrides are irrelevant without arguments or
  # comments in a block's opening-line...closing-line interval. Use the same
  # Prism-backed AST/comment boundaries as the pinned spec, not source text.
  module SymbolProcArgumentBlocks
    def self.match?(source)
      ast = RuboCop::ProcessedSource.new(source, 3.4, nil, parser_engine: :parser_prism).ast
      !ast || ast.each_node.any? { |n| n.any_block_type? && n.send_node.arguments.any? }
    end
  end
  module SymbolProcCommentBlocks
    def self.match?(source)
      processed = RuboCop::ProcessedSource.new(source, 3.4, nil, parser_engine: :parser_prism)
      return true unless processed.ast

      processed.ast.each_node.any? do |node|
        node.any_block_type? && processed.comments.any? do |comment|
          (node.first_line...node.loc.end.line).cover?(comment.loc.line)
        end
      end
    end
  end

  # The comment specs customize whole keyword/character lists. Keep examples
  # whose actual comment tokens have identical annotation captures or ASCII
  # eligibility under both lists. Literal/data lookalikes cannot affect these.
  CommentList = Struct.new(:fixed, :kind) do
    def matches?(spec, source)
      return true if spec == fixed
      return false unless source

      require 'prism'
      Prism.parse(source).comments.all? do |comment|
        text = comment.location.slice
        if kind == :keywords
          signature = lambda do |list|
            words = Regexp.union(Array(list).sort_by { |word| -word.length })
            match = text.match(/^(# ?)(\b#{words}\b)(\s*:)?(\s+)?(\S+)?/i)
            match && match.captures
          end
          signature.call(spec) == signature.call(fixed)
        else
          permitted = ->(list) { (text.chars.reject(&:ascii_only?) - Array(list)).empty? }
          permitted.call(spec) == permitted.call(fixed)
        end
      end
    end
  end

  # Copyright's specs customize Notice almost everywhere. Compare the actual
  # leading-comment result with the reference cop, not regex text or literal
  # lookalikes. This is detection-only: replacement notices never affect it.
  CopyrightNotice = Struct.new(:fixed) do
    def matches?(spec, source)
      return true if spec == fixed
      return false unless source

      processed = RuboCop::ProcessedSource.new(source, FixedPolicy::TARGET_RUBY, nil,
                                              parser_engine: :parser_prism)
      present = lambda do |pattern|
        next true if pattern.nil? || pattern.empty?

        config = RuboCop::Config.new('Style/Copyright' => { 'Notice' => pattern })
        cop = RuboCop::Cop::Style::Copyright.new(config)
        cop.send(:notice_found?, processed)
      end
      present.call(spec) == present.call(fixed)
    end
  end

  # The magic-format spec defaults DirectiveCapitalization to nil, although
  # our fixed policy is lowercase. Keep examples where actual valid leading
  # directive ranges already have that case; do not ignore case-bearing values
  # that the reference's unanchored range search also treats as directives.
  MagicDirectiveCapitalization = Struct.new(:fixed) do
    def matches?(spec, source)
      return true if spec == fixed
      return false unless source && [nil, 'lowercase', 'uppercase'].include?(spec)

      processed = RuboCop::ProcessedSource.new(source, FixedPolicy::TARGET_RUBY, nil,
                                              parser_engine: :parser_prism)
      return true unless processed.ast

      first = processed.tokens.find { |token| !token.comment? }
      lines = first ? (0...first.line) : (0..)
      processed.each_comment_in_lines(lines).all? do |comment|
        next true unless RuboCop::MagicComment.parse(comment.text).valid?

        ranges = RuboCop::Cop::Style::MagicCommentFormat::CommentRange.new(comment).directives
        ranges.all? do |range|
          wrong = lambda do |style|
            style && range.source != (style == 'lowercase' ? range.source.downcase : range.source.upcase)
          end
          !!wrong.call(spec) == !!wrong.call(fixed)
        end
      end
    end
  end

  # GlobalVars specs permit one custom name globally. Only skip snippets
  # actually using a name whose eligibility changes (including alias operands).
  # Bundler's AllowedGems is fixed empty. A spec allowlist is equivalent
  # unless the source actually declares a listed gem by its str name.
  BundlerAllowedGems = Struct.new(:fixed) do
    def matches?(spec, source)
      return true if Array(spec) == Array(fixed)
      return false unless source

      processed = RuboCop::ProcessedSource.new(source, FixedPolicy::TARGET_RUBY, nil,
                                              parser_engine: :parser_prism)
      return true unless processed.ast

      extra = Array(spec) - Array(fixed)
      processed.ast.each_node(:send).none? do |node|
        node.method?(:gem) && node.receiver.nil? && node.first_argument&.str_type? &&
          extra.include?(node.first_argument.value)
      end
    end
  end

  # Gemspec AllowedGems lists are fixed empty. A spec allowlist is equivalent
  # unless the source sends one of the cop's declaration methods with a listed
  # gem as its str first argument (any receiver, conservatively).
  GemspecAllowedGems = Struct.new(:fixed, :methods) do
    def matches?(spec, source)
      return true if Array(spec) == Array(fixed)
      return false unless source

      processed = RuboCop::ProcessedSource.new(source, FixedPolicy::TARGET_RUBY, nil,
                                              parser_engine: :parser_prism)
      return true unless processed.ast

      extra = Array(spec) - Array(fixed)
      processed.ast.each_node(:send, :csend).none? do |node|
        methods.include?(node.method_name) && node.first_argument&.str_type? &&
          extra.include?(node.first_argument.value)
      end
    end
  end

  GlobalAllowedVariables = Struct.new(:fixed) do
    def matches?(spec, source)
      return true if spec == fixed
      return false unless source

      processed = RuboCop::ProcessedSource.new(source, FixedPolicy::TARGET_RUBY, nil,
                                              parser_engine: :parser_prism)
      return true unless processed.ast

      builtins = RuboCop::Cop::Style::GlobalVars::BUILT_IN_VARS
      processed.ast.each_node(:gvar, :gvasgn).all? do |node|
        builtins.include?(node.name) ||
          Array(spec).include?(node.name.to_s) == Array(fixed).include?(node.name.to_s)
      end
    end
  end

  # The equality spec clears AllowedMethods globally. Compare the *combined*
  # method/pattern exemption only for nearest defs of actual reference matcher
  # candidates. This retains all unaffected examples, including equivalent
  # method-vs-pattern allowances, without widening the fixed public policy.
  module ClassEqualityExemptions
    def self.equivalent?(options, fixed, source)
      return false unless source

      processed = RuboCop::ProcessedSource.new(source, FixedPolicy::TARGET_RUBY, nil,
                                              parser_engine: :parser_prism)
      return true unless processed.ast

      cop = RuboCop::Cop::Style::ClassEqualityComparison.new
      allowed = lambda do |settings, name|
        Array(settings['AllowedMethods']).include?(name.to_s) ||
          Array(settings['AllowedPatterns']).any? { |pattern| Regexp.new(pattern).match?(name.to_s) }
      end
      processed.ast.each_node(:send).all? do |node|
        captures = cop.send(:class_comparison_candidate?, node)
        next true unless captures && !captures.last.dstr_type?

        definition = node.each_ancestor(:any_def).first
        !definition || allowed.call(options, definition.method_name) == allowed.call(fixed, definition.method_name)
      end
    end
  end

  # Collection specs narrow the alias map globally, and Compact changes an
  # allowlist. Compare only actual pinned matcher candidates, not source text.
  module CollectionSettings
    def self.equivalent?(cop, options, fixed, source)
      return false unless source

      ast = RuboCop::ProcessedSource.new(source, FixedPolicy::TARGET_RUBY, nil,
                                        parser_engine: :parser_prism).ast
      return true unless ast

      if cop == 'Style/CollectionMethods'
        matcher = RuboCop::Cop::Style::CollectionMethods.new
        ast.each_node(:send, :csend).all? do |node|
          next true unless node.parent&.any_block_type? && node.parent.send_node.equal?(node) ||
                           matcher.send(:implicit_block?, node)

          options.fetch('PreferredMethods', {})[node.method_name.to_s] ==
            fixed.fetch('PreferredMethods', {})[node.method_name.to_s]
        end
      else
        klass = RuboCop::Cop::Style::CollectionCompact
        matchers = [options, fixed].map do |settings|
          klass.new(RuboCop::Config.new(cop => settings))
        end
        ast.each_node(:send, :csend).all? do |node|
          matcher = matchers.last
          candidate = matcher.send(:reject_method_with_block_pass?, node) ||
                      matcher.send(:grep_v_with_nil?, node) ||
                      (node.parent&.any_block_type? && node.parent.send_node.equal?(node) &&
                       matcher.send(:match_block_method?, node.parent))
          next true unless klass::RESTRICT_ON_SEND.include?(node.method_name) && candidate

          matchers.first.send(:allowed_receiver?, node.receiver) ==
            matchers.last.send(:allowed_receiver?, node.receiver)
        end
      end
    end
  end

  # Nested-call specs narrow the default list to `be` globally. Retain every
  # example where that cannot change the exemption of an eligible direct child.
  module NestedCallExemptions
    def self.equivalent?(options, fixed, source)
      return false unless source

      processed = RuboCop::ProcessedSource.new(source, FixedPolicy::TARGET_RUBY, nil,
                                              parser_engine: :parser_prism)
      return true unless processed.ast

      processed.ast.each_node(:send, :csend).all? do |parent|
        next true unless parent.parenthesized? && parent.arguments.one?

        parent.each_child_node(:call).all? do |nested|
          next true unless nested.arguments.one? && !nested.parenthesized? &&
                           !nested.setter_method? && !nested.operator_method?

          Array(options['AllowedMethods']).include?(nested.method_name.to_s) ==
            Array(fixed['AllowedMethods']).include?(nested.method_name.to_s)
        end
      end
    end
  end

  # PredicateMethod specs clear the default call/wayward lists globally.
  # Project only lists with identical decisions on actual def/call names;
  # alternate modes and bang policy remain genuine config exclusions.
  module PredicateMethodSettings
    KEYS = %w[AllowedMethods AllowedPatterns WaywardPredicates].freeze
    def self.equivalent?(options, fixed, source)
      return false unless source

      ast = RuboCop::ProcessedSource.new(source, FixedPolicy::TARGET_RUBY, nil,
                                         parser_engine: :parser_prism).ast
      return false unless ast

      allowed = lambda do |settings, name|
        Array(settings['AllowedMethods']).include?(name) ||
          Array(settings['AllowedPatterns']).any? { |pattern| Regexp.new(pattern).match?(name) }
      end
      ast.each_node(:def, :defs, :send, :csend).all? do |node|
        name = node.method_name.to_s
        if node.any_def_type?
          allowed.call(options, name) == allowed.call(fixed, name)
        else
          Array(options['WaywardPredicates']).include?(name) ==
            Array(fixed['WaywardPredicates']).include?(name)
        end
      end
    end
  end

  # Optional-parameter specs narrow their name lists globally. Compare only
  # actual matcher candidates, so unrelated examples still exercise fixed policy.
  module OptionalParameterSettings
    def self.equivalent?(cop, options, fixed, source)
      return false unless source

      processed = RuboCop::ProcessedSource.new(source, FixedPolicy::TARGET_RUBY, nil,
                                              parser_engine: :parser_prism)
      return true unless processed.ast

      if cop == 'Style/OptionalBooleanParameter'
        processed.ast.each_node(:def, :defs).all? do |node|
          next true unless node.arguments.any? { |arg| arg.optarg_type? && arg.default_value.boolean_type? }

          Array(options['AllowedMethods']).include?(node.method_name.to_s) ==
            Array(fixed['AllowedMethods']).include?(node.method_name.to_s)
        end
      else
        processed.ast.each_node(:args).all? do |node|
          arg = node.children.last
          next true unless arg&.optarg_type? && arg.default_value.hash_type? && arg.default_value.children.empty?
          next true if node.parent.each_node(:zsuper, :super).any?

          eligible = lambda do |settings|
            Array(settings['SuspiciousParamNames']).include?(arg.name.to_s) &&
              !Array(settings['Allowlist']).include?(node.parent.method_name.to_s)
          end
          eligible.call(options) == eligible.call(fixed)
        end
      end
    end
  end

  # Threshold differences matter only for entirely unnamed detections in a
  # direct formatter argument. Named sequences and non-format data do not read
  # this threshold. Use the pinned token grammar and source-map ownership.
  module FormatTokenThreshold
    def self.equivalent?(limit, source)
      return false unless limit.is_a?(Integer) && source
      ast = RuboCop::ProcessedSource.new(source, 3.4, :prism).ast
      return false unless ast
      ast.each_node(:str).all? do |node|
        parent = node.parent
        typical = parent&.send_type? &&
          ((%i[format sprintf printf].include?(parent.method_name) && parent.first_argument.equal?(node)) ||
           (parent.method?(:%) && parent.receiver.equal?(node) && parent.arguments.one?))
        next true unless typical
        map = node.loc
        contents = if map.is_a?(Parser::Source::Map::Heredoc)
                     map.heredoc_body
                   elsif map.begin
                     map.expression.adjust(begin_pos: +1, end_pos: -1)
                   else
                     map.expression
                   end
        tokens = RuboCop::Cop::Utils::FormatString.new(contents.source).format_sequences.reject(&:percent?)
        next true unless tokens.all? { |token| token.style == :unannotated }
        (tokens.size <= limit) == (tokens.size <= 1)
      end
    end
  end

  # The ParameterLists main spec globally uses Max=4. Retain every unchanged
  # example whose actual offense locations agree at fixed Max=5/keywords=true/
  # optional=3; count genuinely different threshold cases as exclusions. This
  # runs only the pinned detector on parsed source, never the source itself.
  module ParameterListSettings
    KEYS = %w[Max CountKeywordArgs MaxOptionalParameters].freeze
    def self.equivalent?(options, fixed, source)
      return false unless source

      locations = [options, fixed].map do |settings|
        config = RuboCop::Config.new('Metrics/ParameterLists' => settings)
        cop = RuboCop::Cop::Metrics::ParameterLists.new(config)
        processed = RuboCop::ProcessedSource.new(source, 3.4, '(oracle)', parser_engine: :parser_prism)
        processed.config = config
        processed.registry = RuboCop::Cop::Registry.global
        report = RuboCop::Cop::Team.new([cop], config, { raise_error: true }).investigate(processed)
        report.offenses.reject(&:disabled?).map { |o| [o.line, o.column, o.last_line, o.last_column] }.sort
      end
      locations.first == locations.last
    end
  end

  # A setting that does not change which offenses the cop reports.
  INERT = :inert

  # rustocop's settings where they are not RuboCop's defaults (see
  # README.md). Keys absent here are RuboCop's default.
  # Alternate boolean-cop options only matter when an actual candidate would
  # change. Keep shared negative examples and conditional-only `always` cases.
  class CaseEqualityAllowance
    def initialize(kind)
      @kind = kind
    end

    def match?(source)
      ast = RuboCop::ProcessedSource.new(source, 3.4, '(oracle)', parser_engine: :parser_prism).ast
      ast && ast.each_node(:send).any? do |node|
        next false unless node.method?(:===) && node.arguments.one?
        lhs = node.receiver
        if @kind == :constant
          lhs&.const_type? && lhs.module_name?
        else
          lhs&.send_type? && lhs.method?(:class) && lhs.receiver&.self_type? && !lhs.arguments?
        end
      end
    end
  end

  module AndOrUnconditional
    def self.match?(source)
      ast = RuboCop::ProcessedSource.new(source, 3.4, '(oracle)', parser_engine: :parser_prism).ast
      ast && ast.each_node(:and, :or).any? do |node|
        !node.logical_operator? && node.each_ancestor.none? do |ancestor|
          ancestor.type?(:if, :while, :until, :while_post, :until_post) &&
            ancestor.condition.source_range.begin_pos <= node.source_range.begin_pos &&
            ancestor.condition.source_range.end_pos >= node.source_range.end_pos
        end
      end
    end
  end

  SETTINGS = FixedPolicy::OPTIONS.reject { |_, options| options.empty? }.merge({
    "Style/EmptyClassDefinition" => { "EnforcedStyle" => AnyOf.new(%w[class_keyword class_definition]) },
    # rustocop's default, which a cop that is not in READ_BY_OTHER_COPS has
    # whatever the rustocop.toml says.
    "Style/StringLiterals" => { "EnforcedStyle" => "double_quotes" },
    "Style/StringLiteralsInInterpolation" => { "EnforcedStyle" => "double_quotes" },
    # RuboCop's spec for Layout/LineLength leaves the exemptions unset (off)
    # outside the examples about them.
    "Layout/LineLength" => {
      "AllowHeredoc" => Relevant.new(true, /<<[~-]?["'`]?[A-Za-z_]/),
      "AllowURI" => Relevant.new(true, /https?:/i),
      "URISchemes" => Relevant.new(%w[http https], /[A-Za-z][A-Za-z0-9+.-]*:[^\s:]/),
      "AllowQualifiedName" => Relevant.new(true, /::/),
      "AllowCopDirectives" => Relevant.new(true, /(?:rubocop|rustocop):/),
      # Only shapes autocorrection.
      "SplitStrings" => INERT
    },
    "Naming/AsciiIdentifiers" => {
      "AsciiConstants" => Relevant.new(true, NonAsciiConstantToken)
    },
    "Naming/MethodParameterName" => {
      "AllowNamesEndingInNumbers" => Relevant.new(true, /[[:alpha:]_]\d+\b/)
    },
    "Style/HashSyntax" => {
      "EnforcedShorthandSyntax" => Relevant.new("either", HashShorthandCandidate)
    },
    'Style/AndOr' => { 'EnforcedStyle' => Relevant.new('conditionals', AndOrUnconditional) },
    'Style/CaseEquality' => {
      'AllowOnConstant' => Relevant.new(false, CaseEqualityAllowance.new(:constant)),
      'AllowOnSelfClass' => Relevant.new(false, CaseEqualityAllowance.new(:self_class))
    },
    'Style/GlobalVars' => { 'AllowedVariables' => GlobalAllowedVariables.new([]) },
    'Bundler/GemComment' => { 'AllowedGems' => BundlerAllowedGems.new([]) },
    'Bundler/GemVersion' => { 'AllowedGems' => BundlerAllowedGems.new([]) },
    # TreatCommentsAsGroupSeparators only matters where a comment line can
    # sit between two declarations; inline comments never separate them.
    'Bundler/OrderedGems' => { 'TreatCommentsAsGroupSeparators' => Relevant.new(true, /^[ \t]*#/) },
    'Gemspec/OrderedDependencies' => { 'TreatCommentsAsGroupSeparators' => Relevant.new(true, /^[ \t]*#/) },
    'Gemspec/DependencyVersion' => {
      'AllowedGems' => GemspecAllowedGems.new([], %i[add_dependency add_runtime_dependency add_development_dependency])
    },
    'Gemspec/DevelopmentDependencies' => {
      'AllowedGems' => GemspecAllowedGems.new([], %i[add_development_dependency gem])
    },
    'Style/FormatStringToken' => {
      'EnforcedStyle' => AnyOf.new(['annotated', :annotated]),
      'Mode' => AnyOf.new(['aggressive', :aggressive])
    },
    'Style/SpecialGlobalVars' => { 'RequireEnglish' => INERT },
    'Style/MagicCommentFormat' => {
      'DirectiveCapitalization' => MagicDirectiveCapitalization.new('lowercase')
    },
    'Style/Copyright' => {
      'Notice' => CopyrightNotice.new('^Copyright (\\(c\\) )?2[0-9]{3} .+'),
      'AutocorrectNotice' => INERT
    },
    'Style/AsciiComments' => {
      'AllowedChars' => CommentList.new(['©'], :characters)
    },
    'Style/CommentAnnotation' => {
      'Keywords' => CommentList.new(%w[TODO FIXME OPTIMIZE HACK REVIEW NOTE], :keywords)
    },
    # SymbolProc's AllCops-only configs omit the cop allowlist. Retain those
    # examples unless the actual block dispatch's allowance would change.
    'Style/SymbolProc' => {
      'AllowedMethods' => BlockMethodList.new(['define_method'], false),
      'AllowedPatterns' => BlockMethodList.new([], true),
      'AllowMethodsWithArguments' => Relevant.new(false, SymbolProcArgumentBlocks),
      'AllowComments' => Relevant.new(false, SymbolProcCommentBlocks)
    },
    'Style/BlockDelimiters' => {
      'AllowedMethods' => BlockMethodList.new(%w[lambda proc it], false),
      'AllowedPatterns' => BlockMethodList.new([], true),
      'BracesRequiredMethods' => BlockMethodList.new([], false)
    },
    # StringMethods' pinned spec has a legacy top-level alias key. The cop
    # actually reads PreferredMethods (still compared strictly); this redundant
    # key neither replaces nor overrides that effective fixed map.
    "Style/StringMethods" => { "intern" => AnyOf.new([nil, "to_sym"]) },
    # Pinned macro specs spell this boolean as a string. The matcher checks
    # truthiness, so both spellings exercise the same fixed IgnoreMacros policy.
    "Style/MethodCallWithArgsParentheses" => {
      "IgnoreMacros" => AnyOf.new([true, "true"])
    },
    # Layout/LineLength counts a leading tab as the indentation width, 2;
    # unset, RuboCop falls back to Layout/IndentationWidth's Width, also 2.
    "Layout/IndentationStyle" => { "IndentationWidth" => AnyOf.new([nil, 2]) },
    # The operator spec's custom config leaves these styles nil; the cop
    # tests only == 'space', so nil implements the pinned no_space policy.
    "Layout/SpaceAroundOperators" => {
      "EnforcedStyleForExponentOperator" => AnyOf.new([nil, "no_space"]),
      "EnforcedStyleForRationalLiterals" => AnyOf.new([nil, "no_space"]),
      "SupportedStylesForExponentOperator" => INERT,
      "SupportedStylesForRationalLiterals" => INERT
    },
    # SpaceAroundOperators reads ForceEqualSignAlignment, not Enabled.
    "Layout/ExtraSpacing" => { "Enabled" => INERT },
    "Layout/EndAlignment" => {
      "EnforcedStyleAlignWith" => Relevant.new("keyword", InlineConditional)
    },
    "Layout/EmptyLineBetweenDefs" => {
      "AllowAdjacentOneLineDefs" => Relevant.new(true, AdjacentOneLineDefs)
    },
    # The HashAlignment spec builds its config without the defaults. Only the
    # validator reads AllowMultipleStyles/Supported*; the cop's `case` on an
    # unset last-argument style falls through to inspecting, as always_inspect.
    "Layout/HashAlignment" => {
      "AllowMultipleStyles" => INERT,
      "SupportedHashRocketStyles" => INERT,
      "SupportedColonStyles" => INERT,
      "SupportedLastArgumentHashStyles" => INERT,
      "EnforcedLastArgumentHashStyle" => AnyOf.new([nil, "always_inspect"])
    }
  }) { |_, fixed, custom| fixed.merge(custom) }.freeze

  # What a RuboCop cop reads for a key its configuration leaves out, where
  # that is not nil (`cop_config.fetch(key, fallback)`).
  FALLBACKS = {
    "Style/HashSyntax" => { "EnforcedShorthandSyntax" => "always" },
    "Lint/DuplicateMethods" => { "DelegatingMethods" => ["delegate"] }
  }.freeze

  # Keys that never change which offenses a cop reports.
  # The declared settings of a cop that other cops read here as they do in
  # RuboCop, by rustocop.toml key: `:all`, or the names of the cops that do.
  # A cop that reads the setting in RuboCop and is not named here runs only
  # the examples that leave the setting at rustocop's default.
  READ_BY_OTHER_COPS = {
    "Layout/LineLength" => { "max" => :all },
    "Style/StringLiterals" => {
      "enforced_style" => %w[Style/QuotedSymbols Style/EmptyHeredoc Style/EmptyLiteral Style/RedundantRegexpArgument]
    }
  }.freeze

  INERT_KEYS = %w[
    Description StyleGuide Reference References Details VersionAdded VersionChanged
    VersionRemoved Safe SafeAutoCorrect AutoCorrect Enabled Severity SupportedStyles
    SupportedShorthandSyntax Preview Include Exclude inherit_mode
  ].freeze

  # Ruby applicability is compared separately against the fixed Ruby 3.4
  # contract. Parser-engine tags can still select 3.3 for older spec contexts.
  INERT_ALL_COPS_KEYS = %w[TargetRubyVersion].freeze

  class << self
    # Why an example's configuration does not apply to rustocop (a list of
    # mismatching settings), or nil; and the rustocop.toml settings it needs.
    # `correction`: `source` is the corrected source of an example.
    def config_mismatch(cop, config, defaults, source, correction: false)
      target = config.fetch("AllCops", {}).fetch("TargetRubyVersion", FixedPolicy::TARGET_RUBY)
      version_reason = FixedPolicy.ruby_mismatch(cop, target)
      mismatches = version_reason ? [version_reason] : []
      if %w[Style/ArgumentsForwarding Naming/BlockForwarding].include?(cop) && target.to_f < 3.4 &&
         ArgumentsForwardingBlockVersion.match?(source, block_only: cop == 'Naming/BlockForwarding')
        mismatches << 'Ruby behavior: anonymous forwarding in blocks differs before fixed Ruby 3.4 (ruby-lang #20090)'
      end
      if cop == 'Style/ArrayIntersect' && target.to_f < 3.4 && IntersectionImplicitItVersion.match?(source)
        mismatches << 'Ruby behavior: implicit it binding differs before fixed Ruby 3.4'
      end
      toml = {}
      # RuboCop's examples for this cop name Metrics cops, which are on there
      # and opt-in here: switch the department on, as the examples assume.
      toml["Metrics"] = { "enabled" => true } if cop == "Lint/RedundantCopEnableDirective"
      run = FixedPolicy.run_ruby(target)
      toml["ruby_version"] = run.to_s unless run == FixedPolicy::TARGET_RUBY
      sections = { cop => {} }.merge(config)
      # Another cop's setting that the cop reads, left at RuboCop's default
      # where rustocop's default differs (the quotes of Style/StringLiterals).
      READ_BY_OTHER_COPS.each do |section, keys|
        next if config.key?(section)

        keys.each do |key, readers|
          next unless readers == :all || readers.include?(cop)

          name = defaults.fetch(section, {}).keys.find { |k| FixedPolicy.declared_setting(section, k)&.key == key }
          next unless name && SETTINGS.dig(section, name) && SETTINGS.dig(section, name) != defaults[section][name]

          sections[section] = { name => defaults[section][name] }
        end
      end
      sections.each do |section, values|
        next unless values.is_a?(Hash)

        if section == cop
          if cop == 'Metrics/ParameterLists'
            fixed = defaults.fetch(cop)
            if ParameterListSettings.equivalent?(fixed.merge(values), fixed, source)
              values = values.merge(fixed.slice(*ParameterListSettings::KEYS))
            end
          end
          # RescueModifier's pinned excluded-file example exercises a real
          # supported per-cop path setting, not an inert style option.
          # This cop consults only the exact source strings of located :const
          # callbacks. Extra spec-only table keys cannot change a source that
          # never reads them; do not project custom entries that are consulted.
          if cop == 'Lint/DeprecatedConstants' && source && values['DeprecatedConstants'].is_a?(Hash)
            fixed_table = defaults.fetch(cop).fetch('DeprecatedConstants')
            ast = RuboCop::ProcessedSource.new(source, FixedPolicy::TARGET_RUBY, nil,
                                              parser_engine: :parser_prism).ast
            if ast && ast.each_node(:const).all? { |node|
              !node.loc || values['DeprecatedConstants'][node.source.delete_prefix('::')] == fixed_table[node.source.delete_prefix('::')]
            }
              values = values.merge('DeprecatedConstants' => fixed_table)
            end
          end
          if cop == 'Naming/HeredocDelimiterNaming' &&
             HeredocForbiddenSettings.equivalent?(values['ForbiddenDelimiters'], defaults.fetch(cop).fetch('ForbiddenDelimiters'), source)
            values = values.merge('ForbiddenDelimiters' => defaults.fetch(cop).fetch('ForbiddenDelimiters'))
          end
          if cop == 'Style/RescueModifier' && values['Exclude'].is_a?(Array)
            toml[section] = (toml[section] || {}).merge('exclude' => values['Exclude'])
          end
          if ['Style/WordArray', 'Style/SymbolArray'].include?(cop) &&
             WordArrayMinimum.equivalent?(values['MinSize'], source, cop == 'Style/SymbolArray' ? :sym : :str)
            values = values.merge('MinSize' => 2)
          end
          if cop == 'Style/FormatStringToken' &&
             FormatTokenThreshold.equivalent?(values['MaxUnannotatedPlaceholdersAllowed'], source)
            values = values.merge('MaxUnannotatedPlaceholdersAllowed' => 1)
          end
          if cop == 'Style/ClassEqualityComparison'
            fixed = defaults.fetch(cop, {})
            if ClassEqualityExemptions.equivalent?(fixed.merge(values), fixed, source)
              values = values.merge(fixed.slice('AllowedMethods', 'AllowedPatterns'))
            end
          end
          if ['Style/OptionalBooleanParameter', 'Style/OptionHash'].include?(cop)
            fixed = defaults.fetch(cop, {})
            if OptionalParameterSettings.equivalent?(cop, fixed.merge(values), fixed, source)
              keys = cop == 'Style/OptionHash' ? %w[SuspiciousParamNames Allowlist] : ['AllowedMethods']
              values = values.merge(fixed.slice(*keys))
            end
          end
          if cop == 'Style/PercentLiteralDelimiters'
            fixed = defaults.fetch(cop, {})
            if PercentDelimiterSettings.equivalent?(fixed.merge(values), fixed, source)
              values = values.merge(fixed.slice('PreferredDelimiters'))
            end
          end
          # This flag is only read for an AND clause whose LHS is !x.nil?.
          # Retain the matching plain-object examples in the configured-true
          # group without changing our fixed false policy.
          if cop == 'Style/SafeNavigation' && values['ConvertCodeThatCanStartToReturnNil'] == true && source
            ast = RuboCop::ProcessedSource.new(source, FixedPolicy::TARGET_RUBY, nil,
                                              parser_engine: :parser_prism).ast
            changes_nil = ast && ast.each_node(:and).any? do |node|
              lhs = node.lhs
              lhs.send_type? && lhs.method?(:!) && lhs.receiver&.send_type? && lhs.receiver.method?(:nil?)
            end
            values = values.merge('ConvertCodeThatCanStartToReturnNil' => false) if ast && !changes_nil
          end
          if cop == 'Style/RedundantFetchBlock' && FetchBlockConstants.irrelevant?(source)
            values = values.merge('SafeForConstants' => false)
          end
          if cop == 'Style/Next' && values.key?('MinBodyLength') &&
             NextMinimum.equivalent?(values['MinBodyLength'], source, defaults.fetch(cop))
            values = values.merge('MinBodyLength' => 3)
          end
          if cop == 'Style/MultipleComparison' && MultipleComparisonMinimum.equivalent?(values['ComparisonsThreshold'], source)
            values = values.merge('ComparisonsThreshold' => 2)
          end
          if cop == 'Style/YodaExpression'
            fixed = defaults.fetch(cop, {})
            if YodaOperatorSettings.equivalent?(values['SupportedOperators'], fixed.fetch('SupportedOperators'), source)
              values = values.merge(fixed.slice('SupportedOperators'))
            end
          end
          if cop == 'Style/InvertibleUnlessCondition'
            fixed = defaults.fetch(cop)
            if UnlessInverseMapSettings.equivalent?(values['InverseMethods'], fixed.fetch('InverseMethods'), source)
              values = values.merge('InverseMethods' => fixed.fetch('InverseMethods'))
            end
          end
          if cop == 'Style/InverseMethods'
            fixed = defaults.fetch(cop, {})
            if InverseMapSettings.equivalent?(fixed.merge(values), fixed, source)
              values = values.merge(fixed.slice('InverseMethods', 'InverseBlocks'))
            end
          end
          if cop == 'Style/CaseLikeIf' && values.key?('MinBranchesCount') &&
             CaseLikeIfMinimum.equivalent?(values['MinBranchesCount'], source)
            values = values.merge('MinBranchesCount' => 3)
          end
          if cop == 'Style/HashLikeCase' && values.key?('MinBranchesCount') &&
             HashLikeCaseMinimum.equivalent?(values['MinBranchesCount'], source)
            values = values.merge('MinBranchesCount' => 3)
          end
          if cop == 'Style/NumberedParametersLimit'
            fixed = defaults.fetch(cop, {})
            if NumberedLimitSettings.equivalent?(fixed.merge(values), fixed, source)
              values = values.merge(fixed.slice('Max'))
            end
          end
          if cop == 'Style/SingleLineBlockParams'
            fixed = defaults.fetch(cop, {})
            if BlockParameterSettings.equivalent?(fixed.merge(values), fixed, source)
              values = values.merge(fixed.slice('Methods'))
            end
          end
          if ['Style/CollectionMethods', 'Style/CollectionCompact'].include?(cop)
            fixed = defaults.fetch(cop, {})
            if CollectionSettings.equivalent?(cop, fixed.merge(values), fixed, source)
              key = cop == 'Style/CollectionMethods' ? 'PreferredMethods' : 'AllowedReceivers'
              values = values.merge(fixed.slice(key))
            end
          end
          # AllowedAcronyms is read only by definition matching, never by the
          # snake_case filename check. The pin's hand-built default spec config
          # omits it. Do not project it for alternate definition-enabled cases.
          if cop == 'Naming/FileName' && values['ExpectMatchingDefinition'] == false
            values = values.merge('AllowedAcronyms' => defaults.fetch(cop).fetch('AllowedAcronyms'))
          end
          if cop == 'Naming/PredicateMethod'
            fixed = defaults.fetch(cop)
            if PredicateMethodSettings.equivalent?(fixed.merge(values), fixed, source)
              values = values.merge(fixed.slice(*PredicateMethodSettings::KEYS))
            end
            values = values.merge('Mode' => 'conservative') if values['Mode'] == :conservative
          end
          if cop == 'Naming/PredicatePrefix'
            fixed = defaults.fetch(cop)
            if PredicatePrefixSettings.equivalent?(values, fixed, source)
              values = values.merge(fixed.slice(*PredicatePrefixSettings::KEYS))
            end
          end
          if cop == 'Naming/BlockParameterName'
            fixed = defaults.fetch(cop, {})
            if BlockNameSettings::KEYS.all? { |key| values.key?(key) } &&
               BlockNameSettings.equivalent?(values, fixed, source)
              values = values.merge(fixed.slice(*BlockNameSettings::KEYS))
            end
          end
          if cop == 'Style/NestedParenthesizedCalls'
            fixed = defaults.fetch(cop, {})
            if NestedCallExemptions.equivalent?(fixed.merge(values), fixed, source)
              values = values.merge(fixed.slice('AllowedMethods'))
            end
          end
          compare_section(section, values, defaults.fetch(section, {}), source,
                          strict: true, mismatches: mismatches, toml: toml)
        elsif section == "AllCops"
          values = values.reject { |key, _| INERT_ALL_COPS_KEYS.include?(key) }
          if cop == 'Style/HashLookupMethod' && config.dig('Style/FetchEnvVar', 'Enabled') == 'pending'
            values = values.reject { |key, _| key == 'NewCops' && values[key] == 'enable' }
          end
          compare_section(section, values, defaults.fetch(section, {}), source,
                          strict: false, mismatches: mismatches, toml: toml)
        else
          # IfUnlessModifier reads LineLength's enable/max policy even when
          # --only selects just the Style cop. Disabled is representable in
          # our TOML, not an alternate fixed style to skip.
          if ['Style/IfUnlessModifier', 'Style/WhileUntilModifier', 'Style/MultilineMethodSignature', 'Style/SingleLineDoEndBlock', 'Style/RedundantBegin', 'Style/EmptyMethod', 'Layout/MultilineBlockLayout', 'Layout/RedundantLineBreak'].include?(cop) && section == 'Layout/LineLength' &&
             values['Enabled'] == false
            toml[section] = (toml[section] || {}).merge('enabled' => false)
            values = values.reject { |key, _| key == 'Enabled' }
          end
          # HashLookupMethod's ENV exemption follows configured enablement.
          # The reference's pending/NewCops states project to our boolean policy;
          # this is test translation, not a public NewCops option.
          if cop == 'Style/HashLookupMethod' && section == 'Style/FetchEnvVar'
            enabled = values['Enabled']
            enabled = config.dig('AllCops', 'NewCops') == 'enable' if enabled == 'pending'
            if [true, false].include?(enabled)
              toml[section] = (toml[section] || {}).merge('enabled' => enabled)
              values = values.reject { |key, _| key == 'Enabled' }
            end
          end
          # EmptyLiteral reads configured FrozenStringLiteralComment even
          # under --only. Both enable states are expressible in real TOML.
          if cop == 'Style/EmptyLiteral' && section == 'Style/FrozenStringLiteralComment' &&
             [true, false].include?(values['Enabled'])
            toml[section] = (toml[section] || {}).merge('enabled' => values['Enabled'])
            values = values.reject { |key, _| key == 'Enabled' }
          end
          # EmptyElse withholds removal when MissingElse is configured on,
          # independently of --only. Enabled is representable in strict TOML;
          # alternate if/case styles remain genuine fixed-policy skips.
          if cop == 'Style/EmptyElse' && section == 'Style/MissingElse' &&
             [true, false].include?(values['Enabled'])
            toml[section] = (toml[section] || {}).merge('enabled' => values['Enabled'])
            values = values.reject { |key, _| key == 'Enabled' }
          end
          # SingleArgumentDig delegates based on configured DigChain, even
          # under --only. Both boolean states are supported by strict TOML.
          if cop == 'Style/SingleArgumentDig' && section == 'Style/DigChain' &&
             [true, false].include?(values['Enabled'])
            toml[section] = (toml[section] || {}).merge('enabled' => values['Enabled'])
            values = values.reject { |key, _| key == 'Enabled' }
          end
          # NumericPredicate delegates bitwise ==/> checks based on configured
          # enablement, even if --only selects only NumericPredicate.
          if cop == 'Style/NumericPredicate' && section == 'Style/BitwisePredicate' &&
             [true, false].include?(values['Enabled'])
            toml[section] = (toml[section] || {}).merge('enabled' => values['Enabled'])
            values = values.reject { |key, _| key == 'Enabled' }
          end
          # SingleLineMethods reads EndlessMethod's configured enablement for
          # canonical corrections. Both enable states are representable; styles
          # other than the fixed allow_single_line remain genuine config skips.
          if cop == 'Style/SingleLineMethods' && section == 'Style/EndlessMethod' &&
             [true, false].include?(values['Enabled'])
            toml[section] = (toml[section] || {}).merge('enabled' => values['Enabled'])
            values = values.reject { |key, _| key == 'Enabled' }
          end
          # RedundantLineBreak defers chains on single-line blocks to the
          # configured SingleLineBlockChain enablement, even under --only.
          if cop == 'Layout/RedundantLineBreak' && section == 'Layout/SingleLineBlockChain' &&
             [true, false].include?(values['Enabled'])
            toml[section] = (toml[section] || {}).merge('enabled' => values['Enabled'])
            values = values.reject { |key, _| key == 'Enabled' }
          end
          if cop == 'Style/SingleLineDoEndBlock' && section == 'Layout/RedundantLineBreak' &&
             DoEndBlockSuitability.unchanged?(source, config.dig('Layout/LineLength', 'Max') || defaults.dig('Layout/LineLength', 'Max'))
            values = values.merge(defaults.fetch(section, {}).slice('Enabled', 'InspectBlocks'))
          end
          # The delimiters matter only for the percent literals a correction writes.
          if ['Style/WordArray', 'Style/SymbolArray'].include?(cop) && section == 'Style/PercentLiteralDelimiters' &&
             (!correction || PercentDelimiterSettings.equivalent?(defaults.fetch(section, {}).merge(values),
                                                                  defaults.fetch(section, {}), source))
            values = values.reject { |key, _| key == 'PreferredDelimiters' }
          end
          # CommandLiteral reads only the effective %x delimiter (falling
          # back to default), not unrelated word/regexp entries in this map.
          if cop == 'Style/CommandLiteral' && section == 'Style/PercentLiteralDelimiters' &&
             values['PreferredDelimiters'].is_a?(Hash)
            map = values['PreferredDelimiters']
            fixed = defaults.fetch(section, {}).fetch('PreferredDelimiters', {})
            if map.fetch('%x', map.fetch('default', '()')) == fixed.fetch('%x', fixed.fetch('default', '()'))
              values = values.merge('PreferredDelimiters' => fixed)
            end
          end
          # RegexpLiteral reads the explicit %r entry, not the default or
          # any of the other percent literal types in the partial spec config.
          if cop == 'Style/RegexpLiteral' && section == 'Style/PercentLiteralDelimiters' &&
             values['PreferredDelimiters'].is_a?(Hash)
            map = values['PreferredDelimiters']
            fixed = defaults.fetch(section, {}).fetch('PreferredDelimiters', {})
            if map['%r'] == fixed['%r']
              values = values.merge('PreferredDelimiters' => fixed)
            end
          end
          # MapIntoArray reads only the replacement for map, falling back to
          # map. Unrelated collection aliases cannot affect this cop's examples.
          if cop == 'Style/MapIntoArray' && section == 'Style/CollectionMethods' &&
             values['PreferredMethods'].is_a?(Hash) &&
             values['PreferredMethods'].fetch('map', 'map') == 'map'
            values = values.reject { |key, _| key == 'PreferredMethods' }
          end
          # A cop that takes its quotes from Style/StringLiterals writes
          # single quotes when that cop is disabled.
          if section == 'Style/StringLiterals' && [true, false].include?(values['Enabled']) &&
             READ_BY_OTHER_COPS.dig(section, 'enforced_style').include?(cop)
            toml[section] = (toml[section] || {}).merge('enabled' => values['Enabled'])
            values = values.reject { |key, _| key == 'Enabled' }
          end
          # Another cop's settings that the cop under test reads.
          compare_section(section, values, defaults.fetch(section, {}), source,
                          strict: false, mismatches: mismatches, toml: toml,
                          check_enabled: true, reader: cop)
        end
      end
      [mismatches.empty? ? nil : mismatches, toml]
    end

    private

    # Compare one config section with rustocop's. `strict`: every key
    # counts, and those the spec's config leaves out read as the cop would
    # read them (a custom `let(:config)` has no defaults); otherwise only the
    # keys given count. `reader`: the cop under test, when the section is
    # that of another cop.
    def compare_section(section, values, defaults, source, strict:, mismatches:, toml:,
                        check_enabled: false, reader: nil)
      ours = SETTINGS.fetch(section, {})
      fallbacks = FALLBACKS.fetch(section, {})
      keys = values.keys
      keys |= defaults.keys | ours.keys if strict
      keys.each do |key|
        next if INERT_KEYS.include?(key) && !(check_enabled && key == "Enabled")

        spec_value = values.fetch(key) { fallbacks[key] }
        declared = FixedPolicy.declared_setting(section, key)
        readers = READ_BY_OTHER_COPS.dig(section, declared&.key)
        rustocop_value = if declared && (reader.nil? || readers == :all || Array(readers).include?(reader))
                           declared
                         elsif key == "Enabled" && FixedPolicy::STYLE.key?(section)
                           FixedPolicy.default_enabled(section)
                         else
                           ours.fetch(key) { defaults[key] }
                         end
        next if setting_matches?(section, spec_value, rustocop_value, source, toml)

        mismatches << "#{section} #{key}=#{spec_value.inspect}"
      end
    end

    def setting_matches?(section, spec_value, rustocop_value, source, toml)
      case rustocop_value
      when INERT
        true
      when FixedPolicy::Declared
        value = rustocop_value.toml_value(spec_value)
        return false if value.nil?

        # No patterns is what a rustocop.toml without the key says.
        toml[section] = (toml[section] || {}).merge(rustocop_value.key => value) unless value == []
        true
      when AnyOf
        rustocop_value.choices.any? { |value| same_setting?(spec_value, value) }
      when BlockMethodList, CommentList, CopyrightNotice, MagicDirectiveCapitalization, GlobalAllowedVariables,
           BundlerAllowedGems, GemspecAllowedGems
        rustocop_value.matches?(spec_value, source)
      when Relevant
        same_setting?(spec_value, rustocop_value.value) ||
          (!source.nil? && !rustocop_value.pattern.match?(source))
      else
        same_setting?(spec_value, rustocop_value)
      end
    end

    # nil, false and empty collections all read as "off" in cop code.
    def same_setting?(left, right)
      (blank?(left) && blank?(right)) || left == right
    end

    def blank?(value)
      value.nil? || value == false || (value.respond_to?(:empty?) && value.empty?)
    end
  end
end
