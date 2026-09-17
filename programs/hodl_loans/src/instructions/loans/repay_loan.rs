use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{ACCESS_SEED, MARKET_SEED};
use crate::errors::HodlError;
use crate::events::{LoanPartiallyRepaid, LoanRepaid};
use crate::math::checked::{sub, to_u64};
use crate::math::loan::{accrued_lp_interest, loan_balance, lp_contribution, reserve_share};
use crate::state::{Access, Market, Position};
use crate::token::transfer::transfer_from_user;

#[derive(Accounts)]
pub struct RepayLoan<'info> {
    /// Any whitelisted wallet may repay any position's loan.
    pub payer: Signer<'info>,
    #[account(seeds = [ACCESS_SEED, payer.key().as_ref()], bump = access.bump)]
    pub access: Account<'info, Access>,
    #[account(mut)]
    pub position: AccountLoader<'info, Position>,
    #[account(mut, seeds = [MARKET_SEED, mint.key().as_ref()], bump = market.bump, has_one = mint, has_one = vault)]
    pub market: Box<Account<'info, Market>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, token::mint = mint, token::authority = payer, token::token_program = token_program)]
    pub payer_token: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
}

/// Spec §10 `repay_loan`. Needs no prices, so it works during an oracle outage or pause.
pub fn handle_repay_loan(ctx: Context<RepayLoan>, loan_id: u64, amount: u64) -> Result<()> {
    ctx.accounts.access.require_active()?;
    require!(amount > 0, HodlError::AmountTooSmall);
    let market_key = ctx.accounts.market.key();
    let now = Clock::get()?.unix_timestamp;

    let market = &mut ctx.accounts.market;
    market.accrue(now)?;

    let mut position = ctx.accounts.position.load_mut()?;
    require_keys_eq!(position.market, market_key, HodlError::MarketMismatch);
    let index = position.loan_index(loan_id).ok_or(HodlError::LoanNotFound)?;
    let loan = position.loans[index];
    let owner = position.owner;

    let balance = loan_balance(&loan.terms(), now)?;
    let due = balance.due()?;
    let paid = (amount as u128).min(balance.total()?);
    require!(paid >= due, HodlError::RepaymentBelowInterest);
    let principal_repaid = to_u64(paid - due)?;
    let paid = to_u64(paid)?;

    // Release exactly the lender interest the market accrued for this loan since its anchor.
    let released = accrued_lp_interest(loan.principal, loan.rate_bps, loan.reserve_factor_bps, loan.interest_anchor, now)?;
    market.accrued_interest = market.accrued_interest.saturating_sub(released);
    market.protocol_reserve = market
        .protocol_reserve
        .checked_add(to_u64(reserve_share(due, loan.reserve_factor_bps)?)?)
        .ok_or(HodlError::MathOverflow)?;
    market.total_borrows = market.total_borrows.checked_sub(principal_repaid).ok_or(HodlError::MathOverflow)?;
    market.lp_rate_product = sub(
        market.lp_rate_product,
        lp_contribution(principal_repaid, loan.rate_bps, loan.reserve_factor_bps)?,
    )?;
    market.cash = market.cash.checked_add(paid).ok_or(HodlError::MathOverflow)?;

    let slot = &mut position.loans[index];
    slot.principal -= principal_repaid;
    slot.repaid = slot.repaid.checked_add(paid).ok_or(HodlError::MathOverflow)?;
    slot.interest_anchor = now;
    let remaining_principal = slot.principal;
    if remaining_principal == 0 {
        *slot = bytemuck::Zeroable::zeroed();
        if !position.has_active_loans() {
            position.promo_last_activity_at = now;
        }
    }
    drop(position);

    transfer_from_user(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.payer_token.to_account_info(),
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.payer.to_account_info(),
        paid,
    )?;

    let position_key = ctx.accounts.position.key();
    let payer = ctx.accounts.payer.key();
    let interest_paid = to_u64(due)?;
    if remaining_principal == 0 {
        emit!(LoanRepaid { market: market_key, position: position_key, owner, payer, loan_id, amount: paid, principal_repaid, interest_paid, remaining_principal });
    } else {
        emit!(LoanPartiallyRepaid { market: market_key, position: position_key, owner, payer, loan_id, amount: paid, principal_repaid, interest_paid, remaining_principal });
    }
    Ok(())
}
