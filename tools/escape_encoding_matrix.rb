#!/usr/bin/env ruby
# frozen_string_literal: true

# Development-only finite source-encoding audit. Never executes fixtures.
# ruby tools/escape_encoding_matrix.rb /tmp/escape-matrix.json
# Uses the fixed production Prism encoding registry/aliases and pinned reference;
# every codec/parser exclusion is recorded per spelling/control, not suppressed.
require 'json'
require 'open3'
require 'tmpdir'
require 'fileutils'
require 'rbconfig'
require 'digest'
require 'prism'
require 'rubocop'

root = File.expand_path('..', __dir__)
abort 'expected pinned RuboCop 1.92.0' unless RuboCop::Version::STRING == '1.92.0'
ref = ENV.fetch('RUBOCOP_SRC', '/opt/rubocop')
pin, status = Open3.capture2('git', '-C', ref, 'rev-parse', 'HEAD')
abort 'wrong reference pin' unless status.success? && pin.strip == '97f30c4fe1ae5c11dd5844bbe00a822c25e6f255'
abort 'expected Unicode 15.0' unless RbConfig::CONFIG['UNICODE_VERSION'] == '15.0.0'
binary = File.join(ENV.fetch('CARGO_TARGET_DIR', File.join(root, 'target')), 'debug/rustocop')
c_source = File.read(File.join(root, 'vendor/ruby-prism-sys/vendor/prism-1.9.0/src/encoding.c'))
profiles = c_source.scan(/\[PM_ENCODING_(\w+)\] = \{\s*\.name = "([^"]+)".*?\.multibyte = (true|false)/m)
lookup = c_source.split('pm_encoding_find(const uint8_t *start, const uint8_t *end)', 2).last
aliases = lookup.scan(/ENCODING[12]\((.*?)\);/).flat_map { |m| m[0].scan(/"([^"]+)"/).flatten }
# Prism's UTF-8 prefix rule accepts an unbounded suffix spelling family. These
# representatives exercise the explicit HFS exception and generic prefix path.
names = (profiles.map { |p| p[1] } + aliases + Encoding.name_list + %w[UTF-8-UNIX UTF-8-arbitrary-suffix]).uniq.sort
controls = {
  'ascii-letter' => 'A', 'ascii-decimal' => '1', 'ascii-punctuation' => '!',
  'cjk-letter' => '中', 'kana-letter' => 'ソ', 'hangul-letter' => '가',
  'latin-letter' => 'é', 'fullwidth-decimal' => '１', 'roman-alphabetic' => 'Ⅰ',
  'punctuation' => '、', 'star' => '☆', 'numeric-not-decimal' => '²',
  'unicode15-letter' => "\uA7CA", 'post15-cyrillic' => "\u1C89", 'post15-latin' => "\uA7CB",
  'supplementary-letter' => "\u{10428}", 'supplementary-decimal' => "\u{1D7D9}",
  'post15-supplementary' => "\u{16100}", 'emoji' => "\u{1F600}"
}
config = RuboCop::ConfigLoader.default_configuration
config['Style/RedundantStringEscape']['Enabled'] = true
config['AllCops']['NewCops'] = 'enable'
cop_class = RuboCop::Cop::Style::RedundantStringEscape
loaded_source = cop_class.instance_method(:on_str).source_location.first
pinned_source = File.join(ref, 'lib/rubocop/cop/style/redundant_string_escape.rb')
abort 'loaded reference cop differs from pinned source' unless Digest::SHA256.file(loaded_source).hexdigest == Digest::SHA256.file(pinned_source).hexdigest
rows = []
headers = []
Dir.mktmpdir('escape-encoding-matrix-') do |dir|
  FileUtils.mkdir_p(File.join(dir, '.git'))
  File.write(File.join(dir, 'rustocop.toml'), '')
  path = File.join(dir, 'a.rb')
  names.each do |name|
    header = "# encoding: #{name}\n"
    File.binwrite(path, header + "p nil\n")
    out, err, status = Open3.capture3(binary, '--only', cop_class.cop_name, path, chdir: dir)
    begin
      processed = RuboCop::ProcessedSource.new(header + "p nil\n", 3.4, path, parser_engine: :parser_prism)
      ref_error = processed.valid_syntax? ? nil : processed.diagnostics.map(&:message).join('; ')
      ref_error = processed.parser_error.inspect if ref_error == ''
    rescue StandardError => e
      ref_error = "#{e.class}: #{e.message}"
    end
    headers << { name: name, production: status.exitstatus, production_error: err.empty? ? out : err,
                 reference_error: ref_error }
    # Keep process/header result immutable; later probes have separate statuses.
    # Exhaust every high byte of single-byte profiles, not only Latin letters.
    # Also attempt raw characters where the host has a decoder but no encoder;
    # genuine decoder absence is a separately recorded reference exclusion.
    profile = profiles.find { |_, canonical, _| canonical.casecmp?(name) }
    begin
      ruby_encoding = Encoding.find(name)
      profile ||= profiles.find { |_, canonical, _| canonical == ruby_encoding&.name }
    rescue ArgumentError
      ruby_encoding = nil
    end
    raw_controls = if profile && profile[2] == 'false'
                     (128..255).to_h { |b| [format('raw-byte-%02x', b), [b].pack('C')] }
                   elsif %w[MacJapanese MacJapan].include?(name)
                     { 'raw-letter' => "\x83\x5c".b, 'raw-punctuation' => "\x81\x41".b }
                   elsif %w[EUC-TW eucTW].include?(name)
                     { 'raw-letter' => "\xc4\xe3".b, 'raw-punctuation' => "\xa1\xa2".b, 'raw-plane2' => "\x8e\xa2\xa1\xa1".b }
                   elsif name == 'Emacs-Mule'
                     { 'raw-letter' => "\x92\xc3\xe6".b, 'raw-punctuation' => "\x92\xa1\xa2".b }
                   else
                     {}
                   end
    if name.match?(/\AUTF8-(?:DoCoMo|KDDI|SoftBank)\z/i)
      # Audit the entire mobile private-use mapping family, then test every
      # class-changing scalar plus adjacent controls through both real cops.
      (0xe000..0xf8ff).each do |code|
        raw = [code].pack('U')
        begin
          projected = raw.dup.force_encoding(name).encode('UTF-8')
        rescue EncodingError
          next
        end
        next if /\A[[:alnum:]]\z/.match?(raw) == /\A[[:alnum:]]\z/.match?(projected[0])
        [code - 1, code, code + 1].each { |c| raw_controls[format('mobile-projection-%04x', c)] = [c].pack('U').b }
      end
    end
    controls.merge(raw_controls).each do |control, character|
      row = { name: name, control: control }
      # Distinguish name rejection from missing non-ASCII codec support.
      if status.exitstatus == 2 || ref_error
        row[:exclusion] = 'header/parser'; rows << row; next
      end
      begin
        # CESU-8 and mobile Ruby codecs can lack an encoder even for valid BMP
        # source. Direct bytes are valid BMP UTF-8; supplementary CESU-8 is the
        # two UTF-16 surrogate units, NOT ordinary four-byte UTF-8.
        encoded = if raw_controls.key?(control)
                    character
                  elsif character.ascii_only? || name.match?(/\AUTF-8(?:-|\z)/i) && !name.casecmp?('UTF-8-HFS') && !name.casecmp?('UTF-8-MAC')
                    character.b
                  elsif name.casecmp?('CESU-8')
                    character.encode('UTF-16BE').unpack('n*').pack('U*').b
                  elsif name.match?(/\AUTF8-(?:DoCoMo|KDDI|SoftBank)\z/i)
                    character.b
                  else
                    character.encode(name).b
                  end
        source = header.b + 'p "\\'.b + encoded + "\"\n".b
        row[:hex] = encoded.unpack1('H*')
        processed = RuboCop::ProcessedSource.new(source.dup, 3.4, path, parser_engine: :parser_prism)
        unless processed.valid_syntax?
          row[:exclusion] = 'reference/parser'
          row[:detail] = processed.diagnostics.map(&:message).join('; ')
          row[:detail] = processed.parser_error.inspect if row[:detail].empty? && processed.respond_to?(:parser_error)
          rows << row; next
        end
        team = RuboCop::Cop::Team.new([cop_class.new(config)], config, raise_error: true)
        report = team.investigate(processed)
        expected = report.offenses.map { |o| [o.line, o.column + 1] }
        File.binwrite(path, source)
        out, err, result = Open3.capture3(binary, '--only', cop_class.cop_name, path, chdir: dir)
        if result.exitstatus == 2
          row[:exclusion] = 'production/parser'; row[:detail] = err.empty? ? out : err
        else
          actual = out.scan(/a\.rb:(\d+):(\d+):.*Style\/RedundantStringEscape/).map { |a| a.map(&:to_i) }
          row[:expected] = expected; row[:actual] = actual
          row[:match] = expected == actual
          # Even clean/uncorrectable encodings must retain bytes and metadata.
          if control == 'ascii-punctuation'
            expected_fix = source.sub('\\!', '!')
            _, _, fixed = Open3.capture3(binary, '--only', cop_class.cop_name, '--fix', path, chdir: dir)
            row[:canonical_fix] = fixed.exitstatus == 0 && File.binread(path) == expected_fix
            time = File.mtime(path)
            _, _, stable = Open3.capture3(binary, '--only', cop_class.cop_name, '--fix', path, chdir: dir)
            row[:canonical_fix] &&= stable.exitstatus == 0 && File.binread(path) == expected_fix && File.mtime(path) == time
          else
            time = File.mtime(path)
            _, fix_err, fix_status = Open3.capture3(binary, '--only', cop_class.cop_name, '--fix', path, chdir: dir)
            row[:no_write] = File.binread(path) == source && File.mtime(path) == time && fix_status.exitstatus == result.exitstatus
            unless row[:no_write]
              row[:fix_status] = fix_status.exitstatus
              row[:lint_status] = result.exitstatus
              row[:fixed_hex] = File.binread(path).unpack1('H*')
              row[:fix_error] = fix_err
            end
          end
        end
      rescue EncodingError, ArgumentError, NotImplementedError => e
        row[:exclusion] = 'reference/codec'; row[:detail] = "#{e.class}: #{e.message}"
      end
      rows << row
    end
  end
end
report = { host: RUBY_DESCRIPTION, unicode: RbConfig::CONFIG['UNICODE_VERSION'], pin: pin.strip,
           profiles: profiles.map { |id, name, multi| { id: id, name: name, multibyte: multi == 'true' } },
           explicit_parser_aliases: aliases.uniq.sort,
           utf8_suffix_rule: 'case-insensitive UTF-8 prefix; exact UTF-8-HFS maps to UTF8-MAC, all other suffixes map to UTF-8',
           headers: headers, rows: rows,
           summary: { names: names.length, profiles: profiles.length, cases: rows.length,
                      comparable: rows.count { |r| r.key?(:match) },
                      mismatches: rows.count { |r| r[:match] == false },
                      write_failures: rows.count { |r| r[:no_write] == false },
                      canonical_failures: rows.count { |r| r[:canonical_fix] == false },
                      exclusions: rows.group_by { |r| r[:exclusion] }.reject { |k, _| k.nil? }.transform_values(&:length) } }
File.write(ARGV.fetch(0), JSON.pretty_generate(report) + "\n")
puts JSON.pretty_generate(report[:summary])
exit(report[:summary].values_at(:mismatches, :write_failures, :canonical_failures).all?(&:zero?) ? 0 : 1)
