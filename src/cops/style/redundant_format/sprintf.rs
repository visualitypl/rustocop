//! Ruby's `Kernel#format` for the literal arguments `Style/RedundantFormat`
//! corrects: `sprintf.c` for `%s %d %i %u %f` and templates, `flo_to_s`,
//! and `BSD__dtoa` mode 3 (with Ruby's `half` change) for `%f` of a Float.

use crate::cops::ast::float_to_s;

/// An argument as upstream's `argument_value` computes it.
pub(super) enum Value {
    Str(Vec<u8>),
    Sym(Vec<u8>),
    Int(i128),
    /// An Integer past `i128`: its sign and decimal digits.
    Big(bool, String),
    Float(f64),
    /// Reduced, with a positive denominator.
    Rational(i128, i128),
    Complex(Box<Value>, Box<Value>),
    Nil,
    /// Symbol keys in source order; a later key wins.
    Hash(Vec<(Vec<u8>, Value)>),
}

// Output past this size is not rendered (upstream would build it);
// such calls keep their offense without a correction.
const MAX_OUTPUT: i64 = 1 << 20;

#[derive(Default, Clone, Copy)]
struct Flags {
    space: bool,
    sharp: bool,
    plus: bool,
    minus: bool,
    zero: bool,
    prec: bool,
}

/// `format(fmt, *args)`, or `None` where Ruby raises or a value is out of
/// this port's range.
pub(super) fn format(fmt: &[u8], args: &[Value]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut next = 0;
    while i < fmt.len() {
        if fmt[i] != b'%' {
            out.push(fmt[i]);
            i += 1;
            continue;
        }
        i += 1;
        let mut flags = Flags::default();
        let mut width: i64 = -1;
        let mut prec: i64 = -1;
        let mut posarg: Option<usize> = None;
        let mut value: Option<&Value> = None;
        let mut started = false;
        loop {
            let c = *fmt.get(i)?;
            match c {
                b' ' => flags.space = true,
                b'#' => flags.sharp = true,
                b'+' => flags.plus = true,
                b'-' => flags.minus = true,
                b'0' => flags.zero = true,
                b'1'..=b'9' => {
                    let n = number(fmt, &mut i)?;
                    if fmt.get(i) == Some(&b'$') {
                        if posarg.is_some() || value.is_some() {
                            return None;
                        }
                        posarg = Some(n);
                    } else {
                        if width >= 0 {
                            return None;
                        }
                        width = i64::try_from(n).ok()?;
                        started = true;
                        continue;
                    }
                }
                b'<' | b'{' => {
                    let term = if c == b'<' { b'>' } else { b'}' };
                    let len = fmt[i + 1..].iter().position(|&b| b == term)?;
                    let name = &fmt[i + 1..i + 1 + len];
                    let [Value::Hash(hash)] = args else {
                        return None;
                    };
                    let found = hash.iter().rev().find(|(key, _)| key == name)?;
                    value = Some(&found.1);
                    i += len + 2;
                    if term == b'}' {
                        format_s(&mut out, value?, flags, width, prec)?;
                        break;
                    }
                    continue;
                }
                b'*' => {
                    i += 1;
                    let arg = star(fmt, &mut i, args, &mut next)?;
                    let mut w = num_to_int(arg)?;
                    if w < 0 {
                        flags.minus = true;
                        w = -w;
                    }
                    width = w;
                    started = true;
                    continue;
                }
                b'.' => {
                    flags.prec = true;
                    i += 1;
                    if fmt.get(i) == Some(&b'*') {
                        i += 1;
                        prec = num_to_int(star(fmt, &mut i, args, &mut next)?)?;
                        if prec < 0 {
                            flags.prec = false;
                        }
                    } else {
                        prec = if fmt.get(i).is_some_and(u8::is_ascii_digit) {
                            i64::try_from(number(fmt, &mut i)?).ok()?
                        } else {
                            0
                        };
                    }
                    continue;
                }
                b'%' if !started && posarg.is_none() && value.is_none() => {
                    out.push(b'%');
                    i += 1;
                    break;
                }
                b's' | b'd' | b'i' | b'u' | b'f' => {
                    let arg = match (value, posarg) {
                        (Some(v), _) => v,
                        (None, Some(n)) => args.get(n.checked_sub(1)?)?,
                        (None, None) => {
                            next += 1;
                            args.get(next - 1)?
                        }
                    };
                    if width > MAX_OUTPUT || prec > MAX_OUTPUT {
                        return None;
                    }
                    match c {
                        b's' => format_s(&mut out, arg, flags, width, prec)?,
                        b'f' => format_f(&mut out, arg, flags, width, prec)?,
                        _ => format_d(&mut out, arg, flags, width, prec)?,
                    }
                    i += 1;
                    break;
                }
                _ => return None,
            }
            i += 1;
        }
        if out.len() as i64 > MAX_OUTPUT {
            return None;
        }
    }
    Some(out)
}

fn number(fmt: &[u8], i: &mut usize) -> Option<usize> {
    let mut n = 0usize;
    while let Some(d) = fmt.get(*i).filter(|b| b.is_ascii_digit()) {
        n = n.checked_mul(10)?.checked_add(usize::from(d - b'0'))?;
        *i += 1;
    }
    Some(n)
}

/// `GETASTER`: `*n$` reads argument n, `*` the next one.
fn star<'a>(fmt: &[u8], i: &mut usize, args: &'a [Value], next: &mut usize) -> Option<&'a Value> {
    let start = *i;
    let n = number(fmt, i)?;
    if *i > start && fmt.get(*i) == Some(&b'$') {
        *i += 1;
        args.get(n.checked_sub(1)?)
    } else {
        *i = start;
        *next += 1;
        args.get(*next - 1)
    }
}

fn pad(out: &mut Vec<u8>, byte: u8, count: i64) {
    if count > 0 {
        out.extend(std::iter::repeat_n(byte, count as usize));
    }
}

fn format_s(out: &mut Vec<u8>, arg: &Value, flags: Flags, width: i64, prec: i64) -> Option<()> {
    let s = to_s(arg)?;
    let s = String::from_utf8(s).ok()?;
    let mut chars = s.chars().count() as i64;
    let s = if flags.prec && prec < chars {
        chars = prec;
        s.chars().take(prec as usize).collect()
    } else {
        s
    };
    if !flags.minus {
        pad(out, b' ', width - chars);
    }
    out.extend_from_slice(s.as_bytes());
    if flags.minus {
        pad(out, b' ', width - chars);
    }
    Some(())
}

fn format_d(
    out: &mut Vec<u8>,
    arg: &Value,
    flags: Flags,
    mut width: i64,
    mut prec: i64,
) -> Option<()> {
    let (negative, digits) = match arg {
        Value::Big(negative, digits) => (*negative, digits.clone()),
        _ => {
            let v = to_int(arg)?;
            (v < 0, v.unsigned_abs().to_string())
        }
    };
    let sign = if negative {
        Some(b'-')
    } else if flags.plus {
        Some(b'+')
    } else if flags.space {
        Some(b' ')
    } else {
        None
    };
    if sign.is_some() {
        width -= 1;
    }
    let mut len = digits.len() as i64;
    if flags.zero && !flags.minus && !flags.prec {
        prec = width;
        width = 0;
    } else {
        if prec < len {
            if prec == 0 && digits == "0" {
                len = 0;
            }
            prec = len;
        }
        width -= prec;
    }
    if !flags.minus {
        pad(out, b' ', width);
    }
    out.extend(sign);
    pad(out, b'0', prec - len);
    out.extend_from_slice(&digits.as_bytes()[..len as usize]);
    if flags.minus {
        pad(out, b' ', width);
    }
    Some(())
}

fn format_f(out: &mut Vec<u8>, arg: &Value, flags: Flags, width: i64, prec: i64) -> Option<()> {
    let prec = if flags.prec { prec } else { 6 };
    match *arg {
        Value::Int(n) => exact_f(out, n, 1, flags, width, prec),
        Value::Big(negative, ref digits) => {
            let mut full = digits.clone().into_bytes();
            pad(&mut full, b'0', prec);
            exact_digits(out, negative, full, flags, width, prec)
        }
        Value::Rational(n, d) => exact_f(out, n, d, flags, width, prec),
        _ => float_f(out, to_f(arg)?, flags, width, prec),
    }
}

/// sprintf.c's `%f` of an Integer or Rational: exact, rounding half up.
fn exact_f(
    out: &mut Vec<u8>,
    num: i128,
    den: i128,
    flags: Flags,
    width: i64,
    prec: i64,
) -> Option<()> {
    let negative = num < 0;
    let num = num.unsigned_abs();
    let den = den.unsigned_abs();
    let mut full = (num / den).to_string().into_bytes();
    let mut rem = num % den;
    for _ in 0..prec {
        rem = rem.checked_mul(10)?;
        full.push(b'0' + (rem / den) as u8);
        rem %= den;
    }
    if rem.checked_mul(2)? >= den {
        increment(&mut full);
    }
    exact_digits(out, negative, full, flags, width, prec)
}

/// The output half of sprintf.c's exact `%f`: `full` holds the integer
/// digits and `prec` fraction digits.
fn exact_digits(
    out: &mut Vec<u8>,
    negative: bool,
    full: Vec<u8>,
    flags: Flags,
    width: i64,
    prec: i64,
) -> Option<()> {
    let full: Vec<u8> = {
        let start = full.iter().position(|&b| b != b'0').unwrap_or(full.len());
        full[start.min(full.len().saturating_sub(prec as usize + 1))..].to_vec()
    };
    let digits = full.len() as i64;
    let mut len = digits.max(prec + 1);
    let sign = if negative {
        Some(b'-')
    } else if flags.plus {
        Some(b'+')
    } else if flags.space {
        Some(b' ')
    } else {
        None
    };
    len += i64::from(sign.is_some()) + i64::from(prec > 0);
    let fill = (width - len).max(0);
    if !flags.minus && !flags.zero {
        pad(out, b' ', fill);
    }
    out.extend(sign);
    if !flags.minus && flags.zero {
        pad(out, b'0', fill);
    }
    if digits > prec {
        out.extend_from_slice(&full[..(digits - prec) as usize]);
    } else {
        out.push(b'0');
    }
    if prec > 0 {
        out.push(b'.');
        pad(out, b'0', prec - digits);
        out.extend_from_slice(&full[(digits - prec).max(0) as usize..]);
    }
    if flags.minus {
        pad(out, b' ', fill);
    }
    Some(())
}

fn increment(digits: &mut Vec<u8>) {
    for d in digits.iter_mut().rev() {
        if *d == b'9' {
            *d = b'0';
        } else {
            *d += 1;
            return;
        }
    }
    digits.insert(0, b'1');
}

/// `%f` of a Float through BSD_vfprintf.
fn float_f(out: &mut Vec<u8>, f: f64, flags: Flags, width: i64, prec: i64) -> Option<()> {
    let negative = f.is_sign_negative() && !f.is_nan();
    let sign = if negative {
        Some(b'-')
    } else if flags.plus {
        Some(b'+')
    } else if flags.space {
        Some(b' ')
    } else {
        None
    };
    if !f.is_finite() {
        let text: &[u8] = if f.is_nan() { b"NaN" } else { b"Inf" };
        let need = text.len() as i64 + i64::from(sign.is_some());
        if !flags.minus {
            pad(out, b' ', width - need);
        }
        out.extend(sign);
        out.extend_from_slice(text);
        if flags.minus {
            pad(out, b' ', width - need);
        }
        return Some(());
    }
    let (digits, decpt) = if f == 0.0 {
        (b"0".to_vec(), 1)
    } else {
        dtoa_fixed(f.abs(), prec)
    };
    let mut body = Vec::new();
    if decpt > 0 {
        let d = decpt as usize;
        body.extend_from_slice(&digits[..d.min(digits.len())]);
        pad(&mut body, b'0', d as i64 - digits.len() as i64);
    } else {
        body.push(b'0');
    }
    if prec > 0 || flags.sharp {
        body.push(b'.');
    }
    let mut frac = Vec::new();
    if decpt < 0 {
        pad(&mut frac, b'0', -decpt);
    }
    if (decpt.max(0) as usize) < digits.len() {
        frac.extend_from_slice(&digits[decpt.max(0) as usize..]);
    }
    frac.truncate(prec as usize);
    let missing = prec - frac.len() as i64;
    body.extend_from_slice(&frac);
    pad(&mut body, b'0', missing);
    let fill = width - body.len() as i64 - i64::from(sign.is_some());
    if !flags.minus && !flags.zero {
        pad(out, b' ', fill);
    }
    out.extend(sign);
    if !flags.minus && flags.zero {
        pad(out, b'0', fill);
    }
    out.extend_from_slice(&body);
    if flags.minus {
        pad(out, b' ', fill);
    }
    Some(())
}

const TENS: [f64; 23] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
    1e17, 1e18, 1e19, 1e20, 1e21, 1e22,
];
const BIGTENS: [f64; 5] = [1e16, 1e32, 1e64, 1e128, 1e256];

/// `BSD__dtoa(d, 3, ndigits)` for a positive finite `d`: the digits (no
/// trailing zeros) and the decimal point position.
// dtoa's own log10 estimate constants, not std's.
#[allow(clippy::approx_constant)]
fn dtoa_fixed(d: f64, ndigits: i64) -> (Vec<u8>, i64) {
    let bits = d.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i64;
    if exponent != 0 {
        let d2 = f64::from_bits((bits & ((1 << 52) - 1)) | (1023 << 52));
        let ds = (d2 - 1.5) * 0.289529654602168
            + 0.1760912590558
            + (exponent - 1023) as f64 * 0.301029995663981;
        let mut k = ds as i64;
        if ds < 0.0 && ds != k as f64 {
            k -= 1;
        }
        let mut k_check = true;
        if (0..=22).contains(&k) {
            if d < TENS[k as usize] {
                k -= 1;
            }
            k_check = false;
        }
        let ilim = ndigits + k + 1;
        if (0..=14).contains(&ilim) {
            match quick(d, k, k_check, ilim, ndigits) {
                Quick::Done(digits, decpt) => return (digits, decpt),
                Quick::Failed { half } => return exact(d, ndigits, half),
            }
        }
    }
    exact(d, ndigits, false)
}

enum Quick {
    Done(Vec<u8>, i64),
    Failed { half: bool },
}

/// dtoa's "Try to get by with floating-point arithmetic".
fn quick(d: f64, mut k: i64, k_check: bool, mut ilim: i64, ndigits: i64) -> Quick {
    let mut u = d;
    let mut ieps = 2;
    if k > 0 {
        let mut ds = TENS[(k & 0xf) as usize];
        let mut j = k >> 4;
        if j & 0x10 != 0 {
            j &= 0xf;
            u /= BIGTENS[4];
            ieps += 1;
        }
        let mut i = 0;
        while j != 0 {
            if j & 1 != 0 {
                ieps += 1;
                ds *= BIGTENS[i];
            }
            j >>= 1;
            i += 1;
        }
        u /= ds;
    } else if k < 0 {
        let j1 = -k;
        u *= TENS[(j1 & 0xf) as usize];
        let mut j = j1 >> 4;
        let mut i = 0;
        while j != 0 {
            if j & 1 != 0 {
                ieps += 1;
                u *= BIGTENS[i];
            }
            j >>= 1;
            i += 1;
        }
    }
    if k_check && u < 1.0 && ilim > 0 {
        if ilim - 1 <= 0 {
            return Quick::Failed { half: false };
        }
        ilim -= 1;
        k -= 1;
        u *= 10.0;
        ieps += 1;
    }
    let mut eps = (f64::from(ieps) * u + 7.0) * 2f64.powi(-52);
    if ilim == 0 {
        u -= 5.0;
        if u > eps {
            return Quick::Done(b"1".to_vec(), k + 2);
        }
        if u < -eps {
            return Quick::Done(Vec::new(), -ndigits);
        }
        return Quick::Failed { half: false };
    }
    eps *= TENS[(ilim - 1) as usize];
    let mut digits = Vec::new();
    let mut i = 1;
    loop {
        let l = u as i64;
        u -= l as f64;
        if u == 0.0 {
            ilim = i;
        }
        digits.push(b'0' + l as u8);
        if i == ilim {
            if u > 0.5 + eps || (u >= 0.5 - eps && l & 1 == 1) {
                // bump_up
                while digits.last() == Some(&b'9') {
                    digits.pop();
                }
                match digits.last_mut() {
                    Some(last) => *last += 1,
                    None => {
                        k += 1;
                        digits.push(b'1');
                    }
                }
                return Quick::Done(digits, k + 1);
            }
            if u < 0.5 - eps {
                while digits.last() == Some(&b'0') {
                    digits.pop();
                }
                return Quick::Done(digits, k + 1);
            }
            return Quick::Failed { half: true };
        }
        i += 1;
        u *= 10.0;
    }
}

/// dtoa's bignum path for mode 3, on the exact decimal expansion.
fn exact(d: f64, ndigits: i64, half: bool) -> (Vec<u8>, i64) {
    let text = format!("{d:.1100}");
    let (int, frac) = text.split_once('.').unwrap_or((&text, ""));
    let all: Vec<u8> = int.bytes().chain(frac.bytes()).collect();
    let lead = all.iter().position(|&b| b != b'0').unwrap_or(all.len());
    let decpt = int.len() as i64 - lead as i64;
    let mut sig = all[lead..].to_vec();
    while sig.last() == Some(&b'0') {
        sig.pop();
    }
    let ilim = ndigits + decpt;
    if ilim < 0 {
        return (Vec::new(), -ndigits);
    }
    if ilim == 0 {
        // b <= 5S: no digits.
        let above =
            sig.first().is_some_and(|&b| b > b'5') || (sig.first() == Some(&b'5') && sig.len() > 1);
        return if above {
            (b"1".to_vec(), decpt + 1)
        } else {
            (Vec::new(), -ndigits)
        };
    }
    let ilim = ilim as usize;
    if sig.len() <= ilim {
        return (sig, decpt);
    }
    let rest = &sig[ilim..];
    let mut digits = sig[..ilim].to_vec();
    let j = match rest[0].cmp(&b'5') {
        std::cmp::Ordering::Equal if rest.len() > 1 => std::cmp::Ordering::Greater,
        other => other,
    };
    let odd = digits.last().is_some_and(|d| d & 1 == 1);
    if j.is_gt() || (j.is_eq() && odd) {
        while digits.last() == Some(&b'9') {
            digits.pop();
        }
        match digits.last_mut() {
            None => return (b"1".to_vec(), decpt + 1),
            Some(last) => {
                if !half || *last & 1 == 1 {
                    *last += 1;
                }
            }
        }
    } else {
        while digits.last() == Some(&b'0') {
            digits.pop();
        }
    }
    (digits, decpt)
}

fn to_s(value: &Value) -> Option<Vec<u8>> {
    Some(match value {
        Value::Str(s) | Value::Sym(s) => s.clone(),
        Value::Int(n) => n.to_string().into_bytes(),
        Value::Big(negative, digits) => {
            format!("{}{digits}", if *negative { "-" } else { "" }).into_bytes()
        }
        Value::Float(f) => float_to_s(*f).into_bytes(),
        Value::Rational(n, d) => format!("{n}/{d}").into_bytes(),
        Value::Complex(re, im) => {
            let negative = match **im {
                Value::Int(n) | Value::Rational(n, _) => n < 0,
                Value::Float(f) => f.is_sign_negative(),
                _ => return None,
            };
            let abs = match **im {
                Value::Int(n) => Value::Int(n.abs()),
                Value::Rational(n, d) => Value::Rational(n.abs(), d),
                Value::Float(f) => Value::Float(f.abs()),
                _ => return None,
            };
            let finite = !matches!(**im, Value::Float(f) if !f.is_finite());
            let mut s = to_s(re)?;
            s.push(if negative { b'-' } else { b'+' });
            s.extend(to_s(&abs)?);
            if !finite {
                s.push(b'*');
            }
            s.push(b'i');
            s
        }
        Value::Nil => Vec::new(),
        Value::Hash(_) => return None,
    })
}

/// Complex#to_i / #to_f need an exact zero imaginary part.
fn real_part(re: &Value, im: &Value) -> Option<Value> {
    let exact_zero = match im {
        Value::Int(0) => true,
        Value::Rational(n, _) => *n == 0,
        _ => false,
    };
    exact_zero.then_some(match re {
        Value::Int(n) => Value::Int(*n),
        Value::Float(f) => Value::Float(*f),
        Value::Rational(n, d) => Value::Rational(*n, *d),
        _ => Value::Nil,
    })
}

fn float_to_int(f: f64) -> Option<i128> {
    (f.is_finite() && f.abs() < 1e38).then(|| f.trunc() as i128)
}

/// sprintf.c's `%d` conversion.
fn to_int(value: &Value) -> Option<i128> {
    match value {
        Value::Int(n) => Some(*n),
        Value::Float(f) => float_to_int(*f),
        Value::Str(s) => str_to_inum(s),
        Value::Rational(n, d) => Some(n / d),
        Value::Complex(re, im) => to_int(&real_part(re, im)?),
        _ => None,
    }
}

/// `NUM2INT`, for `*` widths and precisions.
fn num_to_int(value: &Value) -> Option<i64> {
    let n = match value {
        Value::Int(n) => *n,
        Value::Float(f) => float_to_int(*f)?,
        Value::Rational(n, d) => n / d,
        Value::Complex(re, im) => return num_to_int(&real_part(re, im)?),
        _ => return None,
    };
    i64::try_from(n).ok().filter(|n| i32::try_from(*n).is_ok())
}

/// `rb_Float`.
fn to_f(value: &Value) -> Option<f64> {
    match value {
        Value::Float(f) => Some(*f),
        Value::Int(n) => Some(*n as f64),
        Value::Rational(n, d) => Some(*n as f64 / *d as f64),
        Value::Str(s) => str_to_f(s),
        Value::Complex(re, im) => to_f(&real_part(re, im)?),
        _ => None,
    }
}

fn trim(s: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(s).ok()?;
    Some(
        s.trim_matches(|c: char| c.is_ascii_whitespace() || c == '\x0b')
            .replace('_', ""),
    )
}

/// `rb_str_to_inum(str, 0, TRUE)` (the cop has checked the grammar).
fn str_to_inum(s: &[u8]) -> Option<i128> {
    let s = trim(s)?;
    let (negative, unsigned) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, &s[..]),
    };
    let lower = unsigned.to_ascii_lowercase();
    let (digits, radix) = if let Some(d) = lower.strip_prefix("0x") {
        (d.to_string(), 16)
    } else if let Some(d) = lower.strip_prefix("0b") {
        (d.to_string(), 2)
    } else if let Some(d) = lower.strip_prefix("0o") {
        (d.to_string(), 8)
    } else if let Some(d) = lower.strip_prefix("0d") {
        (d.to_string(), 10)
    } else if lower.len() > 1 && lower.starts_with('0') {
        (lower[1..].to_string(), 8)
    } else {
        (lower, 10)
    };
    let n = i128::from_str_radix(&digits, radix).ok()?;
    Some(if negative { -n } else { n })
}

/// `Float(str)` (the cop has checked the grammar).
fn str_to_f(s: &[u8]) -> Option<f64> {
    let s = trim(s)?;
    let (negative, unsigned) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, &s[..]),
    };
    let value = if let Some(hex) = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
    {
        let (mantissa, exp) = match hex.find(['p', 'P']) {
            Some(i) => (&hex[..i], hex[i + 1..].parse::<i32>().ok()?),
            None => (hex, 0),
        };
        let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        let m = u128::from_str_radix(&format!("{int}{frac}"), 16).ok()?;
        m as f64 * 2f64.powi(exp - 4 * frac.len() as i32)
    } else {
        unsigned.parse::<f64>().ok()?
    };
    Some(if negative { -value } else { value })
}

/// `String#to_r`: the leading rational, ignoring the rest.
pub(super) fn str_to_r(s: &[u8]) -> Option<(i128, i128)> {
    let mut i = 0;
    while s.get(i).is_some_and(|b| b.is_ascii_whitespace()) {
        i += 1;
    }
    let negative = match s.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let digits = |i: &mut usize| -> String {
        let mut out = String::new();
        while let Some(&b) = s.get(*i) {
            if b.is_ascii_digit() {
                out.push(char::from(b));
            } else if b == b'_' && !out.is_empty() && s.get(*i + 1).is_some_and(u8::is_ascii_digit)
            {
            } else {
                break;
            }
            *i += 1;
        }
        out
    };
    let int = digits(&mut i);
    if int.is_empty() {
        return Some((0, 1));
    }
    let mut frac = String::new();
    if s.get(i) == Some(&b'.') && s.get(i + 1).is_some_and(u8::is_ascii_digit) {
        i += 1;
        frac = digits(&mut i);
    }
    let mut exp = 0i32;
    if matches!(s.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        let sign = match s.get(j) {
            Some(b'-') => {
                j += 1;
                -1
            }
            Some(b'+') => {
                j += 1;
                1
            }
            _ => 1,
        };
        let e = digits(&mut j);
        if !e.is_empty() {
            exp = sign * e.parse::<i32>().ok()?;
            i = j;
        }
    }
    let mut num: i128 = format!("{int}{frac}").parse().ok()?;
    let mut den: i128 = 10i128.checked_pow(frac.len() as u32)?;
    if exp > 0 {
        num = num.checked_mul(10i128.checked_pow(exp as u32)?)?;
    } else if exp < 0 {
        den = den.checked_mul(10i128.checked_pow(exp.unsigned_abs())?)?;
    }
    if s.get(i) == Some(&b'/') {
        let mut j = i + 1;
        let d = digits(&mut j);
        if !d.is_empty() {
            den = den.checked_mul(d.parse().ok()?)?;
        }
    }
    if den == 0 {
        return None;
    }
    let g = gcd(num, den);
    let (num, den) = (num / g, den / g);
    Some((if negative { -num } else { num }, den))
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(fmt: &str, args: Vec<Value>) -> String {
        String::from_utf8(format(fmt.as_bytes(), &args).unwrap()).unwrap()
    }

    #[test]
    fn matches_ruby_32_output() {
        // Each expectation printed by Ruby 3.2.3.
        use Value::*;
        assert_eq!(
            f(
                "%05s|%-5s|%.2s|%5.1s",
                vec![
                    Str(b"ab".into()),
                    Str(b"ab".into()),
                    Str(b"abc".into()),
                    Str(b"xyz".into())
                ]
            ),
            "   ab|ab   |ab|    x"
        );
        assert_eq!(
            f(
                "%+05d|% d|%.3d|%05.3d|%.0d|%-5d|%+.0d",
                vec![Int(5), Int(5), Int(10), Int(7), Int(0), Int(-3), Int(0)]
            ),
            "+0005| 5|010|  007||-3   |+"
        );
        assert_eq!(
            f(
                "%d|%d|%d|%d",
                vec![Float(5.9), Float(-5.9), Rational(3, 8), Rational(-7, 2)]
            ),
            "5|-5|0|-3"
        );
        assert_eq!(
            f(
                "%f|%.0f|%.0f|%.0f|%.1f|%#.0f|%+f|% f|%010.2f|%-10.2f|",
                vec![
                    Int(5),
                    Float(0.5),
                    Float(1.5),
                    Float(2.5),
                    Float(0.25),
                    Int(3),
                    Float(1.5),
                    Float(1.5),
                    Float(-3.14259),
                    Int(2)
                ]
            ),
            "5.000000|0|2|2|0.2|3|+1.500000| 1.500000|-000003.14|2.00      |"
        );
        assert_eq!(
            f(
                "%.0f|%.0f|%.1f|%.2f",
                vec![
                    Rational(1, 2),
                    Rational(3, 2),
                    Rational(1, 4),
                    Rational(-1, 8)
                ]
            ),
            "1|2|0.3|-0.13"
        );
        assert_eq!(
            f(
                "%f|%10f|%010f",
                vec![
                    Float(f64::INFINITY),
                    Float(f64::NEG_INFINITY),
                    Float(f64::INFINITY)
                ]
            ),
            "Inf|      -Inf|       Inf"
        );
        let near_ties = [
            (0.35, 1, "0.4"),
            (0.45, 1, "0.4"),
            (1.005, 2, "1.00"),
            (1.115, 2, "1.12"),
            (0.125, 2, "0.12"),
            (0.375, 2, "0.38"),
            (2.675, 2, "2.68"),
            (1.0049999999999, 2, "1.00"),
            (0.15, 1, "0.2"),
            (0.1499999, 1, "0.1"),
            (0.149999999999999, 1, "0.1"),
            (0.14999999999999997, 1, "0.2"),
            (1.45, 1, "1.4"),
            (3.5, 0, "4"),
            (1234.5678, 3, "1234.568"),
            (0.000125, 5, "0.00012"),
            (8.345, 2, "8.34"),
            (8.335, 2, "8.34"),
            (2486.2157170254527, 10, "2486.2157170255"),
            (5.0e-09, 8, "0.00000001"),
            (10936810612419.467, 0, "10936810612419"),
            (-0.0598848441433965, 15, "-0.059884844143396"),
            (7.657264619700745e-05, 18, "0.000076572646197008"),
            (-0.0, 2, "-0.00"),
            (-0.04, 1, "-0.0"),
        ];
        for (value, prec, want) in near_ties {
            assert_eq!(
                f(&format!("%.{prec}f"), vec![Float(value)]),
                want,
                "{value} {prec}"
            );
        }
        assert_eq!(
            [
                1e16,
                1e15,
                1e14,
                0.0001,
                0.00001,
                1.5e-7,
                123456789.123,
                1e100,
                -0.0,
                100.0,
                1234567890123456.0
            ]
            .map(float_to_s),
            [
                "1.0e+16",
                "1.0e+15",
                "100000000000000.0",
                "0.0001",
                "1.0e-05",
                "1.5e-07",
                "123456789.123",
                "1.0e+100",
                "-0.0",
                "100.0",
                "1.234567890123456e+15"
            ]
        );
        assert_eq!(
            f(
                "%d|%d|%d|%f|%f",
                vec![
                    Str(b"0x1A".into()),
                    Str(b" 12 ".into()),
                    Str(b"1_000".into()),
                    Str(b"0x1.8p1".into()),
                    Str(b"1_0.5".into())
                ]
            ),
            "26|12|1000|3.000000|10.500000"
        );
        assert_eq!(
            f(
                "%*d|%-*d|%*d",
                vec![Int(5), Int(14), Int(5), Int(14), Int(-5), Int(14)]
            ),
            "   14|14   |14   "
        );
        let hash = || Hash(vec![(b"a".to_vec(), Str(b"x".into()))]);
        assert_eq!(f("%{a}|%-6{a}|%6{a}", vec![hash()]), "x|x     |     x");
        assert_eq!(
            f("%.3s|%6s|", vec![Str("żółw".into()), Str("żó".into())]),
            "żół|    żó|"
        );
        assert_eq!(
            f(
                "%3$s %1$s %2$s",
                vec![Str(b"a".into()), Str(b"b".into()), Str(b"c".into())]
            ),
            "c a b"
        );
        assert_eq!(
            f("%-+5d|%0-5d|%+ d", vec![Int(3), Int(3), Int(3)]),
            "+3   |3    |+3"
        );
        assert_eq!(str_to_r(b"1.5r"), Some((3, 2)));
        assert_eq!(str_to_r(b"3/8r"), Some((3, 8)));
        assert_eq!(str_to_r(b"0x10r"), Some((0, 1)));
    }
}
