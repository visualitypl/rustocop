# frozen_string_literal: true

# The settings of rubocop-rails-omakase 1.1.0.
RSpec.describe RuboCop::Cop::Style::RedundantReturn, :config do
  let(:cop_config) { { 'AllowMultipleReturnValues' => true } }

  it 'accepts a `return` of several values' do
    expect_no_offenses(<<~RUBY)
      def pair
        return 1, 2
      end
    RUBY
  end
end
