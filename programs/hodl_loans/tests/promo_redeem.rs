mod common;

use common::*;
use anchor_lang::prelude::Pubkey;
use hodl_loans::HodlError;
use solana_keypair::Keypair;
use solana_signer::Signer;

const FUNDING: u64 = 1_000_000 * ONE_CNGN;
const BUDGET: u64 = 300_000 * ONE_CNGN;
const GRANT: u64 = 5_000 * ONE_CNGN;

/// A promo vault funded and one campaign open, with a whitelisted borrower holding a position.
fn ready() -> (Env, Pubkey, Borrower) {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    env.create_campaign(&cngn, 1, BUDGET);
    let borrower = env.new_borrower();
    (env, cngn, borrower)
}

#[test]
fn a_voucher_turns_into_promo_borrowing_power() {
    let (mut env, cngn, borrower) = ready();
    let owner = borrower.pubkey();
    env.redeem_promo(&borrower, &cngn, 1, GRANT, 7).unwrap();

    // The cNGN never moves: it is reassigned from the campaign's reservation to the position.
    let vault = env.promo_vault(&cngn);
    assert_eq!((vault.cash, vault.unissued, vault.outstanding), (FUNDING, BUDGET - GRANT, GRANT));
    assert_eq!(vault.free().unwrap(), FUNDING - BUDGET);
    assert_eq!(env.campaign(&cngn, 1).granted, GRANT);

    let position = env.position(&owner);
    assert_eq!(position.promo_balance, GRANT);
    assert_eq!(position.promo_last_activity_at, env.now());

    let receipt = env.voucher_receipt(&campaign_pda(&cngn, 1), 7);
    assert_eq!((receipt.nonce, receipt.campaign), (7, campaign_pda(&cngn, 1)));
    assert_eq!(receipt.rent_payer, env.admin.pubkey());

    // A second voucher with a fresh nonce adds to the balance.
    env.redeem_promo(&borrower, &cngn, 1, GRANT, 8).unwrap();
    assert_eq!(env.position(&owner).promo_balance, 2 * GRANT);
    assert_eq!(env.promo_vault(&cngn).outstanding, 2 * GRANT);
}

#[test]
fn a_nonce_can_only_be_redeemed_once() {
    let (mut env, cngn, borrower) = ready();
    env.redeem_promo(&borrower, &cngn, 1, GRANT, 7).unwrap();
    // The receipt already exists, so the second redemption cannot initialize it.
    assert!(env.redeem_promo(&borrower, &cngn, 1, GRANT, 7).is_err());
    assert_eq!(env.position(&borrower.pubkey()).promo_balance, GRANT);
}

#[test]
fn one_ed25519_instruction_cannot_authorise_two_redemptions_in_one_transaction() {
    let (mut env, cngn, borrower) = ready();
    let owner = borrower.pubkey();
    let admin = env.admin.pubkey();
    let expiry = env.now() + 86_400;
    let signer = env.promo_signer.insecure_clone();
    let message = env.voucher_message(&cngn, 1, &owner, GRANT, 7, expiry);

    // `require_ed25519_signature` only checks that SOME earlier instruction in the transaction
    // verifies the signature — it does not mark that instruction as consumed. So one Ed25519
    // instruction, by itself, would authorise both `redeem_promo` calls below if nothing else
    // stopped it. What has to stop it is the `voucher_receipt` PDA: the first call creates it
    // with `init`, so the second call's `init` of the same address cannot succeed.
    let instructions = vec![
        ed25519_verify_ix(&signer, &message),
        redeem_promo_ix(&admin, &owner, &cngn, 1, GRANT, 7, expiry),
        redeem_promo_ix(&admin, &owner, &cngn, 1, GRANT, 7, expiry),
    ];
    let result = send(&mut env.svm, &instructions, &[&env.admin, &borrower.key]);
    assert!(result.is_err(), "a single transaction must not redeem the same voucher twice");

    // The whole transaction reverts atomically: even the first, individually-valid redemption
    // never lands.
    assert_eq!(env.position(&owner).promo_balance, 0);
    assert!(env.svm.get_account(&voucher_pda(&campaign_pda(&cngn, 1), 7)).is_none());
}

#[test]
fn only_the_promo_signers_signature_counts() {
    let (mut env, cngn, borrower) = ready();
    let owner = borrower.pubkey();
    let admin = env.admin.pubkey();

    // No Ed25519 instruction at all.
    let expiry = env.now() + 86_400;
    let bare = redeem_promo_ix(&admin, &owner, &cngn, 1, GRANT, 7, expiry);
    let result = send(&mut env.svm, &[bare], &[&env.admin, &borrower.key]);
    assert_hodl_error(result, HodlError::InvalidVoucherSignature);

    // A perfectly valid signature by the wrong key.
    let impostor = Keypair::new();
    let result = env.redeem_voucher_signed_by(&impostor, &borrower, &cngn, 1, GRANT, 7, expiry);
    assert_hodl_error(result, HodlError::InvalidVoucherSignature);

    // The real signer, but signing a message for someone else's wallet.
    let other = env.new_borrower();
    let message = env.voucher_message(&cngn, 1, &other.pubkey(), GRANT, 7, expiry);
    let signer = env.promo_signer.insecure_clone();
    let instructions = vec![
        ed25519_verify_ix(&signer, &message),
        redeem_promo_ix(&admin, &owner, &cngn, 1, GRANT, 7, expiry),
    ];
    let result = send(&mut env.svm, &instructions, &[&env.admin, &borrower.key]);
    assert_hodl_error(result, HodlError::InvalidVoucherSignature);
}

#[test]
fn the_signature_covers_the_amount_the_nonce_and_the_expiry() {
    let (mut env, cngn, borrower) = ready();
    let owner = borrower.pubkey();
    let admin = env.admin.pubkey();
    let expiry = env.now() + 86_400;
    let signer = env.promo_signer.insecure_clone();

    // Sign for one set of values, then ask for another. Each field is part of the message, so
    // each substitution leaves the signature covering bytes the program did not build.
    for (amount, nonce, claimed_expiry) in
        [(GRANT * 2, 7, expiry), (GRANT, 8, expiry), (GRANT, 7, expiry + 1)]
    {
        let message = env.voucher_message(&cngn, 1, &owner, GRANT, 7, expiry);
        let instructions = vec![
            ed25519_verify_ix(&signer, &message),
            redeem_promo_ix(&admin, &owner, &cngn, 1, amount, nonce, claimed_expiry),
        ];
        let result = send(&mut env.svm, &instructions, &[&env.admin, &borrower.key]);
        assert_hodl_error(result, HodlError::InvalidVoucherSignature);
    }

    // The unaltered voucher still works, so the rejections above are about the substitution.
    env.redeem_promo(&borrower, &cngn, 1, GRANT, 7).unwrap();
}

#[test]
fn redemption_respects_the_expiry_the_campaign_and_the_cap() {
    let (mut env, cngn, borrower) = ready();
    let admin = env.admin.pubkey();
    let signer = env.promo_signer.insecure_clone();

    // A voucher whose own expiry has passed.
    let stale = env.now() - 1;
    let result = env.redeem_voucher_signed_by(&signer, &borrower, &cngn, 1, GRANT, 7, stale);
    assert_hodl_error(result, HodlError::VoucherExpired);

    // More than the campaign's remaining budget.
    let result = env.redeem_promo(&borrower, &cngn, 1, BUDGET + 1, 7);
    assert_hodl_error(result, HodlError::CampaignBudgetExceeded);

    // More than one position may hold: the market's cap is 50,000 cNGN.
    let result = env.redeem_promo(&borrower, &cngn, 1, 50_001 * ONE_CNGN, 7);
    assert_hodl_error(result, HodlError::PromoCapExceeded);
    env.redeem_promo(&borrower, &cngn, 1, 50_000 * ONE_CNGN, 7).unwrap();
    let result = env.redeem_promo(&borrower, &cngn, 1, ONE_CNGN, 8);
    assert_hodl_error(result, HodlError::PromoCapExceeded);

    // A closed campaign issues nothing more.
    let other = env.new_borrower();
    send(&mut env.svm, &[close_campaign_ix(&admin, &cngn, 1)], &[&env.admin]).unwrap();
    let result = env.redeem_promo(&other, &cngn, 1, GRANT, 9);
    assert_hodl_error(result, HodlError::CampaignInactive);
}

#[test]
fn a_campaign_past_its_window_issues_nothing() {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();
    let until = env.now() + 86_400;
    send(&mut env.svm, &[create_campaign_ix(&admin, &cngn, 1, BUDGET, until)], &[&env.admin]).unwrap();
    let borrower = env.new_borrower();

    env.warp_seconds(86_401);
    let result = env.redeem_promo(&borrower, &cngn, 1, GRANT, 7);
    assert_hodl_error(result, HodlError::CampaignInactive);
}

#[test]
fn redemption_needs_an_active_whitelist() {
    let (mut env, cngn, borrower) = ready();
    env.blacklist(&borrower.pubkey());
    let result = env.redeem_promo(&borrower, &cngn, 1, GRANT, 7);
    assert_hodl_error(result, HodlError::Blacklisted);
}

#[test]
fn the_expiry_boundary_is_the_only_thing_separating_redeem_from_close() {
    let (mut env, cngn, borrower) = ready();
    let campaign = campaign_pda(&cngn, 1);
    let admin = env.admin.pubkey();
    let signer = env.promo_signer.insecure_clone();
    let expiry = env.now() + 86_400;

    env.redeem_voucher_signed_by(&signer, &borrower, &cngn, 1, GRANT, 7, expiry).unwrap();

    // Warp to the exact expiry second. `redeem_promo` allows `now <= voucher_expiry` and
    // `close_voucher_receipt` requires `now > voucher_expiry` (strict) — these must be exact
    // complements, or the boundary second lets a single transaction replay the voucher via
    // [redeem, close, redeem, close, ...].
    env.warp_seconds(expiry - env.now());
    assert_eq!(env.now(), expiry);

    // Not yet closable at the boundary second.
    let close = close_voucher_receipt_ix(&admin, &campaign, 7);
    assert_hodl_error(send(&mut env.svm, &[close], &[&env.admin]), HodlError::PromoNotExpired);

    // A fresh voucher at the same instant still redeems successfully.
    env.redeem_voucher_signed_by(&signer, &borrower, &cngn, 1, GRANT, 8, expiry).unwrap();
    assert_eq!(env.position(&borrower.pubkey()).promo_balance, 2 * GRANT);
}

#[test]
fn redemption_respects_the_market_pause() {
    let (mut env, cngn, borrower) = ready();
    let admin = env.admin.pubkey();

    send(&mut env.svm, &[set_market_paused_ix(&admin, &cngn, true)], &[&env.admin]).unwrap();
    let result = env.redeem_promo(&borrower, &cngn, 1, GRANT, 7);
    assert_hodl_error(result, HodlError::MarketPaused);

    // Unpausing lets it through, same voucher untouched (the receipt never got created).
    send(&mut env.svm, &[set_market_paused_ix(&admin, &cngn, false)], &[&env.admin]).unwrap();
    env.redeem_promo(&borrower, &cngn, 1, GRANT, 7).unwrap();
    assert_eq!(env.position(&borrower.pubkey()).promo_balance, GRANT);
}

#[test]
fn redemption_cannot_rebind_a_position_already_bound_to_another_market() {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 30 * 86_400).unwrap();
    assert_eq!(env.position(&setup.borrower.pubkey()).market, market_pda(&setup.cngn));

    // A second market, with its own funded promo vault and campaign.
    let other = env.create_mint(MintKind::CngnLike, 6);
    env.create_market_with_promo(&other);
    let admin = env.admin.pubkey();
    let source = env.create_token_account(&other, &admin);
    env.mint_to(&other, &source, FUNDING);
    send(&mut env.svm, &[fund_promo_vault_ix(&admin, &other, &source, FUNDING)], &[&env.admin]).unwrap();
    env.create_campaign(&other, 1, BUDGET);

    // The position is already bound to `setup.cngn` from the loan above; redeeming against the
    // second market's own valid voucher must not silently rebind it.
    let result = env.redeem_promo(&setup.borrower, &other, 1, GRANT, 7);
    assert_hodl_error(result, HodlError::MarketMismatch);
}

#[test]
fn a_receipt_is_closable_once_its_voucher_can_no_longer_be_used() {
    let (mut env, cngn, borrower) = ready();
    let campaign = campaign_pda(&cngn, 1);
    let admin = env.admin.pubkey();
    env.redeem_promo(&borrower, &cngn, 1, GRANT, 7).unwrap();

    // While the voucher could still be presented, the receipt has to stay.
    let early = close_voucher_receipt_ix(&admin, &campaign, 7);
    assert_hodl_error(send(&mut env.svm, &[early], &[&env.admin]), HodlError::PromoNotExpired);

    env.warp_seconds(86_401);
    let before = env.svm.get_account(&admin).unwrap().lamports;
    let close = close_voucher_receipt_ix(&admin, &campaign, 7);
    send(&mut env.svm, &[close], &[&env.admin]).unwrap();
    assert!(env.svm.get_account(&voucher_pda(&campaign, 7)).is_none_or(|a| a.lamports == 0));
    assert!(env.svm.get_account(&admin).unwrap().lamports > before);

    // The promo the voucher granted is untouched by closing its receipt.
    assert_eq!(env.position(&borrower.pubkey()).promo_balance, GRANT);
}
