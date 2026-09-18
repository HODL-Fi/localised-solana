use anchor_lang::prelude::*;

use crate::errors::HodlError;
use crate::math::checked::add;
use crate::math::health::{compute_health, CollateralValue, Health};
use crate::math::price::UsdPrice;
use crate::math::loan::loan_balance;
use crate::oracle::pyth::read_pyth_price;
use crate::oracle::switchboard::read_ngn_price;
use crate::state::{CollateralAsset, Market, Position};

/// Accounts per used collateral slot in `remaining_accounts`: `(CollateralAsset, PriceUpdateV2)`.
pub const ACCOUNTS_PER_COLLATERAL: usize = 2;

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
    require!(
        remaining.len() == used.len() * ACCOUNTS_PER_COLLATERAL,
        HodlError::PriceAccountMismatch
    );
    let mut values = Vec::with_capacity(used.len());
    for (slot, accounts) in used.iter().zip(remaining.chunks(ACCOUNTS_PER_COLLATERAL)) {
        let (asset_info, price_info) = (&accounts[0], &accounts[1]);
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
        values.push(CollateralValue {
            amount: slot.amount,
            decimals: asset.decimals,
            price,
            ltv_bps: asset.ltv_bps,
            liquidation_threshold_bps: asset.liquidation_threshold_bps,
        });
    }
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
pub fn load_valuation(
    program_id: &Pubkey,
    position: &Position,
    market: &Market,
    ngn_feed: &AccountInfo,
    remaining: &[AccountInfo],
    extra_debt: u64,
    clock: &Clock,
) -> Result<Valuation> {
    let collateral = load_collateral_values(program_id, position, remaining, clock)?;
    let ngn = read_ngn_price(ngn_feed, market, clock)?;
    let debt = add(total_debt(position, clock.unix_timestamp)?, extra_debt as u128)?;
    let health = compute_health(&collateral, debt, market.decimals, ngn)?;
    Ok(Valuation { health, collateral, ngn })
}

/// Spec §8 health for a position, optionally including `extra_debt` about to be borrowed.
pub fn load_health(
    program_id: &Pubkey,
    position: &Position,
    market: &Market,
    ngn_feed: &AccountInfo,
    remaining: &[AccountInfo],
    extra_debt: u64,
    clock: &Clock,
) -> Result<Health> {
    Ok(load_valuation(program_id, position, market, ngn_feed, remaining, extra_debt, clock)?.health)
}
