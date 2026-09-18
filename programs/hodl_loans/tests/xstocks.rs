mod common;

use common::*;
use hodl_loans::{CollateralKind, HodlError};
use anchor_lang::prelude::Pubkey;
use solana_signer::Signer;
use spl_token_2022_interface::state::AccountState;

const DAY: i64 = 86_400;

#[test]
fn a_live_shaped_xstock_lists_as_xstock_only() {
    let mut env = Env::initialized();
    let mint = env.create_mint(MintKind::XStock, XSTOCK_DECIMALS);
    let admin = env.admin.pubkey();

    // The same mint is not acceptable as a Standard asset: metadata only, there.
    let as_standard = list_collateral_ix(&admin, &mint, &TOKEN_2022, default_collateral_params(&mint), CollateralKind::Standard);
    assert_hodl_error(send(&mut env.svm, &[as_standard], &[&env.admin]), HodlError::UnsupportedMintExtension);

    let as_xstock = list_collateral_ix(&admin, &mint, &TOKEN_2022, xstock_collateral_params(&mint), CollateralKind::XStock);
    send(&mut env.svm, &[as_xstock], &[&env.admin]).unwrap();

    let asset = env.collateral(&mint);
    assert_eq!(asset.kind, CollateralKind::XStock);
    assert_eq!(asset.decimals, XSTOCK_DECIMALS);
    assert_eq!((asset.ltv_bps, asset.liquidation_threshold_bps, asset.liquidation_bonus_bps), (5_000, 7_500, 1_000));
}

#[test]
fn listing_rejects_a_hook_program_or_a_frozen_default() {
    let mut env = Env::initialized();
    let admin = env.admin.pubkey();

    // A transfer hook that names a program would run issuer code inside every transfer.
    let hooked = env.create_mint(MintKind::XStock, XSTOCK_DECIMALS);
    env.set_transfer_hook(&hooked, Some(Pubkey::new_unique()));
    let listing = list_collateral_ix(&admin, &hooked, &TOKEN_2022, xstock_collateral_params(&hooked), CollateralKind::XStock);
    assert_hodl_error(send(&mut env.svm, &[listing], &[&env.admin]), HodlError::UnsupportedMintExtension);

    // Frozen by default would freeze the collateral vault we are about to create.
    let frozen = env.create_mint(MintKind::XStock, XSTOCK_DECIMALS);
    env.set_default_account_state(&frozen, AccountState::Frozen);
    let listing = list_collateral_ix(&admin, &frozen, &TOKEN_2022, xstock_collateral_params(&frozen), CollateralKind::XStock);
    assert_hodl_error(send(&mut env.svm, &[listing], &[&env.admin]), HodlError::UnsupportedMintExtension);

    // A transfer fee is outside the set whatever the kind claims.
    let fee = env.create_mint(MintKind::TransferFee, 6);
    let listing = list_collateral_ix(&admin, &fee, &TOKEN_2022, xstock_collateral_params(&fee), CollateralKind::XStock);
    assert_hodl_error(send(&mut env.svm, &[listing], &[&env.admin]), HodlError::UnsupportedMintExtension);
}

#[test]
fn an_issuer_pause_blocks_transfers_of_that_asset() {
    let mut env = Env::initialized();
    let stock = env.list_xstock_collateral(200);
    let borrower = env.new_borrower();
    let token = env.deposit_collateral(&borrower, &stock, 10 * ONE_XSTOCK);
    env.mint_to(&stock, &token, ONE_XSTOCK);
    let owner = borrower.pubkey();

    env.set_mint_paused(&stock, true);
    let deposit = deposit_collateral_ix(&owner, &stock, &TOKEN_2022, &token, ONE_XSTOCK);
    assert!(env.sponsored(deposit, &borrower.key).is_err());
    let withdraw = withdraw_collateral_ix(&owner, &stock, &TOKEN_2022, &token, None, ONE_XSTOCK, vec![]);
    assert!(env.sponsored(withdraw, &borrower.key).is_err());

    // The pause is the issuer's, not ours: resuming restores both directions.
    env.set_mint_paused(&stock, false);
    let withdraw = withdraw_collateral_ix(&owner, &stock, &TOKEN_2022, &token, None, ONE_XSTOCK, vec![]);
    env.sponsored(withdraw, &borrower.key).unwrap();
    assert_eq!(env.token_balance(&token), 2 * ONE_XSTOCK);
}

#[test]
fn a_hook_switched_on_after_listing_stops_transfers_cleanly() {
    let mut env = Env::initialized();
    let stock = env.list_xstock_collateral(200);
    let borrower = env.new_borrower();
    let token = env.deposit_collateral(&borrower, &stock, 10 * ONE_XSTOCK);
    env.mint_to(&stock, &token, ONE_XSTOCK);
    let owner = borrower.pubkey();

    env.set_transfer_hook(&stock, Some(Pubkey::new_unique()));

    let deposit = deposit_collateral_ix(&owner, &stock, &TOKEN_2022, &token, ONE_XSTOCK);
    assert_hodl_error(env.sponsored(deposit, &borrower.key), HodlError::UnsupportedMintExtension);
    let withdraw = withdraw_collateral_ix(&owner, &stock, &TOKEN_2022, &token, None, ONE_XSTOCK, vec![]);
    assert_hodl_error(env.sponsored(withdraw, &borrower.key), HodlError::UnsupportedMintExtension);

    // Clearing it again lets the collateral out.
    env.set_transfer_hook(&stock, None);
    let withdraw = withdraw_collateral_ix(&owner, &stock, &TOKEN_2022, &token, None, ONE_XSTOCK, vec![]);
    env.sponsored(withdraw, &borrower.key).unwrap();
}

#[test]
fn a_hook_switched_on_after_listing_blocks_the_admin_sweep() {
    let mut env = Env::initialized();
    let stock = env.list_xstock_collateral(200);
    let admin = env.admin.pubkey();
    let treasury = env.treasury.pubkey();
    let destination = env.create_token_account(&stock, &treasury);

    // A donation sits in the vault with nothing recorded as deposited, the same shape as
    // `collateral.rs::collateral_sweep_moves_only_donations`.
    env.mint_to(&stock, &collateral_vault_pda(&stock), 7 * ONE_XSTOCK);

    env.set_transfer_hook(&stock, Some(Pubkey::new_unique()));
    let sweep = sweep_collateral_excess_ix(&admin, &stock, &TOKEN_2022, &destination);
    assert_hodl_error(
        send(&mut env.svm, std::slice::from_ref(&sweep), &[&env.admin]),
        HodlError::UnsupportedMintExtension,
    );

    // Clearing the hook lets the donation through.
    env.set_transfer_hook(&stock, None);
    send(&mut env.svm, &[sweep], &[&env.admin]).unwrap();
    assert_eq!(env.token_balance(&destination), 7 * ONE_XSTOCK);
}

#[test]
fn a_hook_switched_on_after_listing_blocks_the_liquidation_seizure() {
    let (mut env, setup) = Env::loan_ready();
    let stock = env.list_xstock_collateral(200);
    let borrower = env.new_borrower();
    let owner = borrower.pubkey();
    env.deposit_collateral(&borrower, &stock, 10 * ONE_XSTOCK);
    let borrower_cngn = env.create_token_account(&setup.cngn, &owner);
    let setup = LoanSetup { borrower, borrower_cngn, ..setup };

    // 10 xStock at $200 backs 1,000,000 cNGN ($625.625 at the NGN ask), under the 50% LTV limit.
    env.take_loan(&setup.borrower, &setup, 1_000_000 * ONE_CNGN, 365 * DAY).unwrap();
    // Crash to $80: $800 of collateral × the 75% threshold is $600, under the $625.625 debt.
    env.set_pyth_price(&stock, 80 * ONE_DOLLAR, 0);

    let liquidator = env.new_liquidator(&setup.cngn, 1_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&stock, &liquidator.pubkey());

    env.set_transfer_hook(&stock, Some(Pubkey::new_unique()));
    let hooked = env.liquidate(&liquidator, &setup, &stock, &seized_to, 0, 100_000 * ONE_CNGN);
    assert_hodl_error(hooked, HodlError::UnsupportedMintExtension);

    // Clearing the hook lets the seizure through.
    env.set_transfer_hook(&stock, None);
    env.liquidate(&liquidator, &setup, &stock, &seized_to, 0, 100_000 * ONE_CNGN).unwrap();
    assert!(env.token_balance(&seized_to) > 0);
}

#[test]
fn the_permanent_delegate_can_empty_the_vault() {
    let mut env = Env::initialized();
    let stock = env.list_xstock_collateral(200);
    let borrower = env.new_borrower();
    let token = env.deposit_collateral(&borrower, &stock, 10 * ONE_XSTOCK);
    let vault = collateral_vault_pda(&stock);

    // The issuer burns half the custody vault out from under the protocol.
    env.delegate_burn(&stock, &vault, 5 * ONE_XSTOCK);

    // The program's books still say 10 shares; the vault holds 5.
    assert_eq!(env.collateral(&stock).total_deposited, 10 * ONE_XSTOCK);
    assert_eq!(env.token_balance(&vault), 5 * ONE_XSTOCK);
    assert_eq!(env.position(&borrower.pubkey()).collateral[0].amount, 10 * ONE_XSTOCK);

    // Withdrawals drain what is left and then fail in the token program, not in our accounting.
    let withdraw = withdraw_collateral_ix(&borrower.pubkey(), &stock, &TOKEN_2022, &token, None, 5 * ONE_XSTOCK, vec![]);
    env.sponsored(withdraw, &borrower.key).unwrap();
    let withdraw = withdraw_collateral_ix(&borrower.pubkey(), &stock, &TOKEN_2022, &token, None, ONE_XSTOCK, vec![]);
    assert!(env.sponsored(withdraw, &borrower.key).is_err());
}
