use anchor_lang::prelude::*;

use crate::state::{CollateralKind, CollateralParams, MarketParams};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Guardian,
    Whitelister,
    PromoSigner,
    Treasury,
}

#[event]
pub struct ConfigInitialized {
    pub admin: Pubkey,
    pub guardian: Pubkey,
    pub whitelister: Pubkey,
    pub promo_signer: Pubkey,
    pub treasury: Pubkey,
}

#[event]
pub struct AdminProposed {
    pub admin: Pubkey,
    pub old_pending: Option<Pubkey>,
    pub proposed: Pubkey,
}

#[event]
pub struct AdminAccepted {
    pub old_admin: Pubkey,
    pub new_admin: Pubkey,
}

#[event]
pub struct RoleUpdated {
    pub role: Role,
    pub old: Pubkey,
    pub new: Pubkey,
}

#[event]
pub struct AccessUpdated {
    pub wallet: Pubkey,
    pub old_whitelisted: bool,
    pub old_blacklisted: bool,
    pub whitelisted: bool,
    pub blacklisted: bool,
}

#[event]
pub struct MarketCreated {
    pub market: Pubkey,
    pub mint: Pubkey,
    pub vault: Pubkey,
}

#[event]
pub struct MarketParamsUpdated {
    pub market: Pubkey,
    pub old: MarketParams,
    pub new: MarketParams,
}

#[event]
pub struct MarketPauseSet {
    pub market: Pubkey,
    pub old_paused: bool,
    pub paused: bool,
    pub by: Pubkey,
}

#[event]
pub struct LiquidityDeposited {
    pub market: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    pub shares: u128,
}

#[event]
pub struct LiquidityWithdrawn {
    pub market: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    pub shares: u128,
}

#[event]
pub struct ExcessSwept {
    pub vault: Pubkey,
    pub destination: Pubkey,
    pub amount: u64,
}

#[event]
pub struct CollateralListed {
    pub collateral: Pubkey,
    pub mint: Pubkey,
    pub vault: Pubkey,
    pub kind: CollateralKind,
    pub params: CollateralParams,
}

#[event]
pub struct CollateralParamsUpdated {
    pub collateral: Pubkey,
    pub old: CollateralParams,
    pub new: CollateralParams,
}

#[event]
pub struct CollateralPauseSet {
    pub collateral: Pubkey,
    pub old_paused: bool,
    pub paused: bool,
    pub by: Pubkey,
}

#[event]
pub struct CollateralDelisted {
    pub collateral: Pubkey,
    pub mint: Pubkey,
}

#[event]
pub struct PositionOpened {
    pub position: Pubkey,
    pub owner: Pubkey,
    pub rent_payer: Pubkey,
}

#[event]
pub struct PositionClosed {
    pub position: Pubkey,
    pub owner: Pubkey,
    pub rent_payer: Pubkey,
}

#[event]
pub struct CollateralDeposited {
    pub position: Pubkey,
    pub owner: Pubkey,
    pub mint: Pubkey,
    pub amount: u64,
    pub slot_amount: u64,
}

#[event]
pub struct LoanOpened {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub loan_id: u64,
    pub principal: u64,
    pub tenure_seconds: i64,
    pub rate_bps: u16,
    pub penalty_rate_bps: u16,
    pub reserve_factor_bps: u16,
    pub originated_at: i64,
}

#[event]
pub struct LoanRepaid {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub payer: Pubkey,
    pub loan_id: u64,
    pub amount: u64,
    pub principal_repaid: u64,
    pub interest_paid: u64,
    pub remaining_principal: u64,
}

#[event]
pub struct LoanPartiallyRepaid {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub payer: Pubkey,
    pub loan_id: u64,
    pub amount: u64,
    pub principal_repaid: u64,
    pub interest_paid: u64,
    pub remaining_principal: u64,
}

#[event]
pub struct CollateralWithdrawn {
    pub position: Pubkey,
    pub owner: Pubkey,
    pub mint: Pubkey,
    pub amount: u64,
    pub slot_amount: u64,
}

#[event]
pub struct ReserveHarvested {
    pub market: Pubkey,
    pub destination: Pubkey,
    pub amount: u64,
    pub old_reserve: u64,
    pub new_reserve: u64,
}

#[event]
pub struct LoanLiquidated {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub liquidator: Pubkey,
    pub loan_id: u64,
    pub amount: u64,
    pub principal_repaid: u64,
    pub interest_paid: u64,
    pub collateral_mint: Pubkey,
    pub collateral_seized: u64,
    pub remaining_principal: u64,
}

#[event]
pub struct LoanPartiallyLiquidated {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub liquidator: Pubkey,
    pub loan_id: u64,
    pub amount: u64,
    pub principal_repaid: u64,
    pub interest_paid: u64,
    pub collateral_mint: Pubkey,
    pub collateral_seized: u64,
    pub remaining_principal: u64,
}

#[event]
pub struct LoanWrittenOff {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub loan_id: u64,
    pub principal: u64,
    /// Principal plus the lender interest released for it.
    pub loss: u128,
    pub covered_by_reserve: u64,
    pub total_bad_debt: u128,
}

#[event]
pub struct PromoVaultCreated {
    pub market: Pubkey,
    pub promo_vault: Pubkey,
    pub vault: Pubkey,
}

#[event]
pub struct PromoVaultFunded {
    pub market: Pubkey,
    pub amount: u64,
    pub cash: u64,
}

#[event]
pub struct PromoVaultWithdrawn {
    pub market: Pubkey,
    pub amount: u64,
    pub cash: u64,
}

#[event]
pub struct CampaignCreated {
    pub market: Pubkey,
    pub campaign: Pubkey,
    pub campaign_id: u64,
    pub budget: u64,
    pub redeem_until: i64,
}

#[event]
pub struct CampaignClosed {
    pub market: Pubkey,
    pub campaign: Pubkey,
    pub campaign_id: u64,
    pub granted: u64,
    /// The unspent budget handed back to the vault's free cNGN.
    pub returned: u64,
}
