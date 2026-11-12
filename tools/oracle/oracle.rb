#!/usr/bin/env ruby
# frozen_string_literal: true

# RuboCop spec oracle: run RuboCop's `expect_offense` / `expect_no_offenses`
# examples for rustocop's cops against the rustocop binary and report misses
# (offenses RuboCop's spec expects that rustocop does not report) and false
# positives (offenses rustocop reports that the spec does not expect), by
# line and column. Messages are not compared. See tools/oracle/README.md.
#
# Usage: tools/oracle/oracle.rb [options] [COP ...]

require "digest"
require "fileutils"
require "json"
require "open3"
require "optparse"
require "tmpdir"
require_relative "../fixed_policy"
require_relative "gem_filename_lifecycle"
require_relative "code_length_lifecycle"

module Oracle
  ROOT = File.expand_path("../..", __dir__)
  BASELINE = File.join(__dir__, "baseline.txt")
  STYLE_INVENTORY = File.join(ROOT, "tools/oracle/style_reference.json")
  # Exit status when the prerequisites (RuboCop's source, its runtime gems)
  # are missing: the cargo test treats it as a skip.
  EXIT_UNAVAILABLE = 3

  Options = Struct.new(:binary, :rubocop, :verbose, :quiet, :pinned, :write_baseline, :keep,
                       :cops, keyword_init: true)

  # A runnable example and what came of it.
  Case = Struct.new(:id, :expectation, :toml, :path, :actual, :errors, keyword_init: true) do
    def misses
      subtract(expected_positions, actual_positions)
    end

    def false_positives
      subtract(actual_positions, expected_positions)
    end

    def expected_positions
      expectation.offenses.map { |o| [o.line, o.col] }
    end

    def actual_positions
      actual.map { |o| [o[:line], o[:col]] }
    end

    # One line describing how rustocop deviates from the spec; nil on a match.
    def signature
      parts = []
      parts << "miss #{misses.map { |p| p.join(":") }.join(" ")}" unless misses.empty?
      unless false_positives.empty?
        parts << "fp #{false_positives.map { |p| p.join(":") }.join(" ")}"
      end
      parts << "error" unless errors.empty?
      parts.empty? ? nil : parts.join("; ")
    end

    private

    # Multiset difference.
    def subtract(left, right)
      remaining = right.dup
      left.reject do |item|
        index = remaining.index(item)
        remaining.delete_at(index) if index
      end
    end
  end

  # A known deviation from tools/oracle/baseline.txt.
  BaselineEntry = Struct.new(:id, :cop, :signature, :reason, :comment, keyword_init: true)

  # `path:line:col: E: message (Cop/Name)`, as `--format text` prints it. A
  # message quoting multi-line source (as RuboCop's do) spans several lines.
  OFFENSE_LINE = %r{
    \A(?<path>e\d+/.*?):(?<line>\d+):(?<col>\d+):\s[EC]:\s(?<message>.*)
    \s\((?<cop>[A-Za-z]+/[A-Za-z0-9]+)\)\z
  }xm.freeze
  # An offense record whose message continues on the next output line.
  OFFENSE_START = %r{\Ae\d+/.*?:\d+:\d+:\s[EC]:\s}.freeze
  # A syntax error on stdout, an internal error (a panic) on stderr.
  ERROR_LINE = %r{\A(?<path>e\d+/.*?)(?::\d+:\d+)?: (?<message>(?:internal )?error: .*)\z}.freeze

  # A location-free finding. RuboCop's expect_offense writes one as a `^{}`
  # annotation on the first line (line 1, column 1), so it is compared there.
  GLOBAL_LINE = %r{
    \A(?<path>e\d+/.*?):\s[EWC]:\s(?<message>.*)
    \s\((?<cop>[A-Za-z]+/[A-Za-z0-9]+)\)\z
  }x.freeze

  class << self
    def main(argv)
      options = parse_options(argv)
      implemented = implemented_cops
      cops = options.cops.empty? ? implemented : options.cops.uniq
      unknown = cops - implemented
      unless unknown.empty?
        warn "oracle: unknown or unimplemented cops: #{unknown.join(', ')}"
        return 2
      end
      src = locate_rubocop(options.rubocop)
      baseline = read_baseline
      require_pinned_revision(src, baseline[:revision]) if options.pinned
      load_rubocop(src)
      defaults = RuboCop::ConfigLoader.default_configuration.to_h
      binary = options.binary || build_binary

      note_rubocop_revision(src, baseline[:revision])
      results = Dir.mktmpdir("rustocop-oracle") do |tmp|
        cops.map { |cop| check_cop(cop, src, defaults, binary, tmp) }
      ensure
        FileUtils.cp_r(tmp, "#{tmp}-kept") if options.keep && tmp
        warn "kept snippets in #{tmp}-kept" if options.keep && tmp
      end

      if options.write_baseline
        if results.any? { |result| result[:fatal] || result[:no_spec] || !result[:errors].empty? }
          warn 'oracle: refusing to write a baseline with missing specs or unevaluated examples'
          return 1
        end
        write_baseline(baseline, results, src)
        puts "wrote #{relative(BASELINE)}"
        return 0
      end
      report(results, baseline, options)
    end

    def parse_options(argv)
      options = Options.new(verbose: false, quiet: false, pinned: false, write_baseline: false,
                            keep: false, cops: [])
      parser = OptionParser.new do |opts|
        opts.banner = "Usage: tools/oracle/oracle.rb [options] [COP ...]"
        opts.separator ""
        opts.separator "Runs every implemented cop when no COP is given."
        opts.on("--binary PATH", "rustocop binary (default: cargo build)") { |v| options.binary = v }
        opts.on("--rubocop PATH", "RuboCop source checkout (default: $RUBOCOP_SRC, " \
                                  "~/.cache/rubocop)") { |v| options.rubocop = v }
        opts.on("-v", "--verbose", "Also list known deviations, skips and evaluation errors") do
          options.verbose = true
        end
        opts.on("-q", "--quiet", "Only report cops with unexpected results") do
          options.quiet = true
        end
        opts.on("--pinned", "Skip (exit 3) unless the RuboCop checkout is at the " \
                            "revision the baseline records") { options.pinned = true }
        opts.on("--write-baseline", "Record the current deviations as known") do
          options.write_baseline = true
        end
        opts.on("--keep", "Keep the generated snippets") { options.keep = true }
      end
      options.cops = parser.parse(argv)
      options
    end

    def locate_rubocop(explicit)
      src = explicit || ENV.fetch("RUBOCOP_SRC", nil)
      src = File.expand_path("~/.cache/rubocop") if src.nil? || src.empty?
      return src if File.directory?(File.join(src, "spec", "rubocop", "cop"))

      warn "oracle: no RuboCop source at #{src}. Set RUBOCOP_SRC, or run\n  " \
           "git clone --depth 1 https://github.com/rubocop/rubocop ~/.cache/rubocop"
      exit EXIT_UNAVAILABLE
    end

    def load_rubocop(src)
      $LOAD_PATH.unshift(File.join(src, "lib"))
      require "rubocop"
      require "timeout"
      require File.join(src, "spec", "core_ext", "string")
      require File.join(src, "spec", "support", "misc_helper")
      require File.join(src, "spec", "support", "encoding_helper")
      require_relative "rspec_shim"
      require File.join(src, "spec", "support", "empty_lines_around_body_shared_examples")
      require File.join(src, "spec", "support", "alignment_examples")
      require File.join(src, "spec", "support", "condition_modifier_cop")
      require File.join(src, "spec", "support", "multiline_literal_brace_helper")
      require File.join(src, "spec", "support", "multiline_literal_brace_layout_examples")
      require File.join(src, "spec", "support",
                        "multiline_literal_brace_layout_method_argument_examples")
      require File.join(src, "spec", "support",
                        "multiline_literal_brace_layout_trailing_comma_examples")
      require_relative "settings"
    rescue LoadError => e
      warn "oracle: could not load RuboCop from #{src}/lib (#{e.message}).\n  " \
           "Install its runtime gems: `bundle install` in #{src}, or `gem install rubocop`."
      exit EXIT_UNAVAILABLE
    end

    # The cops the live registry implements.
    def implemented_cops
      FixedPolicy.implemented_cops
    end

    def build_binary
      output, status = Open3.capture2e("cargo", "build", "--quiet", chdir: ROOT)
      abort "oracle: cargo build failed:\n#{output}" unless status.success?
      target = ENV.fetch("CARGO_TARGET_DIR", nil)
      target = File.join(ROOT, "target") if target.nil? || target.empty?
      File.join(File.expand_path(target, ROOT), "debug", "rustocop")
    end

    # The checkout's commit, if it is a git checkout.
    def rubocop_revision(src)
      head, _, status = Open3.capture3("git", "-C", src, "rev-parse", "HEAD")
      status.success? ? head.strip : nil
    rescue SystemCallError
      nil
    end

    def require_pinned_revision(src, recorded)
      head = rubocop_revision(src)
      return if recorded && head == recorded

      warn "oracle: #{src} is at RuboCop #{head || "an unknown revision"}, the baseline " \
           "records #{recorded || "none"}. To run the oracle:\n  " \
           "git -C #{src} fetch --depth 1 origin #{recorded} && " \
           "git -C #{src} checkout #{recorded}"
      exit EXIT_UNAVAILABLE
    end

    def note_rubocop_revision(src, recorded)
      head = rubocop_revision(src)
      return if recorded.nil? || head.nil? || head == recorded

      warn "oracle: note: the baseline was recorded against RuboCop #{recorded[0, 12]}; " \
           "#{src} is at #{head[0, 12]}."
    end

    # The spec file for a cop: spec/rubocop/cop/<department>/<name>_spec.rb.
    def spec_path(src, cop)
      parts = cop.split("/").map do |part|
        part.gsub(/([A-Z]+)([A-Z][a-z])/, '\1_\2').gsub(/([a-z\d])([A-Z])/, '\1_\2').downcase
      end
      File.join(src, "spec", "rubocop", "cop", parts[0], "#{parts[1]}_spec.rb")
    end

    # Exact spec paths, not a prefix glob (which can select another cop).
    # Non-Style cops retain conventional discovery. The reference table is
    # path data only: it never selects cops.
    def source_and_specs(src, cop)
      records = JSON.parse(File.read(STYLE_INVENTORY)).fetch("cops")
      record = records.find { |entry| entry.fetch("name") == cop }
      if record
        { source: File.join(src, record.fetch("source")),
          specs: record.fetch("specs").map { |path| File.join(src, path) }.uniq }
      else
        spec = spec_path(src, cop)
        source = spec.sub("/spec/rubocop/cop/", "/lib/rubocop/cop/").sub(/_spec\.rb\z/, ".rb")
        { source: source, specs: [spec] }
      end
    end

    def spec_paths(src, cop)
      source_and_specs(src, cop).fetch(:specs)
    end

    # Everything the oracle learns about one cop.
    def check_cop(cop, src, defaults, binary, tmp)
      mapping = source_and_specs(src, cop)
      specs = mapping.fetch(:specs)
      result = { cop: cop, spec: specs.first, specs: specs, cases: [], skipped: [], errors: [],
                 fatal: nil, no_spec: false }
      missing = [mapping.fetch(:source), *specs].reject { |path| File.file?(path) }
      unless missing.empty? && !specs.empty?
        result[:no_spec] = true
        result[:fatal] = "missing RuboCop source/spec: #{missing.map { |p| relative_spec(p) }.join(', ')}"
        return result
      end

      recorder = Recorder.new
      ScriptPermissionLifecycle.reset if cop == ScriptPermissionLifecycle::COP
      specs.each do |spec|
        begin
          evaluate_spec(spec, cop, recorder)
        rescue StandardError, ScriptError => e
          result[:fatal] = "could not evaluate #{relative_spec(spec)}: #{e.class}: #{e.message}"
          return result
        end
      end
      if cop == GemFilenameLifecycle::COP
        return GemFilenameLifecycle.check(result, recorder, binary, tmp)
      end
      if cop == 'Lint/RedundantCopDisableDirective'
        return RedundantDisableLifecycle.check(result, recorder, tmp)
      end
      if cop == ScriptPermissionLifecycle::COP
        return ScriptPermissionLifecycle.check(result, recorder, binary, tmp)
      end
      result[:skipped].concat(recorder.skipped)
      if CodeLengthLifecycle::COPS.include?(cop)
        result[:skipped].concat(CodeLengthLifecycle.account(cop, recorder, defaults))
      end
      recorder.errors.each do |error|
        # A source-less helper under a demonstrably different fixed config is
        # inapplicable. With no source, never assume a Relevant option is inert.
        mismatch = if error.recorded.zero? && error.config
                     config_mismatch(cop, error.config, defaults, nil).first
                   end
        if mismatch
          result[:skipped] << [error, "config: #{mismatch.join(', ')}"]
        else
          result[:errors] << error
        end
      end

      seen = {}
      recorder.expectations.each do |expectation|
        # These landed gemspec specs omit a filename. Replay their actual
        # matcher on a supplied eligible path, rather than silently skipping
        # every example through the mandatory production Include gate.
        if %w[Gemspec/AttributeAssignment Gemspec/DuplicatedAssignment Gemspec/OrderedDependencies Gemspec/AddRuntimeDependency Gemspec/DependencyVersion Gemspec/DeprecatedAttributeAssignment Gemspec/RequiredRubyVersion Gemspec/RubyVersionGlobalsUsage].include?(cop)
          expectation.file ||= 'example.gemspec'
        end
        # Likewise these landed Gemfile specs (an inline source is always
        # relevant upstream): replay them on a supplied eligible Gemfile.
        if %w[Bundler/GemVersion Bundler/InsecureProtocolSource Bundler/OrderedGems].include?(cop)
          expectation.file ||= 'Gemfile'
        end
        reason = expectation.skip
        unless reason || FixedPolicy.path_allowed?(cop, expectation.file || "example.rb")
          reason = "path: excluded by pinned #{cop} Include/Exclude policy"
        end
        toml = {}
        unless reason
          mismatch, toml = config_mismatch(cop, expectation.config, defaults, expectation.source)
          reason = "config: #{mismatch.join(", ")}" if mismatch
        end
        if reason
          result[:skipped] << [expectation, reason]
          next
        end

        id = case_id(cop, expectation, toml)
        next if seen[id]

        seen[id] = true
        result[:cases] << Case.new(id: id, expectation: expectation, toml: toml,
                                   actual: [], errors: [])
      end
      run_cases(cop, result[:cases], binary, File.join(tmp, cop.tr("/", "_")))
      result
    end

    def case_id(cop, expectation, toml)
      positions = expectation.offenses.map { |o| "#{o.line}:#{o.col}" }.join(",")
      key = [cop, expectation.file.to_s, toml_text(toml), expectation.source, positions]
      Digest::SHA256.hexdigest(key.join("\0"))[0, 12]
    end

    def toml_text(toml)
      toml.sort.map do |section, values|
        lines = values.sort.map { |key, value| "#{key} = #{value.inspect}" }
        "[cops.#{section.inspect}]\n#{lines.join("\n")}\n"
      end.join
    end

    # Lint every case's snippet with the binary, one run per rustocop.toml.
    def run_cases(cop, cases, binary, dir)
      cases.group_by { |c| toml_text(c.toml) }.each_with_index do |(toml, group), index|
        root = File.join(dir, "config#{index}")
        FileUtils.mkdir_p(File.join(root, ".git"))
        File.write(File.join(root, "rustocop.toml"), toml) unless toml.empty?
        group.each_with_index do |kase, number|
          kase.path = File.join("e#{number}", snippet_name(kase.expectation.file))
          FileUtils.mkdir_p(File.join(root, File.dirname(kase.path)))
          File.binwrite(File.join(root, kase.path), kase.expectation.source)
        end
        lint(cop, group, binary, root)
      end
    end

    # A relative path for the file name a spec passes to expect_offense.
    def snippet_name(file)
      name = file.to_s.split("/").reject { |part| part.empty? || part == "." || part == ".." }
      name.empty? ? "example.rb" : name.join("/")
    end

    def lint(cop, cases, binary, root)
      return SyntaxLifecycle.lint(cases, binary, root) if cop == 'Lint/Syntax'

      by_path = cases.to_h { |kase| [kase.path, kase] }
      stdout, stderr, status = Open3.capture3(binary, "--only", cop, "--format", "text",
                                              *by_path.keys, chdir: root)
      unless [0, 1, 2].include?(status.exitstatus)
        abort "oracle: #{binary} failed (#{status}) for #{cop}:\n#{stderr}"
      end

      pending = nil
      (stdout.lines + stderr.lines).each do |line|
        line = line.chomp
        if pending && !(OFFENSE_START.match?(line) || ERROR_LINE.match?(line) || GLOBAL_LINE.match?(line))
          line = "#{pending}\n#{line}"
        end
        pending = nil
        if (match = OFFENSE_LINE.match(line))
          kase = by_path[match[:path]]
          next unless kase && match[:cop] == cop

          kase.actual << { line: match[:line].to_i, col: match[:col].to_i,
                           message: match[:message] }
        elsif (match = ERROR_LINE.match(line))
          by_path[match[:path]]&.errors&.push(match[:message])
        elsif (match = GLOBAL_LINE.match(line))
          kase = by_path[match[:path]]
          next unless kase && match[:cop] == cop

          kase.actual << { line: 1, col: 1, message: match[:message] }
        elsif OFFENSE_START.match?(line)
          # Unterminated: join the next line. A record that never ends with
          # its cop name (before the next record starts) is dropped and stays
          # a miss.
          pending = line
        end
      end
      # Exit status 2 without an error for some snippet: the run itself failed
      # (a bad rustocop.toml, say), and its silence would read as matches.
      return unless status.exitstatus == 2 && cases.all? { |kase| kase.errors.empty? }

      abort "oracle: #{binary} failed for #{cop} in #{root}:\n#{stdout}#{stderr}"
    end

    def read_baseline(path = BASELINE)
      entries = {}
      revision = nil
      comment = nil
      return { entries: entries, revision: revision } unless File.exist?(path)

      File.readlines(path, chomp: true).each do |line|
        if (match = /\A# RuboCop revision: (\h+)/.match(line))
          revision = match[1]
        elsif line.start_with?("#") || line.strip.empty?
          comment = line.start_with?("# ") ? line.delete_prefix("# ") : nil
        elsif (match = %r{\A(?<id>\h{12}) (?<cop>\S+) (?<sig>.+?) -- (?<reason>.*)\z}.match(line))
          entries[match[:id]] = BaselineEntry.new(id: match[:id], cop: match[:cop],
                                                  signature: match[:sig], reason: match[:reason],
                                                  comment: comment)
          comment = nil
        else
          abort "oracle: malformed baseline line: #{line}"
        end
      end
      { entries: entries, revision: revision }
    end

    def write_baseline(baseline, results, src, path = BASELINE)
      selected = results.map { |r| r[:cop] }
      kept = baseline[:entries].values.reject { |entry| selected.include?(entry.cop) }
      fresh = results.flat_map do |result|
        result[:cases].filter_map do |kase|
          signature = kase.signature or next
          old = baseline[:entries][kase.id]
          BaselineEntry.new(id: kase.id, cop: result[:cop], signature: signature,
                            reason: old ? old.reason : "TODO: triage",
                            comment: case_comment(result, kase))
        end
      end
      entries = (kept + fresh).sort_by { |e| [e.cop, e.comment.to_s[/:(\d+)/, 1].to_i, e.id] }

      head = rubocop_revision(src)
      revision = head || baseline[:revision]
      File.open(path, "w") do |out|
        out.puts "# Known deviations of rustocop from RuboCop's spec examples: the oracle"
        out.puts "# fails on any deviation not listed here, and on entries that no longer"
        out.puts "# deviate. Refresh with `tools/oracle/oracle.rb --write-baseline [COP ...]`,"
        out.puts "# then give every new entry a reason. Format, under a comment naming the"
        out.puts "# example: <id> <cop> <deviation> -- <reason>."
        out.puts "# RuboCop revision: #{revision}" if revision
        entries.group_by(&:cop).each do |cop, group|
          out.puts
          out.puts "## #{cop}"
          group.each do |entry|
            out.puts "# #{entry.comment}" if entry.comment
            out.puts "#{entry.id} #{entry.cop} #{entry.signature} -- #{entry.reason}"
          end
        end
      end
    end

    def case_comment(result, kase)
      expectation = kase.expectation
      "#{relative_spec(expectation.spec || result[:spec])}:#{expectation.spec_line} #{expectation.description}"
        .gsub(/\s+/, " ")
    end

    def relative_spec(spec)
      spec[%r{spec/rubocop/cop/.*}] || spec
    end

    def relative(path)
      path.delete_prefix("#{ROOT}/")
    end

    # Compare one cop's cases with the baseline: counts, problems (new,
    # changed and stale deviations) and the known deviations.
    def classify(result, entries)
      counts = Hash.new(0)
      problems = []
      known = []
      seen = {}
      result[:cases].each do |kase|
        counts[:misses] += kase.misses.size
        counts[:false_positives] += kase.false_positives.size
        counts[:parse_errors] += 1 unless kase.errors.empty?
        signature = kase.signature
        entry = entries[kase.id]
        seen[kase.id] = true if entry
        if signature.nil?
          counts[:match] += 1
          problems << [:stale, kase, entry] if entry
        elsif entry.nil?
          problems << [:new, kase, nil]
        elsif entry.signature != signature
          problems << [:changed, kase, entry]
        else
          known << [kase, entry]
        end
      end
      entries.each_value do |entry|
        problems << [:stale, nil, entry] if entry.cop == result[:cop] && !seen[entry.id]
      end
      counts[:known] = known.size
      { counts: counts, problems: problems, known: known }
    end

    # Print the report; returns the exit status.
    def report(results, baseline, options)
      totals = Hash.new(0)
      results.each do |result|
        verdict = classify(result, baseline[:entries])
        broken = result[:fatal] || result[:no_spec] || !result[:errors].empty?
        quiet = options.quiet && verdict[:problems].empty? && !broken
        print_cop_summary(result, verdict[:counts]) unless quiet
        totals[:fatal] += 1 if result[:fatal] || result[:no_spec]
        verdict[:problems].each { |kind, kase, entry| print_problem(result, kind, kase, entry) }
        print_errors(result) unless options.verbose
        print_details(result, verdict[:known]) if options.verbose
        totals[:cops] += 1
        totals[:run] += result[:cases].size
        totals[:skipped] += result[:skipped].size
        totals[:errors] += result[:errors].size
        verdict[:counts].each { |key, n| totals[key] += n }
        verdict[:problems].each { |kind, _, _| totals[kind] += 1 }
      end

      puts
      puts "#{totals[:cops]} cops: #{summary_counts(totals[:run], totals)}; " \
           "#{totals[:skipped]} skipped, #{totals[:errors]} not evaluated"
      failures = totals[:new] + totals[:changed] + totals[:stale]
      return 0 if failures.zero? && totals[:fatal].zero? && totals[:errors].zero?

      if totals[:fatal].positive?
        puts "FAILED: #{totals[:fatal]} specs are missing or could not be evaluated."
      end
      if totals[:errors].positive?
        puts "FAILED: #{totals[:errors]} examples could not be evaluated. Teach " \
             "tools/oracle/rspec_shim.rb what they use."
      end
      if failures.positive?
        puts "FAILED: #{totals[:new]} new, #{totals[:changed]} changed and #{totals[:stale]} " \
             "stale deviations against #{relative(BASELINE)}. Fix rustocop, or run " \
             "tools/oracle/oracle.rb --write-baseline [COP ...] and give each new entry a reason."
      end
      1
    end

    def summary_counts(run, counts)
      deviate = run - counts[:match]
      "#{run} examples run, #{counts[:match]} match, #{deviate} deviate " \
        "(#{counts[:misses]} misses, #{counts[:false_positives]} false positives, " \
        "#{counts[:parse_errors]} unparsable; #{counts[:known]} known)"
    end

    def print_cop_summary(result, counts)
      if result[:fatal] || result[:no_spec]
        puts "#{result[:cop]}: #{result[:fatal] || "no RuboCop spec"}"
        return
      end
      skips = result[:skipped].map { |_, reason| reason[/\A[^:]+/] }.tally
      skip_text = skips.sort.map { |reason, n| "#{reason} #{n}" }.join(", ")
      puts "#{result[:cop]}: #{summary_counts(result[:cases].size, counts)}; " \
           "#{result[:skipped].size} skipped#{skip_text.empty? ? "" : " (#{skip_text})"}, " \
           "#{result[:errors].size} not evaluated"
    end

    def print_problem(result, kind, kase, entry)
      label = { new: "NEW DEVIATION", changed: "CHANGED DEVIATION",
                stale: "STALE BASELINE ENTRY" }.fetch(kind)
      if kase.nil?
        puts "  #{label} #{entry.id}: #{entry.comment} (#{entry.signature}); " \
             "no longer deviates or no longer runs"
        return
      end
      print_case(result, kase, label)
      puts "    baseline: #{entry.signature} -- #{entry.reason}" if entry
    end

    def print_case(result, kase, label)
      expectation = kase.expectation
      puts "  #{label} #{kase.id} #{relative_spec(expectation.spec || result[:spec])}:#{expectation.spec_line}"
      puts "    #{expectation.description.gsub(/\s+/, " ")}"
      kase.misses.each do |line, col|
        offense = expectation.offenses.find { |o| o.line == line && o.col == col }
        puts "    miss #{line}:#{col} #{offense&.message}"
      end
      kase.false_positives.each do |line, col|
        offense = kase.actual.find { |o| o[:line] == line && o[:col] == col }
        puts "    false positive #{line}:#{col} #{offense && offense[:message]}"
      end
      kase.errors.each { |error| puts "    #{error}" }
      unless kase.toml.empty?
        puts "    rustocop.toml: #{toml_text(kase.toml).gsub("\n", " ").strip}"
      end
      if expectation.ruby_version != FixedPolicy::TARGET_RUBY
        puts "    the spec targets Ruby #{expectation.ruby_version}"
      end
      expectation.source.each_line.with_index(1).first(15).each do |text, number|
        puts format("    %3d| %s", number, text.chomp)
      end
    end

    def print_details(result, known_lines)
      known_lines.each do |kase, entry|
        print_case(result, kase, "known deviation")
        puts "    reason: #{entry.reason}"
      end
      result[:skipped].each do |expectation, reason|
        puts "  skipped #{relative_spec(expectation.spec || result[:spec])}:#{expectation.spec_line} #{reason}"
      end
      print_errors(result)
    end

    def print_errors(result)
      result[:errors].each do |error|
        puts "  not evaluated #{relative_spec(error.spec || result[:spec])}:#{error.spec_line} " \
             "#{error.description.gsub(/\s+/, " ")}: #{error.error}" \
             "#{error.recorded.positive? ? " (after #{error.recorded} expectations)" : ""}"
      end
    end
  end
end

exit Oracle.main(ARGV) if $PROGRAM_NAME == __FILE__
