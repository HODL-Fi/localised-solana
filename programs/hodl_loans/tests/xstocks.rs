mod common;

use common::*;
use hodl_loans::{CollateralKind, HodlError};
use anchor_lang::prelude::{AccountMeta, Pubkey};
use solana_signer::Signer;
use spl_token_2022_interface::state::AccountState;

const DAY: i64 = 86_400;
/// 10 shares of a $200 stock at 50% LTV back $1,000, which is 1,598,401 cNGN at the ask.
const CEILING: u64 = 1_598_401 * ONE_CNGN;
/// The same position once a 1.5 multiplier makes the balance 15 shares.
const CEILING_AT_1_5: u64 = 2_397_602 * ONE_CNGN;

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
fn an_xstock_mint_without_a_multiplier_is_rejected() {
    let mut env = Env::initialized();
    let admin = env.admin.pubkey();

    // An xStock without a ScaledUiAmount extension would be listable but unpriceable: an XStock
    // with no multiplier is not an xStock. Listing is the one moment we can reject it cheaply.
    let no_multiplier = env.create_mint(MintKind::CngnLike, XSTOCK_DECIMALS);
    let listing = list_collateral_ix(&admin, &no_multiplier, &TOKEN_2022, xstock_collateral_params(&no_multiplier), CollateralKind::XStock);
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

/// The freeze authority's blocklist lever (spec §14, §20 item 3) must not seal a live position.
/// `DefaultAccountState` governs the state *new* accounts are initialized in; the vault, the
/// borrower's account and the liquidator's already exist, so flipping it to `Frozen` changes
/// nothing about whether their transfers are legal. Enforcing it on the way out would turn an
/// expected issuer action into a permanent trap: withdraw, liquidate and sweep would all revert,
/// `write_off_loan` needs the collateral to be dust, and `delist_collateral` needs an empty vault.
#[test]
fn a_frozen_default_after_listing_does_not_trap_the_collateral() {
    let (mut env, setup) = Env::loan_ready();
    let stock = env.list_xstock_collateral(200);
    let borrower = env.new_borrower();
    let owner = borrower.pubkey();
    let token = env.deposit_collateral(&borrower, &stock, 10 * ONE_XSTOCK);
    let borrower_cngn = env.create_token_account(&setup.cngn, &owner);
    let setup = LoanSetup { borrower, borrower_cngn, ..setup };

    // 10 xStock at $200 backs 1,000,000 cNGN ($625.625 at the NGN ask), under the 50% LTV limit.
    env.take_loan(&setup.borrower, &setup, 1_000_000 * ONE_CNGN, 365 * DAY).unwrap();

    // A donation to sweep later, and the liquidator's account, both created before the flip.
    env.mint_to(&stock, &collateral_vault_pda(&stock), 7 * ONE_XSTOCK);
    let liquidator = env.new_liquidator(&setup.cngn, 1_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&stock, &liquidator.pubkey());
    let destination = env.create_token_account(&stock, &env.treasury.pubkey());

    // The issuer switches on blocklist-style compliance.
    env.set_default_account_state(&stock, AccountState::Frozen);

    // The borrower still gets collateral out: 9 shares at $200 still back the $625.625 debt.
    let withdraw =
        withdraw_collateral_ix(&owner, &stock, &TOKEN_2022, &token, Some(&setup.cngn), ONE_XSTOCK, env.price_accounts(&owner));
    env.sponsored(withdraw, &setup.borrower.key).unwrap();
    assert_eq!(env.token_balance(&token), ONE_XSTOCK);

    // A liquidator still seizes: $720 of collateral × the 75% threshold is under the debt.
    env.set_pyth_price(&stock, 80 * ONE_DOLLAR, 0);
    env.liquidate(&liquidator, &setup, &stock, &seized_to, 0, 100_000 * ONE_CNGN).unwrap();
    assert!(env.token_balance(&seized_to) > 0);

    // And the admin still sweeps the donation out.
    let sweep = sweep_collateral_excess_ix(&env.admin.pubkey(), &stock, &TOKEN_2022, &destination);
    send(&mut env.svm, &[sweep], &[&env.admin]).unwrap();
    assert_eq!(env.token_balance(&destination), 7 * ONE_XSTOCK);
}

/// The mirror of the test above: the entry policy still holds the `DefaultAccountState` line.
/// `listing_rejects_a_hook_program_or_a_frozen_default` covers a fresh listing; this covers the
/// other entry point, a deposit into an asset whose mint has flipped since it was listed.
#[test]
fn a_frozen_default_after_listing_still_blocks_new_deposits() {
    let mut env = Env::initialized();
    let stock = env.list_xstock_collateral(200);
    let borrower = env.new_borrower();
    let token = env.deposit_collateral(&borrower, &stock, 10 * ONE_XSTOCK);
    env.mint_to(&stock, &token, ONE_XSTOCK);
    let owner = borrower.pubkey();

    env.set_default_account_state(&stock, AccountState::Frozen);

    let deposit = deposit_collateral_ix(&owner, &stock, &TOKEN_2022, &token, ONE_XSTOCK);
    assert_hodl_error(env.sponsored(deposit, &borrower.key), HodlError::UnsupportedMintExtension);

    // Reverting the flag lets new exposure in again.
    env.set_default_account_state(&stock, AccountState::Initialized);
    let deposit = deposit_collateral_ix(&owner, &stock, &TOKEN_2022, &token, ONE_XSTOCK);
    env.sponsored(deposit, &borrower.key).unwrap();
    assert_eq!(env.position(&owner).collateral[0].amount, 11 * ONE_XSTOCK);
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

#[test]
fn the_multiplier_scales_borrowing_power() {
    let (mut env, setup) = Env::loan_ready();
    let stock = env.list_xstock_collateral(200);
    let borrower = env.new_borrower();
    env.deposit_collateral(&borrower, &stock, 10 * ONE_XSTOCK);
    let borrower_cngn = env.create_token_account(&setup.cngn, &borrower.pubkey());
    let setup = LoanSetup { borrower, borrower_cngn, ..setup };

    assert_hodl_error(env.take_loan(&setup.borrower, &setup, CEILING + ONE_CNGN, 30 * DAY), HodlError::Unhealthy);

    // A dividend reinvestment lifts the multiplier to 1.5: the same balance is 15 shares.
    let now = env.now();
    env.set_multiplier(&stock, 1.5, now);
    assert_hodl_error(env.take_loan(&setup.borrower, &setup, CEILING_AT_1_5 + ONE_CNGN, 30 * DAY), HodlError::Unhealthy);
    env.take_loan(&setup.borrower, &setup, CEILING_AT_1_5, 30 * DAY).unwrap();
    assert_eq!(env.token_balance(&setup.borrower_cngn), CEILING_AT_1_5);
}

#[test]
fn a_scheduled_multiplier_takes_effect_on_its_timestamp() {
    let (mut env, setup) = Env::loan_ready();
    let stock = env.list_xstock_collateral(200);
    let borrower = env.new_borrower();
    env.deposit_collateral(&borrower, &stock, 10 * ONE_XSTOCK);
    let borrower_cngn = env.create_token_account(&setup.cngn, &borrower.pubkey());
    let setup = LoanSetup { borrower, borrower_cngn, ..setup };

    // Scheduled for tomorrow: today's borrowing power is still the old multiplier's.
    let effective_at = env.now() + DAY;
    env.set_multiplier(&stock, 1.5, effective_at);
    assert_hodl_error(env.take_loan(&setup.borrower, &setup, CEILING + ONE_CNGN, 30 * DAY), HodlError::Unhealthy);

    env.warp_seconds(DAY);
    env.set_pyth_price(&stock, 200 * ONE_DOLLAR, 0);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    env.take_loan(&setup.borrower, &setup, CEILING_AT_1_5, 30 * DAY).unwrap();
}

#[test]
fn a_foreign_mint_with_a_generous_multiplier_does_not_inflate_collateral() {
    let (mut env, setup) = Env::loan_ready();
    let stock = env.list_xstock_collateral(200);
    let foreign = env.list_xstock_collateral(200);
    // The foreign mint's own multiplier is as generous as the extension allows.
    let now = env.now();
    env.set_multiplier(&foreign, 1_000_000.0, now);

    let borrower = env.new_borrower();
    env.deposit_collateral(&borrower, &stock, 10 * ONE_XSTOCK);
    let borrower_cngn = env.create_token_account(&setup.cngn, &borrower.pubkey());
    let setup = LoanSetup { borrower, borrower_cngn, ..setup };
    let owner = setup.borrower.pubkey();

    // The only used slot is `stock`'s: its correct (asset, price) pair, but a stranger's mint
    // stands in for the third account.
    let mut prices = price_pairs(&[stock]);
    prices.push(AccountMeta::new_readonly(foreign, false));
    let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, prices);
    let result = send(&mut env.svm, &[ixn], &[&env.admin, &setup.borrower.key]);
    assert_hodl_error(result, HodlError::PriceAccountMismatch);
}

#[test]
fn an_xstock_slot_rejects_the_two_account_standard_shape() {
    let (mut env, setup) = Env::loan_ready();
    let stock = env.list_xstock_collateral(200);
    let borrower = env.new_borrower();
    env.deposit_collateral(&borrower, &stock, 10 * ONE_XSTOCK);
    let borrower_cngn = env.create_token_account(&setup.cngn, &borrower.pubkey());
    let setup = LoanSetup { borrower, borrower_cngn, ..setup };
    let owner = setup.borrower.pubkey();

    // Only the (asset, price) pair: the mint account an XStock slot needs is missing.
    let prices = price_pairs(&[stock]);
    let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, prices);
    let result = send(&mut env.svm, &[ixn], &[&env.admin, &setup.borrower.key]);
    assert_hodl_error(result, HodlError::PriceAccountMismatch);
}

#[test]
fn liquidating_an_xstock_seizes_at_the_display_price() {
    let (mut env, setup) = Env::loan_ready();
    let stock = env.list_xstock_collateral(200);
    let borrower = env.new_borrower();
    env.deposit_collateral(&borrower, &stock, 10 * ONE_XSTOCK);
    let borrower_cngn = env.create_token_account(&setup.cngn, &borrower.pubkey());
    let setup = LoanSetup { borrower, borrower_cngn, ..setup };

    // $2,000 of collateral backs 1,500,000 cNGN ($937.50 at the plain NGN price); a crash to
    // $80 a share puts the debt over the 75% line ($600).
    env.take_loan(&setup.borrower, &setup, 1_500_000 * ONE_CNGN, 365 * DAY).unwrap();
    env.set_pyth_price(&stock, 80 * ONE_DOLLAR, 0);

    let liquidator = env.new_liquidator(&setup.cngn, 10_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&stock, &liquidator.pubkey());
    // 160,000 cNGN is $100; with the 10% bonus that seizes $110 of stock at $80 a share.
    env.liquidate(&liquidator, &setup, &stock, &seized_to, 0, 160_000 * ONE_CNGN).unwrap();
    assert_eq!(env.token_balance(&seized_to), 137_500_000);

    // A 0.5 multiplier halves what a raw unit is worth, so the same $110 costs twice the units.
    let now = env.now();
    env.set_multiplier(&stock, 0.5, now);
    env.liquidate(&liquidator, &setup, &stock, &seized_to, 0, 160_000 * ONE_CNGN).unwrap();
    assert_eq!(env.token_balance(&seized_to), 137_500_000 + 275_000_000);
}

/// A position mixing kinds, in the order `Standard, XStock, Standard`. Every other xStock test
/// holds a single kind, so the cursor's *variable* stride — the new control flow in this plan —
/// is never exercised across kinds: a fixed stride of two or of three would walk the same
/// accounts for a uniform position and only diverge here.
///
/// Slot 0 is 1,000 USDC at $1 and 70% LTV ($700 of limit); slot 1 is 10 xStock at $200 and 50%
/// LTV ($1,000); slot 2 is 500 Token-2022 units at $1 and 70% LTV ($350). The combined limit is
/// $2,050, which is 3,276,723 cNGN at the ask.
#[test]
fn a_mixed_standard_and_xstock_position_values_every_slot() {
    const MIXED_CEILING: u64 = 3_276_723 * ONE_CNGN;

    let (mut env, setup) = Env::loan_ready();
    let stock = env.list_xstock_collateral(200);
    let plain = env.list_t22_collateral(6);
    let borrower = env.new_borrower();
    let owner = borrower.pubkey();
    env.deposit_collateral(&borrower, &setup.usdc, 1_000 * ONE_USDC);
    env.deposit_collateral(&borrower, &stock, 10 * ONE_XSTOCK);
    env.deposit_collateral(&borrower, &plain, 500 * ONE_USDC);
    let borrower_cngn = env.create_token_account(&setup.cngn, &owner);
    let setup = LoanSetup { borrower, borrower_cngn, ..setup };

    // Two accounts, three, two — seven in all, not six and not nine.
    let position = env.position(&owner);
    assert_eq!([position.collateral[0].mint, position.collateral[1].mint, position.collateral[2].mint], [setup.usdc, stock, plain]);
    assert_eq!(env.price_accounts(&owner).len(), 7);

    // The uniform two-per-slot shape lands the xStock slot's third account on the *next* slot's
    // `CollateralAsset`, which is not its mint.
    let uniform = price_pairs(&[setup.usdc, stock, plain]);
    let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, uniform);
    assert_hodl_error(send(&mut env.svm, &[ixn], &[&env.admin, &setup.borrower.key]), HodlError::PriceAccountMismatch);

    // With the right shape the three slots sum to exactly $2,050 of borrowing power.
    assert_hodl_error(env.take_loan(&setup.borrower, &setup, MIXED_CEILING + ONE_CNGN, 30 * DAY), HodlError::Unhealthy);
    env.take_loan(&setup.borrower, &setup, MIXED_CEILING, 30 * DAY).unwrap();
    assert_eq!(env.token_balance(&setup.borrower_cngn), MIXED_CEILING);
}

/// `withdraw_collateral`'s health path with an active loan against an xStock. Both existing
/// xStock withdraw tests pass `None` for the market and no price accounts — the no-loan path —
/// so the three-account shape its doc comment describes was never actually walked here.
#[test]
fn withdrawing_against_an_active_loan_prices_the_xstock_mint() {
    let (mut env, setup) = Env::loan_ready();
    let stock = env.list_xstock_collateral(200);
    let borrower = env.new_borrower();
    let owner = borrower.pubkey();
    let token = env.deposit_collateral(&borrower, &stock, 10 * ONE_XSTOCK);
    let borrower_cngn = env.create_token_account(&setup.cngn, &owner);
    let setup = LoanSetup { borrower, borrower_cngn, ..setup };

    // 800,000 cNGN is $500.50 at the ask, against $1,000 of limit.
    env.take_loan(&setup.borrower, &setup, 800_000 * ONE_CNGN, 365 * DAY).unwrap();

    // The two-account shape is short the mint the slot needs.
    let short = withdraw_collateral_ix(&owner, &stock, &TOKEN_2022, &token, Some(&setup.cngn), 3 * ONE_XSTOCK, price_pairs(&[stock]));
    assert_hodl_error(env.sponsored(short, &setup.borrower.key), HodlError::PriceAccountMismatch);

    // 7 shares left is $1,400 of collateral, $700 of limit — still over the $500.50 debt.
    let ok = withdraw_collateral_ix(&owner, &stock, &TOKEN_2022, &token, Some(&setup.cngn), 3 * ONE_XSTOCK, env.price_accounts(&owner));
    env.sponsored(ok, &setup.borrower.key).unwrap();
    assert_eq!(env.token_balance(&token), 3 * ONE_XSTOCK);

    // 4 shares would be $800, $400 of limit — under the debt.
    let too_much = withdraw_collateral_ix(&owner, &stock, &TOKEN_2022, &token, Some(&setup.cngn), 3 * ONE_XSTOCK, env.price_accounts(&owner));
    assert_hodl_error(env.sponsored(too_much, &setup.borrower.key), HodlError::Unhealthy);

    // The multiplier is read from the mint on this path too: at 1.5 the same 4 shares are 6
    // display tokens, $1,200 of collateral and $600 of limit, and the withdrawal goes through.
    let now = env.now();
    env.set_multiplier(&stock, 1.5, now);
    let now_ok = withdraw_collateral_ix(&owner, &stock, &TOKEN_2022, &token, Some(&setup.cngn), 3 * ONE_XSTOCK, env.price_accounts(&owner));
    env.sponsored(now_ok, &setup.borrower.key).unwrap();
    assert_eq!(env.position(&owner).collateral[0].amount, 4 * ONE_XSTOCK);
}

/// `write_off_loan` against an xStock. Its doc comment claims the three-account shape; nothing
/// pinned it, and a write-off is the only escape from a position whose collateral is dust.
#[test]
fn writing_off_an_xstock_position_prices_the_mint() {
    let (mut env, setup) = Env::loan_ready();
    let stock = env.list_xstock_collateral(200);
    let borrower = env.new_borrower();
    let owner = borrower.pubkey();
    env.deposit_collateral(&borrower, &stock, 10 * ONE_XSTOCK);
    let borrower_cngn = env.create_token_account(&setup.cngn, &owner);
    let setup = LoanSetup { borrower, borrower_cngn, ..setup };

    env.take_loan(&setup.borrower, &setup, 700_000 * ONE_CNGN, 365 * DAY).unwrap();
    // $0.001 a share: 10 shares is $0.01, under the market's $5 dust threshold.
    env.set_pyth_price(&stock, 100_000, 0);

    // The two-account shape reaches the end of the supplied accounts with a slot still to value.
    let admin = env.admin.pubkey();
    let short = write_off_loan_ix(&admin, &owner, &setup.cngn, 0, price_pairs(&[stock]));
    assert_hodl_error(send(&mut env.svm, &[short], &[&env.admin]), HodlError::PriceAccountMismatch);

    env.write_off(&setup, 0).unwrap();
    assert_eq!(env.market(&setup.cngn).total_bad_debt, 700_000 * ONE_CNGN as u128);
    // The write-off clears the debt and leaves the dust in the position.
    assert!(!env.position(&owner).has_active_loans());
    assert_eq!(env.position(&owner).collateral[0].amount, 10 * ONE_XSTOCK);
}

/// A multiplier past `MAX_MULTIPLIER` reaching a health check as `InvalidPrice`. `scale_multiplier`
/// unit-tests its own rejections, but nothing drove one through the program — and this is the
/// failure mode that argues against tightening the cap: an out-of-range multiplier does not
/// merely undervalue the asset, it seals every priced path against the position.
///
/// `a_foreign_mint_with_a_generous_multiplier_does_not_inflate_collateral` uses `1_000_000.0`,
/// which is exactly the cap and passes the `<=` bound, so it never reaches this branch.
#[test]
fn a_multiplier_above_the_cap_fails_every_priced_path() {
    let (mut env, setup) = Env::loan_ready();
    let stock = env.list_xstock_collateral(200);
    let borrower = env.new_borrower();
    let owner = borrower.pubkey();
    let token = env.deposit_collateral(&borrower, &stock, 10 * ONE_XSTOCK);
    let borrower_cngn = env.create_token_account(&setup.cngn, &owner);
    let setup = LoanSetup { borrower, borrower_cngn, ..setup };
    env.take_loan(&setup.borrower, &setup, 500_000 * ONE_CNGN, 365 * DAY).unwrap();

    // Twice `MAX_MULTIPLIER`, which the mint accepts and the program will not.
    let now = env.now();
    env.set_multiplier(&stock, 2_000_000.0, now);

    assert_hodl_error(env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 30 * DAY), HodlError::InvalidPrice);
    let withdraw = withdraw_collateral_ix(&owner, &stock, &TOKEN_2022, &token, Some(&setup.cngn), ONE_XSTOCK, env.price_accounts(&owner));
    assert_hodl_error(env.sponsored(withdraw, &setup.borrower.key), HodlError::InvalidPrice);

    // The way out is the unpriced one: repay, then withdraw without a market.
    env.mint_to(&setup.cngn, &setup.borrower_cngn, 100_000 * ONE_CNGN);
    env.repay(&setup, 0, u64::MAX).unwrap();
    let withdraw = withdraw_collateral_ix(&owner, &stock, &TOKEN_2022, &token, None, 10 * ONE_XSTOCK, vec![]);
    env.sponsored(withdraw, &setup.borrower.key).unwrap();
    assert_eq!(env.token_balance(&token), 10 * ONE_XSTOCK);
}
