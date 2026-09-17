use anchor_lang::prelude::*;

use crate::constants::CONFIG_SEED;
use crate::errors::HodlError;
use crate::events::{AdminAccepted, AdminProposed};
use crate::state::Config;

/// Accounts for admin-only changes to `Config`.
#[derive(Accounts)]
pub struct AdminConfig<'info> {
    pub admin: Signer<'info>,
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
}

#[derive(Accounts)]
pub struct AcceptAdmin<'info> {
    pub new_admin: Signer<'info>,
    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump,
        constraint = config.pending_admin == Some(new_admin.key()) @ HodlError::Unauthorized
    )]
    pub config: Account<'info, Config>,
}

pub fn handle_propose_admin(ctx: Context<AdminConfig>, proposed: Pubkey) -> Result<()> {
    let config = &mut ctx.accounts.config;
    config.pending_admin = Some(proposed);
    emit!(AdminProposed { admin: config.admin, proposed });
    Ok(())
}

pub fn handle_accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
    let config = &mut ctx.accounts.config;
    let old_admin = config.admin;
    config.admin = ctx.accounts.new_admin.key();
    config.pending_admin = None;
    emit!(AdminAccepted { old_admin, new_admin: config.admin });
    Ok(())
}
