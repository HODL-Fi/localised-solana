mod common;

use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

const DAY: i64 = 86_400;
/// `default_market_params` leaves promo idle for 90 days before anyone can reclaim it.
const INACTIVITY: i64 = 90 * DAY;
const GRANT: u64 = 50_000 * ONE_CNGN;
const OWN_CEILING: u64 = 1_118_881 * ONE_CNGN;
const WITH_PROMO_CEILING: u64 = 1_168_781 * ONE_CNGN;

#[test]
fn idle_promo_expires_back_into_the_vault() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();
    let committed = env.promo_vault(&setup.cngn).free().unwrap();

    env.warp_seconds(INACTIVITY);
    // Anyone may do this — it is the protocol's own housekeeping, not the borrower's.
    let stranger = env.funded_keypair();
    send(&mut env.svm, &[expire_promo_ix(&setup.cngn, &owner)], &[&stranger]).unwrap();

    assert_eq!(env.position(&owner).promo_balance, 0);
    let vault = env.promo_vault(&setup.cngn);
    assert_eq!(vault.outstanding, 0);
    // The cNGN never moved; it is simply free again.
    assert_eq!(vault.free().unwrap(), committed + GRANT);
    assert_eq!(env.token_balance(&promo_vault_token_pda(&setup.cngn)), vault.cash);
}

#[test]
fn promo_will_not_expire_early_or_under_a_live_loan() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();

    // One second short of the window.
    env.warp_seconds(INACTIVITY - 1);
    let early = expire_promo_ix(&setup.cngn, &owner);
    assert_hodl_error(send(&mut env.svm, &[early], &[&env.admin]), HodlError::PromoNotExpired);

    // Borrowing restarts the clock, and promo cannot be pulled from under a live loan.
    env.set_pyth_price(&setup.usdc, ONE_DOLLAR, 0);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    env.take_loan(borrower, &setup, 1_000 * ONE_CNGN, 365 * DAY).unwrap();
    env.warp_seconds(INACTIVITY + 1);
    let live = expire_promo_ix(&setup.cngn, &owner);
    assert_hodl_error(send(&mut env.svm, &[live], &[&env.admin]), HodlError::PositionNotEmpty);

    // And a position with no promo has nothing to expire.
    let empty = env.new_borrower();
    let none = expire_promo_ix(&setup.cngn, &empty.pubkey());
    assert_hodl_error(send(&mut env.svm, &[none], &[&env.admin]), HodlError::AmountTooSmall);
}

#[test]
fn taking_a_loan_expires_stale_promo_before_pricing_the_position() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();
    env.warp_seconds(INACTIVITY);
    env.set_pyth_price(&setup.usdc, ONE_DOLLAR, 0);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);

    // Spec §10 step 3: the promo is gone before the health check runs, so the loan the promo
    // would have supported no longer fits.
    let result = env.take_loan(borrower, &setup, WITH_PROMO_CEILING, 30 * DAY);
    assert_hodl_error(result, HodlError::Unhealthy);

    env.take_loan(borrower, &setup, OWN_CEILING, 30 * DAY).unwrap();
    assert_eq!(env.position(&borrower.pubkey()).promo_balance, 0);
    assert_eq!(env.promo_vault(&setup.cngn).outstanding, 0);
}

#[test]
fn an_admin_can_revoke_promo_without_waiting() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    let admin = env.admin.pubkey();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();

    let stranger = env.funded_keypair();
    let by_stranger = revoke_promo_ix(&stranger.pubkey(), &setup.cngn, &owner);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);

    send(&mut env.svm, &[revoke_promo_ix(&admin, &setup.cngn, &owner)], &[&env.admin]).unwrap();
    assert_eq!(env.position(&owner).promo_balance, 0);
    assert_eq!(env.promo_vault(&setup.cngn).outstanding, 0);

    // Revocation is still barred while a loan is live, so it cannot force a liquidation.
    let other = env.new_borrower();
    env.deposit_collateral(&other, &setup.usdc, 1_000 * ONE_USDC);
    let other_cngn = env.create_token_account(&setup.cngn, &other.pubkey());
    env.redeem_promo(&other, &setup.cngn, 1, GRANT, 8).unwrap();
    let prices = env.price_accounts(&other.pubkey());
    let borrow = take_loan_ix(&other.pubkey(), &setup.cngn, &other_cngn, 1_000 * ONE_CNGN, 365 * DAY, prices);
    send(&mut env.svm, &[borrow], &[&env.admin, &other.key]).unwrap();
    let live = revoke_promo_ix(&admin, &setup.cngn, &other.pubkey());
    assert_hodl_error(send(&mut env.svm, &[live], &[&env.admin]), HodlError::PositionNotEmpty);
}

#[test]
fn closing_a_position_hands_its_promo_back() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = env.new_borrower();
    let owner = borrower.pubkey();
    // Redemption only moves GRANT from `unissued` (promised to the campaign) to `outstanding`
    // (promised to the position) — `free()` does not move yet.
    let free_before = env.promo_vault(&setup.cngn).free().unwrap();
    env.redeem_promo(&borrower, &setup.cngn, 1, GRANT, 7).unwrap();
    assert_eq!(env.promo_vault(&setup.cngn).outstanding, GRANT);
    assert_eq!(env.promo_vault(&setup.cngn).free().unwrap(), free_before);

    // Closing without naming the vault would strand the promo, so it is refused.
    let bare = close_position_ix(&owner, &env.admin.pubkey());
    assert_hodl_error(env.sponsored(bare, &borrower.key), HodlError::PromoAccountsRequired);
    // The position, and the vault's committed promo, are both still there: the refusal did not
    // silently drop the promo along with the close.
    assert_eq!(env.promo_vault(&setup.cngn).outstanding, GRANT);

    let close = close_position_with_promo_ix(&owner, &env.admin.pubkey(), &setup.cngn);
    env.sponsored(close, &borrower.key).unwrap();
    // The position is gone, so `position.promo_balance == 0` alone would prove nothing — the
    // vault side is what confirms the promo actually came back rather than being stranded.
    // Release does not credit the campaign back (its `granted` stays permanently spent), so the
    // GRANT becomes genuinely free vault cash — `free()` ends up `free_before + GRANT`, not
    // merely back at `free_before`.
    let vault = env.promo_vault(&setup.cngn);
    assert_eq!(vault.outstanding, 0);
    assert_eq!(vault.free().unwrap(), free_before + GRANT);
    assert!(env.svm.get_account(&position_pda(&owner)).is_none_or(|a| a.lamports == 0));
}

#[test]
fn the_promo_clock_restarts_only_when_the_last_loan_closes() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();
    env.take_loan(borrower, &setup, 1_000 * ONE_CNGN, 365 * DAY).unwrap();
    env.take_loan(borrower, &setup, 1_000 * ONE_CNGN, 365 * DAY).unwrap();
    let after_second = env.position(&owner).promo_last_activity_at;

    // Repaying one of two loans leaves the clock alone: the position is still active.
    env.warp_seconds(DAY);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    env.mint_to(&setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN);
    env.repay(&setup, 0, 2_000 * ONE_CNGN).unwrap();
    assert_eq!(env.position(&owner).promo_last_activity_at, after_second);

    // Repaying the last one restarts it, which is when the idle window begins.
    env.warp_seconds(DAY);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    env.mint_to(&setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN);
    env.repay(&setup, 1, 2_000 * ONE_CNGN).unwrap();
    assert_eq!(env.position(&owner).promo_last_activity_at, env.now());
}

#[test]
fn take_loan_requires_the_promo_vault_when_promo_is_due_for_release() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();
    env.warp_seconds(INACTIVITY);
    env.set_pyth_price(&setup.usdc, ONE_DOLLAR, 0);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);

    // Without the fix, an `if let Some(...)` refactor here would silently let the stale promo
    // survive into the health check ~30 lines later — this pins the `ok_or` guard directly.
    let prices = env.price_accounts(&owner);
    let borrow = take_loan_ix_no_promo_vault(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, prices);
    let result = send(&mut env.svm, &[borrow], &[&env.admin, &borrower.key]);
    assert_hodl_error(result, HodlError::PromoAccountsRequired);

    // The promo is untouched — it did not silently survive past the missing-vault guard either.
    assert_eq!(env.position(&owner).promo_balance, GRANT);
}
