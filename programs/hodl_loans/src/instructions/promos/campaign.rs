use anchor_lang::prelude::*;

use crate::constants::{ACCOUNT_VERSION, CAMPAIGN_SEED, CONFIG_SEED, PROMO_VAULT_SEED};
use crate::errors::HodlError;
use crate::events::{CampaignClosed, CampaignCreated};
use crate::math::checked::{add, sub, to_u64};
use crate::state::{Campaign, Config, Market, PromoVault};

#[derive(Accounts)]
#[instruction(campaign_id: u64)]
pub struct CreateCampaign<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
    #[account(
        init,
        payer = admin,
        space = 8 + Campaign::INIT_SPACE,
        seeds = [CAMPAIGN_SEED, market.key().as_ref(), &campaign_id.to_le_bytes()],
        bump
    )]
    pub campaign: Box<Account<'info, Campaign>>,
    pub system_program: Program<'info, System>,
}

/// Spec §12. Reserves `budget` out of the promo vault's free cNGN, so every voucher the signer
/// issues against this campaign is already backed before it is redeemed. The reservation is what
/// `unissued` counts.
pub fn handle_create_campaign(
    ctx: Context<CreateCampaign>,
    campaign_id: u64,
    budget: u64,
    redeem_until: i64,
) -> Result<()> {
    require!(budget > 0, HodlError::AmountTooSmall);
    require!(redeem_until > Clock::get()?.unix_timestamp, HodlError::InvalidParameters);
    require!(budget <= ctx.accounts.promo_vault.free()?, HodlError::PromoVaultInsufficient);

    let promo_vault = &mut ctx.accounts.promo_vault;
    promo_vault.unissued = to_u64(add(promo_vault.unissued as u128, budget as u128)?)?;
    promo_vault.require_invariant()?;

    ctx.accounts.campaign.set_inner(Campaign {
        version: ACCOUNT_VERSION,
        bump: ctx.bumps.campaign,
        market: ctx.accounts.market.key(),
        campaign_id,
        budget,
        granted: 0,
        redeem_until,
        active: true,
        reserved: [0; 32],
    });
    emit!(CampaignCreated {
        market: ctx.accounts.market.key(),
        campaign: ctx.accounts.campaign.key(),
        campaign_id,
        budget,
        redeem_until,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct CloseCampaign<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
    #[account(
        mut,
        seeds = [CAMPAIGN_SEED, market.key().as_ref(), &campaign.campaign_id.to_le_bytes()],
        bump = campaign.bump,
        has_one = market
    )]
    pub campaign: Box<Account<'info, Campaign>>,
}

/// Spec §12. Hands the unspent part of the budget back to the vault's free cNGN. The account
/// stays so its `granted` total remains readable, and so vouchers already redeemed against it
/// keep a campaign to point at.
pub fn handle_close_campaign(ctx: Context<CloseCampaign>) -> Result<()> {
    require!(ctx.accounts.campaign.active, HodlError::CampaignInactive);
    let unspent = sub(ctx.accounts.campaign.budget as u128, ctx.accounts.campaign.granted as u128)?;

    let promo_vault = &mut ctx.accounts.promo_vault;
    promo_vault.unissued = to_u64(sub(promo_vault.unissued as u128, unspent)?)?;
    promo_vault.require_invariant()?;

    let campaign = &mut ctx.accounts.campaign;
    campaign.active = false;
    emit!(CampaignClosed {
        market: ctx.accounts.market.key(),
        campaign: campaign.key(),
        campaign_id: campaign.campaign_id,
        granted: campaign.granted,
        returned: unspent as u64,
    });
    Ok(())
}
