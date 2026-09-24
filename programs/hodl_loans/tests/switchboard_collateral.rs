//! Collateral priced by a Switchboard On-Demand pull feed instead of Pyth.
//!
//! Why the source exists: Pyth publishes no on-chain feed for a private-company SPV mark. Of the
//! eight PreStocks tokens (`.devnet/prestocks/FINDINGS.md`) Pyth's catalogue covers three, none
//! of the three is pushed on chain, and Hermes now requires an API key. The issuer's own HTTP
//! JSON endpoint prices all eight, and an HTTP endpoint is a Switchboard job.
//!
//! The numbers are the live OPENAI ones, checked 2026-09-24: $1,023.01 per **display** token at
//! a scaled-UI multiplier of 1.4861347. The harness's xStock mint carries 8 decimals where a
//! PreStocks mint carries 9; the borrow ceiling below is identical at both, because a share is a
//! share once `token_value` has divided by `10^decimals`.

mod common;

use anchor_lang::prelude::{AccountMeta, Pubkey};
use common::*;
use hodl_loans::{CollateralKind, HodlError, PriceSource};
use solana_signer::Signer;

const DAY: i64 = 86_400;
/// $1,023.01 at Switchboard's 18 decimals.
const OPENAI_USD_18: i128 = 1_023_010_000_000_000_000_000;
/// The same price at Pyth's exponent -8.
const OPENAI_USD_E8: i64 = 102_301_000_000;
/// OPENAI's live multiplier, effective since 2026-07-17.
const OPENAI_MULTIPLIER: f64 = 1.4861347;
/// USD per NGN, the value `Env::loan_ready` cranks the market feed to.
const NGN_USD_18: i128 = 625_000_000_000_000;
/// The largest loan one share backs at $1,023.01, a 1.4861347 multiplier and 50% LTV: 1,215,049
/// cNGN. Measured by bisection in `the_price_source_does_not_change_what_collateral_is_worth`
/// rather than derived, so a valuation change fails there instead of drifting silently. Pinning
/// the figure as well as the equivalence matters — an earlier run had both sources agreeing on
/// 1,214,355,468,750, which was the utilization cap rather than the LTV limit.
const CEILING: u64 = 1_215_049_478_079;

/// A borrower holding one collateral asset, with somewhere to receive cNGN.
struct Holder {
    borrower: Borrower,
    cngn_account: Pubkey,
}

fn holder(env: &mut Env, cngn: &Pubkey, mint: &Pubkey, amount: u64) -> Holder {
    let borrower = env.new_borrower();
    env.deposit_collateral(&borrower, mint, amount);
    let cngn_account = env.create_token_account(cngn, &borrower.pubkey());
    Holder { borrower, cngn_account }
}

fn borrow(env: &mut Env, cngn: &Pubkey, h: &Holder, amount: u64) -> TxResult {
    let prices = env.price_accounts(&h.borrower.pubkey());
    let instruction =
        take_loan_ix(&h.borrower.pubkey(), cngn, &h.cngn_account, amount, 30 * DAY, prices);
    send(&mut env.svm, &[instruction], &[&env.admin, &h.borrower.key])
}

/// The largest loan a one-share deposit of `stock` supports, by bisection — so no figure in this
/// file has to be derived by hand.
///
/// `build` returns a **fresh** environment for every probe, and that is the whole design. A
/// successful `take_loan` commits, so probes sharing one market accumulate utilization until the
/// 90% cap refuses a loan the LTV limit would have allowed. The first version of this helper
/// shared an environment and measured the same ceiling for both price sources — which looked like
/// the agreement being tested for, and was actually the utilization cap in both cases, a whisker
/// below the real LTV ceiling. The guard on the failure path is what turned that into a failure
/// instead of a passing test, and it is the reason this helper takes a builder.
fn borrow_ceiling(mut build: impl FnMut() -> (Env, Pubkey, Pubkey, u64)) -> u64 {
    let unhealthy = format!("Custom({})", u32::from(HodlError::Unhealthy));
    let (mut lo, mut hi) = (0u64, 3_000_000 * ONE_CNGN);
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        let (mut env, cngn, stock, one_share) = build();
        let h = holder(&mut env, &cngn, &stock, one_share);
        match borrow(&mut env, &cngn, &h, mid) {
            Ok(()) => lo = mid,
            Err(e) => {
                assert!(e.contains(&unhealthy), "bisection hit a non-LTV failure at {mid}: {e}");
                hi = mid - 1;
            }
        }
    }
    lo
}

/// A fresh market with one Pyth-priced xStock at OPENAI's price and multiplier.
fn pyth_env() -> (Env, Pubkey, Pubkey, u64) {
    let (mut env, base) = Env::loan_ready();
    let stock = env.list_xstock_collateral(1);
    env.set_pyth_price(&stock, OPENAI_USD_E8, 0);
    env.set_multiplier(&stock, OPENAI_MULTIPLIER, env.now() - 1);
    (env, base.cngn, stock, ONE_XSTOCK)
}

/// The same, priced by a Switchboard pull feed. `decimals` is a parameter so the 8-vs-9 question
/// can be asked separately from the price-source question.
fn switchboard_env(decimals: u8) -> (Env, Pubkey, Pubkey, u64) {
    let (mut env, base) = Env::loan_ready();
    let stock = env.list_switchboard_xstock(decimals, OPENAI_USD_18);
    env.set_multiplier(&stock, OPENAI_MULTIPLIER, env.now() - 1);
    (env, base.cngn, stock, 10u64.pow(decimals as u32))
}

/// The property that matters most: which oracle prices an asset must not change what the asset is
/// worth. Two xStocks, same decimals, same multiplier, same dollar price — one Pyth, one
/// Switchboard — must support the same largest loan, to the raw unit.
#[test]
fn the_price_source_does_not_change_what_collateral_is_worth() {
    // The listings record the source, and the Switchboard one records the two things that bind it.
    let (env, _, sb_stock, _) = switchboard_env(XSTOCK_DECIMALS);
    let asset = env.collateral(&sb_stock);
    assert_eq!(asset.price_source, PriceSource::SwitchboardOnDemand);
    assert_eq!(asset.kind, CollateralKind::XStock);
    assert_eq!(asset.sb_feed_hash, sb_feed_hash(&sb_stock));
    assert_eq!(asset.price_account, sb_feed_account(&sb_stock));
    let (env, _, pyth_stock, _) = pyth_env();
    assert_eq!(env.collateral(&pyth_stock).price_source, PriceSource::Pyth);

    let on_pyth = borrow_ceiling(pyth_env);
    let on_switchboard = borrow_ceiling(|| switchboard_env(XSTOCK_DECIMALS));

    assert_eq!(on_pyth, on_switchboard, "the oracle changed the valuation");
    assert_eq!(on_switchboard, CEILING);
}

/// The same ceiling at 9 decimals, which is what every PreStocks mint carries. Kept separate from
/// the source-equivalence test so each varies one thing.
#[test]
fn nine_decimals_values_the_same_as_eight() {
    assert_eq!(borrow_ceiling(|| switchboard_env(9)), CEILING);
}

/// The scaled-UI multiplier applies on the Switchboard path exactly as on the Pyth one. At
/// multiplier 1 the same deposit is worth 1/1.4861347 of what it is worth at OPENAI's live
/// multiplier, so a loan that fits at 1.4861347 must not fit at 1.
#[test]
fn the_multiplier_scales_a_switchboard_priced_asset() {
    let (mut env, base) = Env::loan_ready();
    let stock = env.list_switchboard_xstock(9, OPENAI_USD_18);
    let h = holder(&mut env, &base.cngn, &stock, 1_000_000_000);

    // Multiplier 1 — what `create_mint` leaves — cannot carry it.
    assert_hodl_error(borrow(&mut env, &base.cngn, &h, CEILING), HodlError::Unhealthy);

    // OPENAI's live multiplier can. Nothing about the feed changed.
    env.set_multiplier(&stock, OPENAI_MULTIPLIER, env.now() - 1);
    borrow(&mut env, &base.cngn, &h, CEILING).unwrap();
}

/// `sb_feed_hash` is the whole reason a pinned address is not enough. A pull feed's `authority`
/// can rewrite the account's `feed_hash`, repointing a pinned address from "PreStocks OPENAI mark
/// price" at any other job — the address, the owning program and the discriminator all still
/// check out, and the price the program reads is now someone else's.
///
/// Note which paths survive. Borrowing stops; depositing does not. Deposit reads no price, and
/// letting an oracle problem trap collateral would be a worse failure than the one it prevents.
#[test]
fn a_feed_repointed_to_another_job_stops_pricing_the_asset() {
    let (mut env, base) = Env::loan_ready();
    let stock = env.list_switchboard_xstock(9, OPENAI_USD_18);
    env.set_multiplier(&stock, OPENAI_MULTIPLIER, env.now() - 1);
    let h = holder(&mut env, &base.cngn, &stock, 1_000_000_000);

    env.repoint_switchboard_feed(&stock, OPENAI_USD_18, 0);
    assert_hodl_error(borrow(&mut env, &base.cngn, &h, CEILING), HodlError::PriceAccountMismatch);

    // Depositing more still works: no price is read on the way in.
    let owner = h.borrower.pubkey();
    let token = env.create_token_account(&stock, &owner);
    env.mint_to(&stock, &token, 1_000_000_000);
    let deposit = deposit_collateral_ix(&owner, &stock, &TOKEN_2022, &token, 1_000_000_000);
    env.sponsored(deposit, &h.borrower.key).unwrap();

    // Restoring the job the asset was listed against restores borrowing.
    env.set_switchboard_price(&stock, OPENAI_USD_18, 0);
    borrow(&mut env, &base.cngn, &h, CEILING).unwrap();
}

/// Unpinned is a coherent choice on the Pyth path — a `PriceUpdateV2` proves which feed it
/// carries. A `PullFeedAccountData` proves nothing of the kind, so the pinned address is load
/// bearing, and a listing that omits it is refused at the instruction rather than left to fail on
/// a later price read.
#[test]
fn a_malformed_switchboard_listing_is_refused() {
    let mut env = Env::initialized();
    let admin = env.admin.pubkey();
    let mint = env.create_mint(MintKind::XStock, 9);
    let good = switchboard_collateral_params(&mint);

    let cases = [
        ("unpinned", hodl_loans::CollateralParams { price_account: Pubkey::default(), ..good }),
        ("no job hash", hodl_loans::CollateralParams { sb_feed_hash: [0; 32], ..good }),
        ("no quorum", hodl_loans::CollateralParams { sb_min_samples: 0, ..good }),
        ("past the slot ceiling", hodl_loans::CollateralParams { sb_max_stale_slots: 151, ..good }),
        // A Pyth bound on a Switchboard asset would never be read, so it must not be settable.
        ("carries max_price_age_seconds", hodl_loans::CollateralParams { max_price_age_seconds: 60, ..good }),
        ("carries a pyth feed id", hodl_loans::CollateralParams { pyth_feed_id: [1; 32], ..good }),
    ];
    for (name, params) in cases {
        let listing = list_collateral_ix(&admin, &mint, &TOKEN_2022, params, CollateralKind::XStock);
        assert_hodl_error(send(&mut env.svm, &[listing], &[&env.admin]), HodlError::InvalidParameters);
        println!("refused: {name}");
    }

    // The well-formed listing goes through, so every rejection above is about the field under test
    // and not about the mint.
    let listing = list_collateral_ix(&admin, &mint, &TOKEN_2022, good, CollateralKind::XStock);
    send(&mut env.svm, &[listing], &[&env.admin]).unwrap();
}

/// A Switchboard result is bounded in slots, and the bound is the asset's own. Moving the wall
/// clock does not age it — the two sources genuinely measure freshness in different units, which
/// is why `sb_max_stale_slots` exists instead of reusing `max_price_age_seconds`.
#[test]
fn a_stale_switchboard_result_is_refused_at_the_assets_own_bound() {
    let (mut env, base) = Env::loan_ready();
    let stock = env.list_switchboard_xstock(9, OPENAI_USD_18);
    env.set_multiplier(&stock, OPENAI_MULTIPLIER, env.now() - 1);
    let h = holder(&mut env, &base.cngn, &stock, 1_000_000_000);
    let bound = env.collateral(&stock).sb_max_stale_slots;
    assert_eq!(bound, 150);
    let small = 1_000 * ONE_CNGN;

    // A whole day of wall clock leaves a slot-bounded result fresh.
    env.warp_seconds(DAY);
    borrow(&mut env, &base.cngn, &h, small).unwrap();

    // Exactly at the bound is still fresh. The NGN feed ages in the same unit, so re-crank it.
    env.warp_slots(bound);
    env.set_ngn_price(NGN_USD_18, 0);
    borrow(&mut env, &base.cngn, &h, small).unwrap();

    // One slot past it is not.
    env.set_switchboard_price(&stock, OPENAI_USD_18, 0);
    env.warp_slots(bound + 1);
    env.set_ngn_price(NGN_USD_18, 0);
    assert_hodl_error(borrow(&mut env, &base.cngn, &h, small), HodlError::StalePrice);
}

/// Supplying the wrong *kind* of price account fails on the owning program rather than reading one
/// layout as the other. This is what makes the branch in `valuation.rs` safe: the stride is
/// identical for both sources, so the owner check is all that separates them.
#[test]
fn a_pyth_account_where_a_switchboard_feed_belongs_is_refused() {
    let (mut env, base) = Env::loan_ready();
    let stock = env.list_switchboard_xstock(9, OPENAI_USD_18);
    env.set_multiplier(&stock, OPENAI_MULTIPLIER, env.now() - 1);
    let h = holder(&mut env, &base.cngn, &stock, 1_000_000_000);

    // A perfectly valid, perfectly fresh Pyth update for this very mint, in the slot the
    // Switchboard feed should occupy.
    env.set_pyth_price(&stock, OPENAI_USD_E8, 0);
    let forged = vec![
        AccountMeta::new_readonly(collateral_pda(&stock), false),
        AccountMeta::new_readonly(pyth_account(&stock), false),
        AccountMeta::new_readonly(stock, false),
    ];
    let instruction = take_loan_ix(
        &h.borrower.pubkey(), &base.cngn, &h.cngn_account, 1_000 * ONE_CNGN, 30 * DAY, forged,
    );
    assert_hodl_error(
        send(&mut env.svm, &[instruction], &[&env.admin, &h.borrower.key]),
        HodlError::PriceAccountMismatch,
    );
}
