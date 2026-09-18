use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{ACCESS_SEED, COLLATERAL_SEED, POSITION_SEED};
use crate::errors::HodlError;
use crate::events::CollateralWithdrawn;
use crate::state::{Access, CollateralAsset, Market, Position};
use crate::token::extensions::require_collateral_mint;
use crate::token::transfer::transfer_from_vault;
use crate::valuation::load_health;

#[derive(Accounts)]
pub struct WithdrawCollateral<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [ACCESS_SEED, owner.key().as_ref()], bump = access.bump)]
    pub access: Account<'info, Access>,
    #[account(mut, seeds = [POSITION_SEED, owner.key().as_ref()], bump)]
    pub position: AccountLoader<'info, Position>,
    #[account(mut, seeds = [COLLATERAL_SEED, mint.key().as_ref()], bump = collateral.bump, has_one = mint, has_one = vault)]
    pub collateral: Box<Account<'info, CollateralAsset>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, token::mint = mint, token::authority = owner, token::token_program = token_program)]
    pub owner_token: Box<InterfaceAccount<'info, TokenAccount>>,
    /// The market the position borrows from. Required only while loans are active.
    pub market: Option<Box<Account<'info, Market>>>,
    /// CHECK: required only while loans are active; `read_ngn_price` pins it to `market.ngn_feed`.
    pub ngn_feed: Option<UncheckedAccount<'info>>,
    pub token_program: Interface<'info, TokenInterface>,
}

/// Without active loans no prices are read, and `market` and `ngn_feed` may be omitted.
/// With active loans, `market` and `ngn_feed` are required, and `remaining_accounts` must
/// hold, per collateral slot still used **after** this withdrawal, in slot order, a
/// `(CollateralAsset, PriceUpdateV2)` pair — an `XStock` slot needs its mint too, as a third
/// account, for its scaled-UI multiplier — and the position must stay healthy.
pub fn handle_withdraw_collateral<'info>(ctx: Context<'info, WithdrawCollateral<'info>>, amount: u64) -> Result<()> {
    ctx.accounts.access.require_active()?;
    require!(amount > 0, HodlError::AmountTooSmall);
    let mint_key = ctx.accounts.mint.key();

    let slot_amount = {
        let mut position = ctx.accounts.position.load_mut()?;
        let index = position.collateral_index(&mint_key).ok_or(HodlError::InsufficientCollateral)?;
        let slot = &mut position.collateral[index];
        require!(slot.amount >= amount, HodlError::InsufficientCollateral);
        slot.amount -= amount;
        let slot_amount = slot.amount;

        if position.has_active_loans() {
            let (Some(market), Some(ngn_feed)) = (&ctx.accounts.market, &ctx.accounts.ngn_feed) else {
                return err!(HodlError::PriceAccountMismatch);
            };
            require_keys_eq!(position.market, market.key(), HodlError::MarketMismatch);
            let health = load_health(
                ctx.program_id,
                &position,
                market,
                &ngn_feed.to_account_info(),
                ctx.remaining_accounts,
                0,
                &Clock::get()?,
            )?;
            require!(health.is_healthy(), HodlError::Unhealthy);
        }
        slot_amount
    };

    let collateral = &mut ctx.accounts.collateral;
    collateral.total_deposited = collateral.total_deposited.checked_sub(amount).ok_or(HodlError::MathOverflow)?;

    require_collateral_mint(&ctx.accounts.mint.to_account_info(), ctx.accounts.collateral.kind)?;
    let seeds: &[&[u8]] = &[COLLATERAL_SEED, mint_key.as_ref(), &[ctx.accounts.collateral.bump]];
    transfer_from_vault(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.owner_token.to_account_info(),
        ctx.accounts.collateral.to_account_info(),
        amount,
        &[seeds],
    )?;
    emit!(CollateralWithdrawn {
        position: ctx.accounts.position.key(),
        owner: ctx.accounts.owner.key(),
        mint: mint_key,
        amount,
        slot_amount,
    });
    Ok(())
}
