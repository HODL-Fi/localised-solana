use anchor_lang::prelude::*;

use crate::constants::{BPS, MAX_BPS};
use crate::errors::HodlError;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum CollateralKind {
    /// Classic SPL Token or metadata-only Token-2022 mint (SOL, USDC, USDT).
    Standard,
    /// Admin-listed xStock (Plan 4).
    XStock,
}

#[account]
#[derive(InitSpace)]
pub struct CollateralAsset {
    pub version: u8,
    pub bump: u8,
    pub vault_bump: u8,
    pub mint: Pubkey,
    pub token_program: Pubkey,
    pub vault: Pubkey,
    pub decimals: u8,
    pub kind: CollateralKind,
    pub pyth_feed_id: [u8; 32],
    pub max_price_age_seconds: u64,
    pub max_conf_bps: u16,
    pub ltv_bps: u16,
    pub liquidation_threshold_bps: u16,
    pub liquidation_bonus_bps: u16,
    /// Raw token amounts.
    pub deposit_cap: u64,
    pub total_deposited: u64,
    /// Blocks new deposits of this asset only.
    pub paused: bool,
    pub reserved: [u8; 128],
}

/// Admin-settable collateral parameters, used by `list_collateral` and `update_collateral_params`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct CollateralParams {
    pub pyth_feed_id: [u8; 32],
    pub max_price_age_seconds: u64,
    pub max_conf_bps: u16,
    pub ltv_bps: u16,
    pub liquidation_threshold_bps: u16,
    pub liquidation_bonus_bps: u16,
    pub deposit_cap: u64,
}

impl CollateralParams {
    /// Spec §8 collateral rules, checked against the current `Config::promo_cap_bps`.
    pub fn validate(&self, promo_cap_bps: u16) -> Result<()> {
        require!(self.pyth_feed_id != [0u8; 32], HodlError::InvalidParameters);
        require!(self.max_price_age_seconds > 0, HodlError::InvalidParameters);
        require!(self.max_conf_bps <= MAX_BPS, HodlError::InvalidParameters);
        require!(self.ltv_bps >= 1_000, HodlError::InvalidParameters);
        require!(
            self.ltv_bps as u32 + promo_cap_bps as u32 <= self.liquidation_threshold_bps as u32,
            HodlError::InvalidParameters
        );
        require!(
            self.liquidation_threshold_bps as u128 * (BPS + self.liquidation_bonus_bps as u128) <= BPS * BPS,
            HodlError::InvalidParameters
        );
        Ok(())
    }
}

impl CollateralAsset {
    pub fn params(&self) -> CollateralParams {
        CollateralParams {
            pyth_feed_id: self.pyth_feed_id,
            max_price_age_seconds: self.max_price_age_seconds,
            max_conf_bps: self.max_conf_bps,
            ltv_bps: self.ltv_bps,
            liquidation_threshold_bps: self.liquidation_threshold_bps,
            liquidation_bonus_bps: self.liquidation_bonus_bps,
            deposit_cap: self.deposit_cap,
        }
    }

    pub fn apply_params(&mut self, p: &CollateralParams) {
        self.pyth_feed_id = p.pyth_feed_id;
        self.max_price_age_seconds = p.max_price_age_seconds;
        self.max_conf_bps = p.max_conf_bps;
        self.ltv_bps = p.ltv_bps;
        self.liquidation_threshold_bps = p.liquidation_threshold_bps;
        self.liquidation_bonus_bps = p.liquidation_bonus_bps;
        self.deposit_cap = p.deposit_cap;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sol() -> CollateralParams {
        CollateralParams {
            pyth_feed_id: [1; 32],
            max_price_age_seconds: 60,
            max_conf_bps: 200,
            ltv_bps: 7_000,
            liquidation_threshold_bps: 9_000,
            liquidation_bonus_bps: 1_000,
            deposit_cap: u64::MAX,
        }
    }

    #[test]
    fn launch_values_pass_with_twenty_percent_promo_cap() {
        sol().validate(2_000).unwrap();
        CollateralParams { ltv_bps: 5_000, liquidation_threshold_bps: 7_500, ..sol() }
            .validate(2_000)
            .unwrap();
    }

    #[test]
    fn each_rule_is_enforced() {
        let cases = [
            CollateralParams { pyth_feed_id: [0; 32], ..sol() },
            CollateralParams { max_price_age_seconds: 0, ..sol() },
            CollateralParams { max_conf_bps: 10_001, ..sol() },
            CollateralParams { ltv_bps: 999, liquidation_threshold_bps: 2_999, ..sol() },
            // 80% LTV + 20% promo cap exceeds a 90% threshold.
            CollateralParams { ltv_bps: 8_000, ..sol() },
            // 90% threshold × 1.12 bonus exceeds 100%.
            CollateralParams { liquidation_bonus_bps: 1_200, ..sol() },
        ];
        for case in cases {
            assert!(case.validate(2_000).is_err(), "{case:?}");
        }
        // The promo cap is part of the rule: 80% LTV is fine with a 10% cap.
        CollateralParams { ltv_bps: 8_000, ..sol() }.validate(1_000).unwrap();
    }
}
