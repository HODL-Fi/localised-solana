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

    // 10th (last) loan slot: the health check walks all 8 collateral slots and the 9 existing
    // overdue loans. Measured 66,126 CU.
    let prices = env.price_accounts(&owner);
    let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, prices);
    let cu = send_cu(&mut env.svm, &[ixn], &[&env.admin, &setup.borrower.key]).unwrap();
    assert!(cu < 75_000, "take_loan at 8 collateral slots / 9 existing overdue loans used {cu} CU");

    // withdraw_collateral's post-withdrawal health check walks the same 8 slots and now 10
    // loans. Measured 70,170 CU.
    let token = env.create_token_account(&setup.usdc, &owner);
    let wd = withdraw_collateral_ix(&owner, &setup.usdc, &SPL_TOKEN, &token, Some(&setup.cngn), ONE_USDC, env.price_accounts(&owner));
    let cu = send_cu(&mut env.svm, &[wd], &[&env.admin, &setup.borrower.key]).unwrap();
    assert!(cu < 75_000, "withdraw_collateral at 8 collateral slots / 10 loans used {cu} CU");

    // liquidate prices all 8 collateral slots and all 10 loans, then moves two token types.
    // Measured 83,018 CU. Crash every collateral price to $0.001 so the position is liquidatable.
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
    assert!(cu < 100_000, "liquidate at 8 collateral slots / 10 loans used {cu} CU");

    // repay_loan needs no price accounts but still scans all 10 loan slots to find loan 0.
    // Measured 19,215 CU.
    env.mint_to(&setup.cngn, &setup.borrower_cngn, 100_000 * ONE_CNGN);
    let rp = repay_loan_ix(&owner, &owner, &setup.cngn, &setup.borrower_cngn, 0, u64::MAX);
    let cu = send_cu(&mut env.svm, &[rp], &[&env.admin, &setup.borrower.key]).unwrap();
    assert!(cu < 25_000, "repay_loan at 10 loan slots used {cu} CU");
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
    // slots and 9 existing loans. **Measured 72,533 CU — this is the figure spec §15 points at,
    // and the only place it is written down.**
    //
    // It is a floor for mainnet, not an estimate of it: `MintKind::XStock` initializes a
    // metadata *pointer* but writes no `TokenMetadata` extension, so the fixture mint is
    // smaller than a live Backed xStock, which carries name, symbol and URI. Unpacking the
    // real mint and walking its TLV entries costs more, so a mainnet `take_loan` reads
    // somewhat higher than this.
    let prices = env.price_accounts(&owner);
    let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, prices);
    let cu = send_cu(&mut env.svm, &[ixn], &[&env.admin, &setup.borrower.key]).unwrap();
    assert!(cu < 100_000, "take_loan at 8 xStock slots / 9 existing loans used {cu} CU");

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
