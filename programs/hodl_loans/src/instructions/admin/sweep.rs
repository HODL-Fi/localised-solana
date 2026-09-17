use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{COLLATERAL_SEED, CONFIG_SEED, MARKET_SEED};
use crate::errors::HodlError;
use crate::events::ExcessSwept;
use crate::state::{CollateralAsset, Config, Market};
use crate::token::transfer::transfer_from_vault;

#[derive(Accounts)]
pub struct SweepMarketExcess<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(seeds = [MARKET_SEED, mint.key().as_ref()], bump = market.bump, has_one = mint, has_one = vault)]
    pub market: Box<Account<'info, Market>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = mint,
        token::authority = config.treasury,
        token::token_program = token_program
    )]
    pub destination: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
}

/// Sends vault tokens above `market.cash` (direct donations) to the treasury.
pub fn handle_sweep_market_excess(ctx: Context<SweepMarketExcess>) -> Result<()> {
    let excess = ctx.accounts.vault.amount.saturating_sub(ctx.accounts.market.cash);
    require!(excess > 0, HodlError::AmountTooSmall);

    let mint_key = ctx.accounts.mint.key();
    let seeds: &[&[u8]] = &[MARKET_SEED, mint_key.as_ref(), &[ctx.accounts.market.bump]];
    transfer_from_vault(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.destination.to_account_info(),
        ctx.accounts.market.to_account_info(),
        excess,
        &[seeds],
    )?;
    emit!(ExcessSwept {
        vault: ctx.accounts.vault.key(),
        destination: ctx.accounts.destination.key(),
        amount: excess,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct SweepCollateralExcess<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(
        seeds = [COLLATERAL_SEED, mint.key().as_ref()],
        bump = collateral.bump,
        has_one = mint,
        has_one = vault
    )]
    pub collateral: Box<Account<'info, CollateralAsset>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = mint,
        token::authority = config.treasury,
        token::token_program = token_program
    )]
    pub destination: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
}

/// Sends collateral vault tokens above `total_deposited` (direct donations) to the treasury.
pub fn handle_sweep_collateral_excess(ctx: Context<SweepCollateralExcess>) -> Result<()> {
    let excess = ctx.accounts.vault.amount.saturating_sub(ctx.accounts.collateral.total_deposited);
    require!(excess > 0, HodlError::AmountTooSmall);

    let mint_key = ctx.accounts.mint.key();
    let seeds: &[&[u8]] = &[COLLATERAL_SEED, mint_key.as_ref(), &[ctx.accounts.collateral.bump]];
    transfer_from_vault(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.destination.to_account_info(),
        ctx.accounts.collateral.to_account_info(),
        excess,
        &[seeds],
    )?;
    emit!(ExcessSwept {
        vault: ctx.accounts.vault.key(),
        destination: ctx.accounts.destination.key(),
        amount: excess,
    });
    Ok(())
}
