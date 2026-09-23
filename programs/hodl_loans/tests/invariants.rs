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

    // Promo invariants (spec §12). This harness tracks a single borrower per `LoanSetup`, so
    // "sum of all positions' promo_balance" reduces to that one position's balance — the only
    // one any scenario here ever grants promo to.
    let promo_vault = env.promo_vault(&setup.cngn);
    assert!(
        promo_vault.outstanding + promo_vault.unissued <= promo_vault.cash,
        "{label}: promo_vault outstanding {} + unissued {} > cash {}",
        promo_vault.outstanding,
        promo_vault.unissued,
        promo_vault.cash
    );
    assert_eq!(
        position.promo_balance, promo_vault.outstanding,
        "{label}: position.promo_balance {} != promo_vault.outstanding {}",
        position.promo_balance, promo_vault.outstanding
    );
    let promo_vault_balance = env.token_balance(&promo_vault_token_pda(&setup.cngn));
    assert!(
        promo_vault_balance >= promo_vault.cash,
        "{label}: promo vault token balance {promo_vault_balance} < promo_vault.cash {}",
        promo_vault.cash
    );
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

/// The whole default path: a healthy loan, a price crash, a partial liquidation, then a
/// write-off of what is left. The market's accounting invariants hold after every step, and
/// the lender never gets back more than it put in.
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

    // The lender's exit is short by exactly the bad debt: it is the pool's only depositor, so
    // a full withdrawal returns total_assets, which is POOL_CNGN minus what the write-off
    // recorded (the liquidation already recovered 300,000 of the original 700,000 cNGN loan,
    // so only the unrecovered 400,000 cNGN became bad debt). The virtual-share offset that
    // protects the first deposit (1,000 shares / 1 asset) rounds `redeemable_amount` down from
    // total_assets by less than 1 base unit here — total_shares (10,000,000,000,000,000) is far
    // larger than 1,000 × total_assets — so the floor lands on total_assets exactly, with no
    // dust on top of the recorded loss.
    env.withdraw(&setup.lender, &setup.cngn, u64::MAX).unwrap();
    let returned = env.token_balance(&setup.lender.token);
    let shortfall = POOL_CNGN - returned;
    assert_eq!(
        shortfall, market.total_bad_debt as u64,
        "lender shortfall {shortfall} does not match the recorded bad debt {}", market.total_bad_debt
    );

    // With no loans left, the borrower can still withdraw the dust that was never seized.
    let dust = env.position(&owner).collateral[0].amount;
    let token = env.create_token_account(&setup.usdc, &owner);
    let withdraw = withdraw_collateral_ix(&owner, &setup.usdc, &SPL_TOKEN, &token, None, dust, vec![]);
    env.sponsored(withdraw, &setup.borrower.key).unwrap();
    assert_eq!(env.token_balance(&token), dust);
}

/// Same sequence as `a_default_runs_from_liquidation_to_write_off`, but the position holds
/// promo throughout: `Env::loan_ready` never grants any, so that test never exercises a
/// forfeit or the promo invariants. This one does, with all invariants asserted after every
/// step.
#[test]
fn a_default_runs_from_liquidation_to_write_off_with_promo() {
    let (mut env, setup) = Env::promo_ready();
    let owner = setup.borrower.pubkey();
    let grant = 50_000 * ONE_CNGN;
    env.redeem_promo(&setup.borrower, &setup.cngn, 1, grant, 7).unwrap();
    assert_invariants(&env, &setup, "after redeeming promo");

    env.take_loan(&setup.borrower, &setup, 700_000 * ONE_CNGN, 365 * DAY).unwrap();
    assert_invariants(&env, &setup, "after borrowing");

    // USDC at $0.45 puts $437.94 of debt over the liquidation line, even counting the promo.
    env.set_pyth_price(&setup.usdc, 45_000_000, 0);
    let liquidator = env.new_liquidator(&setup.cngn, 10_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 300_000 * ONE_CNGN).unwrap();
    assert_invariants(&env, &setup, "after liquidating");
    // The forfeit fires exactly once, at the first liquidation to touch this loan.
    assert_eq!(env.position(&owner).promo_balance, 0);

    // The rest of the collateral collapses to dust, so the remaining loan is written off.
    env.set_pyth_price(&setup.usdc, 100_000, 0);
    env.write_off(&setup, 0).unwrap();
    assert_invariants(&env, &setup, "after the write-off");
    let market = env.market(&setup.cngn);
    assert_eq!(market.total_borrows, 0);
    assert!(market.total_bad_debt > 0);
    assert!(!env.position(&owner).has_active_loans());

    // Same shortfall check as the non-promo variant, but reconciled against the full `grant`:
    // the forfeit fired at liquidation, not at write-off (per the assertion above), so its cNGN
    // was already in the market vault by the time `total_bad_debt` was booked — the lender's
    // real shortfall comes in `grant` lower than `total_bad_debt` records, because
    // `write_off_loan` books the loan's raw shortfall without knowing an earlier instruction
    // already covered part of it (`write_off_loan`'s own `forfeited` is zero here; Fix 5 only
    // nets a forfeit that fires inside the SAME write-off call). `shortfall + grant` reconstructs
    // `total_bad_debt` exactly: what the lender actually lost, plus what already made up for it.
    env.withdraw(&setup.lender, &setup.cngn, u64::MAX).unwrap();
    let returned = env.token_balance(&setup.lender.token);
    let shortfall = POOL_CNGN - returned;
    assert_eq!(
        shortfall + grant, market.total_bad_debt as u64,
        "lender shortfall {shortfall} plus the forfeited promo {grant} does not match the recorded bad debt {}",
        market.total_bad_debt
    );
}

#[test]
fn two_consecutive_partial_liquidations_converge() {
    // The first of the two probes the Plan 3 follow-ups asked a fuzzer for. It is a property
    // about a *sequence*, which is why a single-step test cannot see it: each partial
    // liquidation seizes collateral and repays debt, and the position's health must move
    // monotonically toward solvency. If the seizure and the repayment disagreed — the bonus
    // taking more value than the repayment retires — a position could be liquidated
    // repeatedly and end further under water each time, which is the shape of a drain.
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    env.take_loan(&setup.borrower, &setup, 700_000 * ONE_CNGN, 365 * DAY).unwrap();
    env.set_pyth_price(&setup.usdc, 45_000_000, 0);
    assert_invariants(&env, &setup, "before any liquidation");

    let liquidator = env.new_liquidator(&setup.cngn, 1_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    let debt_and_collateral = |env: &Env| {
        let p = env.position(&owner);
        (p.loans[0].principal, p.collateral.iter().find(|s| s.mint == setup.usdc).unwrap().amount)
    };
    let (debt0, coll0) = debt_and_collateral(&env);

    // Two partial liquidations, each repaying a tenth of the principal.
    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 70_000 * ONE_CNGN).unwrap();
    assert_invariants(&env, &setup, "after the first partial liquidation");
    let (debt1, coll1) = debt_and_collateral(&env);

    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 70_000 * ONE_CNGN).unwrap();
    assert_invariants(&env, &setup, "after the second partial liquidation");
    let (debt2, coll2) = debt_and_collateral(&env);

    // Both quantities fall, every time — no oscillation, no growth.
    assert!(debt1 < debt0 && debt2 < debt1, "principal must fall with each liquidation");
    assert!(coll1 < coll0 && coll2 < coll1, "collateral must fall with each liquidation");

    // The property worth pinning is not that the two steps match each other — at a fixed
    // price they are symmetric by construction, so asserting that catches almost nothing.
    // It is that **each step leaves the position better collateralised than it found it**:
    // the value seized must not exceed the value of the debt retired by more than the bonus
    // the asset is configured to pay. A liquidation that took more than that would let a
    // liquidator walk a healthy-ish position down to nothing one call at a time.
    //
    // Both sides converted to micro-dollars, which is the trap here: the crashed Pyth price
    // is at exponent -8 (45_000_000 == $0.45) while `NGN_USD` is at Switchboard's 18 decimals
    // (625_000_000_000_000 == $0.000625). Mixing the two silently inflates one side by ten
    // orders of magnitude and makes any ceiling vacuous — which is exactly what a first
    // version of this assertion did.
    const MICRO: u128 = 1_000_000;
    let seized_usd = |atoms: u64| atoms as u128 * 45_000_000 * MICRO / 100_000_000 / ONE_USDC as u128;
    let retired_usd =
        |atoms: u64| atoms as u128 * NGN_USD as u128 * MICRO / 1_000_000_000_000_000_000 / ONE_CNGN as u128;
    for (label, seized, retired) in
        [("first", coll0 - coll1, debt0 - debt1), ("second", coll1 - coll2, debt1 - debt2)]
    {
        let taken = seized_usd(seized);
        let given = retired_usd(retired);
        // 5% is `default_collateral_params`' `liquidation_bonus_bps`. One atom of slack for
        // the per-step flooring in `principal_share` and `seize_for_repayment`.
        let ceiling = given * 10_500 / 10_000 + 1;
        assert!(
            taken <= ceiling,
            "{label} step seized {taken} USD against {given} retired — past the 5% bonus"
        );
        assert!(taken > 0 && given > 0, "{label} step moved nothing");
    }
}

#[test]
fn a_lender_cannot_sandwich_a_liquidation_for_the_penalty_step() {
    // The second probe. Spec §11's accepted-risk note records that overdue penalty interest
    // reaches lenders as a *step* at repayment or liquidation rather than continuously, and
    // that `liquidate` has no access check — so a lender could deposit immediately before
    // someone else's liquidation, collect a share of the step, and withdraw, diluting the
    // lenders who actually carried the loan.
    //
    // This pins the accounting around that sequence. It does not prevent the sandwich — the
    // fix is continuous penalty accrual, which is its own plan — but it establishes that the
    // sandwich cannot extract *more* than the step it is capturing, and that every invariant
    // survives the sequence. A regression that let the sandwicher withdraw more than they put
    // in plus their share would fail here.
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, 700_000 * ONE_CNGN, 30 * DAY).unwrap();

    // Go well past maturity so the penalty term is substantial, then crash the price.
    env.warp_seconds(400 * DAY);
    env.set_pyth_price(&setup.usdc, 45_000_000, 0);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    assert_invariants(&env, &setup, "overdue, before the sandwich");

    // The sandwicher deposits just before the liquidation.
    let sandwicher = env.new_lender(&setup.cngn, 1_000_000 * ONE_CNGN);
    let deposited = 1_000_000 * ONE_CNGN;
    env.deposit(&sandwicher, &setup.cngn, deposited).unwrap();
    assert_invariants(&env, &setup, "after the sandwicher deposits");

    let liquidator = env.new_liquidator(&setup.cngn, 1_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 100_000 * ONE_CNGN).unwrap();
    assert_invariants(&env, &setup, "after the liquidation releases the penalty step");

    // Withdraw everything the sandwicher can.
    let before = env.token_balance(&sandwicher.token);
    env.withdraw(&sandwicher, &setup.cngn, u64::MAX).unwrap();
    let gained = env.token_balance(&sandwicher.token) - before;
    assert_invariants(&env, &setup, "after the sandwicher withdraws");

    // They get back what they put in, plus at most their pro-rata share of what the
    // liquidation released — never more. The point of the assertion is the upper bound: a
    // change that let a same-slot deposit claim more than its share of the step would break
    // it, and that is the failure mode the accepted-risk note is about.
    assert!(gained >= deposited, "a lender must never lose principal to someone else's liquidation");
    let share = gained - deposited;
    assert!(
        share < deposited / 100,
        "a same-slot sandwich took {share} atoms on {deposited} deposited — more than a \
         pro-rata share of one liquidation's penalty step"
    );
}
