use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{CONFIG_SEED, MARKET_SEED, PROMO_VAULT_SEED};
use crate::errors::HodlError;
use crate::events::LoanWrittenOff;
use crate::instructions::promos::{forfeit_promo, ForfeitAccounts};
use crate::math::checked::{add, sub, to_u64};
use crate::math::loan::{accrued_lp_interest, lp_contribution};
use crate::state::{Config, Market, Position, PromoVault};
use crate::valuation::{load_valuation, ValuationRequest};

#[derive(Accounts)]
pub struct WriteOffLoan<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Box<Account<'info, Config>>,
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
    #[account(address = market.mint, mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, address = market.vault @ HodlError::MarketMismatch)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    /// Forfeiture moves the position's promo backing into the market vault (spec §11 step 3),
    /// so a written-off loan still returns what the protocol lent the borrower for free.
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
    #[account(mut, address = promo_vault.vault @ HodlError::PromoVaultInsufficient)]
    pub promo_vault_token: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
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
///
/// Any promo backing the position is forfeited to lenders before the loss is booked (spec §11
/// step 3), offsetting part of what the reserve and lenders would otherwise absorb alone.
pub fn handle_write_off_loan<'info>(ctx: Context<'info, WriteOffLoan<'info>>, loan_id: u64) -> Result<()> {
    let market_key = ctx.accounts.market.key();
    let position_key = ctx.accounts.position.key();
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    let market = &mut ctx.accounts.market;
    market.accrue(now)?;

    let mut position = ctx.accounts.position.load_mut()?;
    require_keys_eq!(position.market, market_key, HodlError::MarketMismatch);
    let index = position.loan_index(loan_id).ok_or(HodlError::LoanNotFound)?;

    let ngn_feed = ctx.accounts.ngn_feed.to_account_info();
    let valuation = load_valuation(
        &position,
        &ValuationRequest {
            program_id: ctx.program_id,
            market,
            ngn_feed: &ngn_feed,
            remaining: ctx.remaining_accounts,
            extra_debt: 0,
            promo_cap_bps: ctx.accounts.config.promo_cap_bps,
            clock: &clock,
        },
    )?;
    require!(valuation.health.is_liquidatable(), HodlError::NotLiquidatable);
    require!(
        valuation.health.own_value < market.bad_debt_dust_usd,
        HodlError::WriteOffNotAllowed
    );

    // Spec §11 step 3: the promo behind a defaulting position goes to lenders, before the loss
    // is booked. It does not reduce what the borrower owed.
    let forfeited = forfeit_promo(
        &mut position,
        position_key,
        market_key,
        &mut ForfeitAccounts {
            promo_vault: &mut ctx.accounts.promo_vault,
            promo_token: &ctx.accounts.promo_vault_token,
            market_vault: &ctx.accounts.vault,
            mint: &ctx.accounts.mint,
            token_program: ctx.accounts.token_program.key(),
        },
    )?;
    market.cash = to_u64(add(market.cash as u128, forfeited as u128)?)?;

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
    // Superseded by forfeiture above: `promo_balance` is already 0 by this point (it was either
    // 0 coming in, or `forfeit_promo` zeroed it earlier in this same call), so this write only
    // ever lands on a position that already has no promo for the clock to gate.
    if !position.has_active_loans() {
        position.promo_last_activity_at = now;
    }
    let owner = position.owner;
    let total_bad_debt = market.total_bad_debt;
    drop(position);

    emit!(LoanWrittenOff {
        market: market_key,
        position: position_key,
        owner,
        loan_id,
        principal: loan.principal,
        loss,
        covered_by_reserve: covered,
        total_bad_debt,
    });
    Ok(())
}
