mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use anchor_lang::prelude::AccountMeta;
use common::*;
use hodl_loans::HodlError;
use solana_keypair::Keypair;
use solana_signer::Signer;

#[test]
fn sponsor_opens_a_position_for_a_wallet_without_sol() {
    let mut env = Env::initialized();
    let borrower = env.new_borrower();

    let position = env.position(&borrower.pubkey());
    assert_eq!(position.version, 1);
    assert_eq!(position.owner, borrower.pubkey());
    assert_eq!(position.rent_payer, env.admin.pubkey());
    assert_eq!(position.market, anchor_lang::prelude::Pubkey::default());
    assert_eq!((position.next_loan_id, position.promo_balance), (0, 0));
    assert!(!position.has_collateral() && !position.has_active_loans());
    assert_eq!(env.svm.get_account(&borrower.pubkey()).map_or(0, |a| a.lamports), 0);

    // One position per owner.
    let again = open_position_ix(&env.admin.pubkey(), &borrower.pubkey());
    assert!(send(&mut env.svm, &[again], &[&env.admin, &borrower.key]).is_err());
}

#[test]
fn only_active_wallets_open_positions() {
    let mut env = Env::initialized();
    let admin = env.admin.pubkey();

    let unknown = Keypair::new();
    let instruction = open_position_ix(&admin, &unknown.pubkey());
    assert_anchor_error(send(&mut env.svm, &[instruction], &[&env.admin, &unknown]), AnchorError::AccountNotInitialized);

    let blacklisted = Keypair::new();
    env.whitelist(&blacklisted.pubkey());
    env.blacklist(&blacklisted.pubkey());
    let instruction = open_position_ix(&admin, &blacklisted.pubkey());
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin, &blacklisted]), HodlError::Blacklisted);
}

#[test]
fn deposits_fill_matching_then_free_slots() {
    let mut env = Env::initialized();
    let usdc = env.list_spl_collateral(6);
    let sol = env.list_spl_collateral(9);
    let borrower = env.new_borrower();

    let token = env.deposit_collateral(&borrower, &usdc, 300 * ONE_USDC);
    assert_eq!(env.token_balance(&token), 0);
    env.deposit_collateral(&borrower, &sol, 2_000_000_000);
    env.deposit_collateral(&borrower, &usdc, 200 * ONE_USDC);

    let position = env.position(&borrower.pubkey());
    assert_eq!((position.collateral[0].mint, position.collateral[0].amount), (usdc, 500 * ONE_USDC));
    assert_eq!((position.collateral[1].mint, position.collateral[1].amount), (sol, 2_000_000_000));
    assert_eq!(position.collateral[2].amount, 0);
    assert_eq!(env.collateral(&usdc).total_deposited, 500 * ONE_USDC);
    assert_eq!(env.token_balance(&collateral_vault_pda(&usdc)), 500 * ONE_USDC);
    assert_eq!(env.collateral(&sol).total_deposited, 2_000_000_000);
}

#[test]
fn deposit_rejections() {
    let mut env = Env::initialized();
    let usdc = env.list_spl_collateral(6);
    let borrower = env.new_borrower();
    let owner = borrower.pubkey();
    let token = env.create_token_account(&usdc, &owner);
    env.mint_to(&usdc, &token, 100 * ONE_USDC);
    let deposit = |amount| deposit_collateral_ix(&owner, &usdc, &SPL_TOKEN, &token, amount);

    assert_hodl_error(send(&mut env.svm, &[deposit(0)], &[&env.admin, &borrower.key]), HodlError::AmountTooSmall);

    let pause = set_collateral_paused_ix(&env.guardian.pubkey(), &usdc, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();
    assert_hodl_error(send(&mut env.svm, &[deposit(ONE_USDC)], &[&env.admin, &borrower.key]), HodlError::CollateralPaused);
    let unpause = set_collateral_paused_ix(&env.admin.pubkey(), &usdc, false);
    send(&mut env.svm, &[unpause], &[&env.admin]).unwrap();

    let capped = hodl_loans::CollateralParams { deposit_cap: 50 * ONE_USDC, ..default_collateral_params(&usdc) };
    let update = update_collateral_params_ix(&env.admin.pubkey(), &usdc, capped);
    send(&mut env.svm, &[update], &[&env.admin]).unwrap();
    send(&mut env.svm, &[deposit(50 * ONE_USDC)], &[&env.admin, &borrower.key]).unwrap();
    assert_hodl_error(send(&mut env.svm, &[deposit(1)], &[&env.admin, &borrower.key]), HodlError::DepositCapExceeded);

    env.blacklist(&owner);
    assert_hodl_error(send(&mut env.svm, &[deposit(1)], &[&env.admin, &borrower.key]), HodlError::Blacklisted);
}

#[test]
fn a_ninth_collateral_mint_has_no_slot() {
    let mut env = Env::initialized();
    let borrower = env.new_borrower();
    for _ in 0..8 {
        let mint = env.list_spl_collateral(6);
        env.deposit_collateral(&borrower, &mint, 1);
    }
    let ninth = env.list_spl_collateral(6);
    let token = env.create_token_account(&ninth, &borrower.pubkey());
    env.mint_to(&ninth, &token, 1);
    let deposit = deposit_collateral_ix(&borrower.pubkey(), &ninth, &SPL_TOKEN, &token, 1);
    assert_hodl_error(send(&mut env.svm, &[deposit], &[&env.admin, &borrower.key]), HodlError::NoFreeCollateralSlot);
}

#[test]
fn closing_refunds_rent_to_the_sponsor() {
    let mut env = Env::initialized();
    let usdc = env.list_spl_collateral(6);
    let borrower = env.new_borrower();
    let owner = borrower.pubkey();
    let admin = env.admin.pubkey();

    let stranger = env.funded_keypair();
    let wrong_payer = close_position_ix(&owner, &stranger.pubkey());
    assert_hodl_error(send(&mut env.svm, &[wrong_payer], &[&env.admin, &borrower.key]), HodlError::Unauthorized);

    // A separate fee payer, so the sponsor's balance changes only by the refund.
    let fee_payer = env.funded_keypair();
    let rent = env.svm.get_account(&position_pda(&owner)).unwrap().lamports;
    let before = env.svm.get_account(&admin).unwrap().lamports;
    let close = close_position_ix(&owner, &admin);
    send(&mut env.svm, &[close], &[&fee_payer, &borrower.key]).unwrap();
    assert_eq!(env.svm.get_account(&admin).unwrap().lamports, before + rent);
    assert!(env.svm.get_account(&position_pda(&owner)).is_none_or(|a| a.lamports == 0));

    // A position holding collateral can't be closed.
    let other = env.new_borrower();
    env.deposit_collateral(&other, &usdc, ONE_USDC);
    let close = close_position_ix(&other.pubkey(), &admin);
    assert_hodl_error(send(&mut env.svm, &[close], &[&env.admin, &other.key]), HodlError::PositionNotEmpty);
}

#[test]
fn deposit_collateral_rejects_a_foreign_position() {
    // Since the stored-bump change, `take_loan`, `close_position`, `deposit_collateral`,
    // `withdraw_collateral` and `redeem_promo` all read `bump = position.load()?.bump` instead
    // of a bare `bump` — Anchor now compares the account key against
    // `create_program_address([POSITION_SEED, owner.key()], stored_bump)` rather than deriving
    // the bump itself. The account→owner binding this produces is unchanged (the seeds still
    // pin `owner.key()`, the signer), but nothing in the suite ever substituted another owner's
    // `Position` to prove it — every ix builder derives `position: position_pda(owner)`
    // internally, so the substitution case is only reachable by editing the built instruction
    // by hand, as below. `deposit_collateral` stands in for all five of these owner-signer
    // sites: they share the exact same `#[account(seeds = [...], bump = ...)]` shape, so Anchor
    // generates identical constraint code for each.
    let mut env = Env::initialized();
    let usdc = env.list_spl_collateral(6);
    let victim = env.new_borrower();
    let intruder = env.new_borrower();
    let token = env.create_token_account(&usdc, &intruder.pubkey());
    env.mint_to(&usdc, &token, 10 * ONE_USDC);

    let mut ix = deposit_collateral_ix(&intruder.pubkey(), &usdc, &SPL_TOKEN, &token, 10 * ONE_USDC);
    let slot = ix.accounts.iter().position(|a| a.pubkey == position_pda(&intruder.pubkey())).unwrap();
    ix.accounts[slot] = AccountMeta::new(position_pda(&victim.pubkey()), false);
    let result = send(&mut env.svm, &[ix], &[&env.admin, &intruder.key]);
    assert_anchor_error(result, AnchorError::ConstraintSeeds);

    // Neither position was touched by the rejected attempt.
    assert!(!env.position(&victim.pubkey()).has_collateral());
    assert!(!env.position(&intruder.pubkey()).has_collateral());
}
