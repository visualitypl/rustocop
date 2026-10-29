# Copyright 2026 Test
# Controlled declarations, evaluation order, exception results and literals.
class CorrectionProbe
  attr_reader :one
  attr_reader :two

  def self.values( value=1 )
    # This comment belongs to the returned collection.
    return [ 'héllo' ,value+1,{:key=>'v'}, 12345, 0XFF, %w(one two), %W(cat dog), %q(text), :"word", /[a]/.match?('a') ];
  end

  def self.accumulate(items)
    total = 0
    items.each do |item|
      total = total + item
    end
    total
  end

  def self.failure
    fail 'controlled'
  end
end

events = []
record = ->(value) { events << value; value }
a, b = 1, 2
result = record.call(a) + record.call(b)
puts [CorrectionProbe.values, CorrectionProbe.accumulate([1,2]), result, events].inspect
begin
  CorrectionProbe.failure
rescue => error
  puts "#{ 'message'.upcase }: #{error.class}: #{error.message}"
end
