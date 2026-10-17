//! Deterministic number formatting for writers.

/// Decimal places used for lengths and angles in output files.
pub const DECIMALS: usize = 6;

/// `value` rounded to `decimals` places, without trailing zeros but always with one digit after
/// the point, and never as negative zero: `1.5`, `2.0`, `0.0`.
pub fn fixed(value: f64, decimals: usize) -> String {
    let mut s = format!("{value:.decimals$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.push('0');
        }
    } else if value.is_finite() {
        s.push_str(".0");
    }
    if s == "-0.0" { "0.0".to_string() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(fixed(1.5, 6), "1.5");
        assert_eq!(fixed(2.0, 6), "2.0");
        assert_eq!(fixed(0.0, 6), "0.0");
        assert_eq!(fixed(-0.0, 6), "0.0");
        assert_eq!(fixed(-1e-9, 6), "0.0");
        assert_eq!(fixed(-2.6e-6, 6), "-0.000003");
        assert_eq!(fixed(123.4567894, 6), "123.456789");
        assert_eq!(fixed(1e-7, 6), "0.0");
        assert_eq!(fixed(1234.0, 0), "1234.0");
        assert_eq!(fixed(0.1 + 0.2, 6), "0.3");
    }
}
