use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{COLLATERAL_SEED, CONFIG_SEED, MARKET_SEED};
use crate::errors::HodlError;
use crate::events::ExcessSwept;
use crate::state::{CollateralAsset, Config, Market};
use crate::token::extensions::require_collateral_mint_on_exit;
use crate::token::transfer::transfer_from_vault;

/// The body every vault sweep shares: whatever the token account holds above what the program
/// has recorded is a direct donation, and goes to the treasury. Three vaults record their
/// holdings in three different fields, which is the only thing that differs between them.
pub fn sweep_to_treasury<'info>(
    token_program: &Interface<'info, TokenInterface>,
    mint: &InterfaceAccount<'info, Mint>,
    vault: &InterfaceAccount<'info, TokenAccount>,
    destination: &InterfaceAccount<'info, TokenAccount>,
    authority: AccountInfo<'info>,
    recorded: u64,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    let excess = vault.amount.saturating_sub(recorded);
    require!(excess > 0, HodlError::AmountTooSmall);
    transfer_from_vault(
        token_program.key(),
        mint.to_account_info(),
        mint.decimals,
        vault.to_account_info(),
        destination.to_account_info(),
        authority,
        excess,
        signer_seeds,
    )?;
    emit!(ExcessSwept {
        vault: vault.key(),
        destination: destination.key(),
        amount: excess,
    });
    Ok(())
}

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
    let mint_key = ctx.accounts.mint.key();
    let seeds: &[&[u8]] = &[MARKET_SEED, mint_key.as_ref(), &[ctx.accounts.market.bump]];
    sweep_to_treasury(
        &ctx.accounts.token_program,
        &ctx.accounts.mint,
        &ctx.accounts.vault,
        &ctx.accounts.destination,
        ctx.accounts.market.to_account_info(),
        ctx.accounts.market.cash,
        &[seeds],
    )
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
    require_collateral_mint_on_exit(&ctx.accounts.mint.to_account_info())?;

    let mint_key = ctx.accounts.mint.key();
    let seeds: &[&[u8]] = &[COLLATERAL_SEED, mint_key.as_ref(), &[ctx.accounts.collateral.bump]];
    sweep_to_treasury(
        &ctx.accounts.token_program,
        &ctx.accounts.mint,
        &ctx.accounts.vault,
        &ctx.accounts.destination,
        ctx.accounts.collateral.to_account_info(),
        ctx.accounts.collateral.total_deposited,
        &[seeds],
    )
}
