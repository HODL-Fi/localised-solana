use anchor_lang::prelude::*;

use crate::constants::ACCOUNT_VERSION;

#[account]
#[derive(InitSpace)]
pub struct LenderPosition {
    pub version: u8,
    pub bump: u8,
    pub market: Pubkey,
    pub owner: Pubkey,
    pub shares: u128,
    pub reserved: [u8; 32],
}

impl LenderPosition {
    pub fn init_if_new(&mut self, market: Pubkey, owner: Pubkey, bump: u8) {
        if self.version == 0 {
            self.version = ACCOUNT_VERSION;
            self.bump = bump;
            self.market = market;
            self.owner = owner;
        }
    }
}
