use anchor_lang::prelude::*;

use crate::constants::{BPS, MULTIPLIER_SCALE};
use crate::errors::HodlError;
use crate::math::checked::{add, mul_div_floor, pow10, to_u64};

/// How much cNGN a liquidator pays and how much collateral it takes for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seizure {
    pub repay_amount: u64,
    pub seize_amount: u64,
}

/// Everything `seize_for_repayment` needs, named.
///
/// It used to take these as eight positional arguments behind an
/// `#[allow(clippy::too_many_arguments)]`. Two of them — `collateral_price` and `multiplier` —
/// are both `u128` at the same `10^12` scale, so transposing them compiled cleanly and produced
/// a seizure wrong by the multiplier. Named fields make that particular mistake unrepresentable.
pub struct SeizureInputs {
    pub repay_amount: u64,
    pub ngn_price: u128,
    pub cngn_decimals: u8,
    pub collateral_price: u128,
    pub collateral_decimals: u8,
    pub multiplier: u128,
    pub bonus_bps: u16,
    pub slot_amount: u64,
}

/// Spec §11 seizure, at plain prices (no confidence or spread adjustment):
///
/// ```text
/// seize = repay × ngn_price × (BPS + bonus) × 10^collateral_decimals × MULTIPLIER_SCALE
///         / (10^cngn_decimals × collateral_price × BPS × multiplier)
/// ```
///
/// `multiplier` is the asset's scaled-UI factor (`MULTIPLIER_ONE` for a `Standard` asset):
/// Pyth quotes an xStock per display token, so the seizure converts back to raw units. For a
/// `Standard` asset the factor is 1 and the step leaves the value exact.
///
/// **It is not a no-op for a `Standard` asset, though.** The `× MULTIPLIER_SCALE` runs
/// unconditionally, so `display` above ≈3.4 × 10^26 (`u128::MAX / MULTIPLIER_SCALE`) returns
/// `MathOverflow` where the pre-multiplier code fell through to the slot cap below. It fails
/// closed, which is the right direction, but the consequence is specific: such an asset is
/// **unliquidatable** rather than slot-capped. Reaching it needs a collateral price near zero
/// and a repayment near `u64::MAX`, and the same position would already be a write-off
/// candidate — but the failure mode is a revert, not a capped seizure.
///
/// The division is interleaved so a large repayment cannot overflow `u128`, and every step
/// rounds down, so the liquidator never receives more collateral than the formula allows.
/// When the slot holds less than that, the seizure takes the whole slot and the repayment
/// shrinks in proportion (spec §11 step 6).
pub fn seize_for_repayment(inputs: &SeizureInputs) -> Result<Seizure> {
    let &SeizureInputs {
        repay_amount,
        ngn_price,
        cngn_decimals,
        collateral_price,
        collateral_decimals,
        multiplier,
        bonus_bps,
        slot_amount,
    } = inputs;
    require!(collateral_price > 0 && ngn_price > 0, HodlError::InvalidPrice);
    require!(multiplier > 0, HodlError::InvalidPrice);
    let repaid_usd = mul_div_floor(repay_amount as u128, ngn_price, pow10(cngn_decimals as u32)?)?;
    let with_bonus = mul_div_floor(repaid_usd, add(BPS, bonus_bps as u128)?, BPS)?;
    // The price is per display token, so the display amount converts back to raw units.
    let display = mul_div_floor(with_bonus, pow10(collateral_decimals as u32)?, collateral_price)?;
    let seize = mul_div_floor(display, MULTIPLIER_SCALE, multiplier)?;

    if seize <= slot_amount as u128 {
        return Ok(Seizure { repay_amount, seize_amount: to_u64(seize)? });
    }
    // The slot caps the seizure, so the liquidator repays only the share it can cover.
    let capped_repayment = mul_div_floor(repay_amount as u128, slot_amount as u128, seize)?;
    Ok(Seizure { repay_amount: to_u64(capped_repayment)?, seize_amount: slot_amount })
}

/// The principal share of a partial repayment: `repay × principal / balance` (spec §11 step 7),
/// rounded down, so interest is paid first.
pub fn principal_share(repay_amount: u64, principal: u64, balance_total: u128) -> Result<u64> {
    require!(balance_total > 0, HodlError::MathOverflow);
    to_u64(mul_div_floor(repay_amount as u128, principal as u128, balance_total)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{MAX_COLLATERAL_DECIMALS, MULTIPLIER_ONE};

    const USD: u128 = 1_000_000_000_000;
    /// 1 NGN = $0.000625.
    const NGN: u128 = 625_000_000;
    /// 1,600,000 cNGN, worth $1,000.
    const REPAY: u64 = 1_600_000_000_000;

    /// The common case, so each test states only what it is varying.
    fn base() -> SeizureInputs {
        SeizureInputs {
            repay_amount: REPAY,
            ngn_price: NGN,
            cngn_decimals: 6,
            collateral_price: USD,
            collateral_decimals: 6,
            multiplier: MULTIPLIER_ONE,
            bonus_bps: 500,
            slot_amount: u64::MAX,
        }
    }

    /// The worst repayment the program permits — `u64::MAX` cNGN, a 100% bonus, a $0.01
    /// collateral price — at a given `collateral_decimals`. Pins the derivation behind
    /// `MAX_COLLATERAL_DECIMALS` (see `constants.rs`).
    fn worst_case(collateral_decimals: u8) -> SeizureInputs {
        SeizureInputs {
            repay_amount: u64::MAX,
            bonus_bps: 10_000,
            collateral_price: 10_000_000_000, // $0.01 at USD_SCALE
            collateral_decimals,
            ..base()
        }
    }

    #[test]
    fn the_max_collateral_decimals_bound_is_derived_not_asserted() {
        // Nothing else in this suite ever calls `seize_for_repayment` at 12 or 15 decimals — the
        // SVM test only proves the *policy* (13 refused, 12 accepted), not the arithmetic behind
        // it. Without this test, raising `MAX_COLLATERAL_DECIMALS` in constants.rs is free: the
        // rest of the suite stays green even past the cliff. If this test starts failing,
        // re-derive the cliff (see constants.rs's doc comment) — do not just widen the constant
        // to make it pass.
        seize_for_repayment(&worst_case(MAX_COLLATERAL_DECIMALS)).unwrap();
        assert!(seize_for_repayment(&worst_case(15)).is_err());
    }

    #[test]
    fn seizure_pays_the_bonus_on_top_of_the_repaid_value() {
        // 1,050 USDC (6 decimals at $1) for $1,000 of cNGN at a 5% bonus.
        let s = seize_for_repayment(&base()).unwrap();
        assert_eq!(s, Seizure { repay_amount: REPAY, seize_amount: 1_050_000_000 });

        // The same $1,050 is 7 SOL (9 decimals at $150).
        let s = seize_for_repayment(&SeizureInputs { collateral_price: 150 * USD, collateral_decimals: 9, ..base() })
            .unwrap();
        assert_eq!(s.seize_amount, 7_000_000_000);

        // No bonus seizes exactly the repaid value.
        let s = seize_for_repayment(&SeizureInputs { bonus_bps: 0, ..base() }).unwrap();
        assert_eq!(s.seize_amount, 1_000_000_000);
    }

    #[test]
    fn a_full_bonus_still_scales_the_seizure() {
        // A 100% bonus doubles the collateral seized for the same repayment.
        let s = seize_for_repayment(&SeizureInputs { bonus_bps: 10_000, ..base() }).unwrap();
        assert_eq!(s.seize_amount, 2_000_000_000);
    }

    #[test]
    fn a_short_slot_caps_both_the_seizure_and_the_repayment() {
        // The slot holds 500 USDC of the 1,050 the full repayment would take.
        let s = seize_for_repayment(&SeizureInputs { slot_amount: 500_000_000, ..base() }).unwrap();
        assert_eq!(s.seize_amount, 500_000_000);
        // 1,600,000 × 500 / 1,050 cNGN, rounded down.
        assert_eq!(s.repay_amount, 761_904_761_904);
        // Re-pricing the capped repayment seizes no more than the slot.
        let again =
            seize_for_repayment(&SeizureInputs { repay_amount: s.repay_amount, slot_amount: 500_000_000, ..base() })
                .unwrap();
        assert!(again.seize_amount <= 500_000_000);
    }

    #[test]
    fn seizure_rejects_missing_prices() {
        assert!(seize_for_repayment(&SeizureInputs { collateral_price: 0, ..base() }).is_err());
        assert!(seize_for_repayment(&SeizureInputs { ngn_price: 0, ..base() }).is_err());
        assert!(seize_for_repayment(&SeizureInputs { multiplier: 0, ..base() }).is_err());
    }

    #[test]
    fn the_seizure_floors_an_inexact_multiplier() {
        // A live AAPLX-shaped multiplier (token/scaled_ui.rs documents this exact value; and
        // math/health.rs uses it for the same purpose on the pricing side) divides the display
        // amount inexactly. Flooring the raw-unit division is what stops a liquidator from being
        // paid more collateral than the formula allows — a `mul_div_ceil` here would round in
        // the liquidator's favor, at the borrower's expense.
        let s = seize_for_repayment(&SeizureInputs { multiplier: 1_000_899_999_999, ..base() }).unwrap();
        assert_eq!(s.seize_amount, 1_049_055_849);
    }

    #[test]
    fn principal_share_pays_interest_first_and_rounds_down() {
        // A 103,000 cNGN balance on 100,000 of principal.
        let (principal, balance) = (100_000_000_000u64, 103_000_000_000u128);
        assert_eq!(principal_share(103_000_000_000, principal, balance).unwrap(), principal);
        assert_eq!(principal_share(51_500_000_000, principal, balance).unwrap(), 50_000_000_000);
        // Dust repayments pay interest only.
        assert_eq!(principal_share(1, principal, balance).unwrap(), 0);
        assert!(principal_share(1, principal, 0).is_err());
    }
}
