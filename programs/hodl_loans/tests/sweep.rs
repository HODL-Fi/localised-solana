mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

#[test]
fn sweep_sends_only_donations_to_treasury() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, ONE_CNGN);
    env.deposit(&lender, &mint, ONE_CNGN).unwrap();
    let vault = market_vault_pda(&mint);
    env.mint_to(&mint, &vault, 250_000);

    let treasury_owner = env.treasury.pubkey();
    let destination = env.create_token_account(&mint, &treasury_owner);
    let sweep = sweep_market_excess_ix(&env.admin.pubkey(), &mint, &destination);
    send(&mut env.svm, &[sweep], &[&env.admin]).unwrap();

    assert_eq!(env.token_balance(&destination), 250_000);
    assert_eq!(env.token_balance(&vault), ONE_CNGN);
    assert_eq!(env.market(&mint).cash, ONE_CNGN);
}

#[test]
fn sweep_fails_without_excess() {
    let (mut env, mint) = Env::with_cngn_market();
    let treasury_owner = env.treasury.pubkey();
    let destination = env.create_token_account(&mint, &treasury_owner);
    let sweep = sweep_market_excess_ix(&env.admin.pubkey(), &mint, &destination);
    assert_hodl_error(send(&mut env.svm, &[sweep], &[&env.admin]), HodlError::AmountTooSmall);
}

#[test]
fn sweep_only_to_treasury_and_only_by_admin() {
    let (mut env, mint) = Env::with_cngn_market();
    env.mint_to(&mint, &market_vault_pda(&mint), 250_000);

    let stranger = env.funded_keypair();
    let wrong_destination = env.create_token_account(&mint, &stranger.pubkey());
    let to_stranger = sweep_market_excess_ix(&env.admin.pubkey(), &mint, &wrong_destination);
    assert_anchor_error(send(&mut env.svm, &[to_stranger], &[&env.admin]), AnchorError::ConstraintTokenOwner);

    let treasury_owner = env.treasury.pubkey();
    let destination = env.create_token_account(&mint, &treasury_owner);
    let by_stranger = sweep_market_excess_ix(&stranger.pubkey(), &mint, &destination);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);
}
