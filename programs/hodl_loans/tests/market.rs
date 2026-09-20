mod common;

use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

#[test]
fn admin_creates_cngn_market() {
    let (env, mint) = Env::with_cngn_market();
    let market = env.market(&mint);
    let params = default_market_params();
    assert_eq!(market.version, 1);
    assert_eq!(market.mint, mint);
    assert_eq!(market.vault, market_vault_pda(&mint));
    assert_eq!(market.token_program, TOKEN_2022);
    assert_eq!(market.decimals, 6);
    assert_eq!(market.params(), params);
    assert_eq!(market.last_accrual_ts, env.now());
    assert_eq!((market.cash, market.total_borrows, market.total_shares), (0, 0, 0));
    assert!(!market.paused);
    assert_eq!(env.token_owner(&market.vault), market_pda(&mint));
}

#[test]
fn classic_spl_mint_is_accepted() {
    let mut env = Env::initialized();
    let mint = env.create_mint(MintKind::SplToken, 6);
    let instruction = create_market_ix(&env.admin.pubkey(), &mint, &SPL_TOKEN, default_market_params());
    send(&mut env.svm, &[instruction], &[&env.admin]).unwrap();
    assert_eq!(env.market(&mint).token_program, SPL_TOKEN);
}

#[test]
fn transfer_fee_mint_is_rejected() {
    let mut env = Env::initialized();
    let mint = env.create_mint(MintKind::TransferFee, 6);
    let instruction = create_market_ix(&env.admin.pubkey(), &mint, &TOKEN_2022, default_market_params());
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::UnsupportedMintExtension);
}

#[test]
fn invalid_params_are_rejected() {
    let mut env = Env::initialized();
    let mint = env.create_mint(MintKind::CngnLike, 6);
    let admin = env.admin.pubkey();

    let mut params = default_market_params();
    params.reserve_factor_bps = 10_001;
    let instruction = create_market_ix(&admin, &mint, &TOKEN_2022, params);
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::InvalidParameters);

    let mut params = default_market_params();
    params.max_utilization_bps = 10_001;
    let instruction = create_market_ix(&admin, &mint, &TOKEN_2022, params);
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::InvalidParameters);

    let mut params = default_market_params();
    params.max_tenure_seconds = 86_399;
    let instruction = create_market_ix(&admin, &mint, &TOKEN_2022, params);
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::InvalidParameters);

    // The NGN feed prices the debt side of every health check, so it may not drift further
    // behind than the collateral feeds do. 150 slots is the same 60 seconds
    // `MAX_PRICE_AGE_SECONDS` allows them, at a 400 ms slot.
    let mut params = default_market_params();
    params.ngn_max_stale_slots = 151;
    let instruction = create_market_ix(&admin, &mint, &TOKEN_2022, params);
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::InvalidParameters);

    let mut params = default_market_params();
    params.ngn_max_stale_slots = 150;
    let instruction = create_market_ix(&admin, &mint, &TOKEN_2022, params);
    send(&mut env.svm, &[instruction], &[&env.admin]).expect("150 slots is the bound, not past it");
}

#[test]
fn only_admin_creates_markets() {
    let mut env = Env::initialized();
    let mint = env.create_mint(MintKind::CngnLike, 6);
    let stranger = env.funded_keypair();
    let instruction = create_market_ix(&stranger.pubkey(), &mint, &TOKEN_2022, default_market_params());
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&stranger]), HodlError::Unauthorized);
}

#[test]
fn admin_updates_params() {
    let (mut env, mint) = Env::with_cngn_market();
    let mut params = default_market_params();
    params.interest_rate_bps = 2_000;
    params.min_loan_amount = 5_000 * ONE_CNGN;
    let instruction = update_market_params_ix(&env.admin.pubkey(), &mint, params);
    send(&mut env.svm, &[instruction], &[&env.admin]).unwrap();
    assert_eq!(env.market(&mint).params(), params);

    let stranger = env.funded_keypair();
    let attempt = update_market_params_ix(&stranger.pubkey(), &mint, params);
    assert_hodl_error(send(&mut env.svm, &[attempt], &[&stranger]), HodlError::Unauthorized);

    params.reserve_factor_bps = 10_001;
    let invalid = update_market_params_ix(&env.admin.pubkey(), &mint, params);
    assert_hodl_error(send(&mut env.svm, &[invalid], &[&env.admin]), HodlError::InvalidParameters);
}

#[test]
fn guardian_pauses_only_admin_unpauses() {
    let (mut env, mint) = Env::with_cngn_market();

    let pause = set_market_paused_ix(&env.guardian.pubkey(), &mint, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();
    assert!(env.market(&mint).paused);

    let guardian_unpause = set_market_paused_ix(&env.guardian.pubkey(), &mint, false);
    assert_hodl_error(send(&mut env.svm, &[guardian_unpause], &[&env.guardian]), HodlError::Unauthorized);

    let admin_unpause = set_market_paused_ix(&env.admin.pubkey(), &mint, false);
    send(&mut env.svm, &[admin_unpause], &[&env.admin]).unwrap();
    assert!(!env.market(&mint).paused);

    let stranger = env.funded_keypair();
    let stranger_pause = set_market_paused_ix(&stranger.pubkey(), &mint, true);
    assert_hodl_error(send(&mut env.svm, &[stranger_pause], &[&stranger]), HodlError::Unauthorized);
}

#[test]
fn rate_bounds_and_price_feed_limits_are_validated() {
    let mut env = Env::initialized();
    let mint = env.create_mint(MintKind::CngnLike, 6);
    let admin = env.admin.pubkey();
    let base = default_market_params();

    let invalid = [
        hodl_loans::MarketParams { interest_rate_bps: 10_001, ..base },
        hodl_loans::MarketParams { penalty_rate_bps: 10_001, ..base },
        hodl_loans::MarketParams { ngn_feed: anchor_lang::prelude::Pubkey::default(), ..base },
        hodl_loans::MarketParams { ngn_max_stale_slots: 0, ..base },
        hodl_loans::MarketParams { ngn_min_samples: 0, ..base },
        hodl_loans::MarketParams { ngn_max_spread_bps: 10_001, ..base },
        hodl_loans::MarketParams { promo_inactivity_seconds: -1, ..base },
        // `0` would let promo expire in the same slot as the redemption that granted it.
        hodl_loans::MarketParams { promo_inactivity_seconds: 0, ..base },
        hodl_loans::MarketParams { bad_debt_dust_usd: hodl_loans::MAX_BAD_DEBT_DUST_USD + 1, ..base },
    ];
    for params in invalid {
        let instruction = create_market_ix(&admin, &mint, &TOKEN_2022, params);
        assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::InvalidParameters);
    }

    let boundaries = hodl_loans::MarketParams {
        interest_rate_bps: 10_000,
        penalty_rate_bps: 10_000,
        ngn_max_stale_slots: 1,
        ngn_min_samples: 1,
        ngn_max_spread_bps: 10_000,
        promo_inactivity_seconds: 1,
        bad_debt_dust_usd: hodl_loans::MAX_BAD_DEBT_DUST_USD,
        ..base
    };
    let instruction = create_market_ix(&admin, &mint, &TOKEN_2022, boundaries);
    send(&mut env.svm, &[instruction], &[&env.admin]).unwrap();
    assert_eq!(env.market(&mint).params(), boundaries);
}
