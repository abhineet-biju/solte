use anyhow::{Result, bail};

pub const LAMPORTS_PER_SOL: u64 = 1_000_000_000;

pub fn parse_sol(value: &str) -> Result<u64> {
    let value = value.trim();
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 9
    {
        bail!("Enter a positive SOL amount with at most 9 decimal places");
    }
    let whole: u64 = whole.parse()?;
    let fraction: u64 = format!("{fraction:0<9}").parse()?;
    let amount = whole
        .checked_mul(LAMPORTS_PER_SOL)
        .and_then(|v| v.checked_add(fraction))
        .ok_or_else(|| anyhow::anyhow!("Amount is too large"))?;
    if amount == 0 {
        bail!("Amount must be greater than zero");
    }
    Ok(amount)
}

pub fn format_sol(lamports: u64) -> String {
    let whole = lamports / LAMPORTS_PER_SOL;
    let fraction = format!("{:09}", lamports % LAMPORTS_PER_SOL);
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        format!("{whole}.0")
    } else {
        format!("{whole}.{fraction}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_are_exact_and_reject_rounding() {
        assert_eq!(parse_sol("0.000000001").unwrap(), 1);
        assert_eq!(parse_sol("123.456789123").unwrap(), 123_456_789_123);
        for invalid in ["0", "-1", "NaN", "1e9", "1.0000000001", "1.2.3", ""] {
            assert!(parse_sol(invalid).is_err(), "{invalid}");
        }
        assert!(parse_sol("18446744074").is_err());
    }

    #[test]
    fn formatting_preserves_lamports() {
        for value in [1, 10, 1_000_000_000, 123_456_789_123, u64::MAX] {
            assert_eq!(parse_sol(&format_sol(value)).unwrap(), value);
        }
    }
}
