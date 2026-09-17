use anchor_lang::prelude::*;

use crate::constants::{VIRTUAL_ASSETS, VIRTUAL_SHARES};
use crate::math::checked::{add, mul_div_ceil, mul_div_floor};

/// Shares minted for depositing `amount`. Rounds down.
pub fn shares_for_deposit(amount: u64, total_shares: u128, total_assets: u128) -> Result<u128> {
    mul_div_floor(
        amount as u128,
        add(total_shares, VIRTUAL_SHARES)?,
        add(total_assets, VIRTUAL_ASSETS)?,
    )
}

/// Shares burned for withdrawing `amount`. Rounds up.
pub fn shares_to_burn(amount: u64, total_shares: u128, total_assets: u128) -> Result<u128> {
    mul_div_ceil(
        amount as u128,
        add(total_shares, VIRTUAL_SHARES)?,
        add(total_assets, VIRTUAL_ASSETS)?,
    )
}

/// Assets redeemable for `shares`. Rounds down.
pub fn redeemable_amount(shares: u128, total_shares: u128, total_assets: u128) -> Result<u128> {
    mul_div_floor(
        shares,
        add(total_assets, VIRTUAL_ASSETS)?,
        add(total_shares, VIRTUAL_SHARES)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_deposit_uses_virtual_offset() {
        assert_eq!(shares_for_deposit(1_000_000, 0, 0).unwrap(), 1_000_000_000);
    }

    #[test]
    fn later_deposits_are_proportional() {
        assert_eq!(shares_for_deposit(500_000, 1_000_000_000, 1_000_000).unwrap(), 500_000_000);
        assert_eq!(
            shares_for_deposit(1_100_000, 1_000_000_000, 1_100_000).unwrap(),
            1_000_000_090
        );
    }

    #[test]
    fn redeem_matches_deposit_when_exact() {
        assert_eq!(redeemable_amount(1_000_000_000, 1_000_000_000, 1_000_000).unwrap(), 1_000_000);
    }

    #[test]
    fn burn_rounds_up_mint_rounds_down() {
        let (s, a) = (1_000_000_007u128, 1_000_003u128);
        for amount in [1u64, 7, 999, 123_456] {
            assert!(shares_to_burn(amount, s, a).unwrap() >= shares_for_deposit(amount, s, a).unwrap());
        }
    }

    #[test]
    fn deposit_then_redeem_never_returns_more() {
        let cases = [(0u128, 0u128), (1_000_000_000, 1_000_000), (3_333_333_333, 1_234_567), (7, 1_000_000_000)];
        for (total_shares, total_assets) in cases {
            for amount in [1u64, 2, 999, 1_000_000, 987_654_321] {
                let shares = shares_for_deposit(amount, total_shares, total_assets).unwrap();
                let back = redeemable_amount(shares, total_shares + shares, total_assets + amount as u128).unwrap();
                assert!(back <= amount as u128, "amount {amount} returned {back}");
            }
        }
    }
}
