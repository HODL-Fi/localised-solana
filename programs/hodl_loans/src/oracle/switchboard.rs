use anchor_lang::prelude::*;
use switchboard_on_demand::{CurrentResult, Discriminator, PullFeedAccountData};

use crate::errors::HodlError;
use crate::math::price::{require_confidence, scale_switchboard_value, UsdPrice};
use crate::state::{CollateralAsset, Market};

/// Read the market's Switchboard On-Demand NGN/USD pull feed.
///
/// The account address is pinned to `market.ngn_feed`, which the admin sets, the account must
/// be owned by the Switchboard On-Demand program, and the data must carry the
/// `PullFeedAccountData` discriminator and full length.
///
/// The owner check matters because `ngn_feed` is a bare `Pubkey` on `Market` with no
/// constraint behind it: without it, an admin who set the field to any account at all would
/// have the program read 3.2 KB of arbitrary bytes as a price feed, and a discriminator is
/// eight bytes an attacker can simply write. It does **not** remove the residual trust in
/// whichever authority controls the real feed's writes — that is a genuine assumption, recorded
/// in spec §20 item 4, and an owner check cannot address it.
///
/// Pinned to `constants::SWITCHBOARD_ON_DEMAND_PID`, chosen at compile time by the `devnet`
/// feature. A build for the wrong cluster reads a feed owned by the other program's id and is
/// refused here; see `constants::SWITCHBOARD_ON_DEMAND_PID` for why that trade is made at
/// compile time rather than through a `Config` field.
///
/// Only the 128-byte aggregated `result` is copied out (by offset, unaligned), keeping the
/// 3.2 KB feed off the stack. `value` is the price and `std_dev` the spread.
pub fn read_ngn_price(account: &AccountInfo, market: &Market, clock: &Clock) -> Result<UsdPrice> {
    read_pull_feed(
        account,
        market.ngn_feed,
        // The NGN feed is not hash-bound. It could be — the same repoint this guards against on
        // the collateral side applies to it — but it is a live feed whose hash is not recorded
        // on `Market`, so binding it is a migration, not a code change. Recorded in spec §20.
        None,
        market.ngn_max_stale_slots,
        market.ngn_min_samples,
        market.ngn_max_spread_bps,
        clock,
    )
}

/// Read a collateral asset's Switchboard On-Demand price.
///
/// Only called for `PriceSource::SwitchboardOnDemand`. Every bound comes from the asset —
/// `sb_max_stale_slots`, `sb_min_samples`, and `max_conf_bps`, which is the same spread field
/// the Pyth path uses, so one asset's confidence policy reads the same whichever oracle prices
/// it.
///
/// Two things bind the read to this asset, and it needs both:
///
/// - **the pinned `price_account`**, re-checked here as well as in `valuation.rs`. A
///   `PriceUpdateV2` proves which feed it carries, so the Pyth path tolerates an unpinned
///   asset; a `PullFeedAccountData` proves nothing, so `CollateralParams::validate` refuses to
///   list a Switchboard asset without a pin.
/// - **`sb_feed_hash`**, because the pin alone is not enough: a pull feed's `authority` can
///   rewrite the account's `feed_hash` and repoint a pinned address at a different job. The
///   address, the owner and the discriminator would all still check out.
pub fn read_collateral_price(
    account: &AccountInfo,
    asset: &CollateralAsset,
    clock: &Clock,
) -> Result<UsdPrice> {
    read_pull_feed(
        account,
        asset.price_account,
        Some(&asset.sb_feed_hash),
        asset.sb_max_stale_slots,
        asset.sb_min_samples,
        asset.max_conf_bps,
        clock,
    )
}

/// The one Switchboard pull-feed read, shared by the market's NGN feed and by any collateral
/// asset priced this way.
///
/// Only the 128-byte aggregated `result` is copied out (by offset, unaligned), keeping the
/// 3.2 KB feed off the stack. `value` is the price and `std_dev` the spread. `feed_hash` is
/// compared in place for the same reason — no need to materialise the account to read 32 bytes.
#[allow(clippy::too_many_arguments)]
fn read_pull_feed(
    account: &AccountInfo,
    expected_key: Pubkey,
    expected_feed_hash: Option<&[u8; 32]>,
    max_stale_slots: u64,
    min_samples: u32,
    max_spread_bps: u16,
    clock: &Clock,
) -> Result<UsdPrice> {
    require_keys_eq!(account.key(), expected_key, HodlError::PriceAccountMismatch);
    require_keys_eq!(
        *account.owner,
        crate::constants::SWITCHBOARD_ON_DEMAND_PID,
        HodlError::PriceAccountMismatch
    );
    let data = account.try_borrow_data()?;
    require!(
        data.len() >= 8 + std::mem::size_of::<PullFeedAccountData>()
            && data[..8] == *PullFeedAccountData::DISCRIMINATOR,
        HodlError::PriceAccountMismatch
    );
    if let Some(expected) = expected_feed_hash {
        let at = 8 + std::mem::offset_of!(PullFeedAccountData, feed_hash);
        require!(&data[at..at + 32] == expected.as_slice(), HodlError::PriceAccountMismatch);
    }
    let start = 8 + std::mem::offset_of!(PullFeedAccountData, result);
    let result: CurrentResult =
        bytemuck::pod_read_unaligned(&data[start..start + std::mem::size_of::<CurrentResult>()]);
    require!(
        result.slot > 0 && clock.slot.saturating_sub(result.slot) <= max_stale_slots,
        HodlError::StalePrice
    );
    require!(result.num_samples as u32 >= min_samples, HodlError::StalePrice);
    let price = scale_switchboard_value(result.value, result.std_dev)?;
    require_confidence(&price, max_spread_bps)?;
    Ok(price)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::state::PriceSource;

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
        read_owned_by(key, data, m, slot, crate::constants::SWITCHBOARD_ON_DEMAND_PID)
    }

    fn read_owned_by(
        key: Pubkey,
        data: &mut [u8],
        m: &Market,
        slot: u64,
        owner: Pubkey,
    ) -> Result<UsdPrice> {
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
        // Right address, right discriminator, wrong owner. `ngn_feed` is a bare `Pubkey` the
        // admin sets, so without this the program would read any account it named as a price.
        assert_eq!(
            err(read_owned_by(key, &mut pull_feed_data(VALUE, 0, 1_000, 5), &m, 1_000, Pubkey::new_unique())),
            HodlError::PriceAccountMismatch.into()
        );
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

    // ---- collateral priced by a Switchboard feed ----

    /// `pull_feed_data` with the `feed_hash` set, which only the collateral path reads.
    fn pull_feed_data_hashed(
        value: i128,
        std_dev: i128,
        slot: u64,
        num_samples: u8,
        feed_hash: [u8; 32],
    ) -> Vec<u8> {
        let mut feed: PullFeedAccountData = bytemuck::Zeroable::zeroed();
        feed.result.value = value;
        feed.result.std_dev = std_dev;
        feed.result.slot = slot;
        feed.result.num_samples = num_samples;
        feed.feed_hash = feed_hash;
        let mut data = PullFeedAccountData::DISCRIMINATOR.to_vec();
        data.extend_from_slice(bytemuck::bytes_of(&feed));
        data
    }

    const HASH: [u8; 32] = [42; 32];

    fn asset(price_account: Pubkey) -> CollateralAsset {
        let mut a: CollateralAsset = unsafe { std::mem::zeroed() };
        a.price_source = PriceSource::SwitchboardOnDemand;
        a.price_account = price_account;
        a.sb_feed_hash = HASH;
        a.sb_max_stale_slots = 150;
        a.sb_min_samples = 3;
        a.max_conf_bps = 200;
        a
    }

    fn read_collateral(
        key: Pubkey,
        data: &mut [u8],
        a: &CollateralAsset,
        slot: u64,
    ) -> Result<UsdPrice> {
        let mut lamports = 1_000_000u64;
        let owner = crate::constants::SWITCHBOARD_ON_DEMAND_PID;
        let info = AccountInfo::new(&key, false, false, &mut lamports, data, &owner, false);
        let clock = Clock { slot, ..Clock::default() };
        read_collateral_price(&info, a, &clock)
    }

    /// $1,023.01 per token — an equity-magnitude price, three orders of magnitude above the
    /// NGN case and six above it in raw units. The NGN tests only ever exercise values far
    /// below 1, so this is the one that would catch a rescale that overflows or truncates at
    /// the top of the range.
    #[test]
    fn an_equity_magnitude_price_is_scaled() {
        let key = Pubkey::new_unique();
        let a = asset(key);
        // 1023.01 × 10^18
        let mut data = pull_feed_data_hashed(1_023_010_000_000_000_000_000, 0, 1_000, 5, HASH);
        let p = read_collateral(key, &mut data, &a, 1_100).unwrap();
        // 1023.01 at USD_SCALE (10^12)
        assert_eq!(p.price, 1_023_010_000_000_000);
    }

    /// The reason `sb_feed_hash` exists. A pull feed's `authority` can rewrite the account's
    /// `feed_hash`, repointing a **pinned** address from one job to another. Address, owner and
    /// discriminator all still check out; only the hash catches it.
    #[test]
    fn a_feed_whose_job_was_repointed_is_refused() {
        let key = Pubkey::new_unique();
        let a = asset(key);
        let mut repointed =
            pull_feed_data_hashed(1_023_010_000_000_000_000_000, 0, 1_000, 5, [43; 32]);
        assert_eq!(
            read_collateral(key, &mut repointed, &a, 1_100).unwrap_err(),
            HodlError::PriceAccountMismatch.into()
        );
    }

    /// The collateral read takes its bounds from the asset, not from the market — so listing a
    /// tighter asset actually tightens it.
    #[test]
    fn the_asset_supplies_its_own_bounds() {
        let key = Pubkey::new_unique();
        let err = |r: Result<UsdPrice>| r.unwrap_err();
        let fresh = || pull_feed_data_hashed(1_023_010_000_000_000_000_000, 0, 1_000, 5, HASH);

        let mut tight = asset(key);
        tight.sb_max_stale_slots = 10;
        assert_eq!(err(read_collateral(key, &mut fresh(), &tight, 1_011)), HodlError::StalePrice.into());
        read_collateral(key, &mut fresh(), &tight, 1_010).unwrap();

        let mut quorum = asset(key);
        quorum.sb_min_samples = 6;
        assert_eq!(err(read_collateral(key, &mut fresh(), &quorum, 1_000)), HodlError::StalePrice.into());

        // Spread is bounded by the asset's `max_conf_bps`, the same field the Pyth path uses.
        let mut narrow = asset(key);
        narrow.max_conf_bps = 10;
        let wide = pull_feed_data_hashed(
            1_023_010_000_000_000_000_000,
            2_046_020_000_000_000_000, // 20 bps of the price
            1_000,
            5,
            HASH,
        );
        assert_eq!(
            err(read_collateral(key, &mut wide.clone(), &narrow, 1_000)),
            HodlError::PriceConfidenceTooWide.into()
        );

        // And the pinned address is re-checked here, not only in `valuation.rs`.
        assert_eq!(
            err(read_collateral(Pubkey::new_unique(), &mut fresh(), &asset(key), 1_000)),
            HodlError::PriceAccountMismatch.into()
        );
    }
}
