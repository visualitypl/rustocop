# frozen_string_literal: true

# Standalone, offline tests; do not run the corpus differential check in CI.
require "fileutils"
require "json"
require "minitest/autorun"
require "open3"
require "rbconfig"
require_relative "../fixed_policy"
require "tmpdir"
require "yaml"

class DifferentialTest < Minitest::Test
  SCRIPT = File.expand_path("../differential.rb", __dir__)

  def setup
    @tmp = Dir.mktmpdir("differential-test")
    @corpus = File.join(@tmp, "corpus with spaces")
    FileUtils.mkdir_p(@corpus)
    @log = File.join(@tmp, "calls.jsonl")
    %w[rubocop rustocop].each do |name|
      path = File.join(@tmp, name)
      File.write(path, "#!#{RbConfig.ruby}\n" + STUB)
      FileUtils.chmod(0o755, path)
    end
  end

  def teardown
    FileUtils.remove_entry(@tmp)
  end

  def source(name, text)
    path = File.join(@corpus, name)
    FileUtils.mkdir_p(File.dirname(path))
    File.write(path, text)
    path
  end

  def run_check(*args, rubocop: File.join(@tmp, "rubocop"))
    Open3.capture3({ "DIFFERENTIAL_TEST_LOG" => @log }, RbConfig.ruby, SCRIPT,
                  "--binary", File.join(@tmp, "rustocop"), "--rubocop", rubocop,
                  *args, @corpus, chdir: @corpus)
  end

  def calls
    File.readlines(@log).map { |line| JSON.parse(line) }
  end

  def test_reports_positions_per_cop_and_ignores_messages
    miss = source("nested/miss: a.rb", "# miss\n")
    fp = source("fp.rb", "# fp\n")
    source("match.rb", "# both\n")
    out, err, status = run_check("--only", "Lint/Debugger,Style/StringLiterals")
    assert_equal 1, status.exitstatus, err
    %w[Lint/Debugger Style/StringLiterals].each do |cop|
      assert_includes out, "MISS #{cop} #{miss}:2:3"
      assert_includes out, "FP #{cop} #{fp}:2:3"
      assert_includes out, "#{cop}: 1 misses, 1 false positives"
    end
    refute_includes out, "match.rb:"
  end

  def test_isolates_configs_and_checks_only_rb_including_ignored_files
    source(".hidden.rb", "# both\n")
    source("vendor/yes.rb", "# both\n")
    source("no.rake", "# miss\n")
    source("UPPER.RB", "# miss\n")
    source("rustocop.toml", "not valid TOML")
    source(".rubocop.yml", "not valid YAML [")
    source(".gitignore", "vendor/\n")
    out, err, status = run_check("--only", "Lint/Debugger")
    assert_equal 0, status.exitstatus, err
    assert_includes out, "2 Ruby files"
    assert_equal 2, calls.size
    assert calls.all? { |call| call.fetch("paths").size == 2 }
    rc = calls.find { |call| call["tool"] == "rubocop" }
    config = rc.fetch("config")
    assert_equal "double_quotes", config.dig("Style/StringLiterals", "EnforcedStyle")
    assert_equal "double_quotes", config.dig("Style/StringLiteralsInInterpolation", "EnforcedStyle")
    assert_equal "ruby19", config.dig("Style/HashSyntax", "EnforcedStyle")
    assert_equal "either", config.dig("Style/HashSyntax", "EnforcedShorthandSyntax")
    assert_equal "only_raise", config.dig("Style/SignalException", "EnforcedStyle")
    assert_equal false, config.dig("Style/RedundantReturn", "AllowMultipleReturnValues")
    assert_equal false, config.dig("Style/Semicolon", "AllowAsExpressionSeparator")
    assert_equal "always", config.dig("Style/FrozenStringLiteralComment", "EnforcedStyle")
    assert_equal 120, config.dig("Layout/LineLength", "Max")
    assert_equal "parser_prism", config.dig("AllCops", "ParserEngine")
    assert_equal 3.4, config.dig("AllCops", "TargetRubyVersion")
    assert_includes rc.fetch("args"), "--cache"
    assert_equal "", calls.find { |call| call["tool"] == "rustocop" }.fetch("toml")
    refute File.exist?(rc.fetch("cwd")), "temporary corpus should be removed"
  end

  def test_directory_names_with_glob_metacharacters_are_literal
    renamed = "#{@corpus} [literal]"
    FileUtils.mv(@corpus, renamed)
    @corpus = renamed
    source("ok.rb", "# both\n")
    out, err, status = run_check("--only", "Lint/Debugger")
    assert_equal 0, status.exitstatus, err
    assert_includes out, "1 Ruby files"
  end

  def test_default_selection_checks_every_implemented_cop_including_opt_in
    source("ok.rb", "# both\n")
    out, err, status = run_check
    assert_equal 0, status.exitstatus, err
    expected = FixedPolicy.implemented_cops
    expected.each { |cop| assert_includes out, "#{cop}: 0 misses, 0 false positives" }
    assert_equal expected.size * 2, calls.size
    calls.select { |call| call["tool"] == "rubocop" }.each do |call|
      selected = call.fetch("args")[call.fetch("args").index("--only") + 1]
      assert_equal true, call.fetch("config").dig(selected, "Enabled"), selected
      unimplemented = "Style/ReverseFind"
      refute call.fetch("config").key?(unimplemented) unless expected.include?(unimplemented)
      unless selected == "Style/FrozenStringLiteralComment"
        assert_equal false, call.fetch("config").dig("Style/FrozenStringLiteralComment", "Enabled")
      end
    end
  end

  def test_successful_batches_keep_all_files
    101.times { |i| source("#{i}.rb", "# both\n") }
    out, err, status = run_check("--only", "Lint/Debugger")
    assert_equal 0, status.exitstatus, err
    assert_includes out, "101 Ruby files; 101 compared; 0 parser-rejected"
    assert_equal [100, 1, 100, 1], calls.map { |call| call.fetch("paths").size }
  end

  def test_parser_rejections_are_listed_once_and_excluded_for_every_cop
    rust = source("rust-rejected.rb", "# reject-rustocop\n# miss\n")
    ruby = source("ruby-rejected.rb", "# reject-rubocop\n# fp\n")
    source("ok.rb", "# both\n")
    out, err, status = run_check("--only", "Lint/Debugger,Style/StringLiterals")
    assert_equal 0, status.exitstatus, err
    assert_includes out, "3 Ruby files; 1 compared; 2 parser-rejected"
    assert_equal 1, out.scan("REJECT rustocop #{rust}:").size
    assert_equal 1, out.scan("REJECT rubocop #{ruby}:").size
    refute_match(/^(MISS|FP) /, out)
  end

  def test_tool_errors_are_not_treated_as_parser_rejections
    source("error.rb", "# crash\n")
    out, err, status = run_check("--only", "Lint/Debugger")
    assert_equal 2, status.exitstatus
    assert_includes err, "internal error:"
    refute_includes out, "0 misses"
    refute_includes out, "REJECT"
  end

  def test_read_errors_are_fatal_not_parser_rejections
    source("read.rb", "# read-error\n")
    out, err, status = run_check("--only", "Lint/Debugger")
    assert_equal 2, status.exitstatus
    assert_includes err, "could not read file"
    refute_includes out, "REJECT"
  end

  def test_incomplete_tool_output_cannot_look_like_parity
    %w[rubocop rustocop].each do |tool|
      source("incomplete.rb", "# incomplete-#{tool}\n")
      _out, err, status = run_check("--only", "Lint/Debugger")
      assert_equal 2, status.exitstatus
      assert_includes err, "#{tool} did not inspect the complete batch"
    end
  end

  def test_invalid_rubocop_json_fails_clearly
    source("bad.rb", "# invalid-json\n")
    _out, err, status = run_check("--only", "Lint/Debugger")
    assert_equal 2, status.exitstatus
    assert_includes err, "differential:"
  end

  def test_missing_executable_fails_without_building_or_downloading
    source("ok.rb", "# both\n")
    _out, err, status = run_check("--only", "Lint/Debugger", rubocop: File.join(@tmp, "missing"))
    assert_equal 2, status.exitstatus
    assert_includes err, "missing"
  end

  def test_unknown_cop_and_empty_corpus_fail_before_running_tools
    source("ok.rb", "# both\n")
    _out, err, status = run_check("--only", "Imaginary/Cop")
    assert_equal 2, status.exitstatus
    assert_includes err, "unknown or unimplemented cops"
    FileUtils.rm(File.join(@corpus, "ok.rb"))
    _out, err, status = run_check("--only", "Lint/Debugger")
    assert_equal 2, status.exitstatus
    assert_includes err, "no .rb files"
    refute File.exist?(@log)
  end

  def test_json_evidence_records_counts_positions_rejections_and_input_hashes
    require "digest"
    miss = source("nested/miss.rb", "# miss\n")
    fp = source("fp.rb", "# fp\n")
    rejected = source("bad.rb", "# reject-rubocop\n")
    matched = source("match.rb", "# both\n")
    report = File.join(@tmp, "report.json")
    _out, err, status = run_check("--only", "Style/StringLiterals", "--json", report)
    assert_equal 1, status.exitstatus, err
    evidence = JSON.parse(File.read(report))
    assert_equal 3.4, evidence.fetch("target_ruby")
    assert_equal ["Style/StringLiterals"], evidence.fetch("requested_cops")
    assert_equal 4, evidence.fetch("files").size
    file = evidence.fetch("files").find { |entry| entry.fetch("path") == miss }
    assert_equal Digest::SHA256.file(miss).hexdigest, file.fetch("sha256")
    result = evidence.fetch("results").fetch("Style/StringLiterals")
    assert_equal 2, result.fetch("reference_count")
    assert_equal 2, result.fetch("actual_count")
    assert_equal 1, result.fetch("matched_count")
    assert_equal [[miss, 2, 3]], result.fetch("misses")
    assert_equal [[fp, 2, 3]], result.fetch("false_positives")
    assert_equal [[matched, 2, 3]], result.fetch("matches")
    assert_equal({ "rubocop" => { rejected => "bad syntax" }, "rustocop" => {} }, evidence.fetch("rejections"))
    assert_equal "double_quotes", evidence.dig("reference_config", "Style/StringLiterals", "EnforcedStyle")
  end

  def test_json_report_cannot_overwrite_corpus_files_even_through_a_directory_symlink
    path = source("input.rb", "# both\n")
    alias_dir = File.join(@tmp, "alias")
    FileUtils.ln_s(@corpus, alias_dir)
    link = File.join(@tmp, "hardlink.json")
    FileUtils.ln(path, link)
    symlink = File.join(@tmp, "symlink.json")
    FileUtils.ln_s(path, symlink)
    [path, File.join(alias_dir, "input.rb"), link, symlink].each do |output|
      _out, err, status = run_check("--only", "Lint/Debugger", "--json", output)
      assert_equal 2, status.exitstatus
      assert_includes err, "JSON report must be outside the corpus"
      assert_equal "# both\n", File.read(path)
      refute File.exist?(@log)
    end
  end

  def test_json_evidence_is_explicit_about_zero_positive_coverage_and_version_inactivity
    source("clean.rb", "# clean\n")
    report = File.join(@tmp, "report.json")
    _out, err, status = run_check("--only", "Style/ReverseFind,Style/StringLiterals", "--json", report)
    assert_equal 0, status.exitstatus, err
    results = JSON.parse(File.read(report)).fetch("results")
    assert_equal 0, results.fetch("Style/StringLiterals").fetch("reference_count")
    assert_nil results.fetch("Style/StringLiterals").fetch("inapplicable")
    assert_match(/requires Ruby >= 4.0/, results.fetch("Style/ReverseFind").fetch("inapplicable"))
  end

  def test_repeated_positions_are_compared_as_a_multiset
    source("duplicate.rb", "# duplicate\n")
    out, err, status = run_check("--only", "Lint/Debugger")
    assert_equal 1, status.exitstatus, err
    assert_includes out, "Lint/Debugger: 1 misses, 0 false positives"
  end

  def test_batches_keep_the_same_file_set_and_reject_unexplained_errors_in_later_batches
    101.times { |i| source("#{i.to_s.rjust(3, '0')}.rb", "# both\n") }
    source("000.rb", "# reject-rustocop\n")
    source("100.rb", "# unexplained-error\n")
    _out, err, status = run_check("--only", "Lint/Debugger")
    assert_equal 2, status.exitstatus
    assert_includes err, "rustocop failed"
    assert_equal [100, 1, 100, 1], calls.map { |call| call.fetch("paths").size }
  end

  STUB = <<~'RUBY'
    require "json"
    require "yaml"
    tool = File.basename($PROGRAM_NAME)
    cop = ARGV.fetch(ARGV.index("--only") + 1)
    paths = ARGV.drop(ARGV.index("--") + 1)
    call = { tool: tool, args: ARGV, paths: paths, cwd: Dir.pwd }
    if tool == "rubocop"
      call[:config] = YAML.load_file(ARGV.fetch(ARGV.index("--config") + 1))
    else
      call[:toml] = File.read("rustocop.toml")
    end
    File.open(ENV.fetch("DIFFERENTIAL_TEST_LOG"), "a") { |f| f.puts(JSON.generate(call)) }
    rejected = false
    unexplained = false
    files = paths.map do |path|
      text = File.read(path)
      if text.include?("# invalid-json") && tool == "rubocop"
        puts "not JSON"
        exit 0
      end
      if text.include?("# crash") && tool == "rustocop"
        warn "#{path}: internal error: #{cop} panicked"
        exit 2
      end
      unexplained ||= text.include?("# unexplained-error") && tool == "rustocop"
      if text.include?("# incomplete-#{tool}")
        puts(tool == "rubocop" ? JSON.generate(files: []) : "0 files inspected, no offenses detected")
        exit 0
      end
      if text.include?("# read-error") && tool == "rustocop"
        puts "::error file=#{path},title=rustocop::#{path}: error: could not read file: denied"
        unexplained = true
      end
      if text.include?("# reject-#{tool}")
        rejected = true
        if tool == "rubocop"
          offenses = [{ "cop_name" => "Lint/Syntax", "message" => "bad syntax",
                        "location" => { "start_line" => 1, "start_column" => 1 } }]
        else
          puts "::error file=#{path},line=1,col=1,title=rustocop::#{path}: error: syntax error on line 1: bad syntax"
          offenses = []
        end
      else
        hit = text.include?("# both") || text.include?("# duplicate") ||
              text.include?(tool == "rubocop" ? "# miss" : "# fp")
        offenses = hit ? [{ "cop_name" => cop, "location" => { "start_line" => 2, "start_column" => 3 } }] : []
        offenses *= 2 if text.include?("# duplicate") && tool == "rubocop"
        if tool == "rustocop" && hit
          puts "::error file=#{path},line=2,col=3,title=#{cop}::a different message"
        end
      end
      { "path" => path, "offenses" => offenses }
    end
    if tool == "rubocop"
      puts JSON.generate(files: files)
    else
      puts "#{paths.size} files inspected, no offenses detected"
    end
    exit 2 if tool == "rustocop" && (rejected || unexplained)
    exit(files.any? { |f| !f["offenses"].empty? } ? 1 : 0)
  RUBY
end
