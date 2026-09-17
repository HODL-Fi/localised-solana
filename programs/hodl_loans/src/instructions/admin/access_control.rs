use anchor_lang::prelude::*;

use crate::constants::{ACCESS_SEED, CONFIG_SEED};
use crate::errors::HodlError;
use crate::events::AccessUpdated;
use crate::state::{Access, Config};

#[derive(Accounts)]
#[instruction(wallet: Pubkey)]
pub struct Whitelist<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,
    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump,
        constraint = signer.key() == config.whitelister || signer.key() == config.admin @ HodlError::Unauthorized
    )]
    pub config: Account<'info, Config>,
    #[account(init_if_needed, payer = signer, space = 8 + Access::INIT_SPACE, seeds = [ACCESS_SEED, wallet.as_ref()], bump)]
    pub access: Account<'info, Access>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(wallet: Pubkey)]
pub struct Blacklist<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(init_if_needed, payer = admin, space = 8 + Access::INIT_SPACE, seeds = [ACCESS_SEED, wallet.as_ref()], bump)]
    pub access: Account<'info, Access>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(wallet: Pubkey)]
pub struct Unblacklist<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(mut, seeds = [ACCESS_SEED, wallet.as_ref()], bump = access.bump)]
    pub access: Account<'info, Access>,
}

pub fn handle_whitelist(ctx: Context<Whitelist>, wallet: Pubkey) -> Result<()> {
    let access = &mut ctx.accounts.access;
    access.init_if_new(wallet, ctx.bumps.access);
    require!(!access.blacklisted, HodlError::Blacklisted);
    access.whitelisted = true;
    emit!(AccessUpdated { wallet, whitelisted: true, blacklisted: false });
    Ok(())
}

pub fn handle_blacklist(ctx: Context<Blacklist>, wallet: Pubkey) -> Result<()> {
    let access = &mut ctx.accounts.access;
    access.init_if_new(wallet, ctx.bumps.access);
    access.whitelisted = false;
    access.blacklisted = true;
    emit!(AccessUpdated { wallet, whitelisted: false, blacklisted: true });
    Ok(())
}

/// Clears the blacklist flag. Does not re-whitelist.
pub fn handle_unblacklist(ctx: Context<Unblacklist>, wallet: Pubkey) -> Result<()> {
    let access = &mut ctx.accounts.access;
    access.blacklisted = false;
    emit!(AccessUpdated { wallet, whitelisted: access.whitelisted, blacklisted: false });
    Ok(())
}
