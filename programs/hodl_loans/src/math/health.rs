use anchor_lang::prelude::*;

use crate::constants::BPS;
use crate::math::checked::{add, mul_div_floor};
use crate::math::price::{token_value, token_value_ceil, UsdPrice};

/// One collateral holding with its price and risk settings.
#[derive(Clone, Copy, Debug)]
pub struct CollateralValue {
    pub amount: u64,
    pub decimals: u8,
    pub price: UsdPrice,
    pub ltv_bps: u16,
    pub liquidation_threshold_bps: u16,
}

/// Spec §8 health values, all at `USD_SCALE`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Health {
    pub own_value: u128,
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
) -> Result<Health> {
    let mut health = Health::default();
    for c in collateral {
        let value = token_value(c.amount as u128, c.decimals, c.price.lower())?;
        health.own_value = add(health.own_value, value)?;
        health.borrow_limit = add(health.borrow_limit, mul_div_floor(value, c.ltv_bps as u128, BPS)?)?;
        health.liquidation_line = add(
            health.liquidation_line,
            mul_div_floor(value, c.liquidation_threshold_bps as u128, BPS)?,
        )?;
    }
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
                price: UsdPrice { price: 150 * USD, conf: USD },
                ltv_bps: 7_000,
                liquidation_threshold_bps: 9_000,
            },
            // 500 USDC (6 decimals) at $1.
            CollateralValue {
                amount: 500_000_000,
                decimals: 6,
                price: UsdPrice { price: USD, conf: 0 },
                ltv_bps: 7_000,
                liquidation_threshold_bps: 9_000,
            },
        ];
        // Debt: 1,600,000 cNGN = $1,000.
        let h = compute_health(&collateral, 1_600_000_000_000, 6, ngn()).unwrap();
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
            price: UsdPrice { price: USD, conf: 0 },
            ltv_bps: 7_000,
            liquidation_threshold_bps: 9_000,
        }];
        // $700 of debt exactly at the 70% limit is healthy.
        let at_limit = compute_health(&collateral, 1_120_000_000_000, 6, ngn()).unwrap();
        assert_eq!(at_limit.debt, 700 * USD);
        assert!(at_limit.is_healthy());
        // A 1% spread pushes the same debt over the limit.
        let wide = UsdPrice { price: 625_000_000, conf: 6_250_000 };
        let over = compute_health(&collateral, 1_120_000_000_000, 6, wide).unwrap();
        assert_eq!(over.debt, 707 * USD);
        assert!(!over.is_healthy());
    }

    #[test]
    fn no_collateral_means_any_debt_is_unhealthy() {
        let h = compute_health(&[], 1, 6, ngn()).unwrap();
        assert_eq!(h.borrow_limit, 0);
        assert!(!h.is_healthy());
        assert!(h.is_liquidatable());
        assert!(compute_health(&[], 0, 6, ngn()).unwrap().is_healthy());
    }
}
