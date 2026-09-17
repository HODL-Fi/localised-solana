use anchor_lang::prelude::*;
use pyth_solana_receiver_sdk::error::GetPriceError;
use pyth_solana_receiver_sdk::price_update::PriceUpdateV2;

use crate::errors::HodlError;
use crate::math::price::{require_confidence, scale_pyth_price, UsdPrice};

/// Read a fully verified Pyth pull price no older than `max_age_seconds` for `feed_id`.
/// The account must be owned by the Pyth receiver program.
pub fn read_pyth_price(
    account: &AccountInfo,
    feed_id: &[u8; 32],
    max_age_seconds: u64,
    max_conf_bps: u16,
    clock: &Clock,
) -> Result<UsdPrice> {
    require_keys_eq!(*account.owner, pyth_solana_receiver_sdk::ID, HodlError::PriceAccountMismatch);
    let data = account.try_borrow_data()?;
    let update =
        PriceUpdateV2::try_deserialize(&mut &data[..]).map_err(|_| HodlError::PriceAccountMismatch)?;
    let price = update
        .get_price_no_older_than(clock, max_age_seconds, feed_id)
        .map_err(|e| match e {
            GetPriceError::PriceTooOld => HodlError::StalePrice,
            GetPriceError::MismatchedFeedId => HodlError::PriceAccountMismatch,
            _ => HodlError::InvalidPrice,
        })?;
    let usd = scale_pyth_price(price.price, price.conf, price.exponent)?;
    require_confidence(&usd, max_conf_bps)?;
    Ok(usd)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use pyth_solana_receiver_sdk::price_update::{PriceFeedMessage, VerificationLevel};

    /// Serialized `PriceUpdateV2` account data (shared with the LiteSVM harness via copy).
    pub fn price_update_data(
        feed_id: [u8; 32],
        price: i64,
        conf: u64,
        exponent: i32,
        publish_time: i64,
        verification_level: VerificationLevel,
    ) -> Vec<u8> {
        let update = PriceUpdateV2 {
            write_authority: Pubkey::new_unique(),
            verification_level,
            price_message: PriceFeedMessage {
                feed_id,
                price,
                conf,
                exponent,
                publish_time,
                prev_publish_time: publish_time - 1,
                ema_price: price,
                ema_conf: conf,
            },
            posted_slot: 1,
        };
        let mut data = Vec::new();
        update.try_serialize(&mut data).unwrap();
        data
    }

    fn read(data: &mut [u8], owner: &Pubkey, feed: [u8; 32], now: i64, max_conf_bps: u16) -> Result<UsdPrice> {
        let key = Pubkey::new_unique();
        let mut lamports = 1_000_000u64;
        let info = AccountInfo::new(&key, false, false, &mut lamports, data, owner, false);
        let clock = Clock { unix_timestamp: now, ..Clock::default() };
        read_pyth_price(&info, &feed, 60, max_conf_bps, &clock)
    }

    const FEED: [u8; 32] = [9; 32];

    #[test]
    fn fresh_full_price_is_scaled() {
        let mut data = price_update_data(FEED, 15_000_000_000, 10_000_000, -8, 1_000, VerificationLevel::Full);
        let p = read(&mut data, &pyth_solana_receiver_sdk::ID, FEED, 1_030, 200).unwrap();
        assert_eq!(p.price, 150_000_000_000_000);
        assert_eq!(p.conf, 100_000_000_000);
    }

    #[test]
    fn rejections_map_to_program_errors() {
        let owner = pyth_solana_receiver_sdk::ID;
        let fresh = || price_update_data(FEED, 15_000_000_000, 10_000_000, -8, 1_000, VerificationLevel::Full);

        let err = |r: Result<UsdPrice>| r.unwrap_err();
        assert_eq!(err(read(&mut fresh(), &Pubkey::new_unique(), FEED, 1_000, 200)), HodlError::PriceAccountMismatch.into());
        assert_eq!(err(read(&mut fresh(), &owner, [8; 32], 1_000, 200)), HodlError::PriceAccountMismatch.into());
        assert_eq!(err(read(&mut fresh(), &owner, FEED, 1_061, 200)), HodlError::StalePrice.into());
        // $0.10 confidence on $150 is 6.67 bps: allowed at 7, rejected at 6.
        read(&mut fresh(), &owner, FEED, 1_000, 7).unwrap();
        assert_eq!(err(read(&mut fresh(), &owner, FEED, 1_000, 6)), HodlError::PriceConfidenceTooWide.into());
        let mut partial =
            price_update_data(FEED, 15_000_000_000, 0, -8, 1_000, VerificationLevel::Partial { num_signatures: 5 });
        assert_eq!(err(read(&mut partial, &owner, FEED, 1_000, 200)), HodlError::InvalidPrice.into());
        let mut negative = price_update_data(FEED, -1, 0, -8, 1_000, VerificationLevel::Full);
        assert_eq!(err(read(&mut negative, &owner, FEED, 1_000, 200)), HodlError::InvalidPrice.into());
    }
}
