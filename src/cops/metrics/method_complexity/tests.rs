//! Shared `MethodComplexity` checks against the pinned RuboCop 1.92.0 cops
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
//!
//! `REFERENCE` holds 102 authored sources and, for every method, the
//! complexity, perceived complexity and ABC vector/size the pinned
//! `Metrics/CyclomaticComplexity`, `Metrics/PerceivedComplexity` and
//! `Metrics/AbcSize` reported for it at `Max: 0`, with Prism's parser
//! translation investigating the source strings (nothing was executed).
//! They cover the parser shapes this port must reproduce: `unless` branch
//! order, a block around its send, in-guards after their pattern, pattern
//! captures and named regexp captures (not `lvasgn`), `begin ... end while`,
//! numbered/`it` blocks, lambdas, rescue chains, shorthand and multiple
//! assignments, setters, comparisons, repeated `&.` resets, `case` weighting
//! and `define_method` name literals.
use super::*;
use crate::cops::metrics::{abc_size::Abc, cyclomatic_complexity, perceived_complexity};

/// `(line, col, method, complexity, perceived, "<A, B, C> size")`.
type Measures = &'static [(u32, u32, &'static str, usize, usize, &'static str)];

const REFERENCE: &[(&str, Measures)] = &[
    (
        r##"def m
  begin
    a
  rescue A
    b
  rescue B
    c
  end
end
"##,
        &[(1, 1, "m", 2, 2, "<0, 3, 1> 3.16")],
    ),
    (
        r##"def m
  begin
    a
  end while b
end
"##,
        &[(1, 1, "m", 1, 1, "<0, 2, 0> 2")],
    ),
    (
        r##"def m
  case x
  in 1
  else
  end
end
"##,
        &[(1, 1, "m", 2, 1, "<0, 1, 1> 1.41")],
    ),
    (
        r##"def m
  case x
  in 1
  else
    y
  end
end
"##,
        &[(1, 1, "m", 2, 1, "<0, 2, 1> 2.24")],
    ),
    (
        r##"def m
  case x
  when 1
  else
  end
end
"##,
        &[(1, 1, "m", 2, 2, "<0, 1, 1> 1.41")],
    ),
    (
        r##"define_method(:m) do
  [1].each { it&.a; it&.b }
end
"##,
        &[(1, 1, "m", 2, 2, "<0, 3, 1> 3.16")],
    ),
    (
        r##"define_method(:m) do
  [1].each { _1&.a; _1&.b }
end
"##,
        &[(1, 1, "m", 2, 2, "<0, 3, 1> 3.16")],
    ),
    (
        r##"define_method(<<~X) do
  a
  b
X
  a&.b
end
"##,
        &[],
    ),
    (
        r##"def m
  a.[]=(1, 2)
  a.b=(1)
end
"##,
        &[(1, 1, "m", 1, 1, "<1, 4, 0> 4.12")],
    ),
    (
        r##"def m
  begin
  rescue => a.b
  end
end
"##,
        &[(1, 1, "m", 2, 2, "<0, 2, 1> 2.24")],
    ),
    (
        r##"def m
  x ? 1 : 2
  unless c
    1
  else
    2
  end
end
"##,
        &[(1, 1, "m", 3, 4, "<0, 2, 3> 3.61")],
    ),
    (
        r##"def m
  if a
  elsif b
  end
end
"##,
        &[(1, 1, "m", 3, 4, "<0, 2, 2> 2.83")],
    ),
    (
        r##"def m
  a&.b ||= 1
  x ||= foo
  y += super
  z ||= defined?(q)
end
"##,
        &[(1, 1, "m", 5, 5, "<7, 4, 4> 9")],
    ),
    (
        r##"def m
  a.b, c[1], *d.e, (f.g, h) = 1
end
"##,
        &[(1, 1, "m", 1, 1, "<5, 8, 0> 9.43")],
    ),
    (
        r##"def m
  /(?<k>.)/ =~ s
  k&.a
end
"##,
        &[(1, 1, "m", 2, 2, "<0, 2, 1> 2.24")],
    ),
    (
        r##"def m
  v = 1
  v&.each { v = 2 }
  v&.a
end
"##,
        &[(1, 1, "m", 4, 4, "<2, 2, 3> 4.12")],
    ),
    (
        r##"def m
  v = 1
  case q
  in Integer if v&.a
  end
  v&.b
end
"##,
        &[(1, 1, "m", 3, 3, "<1, 3, 2> 3.74")],
    ),
    (
        r##"def m
  ->(a, b) { a }
  lambda { |c| c }
  super(&blk)
  x.map(&:y)
  x&.select { 1 }
end
"##,
        &[(1, 1, "m", 4, 4, "<3, 7, 3> 8.19")],
    ),
    (
        r##"def m
  a.[]=(1, 2)
end
"##,
        &[(1, 1, "m", 1, 1, "<0, 2, 0> 2")],
    ),
    (
        r##"def m
  a.b=(1)
end
"##,
        &[(1, 1, "m", 1, 1, "<1, 2, 0> 2.24")],
    ),
    (
        r##"def m
  a::b = 1
end
"##,
        &[(1, 1, "m", 1, 1, "<1, 2, 0> 2.24")],
    ),
    (
        r##"def m
  case x
  in 1
  in 2
  else
  end
end
"##,
        &[(1, 1, "m", 3, 2, "<0, 1, 2> 2.24")],
    ),
    (
        r##"def m
  case x
  in 1
  in 2
  end
end
"##,
        &[(1, 1, "m", 3, 1, "<0, 1, 2> 2.24")],
    ),
    (
        r##"def m
  case x
  when 1
  when 2
  when 3
  else
  end
end
"##,
        &[(1, 1, "m", 4, 2, "<0, 1, 3> 3.16")],
    ),
    (
        r##"def m
  case x
  when 1
  when 2
  when 3
  else
    nil
  end
end
"##,
        &[(1, 1, "m", 4, 3, "<0, 1, 3> 3.16")],
    ),
    (
        r##"def m
  v = 1
  v&.==(2)
  v&.a
  v&.b
end
"##,
        &[(1, 1, "m", 2, 2, "<1, 2, 2> 3")],
    ),
    (
        r##"def m
  v = 1
  v = v&.a
  v&.b
end
"##,
        &[(1, 1, "m", 2, 2, "<2, 2, 2> 3.46")],
    ),
    (
        r##"def m(v)
  v&.a
  v&.b
end
"##,
        &[(1, 1, "m", 2, 2, "<0, 2, 1> 2.24")],
    ),
    (
        r##"def m
  @v&.a
  @v&.b
end
"##,
        &[(1, 1, "m", 3, 3, "<0, 2, 2> 2.83")],
    ),
    (
        r##"def m
  if a
  else
  end
  case b
  when 1 then 2
  else 3
  end
end
"##,
        &[(1, 1, "m", 3, 4, "<0, 2, 3> 3.61")],
    ),
    (
        r##"def m
  x = 1 unless y
  for a, b in c do end
  until d do end
  e while f
  g until h
end
"##,
        &[(1, 1, "m", 6, 6, "<4, 7, 5> 9.49")],
    ),
    (
        r##"def m
  yield
  yield(1) if block_given?
  super
  -x
  !y
  not z
  a == b
  c != d
  e <=> f
  g === h
end
"##,
        &[(1, 1, "m", 2, 2, "<0, 18, 4> 18.44")],
    ),
    (
        r##"def m
  case
  when a then 1
  when b then 2
  end
end
"##,
        &[(1, 1, "m", 3, 3, "<0, 2, 2> 2.83")],
    ),
    (
        r##"def m
  x = [1].each_with_index.map { |a, i| a }
  y = h.transform_values(&:to_s)
  h.fetch(:a) { 1 }
  h.merge(o) { |*| 1 }
  loop { break }
  3.times { }
end
"##,
        &[(1, 1, "m", 5, 5, "<4, 11, 4> 12.37")],
    ),
    (
        r##"def m
  a&.b&.c
  d&.e.f
end
"##,
        &[(1, 1, "m", 4, 4, "<0, 6, 3> 6.71")],
    ),
    (
        r##"def m = a ? b : c
"##,
        &[(1, 1, "m", 2, 2, "<0, 3, 1> 3.16")],
    ),
    (
        r##"def self.m
  def n
    a if b
  end
  c if d
end
"##,
        &[
            (1, 1, "m", 3, 3, "<0, 4, 2> 4.47"),
            (2, 3, "n", 2, 2, "<0, 2, 1> 2.24"),
        ],
    ),
    (
        r##"class X
  def m
    a if b
  end
  define_method :n do
    x if y
  end
  define_method('o') { z if w }
  define_method(:"p") { z if w }
  define_method(:"q#{1}") { z if w }
  self.define_method(:r) { z if w }
  define_method(:s, &blk)
  define_method(:t) { |a| }
  define_method(:u) do
    _1 if _2
  end
  define_method(:v) { it if it }
  define_method(:w, :x) { z if w }
  define_method("y") { z if w }
  define_method(%s(zz)) { z if w }
  define_method(?c) { z if w }
end
"##,
        &[
            (2, 3, "m", 2, 2, "<0, 2, 1> 2.24"),
            (5, 3, "n", 2, 2, "<0, 2, 1> 2.24"),
            (8, 3, "o", 2, 2, "<0, 2, 1> 2.24"),
            (9, 3, "p", 2, 2, "<0, 2, 1> 2.24"),
            (14, 3, "u", 2, 2, "<0, 0, 1> 1"),
            (17, 3, "v", 2, 2, "<0, 0, 1> 1"),
            (19, 3, "y", 2, 2, "<0, 2, 1> 2.24"),
            (20, 3, "zz", 2, 2, "<0, 2, 1> 2.24"),
            (21, 3, "c", 2, 2, "<0, 2, 1> 2.24"),
        ],
    ),
    (
        r##"def m
  x = - 1
  y = -2 ** 2
  z = - 1.5
  /a/ =~ s
  /a#{b}/ =~ s
  s =~ /a/
  /a/.=~(s)
  $~ if /(?<q>x)/ =~ t
end
"##,
        &[(1, 1, "m", 2, 2, "<3, 11, 1> 11.45")],
    ),
    (
        r##"def m
  a.b = 1 if c
  d&.e = 2
  f[1] = 2
  g[1] += 2
  h.i += 1
  j&.k ||= 3
  l[2] ||= 4
  m[3] &&= 5
end
"##,
        &[(1, 1, "m", 7, 7, "<8, 17, 6> 19.72")],
    ),
    (
        r##"def m(a, b = 1, *c, d, e:, f: 2, **g, &h)
  a
end
"##,
        &[(1, 1, "m", 1, 1, "<0, 0, 0> 0")],
    ),
    (
        r##"def m(_a, _b = 1, *_c, _d:, **_e, &_f)
  1
end
"##,
        &[(1, 1, "m", 1, 1, "<0, 0, 0> 0")],
    ),
    (
        r##"def m(*, **, &)
  n(*, **, &)
end
"##,
        &[(1, 1, "m", 1, 1, "<0, 1, 0> 1")],
    ),
    (
        r##"def m(...)
  n(...)
end
"##,
        &[(1, 1, "m", 1, 1, "<0, 1, 0> 1")],
    ),
    (
        r##"def m
  x.each { |a, (b, c), *d; e| a }
  y.map { |q| q }
  z.select { |_| 1 }
end
"##,
        &[(1, 1, "m", 4, 4, "<6, 6, 3> 9")],
    ),
    (
        r##"def m
  begin
    a
  rescue A => e
    b
  rescue => @f
    c
  else
    d
  ensure
    g
  end
end
"##,
        &[(1, 1, "m", 2, 2, "<2, 5, 1> 5.48")],
    ),
    (
        r##"def m
  a rescue b
  c = d rescue e
end
"##,
        &[(1, 1, "m", 3, 3, "<1, 4, 2> 4.58")],
    ),
    (
        r##"def m
  x = y while z
  begin; w; end until v
  loop do break end
end
"##,
        &[(1, 1, "m", 2, 2, "<1, 5, 1> 5.2")],
    ),
    (
        r##"def m
  for i in 1..3
    for j, k in l
      j
    end
  end
end
"##,
        &[(1, 1, "m", 3, 3, "<5, 1, 2> 5.48")],
    ),
    (
        r##"def m
  !a
  a && !b || c and d or e
  not f
end
"##,
        &[(1, 1, "m", 5, 5, "<0, 10, 4> 10.77")],
    ),
    (
        r##"def m
  if a
    b
  elsif c
    d
  elsif e
    f
  else
    g
  end
end
"##,
        &[(1, 1, "m", 4, 5, "<0, 7, 4> 8.06")],
    ),
    (
        r##"def m
  unless a
    b
  end
  c unless d
  e ? f : g ? h : i
end
"##,
        &[(1, 1, "m", 5, 5, "<0, 9, 4> 9.85")],
    ),
    (
        r##"def m
  case
  when a, b then 1
  else 2
  end
  case x
  when 1, 2 then 3
  when 4 then 5
  when 6 then 7
  when 8 then 9
  end
end
"##,
        &[(1, 1, "m", 6, 5, "<0, 3, 5> 5.83")],
    ),
    (
        r##"def m
  case x
  in 1 then a
  in 2 then b
  in 3 then c
  in 4 then d
  in 5 then e
  in 6 then f
  in 7 then g
  in 8 then h
  else i
  end
end
"##,
        &[(1, 1, "m", 9, 3, "<0, 10, 8> 12.81")],
    ),
    (
        r##"def m
  case x
  in [1] then a
  in 2 then b
  in 3 then c
  else
  end
end
"##,
        &[(1, 1, "m", 4, 3, "<0, 4, 3> 5")],
    ),
    (
        r##"def m
  v = 1
  a = v&.b
  v&.c
  v ||= 2
  v&.d
  v += 1
  v&.e
  w, v = 1
  v&.f
  for v in [1]; v&.g; end
  begin; rescue => v; v&.h; end
end
"##,
        &[(1, 1, "m", 10, 10, "<9, 7, 9> 14.53")],
    ),
    (
        r##"def m
  v = 1
  unless c
    v&.a
  else
    v = 1
    v&.b
  end
  v&.c if (v = 2)
end
"##,
        &[(1, 1, "m", 5, 6, "<3, 4, 5> 7.07")],
    ),
    (
        r##"def m
  yield&.a
  super&.b
  super(1)&.c
  defined?(d&.e)
end
"##,
        &[(1, 1, "m", 5, 5, "<0, 6, 4> 7.21")],
    ),
    (
        r##"def m
  a = proc { |x| x }
  b = lambda do |y| y end
  c = ->(z) do z end
  d = -> { _1 }
  e = -> { it }
end
"##,
        &[(1, 1, "m", 1, 1, "<8, 5, 0> 9.43")],
    ),
    (
        r##"def m
  x.each_with_object({}) { |(k, v), h| h[k] = v }
  y.sum(&:size)
  z.tally
  w.each_slice(2).to_a
  q.min_by(&:x).max { |a, b| a <=> b }
end
"##,
        &[(1, 1, "m", 5, 5, "<6, 14, 4> 15.75")],
    ),
    (
        r##"def m
  "#{a&.b}" + "#{a&.c}"
  :"#{d}"
  `#{e}`
  /#{f}/
  %W[#{g} h]
end
"##,
        &[(1, 1, "m", 3, 3, "<0, 9, 2> 9.22")],
    ),
    (
        r##"def m = (a || b) && c
"##,
        &[(1, 1, "m", 3, 3, "<0, 3, 2> 3.61")],
    ),
    (
        r##"def m
  [1, 2].each do |x|
    next unless x
    yield x if a? || b?(x)
  end
  self
end
"##,
        &[(1, 1, "m", 5, 5, "<1, 4, 4> 5.74")],
    ),
    (
        r##"def m
  x = 1
  x&.a.b&.c
  x&.d
  x.e&.f
end
"##,
        &[(1, 1, "m", 4, 4, "<1, 6, 3> 6.78")],
    ),
    (
        r##"def m
  a.b(&c)
  d.map(&e)
  f.reject!(&:g)
  h.fetch(1, &i)
  j.sort_by!(&:k)
end
"##,
        &[(1, 1, "m", 4, 4, "<0, 13, 3> 13.34")],
    ),
    (
        r##"def m
  if a
  end
  if b then c else d end
  e = f if g
  h = (i if j)
end
"##,
        &[(1, 1, "m", 5, 6, "<2, 8, 5> 9.64")],
    ),
    (
        r##"def m
  while a
    break if b
    next unless c
    redo if d
  end
  until e do end
end
"##,
        &[(1, 1, "m", 6, 6, "<0, 5, 5> 7.07")],
    ),
    (
        r##"define_method(:m) do
  i = 1
  case v
  in ^(i = 2) then 1
  in [x, *] then x&.y
  end
  i&.z
  i&.w
end
"##,
        &[(1, 1, "m", 5, 5, "<2, 4, 4> 6")],
    ),
    (
        r##"define_method(:m) do
  a, (b, *c), d = e
  *f = g
  h, = i
  @j, K, $l = 1
  M::N, O::P = 2
end
"##,
        &[(1, 1, "m", 1, 1, "<11, 3, 0> 11.4")],
    ),
    (
        r##"define_method(:m) do
  A::B = 1 rescue nil
  @x ||= {}
  @@y &&= 1
  $z += 1
  @x[:a] ||= []
  self.q ||= 1
  self.r += 1
  C ||= 1
  D::E &&= 2
  F::G += foo
  ::H = 3
end
"##,
        &[(1, 1, "m", 8, 8, "<12, 4, 7> 14.46")],
    ),
    (
        r##"define_method(:m) do
  BEGIN_x = 1
  class X; def r; s if t; end; end
  module Y; end
end
"##,
        &[
            (1, 1, "m", 2, 2, "<1, 2, 1> 2.45"),
            (3, 12, "r", 2, 2, "<0, 2, 1> 2.24"),
        ],
    ),
    (
        r##"define_method(:m) do
  case v
  in 1 | 2 then a
  in Foo(x) then b
  in Foo[y] then c
  in [*, 1, *] then d
  in 5 if q then e
  in 6
  in 7
  in 8
  in 9
  end
end
"##,
        &[(1, 1, "m", 10, 7, "<0, 7, 9> 11.4")],
    ),
    (
        r##"define_method(:m) do
  case v
  in [a, *b] if a
    1
  in {c:, d: [e]} unless c
    2
  in 7 | 8
    3
  in Integer => h
    4
  in ^(i)
    5
  end
  v => [j, k]
  v in [l]
end
"##,
        &[(1, 1, "m", 6, 6, "<0, 4, 5> 6.4")],
    ),
    (
        r##"define_method(:m) do
  y = 0
  case v
  in Integer | Float then 1
  in ^y then 2
  in [*] then 3
  in nil then 4
  in 1.. then 5
  in -1 then 6
  in %w[a] then 7
  in :s then 8
  in /r/ then 9
  in "a#{b}" then 10
  in ::Foo then 11
  in Foo::Bar then 12
  in __ENCODING__ then 13
  in x then 14
  in true then 15
  in __FILE__ then 16
  in __LINE__ then 17
  in 1r then 18
  in 2i then 19
  in 1.5 then 20
  in (1..2) then 21
  in -> x { x } then 22
  in ^(y) then 23
  in {} then 24
  in [] then 25
  in 'a' 'b' then 26
  in :"a#{1}" then 27
  in %i[a] then 28
  in nil.. then 29
  in false then 30
  in ..3 then 31
  in `x` then 32
  in 1...2 then 33
  end
end
"##,
        &[(1, 1, "m", 34, 15, "<2, 3, 33> 33.2")],
    ),
    (
        r##"define_method(%q()) do
  a if b
end
"##,
        &[],
    ),
    (
        r##"define_method(:"") do
  a if b
end
"##,
        &[],
    ),
    (
        r##"define_method("") do
  a if b
end
"##,
        &[(1, 1, "", 2, 2, "<0, 2, 1> 2.24")],
    ),
    (
        r##"define_method('') do
  a if b
end
"##,
        &[(1, 1, "", 2, 2, "<0, 2, 1> 2.24")],
    ),
    (
        r##"define_method("a
b") do
  a if b
end
"##,
        &[],
    ),
    (
        r##"define_method("a\\
b") do
  a if b
end
"##,
        &[],
    ),
    (
        r##"define_method('a\\
b') do
  a if b
end
"##,
        &[],
    ),
    (
        r##"define_method('a
b') do
  a if b
end
"##,
        &[],
    ),
    (
        r##"define_method(:"a
b") do
  a if b
end
"##,
        &[],
    ),
    (
        r##"define_method(%s(a
b)) do
  a if b
end
"##,
        &[],
    ),
    (
        r##"define_method(%(a
b)) do
  a if b
end
"##,
        &[],
    ),
    (
        r##"define_method("a
") do
  a if b
end
"##,
        &[(1, 1, "a\n", 2, 2, "<0, 2, 1> 2.24")],
    ),
    (
        r##"define_method(?a) do
  a if b
end
"##,
        &[(1, 1, "a", 2, 2, "<0, 2, 1> 2.24")],
    ),
    (
        r##"define_method(:a?) do
  a if b
end
"##,
        &[(1, 1, "a?", 2, 2, "<0, 2, 1> 2.24")],
    ),
    (
        r##"define_method(:[]=) do
  a if b
end
"##,
        &[(1, 1, "[]=", 2, 2, "<0, 2, 1> 2.24")],
    ),
    (
        r##"define_method(:"a b") do
  a if b
end
"##,
        &[(1, 1, "a b", 2, 2, "<0, 2, 1> 2.24")],
    ),
    (
        r##"define_method(%q(xy)) do
  a if b
end
"##,
        &[(1, 1, "xy", 2, 2, "<0, 2, 1> 2.24")],
    ),
    (
        r##"define_method(:"a\\
b") do
  a if b
end
"##,
        &[],
    ),
    (
        r##"define_method("a\
b") do
  a if b
end
"##,
        &[(1, 1, "ab", 2, 2, "<0, 2, 1> 2.24")],
    ),
    (
        r##"define_method(:"a\
b") do
  a if b
end
"##,
        &[(1, 1, "ab", 2, 2, "<0, 2, 1> 2.24")],
    ),
    (
        r##"define_method('a\
b') do
  a if b
end
"##,
        &[],
    ),
    (
        r##"define_method(<<~X) do
x
X
  a if b
end
"##,
        &[(1, 1, "x\n", 2, 2, "<0, 2, 1> 2.24")],
    ),
    (
        r##"define_method(<<~X) do
x
y
X
  a if b
end
"##,
        &[],
    ),
    (
        r##"define_method(<<~X) do
X
  a if b
end
"##,
        &[],
    ),
    (
        r##"define_method(<<~'X') do
x\
y
X
  a if b
end
"##,
        &[],
    ),
    (
        r##"define_method(<<~X) do
x\
y
X
  a if b
end
"##,
        &[(1, 1, "xy\n", 2, 2, "<0, 2, 1> 2.24")],
    ),
    (
        r##"define_method(<<-X) do
  x
  X
  a if b
end
"##,
        &[(1, 1, "  x\n", 2, 2, "<0, 2, 1> 2.24")],
    ),
    (
        r##"define_method(%Q(a\
b)) do
  a if b
end
"##,
        &[(1, 1, "ab", 2, 2, "<0, 2, 1> 2.24")],
    ),
];

#[test]
fn every_method_measures_as_the_pinned_cops() {
    assert_eq!(REFERENCE.len(), 102);
    for (source, expected) in REFERENCE {
        let got = measured(source, |body| {
            let abc = Abc::of(body);
            (
                cyclomatic_complexity::complexity(body, cyclomatic_complexity::cyclomatic_score),
                cyclomatic_complexity::complexity(body, perceived_complexity::perceived_score),
                format!(
                    "{} {}",
                    abc.vector(),
                    crate::cops::metrics::abc_size::format_g4(abc.size())
                ),
            )
        });
        let got: Vec<_> = got
            .iter()
            .map(|(l, c, n, (cc, pc, abc))| (*l, *c, n.as_str(), *cc, *pc, abc.as_str()))
            .collect();
        let mut expected = expected.to_vec();
        let mut sorted = got.clone();
        expected.sort_unstable();
        sorted.sort_unstable();
        assert_eq!(sorted, expected, "{source:?}");
    }
}

#[test]
fn only_receiverless_single_literal_define_method_blocks_are_methods() {
    let names = |source: &str| -> Vec<String> {
        measured(source, |_| ()).into_iter().map(|m| m.2).collect()
    };
    assert_eq!(names("define_method(:a) { b }\n"), ["a"]);
    assert_eq!(names("define_method 'a' do\n  b\nend\n"), ["a"]);
    assert_eq!(
        names("define_method(:a) { _1 }\ndefine_method(:b) { it }\n"),
        ["a", "b"]
    );
    assert_eq!(names("def self.a = nil\ndef b(c) = c\n"), ["a", "b"]);
    for lookalike in [
        "self.define_method(:a) { b }\n",
        "x&.define_method(:a) { b }\n",
        "define_method(:a, :b) { c }\n",
        "define_method(name) { c }\n",
        "define_method(:\"a#{1}\") { c }\n",
        "define_method(:a, &blk)\n",
        "define_method(:a) { }\n",
        "define_method(:a) { |x| }\n",
        "define_singleton_method(:a) { b }\n",
        "def a; end\n",
        "def a(b, c = 1); end\n",
        "lambda { a }\n",
    ] {
        assert_eq!(names(lookalike), Vec::<String>::new(), "{lookalike}");
    }
}

#[test]
fn repeated_csend_discount_is_per_variable_until_reassigned() {
    let send = |name: &'static [u8]| Send {
        csend: true,
        lvar_receiver: Some(name),
        comparison: false,
        setter: false,
    };
    let mut csend = RepeatedCsend::default();
    assert!(!csend.discount(&send(b"a")));
    assert!(csend.discount(&send(b"a")));
    assert!(!csend.discount(&send(b"b")));
    csend.reset(b"a");
    assert!(!csend.discount(&send(b"a")));
    let ivar = Send {
        lvar_receiver: None,
        ..send(b"a")
    };
    assert!(!csend.discount(&ivar));
    assert!(!csend.discount(&ivar));
}
