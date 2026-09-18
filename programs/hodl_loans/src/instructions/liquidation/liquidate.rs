use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{COLLATERAL_SEED, MARKET_SEED};
use crate::errors::HodlError;
use crate::events::{LoanLiquidated, LoanPartiallyLiquidated};
use crate::math::checked::{add, sub, to_u64};
use crate::math::liquidation::{principal_share, seize_for_repayment};
use crate::math::loan::{accrued_lp_interest, loan_balance, lp_contribution, reserve_share};
use crate::state::{CollateralAsset, Market, Position};
use crate::token::extensions::require_collateral_mint;
use crate::token::transfer::{transfer_from_user, transfer_from_vault};
use crate::valuation::load_valuation;

/// Open to anyone: no `Access` account, so a liquidation bot needs no whitelist.
#[derive(Accounts)]
pub struct Liquidate<'info> {
    pub liquidator: Signer<'info>,
    #[account(mut)]
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
    #[account(mut, token::mint = mint, token::authority = liquidator, token::token_program = token_program)]
    pub liquidator_token: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        seeds = [COLLATERAL_SEED, collateral_mint.key().as_ref()],
        bump = collateral.bump,
        constraint = collateral.vault == collateral_vault.key() @ HodlError::PriceAccountMismatch
    )]
    pub collateral: Box<Account<'info, CollateralAsset>>,
    #[account(mint::token_program = collateral_token_program)]
    pub collateral_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub collateral_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = collateral_mint,
        token::authority = liquidator,
        token::token_program = collateral_token_program
    )]
    pub liquidator_collateral: Box<InterfaceAccount<'info, TokenAccount>>,
    /// CHECK: address pinned to `market.ngn_feed`; parsed by `read_ngn_price`.
    pub ngn_feed: UncheckedAccount<'info>,
    pub token_program: Interface<'info, TokenInterface>,
    pub collateral_token_program: Interface<'info, TokenInterface>,
}

/// Spec §11 `liquidate`. `remaining_accounts`: per used collateral slot, in slot order, a
/// `(CollateralAsset, PriceUpdateV2)` pair — an `XStock` slot adds its mint as a third
/// account, the source of its scaled-UI multiplier — the whole position is priced, because
/// health decides whether it may be liquidated at all.
///
/// A late loan is not liquidatable on its own: only an unhealthy position is (spec §2).
/// Promo forfeiture (spec §11 step 3) arrives with Plan 5.
pub fn handle_liquidate<'info>(ctx: Context<'info, Liquidate<'info>>, loan_id: u64, amount: u64) -> Result<()> {
    require!(amount > 0, HodlError::AmountTooSmall);
    let market_key = ctx.accounts.market.key();
    let collateral_mint = ctx.accounts.collateral_mint.key();
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    let market = &mut ctx.accounts.market;
    market.accrue(now)?;

    let (paid, principal_repaid, interest_paid, seized, remaining_principal, owner) = {
        let mut position = ctx.accounts.position.load_mut()?;
        require_keys_eq!(position.market, market_key, HodlError::MarketMismatch);
        let loan_index = position.loan_index(loan_id).ok_or(HodlError::LoanNotFound)?;
        let slot_index = position
            .collateral_index(&collateral_mint)
            .ok_or(HodlError::InsufficientCollateral)?;

        let valuation = load_valuation(
            ctx.program_id,
            &position,
            market,
            &ctx.accounts.ngn_feed.to_account_info(),
            ctx.remaining_accounts,
            0,
            &clock,
        )?;
        require!(valuation.health.is_liquidatable(), HodlError::NotLiquidatable);
        // `load_valuation` returns one value per used slot, in slot order.
        let priced = position.collateral[..slot_index].iter().filter(|s| s.amount > 0).count();
        let collateral_price = valuation.collateral[priced].price.price;

        let loan = position.loans[loan_index];
        let balance = loan_balance(&loan.terms(), now)?.total()?;
        let requested = to_u64((amount as u128).min(balance))?;
        let seizure = seize_for_repayment(
            requested,
            valuation.ngn.price,
            market.decimals,
            collateral_price,
            ctx.accounts.collateral.decimals,
            ctx.accounts.collateral.liquidation_bonus_bps,
            position.collateral[slot_index].amount,
        )?;
        let paid = seizure.repay_amount;
        require!(paid > 0 && seizure.seize_amount > 0, HodlError::AmountTooSmall);

        let principal_repaid = principal_share(paid, loan.principal, balance)?;
        require!(principal_repaid > 0, HodlError::ZeroPrincipalRepaid);
        let interest_paid = to_u64(sub(paid as u128, principal_repaid as u128)?)?;

        // Spec §11 step 8: release the lender interest accrued for the repaid principal only.
        let released = accrued_lp_interest(
            principal_repaid,
            loan.rate_bps,
            loan.reserve_factor_bps,
            loan.interest_anchor,
            now,
        )?;
        market.accrued_interest = market.accrued_interest.saturating_sub(released);
        market.protocol_reserve = to_u64(add(
            market.protocol_reserve as u128,
            reserve_share(interest_paid as u128, loan.reserve_factor_bps)?,
        )?)?;
        market.total_borrows = market
            .total_borrows
            .checked_sub(principal_repaid)
            .ok_or(HodlError::MathOverflow)?;
        market.lp_rate_product = sub(
            market.lp_rate_product,
            lp_contribution(principal_repaid, loan.rate_bps, loan.reserve_factor_bps)?,
        )?;
        market.cash = market.cash.checked_add(paid).ok_or(HodlError::MathOverflow)?;

        // Spec §11 step 10: `interest_anchor` is not reset, so the clock keeps running.
        let slot = &mut position.loans[loan_index];
        slot.principal = slot.principal.checked_sub(principal_repaid).ok_or(HodlError::MathOverflow)?;
        slot.repaid = slot.repaid.checked_add(paid).ok_or(HodlError::MathOverflow)?;
        let remaining_principal = slot.principal;
        if remaining_principal == 0 {
            *slot = bytemuck::Zeroable::zeroed();
            if !position.has_active_loans() {
                position.promo_last_activity_at = now;
            }
        }

        let held = &mut position.collateral[slot_index];
        held.amount = held.amount.checked_sub(seizure.seize_amount).ok_or(HodlError::MathOverflow)?;
        (paid, principal_repaid, interest_paid, seizure.seize_amount, remaining_principal, position.owner)
    };

    let collateral = &mut ctx.accounts.collateral;
    collateral.total_deposited = collateral
        .total_deposited
        .checked_sub(seized)
        .ok_or(HodlError::MathOverflow)?;

    transfer_from_user(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.liquidator_token.to_account_info(),
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.liquidator.to_account_info(),
        paid,
    )?;
    require_collateral_mint(&ctx.accounts.collateral_mint.to_account_info(), ctx.accounts.collateral.kind)?;
    let seeds: &[&[u8]] = &[COLLATERAL_SEED, collateral_mint.as_ref(), &[ctx.accounts.collateral.bump]];
    transfer_from_vault(
        ctx.accounts.collateral_token_program.key(),
        ctx.accounts.collateral_mint.to_account_info(),
        ctx.accounts.collateral_mint.decimals,
        ctx.accounts.collateral_vault.to_account_info(),
        ctx.accounts.liquidator_collateral.to_account_info(),
        ctx.accounts.collateral.to_account_info(),
        seized,
        &[seeds],
    )?;

    let position_key = ctx.accounts.position.key();
    let liquidator = ctx.accounts.liquidator.key();
    if remaining_principal == 0 {
        emit!(LoanLiquidated {
            market: market_key,
            position: position_key,
            owner,
            liquidator,
            loan_id,
            amount: paid,
            principal_repaid,
            interest_paid,
            collateral_mint,
            collateral_seized: seized,
            remaining_principal,
        });
    } else {
        emit!(LoanPartiallyLiquidated {
            market: market_key,
            position: position_key,
            owner,
            liquidator,
            loan_id,
            amount: paid,
            principal_repaid,
            interest_paid,
            collateral_mint,
            collateral_seized: seized,
            remaining_principal,
        });
    }
    Ok(())
}
