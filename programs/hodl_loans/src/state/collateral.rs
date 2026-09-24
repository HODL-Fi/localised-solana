use anchor_lang::prelude::*;

use crate::constants::{
    BPS, MAX_BPS, MAX_COLLATERAL_STALE_SLOTS, MAX_MULTIPLIER, MAX_PRICE_AGE_SECONDS, MULTIPLIER_ONE,
};
use crate::errors::HodlError;

/// Which oracle prices a collateral asset.
///
/// `Pyth` is declared first so it is discriminant `0`. Every asset listed before this field
/// existed carries zeroed reserved padding where `price_source` now sits, so those assets
/// deserialize as `Pyth` and keep the behaviour they were listed with. Reordering these
/// variants silently repoints every live asset at a different oracle.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum PriceSource {
    /// Pyth pull oracle (`PriceUpdateV2`), read through `oracle::pyth`. Bounded in seconds by
    /// `max_price_age_seconds`; the account may be pinned or not, because the update carries a
    /// verified feed id of its own.
    Pyth,
    /// Switchboard On-Demand pull feed (`PullFeedAccountData`), read through
    /// `oracle::switchboard` — the same account shape the market's NGN feed uses. Bounded in
    /// slots by `sb_max_stale_slots`, and the account **must** be pinned: see
    /// `CollateralParams::validate`.
    SwitchboardOnDemand,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum CollateralKind {
    /// Classic SPL Token or metadata-only Token-2022 mint (SOL, USDC, USDT).
    Standard,
    /// Admin-listed xStock (Plan 4).
    XStock,
}

#[account]
#[derive(InitSpace)]
pub struct CollateralAsset {
    pub version: u8,
    pub bump: u8,
    pub vault_bump: u8,
    pub mint: Pubkey,
    pub token_program: Pubkey,
    pub vault: Pubkey,
    pub decimals: u8,
    pub kind: CollateralKind,
    pub pyth_feed_id: [u8; 32],
    /// Pyth price account this asset is pinned to. `Pubkey::default()` accepts any verified
    /// update for `pyth_feed_id` inside `max_price_age_seconds`, so the caller may pick the
    /// most favourable update in that window; pinning removes that choice.
    /// Taken from the reserved padding, so the account size is unchanged.
    pub price_account: Pubkey,
    pub max_price_age_seconds: u64,
    pub max_conf_bps: u16,
    pub ltv_bps: u16,
    pub liquidation_threshold_bps: u16,
    pub liquidation_bonus_bps: u16,
    /// Raw token amounts.
    pub deposit_cap: u64,
    pub total_deposited: u64,
    /// Blocks new deposits of this asset only.
    pub paused: bool,
    /// Blocks new *borrowing* backed by this asset. While set, the holding's
    /// `lends_borrowing_power` is false, which suppresses both ways it could raise
    /// `borrow_limit`: its own LTV term and the promo cap its value would otherwise unlock.
    /// It still counts in full at `own_value` and at the liquidation line — including the
    /// promo lift, which takes its own ungated cap — so pausing an asset can neither make a
    /// live loan liquidatable nor make a position that was liquidatable a moment ago suddenly
    /// safe. Deposits and seizure are unaffected.
    ///
    /// **Withdrawal is not.** `withdraw_collateral` gates on the same `is_healthy()` the
    /// borrow does, so while a loan is live a borrow-paused asset backs no withdrawal at all;
    /// `a_borrow_paused_asset_backs_no_withdrawal_while_a_loan_is_live` pins that. Withdrawing
    /// is exposure-increasing in the same way borrowing is, so this is intended — but it means
    /// a pause does strand collateral behind a live loan until the loan is repaid or the pause
    /// lifted.
    /// Taken from the reserved padding, so the account size is unchanged.
    pub borrow_paused: bool,
    /// Per-asset ceiling on the mint's scaled-UI multiplier, in `MULTIPLIER_SCALE` fixed
    /// point. `0` means no per-asset ceiling — only the global `MAX_MULTIPLIER` applies.
    /// Exceeding it does not fail the price read: the asset simply stops lending borrowing
    /// power, exactly as `borrow_paused` does. See the walk in `valuation.rs`.
    /// Taken from the reserved padding, so the account size is unchanged.
    pub max_multiplier: u128,
    /// Which oracle `valuation.rs` reads for this asset. See `PriceSource`.
    /// Taken from the reserved padding, so the account size is unchanged.
    pub price_source: PriceSource,
    /// `SwitchboardOnDemand` only: the job definition this asset's feed must be running,
    /// checked against `PullFeedAccountData.feed_hash` on every price read.
    ///
    /// The pinned `price_account` alone is not enough. A pull feed's `authority` may rewrite
    /// the account's `feed_hash`, repointing a pinned address from "PreStocks OPENAI mark
    /// price" at any other job — and the account would still be the right address, owned by
    /// the right program, carrying the right discriminator. This field is what makes that
    /// repoint fail instead of silently repricing the collateral.
    /// Taken from the reserved padding, so the account size is unchanged.
    pub sb_feed_hash: [u8; 32],
    /// `SwitchboardOnDemand` only: freshness bound in slots, the counterpart of
    /// `max_price_age_seconds`. Switchboard's `CurrentResult` carries a slot and no timestamp.
    /// Taken from the reserved padding, so the account size is unchanged.
    pub sb_max_stale_slots: u64,
    /// `SwitchboardOnDemand` only: minimum oracle submissions behind the aggregated result.
    /// Taken from the reserved padding, so the account size is unchanged.
    pub sb_min_samples: u32,
    pub reserved: [u8; 34],
}

/// Admin-settable collateral parameters, used by `list_collateral` and `update_collateral_params`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct CollateralParams {
    pub pyth_feed_id: [u8; 32],
    /// `Pubkey::default()` leaves the asset unpinned (see `CollateralAsset::price_account`).
    pub price_account: Pubkey,
    pub max_price_age_seconds: u64,
    pub max_conf_bps: u16,
    pub ltv_bps: u16,
    pub liquidation_threshold_bps: u16,
    pub liquidation_bonus_bps: u16,
    pub deposit_cap: u64,
    /// See `CollateralAsset::max_multiplier`. Appended, so existing field order is unchanged.
    pub max_multiplier: u128,
    /// See `CollateralAsset::price_source`. Appended, so existing field order is unchanged.
    pub price_source: PriceSource,
    /// See `CollateralAsset::sb_feed_hash`. Appended, so existing field order is unchanged.
    pub sb_feed_hash: [u8; 32],
    /// See `CollateralAsset::sb_max_stale_slots`. Appended, so existing field order is unchanged.
    pub sb_max_stale_slots: u64,
    /// See `CollateralAsset::sb_min_samples`. Appended, so existing field order is unchanged.
    pub sb_min_samples: u32,
}

impl CollateralParams {
    /// Spec §8 collateral rules, checked against the current `Config::promo_cap_bps`.
    pub fn validate(&self, promo_cap_bps: u16) -> Result<()> {
        // Each source validates its own feed identity and freshness bound, and must leave the
        // other's fields zero. The zero rule is not tidiness: a Pyth asset carrying
        // `sb_max_stale_slots`, or a Switchboard asset carrying `max_price_age_seconds`,
        // advertises a freshness bound that nothing on its path ever reads. An operator tunes
        // it, nothing changes, and the asset looks tighter than it is.
        match self.price_source {
            PriceSource::Pyth => {
                require!(self.pyth_feed_id != [0u8; 32], HodlError::InvalidParameters);
                require!(
                    self.max_price_age_seconds > 0
                        && self.max_price_age_seconds <= MAX_PRICE_AGE_SECONDS,
                    HodlError::InvalidParameters
                );
                require!(
                    self.sb_feed_hash == [0u8; 32]
                        && self.sb_max_stale_slots == 0
                        && self.sb_min_samples == 0,
                    HodlError::InvalidParameters
                );
            }
            PriceSource::SwitchboardOnDemand => {
                require!(self.sb_feed_hash != [0u8; 32], HodlError::InvalidParameters);
                // Unpinned is a coherent choice on the Pyth path — a `PriceUpdateV2` proves
                // which feed it carries, so an unpinned asset only lets the caller choose
                // among verified updates inside its age window. A `PullFeedAccountData` proves
                // nothing of the kind: any account owned by the Switchboard program with the
                // right discriminator would satisfy an unpinned read. The address is load
                // bearing here, so refuse to list without it.
                require!(self.price_account != Pubkey::default(), HodlError::InvalidParameters);
                require!(
                    self.sb_max_stale_slots > 0
                        && self.sb_max_stale_slots <= MAX_COLLATERAL_STALE_SLOTS,
                    HodlError::InvalidParameters
                );
                require!(self.sb_min_samples >= 1, HodlError::InvalidParameters);
                require!(
                    self.pyth_feed_id == [0u8; 32] && self.max_price_age_seconds == 0,
                    HodlError::InvalidParameters
                );
            }
        }
        require!(self.max_conf_bps <= MAX_BPS, HodlError::InvalidParameters);
        require!(self.ltv_bps >= 1_000, HodlError::InvalidParameters);
        require!(
            self.ltv_bps as u32 + promo_cap_bps as u32 <= self.liquidation_threshold_bps as u32,
            HodlError::InvalidParameters
        );
        // This checks LT alone, but counting promo lifts the *effective* liquidation line to
        // LT×V + promo_counted, and promo_counted can reach promo_cap_bps×V — at the shipped
        // config (LT 90%, cap 20%, bonus 0) the effective line is 110% of collateral value, past
        // what this formula describes. That is intentional, and what covers the excess is promo
        // forfeiture: seizure returns the position's full, *uncapped* promo balance to the
        // market vault, while the line was lifted only by the *capped* value, so recovery ≥ lift.
        // Do not tighten this to `(LT + promo_cap_bps)` — that would reject the shipped config.
        // Anyone raising `liquidation_bonus_bps` must reason about `LT + promo_cap_bps` fitting
        // inside 100%, not `LT` alone.
        require!(
            self.liquidation_threshold_bps as u128 * (BPS + self.liquidation_bonus_bps as u128) <= BPS * BPS,
            HodlError::InvalidParameters
        );
        // `max_multiplier` is `MULTIPLIER_SCALE` fixed point, so a ceiling below `1.0` is
        // almost certainly an admin who meant "cap at 1x" and wrote `1`. Left unchecked that
        // reads as 10^-12 and puts *every* asset over its ceiling — a `Standard` asset
        // included, whose multiplier is the constant `MULTIPLIER_ONE`. The asset would then
        // silently lend no borrowing power, which also freezes every existing borrower out of
        // withdrawing collateral, with no event and no error at the moment it was set.
        // Above `MAX_MULTIPLIER` the ceiling can never bind, since the price read rejects
        // those multipliers first; accepting it would advertise a bound that does nothing.
        require!(
            self.max_multiplier == 0
                || (self.max_multiplier >= MULTIPLIER_ONE && self.max_multiplier <= MAX_MULTIPLIER),
            HodlError::InvalidParameters
        );
        Ok(())
    }
}

impl CollateralAsset {
    pub fn params(&self) -> CollateralParams {
        CollateralParams {
            pyth_feed_id: self.pyth_feed_id,
            price_account: self.price_account,
            max_price_age_seconds: self.max_price_age_seconds,
            max_conf_bps: self.max_conf_bps,
            ltv_bps: self.ltv_bps,
            liquidation_threshold_bps: self.liquidation_threshold_bps,
            liquidation_bonus_bps: self.liquidation_bonus_bps,
            deposit_cap: self.deposit_cap,
            max_multiplier: self.max_multiplier,
            price_source: self.price_source,
            sb_feed_hash: self.sb_feed_hash,
            sb_max_stale_slots: self.sb_max_stale_slots,
            sb_min_samples: self.sb_min_samples,
        }
    }

    pub fn apply_params(&mut self, p: &CollateralParams) {
        self.pyth_feed_id = p.pyth_feed_id;
        self.price_account = p.price_account;
        self.max_price_age_seconds = p.max_price_age_seconds;
        self.max_conf_bps = p.max_conf_bps;
        self.ltv_bps = p.ltv_bps;
        self.liquidation_threshold_bps = p.liquidation_threshold_bps;
        self.liquidation_bonus_bps = p.liquidation_bonus_bps;
        self.deposit_cap = p.deposit_cap;
        self.max_multiplier = p.max_multiplier;
        self.price_source = p.price_source;
        self.sb_feed_hash = p.sb_feed_hash;
        self.sb_max_stale_slots = p.sb_max_stale_slots;
        self.sb_min_samples = p.sb_min_samples;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sol() -> CollateralParams {
        CollateralParams {
            pyth_feed_id: [1; 32],
            price_account: Pubkey::default(),
            max_price_age_seconds: 60,
            max_conf_bps: 200,
            ltv_bps: 7_000,
            liquidation_threshold_bps: 9_000,
            liquidation_bonus_bps: 1_000,
            deposit_cap: u64::MAX,
            max_multiplier: 0,
            price_source: PriceSource::Pyth,
            sb_feed_hash: [0; 32],
            sb_max_stale_slots: 0,
            sb_min_samples: 0,
        }
    }

    #[test]
    fn launch_values_pass_with_twenty_percent_promo_cap() {
        sol().validate(2_000).unwrap();
        CollateralParams { ltv_bps: 5_000, liquidation_threshold_bps: 7_500, ..sol() }
            .validate(2_000)
            .unwrap();
    }

    #[test]
    fn each_rule_is_enforced() {
        let cases = [
            CollateralParams { pyth_feed_id: [0; 32], ..sol() },
            CollateralParams { max_price_age_seconds: 0, ..sol() },
            // Above the 60-second cap, which bounds how far a caller may shop for a price.
            CollateralParams { max_price_age_seconds: 61, ..sol() },
            CollateralParams { max_conf_bps: 10_001, ..sol() },
            CollateralParams { ltv_bps: 999, liquidation_threshold_bps: 2_999, ..sol() },
            // 80% LTV + 20% promo cap exceeds a 90% threshold.
            CollateralParams { ltv_bps: 8_000, ..sol() },
            // 90% threshold × 1.12 bonus exceeds 100%.
            CollateralParams { liquidation_bonus_bps: 1_200, ..sol() },
            // One unit below the lower bound: `0` is the escape hatch (no ceiling), but anything
            // strictly between `0` and `MULTIPLIER_ONE` is the "meant 1x, wrote 1" trap the rule
            // exists to catch. Pins the boundary itself, not just an extreme low value.
            CollateralParams { max_multiplier: MULTIPLIER_ONE - 1, ..sol() },
        ];
        for case in cases {
            assert!(case.validate(2_000).is_err(), "{case:?}");
        }
        // The promo cap is part of the rule: 80% LTV is fine with a 10% cap.
        CollateralParams { ltv_bps: 8_000, ..sol() }.validate(1_000).unwrap();
        // `max_multiplier`'s upper bound is inclusive: exactly `MAX_MULTIPLIER` still passes,
        // since a ceiling equal to the arithmetic cap is a real (if permissive) ceiling, not
        // the "can never bind" case `validate` rejects.
        CollateralParams { max_multiplier: MAX_MULTIPLIER, ..sol() }.validate(2_000).unwrap();
    }
}

#[cfg(test)]
mod price_source_tests {
    use super::*;

    fn pyth() -> CollateralParams {
        CollateralParams {
            price_source: PriceSource::Pyth,
            pyth_feed_id: [1; 32],
            price_account: Pubkey::default(),
            max_price_age_seconds: 60,
            sb_feed_hash: [0; 32],
            sb_max_stale_slots: 0,
            sb_min_samples: 0,
            max_conf_bps: 200,
            ltv_bps: 5_000,
            liquidation_threshold_bps: 7_500,
            liquidation_bonus_bps: 1_000,
            deposit_cap: u64::MAX,
            max_multiplier: 0,
        }
    }

    fn switchboard() -> CollateralParams {
        CollateralParams {
            price_source: PriceSource::SwitchboardOnDemand,
            pyth_feed_id: [0; 32],
            price_account: Pubkey::new_unique(),
            max_price_age_seconds: 0,
            sb_feed_hash: [7; 32],
            sb_max_stale_slots: 150,
            sb_min_samples: 3,
            ..pyth()
        }
    }

    /// An asset listed before this field existed carries zeroed padding where `price_source`
    /// now sits, so it has to deserialize as `Pyth` or every live position changes meaning on
    /// the next upgrade. This is the whole reason `Pyth` is declared first.
    #[test]
    fn pyth_is_the_zero_discriminant() {
        let mut buf = Vec::new();
        PriceSource::Pyth.serialize(&mut buf).unwrap();
        assert_eq!(buf, vec![0u8]);
        assert_eq!(PriceSource::try_from_slice(&[0u8]).unwrap(), PriceSource::Pyth);
    }

    #[test]
    fn both_sources_accept_a_well_formed_configuration() {
        pyth().validate(2_000).unwrap();
        switchboard().validate(2_000).unwrap();
    }

    /// A `PriceUpdateV2` carries a verified feed id, so the Pyth read binds itself to the
    /// asset and an unpinned account is merely a price-selection window. `PullFeedAccountData`
    /// carries no such binding, so the pinned address and the stored `feed_hash` are the only
    /// two things tying the read to this asset's feed — and the hash is what catches the feed
    /// authority repointing a pinned account's job.
    #[test]
    fn a_switchboard_asset_must_be_pinned_and_hash_bound() {
        let cases = [
            CollateralParams { price_account: Pubkey::default(), ..switchboard() },
            CollateralParams { sb_feed_hash: [0; 32], ..switchboard() },
            CollateralParams { sb_max_stale_slots: 0, ..switchboard() },
            CollateralParams { sb_max_stale_slots: MAX_COLLATERAL_STALE_SLOTS + 1, ..switchboard() },
            CollateralParams { sb_min_samples: 0, ..switchboard() },
        ];
        for case in cases {
            assert!(case.validate(2_000).is_err(), "{case:?}");
        }
        // The ceiling is inclusive, as `MAX_PRICE_AGE_SECONDS` is on the Pyth path.
        CollateralParams { sb_max_stale_slots: MAX_COLLATERAL_STALE_SLOTS, ..switchboard() }
            .validate(2_000)
            .unwrap();
    }

    /// Each source must leave the other's fields zero. A Pyth asset carrying a stale-slot
    /// bound, or a Switchboard asset carrying a `max_price_age_seconds`, advertises a freshness
    /// rule nothing ever reads — the kind of parameter an operator tunes and then cannot work
    /// out why it had no effect.
    #[test]
    fn neither_source_may_carry_the_others_fields() {
        let cases = [
            CollateralParams { sb_feed_hash: [7; 32], ..pyth() },
            CollateralParams { sb_max_stale_slots: 150, ..pyth() },
            CollateralParams { sb_min_samples: 3, ..pyth() },
            CollateralParams { pyth_feed_id: [1; 32], ..switchboard() },
            CollateralParams { max_price_age_seconds: 60, ..switchboard() },
        ];
        for case in cases {
            assert!(case.validate(2_000).is_err(), "{case:?}");
        }
    }

    /// The shared rules stay shared: an LTV, threshold, confidence or multiplier violation is
    /// rejected on the Switchboard path too, not only on the Pyth path that first defined them.
    #[test]
    fn the_shared_collateral_rules_still_apply_to_switchboard() {
        let cases = [
            CollateralParams { ltv_bps: 999, liquidation_threshold_bps: 2_999, ..switchboard() },
            CollateralParams { ltv_bps: 8_000, liquidation_threshold_bps: 9_000, ..switchboard() },
            CollateralParams { max_conf_bps: 10_001, ..switchboard() },
            CollateralParams { liquidation_bonus_bps: 1_200, liquidation_threshold_bps: 9_000, ltv_bps: 7_000, ..switchboard() },
            CollateralParams { max_multiplier: MULTIPLIER_ONE - 1, ..switchboard() },
        ];
        for case in cases {
            assert!(case.validate(2_000).is_err(), "{case:?}");
        }
    }
}

#[cfg(test)]
mod layout {
    use super::*;

    #[test]
    fn init_space_is_pinned_so_new_fields_come_out_of_the_padding() {
        // `price_account` was taken out of the reserved padding in Plan 2, and `kind` in Plan 4, so the account's size did not change. That is the whole
        // contract: a field added on top of `reserved` rather than out of it grows
        // `INIT_SPACE`, and every account already on chain is then too small to deserialize
        // into — with no error until someone touches one.
        //
        // `Position` pins the same property with `size_of` (it is zero-copy); these two are
        // Borsh, so `INIT_SPACE` is the number that matters. If this assertion fails, take the
        // bytes out of `reserved` instead of appending them.
        assert_eq!(CollateralAsset::INIT_SPACE, 294);
    }
}
