use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, CloseAccount, Mint, TokenAccount, TokenInterface};

use crate::constants::{ACCOUNT_VERSION, COLLATERAL_SEED, COLLATERAL_VAULT_SEED, CONFIG_SEED};
use crate::errors::HodlError;
use crate::events::{CollateralDelisted, CollateralListed, CollateralParamsUpdated, CollateralPauseSet};
use crate::state::{CollateralAsset, CollateralKind, CollateralParams, Config};
use crate::token::extensions::{require_allowed_extensions, STANDARD_COLLATERAL_EXTENSIONS};

#[derive(Accounts)]
pub struct ListCollateral<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        init,
        payer = admin,
        space = 8 + CollateralAsset::INIT_SPACE,
        seeds = [COLLATERAL_SEED, mint.key().as_ref()],
        bump
    )]
    pub collateral: Box<Account<'info, CollateralAsset>>,
    #[account(
        init,
        payer = admin,
        token::mint = mint,
        token::authority = collateral,
        token::token_program = token_program,
        seeds = [COLLATERAL_VAULT_SEED, mint.key().as_ref()],
        bump
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateCollateralParams<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(mut, seeds = [COLLATERAL_SEED, collateral.mint.as_ref()], bump = collateral.bump)]
    pub collateral: Box<Account<'info, CollateralAsset>>,
}

#[derive(Accounts)]
pub struct SetCollateralPaused<'info> {
    pub signer: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(mut, seeds = [COLLATERAL_SEED, collateral.mint.as_ref()], bump = collateral.bump)]
    pub collateral: Box<Account<'info, CollateralAsset>>,
}

#[derive(Accounts)]
pub struct DelistCollateral<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(
        mut,
        seeds = [COLLATERAL_SEED, mint.key().as_ref()],
        bump = collateral.bump,
        has_one = mint,
        has_one = vault,
        close = admin
    )]
    pub collateral: Box<Account<'info, CollateralAsset>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
}

pub fn handle_list_collateral(ctx: Context<ListCollateral>, params: CollateralParams) -> Result<()> {
    params.validate(ctx.accounts.config.promo_cap_bps)?;
    require_allowed_extensions(&ctx.accounts.mint.to_account_info(), STANDARD_COLLATERAL_EXTENSIONS)?;

    let mut asset = CollateralAsset {
        version: ACCOUNT_VERSION,
        bump: ctx.bumps.collateral,
        vault_bump: ctx.bumps.vault,
        mint: ctx.accounts.mint.key(),
        token_program: ctx.accounts.token_program.key(),
        vault: ctx.accounts.vault.key(),
        decimals: ctx.accounts.mint.decimals,
        kind: CollateralKind::Standard,
        pyth_feed_id: [0; 32],
        max_price_age_seconds: 0,
        max_conf_bps: 0,
        ltv_bps: 0,
        liquidation_threshold_bps: 0,
        liquidation_bonus_bps: 0,
        deposit_cap: 0,
        total_deposited: 0,
        paused: false,
        reserved: [0; 128],
    };
    asset.apply_params(&params);
    ctx.accounts.collateral.set_inner(asset);

    let config = &mut ctx.accounts.config;
    config.collateral_count = config.collateral_count.checked_add(1).ok_or(HodlError::MathOverflow)?;

    emit!(CollateralListed {
        collateral: ctx.accounts.collateral.key(),
        mint: ctx.accounts.mint.key(),
        vault: ctx.accounts.vault.key(),
        params,
    });
    Ok(())
}

/// Lowering an LTV or threshold affects existing positions immediately.
pub fn handle_update_collateral_params(ctx: Context<UpdateCollateralParams>, params: CollateralParams) -> Result<()> {
    params.validate(ctx.accounts.config.promo_cap_bps)?;
    let collateral_key = ctx.accounts.collateral.key();
    let collateral = &mut ctx.accounts.collateral;
    let old = collateral.params();
    collateral.apply_params(&params);
    emit!(CollateralParamsUpdated { collateral: collateral_key, old, new: params });
    Ok(())
}

/// Guardian or admin may pause; only admin may unpause.
pub fn handle_set_collateral_paused(ctx: Context<SetCollateralPaused>, paused: bool) -> Result<()> {
    let signer = ctx.accounts.signer.key();
    let config = &ctx.accounts.config;
    if paused {
        require!(signer == config.admin || signer == config.guardian, HodlError::Unauthorized);
    } else {
        require!(signer == config.admin, HodlError::Unauthorized);
    }
    let collateral_key = ctx.accounts.collateral.key();
    let old_paused = ctx.accounts.collateral.paused;
    ctx.accounts.collateral.paused = paused;
    emit!(CollateralPauseSet { collateral: collateral_key, old_paused, paused, by: signer });
    Ok(())
}

/// Requires no recorded deposits and an empty vault (sweep donations first). Closes the vault
/// and the `CollateralAsset`, refunding rent to the admin.
pub fn handle_delist_collateral(ctx: Context<DelistCollateral>) -> Result<()> {
    require!(ctx.accounts.collateral.total_deposited == 0, HodlError::CollateralStillInUse);
    require!(ctx.accounts.vault.amount == 0, HodlError::CollateralStillInUse);

    let mint_key = ctx.accounts.mint.key();
    let seeds: &[&[u8]] = &[COLLATERAL_SEED, mint_key.as_ref(), &[ctx.accounts.collateral.bump]];
    token_interface::close_account(CpiContext::new_with_signer(
        ctx.accounts.token_program.key(),
        CloseAccount {
            account: ctx.accounts.vault.to_account_info(),
            destination: ctx.accounts.admin.to_account_info(),
            authority: ctx.accounts.collateral.to_account_info(),
        },
        &[seeds],
    ))?;

    let config = &mut ctx.accounts.config;
    config.collateral_count = config.collateral_count.checked_sub(1).ok_or(HodlError::MathOverflow)?;
    emit!(CollateralDelisted { collateral: ctx.accounts.collateral.key(), mint: mint_key });
    Ok(())
}
