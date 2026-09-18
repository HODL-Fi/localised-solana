mod common;

use common::*;
use solana_signer::Signer;

const DAY: i64 = 86_400;

/// Recomputes the market's accounting invariants from the position and vault state and asserts
/// each one holds. Called after every state-changing step in `multi_loan_accounting_holds`.
fn assert_invariants(env: &Env, setup: &LoanSetup, label: &str) {
    let market = env.market(&setup.cngn);
    let position = env.position(&setup.borrower.pubkey());

    let sum_principal: u64 = position.loans.iter().filter(|l| l.is_active()).map(|l| l.principal).sum();
    assert_eq!(market.total_borrows, sum_principal, "{label}: total_borrows != sum of active principals");

    // Market::lp_rate_product is the running sum of each active loan's principal × rate_bps ×
    // (BPS − reserve_factor_bps); see `math::loan::lp_contribution`.
    let sum_contrib: u128 = position
        .loans
        .iter()
        .filter(|l| l.is_active())
        .map(|l| l.principal as u128 * l.rate_bps as u128 * (10_000 - l.reserve_factor_bps as u128))
        .sum();
    assert_eq!(market.lp_rate_product, sum_contrib, "{label}: lp_rate_product != sum of per-loan contributions");

    assert!(market.cash >= market.protocol_reserve, "{label}: cash {} < protocol_reserve {}", market.cash, market.protocol_reserve);

    let vault_balance = env.token_balance(&market_vault_pda(&setup.cngn));
    assert!(vault_balance >= market.cash, "{label}: vault balance {vault_balance} < market.cash {}", market.cash);
}

#[test]
fn multi_loan_accounting_holds() {
    let (mut env, setup) = Env::loan_ready();
    env.deposit_collateral(&setup.borrower, &setup.usdc, 10_000 * ONE_USDC);
    env.take_loan(&setup.borrower, &setup, 1_000_000 * ONE_CNGN, 60 * DAY).unwrap();
    env.warp_seconds(17 * DAY + 3);
    env.set_pyth_price(&setup.usdc, ONE_DOLLAR, 0);
    let params = hodl_loans::MarketParams { interest_rate_bps: 3_000, reserve_factor_bps: 2_500, penalty_rate_bps: 1_000, ..default_market_params() };
    send(&mut env.svm, &[update_market_params_ix(&env.admin.pubkey(), &setup.cngn, params)], &[&env.admin]).unwrap();
    // The second loan takes new market terms (30% rate, 25% reserve, 10% penalty); the first
    // loan keeps the terms it originated under.
    env.take_loan(&setup.borrower, &setup, 777_777 * ONE_CNGN + 1, 30 * DAY).unwrap();
    assert_invariants(&env, &setup, "after two loans");

    env.mint_to(&setup.cngn, &setup.borrower_cngn, 5_000_000 * ONE_CNGN);
    env.warp_seconds(11 * DAY + 17);
    // Partial repayment before maturity: pays interest first, then reduces principal.
    env.repay(&setup, 1, 100_000 * ONE_CNGN).unwrap();
    assert_invariants(&env, &setup, "partial repay loan 1 (before maturity)");

    env.warp_seconds(50 * DAY + 5); // both loans are now overdue
    // Partial repayment after maturity: interest-first math now includes penalty interest too.
    env.repay(&setup, 0, 400_000 * ONE_CNGN + 13).unwrap();
    assert_invariants(&env, &setup, "overdue partial repay loan 0");
    env.repay(&setup, 1, 150_000 * ONE_CNGN).unwrap();
    assert_invariants(&env, &setup, "overdue partial repay loan 1");

    // 50 small repayments in a row exercise the interest-anchor reset and rounding-remainder
    // bookkeeping under repeated tiny partial repayments.
    for _ in 0..50 {
        env.warp_seconds(3);
        env.repay(&setup, 1, 20 * ONE_CNGN).unwrap();
    }
    assert_invariants(&env, &setup, "50 small repays loan 1");

    // A second lender deposits then immediately withdraws everything: the round trip must never
    // return more than was deposited (share-price rounding always favors the pool).
    let second_lender = env.new_lender(&setup.cngn, 1_000_000 * ONE_CNGN);
    env.deposit(&second_lender, &setup.cngn, 1_000_000 * ONE_CNGN).unwrap();
    env.withdraw(&second_lender, &setup.cngn, u64::MAX).unwrap();
    let returned = env.token_balance(&second_lender.token);
    assert!(returned <= 1_000_000 * ONE_CNGN, "lender round trip returned {returned}, more than the 1,000,000 cNGN deposited");
    assert_invariants(&env, &setup, "after second lender round trip");

    env.warp_seconds(9 * DAY);
    env.repay(&setup, 0, u64::MAX).unwrap();
    env.repay(&setup, 1, u64::MAX).unwrap();
    assert_invariants(&env, &setup, "all repaid");

    let market = env.market(&setup.cngn);
    assert!(!env.position(&setup.borrower.pubkey()).has_active_loans(), "all repaid: a loan slot is still active");
    // Residual accrued_interest is dust left by per-second rounding across the whole scenario,
    // not unpaid interest — everything owed was paid off above.
    assert!(market.accrued_interest <= 60, "residual accrued_interest {} is more than rounding dust", market.accrued_interest);
}

#[test]
fn a_default_runs_from_liquidation_to_write_off() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    env.take_loan(&setup.borrower, &setup, 700_000 * ONE_CNGN, 365 * DAY).unwrap();
    assert_invariants(&env, &setup, "after borrowing");

    // USDC at $0.45 puts $437.94 of debt over the $405 liquidation line.
    env.set_pyth_price(&setup.usdc, 45_000_000, 0);
    let liquidator = env.new_liquidator(&setup.cngn, 10_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 300_000 * ONE_CNGN).unwrap();
    assert_invariants(&env, &setup, "after liquidating");
    let collateral = env.collateral(&setup.usdc);
    assert_eq!(env.token_balance(&collateral_vault_pda(&setup.usdc)), collateral.total_deposited);
    assert_eq!(
        env.position(&owner).collateral[0].amount + env.token_balance(&seized_to),
        1_000 * ONE_USDC
    );

    // The rest of the collateral collapses to dust, so the remaining loan is written off.
    env.set_pyth_price(&setup.usdc, 100_000, 0);
    env.write_off(&setup, 0).unwrap();
    assert_invariants(&env, &setup, "after the write-off");
    let market = env.market(&setup.cngn);
    assert_eq!(market.total_borrows, 0);
    assert!(market.total_bad_debt > 0);
    assert!(!env.position(&owner).has_active_loans());

    // The lender's exit is short by the bad debt, never above its deposit.
    env.withdraw(&setup.lender, &setup.cngn, u64::MAX).unwrap();
    let returned = env.token_balance(&setup.lender.token);
    assert!(returned < POOL_CNGN, "lender got back {returned} of {POOL_CNGN} despite bad debt");

    // With no loans left, the borrower can still withdraw the dust that was never seized.
    let dust = env.position(&owner).collateral[0].amount;
    let token = env.create_token_account(&setup.usdc, &owner);
    let withdraw = withdraw_collateral_ix(&owner, &setup.usdc, &SPL_TOKEN, &token, None, dust, vec![]);
    env.sponsored(withdraw, &setup.borrower.key).unwrap();
    assert_eq!(env.token_balance(&token), dust);
}
