use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{ACCESS_SEED, LENDER_SEED, MARKET_SEED};
use crate::errors::HodlError;
use crate::events::LiquidityDeposited;
use crate::math::checked::add;
use crate::math::shares::shares_for_deposit;
use crate::state::{Access, LenderPosition, Market};
use crate::token::transfer::transfer_from_user;

#[derive(Accounts)]
pub struct DepositLiquidity<'info> {
    /// Pays rent for a new lender account (e.g. the gas-relay sponsor).
    #[account(mut)]
    pub payer: Signer<'info>,
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
        init_if_needed,
        payer = payer,
        space = 8 + LenderPosition::INIT_SPACE,
        seeds = [LENDER_SEED, market.key().as_ref(), owner.key().as_ref()],
        bump
    )]
    pub lender: Box<Account<'info, LenderPosition>>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn handle_deposit_liquidity(ctx: Context<DepositLiquidity>, amount: u64) -> Result<()> {
    ctx.accounts.access.require_active()?;
    require!(amount > 0, HodlError::AmountTooSmall);
    require!(!ctx.accounts.market.paused, HodlError::MarketPaused);

    let market_key = ctx.accounts.market.key();
    let owner_key = ctx.accounts.owner.key();
    let market = &mut ctx.accounts.market;
    market.accrue(Clock::get()?.unix_timestamp)?;
    let shares = shares_for_deposit(amount, market.total_shares, market.total_assets()?)?;
    require!(shares > 0, HodlError::ZeroShares);
    market.cash = market.cash.checked_add(amount).ok_or(HodlError::MathOverflow)?;
    market.total_shares = add(market.total_shares, shares)?;

    let lender = &mut ctx.accounts.lender;
    lender.init_if_new(market_key, owner_key, ctx.bumps.lender);
    lender.shares = add(lender.shares, shares)?;

    transfer_from_user(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.owner_token.to_account_info(),
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.owner.to_account_info(),
        amount,
    )?;

    emit!(LiquidityDeposited { market: market_key, owner: owner_key, amount, shares });
    Ok(())
}
