# frozen_string_literal: true

require 'minitest/autorun'
require 'open3'
require 'json'
require 'tmpdir'
require 'rbconfig'

class EscapeEncodingMatrixTest < Minitest::Test
  def test_complete_profile_alias_matrix_matches_pin_and_preserves_files
    root = File.expand_path('../..', __dir__)
    Dir.mktmpdir('escape-matrix-report-') do |dir|
      path = File.join(dir, 'matrix.json')
      output, error, status = Open3.capture3(RbConfig.ruby, File.join(root, 'tools/escape_encoding_matrix.rb'), path)
      assert status.success?, output + error
      report = JSON.parse(File.read(path))
      assert_equal 90, report.fetch('profiles').length
      assert_equal 149, report.fetch('explicit_parser_aliases').length
      assert_equal 177, report.fetch('headers').length
      assert_equal 0, report.fetch('summary').fetch('mismatches')
      assert_equal 0, report.fetch('summary').fetch('write_failures')
      assert_equal 0, report.fetch('summary').fetch('canonical_failures')
      report.fetch('profiles').each do |profile|
        assert report.fetch('headers').any? { |h| h.fetch('name') == profile.fetch('name') && h.fetch('production') != 2 }, profile.inspect
      end
      %w[UTF8-MAC UTF8-DoCoMo CESU-8].each do |name|
        %w[cjk-letter unicode15-letter].each do |control|
          row = report.fetch('rows').find { |r| r.fetch('name') == name && r.fetch('control') == control }
          assert_equal [], row.fetch('expected')
          assert row.fetch('match') && row.fetch('no_write')
        end
      end
      # No new baseline or generic skip of encodings: every exclusion carries
      # its specific name, control, and (where bytes exist) parser/codec reason.
      report.fetch('rows').select { |r| r.key?('exclusion') && r.key?('hex') }.each do |row|
        refute_empty row.fetch('detail'), row.inspect
      end
    end
  end
end
