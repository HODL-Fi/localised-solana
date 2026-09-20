use anchor_lang::prelude::*;

use crate::constants::{BPS, MULTIPLIER_SCALE};
use crate::math::checked::{add, mul_div_floor};
use crate::math::price::{token_value, token_value_ceil, UsdPrice};

/// One collateral holding with its price and risk settings.
#[derive(Clone, Copy, Debug)]
pub struct CollateralValue {
    pub amount: u64,
    pub decimals: u8,
    /// The mint's scaled-UI multiplier at `MULTIPLIER_SCALE` (always `MULTIPLIER_ONE` for a
    /// `Standard` asset). Pyth quotes an xStock per display token, so the raw balance is
    /// scaled by it before pricing.
    pub multiplier: u128,
    pub price: UsdPrice,
    pub ltv_bps: u16,
    pub liquidation_threshold_bps: u16,
}

impl CollateralValue {
    /// The balance Pyth's price applies to: raw amount × multiplier, rounded down.
    pub fn display_amount(&self) -> Result<u128> {
        mul_div_floor(self.amount as u128, self.multiplier, MULTIPLIER_SCALE)
    }
}

/// Spec §8 health values, all at `USD_SCALE`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Health {
    pub own_value: u128,
    /// Promo actually counted: the position's promo valued at the NGN bid, capped at
    /// `promo_cap_bps` of the collateral the borrower put up themselves. It lifts both limits
    /// below, and is zero for a position holding no collateral of its own.
    pub promo_counted: u128,
    pub borrow_limit: u128,
    pub liquidation_line: u128,
    pub debt: u128,
}

impl Health {
    pub fn is_healthy(&self) -> bool {
        self.debt <= self.borrow_limit
    }

    pub fn is_liquidatable(&self) -> bool {
        self.debt > self.liquidation_line
    }
}

/// Collateral is valued at `price − conf`; debt (cNGN base units) at `ngn price + spread`, rounded up.
pub fn compute_health(
    collateral: &[CollateralValue],
    debt_cngn: u128,
    cngn_decimals: u8,
    ngn: UsdPrice,
    promo_balance: u64,
    promo_cap_bps: u16,
) -> Result<Health> {
    let mut health = Health::default();
    let mut promo_cap_total: u128 = 0;
    for c in collateral {
        let value = token_value(c.display_amount()?, c.decimals, c.price.lower())?;
        health.own_value = add(health.own_value, value)?;
        health.borrow_limit = add(health.borrow_limit, mul_div_floor(value, c.ltv_bps as u128, BPS)?)?;
        health.liquidation_line = add(
            health.liquidation_line,
            mul_div_floor(value, c.liquidation_threshold_bps as u128, BPS)?,
        )?;
        // Floor the promo cap PER ASSET, inside this same loop, instead of once on the
        // aggregate `own_value` below. `floor(sum(v_i) * bps / BPS)` can exceed
        // `sum(floor(v_i * bps / BPS))` by up to n-1 base units; at the shipped defaults
        // (ltv 7000 + cap 2000 == lt 9000, zero slack) that gap alone was enough to push a
        // maxed-out multi-asset position's `borrow_limit` a hair above `liquidation_line` the
        // moment the cap changed — instant liquidation with no price move. Sum-of-floors ≤
        // floor-of-sum always, so accumulating here is conservative: promo counts for slightly
        // less, never more.
        promo_cap_total = add(promo_cap_total, mul_div_floor(value, promo_cap_bps as u128, BPS)?)?;
    }
    // Spec §12: promo is a topping on collateral the borrower owns, never a substitute for it.
    // The cap is a fraction of `own_value`, so a position with nothing of its own counts none of
    // it — which is what makes defaulting a loss for the borrower rather than a way to profit.
    let promo_value = token_value(promo_balance as u128, cngn_decimals, ngn.lower())?;
    health.promo_counted = promo_value.min(promo_cap_total);
    health.borrow_limit = add(health.borrow_limit, health.promo_counted)?;
    health.liquidation_line = add(health.liquidation_line, health.promo_counted)?;

    health.debt = token_value_ceil(debt_cngn, cngn_decimals, ngn.upper()?)?;
    Ok(health)
}

#[cfg(test)]
mod tests {
    use super::*;

    const USD: u128 = 1_000_000_000_000;

    fn ngn() -> UsdPrice {
        // 1 NGN = $0.000625, no spread: 1,600 NGN per dollar.
        UsdPrice { price: 625_000_000, conf: 0 }
    }

    #[test]
    fn mixed_decimals_sum_into_limits() {
        let collateral = [
            // 10 SOL (9 decimals) at $150 with $1 confidence.
            CollateralValue {
                amount: 10_000_000_000,
                decimals: 9,
                multiplier: MULTIPLIER_SCALE,
                price: UsdPrice { price: 150 * USD, conf: USD },
                ltv_bps: 7_000,
                liquidation_threshold_bps: 9_000,
            },
            // 500 USDC (6 decimals) at $1.
            CollateralValue {
                amount: 500_000_000,
                decimals: 6,
                multiplier: MULTIPLIER_SCALE,
                price: UsdPrice { price: USD, conf: 0 },
                ltv_bps: 7_000,
                liquidation_threshold_bps: 9_000,
            },
        ];
        // Debt: 1,600,000 cNGN = $1,000.
        let h = compute_health(&collateral, 1_600_000_000_000, 6, ngn(), 0, 0).unwrap();
        assert_eq!(h.own_value, 1_990 * USD);
        assert_eq!(h.borrow_limit, 1_393 * USD);
        assert_eq!(h.liquidation_line, 1_791 * USD);
        assert_eq!(h.debt, 1_000 * USD);
        assert!(h.is_healthy());
        assert!(!h.is_liquidatable());
    }

    #[test]
    fn spread_raises_debt_and_boundary_is_inclusive() {
        let collateral = [CollateralValue {
            amount: 1_000_000_000,
            decimals: 6,
            multiplier: MULTIPLIER_SCALE,
            price: UsdPrice { price: USD, conf: 0 },
            ltv_bps: 7_000,
            liquidation_threshold_bps: 9_000,
        }];
        // $700 of debt exactly at the 70% limit is healthy.
        let at_limit = compute_health(&collateral, 1_120_000_000_000, 6, ngn(), 0, 0).unwrap();
        assert_eq!(at_limit.debt, 700 * USD);
        assert!(at_limit.is_healthy());
        // A 1% spread pushes the same debt over the limit.
        let wide = UsdPrice { price: 625_000_000, conf: 6_250_000 };
        let over = compute_health(&collateral, 1_120_000_000_000, 6, wide, 0, 0).unwrap();
        assert_eq!(over.debt, 707 * USD);
        assert!(!over.is_healthy());
    }

    #[test]
    fn an_xstock_multiplier_scales_the_balance_before_pricing() {
        // 100 raw AAPLX (8 decimals) at a 1.5 multiplier is 150 display tokens at $200 = $30,000.
        let split = [CollateralValue {
            amount: 10_000_000_000,
            decimals: 8,
            multiplier: 1_500_000_000_000,
            price: UsdPrice { price: 200 * USD, conf: 0 },
            ltv_bps: 5_000,
            liquidation_threshold_bps: 7_500,
        }];
        let h = compute_health(&split, 0, 6, ngn(), 0, 0).unwrap();
        assert_eq!(h.own_value, 30_000 * USD);
        assert_eq!(h.borrow_limit, 15_000 * USD);
        assert_eq!(h.liquidation_line, 22_500 * USD);
        // The same holding at multiplier 1 is worth the raw balance.
        let plain = [CollateralValue { multiplier: MULTIPLIER_SCALE, ..split[0] }];
        assert_eq!(compute_health(&plain, 0, 6, ngn(), 0, 0).unwrap().own_value, 20_000 * USD);
    }

    #[test]
    fn display_amount_rounds_down_not_up() {
        // A live AAPLX-shaped multiplier (token/scaled_ui.rs documents this exact value)
        // applied to a raw balance of 3: floor gives 3 display units, ceil would give 4. An
        // xStock balance must never be valued above what it represents.
        let c = CollateralValue {
            amount: 3,
            decimals: 8,
            multiplier: 1_000_899_999_999,
            price: UsdPrice { price: 200 * USD, conf: 0 },
            ltv_bps: 5_000,
            liquidation_threshold_bps: 7_500,
        };
        assert_eq!(c.display_amount().unwrap(), 3);
    }

    #[test]
    fn no_collateral_means_any_debt_is_unhealthy() {
        let h = compute_health(&[], 1, 6, ngn(), 0, 0).unwrap();
        assert_eq!(h.borrow_limit, 0);
        assert!(!h.is_healthy());
        assert!(h.is_liquidatable());
        assert!(compute_health(&[], 0, 6, ngn(), 0, 0).unwrap().is_healthy());
    }

    // ---- promo_counted (Task 5) — previously exercised only through SVM tests. ----

    #[test]
    fn promo_cap_rounds_down_on_the_boundary() {
        // own_value = 4 (single base units) with a 20% cap: 4 * 2000 / 10000 = 0.8, which
        // floors to 0 but would ceil to 1. Pins the rounding direction so a ceiling swap at
        // the cap computation does not slip through with the other tests still green.
        let tiny = [CollateralValue {
            amount: 4,
            decimals: 0,
            multiplier: MULTIPLIER_SCALE,
            price: UsdPrice { price: 1, conf: 0 },
            ltv_bps: 5_000,
            liquidation_threshold_bps: 7_500,
        }];
        // 1 unit of promo is worth 625 (from `ngn()`'s bid) — comfortably above either
        // candidate cap, so the assertion below isolates the cap's own rounding.
        let h = compute_health(&tiny, 0, 6, ngn(), 1, 2_000).unwrap();
        assert_eq!(h.own_value, 4);
        assert_eq!(h.promo_counted, 0);
    }

    #[test]
    fn promo_above_the_cap_counts_only_the_cap() {
        // $1,000 of collateral at a 20% cap admits at most $200 of promo.
        let collateral = [CollateralValue {
            amount: 1_000_000_000,
            decimals: 6,
            multiplier: MULTIPLIER_SCALE,
            price: UsdPrice { price: USD, conf: 0 },
            ltv_bps: 7_000,
            liquidation_threshold_bps: 9_000,
        }];
        // 500,000 cNGN is worth $312.50 at the bid — well past the $200 cap.
        let h = compute_health(&collateral, 0, 6, ngn(), 500_000_000_000, 2_000).unwrap();
        assert_eq!(h.own_value, 1_000 * USD);
        assert_eq!(h.promo_counted, 200 * USD);
    }

    #[test]
    fn promo_below_the_cap_counts_in_full() {
        // Same $1,000 / 20% cap ($200 ceiling), but only $62.50 of promo offered.
        let collateral = [CollateralValue {
            amount: 1_000_000_000,
            decimals: 6,
            multiplier: MULTIPLIER_SCALE,
            price: UsdPrice { price: USD, conf: 0 },
            ltv_bps: 7_000,
            liquidation_threshold_bps: 9_000,
        }];
        let h = compute_health(&collateral, 0, 6, ngn(), 100_000_000_000, 2_000).unwrap();
        assert_eq!(h.promo_counted, 62_500_000_000_000);
    }

    #[test]
    fn zero_own_collateral_counts_no_promo() {
        let h = compute_health(&[], 0, 6, ngn(), 500_000_000_000, 2_000).unwrap();
        assert_eq!(h.own_value, 0);
        assert_eq!(h.promo_counted, 0);
    }

    #[test]
    fn lowering_the_cap_never_makes_a_maxed_out_multi_asset_position_liquidatable() {
        // Two assets, each valued 10_000_000_000_003 at USD_SCALE (amount == value here: 0
        // decimals, price 1, multiplier 1), at the shipped defaults — 70% LTV / 90% LT, zero
        // slack against a 20% promo cap (7000 + 2000 == 9000). This is the exact boundary that
        // used to break: floor(sum) for the cap disagreed with sum(floor) for the LTV/LT terms.
        let asset = || CollateralValue {
            amount: 10_000_000_000_003,
            decimals: 0,
            multiplier: MULTIPLIER_SCALE,
            price: UsdPrice { price: 1, conf: 0 },
            ltv_bps: 7_000,
            liquidation_threshold_bps: 9_000,
        };
        let collateral = [asset(), asset()];
        // Plenty of promo to saturate the cap either way; 1:1 NGN pricing keeps the numbers
        // exact so the assertions below match the worked example precisely.
        let promo_balance = 5_000_000_000_000u64;
        let ngn_1to1 = UsdPrice { price: 1, conf: 0 };

        // Borrow to exactly the position's limit while the cap is still the default 20%.
        let at_cap = compute_health(&collateral, 0, 0, ngn_1to1, promo_balance, 2_000).unwrap();
        assert_eq!(at_cap.borrow_limit, 18_000_000_000_004);
        let debt = at_cap.borrow_limit;

        // The admin lowers the cap to 0. Re-pricing the same debt against the recomputed
        // liquidation line must not flip the position into liquidatable — the per-asset floor
        // keeps `borrow_limit` and `liquidation_line` moving together instead of leaving a
        // floor-of-sum-vs-sum-of-floors gap for a cap change to fall into.
        let h = compute_health(&collateral, debt, 0, ngn_1to1, promo_balance, 0).unwrap();
        assert_eq!(h.liquidation_line, 18_000_000_000_004);
        assert_eq!(h.debt, debt);
        assert!(!h.is_liquidatable());
    }
}
