# Examples adapted from RuboCop's space_inside_{hash_literal,block}_braces specs
# at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
h = {a: 1, b: 2}
empty = { }
nested = {{ x: 1 } => {y: 2}}
foo({key: value} => {key: value})
case h
in {a: 1, b: 2}
end
each { puts }
each { |x| puts }
[1, 2, 3].each { _1 * 2 }
[1, 2, 3].each { it * 2 }
->(x) { x }
each {}
each {
}
each { |x|
 x}
f(get: "#{x}")
each { {key: 1} }
h = {# Comment
  a: 1
}
