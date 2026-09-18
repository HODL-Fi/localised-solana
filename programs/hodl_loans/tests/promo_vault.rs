mod common;

use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

const FUNDING: u64 = 1_000_000 * ONE_CNGN;

#[test]
fn a_promo_vault_holds_cngn_the_admin_funds_and_can_take_back() {
    let (mut env, cngn) = Env::with_promo_vault(0);
    let admin = env.admin.pubkey();
    let vault = env.promo_vault(&cngn);
    assert_eq!((vault.market, vault.vault), (market_pda(&cngn), promo_vault_token_pda(&cngn)));
    assert_eq!((vault.cash, vault.outstanding, vault.unissued), (0, 0, 0));

    let source = env.create_token_account(&cngn, &admin);
    env.mint_to(&cngn, &source, FUNDING);
    send(&mut env.svm, &[fund_promo_vault_ix(&admin, &cngn, &source, FUNDING)], &[&env.admin]).unwrap();
    assert_eq!(env.promo_vault(&cngn).cash, FUNDING);
    assert_eq!(env.token_balance(&promo_vault_token_pda(&cngn)), FUNDING);

    // Nothing is committed yet, so all of it is free to withdraw.
    let destination = env.treasury_token(&cngn);
    let withdraw = withdraw_promo_vault_ix(&admin, &cngn, &destination, FUNDING);
    send(&mut env.svm, &[withdraw], &[&env.admin]).unwrap();
    assert_eq!(env.promo_vault(&cngn).cash, 0);
    assert_eq!(env.token_balance(&destination), FUNDING);
}

#[test]
fn promo_vault_rejections() {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();
    let stranger = env.funded_keypair();
    let destination = env.treasury_token(&cngn);

    // Creating it twice fails: the PDA already exists.
    let again = create_promo_vault_ix(&admin, &cngn);
    assert!(send(&mut env.svm, &[again], &[&env.admin]).is_err());

    // Every promo vault instruction is admin-only. `create_promo_vault` needs a market that
    // doesn't already have one, so `init` doesn't fail on "already in use" before the
    // authorization check ever runs.
    let other_mint = env.create_mint(MintKind::CngnLike, 6);
    let create_market = create_market_ix(&admin, &other_mint, &TOKEN_2022, default_market_params());
    send(&mut env.svm, &[create_market], &[&env.admin]).expect("create market");
    let by_stranger = create_promo_vault_ix(&stranger.pubkey(), &other_mint);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);
    let source = env.create_token_account(&cngn, &stranger.pubkey());
    let by_stranger = fund_promo_vault_ix(&stranger.pubkey(), &cngn, &source, ONE_CNGN);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);
    let by_stranger = withdraw_promo_vault_ix(&stranger.pubkey(), &cngn, &destination, ONE_CNGN);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);
    let by_stranger = sweep_promo_excess_ix(&stranger.pubkey(), &cngn, &destination);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);

    // Zero moves nothing.
    let zero = fund_promo_vault_ix(&admin, &cngn, &source, 0);
    assert_hodl_error(send(&mut env.svm, &[zero], &[&env.admin]), HodlError::AmountTooSmall);
    let zero = withdraw_promo_vault_ix(&admin, &cngn, &destination, 0);
    assert_hodl_error(send(&mut env.svm, &[zero], &[&env.admin]), HodlError::AmountTooSmall);

    // More than the vault holds fails on our own accounting, before the token program sees it.
    let too_much = withdraw_promo_vault_ix(&admin, &cngn, &destination, FUNDING + 1);
    assert_hodl_error(send(&mut env.svm, &[too_much], &[&env.admin]), HodlError::PromoVaultInsufficient);

    // The destination must belong to the treasury.
    let wrong = env.create_token_account(&cngn, &stranger.pubkey());
    let to_stranger = withdraw_promo_vault_ix(&admin, &cngn, &wrong, ONE_CNGN);
    assert!(send(&mut env.svm, &[to_stranger], &[&env.admin]).is_err());
}

#[test]
fn committed_promo_cannot_be_withdrawn() {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();
    let destination = env.treasury_token(&cngn);

    // Simulate a campaign's reservation and a position's balance against the same cash.
    let mut vault = env.promo_vault(&cngn);
    vault.unissued = 400_000 * ONE_CNGN;
    vault.outstanding = 100_000 * ONE_CNGN;
    env.write(&promo_vault_pda(&cngn), &vault);
    assert_eq!(env.promo_vault(&cngn).free().unwrap(), 500_000 * ONE_CNGN);

    let over = withdraw_promo_vault_ix(&admin, &cngn, &destination, 500_000 * ONE_CNGN + 1);
    assert_hodl_error(send(&mut env.svm, &[over], &[&env.admin]), HodlError::PromoVaultInsufficient);
    let exact = withdraw_promo_vault_ix(&admin, &cngn, &destination, 500_000 * ONE_CNGN);
    send(&mut env.svm, &[exact], &[&env.admin]).unwrap();
    assert_eq!(env.promo_vault(&cngn).cash, 500_000 * ONE_CNGN);
}

#[test]
fn the_promo_sweep_moves_only_donations() {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();
    let destination = env.treasury_token(&cngn);

    // Nothing donated yet.
    let sweep = sweep_promo_excess_ix(&admin, &cngn, &destination);
    assert_hodl_error(send(&mut env.svm, std::slice::from_ref(&sweep), &[&env.admin]), HodlError::AmountTooSmall);

    // A direct transfer to the vault is not promo backing until it is swept.
    env.mint_to(&cngn, &promo_vault_token_pda(&cngn), 7);
    assert_eq!(env.promo_vault(&cngn).cash, FUNDING);
    send(&mut env.svm, &[sweep], &[&env.admin]).unwrap();
    assert_eq!(env.token_balance(&destination), 7);
    assert_eq!(env.promo_vault(&cngn).cash, FUNDING);
    assert_eq!(env.token_balance(&promo_vault_token_pda(&cngn)), FUNDING);
}
