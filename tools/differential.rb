#!/usr/bin/env ruby
# frozen_string_literal: true

# Optional, offline corpus comparison. Never invoked by cargo test or CI.
# Only standard-library dependencies; RuboCop is a separate local executable.
require "digest"
require "fileutils"
require "json"
require "open3"
require "optparse"
require "set"
require "tmpdir"
require "yaml"
require_relative "fixed_policy"

module Differential
  ROOT = File.expand_path("..", __dir__)
  BATCH_SIZE = 100 # Bound argv size even for a large corpus.
  Error = Class.new(StandardError)
  Result = Struct.new(:positions, :rejected, keyword_init: true)
  ESCAPES = { "%25" => "%", "%0D" => "\r", "%0A" => "\n", "%3A" => ":", "%2C" => "," }.freeze

  # Keep aligned with the defaults in docs/migrating-from-rubocop.md and tools/oracle/settings.rb.
  # Unlisted settings are RuboCop's defaults (including settings read from
  # other cops). Do not use DisabledByDefault: that changes those semantics.
  CONFIG = {
    "AllCops" => {
      "NewCops" => "enable", "TargetRubyVersion" => 3.4,
      "ParserEngine" => "parser_prism", "Include" => ["**/*.rb"], "Exclude" => []
    },
    "Layout/LineLength" => {
      "Max" => 120, "AllowHeredoc" => true, "AllowURI" => true,
      "URISchemes" => %w[http https], "AllowQualifiedName" => true,
      "AllowCopDirectives" => true, "AllowRBSInlineAnnotation" => false, "AllowedPatterns" => []
    },
    "Layout/IndentationStyle" => { "IndentationWidth" => 2 },
    "Naming/MethodParameterName" => {
      "MinNameLength" => 3, "AllowNamesEndingInNumbers" => true,
      "AllowedNames" => %w[as at by cc db id if in io ip of on os pp to], "ForbiddenNames" => []
    }
  }.merge(FixedPolicy.reference_config(FixedPolicy.implemented_cops)).freeze

  class << self
    def main(argv)
      options, directory, cops = options(argv)
      files = Dir.glob("**/*.rb", File::FNM_DOTMATCH, base: directory)
                 .map { |path| File.join(directory, path) }.select { |path| File.file?(path) }.sort
      raise Error, "no .rb files in #{directory}" if files.empty?
      if (json = options[:json])
        destination = File.join(File.realpath(File.dirname(json)), File.basename(json))
        root = File.realpath(directory)
        if destination.start_with?(File.join(root, "")) || File.symlink?(json) ||
           (File.exist?(json) && files.any? { |path| File.identical?(path, json) })
          raise Error, "JSON report must be outside the corpus and not alias any input"
        end
      end

      Dir.mktmpdir("rustocop-differential") do |tmp|
        # Work on a snapshot, outside both projects' config/exclusion trees.
        # Short synthetic names also make argv bounded and output unambiguous.
        FileUtils.mkdir_p(File.join(tmp, "files"))
        mapping = files.each_with_index.to_h do |path, index|
          staged = "files/#{index}.rb"
          FileUtils.cp(path, File.join(tmp, staged))
          [staged, path]
        end
        File.write(File.join(tmp, "rustocop.toml"), "")
        checksums = mapping.keys.to_h { |path| [path, Digest::SHA256.file(File.join(tmp, path)).hexdigest] }
        results = cops.to_h do |cop|
          # Force only the cop under test, including explicit upstream opt-ins.
          # Other cops retain fixed defaults because matchers can read them.
          reference = CONFIG.merge(cop => CONFIG.fetch(cop, {}).merge("Enabled" => true))
          File.write(File.join(tmp, ".rubocop.yml"), YAML.dump(reference))
          [cop, %w[rubocop rustocop].to_h do |tool|
            [tool, run_tool(tool, options.fetch(tool.to_sym), cop, mapping.keys, tmp)]
          end]
        end
        report(results, mapping, json: options[:json], checksums: checksums)
      end
    rescue OptionParser::ParseError, Error, SystemCallError, JSON::ParserError, KeyError => e
      warn "differential: #{e.message}"
      2
    end

    def options(argv)
      target = ENV.fetch("CARGO_TARGET_DIR", "target")
      options = { binary: File.expand_path(File.join(target, "debug", "rustocop"), ROOT),
                  rubocop: "rubocop", cops: [] }
      parser = OptionParser.new do |opts|
        opts.banner = "Usage: tools/differential.rb [options] DIRECTORY"
        opts.separator "Local only: no builds, downloads, or corpus checks in CI."
        opts.on("--binary PATH", "Existing rustocop binary (default: debug build)") do |v|
          options[:binary] = File.expand_path(v)
        end
        opts.on("--rubocop PATH", "Local RuboCop executable (default: rubocop on PATH)") do |v|
          options[:rubocop] = v.include?(File::SEPARATOR) ? File.expand_path(v) : v
        end
        opts.on("--only COP,COP", Array, "Cops to compare (default: all implemented cops)") do |v|
          options[:cops].concat(v)
        end
        opts.on("--json PATH", "Write reproducible counts, positions, settings and snapshot hashes") do |v|
          options[:json] = File.expand_path(v)
        end
        opts.on("-h", "--help", "Show usage") { puts opts; exit 0 }
      end
      args = parser.parse(argv)
      raise Error, parser.to_s unless args.size == 1 && File.directory?(args.first)

      implemented = FixedPolicy.implemented_cops
      cops = options[:cops].empty? ? implemented : options[:cops].uniq
      unknown = cops - implemented
      raise Error, "unknown or unimplemented cops: #{unknown.join(', ')}" unless unknown.empty?

      options[:rustocop] = options.fetch(:binary)
      [options, File.expand_path(args.first), cops]
    end

    def run_tool(tool, executable, cop, paths, tmp)
      result = Result.new(positions: [], rejected: {})
      paths.each_slice(BATCH_SIZE) do |batch|
        args = if tool == "rubocop"
                 ["--config", ".rubocop.yml", "--cache", "false", "--format", "json"]
               else
                 ["--format", "github"]
               end
        stdout, stderr, status = Open3.capture3(executable, *args, "--only", cop, "--", *batch,
                                               chdir: tmp)
        # Warnings are useful; internal/cop errors must never look like parity.
        if stderr.match?(/internal error:|An error occurred|errors? occurred|unrecognized cop/i)
          raise Error, "#{tool} failed for #{cop}: #{stderr}"
        end
        warn "#{tool} (#{cop}): #{stderr.strip}" unless stderr.strip.empty?

        batch_result = Result.new(positions: [], rejected: {})
        if tool == "rubocop"
          raise Error, "rubocop failed for #{cop}: #{stderr}\n#{stdout}" unless [0, 1].include?(status.exitstatus)

          parse_rubocop(stdout, batch, cop, batch_result)
        else
          raise Error, "rustocop failed for #{cop}: #{stderr}\n#{stdout}" unless [0, 1, 2].include?(status.exitstatus)

          parse_rustocop(stdout, batch, cop, batch_result)
          if status.exitstatus == 2 && batch_result.rejected.empty?
            raise Error, "rustocop failed for #{cop}: #{stderr}\n#{stdout}"
          end
        end
        result.positions.concat(batch_result.positions)
        result.rejected.merge!(batch_result.rejected)
      end
      result
    end

    def parse_rubocop(stdout, batch, cop, result)
      files = JSON.parse(stdout).fetch("files")
      raise Error, "rubocop did not inspect the complete batch for #{cop}" unless files.map { |f| f.fetch("path") }.sort == batch.sort

      files.each do |file|
        path = file.fetch("path")
        file.fetch("offenses").each do |offense|
          name = offense.fetch("cop_name")
          location = offense.fetch("location")
          if name == "Lint/Syntax"
            result.rejected[path] = offense.fetch("message", "syntax error")
          elsif name == cop
            result.positions << [path, location.fetch("start_line"), location.fetch("start_column")]
          else
            raise Error, "unexpected RuboCop offense: #{name.inspect}"
          end
        end
      end
    end

    def parse_rustocop(stdout, batch, cop, result)
      count = stdout[/^(\d+) files? inspected,/, 1]
      raise Error, "rustocop did not inspect the complete batch for #{cop}" unless count.to_i == batch.size

      stdout.each_line do |line|
        next unless line.start_with?("::error ")

        properties, message = line.delete_prefix("::error ").split("::", 2)
        fields = properties.split(",").to_h { |p| p.split("=", 2) }
        fields.transform_values! { |value| value.gsub(/%(?:25|0D|0A|3A|2C)/) { |v| ESCAPES.fetch(v) } }
        path = fields.fetch("file")
        raise Error, "unexpected rustocop path: #{path}" unless batch.include?(path)

        if fields.fetch("title") == "rustocop" && message.include?("error: syntax error")
          result.rejected[path] = message.strip
        elsif fields.fetch("title") == cop
          result.positions << [path, Integer(fields.fetch("line")), Integer(fields.fetch("col"))]
        else
          raise Error, "rustocop error for #{cop}: #{message.strip}"
        end
      end
    end

    # Multiset subtraction: repeated offenses at the same position matter.
    def subtract(left, right)
      remaining = right.tally
      left.reject do |position|
        count = remaining.fetch(position, 0)
        remaining[position] = count - 1 if count.positive?
        count.positive?
      end
    end

    def report(results, mapping, json: nil, checksums: {})
      rejected = results.values.flat_map { |tools| tools.values.flat_map { |r| r.rejected.keys } }.to_set
      puts "#{mapping.size} Ruby files; #{mapping.size - rejected.size} compared; #{rejected.size} parser-rejected"
      seen = Set.new
      results.each_value do |tools|
        tools.each do |tool, result|
          result.rejected.each do |path, reason|
            next unless seen.add?([tool, path])

            puts "REJECT #{tool} #{mapping.fetch(path)}: #{reason}"
          end
        end
      end
      different = false
      evidence = {}
      original_positions = ->(positions) { positions.sort.map { |path, line, col| [mapping.fetch(path), line, col] } }
      results.each do |cop, tools|
        expected = tools.fetch("rubocop").positions.reject { |p| rejected.include?(p.first) }
        actual = tools.fetch("rustocop").positions.reject { |p| rejected.include?(p.first) }
        misses = subtract(expected, actual)
        fps = subtract(actual, expected)
        { "MISS" => misses, "FP" => fps }.each do |label, positions|
          positions.sort.each { |path, line, col| puts "#{label} #{cop} #{mapping.fetch(path)}:#{line}:#{col}" }
        end
        matches = subtract(expected, misses)
        evidence[cop] = {
          reference_count: expected.size, actual_count: actual.size, matched_count: matches.size,
          matches: original_positions.call(matches), misses: original_positions.call(misses),
          false_positives: original_positions.call(fps),
          inapplicable: FixedPolicy.ruby_mismatch(cop, FixedPolicy::TARGET_RUBY)
        }
        puts "#{cop}: #{misses.size} misses, #{fps.size} false positives; " \
             "#{expected.size} reference, #{actual.size} actual, #{matches.size} matched"
        different ||= !misses.empty? || !fps.empty?
      end
      if json
        rejections = %w[rubocop rustocop].to_h do |tool|
          entries = results.values.each_with_object({}) do |tools, entries|
            tools.fetch(tool).rejected.each { |path, reason| entries[mapping.fetch(path)] = reason }
          end
          [tool, entries]
        end
        File.write(json, JSON.pretty_generate(
          schema: 1, target_ruby: FixedPolicy::TARGET_RUBY, requested_cops: results.keys,
          reference_config: CONFIG,
          files: mapping.map { |path, original| { path: original, sha256: checksums.fetch(path) } },
          rejections: rejections, results: evidence
        ) + "\n")
      end
      different ? 1 : 0
    end
  end
end

exit Differential.main(ARGV) if $PROGRAM_NAME == __FILE__
