# frozen_string_literal: true

# The settings of rubocop-rails-omakase 1.1.0.
RSpec.describe RuboCop::Cop::Style::Semicolon, :config do
  let(:cop_config) { { 'AllowAsExpressionSeparator' => true } }

  it 'accepts a semicolon between expressions' do
    expect_no_offenses(<<~RUBY)
      x = 1; y = 2
    RUBY
  end
end
