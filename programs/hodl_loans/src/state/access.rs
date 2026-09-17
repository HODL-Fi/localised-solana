use anchor_lang::prelude::*;

use crate::constants::ACCOUNT_VERSION;
use crate::errors::HodlError;

#[account]
#[derive(InitSpace)]
pub struct Access {
    pub version: u8,
    pub bump: u8,
    pub wallet: Pubkey,
    pub whitelisted: bool,
    pub blacklisted: bool,
    pub reserved: [u8; 32],
}

impl Access {
    /// Fills in identity fields the first time an `init_if_needed` account is used.
    pub fn init_if_new(&mut self, wallet: Pubkey, bump: u8) {
        if self.version == 0 {
            self.version = ACCOUNT_VERSION;
            self.bump = bump;
            self.wallet = wallet;
        }
    }

    /// Blacklisted wins over whitelisted.
    pub fn require_active(&self) -> Result<()> {
        require!(!self.blacklisted, HodlError::Blacklisted);
        require!(self.whitelisted, HodlError::NotWhitelisted);
        Ok(())
    }
}
