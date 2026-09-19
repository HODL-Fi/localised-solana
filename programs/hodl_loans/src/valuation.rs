use anchor_lang::prelude::*;

use crate::errors::HodlError;
use crate::math::checked::add;
use crate::math::health::{compute_health, CollateralValue, Health};
use crate::math::price::UsdPrice;
use crate::math::loan::loan_balance;
use crate::oracle::pyth::read_pyth_price;
use crate::oracle::switchboard::read_ngn_price;
use crate::constants::MULTIPLIER_ONE;
use crate::state::{CollateralAsset, CollateralKind, Market, Position};
use crate::token::scaled_ui::read_xstock_multiplier;

/// Accounts per used collateral slot in `remaining_accounts`: `(CollateralAsset, PriceUpdateV2)`
/// for a `Standard` asset, and `(CollateralAsset, PriceUpdateV2, mint)` for an `XStock`, whose
/// scaled-UI multiplier lives on the mint.
pub const ACCOUNTS_PER_COLLATERAL: usize = 2;
pub const ACCOUNTS_PER_XSTOCK: usize = 3;

/// Value every used collateral slot, in slot order, from `remaining` pairs.
///
/// The loop runs over the position's slots, not over the accounts supplied, so a missing,
/// extra or mismatched account fails with `PriceAccountMismatch` instead of skipping collateral.
/// An asset with a pinned `price_account` accepts only that account, so the caller cannot
/// choose among the verified updates inside the asset's age window.
pub fn load_collateral_values(
    program_id: &Pubkey,
    position: &Position,
    remaining: &[AccountInfo],
    clock: &Clock,
) -> Result<Vec<CollateralValue>> {
    let used: Vec<_> = position.collateral.iter().filter(|s| s.amount > 0).collect();
    let mut values = Vec::with_capacity(used.len());
    let mut cursor = 0usize;
    for slot in used.iter() {
        let asset_info = remaining.get(cursor).ok_or(HodlError::PriceAccountMismatch)?;
        let price_info = remaining.get(cursor + 1).ok_or(HodlError::PriceAccountMismatch)?;
        require_keys_eq!(*asset_info.owner, *program_id, HodlError::PriceAccountMismatch);
        let asset = {
            let data = asset_info.try_borrow_data()?;
            CollateralAsset::try_deserialize(&mut &data[..]).map_err(|_| HodlError::PriceAccountMismatch)?
        };
        require_keys_eq!(asset.mint, slot.mint, HodlError::PriceAccountMismatch);
        if asset.price_account != Pubkey::default() {
            require_keys_eq!(price_info.key(), asset.price_account, HodlError::PriceAccountMismatch);
        }
        let price = read_pyth_price(
            price_info,
            &asset.pyth_feed_id,
            asset.max_price_age_seconds,
            asset.max_conf_bps,
            clock,
        )?;
        // An xStock passes its mint too: the multiplier its issuer applies to balances lives
        // there, and Pyth prices the display token, not the raw unit.
        // The match is exhaustive, so a third `CollateralKind` has to state its own stride and
        // multiplier here rather than silently inherit the xStock's.
        let (stride, multiplier) = match asset.kind {
            CollateralKind::Standard => (ACCOUNTS_PER_COLLATERAL, MULTIPLIER_ONE),
            CollateralKind::XStock => {
                let mint_info = remaining.get(cursor + 2).ok_or(HodlError::PriceAccountMismatch)?;
                require_keys_eq!(mint_info.key(), slot.mint, HodlError::PriceAccountMismatch);
                (ACCOUNTS_PER_XSTOCK, read_xstock_multiplier(mint_info, clock.unix_timestamp)?)
            }
        };
        cursor += stride;
        values.push(CollateralValue {
            amount: slot.amount,
            decimals: asset.decimals,
            multiplier,
            price,
            ltv_bps: asset.ltv_bps,
            liquidation_threshold_bps: asset.liquidation_threshold_bps,
        });
    }
    // Every account supplied must have been consumed: an extra one is a mismatch.
    require!(cursor == remaining.len(), HodlError::PriceAccountMismatch);
    Ok(values)
}

/// Total cNGN owed across active loans at `now` (principal + interest + penalty).
pub fn total_debt(position: &Position, now: i64) -> Result<u128> {
    let mut total = 0u128;
    for loan in position.loans.iter().filter(|l| l.is_active()) {
        total = add(total, loan_balance(&loan.terms(), now)?.total()?)?;
    }
    Ok(total)
}

/// Everything one price read of a position yields: its health, the per-slot values behind it
/// (in used-slot order), and the cNGN price. Liquidation needs the values and the price;
/// borrowing and withdrawing need only the health.
pub struct Valuation {
    pub health: Health,
    pub collateral: Vec<CollateralValue>,
    pub ngn: UsdPrice,
}

/// Spec §8 valuation of a position, optionally including `extra_debt` about to be borrowed.
/// Everything a health check needs besides the position itself. Grouped rather than passed
/// positionally: the list grew past what a reader can keep straight, and `extra_debt` and
/// `promo_cap_bps` are both small integers that would transpose silently.
pub struct ValuationRequest<'a, 'info> {
    pub program_id: &'a Pubkey,
    pub market: &'a Market,
    pub ngn_feed: &'a AccountInfo<'info>,
    pub remaining: &'a [AccountInfo<'info>],
    /// Debt to count on top of the position's own — the loan `take_loan` is about to write.
    pub extra_debt: u64,
    pub promo_cap_bps: u16,
    pub clock: &'a Clock,
}

/// Spec §8 valuation of a position: loads each used collateral's price, reads the NGN price,
/// totals debt (plus any `extra_debt` about to be borrowed), and computes health from all three.
pub fn load_valuation(position: &Position, request: &ValuationRequest) -> Result<Valuation> {
    let ValuationRequest { program_id, market, ngn_feed, remaining, extra_debt, promo_cap_bps, clock } = *request;
    let collateral = load_collateral_values(program_id, position, remaining, clock)?;
    let ngn = read_ngn_price(ngn_feed, market, clock)?;
    let debt = add(total_debt(position, clock.unix_timestamp)?, extra_debt as u128)?;
    let health = compute_health(
        &collateral,
        debt,
        market.decimals,
        ngn,
        position.promo_balance,
        promo_cap_bps,
    )?;
    Ok(Valuation { health, collateral, ngn })
}

/// Spec §8 health for a position, optionally including `extra_debt` about to be borrowed.
pub fn load_health(position: &Position, request: &ValuationRequest) -> Result<Health> {
    Ok(load_valuation(position, request)?.health)
}
