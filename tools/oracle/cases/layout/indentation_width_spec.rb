# frozen_string_literal: true

# The settings of rubocop-rails-omakase 1.1.0.
RSpec.describe RuboCop::Cop::Layout::IndentationWidth, :config do
  let(:other_cops) { { 'Layout/EndAlignment' => { 'EnforcedStyleAlignWith' => 'variable' } } }

  it 'accepts the body of an assigned `if` indented from the variable' do
    expect_no_offenses(<<~RUBY)
      result = if ready?
        1
      else
        2
      end
    RUBY
  end

  it 'registers an offense for the body of an assigned `if` indented from the `if`' do
    expect_offense(<<~RUBY)
      result = if ready?
                 1
      ^^^^^^^^^^^ Use 2 (not 11) spaces for indentation.
      else
        2
      end
    RUBY

    expect_correction(<<~RUBY)
      result = if ready?
        1
      else
        2
      end
    RUBY
  end
end
