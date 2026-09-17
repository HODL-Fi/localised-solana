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
        assert!(mul_div_floor(1, 1, 0).is_err());
        assert!(mul_div_ceil(u128::MAX, 2, 1).is_err());
        assert!(sub(1, 2).is_err());
        assert!(to_u64(u64::MAX as u128 + 1).is_err());
    }
}
