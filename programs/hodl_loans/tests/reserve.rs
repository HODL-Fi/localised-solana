mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

const DAY: i64 = 86_400;
const RESERVE: u64 = 300 * ONE_CNGN;

/// A 100,000 cNGN loan repaid after 73 days: 3,000 cNGN interest, 300 of it to the reserve.
fn repaid_market() -> (Env, LoanSetup) {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, 100_000 * ONE_CNGN, 365 * DAY).unwrap();
    env.warp_seconds(73 * DAY);
    env.mint_to(&setup.cngn, &setup.borrower_cngn, 3_000 * ONE_CNGN);
    env.repay(&setup, 0, u64::MAX).unwrap();
    assert_eq!(env.market(&setup.cngn).protocol_reserve, RESERVE);
    (env, setup)
}

#[test]
fn admin_harvests_the_reserve_to_the_treasury() {
    let (mut env, setup) = repaid_market();
    let admin = env.admin.pubkey();
    let treasury = env.treasury.pubkey();
    let destination = env.create_token_account(&setup.cngn, &treasury);
    let cash = env.market(&setup.cngn).cash;

    let harvest = harvest_reserve_ix(&admin, &setup.cngn, &destination, 100 * ONE_CNGN);
    send(&mut env.svm, &[harvest], &[&env.admin]).unwrap();
    assert_eq!(env.token_balance(&destination), 100 * ONE_CNGN);
    let market = env.market(&setup.cngn);
    assert_eq!((market.protocol_reserve, market.cash), (200 * ONE_CNGN, cash - 100 * ONE_CNGN));

    let too_much = harvest_reserve_ix(&admin, &setup.cngn, &destination, 200 * ONE_CNGN + 1);
    assert_hodl_error(send(&mut env.svm, &[too_much], &[&env.admin]), HodlError::InsufficientCash);
    let zero = harvest_reserve_ix(&admin, &setup.cngn, &destination, 0);
    assert_hodl_error(send(&mut env.svm, &[zero], &[&env.admin]), HodlError::AmountTooSmall);

    let stranger = env.funded_keypair();
    let by_stranger = harvest_reserve_ix(&stranger.pubkey(), &setup.cngn, &destination, ONE_CNGN);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);
    let stranger_account = env.create_token_account(&setup.cngn, &stranger.pubkey());
    let to_stranger = harvest_reserve_ix(&admin, &setup.cngn, &stranger_account, ONE_CNGN);
    assert_anchor_error(send(&mut env.svm, &[to_stranger], &[&env.admin]), AnchorError::ConstraintTokenOwner);

    let rest = harvest_reserve_ix(&admin, &setup.cngn, &destination, 200 * ONE_CNGN);
    send(&mut env.svm, &[rest], &[&env.admin]).unwrap();
    assert_eq!(env.market(&setup.cngn).protocol_reserve, 0);
}

#[test]
fn lenders_and_the_reserve_are_both_paid_in_full() {
    let (mut env, setup) = repaid_market();
    let vault = market_vault_pda(&setup.cngn);

    // The lender's full exit leaves the reserve (plus share rounding) in the vault.
    env.withdraw(&setup.lender, &setup.cngn, u64::MAX).unwrap();
    assert!(env.token_balance(&setup.lender.token) >= POOL_CNGN + 2_700 * ONE_CNGN - 1);
    let market = env.market(&setup.cngn);
    assert!(market.cash >= market.protocol_reserve);
    assert_eq!(env.token_balance(&vault), market.cash);

    let treasury = env.treasury.pubkey();
    let destination = env.create_token_account(&setup.cngn, &treasury);
    let harvest = harvest_reserve_ix(&env.admin.pubkey(), &setup.cngn, &destination, RESERVE);
    send(&mut env.svm, &[harvest], &[&env.admin]).unwrap();
    assert_eq!(env.token_balance(&destination), RESERVE);
    assert!(env.token_balance(&vault) <= 1);
}
