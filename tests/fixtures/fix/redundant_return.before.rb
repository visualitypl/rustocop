def func
  if x
    return (foo && bar), 42
  elsif y
    return :a => 1, :b => 2
  else
    return() # comment
  end
end
-> { return *something }
