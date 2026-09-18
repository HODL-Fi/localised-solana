mod common;

use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

const DAY: i64 = 86_400;

#[test]
fn without_loans_withdrawal_needs_no_market_or_prices() {
    let mut env = Env::initialized();
    let usdc = env.list_spl_collateral(6);
    let borrower = env.new_borrower();
    let owner = borrower.pubkey();
    let token = env.deposit_collateral(&borrower, &usdc, 1_000 * ONE_USDC);

    // Collateral pauses block deposits only.
    let pause = set_collateral_paused_ix(&env.guardian.pubkey(), &usdc, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();

    let withdraw = |amount| withdraw_collateral_ix(&owner, &usdc, &SPL_TOKEN, &token, None, amount, vec![]);
    env.sponsored(withdraw(400 * ONE_USDC), &borrower.key).unwrap();
    assert_eq!(env.token_balance(&token), 400 * ONE_USDC);
    assert_eq!(env.position(&owner).collateral[0].amount, 600 * ONE_USDC);
    assert_eq!(env.collateral(&usdc).total_deposited, 600 * ONE_USDC);

    env.sponsored(withdraw(600 * ONE_USDC), &borrower.key).unwrap();
    assert!(!env.position(&owner).has_collateral());
    assert_eq!(env.token_balance(&collateral_vault_pda(&usdc)), 0);

    // An emptied position can be closed.
    let close = close_position_ix(&owner, &env.admin.pubkey());
    env.sponsored(close, &borrower.key).unwrap();
}

#[test]
fn withdrawal_rejections() {
    let mut env = Env::initialized();
    let usdc = env.list_spl_collateral(6);
    let usdt = env.list_spl_collateral(6);
    let borrower = env.new_borrower();
    let owner = borrower.pubkey();
    let token = env.deposit_collateral(&borrower, &usdc, 10 * ONE_USDC);
    let usdt_token = env.create_token_account(&usdt, &owner);

    let zero = withdraw_collateral_ix(&owner, &usdc, &SPL_TOKEN, &token, None, 0, vec![]);
    assert_hodl_error(env.sponsored(zero, &borrower.key), HodlError::AmountTooSmall);
    let too_much = withdraw_collateral_ix(&owner, &usdc, &SPL_TOKEN, &token, None, 10 * ONE_USDC + 1, vec![]);
    assert_hodl_error(env.sponsored(too_much, &borrower.key), HodlError::InsufficientCollateral);
    let not_held = withdraw_collateral_ix(&owner, &usdt, &SPL_TOKEN, &usdt_token, None, 1, vec![]);
    assert_hodl_error(env.sponsored(not_held, &borrower.key), HodlError::InsufficientCollateral);

    env.blacklist(&owner);
    let blacklisted = withdraw_collateral_ix(&owner, &usdc, &SPL_TOKEN, &token, None, 1, vec![]);
    assert_hodl_error(env.sponsored(blacklisted, &borrower.key), HodlError::Blacklisted);
}

#[test]
fn with_loans_the_position_must_stay_healthy() {
    // 1,000 USDC backing 500,000 cNGN of debt ($312.8125 at the NGN ask).
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    let (usdc, cngn) = (setup.usdc, setup.cngn);
    env.take_loan(&setup.borrower, &setup, 500_000 * ONE_CNGN, 365 * DAY).unwrap();
    let token = env.create_token_account(&usdc, &owner);
    let withdraw = |amount, prices| withdraw_collateral_ix(&owner, &usdc, &SPL_TOKEN, &token, Some(&cngn), amount, prices);
    let key = &setup.borrower.key;

    // The market and prices are required while loans are active.
    let no_market = withdraw_collateral_ix(&owner, &usdc, &SPL_TOKEN, &token, None, ONE_USDC, price_pairs(&[usdc]));
    assert_hodl_error(env.sponsored(no_market, key), HodlError::PriceAccountMismatch);
    assert_hodl_error(env.sponsored(withdraw(ONE_USDC, vec![]), key), HodlError::PriceAccountMismatch);

    // 447 USDC × 70% = $312.90 covers the debt; 446 USDC ($312.20) does not.
    env.sponsored(withdraw(553 * ONE_USDC, price_pairs(&[usdc])), key).unwrap();
    assert_hodl_error(env.sponsored(withdraw(ONE_USDC, price_pairs(&[usdc])), key), HodlError::Unhealthy);
    // Withdrawing everything leaves no used slot to price, and no collateral value.
    assert_hodl_error(env.sponsored(withdraw(447 * ONE_USDC, vec![]), key), HodlError::Unhealthy);

    // Stale prices block withdrawals while loans are active.
    env.warp_seconds(61);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    assert_hodl_error(env.sponsored(withdraw(1, price_pairs(&[usdc])), key), HodlError::StalePrice);
}

#[test]
fn pairs_cover_the_slots_left_after_withdrawal() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    let sol = env.list_spl_collateral(9);
    env.set_pyth_price(&sol, 150 * ONE_DOLLAR, 0);
    env.deposit_collateral(&setup.borrower, &sol, 2_000_000_000);
    env.take_loan(&setup.borrower, &setup, 100_000 * ONE_CNGN, 365 * DAY).unwrap();
    let token = env.create_token_account(&sol, &owner);
    let key = &setup.borrower.key;

    // Emptying the SOL slot leaves only USDC to price.
    let with_sol = withdraw_collateral_ix(&owner, &sol, &SPL_TOKEN, &token, Some(&setup.cngn), 2_000_000_000, price_pairs(&[setup.usdc, sol]));
    assert_hodl_error(env.sponsored(with_sol, key), HodlError::PriceAccountMismatch);
    let usdc_only = withdraw_collateral_ix(&owner, &sol, &SPL_TOKEN, &token, Some(&setup.cngn), 2_000_000_000, price_pairs(&[setup.usdc]));
    env.sponsored(usdc_only, key).unwrap();
    assert_eq!(env.token_balance(&token), 2_000_000_000);
    assert_eq!(env.collateral(&sol).total_deposited, 0);
}

#[test]
fn withdrawal_checks_the_borrowed_market() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 365 * DAY).unwrap();
    let other = env.create_mint(MintKind::CngnLike, 6);
    let create = create_market_ix(&env.admin.pubkey(), &other, &TOKEN_2022, default_market_params());
    send(&mut env.svm, &[create], &[&env.admin]).unwrap();

    let token = env.create_token_account(&setup.usdc, &owner);
    let wrong_market = withdraw_collateral_ix(&owner, &setup.usdc, &SPL_TOKEN, &token, Some(&other), ONE_USDC, price_pairs(&[setup.usdc]));
    assert_hodl_error(env.sponsored(wrong_market, &setup.borrower.key), HodlError::MarketMismatch);
}
