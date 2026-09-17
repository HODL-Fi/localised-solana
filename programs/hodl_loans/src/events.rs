use anchor_lang::prelude::*;

use crate::state::MarketParams;

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
