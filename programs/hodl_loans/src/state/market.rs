use anchor_lang::prelude::*;

use crate::constants::{MAX_BAD_DEBT_DUST_USD, MAX_BPS, MIN_TENURE};
use crate::errors::HodlError;
use crate::math::checked::{add, sub};
use crate::math::interest::accrue_lp_interest;

#[account]
#[derive(InitSpace)]
pub struct Market {
    pub version: u8,
    pub bump: u8,
    pub vault_bump: u8,
    pub mint: Pubkey,
    pub token_program: Pubkey,
    pub vault: Pubkey,
    pub decimals: u8,
    /// cNGN the program has recorded as held. Direct transfers to the vault are ignored.
    pub cash: u64,
    /// Outstanding loan principal.
    pub total_borrows: u64,
    /// Σ over active loans of principal × rate_bps × (BPS − reserve_factor_bps).
    pub lp_rate_product: u128,
    /// Lender interest accrued but not yet paid, already net of reserve.
    pub accrued_interest: u128,
    pub protocol_reserve: u64,
    pub total_bad_debt: u128,
    pub total_shares: u128,
    pub last_accrual_ts: i64,
    pub interest_rate_bps: u16,
    pub penalty_rate_bps: u16,
    pub reserve_factor_bps: u16,
    pub max_utilization_bps: u16,
    pub min_loan_amount: u64,
    pub max_tenure_seconds: i64,
    pub bad_debt_dust_usd: u128,
    pub ngn_feed: Pubkey,
    pub ngn_max_stale_slots: u64,
    pub ngn_min_samples: u32,
    pub ngn_max_spread_bps: u16,
    pub promo_inactivity_seconds: i64,
    pub max_promo_per_position: u64,
    pub paused: bool,
    /// Division remainder carried between accruals (numerator units of `lp_rate_product × seconds`).
    /// Taken from the reserved padding, so the account size is unchanged.
    pub accrual_remainder: u128,
    pub reserved: [u8; 240],
}

/// Admin-settable market parameters, used by `create_market` and `update_market_params`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct MarketParams {
    pub interest_rate_bps: u16,
    pub penalty_rate_bps: u16,
    pub reserve_factor_bps: u16,
    pub max_utilization_bps: u16,
    pub min_loan_amount: u64,
    pub max_tenure_seconds: i64,
    pub bad_debt_dust_usd: u128,
    pub ngn_feed: Pubkey,
    pub ngn_max_stale_slots: u64,
    pub ngn_min_samples: u32,
    pub ngn_max_spread_bps: u16,
    pub promo_inactivity_seconds: i64,
    pub max_promo_per_position: u64,
}

impl MarketParams {
    pub fn validate(&self) -> Result<()> {
        require!(self.interest_rate_bps <= MAX_BPS, HodlError::InvalidParameters);
        require!(self.penalty_rate_bps <= MAX_BPS, HodlError::InvalidParameters);
        require!(self.reserve_factor_bps <= MAX_BPS, HodlError::InvalidParameters);
        require!(self.max_utilization_bps <= MAX_BPS, HodlError::InvalidParameters);
        require!(self.max_tenure_seconds >= MIN_TENURE, HodlError::InvalidParameters);
        require!(self.ngn_feed != Pubkey::default(), HodlError::InvalidParameters);
        require!(self.ngn_max_stale_slots > 0, HodlError::InvalidParameters);
        require!(self.ngn_min_samples >= 1, HodlError::InvalidParameters);
        require!(self.ngn_max_spread_bps <= MAX_BPS, HodlError::InvalidParameters);
        require!(self.promo_inactivity_seconds >= 0, HodlError::InvalidParameters);
        require!(self.bad_debt_dust_usd <= MAX_BAD_DEBT_DUST_USD, HodlError::InvalidParameters);
        Ok(())
    }
}

impl Market {
    pub fn params(&self) -> MarketParams {
        MarketParams {
            interest_rate_bps: self.interest_rate_bps,
            penalty_rate_bps: self.penalty_rate_bps,
            reserve_factor_bps: self.reserve_factor_bps,
            max_utilization_bps: self.max_utilization_bps,
            min_loan_amount: self.min_loan_amount,
            max_tenure_seconds: self.max_tenure_seconds,
            bad_debt_dust_usd: self.bad_debt_dust_usd,
            ngn_feed: self.ngn_feed,
            ngn_max_stale_slots: self.ngn_max_stale_slots,
            ngn_min_samples: self.ngn_min_samples,
            ngn_max_spread_bps: self.ngn_max_spread_bps,
            promo_inactivity_seconds: self.promo_inactivity_seconds,
            max_promo_per_position: self.max_promo_per_position,
        }
    }

    pub fn apply_params(&mut self, p: &MarketParams) {
        self.interest_rate_bps = p.interest_rate_bps;
        self.penalty_rate_bps = p.penalty_rate_bps;
        self.reserve_factor_bps = p.reserve_factor_bps;
        self.max_utilization_bps = p.max_utilization_bps;
        self.min_loan_amount = p.min_loan_amount;
        self.max_tenure_seconds = p.max_tenure_seconds;
        self.bad_debt_dust_usd = p.bad_debt_dust_usd;
        self.ngn_feed = p.ngn_feed;
        self.ngn_max_stale_slots = p.ngn_max_stale_slots;
        self.ngn_min_samples = p.ngn_min_samples;
        self.ngn_max_spread_bps = p.ngn_max_spread_bps;
        self.promo_inactivity_seconds = p.promo_inactivity_seconds;
        self.max_promo_per_position = p.max_promo_per_position;
    }

    /// Adds lender interest accrued since `last_accrual_ts`. Every instruction that
    /// touches the market calls this first.
    pub fn accrue(&mut self, now: i64) -> Result<()> {
        if now <= self.last_accrual_ts {
            return Ok(());
        }
        let elapsed = (now - self.last_accrual_ts) as u64;
        let (interest, remainder) = accrue_lp_interest(self.lp_rate_product, elapsed, self.accrual_remainder)?;
        self.accrued_interest = add(self.accrued_interest, interest)?;
        self.accrual_remainder = remainder;
        self.last_accrual_ts = now;
        Ok(())
    }

    /// cash + total_borrows + accrued_interest − protocol_reserve. Call after `accrue`.
    pub fn total_assets(&self) -> Result<u128> {
        let gross = add(add(self.cash as u128, self.total_borrows as u128)?, self.accrued_interest)?;
        sub(gross, self.protocol_reserve as u128)
    }

    /// Cash lenders may withdraw or borrowers may draw.
    pub fn available_cash(&self) -> u64 {
        self.cash.saturating_sub(self.protocol_reserve)
    }
}
