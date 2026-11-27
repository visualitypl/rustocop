# frozen_string_literal: true

# The settings of rubocop-rails-omakase 1.1.0.
RSpec.describe RuboCop::Cop::Layout::SpaceInsideBlockBraces, :config do
  let(:cop_config) { { 'EnforcedStyleForEmptyBraces' => 'space' } }

  it 'accepts a space inside empty braces' do
    expect_no_offenses(<<~RUBY)
      [ 1, 2 ].each { }
    RUBY
  end

  it 'registers an offense for empty braces without a space' do
    expect_offense(<<~RUBY)
      each {}
           ^^ Space missing inside empty braces.
    RUBY

    expect_correction(<<~RUBY)
      each { }
    RUBY
  end
end
