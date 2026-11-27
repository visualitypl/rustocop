# frozen_string_literal: true

# The settings of rubocop-rails-omakase 1.1.0.
RSpec.describe RuboCop::Cop::Layout::EndAlignment, :config do
  let(:cop_config) { { 'EnforcedStyleAlignWith' => 'variable' } }

  it 'accepts the `end` of an assigned `if` under the variable' do
    expect_no_offenses(<<~RUBY)
      result = if ready?
        1
      else
        2
      end
    RUBY
  end

  it 'registers an offense for the `end` of an assigned `if` under the `if`' do
    expect_offense(<<~RUBY)
      result = if ready?
                 1
               end
               ^^^ `end` at 3, 9 is not aligned with `result = if` at 1, 0.
    RUBY

    expect_correction(<<~RUBY)
      result = if ready?
                 1
      end
    RUBY
  end

  it 'accepts the `end` of an assigned `case` under the variable' do
    expect_no_offenses(<<~RUBY)
      result = case kind
      when :a then 1
      end
    RUBY
  end
end
