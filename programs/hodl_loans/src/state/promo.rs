use anchor_lang::prelude::*;

use crate::errors::HodlError;
use crate::math::checked::{add, sub, to_u64};

/// Spec §12. One per market: the cNGN behind every promo balance, and the accounting that keeps
/// `outstanding + unissued ≤ cash` true at all times.
///
/// - `cash` is what the program has recorded as held; a direct transfer to the token account is
///   ignored until it is swept, exactly as the market and collateral vaults treat donations.
/// - `outstanding` is the sum of every position's `promo_balance` — promo already handed out.
/// - `unissued` is the sum over active campaigns of `budget − granted` — promo promised to a
///   campaign but not yet redeemed.
#[account]
#[derive(InitSpace)]
pub struct PromoVault {
    pub version: u8,
    pub bump: u8,
    pub vault_bump: u8,
    pub market: Pubkey,
    pub vault: Pubkey,
    pub cash: u64,
    pub outstanding: u64,
    pub unissued: u64,
    pub reserved: [u8; 64],
}

impl PromoVault {
    /// cNGN committed to neither a position nor a campaign. Funding a campaign and withdrawing
    /// to the treasury both draw from here, so the §12 invariant holds by construction.
    pub fn free(&self) -> Result<u64> {
        let committed = add(self.outstanding as u128, self.unissued as u128)?;
        to_u64(sub(self.cash as u128, committed)?)
    }

    /// Promo leaving a position, on expiry, revocation, forfeiture or `close_position`. The cNGN
    /// itself does not move on expiry or revocation — it becomes free HODL funds again.
    pub fn release(&mut self, amount: u64) -> Result<()> {
        self.outstanding = to_u64(sub(self.outstanding as u128, amount as u128)?)?;
        Ok(())
    }

    pub fn require_invariant(&self) -> Result<()> {
        let committed = add(self.outstanding as u128, self.unissued as u128)?;
        require!(committed <= self.cash as u128, HodlError::PromoVaultInsufficient);
        Ok(())
    }
}

/// Spec §12. A budget the promo signer may issue vouchers against, until `redeem_until`.
#[account]
#[derive(InitSpace)]
pub struct Campaign {
    pub version: u8,
    pub bump: u8,
    pub market: Pubkey,
    pub campaign_id: u64,
    pub budget: u64,
    pub granted: u64,
    pub redeem_until: i64,
    pub active: bool,
    pub reserved: [u8; 32],
}

/// Spec §12. Proof that one voucher nonce has been redeemed. Its existence is what prevents a
/// replay: `redeem_promo` creates it, so a second redemption of the same nonce cannot init.
#[account]
#[derive(InitSpace)]
pub struct VoucherReceipt {
    pub version: u8,
    pub bump: u8,
    pub campaign: Pubkey,
    pub nonce: u64,
    pub voucher_expiry: i64,
    /// Refunded when the receipt is closed, which anyone may do once the voucher has expired.
    pub rent_payer: Pubkey,
    pub reserved: [u8; 32],
}
