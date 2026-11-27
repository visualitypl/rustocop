# frozen_string_literal: true

# The settings of rubocop-rails-omakase 1.1.0.
RSpec.describe RuboCop::Cop::Layout::SpaceInsideArrayLiteralBrackets, :config do
  let(:cop_config) { { 'EnforcedStyle' => 'space', 'EnforcedStyleForEmptyBrackets' => 'no_space' } }

  it 'accepts spaces inside the brackets' do
    expect_no_offenses(<<~RUBY)
      [ 1, 2 ].each { }
    RUBY
  end

  it 'registers offenses for brackets without spaces' do
    expect_offense(<<~RUBY)
      [1, 2]
      ^ Use space inside array brackets.
           ^ Use space inside array brackets.
    RUBY

    expect_correction(<<~RUBY)
      [ 1, 2 ]
    RUBY
  end

  it 'registers an offense for a space inside empty brackets' do
    expect_offense(<<~RUBY)
      [ ]
      ^^^ Do not use space inside empty array brackets.
    RUBY

    expect_correction(<<~RUBY)
      []
    RUBY
  end
end
