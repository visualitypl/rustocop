//! `Metrics/AbcSize` — methods whose Assignment Branch Condition size
//! exceeds 17.
//!
//! A port of RuboCop's `AbcSizeCalculator` at the pinned defaults (`Max:
//! 17`, `CountRepeatedAttributes: true`, so repeated attribute calls are
//! not discounted, and no allowed method names or patterns). Over the
//! method body's parser nodes, children first:
//!
//! - **Assignments**: every `lvasgn` whose name does not start with `_`,
//!   `ivasgn`, `cvasgn`, `gvasgn` and `casgn`, every setter call (`a.b = 1`,
//!   `a[i] = 1`), `for`, every argument with such a name, and, for a
//!   multiple or shorthand assignment, each attribute/index target or
//!   method-dispatch child that is not a setter.
//! - **Branches**: every `send`, `csend` and `yield` except comparisons.
//! - **Conditions**: every comparison call, `&.` call except a repeated one
//!   on the same untouched local variable, `if` (twice with an `else`
//!   keyword), `while`/`until`, `for`, `rescue` construct, `when`, `in`,
//!   `&&`, `||`, `||=`, `&&=`, and every block or block-pass on a known
//!   iterating method.
//!
//! The size `sqrt(A² + B² + C²)` is rounded to two decimals as Ruby's
//! `Float#round(2)` and printed with `%.4g`. The whole method is reported;
//! there is no correction.

use ruby_prism::Node;

use super::method_complexity::{Compound, Event, Method, Order, RepeatedCsend, events};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The default `Max`.
const MAX: usize = 17;

/// The `Metrics/AbcSize` cop.
pub struct AbcSize {
    /// The value of `Max`.
    max: usize,
}

impl Default for AbcSize {
    fn default() -> Self {
        AbcSize { max: MAX }
    }
}

impl Cop for AbcSize {
    fn name(&self) -> &'static str {
        "Metrics/AbcSize"
    }

    fn max_setting(&self) -> Option<&'static str> {
        Some("max")
    }

    fn with_max(&self, max: usize) -> Option<Box<dyn Cop>> {
        Some(Box::new(AbcSize { max }))
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let max = self.max as f64;
        let Some(method) = Method::of(node) else {
            return;
        };
        let abc = Abc::of(&method.body);
        let size = abc.size();
        if size > max {
            let message = format!(
                "Assignment Branch Condition size for `{}` is too high. [{} {}/{}]",
                method.name,
                abc.vector(),
                format_g4(size),
                format_g4(max)
            );
            method.report(self, message, file, out);
        }
    }
}

/// An ABC vector.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Abc {
    assignments: u64,
    branches: u64,
    conditions: u64,
}

impl Abc {
    /// `AbcSizeCalculator.calculate(root)`, without the repeated attribute
    /// discount.
    pub fn of(root: &Node<'_>) -> Self {
        let mut abc = Self::default();
        let mut csend = RepeatedCsend::default();
        events(root, Order::Post, |event| abc.count(&event, &mut csend));
        abc
    }

    /// `calculate_node`.
    fn count(&mut self, event: &Event<'_, '_>, csend: &mut RepeatedCsend) {
        match *event {
            Event::Compound { kind, dispatches } => {
                self.assignments += dispatches as u64;
                if matches!(kind, Compound::Or | Compound::And) {
                    self.conditions += 1;
                }
            }
            Event::Lvasgn(name) => {
                csend.reset(name);
                self.assignments += u64::from(!name.starts_with(b"_"));
            }
            Event::Asgn | Event::CapturingArgument => self.assignments += 1,
            Event::Send(send) => {
                self.assignments += u64::from(send.setter);
                if send.comparison {
                    self.conditions += 1;
                } else {
                    self.branches += 1;
                    if send.csend && !csend.discount(&send) {
                        self.conditions += 1;
                    }
                }
            }
            Event::Yield => self.branches += 1,
            Event::For => {
                self.assignments += 1;
                self.conditions += 1;
            }
            Event::If { else_keyword, .. } => self.conditions += 1 + u64::from(else_keyword),
            Event::Loop
            | Event::IteratingBlock
            | Event::Rescue
            | Event::When
            | Event::In
            | Event::And
            | Event::Or => self.conditions += 1,
            Event::Case(_) | Event::CaseMatch(_) => {}
        }
    }

    /// `<A, B, C>`.
    pub fn vector(&self) -> String {
        format!(
            "<{}, {}, {}>",
            self.assignments, self.branches, self.conditions
        )
    }

    /// `Math.sqrt(A² + B² + C²).round(2)`.
    pub fn size(&self) -> f64 {
        let squares = [self.assignments, self.branches, self.conditions]
            .iter()
            .map(|n| n * n)
            .sum::<u64>();
        ruby_round2((squares as f64).sqrt())
    }
}

/// Ruby's `Float#round(2)` (`round_half_up` with the scale 100) of a
/// non-negative size. Sizes never reach the magnitudes where Ruby returns
/// the float unchanged (2^59) or rounds through a rational.
fn ruby_round2(x: f64) -> f64 {
    if x == 0.0 {
        return 0.0;
    }
    let s = 100.0;
    let mut f = (x * s).round();
    if (f + 0.5) / s <= x {
        f += 1.0;
    }
    f / s
}

/// Ruby's `'%.4g' % x` for a positive size, as Ruby's patched `BSD__dtoa`
/// (mode 2, four digits) produces it:
///
/// - digits beyond the fourth are rounded off (more than half up, less
///   down), trailing zeros removed;
/// - a near-tie (the rest within about 10⁻¹² of half) rounds up after an
///   odd fourth digit; after an even one it keeps the four digits, and
///   keeps their trailing zeros when the exact value lies above the tie or
///   is an integer (`130.05` prints `130.0`, `120.05` prints `120`,
///   `110.45` prints `110.4`, `10005.0` prints `1.000e+04`);
/// - the exponent form is used from 10⁴ on.
pub(super) fn format_g4(x: f64) -> String {
    const P: usize = 4;
    if x <= 0.0 || !x.is_finite() {
        return "0".into();
    }
    // The exact binary value in decimal (a size has at most 52 fraction
    // bits, so 80 places are exact).
    let exact = format!("{x:.80}");
    let (int, frac) = exact.split_once('.').unwrap_or((&exact, ""));
    let all: Vec<u8> = int.bytes().chain(frac.bytes()).map(|b| b - b'0').collect();
    let first = all.iter().position(|&d| d != 0).unwrap_or(0);
    let mut decpt = int.len() as i32 - first as i32;
    let significant = &all[first..];
    let mut kept: Vec<u8> = (0..P)
        .map(|i| significant.get(i).copied().unwrap_or(0))
        .collect();
    let rest = significant.get(P..).unwrap_or(&[]);
    let half = compare_half(rest);
    let near = rest.starts_with(&[4, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9])
        || rest.starts_with(&[5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let odd = kept[P - 1] % 2 == 1;
    if rest.iter().all(|&d| d == 0) {
        strip_zeros(&mut kept);
    } else if near && !odd {
        if !(half.is_gt() || x.fract() == 0.0) {
            strip_zeros(&mut kept);
        }
    } else if near || half.is_gt() {
        bump_up(&mut kept, &mut decpt);
    } else {
        strip_zeros(&mut kept);
    }
    render_g(&kept, decpt)
}

/// How `rest` (the digits after the kept ones) compares with one half.
fn compare_half(rest: &[u8]) -> std::cmp::Ordering {
    match rest.first() {
        None => std::cmp::Ordering::Less,
        Some(&d) if d != 5 => d.cmp(&5),
        Some(_) if rest[1..].iter().any(|&d| d != 0) => std::cmp::Ordering::Greater,
        Some(_) => std::cmp::Ordering::Equal,
    }
}

fn strip_zeros(digits: &mut Vec<u8>) {
    while digits.len() > 1 && digits.last() == Some(&0) {
        digits.pop();
    }
}

/// `bump_up`: drop trailing nines, increment; all nines become `1` one
/// place up.
fn bump_up(digits: &mut Vec<u8>, decpt: &mut i32) {
    while digits.last() == Some(&9) {
        digits.pop();
    }
    match digits.last_mut() {
        Some(last) => *last += 1,
        None => {
            digits.push(1);
            *decpt += 1;
        }
    }
}

/// BSD `vfprintf`'s `%g` output of `digits` with the decimal point after
/// `decpt` of them.
fn render_g(digits: &[u8], decpt: i32) -> String {
    let text: String = digits.iter().map(|d| char::from(b'0' + d)).collect();
    if decpt <= -4 || decpt > 4 {
        let exponent = decpt - 1;
        let sign = if exponent < 0 { '-' } else { '+' };
        let mantissa = if text.len() > 1 {
            format!("{}.{}", &text[..1], &text[1..])
        } else {
            text
        };
        format!("{mantissa}e{sign}{:02}", exponent.abs())
    } else if decpt <= 0 {
        format!(
            "0.{}{text}",
            "0".repeat(usize::try_from(-decpt).unwrap_or(0))
        )
    } else {
        let point = usize::try_from(decpt).unwrap_or(0);
        if point >= text.len() {
            format!("{text}{}", "0".repeat(point - text.len()))
        } else {
            format!("{}.{}", &text[..point], &text[point..])
        }
    }
}

#[cfg(test)]
mod tests;
