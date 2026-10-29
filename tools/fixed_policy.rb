# frozen_string_literal: true

# Shared reference-tool policy: rustocop's fixed settings for RuboCop's
# reference cops. Reference facts come from tools/oracle/*_reference.json;
# the implemented set comes from the live Rust registry.
require "json"
require "open3"

module FixedPolicy
  ROOT = File.expand_path("..", __dir__)
  INVENTORY = JSON.parse(File.read(File.join(ROOT, "tools/oracle/style_reference.json")))
  STYLE = INVENTORY.fetch("cops").to_h { |record| [record.fetch("name"), record] }.freeze
  # Reference facts only, never an admission list: consult these Lint defaults
  # only for names the live registry implements.
  LINT = JSON.parse(File.read(File.join(ROOT, "tools/oracle/lint_reference.json")))
             .fetch("cops").to_h { |record| [record.fetch("name"), record] }.freeze
  LINT_DEFAULTS = LINT.transform_values { |record| record.fetch("upstream_defaults") }.freeze
  TARGET_RUBY = INVENTORY.fetch("fixed_target_ruby").to_f
  # These two unchanged upstream defaults are also explicit existing choices.
  EXISTING_KEYS = %w[AllowMultipleReturnValues AllowAsExpressionSeparator].freeze
  OPTIONS = STYLE.to_h do |name, record|
    defaults = record.fetch("upstream_defaults").select { |key, _| EXISTING_KEYS.include?(key) }
    [name, defaults.merge(record.fetch("fixed_option_overrides"))]
  end.freeze

  class << self
    # The cops the live Rust registry implements, one name per line from the
    # `list_cops` example.
    def implemented_cops
      @implemented_cops ||= begin
        output, status = Open3.capture2e("cargo", "run", "--quiet", "--offline", "--example", "list_cops", chdir: ROOT)
        raise "fixed_policy: cargo run --example list_cops failed:\n#{output}" unless status.success?

        output.lines.map(&:strip).reject(&:empty?).freeze
      end
    end

    def default_enabled(name)
      return false if name == "Naming/InclusiveLanguage"

      defaults = live_lint_defaults(name)
      return defaults.fetch("Enabled") != false if defaults

      record = STYLE[name]
      !record || (record.fetch("upstream_enabled") != false && name != "Style/FrozenStringLiteralComment")
    end

    def reference_config(implemented)
      implemented.filter_map do |name|
        if STYLE.key?(name)
          [name, OPTIONS.fetch(name).merge("Enabled" => default_enabled(name))]
        elsif live_lint_defaults(name)
          [name, { "Enabled" => default_enabled(name) }]
        end
      end.to_h
    end

    # A fixed-version-inactive cop remains selectable in rustocop, but must not
    # be compared to offenses expected under an unsupported, newer API contract.
    def ruby_mismatch(name, spec_version)
      record = STYLE[name]
      if record
        min = record.fetch("minimum_target_ruby")
        max = record.fetch("maximum_target_ruby")
      elsif %w[Lint/ItWithoutArgumentsInBlock Lint/UselessElseWithoutRescue Lint/NonDeterministicRequireOrder Lint/EmptyInPattern].include?(name) && live_lint_defaults(name)
        # Version-gated Lint cops only.
        record = LINT.fetch(name)
        min = record.fetch("minimum_ruby")
        max = record.fetch("maximum_ruby")
      else
        return
      end
      unless supports?(TARGET_RUBY, min, max)
        bounds = [min && "requires Ruby >= #{min}", max && "requires Ruby <= #{max}"].compact.join(", ")
        return "Ruby applicability: #{name} #{bounds}; rustocop is fixed at #{TARGET_RUBY}"
      end
      return if supports?(spec_version, min, max)

      "Ruby applicability: spec Ruby #{spec_version} is outside #{name}'s fixed-version applicability"
    end

    def path_allowed?(name, path)
      defaults = STYLE.fetch(name, {}).fetch("upstream_defaults", {})
      includes = defaults.fetch("Include", [])
      # Naming/FileName uses a config-root filename, not a recursive basename glob.
      excludes = name == "Naming/FileName" ? ["Rakefile.rb"] : defaults.fetch("Exclude", [])
      path = path.to_s.sub(%r{\A(?:\./)+}, "")
      match = ->(pattern) { File.fnmatch?(pattern, path, File::FNM_PATHNAME | File::FNM_DOTMATCH) }
      (includes.empty? || includes.any?(&match)) && excludes.none?(&match)
    end

    private

    def live_lint_defaults(name)
      return unless name.start_with?("Lint/") && implemented_cops.include?(name)

      LINT_DEFAULTS.fetch(name)
    end

    def supports?(version, min, max)
      parts = ->(value) { value.to_s.split(".").map(&:to_i) }
      (!min || (parts.call(version) <=> parts.call(min)) >= 0) &&
        (!max || (parts.call(version) <=> parts.call(max)) <= 0)
    end
  end
end
