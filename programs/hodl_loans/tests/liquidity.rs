mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use anchor_lang::prelude::Pubkey;
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
fn second_deposit_by_same_lender_adds_shares() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, 2 * ONE_CNGN);
    env.deposit(&lender, &mint, ONE_CNGN).unwrap();

    let key = lender_pda(&market_pda(&mint), &lender.key.pubkey());
    let before: hodl_loans::LenderPosition = env.fetch(&key);

    env.deposit(&lender, &mint, ONE_CNGN / 2).unwrap();

    let after: hodl_loans::LenderPosition = env.fetch(&key);
    assert_eq!(after.shares, 1_000_000_000 + 500_000_000);
    assert_eq!(after.version, before.version);
    assert_eq!(after.bump, before.bump);
    assert_eq!(after.market, before.market);
    assert_eq!(after.owner, before.owner);
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

// ---- withdraw_liquidity (Task 8) ----

#[test]
fn partial_withdraw_burns_shares_and_returns_tokens() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, ONE_CNGN);
    env.deposit(&lender, &mint, ONE_CNGN).unwrap();

    env.withdraw(&lender, &mint, 400_000).unwrap();
    assert_eq!(env.lender_shares(&mint, &lender.key.pubkey()), 600_000_000);
    assert_eq!(env.market(&mint).cash, 600_000);
    assert_eq!(env.token_balance(&lender.token), 400_000);
}

#[test]
fn withdraw_max_takes_everything_available() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, ONE_CNGN);
    env.deposit(&lender, &mint, ONE_CNGN).unwrap();

    env.withdraw(&lender, &mint, u64::MAX).unwrap();
    assert_eq!(env.lender_shares(&mint, &lender.key.pubkey()), 0);
    assert_eq!(env.market(&mint).cash, 0);
    assert_eq!(env.token_balance(&lender.token), ONE_CNGN);
}

#[test]
fn withdrawals_are_limited_to_cash_minus_reserve() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, ONE_CNGN);
    env.deposit(&lender, &mint, ONE_CNGN).unwrap();

    // Simulate 700,000 lent out and a 100,000 protocol reserve.
    let key = market_pda(&mint);
    let mut market = env.market(&mint);
    market.cash = 300_000;
    market.total_borrows = 700_000;
    market.protocol_reserve = 100_000;
    env.write(&key, &market);

    assert_hodl_error(env.withdraw(&lender, &mint, 200_001), HodlError::InsufficientCash);
    env.withdraw(&lender, &mint, u64::MAX).unwrap();
    assert_eq!(env.token_balance(&lender.token), 200_000);
    assert_eq!(env.market(&mint).cash, 100_000);
}

#[test]
fn cannot_withdraw_more_than_own_shares() {
    let (mut env, mint) = Env::with_cngn_market();
    let big = env.new_lender(&mint, ONE_CNGN);
    let small = env.new_lender(&mint, 1_000);
    env.deposit(&big, &mint, ONE_CNGN).unwrap();
    env.deposit(&small, &mint, 1_000).unwrap();
    assert_hodl_error(env.withdraw(&small, &mint, 2_000), HodlError::InsufficientShares);
}

#[test]
fn withdraw_works_while_paused_but_not_when_blacklisted() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, ONE_CNGN);
    env.deposit(&lender, &mint, ONE_CNGN).unwrap();

    let pause = set_market_paused_ix(&env.guardian.pubkey(), &mint, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();
    env.withdraw(&lender, &mint, 100_000).unwrap();

    env.blacklist(&lender.key.pubkey());
    assert_hodl_error(env.withdraw(&lender, &mint, 100_000), HodlError::Blacklisted);
}

#[test]
fn unblacklisted_wallet_needs_rewhitelisting() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, ONE_CNGN);
    env.deposit(&lender, &mint, ONE_CNGN / 2).unwrap();

    env.blacklist(&lender.key.pubkey());
    let unblacklist = unblacklist_ix(&env.admin.pubkey(), &lender.key.pubkey());
    send(&mut env.svm, &[unblacklist], &[&env.admin]).unwrap();
    assert!(!env.access(&lender.key.pubkey()).blacklisted);
    assert!(!env.access(&lender.key.pubkey()).whitelisted);

    assert_hodl_error(env.deposit(&lender, &mint, ONE_CNGN / 2), HodlError::NotWhitelisted);
    assert_hodl_error(env.withdraw(&lender, &mint, 100_000), HodlError::NotWhitelisted);

    env.whitelist(&lender.key.pubkey());
    env.withdraw(&lender, &mint, 100_000).unwrap();
}

// ---- account and zero-path rejections (final review) ----

/// A second cNGN-like market in `env`, distinct from the one `with_cngn_market` created.
fn second_cngn_market(env: &mut Env) -> Pubkey {
    let mint = env.create_mint(MintKind::CngnLike, 6);
    let admin = env.admin.pubkey();
    let instruction = create_market_ix(&admin, &mint, &TOKEN_2022, default_market_params());
    send(&mut env.svm, &[instruction], &[&env.admin]).expect("create second market");
    mint
}

#[test]
fn deposit_with_another_markets_vault_is_rejected() {
    let (mut env, mint) = Env::with_cngn_market();
    let other_mint = second_cngn_market(&mut env);
    let lender = env.new_lender(&mint, ONE_CNGN);

    let accounts = hodl_loans::accounts::DepositLiquidity {
        payer: env.admin.pubkey(),
        owner: lender.key.pubkey(),
        access: access_pda(&lender.key.pubkey()),
        market: market_pda(&mint),
        mint,
        vault: market_vault_pda(&other_mint),
        owner_token: lender.token,
        lender: lender_pda(&market_pda(&mint), &lender.key.pubkey()),
        token_program: TOKEN_2022,
        system_program: anchor_lang::solana_program::system_program::ID,
    };
    let instruction = ix(hodl_loans::instruction::DepositLiquidity { amount: ONE_CNGN }, accounts);
    assert_anchor_error(send(&mut env.svm, &[instruction], &[&env.admin, &lender.key]), AnchorError::ConstraintHasOne);
}

#[test]
fn deposit_with_mint_from_another_market_is_rejected() {
    let (mut env, mint) = Env::with_cngn_market();
    let other_mint = second_cngn_market(&mut env);
    let lender = env.new_lender(&other_mint, ONE_CNGN);

    let accounts = hodl_loans::accounts::DepositLiquidity {
        payer: env.admin.pubkey(),
        owner: lender.key.pubkey(),
        access: access_pda(&lender.key.pubkey()),
        // The `market` account is the FIRST market's PDA; `mint`/`vault` belong to the SECOND
        // market. `market`'s own seeds constraint is derived from the passed-in `mint`, so this
        // is caught as a seeds mismatch on `market` itself (before `has_one` is even reached).
        market: market_pda(&mint),
        mint: other_mint,
        vault: market_vault_pda(&other_mint),
        owner_token: lender.token,
        lender: lender_pda(&market_pda(&mint), &lender.key.pubkey()),
        token_program: TOKEN_2022,
        system_program: anchor_lang::solana_program::system_program::ID,
    };
    let instruction = ix(hodl_loans::instruction::DepositLiquidity { amount: ONE_CNGN }, accounts);
    assert_anchor_error(send(&mut env.svm, &[instruction], &[&env.admin, &lender.key]), AnchorError::ConstraintSeeds);
}

#[test]
fn deposit_with_someone_elses_token_account_is_rejected() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, ONE_CNGN);
    let other = env.new_lender(&mint, ONE_CNGN);

    let instruction = deposit_liquidity_ix(&env.admin.pubkey(), &lender.key.pubkey(), &mint, &other.token, ONE_CNGN);
    assert_anchor_error(send(&mut env.svm, &[instruction], &[&env.admin, &lender.key]), AnchorError::ConstraintTokenOwner);
}

#[test]
fn withdraw_with_another_owners_lender_pda_is_rejected() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender_a = env.new_lender(&mint, ONE_CNGN);
    let lender_b = env.new_lender(&mint, ONE_CNGN);
    env.deposit(&lender_a, &mint, ONE_CNGN).unwrap();
    env.deposit(&lender_b, &mint, ONE_CNGN).unwrap();

    let market = market_pda(&mint);
    let accounts = hodl_loans::accounts::WithdrawLiquidity {
        owner: lender_b.key.pubkey(),
        access: access_pda(&lender_b.key.pubkey()),
        market,
        mint,
        vault: market_vault_pda(&mint),
        owner_token: lender_b.token,
        lender: lender_pda(&market, &lender_a.key.pubkey()),
        token_program: TOKEN_2022,
    };
    let instruction = ix(hodl_loans::instruction::WithdrawLiquidity { amount: 100_000 }, accounts);
    assert_anchor_error(send(&mut env.svm, &[instruction], &[&env.admin, &lender_b.key]), AnchorError::ConstraintSeeds);
}

#[test]
fn deposit_with_zero_total_shares_and_dust_cash_mints_zero_shares() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, ONE_CNGN);

    let key = market_pda(&mint);
    let mut market = env.market(&mint);
    assert_eq!(market.total_shares, 0);
    market.cash = 5_000;
    env.write(&key, &market);

    assert_hodl_error(env.deposit(&lender, &mint, 1), HodlError::ZeroShares);
}

#[test]
fn withdraw_zero_amount_is_rejected() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, ONE_CNGN);
    env.deposit(&lender, &mint, ONE_CNGN).unwrap();

    assert_hodl_error(env.withdraw(&lender, &mint, 0), HodlError::AmountTooSmall);
}

#[test]
fn withdraw_max_with_no_available_cash_is_rejected() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, ONE_CNGN);
    env.deposit(&lender, &mint, ONE_CNGN).unwrap();

    let key = market_pda(&mint);
    let mut market = env.market(&mint);
    market.protocol_reserve = market.cash;
    env.write(&key, &market);

    assert_hodl_error(env.withdraw(&lender, &mint, u64::MAX), HodlError::AmountTooSmall);
}
