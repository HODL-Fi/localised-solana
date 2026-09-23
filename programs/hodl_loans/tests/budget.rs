mod common;

use anchor_lang::prelude::Pubkey;
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
    // the 9 existing overdue loans. Measured 88,512-88,543 CU over 20 runs (was 88,095-98,595 CU
    // over 55 runs pre-fix).
    //
    // **Plan 8 removed the multi-thousand-CU steps from these figures; it did not make them
    // literally single-valued.** A residual 0-109 CU of jitter remains (see the note below the
    // deltas) — small enough to compare a max against, which the geometric ladder never was.
    // Every instruction
    // that pins the position PDA now passes `bump = position.load()?.bump` rather than a bare
    // `bump`, so Anchor reads the stored bump instead of emitting `find_program_address`,
    // which tried candidates from 255 downward at ~1,500 CU each. A randomly-keyed borrower's
    // canonical bump is 255 with p≈1/2, 254 with p≈1/4, and so on — a geometric ladder with an
    // unbounded tail, and that is what every "measured X-Y over N runs" range in this file's
    // history was measuring. `Liquidate` and `RepayLoan` never constrained the position by
    // seeds, which is why their spreads were ~31-62 CU while the others ran to thousands.
    //
    // What the change bought, max-to-max against the ranges this file carried immediately
    // before this task (`take_loan` 88,095-98,595, `withdraw_collateral` 87,646-98,146, xStock
    // `take_loan` 95,990-104,990): `take_loan` -10,052, `withdraw_collateral` -10,170, xStock
    // `take_loan` -8,535. `revoke_promo` does not exist yet at this point in the plan — Task 7
    // adds it and is the one that gets to measure its drop. The `liquidate` / `repay_loan`
    // figures moved the other way, by roughly +300 to +420 CU: they never paid the search
    // either way, so this small rise is ordinary crate-wide inlining drift from the changed
    // code elsewhere in the crate (the same effect this file has always attributed a few tens of
    // CU of `repay_loan` drift to), not a cost the position-PDA fix adds to these instructions
    // directly. `open_position` still uses a bare bump because it is `init`: there is no stored
    // bump to read yet.
    //
    // Ranges below are min-max over 20 runs. The residual 0-109 CU left on these figures is
    // ordinary jitter, not a step — contrast with the multi-thousand-CU spreads the geometric
    // ladder used to produce on the same instructions.
    let prices = env.price_accounts(&owner);
    let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, prices);
    let cu = send_cu(&mut env.svm, &[ixn], &[&env.admin, &setup.borrower.key]).unwrap();
    assert!(cu < 115_000, "take_loan at 8 collateral slots / 9 existing overdue loans used {cu} CU");

    // withdraw_collateral's post-withdrawal health check walks the same 8 slots, the promo cap
    // and now 10 loans. Measured 87,976 CU, reproduced with zero spread across 20 runs (was
    // 87,646-98,146 CU over 55 runs pre-fix); see the note above.
    let token = env.create_token_account(&setup.usdc, &owner);
    let wd = withdraw_collateral_ix(&owner, &setup.usdc, &SPL_TOKEN, &token, Some(&setup.cngn), ONE_USDC, env.price_accounts(&owner));
    let cu = send_cu(&mut env.svm, &[wd], &[&env.admin, &setup.borrower.key]).unwrap();
    assert!(cu < 115_000, "withdraw_collateral at 8 collateral slots / 10 loans used {cu} CU");

    // liquidate prices all 8 collateral slots and all 10 loans, then moves two token types.
    // This position holds NO promo, so `forfeit_promo` resolves its two extra `Option` accounts
    // and takes the zero-balance early return rather than paying for a third CPI. Measured
    // 109,294-109,342 CU over 20 runs (was 108,971-109,033 CU over 55 runs pre-fix) —
    // `liquidate` never constrained the position by seeds, so it paid no search either way; the
    // small rise is ordinary crate-wide inlining drift from the position-PDA fix elsewhere in
    // the crate, not a cost this instruction itself now pays — see the note above. This is
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
    // Measured 19,305 CU, reproduced with zero spread across 20 runs (was 19,250-19,298 CU over
    // 55 runs pre-fix) — repay_loan walks no collateral and never constrained the position by
    // seeds, so this small rise is ordinary crate-wide inlining drift, not a cost the
    // position-PDA fix adds to this instruction directly; see the note above.
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
    // token CPI on top of the no-promo case's two. Measured 114,021-114,052 CU over 20 runs
    // (was 113,701-113,732 CU over 55 runs pre-fix) — see the note on
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
    // slots, the promo cap and 9 existing loans. **Measured 96,407-96,455 CU over 20 runs —
    // this is the figure spec §15 points at, and the only place it is written down. Quote the
    // range, not a sample.**
    //
    // Down 8,535 CU max-to-max from the 95,990-104,990 CU range this file carried immediately
    // before this task: `TakeLoan` now reads the position's stored bump
    // (`bump = position.load()?.bump`) instead of paying `find_program_address`'s
    // geometric-ladder search. See the note on
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
    // This figure is stable where the `take_loan` ones are not, and the reason is the
    // instruction, not the collateral: `Liquidate` takes the position with no seeds constraint,
    // so it never paid `find_program_address`'s bump search even before this task's fix (see
    // the note on `full_position_stays_under_the_default_compute_budget`). Same-shape collateral
    // has nothing to do with it — `an_all_xstock_position_stays_under_the_default_compute_budget`
    // above holds eight identical xStocks and is a `take_loan`, so it still moved by thousands.
    // Measured 120,544-120,653 CU over 20 runs, a spread of 109 rather than the thousands
    // `take_loan` used to show — up from 120,224-120,255 CU over 55 runs pre-fix, ordinary
    // crate-wide inlining drift from the position-PDA fix elsewhere in the crate. Not zero,
    // though — do not restate this as an exact figure. The ceiling below still leaves the same
    // margin the sibling standard-collateral forfeit case above does.
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

/// `set_promo_cap` is the one instruction whose cost scales with an admin-controlled list:
/// `MAX_LISTED_COLLATERAL` assets, each deserialized, PDA-re-derived and re-validated. The
/// bound on that list was derived entirely from `MAX_TX_ACCOUNT_LOCKS` — nothing has ever
/// measured the compute, and 96 assets is well past what the 200,000 default budget covers.
/// This measures the real per-asset cost and states what a full list implies.
#[test]
fn set_promo_cap_costs_scale_with_the_asset_list() {
    let mut env = Env::initialized();
    let admin = env.admin.pubkey();
    let mut assets: Vec<Pubkey> = Vec::new();

    // Ten is enough to fit a legacy transaction and to fix the slope; the interesting figure
    // is per-asset, not the total at ten.
    for _ in 0..10 {
        assets.push(env.list_spl_collateral(6));
    }
    let at_ten = send_cu(&mut env.svm, &[set_promo_cap_ix(&admin, 2_000, &assets)], &[&env.admin]).unwrap();

    // The account count must equal `collateral_count` exactly, so a shorter list cannot be
    // measured on the same env — take the two-asset reading from a fresh one and subtract.
    let mut small = Env::initialized();
    let mut two_assets = Vec::new();
    for _ in 0..2 {
        two_assets.push(small.list_spl_collateral(6));
    }
    let small_admin = small.admin.pubkey();
    let at_two =
        send_cu(&mut small.svm, &[set_promo_cap_ix(&small_admin, 2_000, &two_assets)], &[&small.admin]).unwrap();

    let per_asset = (at_ten - at_two) / 8;
    // **Measured, and unlike the walking figures elsewhere in this file these are
    // deterministic** — no position PDA, so no `find_program_address` bump search: 10,023 CU
    // at two assets, 31,095 at ten, **2,634 CU per asset**, reproduced with zero spread across
    // 8 runs (5 against one SBF build, 3 against a from-scratch rebuild to rule out a stale
    // `.so`). That is a 294-byte Borsh deserialize plus the `create_program_address` this
    // instruction re-derives per asset (~1,587 CU per syscall) plus `validate`. Unaffected by
    // this task's position-PDA fix, as expected — re-confirmed identical (10,023 / 31,095 /
    // 2,634, zero spread) across 20 further runs taken alongside the rest of this file's
    // re-measurement.
    //
    // At the `MAX_LISTED_COLLATERAL` bound of 96 that extrapolates to **~257,600 CU — past the
    // 200,000 default budget.** An admin at a full asset list must send an explicit
    // `ComputeBudgetInstruction::set_compute_unit_limit`; without one, `set_promo_cap` starts
    // failing at 75 assets (74 still fits: 10,023 + 2,634*72 = 199,671 CU; 75 assets does not:
    // 10,023 + 2,634*73 = 202,305 CU). It fits well inside the 1.4M maximum and the extra
    // program key still leaves ~100 of the 128 account locks, so the 96 bound is sound — but
    // the default budget stops covering it first, which is not something the bound's own
    // derivation (account locks) would ever tell you.
    assert!(
        (2_400..2_900).contains(&per_asset),
        "per-asset cost moved to {per_asset} CU from the measured 2,634 — re-derive what the \
         96-asset bound implies for compute before accepting this"
    );
    let at_bound = at_two + per_asset * (hodl_loans::MAX_LISTED_COLLATERAL as u64 - 2);
    assert!(
        at_bound > 200_000,
        "a full asset list now fits the default budget ({at_bound} CU) — the warning above is \
         stale and should be removed"
    );
    // And the default budget runs out well before the bound does.
    let affordable = 2 + (200_000 - at_two) / per_asset;
    assert!(
        affordable < hodl_loans::MAX_LISTED_COLLATERAL as u64,
        "{affordable} assets now fit the default budget, at or past the {} bound",
        hodl_loans::MAX_LISTED_COLLATERAL
    );
}
