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
    let free_before = env.promo_vault(&setup.cngn).free().unwrap();
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
    let vault = env.promo_vault(&setup.cngn);
    assert_eq!(vault.outstanding, 0);
    // Release does not credit the campaign back — same conservation property as every other
    // release path: `free()` rises by GRANT rather than merely returning to `free_before`.
    assert_eq!(vault.free().unwrap(), free_before + GRANT);
}

#[test]
fn an_admin_can_revoke_promo_without_waiting() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    let admin = env.admin.pubkey();
    let free_before = env.promo_vault(&setup.cngn).free().unwrap();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();

    let stranger = env.funded_keypair();
    let by_stranger = revoke_promo_ix(&stranger.pubkey(), &setup.cngn, &owner);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);

    send(&mut env.svm, &[revoke_promo_ix(&admin, &setup.cngn, &owner)], &[&env.admin]).unwrap();
    assert_eq!(env.position(&owner).promo_balance, 0);
    let vault = env.promo_vault(&setup.cngn);
    assert_eq!(vault.outstanding, 0);
    // Same conservation property as every other release path: `free()` rises by GRANT.
    assert_eq!(vault.free().unwrap(), free_before + GRANT);

    // Revocation is no longer barred outright while a loan is live — only when releasing the
    // promo would leave the position unhealthy. `other`'s loan is small next to its own
    // collateral, so it does not need the promo to stand.
    let other = env.new_borrower();
    env.deposit_collateral(&other, &setup.usdc, 1_000 * ONE_USDC);
    let other_cngn = env.create_token_account(&setup.cngn, &other.pubkey());
    env.redeem_promo(&other, &setup.cngn, 1, GRANT, 8).unwrap();
    let prices = env.price_accounts(&other.pubkey());
    let borrow = take_loan_ix(&other.pubkey(), &setup.cngn, &other_cngn, 1_000 * ONE_CNGN, 365 * DAY, prices);
    send(&mut env.svm, &[borrow], &[&env.admin, &other.key]).unwrap();

    // A bare call carries no `ngn_feed` at all, and a feed-less priced call withholds only the
    // feed while still supplying `remaining_accounts` — both fail the same way, because the
    // health check the live loan now requires has no price to run on.
    let bare = revoke_promo_ix(&admin, &setup.cngn, &other.pubkey());
    assert_hodl_error(send(&mut env.svm, &[bare], &[&env.admin]), HodlError::PriceAccountMismatch);
    let feedless = revoke_promo_priced_ix(&admin, &setup.cngn, &other.pubkey(), false, price_pairs(&[setup.usdc]));
    assert_hodl_error(send(&mut env.svm, &[feedless], &[&env.admin]), HodlError::PriceAccountMismatch);

    // Priced, it succeeds: `other`'s loan does not need the promo to stay healthy.
    let priced = revoke_promo_priced_ix(&admin, &setup.cngn, &other.pubkey(), true, price_pairs(&[setup.usdc]));
    send(&mut env.svm, &[priced], &[&env.admin]).unwrap();
    assert_eq!(env.position(&other.pubkey()).promo_balance, 0);
    assert_eq!(env.promo_vault(&setup.cngn).outstanding, 0);
}

#[test]
fn revoking_under_a_live_loan_is_refused_when_the_promo_is_holding_the_position_up() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    let admin = env.admin.pubkey();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();

    // Borrow past what the collateral alone supports: `OWN_CEILING` is the limit without the
    // promo, and the promo lifts it to `WITH_PROMO_CEILING`. Anything above the first is debt
    // the promo is carrying.
    env.take_loan(borrower, &setup, OWN_CEILING + ONE_CNGN, 365 * DAY).unwrap();

    // Taking the promo away would put the position under its own borrow limit, so revocation
    // is refused — the guarantee the old `!has_active_loans()` rule was reaching for, now
    // stated as the thing it actually protects rather than as a blanket ban.
    let priced = revoke_promo_priced_ix(&admin, &setup.cngn, &owner, true, price_pairs(&[setup.usdc]));
    assert_hodl_error(send(&mut env.svm, &[priced], &[&env.admin]), HodlError::Unhealthy);
    assert_eq!(env.position(&owner).promo_balance, GRANT);
    assert_eq!(env.promo_vault(&setup.cngn).outstanding, GRANT);

    // Repaying back under the unaided ceiling makes the same call succeed: the borrower can
    // no longer be hurt by it.
    env.repay(&setup, 0, 2 * ONE_CNGN).unwrap();
    let priced = revoke_promo_priced_ix(&admin, &setup.cngn, &owner, true, price_pairs(&[setup.usdc]));
    send(&mut env.svm, &[priced], &[&env.admin]).unwrap();
    assert_eq!(env.position(&owner).promo_balance, 0);
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
fn expiring_an_already_released_promo_is_rejected() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();
    env.warp_seconds(INACTIVITY);
    let stranger = env.funded_keypair();
    send(&mut env.svm, &[expire_promo_ix(&setup.cngn, &owner)], &[&stranger]).unwrap();
    assert_eq!(env.position(&owner).promo_balance, 0);
    // Unlike the never-promoed position in `promo_will_not_expire_early_or_under_a_live_loan`,
    // `position.market` is set here (redemption bound it) — the chokepoint still rejects on the
    // amount check before it ever looks at the market.
    assert_ne!(env.position(&owner).market, anchor_lang::prelude::Pubkey::default());

    let again = expire_promo_ix(&setup.cngn, &owner);
    assert_hodl_error(send(&mut env.svm, &[again], &[&stranger]), HodlError::AmountTooSmall);
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

#[test]
fn cross_market_vault_and_market_accounts_are_rejected() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();

    // A second market, fully promo-equipped and funded, that this position never redeemed
    // against.
    let other = env.create_mint(MintKind::CngnLike, 6);
    env.create_market_with_promo(&other);
    let lender = env.new_lender(&other, POOL_CNGN);
    env.deposit(&lender, &other, POOL_CNGN).unwrap();

    // Market B's own vault holds real, committed promo — redeemed by one of its own borrowers —
    // so the mismatch checks below are shown to protect funds actually at stake, rather than
    // merely colliding with an empty vault's underflow.
    let admin = env.admin.pubkey();
    let other_source = env.create_token_account(&other, &admin);
    env.mint_to(&other, &other_source, 10_000_000 * ONE_CNGN);
    let fund_other = fund_promo_vault_ix(&admin, &other, &other_source, 10_000_000 * ONE_CNGN);
    send(&mut env.svm, &[fund_other], &[&env.admin]).expect("fund market B promo vault");
    env.create_campaign(&other, 1, 5_000_000 * ONE_CNGN);
    let other_market_borrower = env.new_borrower();
    env.redeem_promo(&other_market_borrower, &other, 1, GRANT, 1).unwrap();
    let other_outstanding_before = env.promo_vault(&other).outstanding;
    assert_eq!(other_outstanding_before, GRANT);

    // expire_promo: market B's market + vault named for a position still bound to market A.
    env.warp_seconds(INACTIVITY);
    let wrong_expire = expire_promo_ix(&other, &owner);
    assert_hodl_error(send(&mut env.svm, &[wrong_expire], &[&env.admin]), HodlError::MarketMismatch);
    // Market B's committed promo is untouched — the rejection happened before any release.
    assert_eq!(env.promo_vault(&other).outstanding, other_outstanding_before);

    // revoke_promo: same mismatch, no waiting required.
    let wrong_revoke = revoke_promo_ix(&admin, &other, &owner);
    assert_hodl_error(send(&mut env.svm, &[wrong_revoke], &[&env.admin]), HodlError::MarketMismatch);
    assert_eq!(env.promo_vault(&other).outstanding, other_outstanding_before);

    // take_loan: naming market B's accounts for a position bound to market A is rejected before
    // the promo is ever inspected.
    let other_account = env.create_token_account(&other, &owner);
    let prices = env.price_accounts(&owner);
    let borrow = take_loan_ix(&owner, &other, &other_account, 1_000 * ONE_CNGN, 30 * DAY, prices);
    assert_hodl_error(send(&mut env.svm, &[borrow], &[&env.admin, &borrower.key]), HodlError::MarketMismatch);

    // The promo survived all three rejected attempts.
    assert_eq!(env.position(&owner).promo_balance, GRANT);

    // close_position: same mismatch, on a second position so the first stays intact for the
    // assertion above. Both `market` and `promo_vault` here come from market B, so this exercises
    // `close_position`'s own `position.market == market.key()` check (`close_position.rs:45`),
    // not `release_promo`'s chokepoint — see
    // `close_position_release_promo_chokepoint_rejects_cross_market_vault` below for that.
    let other_borrower = env.new_borrower();
    let other_owner = other_borrower.pubkey();
    env.redeem_promo(&other_borrower, &setup.cngn, 1, GRANT, 8).unwrap();
    let wrong_close = close_position_with_promo_ix(&other_owner, &env.admin.pubkey(), &other);
    assert_hodl_error(env.sponsored(wrong_close, &other_borrower.key), HodlError::MarketMismatch);
    assert_eq!(env.position(&other_owner).promo_balance, GRANT);
}

#[test]
fn close_position_release_promo_chokepoint_rejects_cross_market_vault() {
    // `close_position_with_promo_ix` derives both `market` and `promo_vault` from a single
    // mint, so it cannot present a mismatched pair — the leg above fires on
    // `close_position`'s own `position.market == market.key()` check, never on
    // `release_promo`'s chokepoint. `promo_vault` is the one account in `close_position` with no
    // `has_one = market` constraint (its seeds are self-referential), so it is the one place a
    // foreign vault can actually reach `release_promo`. This test isolates exactly that: `market`
    // matches the position (market A), only `promo_vault` is foreign (market B).
    let (mut env, setup) = Env::promo_ready();
    let borrower = env.new_borrower();
    let owner = borrower.pubkey();
    env.redeem_promo(&borrower, &setup.cngn, 1, GRANT, 7).unwrap();

    // Market B, funded with real committed promo redeemed by one of its own borrowers — enough
    // to cover the release amount, so a wrongly-accepted call would drain it rather than
    // underflow.
    let other = env.create_mint(MintKind::CngnLike, 6);
    env.create_market_with_promo(&other);
    let admin = env.admin.pubkey();
    let other_source = env.create_token_account(&other, &admin);
    env.mint_to(&other, &other_source, 10_000_000 * ONE_CNGN);
    let fund_other = fund_promo_vault_ix(&admin, &other, &other_source, 10_000_000 * ONE_CNGN);
    send(&mut env.svm, &[fund_other], &[&env.admin]).expect("fund market B promo vault");
    env.create_campaign(&other, 1, 5_000_000 * ONE_CNGN);
    let other_market_borrower = env.new_borrower();
    env.redeem_promo(&other_market_borrower, &other, 1, GRANT, 1).unwrap();
    let other_outstanding_before = env.promo_vault(&other).outstanding;
    assert_eq!(other_outstanding_before, GRANT);

    // `market` names market A (matching the position, so `close_position.rs:45` passes cleanly);
    // `promo_vault` names market B's vault.
    let wrong_close = close_position_with_split_promo_ix(&owner, &env.admin.pubkey(), &setup.cngn, &other);
    assert_hodl_error(env.sponsored(wrong_close, &borrower.key), HodlError::MarketMismatch);

    // Neither side moved: the position still holds its promo, and market B's committed promo
    // is untouched.
    assert_eq!(env.position(&owner).promo_balance, GRANT);
    assert_eq!(env.promo_vault(&other).outstanding, other_outstanding_before);
}

#[test]
fn a_pause_stops_the_inactivity_clock_rather_than_merely_deferring_the_harvest() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();
    let admin = env.admin.pubkey();
    let guardian = env.guardian.pubkey();

    // The guardian pauses the market, then leaves it paused for longer than the whole
    // inactivity window. The borrower cannot act on a paused market, so none of this is
    // inactivity the protocol may charge them for.
    send(&mut env.svm, &[set_market_paused_ix(&guardian, &setup.cngn, true)], &[&env.guardian]).unwrap();
    env.warp_seconds(INACTIVITY + DAY);

    // Expiry is barred outright while paused.
    let stranger = env.funded_keypair();
    let during = expire_promo_ix(&setup.cngn, &owner);
    assert_hodl_error(send(&mut env.svm, &[during], &[&stranger]), HodlError::MarketPaused);

    // And barring it during the pause is not on its own enough: the clock also restarts, so
    // lifting the pause does not leave the promo instantly expirable. Without the restart
    // this call would succeed, charging the borrower for the protocol's own downtime one
    // moment later than before.
    send(&mut env.svm, &[set_market_paused_ix(&admin, &setup.cngn, false)], &[&env.admin]).unwrap();
    let right_after = expire_promo_ix(&setup.cngn, &owner);
    assert_hodl_error(send(&mut env.svm, &[right_after], &[&stranger]), HodlError::PromoNotExpired);
    assert_eq!(env.position(&owner).promo_balance, GRANT);

    // The borrower gets a full fresh window, and no more than one.
    env.warp_seconds(INACTIVITY - 1);
    let one_short = expire_promo_ix(&setup.cngn, &owner);
    assert_hodl_error(send(&mut env.svm, &[one_short], &[&stranger]), HodlError::PromoNotExpired);
    env.warp_seconds(1);
    send(&mut env.svm, &[expire_promo_ix(&setup.cngn, &owner)], &[&stranger]).unwrap();
    assert_eq!(env.position(&owner).promo_balance, 0);
}

#[test]
fn revoke_stays_open_during_a_pause_because_it_measures_nothing() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();
    let admin = env.admin.pubkey();
    let guardian = env.guardian.pubkey();

    send(&mut env.svm, &[set_market_paused_ix(&guardian, &setup.cngn, true)], &[&env.guardian]).unwrap();
    // Unlike expiry, revocation asserts nothing about how long the borrower has been idle —
    // it is the admin retracting a grant — so a pause does not invalidate it.
    send(&mut env.svm, &[revoke_promo_ix(&admin, &setup.cngn, &owner)], &[&env.admin]).unwrap();
    assert_eq!(env.position(&owner).promo_balance, 0);
    assert_eq!(env.promo_vault(&setup.cngn).outstanding, 0);
}

#[test]
fn two_pause_cycles_each_restart_the_clock_for_every_position() {
    // The clock is market-global, so every unpause resets it for every position. Two
    // unrelated incidents inside one inactivity window therefore mean nothing expires at all.
    // Pinned rather than fixed: it keeps the protocol's own promo budget committed — the
    // admin's downtime, the admin's cost — and the alternative is per-position accounting of
    // paused time. This test is what makes that a decision instead of a surprise.
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();
    let admin = env.admin.pubkey();
    let guardian = env.guardian.pubkey();
    let stranger = env.funded_keypair();

    for _ in 0..2 {
        send(&mut env.svm, &[set_market_paused_ix(&guardian, &setup.cngn, true)], &[&env.guardian]).unwrap();
        env.warp_seconds(INACTIVITY - DAY);
        send(&mut env.svm, &[set_market_paused_ix(&admin, &setup.cngn, false)], &[&env.admin]).unwrap();
        env.warp_seconds(DAY);
    }

    // Well over two inactivity windows have passed in wall-clock terms, and the promo is
    // still not expirable: each unpause moved the deadline out by a full window.
    let after = expire_promo_ix(&setup.cngn, &owner);
    assert_hodl_error(send(&mut env.svm, &[after], &[&stranger]), HodlError::PromoNotExpired);
    assert_eq!(env.position(&owner).promo_balance, GRANT);

    // Left alone for one uninterrupted window, it expires as normal.
    env.warp_seconds(INACTIVITY);
    send(&mut env.svm, &[expire_promo_ix(&setup.cngn, &owner)], &[&stranger]).unwrap();
    assert_eq!(env.position(&owner).promo_balance, 0);
}
