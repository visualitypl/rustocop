#!/usr/bin/env ruby
# frozen_string_literal: true
# Development only; no runtime Ruby/codec dependency. WordArray's fixed regex
# runs on RuboCop's UTF-8 source-buffer projection, with Unicode 15.0 at the pin.
# Generate: ruby tools/generate_word_array_classes.rb > /tmp/word_array_classes.rs
abort 'expected Unicode 15.0' unless RbConfig::CONFIG['UNICODE_VERSION'] == '15.0.0'
def ranges(codes)
  result = []
  codes.each do |code|
    if result.last && result.last[1] == code - 1
      result.last[1] = code
    else
      result << [code, code]
    end
  end
  result
end
def emit(name, codes)
  puts "const #{name}: &[(u32, u32)] = &["
  ranges(codes).each { |a, b| puts format('    (0x%x, 0x%x),', a, b) }
  puts '];'
end
puts <<~RUST
  //! Generated Ruby Unicode-15 Word and legacy source-buffer projection facts.
  //! See tools/generate_word_array_classes.rb. Classification, not a codec;
  //! packed source character bytes for legacy encodings, Unicode scalar values
  //! for UTF-8. No runtime Ruby, transcoding, or Unicode-version approximation.
  pub(super) fn contains(ranges: &[(u32, u32)], code: u32) -> bool {
      let i = ranges.partition_point(|&(_, end)| end < code);
      ranges.get(i).is_some_and(|&(start, _)| start <= code)
  }
  pub(super) fn unicode(code: u32) -> bool { contains(UNICODE, code) }
RUST
emit('UNICODE', (0..0x10ffff).reject { |c| (0xd800..0xdfff).cover?(c) }.select { |c| /\A\p{Word}\z/.match?([c].pack('U')) })
# UTF8-MAC's buffer conversion can collapse two Word scalars to one. That
# matters between hyphens (a-e+acute-c must not gain an extra Word unit).
# Keep only source prefixes that become ONE Word scalar, not a general codec.
# Hangul's regular L/V/T compositions are handled algorithmically below.
mac = []
(0..0x10ffff).each do |code|
  next if (0xd800..0xdfff).cover?(code) || (0xac00..0xd7a3).cover?(code)
  text = [code].pack('U')
  next unless /\A\p{Word}\z/.match?(text)
  raw = text.encode('UTF8-MAC').force_encoding('UTF-8')
  chars = raw.chars
  next unless chars.size > 1
  (1...chars.size).each do |length|
    prefix = chars.take(length).join.force_encoding('UTF8-MAC').encode('UTF-8')
    candidate = prefix + chars.drop(length).join
    next unless candidate.length > 1
    decoded = candidate.dup.force_encoding('UTF8-MAC').encode('UTF-8')
    mac << candidate.b if /\A\p{Word}\z/.match?(decoded)
  end
end
mac = mac.sort.uniq
puts <<~RUST
  fn first_scalar(bytes: &[u8]) -> Option<(char, usize)> {
      let first = *bytes.first()?;
      let width = match first { 0..=0x7f => 1, 0xc2..=0xdf => 2, 0xe0..=0xef => 3, 0xf0..=0xf4 => 4, _ => return None };
      Some((std::str::from_utf8(bytes.get(..width)?).ok()?.chars().next()?, width))
  }
  /// Raw UTF8-MAC source prefix reduced to one Word unit by the pinned buffer.
  /// No suffix decoding/scanning: Hangul plus a bounded prefix-table query.
  pub(super) fn mac_word_prefix(bytes: &[u8]) -> usize {
      if let Some((first, a)) = first_scalar(bytes) && let Some((second, b)) = first_scalar(&bytes[a..]) {
              if ('\\u{1100}'..='\\u{1112}').contains(&first) && ('\\u{1161}'..='\\u{1175}').contains(&second) {
                  let t = first_scalar(&bytes[a+b..]).filter(|(c,_)| ('\\u{11a8}'..='\\u{11c2}').contains(c)).map_or(0,|(_,n)|n);
                  return a+b+t;
              }
              if ('\\u{ac00}'..='\\u{d7a3}').contains(&first) && (first as u32 - 0xac00).is_multiple_of(28) && ('\\u{11a8}'..='\\u{11c2}').contains(&second) { return a+b; }
      }
      for n in (3..=bytes.len().min(#{mac.map(&:bytesize).max})).rev() {
          if MAC.binary_search(&&bytes[..n]).is_ok() { return n; }
      }
      0
  }
  const MAC: &[&[u8]] = &[
RUST
mac.each { |raw| puts '    b"' + raw.bytes.map { |b| format('\\x%02x', b) }.join + '",' }
puts '];'
source = File.read(File.expand_path('../vendor/ruby-prism-sys/vendor/prism-1.9.0/src/encoding.c', __dir__))
names = source.scan(/\.name = "([^"]+)"/).flatten & Encoding.list.map(&:name)
names -= %w[UTF-8 US-ASCII ASCII-8BIT CESU-8 UTF8-MAC UTF8-DoCoMo UTF8-KDDI UTF8-SoftBank]
puts 'pub(super) fn legacy(name: &[u8]) -> &\'static [(u32, u32)] { match name {'
names.each_with_index { |n, i| puts "b\"#{n}\" => E#{i}," }
puts '_ => &[], } }'
expansions = Hash.new { |h,k| h[k]=[] }
names.each_with_index do |name, index|
  encoding = Encoding.find(name)
  codes = []
  classify = lambda do |bytes|
    s = bytes.pack('C*').force_encoding(encoding)
    next unless s.valid_encoding? && s.length == 1
    begin
      s = s.encode('UTF-8')
    rescue Encoding::UndefinedConversionError, Encoding::ConverterNotFoundError, Encoding::InvalidByteSequenceError
      next
    end
    expansions[name] << [bytes,s.length] if s.length>1 && /\A\p{Word}+\z/.match?(s)
    codes << bytes.reduce(0) { |v, b| (v << 8) | b } if /\A\p{Word}\z/.match?(s)
  end
  (128..255).each { |a| classify.call([a]) }
  # Encoding has no public max_char_size on the pinned host. Enumerate the
  # bounded two-byte space for every encoding; single-byte pairs are rejected.
  (128..255).each { |a| (0..255).each { |b| classify.call([a, b]) } }
  if ['EUC-JP', 'eucJP-ms', 'CP51932', 'EUC-JIS-2004'].include?(name)
    (0xa1..0xfe).each { |a| (0xa1..0xfe).each { |b| classify.call([0x8f, a, b]) } }
  elsif name == 'GB18030'
    (0x81..0xfe).each { |a| (0x30..0x39).each { |b| (0x81..0xfe).each { |c| (0x30..0x39).each { |d| classify.call([a,b,c,d]) } } } }
  elsif name == 'EUC-TW'
    (0xa1..0xb0).each { |p| (0xa1..0xfe).each { |a| (0xa1..0xfe).each { |b| classify.call([0x8e,p,a,b]) } } }
  end
  emit("E#{index}", codes.sort.uniq)
end
# Mobile UTF-8 codecs expand private-use keycaps to digit + combining mark.
# Other Unicode scalars keep their Word class; non-Word emoji stay non-Word.
%w[UTF8-DoCoMo UTF8-KDDI UTF8-SoftBank].each do |name|
  (0xe000..0xf8ff).each do |code|
    bytes=[code].pack('U').bytes
    begin
      decoded=bytes.pack('C*').force_encoding(name).encode('UTF-8')
    rescue EncodingError
      next
    end
    expansions[name] << [bytes,decoded.length] if decoded.length>1 && /\A\p{Word}+\z/.match?(decoded)
  end
end
puts 'pub(super) fn expansions(name: &[u8]) -> &\'static [(&\'static [u8], usize)] { match name {'
expansions.each do |name, entries|
  puts "b\"#{name}\" => &["
  entries.sort.uniq.each { |bytes,count| puts '    (b"' + bytes.map { |b| format('\\x%02x', b) }.join + '", ' + count.to_s + '),' }
  puts '],'
end
puts '_ => &[], } }'
