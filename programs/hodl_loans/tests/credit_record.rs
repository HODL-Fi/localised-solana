//! `CreditRecord` — one account per borrower recording how many loans they repaid and how many
//! were closed by liquidation.
//!
//! Facts, not a score. The counters say *what happened*; how much a late repayment should cost a
//! borrower's standing is a judgement that belongs off chain, over these accounts and the
//! `CreditRepaymentRecorded` / `CreditDefaultRecorded` events beside them.

mod common;

use common::*;
use anchor_lang::prelude::Pubkey;
use anchor_lang::Space;
use hodl_loans::CreditRecord;
use solana_signer::Signer;

const DAY: i64 = 86_400;
const LOAN: u64 = 700_000 * ONE_CNGN;
/// $0.45 at Pyth exponent -8, which puts 1,000 USDC of collateral under a $437.94 debt.
const USDC_CRASHED: i64 = 45_000_000;

fn record(env: &Env, owner: &Pubkey) -> CreditRecord {
    env.fetch::<CreditRecord>(&credit_record_pda(owner))
}

/// The demo shot: repay a loan in full, and the borrower's record reads 1.
#[test]
fn a_full_repayment_creates_the_record_and_counts_one() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    // No record exists until something closes a loan — the account is created on demand, so a
    // wallet that has never borrowed has no account to read.
    assert!(env.svm.get_account(&credit_record_pda(&owner)).is_none());

    env.take_loan(&setup.borrower, &setup, LOAN, 30 * DAY).unwrap();
    env.mint_to(&setup.cngn, &setup.borrower_cngn, LOAN);
    env.repay(&setup, 0, u64::MAX).unwrap();

    let r = record(&env, &owner);
    assert_eq!(r.owner, owner, "the record is the borrower's, and seeded from their address");
    assert_eq!((r.loans_completed, r.loans_defaulted), (1, 0));
    assert_eq!(r.version, 1);
}

/// Partial payments must not count. A borrower who pays a loan down in ten instalments has repaid
/// one loan, not ten — the counter would be trivially inflatable otherwise.
#[test]
fn partial_repayments_do_not_count() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    env.take_loan(&setup.borrower, &setup, LOAN, 30 * DAY).unwrap();
    env.mint_to(&setup.cngn, &setup.borrower_cngn, LOAN);

    for _ in 0..3 {
        env.repay(&setup, 0, 100_000 * ONE_CNGN).unwrap();
        // The account exists from the first repayment — `init_if_needed` runs whatever the outcome —
        // but the counter has not moved.
        assert_eq!(record(&env, &owner).loans_completed, 0);
    }

    env.repay(&setup, 0, u64::MAX).unwrap();
    assert_eq!(record(&env, &owner).loans_completed, 1);
}

/// A third party may repay anyone's loan, and the history belongs to the borrower. If this credited
/// the payer, a lender could farm a clean record by settling other people's debts.
#[test]
fn a_third_party_repayment_credits_the_borrower_not_the_payer() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    env.take_loan(&setup.borrower, &setup, LOAN, 30 * DAY).unwrap();

    let helper = env.new_lender(&setup.cngn, LOAN * 2);
    let payer = helper.key.pubkey();
    let instruction = repay_loan_ix(&payer, &owner, &setup.cngn, &helper.token, 0, u64::MAX);
    send(&mut env.svm, &[instruction], &[&env.admin, &helper.key]).unwrap();

    assert_eq!(record(&env, &owner).loans_completed, 1);
    // And the payer got no record of their own out of it.
    assert!(env.svm.get_account(&credit_record_pda(&payer)).is_none());
}

/// Append-only across loans: the same account accumulates, rather than a new one per loan.
#[test]
fn the_record_accumulates_and_is_never_reset() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    env.mint_to(&setup.cngn, &setup.borrower_cngn, LOAN * 4);

    for expected in 1..=3u64 {
        env.take_loan(&setup.borrower, &setup, LOAN, 30 * DAY).unwrap();
        let loan_id = expected - 1;
        env.repay(&setup, loan_id, u64::MAX).unwrap();
        assert_eq!(record(&env, &owner).loans_completed, expected);
    }
    assert_eq!(record(&env, &owner).loans_defaulted, 0);
}

/// Lateness lands in the event, not the counter: a loan repaid 400 days late still counts as
/// completed. Encoding degrees of lateness in the counter would be scoring.
#[test]
fn a_late_repayment_still_counts_as_completed() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    env.take_loan(&setup.borrower, &setup, LOAN, 30 * DAY).unwrap();
    env.mint_to(&setup.cngn, &setup.borrower_cngn, LOAN * 8);
    env.warp_seconds(30 * DAY + 400 * DAY);

    env.repay(&setup, 0, u64::MAX).unwrap();
    assert_eq!(record(&env, &owner).loans_completed, 1);
}

/// A default is recorded when the loan actually closes — which in practice is at the **write-off**,
/// not at the liquidation.
///
/// A liquidation is bounded by the collateral it can seize, so it usually leaves principal behind;
/// `write_off_loan` is what finally clears a loan whose remaining collateral is dust. Wiring only
/// `liquidate` would have left `loans_defaulted` almost never moving, which is why the write-off
/// carries it too — and why the counter is required there and optional on `liquidate`.
#[test]
fn a_default_is_recorded_when_the_write_off_closes_the_loan() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    env.set_pyth_price(&setup.usdc, USDC_CRASHED, 0);

    let liquidator = env.new_liquidator(&setup.cngn, LOAN * 4);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, 300_000 * ONE_CNGN).unwrap();

    // A partial liquidation left the loan open, so nothing is recorded yet.
    assert!(env.position(&owner).has_active_loans());
    assert_eq!(record(&env, &owner).loans_defaulted, 0, "a partial liquidation is not a default");

    // A write-off needs the remaining collateral to be worth less than `bad_debt_dust_usd`, so the
    // price has to collapse the rest of the way — the same sequence as
    // `a_default_runs_from_liquidation_to_write_off`.
    env.set_pyth_price(&setup.usdc, 100_000, 0);
    env.write_off(&setup, 0).unwrap();
    assert!(!env.position(&owner).has_active_loans());
    let r = record(&env, &owner);
    assert_eq!((r.loans_completed, r.loans_defaulted), (0, 1));
    assert_eq!(r.owner, owner);
    // The liquidator and the admin both paid into this flow and neither earned a record.
    assert!(env.svm.get_account(&credit_record_pda(&liquidator.pubkey())).is_none());
}

/// The record is optional on `liquidate`, and omitting it must not stop the liquidation.
///
/// This is the property the optionality exists for: adding the account took an 8-slot liquidation
/// past the legacy transaction limit, and a liquidation that cannot be packed is an underwater
/// position that cannot be closed. A counter that did not move is the cheaper failure — and
/// `LoanLiquidated` is still emitted, so an indexer loses nothing.
#[test]
fn a_liquidation_without_the_record_still_closes_the_loan() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    env.set_pyth_price(&setup.usdc, USDC_CRASHED, 0);

    let liquidator = env.new_liquidator(&setup.cngn, LOAN * 2);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    let before = env.token_balance(&collateral_account);
    env.liquidate_without_credit_record(&liquidator, &setup, &setup.usdc, &collateral_account, 0, u64::MAX)
        .unwrap();

    // The liquidation did its job — collateral moved — with no record account in the transaction.
    assert!(env.token_balance(&collateral_account) > before, "collateral should have been seized");
    assert!(
        env.svm.get_account(&credit_record_pda(&owner)).is_none(),
        "no record was passed, so none should have been created"
    );
}

/// Nothing outside the program writes the record, and no instruction lowers a counter.
///
/// The stronger claim — that a borrower cannot shed history by abandoning a position — rests on the
/// seeds: the record is derived from the wallet, never from the position, so a new position finds
/// the same account. That is structural rather than something a test can demonstrate by closing a
/// position, which needs the collateral unwound first.
#[test]
fn the_record_is_program_owned_and_only_grows() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    env.mint_to(&setup.cngn, &setup.borrower_cngn, LOAN * 4);
    env.take_loan(&setup.borrower, &setup, LOAN, 30 * DAY).unwrap();
    env.repay(&setup, 0, u64::MAX).unwrap();
    assert_eq!(record(&env, &owner).loans_completed, 1);

    // A second loan, paid down but not closed, leaves the counter alone.
    env.take_loan(&setup.borrower, &setup, LOAN, 30 * DAY).unwrap();
    env.repay(&setup, 1, 100_000 * ONE_CNGN).unwrap();
    assert_eq!(record(&env, &owner).loans_completed, 1);
    env.repay(&setup, 1, u64::MAX).unwrap();
    assert_eq!(record(&env, &owner).loans_completed, 2);

    // Derived from the wallet, not the position — which is what makes the history unshakeable.
    assert_eq!(credit_record_pda(&owner), credit_record_pda(&setup.borrower.pubkey()));

    let account = env.svm.get_account(&credit_record_pda(&owner)).unwrap();
    assert_eq!(account.owner, hodl_loans::ID);
    assert_eq!(account.data.len(), 8 + CreditRecord::INIT_SPACE);
}
