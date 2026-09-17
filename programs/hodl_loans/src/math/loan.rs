use anchor_lang::prelude::*;

use crate::constants::{BPS, YEAR};
use crate::errors::HodlError;
use crate::math::checked::{add, mul_div_ceil, mul_div_floor, sub};

/// The terms of one fixed-term loan, as stored in its slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoanTerms {
    pub principal: u64,
    pub originated_at: i64,
    pub interest_anchor: i64,
    pub tenure_seconds: i64,
    pub rate_bps: u16,
    pub penalty_rate_bps: u16,
}

/// What a loan owes at a moment in time. All amounts round up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoanBalance {
    pub principal: u128,
    pub interest: u128,
    pub penalty: u128,
}

impl LoanBalance {
    pub fn due(&self) -> Result<u128> {
        add(self.interest, self.penalty)
    }

    pub fn total(&self) -> Result<u128> {
        add(self.principal, self.due()?)
    }
}

fn seconds_between(start: i64, end: i64) -> u128 {
    if end > start {
        (end - start) as u128
    } else {
        0
    }
}

fn mul3(a: u128, b: u128, c: u128) -> Result<u128> {
    Ok(a.checked_mul(b).and_then(|v| v.checked_mul(c)).ok_or(HodlError::MathOverflow)?)
}

/// Spec §10:
/// interest = principal × rate × (min(now, maturity) − anchor)⁺ / (BPS × YEAR)
/// penalty  = (principal + interest) × (rate + penalty_rate) × (now − max(maturity, anchor))⁺ / (BPS × YEAR)
pub fn loan_balance(terms: &LoanTerms, now: i64) -> Result<LoanBalance> {
    let principal = terms.principal as u128;
    let maturity = terms
        .originated_at
        .checked_add(terms.tenure_seconds)
        .ok_or(HodlError::MathOverflow)?;
    let interest_seconds = seconds_between(terms.interest_anchor, now.min(maturity));
    let interest = mul_div_ceil(
        mul3(principal, terms.rate_bps as u128, interest_seconds)?,
        1,
        BPS * YEAR,
    )?;
    let penalty_seconds = seconds_between(maturity.max(terms.interest_anchor), now);
    let penalty_rate = terms.rate_bps as u128 + terms.penalty_rate_bps as u128;
    let penalty = mul_div_ceil(
        mul3(add(principal, interest)?, penalty_rate, penalty_seconds)?,
        1,
        BPS * YEAR,
    )?;
    Ok(LoanBalance { principal, interest, penalty })
}

/// A loan's contribution to `Market::lp_rate_product`.
pub fn lp_contribution(principal: u64, rate_bps: u16, reserve_factor_bps: u16) -> Result<u128> {
    mul3(principal as u128, rate_bps as u128, sub(BPS, reserve_factor_bps as u128)?)
}

/// Lender interest the market has accrued for `principal` of a loan since `interest_anchor`
/// (spec §9 `R`). Rounds down.
pub fn accrued_lp_interest(
    principal: u64,
    rate_bps: u16,
    reserve_factor_bps: u16,
    interest_anchor: i64,
    now: i64,
) -> Result<u128> {
    mul_div_floor(
        lp_contribution(principal, rate_bps, reserve_factor_bps)?,
        seconds_between(interest_anchor, now),
        BPS * BPS * YEAR,
    )
}

/// The protocol's cut of `interest_paid`. Rounds down.
pub fn reserve_share(interest_paid: u128, reserve_factor_bps: u16) -> Result<u128> {
    mul_div_floor(interest_paid, reserve_factor_bps as u128, BPS)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    fn terms() -> LoanTerms {
        // 1,000,000 cNGN (6 decimals), 365-day tenure, 15% APR, 5% penalty.
        LoanTerms {
            principal: 1_000_000_000_000,
            originated_at: 1_000,
            interest_anchor: 1_000,
            tenure_seconds: 365 * DAY,
            rate_bps: 1_500,
            penalty_rate_bps: 500,
        }
    }

    #[test]
    fn before_maturity_charges_simple_interest_only() {
        let t = terms();
        let b = loan_balance(&t, t.originated_at + 73 * DAY).unwrap();
        // 1e12 × 1500 × 73 days / (1e4 × 365 days) = 3e10 exactly.
        assert_eq!(b.interest, 30_000_000_000);
        assert_eq!(b.penalty, 0);
        assert_eq!(b.total().unwrap(), 1_030_000_000_000);
    }

    #[test]
    fn interest_stops_at_maturity_and_penalty_starts() {
        let t = terms();
        let maturity = t.originated_at + t.tenure_seconds;
        let at_maturity = loan_balance(&t, maturity).unwrap();
        assert_eq!(at_maturity.interest, 150_000_000_000);
        assert_eq!(at_maturity.penalty, 0);

        let later = loan_balance(&t, maturity + 73 * DAY).unwrap();
        assert_eq!(later.interest, 150_000_000_000);
        // (1e12 + 1.5e11) × 2000 × 73 days / (1e4 × 365 days) = 4.6e10.
        assert_eq!(later.penalty, 46_000_000_000);
    }

    #[test]
    fn anchor_reset_after_repayment_moves_both_clocks() {
        let mut t = terms();
        let maturity = t.originated_at + t.tenure_seconds;
        // Repaid in full-interest terms 10 days after maturity.
        t.interest_anchor = maturity + 10 * DAY;
        let b = loan_balance(&t, maturity + 10 * DAY).unwrap();
        assert_eq!((b.interest, b.penalty), (0, 0));
        let b = loan_balance(&t, maturity + 83 * DAY).unwrap();
        assert_eq!(b.interest, 0);
        assert_eq!(b.penalty, 40_000_000_000);
    }

    #[test]
    fn balance_rounds_up() {
        let t = LoanTerms { principal: 1, ..terms() };
        let b = loan_balance(&t, t.originated_at + 1).unwrap();
        assert_eq!(b.interest, 1);
    }

    #[test]
    fn lender_interest_matches_contribution_and_rounds_down() {
        let t = terms();
        let r = accrued_lp_interest(t.principal, t.rate_bps, 1_000, t.interest_anchor, t.interest_anchor + 73 * DAY)
            .unwrap();
        // Borrower interest 3e10 × 90% to lenders.
        assert_eq!(r, 27_000_000_000);
        assert_eq!(lp_contribution(t.principal, t.rate_bps, 1_000).unwrap(), 1_000_000_000_000u128 * 1_500 * 9_000);
        assert_eq!(accrued_lp_interest(1, 1, 0, 0, 1).unwrap(), 0);
        assert_eq!(reserve_share(30_000_000_000, 1_000).unwrap(), 3_000_000_000);
        assert_eq!(reserve_share(9, 1_000).unwrap(), 0);
    }

    #[test]
    fn lp_contribution_rejects_reserve_factor_exceeding_bps() {
        assert!(lp_contribution(1, 1, 10_001).is_err());
    }
}
