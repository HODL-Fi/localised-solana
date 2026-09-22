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

#[test]
fn reconciling_writes_cash_down_to_the_balance_an_issuer_clawback_left() {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();
    let token = promo_vault_token_pda(&cngn);
    let destination = env.treasury_token(&cngn);

    // The issuer pulls a quarter of the vault out directly. Nothing in the program observed
    // it, so `cash` still claims the full funding and `free()` reads high with it.
    let clawed = FUNDING / 4;
    env.delegate_burn(&cngn, &token, clawed);
    assert_eq!(env.token_balance(&token), FUNDING - clawed);
    assert_eq!(env.promo_vault(&cngn).cash, FUNDING);
    assert_eq!(env.promo_vault(&cngn).free().unwrap(), FUNDING);

    // Withdrawing what `free()` promises passes our own bound and then fails inside the token
    // program — the wrong place to learn the vault is short.
    let overdraw = withdraw_promo_vault_ix(&admin, &cngn, &destination, FUNDING);
    assert!(send(&mut env.svm, &[overdraw], &[&env.admin]).is_err());

    send(&mut env.svm, &[reconcile_promo_vault_ix(&admin, &cngn)], &[&env.admin]).unwrap();
    let vault = env.promo_vault(&cngn);
    assert_eq!(vault.cash, FUNDING - clawed);
    assert_eq!(vault.free().unwrap(), FUNDING - clawed);

    // And now the books and the tokens agree, so withdrawing everything works.
    let withdraw = withdraw_promo_vault_ix(&admin, &cngn, &destination, FUNDING - clawed);
    send(&mut env.svm, &[withdraw], &[&env.admin]).unwrap();
    assert_eq!(env.token_balance(&destination), FUNDING - clawed);
    assert_eq!(env.promo_vault(&cngn).cash, 0);
}

#[test]
fn reconciling_only_ever_writes_cash_down() {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();

    // Books and balance already agree: there is nothing to reconcile, and the call says so
    // rather than silently doing nothing.
    let noop = reconcile_promo_vault_ix(&admin, &cngn);
    assert_hodl_error(send(&mut env.svm, &[noop], &[&env.admin]), HodlError::AmountTooSmall);

    // A donation straight into the vault's token account leaves the balance *above* `cash`.
    // Reconciling must not book it as protocol funds — that is what `fund_promo_vault` is
    // for, and it is the path that does the accounting.
    env.mint_to(&cngn, &promo_vault_token_pda(&cngn), 1_000 * ONE_CNGN);
    assert_eq!(env.token_balance(&promo_vault_token_pda(&cngn)), FUNDING + 1_000 * ONE_CNGN);
    let up = reconcile_promo_vault_ix(&admin, &cngn);
    assert_hodl_error(send(&mut env.svm, &[up], &[&env.admin]), HodlError::AmountTooSmall);
    assert_eq!(env.promo_vault(&cngn).cash, FUNDING);

    // A stranger cannot call it.
    let stranger = env.funded_keypair();
    let by_stranger = reconcile_promo_vault_ix(&stranger.pubkey(), &cngn);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);
}

#[test]
fn reconciling_refuses_a_clawback_that_has_already_eaten_into_outstanding_promo() {
    let (mut env, setup) = Env::promo_ready();
    let admin = env.admin.pubkey();
    let cngn = setup.cngn;
    let grant = 50_000 * ONE_CNGN;
    env.redeem_promo(&setup.borrower, &cngn, 1, grant, 7).unwrap();
    assert_eq!(env.promo_vault(&cngn).outstanding, grant);

    // The issuer takes the vault down below what positions are already holding.
    let token = promo_vault_token_pda(&cngn);
    let leave = grant / 2;
    env.delegate_burn(&cngn, &token, env.token_balance(&token) - leave);
    assert_eq!(env.token_balance(&token), leave);

    // Writing `cash` down to the real balance would leave `outstanding > cash`, and no single
    // field rewrite can honestly repair that: `outstanding` may only fall through expiry,
    // revocation or liquidation, each of which settles a named position. So the call fails
    // rather than recording a vault that claims to back more than it holds.
    let reconcile = reconcile_promo_vault_ix(&admin, &cngn);
    assert_hodl_error(send(&mut env.svm, &[reconcile], &[&env.admin]), HodlError::PromoVaultInsufficient);
    assert_eq!(env.promo_vault(&cngn).cash, 10_000_000 * ONE_CNGN);

    // Once nothing is committed against the vault any more — the position's promo revoked and
    // the campaign's unissued budget returned — the same call goes through, so the refusal
    // above is the invariant talking and not a blanket block.
    send(&mut env.svm, &[revoke_promo_ix(&admin, &cngn, &setup.borrower.pubkey())], &[&env.admin]).unwrap();
    send(&mut env.svm, &[close_campaign_ix(&admin, &cngn, 1)], &[&env.admin]).unwrap();
    let vault = env.promo_vault(&cngn);
    assert_eq!((vault.outstanding, vault.unissued), (0, 0));
    send(&mut env.svm, &[reconcile_promo_vault_ix(&admin, &cngn)], &[&env.admin]).unwrap();
    assert_eq!(env.promo_vault(&cngn).cash, leave);
}

#[test]
fn a_clawback_then_return_moves_promo_budget_to_the_treasury_once_reconciled() {
    // Reconciling is not undone by the issuer giving the tokens back. `cash` has already been
    // written down, so the returned tokens read as unaccounted balance — which is exactly
    // what `sweep_promo_excess` sends to the treasury. Documented on `reconcile_promo_vault`
    // and pinned here, because an admin reconciling a clawback they expect to be reversed
    // should wait instead.
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();
    let token = promo_vault_token_pda(&cngn);
    let treasury = env.treasury_token(&cngn);
    let clawed = FUNDING / 4;

    env.delegate_burn(&cngn, &token, clawed);
    send(&mut env.svm, &[reconcile_promo_vault_ix(&admin, &cngn)], &[&env.admin]).unwrap();
    assert_eq!(env.promo_vault(&cngn).cash, FUNDING - clawed);

    // The issuer returns what it took.
    env.mint_to(&cngn, &token, clawed);
    assert_eq!(env.token_balance(&token), FUNDING);
    // `cash` does not follow it back up — reconcile only ever writes down.
    assert_eq!(env.promo_vault(&cngn).cash, FUNDING - clawed);

    // So the sweep treats the returned tokens as a donation.
    let sweep = sweep_promo_excess_ix(&admin, &cngn, &treasury);
    send(&mut env.svm, &[sweep], &[&env.admin]).unwrap();
    assert_eq!(env.token_balance(&treasury), clawed);
    assert_eq!(env.promo_vault(&cngn).cash, FUNDING - clawed);
    // Only `fund_promo_vault` puts it back into the promo budget.
}
