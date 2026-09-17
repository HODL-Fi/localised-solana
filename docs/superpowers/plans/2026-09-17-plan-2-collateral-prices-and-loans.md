# Plan 2: Collateral, Prices and Loans — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Whitelisted borrowers deposit standard collateral (SOL, USDC, USDT), take fixed-term cNGN loans priced by Pyth and Switchboard, repay them and withdraw collateral. The admin lists collateral and harvests the protocol reserve. Everything is tested in LiteSVM.

**Architecture:**
- **Builds on Plan 1** (`main` at `f15c9f1`): the lender pool, roles, whitelist and cNGN market already exist.
- **Layers.** Pure price, loan-balance and health arithmetic goes in `math/`. Oracle account parsing goes in `oracle/`. `valuation.rs` walks a position's collateral slots, checks the matching price accounts and produces a health value.
- **Positions** are zero-copy accounts (`AccountLoader`) with fixed-size collateral and loan slot arrays, so bots can read them at fixed offsets.
- **Tests** write Pyth `PriceUpdateV2` and Switchboard `PullFeedAccountData` accounts straight into LiteSVM.

**Tech Stack:** Rust 1.89+, Anchor 1.2.0, Solana CLI 3.1.x (`cargo build-sbf`, platform tools v1.52), LiteSVM 0.10.0, `pyth-solana-receiver-sdk` 2.0.0, `switchboard-on-demand` 0.13.0, `bytemuck` 1.

**Spec:** `docs/superpowers/specs/2026-09-17-solana-fixed-loans-design.md`

## Plan series

| Plan | Delivers |
|---|---|
| 1. Foundation and lender pool (merged) | Workspace, math, roles, whitelist/blacklist, cNGN market, lender deposit/withdraw, donation sweep |
| **2. Collateral, prices and loans** (this plan) | `CollateralAsset` (standard mints), positions, Pyth and Switchboard reads, health checks, `take_loan`, `repay_loan`, reserve harvest |
| 3. Liquidation and bad debt | `liquidate`, `write_off_loan` (reserve first) |
| 4. xStocks | Token-2022 extension policy, scaled-UI multiplier, transfer-time re-checks |
| 5. Promo balance | Promo vault, campaigns, Ed25519 vouchers, forfeiture, expiry, `set_promo_cap` |
| 6. Hardening and devnet | Trident invariant fuzzing, ported EVM regressions, devnet run, pre-audit scan |

## Global Constraints

Plan 1's constraints still apply:

- Anchor `1.2.0` for `anchor-lang` and `anchor-spl`; LiteSVM `0.10.0`; Rust `1.89` or newer; build with `cargo build-sbf --tools-version v1.52` (through `scripts/test.sh`).
- `[profile.release] overflow-checks = true`; all math on `u128` with checked operations.
- Rounding: borrower debt rounds up; lender accrual and minted shares round down; burned shares round up.
- Constants: `BPS = 10_000`, `YEAR = 31_536_000` seconds, `VIRTUAL_SHARES = 1_000`, `VIRTUAL_ASSETS = 1`, `MIN_TENURE = 86_400`, `MAX_COLLATERAL_SLOTS = 8`, `MAX_LOAN_SLOTS = 10`.
- Every account starts with `version: u8` and `bump: u8` and ends with reserved padding.
- In user-facing instructions, wrap `Market`, `LenderPosition` and every `InterfaceAccount` in `Box<…>`. The SBF stack frame is 4 KB.
- One `#[error_code] HodlError` enum. **Append new variants only**, since codes are `6000 + position`.
- The guardian can pause but never unpause; only the admin unpauses.
- `cash` changes only through program instructions. Tokens sent straight to a vault never change the share price.
- Market mints may carry only `MetadataPointer`, `TokenMetadata`, `PermanentDelegate`.
- Commits made by Claude end with the attribution trailer from the session instructions.

New in this plan:

- `USD_SCALE = 10^12` for all USD values. Collateral counts at `price − confidence` (round down); debt counts at `NGN price + spread` (round up).
- Healthy means `debt ≤ Σ value × ltv_bps / BPS`. Borrowing and withdrawing with active loans require it.
- A health check reads one `(CollateralAsset, PriceUpdateV2, mint)` triple per used collateral slot, in slot order, from the remaining accounts. A missing, extra or mismatched account fails with `PriceAccountMismatch`.
- Pyth price accounts must be owned by the receiver program `rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ` and fully verified. The Switchboard feed is trusted by address (`market.ngn_feed`) only.
- No `Option` fields in `Position` or `CollateralAsset` data (bots scan them). Use `Pubkey::default()` as the "unset" value. Optional *instruction accounts* are fine.
- New fields on existing accounts come out of `reserved`, so account sizes don't change.
- Standard collateral mints may carry only `MetadataPointer` and `TokenMetadata`.
- Seeds: `CollateralAsset ["collateral", mint]`, collateral vault `["collateral_vault", mint]`, `Position ["position", owner]`.
- Box `CollateralAsset` like `Market`.
- A loan copies `rate_bps`, `penalty_rate_bps` and `reserve_factor_bps` from the market when it opens. Later market changes never touch open loans.

## Facts verified while writing this plan (2026-09-17)

- **Pyth** (`pyth-solana-receiver-sdk` 2.0.0):
  - `PriceUpdateV2::get_price_no_older_than(&clock, max_age, &feed_id)` requires `VerificationLevel::Full`.
  - It fails with `GetPriceError::PriceTooOld`, `MismatchedFeedId` or other variants (such as insufficient verification).
- **Switchboard On-Demand** (0.13.0):
  - `PullFeedAccountData` is 3,200 bytes after an 8-byte discriminator.
  - Its `result: CurrentResult` holds `value` and `std_dev` as `i128` at 18 decimals, plus `slot` and `num_samples`.
  - `PullFeedAccountData::parse` does an aligned `bytemuck` cast that fails on host test buffers (`i128` aligns to 16). The program copies `result` out by offset with `pod_read_unaligned` instead.
- **Build fixes:**
  - The Pyth SDK pulls in `getrandom` 0.2 (through `pythnet-sdk → borsh 0.9 → ahash`). Without `getrandom = { version = "0.2", features = ["custom"] }`, `cargo build-sbf` fails on the unsupported target.
  - `#[account(zero_copy)]` needs a direct `bytemuck` dependency.
- **Behaviour to expect in tests:**
  - An `AccountLoader` for a position that doesn't exist fails with Anchor `AccountOwnedByWrongProgram` (3007), not `AccountNotInitialized` (3012). Clients should treat both as "no position".
  - LiteSVM starts at slot 0. The program reads a Switchboard result at slot 0 as never updated, so the harness warps to slot 1,000 before posting prices.
- **Resolved crate versions:** `pyth-solana-receiver-sdk` 2.0.0, `switchboard-on-demand` 0.13.0, `bytemuck` 1.25.2, `getrandom` 0.2.17, `solana-account` 3.4.0. If `cargo` resolves a newer `pyth-solana-receiver-sdk` or `switchboard-on-demand` and the build breaks, pin it with `cargo update -p <crate> --precise <version>`.
- **Verification.** The full Plan 2 code was compiled and tested before this plan was written:
  - 117 tests pass (32 unit, 85 LiteSVM), and `cargo clippy -D warnings` is clean.
  - Every task's end state was rebuilt from Plan 1 and passes its own suite and clippy.
  - Each task's failing-test step was run to capture its expected errors.

## Plan-level refinements to the spec

This plan's commit already writes these into the spec.

- **Accounts and constants:**
  - `USD_SCALE` is 10^12, not 10^18. At 10^18, `amount × price` for a large `u64` balance overflows `u128`.
  - `Market.accrual_remainder: u128` carries the accrual division remainder, taken from `reserved` (256 → 240 bytes). Frequent accrual no longer rounds lender interest away.
  - `Position.market` is the market its loans come from (`Pubkey::default()` until the first loan). `take_loan`, `repay_loan` and `withdraw_collateral` reject another market with `MarketMismatch`.
  - `Position` is zero-copy. `LoanSlot.active` is a `u8` stored last. A repaid slot is zeroed.
- **Validation and errors:**
  - Market parameters also require: rates ≤ 100%, a non-default `ngn_feed`, `ngn_max_stale_slots > 0`, `ngn_min_samples ≥ 1`, `ngn_max_spread_bps ≤ 100%` and `promo_inactivity_seconds ≥ 0`.
  - Two errors are appended: `MarketMismatch`, and `InsufficientCollateral` (withdrawing more than the slot holds, or a mint the position doesn't hold).
- **Price reads:**
  - The Switchboard spread is `result.std_dev`. Too few samples or an old result fail with `StalePrice`.
  - A Pyth account not owned by the receiver fails with `PriceAccountMismatch`.
- **Instructions:**
  - `withdraw_collateral` never accrues the market (accrual doesn't change a position's debt). Its `market` and `ngn_feed` accounts are optional and required only while loans are active. The price triples cover the slots still used after the withdrawal.
  - `delist_collateral` closes the vault and the asset account, so the vault must be empty. The new `sweep_collateral_excess` moves donations out first. (A nothing-to-sweep call fails with `AmountTooSmall`, like `sweep_market_excess`.)
  - `harvest_reserve` asking for more than the reserve fails with `InsufficientCash`.
  - `LoanRepaid` and `LoanPartiallyRepaid` carry both the position owner and the payer.
- **Scope notes:**
  - The instruction modules are `instructions/positions/` and `instructions/loans/` (spec §5 says `borrower/`). `positions` avoids clashing with the `state::position` glob re-export.
  - SOL is tested as a 9-decimal classic SPL Token mint. Clients wrap SOL before depositing. These collateral tests also cover classic SPL Token transfers, which Plan 1 left untested.
  - **Deferred:** the promo expiry in `take_loan` (spec §10 step 3), promo in health and the promo release in `close_position` arrive with Plan 5. `CollateralKind::XStock` checks arrive with Plan 4.

## File Structure

New and changed files under `programs/hodl_loans/`:

```text
Cargo.toml                                   + pyth, switchboard, getrandom, bytemuck; dev solana-account
src/lib.rs                                   + pub mod oracle, valuation; 12 entry points
src/constants.rs                             + USD_SCALE, USD_DECIMALS, collateral/position seeds
src/errors.rs                                + MarketMismatch, InsufficientCollateral
src/events.rs                                + collateral, position, loan and reserve events
src/valuation.rs                             position slots + price accounts → Health
src/math/mod.rs
src/math/interest.rs                         + accrue_lp_interest (remainder-carrying)
src/math/price.rs                            UsdPrice, Pyth/Switchboard scaling, token values
src/math/loan.rs                             LoanTerms, loan_balance, lp_contribution, R, reserve share
src/math/health.rs                           CollateralValue, Health, compute_health
src/oracle/mod.rs
src/oracle/pyth.rs                           read_pyth_price
src/oracle/switchboard.rs                    read_ngn_price
src/state/mod.rs
src/state/market.rs                          + accrual_remainder, stricter validate
src/state/collateral.rs                      CollateralAsset, CollateralKind, CollateralParams
src/state/position.rs                        Position, CollateralSlot, LoanSlot (zero-copy)
src/token/extensions.rs                      + STANDARD_COLLATERAL_EXTENSIONS
src/instructions/mod.rs
src/instructions/admin/mod.rs
src/instructions/admin/market_admin.rs       accrual_remainder literal
src/instructions/admin/collateral_admin.rs   list, update params, pause, delist
src/instructions/admin/sweep.rs              + sweep_collateral_excess
src/instructions/admin/reserve.rs            harvest_reserve
src/instructions/positions/mod.rs
src/instructions/positions/open_position.rs
src/instructions/positions/close_position.rs
src/instructions/positions/deposit_collateral.rs
src/instructions/positions/withdraw_collateral.rs
src/instructions/loans/mod.rs
src/instructions/loans/take_loan.rs
src/instructions/loans/repay_loan.rs
tests/common/mod.rs                          + one harness section per task
tests/market.rs                              + validation test
tests/collateral.rs
tests/position.rs
tests/loans.rs
tests/repay.rs
tests/withdraw.rs
tests/reserve.rs
```

All paths in the tasks below are relative to the repository root.

---

### Task 1: Accrual remainder, stricter market validation and shared constants

**Files:**
- Modify: `src/constants.rs`, `src/errors.rs`, `src/math/interest.rs`, `src/state/market.rs`, `src/instructions/admin/market_admin.rs` (under `programs/hodl_loans/`)
- Test: `programs/hodl_loans/tests/market.rs`

**Interfaces:**
- Consumes: Plan 1's `Market`, `MarketParams::validate`, `math::interest::lp_interest`, `math::checked::add`.
- Produces:
  - Constants `USD_SCALE: u128 = 10^12`, `USD_DECIMALS: i32 = 12`, `COLLATERAL_SEED = b"collateral"`, `COLLATERAL_VAULT_SEED = b"collateral_vault"`, `POSITION_SEED = b"position"`
  - `HodlError::MarketMismatch`, `HodlError::InsufficientCollateral` (appended after `MathOverflow`)
  - `math::interest::accrue_lp_interest(lp_rate_product: u128, elapsed_seconds: u64, remainder: u128) -> Result<(u128, u128)>` returning `(interest, new_remainder)`
  - `Market.accrual_remainder: u128` (before `reserved`, which shrinks to `[u8; 240]`)

- [ ] **Step 1: Write the failing tests**

Append to `programs/hodl_loans/tests/market.rs`:

```rust
#[test]
fn rate_bounds_and_price_feed_limits_are_validated() {
    let mut env = Env::initialized();
    let mint = env.create_mint(MintKind::CngnLike, 6);
    let admin = env.admin.pubkey();
    let base = default_market_params();

    let invalid = [
        hodl_loans::MarketParams { interest_rate_bps: 10_001, ..base },
        hodl_loans::MarketParams { penalty_rate_bps: 10_001, ..base },
        hodl_loans::MarketParams { ngn_feed: anchor_lang::prelude::Pubkey::default(), ..base },
        hodl_loans::MarketParams { ngn_max_stale_slots: 0, ..base },
        hodl_loans::MarketParams { ngn_min_samples: 0, ..base },
        hodl_loans::MarketParams { ngn_max_spread_bps: 10_001, ..base },
        hodl_loans::MarketParams { promo_inactivity_seconds: -1, ..base },
    ];
    for params in invalid {
        let instruction = create_market_ix(&admin, &mint, &TOKEN_2022, params);
        assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::InvalidParameters);
    }

    let boundaries = hodl_loans::MarketParams {
        interest_rate_bps: 10_000,
        penalty_rate_bps: 10_000,
        ngn_max_stale_slots: 1,
        ngn_min_samples: 1,
        ngn_max_spread_bps: 10_000,
        promo_inactivity_seconds: 0,
        ..base
    };
    let instruction = create_market_ix(&admin, &mint, &TOKEN_2022, boundaries);
    send(&mut env.svm, &[instruction], &[&env.admin]).unwrap();
    assert_eq!(env.market(&mint).params(), boundaries);
}
```

In `programs/hodl_loans/src/math/interest.rs`, add this test inside `mod tests`:

```rust
    #[test]
    fn remainder_carries_so_per_second_accrual_matches_one_shot() {
        // 1,000 cNGN at 10% with a 10% reserve accrues about 2.85 base units per second.
        let product = 1_000_000_000u128 * 1_000 * 9_000;
        let (mut total, mut remainder) = (0u128, 0u128);
        for _ in 0..3_600 {
            let (interest, next) = accrue_lp_interest(product, 1, remainder).unwrap();
            total += interest;
            remainder = next;
        }
        let (one_shot, one_shot_remainder) = accrue_lp_interest(product, 3_600, 0).unwrap();
        assert_eq!(total, one_shot);
        assert_eq!(remainder, one_shot_remainder);
        assert_eq!(one_shot, lp_interest(product, 3_600).unwrap());
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test market`
Expected: `rate_bounds_and_price_feed_limits_are_validated ... FAILED`, panicking in `assert_custom_error` with `transaction should have failed` (Plan 1 accepts a 10,001 bps interest rate).

Run: `cargo test -p hodl_loans --lib`
Expected: compile error `cannot find function accrue_lp_interest in this scope`.

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

In `programs/hodl_loans/src/errors.rs`, add these two variants after `MathOverflow` (the last variant):

```rust
    #[msg("Position's loans belong to a different market")]
    MarketMismatch,
    #[msg("Not enough collateral in the position")]
    InsufficientCollateral,
```

Replace `programs/hodl_loans/src/math/interest.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{BPS, YEAR};
use crate::errors::HodlError;
use crate::math::checked::{add, mul_div_floor};

/// Lender interest accrued over `elapsed_seconds` for the market's `lp_rate_product`
/// (Σ principal × rate_bps × (BPS − reserve_factor_bps)). Rounds down.
pub fn lp_interest(lp_rate_product: u128, elapsed_seconds: u64) -> Result<u128> {
    mul_div_floor(lp_rate_product, elapsed_seconds as u128, BPS * BPS * YEAR)
}

/// Lender interest for `elapsed_seconds`, carrying the division remainder between calls so
/// frequent accrual loses nothing to rounding. Returns `(interest, new_remainder)`.
pub fn accrue_lp_interest(lp_rate_product: u128, elapsed_seconds: u64, remainder: u128) -> Result<(u128, u128)> {
    let denominator = BPS * BPS * YEAR;
    let numerator = add(
        lp_rate_product.checked_mul(elapsed_seconds as u128).ok_or(HodlError::MathOverflow)?,
        remainder,
    )?;
    Ok((numerator / denominator, numerator % denominator))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_year_at_ten_percent_with_ten_percent_reserve() {
        // 1,000 cNGN (6 decimals) at 10% APR, 10% reserve factor: lenders earn 90 cNGN in a year.
        let principal: u128 = 1_000_000_000;
        let product = principal * 1_000 * 9_000;
        assert_eq!(lp_interest(product, 31_536_000).unwrap(), 90_000_000);
    }

    #[test]
    fn remainder_carries_so_per_second_accrual_matches_one_shot() {
        // 1,000 cNGN at 10% with a 10% reserve accrues about 2.85 base units per second.
        let product = 1_000_000_000u128 * 1_000 * 9_000;
        let (mut total, mut remainder) = (0u128, 0u128);
        for _ in 0..3_600 {
            let (interest, next) = accrue_lp_interest(product, 1, remainder).unwrap();
            total += interest;
            remainder = next;
        }
        let (one_shot, one_shot_remainder) = accrue_lp_interest(product, 3_600, 0).unwrap();
        assert_eq!(total, one_shot);
        assert_eq!(remainder, one_shot_remainder);
        assert_eq!(one_shot, lp_interest(product, 3_600).unwrap());
    }

    #[test]
    fn rounds_down_and_is_zero_without_loans() {
        assert_eq!(lp_interest(0, 31_536_000).unwrap(), 0);
        // One second of 1 unit at 1 bps with no reserve is far below one base unit.
        assert_eq!(lp_interest(10_000, 1).unwrap(), 0);
    }
}
```

Replace `programs/hodl_loans/src/state/market.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{MAX_BPS, MIN_TENURE};
use crate::errors::HodlError;
use crate::math::checked::{add, sub};
use crate::math::interest::accrue_lp_interest;

#[account]
#[derive(InitSpace)]
pub struct Market {
    pub version: u8,
    pub bump: u8,
    pub vault_bump: u8,
    pub mint: Pubkey,
    pub token_program: Pubkey,
    pub vault: Pubkey,
    pub decimals: u8,
    /// cNGN the program has recorded as held. Direct transfers to the vault are ignored.
    pub cash: u64,
    /// Outstanding loan principal.
    pub total_borrows: u64,
    /// Σ over active loans of principal × rate_bps × (BPS − reserve_factor_bps).
    pub lp_rate_product: u128,
    /// Lender interest accrued but not yet paid, already net of reserve.
    pub accrued_interest: u128,
    pub protocol_reserve: u64,
    pub total_bad_debt: u128,
    pub total_shares: u128,
    pub last_accrual_ts: i64,
    pub interest_rate_bps: u16,
    pub penalty_rate_bps: u16,
    pub reserve_factor_bps: u16,
    pub max_utilization_bps: u16,
    pub min_loan_amount: u64,
    pub max_tenure_seconds: i64,
    pub bad_debt_dust_usd: u128,
    pub ngn_feed: Pubkey,
    pub ngn_max_stale_slots: u64,
    pub ngn_min_samples: u32,
    pub ngn_max_spread_bps: u16,
    pub promo_inactivity_seconds: i64,
    pub max_promo_per_position: u64,
    pub paused: bool,
    /// Division remainder carried between accruals (numerator units of `lp_rate_product × seconds`).
    /// Taken from the reserved padding, so the account size is unchanged.
    pub accrual_remainder: u128,
    pub reserved: [u8; 240],
}

/// Admin-settable market parameters, used by `create_market` and `update_market_params`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct MarketParams {
    pub interest_rate_bps: u16,
    pub penalty_rate_bps: u16,
    pub reserve_factor_bps: u16,
    pub max_utilization_bps: u16,
    pub min_loan_amount: u64,
    pub max_tenure_seconds: i64,
    pub bad_debt_dust_usd: u128,
    pub ngn_feed: Pubkey,
    pub ngn_max_stale_slots: u64,
    pub ngn_min_samples: u32,
    pub ngn_max_spread_bps: u16,
    pub promo_inactivity_seconds: i64,
    pub max_promo_per_position: u64,
}

impl MarketParams {
    pub fn validate(&self) -> Result<()> {
        require!(self.interest_rate_bps <= MAX_BPS, HodlError::InvalidParameters);
        require!(self.penalty_rate_bps <= MAX_BPS, HodlError::InvalidParameters);
        require!(self.reserve_factor_bps <= MAX_BPS, HodlError::InvalidParameters);
        require!(self.max_utilization_bps <= MAX_BPS, HodlError::InvalidParameters);
        require!(self.max_tenure_seconds >= MIN_TENURE, HodlError::InvalidParameters);
        require!(self.ngn_feed != Pubkey::default(), HodlError::InvalidParameters);
        require!(self.ngn_max_stale_slots > 0, HodlError::InvalidParameters);
        require!(self.ngn_min_samples >= 1, HodlError::InvalidParameters);
        require!(self.ngn_max_spread_bps <= MAX_BPS, HodlError::InvalidParameters);
        require!(self.promo_inactivity_seconds >= 0, HodlError::InvalidParameters);
        Ok(())
    }
}

impl Market {
    pub fn params(&self) -> MarketParams {
        MarketParams {
            interest_rate_bps: self.interest_rate_bps,
            penalty_rate_bps: self.penalty_rate_bps,
            reserve_factor_bps: self.reserve_factor_bps,
            max_utilization_bps: self.max_utilization_bps,
            min_loan_amount: self.min_loan_amount,
            max_tenure_seconds: self.max_tenure_seconds,
            bad_debt_dust_usd: self.bad_debt_dust_usd,
            ngn_feed: self.ngn_feed,
            ngn_max_stale_slots: self.ngn_max_stale_slots,
            ngn_min_samples: self.ngn_min_samples,
            ngn_max_spread_bps: self.ngn_max_spread_bps,
            promo_inactivity_seconds: self.promo_inactivity_seconds,
            max_promo_per_position: self.max_promo_per_position,
        }
    }

    pub fn apply_params(&mut self, p: &MarketParams) {
        self.interest_rate_bps = p.interest_rate_bps;
        self.penalty_rate_bps = p.penalty_rate_bps;
        self.reserve_factor_bps = p.reserve_factor_bps;
        self.max_utilization_bps = p.max_utilization_bps;
        self.min_loan_amount = p.min_loan_amount;
        self.max_tenure_seconds = p.max_tenure_seconds;
        self.bad_debt_dust_usd = p.bad_debt_dust_usd;
        self.ngn_feed = p.ngn_feed;
        self.ngn_max_stale_slots = p.ngn_max_stale_slots;
        self.ngn_min_samples = p.ngn_min_samples;
        self.ngn_max_spread_bps = p.ngn_max_spread_bps;
        self.promo_inactivity_seconds = p.promo_inactivity_seconds;
        self.max_promo_per_position = p.max_promo_per_position;
    }

    /// Adds lender interest accrued since `last_accrual_ts`. Every instruction that
    /// touches the market calls this first.
    pub fn accrue(&mut self, now: i64) -> Result<()> {
        if now <= self.last_accrual_ts {
            return Ok(());
        }
        let elapsed = (now - self.last_accrual_ts) as u64;
        let (interest, remainder) = accrue_lp_interest(self.lp_rate_product, elapsed, self.accrual_remainder)?;
        self.accrued_interest = add(self.accrued_interest, interest)?;
        self.accrual_remainder = remainder;
        self.last_accrual_ts = now;
        Ok(())
    }

    /// cash + total_borrows + accrued_interest − protocol_reserve. Call after `accrue`.
    pub fn total_assets(&self) -> Result<u128> {
        let gross = add(add(self.cash as u128, self.total_borrows as u128)?, self.accrued_interest)?;
        sub(gross, self.protocol_reserve as u128)
    }

    /// Cash lenders may withdraw or borrowers may draw.
    pub fn available_cash(&self) -> u64 {
        self.cash.saturating_sub(self.protocol_reserve)
    }
}
```

In `programs/hodl_loans/src/instructions/admin/market_admin.rs`, in the `Market { … }` literal of `handle_create_market`, replace `reserved: [0; 256],` with:

```rust
        accrual_remainder: 0,
        reserved: [0; 240],
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh`
Expected: every binary reports `ok`. The unit binary shows `11 passed` and `market` shows `8 passed`.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: carry accrual remainder, validate rate and feed limits, add Plan 2 constants"
```

---

### Task 2: Price scaling and oracle reads

**Files:**
- Create: `programs/hodl_loans/src/math/price.rs`, `src/oracle/mod.rs`, `src/oracle/pyth.rs`, `src/oracle/switchboard.rs` (under `programs/hodl_loans/`)
- Modify: `programs/hodl_loans/Cargo.toml`, `src/math/mod.rs`, `src/lib.rs`, `Cargo.lock` (updated by cargo)

**Interfaces:**
- Consumes: `BPS`, `USD_DECIMALS` (Task 1), `math::checked::{add, mul_div_floor, mul_div_ceil}`, `Market.{ngn_feed, ngn_max_stale_slots, ngn_min_samples, ngn_max_spread_bps}`.
- Produces:
  - `math::price::UsdPrice { price: u128, conf: u128 }` with `lower(&self) -> u128` (saturating `price − conf`) and `upper(&self) -> Result<u128>`
  - `math::price::scale_pyth_price(price: i64, conf: u64, exponent: i32) -> Result<UsdPrice>`
  - `math::price::scale_switchboard_value(value: i128, spread: i128) -> Result<UsdPrice>` (inputs at 18 decimals)
  - `math::price::require_confidence(price: &UsdPrice, max_bps: u16) -> Result<()>`
  - `math::price::{token_value(amount: u128, decimals: u8, price: u128) -> Result<u128>, token_value_ceil(..)}`
  - `oracle::pyth::read_pyth_price(account: &AccountInfo, feed_id: &[u8; 32], max_age_seconds: u64, max_conf_bps: u16, clock: &Clock) -> Result<UsdPrice>`
  - `oracle::switchboard::read_ngn_price(account: &AccountInfo, market: &Market, clock: &Clock) -> Result<UsdPrice>`

- [ ] **Step 1: Add dependencies, wire modules and write the failing tests**

Replace `programs/hodl_loans/Cargo.toml`:

```toml
[package]
name = "hodl_loans"
version = "0.1.0"
description = "HODL fixed-term cNGN loans on Solana"
edition.workspace = true
rust-version.workspace = true

[lib]
crate-type = ["cdylib", "lib"]
name = "hodl_loans"

[features]
default = []
cpi = ["no-entrypoint"]
no-entrypoint = []
no-log-ix-name = []
idl-build = ["anchor-lang/idl-build", "anchor-spl/idl-build"]
anchor-debug = []
custom-heap = []
custom-panic = []

[dependencies]
anchor-lang = { version = "1.2.0", features = ["init-if-needed"] }
anchor-spl = { version = "1.2.0", default-features = false, features = ["token", "token_2022", "token_2022_extensions"] }
pyth-solana-receiver-sdk = "2.0.0"
switchboard-on-demand = { version = "0.13.0", default-features = false, features = ["solana-v3"] }
getrandom = { version = "0.2", features = ["custom"] }
bytemuck = { version = "1", features = ["derive", "min_const_generics"] }

[dev-dependencies]
litesvm = "0.10.0"
solana-account = "3"
solana-keypair = "3.0.1"
solana-message = "3.0.1"
solana-signer = "3.0.0"
solana-transaction = "3.0.2"
spl-token-2022-interface = "2"
spl-token-interface = "2"

[lints.rust]
unexpected_cfgs = { level = "warn", check-cfg = ['cfg(target_os, values("solana"))'] }
```

Replace `programs/hodl_loans/src/math/mod.rs`:

```rust
pub mod checked;
pub mod interest;
pub mod price;
pub mod shares;
```

In `programs/hodl_loans/src/lib.rs`, add `pub mod oracle;` directly below `pub mod math;`.

`programs/hodl_loans/src/oracle/mod.rs`:

```rust
pub mod pyth;
pub mod switchboard;
```

Create each new file containing only its imports and test module for now.

`programs/hodl_loans/src/math/price.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{BPS, USD_DECIMALS};
use crate::errors::HodlError;
use crate::math::checked::{add, mul_div_ceil, mul_div_floor};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pyth_exponents_rescale_to_twelve_decimals() {
        // SOL at $150.12345678 with exponent -8.
        let p = scale_pyth_price(15_012_345_678, 1_500_000, -8).unwrap();
        assert_eq!(p.price, 150_123_456_780_000);
        assert_eq!(p.conf, 15_000_000_000);
        // Exponent -15 shrinks and rounds down.
        let p = scale_pyth_price(1_000_000_000_000_001, 0, -15).unwrap();
        assert_eq!(p.price, 1_000_000_000_000);
        // Positive exponent.
        assert_eq!(scale_pyth_price(3, 0, 2).unwrap().price, 300_000_000_000_000);
    }

    #[test]
    fn pyth_rejects_non_positive_and_vanishing_prices() {
        assert!(scale_pyth_price(0, 0, -8).is_err());
        assert!(scale_pyth_price(-5, 0, -8).is_err());
        assert!(scale_pyth_price(1, 0, -20).is_err());
    }

    #[test]
    fn switchboard_value_rescales_from_eighteen_decimals() {
        // NGN/USD ≈ 0.00065 with a 0.0000013 spread.
        let p = scale_switchboard_value(650_000_000_000_000, 1_300_000_000_000).unwrap();
        assert_eq!(p.price, 650_000_000);
        assert_eq!(p.conf, 1_300_000);
        assert!(scale_switchboard_value(0, 0).is_err());
        assert!(scale_switchboard_value(1, -1).is_err());
    }

    #[test]
    fn confidence_limit_is_inclusive() {
        let p = UsdPrice { price: 1_000_000, conf: 20_000 };
        require_confidence(&p, 200).unwrap();
        assert!(require_confidence(&p, 199).is_err());
    }

    #[test]
    fn token_values_use_decimals_and_round_directions() {
        // 2.5 tokens with 9 decimals at $150.
        let price = 150 * 1_000_000_000_000u128;
        assert_eq!(token_value(2_500_000_000, 9, price).unwrap(), 375 * 1_000_000_000_000);
        // 1 base unit of a 6-decimal token at $0.00065 is 650 USD-scale units per million.
        assert_eq!(token_value(1, 6, 650_000_000).unwrap(), 650);
        assert_eq!(token_value(1, 6, 650_000_001).unwrap(), 650);
        assert_eq!(token_value_ceil(1, 6, 650_000_001).unwrap(), 651);
        assert_eq!(UsdPrice { price: 10, conf: 15 }.lower(), 0);
    }
}
```

`programs/hodl_loans/src/oracle/pyth.rs`:

```rust
use anchor_lang::prelude::*;
use pyth_solana_receiver_sdk::error::GetPriceError;
use pyth_solana_receiver_sdk::price_update::PriceUpdateV2;

use crate::errors::HodlError;
use crate::math::price::{require_confidence, scale_pyth_price, UsdPrice};

#[cfg(test)]
pub mod tests {
    use super::*;
    use pyth_solana_receiver_sdk::price_update::{PriceFeedMessage, VerificationLevel};

    /// Serialized `PriceUpdateV2` account data (shared with the LiteSVM harness via copy).
    pub fn price_update_data(
        feed_id: [u8; 32],
        price: i64,
        conf: u64,
        exponent: i32,
        publish_time: i64,
        verification_level: VerificationLevel,
    ) -> Vec<u8> {
        let update = PriceUpdateV2 {
            write_authority: Pubkey::new_unique(),
            verification_level,
            price_message: PriceFeedMessage {
                feed_id,
                price,
                conf,
                exponent,
                publish_time,
                prev_publish_time: publish_time - 1,
                ema_price: price,
                ema_conf: conf,
            },
            posted_slot: 1,
        };
        let mut data = Vec::new();
        update.try_serialize(&mut data).unwrap();
        data
    }

    fn read(data: &mut [u8], owner: &Pubkey, feed: [u8; 32], now: i64, max_conf_bps: u16) -> Result<UsdPrice> {
        let key = Pubkey::new_unique();
        let mut lamports = 1_000_000u64;
        let info = AccountInfo::new(&key, false, false, &mut lamports, data, owner, false);
        let clock = Clock { unix_timestamp: now, ..Clock::default() };
        read_pyth_price(&info, &feed, 60, max_conf_bps, &clock)
    }

    const FEED: [u8; 32] = [9; 32];

    #[test]
    fn fresh_full_price_is_scaled() {
        let mut data = price_update_data(FEED, 15_000_000_000, 10_000_000, -8, 1_000, VerificationLevel::Full);
        let p = read(&mut data, &pyth_solana_receiver_sdk::ID, FEED, 1_030, 200).unwrap();
        assert_eq!(p.price, 150_000_000_000_000);
        assert_eq!(p.conf, 100_000_000_000);
    }

    #[test]
    fn rejections_map_to_program_errors() {
        let owner = pyth_solana_receiver_sdk::ID;
        let fresh = || price_update_data(FEED, 15_000_000_000, 10_000_000, -8, 1_000, VerificationLevel::Full);

        let err = |r: Result<UsdPrice>| r.unwrap_err();
        assert_eq!(err(read(&mut fresh(), &Pubkey::new_unique(), FEED, 1_000, 200)), HodlError::PriceAccountMismatch.into());
        assert_eq!(err(read(&mut fresh(), &owner, [8; 32], 1_000, 200)), HodlError::PriceAccountMismatch.into());
        assert_eq!(err(read(&mut fresh(), &owner, FEED, 1_061, 200)), HodlError::StalePrice.into());
        // $0.10 confidence on $150 is 6.67 bps: allowed at 7, rejected at 6.
        read(&mut fresh(), &owner, FEED, 1_000, 7).unwrap();
        assert_eq!(err(read(&mut fresh(), &owner, FEED, 1_000, 6)), HodlError::PriceConfidenceTooWide.into());
        let mut partial =
            price_update_data(FEED, 15_000_000_000, 0, -8, 1_000, VerificationLevel::Partial { num_signatures: 5 });
        assert_eq!(err(read(&mut partial, &owner, FEED, 1_000, 200)), HodlError::InvalidPrice.into());
        let mut negative = price_update_data(FEED, -1, 0, -8, 1_000, VerificationLevel::Full);
        assert_eq!(err(read(&mut negative, &owner, FEED, 1_000, 200)), HodlError::InvalidPrice.into());
    }
}
```

`programs/hodl_loans/src/oracle/switchboard.rs`:

```rust
use anchor_lang::prelude::*;
use switchboard_on_demand::{CurrentResult, Discriminator, PullFeedAccountData};

use crate::errors::HodlError;
use crate::math::price::{require_confidence, scale_switchboard_value, UsdPrice};
use crate::state::Market;

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Serialized `PullFeedAccountData` account data with only the aggregated result set.
    pub fn pull_feed_data(value: i128, std_dev: i128, slot: u64, num_samples: u8) -> Vec<u8> {
        let mut feed: PullFeedAccountData = bytemuck::Zeroable::zeroed();
        feed.result.value = value;
        feed.result.std_dev = std_dev;
        feed.result.slot = slot;
        feed.result.num_samples = num_samples;
        let mut data = PullFeedAccountData::DISCRIMINATOR.to_vec();
        data.extend_from_slice(bytemuck::bytes_of(&feed));
        data
    }

    fn market(feed: Pubkey) -> Market {
        let mut m: Market = unsafe { std::mem::zeroed() };
        m.ngn_feed = feed;
        m.ngn_max_stale_slots = 150;
        m.ngn_min_samples = 3;
        m.ngn_max_spread_bps = 200;
        m
    }

    fn read(key: Pubkey, data: &mut [u8], m: &Market, slot: u64) -> Result<UsdPrice> {
        let owner = Pubkey::new_unique();
        let mut lamports = 1_000_000u64;
        let info = AccountInfo::new(&key, false, false, &mut lamports, data, &owner, false);
        let clock = Clock { slot, ..Clock::default() };
        read_ngn_price(&info, m, &clock)
    }

    // $0.000625 per NGN, 18 decimals.
    const VALUE: i128 = 625_000_000_000_000;

    #[test]
    fn fresh_result_is_scaled() {
        let key = Pubkey::new_unique();
        let m = market(key);
        let mut data = pull_feed_data(VALUE, 6_250_000_000_000, 1_000, 5);
        let p = read(key, &mut data, &m, 1_100).unwrap();
        assert_eq!(p.price, 625_000_000);
        assert_eq!(p.conf, 6_250_000);
    }

    #[test]
    fn rejections_map_to_program_errors() {
        let key = Pubkey::new_unique();
        let m = market(key);
        let err = |r: Result<UsdPrice>| r.unwrap_err();
        assert_eq!(err(read(Pubkey::new_unique(), &mut pull_feed_data(VALUE, 0, 1_000, 5), &m, 1_000)), HodlError::PriceAccountMismatch.into());
        let mut garbage = vec![0u8; 3_208];
        assert_eq!(err(read(key, &mut garbage, &m, 1_000)), HodlError::PriceAccountMismatch.into());
        assert_eq!(err(read(key, &mut pull_feed_data(VALUE, 0, 1_000, 5), &m, 1_151)), HodlError::StalePrice.into());
        assert_eq!(err(read(key, &mut pull_feed_data(VALUE, 0, 0, 5), &m, 10)), HodlError::StalePrice.into());
        assert_eq!(err(read(key, &mut pull_feed_data(VALUE, 0, 1_000, 2), &m, 1_000)), HodlError::StalePrice.into());
        assert_eq!(err(read(key, &mut pull_feed_data(0, 0, 1_000, 5), &m, 1_000)), HodlError::InvalidPrice.into());
        // 2% spread is the limit; just above fails.
        read(key, &mut pull_feed_data(VALUE, 12_500_000_000_000, 1_000, 5), &m, 1_000).unwrap();
        assert_eq!(err(read(key, &mut pull_feed_data(VALUE, 12_500_001_000_000, 1_000, 5), &m, 1_000)), HodlError::PriceConfidenceTooWide.into());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p hodl_loans --lib`
Expected: cargo downloads the new crates, then compile errors such as `unresolved imports crate::math::price::require_confidence, crate::math::price::scale_pyth_price, crate::math::price::UsdPrice` and `cannot find function scale_pyth_price in this scope`.

- [ ] **Step 3: Implement**

Replace each file with the full version below. It keeps the same imports and tests and adds the functions.

`programs/hodl_loans/src/math/price.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{BPS, USD_DECIMALS};
use crate::errors::HodlError;
use crate::math::checked::{add, mul_div_ceil, mul_div_floor};

/// A price in USD per whole token, at `USD_SCALE`, with its uncertainty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsdPrice {
    pub price: u128,
    /// Confidence (Pyth) or spread (Switchboard), same scale as `price`.
    pub conf: u128,
}

impl UsdPrice {
    /// `price − conf`, used for collateral and promo value.
    pub fn lower(&self) -> u128 {
        self.price.saturating_sub(self.conf)
    }

    /// `price + conf`, used for debt value.
    pub fn upper(&self) -> Result<u128> {
        add(self.price, self.conf)
    }
}

fn pow10(exp: u32) -> Result<u128> {
    Ok(10u128.checked_pow(exp).ok_or(HodlError::MathOverflow)?)
}

/// Rescale `value × 10^exponent` to `USD_SCALE`. Rounds down when shrinking.
fn rescale(value: u128, exponent: i32) -> Result<u128> {
    let shift = exponent + USD_DECIMALS;
    if shift >= 0 {
        Ok(value.checked_mul(pow10(shift as u32)?).ok_or(HodlError::MathOverflow)?)
    } else {
        Ok(value / pow10((-shift) as u32)?)
    }
}

/// Convert a Pyth `price`/`conf` with `exponent` into a `UsdPrice`. Rejects non-positive prices.
pub fn scale_pyth_price(price: i64, conf: u64, exponent: i32) -> Result<UsdPrice> {
    require!(price > 0, HodlError::InvalidPrice);
    let scaled = rescale(price as u128, exponent)?;
    require!(scaled > 0, HodlError::InvalidPrice);
    Ok(UsdPrice { price: scaled, conf: rescale(conf as u128, exponent)? })
}

/// Convert a Switchboard value and spread (both 18-decimal fixed point) into a `UsdPrice`.
pub fn scale_switchboard_value(value: i128, spread: i128) -> Result<UsdPrice> {
    require!(value > 0 && spread >= 0, HodlError::InvalidPrice);
    let scaled = rescale(value as u128, -18)?;
    require!(scaled > 0, HodlError::InvalidPrice);
    Ok(UsdPrice { price: scaled, conf: rescale(spread as u128, -18)? })
}

/// Reject a price whose uncertainty exceeds `max_bps` of the price.
pub fn require_confidence(price: &UsdPrice, max_bps: u16) -> Result<()> {
    let conf_scaled = price.conf.checked_mul(BPS).ok_or(HodlError::MathOverflow)?;
    let limit = price.price.checked_mul(max_bps as u128).ok_or(HodlError::MathOverflow)?;
    require!(conf_scaled <= limit, HodlError::PriceConfidenceTooWide);
    Ok(())
}

/// USD value (at `USD_SCALE`) of `amount` base units of a token with `decimals`. Rounds down.
pub fn token_value(amount: u128, decimals: u8, price: u128) -> Result<u128> {
    mul_div_floor(amount, price, pow10(decimals as u32)?)
}

/// Same as `token_value` but rounds up (used for debt).
pub fn token_value_ceil(amount: u128, decimals: u8, price: u128) -> Result<u128> {
    mul_div_ceil(amount, price, pow10(decimals as u32)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pyth_exponents_rescale_to_twelve_decimals() {
        // SOL at $150.12345678 with exponent -8.
        let p = scale_pyth_price(15_012_345_678, 1_500_000, -8).unwrap();
        assert_eq!(p.price, 150_123_456_780_000);
        assert_eq!(p.conf, 15_000_000_000);
        // Exponent -15 shrinks and rounds down.
        let p = scale_pyth_price(1_000_000_000_000_001, 0, -15).unwrap();
        assert_eq!(p.price, 1_000_000_000_000);
        // Positive exponent.
        assert_eq!(scale_pyth_price(3, 0, 2).unwrap().price, 300_000_000_000_000);
    }

    #[test]
    fn pyth_rejects_non_positive_and_vanishing_prices() {
        assert!(scale_pyth_price(0, 0, -8).is_err());
        assert!(scale_pyth_price(-5, 0, -8).is_err());
        assert!(scale_pyth_price(1, 0, -20).is_err());
    }

    #[test]
    fn switchboard_value_rescales_from_eighteen_decimals() {
        // NGN/USD ≈ 0.00065 with a 0.0000013 spread.
        let p = scale_switchboard_value(650_000_000_000_000, 1_300_000_000_000).unwrap();
        assert_eq!(p.price, 650_000_000);
        assert_eq!(p.conf, 1_300_000);
        assert!(scale_switchboard_value(0, 0).is_err());
        assert!(scale_switchboard_value(1, -1).is_err());
    }

    #[test]
    fn confidence_limit_is_inclusive() {
        let p = UsdPrice { price: 1_000_000, conf: 20_000 };
        require_confidence(&p, 200).unwrap();
        assert!(require_confidence(&p, 199).is_err());
    }

    #[test]
    fn token_values_use_decimals_and_round_directions() {
        // 2.5 tokens with 9 decimals at $150.
        let price = 150 * 1_000_000_000_000u128;
        assert_eq!(token_value(2_500_000_000, 9, price).unwrap(), 375 * 1_000_000_000_000);
        // 1 base unit of a 6-decimal token at $0.00065 is 650 USD-scale units per million.
        assert_eq!(token_value(1, 6, 650_000_000).unwrap(), 650);
        assert_eq!(token_value(1, 6, 650_000_001).unwrap(), 650);
        assert_eq!(token_value_ceil(1, 6, 650_000_001).unwrap(), 651);
        assert_eq!(UsdPrice { price: 10, conf: 15 }.lower(), 0);
    }
}
```

`programs/hodl_loans/src/oracle/pyth.rs`:

```rust
use anchor_lang::prelude::*;
use pyth_solana_receiver_sdk::error::GetPriceError;
use pyth_solana_receiver_sdk::price_update::PriceUpdateV2;

use crate::errors::HodlError;
use crate::math::price::{require_confidence, scale_pyth_price, UsdPrice};

/// Read a fully verified Pyth pull price no older than `max_age_seconds` for `feed_id`.
/// The account must be owned by the Pyth receiver program.
pub fn read_pyth_price(
    account: &AccountInfo,
    feed_id: &[u8; 32],
    max_age_seconds: u64,
    max_conf_bps: u16,
    clock: &Clock,
) -> Result<UsdPrice> {
    require_keys_eq!(*account.owner, pyth_solana_receiver_sdk::ID, HodlError::PriceAccountMismatch);
    let data = account.try_borrow_data()?;
    let update =
        PriceUpdateV2::try_deserialize(&mut &data[..]).map_err(|_| HodlError::PriceAccountMismatch)?;
    let price = update
        .get_price_no_older_than(clock, max_age_seconds, feed_id)
        .map_err(|e| match e {
            GetPriceError::PriceTooOld => HodlError::StalePrice,
            GetPriceError::MismatchedFeedId => HodlError::PriceAccountMismatch,
            _ => HodlError::InvalidPrice,
        })?;
    let usd = scale_pyth_price(price.price, price.conf, price.exponent)?;
    require_confidence(&usd, max_conf_bps)?;
    Ok(usd)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use pyth_solana_receiver_sdk::price_update::{PriceFeedMessage, VerificationLevel};

    /// Serialized `PriceUpdateV2` account data (shared with the LiteSVM harness via copy).
    pub fn price_update_data(
        feed_id: [u8; 32],
        price: i64,
        conf: u64,
        exponent: i32,
        publish_time: i64,
        verification_level: VerificationLevel,
    ) -> Vec<u8> {
        let update = PriceUpdateV2 {
            write_authority: Pubkey::new_unique(),
            verification_level,
            price_message: PriceFeedMessage {
                feed_id,
                price,
                conf,
                exponent,
                publish_time,
                prev_publish_time: publish_time - 1,
                ema_price: price,
                ema_conf: conf,
            },
            posted_slot: 1,
        };
        let mut data = Vec::new();
        update.try_serialize(&mut data).unwrap();
        data
    }

    fn read(data: &mut [u8], owner: &Pubkey, feed: [u8; 32], now: i64, max_conf_bps: u16) -> Result<UsdPrice> {
        let key = Pubkey::new_unique();
        let mut lamports = 1_000_000u64;
        let info = AccountInfo::new(&key, false, false, &mut lamports, data, owner, false);
        let clock = Clock { unix_timestamp: now, ..Clock::default() };
        read_pyth_price(&info, &feed, 60, max_conf_bps, &clock)
    }

    const FEED: [u8; 32] = [9; 32];

    #[test]
    fn fresh_full_price_is_scaled() {
        let mut data = price_update_data(FEED, 15_000_000_000, 10_000_000, -8, 1_000, VerificationLevel::Full);
        let p = read(&mut data, &pyth_solana_receiver_sdk::ID, FEED, 1_030, 200).unwrap();
        assert_eq!(p.price, 150_000_000_000_000);
        assert_eq!(p.conf, 100_000_000_000);
    }

    #[test]
    fn rejections_map_to_program_errors() {
        let owner = pyth_solana_receiver_sdk::ID;
        let fresh = || price_update_data(FEED, 15_000_000_000, 10_000_000, -8, 1_000, VerificationLevel::Full);

        let err = |r: Result<UsdPrice>| r.unwrap_err();
        assert_eq!(err(read(&mut fresh(), &Pubkey::new_unique(), FEED, 1_000, 200)), HodlError::PriceAccountMismatch.into());
        assert_eq!(err(read(&mut fresh(), &owner, [8; 32], 1_000, 200)), HodlError::PriceAccountMismatch.into());
        assert_eq!(err(read(&mut fresh(), &owner, FEED, 1_061, 200)), HodlError::StalePrice.into());
        // $0.10 confidence on $150 is 6.67 bps: allowed at 7, rejected at 6.
        read(&mut fresh(), &owner, FEED, 1_000, 7).unwrap();
        assert_eq!(err(read(&mut fresh(), &owner, FEED, 1_000, 6)), HodlError::PriceConfidenceTooWide.into());
        let mut partial =
            price_update_data(FEED, 15_000_000_000, 0, -8, 1_000, VerificationLevel::Partial { num_signatures: 5 });
        assert_eq!(err(read(&mut partial, &owner, FEED, 1_000, 200)), HodlError::InvalidPrice.into());
        let mut negative = price_update_data(FEED, -1, 0, -8, 1_000, VerificationLevel::Full);
        assert_eq!(err(read(&mut negative, &owner, FEED, 1_000, 200)), HodlError::InvalidPrice.into());
    }
}
```

`programs/hodl_loans/src/oracle/switchboard.rs`:

```rust
use anchor_lang::prelude::*;
use switchboard_on_demand::{CurrentResult, Discriminator, PullFeedAccountData};

use crate::errors::HodlError;
use crate::math::price::{require_confidence, scale_switchboard_value, UsdPrice};
use crate::state::Market;

/// Read the market's Switchboard On-Demand NGN/USD pull feed.
///
/// The account address must equal `market.ngn_feed` (set by the admin), so only the feed's
/// owner program can have written it; the data must carry the `PullFeedAccountData`
/// discriminator and full length. Only the 128-byte aggregated `result` is copied out (by
/// offset, unaligned), keeping the 3.2 KB feed off the stack. `value` is the price and
/// `std_dev` the spread.
pub fn read_ngn_price(account: &AccountInfo, market: &Market, clock: &Clock) -> Result<UsdPrice> {
    require_keys_eq!(account.key(), market.ngn_feed, HodlError::PriceAccountMismatch);
    let data = account.try_borrow_data()?;
    require!(
        data.len() >= 8 + std::mem::size_of::<PullFeedAccountData>()
            && data[..8] == *PullFeedAccountData::DISCRIMINATOR,
        HodlError::PriceAccountMismatch
    );
    let start = 8 + std::mem::offset_of!(PullFeedAccountData, result);
    let result: CurrentResult =
        bytemuck::pod_read_unaligned(&data[start..start + std::mem::size_of::<CurrentResult>()]);
    require!(
        result.slot > 0 && clock.slot.saturating_sub(result.slot) <= market.ngn_max_stale_slots,
        HodlError::StalePrice
    );
    require!(result.num_samples as u32 >= market.ngn_min_samples, HodlError::StalePrice);
    let price = scale_switchboard_value(result.value, result.std_dev)?;
    require_confidence(&price, market.ngn_max_spread_bps)?;
    Ok(price)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Serialized `PullFeedAccountData` account data with only the aggregated result set.
    pub fn pull_feed_data(value: i128, std_dev: i128, slot: u64, num_samples: u8) -> Vec<u8> {
        let mut feed: PullFeedAccountData = bytemuck::Zeroable::zeroed();
        feed.result.value = value;
        feed.result.std_dev = std_dev;
        feed.result.slot = slot;
        feed.result.num_samples = num_samples;
        let mut data = PullFeedAccountData::DISCRIMINATOR.to_vec();
        data.extend_from_slice(bytemuck::bytes_of(&feed));
        data
    }

    fn market(feed: Pubkey) -> Market {
        let mut m: Market = unsafe { std::mem::zeroed() };
        m.ngn_feed = feed;
        m.ngn_max_stale_slots = 150;
        m.ngn_min_samples = 3;
        m.ngn_max_spread_bps = 200;
        m
    }

    fn read(key: Pubkey, data: &mut [u8], m: &Market, slot: u64) -> Result<UsdPrice> {
        let owner = Pubkey::new_unique();
        let mut lamports = 1_000_000u64;
        let info = AccountInfo::new(&key, false, false, &mut lamports, data, &owner, false);
        let clock = Clock { slot, ..Clock::default() };
        read_ngn_price(&info, m, &clock)
    }

    // $0.000625 per NGN, 18 decimals.
    const VALUE: i128 = 625_000_000_000_000;

    #[test]
    fn fresh_result_is_scaled() {
        let key = Pubkey::new_unique();
        let m = market(key);
        let mut data = pull_feed_data(VALUE, 6_250_000_000_000, 1_000, 5);
        let p = read(key, &mut data, &m, 1_100).unwrap();
        assert_eq!(p.price, 625_000_000);
        assert_eq!(p.conf, 6_250_000);
    }

    #[test]
    fn rejections_map_to_program_errors() {
        let key = Pubkey::new_unique();
        let m = market(key);
        let err = |r: Result<UsdPrice>| r.unwrap_err();
        assert_eq!(err(read(Pubkey::new_unique(), &mut pull_feed_data(VALUE, 0, 1_000, 5), &m, 1_000)), HodlError::PriceAccountMismatch.into());
        let mut garbage = vec![0u8; 3_208];
        assert_eq!(err(read(key, &mut garbage, &m, 1_000)), HodlError::PriceAccountMismatch.into());
        assert_eq!(err(read(key, &mut pull_feed_data(VALUE, 0, 1_000, 5), &m, 1_151)), HodlError::StalePrice.into());
        assert_eq!(err(read(key, &mut pull_feed_data(VALUE, 0, 0, 5), &m, 10)), HodlError::StalePrice.into());
        assert_eq!(err(read(key, &mut pull_feed_data(VALUE, 0, 1_000, 2), &m, 1_000)), HodlError::StalePrice.into());
        assert_eq!(err(read(key, &mut pull_feed_data(0, 0, 1_000, 5), &m, 1_000)), HodlError::InvalidPrice.into());
        // 2% spread is the limit; just above fails.
        read(key, &mut pull_feed_data(VALUE, 12_500_000_000_000, 1_000, 5), &m, 1_000).unwrap();
        assert_eq!(err(read(key, &mut pull_feed_data(VALUE, 12_500_001_000_000, 1_000, 5), &m, 1_000)), HodlError::PriceConfidenceTooWide.into());
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p hodl_loans --lib`
Expected: `test result: ok. 20 passed`.

Run: `./scripts/test.sh`
Expected: `cargo build-sbf` finishes (the `getrandom` feature is what lets the Pyth SDK build for SBF) and every test binary reports `ok`.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: Pyth and Switchboard price reads scaled to 12-decimal USD"
```

---

### Task 3: Loan balance and health math

**Files:**
- Create: `programs/hodl_loans/src/math/loan.rs`, `programs/hodl_loans/src/math/health.rs`
- Modify: `programs/hodl_loans/src/math/mod.rs`

**Interfaces:**
- Consumes: `BPS`, `YEAR`, `math::checked`, `math::price::{UsdPrice, token_value, token_value_ceil}` (Task 2).
- Produces:
  - `math::loan::LoanTerms { principal: u64, originated_at: i64, interest_anchor: i64, tenure_seconds: i64, rate_bps: u16, penalty_rate_bps: u16 }`
  - `math::loan::LoanBalance { principal: u128, interest: u128, penalty: u128 }` with `due() -> Result<u128>` (interest + penalty) and `total() -> Result<u128>`
  - `math::loan::loan_balance(terms: &LoanTerms, now: i64) -> Result<LoanBalance>` (rounds up)
  - `math::loan::lp_contribution(principal: u64, rate_bps: u16, reserve_factor_bps: u16) -> Result<u128>`
  - `math::loan::accrued_lp_interest(principal: u64, rate_bps: u16, reserve_factor_bps: u16, interest_anchor: i64, now: i64) -> Result<u128>` (spec §9 `R`, rounds down)
  - `math::loan::reserve_share(interest_paid: u128, reserve_factor_bps: u16) -> Result<u128>` (rounds down)
  - `math::health::CollateralValue { amount: u64, decimals: u8, price: UsdPrice, ltv_bps: u16, liquidation_threshold_bps: u16 }`
  - `math::health::Health { own_value, borrow_limit, liquidation_line, debt: u128 }` with `is_healthy()` (`debt ≤ borrow_limit`) and `is_liquidatable()` (`debt > liquidation_line`)
  - `math::health::compute_health(collateral: &[CollateralValue], debt_cngn: u128, cngn_decimals: u8, ngn: UsdPrice) -> Result<Health>`

- [ ] **Step 1: Wire the modules and write the failing tests**

Replace `programs/hodl_loans/src/math/mod.rs`:

```rust
pub mod checked;
pub mod health;
pub mod interest;
pub mod loan;
pub mod price;
pub mod shares;
```

`programs/hodl_loans/src/math/loan.rs`, imports and tests only:

```rust
use anchor_lang::prelude::*;

use crate::constants::{BPS, YEAR};
use crate::errors::HodlError;
use crate::math::checked::{add, mul_div_ceil, mul_div_floor};

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    fn terms() -> LoanTerms {
        // 1,000,000 cNGN (6 decimals), 365-day tenure, 15% APR, 5% penalty.
        LoanTerms {
            principal: 1_000_000_000_000,
            originated_at: 1_000,
            interest_anchor: 1_000,
            tenure_seconds: 365 * DAY,
            rate_bps: 1_500,
            penalty_rate_bps: 500,
        }
    }

    #[test]
    fn before_maturity_charges_simple_interest_only() {
        let t = terms();
        let b = loan_balance(&t, t.originated_at + 73 * DAY).unwrap();
        // 1e12 × 1500 × 73 days / (1e4 × 365 days) = 3e10 exactly.
        assert_eq!(b.interest, 30_000_000_000);
        assert_eq!(b.penalty, 0);
        assert_eq!(b.total().unwrap(), 1_030_000_000_000);
    }

    #[test]
    fn interest_stops_at_maturity_and_penalty_starts() {
        let t = terms();
        let maturity = t.originated_at + t.tenure_seconds;
        let at_maturity = loan_balance(&t, maturity).unwrap();
        assert_eq!(at_maturity.interest, 150_000_000_000);
        assert_eq!(at_maturity.penalty, 0);

        let later = loan_balance(&t, maturity + 73 * DAY).unwrap();
        assert_eq!(later.interest, 150_000_000_000);
        // (1e12 + 1.5e11) × 2000 × 73 days / (1e4 × 365 days) = 4.6e10.
        assert_eq!(later.penalty, 46_000_000_000);
    }

    #[test]
    fn anchor_reset_after_repayment_moves_both_clocks() {
        let mut t = terms();
        let maturity = t.originated_at + t.tenure_seconds;
        // Repaid in full-interest terms 10 days after maturity.
        t.interest_anchor = maturity + 10 * DAY;
        let b = loan_balance(&t, maturity + 10 * DAY).unwrap();
        assert_eq!((b.interest, b.penalty), (0, 0));
        let b = loan_balance(&t, maturity + 83 * DAY).unwrap();
        assert_eq!(b.interest, 0);
        assert_eq!(b.penalty, 40_000_000_000);
    }

    #[test]
    fn balance_rounds_up() {
        let t = LoanTerms { principal: 1, ..terms() };
        let b = loan_balance(&t, t.originated_at + 1).unwrap();
        assert_eq!(b.interest, 1);
    }

    #[test]
    fn lender_interest_matches_contribution_and_rounds_down() {
        let t = terms();
        let r = accrued_lp_interest(t.principal, t.rate_bps, 1_000, t.interest_anchor, t.interest_anchor + 73 * DAY)
            .unwrap();
        // Borrower interest 3e10 × 90% to lenders.
        assert_eq!(r, 27_000_000_000);
        assert_eq!(lp_contribution(t.principal, t.rate_bps, 1_000).unwrap(), 1_000_000_000_000u128 * 1_500 * 9_000);
        assert_eq!(accrued_lp_interest(1, 1, 0, 0, 1).unwrap(), 0);
        assert_eq!(reserve_share(30_000_000_000, 1_000).unwrap(), 3_000_000_000);
        assert_eq!(reserve_share(9, 1_000).unwrap(), 0);
    }
}
```

`programs/hodl_loans/src/math/health.rs`, imports and tests only:

```rust
use anchor_lang::prelude::*;

use crate::constants::BPS;
use crate::math::checked::{add, mul_div_floor};
use crate::math::price::{token_value, token_value_ceil, UsdPrice};

#[cfg(test)]
mod tests {
    use super::*;

    const USD: u128 = 1_000_000_000_000;

    fn ngn() -> UsdPrice {
        // 1 NGN = $0.000625, no spread: 1,600 NGN per dollar.
        UsdPrice { price: 625_000_000, conf: 0 }
    }

    #[test]
    fn mixed_decimals_sum_into_limits() {
        let collateral = [
            // 10 SOL (9 decimals) at $150 with $1 confidence.
            CollateralValue {
                amount: 10_000_000_000,
                decimals: 9,
                price: UsdPrice { price: 150 * USD, conf: USD },
                ltv_bps: 7_000,
                liquidation_threshold_bps: 9_000,
            },
            // 500 USDC (6 decimals) at $1.
            CollateralValue {
                amount: 500_000_000,
                decimals: 6,
                price: UsdPrice { price: USD, conf: 0 },
                ltv_bps: 7_000,
                liquidation_threshold_bps: 9_000,
            },
        ];
        // Debt: 1,600,000 cNGN = $1,000.
        let h = compute_health(&collateral, 1_600_000_000_000, 6, ngn()).unwrap();
        assert_eq!(h.own_value, 1_990 * USD);
        assert_eq!(h.borrow_limit, 1_393 * USD);
        assert_eq!(h.liquidation_line, 1_791 * USD);
        assert_eq!(h.debt, 1_000 * USD);
        assert!(h.is_healthy());
        assert!(!h.is_liquidatable());
    }

    #[test]
    fn spread_raises_debt_and_boundary_is_inclusive() {
        let collateral = [CollateralValue {
            amount: 1_000_000_000,
            decimals: 6,
            price: UsdPrice { price: USD, conf: 0 },
            ltv_bps: 7_000,
            liquidation_threshold_bps: 9_000,
        }];
        // $700 of debt exactly at the 70% limit is healthy.
        let at_limit = compute_health(&collateral, 1_120_000_000_000, 6, ngn()).unwrap();
        assert_eq!(at_limit.debt, 700 * USD);
        assert!(at_limit.is_healthy());
        // A 1% spread pushes the same debt over the limit.
        let wide = UsdPrice { price: 625_000_000, conf: 6_250_000 };
        let over = compute_health(&collateral, 1_120_000_000_000, 6, wide).unwrap();
        assert_eq!(over.debt, 707 * USD);
        assert!(!over.is_healthy());
    }

    #[test]
    fn no_collateral_means_any_debt_is_unhealthy() {
        let h = compute_health(&[], 1, 6, ngn()).unwrap();
        assert_eq!(h.borrow_limit, 0);
        assert!(!h.is_healthy());
        assert!(h.is_liquidatable());
        assert!(compute_health(&[], 0, 6, ngn()).unwrap().is_healthy());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p hodl_loans --lib`
Expected: compile errors such as `cannot find type LoanTerms in this scope`, `cannot find function loan_balance in this scope` and `cannot find function compute_health in this scope`.

- [ ] **Step 3: Implement**

`programs/hodl_loans/src/math/loan.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{BPS, YEAR};
use crate::errors::HodlError;
use crate::math::checked::{add, mul_div_ceil, mul_div_floor};

/// The terms of one fixed-term loan, as stored in its slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoanTerms {
    pub principal: u64,
    pub originated_at: i64,
    pub interest_anchor: i64,
    pub tenure_seconds: i64,
    pub rate_bps: u16,
    pub penalty_rate_bps: u16,
}

/// What a loan owes at a moment in time. All amounts round up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoanBalance {
    pub principal: u128,
    pub interest: u128,
    pub penalty: u128,
}

impl LoanBalance {
    pub fn due(&self) -> Result<u128> {
        add(self.interest, self.penalty)
    }

    pub fn total(&self) -> Result<u128> {
        add(self.principal, self.due()?)
    }
}

fn seconds_between(start: i64, end: i64) -> u128 {
    if end > start {
        (end - start) as u128
    } else {
        0
    }
}

fn mul3(a: u128, b: u128, c: u128) -> Result<u128> {
    Ok(a.checked_mul(b).and_then(|v| v.checked_mul(c)).ok_or(HodlError::MathOverflow)?)
}

/// Spec §10:
/// interest = principal × rate × (min(now, maturity) − anchor)⁺ / (BPS × YEAR)
/// penalty  = (principal + interest) × (rate + penalty_rate) × (now − max(maturity, anchor))⁺ / (BPS × YEAR)
pub fn loan_balance(terms: &LoanTerms, now: i64) -> Result<LoanBalance> {
    let principal = terms.principal as u128;
    let maturity = terms
        .originated_at
        .checked_add(terms.tenure_seconds)
        .ok_or(HodlError::MathOverflow)?;
    let interest_seconds = seconds_between(terms.interest_anchor, now.min(maturity));
    let interest = mul_div_ceil(
        mul3(principal, terms.rate_bps as u128, interest_seconds)?,
        1,
        BPS * YEAR,
    )?;
    let penalty_seconds = seconds_between(maturity.max(terms.interest_anchor), now);
    let penalty_rate = terms.rate_bps as u128 + terms.penalty_rate_bps as u128;
    let penalty = mul_div_ceil(
        mul3(add(principal, interest)?, penalty_rate, penalty_seconds)?,
        1,
        BPS * YEAR,
    )?;
    Ok(LoanBalance { principal, interest, penalty })
}

/// A loan's contribution to `Market::lp_rate_product`.
pub fn lp_contribution(principal: u64, rate_bps: u16, reserve_factor_bps: u16) -> Result<u128> {
    mul3(principal as u128, rate_bps as u128, BPS - reserve_factor_bps as u128)
}

/// Lender interest the market has accrued for `principal` of a loan since `interest_anchor`
/// (spec §9 `R`). Rounds down.
pub fn accrued_lp_interest(
    principal: u64,
    rate_bps: u16,
    reserve_factor_bps: u16,
    interest_anchor: i64,
    now: i64,
) -> Result<u128> {
    mul_div_floor(
        lp_contribution(principal, rate_bps, reserve_factor_bps)?,
        seconds_between(interest_anchor, now),
        BPS * BPS * YEAR,
    )
}

/// The protocol's cut of `interest_paid`. Rounds down.
pub fn reserve_share(interest_paid: u128, reserve_factor_bps: u16) -> Result<u128> {
    mul_div_floor(interest_paid, reserve_factor_bps as u128, BPS)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    fn terms() -> LoanTerms {
        // 1,000,000 cNGN (6 decimals), 365-day tenure, 15% APR, 5% penalty.
        LoanTerms {
            principal: 1_000_000_000_000,
            originated_at: 1_000,
            interest_anchor: 1_000,
            tenure_seconds: 365 * DAY,
            rate_bps: 1_500,
            penalty_rate_bps: 500,
        }
    }

    #[test]
    fn before_maturity_charges_simple_interest_only() {
        let t = terms();
        let b = loan_balance(&t, t.originated_at + 73 * DAY).unwrap();
        // 1e12 × 1500 × 73 days / (1e4 × 365 days) = 3e10 exactly.
        assert_eq!(b.interest, 30_000_000_000);
        assert_eq!(b.penalty, 0);
        assert_eq!(b.total().unwrap(), 1_030_000_000_000);
    }

    #[test]
    fn interest_stops_at_maturity_and_penalty_starts() {
        let t = terms();
        let maturity = t.originated_at + t.tenure_seconds;
        let at_maturity = loan_balance(&t, maturity).unwrap();
        assert_eq!(at_maturity.interest, 150_000_000_000);
        assert_eq!(at_maturity.penalty, 0);

        let later = loan_balance(&t, maturity + 73 * DAY).unwrap();
        assert_eq!(later.interest, 150_000_000_000);
        // (1e12 + 1.5e11) × 2000 × 73 days / (1e4 × 365 days) = 4.6e10.
        assert_eq!(later.penalty, 46_000_000_000);
    }

    #[test]
    fn anchor_reset_after_repayment_moves_both_clocks() {
        let mut t = terms();
        let maturity = t.originated_at + t.tenure_seconds;
        // Repaid in full-interest terms 10 days after maturity.
        t.interest_anchor = maturity + 10 * DAY;
        let b = loan_balance(&t, maturity + 10 * DAY).unwrap();
        assert_eq!((b.interest, b.penalty), (0, 0));
        let b = loan_balance(&t, maturity + 83 * DAY).unwrap();
        assert_eq!(b.interest, 0);
        assert_eq!(b.penalty, 40_000_000_000);
    }

    #[test]
    fn balance_rounds_up() {
        let t = LoanTerms { principal: 1, ..terms() };
        let b = loan_balance(&t, t.originated_at + 1).unwrap();
        assert_eq!(b.interest, 1);
    }

    #[test]
    fn lender_interest_matches_contribution_and_rounds_down() {
        let t = terms();
        let r = accrued_lp_interest(t.principal, t.rate_bps, 1_000, t.interest_anchor, t.interest_anchor + 73 * DAY)
            .unwrap();
        // Borrower interest 3e10 × 90% to lenders.
        assert_eq!(r, 27_000_000_000);
        assert_eq!(lp_contribution(t.principal, t.rate_bps, 1_000).unwrap(), 1_000_000_000_000u128 * 1_500 * 9_000);
        assert_eq!(accrued_lp_interest(1, 1, 0, 0, 1).unwrap(), 0);
        assert_eq!(reserve_share(30_000_000_000, 1_000).unwrap(), 3_000_000_000);
        assert_eq!(reserve_share(9, 1_000).unwrap(), 0);
    }
}
```

`programs/hodl_loans/src/math/health.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::BPS;
use crate::math::checked::{add, mul_div_floor};
use crate::math::price::{token_value, token_value_ceil, UsdPrice};

/// One collateral holding with its price and risk settings.
#[derive(Clone, Copy, Debug)]
pub struct CollateralValue {
    pub amount: u64,
    pub decimals: u8,
    pub price: UsdPrice,
    pub ltv_bps: u16,
    pub liquidation_threshold_bps: u16,
}

/// Spec §8 health values, all at `USD_SCALE`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Health {
    pub own_value: u128,
    pub borrow_limit: u128,
    pub liquidation_line: u128,
    pub debt: u128,
}

impl Health {
    pub fn is_healthy(&self) -> bool {
        self.debt <= self.borrow_limit
    }

    pub fn is_liquidatable(&self) -> bool {
        self.debt > self.liquidation_line
    }
}

/// Collateral is valued at `price − conf`; debt (cNGN base units) at `ngn price + spread`, rounded up.
pub fn compute_health(
    collateral: &[CollateralValue],
    debt_cngn: u128,
    cngn_decimals: u8,
    ngn: UsdPrice,
) -> Result<Health> {
    let mut health = Health::default();
    for c in collateral {
        let value = token_value(c.amount as u128, c.decimals, c.price.lower())?;
        health.own_value = add(health.own_value, value)?;
        health.borrow_limit = add(health.borrow_limit, mul_div_floor(value, c.ltv_bps as u128, BPS)?)?;
        health.liquidation_line = add(
            health.liquidation_line,
            mul_div_floor(value, c.liquidation_threshold_bps as u128, BPS)?,
        )?;
    }
    health.debt = token_value_ceil(debt_cngn, cngn_decimals, ngn.upper()?)?;
    Ok(health)
}

#[cfg(test)]
mod tests {
    use super::*;

    const USD: u128 = 1_000_000_000_000;

    fn ngn() -> UsdPrice {
        // 1 NGN = $0.000625, no spread: 1,600 NGN per dollar.
        UsdPrice { price: 625_000_000, conf: 0 }
    }

    #[test]
    fn mixed_decimals_sum_into_limits() {
        let collateral = [
            // 10 SOL (9 decimals) at $150 with $1 confidence.
            CollateralValue {
                amount: 10_000_000_000,
                decimals: 9,
                price: UsdPrice { price: 150 * USD, conf: USD },
                ltv_bps: 7_000,
                liquidation_threshold_bps: 9_000,
            },
            // 500 USDC (6 decimals) at $1.
            CollateralValue {
                amount: 500_000_000,
                decimals: 6,
                price: UsdPrice { price: USD, conf: 0 },
                ltv_bps: 7_000,
                liquidation_threshold_bps: 9_000,
            },
        ];
        // Debt: 1,600,000 cNGN = $1,000.
        let h = compute_health(&collateral, 1_600_000_000_000, 6, ngn()).unwrap();
        assert_eq!(h.own_value, 1_990 * USD);
        assert_eq!(h.borrow_limit, 1_393 * USD);
        assert_eq!(h.liquidation_line, 1_791 * USD);
        assert_eq!(h.debt, 1_000 * USD);
        assert!(h.is_healthy());
        assert!(!h.is_liquidatable());
    }

    #[test]
    fn spread_raises_debt_and_boundary_is_inclusive() {
        let collateral = [CollateralValue {
            amount: 1_000_000_000,
            decimals: 6,
            price: UsdPrice { price: USD, conf: 0 },
            ltv_bps: 7_000,
            liquidation_threshold_bps: 9_000,
        }];
        // $700 of debt exactly at the 70% limit is healthy.
        let at_limit = compute_health(&collateral, 1_120_000_000_000, 6, ngn()).unwrap();
        assert_eq!(at_limit.debt, 700 * USD);
        assert!(at_limit.is_healthy());
        // A 1% spread pushes the same debt over the limit.
        let wide = UsdPrice { price: 625_000_000, conf: 6_250_000 };
        let over = compute_health(&collateral, 1_120_000_000_000, 6, wide).unwrap();
        assert_eq!(over.debt, 707 * USD);
        assert!(!over.is_healthy());
    }

    #[test]
    fn no_collateral_means_any_debt_is_unhealthy() {
        let h = compute_health(&[], 1, 6, ngn()).unwrap();
        assert_eq!(h.borrow_limit, 0);
        assert!(!h.is_healthy());
        assert!(h.is_liquidatable());
        assert!(compute_health(&[], 0, 6, ngn()).unwrap().is_healthy());
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p hodl_loans --lib`
Expected: `test result: ok. 28 passed`.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: loan balance, lender interest release and health math"
```

---

### Task 4: Collateral listing, parameters, pause, delist and sweep

**Files:**
- Create: `programs/hodl_loans/src/state/collateral.rs`, `src/instructions/admin/collateral_admin.rs` (under `programs/hodl_loans/`)
- Modify: `src/state/mod.rs`, `src/token/extensions.rs`, `src/events.rs`, `src/instructions/admin/sweep.rs`, `src/instructions/admin/mod.rs`, `src/lib.rs`, `tests/common/mod.rs`
- Test: `programs/hodl_loans/tests/collateral.rs`

**Interfaces:**
- Consumes: `COLLATERAL_SEED`, `COLLATERAL_VAULT_SEED` (Task 1), `Config.{admin, guardian, treasury, promo_cap_bps, collateral_count}`, `token::extensions::require_allowed_extensions`, `transfer_from_vault` (Plan 1).
- Produces:
  - `state::CollateralKind { Standard, XStock }`
  - `state::CollateralAsset { version, bump, vault_bump: u8, mint, token_program, vault: Pubkey, decimals: u8, kind: CollateralKind, pyth_feed_id: [u8; 32], max_price_age_seconds: u64, max_conf_bps, ltv_bps, liquidation_threshold_bps, liquidation_bonus_bps: u16, deposit_cap, total_deposited: u64, paused: bool, reserved: [u8; 128] }` with `params() -> CollateralParams` and `apply_params(&CollateralParams)`
  - `state::CollateralParams { pyth_feed_id, max_price_age_seconds, max_conf_bps, ltv_bps, liquidation_threshold_bps, liquidation_bonus_bps, deposit_cap }` with `validate(promo_cap_bps: u16) -> Result<()>`
  - `token::extensions::STANDARD_COLLATERAL_EXTENSIONS`
  - Events `CollateralListed`, `CollateralParamsUpdated`, `CollateralPauseSet`, `CollateralDelisted`
  - Instructions (all accounts in order):
    - `list_collateral(params)`: `admin, config, mint, collateral, vault, token_program, system_program`
    - `update_collateral_params(params)`: `admin, config, collateral`
    - `set_collateral_paused(paused)`: `signer, config, collateral`
    - `delist_collateral()`: `admin, config, collateral, mint, vault, token_program`
    - `sweep_collateral_excess()`: `admin, config, collateral, mint, vault, destination, token_program`
  - Harness: `ONE_USDC`, `collateral_pda`, `collateral_vault_pda`, `feed_id(&mint)`, `default_collateral_params(&mint)`, the five `*_ix` builders, `Env::list_spl_collateral(decimals) -> Pubkey`, `Env::collateral(&mint) -> CollateralAsset`

- [ ] **Step 1: Write the failing tests**

Append to `programs/hodl_loans/tests/common/mod.rs`:

```rust
// ---- Collateral listing (Task 4) ----

pub const ONE_USDC: u64 = 1_000_000;

pub fn collateral_pda(mint: &Pubkey) -> Pubkey {
    pda(&[hodl_loans::constants::COLLATERAL_SEED, mint.as_ref()])
}
pub fn collateral_vault_pda(mint: &Pubkey) -> Pubkey {
    pda(&[hodl_loans::constants::COLLATERAL_VAULT_SEED, mint.as_ref()])
}

/// Tests use a mint's own bytes as its Pyth feed ID.
pub fn feed_id(mint: &Pubkey) -> [u8; 32] {
    mint.to_bytes()
}

/// Spec §8 launch values for a stablecoin: LTV 70%, threshold 90%, bonus 5%.
pub fn default_collateral_params(mint: &Pubkey) -> hodl_loans::CollateralParams {
    hodl_loans::CollateralParams {
        pyth_feed_id: feed_id(mint),
        max_price_age_seconds: 60,
        max_conf_bps: 200,
        ltv_bps: 7_000,
        liquidation_threshold_bps: 9_000,
        liquidation_bonus_bps: 500,
        deposit_cap: u64::MAX,
    }
}

pub fn list_collateral_ix(admin: &Pubkey, mint: &Pubkey, token_program: &Pubkey, params: hodl_loans::CollateralParams) -> Instruction {
    ix(
        hodl_loans::instruction::ListCollateral { params },
        hodl_loans::accounts::ListCollateral {
            admin: *admin,
            config: config_pda(),
            mint: *mint,
            collateral: collateral_pda(mint),
            vault: collateral_vault_pda(mint),
            token_program: *token_program,
            system_program: system_program::ID,
        },
    )
}

pub fn update_collateral_params_ix(admin: &Pubkey, mint: &Pubkey, params: hodl_loans::CollateralParams) -> Instruction {
    ix(
        hodl_loans::instruction::UpdateCollateralParams { params },
        hodl_loans::accounts::UpdateCollateralParams { admin: *admin, config: config_pda(), collateral: collateral_pda(mint) },
    )
}

pub fn set_collateral_paused_ix(signer: &Pubkey, mint: &Pubkey, paused: bool) -> Instruction {
    ix(
        hodl_loans::instruction::SetCollateralPaused { paused },
        hodl_loans::accounts::SetCollateralPaused { signer: *signer, config: config_pda(), collateral: collateral_pda(mint) },
    )
}

pub fn delist_collateral_ix(admin: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::DelistCollateral {},
        hodl_loans::accounts::DelistCollateral {
            admin: *admin,
            config: config_pda(),
            collateral: collateral_pda(mint),
            mint: *mint,
            vault: collateral_vault_pda(mint),
            token_program: *token_program,
        },
    )
}

pub fn sweep_collateral_excess_ix(admin: &Pubkey, mint: &Pubkey, token_program: &Pubkey, destination: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::SweepCollateralExcess {},
        hodl_loans::accounts::SweepCollateralExcess {
            admin: *admin,
            config: config_pda(),
            collateral: collateral_pda(mint),
            mint: *mint,
            vault: collateral_vault_pda(mint),
            destination: *destination,
            token_program: *token_program,
        },
    )
}

impl Env {
    /// Creates a classic SPL Token mint and lists it with default parameters. Returns the mint.
    pub fn list_spl_collateral(&mut self, decimals: u8) -> Pubkey {
        let mint = self.create_mint(MintKind::SplToken, decimals);
        let instruction = list_collateral_ix(&self.admin.pubkey(), &mint, &SPL_TOKEN, default_collateral_params(&mint));
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("list collateral");
        mint
    }

    pub fn collateral(&self, mint: &Pubkey) -> hodl_loans::CollateralAsset {
        self.fetch(&collateral_pda(mint))
    }
}
```

`programs/hodl_loans/tests/collateral.rs`:

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
        CollateralParams { max_conf_bps: 10_000, max_price_age_seconds: 1, ..base },
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

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test collateral`
Expected: compile errors, including `unresolved imports hodl_loans::CollateralKind, hodl_loans::CollateralParams` and `cannot find struct, variant or union type ListCollateral in module hodl_loans::instruction`.

- [ ] **Step 3: Implement**

`programs/hodl_loans/src/state/collateral.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{BPS, MAX_BPS};
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
    pub reserved: [u8; 128],
}

/// Admin-settable collateral parameters, used by `list_collateral` and `update_collateral_params`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct CollateralParams {
    pub pyth_feed_id: [u8; 32],
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
        require!(self.max_price_age_seconds > 0, HodlError::InvalidParameters);
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

Replace `programs/hodl_loans/src/state/mod.rs`:

```rust
pub mod access;
pub mod collateral;
pub mod config;
pub mod lender;
pub mod market;

pub use access::*;
pub use collateral::*;
pub use config::*;
pub use lender::*;
pub use market::*;
```

Replace `programs/hodl_loans/src/token/extensions.rs`:

```rust
use anchor_lang::prelude::*;
use anchor_spl::token_2022::spl_token_2022::{
    extension::{BaseStateWithExtensions, ExtensionType, StateWithExtensions},
    state::Mint as MintState,
};

use crate::errors::HodlError;

/// Extensions a borrowable (market) mint may carry. The Solana cNGN mint uses exactly
/// these (checked 2026-09-17): its issuer can move tokens through the permanent delegate,
/// which is an accepted issuer risk.
pub const MARKET_MINT_EXTENSIONS: &[ExtensionType] = &[
    ExtensionType::MetadataPointer,
    ExtensionType::TokenMetadata,
    ExtensionType::PermanentDelegate,
];

/// Extensions a `Standard` collateral mint may carry (spec §14): metadata only.
pub const STANDARD_COLLATERAL_EXTENSIONS: &[ExtensionType] =
    &[ExtensionType::MetadataPointer, ExtensionType::TokenMetadata];

/// Extension types on a mint. Classic SPL Token mints have none.
pub fn mint_extension_types(mint: &AccountInfo) -> Result<Vec<ExtensionType>> {
    if *mint.owner != anchor_spl::token_2022::ID {
        return Ok(Vec::new());
    }
    let data = mint.try_borrow_data()?;
    let state = StateWithExtensions::<MintState>::unpack(&data)?;
    Ok(state.get_extension_types()?)
}

pub fn require_allowed_extensions(mint: &AccountInfo, allowed: &[ExtensionType]) -> Result<()> {
    for extension in mint_extension_types(mint)? {
        require!(allowed.contains(&extension), HodlError::UnsupportedMintExtension);
    }
    Ok(())
}
```

In `programs/hodl_loans/src/events.rs`, change `use crate::state::MarketParams;` to `use crate::state::{CollateralParams, MarketParams};`, then append:

```rust
#[event]
pub struct CollateralListed {
    pub collateral: Pubkey,
    pub mint: Pubkey,
    pub vault: Pubkey,
    pub params: CollateralParams,
}

#[event]
pub struct CollateralParamsUpdated {
    pub collateral: Pubkey,
    pub old: CollateralParams,
    pub new: CollateralParams,
}

#[event]
pub struct CollateralPauseSet {
    pub collateral: Pubkey,
    pub old_paused: bool,
    pub paused: bool,
    pub by: Pubkey,
}

#[event]
pub struct CollateralDelisted {
    pub collateral: Pubkey,
    pub mint: Pubkey,
}
```

`programs/hodl_loans/src/instructions/admin/collateral_admin.rs`:

```rust
use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, CloseAccount, Mint, TokenAccount, TokenInterface};

use crate::constants::{ACCOUNT_VERSION, COLLATERAL_SEED, COLLATERAL_VAULT_SEED, CONFIG_SEED};
use crate::errors::HodlError;
use crate::events::{CollateralDelisted, CollateralListed, CollateralParamsUpdated, CollateralPauseSet};
use crate::state::{CollateralAsset, CollateralKind, CollateralParams, Config};
use crate::token::extensions::{require_allowed_extensions, STANDARD_COLLATERAL_EXTENSIONS};

#[derive(Accounts)]
pub struct ListCollateral<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        init,
        payer = admin,
        space = 8 + CollateralAsset::INIT_SPACE,
        seeds = [COLLATERAL_SEED, mint.key().as_ref()],
        bump
    )]
    pub collateral: Box<Account<'info, CollateralAsset>>,
    #[account(
        init,
        payer = admin,
        token::mint = mint,
        token::authority = collateral,
        token::token_program = token_program,
        seeds = [COLLATERAL_VAULT_SEED, mint.key().as_ref()],
        bump
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateCollateralParams<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(mut, seeds = [COLLATERAL_SEED, collateral.mint.as_ref()], bump = collateral.bump)]
    pub collateral: Box<Account<'info, CollateralAsset>>,
}

#[derive(Accounts)]
pub struct SetCollateralPaused<'info> {
    pub signer: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(mut, seeds = [COLLATERAL_SEED, collateral.mint.as_ref()], bump = collateral.bump)]
    pub collateral: Box<Account<'info, CollateralAsset>>,
}

#[derive(Accounts)]
pub struct DelistCollateral<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(
        mut,
        seeds = [COLLATERAL_SEED, mint.key().as_ref()],
        bump = collateral.bump,
        has_one = mint,
        has_one = vault,
        close = admin
    )]
    pub collateral: Box<Account<'info, CollateralAsset>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
}

pub fn handle_list_collateral(ctx: Context<ListCollateral>, params: CollateralParams) -> Result<()> {
    params.validate(ctx.accounts.config.promo_cap_bps)?;
    require_allowed_extensions(&ctx.accounts.mint.to_account_info(), STANDARD_COLLATERAL_EXTENSIONS)?;

    let mut asset = CollateralAsset {
        version: ACCOUNT_VERSION,
        bump: ctx.bumps.collateral,
        vault_bump: ctx.bumps.vault,
        mint: ctx.accounts.mint.key(),
        token_program: ctx.accounts.token_program.key(),
        vault: ctx.accounts.vault.key(),
        decimals: ctx.accounts.mint.decimals,
        kind: CollateralKind::Standard,
        pyth_feed_id: [0; 32],
        max_price_age_seconds: 0,
        max_conf_bps: 0,
        ltv_bps: 0,
        liquidation_threshold_bps: 0,
        liquidation_bonus_bps: 0,
        deposit_cap: 0,
        total_deposited: 0,
        paused: false,
        reserved: [0; 128],
    };
    asset.apply_params(&params);
    ctx.accounts.collateral.set_inner(asset);

    let config = &mut ctx.accounts.config;
    config.collateral_count = config.collateral_count.checked_add(1).ok_or(HodlError::MathOverflow)?;

    emit!(CollateralListed {
        collateral: ctx.accounts.collateral.key(),
        mint: ctx.accounts.mint.key(),
        vault: ctx.accounts.vault.key(),
        params,
    });
    Ok(())
}

/// Lowering an LTV or threshold affects existing positions immediately.
pub fn handle_update_collateral_params(ctx: Context<UpdateCollateralParams>, params: CollateralParams) -> Result<()> {
    params.validate(ctx.accounts.config.promo_cap_bps)?;
    let collateral_key = ctx.accounts.collateral.key();
    let collateral = &mut ctx.accounts.collateral;
    let old = collateral.params();
    collateral.apply_params(&params);
    emit!(CollateralParamsUpdated { collateral: collateral_key, old, new: params });
    Ok(())
}

/// Guardian or admin may pause; only admin may unpause.
pub fn handle_set_collateral_paused(ctx: Context<SetCollateralPaused>, paused: bool) -> Result<()> {
    let signer = ctx.accounts.signer.key();
    let config = &ctx.accounts.config;
    if paused {
        require!(signer == config.admin || signer == config.guardian, HodlError::Unauthorized);
    } else {
        require!(signer == config.admin, HodlError::Unauthorized);
    }
    let collateral_key = ctx.accounts.collateral.key();
    let old_paused = ctx.accounts.collateral.paused;
    ctx.accounts.collateral.paused = paused;
    emit!(CollateralPauseSet { collateral: collateral_key, old_paused, paused, by: signer });
    Ok(())
}

/// Requires no recorded deposits and an empty vault (sweep donations first). Closes the vault
/// and the `CollateralAsset`, refunding rent to the admin.
pub fn handle_delist_collateral(ctx: Context<DelistCollateral>) -> Result<()> {
    require!(ctx.accounts.collateral.total_deposited == 0, HodlError::CollateralStillInUse);
    require!(ctx.accounts.vault.amount == 0, HodlError::CollateralStillInUse);

    let mint_key = ctx.accounts.mint.key();
    let seeds: &[&[u8]] = &[COLLATERAL_SEED, mint_key.as_ref(), &[ctx.accounts.collateral.bump]];
    token_interface::close_account(CpiContext::new_with_signer(
        ctx.accounts.token_program.key(),
        CloseAccount {
            account: ctx.accounts.vault.to_account_info(),
            destination: ctx.accounts.admin.to_account_info(),
            authority: ctx.accounts.collateral.to_account_info(),
        },
        &[seeds],
    ))?;

    let config = &mut ctx.accounts.config;
    config.collateral_count = config.collateral_count.checked_sub(1).ok_or(HodlError::MathOverflow)?;
    emit!(CollateralDelisted { collateral: ctx.accounts.collateral.key(), mint: mint_key });
    Ok(())
}
```

Replace `programs/hodl_loans/src/instructions/admin/sweep.rs` (the market sweep is unchanged; the collateral sweep is added):

```rust
use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{COLLATERAL_SEED, CONFIG_SEED, MARKET_SEED};
use crate::errors::HodlError;
use crate::events::ExcessSwept;
use crate::state::{CollateralAsset, Config, Market};
use crate::token::transfer::transfer_from_vault;

#[derive(Accounts)]
pub struct SweepMarketExcess<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(seeds = [MARKET_SEED, mint.key().as_ref()], bump = market.bump, has_one = mint, has_one = vault)]
    pub market: Box<Account<'info, Market>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = mint,
        token::authority = config.treasury,
        token::token_program = token_program
    )]
    pub destination: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
}

/// Sends vault tokens above `market.cash` (direct donations) to the treasury.
pub fn handle_sweep_market_excess(ctx: Context<SweepMarketExcess>) -> Result<()> {
    let excess = ctx.accounts.vault.amount.saturating_sub(ctx.accounts.market.cash);
    require!(excess > 0, HodlError::AmountTooSmall);

    let mint_key = ctx.accounts.mint.key();
    let seeds: &[&[u8]] = &[MARKET_SEED, mint_key.as_ref(), &[ctx.accounts.market.bump]];
    transfer_from_vault(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.destination.to_account_info(),
        ctx.accounts.market.to_account_info(),
        excess,
        &[seeds],
    )?;
    emit!(ExcessSwept {
        vault: ctx.accounts.vault.key(),
        destination: ctx.accounts.destination.key(),
        amount: excess,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct SweepCollateralExcess<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(
        seeds = [COLLATERAL_SEED, mint.key().as_ref()],
        bump = collateral.bump,
        has_one = mint,
        has_one = vault
    )]
    pub collateral: Box<Account<'info, CollateralAsset>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = mint,
        token::authority = config.treasury,
        token::token_program = token_program
    )]
    pub destination: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
}

/// Sends collateral vault tokens above `total_deposited` (direct donations) to the treasury.
pub fn handle_sweep_collateral_excess(ctx: Context<SweepCollateralExcess>) -> Result<()> {
    let excess = ctx.accounts.vault.amount.saturating_sub(ctx.accounts.collateral.total_deposited);
    require!(excess > 0, HodlError::AmountTooSmall);

    let mint_key = ctx.accounts.mint.key();
    let seeds: &[&[u8]] = &[COLLATERAL_SEED, mint_key.as_ref(), &[ctx.accounts.collateral.bump]];
    transfer_from_vault(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.destination.to_account_info(),
        ctx.accounts.collateral.to_account_info(),
        excess,
        &[seeds],
    )?;
    emit!(ExcessSwept {
        vault: ctx.accounts.vault.key(),
        destination: ctx.accounts.destination.key(),
        amount: excess,
    });
    Ok(())
}
```

Replace `programs/hodl_loans/src/instructions/admin/mod.rs`:

```rust
pub mod access_control;
pub mod admin_transfer;
pub mod collateral_admin;
pub mod initialize;
pub mod market_admin;
pub mod roles;
pub mod sweep;

pub use access_control::*;
pub use admin_transfer::*;
pub use collateral_admin::*;
pub use initialize::*;
pub use market_admin::*;
pub use roles::*;
pub use sweep::*;
```

In `lib.rs`, add inside `pub mod hodl_loans { … }` after `sweep_market_excess`:

```rust
    pub fn list_collateral(ctx: Context<ListCollateral>, params: CollateralParams) -> Result<()> {
        instructions::handle_list_collateral(ctx, params)
    }

    pub fn update_collateral_params(ctx: Context<UpdateCollateralParams>, params: CollateralParams) -> Result<()> {
        instructions::handle_update_collateral_params(ctx, params)
    }

    pub fn set_collateral_paused(ctx: Context<SetCollateralPaused>, paused: bool) -> Result<()> {
        instructions::handle_set_collateral_paused(ctx, paused)
    }

    pub fn delist_collateral(ctx: Context<DelistCollateral>) -> Result<()> {
        instructions::handle_delist_collateral(ctx)
    }

    pub fn sweep_collateral_excess(ctx: Context<SweepCollateralExcess>) -> Result<()> {
        instructions::handle_sweep_collateral_excess(ctx)
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh --test collateral`
Expected: `test result: ok. 9 passed`.

Run: `./scripts/test.sh`
Expected: every binary reports `ok`; the unit binary shows `30 passed`.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: list, tune, pause, delist and sweep standard collateral assets"
```

---

### Task 5: Positions and collateral deposits

**Files:**
- Create: `programs/hodl_loans/src/state/position.rs`, `src/instructions/positions/mod.rs`, `src/instructions/positions/open_position.rs`, `src/instructions/positions/close_position.rs`, `src/instructions/positions/deposit_collateral.rs` (under `programs/hodl_loans/`)
- Modify: `src/state/mod.rs`, `src/events.rs`, `src/instructions/mod.rs`, `src/lib.rs`, `tests/common/mod.rs`
- Test: `programs/hodl_loans/tests/position.rs`

**Interfaces:**
- Consumes: `POSITION_SEED`, `MAX_COLLATERAL_SLOTS`, `MAX_LOAN_SLOTS`, `ACCOUNT_VERSION`, `Access::require_active`, `math::loan::LoanTerms` (Task 3), `CollateralAsset` (Task 4), `transfer_from_user`.
- Produces:
  - `state::CollateralSlot { mint: Pubkey, amount: u64 }` (zero-copy; `amount == 0` is free)
  - `state::LoanSlot { id, principal, original_principal, repaid: u64, originated_at, interest_anchor, tenure_seconds: i64, rate_bps, penalty_rate_bps, reserve_factor_bps: u16, active: u8, _padding: u8 }` with `is_active()` and `terms() -> LoanTerms`
  - `state::Position { version, bump, _padding: [u8; 6], owner, rent_payer, market: Pubkey, next_loan_id, promo_balance: u64, promo_last_activity_at: i64, collateral: [CollateralSlot; 8], loans: [LoanSlot; 10], reserved: [u8; 64] }` (`#[account(zero_copy)]`)
  - `Position` helpers: `collateral_index(&Pubkey) -> Option<usize>`, `free_collateral_index()`, `has_collateral()`, `loan_index(id: u64) -> Option<usize>`, `free_loan_index()`, `has_active_loans()`
  - Events `PositionOpened`, `PositionClosed`, `CollateralDeposited`
  - Instructions:
    - `open_position()`: `payer, owner, access, position, system_program`
    - `close_position()`: `owner, access, position, rent_payer`
    - `deposit_collateral(amount)`: `owner, access, position, collateral, mint, vault, owner_token, token_program`
  - Harness: `position_pda`, `open_position_ix`, `close_position_ix`, `deposit_collateral_ix`, `Borrower { key }` with `pubkey()`, `Env::sponsored(instruction, &Keypair) -> TxResult`, `Env::new_borrower()`, `Env::position(&owner) -> Position`, `Env::deposit_collateral(&borrower, &mint, amount) -> Pubkey` (returns the funded token account)

- [ ] **Step 1: Write the failing tests**

Append to `programs/hodl_loans/tests/common/mod.rs`:

```rust
// ---- Positions and collateral deposits (Task 5) ----

pub fn position_pda(owner: &Pubkey) -> Pubkey {
    pda(&[hodl_loans::constants::POSITION_SEED, owner.as_ref()])
}

pub fn open_position_ix(payer: &Pubkey, owner: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::OpenPosition {},
        hodl_loans::accounts::OpenPosition {
            payer: *payer,
            owner: *owner,
            access: access_pda(owner),
            position: position_pda(owner),
            system_program: system_program::ID,
        },
    )
}

pub fn close_position_ix(owner: &Pubkey, rent_payer: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::ClosePosition {},
        hodl_loans::accounts::ClosePosition {
            owner: *owner,
            access: access_pda(owner),
            position: position_pda(owner),
            rent_payer: *rent_payer,
        },
    )
}

pub fn deposit_collateral_ix(owner: &Pubkey, mint: &Pubkey, token_program: &Pubkey, owner_token: &Pubkey, amount: u64) -> Instruction {
    ix(
        hodl_loans::instruction::DepositCollateral { amount },
        hodl_loans::accounts::DepositCollateral {
            owner: *owner,
            access: access_pda(owner),
            position: position_pda(owner),
            collateral: collateral_pda(mint),
            mint: *mint,
            vault: collateral_vault_pda(mint),
            owner_token: *owner_token,
            token_program: *token_program,
        },
    )
}

/// A whitelisted wallet with no SOL; the admin pays its fees and rent.
pub struct Borrower {
    pub key: Keypair,
}

impl Borrower {
    pub fn pubkey(&self) -> Pubkey {
        self.key.pubkey()
    }
}

impl Env {
    /// Sends one instruction with the admin paying fees and `signer` co-signing.
    pub fn sponsored(&mut self, instruction: Instruction, signer: &Keypair) -> TxResult {
        send(&mut self.svm, &[instruction], &[&self.admin, signer])
    }

    /// A whitelisted wallet with an open position.
    pub fn new_borrower(&mut self) -> Borrower {
        let key = Keypair::new();
        self.whitelist(&key.pubkey());
        let instruction = open_position_ix(&self.admin.pubkey(), &key.pubkey());
        send(&mut self.svm, &[instruction], &[&self.admin, &key]).expect("open position");
        Borrower { key }
    }

    /// Reads a zero-copy `Position` (unaligned, since test buffers are not 8-byte aligned).
    pub fn position(&self, owner: &Pubkey) -> hodl_loans::Position {
        let account = self.svm.get_account(&position_pda(owner)).expect("position exists");
        let size = std::mem::size_of::<hodl_loans::Position>();
        bytemuck::pod_read_unaligned(&account.data[8..8 + size])
    }

    /// Mints `amount` of `mint` into a new token account owned by the borrower, then deposits it.
    /// Returns the token account.
    pub fn deposit_collateral(&mut self, borrower: &Borrower, mint: &Pubkey, amount: u64) -> Pubkey {
        let token = self.create_token_account(mint, &borrower.pubkey());
        self.mint_to(mint, &token, amount);
        let program = self.mint_program(mint);
        let instruction = deposit_collateral_ix(&borrower.pubkey(), mint, &program, &token, amount);
        send(&mut self.svm, &[instruction], &[&self.admin, &borrower.key]).expect("deposit collateral");
        token
    }
}
```

`programs/hodl_loans/tests/position.rs`:

```rust
mod common;

use anchor_lang::error::ErrorCode as AnchorError;
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
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test position`
Expected: compile errors, including `cannot find struct, variant or union type OpenPosition in module hodl_loans::instruction` and `cannot find struct, variant or union type DepositCollateral in module hodl_loans::accounts`.

- [ ] **Step 3: Implement**

`programs/hodl_loans/src/state/position.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{MAX_COLLATERAL_SLOTS, MAX_LOAN_SLOTS};
use crate::math::loan::LoanTerms;

/// One collateral holding. `amount == 0` means the slot is free.
#[zero_copy]
#[derive(Debug, PartialEq, Eq)]
pub struct CollateralSlot {
    pub mint: Pubkey,
    pub amount: u64,
}

/// One fixed-term loan. `active == 0` means the slot is free.
#[zero_copy]
#[derive(Debug, PartialEq, Eq)]
pub struct LoanSlot {
    pub id: u64,
    pub principal: u64,
    pub original_principal: u64,
    pub repaid: u64,
    pub originated_at: i64,
    pub interest_anchor: i64,
    pub tenure_seconds: i64,
    pub rate_bps: u16,
    pub penalty_rate_bps: u16,
    pub reserve_factor_bps: u16,
    pub active: u8,
    pub _padding: u8,
}

impl LoanSlot {
    pub fn is_active(&self) -> bool {
        self.active != 0
    }

    pub fn terms(&self) -> LoanTerms {
        LoanTerms {
            principal: self.principal,
            originated_at: self.originated_at,
            interest_anchor: self.interest_anchor,
            tenure_seconds: self.tenure_seconds,
            rate_bps: self.rate_bps,
            penalty_rate_bps: self.penalty_rate_bps,
        }
    }
}

/// A borrower's collateral and loans. Zero-copy with fixed offsets so bots can scan it.
/// No `Option` fields: `market == Pubkey::default()` means no loan has been taken yet.
#[account(zero_copy)]
#[derive(Debug)]
pub struct Position {
    pub version: u8,
    pub bump: u8,
    pub _padding: [u8; 6],
    pub owner: Pubkey,
    pub rent_payer: Pubkey,
    /// The market this position's loans come from.
    pub market: Pubkey,
    pub next_loan_id: u64,
    pub promo_balance: u64,
    pub promo_last_activity_at: i64,
    pub collateral: [CollateralSlot; MAX_COLLATERAL_SLOTS],
    pub loans: [LoanSlot; MAX_LOAN_SLOTS],
    pub reserved: [u8; 64],
}

impl Position {
    pub fn collateral_index(&self, mint: &Pubkey) -> Option<usize> {
        self.collateral.iter().position(|s| s.amount > 0 && s.mint == *mint)
    }

    pub fn free_collateral_index(&self) -> Option<usize> {
        self.collateral.iter().position(|s| s.amount == 0)
    }

    pub fn has_collateral(&self) -> bool {
        self.collateral.iter().any(|s| s.amount > 0)
    }

    pub fn loan_index(&self, id: u64) -> Option<usize> {
        self.loans.iter().position(|l| l.is_active() && l.id == id)
    }

    pub fn free_loan_index(&self) -> Option<usize> {
        self.loans.iter().position(|l| !l.is_active())
    }

    pub fn has_active_loans(&self) -> bool {
        self.loans.iter().any(|l| l.is_active())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_fixed_size_without_padding_surprises() {
        assert_eq!(std::mem::size_of::<CollateralSlot>(), 40);
        assert_eq!(std::mem::size_of::<LoanSlot>(), 64);
        assert_eq!(std::mem::size_of::<Position>(), 8 + 32 * 3 + 8 * 3 + 40 * 8 + 64 * 10 + 64);
    }

    #[test]
    fn slot_helpers_find_used_and_free_slots() {
        let mut p: Position = bytemuck::Zeroable::zeroed();
        let mint = Pubkey::new_unique();
        assert_eq!(p.free_collateral_index(), Some(0));
        assert_eq!(p.collateral_index(&mint), None);
        p.collateral[0] = CollateralSlot { mint, amount: 5 };
        assert_eq!(p.collateral_index(&mint), Some(0));
        assert_eq!(p.free_collateral_index(), Some(1));
        assert!(p.has_collateral());

        assert_eq!(p.free_loan_index(), Some(0));
        p.loans[0].active = 1;
        p.loans[0].id = 7;
        assert_eq!(p.loan_index(7), Some(0));
        assert_eq!(p.loan_index(8), None);
        assert!(p.has_active_loans());
    }
}
```

Replace `programs/hodl_loans/src/state/mod.rs`:

```rust
pub mod access;
pub mod collateral;
pub mod config;
pub mod lender;
pub mod market;
pub mod position;

pub use access::*;
pub use collateral::*;
pub use config::*;
pub use lender::*;
pub use market::*;
pub use position::*;
```

Append to `programs/hodl_loans/src/events.rs`:

```rust
#[event]
pub struct PositionOpened {
    pub position: Pubkey,
    pub owner: Pubkey,
    pub rent_payer: Pubkey,
}

#[event]
pub struct PositionClosed {
    pub position: Pubkey,
    pub owner: Pubkey,
    pub rent_payer: Pubkey,
}

#[event]
pub struct CollateralDeposited {
    pub position: Pubkey,
    pub owner: Pubkey,
    pub mint: Pubkey,
    pub amount: u64,
    pub slot_amount: u64,
}
```

`programs/hodl_loans/src/instructions/positions/mod.rs`:

```rust
pub mod close_position;
pub mod deposit_collateral;
pub mod open_position;

pub use close_position::*;
pub use deposit_collateral::*;
pub use open_position::*;
```

`programs/hodl_loans/src/instructions/positions/open_position.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{ACCESS_SEED, ACCOUNT_VERSION, POSITION_SEED};
use crate::events::PositionOpened;
use crate::state::{Access, Position};

#[derive(Accounts)]
pub struct OpenPosition<'info> {
    /// Pays the position's rent (e.g. the gas-relay sponsor) and is refunded on close.
    #[account(mut)]
    pub payer: Signer<'info>,
    pub owner: Signer<'info>,
    #[account(seeds = [ACCESS_SEED, owner.key().as_ref()], bump = access.bump)]
    pub access: Account<'info, Access>,
    #[account(
        init,
        payer = payer,
        space = 8 + std::mem::size_of::<Position>(),
        seeds = [POSITION_SEED, owner.key().as_ref()],
        bump
    )]
    pub position: AccountLoader<'info, Position>,
    pub system_program: Program<'info, System>,
}

pub fn handle_open_position(ctx: Context<OpenPosition>) -> Result<()> {
    ctx.accounts.access.require_active()?;
    let mut position = ctx.accounts.position.load_init()?;
    position.version = ACCOUNT_VERSION;
    position.bump = ctx.bumps.position;
    position.owner = ctx.accounts.owner.key();
    position.rent_payer = ctx.accounts.payer.key();
    emit!(PositionOpened {
        position: ctx.accounts.position.key(),
        owner: position.owner,
        rent_payer: position.rent_payer,
    });
    Ok(())
}
```

`programs/hodl_loans/src/instructions/positions/close_position.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{ACCESS_SEED, POSITION_SEED};
use crate::errors::HodlError;
use crate::events::PositionClosed;
use crate::state::{Access, Position};

#[derive(Accounts)]
pub struct ClosePosition<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [ACCESS_SEED, owner.key().as_ref()], bump = access.bump)]
    pub access: Account<'info, Access>,
    #[account(
        mut,
        seeds = [POSITION_SEED, owner.key().as_ref()],
        bump,
        close = rent_payer,
        constraint = position.load()?.rent_payer == rent_payer.key() @ HodlError::Unauthorized
    )]
    pub position: AccountLoader<'info, Position>,
    /// CHECK: must equal `position.rent_payer`; only receives the rent refund.
    #[account(mut)]
    pub rent_payer: UncheckedAccount<'info>,
}

/// Requires no collateral and no active loans. Rent goes back to whoever paid it.
pub fn handle_close_position(ctx: Context<ClosePosition>) -> Result<()> {
    ctx.accounts.access.require_active()?;
    let position = ctx.accounts.position.load()?;
    require!(!position.has_collateral() && !position.has_active_loans(), HodlError::PositionNotEmpty);
    emit!(PositionClosed {
        position: ctx.accounts.position.key(),
        owner: position.owner,
        rent_payer: position.rent_payer,
    });
    Ok(())
}
```

`programs/hodl_loans/src/instructions/positions/deposit_collateral.rs`:

```rust
use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{ACCESS_SEED, COLLATERAL_SEED, POSITION_SEED};
use crate::errors::HodlError;
use crate::events::CollateralDeposited;
use crate::state::{Access, CollateralAsset, Position};
use crate::token::transfer::transfer_from_user;

#[derive(Accounts)]
pub struct DepositCollateral<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [ACCESS_SEED, owner.key().as_ref()], bump = access.bump)]
    pub access: Account<'info, Access>,
    #[account(mut, seeds = [POSITION_SEED, owner.key().as_ref()], bump)]
    pub position: AccountLoader<'info, Position>,
    #[account(mut, seeds = [COLLATERAL_SEED, mint.key().as_ref()], bump = collateral.bump, has_one = mint, has_one = vault)]
    pub collateral: Box<Account<'info, CollateralAsset>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, token::mint = mint, token::authority = owner, token::token_program = token_program)]
    pub owner_token: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
}

/// No prices needed. Uses the slot already holding this mint, or the first free slot.
pub fn handle_deposit_collateral(ctx: Context<DepositCollateral>, amount: u64) -> Result<()> {
    ctx.accounts.access.require_active()?;
    require!(amount > 0, HodlError::AmountTooSmall);
    let collateral = &mut ctx.accounts.collateral;
    require!(!collateral.paused, HodlError::CollateralPaused);
    let new_total = collateral.total_deposited.checked_add(amount).ok_or(HodlError::MathOverflow)?;
    require!(new_total <= collateral.deposit_cap, HodlError::DepositCapExceeded);
    collateral.total_deposited = new_total;

    let mint_key = ctx.accounts.mint.key();
    let slot_amount = {
        let mut position = ctx.accounts.position.load_mut()?;
        let index = match position.collateral_index(&mint_key) {
            Some(i) => i,
            None => position.free_collateral_index().ok_or(HodlError::NoFreeCollateralSlot)?,
        };
        let slot = &mut position.collateral[index];
        slot.mint = mint_key;
        slot.amount = slot.amount.checked_add(amount).ok_or(HodlError::MathOverflow)?;
        slot.amount
    };

    transfer_from_user(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.owner_token.to_account_info(),
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.owner.to_account_info(),
        amount,
    )?;
    emit!(CollateralDeposited {
        position: ctx.accounts.position.key(),
        owner: ctx.accounts.owner.key(),
        mint: mint_key,
        amount,
        slot_amount,
    });
    Ok(())
}
```

Replace `programs/hodl_loans/src/instructions/mod.rs`:

```rust
pub mod admin;
pub mod liquidity;
pub mod positions;

pub use admin::*;
pub use liquidity::*;
pub use positions::*;
```

In `lib.rs`, add inside `pub mod hodl_loans { … }` after `sweep_collateral_excess`:

```rust
    pub fn open_position(ctx: Context<OpenPosition>) -> Result<()> {
        instructions::handle_open_position(ctx)
    }

    pub fn close_position(ctx: Context<ClosePosition>) -> Result<()> {
        instructions::handle_close_position(ctx)
    }

    pub fn deposit_collateral(ctx: Context<DepositCollateral>, amount: u64) -> Result<()> {
        instructions::handle_deposit_collateral(ctx, amount)
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh --test position`
Expected: `test result: ok. 6 passed`.

Run: `./scripts/test.sh`
Expected: every binary reports `ok`; the unit binary shows `32 passed`.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: zero-copy positions with sponsored rent and collateral deposits"
```

---

### Task 6: Position valuation and `take_loan`

**Files:**
- Create: `programs/hodl_loans/src/valuation.rs`, `src/instructions/loans/mod.rs`, `src/instructions/loans/take_loan.rs` (under `programs/hodl_loans/`)
- Modify: `src/lib.rs`, `src/events.rs`, `src/instructions/mod.rs`, `tests/common/mod.rs`
- Test: `programs/hodl_loans/tests/loans.rs`

**Interfaces:**
- Consumes: `read_pyth_price`, `read_ngn_price` (Task 2); `loan_balance`, `lp_contribution`, `compute_health`, `CollateralValue`, `Health` (Task 3); `CollateralAsset` (Task 4); `Position`, `LoanSlot` (Task 5); `Market::{accrue, available_cash}`, `transfer_from_vault`.
- Produces:
  - `valuation::ACCOUNTS_PER_COLLATERAL: usize = 3`
  - `valuation::load_collateral_values(program_id: &Pubkey, position: &Position, remaining: &[AccountInfo], clock: &Clock) -> Result<Vec<CollateralValue>>`
  - `valuation::total_debt(position: &Position, now: i64) -> Result<u128>`
  - `valuation::load_health(program_id: &Pubkey, position: &Position, market: &Market, ngn_feed: &AccountInfo, remaining: &[AccountInfo], extra_debt: u64, clock: &Clock) -> Result<Health>`
  - Event `LoanOpened`
  - Instruction `take_loan(amount: u64, tenure_seconds: i64)`: `owner, access, position, market, mint, vault, owner_token, ngn_feed, token_program`, then the price triples
  - Harness constants: `NGN_USD`, `NGN_SPREAD`, `ONE_DOLLAR`, `POOL_CNGN`
  - Harness functions: `pyth_account(&mint)`, `ngn_feed()`, `pull_feed_data(..)`, `price_update_data(..)`, `take_loan_ix`
  - Harness `Env` methods: `set_account_data`, `set_pyth_price`, `set_ngn_price`, `price_accounts(&owner)`, `take_loan`, `loan_ready() -> (Env, LoanSetup)`
  - Harness struct: `LoanSetup { cngn, usdc, lender, borrower, borrower_cngn }`

- [ ] **Step 1: Write the failing tests**

In the `use anchor_lang::{…}` import at the top of `programs/hodl_loans/tests/common/mod.rs`, change `instruction::Instruction` to `instruction::{AccountMeta, Instruction}`. Then append:

```rust
// ---- Prices and loans (Task 6) ----

/// $0.000625 per NGN (NGN/USD 1,600) at Switchboard's 18 decimals.
pub const NGN_USD: i128 = 625_000_000_000_000;
/// A 0.1% spread on `NGN_USD`.
pub const NGN_SPREAD: i128 = 625_000_000_000;
/// $1.00 at Pyth exponent -8.
pub const ONE_DOLLAR: i64 = 100_000_000;

/// Where tests store a mint's Pyth `PriceUpdateV2` account.
pub fn pyth_account(mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"pyth", mint.as_ref()], &pyth_solana_receiver_sdk::ID).0
}

pub fn ngn_feed() -> Pubkey {
    default_market_params().ngn_feed
}

/// Raw Switchboard `PullFeedAccountData` with only the aggregated result set.
pub fn pull_feed_data(value: i128, std_dev: i128, slot: u64, num_samples: u8) -> Vec<u8> {
    use switchboard_on_demand::{Discriminator, PullFeedAccountData};
    let mut feed: PullFeedAccountData = bytemuck::Zeroable::zeroed();
    feed.result.value = value;
    feed.result.std_dev = std_dev;
    feed.result.slot = slot;
    feed.result.num_samples = num_samples;
    let mut data = PullFeedAccountData::DISCRIMINATOR.to_vec();
    data.extend_from_slice(bytemuck::bytes_of(&feed));
    data
}

pub fn price_update_data(
    mint: &Pubkey,
    price: i64,
    conf: u64,
    publish_time: i64,
    level: pyth_solana_receiver_sdk::price_update::VerificationLevel,
) -> Vec<u8> {
    use pyth_solana_receiver_sdk::price_update::{PriceFeedMessage, PriceUpdateV2};
    let update = PriceUpdateV2 {
        write_authority: Pubkey::new_unique(),
        verification_level: level,
        price_message: PriceFeedMessage {
            feed_id: feed_id(mint),
            price,
            conf,
            exponent: -8,
            publish_time,
            prev_publish_time: publish_time - 1,
            ema_price: price,
            ema_conf: conf,
        },
        posted_slot: 1,
    };
    let mut data = Vec::new();
    update.try_serialize(&mut data).unwrap();
    data
}

pub fn take_loan_ix(owner: &Pubkey, mint: &Pubkey, owner_token: &Pubkey, amount: u64, tenure_seconds: i64, prices: Vec<AccountMeta>) -> Instruction {
    let mut instruction = ix(
        hodl_loans::instruction::TakeLoan { amount, tenure_seconds },
        hodl_loans::accounts::TakeLoan {
            owner: *owner,
            access: access_pda(owner),
            position: position_pda(owner),
            market: market_pda(mint),
            mint: *mint,
            vault: market_vault_pda(mint),
            owner_token: *owner_token,
            ngn_feed: ngn_feed(),
            token_program: TOKEN_2022,
        },
    );
    instruction.accounts.extend(prices);
    instruction
}

impl Env {
    pub fn set_account_data(&mut self, key: &Pubkey, owner: &Pubkey, data: Vec<u8>) {
        let account = solana_account::Account {
            lamports: self.svm.minimum_balance_for_rent_exemption(data.len()),
            data,
            owner: *owner,
            executable: false,
            rent_epoch: 0,
        };
        self.svm.set_account(*key, account).unwrap();
    }

    /// Writes a fully verified Pyth price (exponent -8) for `mint`, published now.
    pub fn set_pyth_price(&mut self, mint: &Pubkey, price: i64, conf: u64) {
        let now = self.now();
        let data = price_update_data(mint, price, conf, now, pyth_solana_receiver_sdk::price_update::VerificationLevel::Full);
        self.set_account_data(&pyth_account(mint), &pyth_solana_receiver_sdk::ID, data);
    }

    /// Writes the Switchboard NGN/USD result at the current slot with 5 samples.
    pub fn set_ngn_price(&mut self, value: i128, std_dev: i128) {
        let slot = self.svm.get_sysvar::<Clock>().slot;
        self.set_account_data(&ngn_feed(), &switchboard_on_demand::ON_DEMAND_MAINNET_PID, pull_feed_data(value, std_dev, slot, 5));
    }

    /// One `(CollateralAsset, PriceUpdateV2, mint)` triple per used collateral slot, in slot order.
    pub fn price_accounts(&self, owner: &Pubkey) -> Vec<AccountMeta> {
        let position = self.position(owner);
        position
            .collateral
            .iter()
            .filter(|slot| slot.amount > 0)
            .flat_map(|slot| {
                [
                    AccountMeta::new_readonly(collateral_pda(&slot.mint), false),
                    AccountMeta::new_readonly(pyth_account(&slot.mint), false),
                    AccountMeta::new_readonly(slot.mint, false),
                ]
            })
            .collect()
    }

    pub fn take_loan(&mut self, borrower: &Borrower, setup: &LoanSetup, amount: u64, tenure_seconds: i64) -> TxResult {
        let prices = self.price_accounts(&borrower.pubkey());
        let instruction = take_loan_ix(&borrower.pubkey(), &setup.cngn, &setup.borrower_cngn, amount, tenure_seconds, prices);
        send(&mut self.svm, &[instruction], &[&self.admin, &borrower.key])
    }
}

/// A market ready to lend: see `Env::loan_ready`.
pub struct LoanSetup {
    pub cngn: Pubkey,
    pub usdc: Pubkey,
    pub lender: Lender,
    pub borrower: Borrower,
    /// The borrower's cNGN token account (loans are paid here).
    pub borrower_cngn: Pubkey,
}

/// Lender liquidity in `Env::loan_ready`: 10,000,000 cNGN.
pub const POOL_CNGN: u64 = 10_000_000 * ONE_CNGN;

impl Env {
    /// cNGN market holding `POOL_CNGN` of lender liquidity; NGN at `NGN_USD` with a 0.1% spread;
    /// USDC (6 decimals, classic SPL Token) listed at exactly $1; and a borrower with 1,000 USDC
    /// deposited, so the borrow limit is $700.
    pub fn loan_ready() -> (Self, LoanSetup) {
        let (mut env, cngn) = Self::with_cngn_market();
        // The program reads a Switchboard result from slot 0 as never updated.
        env.svm.warp_to_slot(1_000);
        let lender = env.new_lender(&cngn, POOL_CNGN);
        env.deposit(&lender, &cngn, POOL_CNGN).unwrap();
        env.set_ngn_price(NGN_USD, NGN_SPREAD);

        let usdc = env.list_spl_collateral(6);
        env.set_pyth_price(&usdc, ONE_DOLLAR, 0);

        let borrower = env.new_borrower();
        env.deposit_collateral(&borrower, &usdc, 1_000 * ONE_USDC);
        let borrower_cngn = env.create_token_account(&cngn, &borrower.pubkey());
        (env, LoanSetup { cngn, usdc, lender, borrower, borrower_cngn })
    }
}
```

`programs/hodl_loans/tests/loans.rs`:

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
    let usdt_triple = vec![
        AccountMeta::new_readonly(collateral_pda(&usdt), false),
        AccountMeta::new_readonly(pyth_account(&usdt), false),
        AccountMeta::new_readonly(usdt, false),
    ];
    let replace = |i: usize, key| {
        let mut prices = good.clone();
        prices[i] = AccountMeta::new_readonly(key, false);
        prices
    };

    let cases = vec![
        ("missing", vec![]),
        ("extra triple", [good.clone(), usdt_triple.clone()].concat()),
        ("another asset", usdt_triple.clone()),
        ("another asset's price", replace(1, pyth_account(&usdt))),
        ("another mint", replace(2, usdt)),
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
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test loans`
Expected: compile errors `cannot find struct, variant or union type TakeLoan in module hodl_loans::instruction` and `… in module hodl_loans::accounts`.

- [ ] **Step 3: Implement**

`programs/hodl_loans/src/valuation.rs`:

```rust
use anchor_lang::prelude::*;

use crate::errors::HodlError;
use crate::math::checked::add;
use crate::math::health::{compute_health, CollateralValue, Health};
use crate::math::loan::loan_balance;
use crate::oracle::pyth::read_pyth_price;
use crate::oracle::switchboard::read_ngn_price;
use crate::state::{CollateralAsset, Market, Position};

/// Accounts per used collateral slot in `remaining_accounts`: `(CollateralAsset, PriceUpdateV2, mint)`.
pub const ACCOUNTS_PER_COLLATERAL: usize = 3;

/// Value every used collateral slot, in slot order, from `remaining` triples.
///
/// The loop runs over the position's slots, not over the accounts supplied, so a missing,
/// extra or mismatched account fails with `PriceAccountMismatch` instead of skipping collateral.
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
        let (asset_info, price_info, mint_info) = (&accounts[0], &accounts[1], &accounts[2]);
        require_keys_eq!(*asset_info.owner, *program_id, HodlError::PriceAccountMismatch);
        let asset = {
            let data = asset_info.try_borrow_data()?;
            CollateralAsset::try_deserialize(&mut &data[..]).map_err(|_| HodlError::PriceAccountMismatch)?
        };
        require_keys_eq!(asset.mint, slot.mint, HodlError::PriceAccountMismatch);
        require_keys_eq!(mint_info.key(), slot.mint, HodlError::PriceAccountMismatch);
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
    let collateral = load_collateral_values(program_id, position, remaining, clock)?;
    let ngn = read_ngn_price(ngn_feed, market, clock)?;
    let debt = add(total_debt(position, clock.unix_timestamp)?, extra_debt as u128)?;
    compute_health(&collateral, debt, market.decimals, ngn)
}
```

In `programs/hodl_loans/src/lib.rs`, add `pub mod valuation;` directly below `pub mod token;`.

Append to `programs/hodl_loans/src/events.rs`:

```rust
#[event]
pub struct LoanOpened {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub loan_id: u64,
    pub principal: u64,
    pub tenure_seconds: i64,
    pub rate_bps: u16,
    pub penalty_rate_bps: u16,
    pub reserve_factor_bps: u16,
    pub originated_at: i64,
}
```

`programs/hodl_loans/src/instructions/loans/mod.rs`:

```rust
pub mod take_loan;

pub use take_loan::*;
```

`programs/hodl_loans/src/instructions/loans/take_loan.rs`:

```rust
use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{ACCESS_SEED, BPS, MARKET_SEED, MIN_TENURE, POSITION_SEED};
use crate::errors::HodlError;
use crate::events::LoanOpened;
use crate::math::checked::add;
use crate::math::loan::lp_contribution;
use crate::state::{Access, LoanSlot, Market, Position};
use crate::token::transfer::transfer_from_vault;
use crate::valuation::load_health;

#[derive(Accounts)]
pub struct TakeLoan<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [ACCESS_SEED, owner.key().as_ref()], bump = access.bump)]
    pub access: Account<'info, Access>,
    #[account(mut, seeds = [POSITION_SEED, owner.key().as_ref()], bump)]
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
    #[account(mut, token::mint = mint, token::authority = owner, token::token_program = token_program)]
    pub owner_token: Box<InterfaceAccount<'info, TokenAccount>>,
    /// CHECK: address pinned to `market.ngn_feed`; parsed by `read_ngn_price`.
    pub ngn_feed: UncheckedAccount<'info>,
    pub token_program: Interface<'info, TokenInterface>,
}

/// Spec §10 `take_loan`. `remaining_accounts`: one `(CollateralAsset, PriceUpdateV2, mint)`
/// triple per used collateral slot, in slot order.
pub fn handle_take_loan<'info>(
    ctx: Context<'info, TakeLoan<'info>>,
    amount: u64,
    tenure_seconds: i64,
) -> Result<()> {
    ctx.accounts.access.require_active()?;
    let market_key = ctx.accounts.market.key();
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    let market = &mut ctx.accounts.market;
    require!(!market.paused, HodlError::MarketPaused);
    require!(amount > 0 && amount >= market.min_loan_amount, HodlError::AmountTooSmall);
    require!(
        (MIN_TENURE..=market.max_tenure_seconds).contains(&tenure_seconds),
        HodlError::TenureOutOfRange
    );
    market.accrue(now)?;

    let available = market.available_cash();
    require!(amount <= available, HodlError::InsufficientCash);
    let lendable = add(available as u128, market.total_borrows as u128)?;
    let borrows_after = add(market.total_borrows as u128, amount as u128)?;
    require!(
        borrows_after * BPS <= lendable * market.max_utilization_bps as u128,
        HodlError::UtilizationCapExceeded
    );

    let (loan_id, rate_bps, penalty_rate_bps, reserve_factor_bps) = {
        let mut position = ctx.accounts.position.load_mut()?;
        require!(
            position.market == Pubkey::default() || position.market == market_key,
            HodlError::MarketMismatch
        );
        let index = position.free_loan_index().ok_or(HodlError::NoFreeLoanSlot)?;
        let health = load_health(
            ctx.program_id,
            &position,
            market,
            &ctx.accounts.ngn_feed.to_account_info(),
            ctx.remaining_accounts,
            amount,
            &clock,
        )?;
        require!(health.is_healthy(), HodlError::Unhealthy);

        let loan_id = position.next_loan_id;
        position.next_loan_id = loan_id.checked_add(1).ok_or(HodlError::MathOverflow)?;
        position.loans[index] = LoanSlot {
            id: loan_id,
            principal: amount,
            original_principal: amount,
            repaid: 0,
            originated_at: now,
            interest_anchor: now,
            tenure_seconds,
            rate_bps: market.interest_rate_bps,
            penalty_rate_bps: market.penalty_rate_bps,
            reserve_factor_bps: market.reserve_factor_bps,
            active: 1,
            _padding: 0,
        };
        position.market = market_key;
        position.promo_last_activity_at = now;
        (loan_id, market.interest_rate_bps, market.penalty_rate_bps, market.reserve_factor_bps)
    };

    market.total_borrows = market.total_borrows.checked_add(amount).ok_or(HodlError::MathOverflow)?;
    market.lp_rate_product = add(
        market.lp_rate_product,
        lp_contribution(amount, rate_bps, reserve_factor_bps)?,
    )?;
    market.cash -= amount;

    let mint_key = ctx.accounts.mint.key();
    let seeds: &[&[u8]] = &[MARKET_SEED, mint_key.as_ref(), &[ctx.accounts.market.bump]];
    transfer_from_vault(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.owner_token.to_account_info(),
        ctx.accounts.market.to_account_info(),
        amount,
        &[seeds],
    )?;
    emit!(LoanOpened {
        market: market_key,
        position: ctx.accounts.position.key(),
        owner: ctx.accounts.owner.key(),
        loan_id,
        principal: amount,
        tenure_seconds,
        rate_bps,
        penalty_rate_bps,
        reserve_factor_bps,
        originated_at: now,
    });
    Ok(())
}
```

Replace `programs/hodl_loans/src/instructions/mod.rs`:

```rust
pub mod admin;
pub mod liquidity;
pub mod loans;
pub mod positions;

pub use admin::*;
pub use liquidity::*;
pub use loans::*;
pub use positions::*;
```

In `lib.rs`, add inside `pub mod hodl_loans { … }` after `deposit_collateral`:

```rust
    pub fn take_loan<'info>(ctx: Context<'info, TakeLoan<'info>>, amount: u64, tenure_seconds: i64) -> Result<()> {
        instructions::handle_take_loan(ctx, amount, tenure_seconds)
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh --test loans`
Expected: `test result: ok. 11 passed`.

Run: `./scripts/test.sh`
Expected: every binary reports `ok`.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: price-checked fixed-term loans with utilization cap"
```

---

### Task 7: `repay_loan`

**Files:**
- Create: `programs/hodl_loans/src/instructions/loans/repay_loan.rs`
- Modify: `src/events.rs`, `src/instructions/loans/mod.rs`, `src/lib.rs`, `tests/common/mod.rs` (under `programs/hodl_loans/`)
- Test: `programs/hodl_loans/tests/repay.rs`

**Interfaces:**
- Consumes: `loan_balance`, `accrued_lp_interest`, `lp_contribution`, `reserve_share` (Task 3); `Position` (Task 5); `LoanSetup`, `Env::take_loan` (Task 6); `math::checked::{sub, to_u64}`, `transfer_from_user`.
- Produces:
  - Events `LoanRepaid`, `LoanPartiallyRepaid`, each `{ market, position, owner, payer, loan_id, amount, principal_repaid, interest_paid, remaining_principal }`
  - Instruction `repay_loan(loan_id: u64, amount: u64)` (`u64::MAX` repays the full balance): `payer, access, position, market, mint, vault, payer_token, token_program`. Needs no prices.
  - Harness: `repay_loan_ix(payer, position_owner, mint, payer_token, loan_id, amount)`, `Env::repay(&setup, loan_id, amount)`

- [ ] **Step 1: Write the failing tests**

Append to `programs/hodl_loans/tests/common/mod.rs`:

```rust
// ---- Repayment (Task 7) ----

pub fn repay_loan_ix(payer: &Pubkey, position_owner: &Pubkey, mint: &Pubkey, payer_token: &Pubkey, loan_id: u64, amount: u64) -> Instruction {
    ix(
        hodl_loans::instruction::RepayLoan { loan_id, amount },
        hodl_loans::accounts::RepayLoan {
            payer: *payer,
            access: access_pda(payer),
            position: position_pda(position_owner),
            market: market_pda(mint),
            mint: *mint,
            vault: market_vault_pda(mint),
            payer_token: *payer_token,
            token_program: TOKEN_2022,
        },
    )
}

impl Env {
    /// The borrower repays from its own cNGN account.
    pub fn repay(&mut self, setup: &LoanSetup, loan_id: u64, amount: u64) -> TxResult {
        let owner = setup.borrower.pubkey();
        let instruction = repay_loan_ix(&owner, &owner, &setup.cngn, &setup.borrower_cngn, loan_id, amount);
        send(&mut self.svm, &[instruction], &[&self.admin, &setup.borrower.key])
    }
}
```

`programs/hodl_loans/tests/repay.rs`:

```rust
mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use common::*;
use hodl_loans::{HodlError, LoanSlot};
use solana_keypair::Keypair;
use solana_signer::Signer;

const DAY: i64 = 86_400;
/// 100,000 cNGN.
const LOAN: u64 = 100_000 * ONE_CNGN;
/// 15% a year for 73 days (a fifth of a year) on `LOAN`.
const INTEREST_73_DAYS: u64 = 3_000 * ONE_CNGN;

#[test]
fn full_repayment_pays_lenders_and_the_reserve() {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    env.warp_seconds(73 * DAY);
    env.mint_to(&setup.cngn, &setup.borrower_cngn, INTEREST_73_DAYS);

    // Repayment works while the market is paused and needs no prices (they are stale by now).
    let pause = set_market_paused_ix(&env.guardian.pubkey(), &setup.cngn, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();
    env.repay(&setup, 0, u64::MAX).unwrap();

    assert_eq!(env.token_balance(&setup.borrower_cngn), 0);
    let market = env.market(&setup.cngn);
    assert_eq!((market.total_borrows, market.lp_rate_product, market.accrued_interest), (0, 0, 0));
    assert_eq!(market.cash, POOL_CNGN + INTEREST_73_DAYS);
    // 10% reserve factor.
    assert_eq!(market.protocol_reserve, 300 * ONE_CNGN);
    assert_eq!(env.token_balance(&market_vault_pda(&setup.cngn)), POOL_CNGN + INTEREST_73_DAYS);

    let position = env.position(&setup.borrower.pubkey());
    assert_eq!(position.loans[0], bytemuck::Zeroable::zeroed());
    assert!(!position.has_active_loans());
    assert_eq!(position.promo_last_activity_at, env.now());
    assert_eq!(position.next_loan_id, 1);

    // The lender redeems principal plus 90% of the interest (minus share rounding).
    let unpause = set_market_paused_ix(&env.admin.pubkey(), &setup.cngn, false);
    send(&mut env.svm, &[unpause], &[&env.admin]).unwrap();
    let shares = env.lender_shares(&setup.cngn, &setup.lender.key.pubkey());
    let total_assets = POOL_CNGN as u128 + INTEREST_73_DAYS as u128 - 300 * ONE_CNGN as u128;
    let expected = shares * (total_assets + 1) / (shares + 1_000);
    env.withdraw(&setup.lender, &setup.cngn, u64::MAX).unwrap();
    assert_eq!(env.token_balance(&setup.lender.token) as u128, expected);
    assert!(expected > (POOL_CNGN + 2_699 * ONE_CNGN) as u128);
}

#[test]
fn partial_repayment_pays_interest_first_and_restarts_it() {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    let originated_at = env.now();
    env.warp_seconds(73 * DAY);
    env.mint_to(&setup.cngn, &setup.borrower_cngn, INTEREST_73_DAYS);

    assert_hodl_error(env.repay(&setup, 0, INTEREST_73_DAYS - 1), HodlError::RepaymentBelowInterest);
    env.repay(&setup, 0, INTEREST_73_DAYS + 50_000 * ONE_CNGN).unwrap();

    let loan = env.position(&setup.borrower.pubkey()).loans[0];
    assert_eq!((loan.active, loan.principal, loan.original_principal), (1, 50_000 * ONE_CNGN, LOAN));
    assert_eq!(loan.repaid, INTEREST_73_DAYS + 50_000 * ONE_CNGN);
    assert_eq!((loan.originated_at, loan.interest_anchor), (originated_at, env.now()));
    let market = env.market(&setup.cngn);
    assert_eq!(market.total_borrows, 50_000 * ONE_CNGN);
    assert_eq!(market.lp_rate_product, 50_000 * ONE_CNGN as u128 * 1_500 * 9_000);
    assert_eq!((market.accrued_interest, market.protocol_reserve), (0, 300 * ONE_CNGN));

    // Another 73 days accrues interest only on the remaining 50,000 cNGN.
    env.warp_seconds(73 * DAY);
    env.mint_to(&setup.cngn, &setup.borrower_cngn, 1_500 * ONE_CNGN);
    let before = env.token_balance(&setup.borrower_cngn);
    env.repay(&setup, 0, u64::MAX).unwrap();
    assert_eq!(before - env.token_balance(&setup.borrower_cngn), 51_500 * ONE_CNGN);
    let market = env.market(&setup.cngn);
    assert_eq!((market.total_borrows, market.lp_rate_product, market.accrued_interest), (0, 0, 0));
    assert_eq!(market.protocol_reserve, 450 * ONE_CNGN);
}

#[test]
fn overdue_loans_add_penalty_interest() {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, LOAN, 73 * DAY).unwrap();
    env.warp_seconds(146 * DAY);
    // Interest to maturity 3,000; then (100,000 + 3,000) × (15% + 5%) × 1/5 year = 4,120.
    let due = 7_120 * ONE_CNGN;
    env.mint_to(&setup.cngn, &setup.borrower_cngn, due);

    assert_hodl_error(env.repay(&setup, 0, due - 1), HodlError::RepaymentBelowInterest);
    env.repay(&setup, 0, u64::MAX).unwrap();
    assert_eq!(env.token_balance(&setup.borrower_cngn), 0);
    let market = env.market(&setup.cngn);
    assert_eq!(market.cash, POOL_CNGN + due);
    assert_eq!(market.protocol_reserve, 712 * ONE_CNGN);
    assert_eq!((market.total_borrows, market.accrued_interest), (0, 0));
}

#[test]
fn any_whitelisted_wallet_can_repay_even_for_a_blacklisted_borrower() {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    env.blacklist(&setup.borrower.pubkey());
    assert_hodl_error(env.repay(&setup, 0, LOAN), HodlError::Blacklisted);

    let helper = env.new_lender(&setup.cngn, LOAN);
    let owner = setup.borrower.pubkey();
    let instruction = repay_loan_ix(&helper.key.pubkey(), &owner, &setup.cngn, &helper.token, 0, u64::MAX);
    send(&mut env.svm, &[instruction], &[&env.admin, &helper.key]).unwrap();
    assert_eq!(env.token_balance(&helper.token), 0);
    assert!(!env.position(&owner).has_active_loans());
    // The borrower keeps the loaned cNGN.
    assert_eq!(env.token_balance(&setup.borrower_cngn), LOAN);
}

#[test]
fn repayment_rejections() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();

    // A position that has never borrowed has no market.
    assert_hodl_error(env.repay(&setup, 0, LOAN), HodlError::MarketMismatch);

    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    assert_hodl_error(env.repay(&setup, 0, 0), HodlError::AmountTooSmall);
    assert_hodl_error(env.repay(&setup, 1, LOAN), HodlError::LoanNotFound);
    env.repay(&setup, 0, LOAN).unwrap();
    assert_hodl_error(env.repay(&setup, 0, LOAN), HodlError::LoanNotFound);

    let stranger = Keypair::new();
    let stranger_token = env.create_token_account(&setup.cngn, &stranger.pubkey());
    let instruction = repay_loan_ix(&stranger.pubkey(), &owner, &setup.cngn, &stranger_token, 0, LOAN);
    assert_anchor_error(send(&mut env.svm, &[instruction], &[&env.admin, &stranger]), AnchorError::AccountNotInitialized);

    // Repaying through a different market.
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    let other = env.create_mint(MintKind::CngnLike, 6);
    let create = create_market_ix(&env.admin.pubkey(), &other, &TOKEN_2022, default_market_params());
    send(&mut env.svm, &[create], &[&env.admin]).unwrap();
    let other_token = env.create_token_account(&other, &owner);
    let instruction = repay_loan_ix(&owner, &owner, &other, &other_token, 1, LOAN);
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin, &setup.borrower.key]), HodlError::MarketMismatch);
}

#[test]
fn repaid_slots_are_reused_but_ids_are_not() {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    env.repay(&setup, 0, LOAN).unwrap();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();

    let loans = env.position(&setup.borrower.pubkey()).loans;
    let (first, second): (LoanSlot, LoanSlot) = (loans[0], loans[1]);
    assert_eq!((first.id, first.active, second.id, second.active), (2, 1, 1, 1));
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test repay`
Expected: compile errors `cannot find struct, variant or union type RepayLoan in module hodl_loans::instruction` and `… in module hodl_loans::accounts`.

- [ ] **Step 3: Implement**

Append to `programs/hodl_loans/src/events.rs`:

```rust
#[event]
pub struct LoanRepaid {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub payer: Pubkey,
    pub loan_id: u64,
    pub amount: u64,
    pub principal_repaid: u64,
    pub interest_paid: u64,
    pub remaining_principal: u64,
}

#[event]
pub struct LoanPartiallyRepaid {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub payer: Pubkey,
    pub loan_id: u64,
    pub amount: u64,
    pub principal_repaid: u64,
    pub interest_paid: u64,
    pub remaining_principal: u64,
}
```

`programs/hodl_loans/src/instructions/loans/repay_loan.rs`:

```rust
use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{ACCESS_SEED, MARKET_SEED};
use crate::errors::HodlError;
use crate::events::{LoanPartiallyRepaid, LoanRepaid};
use crate::math::checked::{sub, to_u64};
use crate::math::loan::{accrued_lp_interest, loan_balance, lp_contribution, reserve_share};
use crate::state::{Access, Market, Position};
use crate::token::transfer::transfer_from_user;

#[derive(Accounts)]
pub struct RepayLoan<'info> {
    /// Any whitelisted wallet may repay any position's loan.
    pub payer: Signer<'info>,
    #[account(seeds = [ACCESS_SEED, payer.key().as_ref()], bump = access.bump)]
    pub access: Account<'info, Access>,
    #[account(mut)]
    pub position: AccountLoader<'info, Position>,
    #[account(mut, seeds = [MARKET_SEED, mint.key().as_ref()], bump = market.bump, has_one = mint, has_one = vault)]
    pub market: Box<Account<'info, Market>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, token::mint = mint, token::authority = payer, token::token_program = token_program)]
    pub payer_token: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
}

/// Spec §10 `repay_loan`. Needs no prices, so it works during an oracle outage or pause.
pub fn handle_repay_loan(ctx: Context<RepayLoan>, loan_id: u64, amount: u64) -> Result<()> {
    ctx.accounts.access.require_active()?;
    require!(amount > 0, HodlError::AmountTooSmall);
    let market_key = ctx.accounts.market.key();
    let now = Clock::get()?.unix_timestamp;

    let market = &mut ctx.accounts.market;
    market.accrue(now)?;

    let mut position = ctx.accounts.position.load_mut()?;
    require_keys_eq!(position.market, market_key, HodlError::MarketMismatch);
    let index = position.loan_index(loan_id).ok_or(HodlError::LoanNotFound)?;
    let loan = position.loans[index];
    let owner = position.owner;

    let balance = loan_balance(&loan.terms(), now)?;
    let due = balance.due()?;
    let paid = (amount as u128).min(balance.total()?);
    require!(paid >= due, HodlError::RepaymentBelowInterest);
    let principal_repaid = to_u64(paid - due)?;
    let paid = to_u64(paid)?;

    // Release exactly the lender interest the market accrued for this loan since its anchor.
    let released = accrued_lp_interest(loan.principal, loan.rate_bps, loan.reserve_factor_bps, loan.interest_anchor, now)?;
    market.accrued_interest = market.accrued_interest.saturating_sub(released);
    market.protocol_reserve = market
        .protocol_reserve
        .checked_add(to_u64(reserve_share(due, loan.reserve_factor_bps)?)?)
        .ok_or(HodlError::MathOverflow)?;
    market.total_borrows = market.total_borrows.checked_sub(principal_repaid).ok_or(HodlError::MathOverflow)?;
    market.lp_rate_product = sub(
        market.lp_rate_product,
        lp_contribution(principal_repaid, loan.rate_bps, loan.reserve_factor_bps)?,
    )?;
    market.cash = market.cash.checked_add(paid).ok_or(HodlError::MathOverflow)?;

    let slot = &mut position.loans[index];
    slot.principal -= principal_repaid;
    slot.repaid = slot.repaid.checked_add(paid).ok_or(HodlError::MathOverflow)?;
    slot.interest_anchor = now;
    let remaining_principal = slot.principal;
    if remaining_principal == 0 {
        *slot = bytemuck::Zeroable::zeroed();
        if !position.has_active_loans() {
            position.promo_last_activity_at = now;
        }
    }
    drop(position);

    transfer_from_user(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.payer_token.to_account_info(),
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.payer.to_account_info(),
        paid,
    )?;

    let position_key = ctx.accounts.position.key();
    let payer = ctx.accounts.payer.key();
    let interest_paid = to_u64(due)?;
    if remaining_principal == 0 {
        emit!(LoanRepaid { market: market_key, position: position_key, owner, payer, loan_id, amount: paid, principal_repaid, interest_paid, remaining_principal });
    } else {
        emit!(LoanPartiallyRepaid { market: market_key, position: position_key, owner, payer, loan_id, amount: paid, principal_repaid, interest_paid, remaining_principal });
    }
    Ok(())
}
```

Replace `programs/hodl_loans/src/instructions/loans/mod.rs`:

```rust
pub mod repay_loan;
pub mod take_loan;

pub use repay_loan::*;
pub use take_loan::*;
```

In `lib.rs`, add inside `pub mod hodl_loans { … }` after `take_loan`:

```rust
    pub fn repay_loan(ctx: Context<RepayLoan>, loan_id: u64, amount: u64) -> Result<()> {
        instructions::handle_repay_loan(ctx, loan_id, amount)
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh --test repay`
Expected: `test result: ok. 6 passed`.

Run: `./scripts/test.sh`
Expected: every binary reports `ok`.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: interest-first repayment by any whitelisted payer"
```

---

### Task 8: `withdraw_collateral`

**Files:**
- Create: `programs/hodl_loans/src/instructions/positions/withdraw_collateral.rs`
- Modify: `src/events.rs`, `src/instructions/positions/mod.rs`, `src/lib.rs`, `tests/common/mod.rs` (under `programs/hodl_loans/`)
- Test: `programs/hodl_loans/tests/withdraw.rs`

**Interfaces:**
- Consumes: `load_health` (Task 6), `Position`, `CollateralAsset`, `transfer_from_vault`; harness `loan_ready`, `Env::sponsored`, `pyth_account`.
- Produces:
  - Event `CollateralWithdrawn`
  - Instruction `withdraw_collateral(amount: u64)`: `owner, access, position, collateral, mint, vault, owner_token, market (optional), ngn_feed (optional), token_program`, then price triples for the slots still used after the withdrawal
  - Harness: `withdraw_collateral_ix(owner, mint, token_program, owner_token, market_mint: Option<&Pubkey>, amount, prices)`, `price_triples(&[Pubkey]) -> Vec<AccountMeta>`

- [ ] **Step 1: Write the failing tests**

Append to `programs/hodl_loans/tests/common/mod.rs`:

```rust
// ---- Collateral withdrawals (Task 8) ----

/// `market_mint` is the borrowed market's mint; pass `None` when the position has no active loans.
pub fn withdraw_collateral_ix(
    owner: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
    owner_token: &Pubkey,
    market_mint: Option<&Pubkey>,
    amount: u64,
    prices: Vec<AccountMeta>,
) -> Instruction {
    let mut instruction = ix(
        hodl_loans::instruction::WithdrawCollateral { amount },
        hodl_loans::accounts::WithdrawCollateral {
            owner: *owner,
            access: access_pda(owner),
            position: position_pda(owner),
            collateral: collateral_pda(mint),
            mint: *mint,
            vault: collateral_vault_pda(mint),
            owner_token: *owner_token,
            market: market_mint.map(market_pda),
            ngn_feed: market_mint.map(|_| ngn_feed()),
            token_program: *token_program,
        },
    );
    instruction.accounts.extend(prices);
    instruction
}

/// One `(CollateralAsset, PriceUpdateV2, mint)` triple per listed mint, in the order given.
pub fn price_triples(mints: &[Pubkey]) -> Vec<AccountMeta> {
    mints
        .iter()
        .flat_map(|mint| {
            [
                AccountMeta::new_readonly(collateral_pda(mint), false),
                AccountMeta::new_readonly(pyth_account(mint), false),
                AccountMeta::new_readonly(*mint, false),
            ]
        })
        .collect()
}
```

`programs/hodl_loans/tests/withdraw.rs`:

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
    let no_market = withdraw_collateral_ix(&owner, &usdc, &SPL_TOKEN, &token, None, ONE_USDC, price_triples(&[usdc]));
    assert_hodl_error(env.sponsored(no_market, key), HodlError::PriceAccountMismatch);
    assert_hodl_error(env.sponsored(withdraw(ONE_USDC, vec![]), key), HodlError::PriceAccountMismatch);

    // 447 USDC × 70% = $312.90 covers the debt; 446 USDC ($312.20) does not.
    env.sponsored(withdraw(553 * ONE_USDC, price_triples(&[usdc])), key).unwrap();
    assert_hodl_error(env.sponsored(withdraw(ONE_USDC, price_triples(&[usdc])), key), HodlError::Unhealthy);
    // Withdrawing everything leaves no used slot to price, and no collateral value.
    assert_hodl_error(env.sponsored(withdraw(447 * ONE_USDC, vec![]), key), HodlError::Unhealthy);

    // Stale prices block withdrawals while loans are active.
    env.warp_seconds(61);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    assert_hodl_error(env.sponsored(withdraw(1, price_triples(&[usdc])), key), HodlError::StalePrice);
}

#[test]
fn triples_cover_the_slots_left_after_withdrawal() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    let sol = env.list_spl_collateral(9);
    env.set_pyth_price(&sol, 150 * ONE_DOLLAR, 0);
    env.deposit_collateral(&setup.borrower, &sol, 2_000_000_000);
    env.take_loan(&setup.borrower, &setup, 100_000 * ONE_CNGN, 365 * DAY).unwrap();
    let token = env.create_token_account(&sol, &owner);
    let key = &setup.borrower.key;

    // Emptying the SOL slot leaves only USDC to price.
    let with_sol = withdraw_collateral_ix(&owner, &sol, &SPL_TOKEN, &token, Some(&setup.cngn), 2_000_000_000, price_triples(&[setup.usdc, sol]));
    assert_hodl_error(env.sponsored(with_sol, key), HodlError::PriceAccountMismatch);
    let usdc_only = withdraw_collateral_ix(&owner, &sol, &SPL_TOKEN, &token, Some(&setup.cngn), 2_000_000_000, price_triples(&[setup.usdc]));
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
    let wrong_market = withdraw_collateral_ix(&owner, &setup.usdc, &SPL_TOKEN, &token, Some(&other), ONE_USDC, price_triples(&[setup.usdc]));
    assert_hodl_error(env.sponsored(wrong_market, &setup.borrower.key), HodlError::MarketMismatch);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test withdraw`
Expected: compile errors `cannot find struct, variant or union type WithdrawCollateral in module hodl_loans::instruction` and `… in module hodl_loans::accounts`.

- [ ] **Step 3: Implement**

Append to `programs/hodl_loans/src/events.rs`:

```rust
#[event]
pub struct CollateralWithdrawn {
    pub position: Pubkey,
    pub owner: Pubkey,
    pub mint: Pubkey,
    pub amount: u64,
    pub slot_amount: u64,
}
```

`programs/hodl_loans/src/instructions/positions/withdraw_collateral.rs`:

```rust
use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{ACCESS_SEED, COLLATERAL_SEED, POSITION_SEED};
use crate::errors::HodlError;
use crate::events::CollateralWithdrawn;
use crate::state::{Access, CollateralAsset, Market, Position};
use crate::token::transfer::transfer_from_vault;
use crate::valuation::load_health;

#[derive(Accounts)]
pub struct WithdrawCollateral<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [ACCESS_SEED, owner.key().as_ref()], bump = access.bump)]
    pub access: Account<'info, Access>,
    #[account(mut, seeds = [POSITION_SEED, owner.key().as_ref()], bump)]
    pub position: AccountLoader<'info, Position>,
    #[account(mut, seeds = [COLLATERAL_SEED, mint.key().as_ref()], bump = collateral.bump, has_one = mint, has_one = vault)]
    pub collateral: Box<Account<'info, CollateralAsset>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, token::mint = mint, token::authority = owner, token::token_program = token_program)]
    pub owner_token: Box<InterfaceAccount<'info, TokenAccount>>,
    /// The market the position borrows from. Required only while loans are active.
    pub market: Option<Box<Account<'info, Market>>>,
    /// CHECK: required only while loans are active; `read_ngn_price` pins it to `market.ngn_feed`.
    pub ngn_feed: Option<UncheckedAccount<'info>>,
    pub token_program: Interface<'info, TokenInterface>,
}

/// Without active loans no prices are read, and `market` and `ngn_feed` may be omitted.
/// With active loans, `market` and `ngn_feed` are required, `remaining_accounts` must hold one
/// `(CollateralAsset, PriceUpdateV2, mint)` triple per collateral slot still used **after**
/// this withdrawal, in slot order, and the position must stay healthy.
pub fn handle_withdraw_collateral<'info>(ctx: Context<'info, WithdrawCollateral<'info>>, amount: u64) -> Result<()> {
    ctx.accounts.access.require_active()?;
    require!(amount > 0, HodlError::AmountTooSmall);
    let mint_key = ctx.accounts.mint.key();

    let slot_amount = {
        let mut position = ctx.accounts.position.load_mut()?;
        let index = position.collateral_index(&mint_key).ok_or(HodlError::InsufficientCollateral)?;
        let slot = &mut position.collateral[index];
        require!(slot.amount >= amount, HodlError::InsufficientCollateral);
        slot.amount -= amount;
        let slot_amount = slot.amount;

        if position.has_active_loans() {
            let (Some(market), Some(ngn_feed)) = (&ctx.accounts.market, &ctx.accounts.ngn_feed) else {
                return err!(HodlError::PriceAccountMismatch);
            };
            require_keys_eq!(position.market, market.key(), HodlError::MarketMismatch);
            let health = load_health(
                ctx.program_id,
                &position,
                market,
                &ngn_feed.to_account_info(),
                ctx.remaining_accounts,
                0,
                &Clock::get()?,
            )?;
            require!(health.is_healthy(), HodlError::Unhealthy);
        }
        slot_amount
    };

    let collateral = &mut ctx.accounts.collateral;
    collateral.total_deposited = collateral.total_deposited.checked_sub(amount).ok_or(HodlError::MathOverflow)?;

    let seeds: &[&[u8]] = &[COLLATERAL_SEED, mint_key.as_ref(), &[ctx.accounts.collateral.bump]];
    transfer_from_vault(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.owner_token.to_account_info(),
        ctx.accounts.collateral.to_account_info(),
        amount,
        &[seeds],
    )?;
    emit!(CollateralWithdrawn {
        position: ctx.accounts.position.key(),
        owner: ctx.accounts.owner.key(),
        mint: mint_key,
        amount,
        slot_amount,
    });
    Ok(())
}
```

Replace `programs/hodl_loans/src/instructions/positions/mod.rs`:

```rust
pub mod close_position;
pub mod deposit_collateral;
pub mod open_position;
pub mod withdraw_collateral;

pub use close_position::*;
pub use deposit_collateral::*;
pub use open_position::*;
pub use withdraw_collateral::*;
```

In `lib.rs`, add inside `pub mod hodl_loans { … }` after `deposit_collateral`:

```rust
    pub fn withdraw_collateral<'info>(ctx: Context<'info, WithdrawCollateral<'info>>, amount: u64) -> Result<()> {
        instructions::handle_withdraw_collateral(ctx, amount)
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh --test withdraw`
Expected: `test result: ok. 5 passed`.

Run: `./scripts/test.sh`
Expected: every binary reports `ok`.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: collateral withdrawal with health check while loans are open"
```

---

### Task 9: Reserve harvest and full-suite check

**Files:**
- Create: `programs/hodl_loans/src/instructions/admin/reserve.rs`
- Modify: `src/events.rs`, `src/instructions/admin/mod.rs`, `src/lib.rs`, `tests/common/mod.rs` (under `programs/hodl_loans/`)
- Test: `programs/hodl_loans/tests/reserve.rs`

**Interfaces:**
- Consumes: `Market.protocol_reserve`, `Config.treasury`, `transfer_from_vault`; harness `loan_ready`, `Env::{take_loan, repay}` (Tasks 6–7).
- Produces:
  - Event `ReserveHarvested`
  - Instruction `harvest_reserve(amount: u64)` (admin): `admin, config, market, mint, vault, destination, token_program`. The destination must be owned by `config.treasury`.
  - Harness: `harvest_reserve_ix(admin, mint, destination, amount)`

- [ ] **Step 1: Write the failing tests**

Append to `programs/hodl_loans/tests/common/mod.rs`:

```rust
// ---- Reserve harvest (Task 9) ----

pub fn harvest_reserve_ix(admin: &Pubkey, mint: &Pubkey, destination: &Pubkey, amount: u64) -> Instruction {
    ix(
        hodl_loans::instruction::HarvestReserve { amount },
        hodl_loans::accounts::HarvestReserve {
            admin: *admin,
            config: config_pda(),
            market: market_pda(mint),
            mint: *mint,
            vault: market_vault_pda(mint),
            destination: *destination,
            token_program: TOKEN_2022,
        },
    )
}
```

`programs/hodl_loans/tests/reserve.rs`:

```rust
mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

const DAY: i64 = 86_400;
const RESERVE: u64 = 300 * ONE_CNGN;

/// A 100,000 cNGN loan repaid after 73 days: 3,000 cNGN interest, 300 of it to the reserve.
fn repaid_market() -> (Env, LoanSetup) {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, 100_000 * ONE_CNGN, 365 * DAY).unwrap();
    env.warp_seconds(73 * DAY);
    env.mint_to(&setup.cngn, &setup.borrower_cngn, 3_000 * ONE_CNGN);
    env.repay(&setup, 0, u64::MAX).unwrap();
    assert_eq!(env.market(&setup.cngn).protocol_reserve, RESERVE);
    (env, setup)
}

#[test]
fn admin_harvests_the_reserve_to_the_treasury() {
    let (mut env, setup) = repaid_market();
    let admin = env.admin.pubkey();
    let treasury = env.treasury.pubkey();
    let destination = env.create_token_account(&setup.cngn, &treasury);
    let cash = env.market(&setup.cngn).cash;

    let harvest = harvest_reserve_ix(&admin, &setup.cngn, &destination, 100 * ONE_CNGN);
    send(&mut env.svm, &[harvest], &[&env.admin]).unwrap();
    assert_eq!(env.token_balance(&destination), 100 * ONE_CNGN);
    let market = env.market(&setup.cngn);
    assert_eq!((market.protocol_reserve, market.cash), (200 * ONE_CNGN, cash - 100 * ONE_CNGN));

    let too_much = harvest_reserve_ix(&admin, &setup.cngn, &destination, 200 * ONE_CNGN + 1);
    assert_hodl_error(send(&mut env.svm, &[too_much], &[&env.admin]), HodlError::InsufficientCash);
    let zero = harvest_reserve_ix(&admin, &setup.cngn, &destination, 0);
    assert_hodl_error(send(&mut env.svm, &[zero], &[&env.admin]), HodlError::AmountTooSmall);

    let stranger = env.funded_keypair();
    let by_stranger = harvest_reserve_ix(&stranger.pubkey(), &setup.cngn, &destination, ONE_CNGN);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);
    let stranger_account = env.create_token_account(&setup.cngn, &stranger.pubkey());
    let to_stranger = harvest_reserve_ix(&admin, &setup.cngn, &stranger_account, ONE_CNGN);
    assert_anchor_error(send(&mut env.svm, &[to_stranger], &[&env.admin]), AnchorError::ConstraintTokenOwner);

    let rest = harvest_reserve_ix(&admin, &setup.cngn, &destination, 200 * ONE_CNGN);
    send(&mut env.svm, &[rest], &[&env.admin]).unwrap();
    assert_eq!(env.market(&setup.cngn).protocol_reserve, 0);
}

#[test]
fn lenders_and_the_reserve_are_both_paid_in_full() {
    let (mut env, setup) = repaid_market();
    let vault = market_vault_pda(&setup.cngn);

    // The lender's full exit leaves the reserve (plus share rounding) in the vault.
    env.withdraw(&setup.lender, &setup.cngn, u64::MAX).unwrap();
    assert!(env.token_balance(&setup.lender.token) >= POOL_CNGN + 2_700 * ONE_CNGN - 1);
    let market = env.market(&setup.cngn);
    assert!(market.cash >= market.protocol_reserve);
    assert_eq!(env.token_balance(&vault), market.cash);

    let treasury = env.treasury.pubkey();
    let destination = env.create_token_account(&setup.cngn, &treasury);
    let harvest = harvest_reserve_ix(&env.admin.pubkey(), &setup.cngn, &destination, RESERVE);
    send(&mut env.svm, &[harvest], &[&env.admin]).unwrap();
    assert_eq!(env.token_balance(&destination), RESERVE);
    assert!(env.token_balance(&vault) <= 1);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test reserve`
Expected: compile errors `cannot find struct, variant or union type HarvestReserve in module hodl_loans::instruction` and `… in module hodl_loans::accounts`.

- [ ] **Step 3: Implement**

Append to `programs/hodl_loans/src/events.rs`:

```rust
#[event]
pub struct ReserveHarvested {
    pub market: Pubkey,
    pub destination: Pubkey,
    pub amount: u64,
    pub old_reserve: u64,
    pub new_reserve: u64,
}
```

`programs/hodl_loans/src/instructions/admin/reserve.rs`:

```rust
use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{CONFIG_SEED, MARKET_SEED};
use crate::errors::HodlError;
use crate::events::ReserveHarvested;
use crate::state::{Config, Market};
use crate::token::transfer::transfer_from_vault;

#[derive(Accounts)]
pub struct HarvestReserve<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(mut, seeds = [MARKET_SEED, mint.key().as_ref()], bump = market.bump, has_one = mint, has_one = vault)]
    pub market: Box<Account<'info, Market>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = mint,
        token::authority = config.treasury,
        token::token_program = token_program
    )]
    pub destination: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
}

/// Sends `amount` of the protocol reserve to the treasury. `cash` and `protocol_reserve` both fall.
pub fn handle_harvest_reserve(ctx: Context<HarvestReserve>, amount: u64) -> Result<()> {
    require!(amount > 0, HodlError::AmountTooSmall);
    let market_key = ctx.accounts.market.key();
    let market = &mut ctx.accounts.market;
    require!(amount <= market.protocol_reserve, HodlError::InsufficientCash);
    let old_reserve = market.protocol_reserve;
    market.protocol_reserve -= amount;
    market.cash = market.cash.checked_sub(amount).ok_or(HodlError::MathOverflow)?;
    let new_reserve = market.protocol_reserve;

    let mint_key = ctx.accounts.mint.key();
    let seeds: &[&[u8]] = &[MARKET_SEED, mint_key.as_ref(), &[ctx.accounts.market.bump]];
    transfer_from_vault(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.destination.to_account_info(),
        ctx.accounts.market.to_account_info(),
        amount,
        &[seeds],
    )?;
    emit!(ReserveHarvested {
        market: market_key,
        destination: ctx.accounts.destination.key(),
        amount,
        old_reserve,
        new_reserve,
    });
    Ok(())
}
```

Replace `programs/hodl_loans/src/instructions/admin/mod.rs`:

```rust
pub mod access_control;
pub mod admin_transfer;
pub mod collateral_admin;
pub mod initialize;
pub mod market_admin;
pub mod reserve;
pub mod roles;
pub mod sweep;

pub use access_control::*;
pub use admin_transfer::*;
pub use collateral_admin::*;
pub use initialize::*;
pub use market_admin::*;
pub use reserve::*;
pub use roles::*;
pub use sweep::*;
```

In `lib.rs`, add inside `pub mod hodl_loans { … }` after `sweep_collateral_excess`:

```rust
    pub fn harvest_reserve(ctx: Context<HarvestReserve>, amount: u64) -> Result<()> {
        instructions::handle_harvest_reserve(ctx, amount)
    }
```

- [ ] **Step 4: Run the reserve tests, then the full suite and lints**

Run: `./scripts/test.sh --test reserve`
Expected: `test result: ok. 2 passed`.

Run: `./scripts/test.sh`
Expected: every test binary reports `ok`, 117 tests in all:

| Binary | Tests |
|---|---|
| unit | 32 |
| harness | 3 |
| admin | 6 |
| access | 6 |
| market | 8 |
| liquidity | 20 |
| sweep | 3 |
| collateral | 9 |
| position | 6 |
| loans | 11 |
| repay | 6 |
| withdraw | 5 |
| reserve | 2 |

Run: `cargo clippy -p hodl_loans --all-targets -- -D warnings`
Expected: finishes without errors. If clippy flags an issue, fix it in the file it names and re-run both commands.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: harvest the protocol reserve to the treasury"
```
