use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{ACCESS_SEED, BPS, MARKET_SEED, MIN_TENURE, POSITION_SEED};
use crate::errors::HodlError;
use crate::events::LoanOpened;
use crate::math::checked::add;
use crate::math::loan::lp_contribution;
use crate::state::{Access, LoanSlot, Market, Position};
use crate::token::transfer::transfer_from_vault;
use crate::valuation::load_health;

#[derive(Accounts)]
pub struct TakeLoan<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [ACCESS_SEED, owner.key().as_ref()], bump = access.bump)]
    pub access: Account<'info, Access>,
    #[account(mut, seeds = [POSITION_SEED, owner.key().as_ref()], bump)]
    pub position: AccountLoader<'info, Position>,
    #[account(
        mut,
        seeds = [MARKET_SEED, mint.key().as_ref()],
        bump = market.bump,
        has_one = mint,
        has_one = vault,
        has_one = ngn_feed @ HodlError::PriceAccountMismatch
    )]
    pub market: Box<Account<'info, Market>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, token::mint = mint, token::authority = owner, token::token_program = token_program)]
    pub owner_token: Box<InterfaceAccount<'info, TokenAccount>>,
    /// CHECK: address pinned to `market.ngn_feed`; parsed by `read_ngn_price`.
    pub ngn_feed: UncheckedAccount<'info>,
    pub token_program: Interface<'info, TokenInterface>,
}

/// Spec §10 `take_loan`. `remaining_accounts`: one `(CollateralAsset, PriceUpdateV2, mint)`
/// triple per used collateral slot, in slot order.
pub fn handle_take_loan<'info>(
    ctx: Context<'info, TakeLoan<'info>>,
    amount: u64,
    tenure_seconds: i64,
) -> Result<()> {
    ctx.accounts.access.require_active()?;
    let market_key = ctx.accounts.market.key();
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    let market = &mut ctx.accounts.market;
    require!(!market.paused, HodlError::MarketPaused);
    require!(amount > 0 && amount >= market.min_loan_amount, HodlError::AmountTooSmall);
    require!(
        (MIN_TENURE..=market.max_tenure_seconds).contains(&tenure_seconds),
        HodlError::TenureOutOfRange
    );
    market.accrue(now)?;

    let available = market.available_cash();
    require!(amount <= available, HodlError::InsufficientCash);
    let lendable = add(available as u128, market.total_borrows as u128)?;
    let borrows_after = add(market.total_borrows as u128, amount as u128)?;
    require!(
        borrows_after * BPS <= lendable * market.max_utilization_bps as u128,
        HodlError::UtilizationCapExceeded
    );

    let (loan_id, rate_bps, penalty_rate_bps, reserve_factor_bps) = {
        let mut position = ctx.accounts.position.load_mut()?;
        require!(
            position.market == Pubkey::default() || position.market == market_key,
            HodlError::MarketMismatch
        );
        let index = position.free_loan_index().ok_or(HodlError::NoFreeLoanSlot)?;
        let health = load_health(
            ctx.program_id,
            &position,
            market,
            &ctx.accounts.ngn_feed.to_account_info(),
            ctx.remaining_accounts,
            amount,
            &clock,
        )?;
        require!(health.is_healthy(), HodlError::Unhealthy);

        let loan_id = position.next_loan_id;
        position.next_loan_id = loan_id.checked_add(1).ok_or(HodlError::MathOverflow)?;
        position.loans[index] = LoanSlot {
            id: loan_id,
            principal: amount,
            original_principal: amount,
            repaid: 0,
            originated_at: now,
            interest_anchor: now,
            tenure_seconds,
            rate_bps: market.interest_rate_bps,
            penalty_rate_bps: market.penalty_rate_bps,
            reserve_factor_bps: market.reserve_factor_bps,
            active: 1,
            _padding: 0,
        };
        position.market = market_key;
        position.promo_last_activity_at = now;
        (loan_id, market.interest_rate_bps, market.penalty_rate_bps, market.reserve_factor_bps)
    };

    market.total_borrows = market.total_borrows.checked_add(amount).ok_or(HodlError::MathOverflow)?;
    market.lp_rate_product = add(
        market.lp_rate_product,
        lp_contribution(amount, rate_bps, reserve_factor_bps)?,
    )?;
    market.cash -= amount;

    let mint_key = ctx.accounts.mint.key();
    let seeds: &[&[u8]] = &[MARKET_SEED, mint_key.as_ref(), &[ctx.accounts.market.bump]];
    transfer_from_vault(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.owner_token.to_account_info(),
        ctx.accounts.market.to_account_info(),
        amount,
        &[seeds],
    )?;
    emit!(LoanOpened {
        market: market_key,
        position: ctx.accounts.position.key(),
        owner: ctx.accounts.owner.key(),
        loan_id,
        principal: amount,
        tenure_seconds,
        rate_bps,
        penalty_rate_bps,
        reserve_factor_bps,
        originated_at: now,
    });
    Ok(())
}
