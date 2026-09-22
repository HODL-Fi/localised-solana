mod common;

use anchor_lang::{error::ErrorCode as AnchorError, prelude::AccountMeta};
use common::*;
use hodl_loans::HodlError;
use pyth_solana_receiver_sdk::price_update::VerificationLevel;
use solana_keypair::Keypair;
use solana_signer::Signer;

const DAY: i64 = 86_400;

#[test]
fn a_loan_pays_out_cngn_and_records_fixed_terms() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    env.take_loan(&setup.borrower, &setup, 100_000 * ONE_CNGN, 365 * DAY).unwrap();

    assert_eq!(env.token_balance(&setup.borrower_cngn), 100_000 * ONE_CNGN);
    let position = env.position(&owner);
    let loan = position.loans[0];
    assert_eq!((loan.id, loan.active), (0, 1));
    assert_eq!((loan.principal, loan.original_principal, loan.repaid), (100_000 * ONE_CNGN, 100_000 * ONE_CNGN, 0));
    assert_eq!((loan.originated_at, loan.interest_anchor, loan.tenure_seconds), (env.now(), env.now(), 365 * DAY));
    assert_eq!((loan.rate_bps, loan.penalty_rate_bps, loan.reserve_factor_bps), (1_500, 500, 1_000));
    assert_eq!(position.next_loan_id, 1);
    assert_eq!(position.market, market_pda(&setup.cngn));
    assert_eq!(position.promo_last_activity_at, env.now());

    let market = env.market(&setup.cngn);
    assert_eq!(market.total_borrows, 100_000 * ONE_CNGN);
    assert_eq!(market.cash, POOL_CNGN - 100_000 * ONE_CNGN);
    assert_eq!(market.lp_rate_product, 100_000 * ONE_CNGN as u128 * 1_500 * 9_000);
    assert_eq!(env.token_balance(&market_vault_pda(&setup.cngn)), POOL_CNGN - 100_000 * ONE_CNGN);

    // New market terms apply to new loans only.
    let params = hodl_loans::MarketParams { interest_rate_bps: 2_000, ..default_market_params() };
    let update = update_market_params_ix(&env.admin.pubkey(), &setup.cngn, params);
    send(&mut env.svm, &[update], &[&env.admin]).unwrap();
    env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 30 * DAY).unwrap();
    let position = env.position(&owner);
    assert_eq!((position.loans[0].rate_bps, position.loans[1].rate_bps), (1_500, 2_000));
    assert_eq!((position.loans[1].id, position.next_loan_id), (1, 2));
}

#[test]
fn borrow_limit_is_seventy_percent_of_collateral_at_the_ngn_ask() {
    // $700 limit; each cNGN is valued at $0.000625 + 0.1% = $0.000625625.
    let (mut env, setup) = Env::loan_ready();
    assert_hodl_error(env.take_loan(&setup.borrower, &setup, 1_118_882 * ONE_CNGN, 30 * DAY), HodlError::Unhealthy);
    env.take_loan(&setup.borrower, &setup, 1_118_881 * ONE_CNGN, 30 * DAY).unwrap();
    // Existing debt counts against the limit.
    assert_hodl_error(env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 30 * DAY), HodlError::Unhealthy);
}

#[test]
fn mixed_collateral_is_valued_at_each_assets_bid() {
    let (mut env, setup) = Env::loan_ready();
    let sol = env.list_spl_collateral(9);
    // $150 ± $0.15, so SOL counts at $149.85.
    env.set_pyth_price(&sol, 150 * ONE_DOLLAR, 15_000_000);

    let borrower = env.new_borrower();
    env.deposit_collateral(&borrower, &setup.usdc, 100 * ONE_USDC);
    env.deposit_collateral(&borrower, &sol, 2_000_000_000);
    let cngn_account = env.create_token_account(&setup.cngn, &borrower.pubkey());
    let setup = LoanSetup { borrower_cngn: cngn_account, ..setup };

    // Limit = $70 + $209.79 = $279.79; 447,000 cNGN is $279.65 and 448,000 is $280.28.
    assert_hodl_error(env.take_loan(&borrower, &setup, 448_000 * ONE_CNGN, 30 * DAY), HodlError::Unhealthy);
    env.take_loan(&borrower, &setup, 447_000 * ONE_CNGN, 30 * DAY).unwrap();
}

#[test]
fn amount_tenure_and_pause_rules() {
    let (mut env, setup) = Env::loan_ready();
    let b = &setup.borrower;
    assert_hodl_error(env.take_loan(b, &setup, 999 * ONE_CNGN, 30 * DAY), HodlError::AmountTooSmall);
    assert_hodl_error(env.take_loan(b, &setup, 1_000 * ONE_CNGN, DAY - 1), HodlError::TenureOutOfRange);
    assert_hodl_error(env.take_loan(b, &setup, 1_000 * ONE_CNGN, 365 * DAY + 1), HodlError::TenureOutOfRange);
    env.take_loan(b, &setup, 1_000 * ONE_CNGN, DAY).unwrap();
    env.take_loan(b, &setup, 1_000 * ONE_CNGN, 365 * DAY).unwrap();

    let pause = set_market_paused_ix(&env.guardian.pubkey(), &setup.cngn, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();
    assert_hodl_error(env.take_loan(b, &setup, 1_000 * ONE_CNGN, 30 * DAY), HodlError::MarketPaused);
}

#[test]
fn utilization_cap_and_available_cash() {
    let (mut env, setup) = Env::loan_ready();
    env.deposit_collateral(&setup.borrower, &setup.usdc, 100_000 * ONE_USDC);
    let b = &setup.borrower;
    assert_hodl_error(env.take_loan(b, &setup, POOL_CNGN + 1, 30 * DAY), HodlError::InsufficientCash);
    // 90% of 10,000,000 cNGN.
    assert_hodl_error(env.take_loan(b, &setup, 9_000_000 * ONE_CNGN + 1, 30 * DAY), HodlError::UtilizationCapExceeded);
    env.take_loan(b, &setup, 9_000_000 * ONE_CNGN, 30 * DAY).unwrap();
    assert_hodl_error(env.take_loan(b, &setup, 1_000 * ONE_CNGN, 30 * DAY), HodlError::UtilizationCapExceeded);
}

#[test]
fn a_position_holds_ten_loans() {
    let (mut env, setup) = Env::loan_ready();
    for _ in 0..10 {
        env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 30 * DAY).unwrap();
    }
    assert_hodl_error(env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 30 * DAY), HodlError::NoFreeLoanSlot);
    let ids: Vec<u64> = env.position(&setup.borrower.pubkey()).loans.iter().map(|l| l.id).collect();
    assert_eq!(ids, (0..10).collect::<Vec<u64>>());
}

#[test]
fn price_accounts_must_match_the_positions_slots() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    let usdt = env.list_spl_collateral(6);
    env.set_pyth_price(&usdt, ONE_DOLLAR, 0);
    let good = env.price_accounts(&owner);
    let take = |prices: Vec<AccountMeta>| take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, prices);
    let usdt_pair = price_pairs(&[usdt]);
    let replace = |i: usize, key| {
        let mut prices = good.clone();
        prices[i] = AccountMeta::new_readonly(key, false);
        prices
    };

    let cases = vec![
        ("missing", vec![]),
        ("extra pair", [good.clone(), usdt_pair.clone()].concat()),
        ("another asset", usdt_pair.clone()),
        ("another asset's price", replace(1, pyth_account(&usdt))),
        ("config instead of asset", replace(0, config_pda())),
    ];
    for (name, prices) in cases {
        let result = send(&mut env.svm, &[take(prices)], &[&env.admin, &setup.borrower.key]);
        let err = result.expect_err(name);
        assert!(err.contains(&format!("Custom({})", u32::from(HodlError::PriceAccountMismatch))), "{name}: {err}");
    }

    // A price account not owned by the Pyth receiver.
    let data = env.svm.get_account(&pyth_account(&setup.usdc)).unwrap().data;
    let fake = Keypair::new().pubkey();
    env.set_account_data(&fake, &env.admin.pubkey(), data);
    let instruction = take(replace(1, fake));
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin, &setup.borrower.key]), HodlError::PriceAccountMismatch);

    // The NGN feed must be the market's.
    let mut instruction = take(good.clone());
    // Found by key rather than by index: the account list grows between plans.
    let slot = instruction.accounts.iter().position(|a| a.pubkey == ngn_feed()).unwrap();
    instruction.accounts[slot] = AccountMeta::new_readonly(pyth_account(&setup.usdc), false);
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin, &setup.borrower.key]), HodlError::PriceAccountMismatch);

    send(&mut env.svm, &[take(good)], &[&env.admin, &setup.borrower.key]).unwrap();
}

#[test]
fn stale_uncertain_or_unverified_prices_block_borrowing() {
    let (mut env, setup) = Env::loan_ready();
    let b = &setup.borrower;
    let usdc = setup.usdc;
    let amount = 1_000 * ONE_CNGN;

    // Pyth older than 60 seconds.
    env.warp_seconds(61);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    assert_hodl_error(env.take_loan(b, &setup, amount, 30 * DAY), HodlError::StalePrice);
    env.set_pyth_price(&usdc, ONE_DOLLAR, 0);

    // Switchboard result older than 150 slots.
    let slot = env.svm.get_sysvar::<anchor_lang::prelude::Clock>().slot;
    env.svm.warp_to_slot(slot + 151);
    assert_hodl_error(env.take_loan(b, &setup, amount, 30 * DAY), HodlError::StalePrice);

    // Too few Switchboard samples, then too wide a spread (2.01% > 2%).
    let slot = env.svm.get_sysvar::<anchor_lang::prelude::Clock>().slot;
    env.set_account_data(&ngn_feed(), &switchboard_on_demand::ON_DEMAND_MAINNET_PID, pull_feed_data(NGN_USD, NGN_SPREAD, slot, 2));
    assert_hodl_error(env.take_loan(b, &setup, amount, 30 * DAY), HodlError::StalePrice);
    env.set_ngn_price(NGN_USD, NGN_USD / 10_000 * 201);
    assert_hodl_error(env.take_loan(b, &setup, amount, 30 * DAY), HodlError::PriceConfidenceTooWide);
    env.set_ngn_price(0, 0);
    assert_hodl_error(env.take_loan(b, &setup, amount, 30 * DAY), HodlError::InvalidPrice);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);

    // Pyth confidence 2.01% of price, then a partially verified update, then a non-positive price.
    env.set_pyth_price(&usdc, ONE_DOLLAR, 2_010_000);
    assert_hodl_error(env.take_loan(b, &setup, amount, 30 * DAY), HodlError::PriceConfidenceTooWide);
    let now = env.now();
    let partial = price_update_data(&usdc, ONE_DOLLAR, 0, now, VerificationLevel::Partial { num_signatures: 5 });
    env.set_account_data(&pyth_account(&usdc), &pyth_solana_receiver_sdk::ID, partial);
    assert_hodl_error(env.take_loan(b, &setup, amount, 30 * DAY), HodlError::InvalidPrice);
    env.set_pyth_price(&usdc, 0, 0);
    assert_hodl_error(env.take_loan(b, &setup, amount, 30 * DAY), HodlError::InvalidPrice);

    env.set_pyth_price(&usdc, ONE_DOLLAR, 2_000_000);
    env.take_loan(b, &setup, amount, 30 * DAY).unwrap();
}

#[test]
fn only_active_wallets_with_positions_borrow() {
    let (mut env, setup) = Env::loan_ready();

    // Whitelisted but no position: the empty account is still owned by the system program.
    let lender = setup.lender.key.pubkey();
    let instruction = take_loan_ix(&lender, &setup.cngn, &setup.lender.token, 1_000 * ONE_CNGN, 30 * DAY, vec![]);
    assert_anchor_error(send(&mut env.svm, &[instruction], &[&env.admin, &setup.lender.key]), AnchorError::AccountOwnedByWrongProgram);

    env.blacklist(&setup.borrower.pubkey());
    assert_hodl_error(env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 30 * DAY), HodlError::Blacklisted);
}

#[test]
fn a_position_borrows_from_one_market() {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 30 * DAY).unwrap();

    let other = env.create_mint(MintKind::CngnLike, 6);
    env.create_market_with_promo(&other);
    let lender = env.new_lender(&other, POOL_CNGN);
    env.deposit(&lender, &other, POOL_CNGN).unwrap();

    let other_account = env.create_token_account(&other, &setup.borrower.pubkey());
    let other_setup = LoanSetup { cngn: other, borrower_cngn: other_account, lender, ..setup };
    assert_hodl_error(env.take_loan(&other_setup.borrower, &other_setup, 1_000 * ONE_CNGN, 30 * DAY), HodlError::MarketMismatch);
}

#[test]
fn per_second_accrual_keeps_its_remainder() {
    // 1,000 cNGN at 15% net of a 10% reserve accrues 4.28 raw units a second to lenders.
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 365 * DAY).unwrap();
    let lender = env.new_lender(&setup.cngn, 10 * ONE_CNGN);
    for _ in 0..10 {
        env.warp_seconds(1);
        env.deposit(&lender, &setup.cngn, ONE_CNGN).unwrap();
    }
    // floor(1,000,000,000 × 1,500 × 9,000 × 10 / (10,000² × 31,536,000)) = 42, not 10 × 4.
    assert_eq!(env.market(&setup.cngn).accrued_interest, 42);
}

#[test]
fn a_pinned_price_account_is_the_only_one_accepted() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    let usdc = setup.usdc;
    assert_eq!(env.collateral(&usdc).price_account, pyth_account(&usdc));

    // A second, equally valid update for the same feed, at another address, saying USDC is $2.
    let alternative = Keypair::new().pubkey();
    let now = env.now();
    let data = price_update_data(&usdc, 2 * ONE_DOLLAR, 0, now, VerificationLevel::Full);
    env.set_account_data(&alternative, &pyth_solana_receiver_sdk::ID, data);
    let prices = vec![
        AccountMeta::new_readonly(collateral_pda(&usdc), false),
        AccountMeta::new_readonly(alternative, false),
    ];
    let take = |prices: Vec<AccountMeta>| {
        take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 2_000_000 * ONE_CNGN, 30 * DAY, prices)
    };
    let pinned = send(&mut env.svm, &[take(prices.clone())], &[&env.admin, &setup.borrower.key]);
    assert_hodl_error(pinned, HodlError::PriceAccountMismatch);

    // Unpinning accepts it, and 1,000 USDC at the chosen $2 backs a loan the real price would not.
    let unpinned = hodl_loans::CollateralParams {
        price_account: anchor_lang::prelude::Pubkey::default(),
        ..default_collateral_params(&usdc)
    };
    let update = update_collateral_params_ix(&env.admin.pubkey(), &usdc, unpinned);
    send(&mut env.svm, &[update], &[&env.admin]).unwrap();
    send(&mut env.svm, &[take(prices)], &[&env.admin, &setup.borrower.key]).unwrap();
    assert_eq!(env.token_balance(&setup.borrower_cngn), 2_000_000 * ONE_CNGN);
}

#[test]
fn the_health_walk_rejects_a_collateral_asset_at_a_forged_address() {
    // The walk admits an asset by owner, discriminator and stored `mint`. Those narrow it to
    // "a CollateralAsset this program created for this mint" — but only because listing is the
    // sole creation path, which is an argument about the whole program rather than about this
    // account. Since Plan 4 the same account's `kind` also decides how many accounts the walk
    // consumes, so more rests on it. Re-deriving the PDA makes the argument local, and this is
    // the test that fails if someone removes it.
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();

    // A copy of the real asset with a far more generous LTV, planted program-owned at an
    // unrelated address. `mint` and `bump` are the real ones, so every check except the
    // re-derivation passes: the owner is this program, the discriminator deserializes, and
    // `asset.mint == slot.mint` holds.
    let real: hodl_loans::CollateralAsset = env.fetch(&collateral_pda(&setup.usdc));
    let forged = hodl_loans::CollateralAsset { ltv_bps: 9_000, ..real };
    let forged_key = Keypair::new().pubkey();
    env.set_account_data(&forged_key, &hodl_loans::ID, collateral_asset_bytes(&forged));

    let mut prices = env.price_accounts(&owner);
    let real_key = collateral_pda(&setup.usdc);
    let mut swapped = 0;
    for meta in prices.iter_mut() {
        if meta.pubkey == real_key {
            meta.pubkey = forged_key;
            swapped += 1;
        }
    }
    assert_eq!(swapped, 1, "the collateral asset must appear once for the swap to mean anything");

    let borrow = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 100_000 * ONE_CNGN, 30 * DAY, prices);
    assert_hodl_error(
        send(&mut env.svm, &[borrow], &[&env.admin, &setup.borrower.key]),
        HodlError::PriceAccountMismatch,
    );

    // The same borrow against the real asset succeeds, so the rejection is about the forged
    // address and not about the amount.
    env.take_loan(&setup.borrower, &setup, 100_000 * ONE_CNGN, 30 * DAY).unwrap();
}

#[test]
fn the_ngn_feed_must_be_owned_by_the_switchboard_program() {
    // `Market::ngn_feed` is a bare `Pubkey` the admin sets, with no constraint behind it. The
    // address check alone therefore proves only that the caller passed the account the admin
    // named — not that the account is a Switchboard feed. A discriminator is eight bytes anyone
    // can write, so without the owner check a mis-set `ngn_feed` turns 3.2 KB of arbitrary data
    // into a price.
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();

    // Same bytes the real harness writes, and a plausible-looking owner that is not Switchboard.
    env.set_ngn_price_owned_by(&hodl_loans::ID, NGN_USD, NGN_SPREAD);

    let prices = env.price_accounts(&owner);
    let borrow = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 100_000 * ONE_CNGN, 30 * DAY, prices);
    assert_hodl_error(
        send(&mut env.svm, &[borrow], &[&env.admin, &setup.borrower.key]),
        HodlError::PriceAccountMismatch,
    );

    // Restoring the real owner, with the same data, lets the identical borrow through — so the
    // rejection is about the owner and nothing else.
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    env.take_loan(&setup.borrower, &setup, 100_000 * ONE_CNGN, 30 * DAY).unwrap();
}

#[test]
fn a_borrow_paused_asset_lends_no_borrowing_power() {
    let (mut env, setup) = Env::loan_ready();
    let pause = set_collateral_borrow_paused_ix(&env.guardian.pubkey(), &setup.usdc, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();

    // The $700 limit is gone entirely, not merely reduced: the market's smallest permitted
    // loan is refused. Anything under `min_loan_amount` would trip `AmountTooSmall` first and
    // prove nothing about health.
    assert_hodl_error(env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 30 * DAY), HodlError::Unhealthy);
    // Depositing more of the asset buys none of it back, and is still permitted — the pause
    // stops borrowing against the asset, not holding it.
    env.deposit_collateral(&setup.borrower, &setup.usdc, 1_000 * ONE_USDC);
    assert_eq!(env.position(&setup.borrower.pubkey()).collateral[0].amount, 2_000 * ONE_USDC);
    assert_hodl_error(env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 30 * DAY), HodlError::Unhealthy);

    // Lifting it restores the limit over everything deposited: 2,000 USDC is twice the
    // 1,118,881 cNGN of `borrow_limit_is_seventy_percent_of_collateral_at_the_ngn_ask`.
    let unpause = set_collateral_borrow_paused_ix(&env.admin.pubkey(), &setup.usdc, false);
    send(&mut env.svm, &[unpause], &[&env.admin]).unwrap();
    assert_hodl_error(
        env.take_loan(&setup.borrower, &setup, 2_237_763 * ONE_CNGN, 30 * DAY),
        HodlError::Unhealthy,
    );
    env.take_loan(&setup.borrower, &setup, 2_237_762 * ONE_CNGN, 30 * DAY).unwrap();
}
