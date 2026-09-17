use anchor_lang::prelude::*;

use crate::constants::{ACCESS_SEED, POSITION_SEED};
use crate::errors::HodlError;
use crate::events::PositionClosed;
use crate::state::{Access, Position};

#[derive(Accounts)]
pub struct ClosePosition<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [ACCESS_SEED, owner.key().as_ref()], bump = access.bump)]
    pub access: Account<'info, Access>,
    #[account(
        mut,
        seeds = [POSITION_SEED, owner.key().as_ref()],
        bump,
        close = rent_payer,
        constraint = position.load()?.rent_payer == rent_payer.key() @ HodlError::Unauthorized
    )]
    pub position: AccountLoader<'info, Position>,
    /// CHECK: must equal `position.rent_payer`; only receives the rent refund.
    #[account(mut)]
    pub rent_payer: UncheckedAccount<'info>,
}

/// Requires no collateral and no active loans. Rent goes back to whoever paid it.
pub fn handle_close_position(ctx: Context<ClosePosition>) -> Result<()> {
    ctx.accounts.access.require_active()?;
    let position = ctx.accounts.position.load()?;
    require!(!position.has_collateral() && !position.has_active_loans(), HodlError::PositionNotEmpty);
    emit!(PositionClosed {
        position: ctx.accounts.position.key(),
        owner: position.owner,
        rent_payer: position.rent_payer,
    });
    Ok(())
}
