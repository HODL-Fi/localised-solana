use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{ACCESS_SEED, LENDER_SEED, MARKET_SEED};
use crate::errors::HodlError;
use crate::events::LiquidityWithdrawn;
use crate::math::checked::{sub, to_u64};
use crate::math::shares::{redeemable_amount, shares_to_burn};
use crate::state::{Access, LenderPosition, Market};
use crate::token::transfer::transfer_from_vault;

#[derive(Accounts)]
pub struct WithdrawLiquidity<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [ACCESS_SEED, owner.key().as_ref()], bump = access.bump)]
    pub access: Account<'info, Access>,
    #[account(mut, seeds = [MARKET_SEED, mint.key().as_ref()], bump = market.bump, has_one = mint, has_one = vault)]
    pub market: Box<Account<'info, Market>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, token::mint = mint, token::authority = owner, token::token_program = token_program)]
    pub owner_token: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        seeds = [LENDER_SEED, market.key().as_ref(), owner.key().as_ref()],
        bump = lender.bump,
        has_one = owner
    )]
    pub lender: Box<Account<'info, LenderPosition>>,
    pub token_program: Interface<'info, TokenInterface>,
}

/// `amount == u64::MAX` withdraws the lender's full redeemable value, capped at available cash.
/// Works while the market is paused.
pub fn handle_withdraw_liquidity(ctx: Context<WithdrawLiquidity>, amount: u64) -> Result<()> {
    ctx.accounts.access.require_active()?;
    require!(amount > 0, HodlError::AmountTooSmall);

    let market_key = ctx.accounts.market.key();
    let owner_key = ctx.accounts.owner.key();
    let lender_shares = ctx.accounts.lender.shares;
    let market = &mut ctx.accounts.market;
    market.accrue(Clock::get()?.unix_timestamp)?;
    let total_assets = market.total_assets()?;
    let available = market.available_cash();

    let amount = if amount == u64::MAX {
        let redeemable = redeemable_amount(lender_shares, market.total_shares, total_assets)?;
        to_u64(redeemable.min(available as u128))?
    } else {
        amount
    };
    require!(amount > 0, HodlError::AmountTooSmall);
    require!(amount <= available, HodlError::InsufficientCash);

    let burned = shares_to_burn(amount, market.total_shares, total_assets)?;
    require!(burned <= lender_shares, HodlError::InsufficientShares);
    market.total_shares = sub(market.total_shares, burned)?;
    market.cash -= amount;
    ctx.accounts.lender.shares = sub(lender_shares, burned)?;

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

    emit!(LiquidityWithdrawn { market: market_key, owner: owner_key, amount, shares: burned });
    Ok(())
}
