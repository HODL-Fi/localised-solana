use anchor_lang::prelude::*;

use crate::constants::{BPS, YEAR};
use crate::errors::HodlError;
use crate::math::checked::add;
#[cfg(test)]
use crate::math::checked::mul_div_floor;

/// Lender interest accrued over `elapsed_seconds` for the market's `lp_rate_product`
/// (Σ principal × rate_bps × (BPS − reserve_factor_bps)). Rounds down.
///
/// Test-only. `Market::accrue` uses `accrue_lp_interest`, which carries the division remainder
/// between calls; this is the closed-form version the tests check that one against. Keeping it
/// compiled into the program would ship a second, subtly different interest formula that
/// nothing calls.
#[cfg(test)]
pub fn lp_interest(lp_rate_product: u128, elapsed_seconds: u64) -> Result<u128> {
    mul_div_floor(lp_rate_product, elapsed_seconds as u128, BPS * BPS * YEAR)
}

/// Lender interest for `elapsed_seconds`, carrying the division remainder between calls so
/// frequent accrual loses nothing to rounding. Returns `(interest, new_remainder)`.
pub fn accrue_lp_interest(lp_rate_product: u128, elapsed_seconds: u64, remainder: u128) -> Result<(u128, u128)> {
    let denominator = BPS * BPS * YEAR;
    let numerator = add(
        lp_rate_product.checked_mul(elapsed_seconds as u128).ok_or(HodlError::MathOverflow)?,
        remainder,
    )?;
    Ok((numerator / denominator, numerator % denominator))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_year_at_ten_percent_with_ten_percent_reserve() {
        // 1,000 cNGN (6 decimals) at 10% APR, 10% reserve factor: lenders earn 90 cNGN in a year.
        let principal: u128 = 1_000_000_000;
        let product = principal * 1_000 * 9_000;
        assert_eq!(lp_interest(product, 31_536_000).unwrap(), 90_000_000);
    }

    #[test]
    fn remainder_carries_so_per_second_accrual_matches_one_shot() {
        // 1,000 cNGN at 10% with a 10% reserve accrues about 2.85 base units per second.
        let product = 1_000_000_000u128 * 1_000 * 9_000;
        let (mut total, mut remainder) = (0u128, 0u128);
        for _ in 0..3_600 {
            let (interest, next) = accrue_lp_interest(product, 1, remainder).unwrap();
            total += interest;
            remainder = next;
        }
        let (one_shot, one_shot_remainder) = accrue_lp_interest(product, 3_600, 0).unwrap();
        assert_eq!(total, one_shot);
        assert_eq!(remainder, one_shot_remainder);
        assert_eq!(one_shot, lp_interest(product, 3_600).unwrap());
    }

    #[test]
    fn rounds_down_and_is_zero_without_loans() {
        assert_eq!(lp_interest(0, 31_536_000).unwrap(), 0);
        // One second of 1 unit at 1 bps with no reserve is far below one base unit.
        assert_eq!(lp_interest(10_000, 1).unwrap(), 0);
    }
}
