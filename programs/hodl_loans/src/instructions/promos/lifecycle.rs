use anchor_lang::prelude::*;

use crate::constants::{CONFIG_SEED, POSITION_SEED, PROMO_VAULT_SEED};
use crate::errors::HodlError;
use crate::events::{PromoExpired, PromoRevoked};
use crate::state::{Config, Market, Position, PromoVault};

/// The one way promo leaves a position: expiry, revocation, forfeiture on liquidation or
/// write-off, and `close_position` all end here. The cNGN itself does not move — it stops being
/// promised to this position and becomes free vault funds again.
pub fn release_promo(position: &mut Position, promo_vault: &mut PromoVault) -> Result<u64> {
    // This is the chokepoint every caller funnels through, so it asserts its own invariant
    // rather than trusting callers to have checked it first.
    require_keys_eq!(position.market, promo_vault.market, HodlError::MarketMismatch);
    let amount = position.promo_balance;
    promo_vault.release(amount)?;
    position.promo_balance = 0;
    Ok(amount)
}

/// Shared by `expire_promo` and `revoke_promo`: promo only leaves a quiet position, so neither
/// can be used to strip borrowing power out from under a live loan.
fn release_from_idle_position(position: &mut Position, promo_vault: &mut PromoVault) -> Result<u64> {
    require!(position.promo_balance > 0, HodlError::AmountTooSmall);
    require!(!position.has_active_loans(), HodlError::PositionNotEmpty);
    release_promo(position, promo_vault)
}

#[derive(Accounts)]
pub struct ExpirePromo<'info> {
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
    #[account(mut, seeds = [POSITION_SEED, position.load()?.owner.as_ref()], bump)]
    pub position: AccountLoader<'info, Position>,
}

/// Spec §12. Anyone may reclaim promo a borrower has left idle, which is what stops granted
/// promo sitting on the books forever. The clock runs from the last redemption, the last loan
/// taken, or the moment the last loan closed.
pub fn handle_expire_promo(ctx: Context<ExpirePromo>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let market_key = ctx.accounts.market.key();
    let inactivity = ctx.accounts.market.promo_inactivity_seconds;
    let mut position = ctx.accounts.position.load_mut()?;
    // `saturating_add` can only push the deadline later (never wrap it earlier), so this fails
    // safe on overflow — it depends on `MarketParams::validate` requiring `inactivity > 0`, so
    // the two must not drift apart.
    require!(
        now >= position.promo_last_activity_at.saturating_add(inactivity),
        HodlError::PromoNotExpired
    );
    let amount = release_from_idle_position(&mut position, &mut ctx.accounts.promo_vault)?;
    emit!(PromoExpired {
        market: market_key,
        position: ctx.accounts.position.key(),
        owner: position.owner,
        amount,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct RevokePromo<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
    #[account(mut, seeds = [POSITION_SEED, position.load()?.owner.as_ref()], bump)]
    pub position: AccountLoader<'info, Position>,
}

/// Spec §12. The same release without waiting out the clock — for promo granted in error or to
/// an account the backend has since judged ineligible. It still cannot touch a position with a
/// live loan, so it cannot be used to push someone into liquidation.
pub fn handle_revoke_promo(ctx: Context<RevokePromo>) -> Result<()> {
    let market_key = ctx.accounts.market.key();
    let mut position = ctx.accounts.position.load_mut()?;
    let amount = release_from_idle_position(&mut position, &mut ctx.accounts.promo_vault)?;
    emit!(PromoRevoked {
        market: market_key,
        position: ctx.accounts.position.key(),
        owner: position.owner,
        amount,
    });
    Ok(())
}
