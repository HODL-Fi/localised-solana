mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use common::*;
use hodl_loans::{HodlError, LoanSlot};
use solana_keypair::Keypair;
use solana_signer::Signer;

const DAY: i64 = 86_400;
/// 100,000 cNGN.
const LOAN: u64 = 100_000 * ONE_CNGN;
/// 15% a year for 73 days (a fifth of a year) on `LOAN`.
const INTEREST_73_DAYS: u64 = 3_000 * ONE_CNGN;

#[test]
fn full_repayment_pays_lenders_and_the_reserve() {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    env.warp_seconds(73 * DAY);
    env.mint_to(&setup.cngn, &setup.borrower_cngn, INTEREST_73_DAYS);

    // Repayment works while the market is paused and needs no prices (they are stale by now).
    let pause = set_market_paused_ix(&env.guardian.pubkey(), &setup.cngn, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();
    env.repay(&setup, 0, u64::MAX).unwrap();

    assert_eq!(env.token_balance(&setup.borrower_cngn), 0);
    let market = env.market(&setup.cngn);
    assert_eq!((market.total_borrows, market.lp_rate_product, market.accrued_interest), (0, 0, 0));
    assert_eq!(market.cash, POOL_CNGN + INTEREST_73_DAYS);
    // 10% reserve factor.
    assert_eq!(market.protocol_reserve, 300 * ONE_CNGN);
    assert_eq!(env.token_balance(&market_vault_pda(&setup.cngn)), POOL_CNGN + INTEREST_73_DAYS);

    let position = env.position(&setup.borrower.pubkey());
    assert_eq!(position.loans[0], bytemuck::Zeroable::zeroed());
    assert!(!position.has_active_loans());
    assert_eq!(position.promo_last_activity_at, env.now());
    assert_eq!(position.next_loan_id, 1);

    // The lender redeems principal plus 90% of the interest (minus share rounding).
    let unpause = set_market_paused_ix(&env.admin.pubkey(), &setup.cngn, false);
    send(&mut env.svm, &[unpause], &[&env.admin]).unwrap();
    let shares = env.lender_shares(&setup.cngn, &setup.lender.key.pubkey());
    let total_assets = POOL_CNGN as u128 + INTEREST_73_DAYS as u128 - 300 * ONE_CNGN as u128;
    let expected = shares * (total_assets + 1) / (shares + 1_000);
    env.withdraw(&setup.lender, &setup.cngn, u64::MAX).unwrap();
    assert_eq!(env.token_balance(&setup.lender.token) as u128, expected);
    assert!(expected > (POOL_CNGN + 2_699 * ONE_CNGN) as u128);
}

#[test]
fn partial_repayment_pays_interest_first_and_restarts_it() {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    let originated_at = env.now();
    env.warp_seconds(73 * DAY);
    env.mint_to(&setup.cngn, &setup.borrower_cngn, INTEREST_73_DAYS);

    assert_hodl_error(env.repay(&setup, 0, INTEREST_73_DAYS - 1), HodlError::RepaymentBelowInterest);
    env.repay(&setup, 0, INTEREST_73_DAYS + 50_000 * ONE_CNGN).unwrap();

    let loan = env.position(&setup.borrower.pubkey()).loans[0];
    assert_eq!((loan.active, loan.principal, loan.original_principal), (1, 50_000 * ONE_CNGN, LOAN));
    assert_eq!(loan.repaid, INTEREST_73_DAYS + 50_000 * ONE_CNGN);
    assert_eq!((loan.originated_at, loan.interest_anchor), (originated_at, env.now()));
    let market = env.market(&setup.cngn);
    assert_eq!(market.total_borrows, 50_000 * ONE_CNGN);
    assert_eq!(market.lp_rate_product, 50_000 * ONE_CNGN as u128 * 1_500 * 9_000);
    assert_eq!((market.accrued_interest, market.protocol_reserve), (0, 300 * ONE_CNGN));

    // Another 73 days accrues interest only on the remaining 50,000 cNGN.
    env.warp_seconds(73 * DAY);
    env.mint_to(&setup.cngn, &setup.borrower_cngn, 1_500 * ONE_CNGN);
    let before = env.token_balance(&setup.borrower_cngn);
    env.repay(&setup, 0, u64::MAX).unwrap();
    assert_eq!(before - env.token_balance(&setup.borrower_cngn), 51_500 * ONE_CNGN);
    let market = env.market(&setup.cngn);
    assert_eq!((market.total_borrows, market.lp_rate_product, market.accrued_interest), (0, 0, 0));
    assert_eq!(market.protocol_reserve, 450 * ONE_CNGN);
}

#[test]
fn overdue_loans_add_penalty_interest() {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, LOAN, 73 * DAY).unwrap();
    env.warp_seconds(146 * DAY);
    // Interest to maturity 3,000; then (100,000 + 3,000) × (15% + 5%) × 1/5 year = 4,120.
    let due = 7_120 * ONE_CNGN;
    env.mint_to(&setup.cngn, &setup.borrower_cngn, due);

    assert_hodl_error(env.repay(&setup, 0, due - 1), HodlError::RepaymentBelowInterest);
    env.repay(&setup, 0, u64::MAX).unwrap();
    assert_eq!(env.token_balance(&setup.borrower_cngn), 0);
    let market = env.market(&setup.cngn);
    assert_eq!(market.cash, POOL_CNGN + due);
    assert_eq!(market.protocol_reserve, 712 * ONE_CNGN);
    assert_eq!((market.total_borrows, market.accrued_interest), (0, 0));
}

#[test]
fn any_whitelisted_wallet_can_repay_even_for_a_blacklisted_borrower() {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    env.blacklist(&setup.borrower.pubkey());
    assert_hodl_error(env.repay(&setup, 0, LOAN), HodlError::Blacklisted);

    let helper = env.new_lender(&setup.cngn, LOAN);
    let owner = setup.borrower.pubkey();
    let instruction = repay_loan_ix(&helper.key.pubkey(), &owner, &setup.cngn, &helper.token, 0, u64::MAX);
    send(&mut env.svm, &[instruction], &[&env.admin, &helper.key]).unwrap();
    assert_eq!(env.token_balance(&helper.token), 0);
    assert!(!env.position(&owner).has_active_loans());
    // The borrower keeps the loaned cNGN.
    assert_eq!(env.token_balance(&setup.borrower_cngn), LOAN);
}

#[test]
fn repayment_rejections() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();

    // A position that has never borrowed has no market.
    assert_hodl_error(env.repay(&setup, 0, LOAN), HodlError::MarketMismatch);

    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    assert_hodl_error(env.repay(&setup, 0, 0), HodlError::AmountTooSmall);
    assert_hodl_error(env.repay(&setup, 1, LOAN), HodlError::LoanNotFound);
    env.repay(&setup, 0, LOAN).unwrap();
    assert_hodl_error(env.repay(&setup, 0, LOAN), HodlError::LoanNotFound);

    let stranger = Keypair::new();
    let stranger_token = env.create_token_account(&setup.cngn, &stranger.pubkey());
    let instruction = repay_loan_ix(&stranger.pubkey(), &owner, &setup.cngn, &stranger_token, 0, LOAN);
    assert_anchor_error(send(&mut env.svm, &[instruction], &[&env.admin, &stranger]), AnchorError::AccountNotInitialized);

    // Repaying through a different market.
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    let other = env.create_mint(MintKind::CngnLike, 6);
    let create = create_market_ix(&env.admin.pubkey(), &other, &TOKEN_2022, default_market_params());
    send(&mut env.svm, &[create], &[&env.admin]).unwrap();
    let other_token = env.create_token_account(&other, &owner);
    let instruction = repay_loan_ix(&owner, &owner, &other, &other_token, 1, LOAN);
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin, &setup.borrower.key]), HodlError::MarketMismatch);
}

#[test]
fn repaid_slots_are_reused_but_ids_are_not() {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    env.repay(&setup, 0, LOAN).unwrap();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();

    let loans = env.position(&setup.borrower.pubkey()).loans;
    let (first, second): (LoanSlot, LoanSlot) = (loans[0], loans[1]);
    assert_eq!((first.id, first.active, second.id, second.active), (2, 1, 1, 1));
}
