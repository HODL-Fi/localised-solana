use anchor_lang::prelude::*;

use crate::constants::{BPS, USD_DECIMALS};
use crate::errors::HodlError;
use crate::math::checked::{add, mul_div_ceil, mul_div_floor, pow10};

/// A price in USD per whole token, at `USD_SCALE`, with its uncertainty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsdPrice {
    pub price: u128,
    /// Confidence (Pyth) or spread (Switchboard), same scale as `price`.
    pub conf: u128,
}

impl UsdPrice {
    /// `price − conf`, used for collateral and promo value.
    pub fn lower(&self) -> u128 {
        self.price.saturating_sub(self.conf)
    }

    /// `price + conf`, used for debt value.
    pub fn upper(&self) -> Result<u128> {
        add(self.price, self.conf)
    }
}

/// Rescale `value × 10^exponent` to `USD_SCALE`. Rounds down when shrinking.
///
/// `exponent` comes from the oracle account, so the shift is computed with `checked_add`
/// rather than `+`: a feed reporting an exponent near `i32::MAX` would otherwise abort the
/// transaction on an arithmetic overflow instead of returning `InvalidPrice`.
fn rescale(value: u128, exponent: i32) -> Result<u128> {
    let shift = exponent.checked_add(USD_DECIMALS).ok_or(HodlError::InvalidPrice)?;
    if shift >= 0 {
        Ok(value.checked_mul(pow10(shift as u32)?).ok_or(HodlError::MathOverflow)?)
    } else {
        Ok(value / pow10((-shift) as u32)?)
    }
}

/// `rescale`, rounding **up** when shrinking. Used only for the uncertainty term.
///
/// Rounding a confidence or spread down is anti-conservative on both sides at once, which is
/// easy to miss because the two uses pull in opposite directions: collateral counts at
/// `price − conf`, so a smaller `conf` values it higher, and debt counts at `price + conf`, so
/// a smaller `conf` values it lower. Rounding the uncertainty up is the only direction that is
/// conservative for both.
fn rescale_ceil(value: u128, exponent: i32) -> Result<u128> {
    let shift = exponent.checked_add(USD_DECIMALS).ok_or(HodlError::InvalidPrice)?;
    if shift >= 0 {
        Ok(value.checked_mul(pow10(shift as u32)?).ok_or(HodlError::MathOverflow)?)
    } else {
        Ok(value.div_ceil(pow10((-shift) as u32)?))
    }
}

/// Convert a Pyth `price`/`conf` with `exponent` into a `UsdPrice`. Rejects non-positive prices.
pub fn scale_pyth_price(price: i64, conf: u64, exponent: i32) -> Result<UsdPrice> {
    require!(price > 0, HodlError::InvalidPrice);
    let scaled = rescale(price as u128, exponent)?;
    require!(scaled > 0, HodlError::InvalidPrice);
    Ok(UsdPrice { price: scaled, conf: rescale_ceil(conf as u128, exponent)? })
}

/// Convert a Switchboard value and spread (both 18-decimal fixed point) into a `UsdPrice`.
pub fn scale_switchboard_value(value: i128, spread: i128) -> Result<UsdPrice> {
    require!(value > 0 && spread >= 0, HodlError::InvalidPrice);
    let scaled = rescale(value as u128, -18)?;
    require!(scaled > 0, HodlError::InvalidPrice);
    Ok(UsdPrice { price: scaled, conf: rescale_ceil(spread as u128, -18)? })
}

/// Reject a price whose uncertainty exceeds `max_bps` of the price.
pub fn require_confidence(price: &UsdPrice, max_bps: u16) -> Result<()> {
    let conf_scaled = price.conf.checked_mul(BPS).ok_or(HodlError::MathOverflow)?;
    let limit = price.price.checked_mul(max_bps as u128).ok_or(HodlError::MathOverflow)?;
    require!(conf_scaled <= limit, HodlError::PriceConfidenceTooWide);
    Ok(())
}

/// USD value (at `USD_SCALE`) of `amount` base units of a token with `decimals`. Rounds down.
pub fn token_value(amount: u128, decimals: u8, price: u128) -> Result<u128> {
    mul_div_floor(amount, price, pow10(decimals as u32)?)
}

/// Same as `token_value` but rounds up (used for debt).
pub fn token_value_ceil(amount: u128, decimals: u8, price: u128) -> Result<u128> {
    mul_div_ceil(amount, price, pow10(decimals as u32)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pyth_exponents_rescale_to_twelve_decimals() {
        // SOL at $150.12345678 with exponent -8.
        let p = scale_pyth_price(15_012_345_678, 1_500_000, -8).unwrap();
        assert_eq!(p.price, 150_123_456_780_000);
        assert_eq!(p.conf, 15_000_000_000);
        // Exponent -15 shrinks and rounds down.
        let p = scale_pyth_price(1_000_000_000_000_001, 0, -15).unwrap();
        assert_eq!(p.price, 1_000_000_000_000);
        // Positive exponent.
        assert_eq!(scale_pyth_price(3, 0, 2).unwrap().price, 300_000_000_000_000);
    }

    #[test]
    fn uncertainty_rounds_up_so_it_can_never_vanish() {
        // The price rounds down and the uncertainty rounds up; they are not symmetric, and the
        // asymmetry is the point. Collateral counts at `price - conf` and debt at
        // `price + conf`, so a conf rounded DOWN values collateral too high and debt too low —
        // anti-conservative on both sides at once.
        //
        // A spread finer than USD_SCALE is where it shows: floored it becomes 0, which claims
        // the oracle is perfectly certain.
        let p = scale_switchboard_value(650_000_000_000_000, 1).unwrap();
        assert_eq!(p.price, 650_000_000);
        assert_eq!(p.conf, 1, "a sub-scale spread must not round away to zero uncertainty");

        // And it rounds up, not to nearest: 6_250_000_123_456 / 1e6 is 6_250_000.123456.
        let p = scale_switchboard_value(650_000_000_000_000, 6_250_000_123_456).unwrap();
        assert_eq!(p.conf, 6_250_001);

        // Same rule on the Pyth path, which takes its shift from the feed's exponent.
        let p = scale_pyth_price(15_012_345_678, 1, -15).unwrap();
        assert_eq!(p.conf, 1);
    }

    #[test]
    fn an_absurd_feed_exponent_is_an_invalid_price_not_an_abort() {
        // `exponent` is read from the oracle account. Computing the shift with `+` would
        // overflow i32 and abort the whole transaction; `checked_add` makes it a normal error
        // the caller can see.
        assert!(scale_pyth_price(1, 0, i32::MAX).is_err());
        assert!(scale_pyth_price(1, 0, i32::MIN).is_err());
    }

    #[test]
    fn pyth_rejects_non_positive_and_vanishing_prices() {
        assert!(scale_pyth_price(0, 0, -8).is_err());
        assert!(scale_pyth_price(-5, 0, -8).is_err());
        assert!(scale_pyth_price(1, 0, -20).is_err());
    }

    #[test]
    fn switchboard_value_rescales_from_eighteen_decimals() {
        // NGN/USD ≈ 0.00065 with a 0.0000013 spread.
        let p = scale_switchboard_value(650_000_000_000_000, 1_300_000_000_000).unwrap();
        assert_eq!(p.price, 650_000_000);
        assert_eq!(p.conf, 1_300_000);
        assert!(scale_switchboard_value(0, 0).is_err());
        assert!(scale_switchboard_value(1, -1).is_err());
    }

    #[test]
    fn confidence_limit_is_inclusive() {
        let p = UsdPrice { price: 1_000_000, conf: 20_000 };
        require_confidence(&p, 200).unwrap();
        assert!(require_confidence(&p, 199).is_err());
    }

    #[test]
    fn token_values_use_decimals_and_round_directions() {
        // 2.5 tokens with 9 decimals at $150.
        let price = 150 * 1_000_000_000_000u128;
        assert_eq!(token_value(2_500_000_000, 9, price).unwrap(), 375 * 1_000_000_000_000);
        // 1 base unit of a 6-decimal token at $0.00065 is 650 USD-scale units per million.
        assert_eq!(token_value(1, 6, 650_000_000).unwrap(), 650);
        assert_eq!(token_value(1, 6, 650_000_001).unwrap(), 650);
        assert_eq!(token_value_ceil(1, 6, 650_000_001).unwrap(), 651);
        assert_eq!(UsdPrice { price: 10, conf: 15 }.lower(), 0);
    }
}
