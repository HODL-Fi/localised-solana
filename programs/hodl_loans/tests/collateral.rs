mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use common::*;
use hodl_loans::{CollateralKind, CollateralParams, HodlError};
use solana_signer::Signer;

#[test]
fn admin_lists_spl_token_collateral() {
    let mut env = Env::initialized();
    let usdc = env.list_spl_collateral(6);

    let asset = env.collateral(&usdc);
    assert_eq!(asset.version, 1);
    assert_eq!(asset.mint, usdc);
    assert_eq!(asset.token_program, SPL_TOKEN);
    assert_eq!(asset.vault, collateral_vault_pda(&usdc));
    assert_eq!(asset.decimals, 6);
    assert_eq!(asset.kind, CollateralKind::Standard);
    assert_eq!(asset.params(), default_collateral_params(&usdc));
    assert_eq!(asset.total_deposited, 0);
    assert!(!asset.paused);
    assert_eq!(env.token_owner(&asset.vault), collateral_pda(&usdc));
    assert_eq!(env.config().collateral_count, 1);

    env.list_spl_collateral(9);
    assert_eq!(env.config().collateral_count, 2);
}

#[test]
fn listing_is_admin_only_and_once_per_mint() {
    let mut env = Env::initialized();
    let mint = env.create_mint(MintKind::SplToken, 6);
    let stranger = env.funded_keypair();
    let by_stranger = list_collateral_ix(&stranger.pubkey(), &mint, &SPL_TOKEN, default_collateral_params(&mint), hodl_loans::CollateralKind::Standard);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);

    let listing = list_collateral_ix(&env.admin.pubkey(), &mint, &SPL_TOKEN, default_collateral_params(&mint), hodl_loans::CollateralKind::Standard);
    send(&mut env.svm, std::slice::from_ref(&listing), &[&env.admin]).unwrap();
    assert!(send(&mut env.svm, &[listing], &[&env.admin]).is_err());
    assert_eq!(env.config().collateral_count, 1);
}

#[test]
fn mints_with_non_metadata_extensions_are_rejected() {
    let mut env = Env::initialized();
    let admin = env.admin.pubkey();
    for kind in [MintKind::TransferFee, MintKind::CngnLike] {
        let mint = env.create_mint(kind, 6);
        let instruction = list_collateral_ix(&admin, &mint, &TOKEN_2022, default_collateral_params(&mint), hodl_loans::CollateralKind::Standard);
        assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::UnsupportedMintExtension);
    }
}

#[test]
fn collateral_parameter_rules() {
    let mut env = Env::initialized();
    let mint = env.list_spl_collateral(6);
    let admin = env.admin.pubkey();
    let base = default_collateral_params(&mint);

    let invalid = [
        CollateralParams { pyth_feed_id: [0; 32], ..base },
        CollateralParams { max_price_age_seconds: 0, ..base },
        CollateralParams { max_conf_bps: 10_001, ..base },
        // Above the 60-second cap on how far a caller may shop for a price.
        CollateralParams { max_price_age_seconds: 61, ..base },
        CollateralParams { ltv_bps: 999, liquidation_threshold_bps: 5_000, ..base },
        // LTV 7,500 + promo cap 2,000 > threshold 9,000
        CollateralParams { ltv_bps: 7_500, ..base },
        // threshold 9,000 × (10,000 + 1,112) > 10,000²
        CollateralParams { liquidation_bonus_bps: 1_112, ..base },
    ];
    for params in invalid {
        let instruction = update_collateral_params_ix(&admin, &mint, params);
        assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::InvalidParameters);
    }
    // `list_collateral` applies the same rules.
    let other = env.create_mint(MintKind::SplToken, 6);
    let bad_listing = CollateralParams { ltv_bps: 7_001, ..default_collateral_params(&other) };
    let instruction = list_collateral_ix(&admin, &other, &SPL_TOKEN, bad_listing, hodl_loans::CollateralKind::Standard);
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::InvalidParameters);

    // Every rule's boundary is accepted.
    let boundaries = [
        CollateralParams { ltv_bps: 1_000, liquidation_threshold_bps: 3_000, ..base },
        CollateralParams { liquidation_bonus_bps: 1_111, ..base },
        CollateralParams { max_conf_bps: 10_000, max_price_age_seconds: 60, ..base },
        // Unpinned: any verified update for the feed inside the window is accepted.
        CollateralParams { price_account: anchor_lang::prelude::Pubkey::default(), ..base },
    ];
    for params in boundaries {
        let instruction = update_collateral_params_ix(&admin, &mint, params);
        send(&mut env.svm, &[instruction], &[&env.admin]).unwrap();
        assert_eq!(env.collateral(&mint).params(), params);
    }
}

#[test]
fn only_admin_updates_parameters() {
    let mut env = Env::initialized();
    let mint = env.list_spl_collateral(6);
    let params = CollateralParams { deposit_cap: 5 * ONE_USDC, ..default_collateral_params(&mint) };

    let guardian = env.guardian.pubkey();
    let by_guardian = update_collateral_params_ix(&guardian, &mint, params);
    assert_hodl_error(send(&mut env.svm, &[by_guardian], &[&env.guardian]), HodlError::Unauthorized);

    let by_admin = update_collateral_params_ix(&env.admin.pubkey(), &mint, params);
    send(&mut env.svm, &[by_admin], &[&env.admin]).unwrap();
    assert_eq!(env.collateral(&mint).deposit_cap, 5 * ONE_USDC);
}

#[test]
fn guardian_pauses_and_only_admin_unpauses() {
    let mut env = Env::initialized();
    let mint = env.list_spl_collateral(6);
    let guardian = env.guardian.pubkey();
    let admin = env.admin.pubkey();

    let pause = set_collateral_paused_ix(&guardian, &mint, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();
    assert!(env.collateral(&mint).paused);

    let guardian_unpause = set_collateral_paused_ix(&guardian, &mint, false);
    assert_hodl_error(send(&mut env.svm, &[guardian_unpause], &[&env.guardian]), HodlError::Unauthorized);

    let stranger = env.funded_keypair();
    let stranger_pause = set_collateral_paused_ix(&stranger.pubkey(), &mint, true);
    assert_hodl_error(send(&mut env.svm, &[stranger_pause], &[&stranger]), HodlError::Unauthorized);

    let unpause = set_collateral_paused_ix(&admin, &mint, false);
    send(&mut env.svm, &[unpause], &[&env.admin]).unwrap();
    assert!(!env.collateral(&mint).paused);
}

#[test]
fn delist_closes_an_unused_asset() {
    let mut env = Env::initialized();
    let mint = env.list_spl_collateral(6);
    let admin = env.admin.pubkey();

    let stranger = env.funded_keypair();
    let by_stranger = delist_collateral_ix(&stranger.pubkey(), &mint, &SPL_TOKEN);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);

    let delist = delist_collateral_ix(&admin, &mint, &SPL_TOKEN);
    send(&mut env.svm, &[delist], &[&env.admin]).unwrap();
    assert!(env.svm.get_account(&collateral_pda(&mint)).is_none_or(|a| a.lamports == 0));
    assert!(env.svm.get_account(&collateral_vault_pda(&mint)).is_none_or(|a| a.lamports == 0));
    assert_eq!(env.config().collateral_count, 0);

    // The mint can be listed again.
    let relist = list_collateral_ix(&admin, &mint, &SPL_TOKEN, default_collateral_params(&mint), hodl_loans::CollateralKind::Standard);
    send(&mut env.svm, &[relist], &[&env.admin]).unwrap();
}

#[test]
fn delist_requires_no_deposits_and_an_empty_vault() {
    let mut env = Env::initialized();
    let mint = env.list_spl_collateral(6);
    let admin = env.admin.pubkey();
    let delist = delist_collateral_ix(&admin, &mint, &SPL_TOKEN);

    // Recorded deposits block delisting (simulated; deposits arrive in Task 5).
    let mut asset = env.collateral(&mint);
    asset.total_deposited = 1;
    env.write(&collateral_pda(&mint), &asset);
    assert_hodl_error(send(&mut env.svm, std::slice::from_ref(&delist), &[&env.admin]), HodlError::CollateralStillInUse);
    asset.total_deposited = 0;
    env.write(&collateral_pda(&mint), &asset);

    // So does a donation, until it is swept to the treasury.
    env.mint_to(&mint, &collateral_vault_pda(&mint), 7);
    assert_hodl_error(send(&mut env.svm, std::slice::from_ref(&delist), &[&env.admin]), HodlError::CollateralStillInUse);
    let treasury = env.treasury.pubkey();
    let destination = env.create_token_account(&mint, &treasury);
    let sweep = sweep_collateral_excess_ix(&admin, &mint, &SPL_TOKEN, &destination);
    send(&mut env.svm, &[sweep], &[&env.admin]).unwrap();
    assert_eq!(env.token_balance(&destination), 7);
    send(&mut env.svm, &[delist], &[&env.admin]).unwrap();
}

#[test]
fn collateral_sweep_moves_only_donations() {
    let mut env = Env::initialized();
    let mint = env.list_spl_collateral(6);
    let admin = env.admin.pubkey();
    let vault = collateral_vault_pda(&mint);
    let treasury = env.treasury.pubkey();
    let destination = env.create_token_account(&mint, &treasury);

    // Nothing to sweep yet.
    let sweep = sweep_collateral_excess_ix(&admin, &mint, &SPL_TOKEN, &destination);
    assert_hodl_error(send(&mut env.svm, std::slice::from_ref(&sweep), &[&env.admin]), HodlError::AmountTooSmall);

    // 40 recorded as deposited (simulated) plus a 60 donation: only the 60 moves.
    env.mint_to(&mint, &vault, 100);
    let mut asset = env.collateral(&mint);
    asset.total_deposited = 40;
    env.write(&collateral_pda(&mint), &asset);
    send(&mut env.svm, &[sweep], &[&env.admin]).unwrap();
    assert_eq!(env.token_balance(&destination), 60);
    assert_eq!(env.token_balance(&vault), 40);

    // Only the admin sweeps, and only to a treasury-owned account.
    env.mint_to(&mint, &vault, 5);
    let stranger = env.funded_keypair();
    let wrong_destination = env.create_token_account(&mint, &stranger.pubkey());
    let to_stranger = sweep_collateral_excess_ix(&admin, &mint, &SPL_TOKEN, &wrong_destination);
    assert_anchor_error(send(&mut env.svm, &[to_stranger], &[&env.admin]), AnchorError::ConstraintTokenOwner);
    let by_stranger = sweep_collateral_excess_ix(&stranger.pubkey(), &mint, &SPL_TOKEN, &destination);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);
}

#[test]
fn a_mint_with_too_many_decimals_cannot_be_listed() {
    // The bound comes from the LIQUIDATION path, not the health path. `token_value` copes with
    // roughly 38 decimals; `seize_for_repayment` multiplies twice and, against the worst
    // repayment the program permits, first overflows at 15. Past the bound a position holding
    // the asset could be opened and then never liquidated, so listing is refused instead.
    let mut env = Env::initialized();
    let admin = env.admin.pubkey();

    let too_wide = env.create_mint(MintKind::SplToken, 13);
    let instruction = list_collateral_ix(
        &admin,
        &too_wide,
        &SPL_TOKEN,
        default_collateral_params(&too_wide),
        CollateralKind::Standard,
    );
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::InvalidParameters);
    assert_eq!(env.config().collateral_count, 0);

    // The bound itself is allowed, and so is every decimals count the protocol actually uses.
    env.list_spl_collateral(12);
    env.list_spl_collateral(9);
    env.list_spl_collateral(6);
    assert_eq!(env.config().collateral_count, 3);
}
