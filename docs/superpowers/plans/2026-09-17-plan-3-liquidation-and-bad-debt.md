# Plan 3: Liquidation and Bad Debt — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Anyone can liquidate an unhealthy position — repaying cNGN for collateral plus a bonus — and the admin can write off a loan whose remaining collateral is dust, with the protocol reserve absorbing the loss before lenders.

**Architecture:**
- **Builds on Plan 2** (`main` at `caa6601`): collateral, prices, positions, `take_loan`, `repay_loan`.
- **Prices get pinned first.** A collateral asset may name the one Pyth account its price comes from, so a liquidator cannot shop among the verified updates inside the age window; the window itself is capped for assets with nothing to pin. The mint account leaves the health accounts at the same time, since the asset already carries what it was there for.
- **Seizure arithmetic** lives in `math/liquidation.rs`, pure and unit-tested, with the division interleaved so a large repayment cannot overflow.
- **`liquidate`** prices the whole position (health decides whether it may be touched at all), then repays one loan and seizes from one collateral slot. **`write_off_loan`** clears a loan against dust collateral and books the loss.

**Tech Stack:** Rust 1.89+, Anchor 1.2.0, Solana CLI 3.1.x (`cargo build-sbf`, platform tools v1.52), LiteSVM 0.10.0, `pyth-solana-receiver-sdk` 2.0.0, `switchboard-on-demand` 0.13.0.

**Spec:** `docs/superpowers/specs/2026-09-17-solana-fixed-loans-design.md`

## Plan series

| Plan | Delivers |
|---|---|
| 1. Foundation and lender pool (merged) | Workspace, math, roles, whitelist/blacklist, cNGN market, lender deposit/withdraw, donation sweep |
| 2. Collateral, prices and loans (merged) | `CollateralAsset`, positions, Pyth and Switchboard reads, health checks, `take_loan`, `repay_loan`, reserve harvest |
| **3. Liquidation and bad debt** (this plan) | Pinned price accounts, seizure math, `liquidate`, `write_off_loan` (reserve first) |
| 4. xStocks | Token-2022 extension policy, scaled-UI multiplier, transfer-time re-checks |
| 5. Promo balance | Promo vault, campaigns, Ed25519 vouchers, forfeiture, expiry, `set_promo_cap` |
| 6. Hardening and devnet | Trident invariant fuzzing, ported EVM regressions, devnet run, pre-audit scan |

## Global Constraints

Carried from Plans 1 and 2:

- Anchor `1.2.0` for `anchor-lang` and `anchor-spl`; LiteSVM `0.10.0`; Rust `1.89` or newer; build with `cargo build-sbf --tools-version v1.52` (through `scripts/test.sh`).
- `[profile.release] overflow-checks = true`; all math on `u128` with checked operations.
- Rounding: borrower debt rounds up; lender accrual and minted shares round down; burned shares round up.
- Constants: `BPS = 10_000`, `YEAR = 31_536_000` seconds, `VIRTUAL_SHARES = 1_000`, `VIRTUAL_ASSETS = 1`, `MIN_TENURE = 86_400`, `MAX_COLLATERAL_SLOTS = 8`, `MAX_LOAN_SLOTS = 10`, `USD_SCALE = 10^12`.
- Every account starts with `version: u8` and `bump: u8` and ends with reserved padding; new fields come out of that padding, so account sizes don't change.
- In user-facing instructions, wrap `Market`, `LenderPosition`, `CollateralAsset` and every `InterfaceAccount` in `Box<…>`. The SBF stack frame is 4 KB.
- One `#[error_code] HodlError` enum. **Append new variants only**, since codes are `6000 + position`. This plan adds none: `NotLiquidatable`, `ZeroPrincipalRepaid` and `WriteOffNotAllowed` already exist.
- No `Option` fields in account data that bots scan; use a `Pubkey::default()` sentinel.
- `cash` changes only through program instructions. Tokens sent straight to a vault never change the share price.
- A loan copies `rate_bps`, `penalty_rate_bps` and `reserve_factor_bps` from the market when it opens; later market changes never touch open loans.
- Collateral counts at `price − confidence` (round down); debt counts at `NGN price + spread` (round up). Healthy is `debt ≤ Σ value × ltv_bps / BPS`; liquidatable is `debt > Σ value × liquidation_threshold_bps / BPS`.
- Commits made by Claude end with the attribution trailer from the session instructions.

New in this plan:

- **Checked arithmetic, no exceptions.** Every new arithmetic site uses `checked_*` or the `math::checked` helpers, even where a `require!` on the previous line already proves it safe. (The guarded raw operators Plan 2 left behind stay for the Plan 6 sweep; do not add more.)
- **Seizure uses plain prices** — no confidence subtraction, no NGN spread — so a liquidator is paid at mid price and the bonus is the whole incentive.
- A health check reads one `(CollateralAsset, PriceUpdateV2)` **pair** per used collateral slot, in slot order. The mint account is gone.
- `MAX_PRICE_AGE_SECONDS = 60` caps `CollateralParams::max_price_age_seconds`.
- Liquidation is open to anyone: no `Access` account, no whitelist.

## Facts verified while writing this plan (2026-09-17)

- **Seizure cannot be written as the spec's single expression.** `amount × ngn_price × (BPS + bonus) × 10^decimals` overflows `u128` for a large repayment (≈1.2e41 at the extreme). The division is interleaved instead — value, then bonus, then collateral units — which keeps every intermediate under ≈1.2e31 and rounds down at each step.
- **Liquidation is pro rata, not interest-first.** Spec §11 step 7 splits a partial liquidation as `amount × principal / balance`, unlike `repay_loan`, which pays interest first. Measured in the tests: repaying 8,120 cNGN of a 107,120 cNGN balance repays 7,580.283793 cNGN of principal.
- **`interest_anchor` is not reset by a liquidation** (spec §11 step 10), so the penalty clock keeps running from maturity; `repay_loan` does reset it.
- **Compute at full load, measured in LiteSVM** with 8 collateral slots and 10 loans: `liquidate` ≈ 70k CU, well under the 200,000 default (`take_loan` ≈ 63k, `withdraw_collateral` ≈ 67k, `repay_loan` ≈ 19k).
- **A position stays alive after a write-off:** the loan slot is cleared but the dust collateral stays, and the owner can withdraw it (no loans left means no price accounts are needed).
- **Verification.** The full Plan 3 code was compiled and tested before this plan was written: 138 tests pass (37 unit, 101 LiteSVM), `cargo clippy -D warnings` is clean, every task's end state was rebuilt from Plan 2's head and passes its own suite and clippy, and each task's failing-test step was run to capture its expected errors.

## Plan-level refinements to the spec

This plan's commit already writes these into the spec.

- **Pinned price accounts (§7, §8).** `CollateralAsset.price_account` (out of the reserved padding) names the one Pyth account accepted for the asset; `Pubkey::default()` leaves it unpinned. Pull updates are ephemeral accounts anyone may post, so an unpinned asset lets the caller choose the most favourable verified update inside `max_price_age_seconds` — which matters most when that caller is a liquidator. `MAX_PRICE_AGE_SECONDS = 60` bounds the window for assets with no sponsored feed to pin.
- **Health accounts are pairs (§8).** `(CollateralAsset, PriceUpdateV2)`, no mint: the asset carries the decimals and its own `mint` field identifies it. Plan 4 passes the mint again for `XStock` assets, whose multiplier lives on the mint.
- **Write-off keeps the collateral (§11).** Clearing the loan does not seize the dust; recovering it is a separate admin action.
- **Write-off front-running is accepted (§11).** The admin signs through a timelocked multisig, so a lender watching the queue can withdraw before the loss lands. A write-off requires the collateral to be dust, which bounds what the remaining lenders absorb, and the reserve absorbs it first; freezing lender withdrawals around a write-off would cost more trust than it saves.
- **Deferred:** promo forfeiture on liquidation and write-off (spec §11 step 3) arrives with Plan 5, together with promo in the health check. The `multiplier` in the spec's seizure formula is 1 until Plan 4 adds xStocks.
- The test harness's `bad_debt_dust_usd` default becomes $5 (`5_000_000_000_000` at `USD_SCALE`); the Plan 1 placeholder was $1,000,000, which would have let almost any position be written off.

## File Structure

New and changed files under `programs/hodl_loans/`:

```text
src/constants.rs                              + MAX_PRICE_AGE_SECONDS
src/state/collateral.rs                       + CollateralAsset/CollateralParams.price_account, age cap
src/instructions/admin/collateral_admin.rs    listing literal
src/valuation.rs                              pairs, pinned-account check, Valuation { health, collateral, ngn }
src/math/liquidation.rs                       seize_for_repayment, principal_share
src/math/mod.rs
src/events.rs                                 + LoanLiquidated, LoanPartiallyLiquidated, LoanWrittenOff
src/instructions/liquidation/mod.rs
src/instructions/liquidation/liquidate.rs     liquidate
src/instructions/liquidation/write_off.rs     write_off_loan
src/instructions/mod.rs
src/lib.rs                                    + 2 entry points
src/instructions/loans/take_loan.rs           doc comment only
src/instructions/positions/withdraw_collateral.rs  doc comment only
tests/common/mod.rs                           harness: pinned defaults, pairs, liquidator, write-off
tests/collateral.rs, tests/loans.rs, tests/withdraw.rs   updated for pairs and pinning
tests/liquidation.rs                          new
tests/write_off.rs                            new
tests/budget.rs                               + liquidate at full load
tests/invariants.rs                           + the default lifecycle
```

All paths below are relative to the repository root.

---

### Task 1: Pinned price accounts and pair-based health reads

**Files:**
- Modify: `src/constants.rs`, `src/state/collateral.rs`, `src/instructions/admin/collateral_admin.rs`, `src/valuation.rs`, `src/instructions/loans/take_loan.rs`, `src/instructions/positions/withdraw_collateral.rs`, `tests/common/mod.rs` (under `programs/hodl_loans/`)
- Test: `programs/hodl_loans/tests/collateral.rs`, `tests/loans.rs`, `tests/withdraw.rs`

**Interfaces:**
- Consumes: `CollateralParams::validate`, `valuation::load_collateral_values`, `load_health`, `oracle::pyth::read_pyth_price` (Plan 2).
- Produces:
  - Constant `MAX_PRICE_AGE_SECONDS: u64 = 60`
  - `CollateralAsset.price_account: Pubkey` and `CollateralParams.price_account: Pubkey` (`Pubkey::default()` = unpinned); `CollateralAsset.reserved` shrinks to `[u8; 96]`
  - `valuation::ACCOUNTS_PER_COLLATERAL = 2`; health reads `(CollateralAsset, PriceUpdateV2)` pairs
  - `valuation::Valuation { health: Health, collateral: Vec<CollateralValue>, ngn: UsdPrice }` and `valuation::load_valuation(program_id, position, market, ngn_feed, remaining, extra_debt, clock) -> Result<Valuation>`; `load_health` becomes a thin wrapper (Tasks 3 and 4 consume `load_valuation`)
  - Harness: `default_collateral_params` pins to `pyth_account(mint)`; `price_pairs(&[Pubkey])` replaces `price_triples`; `Env::price_accounts` returns pairs

- [ ] **Step 1: Update the harness and tests**

In `programs/hodl_loans/tests/common/mod.rs`, replace `default_collateral_params` with:

```rust
/// Spec §8 launch values for a stablecoin: LTV 70%, threshold 90%, bonus 5%,
/// pinned to the mint's test price account.
pub fn default_collateral_params(mint: &Pubkey) -> hodl_loans::CollateralParams {
    hodl_loans::CollateralParams {
        pyth_feed_id: feed_id(mint),
        price_account: pyth_account(mint),
        max_price_age_seconds: 60,
        max_conf_bps: 200,
        ltv_bps: 7_000,
        liquidation_threshold_bps: 9_000,
        liquidation_bonus_bps: 500,
        deposit_cap: u64::MAX,
    }
}
```

Replace `Env::price_accounts` with:

```rust
    /// One `(CollateralAsset, PriceUpdateV2)` pair per used collateral slot, in slot order.
    pub fn price_accounts(&self, owner: &Pubkey) -> Vec<AccountMeta> {
        let position = self.position(owner);
        let mints: Vec<Pubkey> = position.collateral.iter().filter(|s| s.amount > 0).map(|s| s.mint).collect();
        price_pairs(&mints)
    }
```

Replace `price_triples` with:

```rust
/// One `(CollateralAsset, PriceUpdateV2)` pair per listed mint, in the order given.
pub fn price_pairs(mints: &[Pubkey]) -> Vec<AccountMeta> {
    mints
        .iter()
        .flat_map(|mint| {
            [
                AccountMeta::new_readonly(collateral_pda(mint), false),
                AccountMeta::new_readonly(pyth_account(mint), false),
            ]
        })
        .collect()
}
```

Replace `programs/hodl_loans/tests/collateral.rs` (the parameter-rule cases gain the 61-second age and an unpinned asset):

```rust
mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use common::*;
use hodl_loans::{CollateralKind, CollateralParams, HodlError};
use solana_signer::Signer;

#[test]
fn admin_lists_spl_token_collateral() {
    let mut env = Env::initialized();
    let usdc = env.list_spl_collateral(6);

    let asset = env.collateral(&usdc);
    assert_eq!(asset.version, 1);
    assert_eq!(asset.mint, usdc);
    assert_eq!(asset.token_program, SPL_TOKEN);
    assert_eq!(asset.vault, collateral_vault_pda(&usdc));
    assert_eq!(asset.decimals, 6);
    assert_eq!(asset.kind, CollateralKind::Standard);
    assert_eq!(asset.params(), default_collateral_params(&usdc));
    assert_eq!(asset.total_deposited, 0);
    assert!(!asset.paused);
    assert_eq!(env.token_owner(&asset.vault), collateral_pda(&usdc));
    assert_eq!(env.config().collateral_count, 1);

    env.list_spl_collateral(9);
    assert_eq!(env.config().collateral_count, 2);
}

#[test]
fn listing_is_admin_only_and_once_per_mint() {
    let mut env = Env::initialized();
    let mint = env.create_mint(MintKind::SplToken, 6);
    let stranger = env.funded_keypair();
    let by_stranger = list_collateral_ix(&stranger.pubkey(), &mint, &SPL_TOKEN, default_collateral_params(&mint));
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);

    let listing = list_collateral_ix(&env.admin.pubkey(), &mint, &SPL_TOKEN, default_collateral_params(&mint));
    send(&mut env.svm, std::slice::from_ref(&listing), &[&env.admin]).unwrap();
    assert!(send(&mut env.svm, &[listing], &[&env.admin]).is_err());
    assert_eq!(env.config().collateral_count, 1);
}

#[test]
fn mints_with_non_metadata_extensions_are_rejected() {
    let mut env = Env::initialized();
    let admin = env.admin.pubkey();
    for kind in [MintKind::TransferFee, MintKind::CngnLike] {
        let mint = env.create_mint(kind, 6);
        let instruction = list_collateral_ix(&admin, &mint, &TOKEN_2022, default_collateral_params(&mint));
        assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::UnsupportedMintExtension);
    }
}

#[test]
fn collateral_parameter_rules() {
    let mut env = Env::initialized();
    let mint = env.list_spl_collateral(6);
    let admin = env.admin.pubkey();
    let base = default_collateral_params(&mint);

    let invalid = [
        CollateralParams { pyth_feed_id: [0; 32], ..base },
        CollateralParams { max_price_age_seconds: 0, ..base },
        CollateralParams { max_conf_bps: 10_001, ..base },
        // Above the 60-second cap on how far a caller may shop for a price.
        CollateralParams { max_price_age_seconds: 61, ..base },
        CollateralParams { ltv_bps: 999, liquidation_threshold_bps: 5_000, ..base },
        // LTV 7,500 + promo cap 2,000 > threshold 9,000
        CollateralParams { ltv_bps: 7_500, ..base },
        // threshold 9,000 × (10,000 + 1,112) > 10,000²
        CollateralParams { liquidation_bonus_bps: 1_112, ..base },
    ];
    for params in invalid {
        let instruction = update_collateral_params_ix(&admin, &mint, params);
        assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::InvalidParameters);
    }
    // `list_collateral` applies the same rules.
    let other = env.create_mint(MintKind::SplToken, 6);
    let bad_listing = CollateralParams { ltv_bps: 7_001, ..default_collateral_params(&other) };
    let instruction = list_collateral_ix(&admin, &other, &SPL_TOKEN, bad_listing);
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::InvalidParameters);

    // Every rule's boundary is accepted.
    let boundaries = [
        CollateralParams { ltv_bps: 1_000, liquidation_threshold_bps: 3_000, ..base },
        CollateralParams { liquidation_bonus_bps: 1_111, ..base },
        CollateralParams { max_conf_bps: 10_000, max_price_age_seconds: 60, ..base },
        // Unpinned: any verified update for the feed inside the window is accepted.
        CollateralParams { price_account: anchor_lang::prelude::Pubkey::default(), ..base },
    ];
    for params in boundaries {
        let instruction = update_collateral_params_ix(&admin, &mint, params);
        send(&mut env.svm, &[instruction], &[&env.admin]).unwrap();
        assert_eq!(env.collateral(&mint).params(), params);
    }
}

#[test]
fn only_admin_updates_parameters() {
    let mut env = Env::initialized();
    let mint = env.list_spl_collateral(6);
    let params = CollateralParams { deposit_cap: 5 * ONE_USDC, ..default_collateral_params(&mint) };

    let guardian = env.guardian.pubkey();
    let by_guardian = update_collateral_params_ix(&guardian, &mint, params);
    assert_hodl_error(send(&mut env.svm, &[by_guardian], &[&env.guardian]), HodlError::Unauthorized);

    let by_admin = update_collateral_params_ix(&env.admin.pubkey(), &mint, params);
    send(&mut env.svm, &[by_admin], &[&env.admin]).unwrap();
    assert_eq!(env.collateral(&mint).deposit_cap, 5 * ONE_USDC);
}

#[test]
fn guardian_pauses_and_only_admin_unpauses() {
    let mut env = Env::initialized();
    let mint = env.list_spl_collateral(6);
    let guardian = env.guardian.pubkey();
    let admin = env.admin.pubkey();

    let pause = set_collateral_paused_ix(&guardian, &mint, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();
    assert!(env.collateral(&mint).paused);

    let guardian_unpause = set_collateral_paused_ix(&guardian, &mint, false);
    assert_hodl_error(send(&mut env.svm, &[guardian_unpause], &[&env.guardian]), HodlError::Unauthorized);

    let stranger = env.funded_keypair();
    let stranger_pause = set_collateral_paused_ix(&stranger.pubkey(), &mint, true);
    assert_hodl_error(send(&mut env.svm, &[stranger_pause], &[&stranger]), HodlError::Unauthorized);

    let unpause = set_collateral_paused_ix(&admin, &mint, false);
    send(&mut env.svm, &[unpause], &[&env.admin]).unwrap();
    assert!(!env.collateral(&mint).paused);
}

#[test]
fn delist_closes_an_unused_asset() {
    let mut env = Env::initialized();
    let mint = env.list_spl_collateral(6);
    let admin = env.admin.pubkey();

    let stranger = env.funded_keypair();
    let by_stranger = delist_collateral_ix(&stranger.pubkey(), &mint, &SPL_TOKEN);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);

    let delist = delist_collateral_ix(&admin, &mint, &SPL_TOKEN);
    send(&mut env.svm, &[delist], &[&env.admin]).unwrap();
    assert!(env.svm.get_account(&collateral_pda(&mint)).is_none_or(|a| a.lamports == 0));
    assert!(env.svm.get_account(&collateral_vault_pda(&mint)).is_none_or(|a| a.lamports == 0));
    assert_eq!(env.config().collateral_count, 0);

    // The mint can be listed again.
    let relist = list_collateral_ix(&admin, &mint, &SPL_TOKEN, default_collateral_params(&mint));
    send(&mut env.svm, &[relist], &[&env.admin]).unwrap();
}

#[test]
fn delist_requires_no_deposits_and_an_empty_vault() {
    let mut env = Env::initialized();
    let mint = env.list_spl_collateral(6);
    let admin = env.admin.pubkey();
    let delist = delist_collateral_ix(&admin, &mint, &SPL_TOKEN);

    // Recorded deposits block delisting (simulated; deposits arrive in Task 5).
    let mut asset = env.collateral(&mint);
    asset.total_deposited = 1;
    env.write(&collateral_pda(&mint), &asset);
    assert_hodl_error(send(&mut env.svm, std::slice::from_ref(&delist), &[&env.admin]), HodlError::CollateralStillInUse);
    asset.total_deposited = 0;
    env.write(&collateral_pda(&mint), &asset);

    // So does a donation, until it is swept to the treasury.
    env.mint_to(&mint, &collateral_vault_pda(&mint), 7);
    assert_hodl_error(send(&mut env.svm, std::slice::from_ref(&delist), &[&env.admin]), HodlError::CollateralStillInUse);
    let treasury = env.treasury.pubkey();
    let destination = env.create_token_account(&mint, &treasury);
    let sweep = sweep_collateral_excess_ix(&admin, &mint, &SPL_TOKEN, &destination);
    send(&mut env.svm, &[sweep], &[&env.admin]).unwrap();
    assert_eq!(env.token_balance(&destination), 7);
    send(&mut env.svm, &[delist], &[&env.admin]).unwrap();
}

#[test]
fn collateral_sweep_moves_only_donations() {
    let mut env = Env::initialized();
    let mint = env.list_spl_collateral(6);
    let admin = env.admin.pubkey();
    let vault = collateral_vault_pda(&mint);
    let treasury = env.treasury.pubkey();
    let destination = env.create_token_account(&mint, &treasury);

    // Nothing to sweep yet.
    let sweep = sweep_collateral_excess_ix(&admin, &mint, &SPL_TOKEN, &destination);
    assert_hodl_error(send(&mut env.svm, std::slice::from_ref(&sweep), &[&env.admin]), HodlError::AmountTooSmall);

    // 40 recorded as deposited (simulated) plus a 60 donation: only the 60 moves.
    env.mint_to(&mint, &vault, 100);
    let mut asset = env.collateral(&mint);
    asset.total_deposited = 40;
    env.write(&collateral_pda(&mint), &asset);
    send(&mut env.svm, &[sweep], &[&env.admin]).unwrap();
    assert_eq!(env.token_balance(&destination), 60);
    assert_eq!(env.token_balance(&vault), 40);

    // Only the admin sweeps, and only to a treasury-owned account.
    env.mint_to(&mint, &vault, 5);
    let stranger = env.funded_keypair();
    let wrong_destination = env.create_token_account(&mint, &stranger.pubkey());
    let to_stranger = sweep_collateral_excess_ix(&admin, &mint, &SPL_TOKEN, &wrong_destination);
    assert_anchor_error(send(&mut env.svm, &[to_stranger], &[&env.admin]), AnchorError::ConstraintTokenOwner);
    let by_stranger = sweep_collateral_excess_ix(&stranger.pubkey(), &mint, &SPL_TOKEN, &destination);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);
}
```

Replace `programs/hodl_loans/tests/loans.rs` (the account-mismatch cases lose the mint and gain a pinning test at the end):

```rust
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
    instruction.accounts[7] = AccountMeta::new_readonly(pyth_account(&setup.usdc), false);
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
    let create = create_market_ix(&env.admin.pubkey(), &other, &TOKEN_2022, default_market_params());
    send(&mut env.svm, &[create], &[&env.admin]).unwrap();
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
```

Replace `programs/hodl_loans/tests/withdraw.rs` (`price_triples` becomes `price_pairs`):

```rust
mod common;

use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

const DAY: i64 = 86_400;

#[test]
fn without_loans_withdrawal_needs_no_market_or_prices() {
    let mut env = Env::initialized();
    let usdc = env.list_spl_collateral(6);
    let borrower = env.new_borrower();
    let owner = borrower.pubkey();
    let token = env.deposit_collateral(&borrower, &usdc, 1_000 * ONE_USDC);

    // Collateral pauses block deposits only.
    let pause = set_collateral_paused_ix(&env.guardian.pubkey(), &usdc, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();

    let withdraw = |amount| withdraw_collateral_ix(&owner, &usdc, &SPL_TOKEN, &token, None, amount, vec![]);
    env.sponsored(withdraw(400 * ONE_USDC), &borrower.key).unwrap();
    assert_eq!(env.token_balance(&token), 400 * ONE_USDC);
    assert_eq!(env.position(&owner).collateral[0].amount, 600 * ONE_USDC);
    assert_eq!(env.collateral(&usdc).total_deposited, 600 * ONE_USDC);

    env.sponsored(withdraw(600 * ONE_USDC), &borrower.key).unwrap();
    assert!(!env.position(&owner).has_collateral());
    assert_eq!(env.token_balance(&collateral_vault_pda(&usdc)), 0);

    // An emptied position can be closed.
    let close = close_position_ix(&owner, &env.admin.pubkey());
    env.sponsored(close, &borrower.key).unwrap();
}

#[test]
fn withdrawal_rejections() {
    let mut env = Env::initialized();
    let usdc = env.list_spl_collateral(6);
    let usdt = env.list_spl_collateral(6);
    let borrower = env.new_borrower();
    let owner = borrower.pubkey();
    let token = env.deposit_collateral(&borrower, &usdc, 10 * ONE_USDC);
    let usdt_token = env.create_token_account(&usdt, &owner);

    let zero = withdraw_collateral_ix(&owner, &usdc, &SPL_TOKEN, &token, None, 0, vec![]);
    assert_hodl_error(env.sponsored(zero, &borrower.key), HodlError::AmountTooSmall);
    let too_much = withdraw_collateral_ix(&owner, &usdc, &SPL_TOKEN, &token, None, 10 * ONE_USDC + 1, vec![]);
    assert_hodl_error(env.sponsored(too_much, &borrower.key), HodlError::InsufficientCollateral);
    let not_held = withdraw_collateral_ix(&owner, &usdt, &SPL_TOKEN, &usdt_token, None, 1, vec![]);
    assert_hodl_error(env.sponsored(not_held, &borrower.key), HodlError::InsufficientCollateral);

    env.blacklist(&owner);
    let blacklisted = withdraw_collateral_ix(&owner, &usdc, &SPL_TOKEN, &token, None, 1, vec![]);
    assert_hodl_error(env.sponsored(blacklisted, &borrower.key), HodlError::Blacklisted);
}

#[test]
fn with_loans_the_position_must_stay_healthy() {
    // 1,000 USDC backing 500,000 cNGN of debt ($312.8125 at the NGN ask).
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    let (usdc, cngn) = (setup.usdc, setup.cngn);
    env.take_loan(&setup.borrower, &setup, 500_000 * ONE_CNGN, 365 * DAY).unwrap();
    let token = env.create_token_account(&usdc, &owner);
    let withdraw = |amount, prices| withdraw_collateral_ix(&owner, &usdc, &SPL_TOKEN, &token, Some(&cngn), amount, prices);
    let key = &setup.borrower.key;

    // The market and prices are required while loans are active.
    let no_market = withdraw_collateral_ix(&owner, &usdc, &SPL_TOKEN, &token, None, ONE_USDC, price_pairs(&[usdc]));
    assert_hodl_error(env.sponsored(no_market, key), HodlError::PriceAccountMismatch);
    assert_hodl_error(env.sponsored(withdraw(ONE_USDC, vec![]), key), HodlError::PriceAccountMismatch);

    // 447 USDC × 70% = $312.90 covers the debt; 446 USDC ($312.20) does not.
    env.sponsored(withdraw(553 * ONE_USDC, price_pairs(&[usdc])), key).unwrap();
    assert_hodl_error(env.sponsored(withdraw(ONE_USDC, price_pairs(&[usdc])), key), HodlError::Unhealthy);
    // Withdrawing everything leaves no used slot to price, and no collateral value.
    assert_hodl_error(env.sponsored(withdraw(447 * ONE_USDC, vec![]), key), HodlError::Unhealthy);

    // Stale prices block withdrawals while loans are active.
    env.warp_seconds(61);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    assert_hodl_error(env.sponsored(withdraw(1, price_pairs(&[usdc])), key), HodlError::StalePrice);
}

#[test]
fn pairs_cover_the_slots_left_after_withdrawal() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    let sol = env.list_spl_collateral(9);
    env.set_pyth_price(&sol, 150 * ONE_DOLLAR, 0);
    env.deposit_collateral(&setup.borrower, &sol, 2_000_000_000);
    env.take_loan(&setup.borrower, &setup, 100_000 * ONE_CNGN, 365 * DAY).unwrap();
    let token = env.create_token_account(&sol, &owner);
    let key = &setup.borrower.key;

    // Emptying the SOL slot leaves only USDC to price.
    let with_sol = withdraw_collateral_ix(&owner, &sol, &SPL_TOKEN, &token, Some(&setup.cngn), 2_000_000_000, price_pairs(&[setup.usdc, sol]));
    assert_hodl_error(env.sponsored(with_sol, key), HodlError::PriceAccountMismatch);
    let usdc_only = withdraw_collateral_ix(&owner, &sol, &SPL_TOKEN, &token, Some(&setup.cngn), 2_000_000_000, price_pairs(&[setup.usdc]));
    env.sponsored(usdc_only, key).unwrap();
    assert_eq!(env.token_balance(&token), 2_000_000_000);
    assert_eq!(env.collateral(&sol).total_deposited, 0);
}

#[test]
fn withdrawal_checks_the_borrowed_market() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 365 * DAY).unwrap();
    let other = env.create_mint(MintKind::CngnLike, 6);
    let create = create_market_ix(&env.admin.pubkey(), &other, &TOKEN_2022, default_market_params());
    send(&mut env.svm, &[create], &[&env.admin]).unwrap();

    let token = env.create_token_account(&setup.usdc, &owner);
    let wrong_market = withdraw_collateral_ix(&owner, &setup.usdc, &SPL_TOKEN, &token, Some(&other), ONE_USDC, price_pairs(&[setup.usdc]));
    assert_hodl_error(env.sponsored(wrong_market, &setup.borrower.key), HodlError::MarketMismatch);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test loans`
Expected: compile errors `struct CollateralParams has no field named price_account` and `no field price_account on type CollateralAsset`.

- [ ] **Step 3: Implement**

Replace `programs/hodl_loans/src/constants.rs`:

```rust
use anchor_lang::prelude::*;

/// Basis-point denominator used in all u128 math.
pub const BPS: u128 = 10_000;
/// Largest valid basis-point parameter (100%).
pub const MAX_BPS: u16 = 10_000;
/// Seconds in a year for interest math.
pub const YEAR: u128 = 31_536_000;
/// Virtual share and asset offsets that protect the first lender deposit.
pub const VIRTUAL_SHARES: u128 = 1_000;
pub const VIRTUAL_ASSETS: u128 = 1;
/// Shortest loan tenure.
pub const MIN_TENURE: i64 = 86_400;
pub const MAX_COLLATERAL_SLOTS: usize = 8;
pub const MAX_LOAN_SLOTS: usize = 10;
/// Layout version written into every account.
pub const ACCOUNT_VERSION: u8 = 1;

pub const DEFAULT_PROMO_CAP_BPS: u16 = 2_000;
/// Fixed-point scale for USD values in price and health math (10^12 per dollar).
/// 10^12 rather than 10^18 keeps `amount × price` inside u128 for any realistic balance.
pub const USD_SCALE: u128 = 1_000_000_000_000;
/// Decimal exponent of `USD_SCALE`.
pub const USD_DECIMALS: i32 = 12;

/// Longest price age an admin may configure for a collateral asset (spec §8).
/// A caller can pick any verified Pyth update inside this window, so the window is the
/// price-selection surface for assets without a pinned price account.
pub const MAX_PRICE_AGE_SECONDS: u64 = 60;

#[constant]
pub const CONFIG_SEED: &[u8] = b"config";
#[constant]
pub const ACCESS_SEED: &[u8] = b"access";
#[constant]
pub const MARKET_SEED: &[u8] = b"market";
#[constant]
pub const MARKET_VAULT_SEED: &[u8] = b"market_vault";
#[constant]
pub const LENDER_SEED: &[u8] = b"lender";
#[constant]
pub const COLLATERAL_SEED: &[u8] = b"collateral";
#[constant]
pub const COLLATERAL_VAULT_SEED: &[u8] = b"collateral_vault";
#[constant]
pub const POSITION_SEED: &[u8] = b"position";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_are_distinct_and_bps_matches() {
        let seeds = [
            CONFIG_SEED,
            ACCESS_SEED,
            MARKET_SEED,
            MARKET_VAULT_SEED,
            LENDER_SEED,
            COLLATERAL_SEED,
            COLLATERAL_VAULT_SEED,
            POSITION_SEED,
        ];
        for (i, a) in seeds.iter().enumerate() {
            for b in &seeds[i + 1..] {
                assert_ne!(a, b);
            }
        }
        assert_eq!(BPS, MAX_BPS as u128);
        assert_eq!(USD_SCALE, 10u128.pow(USD_DECIMALS as u32));
    }
}
```

Replace `programs/hodl_loans/src/state/collateral.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{BPS, MAX_BPS, MAX_PRICE_AGE_SECONDS};
use crate::errors::HodlError;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum CollateralKind {
    /// Classic SPL Token or metadata-only Token-2022 mint (SOL, USDC, USDT).
    Standard,
    /// Admin-listed xStock (Plan 4).
    XStock,
}

#[account]
#[derive(InitSpace)]
pub struct CollateralAsset {
    pub version: u8,
    pub bump: u8,
    pub vault_bump: u8,
    pub mint: Pubkey,
    pub token_program: Pubkey,
    pub vault: Pubkey,
    pub decimals: u8,
    pub kind: CollateralKind,
    pub pyth_feed_id: [u8; 32],
    /// Pyth price account this asset is pinned to. `Pubkey::default()` accepts any verified
    /// update for `pyth_feed_id` inside `max_price_age_seconds`, so the caller may pick the
    /// most favourable update in that window; pinning removes that choice.
    /// Taken from the reserved padding, so the account size is unchanged.
    pub price_account: Pubkey,
    pub max_price_age_seconds: u64,
    pub max_conf_bps: u16,
    pub ltv_bps: u16,
    pub liquidation_threshold_bps: u16,
    pub liquidation_bonus_bps: u16,
    /// Raw token amounts.
    pub deposit_cap: u64,
    pub total_deposited: u64,
    /// Blocks new deposits of this asset only.
    pub paused: bool,
    pub reserved: [u8; 96],
}

/// Admin-settable collateral parameters, used by `list_collateral` and `update_collateral_params`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct CollateralParams {
    pub pyth_feed_id: [u8; 32],
    /// `Pubkey::default()` leaves the asset unpinned (see `CollateralAsset::price_account`).
    pub price_account: Pubkey,
    pub max_price_age_seconds: u64,
    pub max_conf_bps: u16,
    pub ltv_bps: u16,
    pub liquidation_threshold_bps: u16,
    pub liquidation_bonus_bps: u16,
    pub deposit_cap: u64,
}

impl CollateralParams {
    /// Spec §8 collateral rules, checked against the current `Config::promo_cap_bps`.
    pub fn validate(&self, promo_cap_bps: u16) -> Result<()> {
        require!(self.pyth_feed_id != [0u8; 32], HodlError::InvalidParameters);
        require!(
            self.max_price_age_seconds > 0 && self.max_price_age_seconds <= MAX_PRICE_AGE_SECONDS,
            HodlError::InvalidParameters
        );
        require!(self.max_conf_bps <= MAX_BPS, HodlError::InvalidParameters);
        require!(self.ltv_bps >= 1_000, HodlError::InvalidParameters);
        require!(
            self.ltv_bps as u32 + promo_cap_bps as u32 <= self.liquidation_threshold_bps as u32,
            HodlError::InvalidParameters
        );
        require!(
            self.liquidation_threshold_bps as u128 * (BPS + self.liquidation_bonus_bps as u128) <= BPS * BPS,
            HodlError::InvalidParameters
        );
        Ok(())
    }
}

impl CollateralAsset {
    pub fn params(&self) -> CollateralParams {
        CollateralParams {
            pyth_feed_id: self.pyth_feed_id,
            price_account: self.price_account,
            max_price_age_seconds: self.max_price_age_seconds,
            max_conf_bps: self.max_conf_bps,
            ltv_bps: self.ltv_bps,
            liquidation_threshold_bps: self.liquidation_threshold_bps,
            liquidation_bonus_bps: self.liquidation_bonus_bps,
            deposit_cap: self.deposit_cap,
        }
    }

    pub fn apply_params(&mut self, p: &CollateralParams) {
        self.pyth_feed_id = p.pyth_feed_id;
        self.price_account = p.price_account;
        self.max_price_age_seconds = p.max_price_age_seconds;
        self.max_conf_bps = p.max_conf_bps;
        self.ltv_bps = p.ltv_bps;
        self.liquidation_threshold_bps = p.liquidation_threshold_bps;
        self.liquidation_bonus_bps = p.liquidation_bonus_bps;
        self.deposit_cap = p.deposit_cap;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sol() -> CollateralParams {
        CollateralParams {
            pyth_feed_id: [1; 32],
            price_account: Pubkey::default(),
            max_price_age_seconds: 60,
            max_conf_bps: 200,
            ltv_bps: 7_000,
            liquidation_threshold_bps: 9_000,
            liquidation_bonus_bps: 1_000,
            deposit_cap: u64::MAX,
        }
    }

    #[test]
    fn launch_values_pass_with_twenty_percent_promo_cap() {
        sol().validate(2_000).unwrap();
        CollateralParams { ltv_bps: 5_000, liquidation_threshold_bps: 7_500, ..sol() }
            .validate(2_000)
            .unwrap();
    }

    #[test]
    fn each_rule_is_enforced() {
        let cases = [
            CollateralParams { pyth_feed_id: [0; 32], ..sol() },
            CollateralParams { max_price_age_seconds: 0, ..sol() },
            // Above the 60-second cap, which bounds how far a caller may shop for a price.
            CollateralParams { max_price_age_seconds: 61, ..sol() },
            CollateralParams { max_conf_bps: 10_001, ..sol() },
            CollateralParams { ltv_bps: 999, liquidation_threshold_bps: 2_999, ..sol() },
            // 80% LTV + 20% promo cap exceeds a 90% threshold.
            CollateralParams { ltv_bps: 8_000, ..sol() },
            // 90% threshold × 1.12 bonus exceeds 100%.
            CollateralParams { liquidation_bonus_bps: 1_200, ..sol() },
        ];
        for case in cases {
            assert!(case.validate(2_000).is_err(), "{case:?}");
        }
        // The promo cap is part of the rule: 80% LTV is fine with a 10% cap.
        CollateralParams { ltv_bps: 8_000, ..sol() }.validate(1_000).unwrap();
    }
}
```

In `programs/hodl_loans/src/instructions/admin/collateral_admin.rs`, in the `CollateralAsset { … }` literal of `handle_list_collateral`, add the new field after `pyth_feed_id` and shrink the padding:

```rust
        pyth_feed_id: [0; 32],
        price_account: Pubkey::default(),
```

```rust
        reserved: [0; 96],
```

Replace `programs/hodl_loans/src/valuation.rs`:

```rust
use anchor_lang::prelude::*;

use crate::errors::HodlError;
use crate::math::checked::add;
use crate::math::health::{compute_health, CollateralValue, Health};
use crate::math::price::UsdPrice;
use crate::math::loan::loan_balance;
use crate::oracle::pyth::read_pyth_price;
use crate::oracle::switchboard::read_ngn_price;
use crate::state::{CollateralAsset, Market, Position};

/// Accounts per used collateral slot in `remaining_accounts`: `(CollateralAsset, PriceUpdateV2)`.
pub const ACCOUNTS_PER_COLLATERAL: usize = 2;

/// Value every used collateral slot, in slot order, from `remaining` pairs.
///
/// The loop runs over the position's slots, not over the accounts supplied, so a missing,
/// extra or mismatched account fails with `PriceAccountMismatch` instead of skipping collateral.
/// An asset with a pinned `price_account` accepts only that account, so the caller cannot
/// choose among the verified updates inside the asset's age window.
pub fn load_collateral_values(
    program_id: &Pubkey,
    position: &Position,
    remaining: &[AccountInfo],
    clock: &Clock,
) -> Result<Vec<CollateralValue>> {
    let used: Vec<_> = position.collateral.iter().filter(|s| s.amount > 0).collect();
    require!(
        remaining.len() == used.len() * ACCOUNTS_PER_COLLATERAL,
        HodlError::PriceAccountMismatch
    );
    let mut values = Vec::with_capacity(used.len());
    for (slot, accounts) in used.iter().zip(remaining.chunks(ACCOUNTS_PER_COLLATERAL)) {
        let (asset_info, price_info) = (&accounts[0], &accounts[1]);
        require_keys_eq!(*asset_info.owner, *program_id, HodlError::PriceAccountMismatch);
        let asset = {
            let data = asset_info.try_borrow_data()?;
            CollateralAsset::try_deserialize(&mut &data[..]).map_err(|_| HodlError::PriceAccountMismatch)?
        };
        require_keys_eq!(asset.mint, slot.mint, HodlError::PriceAccountMismatch);
        if asset.price_account != Pubkey::default() {
            require_keys_eq!(price_info.key(), asset.price_account, HodlError::PriceAccountMismatch);
        }
        let price = read_pyth_price(
            price_info,
            &asset.pyth_feed_id,
            asset.max_price_age_seconds,
            asset.max_conf_bps,
            clock,
        )?;
        values.push(CollateralValue {
            amount: slot.amount,
            decimals: asset.decimals,
            price,
            ltv_bps: asset.ltv_bps,
            liquidation_threshold_bps: asset.liquidation_threshold_bps,
        });
    }
    Ok(values)
}

/// Total cNGN owed across active loans at `now` (principal + interest + penalty).
pub fn total_debt(position: &Position, now: i64) -> Result<u128> {
    let mut total = 0u128;
    for loan in position.loans.iter().filter(|l| l.is_active()) {
        total = add(total, loan_balance(&loan.terms(), now)?.total()?)?;
    }
    Ok(total)
}

/// Everything one price read of a position yields: its health, the per-slot values behind it
/// (in used-slot order), and the cNGN price. Liquidation needs the values and the price;
/// borrowing and withdrawing need only the health.
pub struct Valuation {
    pub health: Health,
    pub collateral: Vec<CollateralValue>,
    pub ngn: UsdPrice,
}

/// Spec §8 valuation of a position, optionally including `extra_debt` about to be borrowed.
pub fn load_valuation(
    program_id: &Pubkey,
    position: &Position,
    market: &Market,
    ngn_feed: &AccountInfo,
    remaining: &[AccountInfo],
    extra_debt: u64,
    clock: &Clock,
) -> Result<Valuation> {
    let collateral = load_collateral_values(program_id, position, remaining, clock)?;
    let ngn = read_ngn_price(ngn_feed, market, clock)?;
    let debt = add(total_debt(position, clock.unix_timestamp)?, extra_debt as u128)?;
    let health = compute_health(&collateral, debt, market.decimals, ngn)?;
    Ok(Valuation { health, collateral, ngn })
}

/// Spec §8 health for a position, optionally including `extra_debt` about to be borrowed.
pub fn load_health(
    program_id: &Pubkey,
    position: &Position,
    market: &Market,
    ngn_feed: &AccountInfo,
    remaining: &[AccountInfo],
    extra_debt: u64,
    clock: &Clock,
) -> Result<Health> {
    Ok(load_valuation(program_id, position, market, ngn_feed, remaining, extra_debt, clock)?.health)
}
```

Two doc comments still describe triples. In `programs/hodl_loans/src/instructions/loans/take_loan.rs`:

```rust
/// Spec §10 `take_loan`. `remaining_accounts`: one `(CollateralAsset, PriceUpdateV2)` pair
/// per used collateral slot, in slot order.
```

And in `programs/hodl_loans/src/instructions/positions/withdraw_collateral.rs`:

```rust
/// With active loans, `market` and `ngn_feed` are required, `remaining_accounts` must hold one
/// `(CollateralAsset, PriceUpdateV2)` pair per collateral slot still used **after**
/// this withdrawal, in slot order, and the position must stay healthy.
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, 121 tests in all (`loans` is now 12).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: pin collateral price accounts and cap the price age"
```

---

### Task 2: Seizure math

**Files:**
- Create: `programs/hodl_loans/src/math/liquidation.rs`
- Modify: `programs/hodl_loans/src/math/mod.rs`

**Interfaces:**
- Consumes: `BPS`, `math::checked::{mul_div_floor, to_u64}`, `HodlError::{InvalidPrice, MathOverflow}`.
- Produces:
  - `math::liquidation::Seizure { repay_amount: u64, seize_amount: u64 }`
  - `math::liquidation::seize_for_repayment(repay_amount: u64, ngn_price: u128, cngn_decimals: u8, collateral_price: u128, collateral_decimals: u8, bonus_bps: u16, slot_amount: u64) -> Result<Seizure>`
  - `math::liquidation::principal_share(repay_amount: u64, principal: u64, balance_total: u128) -> Result<u64>`

- [ ] **Step 1: Wire the module and write the failing tests**

Replace `programs/hodl_loans/src/math/mod.rs`:

```rust
pub mod checked;
pub mod health;
pub mod interest;
pub mod liquidation;
pub mod loan;
pub mod price;
pub mod shares;
```

Create `programs/hodl_loans/src/math/liquidation.rs` with its imports and test module only:

```rust
use anchor_lang::prelude::*;

use crate::constants::BPS;
use crate::errors::HodlError;
use crate::math::checked::{mul_div_floor, to_u64};

#[cfg(test)]
mod tests {
    use super::*;

    const USD: u128 = 1_000_000_000_000;
    /// 1 NGN = $0.000625.
    const NGN: u128 = 625_000_000;
    /// 1,600,000 cNGN, worth $1,000.
    const REPAY: u64 = 1_600_000_000_000;

    #[test]
    fn seizure_pays_the_bonus_on_top_of_the_repaid_value() {
        // 1,050 USDC (6 decimals at $1) for $1,000 of cNGN at a 5% bonus.
        let s = seize_for_repayment(REPAY, NGN, 6, USD, 6, 500, u64::MAX).unwrap();
        assert_eq!(s, Seizure { repay_amount: REPAY, seize_amount: 1_050_000_000 });

        // The same $1,050 is 7 SOL (9 decimals at $150).
        let s = seize_for_repayment(REPAY, NGN, 6, 150 * USD, 9, 500, u64::MAX).unwrap();
        assert_eq!(s.seize_amount, 7_000_000_000);

        // No bonus seizes exactly the repaid value.
        let s = seize_for_repayment(REPAY, NGN, 6, USD, 6, 0, u64::MAX).unwrap();
        assert_eq!(s.seize_amount, 1_000_000_000);
    }

    #[test]
    fn a_short_slot_caps_both_the_seizure_and_the_repayment() {
        // The slot holds 500 USDC of the 1,050 the full repayment would take.
        let s = seize_for_repayment(REPAY, NGN, 6, USD, 6, 500, 500_000_000).unwrap();
        assert_eq!(s.seize_amount, 500_000_000);
        // 1,600,000 × 500 / 1,050 cNGN, rounded down.
        assert_eq!(s.repay_amount, 761_904_761_904);
        // Re-pricing the capped repayment seizes no more than the slot.
        let again = seize_for_repayment(s.repay_amount, NGN, 6, USD, 6, 500, 500_000_000).unwrap();
        assert!(again.seize_amount <= 500_000_000);
    }

    #[test]
    fn seizure_rejects_missing_prices() {
        assert!(seize_for_repayment(REPAY, NGN, 6, 0, 6, 500, u64::MAX).is_err());
        assert!(seize_for_repayment(REPAY, 0, 6, USD, 6, 500, u64::MAX).is_err());
    }

    #[test]
    fn principal_share_pays_interest_first_and_rounds_down() {
        // A 103,000 cNGN balance on 100,000 of principal.
        let (principal, balance) = (100_000_000_000u64, 103_000_000_000u128);
        assert_eq!(principal_share(103_000_000_000, principal, balance).unwrap(), principal);
        assert_eq!(principal_share(51_500_000_000, principal, balance).unwrap(), 50_000_000_000);
        // Dust repayments pay interest only.
        assert_eq!(principal_share(1, principal, balance).unwrap(), 0);
        assert!(principal_share(1, principal, 0).is_err());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p hodl_loans --lib`
Expected: compile errors `unresolved imports crate::math::liquidation::principal_share, crate::math::liquidation::seize_for_repayment` and `cannot find function seize_for_repayment in this scope`.

- [ ] **Step 3: Implement**

Replace `programs/hodl_loans/src/math/liquidation.rs` with the full version — the same imports and tests with the functions added:

```rust
use anchor_lang::prelude::*;

use crate::constants::BPS;
use crate::errors::HodlError;
use crate::math::checked::{mul_div_floor, to_u64};

/// How much cNGN a liquidator pays and how much collateral it takes for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seizure {
    pub repay_amount: u64,
    pub seize_amount: u64,
}

/// Spec §11 seizure, at plain prices (no confidence or spread adjustment):
///
/// ```text
/// seize = repay × ngn_price × (BPS + bonus) × 10^collateral_decimals
///         / (10^cngn_decimals × collateral_price × BPS)
/// ```
///
/// The division is interleaved so a large repayment cannot overflow `u128`, and every step
/// rounds down, so the liquidator never receives more collateral than the formula allows.
/// When the slot holds less than that, the seizure takes the whole slot and the repayment
/// shrinks in proportion (spec §11 step 6).
pub fn seize_for_repayment(
    repay_amount: u64,
    ngn_price: u128,
    cngn_decimals: u8,
    collateral_price: u128,
    collateral_decimals: u8,
    bonus_bps: u16,
    slot_amount: u64,
) -> Result<Seizure> {
    require!(collateral_price > 0 && ngn_price > 0, HodlError::InvalidPrice);
    let repaid_usd = mul_div_floor(repay_amount as u128, ngn_price, pow10(cngn_decimals)?)?;
    let with_bonus = mul_div_floor(repaid_usd, BPS + bonus_bps as u128, BPS)?;
    let seize = mul_div_floor(with_bonus, pow10(collateral_decimals)?, collateral_price)?;

    if seize <= slot_amount as u128 {
        return Ok(Seizure { repay_amount, seize_amount: to_u64(seize)? });
    }
    // The slot caps the seizure, so the liquidator repays only the share it can cover.
    let capped_repayment = mul_div_floor(repay_amount as u128, slot_amount as u128, seize)?;
    Ok(Seizure { repay_amount: to_u64(capped_repayment)?, seize_amount: slot_amount })
}

/// The principal share of a partial repayment: `repay × principal / balance` (spec §11 step 7),
/// rounded down, so interest is paid first.
pub fn principal_share(repay_amount: u64, principal: u64, balance_total: u128) -> Result<u64> {
    require!(balance_total > 0, HodlError::MathOverflow);
    to_u64(mul_div_floor(repay_amount as u128, principal as u128, balance_total)?)
}

fn pow10(exponent: u8) -> Result<u128> {
    10u128.checked_pow(exponent as u32).ok_or(HodlError::MathOverflow.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const USD: u128 = 1_000_000_000_000;
    /// 1 NGN = $0.000625.
    const NGN: u128 = 625_000_000;
    /// 1,600,000 cNGN, worth $1,000.
    const REPAY: u64 = 1_600_000_000_000;

    #[test]
    fn seizure_pays_the_bonus_on_top_of_the_repaid_value() {
        // 1,050 USDC (6 decimals at $1) for $1,000 of cNGN at a 5% bonus.
        let s = seize_for_repayment(REPAY, NGN, 6, USD, 6, 500, u64::MAX).unwrap();
        assert_eq!(s, Seizure { repay_amount: REPAY, seize_amount: 1_050_000_000 });

        // The same $1,050 is 7 SOL (9 decimals at $150).
        let s = seize_for_repayment(REPAY, NGN, 6, 150 * USD, 9, 500, u64::MAX).unwrap();
        assert_eq!(s.seize_amount, 7_000_000_000);

        // No bonus seizes exactly the repaid value.
        let s = seize_for_repayment(REPAY, NGN, 6, USD, 6, 0, u64::MAX).unwrap();
        assert_eq!(s.seize_amount, 1_000_000_000);
    }

    #[test]
    fn a_short_slot_caps_both_the_seizure_and_the_repayment() {
        // The slot holds 500 USDC of the 1,050 the full repayment would take.
        let s = seize_for_repayment(REPAY, NGN, 6, USD, 6, 500, 500_000_000).unwrap();
        assert_eq!(s.seize_amount, 500_000_000);
        // 1,600,000 × 500 / 1,050 cNGN, rounded down.
        assert_eq!(s.repay_amount, 761_904_761_904);
        // Re-pricing the capped repayment seizes no more than the slot.
        let again = seize_for_repayment(s.repay_amount, NGN, 6, USD, 6, 500, 500_000_000).unwrap();
        assert!(again.seize_amount <= 500_000_000);
    }

    #[test]
    fn seizure_rejects_missing_prices() {
        assert!(seize_for_repayment(REPAY, NGN, 6, 0, 6, 500, u64::MAX).is_err());
        assert!(seize_for_repayment(REPAY, 0, 6, USD, 6, 500, u64::MAX).is_err());
    }

    #[test]
    fn principal_share_pays_interest_first_and_rounds_down() {
        // A 103,000 cNGN balance on 100,000 of principal.
        let (principal, balance) = (100_000_000_000u64, 103_000_000_000u128);
        assert_eq!(principal_share(103_000_000_000, principal, balance).unwrap(), principal);
        assert_eq!(principal_share(51_500_000_000, principal, balance).unwrap(), 50_000_000_000);
        // Dust repayments pay interest only.
        assert_eq!(principal_share(1, principal, balance).unwrap(), 0);
        assert!(principal_share(1, principal, 0).is_err());
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p hodl_loans --lib`
Expected: `test result: ok. 37 passed`.

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, 125 tests in all.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: seizure and pro-rata principal math for liquidation"
```

---

### Task 3: `liquidate`

**Files:**
- Create: `programs/hodl_loans/src/instructions/liquidation/mod.rs`, `src/instructions/liquidation/liquidate.rs` (under `programs/hodl_loans/`)
- Modify: `src/events.rs`, `src/instructions/mod.rs`, `src/lib.rs`, `tests/common/mod.rs`, `tests/budget.rs`
- Test: `programs/hodl_loans/tests/liquidation.rs`

**Interfaces:**
- Consumes: `valuation::load_valuation` (Task 1), `math::liquidation::{seize_for_repayment, principal_share}` (Task 2), `math::loan::{loan_balance, accrued_lp_interest, lp_contribution, reserve_share}`, `Market::accrue`, `token::transfer::{transfer_from_user, transfer_from_vault}`, `Position::{loan_index, collateral_index, has_active_loans}`.
- Produces:
  - Events `LoanLiquidated` and `LoanPartiallyLiquidated`, each `{ market, position, owner, liquidator, loan_id, amount, principal_repaid, interest_paid, collateral_mint, collateral_seized, remaining_principal }`
  - Instruction `liquidate(loan_id: u64, amount: u64)` with accounts `liquidator, position, market, mint, vault, liquidator_token, collateral, collateral_mint, collateral_vault, liquidator_collateral, ngn_feed, token_program, collateral_token_program`, then one price pair per used collateral slot. `amount` is capped at the loan's balance, and again by what the collateral slot can cover.
  - Harness: `Liquidator { key, cngn }` with `pubkey()`, `liquidate_ix(..)`, `Env::new_liquidator(&cngn, balance)`, `Env::liquidate(&liquidator, &setup, &collateral_mint, &liquidator_collateral, loan_id, amount)`

- [ ] **Step 1: Write the failing tests**

Append to `programs/hodl_loans/tests/common/mod.rs`:

```rust
// ---- Liquidation (Task 3) ----

/// Liquidation is open to anyone, so a liquidator needs no `Access` account.
pub struct Liquidator {
    pub key: Keypair,
    /// Its cNGN account, which funds repayments.
    pub cngn: Pubkey,
}

impl Liquidator {
    pub fn pubkey(&self) -> Pubkey {
        self.key.pubkey()
    }
}

#[allow(clippy::too_many_arguments)]
pub fn liquidate_ix(
    liquidator: &Pubkey,
    position_owner: &Pubkey,
    mint: &Pubkey,
    liquidator_token: &Pubkey,
    collateral_mint: &Pubkey,
    collateral_token_program: &Pubkey,
    liquidator_collateral: &Pubkey,
    loan_id: u64,
    amount: u64,
    prices: Vec<AccountMeta>,
) -> Instruction {
    let mut instruction = ix(
        hodl_loans::instruction::Liquidate { loan_id, amount },
        hodl_loans::accounts::Liquidate {
            liquidator: *liquidator,
            position: position_pda(position_owner),
            market: market_pda(mint),
            mint: *mint,
            vault: market_vault_pda(mint),
            liquidator_token: *liquidator_token,
            collateral: collateral_pda(collateral_mint),
            collateral_mint: *collateral_mint,
            collateral_vault: collateral_vault_pda(collateral_mint),
            liquidator_collateral: *liquidator_collateral,
            ngn_feed: ngn_feed(),
            token_program: TOKEN_2022,
            collateral_token_program: *collateral_token_program,
        },
    );
    instruction.accounts.extend(prices);
    instruction
}

impl Env {
    /// A funded wallet holding `balance` cNGN, with no whitelist.
    pub fn new_liquidator(&mut self, cngn: &Pubkey, balance: u64) -> Liquidator {
        let key = self.funded_keypair();
        let account = self.create_token_account(cngn, &key.pubkey());
        self.mint_to(cngn, &account, balance);
        Liquidator { key, cngn: account }
    }

    /// Repays `amount` of `loan_id` against the position's current collateral prices.
    pub fn liquidate(
        &mut self,
        liquidator: &Liquidator,
        setup: &LoanSetup,
        collateral_mint: &Pubkey,
        liquidator_collateral: &Pubkey,
        loan_id: u64,
        amount: u64,
    ) -> TxResult {
        let owner = setup.borrower.pubkey();
        let program = self.mint_program(collateral_mint);
        let prices = self.price_accounts(&owner);
        let instruction = liquidate_ix(
            &liquidator.pubkey(),
            &owner,
            &setup.cngn,
            &liquidator.cngn,
            collateral_mint,
            &program,
            liquidator_collateral,
            loan_id,
            amount,
            prices,
        );
        send(&mut self.svm, &[instruction], &[&liquidator.key])
    }
}
```

`programs/hodl_loans/tests/liquidation.rs`:

```rust
mod common;

use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

const DAY: i64 = 86_400;
/// The borrower's loan: 700,000 cNGN, worth $437.94 at the NGN ask.
const LOAN: u64 = 700_000 * ONE_CNGN;
/// $0.45 at Pyth exponent -8: 1,000 USDC then covers $450, under the $437.94 debt × 90% line.
const USDC_CRASHED: i64 = 45_000_000;

/// A position holding 1,000 USDC that owes `LOAN` and has just gone under water.
fn underwater() -> (Env, LoanSetup) {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    env.set_pyth_price(&setup.usdc, USDC_CRASHED, 0);
    (env, setup)
}

#[test]
fn healthy_positions_cannot_be_liquidated() {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    let liquidator = env.new_liquidator(&setup.cngn, LOAN);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    let result = env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, ONE_CNGN);
    assert_hodl_error(result, HodlError::NotLiquidatable);

    // Being overdue is not enough on its own (spec §2): only an unhealthy position is liquidatable.
    env.warp_seconds(400 * DAY);
    env.set_pyth_price(&setup.usdc, ONE_DOLLAR, 0);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    let result = env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, ONE_CNGN);
    assert_hodl_error(result, HodlError::NotLiquidatable);
}

#[test]
fn liquidation_seizes_collateral_plus_the_bonus() {
    let (mut env, setup) = underwater();
    let liquidator = env.new_liquidator(&setup.cngn, LOAN);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    let cash_before = env.market(&setup.cngn).cash;

    // 100,000 cNGN is $62.50; with the 5% bonus that seizes $65.625 of USDC at $0.45.
    let repaid = 100_000 * ONE_CNGN;
    env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, repaid).unwrap();
    let seized = 145_833_333;

    assert_eq!(env.token_balance(&collateral_account), seized);
    assert_eq!(env.token_balance(&liquidator.cngn), LOAN - repaid);
    assert_eq!(env.token_balance(&collateral_vault_pda(&setup.usdc)), 1_000 * ONE_USDC - seized);
    assert_eq!(env.collateral(&setup.usdc).total_deposited, 1_000 * ONE_USDC - seized);

    let position = env.position(&setup.borrower.pubkey());
    assert_eq!(position.collateral[0].amount, 1_000 * ONE_USDC - seized);
    let loan = position.loans[0];
    // No time has passed, so the whole repayment is principal and the terms are untouched.
    assert_eq!((loan.principal, loan.repaid), (LOAN - repaid, repaid));
    assert_eq!(loan.interest_anchor, loan.originated_at);

    let market = env.market(&setup.cngn);
    assert_eq!(market.total_borrows, LOAN - repaid);
    assert_eq!(market.cash, cash_before + repaid);
    assert_eq!(market.protocol_reserve, 0);
    assert_eq!(market.lp_rate_product, (LOAN - repaid) as u128 * 1_500 * 9_000);
}

#[test]
fn the_collateral_slot_caps_the_repayment() {
    let (mut env, setup) = underwater();
    let liquidator = env.new_liquidator(&setup.cngn, 10_000_000 * ONE_CNGN);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    // Repaying all 700,000 cNGN would seize 1,020.83 USDC, but the slot holds 1,000.
    env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, u64::MAX).unwrap();

    assert_eq!(env.token_balance(&collateral_account), 1_000 * ONE_USDC);
    assert_eq!(env.collateral(&setup.usdc).total_deposited, 0);
    let position = env.position(&setup.borrower.pubkey());
    assert!(!position.has_collateral());
    // 700,000 × 1,000 / 1,020.833333 cNGN, rounded down.
    let paid = 685_714_285_938;
    assert_eq!(position.loans[0].principal, LOAN - paid);
    assert_eq!(env.market(&setup.cngn).total_borrows, LOAN - paid);
}

#[test]
fn liquidation_splits_a_repayment_pro_rata_and_keeps_the_penalty_clock_running() {
    let (mut env, setup) = Env::loan_ready();
    // A 73-day loan left to run 146 days: 3,000 cNGN of interest, then 4,120 of penalty,
    // so the balance is 107,120 cNGN ($67.02). USDC at $0.05 puts the line at $45.
    env.take_loan(&setup.borrower, &setup, 100_000 * ONE_CNGN, 73 * DAY).unwrap();
    env.warp_seconds(146 * DAY);
    env.set_pyth_price(&setup.usdc, 5_000_000, 0);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    let liquidator = env.new_liquidator(&setup.cngn, 10_000_000 * ONE_CNGN);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    let anchor = env.position(&setup.borrower.pubkey()).loans[0].interest_anchor;

    // Spec §11 step 7 splits a liquidation pro rata, not interest-first as a repayment does:
    // 8,120 × 100,000 / 107,120 of it is principal.
    let repaid = 8_120 * ONE_CNGN;
    env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, repaid).unwrap();
    let principal_repaid = 7_580_283_793;
    let interest_paid = repaid - principal_repaid;

    let loan = env.position(&setup.borrower.pubkey()).loans[0];
    assert_eq!(loan.principal, 100_000 * ONE_CNGN - principal_repaid);
    assert_eq!(loan.repaid, repaid);
    // Spec §11 step 10: the anchor does not move, so the penalty keeps accruing from maturity.
    assert_eq!(loan.interest_anchor, anchor);

    let market = env.market(&setup.cngn);
    // The reserve takes 10% of the interest share only.
    assert_eq!(market.protocol_reserve, interest_paid / 10);
    assert_eq!(market.total_borrows, 100_000 * ONE_CNGN - principal_repaid);
    assert_eq!(market.cash, POOL_CNGN - 100_000 * ONE_CNGN + repaid);
    // $5.075 of cNGN plus the 5% bonus, at $0.05 a USDC.
    assert_eq!(env.token_balance(&collateral_account), 106_575_000);
}

#[test]
fn a_fully_repaid_loan_clears_its_slot() {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, 100_000 * ONE_CNGN, 365 * DAY).unwrap();
    env.take_loan(&setup.borrower, &setup, 600_000 * ONE_CNGN, 365 * DAY).unwrap();
    env.set_pyth_price(&setup.usdc, USDC_CRASHED, 0);
    let liquidator = env.new_liquidator(&setup.cngn, 10_000_000 * ONE_CNGN);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, u64::MAX).unwrap();

    let position = env.position(&setup.borrower.pubkey());
    assert_eq!(position.loans[0], bytemuck::Zeroable::zeroed());
    assert_eq!(position.loans[1].principal, 600_000 * ONE_CNGN);
    assert_eq!(env.market(&setup.cngn).total_borrows, 600_000 * ONE_CNGN);
    // 100,000 cNGN is $62.50; the 5% bonus seizes $65.625 at $0.45.
    assert_eq!(env.token_balance(&collateral_account), 145_833_333);
}

#[test]
fn liquidation_rejections() {
    let (mut env, setup) = underwater();
    let usdt = env.list_spl_collateral(6);
    env.set_pyth_price(&usdt, ONE_DOLLAR, 0);
    let liquidator = env.new_liquidator(&setup.cngn, LOAN);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    let usdt_account = env.create_token_account(&usdt, &liquidator.pubkey());

    let zero = env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, 0);
    assert_hodl_error(zero, HodlError::AmountTooSmall);
    let unknown = env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 7, ONE_CNGN);
    assert_hodl_error(unknown, HodlError::LoanNotFound);
    // The position holds no USDT.
    let wrong_asset = env.liquidate(&liquidator, &setup, &usdt, &usdt_account, 0, ONE_CNGN);
    assert_hodl_error(wrong_asset, HodlError::InsufficientCollateral);

    // A repayment too small to seize a whole base unit of collateral.
    let dust = env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, 1);
    assert_hodl_error(dust, HodlError::AmountTooSmall);

    // Prices must be fresh and complete.
    let owner = setup.borrower.pubkey();
    let program = env.mint_program(&setup.usdc);
    let no_prices = liquidate_ix(
        &liquidator.pubkey(), &owner, &setup.cngn, &liquidator.cngn, &setup.usdc, &program,
        &collateral_account, 0, ONE_CNGN, vec![],
    );
    assert_hodl_error(send(&mut env.svm, &[no_prices], &[&liquidator.key]), HodlError::PriceAccountMismatch);
    env.warp_seconds(61);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    let stale = env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, ONE_CNGN);
    assert_hodl_error(stale, HodlError::StalePrice);
}

#[test]
fn liquidation_checks_the_borrowed_market() {
    let (mut env, setup) = underwater();
    let other = env.create_mint(MintKind::CngnLike, 6);
    let create = create_market_ix(&env.admin.pubkey(), &other, &TOKEN_2022, default_market_params());
    send(&mut env.svm, &[create], &[&env.admin]).unwrap();
    let liquidator = env.new_liquidator(&other, LOAN);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    let owner = setup.borrower.pubkey();
    let prices = env.price_accounts(&owner);
    let program = env.mint_program(&setup.usdc);
    let instruction = liquidate_ix(
        &liquidator.pubkey(), &owner, &other, &liquidator.cngn, &setup.usdc, &program,
        &collateral_account, 0, ONE_CNGN, prices,
    );
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&liquidator.key]), HodlError::MarketMismatch);
}
```

In `programs/hodl_loans/tests/budget.rs`, insert this block directly above the `repay_loan` measurement (the comment beginning `// repay_loan needs no price accounts`):

```rust
    // liquidate prices all 8 collateral slots and all 10 loans, then moves two token types.
    // Crash every collateral price to $0.001 so the position is liquidatable.
    for m in &mints {
        env.set_pyth_price(m, 100_000, 0);
    }
    let liquidator = env.new_liquidator(&setup.cngn, 100_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    let prices = env.price_accounts(&owner);
    let lq = liquidate_ix(
        &liquidator.pubkey(), &owner, &setup.cngn, &liquidator.cngn, &setup.usdc, &SPL_TOKEN,
        &seized_to, 0, 100 * ONE_CNGN, prices,
    );
    let cu = send_cu(&mut env.svm, &[lq], &[&liquidator.key]).unwrap();
    assert!(cu < 100_000, "liquidate at 8 collateral slots / 10 loans used {cu} CU");
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test liquidation`
Expected: compile errors `cannot find struct, variant or union type Liquidate in module hodl_loans::instruction` and `… in module hodl_loans::accounts`.

- [ ] **Step 3: Implement**

Append to `programs/hodl_loans/src/events.rs`:

```rust
#[event]
pub struct LoanLiquidated {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub liquidator: Pubkey,
    pub loan_id: u64,
    pub amount: u64,
    pub principal_repaid: u64,
    pub interest_paid: u64,
    pub collateral_mint: Pubkey,
    pub collateral_seized: u64,
    pub remaining_principal: u64,
}

#[event]
pub struct LoanPartiallyLiquidated {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub liquidator: Pubkey,
    pub loan_id: u64,
    pub amount: u64,
    pub principal_repaid: u64,
    pub interest_paid: u64,
    pub collateral_mint: Pubkey,
    pub collateral_seized: u64,
    pub remaining_principal: u64,
}
```

`programs/hodl_loans/src/instructions/liquidation/mod.rs`:

```rust
pub mod liquidate;

pub use liquidate::*;
```

`programs/hodl_loans/src/instructions/liquidation/liquidate.rs`:

```rust
use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{COLLATERAL_SEED, MARKET_SEED};
use crate::errors::HodlError;
use crate::events::{LoanLiquidated, LoanPartiallyLiquidated};
use crate::math::checked::{add, sub, to_u64};
use crate::math::liquidation::{principal_share, seize_for_repayment};
use crate::math::loan::{accrued_lp_interest, loan_balance, lp_contribution, reserve_share};
use crate::state::{CollateralAsset, Market, Position};
use crate::token::transfer::{transfer_from_user, transfer_from_vault};
use crate::valuation::load_valuation;

/// Open to anyone: no `Access` account, so a liquidation bot needs no whitelist.
#[derive(Accounts)]
pub struct Liquidate<'info> {
    pub liquidator: Signer<'info>,
    #[account(mut)]
    pub position: AccountLoader<'info, Position>,
    #[account(
        mut,
        seeds = [MARKET_SEED, mint.key().as_ref()],
        bump = market.bump,
        has_one = mint,
        has_one = vault,
        has_one = ngn_feed @ HodlError::PriceAccountMismatch
    )]
    pub market: Box<Account<'info, Market>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, token::mint = mint, token::authority = liquidator, token::token_program = token_program)]
    pub liquidator_token: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        seeds = [COLLATERAL_SEED, collateral_mint.key().as_ref()],
        bump = collateral.bump,
        constraint = collateral.vault == collateral_vault.key() @ HodlError::PriceAccountMismatch
    )]
    pub collateral: Box<Account<'info, CollateralAsset>>,
    #[account(mint::token_program = collateral_token_program)]
    pub collateral_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub collateral_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = collateral_mint,
        token::authority = liquidator,
        token::token_program = collateral_token_program
    )]
    pub liquidator_collateral: Box<InterfaceAccount<'info, TokenAccount>>,
    /// CHECK: address pinned to `market.ngn_feed`; parsed by `read_ngn_price`.
    pub ngn_feed: UncheckedAccount<'info>,
    pub token_program: Interface<'info, TokenInterface>,
    pub collateral_token_program: Interface<'info, TokenInterface>,
}

/// Spec §11 `liquidate`. `remaining_accounts`: one `(CollateralAsset, PriceUpdateV2)` pair per
/// used collateral slot, in slot order — the whole position is priced, because health decides
/// whether it may be liquidated at all.
///
/// A late loan is not liquidatable on its own: only an unhealthy position is (spec §2).
/// Promo forfeiture (spec §11 step 3) arrives with Plan 5.
pub fn handle_liquidate<'info>(ctx: Context<'info, Liquidate<'info>>, loan_id: u64, amount: u64) -> Result<()> {
    require!(amount > 0, HodlError::AmountTooSmall);
    let market_key = ctx.accounts.market.key();
    let collateral_mint = ctx.accounts.collateral_mint.key();
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    let market = &mut ctx.accounts.market;
    market.accrue(now)?;

    let (paid, principal_repaid, interest_paid, seized, remaining_principal, owner) = {
        let mut position = ctx.accounts.position.load_mut()?;
        require_keys_eq!(position.market, market_key, HodlError::MarketMismatch);
        let loan_index = position.loan_index(loan_id).ok_or(HodlError::LoanNotFound)?;
        let slot_index = position
            .collateral_index(&collateral_mint)
            .ok_or(HodlError::InsufficientCollateral)?;

        let valuation = load_valuation(
            ctx.program_id,
            &position,
            market,
            &ctx.accounts.ngn_feed.to_account_info(),
            ctx.remaining_accounts,
            0,
            &clock,
        )?;
        require!(valuation.health.is_liquidatable(), HodlError::NotLiquidatable);
        // `load_valuation` returns one value per used slot, in slot order.
        let priced = position.collateral[..slot_index].iter().filter(|s| s.amount > 0).count();
        let collateral_price = valuation.collateral[priced].price.price;

        let loan = position.loans[loan_index];
        let balance = loan_balance(&loan.terms(), now)?.total()?;
        let requested = to_u64((amount as u128).min(balance))?;
        let seizure = seize_for_repayment(
            requested,
            valuation.ngn.price,
            market.decimals,
            collateral_price,
            ctx.accounts.collateral.decimals,
            ctx.accounts.collateral.liquidation_bonus_bps,
            position.collateral[slot_index].amount,
        )?;
        let paid = seizure.repay_amount;
        require!(paid > 0 && seizure.seize_amount > 0, HodlError::AmountTooSmall);

        let principal_repaid = principal_share(paid, loan.principal, balance)?;
        require!(principal_repaid > 0, HodlError::ZeroPrincipalRepaid);
        let interest_paid = to_u64(sub(paid as u128, principal_repaid as u128)?)?;

        // Spec §11 step 8: release the lender interest accrued for the repaid principal only.
        let released = accrued_lp_interest(
            principal_repaid,
            loan.rate_bps,
            loan.reserve_factor_bps,
            loan.interest_anchor,
            now,
        )?;
        market.accrued_interest = market.accrued_interest.saturating_sub(released);
        market.protocol_reserve = to_u64(add(
            market.protocol_reserve as u128,
            reserve_share(interest_paid as u128, loan.reserve_factor_bps)?,
        )?)?;
        market.total_borrows = market
            .total_borrows
            .checked_sub(principal_repaid)
            .ok_or(HodlError::MathOverflow)?;
        market.lp_rate_product = sub(
            market.lp_rate_product,
            lp_contribution(principal_repaid, loan.rate_bps, loan.reserve_factor_bps)?,
        )?;
        market.cash = market.cash.checked_add(paid).ok_or(HodlError::MathOverflow)?;

        // Spec §11 step 10: `interest_anchor` is not reset, so the clock keeps running.
        let slot = &mut position.loans[loan_index];
        slot.principal = slot.principal.checked_sub(principal_repaid).ok_or(HodlError::MathOverflow)?;
        slot.repaid = slot.repaid.checked_add(paid).ok_or(HodlError::MathOverflow)?;
        let remaining_principal = slot.principal;
        if remaining_principal == 0 {
            *slot = bytemuck::Zeroable::zeroed();
            if !position.has_active_loans() {
                position.promo_last_activity_at = now;
            }
        }

        let held = &mut position.collateral[slot_index];
        held.amount = held.amount.checked_sub(seizure.seize_amount).ok_or(HodlError::MathOverflow)?;
        (paid, principal_repaid, interest_paid, seizure.seize_amount, remaining_principal, position.owner)
    };

    let collateral = &mut ctx.accounts.collateral;
    collateral.total_deposited = collateral
        .total_deposited
        .checked_sub(seized)
        .ok_or(HodlError::MathOverflow)?;

    transfer_from_user(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.liquidator_token.to_account_info(),
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.liquidator.to_account_info(),
        paid,
    )?;
    let seeds: &[&[u8]] = &[COLLATERAL_SEED, collateral_mint.as_ref(), &[ctx.accounts.collateral.bump]];
    transfer_from_vault(
        ctx.accounts.collateral_token_program.key(),
        ctx.accounts.collateral_mint.to_account_info(),
        ctx.accounts.collateral_mint.decimals,
        ctx.accounts.collateral_vault.to_account_info(),
        ctx.accounts.liquidator_collateral.to_account_info(),
        ctx.accounts.collateral.to_account_info(),
        seized,
        &[seeds],
    )?;

    let position_key = ctx.accounts.position.key();
    let liquidator = ctx.accounts.liquidator.key();
    if remaining_principal == 0 {
        emit!(LoanLiquidated {
            market: market_key,
            position: position_key,
            owner,
            liquidator,
            loan_id,
            amount: paid,
            principal_repaid,
            interest_paid,
            collateral_mint,
            collateral_seized: seized,
            remaining_principal,
        });
    } else {
        emit!(LoanPartiallyLiquidated {
            market: market_key,
            position: position_key,
            owner,
            liquidator,
            loan_id,
            amount: paid,
            principal_repaid,
            interest_paid,
            collateral_mint,
            collateral_seized: seized,
            remaining_principal,
        });
    }
    Ok(())
}
```

Replace `programs/hodl_loans/src/instructions/mod.rs`:

```rust
pub mod admin;
pub mod liquidation;
pub mod liquidity;
pub mod loans;
pub mod positions;

pub use admin::*;
pub use liquidation::*;
pub use liquidity::*;
pub use loans::*;
pub use positions::*;
```

In `lib.rs`, add inside `pub mod hodl_loans { … }` after `repay_loan`:

```rust
    pub fn liquidate<'info>(ctx: Context<'info, Liquidate<'info>>, loan_id: u64, amount: u64) -> Result<()> {
        instructions::handle_liquidate(ctx, loan_id, amount)
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh --test liquidation`
Expected: `test result: ok. 7 passed`.

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, 132 tests in all. The budget test prints nothing on success; it fails if `liquidate` crosses 100,000 CU.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: permissionless liquidation with a collateral bonus"
```

---

### Task 4: `write_off_loan`

**Files:**
- Create: `programs/hodl_loans/src/instructions/liquidation/write_off.rs`
- Modify: `src/events.rs`, `src/instructions/liquidation/mod.rs`, `src/lib.rs`, `tests/common/mod.rs`, `tests/invariants.rs` (under `programs/hodl_loans/`)
- Test: `programs/hodl_loans/tests/write_off.rs`

**Interfaces:**
- Consumes: `valuation::load_valuation`, `math::loan::{accrued_lp_interest, lp_contribution}`, `Market::{accrue, bad_debt_dust_usd, protocol_reserve, total_bad_debt}`, `Config.admin`.
- Produces:
  - Event `LoanWrittenOff { market, position, owner, loan_id, principal, loss, covered_by_reserve, total_bad_debt }`
  - Instruction `write_off_loan(loan_id: u64)` (admin) with accounts `admin, config, position, market, ngn_feed`, then one price pair per used collateral slot. No token accounts: nothing moves.
  - Harness: `write_off_loan_ix(..)`, `Env::write_off(&setup, loan_id)`, and a `bad_debt_dust_usd` default of $5

- [ ] **Step 1: Write the failing tests**

In `programs/hodl_loans/tests/common/mod.rs`, replace the `bad_debt_dust_usd` line of `default_market_params` with:

```rust
        // $5 at USD_SCALE: below that, liquidating costs more than it recovers.
        bad_debt_dust_usd: 5_000_000_000_000,
```

Then append:

```rust
// ---- Write-off (Task 4) ----

pub fn write_off_loan_ix(admin: &Pubkey, position_owner: &Pubkey, mint: &Pubkey, loan_id: u64, prices: Vec<AccountMeta>) -> Instruction {
    let mut instruction = ix(
        hodl_loans::instruction::WriteOffLoan { loan_id },
        hodl_loans::accounts::WriteOffLoan {
            admin: *admin,
            config: config_pda(),
            position: position_pda(position_owner),
            market: market_pda(mint),
            ngn_feed: ngn_feed(),
        },
    );
    instruction.accounts.extend(prices);
    instruction
}

impl Env {
    pub fn write_off(&mut self, setup: &LoanSetup, loan_id: u64) -> TxResult {
        let owner = setup.borrower.pubkey();
        let prices = self.price_accounts(&owner);
        let admin = self.admin.pubkey();
        let instruction = write_off_loan_ix(&admin, &owner, &setup.cngn, loan_id, prices);
        send(&mut self.svm, &[instruction], &[&self.admin])
    }
}
```

`programs/hodl_loans/tests/write_off.rs`:

```rust
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
```

Append to `programs/hodl_loans/tests/invariants.rs`:

```rust
#[test]
fn a_default_runs_from_liquidation_to_write_off() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    env.take_loan(&setup.borrower, &setup, 700_000 * ONE_CNGN, 365 * DAY).unwrap();
    assert_invariants(&env, &setup, "after borrowing");

    // USDC at $0.45 puts $437.94 of debt over the $405 liquidation line.
    env.set_pyth_price(&setup.usdc, 45_000_000, 0);
    let liquidator = env.new_liquidator(&setup.cngn, 10_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 300_000 * ONE_CNGN).unwrap();
    assert_invariants(&env, &setup, "after liquidating");
    let collateral = env.collateral(&setup.usdc);
    assert_eq!(env.token_balance(&collateral_vault_pda(&setup.usdc)), collateral.total_deposited);
    assert_eq!(
        env.position(&owner).collateral[0].amount + env.token_balance(&seized_to),
        1_000 * ONE_USDC
    );

    // The rest of the collateral collapses to dust, so the remaining loan is written off.
    env.set_pyth_price(&setup.usdc, 100_000, 0);
    env.write_off(&setup, 0).unwrap();
    assert_invariants(&env, &setup, "after the write-off");
    let market = env.market(&setup.cngn);
    assert_eq!(market.total_borrows, 0);
    assert!(market.total_bad_debt > 0);
    assert!(!env.position(&owner).has_active_loans());

    // The lender's exit is short by the bad debt, never above its deposit.
    env.withdraw(&setup.lender, &setup.cngn, u64::MAX).unwrap();
    let returned = env.token_balance(&setup.lender.token);
    assert!(returned < POOL_CNGN, "lender got back {returned} of {POOL_CNGN} despite bad debt");

    // With no loans left, the borrower can still withdraw the dust that was never seized.
    let dust = env.position(&owner).collateral[0].amount;
    let token = env.create_token_account(&setup.usdc, &owner);
    let withdraw = withdraw_collateral_ix(&owner, &setup.usdc, &SPL_TOKEN, &token, None, dust, vec![]);
    env.sponsored(withdraw, &setup.borrower.key).unwrap();
    assert_eq!(env.token_balance(&token), dust);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test write_off`
Expected: compile errors `cannot find struct, variant or union type WriteOffLoan in module hodl_loans::instruction` and `… in module hodl_loans::accounts`.

- [ ] **Step 3: Implement**

Append to `programs/hodl_loans/src/events.rs`:

```rust
#[event]
pub struct LoanWrittenOff {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub loan_id: u64,
    pub principal: u64,
    /// Principal plus the lender interest released for it.
    pub loss: u128,
    pub covered_by_reserve: u64,
    pub total_bad_debt: u128,
}
```

`programs/hodl_loans/src/instructions/liquidation/write_off.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{CONFIG_SEED, MARKET_SEED};
use crate::errors::HodlError;
use crate::events::LoanWrittenOff;
use crate::math::checked::{add, sub, to_u64};
use crate::math::loan::{accrued_lp_interest, lp_contribution};
use crate::state::{Config, Market, Position};
use crate::valuation::load_valuation;

#[derive(Accounts)]
pub struct WriteOffLoan<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(mut)]
    pub position: AccountLoader<'info, Position>,
    #[account(
        mut,
        seeds = [MARKET_SEED, market.mint.as_ref()],
        bump = market.bump,
        has_one = ngn_feed @ HodlError::PriceAccountMismatch
    )]
    pub market: Box<Account<'info, Market>>,
    /// CHECK: address pinned to `market.ngn_feed`; parsed by `read_ngn_price`.
    pub ngn_feed: UncheckedAccount<'info>,
}

/// Spec §11 `write_off_loan`. `remaining_accounts`: one `(CollateralAsset, PriceUpdateV2)` pair
/// per used collateral slot, in slot order.
///
/// Only for a liquidatable position whose remaining collateral is worth less than
/// `bad_debt_dust_usd` — below that, liquidating costs more than it recovers, so the loan is
/// cleared and the loss taken: the protocol reserve first, then the lenders through the share
/// price. The collateral stays in the position; sweeping it is a separate admin action.
///
/// The admin signs through a timelocked multisig, so a lender who watches the queue can
/// withdraw before the loss lands (spec §18). That is accepted: a write-off needs the
/// collateral to be dust, which bounds what the remaining lenders absorb.
/// Promo forfeiture (spec §11 step 3) arrives with Plan 5.
pub fn handle_write_off_loan<'info>(ctx: Context<'info, WriteOffLoan<'info>>, loan_id: u64) -> Result<()> {
    let market_key = ctx.accounts.market.key();
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    let market = &mut ctx.accounts.market;
    market.accrue(now)?;

    let mut position = ctx.accounts.position.load_mut()?;
    require_keys_eq!(position.market, market_key, HodlError::MarketMismatch);
    let index = position.loan_index(loan_id).ok_or(HodlError::LoanNotFound)?;

    let valuation = load_valuation(
        ctx.program_id,
        &position,
        market,
        &ctx.accounts.ngn_feed.to_account_info(),
        ctx.remaining_accounts,
        0,
        &clock,
    )?;
    require!(valuation.health.is_liquidatable(), HodlError::NotLiquidatable);
    require!(
        valuation.health.own_value < market.bad_debt_dust_usd,
        HodlError::WriteOffNotAllowed
    );

    let loan = position.loans[index];
    let released = accrued_lp_interest(
        loan.principal,
        loan.rate_bps,
        loan.reserve_factor_bps,
        loan.interest_anchor,
        now,
    )?;
    let loss = add(loan.principal as u128, released)?;

    market.accrued_interest = market.accrued_interest.saturating_sub(released);
    market.total_borrows = market
        .total_borrows
        .checked_sub(loan.principal)
        .ok_or(HodlError::MathOverflow)?;
    market.lp_rate_product = sub(
        market.lp_rate_product,
        lp_contribution(loan.principal, loan.rate_bps, loan.reserve_factor_bps)?,
    )?;

    // The reserve absorbs what it can; the rest reaches lenders as a fall in the share price.
    let covered = to_u64(loss.min(market.protocol_reserve as u128))?;
    market.protocol_reserve = market
        .protocol_reserve
        .checked_sub(covered)
        .ok_or(HodlError::MathOverflow)?;
    market.total_bad_debt = add(market.total_bad_debt, loss)?;

    position.loans[index] = bytemuck::Zeroable::zeroed();
    if !position.has_active_loans() {
        position.promo_last_activity_at = now;
    }
    let owner = position.owner;
    let total_bad_debt = market.total_bad_debt;
    drop(position);

    emit!(LoanWrittenOff {
        market: market_key,
        position: ctx.accounts.position.key(),
        owner,
        loan_id,
        principal: loan.principal,
        loss,
        covered_by_reserve: covered,
        total_bad_debt,
    });
    Ok(())
}
```

Replace `programs/hodl_loans/src/instructions/liquidation/mod.rs`:

```rust
pub mod liquidate;
pub mod write_off;

pub use liquidate::*;
pub use write_off::*;
```

In `lib.rs`, add inside `pub mod hodl_loans { … }` after `liquidate`:

```rust
    pub fn write_off_loan<'info>(ctx: Context<'info, WriteOffLoan<'info>>, loan_id: u64) -> Result<()> {
        instructions::handle_write_off_loan(ctx, loan_id)
    }
```

- [ ] **Step 4: Run the write-off tests, then the full suite and lints**

Run: `./scripts/test.sh --test write_off`
Expected: `test result: ok. 5 passed`.

Run: `./scripts/test.sh`
Expected: every test binary reports `ok`, 138 tests in all:

| Binary | Tests |
|---|---|
| unit | 37 |
| harness | 3 |
| admin | 6 |
| access | 6 |
| market | 8 |
| liquidity | 20 |
| sweep | 3 |
| collateral | 9 |
| position | 6 |
| loans | 12 |
| repay | 6 |
| withdraw | 5 |
| reserve | 2 |
| budget | 1 |
| invariants | 2 |
| liquidation | 7 |
| write_off | 5 |

Run: `cargo clippy -p hodl_loans --all-targets -- -D warnings`
Expected: finishes without errors. If clippy flags an issue, fix it in the file it names and re-run both commands.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: admin write-off of dust-collateral loans, reserve first"
```
