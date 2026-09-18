use anchor_lang::prelude::*;

use crate::constants::{CONFIG_SEED, MARKET_SEED};
use crate::errors::HodlError;
use crate::events::LoanWrittenOff;
use crate::math::checked::{add, sub, to_u64};
use crate::math::loan::{accrued_lp_interest, lp_contribution};
use crate::state::{Config, Market, Position};
use crate::valuation::load_valuation;

#[derive(Accounts)]
pub struct WriteOffLoan<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(mut)]
    pub position: AccountLoader<'info, Position>,
    #[account(
        mut,
        seeds = [MARKET_SEED, market.mint.as_ref()],
        bump = market.bump,
        has_one = ngn_feed @ HodlError::PriceAccountMismatch
    )]
    pub market: Box<Account<'info, Market>>,
    /// CHECK: address pinned to `market.ngn_feed`; parsed by `read_ngn_price`.
    pub ngn_feed: UncheckedAccount<'info>,
}

/// Spec §11 `write_off_loan`. `remaining_accounts`: per used collateral slot, in slot order, a
/// `(CollateralAsset, PriceUpdateV2)` pair, plus the mint as a third account for an `XStock`
/// slot — its scaled-UI multiplier is read from there.
///
/// Only for a liquidatable position whose remaining collateral is worth less than
/// `bad_debt_dust_usd` — below that, liquidating costs more than it recovers, so the loan is
/// cleared and the loss taken: the protocol reserve first, then the lenders through the share
/// price. The collateral stays in the position; sweeping it is a separate admin action.
///
/// The admin signs through a timelocked multisig, so a lender who watches the queue can
/// withdraw before the loss lands (spec §18). That is accepted: a write-off needs the
/// collateral to be dust, which bounds what the remaining lenders absorb.
/// Promo forfeiture (spec §11 step 3) arrives with Plan 5.
pub fn handle_write_off_loan<'info>(ctx: Context<'info, WriteOffLoan<'info>>, loan_id: u64) -> Result<()> {
    let market_key = ctx.accounts.market.key();
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    let market = &mut ctx.accounts.market;
    market.accrue(now)?;

    let mut position = ctx.accounts.position.load_mut()?;
    require_keys_eq!(position.market, market_key, HodlError::MarketMismatch);
    let index = position.loan_index(loan_id).ok_or(HodlError::LoanNotFound)?;

    let valuation = load_valuation(
        ctx.program_id,
        &position,
        market,
        &ctx.accounts.ngn_feed.to_account_info(),
        ctx.remaining_accounts,
        0,
        &clock,
    )?;
    require!(valuation.health.is_liquidatable(), HodlError::NotLiquidatable);
    require!(
        valuation.health.own_value < market.bad_debt_dust_usd,
        HodlError::WriteOffNotAllowed
    );

    let loan = position.loans[index];
    let released = accrued_lp_interest(
        loan.principal,
        loan.rate_bps,
        loan.reserve_factor_bps,
        loan.interest_anchor,
        now,
    )?;
    let loss = add(loan.principal as u128, released)?;

    market.accrued_interest = market.accrued_interest.saturating_sub(released);
    market.total_borrows = market
        .total_borrows
        .checked_sub(loan.principal)
        .ok_or(HodlError::MathOverflow)?;
    market.lp_rate_product = sub(
        market.lp_rate_product,
        lp_contribution(loan.principal, loan.rate_bps, loan.reserve_factor_bps)?,
    )?;

    // The reserve absorbs what it can; the rest reaches lenders as a fall in the share price.
    let covered = to_u64(loss.min(market.protocol_reserve as u128))?;
    market.protocol_reserve = market
        .protocol_reserve
        .checked_sub(covered)
        .ok_or(HodlError::MathOverflow)?;
    market.total_bad_debt = add(market.total_bad_debt, loss)?;

    position.loans[index] = bytemuck::Zeroable::zeroed();
    if !position.has_active_loans() {
        position.promo_last_activity_at = now;
    }
    let owner = position.owner;
    let total_bad_debt = market.total_bad_debt;
    drop(position);

    emit!(LoanWrittenOff {
        market: market_key,
        position: ctx.accounts.position.key(),
        owner,
        loan_id,
        principal: loan.principal,
        loss,
        covered_by_reserve: covered,
        total_bad_debt,
    });
    Ok(())
}
