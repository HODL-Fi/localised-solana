use anchor_lang::prelude::*;

use crate::errors::HodlError;

/// `a × b / denominator`, rounded down.
pub fn mul_div_floor(a: u128, b: u128, denominator: u128) -> Result<u128> {
    require!(denominator != 0, HodlError::MathOverflow);
    let product = a.checked_mul(b).ok_or(HodlError::MathOverflow)?;
    Ok(product / denominator)
}

/// `a × b / denominator`, rounded up.
pub fn mul_div_ceil(a: u128, b: u128, denominator: u128) -> Result<u128> {
    require!(denominator != 0, HodlError::MathOverflow);
    let product = a.checked_mul(b).ok_or(HodlError::MathOverflow)?;
    Ok(product.div_ceil(denominator))
}

pub fn add(a: u128, b: u128) -> Result<u128> {
    Ok(a.checked_add(b).ok_or(HodlError::MathOverflow)?)
}

pub fn sub(a: u128, b: u128) -> Result<u128> {
    Ok(a.checked_sub(b).ok_or(HodlError::MathOverflow)?)
}

/// `10^exp`. The only power-of-ten helper in the program: `math/price.rs` and
/// `math/liquidation.rs` each had their own, with different parameter types (`u32` and `u8`),
/// which is how the same bound came to be reasoned about twice. Callers holding a `u8` widen.
pub fn pow10(exp: u32) -> Result<u128> {
    Ok(10u128.checked_pow(exp).ok_or(HodlError::MathOverflow)?)
}

pub fn to_u64(value: u128) -> Result<u64> {
    Ok(u64::try_from(value).map_err(|_| HodlError::MathOverflow)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_and_ceil_differ_only_on_remainder() {
        assert_eq!(mul_div_floor(10, 3, 4).unwrap(), 7);
        assert_eq!(mul_div_ceil(10, 3, 4).unwrap(), 8);
        assert_eq!(mul_div_floor(8, 3, 4).unwrap(), 6);
        assert_eq!(mul_div_ceil(8, 3, 4).unwrap(), 6);
    }

    #[test]
    fn zero_denominator_and_overflow_fail() {
        // `is_err()` would pass for *any* error, and `MathOverflow` is the most-raised variant
        // in the program — 37 sites — with nothing anywhere pinning that it is what these
        // return. A helper that started returning `InvalidParameters` would be caught by
        // nothing. Every branch of every helper, by the variant.
        // Compare the error *code*, not the Debug string: Anchor stamps a source file and
        // line into the latter, so two `MathOverflow`s from different helpers never match.
        // The code is also what a client actually sees.
        let code = |e: anchor_lang::error::Error| match e {
            anchor_lang::error::Error::AnchorError(a) => a.error_code_number,
            other => panic!("expected an AnchorError, got {other:?}"),
        };
        let want = u32::from(HodlError::MathOverflow);
        let is_overflow = |r: Result<u128>| assert_eq!(code(r.unwrap_err()), want);
        is_overflow(mul_div_floor(1, 1, 0));
        is_overflow(mul_div_floor(u128::MAX, 2, 1));
        is_overflow(mul_div_ceil(1, 1, 0));
        is_overflow(mul_div_ceil(u128::MAX, 2, 1));
        is_overflow(add(u128::MAX, 1));
        is_overflow(sub(1, 2));
        is_overflow(pow10(39));
        assert_eq!(code(to_u64(u64::MAX as u128 + 1).unwrap_err()), want);

        // The boundaries on each side still succeed, so the guards are not simply always-on.
        assert_eq!(mul_div_floor(u128::MAX, 1, u128::MAX).unwrap(), 1);
        assert_eq!(add(u128::MAX - 1, 1).unwrap(), u128::MAX);
        assert_eq!(sub(1, 1).unwrap(), 0);
        assert_eq!(pow10(38).unwrap(), 10u128.pow(38));
        assert_eq!(to_u64(u64::MAX as u128).unwrap(), u64::MAX);
    }
}
