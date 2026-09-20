use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{ACCOUNT_VERSION, CONFIG_SEED, MARKET_SEED, PROMO_VAULT_SEED, PROMO_VAULT_TOKEN_SEED};
use crate::errors::HodlError;
use crate::events::{PromoVaultCreated, PromoVaultFunded, PromoVaultWithdrawn};
use crate::instructions::admin::sweep::sweep_to_treasury;
use crate::math::checked::{add, sub, to_u64};
use crate::state::{Config, Market, PromoVault};
use crate::token::transfer::{transfer_from_user, transfer_from_vault};

#[derive(Accounts)]
pub struct CreatePromoVault<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(seeds = [MARKET_SEED, mint.key().as_ref()], bump = market.bump, has_one = mint)]
    pub market: Box<Account<'info, Market>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        init,
        payer = admin,
        space = 8 + PromoVault::INIT_SPACE,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
    #[account(
        init,
        payer = admin,
        token::mint = mint,
        token::authority = promo_vault,
        token::token_program = token_program,
        seeds = [PROMO_VAULT_TOKEN_SEED, market.key().as_ref()],
        bump
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

/// Spec §12. One promo vault per market, holding the cNGN behind every promo balance. Separate
/// from `create_market` so a market can run without promo, and so turning promo on is its own
/// admin action rather than a decision baked in at market creation.
pub fn handle_create_promo_vault(ctx: Context<CreatePromoVault>) -> Result<()> {
    let promo_vault = &mut ctx.accounts.promo_vault;
    promo_vault.set_inner(PromoVault {
        version: ACCOUNT_VERSION,
        bump: ctx.bumps.promo_vault,
        vault_bump: ctx.bumps.vault,
        market: ctx.accounts.market.key(),
        vault: ctx.accounts.vault.key(),
        cash: 0,
        outstanding: 0,
        unissued: 0,
        reserved: [0; 64],
    });
    emit!(PromoVaultCreated {
        market: ctx.accounts.market.key(),
        promo_vault: promo_vault.key(),
        vault: ctx.accounts.vault.key(),
    });
    Ok(())
}

#[derive(Accounts)]
pub struct FundPromoVault<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(seeds = [MARKET_SEED, mint.key().as_ref()], bump = market.bump, has_one = mint)]
    pub market: Box<Account<'info, Market>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market,
        has_one = vault
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, token::mint = mint, token::token_program = token_program)]
    pub source: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
}

/// Spec §12. cNGN in; `cash += amount`. The tokens are the backing for promo the program has
/// not yet handed out, so this is the only way `free()` grows.
pub fn handle_fund_promo_vault(ctx: Context<FundPromoVault>, amount: u64) -> Result<()> {
    require!(amount > 0, HodlError::AmountTooSmall);
    transfer_from_user(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.source.to_account_info(),
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.admin.to_account_info(),
        amount,
    )?;

    let promo_vault = &mut ctx.accounts.promo_vault;
    promo_vault.cash = to_u64(add(promo_vault.cash as u128, amount as u128)?)?;
    promo_vault.require_invariant()?;
    emit!(PromoVaultFunded {
        market: ctx.accounts.market.key(),
        amount,
        cash: promo_vault.cash,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct WithdrawPromoVault<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(seeds = [MARKET_SEED, mint.key().as_ref()], bump = market.bump, has_one = mint)]
    pub market: Box<Account<'info, Market>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market,
        has_one = vault
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
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

/// Spec §12. Only free cNGN leaves: what is already promised to a position (`outstanding`) or to
/// a campaign (`unissued`) stays, so the invariant survives the withdrawal.
pub fn handle_withdraw_promo_vault(ctx: Context<WithdrawPromoVault>, amount: u64) -> Result<()> {
    require!(amount > 0, HodlError::AmountTooSmall);
    require!(amount <= ctx.accounts.promo_vault.free()?, HodlError::PromoVaultInsufficient);

    let market_key = ctx.accounts.market.key();
    let seeds: &[&[u8]] = &[PROMO_VAULT_SEED, market_key.as_ref(), &[ctx.accounts.promo_vault.bump]];
    transfer_from_vault(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.destination.to_account_info(),
        ctx.accounts.promo_vault.to_account_info(),
        amount,
        &[seeds],
    )?;

    let promo_vault = &mut ctx.accounts.promo_vault;
    promo_vault.cash = to_u64(sub(promo_vault.cash as u128, amount as u128)?)?;
    promo_vault.require_invariant()?;
    emit!(PromoVaultWithdrawn {
        market: market_key,
        amount,
        cash: promo_vault.cash,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct SweepPromoExcess<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(seeds = [MARKET_SEED, mint.key().as_ref()], bump = market.bump, has_one = mint)]
    pub market: Box<Account<'info, Market>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market,
        has_one = vault
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
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

/// Sends promo vault tokens above `cash` (direct donations) to the treasury. The third vault to
/// need this, and the reason the three share `sweep_to_treasury`.
pub fn handle_sweep_promo_excess(ctx: Context<SweepPromoExcess>) -> Result<()> {
    let market_key = ctx.accounts.market.key();
    let seeds: &[&[u8]] = &[PROMO_VAULT_SEED, market_key.as_ref(), &[ctx.accounts.promo_vault.bump]];
    sweep_to_treasury(
        &ctx.accounts.token_program,
        &ctx.accounts.mint,
        &ctx.accounts.vault,
        &ctx.accounts.destination,
        ctx.accounts.promo_vault.to_account_info(),
        ctx.accounts.promo_vault.cash,
        &[seeds],
    )
}
