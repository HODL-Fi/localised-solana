mod common;

use anchor_lang::prelude::Pubkey;
use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

#[test]
fn whitelister_and_admin_can_whitelist() {
    let mut env = Env::initialized();
    let (a, b) = (Pubkey::new_unique(), Pubkey::new_unique());
    env.whitelist(&a);
    let by_admin = whitelist_ix(&env.admin.pubkey(), &b);
    send(&mut env.svm, &[by_admin], &[&env.admin]).unwrap();

    for wallet in [a, b] {
        let access = env.access(&wallet);
        assert_eq!(access.version, 1);
        assert_eq!(access.wallet, wallet);
        assert!(access.whitelisted);
        assert!(!access.blacklisted);
    }
}

#[test]
fn others_cannot_whitelist() {
    let mut env = Env::initialized();
    let stranger = env.funded_keypair();
    let attempt = whitelist_ix(&stranger.pubkey(), &stranger.pubkey());
    assert_hodl_error(send(&mut env.svm, &[attempt], &[&stranger]), HodlError::Unauthorized);
}

#[test]
fn blacklist_clears_whitelist_and_blocks_readmission() {
    let mut env = Env::initialized();
    let wallet = Pubkey::new_unique();
    env.whitelist(&wallet);
    env.blacklist(&wallet);
    let access = env.access(&wallet);
    assert!(!access.whitelisted);
    assert!(access.blacklisted);

    let by_whitelister = whitelist_ix(&env.whitelister.pubkey(), &wallet);
    assert_hodl_error(send(&mut env.svm, &[by_whitelister], &[&env.whitelister]), HodlError::Blacklisted);
    let by_admin = whitelist_ix(&env.admin.pubkey(), &wallet);
    assert_hodl_error(send(&mut env.svm, &[by_admin], &[&env.admin]), HodlError::Blacklisted);
}

#[test]
fn unblacklist_does_not_rewhitelist() {
    let mut env = Env::initialized();
    let wallet = Pubkey::new_unique();
    env.whitelist(&wallet);
    env.blacklist(&wallet);
    let clear = unblacklist_ix(&env.admin.pubkey(), &wallet);
    send(&mut env.svm, &[clear], &[&env.admin]).unwrap();
    let access = env.access(&wallet);
    assert!(!access.blacklisted);
    assert!(!access.whitelisted);

    env.whitelist(&wallet);
    assert!(env.access(&wallet).whitelisted);
}

#[test]
fn only_admin_can_blacklist_or_unblacklist() {
    let mut env = Env::initialized();
    let wallet = Pubkey::new_unique();
    env.whitelist(&wallet);
    let by_whitelister = blacklist_ix(&env.whitelister.pubkey(), &wallet);
    assert_hodl_error(send(&mut env.svm, &[by_whitelister], &[&env.whitelister]), HodlError::Unauthorized);

    env.blacklist(&wallet);
    let clear = unblacklist_ix(&env.whitelister.pubkey(), &wallet);
    assert_hodl_error(send(&mut env.svm, &[clear], &[&env.whitelister]), HodlError::Unauthorized);
}

#[test]
fn blacklisting_an_unknown_wallet_creates_its_access_account() {
    let mut env = Env::initialized();
    let wallet = Pubkey::new_unique();
    env.blacklist(&wallet);
    let access = env.access(&wallet);
    assert!(access.blacklisted);
    assert!(!access.whitelisted);
}
