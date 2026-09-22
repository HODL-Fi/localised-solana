use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount};

use crate::constants::{CONFIG_SEED, POSITION_SEED, PROMO_VAULT_SEED};
use crate::errors::HodlError;
use crate::events::{PromoExpired, PromoForfeited, PromoRevoked};
use crate::math::checked::{sub, to_u64};
use crate::state::{Config, Market, Position, PromoVault};
use crate::token::transfer::transfer_from_vault;
use crate::valuation::{load_health, ValuationRequest};

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

/// `expire_promo`'s alone: promo only expires out of a quiet position, so an inactivity claim
/// can never be used to strip borrowing power out from under a live loan. `revoke_promo` no
/// longer routes through here — it prices the position after the release instead.
fn release_from_idle_position(position: &mut Position, promo_vault: &mut PromoVault) -> Result<u64> {
    require!(position.promo_balance > 0, HodlError::AmountTooSmall);
    require!(!position.has_active_loans(), HodlError::PositionNotEmpty);
    release_promo(position, promo_vault)
}

/// The accounts forfeiture moves cNGN between. Grouped and passed by reference because the
/// callers are already close to the SBF 4 KB stack frame: nine positional arguments, three of
/// them `AccountInfo`, was enough to overflow `liquidate`.
pub struct ForfeitAccounts<'a, 'info> {
    pub promo_vault: &'a mut Account<'info, PromoVault>,
    pub promo_token: &'a InterfaceAccount<'info, TokenAccount>,
    pub market_vault: &'a InterfaceAccount<'info, TokenAccount>,
    pub mint: &'a InterfaceAccount<'info, Mint>,
    pub token_program: Pubkey,
}

/// Spec §11 step 3. Unlike expiry, forfeiture *moves* the cNGN: the promo backing a defaulting
/// position leaves the promo vault for the market vault, where it becomes lender cash. It does
/// not reduce the borrower's debt — it is the protocol taking back what it lent them for free.
///
/// Returns the amount actually moved into the market vault, which the caller adds to
/// `market.cash`; the market account is already borrowed mutably at every call site. This can be
/// less than the position's released promo balance after a clawback (see the comment below) —
/// `outstanding` still drops by the full balance either way. Routes through `release_promo` —
/// the single chokepoint through which promo leaves a position — rather than duplicating its
/// accounting.
pub fn forfeit_promo(
    position: &mut Position,
    position_key: Pubkey,
    market_key: Pubkey,
    accounts: &mut ForfeitAccounts,
) -> Result<u64> {
    if position.promo_balance == 0 {
        return Ok(0);
    }
    let amount = release_promo(position, accounts.promo_vault)?;

    // cNGN carries a PermanentDelegate, so its issuer can move tokens out of the promo vault's
    // token account without the program's involvement. After such a clawback,
    // `promo_vault.cash` (this program's ledger) can overstate the vault's real token balance.
    // Transferring the full nominal `amount` unconditionally would then revert — bricking every
    // liquidation and write-off of a promo-holding position on this market, a liveness failure
    // on the protocol's solvency backstop. Clamp the transfer, and the cash debit, to what the
    // vault actually holds: lenders receive whatever remains instead of the call reverting
    // outright.
    let available = accounts.promo_token.amount;
    let moved = amount.min(available);

    if moved > 0 {
        let seeds: &[&[u8]] = &[PROMO_VAULT_SEED, market_key.as_ref(), &[accounts.promo_vault.bump]];
        transfer_from_vault(
            accounts.token_program,
            accounts.mint.to_account_info(),
            accounts.mint.decimals,
            accounts.promo_token.to_account_info(),
            accounts.market_vault.to_account_info(),
            accounts.promo_vault.to_account_info(),
            moved,
            &[seeds],
        )?;
    }

    accounts.promo_vault.cash = to_u64(sub(accounts.promo_vault.cash as u128, moved as u128)?)?;
    accounts.promo_vault.require_invariant()?;

    emit!(PromoForfeited {
        market: market_key,
        position: position_key,
        owner: position.owner,
        amount,
        moved,
    });
    Ok(moved)
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
    // Expiry is the one promo operation barred by a pause, which looks backwards next to the
    // rule that pauses stop exposure-*increasing* work and leave the rest open. It is not an
    // exception to that rule but to a different one: this instruction's precondition is a
    // measurement of how long the borrower has been inactive, and a paused market is one the
    // borrower cannot act on. The measurement is invalid during a pause, not the operation
    // unsafe. `revoke_promo` stays open — it makes no such measurement.
    require!(!ctx.accounts.market.paused, HodlError::MarketPaused);
    let resumed_at = ctx.accounts.market.promo_clock_resumed_at;
    let mut position = ctx.accounts.position.load_mut()?;
    // The clock runs from the borrower's last activity or the market's last unpause,
    // whichever is later: see `Market::promo_clock_resumed_at`.
    // `saturating_add` can only push the deadline later (never wrap it earlier), so this fails
    // safe on overflow — it depends on `MarketParams::validate` requiring `inactivity > 0`, so
    // the two must not drift apart.
    let since = position.promo_last_activity_at.max(resumed_at);
    require!(now >= since.saturating_add(inactivity), HodlError::PromoNotExpired);
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
    /// CHECK: required only when the position has active loans, to price the health check
    /// below; `read_ngn_price` pins it to `market.ngn_feed`.
    pub ngn_feed: Option<UncheckedAccount<'info>>,
}

/// Spec §12. The same release without waiting out the clock — for promo granted in error or to
/// an account the backend has since judged ineligible.
///
/// A live loan no longer blocks it outright. The old rule — revoke only an idle position —
/// was sound in its purpose (revocation must not push anyone into liquidation) but handed the
/// borrower the wrong lever: one minimum-size loan, held open, made promo permanently
/// unrevokable, which defeats the instruction in exactly the case it exists for. Instead the
/// position is checked for health **after** the promo is taken away, and the whole
/// transaction reverts if it would not survive.
///
/// Note what changed class. The old rule was unconditional: no parameter could bypass it.
/// `is_healthy()` is not — the same admin signing this also sets `ltv_bps`,
/// `liquidation_threshold_bps` and `max_conf_bps` through `update_collateral_params`, with no
/// in-program timelock, so three instructions in one transaction can raise the LTV, revoke,
/// and restore it, leaving the borrower promo-less and liquidatable. What stands between that
/// and a borrower is spec §18's timelocked multisig, which is an operational control rather
/// than an on-chain one. The trade is deliberate: the unconditional rule let any borrower make
/// promo permanently unrevokable by holding one minimum-size loan open, which defeats the
/// instruction in exactly the case it exists for.
///
/// With active loans, `ngn_feed` is required and `remaining_accounts` must hold, per used
/// collateral slot in slot order, a `(CollateralAsset, PriceUpdateV2)` pair — an `XStock` slot
/// needs its mint too, as a third account.
pub fn handle_revoke_promo<'info>(ctx: Context<'info, RevokePromo<'info>>) -> Result<()> {
    let market_key = ctx.accounts.market.key();
    let mut position = ctx.accounts.position.load_mut()?;
    require!(position.promo_balance > 0, HodlError::AmountTooSmall);
    let has_loans = position.has_active_loans();
    let amount = release_promo(&mut position, &mut ctx.accounts.promo_vault)?;
    if has_loans {
        // Health is measured on the position as it stands *after* the release, so what is
        // being asked is exactly "can this borrower stand without the promo?". A failure
        // reverts the release along with everything else in the transaction.
        let Some(ngn_feed) = &ctx.accounts.ngn_feed else {
            return err!(HodlError::PriceAccountMismatch);
        };
        let ngn_feed = ngn_feed.to_account_info();
        let clock = Clock::get()?;
        let health = load_health(
            &position,
            &ValuationRequest {
                program_id: ctx.program_id,
                market: &ctx.accounts.market,
                ngn_feed: &ngn_feed,
                remaining: ctx.remaining_accounts,
                extra_debt: 0,
                promo_cap_bps: ctx.accounts.config.promo_cap_bps,
                clock: &clock,
            },
        )?;
        require!(health.is_healthy(), HodlError::Unhealthy);
    }
    emit!(PromoRevoked {
        market: market_key,
        position: ctx.accounts.position.key(),
        owner: position.owner,
        amount,
    });
    Ok(())
}
