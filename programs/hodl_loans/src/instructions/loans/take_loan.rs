use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{ACCESS_SEED, BPS, CONFIG_SEED, MARKET_SEED, MIN_TENURE, POSITION_SEED, PROMO_VAULT_SEED};
use crate::errors::HodlError;
use crate::events::{LoanOpened, PromoExpired};
use crate::instructions::promos::release_promo;
use crate::math::checked::add;
use crate::math::loan::lp_contribution;
use crate::state::{Access, Config, LoanSlot, Market, Position, PromoVault};
use crate::token::transfer::transfer_from_vault;
use crate::valuation::{load_health, ValuationRequest};

#[derive(Accounts)]
pub struct TakeLoan<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [ACCESS_SEED, owner.key().as_ref()], bump = access.bump)]
    pub access: Account<'info, Access>,
    /// Carries `promo_cap_bps`, which bounds how much of a position's promo counts (spec §12).
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    /// Required when the position holds promo: step 3 may expire it, which credits the vault.
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market
    )]
    pub promo_vault: Option<Box<Account<'info, PromoVault>>>,
    #[account(mut, seeds = [POSITION_SEED, owner.key().as_ref()], bump)]
    pub position: AccountLoader<'info, Position>,
    #[account(
        mut,
        seeds = [MARKET_SEED, mint.key().as_ref()],
        bump = market.bump,
        has_one = mint,
        has_one = vault,
        has_one = ngn_feed @ HodlError::PriceAccountMismatch
    )]
    pub market: Box<Account<'info, Market>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, token::mint = mint, token::authority = owner, token::token_program = token_program)]
    pub owner_token: Box<InterfaceAccount<'info, TokenAccount>>,
    /// CHECK: address pinned to `market.ngn_feed`; parsed by `read_ngn_price`.
    pub ngn_feed: UncheckedAccount<'info>,
    pub token_program: Interface<'info, TokenInterface>,
}

/// Spec §10 `take_loan`. `remaining_accounts`: per used collateral slot, in slot order, a
/// `(CollateralAsset, PriceUpdateV2)` pair — an `XStock` slot adds its mint as a third
/// account, the source of its scaled-UI multiplier.
pub fn handle_take_loan<'info>(
    ctx: Context<'info, TakeLoan<'info>>,
    amount: u64,
    tenure_seconds: i64,
) -> Result<()> {
    ctx.accounts.access.require_active()?;
    let market_key = ctx.accounts.market.key();
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    let market = &mut ctx.accounts.market;
    require!(!market.paused, HodlError::MarketPaused);
    require!(amount > 0 && amount >= market.min_loan_amount, HodlError::AmountTooSmall);
    require!(
        (MIN_TENURE..=market.max_tenure_seconds).contains(&tenure_seconds),
        HodlError::TenureOutOfRange
    );

    // Spec §10 puts both position preconditions in steps 1-2, before step 3 accrues. Taken
    // later, a borrow against the wrong market at full utilization reported
    // `UtilizationCapExceeded` — true of the market, but not what was wrong with the call.
    // Only the reported error changes: both are read-only, and neither can succeed here and
    // fail below, because nothing between the two points writes to the position.
    let free_index = {
        let position = ctx.accounts.position.load()?;
        require!(
            position.market == Pubkey::default() || position.market == market_key,
            HodlError::MarketMismatch
        );
        position.free_loan_index().ok_or(HodlError::NoFreeLoanSlot)?
    };

    market.accrue(now)?;

    let available = market.available_cash();
    require!(amount <= available, HodlError::InsufficientCash);
    let lendable = add(available as u128, market.total_borrows as u128)?;
    let borrows_after = add(market.total_borrows as u128, amount as u128)?;
    require!(
        borrows_after * BPS <= lendable * market.max_utilization_bps as u128,
        HodlError::UtilizationCapExceeded
    );

    let (loan_id, rate_bps, penalty_rate_bps, reserve_factor_bps) = {
        let mut position = ctx.accounts.position.load_mut()?;
        let index = free_index;

        // Spec §10 step 3: a quiet position's promo expires before it can support a new loan.
        // `saturating_add` can only push the deadline later (never wrap it earlier), so this
        // fails safe on overflow — it depends on `MarketParams::validate` requiring
        // `promo_inactivity_seconds > 0`, so the two must not drift apart.
        if position.promo_balance > 0
            && !position.has_active_loans()
            && now >= position.promo_last_activity_at.saturating_add(market.promo_inactivity_seconds)
        {
            let promo_vault = ctx
                .accounts
                .promo_vault
                .as_mut()
                .ok_or(HodlError::PromoAccountsRequired)?;
            let amount = release_promo(&mut position, promo_vault)?;
            emit!(PromoExpired {
                market: market_key,
                position: ctx.accounts.position.key(),
                owner: position.owner,
                amount,
            });
        }

        let ngn_feed = ctx.accounts.ngn_feed.to_account_info();
        let health = load_health(
            &position,
            &ValuationRequest {
                program_id: ctx.program_id,
                market,
                ngn_feed: &ngn_feed,
                remaining: ctx.remaining_accounts,
                extra_debt: amount,
                promo_cap_bps: ctx.accounts.config.promo_cap_bps,
                clock: &clock,
            },
        )?;
        require!(health.is_healthy(), HodlError::Unhealthy);

        let loan_id = position.next_loan_id;
        position.next_loan_id = loan_id.checked_add(1).ok_or(HodlError::MathOverflow)?;
        position.loans[index] = LoanSlot {
            id: loan_id,
            principal: amount,
            original_principal: amount,
            repaid: 0,
            originated_at: now,
            interest_anchor: now,
            tenure_seconds,
            rate_bps: market.interest_rate_bps,
            penalty_rate_bps: market.penalty_rate_bps,
            reserve_factor_bps: market.reserve_factor_bps,
            active: 1,
            _padding: 0,
        };
        position.market = market_key;
        position.promo_last_activity_at = now;
        (loan_id, market.interest_rate_bps, market.penalty_rate_bps, market.reserve_factor_bps)
    };

    market.total_borrows = market.total_borrows.checked_add(amount).ok_or(HodlError::MathOverflow)?;
    market.lp_rate_product = add(
        market.lp_rate_product,
        lp_contribution(amount, rate_bps, reserve_factor_bps)?,
    )?;
    market.cash = market.cash.checked_sub(amount).ok_or(HodlError::MathOverflow)?;

    let mint_key = ctx.accounts.mint.key();
    let seeds: &[&[u8]] = &[MARKET_SEED, mint_key.as_ref(), &[ctx.accounts.market.bump]];
    transfer_from_vault(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.owner_token.to_account_info(),
        ctx.accounts.market.to_account_info(),
        amount,
        &[seeds],
    )?;
    emit!(LoanOpened {
        market: market_key,
        position: ctx.accounts.position.key(),
        owner: ctx.accounts.owner.key(),
        loan_id,
        principal: amount,
        tenure_seconds,
        rate_bps,
        penalty_rate_bps,
        reserve_factor_bps,
        originated_at: now,
    });
    Ok(())
}
