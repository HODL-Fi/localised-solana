use anchor_lang::prelude::*;

use crate::constants::{ACCESS_SEED, POSITION_SEED, PROMO_VAULT_SEED};
use crate::errors::HodlError;
use crate::events::{PositionClosed, PromoReleased};
use crate::instructions::promos::release_promo;
use crate::state::{Access, Market, Position, PromoVault};

#[derive(Accounts)]
pub struct ClosePosition<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [ACCESS_SEED, owner.key().as_ref()], bump = access.bump)]
    pub access: Account<'info, Access>,
    #[account(
        mut,
        seeds = [POSITION_SEED, owner.key().as_ref()],
        bump,
        close = rent_payer,
        constraint = position.load()?.rent_payer == rent_payer.key() @ HodlError::Unauthorized
    )]
    pub position: AccountLoader<'info, Position>,
    /// CHECK: must equal `position.rent_payer`; only receives the rent refund.
    #[account(mut)]
    pub rent_payer: UncheckedAccount<'info>,
    /// Both required when the position still holds promo: closing it hands the promo back.
    pub market: Option<Box<Account<'info, Market>>>,
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, promo_vault.market.as_ref()],
        bump = promo_vault.bump
    )]
    pub promo_vault: Option<Box<Account<'info, PromoVault>>>,
}

/// Requires no collateral and no active loans. Any promo goes back to the vault, and the rent
/// goes back to whoever paid it.
pub fn handle_close_position(ctx: Context<ClosePosition>) -> Result<()> {
    ctx.accounts.access.require_active()?;
    let mut position = ctx.accounts.position.load_mut()?;
    require!(!position.has_collateral() && !position.has_active_loans(), HodlError::PositionNotEmpty);

    if position.promo_balance > 0 {
        let market = ctx.accounts.market.as_ref().ok_or(HodlError::PromoAccountsRequired)?;
        let promo_vault = ctx.accounts.promo_vault.as_mut().ok_or(HodlError::PromoAccountsRequired)?;
        require_keys_eq!(position.market, market.key(), HodlError::MarketMismatch);
        // `promo_vault.market == market.key()` is now enforced inside `release_promo` itself
        // (position.market == promo_vault.market, combined with the check above).
        let amount = release_promo(&mut position, promo_vault)?;
        emit!(PromoReleased {
            market: market.key(),
            position: ctx.accounts.position.key(),
            owner: position.owner,
            amount,
        });
    }

    emit!(PositionClosed {
        position: ctx.accounts.position.key(),
        owner: position.owner,
        rent_payer: position.rent_payer,
    });
    Ok(())
}
