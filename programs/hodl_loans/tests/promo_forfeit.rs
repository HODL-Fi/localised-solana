mod common;

use common::*;
use anchor_lang::error::ErrorCode as AnchorError;
use anchor_lang::prelude::AccountMeta;
use hodl_loans::HodlError;
use solana_signer::Signer;

const DAY: i64 = 86_400;
const GRANT: u64 = 50_000 * ONE_CNGN;
const LOAN: u64 = 700_000 * ONE_CNGN;

/// A position holding 1,000 USDC and `GRANT` of promo, owing `LOAN`, with USDC crashed to
/// $0.45 — under its 90% line even with the promo counted.
fn underwater_with_promo() -> (Env, LoanSetup) {
    let (mut env, setup) = Env::promo_ready();
    env.redeem_promo(&setup.borrower, &setup.cngn, 1, GRANT, 7).unwrap();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    env.set_pyth_price(&setup.usdc, 45_000_000, 0);
    (env, setup)
}

#[test]
fn liquidating_a_position_hands_its_promo_to_lenders() {
    let (mut env, setup) = underwater_with_promo();
    let owner = setup.borrower.pubkey();
    let liquidator = env.new_liquidator(&setup.cngn, 1_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    let vault_before = env.promo_vault(&setup.cngn);
    let cash_before = env.market(&setup.cngn).cash;
    let market_tokens_before = env.token_balance(&market_vault_pda(&setup.cngn));
    let repaid_before = env.position(&owner).loans[0].repaid;

    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 100_000 * ONE_CNGN).unwrap();

    // The promo is gone from the position and from the vault's books, and this time the cNGN
    // really moved: `cash` falls with `outstanding`, unlike expiry.
    assert_eq!(env.position(&owner).promo_balance, 0);
    let vault = env.promo_vault(&setup.cngn);
    assert_eq!(vault.outstanding, vault_before.outstanding - GRANT);
    assert_eq!(vault.cash, vault_before.cash - GRANT);
    assert_eq!(env.token_balance(&promo_vault_token_pda(&setup.cngn)), vault.cash);

    // Lenders receive it on top of the liquidator's repayment.
    let paid = env.position(&owner).loans[0].repaid - repaid_before;
    assert_eq!(env.market(&setup.cngn).cash, cash_before + paid + GRANT);
    assert_eq!(env.token_balance(&market_vault_pda(&setup.cngn)), market_tokens_before + paid + GRANT);
}

#[test]
fn the_forfeit_does_not_reduce_what_the_borrower_owes() {
    // Two identical debts, one backed by promo. Liquidating both by the same amount must leave
    // the two loans in exactly the same state: the promo goes to lenders, not to the borrower's
    // balance. Anything else would mean the protocol had paid down its own borrower's debt.
    let (mut env, setup) = underwater_with_promo();
    let promoed = setup.borrower.pubkey();

    let plain = env.new_borrower();
    env.deposit_collateral(&plain, &setup.usdc, 1_000 * ONE_USDC);
    let plain_cngn = env.create_token_account(&setup.cngn, &plain.pubkey());
    env.set_pyth_price(&setup.usdc, ONE_DOLLAR, 0);
    let prices = env.price_accounts(&plain.pubkey());
    let borrow = take_loan_ix(&plain.pubkey(), &setup.cngn, &plain_cngn, LOAN, 365 * DAY, prices);
    send(&mut env.svm, &[borrow], &[&env.admin, &plain.key]).unwrap();
    env.set_pyth_price(&setup.usdc, 45_000_000, 0);

    let liquidator = env.new_liquidator(&setup.cngn, 2_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 100_000 * ONE_CNGN).unwrap();

    let prices = env.price_accounts(&plain.pubkey());
    let seize = liquidate_ix(
        &liquidator.pubkey(), &plain.pubkey(), &setup.cngn, &liquidator.cngn, &setup.usdc,
        &SPL_TOKEN, &seized_to, 0, 100_000 * ONE_CNGN, prices,
    );
    send(&mut env.svm, &[seize], &[&liquidator.key]).unwrap();

    let with_promo = env.position(&promoed).loans[0];
    let without = env.position(&plain.pubkey()).loans[0];
    assert_eq!(with_promo.principal, without.principal);
    assert_eq!(with_promo.repaid, without.repaid);
    assert!(with_promo.principal > 0);

    // The only difference is where the promo went.
    assert_eq!(env.position(&promoed).promo_balance, 0);
    assert_eq!(env.promo_vault(&setup.cngn).outstanding, 0);
}

#[test]
fn promo_is_forfeited_once_and_a_later_liquidation_finds_none() {
    let (mut env, setup) = underwater_with_promo();
    let owner = setup.borrower.pubkey();
    let liquidator = env.new_liquidator(&setup.cngn, 1_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 50_000 * ONE_CNGN).unwrap();
    let after_first = env.promo_vault(&setup.cngn);
    assert_eq!(env.position(&owner).promo_balance, 0);

    // The position is still under water, so it can be liquidated again — and the vault is not
    // charged a second time.
    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 50_000 * ONE_CNGN).unwrap();
    let after_second = env.promo_vault(&setup.cngn);
    assert_eq!((after_second.cash, after_second.outstanding), (after_first.cash, after_first.outstanding));
}

#[test]
fn writing_off_a_loan_forfeits_the_promo_as_well() {
    let (mut env, setup) = underwater_with_promo();
    let owner = setup.borrower.pubkey();
    let admin = env.admin.pubkey();

    // Collateral worth $1 is below the $5 dust threshold, so the loan can be written off.
    env.set_pyth_price(&setup.usdc, 100_000, 0);
    let vault_before = env.promo_vault(&setup.cngn);
    let cash_before = env.market(&setup.cngn).cash;
    let market_tokens_before = env.token_balance(&market_vault_pda(&setup.cngn));

    let prices = env.price_accounts(&owner);
    let write_off = write_off_loan_ix(&admin, &owner, &setup.cngn, 0, prices);
    send(&mut env.svm, &[write_off], &[&env.admin]).unwrap();

    assert_eq!(env.position(&owner).promo_balance, 0);
    let vault = env.promo_vault(&setup.cngn);
    assert_eq!(vault.cash, vault_before.cash - GRANT);
    assert_eq!(vault.outstanding, vault_before.outstanding - GRANT);
    // The promo offsets part of the loss the lenders would otherwise carry alone.
    assert_eq!(env.market(&setup.cngn).cash, cash_before + GRANT);
    assert!(env.market(&setup.cngn).total_bad_debt > 0);

    // `write_off_loan` moving tokens at all is the most novel behaviour in this task — pin
    // both legs of the transfer, not just the program-side bookkeeping.
    assert_eq!(env.token_balance(&promo_vault_token_pda(&setup.cngn)), vault.cash);
    assert_eq!(env.token_balance(&market_vault_pda(&setup.cngn)), market_tokens_before + GRANT);
}

#[test]
fn a_liquidation_must_name_the_positions_own_promo_vault() {
    let (mut env, setup) = underwater_with_promo();
    let owner = setup.borrower.pubkey();
    let liquidator = env.new_liquidator(&setup.cngn, 1_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    // Another market's promo vault would let a liquidator skip the forfeit by charging a vault
    // the position never drew on. The seeds are derived from this market, so it cannot be used.
    let other = env.create_mint(MintKind::CngnLike, 6);
    env.create_market_with_promo(&other);
    let prices = env.price_accounts(&owner);
    let mut instruction = liquidate_ix(
        &liquidator.pubkey(), &owner, &setup.cngn, &liquidator.cngn, &setup.usdc, &SPL_TOKEN,
        &seized_to, 0, 100_000 * ONE_CNGN, prices,
    );
    let slot = instruction.accounts.iter().position(|a| a.pubkey == promo_vault_pda(&setup.cngn)).unwrap();
    instruction.accounts[slot] = AccountMeta::new(promo_vault_pda(&other), false);
    // The seeds are derived from this market, so another market's vault cannot be substituted.
    let result = send(&mut env.svm, &[instruction], &[&liquidator.key]);
    assert_anchor_error(result, AnchorError::ConstraintSeeds);

    // Named correctly, the same liquidation goes through.
    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 100_000 * ONE_CNGN).unwrap();
    assert_eq!(env.position(&owner).promo_balance, 0);
}

#[test]
fn a_dust_liquidation_still_forfeits_the_entire_promo_balance() {
    // A liquidator can trigger the forfeit with an arbitrarily small repayment: `forfeit_promo`
    // releases the whole `promo_balance` regardless of how much of the loan `amount` actually
    // repays. Pinned so this cannot silently change to a proportional release.
    let (mut env, setup) = underwater_with_promo();
    let owner = setup.borrower.pubkey();
    let liquidator = env.new_liquidator(&setup.cngn, 10_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    let debt_before = env.position(&owner).loans[0].principal;
    // 1,000 cNGN against a 700,000 cNGN loan: a dust-sized repayment, not a real dent.
    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 1_000 * ONE_CNGN).unwrap();

    // The whole promo balance is gone in one dust-sized repayment.
    assert_eq!(env.position(&owner).promo_balance, 0);
    assert_eq!(env.promo_vault(&setup.cngn).outstanding, 0);

    // The position survives: only a sliver of principal was actually repaid, and the loan
    // stays open with debt remaining rather than closing.
    let debt_after = env.position(&owner).loans[0].principal;
    assert!(debt_after > 0, "the loan must still be open after a dust repayment");
    assert!(debt_after < debt_before, "some principal must have been repaid");
}

#[test]
fn liquidation_reverts_when_a_promo_holding_position_omits_the_promo_accounts() {
    // `promo_balance > 0` implies the promo vault exists, so there is no legitimate reason to
    // omit these accounts — the liquidator cannot use their absence to skip the forfeit.
    let (mut env, setup) = underwater_with_promo();
    let owner = setup.borrower.pubkey();
    let liquidator = env.new_liquidator(&setup.cngn, 1_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    let program = env.mint_program(&setup.usdc);
    let prices = env.price_accounts(&owner);
    let instruction = liquidate_ix_no_promo(
        &liquidator.pubkey(), &owner, &setup.cngn, &liquidator.cngn, &setup.usdc, &program,
        &seized_to, 0, 100_000 * ONE_CNGN, prices,
    );
    let result = send(&mut env.svm, &[instruction], &[&liquidator.key]);
    assert_hodl_error(result, HodlError::PromoAccountsRequired);

    // The promo survives the failed attempt untouched.
    assert_eq!(env.position(&owner).promo_balance, GRANT);
}

#[test]
fn write_off_reverts_when_a_promo_holding_position_omits_the_promo_accounts() {
    let (mut env, setup) = underwater_with_promo();
    let owner = setup.borrower.pubkey();
    let admin = env.admin.pubkey();

    // Collateral worth $1 is below the $5 dust threshold, so the loan is write-off eligible.
    env.set_pyth_price(&setup.usdc, 100_000, 0);
    let prices = env.price_accounts(&owner);
    let instruction = write_off_loan_ix_no_promo(&admin, &owner, &setup.cngn, 0, prices);
    let result = send(&mut env.svm, &[instruction], &[&env.admin]);
    assert_hodl_error(result, HodlError::PromoAccountsRequired);

    // The promo survives the failed attempt untouched, and no loss was booked.
    assert_eq!(env.position(&owner).promo_balance, GRANT);
    assert_eq!(env.market(&setup.cngn).total_bad_debt, 0);
}

#[test]
fn liquidation_at_the_promo_lifted_boundary_never_books_bad_debt() {
    // At the shipped config (LT 90%, promo cap 20%), `collateral.rs` argues the effective
    // liquidation line — LT×V + promo_counted — can sit above 100% of collateral value V,
    // because forfeiture returns the FULL, uncapped promo balance while the line was lifted
    // only by the CAPPED value. This pins that claim at the boundary itself, rather than the
    // deep-crash scenario `underwater_with_promo` exercises (which manufactures bad debt from
    // the price crash regardless of promo, so it cannot distinguish "the lift was covered"
    // from "it wasn't").
    let (mut env, setup) = Env::promo_ready();
    let owner = setup.borrower.pubkey();

    // Comfortably clears the 20% promo cap ($200 at $1,000 own_value) without hitting
    // `max_promo_per_position`.
    let grant: u64 = 500_000 * ONE_CNGN;
    env.set_max_promo_per_position(&setup.cngn, grant);
    env.redeem_promo(&setup.borrower, &setup.cngn, 1, grant, 7).unwrap();

    // Borrow while USDC is temporarily worth $5/token (own_value $5,000), so this debt is
    // comfortably healthy at origination — `take_loan`'s cap is the LTV-based borrow limit,
    // which sits below the LT-based liquidation line this test targets.
    //
    // `debt_raw` is chosen so its USD value at the NGN ask is $1,099.999999999526 — the
    // closest a u64 cNGN base-unit amount can land below $1,100, which is exactly
    // LT 90% × $1,000 + cap 20% × $1,000, the line at $1,000 own_value (derived via
    // `hodl_loans::math::price::token_value_ceil` and `hodl_loans::math::health::compute_health`
    // offline, not restated here).
    env.set_pyth_price(&setup.usdc, 5 * ONE_DOLLAR, 0);
    let debt_raw: u64 = 1_758_241_758_241;
    let prices = env.price_accounts(&owner);
    let borrow = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, debt_raw, 365 * DAY, prices);
    send(&mut env.svm, &[borrow], &[&env.admin, &setup.borrower.key]).unwrap();

    // Back to $1/token: own_value is exactly $1,000 again, so the fixed debt sits a hair under
    // the line — not yet liquidatable. No time has elapsed (no `warp_seconds` since origination),
    // so no interest or penalty has accrued and the debt is still exactly `debt_raw`.
    env.set_pyth_price(&setup.usdc, ONE_DOLLAR, 0);
    let liquidator = env.new_liquidator(&setup.cngn, 2_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    let at_the_line = env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, debt_raw);
    assert_hodl_error(at_the_line, HodlError::NotLiquidatable);

    // A tenth-of-a-cent nudge crosses it: $0.999/token drops own_value (and the line with it)
    // just below the fixed debt. The debt now exceeds 100% of collateral value, so — even
    // before the liquidation bonus, which only shrinks recoverable value further — a single
    // seizure can never fully repay it; this is necessarily a partial liquidation.
    env.set_pyth_price(&setup.usdc, 999 * ONE_DOLLAR / 1_000, 0);
    let market_cash_before = env.market(&setup.cngn).cash;
    let promo_cash_before = env.promo_vault(&setup.cngn).cash;
    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, debt_raw)
        .expect("liquidation must succeed once nudged past the line");

    // This is the actual recovery-vs-lift comparison, and the reason the assertions above are
    // not enough on their own. The lift is bounded by the 20% cap on $1,000 of own collateral,
    // so it is at most $200 — pinned by the `NotLiquidatable` assertion above, which only holds
    // because promo raised the line that far. The recovery is the FULL grant, uncapped: 500,000
    // cNGN, worth about $312 at the NGN bid. Recovery therefore exceeds lift by roughly $112,
    // and these two assertions pin that the whole grant really moved rather than the counted
    // portion.
    assert_eq!(env.promo_vault(&setup.cngn).cash, promo_cash_before - grant);
    assert!(env.market(&setup.cngn).cash >= market_cash_before + grant);

    // `total_bad_debt` is written only by `write_off_loan`, which this test never calls, so on
    // its own this says little — it is here to record that the position never reached the
    // write-off path at all, not as evidence that the lift was covered.
    assert_eq!(env.market(&setup.cngn).total_bad_debt, 0);
    assert_eq!(env.position(&owner).promo_balance, 0);
    assert_eq!(env.promo_vault(&setup.cngn).outstanding, 0);
}
