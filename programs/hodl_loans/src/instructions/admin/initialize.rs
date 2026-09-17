use anchor_lang::prelude::*;

use crate::constants::{ACCOUNT_VERSION, CONFIG_SEED, DEFAULT_PROMO_CAP_BPS};
use crate::errors::HodlError;
use crate::events::ConfigInitialized;
use crate::state::Config;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug)]
pub struct InitializeArgs {
    pub guardian: Pubkey,
    pub whitelister: Pubkey,
    pub promo_signer: Pubkey,
    pub treasury: Pubkey,
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    /// Must be the program's upgrade authority; becomes the admin.
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(init, payer = authority, space = 8 + Config::INIT_SPACE, seeds = [CONFIG_SEED], bump)]
    pub config: Account<'info, Config>,
    #[account(constraint = program.programdata_address()? == Some(program_data.key()) @ HodlError::Unauthorized)]
    pub program: Program<'info, crate::program::HodlLoans>,
    #[account(constraint = program_data.upgrade_authority_address == Some(authority.key()) @ HodlError::Unauthorized)]
    pub program_data: Account<'info, ProgramData>,
    pub system_program: Program<'info, System>,
}

pub fn handle_initialize(ctx: Context<Initialize>, args: InitializeArgs) -> Result<()> {
    let admin = ctx.accounts.authority.key();
    ctx.accounts.config.set_inner(Config {
        version: ACCOUNT_VERSION,
        bump: ctx.bumps.config,
        admin,
        pending_admin: None,
        guardian: args.guardian,
        whitelister: args.whitelister,
        promo_signer: args.promo_signer,
        treasury: args.treasury,
        promo_cap_bps: DEFAULT_PROMO_CAP_BPS,
        collateral_count: 0,
        reserved: [0; 128],
    });
    emit!(ConfigInitialized {
        admin,
        guardian: args.guardian,
        whitelister: args.whitelister,
        promo_signer: args.promo_signer,
        treasury: args.treasury,
    });
    Ok(())
}
