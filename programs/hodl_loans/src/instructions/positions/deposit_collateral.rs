use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{ACCESS_SEED, COLLATERAL_SEED, POSITION_SEED};
use crate::errors::HodlError;
use crate::events::CollateralDeposited;
use crate::state::{Access, CollateralAsset, Position};
use crate::token::extensions::require_collateral_mint_on_entry;
use crate::token::transfer::transfer_from_user;

#[derive(Accounts)]
pub struct DepositCollateral<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [ACCESS_SEED, owner.key().as_ref()], bump = access.bump)]
    pub access: Account<'info, Access>,
    #[account(mut, seeds = [POSITION_SEED, owner.key().as_ref()], bump = position.load()?.bump)]
    pub position: AccountLoader<'info, Position>,
    #[account(mut, seeds = [COLLATERAL_SEED, mint.key().as_ref()], bump = collateral.bump, has_one = mint, has_one = vault)]
    pub collateral: Box<Account<'info, CollateralAsset>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, token::mint = mint, token::authority = owner, token::token_program = token_program)]
    pub owner_token: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
}

/// No prices needed. Uses the slot already holding this mint, or the first free slot.
pub fn handle_deposit_collateral(ctx: Context<DepositCollateral>, amount: u64) -> Result<()> {
    ctx.accounts.access.require_active()?;
    require!(amount > 0, HodlError::AmountTooSmall);
    let collateral = &mut ctx.accounts.collateral;
    require!(!collateral.paused, HodlError::CollateralPaused);
    let new_total = collateral.total_deposited.checked_add(amount).ok_or(HodlError::MathOverflow)?;
    require!(new_total <= collateral.deposit_cap, HodlError::DepositCapExceeded);
    collateral.total_deposited = new_total;

    let mint_key = ctx.accounts.mint.key();
    let slot_amount = {
        let mut position = ctx.accounts.position.load_mut()?;
        let index = match position.collateral_index(&mint_key) {
            Some(i) => i,
            None => position.free_collateral_index().ok_or(HodlError::NoFreeCollateralSlot)?,
        };
        let slot = &mut position.collateral[index];
        slot.mint = mint_key;
        slot.amount = slot.amount.checked_add(amount).ok_or(HodlError::MathOverflow)?;
        slot.amount
    };

    // New exposure, so the full entry policy runs again: an issuer can turn something on
    // after listing, and refusing a deposit only declines new business.
    require_collateral_mint_on_entry(&ctx.accounts.mint.to_account_info(), ctx.accounts.collateral.kind)?;
    transfer_from_user(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.owner_token.to_account_info(),
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.owner.to_account_info(),
        amount,
    )?;
    emit!(CollateralDeposited {
        position: ctx.accounts.position.key(),
        owner: ctx.accounts.owner.key(),
        mint: mint_key,
        amount,
        slot_amount,
    });
    Ok(())
}
