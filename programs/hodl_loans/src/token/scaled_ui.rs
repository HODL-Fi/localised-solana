use anchor_lang::prelude::*;
use anchor_spl::token_2022::spl_token_2022::{
    extension::{scaled_ui_amount::ScaledUiAmountConfig, BaseStateWithExtensions, StateWithExtensions},
    state::Mint as MintState,
};

use crate::constants::{MAX_MULTIPLIER, MULTIPLIER_ONE, MULTIPLIER_SCALE};
use crate::errors::HodlError;
use crate::state::CollateralKind;

/// The multiplier to apply to raw token amounts, at `MULTIPLIER_SCALE`.
///
/// `Standard` assets are always 1. For an `XStock`, the issuer's `ScaledUiAmount` extension
/// carries the factor its corporate actions (dividend reinvestment, splits) apply to balances:
/// `new_multiplier` once `now` reaches `new_multiplier_effective_timestamp`, else `multiplier`.
/// The stored value never moves into `multiplier` on its own, so reading that field alone goes
/// stale the moment a scheduled change takes effect.
pub fn read_multiplier(mint: &AccountInfo, kind: CollateralKind, now: i64) -> Result<u128> {
    if kind == CollateralKind::Standard {
        return Ok(MULTIPLIER_ONE);
    }
    let data = mint.try_borrow_data()?;
    let state = StateWithExtensions::<MintState>::unpack(&data)
        .map_err(|_| HodlError::UnsupportedMintExtension)?;
    let config = state
        .get_extension::<ScaledUiAmountConfig>()
        .map_err(|_| HodlError::UnsupportedMintExtension)?;
    let raw: f64 = if now >= i64::from(config.new_multiplier_effective_timestamp) {
        config.new_multiplier.into()
    } else {
        config.multiplier.into()
    };
    scale_multiplier(raw)
}

/// Convert the extension's `f64` to `MULTIPLIER_SCALE` fixed point, rejecting anything that
/// cannot price collateral: not finite, not positive, above `MAX_MULTIPLIER`, or so small it
/// rounds to nothing.
fn scale_multiplier(raw: f64) -> Result<u128> {
    require!(raw.is_finite() && raw > 0.0, HodlError::InvalidPrice);
    let scaled = raw * MULTIPLIER_SCALE as f64;
    require!(scaled <= MAX_MULTIPLIER as f64, HodlError::InvalidPrice);
    let scaled = scaled as u128;
    require!(scaled > 0, HodlError::InvalidPrice);
    Ok(scaled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multipliers_scale_to_twelve_decimals() {
        assert_eq!(scale_multiplier(1.0).unwrap(), MULTIPLIER_SCALE);
        assert_eq!(scale_multiplier(1.000123).unwrap(), 1_000_123_000_000);
        // A dividend-reinvestment factor just above 1, as the live AAPLX mint carries. The
        // conversion truncates, so a factor the binary float cannot hold exactly lands one
        // unit low — which undervalues collateral by 10^-12 of a token, in the protocol's
        // favour, and never the borrower's.
        assert_eq!(scale_multiplier(1.0009).unwrap(), 1_000_899_999_999);
        assert_eq!(scale_multiplier(0.5).unwrap(), 500_000_000_000);
    }

    #[test]
    fn unusable_multipliers_are_rejected() {
        assert!(scale_multiplier(0.0).is_err());
        assert!(scale_multiplier(-1.0).is_err());
        assert!(scale_multiplier(f64::NAN).is_err());
        assert!(scale_multiplier(f64::INFINITY).is_err());
        // Above MAX_MULTIPLIER (1,000,000).
        assert!(scale_multiplier(1_000_001.0).is_err());
        // Positive but rounds to zero at 10^12.
        assert!(scale_multiplier(1e-13).is_err());
    }
}
