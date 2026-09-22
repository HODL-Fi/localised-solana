mod common;

use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

const DAY: i64 = 86_400;
/// The borrower's loan: 700,000 cNGN, worth $437.94 at the NGN ask.
const LOAN: u64 = 700_000 * ONE_CNGN;
/// $0.45 at Pyth exponent -8: 1,000 USDC then covers $450, under the $437.94 debt × 90% line.
const USDC_CRASHED: i64 = 45_000_000;

/// A position holding 1,000 USDC that owes `LOAN` and has just gone under water.
fn underwater() -> (Env, LoanSetup) {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    env.set_pyth_price(&setup.usdc, USDC_CRASHED, 0);
    (env, setup)
}

#[test]
fn healthy_positions_cannot_be_liquidated() {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    let liquidator = env.new_liquidator(&setup.cngn, LOAN);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    let result = env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, ONE_CNGN);
    assert_hodl_error(result, HodlError::NotLiquidatable);

    // Being overdue is not enough on its own (spec §2): only an unhealthy position is liquidatable.
    env.warp_seconds(400 * DAY);
    env.set_pyth_price(&setup.usdc, ONE_DOLLAR, 0);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    let result = env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, ONE_CNGN);
    assert_hodl_error(result, HodlError::NotLiquidatable);
}

#[test]
fn liquidation_seizes_collateral_plus_the_bonus() {
    let (mut env, setup) = underwater();
    let liquidator = env.new_liquidator(&setup.cngn, LOAN);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    let cash_before = env.market(&setup.cngn).cash;

    // 100,000 cNGN is $62.50; with the 5% bonus that seizes $65.625 of USDC at $0.45.
    let repaid = 100_000 * ONE_CNGN;
    env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, repaid).unwrap();
    let seized = 145_833_333;

    assert_eq!(env.token_balance(&collateral_account), seized);
    assert_eq!(env.token_balance(&liquidator.cngn), LOAN - repaid);
    assert_eq!(env.token_balance(&collateral_vault_pda(&setup.usdc)), 1_000 * ONE_USDC - seized);
    assert_eq!(env.collateral(&setup.usdc).total_deposited, 1_000 * ONE_USDC - seized);

    let position = env.position(&setup.borrower.pubkey());
    assert_eq!(position.collateral[0].amount, 1_000 * ONE_USDC - seized);
    let loan = position.loans[0];
    // No time has passed, so the whole repayment is principal and the terms are untouched.
    assert_eq!((loan.principal, loan.repaid), (LOAN - repaid, repaid));
    assert_eq!(loan.interest_anchor, loan.originated_at);

    let market = env.market(&setup.cngn);
    assert_eq!(market.total_borrows, LOAN - repaid);
    assert_eq!(market.cash, cash_before + repaid);
    assert_eq!(market.protocol_reserve, 0);
    assert_eq!(market.lp_rate_product, (LOAN - repaid) as u128 * 1_500 * 9_000);
}

#[test]
fn liquidating_the_second_collateral_slot_uses_its_own_price() {
    let (mut env, setup) = underwater();
    // A second, 9-decimal mint fills slot 1 (slot 0 already holds the USDC from `loan_ready`).
    // `load_valuation` values used slots in position order via the `priced` count in
    // `liquidate.rs`; this exercises that index for a slot other than 0 — a mismatch there
    // would price the seizure off USDC's $0.45 instead of this mint's $10.
    let sol = env.list_spl_collateral(9);
    env.set_pyth_price(&sol, 10 * ONE_DOLLAR, 0);
    let sol_deposit = 2_000_000_000; // 2 tokens, 9 decimals
    env.deposit_collateral(&setup.borrower, &sol, sol_deposit);
    let liquidator = env.new_liquidator(&setup.cngn, LOAN);
    let collateral_account = env.create_token_account(&sol, &liquidator.pubkey());

    // No time has passed since origination, so the whole repayment is principal and the
    // balance cap does not bind. 10,000 cNGN is $6.25 at the plain NGN price (625,000,000 of
    // USD_SCALE, not the ask): floor(10,000,000,000 × 625,000,000 / 1e6) = 6,250,000,000,000.
    // With the 5% bonus that's floor(6,250,000,000,000 × 10,500 / 10,000) = 6,562,500,000,000
    // ($6.5625), seized from the 9-decimal slot-1 mint at $10 (10,000,000,000,000 of
    // USD_SCALE): floor(6,562,500,000,000 × 1e9 / 1e13) = 656,250,000 — well under the
    // 2,000,000,000 the slot holds, so the seizure is not slot-capped either.
    let repaid = 10_000 * ONE_CNGN;
    env.liquidate(&liquidator, &setup, &sol, &collateral_account, 0, repaid).unwrap();
    let seized = 656_250_000;

    assert_eq!(env.token_balance(&collateral_account), seized);
    assert_eq!(env.token_balance(&liquidator.cngn), LOAN - repaid);
    assert_eq!(env.collateral(&sol).total_deposited, sol_deposit - seized);

    let position = env.position(&setup.borrower.pubkey());
    // Slot 0 (USDC) is untouched: pricing and seizing slot 1 must not reach into slot 0.
    assert_eq!(position.collateral[0].amount, 1_000 * ONE_USDC);
    assert_eq!(position.collateral[1].amount, sol_deposit - seized);
}

#[test]
fn the_collateral_slot_caps_the_repayment() {
    let (mut env, setup) = underwater();
    let liquidator = env.new_liquidator(&setup.cngn, 10_000_000 * ONE_CNGN);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    // Repaying all 700,000 cNGN would seize 1,020.83 USDC, but the slot holds 1,000.
    env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, u64::MAX).unwrap();

    assert_eq!(env.token_balance(&collateral_account), 1_000 * ONE_USDC);
    assert_eq!(env.collateral(&setup.usdc).total_deposited, 0);
    let position = env.position(&setup.borrower.pubkey());
    assert!(!position.has_collateral());
    // 700,000 × 1,000 / 1,020.833333 cNGN, rounded down.
    let paid = 685_714_285_938;
    assert_eq!(position.loans[0].principal, LOAN - paid);
    assert_eq!(env.market(&setup.cngn).total_borrows, LOAN - paid);
}

#[test]
fn liquidation_splits_a_repayment_pro_rata_and_keeps_the_penalty_clock_running() {
    let (mut env, setup) = Env::loan_ready();
    // A 73-day loan left to run 146 days: 3,000 cNGN of interest, then 4,120 of penalty,
    // so the balance is 107,120 cNGN ($67.02). USDC at $0.05 puts the line at $45.
    env.take_loan(&setup.borrower, &setup, 100_000 * ONE_CNGN, 73 * DAY).unwrap();
    env.warp_seconds(146 * DAY);
    env.set_pyth_price(&setup.usdc, 5_000_000, 0);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    let liquidator = env.new_liquidator(&setup.cngn, 10_000_000 * ONE_CNGN);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    let anchor = env.position(&setup.borrower.pubkey()).loans[0].interest_anchor;
    // No market-touching instruction has run since `take_loan`, so nothing has accrued yet.
    let accrued_before = env.market(&setup.cngn).accrued_interest;

    // Spec §11 step 7 splits a liquidation pro rata, not interest-first as a repayment does:
    // 8,120 × 100,000 / 107,120 of it is principal.
    let repaid = 8_120 * ONE_CNGN;
    env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, repaid).unwrap();
    let principal_repaid = 7_580_283_793;
    let interest_paid = repaid - principal_repaid;

    let loan = env.position(&setup.borrower.pubkey()).loans[0];
    assert_eq!(loan.principal, 100_000 * ONE_CNGN - principal_repaid);
    assert_eq!(loan.repaid, repaid);
    // Spec §11 step 10: the anchor does not move, so the penalty keeps accruing from maturity.
    assert_eq!(loan.interest_anchor, anchor);

    let market = env.market(&setup.cngn);
    // The reserve takes 10% of the interest share only.
    assert_eq!(market.protocol_reserve, interest_paid / 10);
    assert_eq!(market.total_borrows, 100_000 * ONE_CNGN - principal_repaid);
    assert_eq!(market.cash, POOL_CNGN - 100_000 * ONE_CNGN + repaid);
    // $5.075 of cNGN plus the 5% bonus, at $0.05 a USDC.
    assert_eq!(env.token_balance(&collateral_account), 106_575_000);

    // Spec §11 step 8: `market.accrue` runs first, on the untouched 100,000 cNGN principal —
    // 90% of 15% APR over 146/365 of a year (exactly 0.4) is 100,000 × 0.15 × 0.9 × 0.4 =
    // 5,400 cNGN, i.e. 5,400,000,000 base units. Liquidation then releases only the repaid
    // share via R(principal_repaid, loan):
    // released = floor(principal_repaid × rate_bps × (BPS − reserve_factor_bps) × 146 days
    //                   / (BPS × BPS × YEAR))
    //          = floor(7,580,283,793 × 1,500 × 9,000 × 12,614,400 / (10,000 × 10,000 × 31,536,000))
    // rate_bps × (BPS − reserve_factor_bps) / (BPS × BPS) × (146 days / YEAR) reduces exactly to
    // 54 / 1,000 (1,500 × 9,000 / 10,000 = 1,350; 1,350 / 10,000 × 12,614,400 / 31,536,000, and
    // 12,614,400 / 31,536,000 = 146/365 = 0.4 exactly, so 1,350 × 0.4 / 10,000 = 540/10,000 = 0.054):
    //          = floor(7,580,283,793 × 54 / 1,000) = floor(409,335,324,822 / 1,000) = 409,335,324.
    let released = 409_335_324;
    assert_eq!(market.accrued_interest, accrued_before + 5_400_000_000 - released);
}

#[test]
fn a_fully_repaid_loan_clears_its_slot() {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, 100_000 * ONE_CNGN, 365 * DAY).unwrap();
    env.take_loan(&setup.borrower, &setup, 600_000 * ONE_CNGN, 365 * DAY).unwrap();
    env.set_pyth_price(&setup.usdc, USDC_CRASHED, 0);
    let liquidator = env.new_liquidator(&setup.cngn, 10_000_000 * ONE_CNGN);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, u64::MAX).unwrap();

    let position = env.position(&setup.borrower.pubkey());
    assert_eq!(position.loans[0], bytemuck::Zeroable::zeroed());
    assert_eq!(position.loans[1].principal, 600_000 * ONE_CNGN);
    assert_eq!(env.market(&setup.cngn).total_borrows, 600_000 * ONE_CNGN);
    // 100,000 cNGN is $62.50; the 5% bonus seizes $65.625 at $0.45.
    assert_eq!(env.token_balance(&collateral_account), 145_833_333);
}

#[test]
fn two_consecutive_partial_liquidations_converge_to_healthy() {
    let (mut env, setup) = Env::loan_ready();
    // 1,200,000 cNGN is $750.75 of debt (1,200,000 × 625,625,000 of USD_SCALE, the NGN ask).
    // At the 70% LTV limit that needs collateral worth more than $1,072.50, so price USDC at
    // $1.20 ($1,200 of collateral, an $840 limit) to clear take_loan's health check.
    env.set_pyth_price(&setup.usdc, 120_000_000, 0);
    let loan = 1_200_000 * ONE_CNGN;
    env.take_loan(&setup.borrower, &setup, loan, 365 * DAY).unwrap();

    // Crash to $0.80: 1,000 USDC is now $800 of collateral, a $720 liquidation line (90%
    // threshold) under the $750.75 debt — a $30.75 gap, and the position is liquidatable.
    env.set_pyth_price(&setup.usdc, 80_000_000, 0);
    let liquidator = env.new_liquidator(&setup.cngn, loan);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    // Each whole cNGN repaid removes 625,625,000 (the NGN ask) from debt, and removes
    // 625,000,000 × 1.05 × 90% = 590,625,000 (the seized dollar value, marked up by the 5%
    // bonus, times the 90% threshold) from the liquidation line — a net closing rate of
    // 35,000,000 of USD_SCALE ($0.035) per cNGN. 400,000 cNGN closes $14 of the $30.75 gap,
    // leaving $16.75 open.
    let repaid1 = 400_000 * ONE_CNGN;
    env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, repaid1).unwrap();
    // 400,000 cNGN is $250 at the plain NGN price; with the 5% bonus, $262.5; at $0.80/USDC
    // that seizes 328.125 USDC.
    let seized1 = 328_125_000;
    assert_eq!(env.token_balance(&collateral_account), seized1);
    let after_first = env.position(&setup.borrower.pubkey());
    assert_eq!(after_first.loans[0].principal, loan - repaid1);
    assert_eq!(after_first.collateral[0].amount, 1_000 * ONE_USDC - seized1);

    // Debt is now 800,000 × 625,625,000 = $500.50; the line is (1,000 − 328.125) USDC × $0.80
    // × 90% = $483.75 — a $16.75 gap still open, so the position is still liquidatable and a
    // second liquidation is accepted.
    let repaid2 = 500_000 * ONE_CNGN;
    env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, repaid2).unwrap();
    // 500,000 cNGN is $312.5 at the plain NGN price; with the bonus, $328.125; at $0.80/USDC
    // that seizes 410.15625 USDC.
    let seized2 = 410_156_250;
    assert_eq!(env.token_balance(&collateral_account), seized1 + seized2);
    let after_second = env.position(&setup.borrower.pubkey());
    assert_eq!(after_second.loans[0].principal, loan - repaid1 - repaid2);

    // Health has crossed over: debt is now 300,000 × 625,625,000 = $187.6875; the line is
    // (671.875 − 410.15625) USDC × $0.80 × 90% = $188.4375 — debt is under the line, so a
    // third liquidation attempt is rejected as not liquidatable, however small the request.
    let third = env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, ONE_CNGN);
    assert_hodl_error(third, HodlError::NotLiquidatable);
}

#[test]
fn liquidation_rejections() {
    let (mut env, setup) = underwater();
    let usdt = env.list_spl_collateral(6);
    env.set_pyth_price(&usdt, ONE_DOLLAR, 0);
    let liquidator = env.new_liquidator(&setup.cngn, LOAN);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    let usdt_account = env.create_token_account(&usdt, &liquidator.pubkey());

    let zero = env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, 0);
    assert_hodl_error(zero, HodlError::AmountTooSmall);
    let unknown = env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 7, ONE_CNGN);
    assert_hodl_error(unknown, HodlError::LoanNotFound);
    // The position holds no USDT.
    let wrong_asset = env.liquidate(&liquidator, &setup, &usdt, &usdt_account, 0, ONE_CNGN);
    assert_hodl_error(wrong_asset, HodlError::InsufficientCollateral);

    // A repayment too small to seize a whole base unit of collateral.
    let dust = env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, 1);
    assert_hodl_error(dust, HodlError::AmountTooSmall);

    // Prices must be fresh and complete.
    let owner = setup.borrower.pubkey();
    let program = env.mint_program(&setup.usdc);
    let no_prices = liquidate_ix(
        &liquidator.pubkey(), &owner, &setup.cngn, &liquidator.cngn, &setup.usdc, &program,
        &collateral_account, 0, ONE_CNGN, vec![],
    );
    assert_hodl_error(send(&mut env.svm, &[no_prices], &[&liquidator.key]), HodlError::PriceAccountMismatch);
    env.warp_seconds(61);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    let stale = env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, ONE_CNGN);
    assert_hodl_error(stale, HodlError::StalePrice);
}

#[test]
fn liquidation_checks_the_borrowed_market() {
    let (mut env, setup) = underwater();
    let other = env.create_mint(MintKind::CngnLike, 6);
    // Deliberately no promo vault on `other`: the market-mismatch check must fire from the
    // handler body, not depend on `other` having every optional account populated.
    let create = create_market_ix(&env.admin.pubkey(), &other, &TOKEN_2022, default_market_params());
    send(&mut env.svm, &[create], &[&env.admin]).unwrap();
    let liquidator = env.new_liquidator(&other, LOAN);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    let owner = setup.borrower.pubkey();
    let prices = env.price_accounts(&owner);
    let program = env.mint_program(&setup.usdc);
    let instruction = liquidate_ix_no_promo(
        &liquidator.pubkey(), &owner, &other, &liquidator.cngn, &setup.usdc, &program,
        &collateral_account, 0, ONE_CNGN, prices,
    );
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&liquidator.key]), HodlError::MarketMismatch);
}

#[test]
fn liquidation_and_write_off_both_succeed_on_a_market_with_no_promo_vault() {
    // `create_promo_vault` is a separate admin action (`vault.rs`): a market that never got
    // one must still support both liquidation and write-off. Before the promo accounts became
    // `Option`, Anchor failed to deserialize the uninitialized promo-vault PDA before the
    // handler ever ran, making both instructions permanently impossible on such a market.
    let (mut env, setup) = Env::loan_ready_no_promo_vault();
    let owner = setup.borrower.pubkey();

    let prices = env.price_accounts(&owner);
    let borrow = take_loan_ix_no_promo_vault(&owner, &setup.cngn, &setup.borrower_cngn, LOAN, 365 * DAY, prices);
    send(&mut env.svm, &[borrow], &[&env.admin, &setup.borrower.key]).unwrap();
    env.set_pyth_price(&setup.usdc, USDC_CRASHED, 0);

    let liquidator = env.new_liquidator(&setup.cngn, LOAN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    let program = env.mint_program(&setup.usdc);
    let prices = env.price_accounts(&owner);
    let liquidate = liquidate_ix_no_promo(
        &liquidator.pubkey(), &owner, &setup.cngn, &liquidator.cngn, &setup.usdc, &program,
        &seized_to, 0, 100_000 * ONE_CNGN, prices,
    );
    send(&mut env.svm, &[liquidate], &[&liquidator.key]).expect("liquidate must succeed with no promo vault on the market");
    assert!(env.position(&owner).loans[0].principal > 0, "a partial liquidation must leave debt remaining");

    // Crash the price further so the remaining collateral is dust, then write off the rest.
    env.set_pyth_price(&setup.usdc, 100_000, 0);
    let prices = env.price_accounts(&owner);
    let write_off = write_off_loan_ix_no_promo(&env.admin.pubkey(), &owner, &setup.cngn, 0, prices);
    send(&mut env.svm, &[write_off], &[&env.admin]).expect("write_off must succeed with no promo vault on the market");
    assert!(env.market(&setup.cngn).total_bad_debt > 0);
}

#[test]
fn a_liquidation_must_name_the_collateral_assets_own_vault() {
    // The constraint that pins `collateral_vault` used to report `PriceAccountMismatch`, which
    // reads as an oracle problem to whoever is debugging a bot. Substituting another listed
    // asset's vault is a vault substitution and now says so.
    let (mut env, setup) = underwater();
    let liquidator = env.new_liquidator(&setup.cngn, LOAN);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    let owner = setup.borrower.pubkey();
    let program = env.mint_program(&setup.usdc);

    // A second listed asset, so the substituted account is a real, initialized collateral vault
    // rather than something Anchor would reject before the constraint runs.
    let other = env.list_spl_collateral(6);
    let real_vault = collateral_vault_pda(&setup.usdc);
    let other_vault = collateral_vault_pda(&other);

    let prices = env.price_accounts(&owner);
    let mut swapped = liquidate_ix(
        &liquidator.pubkey(), &owner, &setup.cngn, &liquidator.cngn, &setup.usdc, &program,
        &collateral_account, 0, ONE_CNGN, prices,
    );
    let mut replaced = 0;
    for meta in swapped.accounts.iter_mut() {
        if meta.pubkey == real_vault {
            meta.pubkey = other_vault;
            replaced += 1;
        }
    }
    assert_eq!(replaced, 1, "the collateral vault must appear exactly once to be substituted");

    assert_hodl_error(
        send(&mut env.svm, &[swapped], &[&liquidator.key]),
        HodlError::CollateralVaultMismatch,
    );

    // The same call against the position's own vault succeeds, so the rejection is about the
    // substitution and not about the rest of the setup.
    env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, ONE_CNGN)
        .expect("the real vault liquidates");
}

#[test]
fn pausing_borrowing_leaves_the_liquidation_line_where_it_was() {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    let pause = set_collateral_borrow_paused_ix(&env.guardian.pubkey(), &setup.usdc, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();

    let liquidator = env.new_liquidator(&setup.cngn, LOAN);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    // The position is still healthy. The pause removed borrowing power, not the collateral
    // standing behind debt already taken — otherwise every live loan against the asset would
    // become liquidatable the instant an admin paused it.
    //
    // What holds that in place is *which* accumulations in `compute_health` consult
    // `lends_borrowing_power`: the LTV term and the promo cap, and nothing else. `own_value`
    // and `liquidation_line` count a withheld holding in full, so `is_liquidatable` cannot
    // move. That is a property of two specific call sites, not of the type — an earlier draft
    // gated only the LTV term and left the promo cap still unlocking borrowing power against a
    // paused asset. Anything new that raises `borrow_limit` has to be gated as well, and this
    // test will not notice if it is not: it pins the blast radius, not the gate count.
    let result = env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, ONE_CNGN);
    assert_hodl_error(result, HodlError::NotLiquidatable);

    // And once the price does fall, the pause is no obstacle to seizing it.
    env.set_pyth_price(&setup.usdc, USDC_CRASHED, 0);
    env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, ONE_CNGN).unwrap();
}
