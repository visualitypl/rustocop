# Copyright 2026 Test
# Controlled block binding, guards, rescue, forwarding and numbered parameters.
def collect(*args)
  args.map { |x| yield x }
end

def forward(*args, **kwargs, &block)
  collect(*args, **kwargs, &block)
end

def guarded(ready)
  if ready
    :yes
  else
    :no
  end
end

values = forward(1, 2) { |x| x + 1 }
puts [values, guarded(true), guarded(false), [3,1].sort_by { |x| x }, ['one','two'], [:one,:two]].inspect
puts [1,2].map { puts _1 }.inspect
puts (1 rescue nil).inspect
