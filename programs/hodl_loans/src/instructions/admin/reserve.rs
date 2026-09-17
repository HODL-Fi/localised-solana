use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{CONFIG_SEED, MARKET_SEED};
use crate::errors::HodlError;
use crate::events::ReserveHarvested;
use crate::state::{Config, Market};
use crate::token::transfer::transfer_from_vault;

#[derive(Accounts)]
pub struct HarvestReserve<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(mut, seeds = [MARKET_SEED, mint.key().as_ref()], bump = market.bump, has_one = mint, has_one = vault)]
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

/// Sends `amount` of the protocol reserve to the treasury. `cash` and `protocol_reserve` both fall.
pub fn handle_harvest_reserve(ctx: Context<HarvestReserve>, amount: u64) -> Result<()> {
    require!(amount > 0, HodlError::AmountTooSmall);
    let market_key = ctx.accounts.market.key();
    let market = &mut ctx.accounts.market;
    require!(amount <= market.protocol_reserve, HodlError::InsufficientCash);
    let old_reserve = market.protocol_reserve;
    market.protocol_reserve -= amount;
    market.cash = market.cash.checked_sub(amount).ok_or(HodlError::MathOverflow)?;
    let new_reserve = market.protocol_reserve;

    let mint_key = ctx.accounts.mint.key();
    let seeds: &[&[u8]] = &[MARKET_SEED, mint_key.as_ref(), &[ctx.accounts.market.bump]];
    transfer_from_vault(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.destination.to_account_info(),
        ctx.accounts.market.to_account_info(),
        amount,
        &[seeds],
    )?;
    emit!(ReserveHarvested {
        market: market_key,
        destination: ctx.accounts.destination.key(),
        amount,
        old_reserve,
        new_reserve,
    });
    Ok(())
}
