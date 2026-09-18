use anchor_lang::prelude::*;

/// Basis-point denominator used in all u128 math.
pub const BPS: u128 = 10_000;
/// Largest valid basis-point parameter (100%).
pub const MAX_BPS: u16 = 10_000;
/// Seconds in a year for interest math.
pub const YEAR: u128 = 31_536_000;
/// Virtual share and asset offsets that protect the first lender deposit.
pub const VIRTUAL_SHARES: u128 = 1_000;
pub const VIRTUAL_ASSETS: u128 = 1;
/// Shortest loan tenure.
pub const MIN_TENURE: i64 = 86_400;
pub const MAX_COLLATERAL_SLOTS: usize = 8;
pub const MAX_LOAN_SLOTS: usize = 10;
/// Layout version written into every account.
pub const ACCOUNT_VERSION: u8 = 1;

pub const DEFAULT_PROMO_CAP_BPS: u16 = 2_000;
/// Fixed-point scale for USD values in price and health math (10^12 per dollar).
/// 10^12 rather than 10^18 keeps `amount × price` inside u128 for any realistic balance.
pub const USD_SCALE: u128 = 1_000_000_000_000;
/// Decimal exponent of `USD_SCALE`.
pub const USD_DECIMALS: i32 = 12;

/// Longest price age an admin may configure for a collateral asset (spec §8).
/// A caller can pick any verified Pyth update inside this window, so the window is the
/// price-selection surface for assets without a pinned price account.
pub const MAX_PRICE_AGE_SECONDS: u64 = 60;

#[constant]
pub const CONFIG_SEED: &[u8] = b"config";
#[constant]
pub const ACCESS_SEED: &[u8] = b"access";
#[constant]
pub const MARKET_SEED: &[u8] = b"market";
#[constant]
pub const MARKET_VAULT_SEED: &[u8] = b"market_vault";
#[constant]
pub const LENDER_SEED: &[u8] = b"lender";
#[constant]
pub const COLLATERAL_SEED: &[u8] = b"collateral";
#[constant]
pub const COLLATERAL_VAULT_SEED: &[u8] = b"collateral_vault";
#[constant]
pub const POSITION_SEED: &[u8] = b"position";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_are_distinct_and_bps_matches() {
        let seeds = [
            CONFIG_SEED,
            ACCESS_SEED,
            MARKET_SEED,
            MARKET_VAULT_SEED,
            LENDER_SEED,
            COLLATERAL_SEED,
            COLLATERAL_VAULT_SEED,
            POSITION_SEED,
        ];
        for (i, a) in seeds.iter().enumerate() {
            for b in &seeds[i + 1..] {
                assert_ne!(a, b);
            }
        }
        assert_eq!(BPS, MAX_BPS as u128);
        assert_eq!(USD_SCALE, 10u128.pow(USD_DECIMALS as u32));
    }
}
