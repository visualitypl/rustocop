# frozen_string_literal: true

require 'minitest/autorun'
require 'open3'
require 'rbconfig'

# A development-time reproducibility check, not a runtime dependency. The
# pinned source-buffer projection uses Ruby's charset maps and Unicode 15.0.
class EscapeAlnumTest < Minitest::Test
  def test_generated_posix_facts_are_current
    skip 'generator pins Unicode 15.0' unless RbConfig::CONFIG['UNICODE_VERSION'] == '15.0.0'

    root = File.expand_path('../..', __dir__)
    output, error, status = Open3.capture3(RbConfig.ruby, File.join(root, 'tools/generate_escape_alnum.rb'))
    assert status.success?, error
    assert_equal File.binread(File.join(root, 'src/source/escape_alnum.rs')), output
  end
end
