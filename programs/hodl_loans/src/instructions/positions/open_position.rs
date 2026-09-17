use anchor_lang::prelude::*;

use crate::constants::{ACCESS_SEED, ACCOUNT_VERSION, POSITION_SEED};
use crate::events::PositionOpened;
use crate::state::{Access, Position};

#[derive(Accounts)]
pub struct OpenPosition<'info> {
    /// Pays the position's rent (e.g. the gas-relay sponsor) and is refunded on close.
    #[account(mut)]
    pub payer: Signer<'info>,
    pub owner: Signer<'info>,
    #[account(seeds = [ACCESS_SEED, owner.key().as_ref()], bump = access.bump)]
    pub access: Account<'info, Access>,
    #[account(
        init,
        payer = payer,
        space = 8 + std::mem::size_of::<Position>(),
        seeds = [POSITION_SEED, owner.key().as_ref()],
        bump
    )]
    pub position: AccountLoader<'info, Position>,
    pub system_program: Program<'info, System>,
}

pub fn handle_open_position(ctx: Context<OpenPosition>) -> Result<()> {
    ctx.accounts.access.require_active()?;
    let mut position = ctx.accounts.position.load_init()?;
    position.version = ACCOUNT_VERSION;
    position.bump = ctx.bumps.position;
    position.owner = ctx.accounts.owner.key();
    position.rent_payer = ctx.accounts.payer.key();
    emit!(PositionOpened {
        position: ctx.accounts.position.key(),
        owner: position.owner,
        rent_payer: position.rent_payer,
    });
    Ok(())
}
