mod common;

use common::*;
use anchor_lang::error::ErrorCode as AnchorError;
use anchor_lang::prelude::AccountMeta;
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
