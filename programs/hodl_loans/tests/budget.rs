mod common;

use common::*;
use solana_signer::Signer;

const DAY: i64 = 86_400;

/// A position at its structural limits — all 8 collateral slots occupied and all 10 loan slots
/// active and overdue (so penalty math runs on every loan) — stays comfortably under Solana's
/// 200,000 CU default transaction budget. The ceilings below give generous headroom over what
/// was actually measured so the test still catches a real budget regression.
#[test]
fn full_position_stays_under_the_default_compute_budget() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();

    // `Env::loan_ready` already deposits usdc into slot 0; 7 more mints fill the other 7 slots.
    let mut mints = vec![setup.usdc];
    for i in 0..7 {
        let decimals = if i % 2 == 0 { 9 } else { 6 };
        let mint = env.list_spl_collateral(decimals);
        env.set_pyth_price(&mint, 150 * ONE_DOLLAR, 10_000_000);
        env.deposit_collateral(&setup.borrower, &mint, 10u64.pow(decimals as u32) * 10);
        mints.push(mint);
    }
    assert_eq!(mints.len(), 8);

    // Fill 9 of the position's 10 loan slots, each priced against all 8 collateral slots.
    for i in 0..9 {
        let prices = env.price_accounts(&owner);
        let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, prices);
        send_cu(&mut env.svm, &[ixn], &[&env.admin, &setup.borrower.key]).unwrap_or_else(|e| panic!("take_loan #{i} failed: {e}"));
    }

    // Warp past every loan's 30-day maturity so penalty math runs on all of them, then refresh
    // both price feeds (they are now stale).
    env.warp_seconds(40 * DAY);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    for m in &mints {
        let (price, conf) = if *m == setup.usdc { (ONE_DOLLAR, 0) } else { (150 * ONE_DOLLAR, 10_000_000) };
        env.set_pyth_price(m, price, conf);
    }

    // 10th (last) loan slot: the health check walks all 8 collateral slots, the promo cap and
    // the 9 existing overdue loans. Measured 87,461-93,461 CU over 55 runs (was 74,763-79,263 CU
    // over 17 runs pre-Plan-7).
    //
    // These figures are NOT deterministic: the harness keys its mints randomly, so where a
    // target sorts into the collateral slot array shifts the scan, moving the cost in steps of
    // ~1,500 CU. Each range below is min-max observed over 55 runs, and the tail is not fully
    // characterised — separate batches produced different maxima. Compare against the max,
    // never a single sample.
    //
    // **The `create_program_address` syscall this walk added costs ~1,587-1,588 CU per
    // collateral slot, stable — ~12,700 CU at eight slots.** That is min-to-min against the
    // pre-Plan-7 baselines above: (87,461-74,763)/8 = 1,587.25 here, (86,883-74,182)/8 = 1,587.6
    // for `withdraw_collateral` below.
    //
    // Subtracting the recorded range ENDPOINTS instead does not give that number, and the gap is
    // not noise: max-to-max here is (93,461-79,263)/8 = 1,774.75, and (92,883-78,682)/8 =
    // 1,775.125 for `withdraw_collateral` — both a real ~190 CU/slot higher, not a rounding
    // difference. The cause is unequal bucket coverage, not a variable cost: this task's 55-run
    // samples hit 5 distinct ~1,500-CU-spaced slot-order buckets for both figures (confirmed in
    // the raw per-run data — 87,461/88,961/90,461/91,961/93,461 and
    // 86,883/88,383/89,883/91,383/92,883, each with ±30 CU of sub-noise on top; I checked this
    // against the raw runs, not just the two recorded endpoints), while the pre-Plan-7 baseline's
    // 17 runs only span a 4,500 CU range — 4 buckets at the same 1,500 CU step, inferred from its
    // documented spread since that raw data is git history, not something I hold to re-measure.
    // A 5-bucket max minus a 4-bucket max charges the syscall for one extra, unsampled step it
    // didn't cause; min-to-min avoids this because bucket 0 (cheapest slot order) is common
    // enough to land in both a 17-run and a 55-run batch. The xStock `take_loan` case below is
    // the control that shows the mechanism cleanly: there, pre (17 runs) and post (55 runs) both
    // happen to span 5 buckets, so min-to-min and max-to-max agree exactly — 12,694 either way.
    //
    // The two `liquidate` paths below (with and without a promo forfeit) are a separate, genuine
    // effect, not this same artifact: both are stable (essentially single-valued, no stepped
    // bucketing) on both sides of the change, so no coverage correction applies, and their
    // measured delta is still only ~12,044-12,077 CU across the same eight slots
    // (~1,506-1,510 CU/slot) — a few hundred CU below the ~1,587-1,588 figure above.
    // `repay_loan`, which walks no collateral, moved only +30 CU at the observed max
    // (19,247-19,278 CU vs. 19,248 CU before pre-Plan-7). Adding code to the crate shifts
    // inlining decisions across it, so figures drift by tens of CU on changes that do not touch
    // the measured path at all. Treat a drift of that order as noise and anything near 1,500 as
    // the slot-order variance above.
    //
    // The cost buys a local admission argument in place of a whole-program one, and these
    // markers exist to make a change of this size visible rather than to veto it; nothing
    // here is near the 200,000 default budget. Spec §15 and
    // `docs/superpowers/plans/2026-09-21-plan-7-followups.md` carry the reasoning.
    let prices = env.price_accounts(&owner);
    let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, prices);
    let cu = send_cu(&mut env.svm, &[ixn], &[&env.admin, &setup.borrower.key]).unwrap();
    assert!(cu < 115_000, "take_loan at 8 collateral slots / 9 existing overdue loans used {cu} CU");

    // withdraw_collateral's post-withdrawal health check walks the same 8 slots, the promo cap
    // and now 10 loans. Measured 86,883-92,883 CU over 55 runs (was 74,182-78,682 CU over 17 runs
    // pre-Plan-7); see the note above.
    let token = env.create_token_account(&setup.usdc, &owner);
    let wd = withdraw_collateral_ix(&owner, &setup.usdc, &SPL_TOKEN, &token, Some(&setup.cngn), ONE_USDC, env.price_accounts(&owner));
    let cu = send_cu(&mut env.svm, &[wd], &[&env.admin, &setup.borrower.key]).unwrap();
    assert!(cu < 115_000, "withdraw_collateral at 8 collateral slots / 10 loans used {cu} CU");

    // liquidate prices all 8 collateral slots and all 10 loans, then moves two token types.
    // This position holds NO promo, so `forfeit_promo` resolves its two extra `Option` accounts
    // and takes the zero-balance early return rather than paying for a third CPI — the two
    // accounts alone are still ~9,900 CU over the pre-Task-7 baseline of 86,320. Measured
    // 108,208-108,239 CU over 55 runs, up from 96,162 pre-Plan 7 (see the note above). This is
    // the CHEAP no-forfeit path, not the most expensive liquidate path
    // overall — see `full_position_liquidation_with_promo_forfeit_stays_under_the_default_compute_budget`
    // below for the case where the forfeit actually fires (token CPI + event).
    // Crash every collateral price to $0.001 so the position is liquidatable.
    for m in &mints {
        env.set_pyth_price(m, 100_000, 0);
    }
    let liquidator = env.new_liquidator(&setup.cngn, 100_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    let prices = env.price_accounts(&owner);
    let lq = liquidate_ix(
        &liquidator.pubkey(), &owner, &setup.cngn, &liquidator.cngn, &setup.usdc, &SPL_TOKEN,
        &seized_to, 0, 100 * ONE_CNGN, prices,
    );
    let cu = send_cu(&mut env.svm, &[lq], &[&liquidator.key]).unwrap();
    assert!(cu < 120_000, "liquidate at 8 collateral slots / 10 loans used {cu} CU");

    // repay_loan needs no price accounts but still scans all 10 loan slots to find loan 0.
    // Measured 19,247-19,278 CU over 55 runs (was 19,248 CU pre-Plan 7) — repay_loan walks no
    // collateral, so this is ordinary inlining drift, not the syscall cost; see the note above.
    env.mint_to(&setup.cngn, &setup.borrower_cngn, 100_000 * ONE_CNGN);
    let rp = repay_loan_ix(&owner, &owner, &setup.cngn, &setup.borrower_cngn, 0, u64::MAX);
    let cu = send_cu(&mut env.svm, &[rp], &[&env.admin, &setup.borrower.key]).unwrap();
    assert!(cu < 25_000, "repay_loan at 10 loan slots used {cu} CU");
}

/// Same shape as `full_position_stays_under_the_default_compute_budget`'s liquidate case, but
/// the position actually holds promo, so `forfeit_promo` pays for its token CPI, its
/// `require_invariant` check and its event — not the zero-balance early return. This is the
/// most expensive `liquidate` path the program can be asked to run.
#[test]
fn full_position_liquidation_with_promo_forfeit_stays_under_the_default_compute_budget() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();

    // Fund the promo vault, open a campaign, and redeem promo onto the position so the final
    // liquidate below actually forfeits a nonzero balance.
    let admin = env.admin.pubkey();
    let source = env.create_token_account(&setup.cngn, &admin);
    env.mint_to(&setup.cngn, &source, 10_000_000 * ONE_CNGN);
    let fund = fund_promo_vault_ix(&admin, &setup.cngn, &source, 10_000_000 * ONE_CNGN);
    send(&mut env.svm, &[fund], &[&env.admin]).expect("fund promo vault");
    env.create_campaign(&setup.cngn, 1, 5_000_000 * ONE_CNGN);
    env.redeem_promo(&setup.borrower, &setup.cngn, 1, 50_000 * ONE_CNGN, 7).unwrap();

    // `Env::loan_ready` already deposits usdc into slot 0; 7 more mints fill the other 7 slots.
    let mut mints = vec![setup.usdc];
    for i in 0..7 {
        let decimals = if i % 2 == 0 { 9 } else { 6 };
        let mint = env.list_spl_collateral(decimals);
        env.set_pyth_price(&mint, 150 * ONE_DOLLAR, 10_000_000);
        env.deposit_collateral(&setup.borrower, &mint, 10u64.pow(decimals as u32) * 10);
        mints.push(mint);
    }
    assert_eq!(mints.len(), 8);

    // Fill 9 of the position's 10 loan slots, each priced against all 8 collateral slots.
    for i in 0..9 {
        let prices = env.price_accounts(&owner);
        let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, prices);
        send_cu(&mut env.svm, &[ixn], &[&env.admin, &setup.borrower.key]).unwrap_or_else(|e| panic!("take_loan #{i} failed: {e}"));
    }

    // Warp past every loan's 30-day maturity so penalty math runs on all of them, then refresh
    // both price feeds (they are now stale).
    env.warp_seconds(40 * DAY);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    for m in &mints {
        let (price, conf) = if *m == setup.usdc { (ONE_DOLLAR, 0) } else { (150 * ONE_DOLLAR, 10_000_000) };
        env.set_pyth_price(m, price, conf);
    }

    // 10th (last) loan slot, filling the position out to its structural limits.
    let prices = env.price_accounts(&owner);
    let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, prices);
    send_cu(&mut env.svm, &[ixn], &[&env.admin, &setup.borrower.key]).unwrap();

    // liquidate prices all 8 collateral slots and all 10 loans, moves the liquidator's
    // repayment and the seized collateral, and now also forfeits the promo balance: a third
    // token CPI on top of the no-promo case's two. Measured 112,938-112,986 CU over 55 runs, up
    // from 100,884-100,915 pre-Plan 7 — see the note on
    // `full_position_stays_under_the_default_compute_budget` above.
    for m in &mints {
        env.set_pyth_price(m, 100_000, 0);
    }
    let liquidator = env.new_liquidator(&setup.cngn, 100_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    let prices = env.price_accounts(&owner);
    let lq = liquidate_ix(
        &liquidator.pubkey(), &owner, &setup.cngn, &liquidator.cngn, &setup.usdc, &SPL_TOKEN,
        &seized_to, 0, 100 * ONE_CNGN, prices,
    );
    let cu = send_cu(&mut env.svm, &[lq], &[&liquidator.key]).unwrap();
    assert_eq!(env.position(&owner).promo_balance, 0, "the forfeit must actually have fired");
    assert!(cu < 120_000, "liquidate-with-forfeit at 8 collateral slots / 10 loans used {cu} CU");
}

/// Pins `liquidate`'s legacy transaction size: the two promo accounts cost exactly +66 bytes
/// of the 1,232-byte legacy budget, permanently and unconditionally, whether or not the
/// position holds promo. litesvm does not enforce the packet limit, so a passing test suite is
/// not evidence this fits — this assertion is. Measured 1,185 bytes.
#[test]
fn liquidate_fits_a_legacy_transaction_at_eight_collateral_slots() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();

    let mut mints = vec![setup.usdc];
    for i in 0..7 {
        let decimals = if i % 2 == 0 { 9 } else { 6 };
        let mint = env.list_spl_collateral(decimals);
        env.set_pyth_price(&mint, 150 * ONE_DOLLAR, 10_000_000);
        env.deposit_collateral(&setup.borrower, &mint, 10u64.pow(decimals as u32) * 10);
        mints.push(mint);
    }
    assert_eq!(mints.len(), 8);

    let prices = env.price_accounts(&owner);
    let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 100_000 * ONE_CNGN, 30 * DAY, prices);
    send(&mut env.svm, &[ixn], &[&env.admin, &setup.borrower.key]).unwrap();

    for m in &mints {
        env.set_pyth_price(m, 100_000, 0);
    }
    let liquidator = env.new_liquidator(&setup.cngn, 100_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    let prices = env.price_accounts(&owner);
    let lq = liquidate_ix(
        &liquidator.pubkey(), &owner, &setup.cngn, &liquidator.cngn, &setup.usdc, &SPL_TOKEN,
        &seized_to, 0, 100 * ONE_CNGN, prices,
    );
    let size = legacy_tx_size(&lq, &liquidator.pubkey(), 1);
    assert!(size <= PACKET_DATA_SIZE, "liquidate at 8 collateral slots no longer fits a legacy transaction ({size} bytes)");
}

/// The two-sided counterpart to `liquidate_fits_a_legacy_transaction_at_eight_collateral_slots`
/// for spec §15's tighter promise: `liquidate` fits a legacy transaction with at most ONE xStock
/// slot among its eight, not two — an xStock slot's third account (its mint, for the scaled-UI
/// multiplier) costs enough that a second one no longer fits. Measured 1,218 bytes at 7
/// Standard + 1 xStock (14 bytes of margin under the 1,232-byte limit) and 1,251 bytes at 6
/// Standard + 2 xStock.
#[test]
fn liquidate_fits_a_legacy_transaction_with_one_xstock_slot_but_not_two() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();

    // 7 Standard + 1 xStock slot.
    let mut mints = vec![setup.usdc];
    for i in 0..6 {
        let decimals = if i % 2 == 0 { 9 } else { 6 };
        let mint = env.list_spl_collateral(decimals);
        env.set_pyth_price(&mint, 150 * ONE_DOLLAR, 10_000_000);
        env.deposit_collateral(&setup.borrower, &mint, 10u64.pow(decimals as u32) * 10);
        mints.push(mint);
    }
    let xstock = env.list_xstock_collateral(200);
    env.deposit_collateral(&setup.borrower, &xstock, 8 * ONE_XSTOCK);
    mints.push(xstock);
    assert_eq!(mints.len(), 8);

    let prices = env.price_accounts(&owner);
    let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 100_000 * ONE_CNGN, 30 * DAY, prices);
    send(&mut env.svm, &[ixn], &[&env.admin, &setup.borrower.key]).unwrap();

    for m in &mints {
        env.set_pyth_price(m, 100_000, 0);
    }
    let liquidator = env.new_liquidator(&setup.cngn, 100_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    let prices = env.price_accounts(&owner);
    let lq = liquidate_ix(
        &liquidator.pubkey(), &owner, &setup.cngn, &liquidator.cngn, &setup.usdc, &SPL_TOKEN,
        &seized_to, 0, 100 * ONE_CNGN, prices,
    );
    let size = legacy_tx_size(&lq, &liquidator.pubkey(), 1);
    assert!(
        size <= PACKET_DATA_SIZE,
        "liquidate at 7 standard + 1 xStock slot no longer fits a legacy transaction ({size} bytes)"
    );
}

#[test]
fn liquidate_no_longer_fits_a_legacy_transaction_with_two_xstock_slots() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();

    // 6 Standard + 2 xStock slots.
    let mut mints = vec![setup.usdc];
    for i in 0..5 {
        let decimals = if i % 2 == 0 { 9 } else { 6 };
        let mint = env.list_spl_collateral(decimals);
        env.set_pyth_price(&mint, 150 * ONE_DOLLAR, 10_000_000);
        env.deposit_collateral(&setup.borrower, &mint, 10u64.pow(decimals as u32) * 10);
        mints.push(mint);
    }
    for _ in 0..2 {
        let xstock = env.list_xstock_collateral(200);
        env.deposit_collateral(&setup.borrower, &xstock, 8 * ONE_XSTOCK);
        mints.push(xstock);
    }
    assert_eq!(mints.len(), 8);

    let prices = env.price_accounts(&owner);
    let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 100_000 * ONE_CNGN, 30 * DAY, prices);
    send(&mut env.svm, &[ixn], &[&env.admin, &setup.borrower.key]).unwrap();

    for m in &mints {
        env.set_pyth_price(m, 100_000, 0);
    }
    let liquidator = env.new_liquidator(&setup.cngn, 100_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    let prices = env.price_accounts(&owner);
    let lq = liquidate_ix(
        &liquidator.pubkey(), &owner, &setup.cngn, &liquidator.cngn, &setup.usdc, &SPL_TOKEN,
        &seized_to, 0, 100 * ONE_CNGN, prices,
    );
    let size = legacy_tx_size(&lq, &liquidator.pubkey(), 1);
    assert!(
        size > PACKET_DATA_SIZE,
        "liquidate at 6 standard + 2 xStock slots now fits a legacy transaction ({size} bytes)"
    );
}

/// An all-xStock position is the most expensive health check the program can be asked to run:
/// three accounts and a mint unpack per slot instead of two accounts and none.
#[test]
fn an_all_xstock_position_stays_under_the_default_compute_budget() {
    let (mut env, setup) = Env::loan_ready();
    let borrower = env.new_borrower();
    let owner = borrower.pubkey();
    let borrower_cngn = env.create_token_account(&setup.cngn, &owner);
    let setup = LoanSetup { borrower, borrower_cngn, ..setup };

    // All 8 collateral slots hold an xStock priced at $200 a share, so 8 shares is $1,600.
    for _ in 0..8 {
        let mint = env.list_xstock_collateral(200);
        env.deposit_collateral(&setup.borrower, &mint, 8 * ONE_XSTOCK);
    }
    assert_eq!(env.price_accounts(&owner).len(), 24);

    for i in 0..9 {
        let prices = env.price_accounts(&owner);
        let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, prices);
        send_cu(&mut env.svm, &[ixn], &[&env.admin, &setup.borrower.key]).unwrap_or_else(|e| panic!("take_loan #{i} failed: {e}"));
    }

    // 10th (last) loan slot: the health check unpacks 8 mints on top of the usual 8 collateral
    // slots, the promo cap and 9 existing loans. **Measured 95,364-101,364 CU over 55 runs —
    // this is the figure spec §15 points at, and the only place it is written down. Quote the
    // range, not a sample: two batches of 25 and 30 runs produced maxima 1,500 CU apart.**
    //
    // Up from 82,670-88,670 pre-Plan 7 — a rise of ~12,700 CU, one `create_program_address`
    // syscall per collateral slot; see the note on
    // `full_position_stays_under_the_default_compute_budget` above.
    //
    // It is a floor for mainnet, not an estimate of it: `MintKind::XStock` initializes a
    // metadata *pointer* but writes no `TokenMetadata` extension, so the fixture mint is
    // smaller than a live Backed xStock, which carries name, symbol and URI. Unpacking the
    // real mint and walking its TLV entries costs more, so a mainnet `take_loan` reads
    // somewhat higher than this.
    let prices = env.price_accounts(&owner);
    let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, prices);
    let cu = send_cu(&mut env.svm, &[ixn], &[&env.admin, &setup.borrower.key]).unwrap();
    assert!(cu < 115_000, "take_loan at 8 xStock slots / 9 existing loans used {cu} CU");

    // Compute is not the binding limit here: at three accounts a slot the transaction no longer
    // fits in a packet, so such a position needs a v0 transaction with an address lookup table.
    // Measured 1,346 bytes against the 1,232-byte limit.
    //
    // This assertion pins a limitation rather than a behaviour, so it is kept deliberately: spec
    // §15 tells clients and liquidators they must use versioned transactions with lookup tables
    // for an all-xStock position, and that promise is only true while this holds. If a future
    // change shrinks the layout enough to fit, this test failing is the prompt to rewrite §15 —
    // a spec change, not a broken test.
    let prices = env.price_accounts(&owner);
    let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, prices);
    let size = legacy_tx_size(&ixn, &env.admin.pubkey(), 2);
    assert!(size > PACKET_DATA_SIZE, "8 xStock slots now fit a legacy transaction ({size} bytes)");
}

/// The most expensive `liquidate` call the program can be asked to run: 8 xStock collateral
/// slots (three accounts and a mint unpack each, instead of two accounts and none) combined
/// with 10 active overdue loans AND a promo forfeit. xStocks (Plan 4) and the forfeit path
/// (Task 7) were built weeks apart and this combination had never been exercised before —
/// `liquidate` has blown the 4 KB SBF stack before, so the worst case across both features
/// needs its own ceiling rather than trusting the two cheaper cases above to bound it.
#[test]
fn full_all_xstock_position_liquidation_with_promo_forfeit_stays_under_the_default_compute_budget() {
    let (mut env, setup) = Env::loan_ready();
    let borrower = env.new_borrower();
    let owner = borrower.pubkey();
    let borrower_cngn = env.create_token_account(&setup.cngn, &owner);
    let setup = LoanSetup { borrower, borrower_cngn, ..setup };

    // Fund the promo vault, open a campaign, and redeem promo onto the position so the final
    // liquidate below actually forfeits a nonzero balance.
    let admin = env.admin.pubkey();
    let source = env.create_token_account(&setup.cngn, &admin);
    env.mint_to(&setup.cngn, &source, 10_000_000 * ONE_CNGN);
    let fund = fund_promo_vault_ix(&admin, &setup.cngn, &source, 10_000_000 * ONE_CNGN);
    send(&mut env.svm, &[fund], &[&env.admin]).expect("fund promo vault");
    env.create_campaign(&setup.cngn, 1, 5_000_000 * ONE_CNGN);
    env.redeem_promo(&setup.borrower, &setup.cngn, 1, 50_000 * ONE_CNGN, 7).unwrap();

    // All 8 collateral slots hold an xStock priced at $200 a share, so 8 shares is $1,600.
    let mut mints = Vec::new();
    for _ in 0..8 {
        let mint = env.list_xstock_collateral(200);
        env.deposit_collateral(&setup.borrower, &mint, 8 * ONE_XSTOCK);
        mints.push(mint);
    }
    assert_eq!(env.price_accounts(&owner).len(), 24);

    // Fill 9 of the position's 10 loan slots, each priced against all 8 xStock slots.
    for i in 0..9 {
        let prices = env.price_accounts(&owner);
        let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, prices);
        send_cu(&mut env.svm, &[ixn], &[&env.admin, &setup.borrower.key]).unwrap_or_else(|e| panic!("take_loan #{i} failed: {e}"));
    }

    // Warp past every loan's 30-day maturity so penalty math runs on all of them, then refresh
    // both price feeds (they are now stale).
    env.warp_seconds(40 * DAY);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    for m in &mints {
        env.set_pyth_price(m, 200 * ONE_DOLLAR, 0);
    }

    // 10th (last) loan slot, filling the position out to its structural limits.
    let prices = env.price_accounts(&owner);
    let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, prices);
    send_cu(&mut env.svm, &[ixn], &[&env.admin, &setup.borrower.key]).unwrap();

    // Crash every xStock price so the position is liquidatable, then liquidate: 8 xStock slots
    // priced (three accounts and a mint unpack each), all 10 loans scanned, the liquidator's
    // repayment and the seized collateral moved, and the promo forfeited on top of it all — a
    // fourth token CPI beyond the standard-collateral no-forfeit case's two.
    //
    // This suite's CU is not deterministic in general — the harness keys its mints randomly, so
    // where a target sorts into the collateral slot array can shift the scan in ~1,500 CU steps
    // (see the note on `full_position_stays_under_the_default_compute_budget`) — but every slot
    // here holds the same asset shape (an xStock), so that sort order has nothing to bite on and
    // the big step disappears: measured 119,469-119,561 CU over 55 runs, a spread of ~92 rather
    // than ~1,500 — up from 107,425-107,487 pre-Plan 7 (a rise of ~12,050-12,070 CU; see the
    // note on `full_position_stays_under_the_default_compute_budget` above). Not zero, though —
    // do not restate this as an exact figure. The ceiling below still leaves the same margin the
    // sibling standard-collateral forfeit case above does.
    for m in &mints {
        env.set_pyth_price(m, 100_000, 0);
    }
    let liquidator = env.new_liquidator(&setup.cngn, 100_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&mints[0], &liquidator.pubkey());
    let prices = env.price_accounts(&owner);
    let lq = liquidate_ix(
        &liquidator.pubkey(), &owner, &setup.cngn, &liquidator.cngn, &mints[0], &TOKEN_2022,
        &seized_to, 0, 100 * ONE_CNGN, prices,
    );
    let cu = send_cu(&mut env.svm, &[lq], &[&liquidator.key]).unwrap();
    assert_eq!(env.position(&owner).promo_balance, 0, "the forfeit must actually have fired");
    assert!(cu < 130_000, "liquidate-with-forfeit at 8 xStock slots / 10 loans used {cu} CU");
}
