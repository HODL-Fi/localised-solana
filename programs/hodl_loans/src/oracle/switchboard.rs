use anchor_lang::prelude::*;
use switchboard_on_demand::{CurrentResult, Discriminator, PullFeedAccountData};

use crate::errors::HodlError;
use crate::math::price::{require_confidence, scale_switchboard_value, UsdPrice};
use crate::state::Market;

/// Read the market's Switchboard On-Demand NGN/USD pull feed.
///
/// The account address is pinned to `market.ngn_feed`, which the admin sets, and the data must
/// carry the `PullFeedAccountData` discriminator and full length. There is no check that the
/// account is owned by the Switchboard On-Demand program: this trusts the admin's choice of
/// feed and, transitively, whichever authority controls that feed's writes. Adding an owner
/// check against the Switchboard On-Demand program ID is deferred to Plan 6 hardening. Only the
/// 128-byte aggregated `result` is copied out (by offset, unaligned), keeping the 3.2 KB feed
/// off the stack. `value` is the price and `std_dev` the spread.
pub fn read_ngn_price(account: &AccountInfo, market: &Market, clock: &Clock) -> Result<UsdPrice> {
    require_keys_eq!(account.key(), market.ngn_feed, HodlError::PriceAccountMismatch);
    let data = account.try_borrow_data()?;
    require!(
        data.len() >= 8 + std::mem::size_of::<PullFeedAccountData>()
            && data[..8] == *PullFeedAccountData::DISCRIMINATOR,
        HodlError::PriceAccountMismatch
    );
    let start = 8 + std::mem::offset_of!(PullFeedAccountData, result);
    let result: CurrentResult =
        bytemuck::pod_read_unaligned(&data[start..start + std::mem::size_of::<CurrentResult>()]);
    require!(
        result.slot > 0 && clock.slot.saturating_sub(result.slot) <= market.ngn_max_stale_slots,
        HodlError::StalePrice
    );
    require!(result.num_samples as u32 >= market.ngn_min_samples, HodlError::StalePrice);
    let price = scale_switchboard_value(result.value, result.std_dev)?;
    require_confidence(&price, market.ngn_max_spread_bps)?;
    Ok(price)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Serialized `PullFeedAccountData` account data with only the aggregated result set.
    pub fn pull_feed_data(value: i128, std_dev: i128, slot: u64, num_samples: u8) -> Vec<u8> {
        let mut feed: PullFeedAccountData = bytemuck::Zeroable::zeroed();
        feed.result.value = value;
        feed.result.std_dev = std_dev;
        feed.result.slot = slot;
        feed.result.num_samples = num_samples;
        let mut data = PullFeedAccountData::DISCRIMINATOR.to_vec();
        data.extend_from_slice(bytemuck::bytes_of(&feed));
        data
    }

    fn market(feed: Pubkey) -> Market {
        let mut m: Market = unsafe { std::mem::zeroed() };
        m.ngn_feed = feed;
        m.ngn_max_stale_slots = 150;
        m.ngn_min_samples = 3;
        m.ngn_max_spread_bps = 200;
        m
    }

    fn read(key: Pubkey, data: &mut [u8], m: &Market, slot: u64) -> Result<UsdPrice> {
        let owner = Pubkey::new_unique();
        let mut lamports = 1_000_000u64;
        let info = AccountInfo::new(&key, false, false, &mut lamports, data, &owner, false);
        let clock = Clock { slot, ..Clock::default() };
        read_ngn_price(&info, m, &clock)
    }

    // $0.000625 per NGN, 18 decimals.
    const VALUE: i128 = 625_000_000_000_000;

    #[test]
    fn fresh_result_is_scaled() {
        let key = Pubkey::new_unique();
        let m = market(key);
        let mut data = pull_feed_data(VALUE, 6_250_000_000_000, 1_000, 5);
        let p = read(key, &mut data, &m, 1_100).unwrap();
        assert_eq!(p.price, 625_000_000);
        assert_eq!(p.conf, 6_250_000);
    }

    #[test]
    fn rejections_map_to_program_errors() {
        let key = Pubkey::new_unique();
        let m = market(key);
        let err = |r: Result<UsdPrice>| r.unwrap_err();
        assert_eq!(err(read(Pubkey::new_unique(), &mut pull_feed_data(VALUE, 0, 1_000, 5), &m, 1_000)), HodlError::PriceAccountMismatch.into());
        let mut garbage = vec![0u8; 3_208];
        assert_eq!(err(read(key, &mut garbage, &m, 1_000)), HodlError::PriceAccountMismatch.into());
        assert_eq!(err(read(key, &mut pull_feed_data(VALUE, 0, 1_000, 5), &m, 1_151)), HodlError::StalePrice.into());
        assert_eq!(err(read(key, &mut pull_feed_data(VALUE, 0, 0, 5), &m, 10)), HodlError::StalePrice.into());
        assert_eq!(err(read(key, &mut pull_feed_data(VALUE, 0, 1_000, 2), &m, 1_000)), HodlError::StalePrice.into());
        assert_eq!(err(read(key, &mut pull_feed_data(0, 0, 1_000, 5), &m, 1_000)), HodlError::InvalidPrice.into());
        // 2% spread is the limit; just above fails.
        read(key, &mut pull_feed_data(VALUE, 12_500_000_000_000, 1_000, 5), &m, 1_000).unwrap();
        assert_eq!(err(read(key, &mut pull_feed_data(VALUE, 12_500_001_000_000, 1_000, 5), &m, 1_000)), HodlError::PriceConfidenceTooWide.into());
    }
}
