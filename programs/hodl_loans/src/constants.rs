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

/// Upper bound on `CollateralParams::sb_max_stale_slots` — the slot-denominated equivalent of
/// `MAX_PRICE_AGE_SECONDS` for a collateral asset priced by a Switchboard On-Demand pull feed.
///
/// Switchboard's `CurrentResult` carries a `slot` and no timestamp, so a Switchboard-priced
/// asset can only be bounded in slots. 150 slots is the same 60 seconds at the 400 ms target,
/// which keeps the two collateral price sources equally strict on paper. They are not equally
/// strict in practice: slots stretch to 400-650 ms under load, so a slot bound drifts further
/// behind wall clock exactly when prices move fastest. That is the same trade
/// `MAX_NGN_STALE_SLOTS` already takes for the debt side of every health check.
pub const MAX_COLLATERAL_STALE_SLOTS: u64 = 150;

/// Upper bound on `Config::collateral_count`, derived from `set_promo_cap`, the one
/// instruction that must name **every** listed asset at once: its `remaining_accounts` count
/// has to equal `collateral_count` exactly.
///
/// The binding limit is **`MAX_TX_ACCOUNT_LOCKS = 128`** — the total accounts a transaction
/// may lock, read-only included. Address lookup tables relieve the *message size* limit (32
/// bytes per key), not the lock limit: an ALT-loaded address still takes a lock. The `u8`
/// account index caps a message at 256 keys, but that ceiling is never reached because the
/// lock limit bites at half of it. `SetPromoCap` spends three locks on the admin, the config
/// and the program id, leaving **125**.
///
/// 96 leaves 29 spare, for accounts a future `SetPromoCap` might need and for anything a
/// client's own lookup-table usage costs. The bound matters because there is no way back:
/// `collateral_count` only falls when an asset is delisted, and delisting requires the asset
/// to be unused, so a protocol that listed its way past the ceiling would have `set_promo_cap`
/// frozen until positions unwound. `the_asset_list_bound_keeps_set_promo_cap_inside_the_lock_limit`
/// pins the arithmetic so the constant cannot drift past it.
pub const MAX_LISTED_COLLATERAL: u16 = 96;
/// Upper bound on how far ahead of creation a campaign's `redeem_until` may sit, and so —
/// via the voucher-outlives-campaign rule in `redeem_promo` — on how long a voucher receipt
/// can hold its rent before `close_voucher_receipt` will take it.
///
/// Unlike `MAX_COLLATERAL_DECIMALS` this is a policy bound, not a derived one: no arithmetic
/// fails past it. A promo campaign still running a year after it was created is a decision
/// worth re-making by opening a new campaign, rather than one that should quietly keep
/// rent locked in receipts nobody can close.
pub const MAX_CAMPAIGN_LIFETIME: i64 = 365 * 86_400;

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

/// The Switchboard On-Demand program that must own the NGN feed (`oracle/switchboard.rs`).
///
/// Chosen at **compile time**, not stored in `Config`, and that is the whole point. The owner
/// check exists because `Market::ngn_feed` is an admin-settable bare `Pubkey` with nothing
/// behind it — a mis-set feed would have the program read 3.2 KB of arbitrary bytes as a
/// price. Putting the expected *owner* in an account would reintroduce exactly that shape one
/// level up: another admin-settable value that, set wrong, turns the check off. A deployed
/// binary cannot be misconfigured after the fact.
///
/// The cost is two binaries to keep straight, and a devnet build that is silently wrong if
/// someone forgets `--features devnet`. `the_switchboard_pid_matches_the_build` below pins
/// each `#[cfg]` arm against the crate's own constant, so an edit that swapped the two arms
/// cannot ship green. **It cannot detect a forgotten `--features devnet`** — the test is
/// selected by the exact same `cfg` as the constant, so the flag being absent looks identical
/// from inside the test to the flag never having been needed. That gap is closed by Gate B in
/// `docs/superpowers/runbooks/2026-09-23-devnet-deployment.md`, whose `.so`-hash comparison is
/// the check that actually catches a mis-built binary.
///
/// The `switchboard_on_demand` crate has its own selector, but it is client-only: it reads
/// `std::env::var("SB_ENV")`, which does not exist on SBF. The two PID constants themselves
/// are plain and usable on-chain, so we choose between them ourselves.
#[cfg(not(feature = "devnet"))]
pub const SWITCHBOARD_ON_DEMAND_PID: Pubkey = switchboard_on_demand::ON_DEMAND_MAINNET_PID;
#[cfg(feature = "devnet")]
pub const SWITCHBOARD_ON_DEMAND_PID: Pubkey = switchboard_on_demand::ON_DEMAND_DEVNET_PID;

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

    #[test]
    fn the_asset_list_bound_keeps_set_promo_cap_inside_the_lock_limit() {
        // `set_promo_cap` must name every listed asset in one transaction, so the asset list
        // is bounded by how many accounts a transaction may lock — not by how many a message
        // can index. Solana's `MAX_TX_ACCOUNT_LOCKS` is 128 (solana-transaction 3.1.0,
        // `sanitized.rs`); it is not importable from an on-chain crate, so it is restated
        // here and this test is what keeps the two in step.
        //
        // The first draft of `MAX_LISTED_COLLATERAL` was 128, derived from the 256-key `u8`
        // index limit, which is not the binding one — it would have permitted an asset list
        // that makes `set_promo_cap` permanently unsendable, the exact state the bound exists
        // to prevent. This assertion is why that cannot recur silently.
        const MAX_TX_ACCOUNT_LOCKS: usize = 128;
        // admin, config, program id.
        const SET_PROMO_CAP_FIXED_ACCOUNTS: usize = 3;
        assert!(
            MAX_LISTED_COLLATERAL as usize + SET_PROMO_CAP_FIXED_ACCOUNTS <= MAX_TX_ACCOUNT_LOCKS,
            "MAX_LISTED_COLLATERAL ({MAX_LISTED_COLLATERAL}) + {SET_PROMO_CAP_FIXED_ACCOUNTS} \
             exceeds MAX_TX_ACCOUNT_LOCKS ({MAX_TX_ACCOUNT_LOCKS}): set_promo_cap would be \
             unsendable at a full asset list"
        );
    }

    #[test]
    fn the_switchboard_pid_matches_the_build() {
        // Pins each `#[cfg]` arm above against the crate's own constant: `cargo test` and
        // `cargo test --features devnet` each assert their own half, so an edit that swapped
        // the two arms cannot ship green. This is NOT a check that the binary was built for the
        // right cluster — `cfg!(feature = "devnet")` here is the exact same signal that selected
        // `SWITCHBOARD_ON_DEMAND_PID` above, so a forgotten `--features devnet` changes both
        // sides of the assertion together and passes. Catching that gap is Gate B in
        // `docs/superpowers/runbooks/2026-09-23-devnet-deployment.md`, not this test.
        if cfg!(feature = "devnet") {
            assert_eq!(SWITCHBOARD_ON_DEMAND_PID, switchboard_on_demand::ON_DEMAND_DEVNET_PID);
        } else {
            assert_eq!(SWITCHBOARD_ON_DEMAND_PID, switchboard_on_demand::ON_DEMAND_MAINNET_PID);
        }
        // And the two are genuinely different, so the assertion above is not vacuous on a
        // future crate version that collapsed them.
        assert_ne!(
            switchboard_on_demand::ON_DEMAND_MAINNET_PID,
            switchboard_on_demand::ON_DEMAND_DEVNET_PID
        );
    }
}
