mod common;

use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

const FUNDING: u64 = 1_000_000 * ONE_CNGN;
const BUDGET: u64 = 300_000 * ONE_CNGN;

#[test]
fn a_campaign_reserves_budget_out_of_the_vaults_free_cngn() {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    env.create_campaign(&cngn, 1, BUDGET);

    let campaign = env.campaign(&cngn, 1);
    assert_eq!((campaign.campaign_id, campaign.budget, campaign.granted), (1, BUDGET, 0));
    assert!(campaign.active);
    assert_eq!(campaign.market, market_pda(&cngn));

    // The reservation shows up as `unissued`, and the free balance shrinks by exactly it.
    let vault = env.promo_vault(&cngn);
    assert_eq!((vault.cash, vault.unissued, vault.outstanding), (FUNDING, BUDGET, 0));
    assert_eq!(vault.free().unwrap(), FUNDING - BUDGET);

    // A second campaign reserves against what is left, not against the whole vault.
    env.create_campaign(&cngn, 2, FUNDING - BUDGET);
    assert_eq!(env.promo_vault(&cngn).free().unwrap(), 0);
}

#[test]
fn campaign_rejections() {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();
    let stranger = env.funded_keypair();
    let until = env.now() + 86_400;

    let by_stranger = create_campaign_ix(&stranger.pubkey(), &cngn, 1, BUDGET, until);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);

    let zero = create_campaign_ix(&admin, &cngn, 1, 0, until);
    assert_hodl_error(send(&mut env.svm, &[zero], &[&env.admin]), HodlError::AmountTooSmall);

    // A campaign that is already over cannot be created.
    let past = create_campaign_ix(&admin, &cngn, 1, BUDGET, env.now());
    assert_hodl_error(send(&mut env.svm, &[past], &[&env.admin]), HodlError::InvalidParameters);

    // More than the vault holds free.
    let over = create_campaign_ix(&admin, &cngn, 1, FUNDING + 1, until);
    assert_hodl_error(send(&mut env.svm, &[over], &[&env.admin]), HodlError::PromoVaultInsufficient);

    // One account per (market, campaign_id): the same id twice fails to init.
    env.create_campaign(&cngn, 1, BUDGET);
    let again = create_campaign_ix(&admin, &cngn, 1, ONE_CNGN, until);
    assert!(send(&mut env.svm, &[again], &[&env.admin]).is_err());

    // Closing is admin-only, and only once.
    let by_stranger = close_campaign_ix(&stranger.pubkey(), &cngn, 1);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);
    send(&mut env.svm, &[close_campaign_ix(&admin, &cngn, 1)], &[&env.admin]).unwrap();
    let twice = close_campaign_ix(&admin, &cngn, 1);
    assert_hodl_error(send(&mut env.svm, &[twice], &[&env.admin]), HodlError::CampaignInactive);
}

#[test]
fn closing_a_campaign_twice_is_rejected_even_with_a_second_campaign_still_open() {
    // A single-campaign vault drains `unissued` to zero on its first close, so closing it again
    // happens to fail on an underflow in `sub` — the right outcome, for the wrong reason. With a
    // second campaign still holding its own budget in `unissued`, that underflow no longer fires:
    // a second close would silently subtract campaign 1's `unspent` a second time, over-crediting
    // the vault's `free()` at campaign 2's expense. This pins the `active` guard itself.
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();

    let budget_1 = 200_000 * ONE_CNGN;
    let budget_2 = 300_000 * ONE_CNGN;
    env.create_campaign(&cngn, 1, budget_1);
    env.create_campaign(&cngn, 2, budget_2);

    send(&mut env.svm, &[close_campaign_ix(&admin, &cngn, 1)], &[&env.admin]).unwrap();
    assert!(!env.campaign(&cngn, 1).active);
    assert!(env.campaign(&cngn, 2).active);
    // Only campaign 1's reservation came back; campaign 2's is untouched.
    assert_eq!(env.promo_vault(&cngn).unissued, budget_2);

    let twice = close_campaign_ix(&admin, &cngn, 1);
    assert_hodl_error(send(&mut env.svm, &[twice], &[&env.admin]), HodlError::CampaignInactive);
}

#[test]
fn closing_a_campaign_returns_only_what_it_never_granted() {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();
    env.create_campaign(&cngn, 1, BUDGET);

    // Simulate the campaign having issued a third of its budget.
    let granted = 100_000 * ONE_CNGN;
    let mut campaign = env.campaign(&cngn, 1);
    campaign.granted = granted;
    env.write(&campaign_pda(&cngn, 1), &campaign);
    let mut vault = env.promo_vault(&cngn);
    vault.unissued -= granted;
    vault.outstanding += granted;
    env.write(&promo_vault_pda(&cngn), &vault);

    send(&mut env.svm, &[close_campaign_ix(&admin, &cngn, 1)], &[&env.admin]).unwrap();

    // The granted third stays committed as `outstanding`; only the rest returns to free.
    let vault = env.promo_vault(&cngn);
    assert_eq!((vault.unissued, vault.outstanding, vault.cash), (0, granted, FUNDING));
    assert_eq!(vault.free().unwrap(), FUNDING - granted);
    assert!(!env.campaign(&cngn, 1).active);
}
