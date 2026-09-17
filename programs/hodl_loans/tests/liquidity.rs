mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

// ---- deposit_liquidity (Task 7) ----

#[test]
fn first_deposit_mints_shares_and_moves_tokens() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, 5 * ONE_CNGN);
    env.deposit(&lender, &mint, ONE_CNGN).unwrap();

    let market = env.market(&mint);
    assert_eq!(market.cash, ONE_CNGN);
    assert_eq!(market.total_shares, 1_000_000_000);
    assert_eq!(env.lender_shares(&mint, &lender.key.pubkey()), 1_000_000_000);
    assert_eq!(env.token_balance(&market_vault_pda(&mint)), ONE_CNGN);
    assert_eq!(env.token_balance(&lender.token), 4 * ONE_CNGN);
}

#[test]
fn later_deposits_get_proportional_shares() {
    let (mut env, mint) = Env::with_cngn_market();
    let first = env.new_lender(&mint, ONE_CNGN);
    let second = env.new_lender(&mint, ONE_CNGN);
    env.deposit(&first, &mint, ONE_CNGN).unwrap();
    env.deposit(&second, &mint, ONE_CNGN / 2).unwrap();
    assert_eq!(env.lender_shares(&mint, &second.key.pubkey()), 500_000_000);
    assert_eq!(env.market(&mint).total_shares, 1_500_000_000);
}

#[test]
fn donations_do_not_change_share_price() {
    let (mut env, mint) = Env::with_cngn_market();
    let first = env.new_lender(&mint, ONE_CNGN);
    let second = env.new_lender(&mint, ONE_CNGN);
    env.deposit(&first, &mint, ONE_CNGN).unwrap();
    env.mint_to(&mint, &market_vault_pda(&mint), 10 * ONE_CNGN);

    env.deposit(&second, &mint, ONE_CNGN / 2).unwrap();
    assert_eq!(env.lender_shares(&mint, &second.key.pubkey()), 500_000_000);
    assert_eq!(env.market(&mint).cash, ONE_CNGN + ONE_CNGN / 2);
}

#[test]
fn accrued_interest_raises_share_price() {
    let (mut env, mint) = Env::with_cngn_market();
    let first = env.new_lender(&mint, 1_000 * ONE_CNGN);
    let second = env.new_lender(&mint, 1_000 * ONE_CNGN);
    env.deposit(&first, &mint, 1_000 * ONE_CNGN).unwrap();

    // Simulate a 1,000 cNGN loan at 10% with a 10% reserve factor, then let a year pass.
    let key = market_pda(&mint);
    let mut market = env.market(&mint);
    market.lp_rate_product = (1_000 * ONE_CNGN as u128) * 1_000 * 9_000;
    env.write(&key, &market);
    env.warp_seconds(YEAR_SECONDS);

    env.deposit(&second, &mint, 1_000 * ONE_CNGN).unwrap();
    assert_eq!(env.market(&mint).accrued_interest, 90 * ONE_CNGN as u128);
    assert!(env.lender_shares(&mint, &second.key.pubkey()) < env.lender_shares(&mint, &first.key.pubkey()));
}

#[test]
fn deposit_requires_an_active_whitelisted_wallet() {
    let (mut env, mint) = Env::with_cngn_market();
    let stranger = solana_keypair::Keypair::new();
    let token = env.create_token_account(&mint, &stranger.pubkey());
    env.mint_to(&mint, &token, ONE_CNGN);
    let never_whitelisted = deposit_liquidity_ix(&env.admin.pubkey(), &stranger.pubkey(), &mint, &token, ONE_CNGN);
    assert_anchor_error(send(&mut env.svm, &[never_whitelisted], &[&env.admin, &stranger]), AnchorError::AccountNotInitialized);

    let lender = env.new_lender(&mint, ONE_CNGN);
    env.blacklist(&lender.key.pubkey());
    assert_hodl_error(env.deposit(&lender, &mint, ONE_CNGN), HodlError::Blacklisted);
}

#[test]
fn deposit_is_blocked_while_paused_and_for_zero() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, ONE_CNGN);
    assert_hodl_error(env.deposit(&lender, &mint, 0), HodlError::AmountTooSmall);

    let pause = set_market_paused_ix(&env.guardian.pubkey(), &mint, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();
    assert_hodl_error(env.deposit(&lender, &mint, ONE_CNGN), HodlError::MarketPaused);
}
