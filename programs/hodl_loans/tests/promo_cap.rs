mod common;

use anchor_lang::prelude::AccountMeta;
use common::*;
use hodl_loans::HodlError;
use solana_keypair::Keypair;
use solana_signer::Signer;

#[test]
fn the_cap_is_re_checked_against_every_listed_asset() {
    let (mut env, _cngn) = Env::with_cngn_market();
    let admin = env.admin.pubkey();
    // Default collateral is 70% LTV against a 90% threshold, so 20 points of room.
    let usdc = env.list_spl_collateral(6);
    let sol = env.list_spl_collateral(9);
    assert_eq!(env.config().collateral_count, 2);
    assert_eq!(env.config().promo_cap_bps, 2_000);

    // Exactly the room every asset has is allowed; one point more is not.
    let raise = set_promo_cap_ix(&admin, 2_001, &[usdc, sol]);
    assert_hodl_error(send(&mut env.svm, &[raise], &[&env.admin]), HodlError::InvalidParameters);
    let exact = set_promo_cap_ix(&admin, 2_000, &[usdc, sol]);
    send(&mut env.svm, &[exact], &[&env.admin]).unwrap();

    // Lowering it is always safe.
    let lower = set_promo_cap_ix(&admin, 500, &[usdc, sol]);
    send(&mut env.svm, &[lower], &[&env.admin]).unwrap();
    assert_eq!(env.config().promo_cap_bps, 500);

    // The tightest asset is the one that binds: 5% of room leaves room for a 5% cap.
    let tight = hodl_loans::CollateralParams {
        ltv_bps: 7_000,
        liquidation_threshold_bps: 7_500,
        ..default_collateral_params(&sol)
    };
    let update = update_collateral_params_ix(&admin, &sol, tight);
    send(&mut env.svm, &[update], &[&env.admin]).unwrap();
    let over = set_promo_cap_ix(&admin, 501, &[usdc, sol]);
    assert_hodl_error(send(&mut env.svm, &[over], &[&env.admin]), HodlError::InvalidParameters);
    let ok = set_promo_cap_ix(&admin, 500, &[usdc, sol]);
    send(&mut env.svm, &[ok], &[&env.admin]).unwrap();
}

#[test]
fn no_listed_asset_can_be_skipped_or_stood_in_for() {
    let (mut env, _cngn) = Env::with_cngn_market();
    let admin = env.admin.pubkey();
    let permissive = env.list_spl_collateral(6);
    let strict_params_mint = env.list_spl_collateral(9);
    // The cap has to come down before an asset can be tightened past it: the same rule binds
    // both directions, and it is currently 20%.
    send(&mut env.svm, &[set_promo_cap_ix(&admin, 100, &[permissive, strict_params_mint])], &[&env.admin]).unwrap();
    let strict = hodl_loans::CollateralParams {
        ltv_bps: 7_000,
        liquidation_threshold_bps: 7_100,
        ..default_collateral_params(&strict_params_mint)
    };
    send(&mut env.svm, &[update_collateral_params_ix(&admin, &strict_params_mint, strict)], &[&env.admin]).unwrap();

    // Leaving the strict asset out fails on the count.
    let short = set_promo_cap_ix(&admin, 1_000, &[permissive]);
    assert_hodl_error(send(&mut env.svm, &[short], &[&env.admin]), HodlError::InvalidParameters);

    // Passing the permissive one twice satisfies a bare count check, which is why the keys must
    // strictly increase: a duplicate is not a second asset.
    let duplicated = set_promo_cap_ix(&admin, 1_000, &[permissive, permissive]);
    assert_hodl_error(send(&mut env.svm, &[duplicated], &[&env.admin]), HodlError::InvalidParameters);

    // With both passed honestly, the strict asset's 1% of room is what binds.
    let over = set_promo_cap_ix(&admin, 101, &[permissive, strict_params_mint]);
    assert_hodl_error(send(&mut env.svm, &[over], &[&env.admin]), HodlError::InvalidParameters);
    let ok = set_promo_cap_ix(&admin, 100, &[permissive, strict_params_mint]);
    send(&mut env.svm, &[ok], &[&env.admin]).unwrap();
}

#[test]
fn a_forged_asset_at_an_arbitrary_key_cannot_stand_in_for_the_real_one() {
    // The count, ordering, owner and discriminator checks all pass for a look-alike account —
    // only the PDA re-derivation catches it. Copy a real listed asset's `mint` and `bump` (so
    // re-derivation would still reproduce the *real* collateral PDA) but drop `ltv_bps` to a
    // value permissive enough to pass `validate` at a cap the real 70%/90% asset would reject,
    // then plant that copy program-owned at an unrelated key instead of the real PDA.
    let (mut env, _cngn) = Env::with_cngn_market();
    let admin = env.admin.pubkey();
    let mint = env.list_spl_collateral(6);
    assert_eq!(env.config().collateral_count, 1);

    let real: hodl_loans::CollateralAsset = env.fetch(&collateral_pda(&mint));
    let forged = hodl_loans::CollateralAsset { ltv_bps: 1_000, ..real };
    let forged_key = Keypair::new().pubkey();
    env.set_account_data(&forged_key, &hodl_loans::ID, collateral_asset_bytes(&forged));

    // 50% would fail against the real asset (70% LTV + 50% cap > 90% threshold) but passes
    // against the forged one's 10% LTV — the only thing standing in the way is re-derivation.
    let mut instruction = ix(
        hodl_loans::instruction::SetPromoCap { promo_cap_bps: 5_000 },
        hodl_loans::accounts::SetPromoCap { admin, config: config_pda() },
    );
    instruction.accounts.push(AccountMeta::new_readonly(forged_key, false));
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::InvalidParameters);
}

#[test]
fn setting_the_cap_is_admin_only_and_bounded() {
    let (mut env, _cngn) = Env::with_cngn_market();
    let admin = env.admin.pubkey();
    let stranger = env.funded_keypair();

    let by_stranger = set_promo_cap_ix(&stranger.pubkey(), 100, &[]);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);

    // Above 100% is meaningless, and with no assets listed there is nothing to contradict it.
    let absurd = set_promo_cap_ix(&admin, 10_001, &[]);
    assert_hodl_error(send(&mut env.svm, &[absurd], &[&env.admin]), HodlError::InvalidParameters);
    // A 100% cap is accepted here only because no asset is listed to re-check it against. It
    // leaves the program unable to list any *new* asset afterwards — `validate` would need
    // `ltv >= 1_000` and `ltv + 10_000 <= lt <= 10_000`, which is unsatisfiable — but that is
    // correct and recoverable (the admin can lower the cap again), not a lockout of the program.
    send(&mut env.svm, &[set_promo_cap_ix(&admin, 10_000, &[])], &[&env.admin]).unwrap();
    assert_eq!(env.config().promo_cap_bps, 10_000);
}
