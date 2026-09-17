use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{ACCOUNT_VERSION, CONFIG_SEED, MARKET_SEED, MARKET_VAULT_SEED};
use crate::errors::HodlError;
use crate::events::{MarketCreated, MarketParamsUpdated, MarketPauseSet};
use crate::state::{Config, Market, MarketParams};
use crate::token::extensions::{require_allowed_extensions, MARKET_MINT_EXTENSIONS};

#[derive(Accounts)]
pub struct CreateMarket<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(init, payer = admin, space = 8 + Market::INIT_SPACE, seeds = [MARKET_SEED, mint.key().as_ref()], bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        init,
        payer = admin,
        token::mint = mint,
        token::authority = market,
        token::token_program = token_program,
        seeds = [MARKET_VAULT_SEED, mint.key().as_ref()],
        bump
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateMarketParams<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(mut, seeds = [MARKET_SEED, market.mint.as_ref()], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
}

#[derive(Accounts)]
pub struct SetMarketPaused<'info> {
    pub signer: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(mut, seeds = [MARKET_SEED, market.mint.as_ref()], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
}

pub fn handle_create_market(ctx: Context<CreateMarket>, params: MarketParams) -> Result<()> {
    params.validate()?;
    require_allowed_extensions(&ctx.accounts.mint.to_account_info(), MARKET_MINT_EXTENSIONS)?;

    let mut market = Market {
        version: ACCOUNT_VERSION,
        bump: ctx.bumps.market,
        vault_bump: ctx.bumps.vault,
        mint: ctx.accounts.mint.key(),
        token_program: ctx.accounts.token_program.key(),
        vault: ctx.accounts.vault.key(),
        decimals: ctx.accounts.mint.decimals,
        cash: 0,
        total_borrows: 0,
        lp_rate_product: 0,
        accrued_interest: 0,
        protocol_reserve: 0,
        total_bad_debt: 0,
        total_shares: 0,
        last_accrual_ts: Clock::get()?.unix_timestamp,
        interest_rate_bps: 0,
        penalty_rate_bps: 0,
        reserve_factor_bps: 0,
        max_utilization_bps: 0,
        min_loan_amount: 0,
        max_tenure_seconds: 0,
        bad_debt_dust_usd: 0,
        ngn_feed: Pubkey::default(),
        ngn_max_stale_slots: 0,
        ngn_min_samples: 0,
        ngn_max_spread_bps: 0,
        promo_inactivity_seconds: 0,
        max_promo_per_position: 0,
        paused: false,
        accrual_remainder: 0,
        reserved: [0; 240],
    };
    market.apply_params(&params);
    ctx.accounts.market.set_inner(market);

    emit!(MarketCreated {
        market: ctx.accounts.market.key(),
        mint: ctx.accounts.mint.key(),
        vault: ctx.accounts.vault.key(),
    });
    Ok(())
}

/// Rate and reserve-factor changes apply to new loans only; existing loans keep their terms.
pub fn handle_update_market_params(ctx: Context<UpdateMarketParams>, params: MarketParams) -> Result<()> {
    params.validate()?;
    let market_key = ctx.accounts.market.key();
    let market = &mut ctx.accounts.market;
    market.accrue(Clock::get()?.unix_timestamp)?;
    let old = market.params();
    market.apply_params(&params);
    emit!(MarketParamsUpdated { market: market_key, old, new: params });
    Ok(())
}

/// Guardian or admin may pause; only admin may unpause.
pub fn handle_set_market_paused(ctx: Context<SetMarketPaused>, paused: bool) -> Result<()> {
    let signer = ctx.accounts.signer.key();
    let config = &ctx.accounts.config;
    if paused {
        require!(signer == config.admin || signer == config.guardian, HodlError::Unauthorized);
    } else {
        require!(signer == config.admin, HodlError::Unauthorized);
    }
    let market_key = ctx.accounts.market.key();
    let old_paused = ctx.accounts.market.paused;
    ctx.accounts.market.paused = paused;
    emit!(MarketPauseSet { market: market_key, old_paused, paused, by: signer });
    Ok(())
}
