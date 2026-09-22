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

/// Upper bound on a collateral mint's `decimals`, set by the LIQUIDATION path rather than the
/// health path.
///
/// `token_value` tolerates roughly 38 decimals before `u128` gives out, which is the number a
/// reader reaches for. `seize_for_repayment` is far tighter, because it multiplies twice:
/// `with_bonus × 10^decimals` first, then `display × MULTIPLIER_SCALE`. The second term carries
/// the collateral price in its denominator, so a cheap asset overflows sooner. Measured against
/// the worst repayment the program permits — `u64::MAX` cNGN, a 100% liquidation bonus, a $0.01
/// collateral, and today's NGN price ($0.000625) — the first failure is at 15 decimals.
///
/// 12 clears every asset the protocol lists (USDC 6, SOL 9, the xStocks 8) with three decimals
/// of margin against that measured cliff. Past the bound the failure is not a clean liquidator
/// lockout: the overflow scales linearly in `repay_amount`, and a liquidator picks `amount`
/// itself, so it can route around the overflow by repaying less. Liquidation degrades into
/// chunked repayments from roughly 13 to 20 decimals and only becomes genuinely impossible past
/// about 22. The bound exists to make the failure loud at listing time, where it's a cheap,
/// one-time refusal, instead of surprising a liquidator deep in the liquidation path — and to
/// keep every `u128` headroom argument elsewhere in the program valid.
pub const MAX_COLLATERAL_DECIMALS: u8 = 12;
/// Upper bound on `MarketParams::ngn_max_stale_slots`. At the 400 ms slot target, 150 slots is
/// the same 60 seconds `MAX_PRICE_AGE_SECONDS` allows the collateral feeds; mainnet slots run
/// 400-650 ms under load, so in practice this is closer to 60-100 seconds — the NGN feed prices
/// the debt side of every health check and can drift further behind than the collateral feeds.
pub const MAX_NGN_STALE_SLOTS: u64 = 150;

/// Fixed-point scale for an xStock's scaled-UI multiplier (10^12 per whole multiple).
pub const MULTIPLIER_SCALE: u128 = 1_000_000_000_000;
/// The multiplier a `Standard` asset always carries.
pub const MULTIPLIER_ONE: u128 = MULTIPLIER_SCALE;
/// Largest multiplier an xStock mint may declare. A corporate action moves it by small
/// factors; anything beyond this is a misconfigured or hostile mint.
///
/// **Where the arithmetic ceiling actually is.** `math::price::token_value` multiplies before
/// it divides, so `display_amount × price` has to stay inside `u128` (≈3.4 × 10^38).
/// `display_amount` is at most `deposit_cap × MAX_MULTIPLIER / MULTIPLIER_SCALE`, so with this
/// cap the real constraint is roughly `deposit_cap × 10^6 × price < 3.4 × 10^38`, with `price`
/// at `USD_SCALE`. Inside that an asset is clear; past it the health check fails closed with
/// `MathOverflow`. `math::liquidation::seize_for_repayment` has a tighter ceiling of its own —
/// see the note there.
///
/// **Deliberately not tightened.** A lower cap looks like cheap protection against the
/// scaled-UI authority (spec §14) and is not: an effective multiplier above the cap makes
/// `read_xstock_multiplier` return `InvalidPrice`, which fails *every* health check that
/// touches the asset — borrow, withdraw against a loan, liquidate, write off — and seals the
/// position the same way an over-eager exit check would. That trades a remote economic risk
/// for a more likely liveness failure. The shape that bounds the authority without that cost
/// is a per-asset, admin-settable ceiling which withholds *borrowing power* instead of
/// rejecting the price: `CollateralAsset::max_multiplier`, added in Plan 7. This constant
/// stays loose on purpose — it is the arithmetic backstop, not the policy knob.
pub const MAX_MULTIPLIER: u128 = 1_000_000 * MULTIPLIER_SCALE;

/// Largest `bad_debt_dust_usd` an admin may set (spec §11: a write-off's loss is bounded by
/// the dust collateral it leaves behind).
pub const MAX_BAD_DEBT_DUST_USD: u128 = 1_000 * USD_SCALE;

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
#[constant]
pub const PROMO_VAULT_SEED: &[u8] = b"promo_vault";
#[constant]
pub const PROMO_VAULT_TOKEN_SEED: &[u8] = b"promo_vault_token";
#[constant]
pub const CAMPAIGN_SEED: &[u8] = b"campaign";
#[constant]
pub const VOUCHER_SEED: &[u8] = b"voucher";

/// Domain separator in the voucher message (spec §12). It binds a signature to this program's
/// voucher format, so a `promo_signer` key reused elsewhere cannot produce a valid voucher.
/// Published in the IDL so the off-chain promo signer reads it rather than hardcoding a copy
/// that could drift from the program's.
#[constant]
pub const VOUCHER_DOMAIN: &str = "hodl_loans:promo_voucher:v1";

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
            PROMO_VAULT_SEED,
            PROMO_VAULT_TOKEN_SEED,
            CAMPAIGN_SEED,
            VOUCHER_SEED,
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
