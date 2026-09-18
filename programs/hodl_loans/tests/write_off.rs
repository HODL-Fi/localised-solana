mod common;

use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

const DAY: i64 = 86_400;
const LOAN: u64 = 700_000 * ONE_CNGN;
/// $0.001 at Pyth exponent -8: 1,000 USDC is then worth $1, under the $5 dust threshold.
const USDC_DUST: i64 = 100_000;

/// A position whose collateral has collapsed to $1 against a 700,000 cNGN loan.
fn dust_collateral() -> (Env, LoanSetup) {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    env.set_pyth_price(&setup.usdc, USDC_DUST, 0);
    (env, setup)
}

#[test]
fn writing_off_clears_the_loan_and_records_the_bad_debt() {
    let (mut env, setup) = dust_collateral();
    env.warp_seconds(73 * DAY);
    env.set_pyth_price(&setup.usdc, USDC_DUST, 0);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    env.write_off(&setup, 0).unwrap();
    // The write-off accrues first, then releases 73 days of lender interest on 700,000 cNGN
    // at 15%, net of the 10% reserve factor.
    let released = 18_900 * ONE_CNGN as u128;

    let market = env.market(&setup.cngn);
    assert_eq!((market.total_borrows, market.lp_rate_product, market.accrued_interest), (0, 0, 0));
    assert_eq!(market.total_bad_debt, LOAN as u128 + released);
    // Nothing was in the reserve, so the lenders absorb all of it.
    assert_eq!(market.protocol_reserve, 0);
    assert_eq!(market.cash, POOL_CNGN - LOAN);
    assert_eq!(market.total_assets().unwrap(), (POOL_CNGN - LOAN) as u128);

    let position = env.position(&setup.borrower.pubkey());
    assert_eq!(position.loans[0], bytemuck::Zeroable::zeroed());
    assert!(!position.has_active_loans());
    // The write-off takes no collateral: the dust stays in the position.
    assert_eq!(position.collateral[0].amount, 1_000 * ONE_USDC);

    // The borrower keeps the cNGN, and the lender's exit is short by the loss.
    assert_eq!(env.token_balance(&setup.borrower_cngn), LOAN);
    env.withdraw(&setup.lender, &setup.cngn, u64::MAX).unwrap();
    assert!(env.token_balance(&setup.lender.token) >= POOL_CNGN - LOAN - 1);
}

#[test]
fn the_reserve_absorbs_the_loss_before_lenders() {
    let (mut env, setup) = Env::loan_ready();
    // One 1,000,000 cNGN loan repaid after 73 days leaves 3,000 cNGN in the reserve.
    env.take_loan(&setup.borrower, &setup, 1_000_000 * ONE_CNGN, 365 * DAY).unwrap();
    env.warp_seconds(73 * DAY);
    env.mint_to(&setup.cngn, &setup.borrower_cngn, 30_000 * ONE_CNGN);
    env.repay(&setup, 0, u64::MAX).unwrap();
    assert_eq!(env.market(&setup.cngn).protocol_reserve, 3_000 * ONE_CNGN);

    // A second, tiny loan goes bad while its collateral is dust: 2,000 cNGN is $1.25,
    // over the $0.90 line that $1 of collateral leaves.
    env.set_pyth_price(&setup.usdc, ONE_DOLLAR, 0);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    env.take_loan(&setup.borrower, &setup, 2_000 * ONE_CNGN, 365 * DAY).unwrap();
    env.set_pyth_price(&setup.usdc, USDC_DUST, 0);
    let assets_before = env.market(&setup.cngn).total_assets().unwrap();

    env.write_off(&setup, 1).unwrap();

    let market = env.market(&setup.cngn);
    // The loss is the 2,000 cNGN of principal (no time passed, so no interest to release).
    assert_eq!(market.total_bad_debt, 2_000 * ONE_CNGN as u128);
    assert_eq!(market.protocol_reserve, 1_000 * ONE_CNGN);
    // Fully covered, so lenders see no change in total assets.
    assert_eq!(market.total_assets().unwrap(), assets_before);
}

#[test]
fn write_off_needs_an_unhealthy_position_with_dust_collateral() {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();

    // Healthy at $1.
    assert_hodl_error(env.write_off(&setup, 0), HodlError::NotLiquidatable);

    // Unhealthy at $0.45, but $450 of collateral is far above the $5 dust threshold.
    env.set_pyth_price(&setup.usdc, 45_000_000, 0);
    assert_hodl_error(env.write_off(&setup, 0), HodlError::WriteOffNotAllowed);

    // Dust at $0.001, and now it can be written off.
    env.set_pyth_price(&setup.usdc, USDC_DUST, 0);
    assert_hodl_error(env.write_off(&setup, 7), HodlError::LoanNotFound);
    env.write_off(&setup, 0).unwrap();
}

#[test]
fn only_the_admin_writes_off() {
    let (mut env, setup) = dust_collateral();
    let stranger = env.funded_keypair();
    let owner = setup.borrower.pubkey();
    let prices = env.price_accounts(&owner);
    let instruction = write_off_loan_ix(&stranger.pubkey(), &owner, &setup.cngn, 0, prices);
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&stranger]), HodlError::Unauthorized);
}

#[test]
fn write_off_prices_the_whole_position() {
    let (mut env, setup) = dust_collateral();
    let owner = setup.borrower.pubkey();
    let admin = env.admin.pubkey();

    let no_prices = write_off_loan_ix(&admin, &owner, &setup.cngn, 0, vec![]);
    assert_hodl_error(send(&mut env.svm, &[no_prices], &[&env.admin]), HodlError::PriceAccountMismatch);

    env.warp_seconds(61);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    assert_hodl_error(env.write_off(&setup, 0), HodlError::StalePrice);
}
