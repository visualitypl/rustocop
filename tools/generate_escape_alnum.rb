#!/usr/bin/env ruby
# frozen_string_literal: true

# Development-only generator for RedundantStringEscape's pinned POSIX alnum
# facts. The executable never transcodes source or invokes Ruby. RuboCop's
# Parser::Source::Buffer transcodes declared legacy encodings to UTF-8 before
# its POSIX regex runs, so Prism's identifier/alnum callback is NOT the oracle.
# Run: ruby tools/generate_escape_alnum.rb > /tmp/escape_alnum.rs
# Then compare with src/source/escape_alnum.rs (cargo fmt formats the result).
# Ruby 3.2.3, Unicode 15.0, as used by the pinned RuboCop 1.92 oracle.
abort 'expected Unicode 15.0' unless RbConfig::CONFIG['UNICODE_VERSION'] == '15.0.0'

# Enumerate the finite production registry rather than grow a reported-example
# whitelist. Canonical lookup in Rust routes every parser alias to these facts.
registry = File.read(File.expand_path('../vendor/ruby-prism-sys/vendor/prism-1.9.0/src/encoding.c', __dir__))
profiles = registry.scan(/\[PM_ENCODING_(\w+)\] = \{\s*\.name = "([^"]+)".*?\.multibyte = (true|false)/m)
profiles.reject! { |_, name, _| name == 'UTF-8' || name.start_with?('UTF8') || %w[CESU-8 ASCII-8BIT US-ASCII].include?(name) }
def emit_ranges(name, ranges)
  entries = ranges.map { |first, last| format('(0x%x, 0x%x)', first, last) }
  line = "const #{name}: &[(u32, u32)] = &[#{entries.join(', ')}];"
  if line.length <= 100
    puts "\n#{line}"
  else
    puts "\nconst #{name}: &[(u32, u32)] = &["
    entries.each { |entry| puts "    #{entry}," }
    puts '];'
  end
end
names = profiles.map { |_, name, _| name }
identifiers = profiles.to_h { |id, name, _| [name, id] }
multibyte = profiles.to_h { |_, name, multi| [name, multi == 'true'] }
puts <<~RUST
  //! Generated Unicode 15.0 POSIX Alphabetic/Decimal_Number escape facts.
  //! See tools/generate_escape_alnum.rb. UNICODE holds scalar ranges; legacy
  //! tables hold packed source character bytes, NOT Prism identifier predicates.
  //! Undefined mappings are excluded. No codec or runtime Ruby dependency.

  pub(super) fn unicode(code: u32) -> bool {
      let i = UNICODE.partition_point(|&(_, end)| end < code);
      UNICODE.get(i).is_some_and(|&(start, _)| start <= code)
  }

  pub(super) fn ranges(name: &[u8]) -> &'static [(u32, u32)] {
      match name {
RUST
names.each { |name| puts "        b\"#{name}\" => #{identifiers.fetch(name)}," }
# All Unicode source profiles are audited below for first-scalar projection
# changes, not assumed to be byte/Unicode-class-identical mobile codecs.
unicode_profiles = %w[UTF8-MAC UTF8-DoCoMo UTF8-KDDI UTF8-SoftBank]
unicode_profiles.each { |name| puts "        b\"#{name}\" => #{name.upcase.tr('-', '_')}," }
puts "        _ => &[],\n    }\n}\n"
# Scalar facts use the SAME pinned POSIX predicate as legacy projections, not
# the Rust regex crate's newer Unicode version. Surrogates are not scalars.
unicode_ranges = []
(0..0x10ffff).each do |code|
  next if (0xd800..0xdfff).cover?(code)
  next unless /\A[[:alnum:]]\z/.match?([code].pack('U'))

  if unicode_ranges.last && unicode_ranges.last[1] == code - 1
    unicode_ranges.last[1] = code
  else
    unicode_ranges << [code, code]
  end
end
puts "\nconst UNICODE: &[(u32, u32)] = &["
unicode_ranges.each { |first, last| puts format('    (0x%x, 0x%x),', first, last) }
puts '];'
unicode_profiles.each do |name|
  ranges = []
  (0..0x10ffff).each do |code|
    next if (0xd800..0xdfff).cover?(code)
    raw = [code].pack('U')
    begin
      projected = raw.dup.force_encoding(name).encode('UTF-8')
    rescue EncodingError
      next
    end
    before = /\A[[:alnum:]]\z/.match?(raw)
    after = /\A[[:alnum:]]\z/.match?(projected[0])
    next if before == after
    abort "unexpected negative Unicode projection: #{name} #{code}" unless after
    packed = raw.bytes.reduce(0) { |v, b| (v << 8) | b }
    if ranges.last && ranges.last[1] == packed - 1
      ranges.last[1] = packed
    else
      ranges << [packed, packed]
    end
  end
  emit_ranges(name.upcase.tr('-', '_'), ranges)
end
names.each do |name|
  encoding = Encoding.find(name)
  ranges = []
  classify = lambda do |bytes|
    s = bytes.pack('C*').force_encoding(encoding)
    return unless s.valid_encoding? && s.length == 1

    begin
      unicode = s.encode('UTF-8')
    rescue EncodingError, NotImplementedError
      return
    end
    # The pin tests escaped[0] after source-buffer projection. A legacy source
    # character can project to multiple scalars (JIS2004 accents/mobile emoji).
    return unless /\A[[:alnum:]]\z/.match?(unicode[0])

    code = bytes.reduce(0) { |value, byte| (value << 8) | byte }
    if ranges.last && ranges.last[1] == code - 1
      ranges.last[1] = code
    else
      ranges << [code, code]
    end
  end
  (128..255).each { |a| classify.call([a]) }
  if multibyte.fetch(name)
    (128..255).each { |a| (0..255).each { |b| classify.call([a, b]) } }
  end
  if %w[EUC-JP CP51932 eucJP-ms EUC-JIS-2004].include?(name)
    (0xa1..0xfe).each { |a| (0xa1..0xfe).each { |b| classify.call([0x8f, a, b]) } }
  elsif name.start_with?('stateless-ISO-2022-JP')
    # These are Emacs-Mule-shaped characters: 0x92 introduces JIS0208,
    # 0x93 introduces JIS0212. Other supported high-byte heads are enumerated
    # too; undefined mappings are individually absent, never guessed as EUC-JP.
    (0x90..0x9b).each do |head|
      (0xa0..0xff).each { |a| (0xa0..0xff).each { |b| classify.call([head, a, b]) } }
    end
    # Complete the parser's four-byte forms as well. The current stateless
    # reference codecs reject these, but do not assume that from examples.
    [[0x9c, 0xf0..0xf4], [0x9d, 0xf5..0xfe]].each do |head, planes|
      planes.each do |plane|
        (0xa0..0xff).each { |a| (0xa0..0xff).each { |b| classify.call([head, plane, a, b]) } }
      end
    end
  elsif name == 'GB18030'
    # The four-byte space includes BMP and supplementary letters/decimals as
    # well as symbols. Enumerate only its bounded character form, in packed
    # byte order so ranges stay sorted across the two- and four-byte spaces.
    (0x81..0xfe).each do |a|
      (0x30..0x39).each do |b|
        (0x81..0xfe).each do |c|
          (0x30..0x39).each { |d| classify.call([a, b, c, d]) }
        end
      end
    end
  end
  emit_ranges(identifiers.fetch(name), ranges)
end
