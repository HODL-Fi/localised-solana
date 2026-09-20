use anchor_lang::prelude::*;

use crate::constants::{COLLATERAL_SEED, CONFIG_SEED, MAX_BPS};
use crate::errors::HodlError;
use crate::events::PromoCapSet;
use crate::state::{CollateralAsset, Config};

#[derive(Accounts)]
pub struct SetPromoCap<'info> {
    pub admin: Signer<'info>,
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
}

/// Spec §8 and §12. The cap bounds how much promo counts against a borrower's own collateral,
/// so raising it eats into the gap between every asset's LTV and its liquidation threshold —
/// the gap that keeps a fully drawn position solvent. Every listed asset is re-checked against
/// the new value before it takes effect.
///
/// Lowering the cap affects existing positions immediately, the same class of accepted admin
/// power as lowering an LTV or liquidation threshold (`collateral_admin.rs`): it reduces
/// `promo_counted`, which reduces `liquidation_line` for every promo-holding position at once. A
/// borrower whose debt has accrued above their borrow limit but below their liquidation line
/// becomes liquidatable in this same transaction, with no price movement.
///
/// `remaining_accounts` carries every `CollateralAsset`, **in ascending key order**. The count
/// must equal `config.collateral_count` and the keys must strictly increase, which together
/// rule out the mistake a bare count check allows: passing one permissive asset several times
/// and leaving the rest unexamined.
pub fn handle_set_promo_cap(ctx: Context<SetPromoCap>, promo_cap_bps: u16) -> Result<()> {
    require!(promo_cap_bps <= MAX_BPS, HodlError::InvalidParameters);
    require!(
        ctx.remaining_accounts.len() == ctx.accounts.config.collateral_count as usize,
        HodlError::InvalidParameters
    );

    let mut previous = Pubkey::default();
    for info in ctx.remaining_accounts {
        require_keys_eq!(*info.owner, *ctx.program_id, HodlError::InvalidParameters);
        require!(info.key() > previous, HodlError::InvalidParameters);
        previous = info.key();

        let data = info.try_borrow_data()?;
        let asset = CollateralAsset::try_deserialize(&mut &data[..])
            .map_err(|_| HodlError::InvalidParameters)?;
        // The asset account must be the one the program derives for its own mint, so a
        // look-alike cannot stand in for a stricter asset.
        let expected = Pubkey::create_program_address(
            &[COLLATERAL_SEED, asset.mint.as_ref(), &[asset.bump]],
            ctx.program_id,
        )
        .map_err(|_| HodlError::InvalidParameters)?;
        require_keys_eq!(info.key(), expected, HodlError::InvalidParameters);

        // Re-run the same rule `CollateralParams::validate` enforces, instead of hand-copying
        // it: `state/collateral.rs` already documents that a future editor changing
        // `liquidation_bonus_bps` must reason about `LT + promo_cap_bps`, so `validate` is
        // where a second cap-dependent rule will land. A hand-copied `require!` here would
        // silently stop covering it. This is safe and idempotent — `apply_params` is the sole
        // writer of `ltv_bps`/`liquidation_threshold_bps`/`liquidation_bonus_bps` and validates
        // at both of its call sites, so every stored asset already satisfies the
        // cap-independent rules `validate` also re-checks here.
        asset.params().validate(promo_cap_bps)?;
    }

    let by = ctx.accounts.admin.key();
    let config = &mut ctx.accounts.config;
    let old = config.promo_cap_bps;
    config.promo_cap_bps = promo_cap_bps;
    emit!(PromoCapSet { old, new: promo_cap_bps, by });
    Ok(())
}
