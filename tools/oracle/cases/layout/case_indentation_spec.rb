# frozen_string_literal: true

# `case` on the right of an assignment, with `EnforcedStyle: end` as
# rubocop-rails-omakase 1.1.0 sets it.
RSpec.describe RuboCop::Cop::Layout::CaseIndentation, :config do
  let(:cop_config) { { 'EnforcedStyle' => 'end' } }

  it 'accepts `when` and `end` at the start of the line' do
    expect_no_offenses(<<~RUBY)
      output = case variable
      when 'value1'
        'output1'
      else
        'output2'
      end
    RUBY
  end

  it 'accepts `when` and `end` under `case`' do
    expect_no_offenses(<<~RUBY)
      output = case variable
               when 'value1'
                 'output1'
               else
                 'output2'
               end
    RUBY
  end

  it 'registers an offense for `when` under `case` and `end` at the start of the line' do
    expect_offense(<<~RUBY)
      output = case variable
               when 'value1'
               ^^^^ Indent `when` as deep as `end`.
                 'output1'
      end
    RUBY

    expect_correction(<<~RUBY)
      output = case variable
      when 'value1'
                 'output1'
      end
    RUBY
  end

  it 'registers an offense for `when` at the start of the line and `end` under `case`' do
    expect_offense(<<~RUBY)
      output = case variable
      when 'value1'
      ^^^^ Indent `when` as deep as `end`.
        'output1'
               end
    RUBY

    expect_correction(<<~RUBY)
      output = case variable
               when 'value1'
        'output1'
               end
    RUBY
  end

  it 'registers an offense for `in` under `case` and `end` at the start of the line' do
    expect_offense(<<~RUBY)
      output = case variable
               in [x]
               ^^ Indent `in` as deep as `end`.
                 x
      end
    RUBY

    expect_correction(<<~RUBY)
      output = case variable
      in [x]
                 x
      end
    RUBY
  end

  it 'accepts an `end` on the line of the `else`' do
    expect_no_offenses(<<~RUBY)
      output = case variable
               when 'value1' then 'output1'
               else 'output2' end
    RUBY
  end

  it 'accepts an `end` on the line of the last `then`' do
    expect_no_offenses(<<~RUBY)
      output = case variable
               when 'value1' then 'output1' end
    RUBY
  end
end
