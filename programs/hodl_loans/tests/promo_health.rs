mod common;

use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

const DAY: i64 = 86_400;
/// 1,000 USDC at 70% LTV backs $700, which is 1,118,881 cNGN at the NGN ask.
const OWN_CEILING: u64 = 1_118_881 * ONE_CNGN;
/// The same position holding 50,000 cNGN of promo: $700 + $31.21875 of counted promo.
const WITH_PROMO_CEILING: u64 = 1_168_781 * ONE_CNGN;
/// Promo worth $312.19 but capped at 20% of $1,000 of collateral, so $900 in all.
const CAPPED_CEILING: u64 = 1_438_561 * ONE_CNGN;

#[test]
fn promo_lifts_the_borrow_limit_by_what_it_is_worth() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;

    // Before any promo, the collateral alone sets the ceiling.
    assert_hodl_error(env.take_loan(borrower, &setup, OWN_CEILING + ONE_CNGN, 30 * DAY), HodlError::Unhealthy);

    env.redeem_promo(borrower, &setup.cngn, 1, 50_000 * ONE_CNGN, 7).unwrap();

    // The same loan the collateral could not carry now fits, and the new ceiling is exact.
    assert_hodl_error(env.take_loan(borrower, &setup, WITH_PROMO_CEILING + ONE_CNGN, 30 * DAY), HodlError::Unhealthy);
    env.take_loan(borrower, &setup, WITH_PROMO_CEILING, 30 * DAY).unwrap();
    assert_eq!(env.token_balance(&setup.borrower_cngn), WITH_PROMO_CEILING);
}

#[test]
fn promo_counts_only_up_to_a_fifth_of_the_borrowers_own_collateral() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    env.set_max_promo_per_position(&setup.cngn, 1_000_000 * ONE_CNGN);

    // 500,000 cNGN of promo is worth $312.19, but only $200 of it can count against $1,000
    // of collateral at the 20% cap — so the ceiling stops at $900, not $1,012.19.
    env.redeem_promo(borrower, &setup.cngn, 1, 500_000 * ONE_CNGN, 7).unwrap();
    assert_eq!(env.position(&borrower.pubkey()).promo_balance, 500_000 * ONE_CNGN);

    assert_hodl_error(env.take_loan(borrower, &setup, CAPPED_CEILING + ONE_CNGN, 30 * DAY), HodlError::Unhealthy);
    env.take_loan(borrower, &setup, CAPPED_CEILING, 30 * DAY).unwrap();
}

#[test]
fn promo_is_worth_nothing_to_a_position_holding_no_collateral() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = env.new_borrower();
    let borrower_cngn = env.create_token_account(&setup.cngn, &borrower.pubkey());
    let setup = LoanSetup { borrower, borrower_cngn, ..setup };

    env.redeem_promo(&setup.borrower, &setup.cngn, 1, 50_000 * ONE_CNGN, 7).unwrap();
    assert_eq!(env.position(&setup.borrower.pubkey()).promo_balance, 50_000 * ONE_CNGN);

    // The cap is a fraction of the borrower's own collateral, and a fraction of nothing is
    // nothing — this is what stops promo being borrowed against on its own.
    let result = env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 30 * DAY);
    assert_hodl_error(result, HodlError::Unhealthy);
}

#[test]
fn promo_lifts_the_liquidation_line_with_the_borrow_limit() {
    // Two identical positions and one price crash: the promo decides which is liquidatable.
    let (mut env, setup) = Env::promo_ready();
    let plain = &setup.borrower;
    env.take_loan(plain, &setup, 700_000 * ONE_CNGN, 365 * DAY).unwrap();

    let promoed = env.new_borrower();
    let owner = promoed.pubkey();
    env.deposit_collateral(&promoed, &setup.usdc, 1_000 * ONE_USDC);
    let promoed_cngn = env.create_token_account(&setup.cngn, &owner);
    env.redeem_promo(&promoed, &setup.cngn, 1, 50_000 * ONE_CNGN, 7).unwrap();
    let prices = env.price_accounts(&owner);
    let borrow = take_loan_ix(&owner, &setup.cngn, &promoed_cngn, 700_000 * ONE_CNGN, 365 * DAY, prices);
    send(&mut env.svm, &[borrow], &[&env.admin, &promoed.key]).unwrap();

    // $0.4525 leaves the plain position under its 90% line ($407.25 against $437.94 of debt)
    // and the promoed one just over it ($438.47), on $31.21875 of counted promo.
    env.set_pyth_price(&setup.usdc, 45_250_000, 0);
    let liquidator = env.new_liquidator(&setup.cngn, 1_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    let prices = env.price_accounts(&owner);
    let seize = liquidate_ix(
        &liquidator.pubkey(),
        &owner,
        &setup.cngn,
        &liquidator.cngn,
        &setup.usdc,
        &SPL_TOKEN,
        &seized_to,
        0,
        1_000 * ONE_CNGN,
        prices,
    );
    let result = send(&mut env.svm, &[seize], &[&liquidator.key]);
    assert_hodl_error(result, HodlError::NotLiquidatable);

    // The only difference between the two positions is the promo.
    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 1_000 * ONE_CNGN).unwrap();
}

#[test]
fn a_borrow_paused_asset_unlocks_no_promo_either() {
    // The brake has to stop *both* routes from a holding to the borrow limit. Its own LTV
    // term is the obvious one; the promo cap is the other, and it is a fraction of the
    // holding's value that never consulted `ltv_bps`. A test on a promo-free position cannot
    // tell the two apart — this one can.
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    env.redeem_promo(borrower, &setup.cngn, 1, 50_000 * ONE_CNGN, 7).unwrap();
    env.take_loan(borrower, &setup, WITH_PROMO_CEILING, 30 * DAY).unwrap();

    // Repay in full so the position is idle, then pause borrowing against the collateral.
    env.mint_to(&setup.cngn, &setup.borrower_cngn, WITH_PROMO_CEILING);
    env.repay(&setup, 0, u64::MAX).unwrap();
    let pause = set_collateral_borrow_paused_ix(&env.guardian.pubkey(), &setup.usdc, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();
    assert_eq!(env.position(&setup.borrower.pubkey()).promo_balance, 50_000 * ONE_CNGN);

    // Promo is still held and the collateral is still there, but neither may be borrowed
    // against: the market's smallest permitted loan is refused. Before the promo cap was
    // gated on the same flag, this call succeeded for up to 20% of the collateral's value.
    assert_hodl_error(
        env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 30 * DAY),
        HodlError::Unhealthy,
    );

    // Lifting it restores exactly the promo-assisted ceiling, so nothing else moved.
    let unpause = set_collateral_borrow_paused_ix(&env.admin.pubkey(), &setup.usdc, false);
    send(&mut env.svm, &[unpause], &[&env.admin]).unwrap();
    env.take_loan(&setup.borrower, &setup, WITH_PROMO_CEILING, 30 * DAY).unwrap();
}
