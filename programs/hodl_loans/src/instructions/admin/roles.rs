use anchor_lang::prelude::*;

use crate::events::{Role, RoleUpdated};
use crate::instructions::admin::admin_transfer::AdminConfig;

/// Shared body of `set_guardian`, `set_whitelister`, `set_promo_signer` and `set_treasury`.
pub fn handle_set_role(ctx: Context<AdminConfig>, role: Role, new: Pubkey) -> Result<()> {
    let config = &mut ctx.accounts.config;
    let slot = match role {
        Role::Guardian => &mut config.guardian,
        Role::Whitelister => &mut config.whitelister,
        Role::PromoSigner => &mut config.promo_signer,
        Role::Treasury => &mut config.treasury,
    };
    let old = *slot;
    *slot = new;
    emit!(RoleUpdated { role, old, new });
    Ok(())
}
