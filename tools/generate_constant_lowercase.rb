#!/usr/bin/env ruby
# frozen_string_literal: true

# Development-only POSIX lowercase facts for CaseEquality's module-name test.
# --uppercase-change instead emits InvertibleUnlessCondition's upcase predicate.
# --uppercase emits Naming/BlockParameterName's POSIX uppercase predicate.
# Like the pinned Parser::Source::Buffer, classify the UTF-8 projection, not
# Prism's identifier predicate. Runtime is table-only; no codec or Ruby needed.
# ruby tools/generate_constant_lowercase.rb > /tmp/constant_lowercase.rs
# ruby tools/generate_constant_lowercase.rb --uppercase-change > /tmp/constant_uppercase_change.rs
uppercase_change = ARGV.delete('--uppercase-change')
uppercase = ARGV.delete('--uppercase')
abort 'unexpected arguments' unless ARGV.empty? && !(uppercase && uppercase_change)
classify_unicode = if uppercase
                     ->(s) { /[[:upper:]]/.match?(s) }
                   elsif uppercase_change
                     ->(s) { s.upcase != s }
                   else
                     ->(s) { /[[:lower:]]/.match?(s) }
                   end
abort 'expected Unicode 15.0' unless RbConfig::CONFIG['UNICODE_VERSION'] == '15.0.0'
source = File.read(File.expand_path('../vendor/ruby-prism-sys/vendor/prism-1.9.0/src/encoding.c', __dir__))
names = source.scan(/\.name = "([^"]+)"/).flatten & Encoding.list.map(&:name)
names -= %w[UTF-8 US-ASCII ASCII-8BIT CESU-8 UTF8-MAC UTF8-DoCoMo UTF8-KDDI UTF8-SoftBank]
puts <<~RUST
  //! Generated #{uppercase ? 'POSIX uppercase' : uppercase_change ? 'uppercase-change' : 'POSIX lowercase'} facts for #{uppercase ? 'Naming identifiers' : 'constant identifiers'}.
  //! See tools/generate_constant_lowercase.rb#{uppercase ? ' --uppercase' : uppercase_change ? ' --uppercase-change' : ''} (Ruby Unicode 15.0 / oracle pin).
  //! Legacy ranges pack source bytes after the pinned buffer's UTF-8 projection.
  //! Unicode ranges use scalar values, independent of Rust's Unicode version.
  pub(super) fn unicode(code: u32) -> bool {
      let i = UNICODE.partition_point(|&(_, end)| end < code);
      UNICODE.get(i).is_some_and(|&(start, _)| start <= code)
  }
  pub(super) fn ranges(name: &[u8]) -> &'static [(u32,u32)] { match name {
RUST
names.each_with_index { |n,i| puts "b\"#{n}\" => E#{i}," }
puts '_ => &[], } }'
unicode_ranges = []
(0..0x10ffff).each do |code|
  next if (0xd800..0xdfff).cover?(code)
  next unless classify_unicode.call([code].pack('U'))
  if unicode_ranges.last && unicode_ranges.last.last == code - 1
    unicode_ranges.last[1] = code
  else
    unicode_ranges << [code, code]
  end
end
puts 'const UNICODE: &[(u32,u32)] = &['
unicode_ranges.each { |a,b| puts "(0x#{a.to_s(16)},0x#{b.to_s(16)})," }
puts '];'
names.each_with_index do |name,index|
  codes=[]
  classify=lambda do |bytes|
    s=bytes.pack('C*').force_encoding(name)
    next unless s.valid_encoding? && s.length==1
    begin
      s=s.encode('UTF-8')
    rescue EncodingError
      next
    end
    codes << bytes.reduce(0) { |v,b| (v<<8)|b } if classify_unicode.call(s)
  end
  (128..255).each { |a| classify.call([a]) }
  (128..255).each { |a| (0..255).each { |b| classify.call([a,b]) } }
  if %w[EUC-JP eucJP-ms CP51932 EUC-JIS-2004].include?(name)
    (0xa1..0xfe).each { |a| (0xa1..0xfe).each { |b| classify.call([0x8f,a,b]) } }
  elsif name=='GB18030'
    (0x81..0xfe).each { |a| (0x30..0x39).each { |b| (0x81..0xfe).each { |c| (0x30..0x39).each { |d| classify.call([a,b,c,d]) } } } }
  elsif name=='EUC-TW'
    (0xa1..0xb0).each { |p| (0xa1..0xfe).each { |a| (0xa1..0xfe).each { |b| classify.call([0x8e,p,a,b]) } } }
  end
  ranges=[]
  codes.sort.uniq.each do |code|
    if ranges.last && ranges.last.last==code-1 then ranges.last[1]=code else ranges << [code,code] end
  end
  puts "const E#{index}: &[(u32,u32)] = &["
  ranges.each { |a,b| puts "(0x#{a.to_s(16)},0x#{b.to_s(16)})," }
  puts '];'
end
