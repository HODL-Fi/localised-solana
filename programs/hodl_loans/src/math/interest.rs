use anchor_lang::prelude::*;

use crate::constants::{BPS, YEAR};
use crate::math::checked::mul_div_floor;

/// Lender interest accrued over `elapsed_seconds` for the market's `lp_rate_product`
/// (Σ principal × rate_bps × (BPS − reserve_factor_bps)). Rounds down.
pub fn lp_interest(lp_rate_product: u128, elapsed_seconds: u64) -> Result<u128> {
    mul_div_floor(lp_rate_product, elapsed_seconds as u128, BPS * BPS * YEAR)
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
    fn rounds_down_and_is_zero_without_loans() {
        assert_eq!(lp_interest(0, 31_536_000).unwrap(), 0);
        // One second of 1 unit at 1 bps with no reserve is far below one base unit.
        assert_eq!(lp_interest(10_000, 1).unwrap(), 0);
    }
}
