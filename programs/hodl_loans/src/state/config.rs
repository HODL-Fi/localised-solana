use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct Config {
    pub version: u8,
    pub bump: u8,
    pub admin: Pubkey,
    pub pending_admin: Option<Pubkey>,
    pub guardian: Pubkey,
    pub whitelister: Pubkey,
    pub promo_signer: Pubkey,
    /// Owner of the token accounts that receive harvested reserve, swept donations
    /// and withdrawn promo funds.
    pub treasury: Pubkey,
    pub promo_cap_bps: u16,
    pub collateral_count: u16,
    pub reserved: [u8; 128],
}
