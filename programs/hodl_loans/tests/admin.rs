mod common;

use anchor_lang::prelude::Pubkey;
use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

#[test]
fn upgrade_authority_initializes_config() {
    let mut env = Env::new();
    env.initialize().unwrap();
    let config = env.config();
    assert_eq!(config.version, 1);
    assert_eq!(config.admin, env.admin.pubkey());
    assert_eq!(config.pending_admin, None);
    assert_eq!(config.guardian, env.guardian.pubkey());
    assert_eq!(config.whitelister, env.whitelister.pubkey());
    assert_eq!(config.promo_signer, env.promo_signer.pubkey());
    assert_eq!(config.treasury, env.treasury.pubkey());
    assert_eq!(config.promo_cap_bps, 2_000);
    assert_eq!(config.collateral_count, 0);
}

#[test]
fn only_upgrade_authority_can_initialize() {
    let mut env = Env::new();
    let stranger = env.funded_keypair();
    let instruction = env.initialize_ix(&stranger.pubkey());
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&stranger]), HodlError::Unauthorized);
}

#[test]
fn initialize_cannot_run_twice() {
    let mut env = Env::initialized();
    assert!(env.initialize().is_err());
}

#[test]
fn admin_transfer_is_two_step() {
    let mut env = Env::initialized();
    let new_admin = env.funded_keypair();

    let propose = ix(hodl_loans::instruction::ProposeAdmin { proposed: new_admin.pubkey() }, admin_config_accounts(&env.admin.pubkey()));
    send(&mut env.svm, &[propose], &[&env.admin]).unwrap();
    assert_eq!(env.config().pending_admin, Some(new_admin.pubkey()));
    assert_eq!(env.config().admin, env.admin.pubkey());

    let accept = ix(
        hodl_loans::instruction::AcceptAdmin {},
        hodl_loans::accounts::AcceptAdmin { new_admin: new_admin.pubkey(), config: config_pda() },
    );
    send(&mut env.svm, &[accept], &[&new_admin]).unwrap();
    assert_eq!(env.config().admin, new_admin.pubkey());
    assert_eq!(env.config().pending_admin, None);

    let old_admin_propose = ix(hodl_loans::instruction::ProposeAdmin { proposed: Pubkey::new_unique() }, admin_config_accounts(&env.admin.pubkey()));
    assert_hodl_error(send(&mut env.svm, &[old_admin_propose], &[&env.admin]), HodlError::Unauthorized);
}

#[test]
fn only_proposed_admin_can_accept() {
    let mut env = Env::initialized();
    let proposed = Pubkey::new_unique();
    let propose = ix(hodl_loans::instruction::ProposeAdmin { proposed }, admin_config_accounts(&env.admin.pubkey()));
    send(&mut env.svm, &[propose], &[&env.admin]).unwrap();

    let imposter = env.funded_keypair();
    let accept = ix(
        hodl_loans::instruction::AcceptAdmin {},
        hodl_loans::accounts::AcceptAdmin { new_admin: imposter.pubkey(), config: config_pda() },
    );
    assert_hodl_error(send(&mut env.svm, &[accept], &[&imposter]), HodlError::Unauthorized);
}

#[test]
fn admin_sets_roles_and_others_cannot() {
    let mut env = Env::initialized();
    let (g, w, p, t) = (Pubkey::new_unique(), Pubkey::new_unique(), Pubkey::new_unique(), Pubkey::new_unique());
    let admin = env.admin.pubkey();
    let ixs = vec![
        ix(hodl_loans::instruction::SetGuardian { guardian: g }, admin_config_accounts(&admin)),
        ix(hodl_loans::instruction::SetWhitelister { whitelister: w }, admin_config_accounts(&admin)),
        ix(hodl_loans::instruction::SetPromoSigner { promo_signer: p }, admin_config_accounts(&admin)),
        ix(hodl_loans::instruction::SetTreasury { treasury: t }, admin_config_accounts(&admin)),
    ];
    send(&mut env.svm, &ixs, &[&env.admin]).unwrap();
    let config = env.config();
    assert_eq!((config.guardian, config.whitelister, config.promo_signer, config.treasury), (g, w, p, t));

    let stranger = env.funded_keypair();
    let attempt = ix(hodl_loans::instruction::SetGuardian { guardian: stranger.pubkey() }, admin_config_accounts(&stranger.pubkey()));
    assert_hodl_error(send(&mut env.svm, &[attempt], &[&stranger]), HodlError::Unauthorized);
}
