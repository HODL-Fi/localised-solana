use anchor_lang::prelude::*;
use solana_instructions_sysvar::ID as INSTRUCTIONS_SYSVAR_ID;

use crate::constants::{ACCESS_SEED, ACCOUNT_VERSION, CAMPAIGN_SEED, CONFIG_SEED, POSITION_SEED, PROMO_VAULT_SEED, VOUCHER_SEED};
use crate::errors::HodlError;
use crate::events::PromoRedeemed;
use crate::math::checked::{add, sub, to_u64};
use crate::state::{Access, Campaign, Config, Market, Position, PromoVault, VoucherReceipt};
use crate::voucher::{require_ed25519_signature, PromoVoucher};

#[derive(Accounts)]
#[instruction(amount: u64, nonce: u64)]
pub struct RedeemPromo<'info> {
    /// Pays the receipt's rent (e.g. the gas-relay sponsor) and is refunded when it is closed.
    #[account(mut)]
    pub payer: Signer<'info>,
    pub owner: Signer<'info>,
    #[account(seeds = [ACCESS_SEED, owner.key().as_ref()], bump = access.bump)]
    pub access: Box<Account<'info, Access>>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    pub market: Box<Account<'info, Market>>,
    #[account(mut, seeds = [POSITION_SEED, owner.key().as_ref()], bump)]
    pub position: AccountLoader<'info, Position>,
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
    /// Its creation is what stops a voucher being redeemed twice while this receipt lives: the
    /// second attempt cannot initialize an account that already exists. The PDA seeds
    /// `[VOUCHER_SEED, campaign, nonce]` do NOT include `voucher_expiry`, so the on-chain
    /// guarantee is "one redemption per (campaign, nonce) per expiry epoch", not per
    /// (campaign, nonce) outright — once the receipt is closed past expiry, only backend nonce
    /// discipline (never reissuing the same nonce with a later expiry) prevents a replay.
    #[account(
        init,
        payer = payer,
        space = 8 + VoucherReceipt::INIT_SPACE,
        seeds = [VOUCHER_SEED, campaign.key().as_ref(), &nonce.to_le_bytes()],
        bump
    )]
    pub voucher_receipt: Box<Account<'info, VoucherReceipt>>,
    /// CHECK: the address is the only thing that matters; the sysvar's contents are read
    /// through `load_instruction_at_checked`, which validates the layout itself.
    #[account(address = INSTRUCTIONS_SYSVAR_ID)]
    pub instructions_sysvar: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

/// Spec §12 `redeem_promo`. Turns a voucher the promo signer issued off-chain into promo
/// borrowing power on the caller's position, against a campaign's reserved budget.
///
/// Nothing moves: the cNGN backing the promo is already in the vault, and redeeming only moves
/// it from `unissued` (promised to a campaign) to `outstanding` (promised to a position).
pub fn handle_redeem_promo(
    ctx: Context<RedeemPromo>,
    amount: u64,
    nonce: u64,
    voucher_expiry: i64,
) -> Result<()> {
    ctx.accounts.access.require_active()?;
    // Redemption draws down the promo vault and raises borrowing power, so it is paused
    // alongside `take_loan` / `deposit_collateral` — operations that reduce exposure
    // (`repay_loan`, `withdraw_collateral`, `close_voucher_receipt`) stay open under a pause.
    require!(!ctx.accounts.market.paused, HodlError::MarketPaused);
    require!(amount > 0, HodlError::AmountTooSmall);
    let now = Clock::get()?.unix_timestamp;

    // The program builds the message it expects and requires that exact signature, so the
    // wallet, amount, nonce and expiry below are the ones the signer actually authorised.
    let voucher = PromoVoucher::new(
        ctx.accounts.market.key(),
        ctx.accounts.campaign.campaign_id,
        ctx.accounts.owner.key(),
        amount,
        nonce,
        voucher_expiry,
    );
    require_ed25519_signature(
        &ctx.accounts.instructions_sysvar.to_account_info(),
        &ctx.accounts.config.promo_signer,
        &voucher.message()?,
    )?;

    require!(now <= voucher_expiry, HodlError::VoucherExpired);
    require!(ctx.accounts.campaign.active, HodlError::CampaignInactive);
    require!(now <= ctx.accounts.campaign.redeem_until, HodlError::CampaignInactive);

    // A voucher may not outlive the campaign it draws on. Without this the promo signer alone
    // decides how long the receipt this redemption creates holds its rent: expiry is not part
    // of the receipt's seeds, so `close_voucher_receipt` waits for `now > voucher_expiry` and
    // a far-future expiry locks the rent indefinitely. Tying it to `redeem_until` — itself
    // bounded by `MAX_CAMPAIGN_LIFETIME` — puts that bound back under admin control, where
    // the campaign's budget and lifetime already sit.
    //
    // This is a real behavioural change, not a free one. `voucher_expiry` and `now` are
    // different quantities: a voucher expiring after `redeem_until` is perfectly redeemable
    // at any `now <= redeem_until`, and both prior checks pass for it. A backend issuing
    // rolling 30-day vouchers signs such a voucher every day of a campaign's last 30, and
    // every one of them stops working here. Those vouchers must be re-signed with expiries
    // clamped to the campaign — the migration note for this change.
    require!(voucher_expiry <= ctx.accounts.campaign.redeem_until, HodlError::VoucherOutlivesCampaign);

    let granted = add(ctx.accounts.campaign.granted as u128, amount as u128)?;
    require!(granted <= ctx.accounts.campaign.budget as u128, HodlError::CampaignBudgetExceeded);

    {
        let position = ctx.accounts.position.load()?;
        require_keys_eq!(position.owner, ctx.accounts.owner.key(), HodlError::Unauthorized);
        // A position is bound to one market from its first loan; promo is market-scoped too.
        if position.market != Pubkey::default() {
            require_keys_eq!(position.market, ctx.accounts.market.key(), HodlError::MarketMismatch);
        }
        let balance = add(position.promo_balance as u128, amount as u128)?;
        require!(
            balance <= ctx.accounts.market.max_promo_per_position as u128,
            HodlError::PromoCapExceeded
        );
    }

    ctx.accounts.campaign.granted = to_u64(granted)?;
    let promo_vault = &mut ctx.accounts.promo_vault;
    promo_vault.unissued = to_u64(sub(promo_vault.unissued as u128, amount as u128)?)?;
    promo_vault.outstanding = to_u64(add(promo_vault.outstanding as u128, amount as u128)?)?;
    promo_vault.require_invariant()?;

    ctx.accounts.voucher_receipt.set_inner(VoucherReceipt {
        version: ACCOUNT_VERSION,
        bump: ctx.bumps.voucher_receipt,
        campaign: ctx.accounts.campaign.key(),
        nonce,
        voucher_expiry,
        rent_payer: ctx.accounts.payer.key(),
        reserved: [0; 32],
    });

    let mut position = ctx.accounts.position.load_mut()?;
    // Promo is backed by one market's vault and counted against its cap, so redeeming binds the
    // position to that market exactly as a first loan would — and `expire_promo` needs to know
    // which vault to credit when the position has never borrowed.
    position.market = ctx.accounts.market.key();
    position.promo_balance = to_u64(add(position.promo_balance as u128, amount as u128)?)?;
    position.promo_last_activity_at = now;
    emit!(PromoRedeemed {
        market: ctx.accounts.market.key(),
        position: ctx.accounts.position.key(),
        owner: position.owner,
        campaign_id: ctx.accounts.campaign.campaign_id,
        nonce,
        amount,
        promo_balance: position.promo_balance,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct CloseVoucherReceipt<'info> {
    /// CHECK: only receives the rent it paid; the receipt names it.
    #[account(mut, address = voucher_receipt.rent_payer)]
    pub rent_payer: UncheckedAccount<'info>,
    #[account(mut, close = rent_payer)]
    pub voucher_receipt: Box<Account<'info, VoucherReceipt>>,
}

/// Spec §12. Once a voucher can no longer be redeemed, its receipt has nothing left to prevent,
/// so anyone may close it and return the rent to whoever paid it.
pub fn handle_close_voucher_receipt(ctx: Context<CloseVoucherReceipt>) -> Result<()> {
    // Must stay strict `>`: `redeem_promo` allows `now <= voucher_expiry`, so these two checks
    // are exact complements. A `>=` here would let the boundary second (`now == voucher_expiry`)
    // satisfy both guards at once, so a single transaction could loop
    // [redeem, close, redeem, close, ...] and replay the same voucher repeatedly, since the
    // Ed25519 instruction is never consumed and each close frees the receipt PDA for the next
    // `init`.
    require!(
        Clock::get()?.unix_timestamp > ctx.accounts.voucher_receipt.voucher_expiry,
        HodlError::PromoNotExpired
    );
    Ok(())
}
