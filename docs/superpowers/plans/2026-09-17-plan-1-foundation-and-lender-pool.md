# Plan 1: Foundation and Lender Pool — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the `hodl_loans` Anchor program with admin roles, the whitelist and blacklist, a cNGN market, and a working lender pool (deposit, withdraw, donation sweep), fully tested in LiteSVM.

**Architecture:**
- **One Anchor 1.2 program** in a new Cargo workspace. Instruction logic lives in `instructions/`, account layouts in `state/`, pure arithmetic in `math/`, and token helpers in `token/`.
- **Tests.** Every behaviour is tested against the compiled SBF program in LiteSVM through one shared harness (`tests/common/mod.rs`). Math is unit-tested on the host.
- **Borrowing comes later.** Loans, collateral and prices arrive in Plan 2. This plan simulates loan effects in tests by writing `Market` fields directly.

**Tech Stack:** Rust 1.89+, Anchor 1.2.0 (`anchor-lang`, `anchor-spl`), Solana CLI 3.1.x (`cargo build-sbf`, platform tools v1.52), LiteSVM 0.10.0, `spl-token-2022-interface` 2.

**Spec:** `docs/superpowers/specs/2026-09-17-solana-fixed-loans-design.md`

## Plan series

The spec is split into six plans. Each one produces a working, tested program. Later plans are written after the previous one lands, so they can reference the real code.

| Plan | Delivers |
|---|---|
| **1. Foundation and lender pool** (this plan) | Workspace, math, roles, whitelist/blacklist, cNGN market, lender deposit/withdraw, donation sweep |
| 2. Collateral, prices and loans | `CollateralAsset` (standard mints), positions, Pyth and Switchboard reads, health checks, `take_loan`, `repay_loan`, reserve harvest |
| 3. Liquidation and bad debt | `liquidate`, `write_off_loan` (reserve first) |
| 4. xStocks | Token-2022 extension policy, scaled-UI multiplier, transfer-time re-checks |
| 5. Promo balance | Promo vault, campaigns, Ed25519 vouchers, forfeiture, expiry, `set_promo_cap` |
| 6. Hardening and devnet | Trident invariant fuzzing, ported EVM regressions, devnet run, pre-audit scan |

## Global Constraints

- Anchor `1.2.0` for `anchor-lang` and `anchor-spl`; LiteSVM `0.10.0`; Rust `1.89` or newer; build with `cargo build-sbf --tools-version v1.52` (through `scripts/test.sh`).
- `[profile.release] overflow-checks = true`; all math on `u128` with checked operations.
- Rounding: borrower debt rounds up; lender accrual and minted shares round down; burned shares round up.
- Constants: `BPS = 10_000`, `YEAR = 31_536_000` seconds, `VIRTUAL_SHARES = 1_000`, `VIRTUAL_ASSETS = 1`, `MIN_TENURE = 86_400`, `MAX_COLLATERAL_SLOTS = 8`, `MAX_LOAN_SLOTS = 10`.
- Every account starts with `version: u8` and `bump: u8` and ends with reserved padding.
- In user-facing instructions, wrap `Market`, `LenderPosition` and every `InterfaceAccount` in `Box<…>`. The SBF stack frame is 4 KB; without boxing, `deposit_liquidity` fails with `Access violation in stack frame` (reproduced while writing this plan).
- Seeds: `Config ["config"]`, `Access ["access", wallet]`, `Market ["market", mint]`, `LenderPosition ["lender", market, owner]`.
- One `#[error_code] HodlError` enum. **Append new variants only**, since codes are `6000 + position`.
- The guardian can pause but never unpause; only the admin unpauses.
- `cash` changes only through program instructions. Tokens sent straight to a vault never change the share price.
- Market mints may carry only `MetadataPointer`, `TokenMetadata`, `PermanentDelegate`.
- Commits made by Claude end with the attribution trailer from the session instructions.

## Facts verified while writing this plan (2026-09-17)

- **cNGN on Solana** (`3jiqwBQVRC5zRwHyqvnkQurebJ5RNxg3F5fXMwaxgkv8`, mainnet RPC):
  - Token-2022, 6 decimals, freeze authority set.
  - Extensions: `permanentDelegate`, `metadataPointer`, `tokenMetadata`.
  - Accepted issuer risks: the delegate can move vault funds, and the freeze authority can freeze the vault.
  - This resolves spec §20 item 1.
- **Anchor 1.2** (read from the v1.2.0 source):
  - `CpiContext::new(program_id: Pubkey, accounts)` takes a program ID, not an `AccountInfo`.
  - `anchor_spl::token_2022::spl_token_2022` re-exports `spl-token-2022-interface`.
  - `init_if_needed` requires the `init-if-needed` feature.
  - The official LiteSVM template uses `litesvm 0.10.0` with `solana-* 3.x` crates.
- **LiteSVM `add_program`** deploys through the upgradeable loader. Tests set the upgrade authority by rewriting the 45-byte ProgramData header.
- **Build toolchain.** Anchor 1.2's own CLI builds with `--arch v3` and platform tools v1.57. `cargo build-sbf` from Solana CLI 3.1.15 accepts only `sbfv1`/`sbfv2` and fails with v1.57 (`can't find crate for core`). This plan calls `cargo build-sbf --tools-version v1.52` directly, and the Anchor CLI isn't needed. Revisit when Plan 6 upgrades the Solana CLI for deployment.
- **The full Plan 1 code was compiled and tested before this plan was written:** 46 tests pass, `cargo clippy -D warnings` is clean, and every intermediate task state compiles.

## Plan-level refinements to the spec

- The market's cNGN vault uses seeds `["market_vault", mint]`; the spec leaves vault seeds open.
- The spec's `sweep_excess` becomes one instruction per vault type. This plan adds `sweep_market_excess`; Plans 2 and 5 add collateral and promo sweeps.
- `deposit_liquidity` takes a separate `payer` signer, so the gas-relay sponsor can pay lender-account rent for wallets holding no SOL.
- The upgrade authority becomes the first admin. `accept_admin` does not move the upgrade authority; that is a separate Squads action.

## File Structure

```text
lendbit-solana/
  Cargo.toml                         workspace + release profile
  README.md                          build/test instructions
  .gitignore
  scripts/test.sh                    cargo build-sbf, then cargo test
  programs/hodl_loans/
    Cargo.toml
    src/lib.rs                       declare_id!, module wiring, #[program] entry points
    src/constants.rs                 numeric constants and PDA seeds
    src/errors.rs                    HodlError (append-only)
    src/events.rs                    #[event] structs and Role
    src/math/mod.rs
    src/math/checked.rs              mul_div_floor/ceil, checked add/sub, to_u64
    src/math/interest.rs             lp_interest
    src/math/shares.rs               shares_for_deposit, shares_to_burn, redeemable_amount
    src/state/mod.rs
    src/state/config.rs              Config
    src/state/access.rs              Access (+ require_active)
    src/state/market.rs              Market, MarketParams, accrue, total_assets
    src/state/lender.rs              LenderPosition
    src/token/mod.rs
    src/token/extensions.rs          mint extension allowlists
    src/token/transfer.rs            transfer_from_user, transfer_from_vault
    src/instructions/mod.rs
    src/instructions/admin/mod.rs
    src/instructions/admin/initialize.rs
    src/instructions/admin/admin_transfer.rs   AdminConfig, propose/accept admin
    src/instructions/admin/roles.rs            set_guardian/whitelister/promo_signer/treasury
    src/instructions/admin/access_control.rs   whitelist, blacklist, unblacklist
    src/instructions/admin/market_admin.rs     create_market, update_market_params, set_market_paused
    src/instructions/admin/sweep.rs            sweep_market_excess
    src/instructions/liquidity/mod.rs
    src/instructions/liquidity/deposit_liquidity.rs
    src/instructions/liquidity/withdraw_liquidity.rs
    tests/common/mod.rs              LiteSVM harness, grows one section per task
    tests/harness.rs
    tests/admin.rs
    tests/access.rs
    tests/market.rs
    tests/liquidity.rs
    tests/sweep.rs
```

---

### Task 1: Workspace scaffold and build pipeline

**Files:**
- Create: `Cargo.toml`, `README.md`, `.gitignore`, `scripts/test.sh`
- Create: `programs/hodl_loans/Cargo.toml`, `programs/hodl_loans/src/lib.rs`, `programs/hodl_loans/src/constants.rs`, `programs/hodl_loans/src/errors.rs`

**Interfaces:**
- Produces: crate `hodl_loans`; constants `BPS: u128`, `MAX_BPS: u16`, `YEAR: u128`, `VIRTUAL_SHARES: u128`, `VIRTUAL_ASSETS: u128`, `MIN_TENURE: i64`, `MAX_COLLATERAL_SLOTS: usize`, `MAX_LOAN_SLOTS: usize`, `ACCOUNT_VERSION: u8`, `DEFAULT_PROMO_CAP_BPS: u16`, seeds `CONFIG_SEED`, `ACCESS_SEED`, `MARKET_SEED`, `MARKET_VAULT_SEED`, `LENDER_SEED` (all `&[u8]`); enum `HodlError` (36 variants, spec §16 order); `./scripts/test.sh [cargo test args]`.

- [ ] **Step 1: Confirm the toolchain**

Run: `rustc --version && solana --version && cargo build-sbf --version`
Expected: rustc 1.89 or newer; `solana-cli 3.1.x`; `solana-cargo-build-sbf 3.1.x`. If `solana` is missing, install the Agave CLI 3.1.x first.

- [ ] **Step 2: Create the workspace files**

`Cargo.toml`:

```toml
[workspace]
members = ["programs/*"]
resolver = "2"

[workspace.package]
edition = "2021"
rust-version = "1.89"

[profile.release]
overflow-checks = true
lto = "fat"
codegen-units = 1

[profile.release.build-override]
opt-level = 3
incremental = false
codegen-units = 1
```

`programs/hodl_loans/Cargo.toml`:

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

[dev-dependencies]
litesvm = "0.10.0"
solana-keypair = "3.0.1"
solana-message = "3.0.1"
solana-signer = "3.0.0"
solana-transaction = "3.0.2"
spl-token-2022-interface = "2"
spl-token-interface = "2"

[lints.rust]
unexpected_cfgs = { level = "warn", check-cfg = ['cfg(target_os, values("solana"))'] }
```

`scripts/test.sh` (then `chmod +x scripts/test.sh`):

```bash
#!/usr/bin/env bash
# Build the SBF program, then run unit and LiteSVM integration tests.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build-sbf --tools-version v1.52
cargo test -p hodl_loans "$@"
```

`.gitignore`:

```text
target/
.DS_Store
*.log
```

`README.md`:

````markdown
# lendbit-solana

HODL fixed-term cNGN loans on Solana: an Anchor program (`hodl_loans`) with a lender pool,
collateral positions, fixed-term loans, liquidation and promo balances.

Design: `docs/superpowers/specs/2026-09-17-solana-fixed-loans-design.md`

## Prerequisites

- Rust 1.89 or newer
- Solana CLI 3.1.x (Agave) — provides `cargo build-sbf`; builds use platform tools v1.52 (downloaded on first build if missing)

## Build and test

```bash
./scripts/test.sh            # cargo build-sbf, then unit + LiteSVM integration tests
./scripts/test.sh --lib      # unit tests only (still builds the program first)
```

Integration tests load `target/deploy/hodl_loans.so`, so the program must be built before `cargo test`.

## Program ID

`target/deploy/hodl_loans-keypair.json` holds the program keypair and is not committed.
Back it up before any devnet or mainnet deploy: losing it changes the program ID.
````

- [ ] **Step 3: Generate the program keypair**

Run: `mkdir -p target/deploy && solana-keygen new --no-bip39-passphrase --silent -o target/deploy/hodl_loans-keypair.json && solana-keygen pubkey target/deploy/hodl_loans-keypair.json`
Expected: prints a base58 public key. Use it in place of `REPLACE_WITH_PROGRAM_ID` in the next step. `target/` is git-ignored; back the keypair up outside the repo.

- [ ] **Step 4: Write `lib.rs`, `constants.rs` (with its unit test) and `errors.rs`**

`programs/hodl_loans/src/lib.rs`:

```rust
use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;

pub use constants::*;
pub use errors::*;

declare_id!("REPLACE_WITH_PROGRAM_ID");

#[program]
pub mod hodl_loans {}
```

`programs/hodl_loans/src/constants.rs`:

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_are_distinct_and_bps_matches() {
        let seeds = [CONFIG_SEED, ACCESS_SEED, MARKET_SEED, MARKET_VAULT_SEED, LENDER_SEED];
        for (i, a) in seeds.iter().enumerate() {
            for b in &seeds[i + 1..] {
                assert_ne!(a, b);
            }
        }
        assert_eq!(BPS, MAX_BPS as u128);
    }
}
```

`programs/hodl_loans/src/errors.rs`:

```rust
use anchor_lang::prelude::*;

/// Program errors. Append new variants at the end only: codes are 6000 + position.
#[error_code]
pub enum HodlError {
    #[msg("Wallet is not whitelisted")]
    NotWhitelisted,
    #[msg("Wallet is blacklisted")]
    Blacklisted,
    #[msg("Signer is not authorized for this action")]
    Unauthorized,
    #[msg("Market is paused")]
    MarketPaused,
    #[msg("Collateral asset is paused")]
    CollateralPaused,
    #[msg("Price is stale")]
    StalePrice,
    #[msg("Price confidence interval is too wide")]
    PriceConfidenceTooWide,
    #[msg("Price account does not match the expected asset")]
    PriceAccountMismatch,
    #[msg("Price is invalid")]
    InvalidPrice,
    #[msg("Mint has an unsupported extension")]
    UnsupportedMintExtension,
    #[msg("Parameters are invalid")]
    InvalidParameters,
    #[msg("Position would be unhealthy")]
    Unhealthy,
    #[msg("Position is not liquidatable")]
    NotLiquidatable,
    #[msg("Utilization cap exceeded")]
    UtilizationCapExceeded,
    #[msg("Not enough cash in the market")]
    InsufficientCash,
    #[msg("No free loan slot")]
    NoFreeLoanSlot,
    #[msg("No free collateral slot")]
    NoFreeCollateralSlot,
    #[msg("Loan not found")]
    LoanNotFound,
    #[msg("Deposit cap exceeded")]
    DepositCapExceeded,
    #[msg("Amount is too small")]
    AmountTooSmall,
    #[msg("Tenure is out of range")]
    TenureOutOfRange,
    #[msg("Repayment does not cover interest and penalty")]
    RepaymentBelowInterest,
    #[msg("Liquidation would repay zero principal")]
    ZeroPrincipalRepaid,
    #[msg("Deposit would mint zero shares")]
    ZeroShares,
    #[msg("Not enough shares")]
    InsufficientShares,
    #[msg("Collateral is still in use")]
    CollateralStillInUse,
    #[msg("Position is not empty")]
    PositionNotEmpty,
    #[msg("Write-off is not allowed")]
    WriteOffNotAllowed,
    #[msg("Voucher signature is invalid")]
    InvalidVoucherSignature,
    #[msg("Voucher has expired")]
    VoucherExpired,
    #[msg("Campaign is inactive")]
    CampaignInactive,
    #[msg("Campaign budget exceeded")]
    CampaignBudgetExceeded,
    #[msg("Promo cap exceeded")]
    PromoCapExceeded,
    #[msg("Promo has not expired")]
    PromoNotExpired,
    #[msg("Promo vault has insufficient free funds")]
    PromoVaultInsufficient,
    #[msg("Math overflow")]
    MathOverflow,
}
```

- [ ] **Step 5: Build and run the unit test**

Run: `./scripts/test.sh --lib`
Expected: the build writes `target/deploy/hodl_loans.so` (the first run may download platform tools v1.52) and the output ends with `test tests::seeds_are_distinct_and_bps_matches ... ok` and `test result: ok. 1 passed`.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "chore: scaffold hodl_loans Anchor 1.2 workspace"
```

---

### Task 2: Checked math, lender shares and interest accrual

**Files:**
- Create: `programs/hodl_loans/src/math/mod.rs`, `programs/hodl_loans/src/math/checked.rs`, `programs/hodl_loans/src/math/interest.rs`, `programs/hodl_loans/src/math/shares.rs`
- Modify: `programs/hodl_loans/src/lib.rs` (add `pub mod math;`)

**Interfaces:**
- Consumes: `BPS`, `YEAR`, `VIRTUAL_SHARES`, `VIRTUAL_ASSETS`, `HodlError::MathOverflow` (Task 1).
- Produces:
  - `math::checked::{mul_div_floor(a: u128, b: u128, d: u128) -> Result<u128>, mul_div_ceil(..) -> Result<u128>, add(a: u128, b: u128) -> Result<u128>, sub(a: u128, b: u128) -> Result<u128>, to_u64(v: u128) -> Result<u64>}`
  - `math::interest::lp_interest(lp_rate_product: u128, elapsed_seconds: u64) -> Result<u128>`
  - `math::shares::{shares_for_deposit(amount: u64, total_shares: u128, total_assets: u128) -> Result<u128>, shares_to_burn(..) -> Result<u128>, redeemable_amount(shares: u128, total_shares: u128, total_assets: u128) -> Result<u128>}`

- [ ] **Step 1: Wire the module and write the failing tests**

In `lib.rs`, add `pub mod math;` directly below `pub mod errors;`.

`programs/hodl_loans/src/math/mod.rs`:

```rust
pub mod checked;
pub mod interest;
pub mod shares;
```

Create each math file containing only its imports and test module for now.

`programs/hodl_loans/src/math/checked.rs`:

```rust
use anchor_lang::prelude::*;

use crate::errors::HodlError;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_and_ceil_differ_only_on_remainder() {
        assert_eq!(mul_div_floor(10, 3, 4).unwrap(), 7);
        assert_eq!(mul_div_ceil(10, 3, 4).unwrap(), 8);
        assert_eq!(mul_div_floor(8, 3, 4).unwrap(), 6);
        assert_eq!(mul_div_ceil(8, 3, 4).unwrap(), 6);
    }

    #[test]
    fn zero_denominator_and_overflow_fail() {
        assert!(mul_div_floor(1, 1, 0).is_err());
        assert!(mul_div_ceil(u128::MAX, 2, 1).is_err());
        assert!(sub(1, 2).is_err());
        assert!(to_u64(u64::MAX as u128 + 1).is_err());
    }
}
```

`programs/hodl_loans/src/math/interest.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{BPS, YEAR};
use crate::math::checked::mul_div_floor;

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
    fn rounds_down_and_is_zero_without_loans() {
        assert_eq!(lp_interest(0, 31_536_000).unwrap(), 0);
        // One second of 1 unit at 1 bps with no reserve is far below one base unit.
        assert_eq!(lp_interest(10_000, 1).unwrap(), 0);
    }
}
```

`programs/hodl_loans/src/math/shares.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{VIRTUAL_ASSETS, VIRTUAL_SHARES};
use crate::math::checked::{add, mul_div_ceil, mul_div_floor};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_deposit_uses_virtual_offset() {
        assert_eq!(shares_for_deposit(1_000_000, 0, 0).unwrap(), 1_000_000_000);
    }

    #[test]
    fn later_deposits_are_proportional() {
        assert_eq!(shares_for_deposit(500_000, 1_000_000_000, 1_000_000).unwrap(), 500_000_000);
        assert_eq!(
            shares_for_deposit(1_100_000, 1_000_000_000, 1_100_000).unwrap(),
            1_000_000_090
        );
    }

    #[test]
    fn redeem_matches_deposit_when_exact() {
        assert_eq!(redeemable_amount(1_000_000_000, 1_000_000_000, 1_000_000).unwrap(), 1_000_000);
    }

    #[test]
    fn burn_rounds_up_mint_rounds_down() {
        let (s, a) = (1_000_000_007u128, 1_000_003u128);
        for amount in [1u64, 7, 999, 123_456] {
            assert!(shares_to_burn(amount, s, a).unwrap() >= shares_for_deposit(amount, s, a).unwrap());
        }
    }

    #[test]
    fn deposit_then_redeem_never_returns_more() {
        let cases = [(0u128, 0u128), (1_000_000_000, 1_000_000), (3_333_333_333, 1_234_567), (7, 1_000_000_000)];
        for (total_shares, total_assets) in cases {
            for amount in [1u64, 2, 999, 1_000_000, 987_654_321] {
                let shares = shares_for_deposit(amount, total_shares, total_assets).unwrap();
                let back = redeemable_amount(shares, total_shares + shares, total_assets + amount as u128).unwrap();
                assert!(back <= amount as u128, "amount {amount} returned {back}");
            }
        }
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p hodl_loans --lib`
Expected: compile errors such as `cannot find function mul_div_floor in this scope` and `cannot find function shares_for_deposit`.

- [ ] **Step 3: Implement**

Replace each file with the full version below. It is the same imports and tests with the functions added.

`programs/hodl_loans/src/math/checked.rs`:

```rust
use anchor_lang::prelude::*;

use crate::errors::HodlError;

/// `a × b / denominator`, rounded down.
pub fn mul_div_floor(a: u128, b: u128, denominator: u128) -> Result<u128> {
    require!(denominator != 0, HodlError::MathOverflow);
    let product = a.checked_mul(b).ok_or(HodlError::MathOverflow)?;
    Ok(product / denominator)
}

/// `a × b / denominator`, rounded up.
pub fn mul_div_ceil(a: u128, b: u128, denominator: u128) -> Result<u128> {
    require!(denominator != 0, HodlError::MathOverflow);
    let product = a.checked_mul(b).ok_or(HodlError::MathOverflow)?;
    Ok(product.div_ceil(denominator))
}

pub fn add(a: u128, b: u128) -> Result<u128> {
    Ok(a.checked_add(b).ok_or(HodlError::MathOverflow)?)
}

pub fn sub(a: u128, b: u128) -> Result<u128> {
    Ok(a.checked_sub(b).ok_or(HodlError::MathOverflow)?)
}

pub fn to_u64(value: u128) -> Result<u64> {
    Ok(u64::try_from(value).map_err(|_| HodlError::MathOverflow)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_and_ceil_differ_only_on_remainder() {
        assert_eq!(mul_div_floor(10, 3, 4).unwrap(), 7);
        assert_eq!(mul_div_ceil(10, 3, 4).unwrap(), 8);
        assert_eq!(mul_div_floor(8, 3, 4).unwrap(), 6);
        assert_eq!(mul_div_ceil(8, 3, 4).unwrap(), 6);
    }

    #[test]
    fn zero_denominator_and_overflow_fail() {
        assert!(mul_div_floor(1, 1, 0).is_err());
        assert!(mul_div_ceil(u128::MAX, 2, 1).is_err());
        assert!(sub(1, 2).is_err());
        assert!(to_u64(u64::MAX as u128 + 1).is_err());
    }
}
```

`programs/hodl_loans/src/math/interest.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{BPS, YEAR};
use crate::math::checked::mul_div_floor;

/// Lender interest accrued over `elapsed_seconds` for the market's `lp_rate_product`
/// (Σ principal × rate_bps × (BPS − reserve_factor_bps)). Rounds down.
pub fn lp_interest(lp_rate_product: u128, elapsed_seconds: u64) -> Result<u128> {
    mul_div_floor(lp_rate_product, elapsed_seconds as u128, BPS * BPS * YEAR)
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
    fn rounds_down_and_is_zero_without_loans() {
        assert_eq!(lp_interest(0, 31_536_000).unwrap(), 0);
        // One second of 1 unit at 1 bps with no reserve is far below one base unit.
        assert_eq!(lp_interest(10_000, 1).unwrap(), 0);
    }
}
```

`programs/hodl_loans/src/math/shares.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{VIRTUAL_ASSETS, VIRTUAL_SHARES};
use crate::math::checked::{add, mul_div_ceil, mul_div_floor};

/// Shares minted for depositing `amount`. Rounds down.
pub fn shares_for_deposit(amount: u64, total_shares: u128, total_assets: u128) -> Result<u128> {
    mul_div_floor(
        amount as u128,
        add(total_shares, VIRTUAL_SHARES)?,
        add(total_assets, VIRTUAL_ASSETS)?,
    )
}

/// Shares burned for withdrawing `amount`. Rounds up.
pub fn shares_to_burn(amount: u64, total_shares: u128, total_assets: u128) -> Result<u128> {
    mul_div_ceil(
        amount as u128,
        add(total_shares, VIRTUAL_SHARES)?,
        add(total_assets, VIRTUAL_ASSETS)?,
    )
}

/// Assets redeemable for `shares`. Rounds down.
pub fn redeemable_amount(shares: u128, total_shares: u128, total_assets: u128) -> Result<u128> {
    mul_div_floor(
        shares,
        add(total_assets, VIRTUAL_ASSETS)?,
        add(total_shares, VIRTUAL_SHARES)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_deposit_uses_virtual_offset() {
        assert_eq!(shares_for_deposit(1_000_000, 0, 0).unwrap(), 1_000_000_000);
    }

    #[test]
    fn later_deposits_are_proportional() {
        assert_eq!(shares_for_deposit(500_000, 1_000_000_000, 1_000_000).unwrap(), 500_000_000);
        assert_eq!(
            shares_for_deposit(1_100_000, 1_000_000_000, 1_100_000).unwrap(),
            1_000_000_090
        );
    }

    #[test]
    fn redeem_matches_deposit_when_exact() {
        assert_eq!(redeemable_amount(1_000_000_000, 1_000_000_000, 1_000_000).unwrap(), 1_000_000);
    }

    #[test]
    fn burn_rounds_up_mint_rounds_down() {
        let (s, a) = (1_000_000_007u128, 1_000_003u128);
        for amount in [1u64, 7, 999, 123_456] {
            assert!(shares_to_burn(amount, s, a).unwrap() >= shares_for_deposit(amount, s, a).unwrap());
        }
    }

    #[test]
    fn deposit_then_redeem_never_returns_more() {
        let cases = [(0u128, 0u128), (1_000_000_000, 1_000_000), (3_333_333_333, 1_234_567), (7, 1_000_000_000)];
        for (total_shares, total_assets) in cases {
            for amount in [1u64, 2, 999, 1_000_000, 987_654_321] {
                let shares = shares_for_deposit(amount, total_shares, total_assets).unwrap();
                let back = redeemable_amount(shares, total_shares + shares, total_assets + amount as u128).unwrap();
                assert!(back <= amount as u128, "amount {amount} returned {back}");
            }
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p hodl_loans --lib`
Expected: `test result: ok. 10 passed` (9 math tests plus the constants test).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: checked math, lender share and interest accrual functions"
```

---

### Task 3: LiteSVM test harness

**Files:**
- Create: `programs/hodl_loans/tests/common/mod.rs`, `programs/hodl_loans/tests/harness.rs`

**Interfaces:**
- Consumes: `target/deploy/hodl_loans.so` (built by `scripts/test.sh`), seed constants (Task 1).
- Produces, used by every later test:
  - Types and constants: `TxResult = Result<(), String>`, `TOKEN_2022`, `SPL_TOKEN`, `ONE_CNGN = 1_000_000`, `YEAR_SECONDS = 31_536_000`.
  - Sending and error checks: `send(&mut LiteSVM, &[Instruction], &[&Keypair]) -> TxResult` (first signer pays), `assert_custom_error(TxResult, u32)`, `assert_hodl_error(TxResult, HodlError)`, `assert_anchor_error(TxResult, anchor_lang::error::ErrorCode)`.
  - Addresses and instructions: `pda`, `config_pda`, `access_pda`, `market_pda`, `market_vault_pda`, `lender_pda`, `ix(data, accounts) -> Instruction`.
  - Mints: `MintKind::{SplToken, CngnLike, TransferFee}`.
  - `Env { svm, admin, guardian, whitelister, promo_signer, treasury }` with:
    - setup: `Env::new()`, `set_upgrade_authority`, `upgrade_authority`, `funded_keypair`;
    - time: `now`, `warp_seconds`;
    - accounts: `fetch::<T>`, `write::<T>`;
    - tokens: `create_mint(kind, decimals)`, `mint_program`, `mint_decimals`, `mint_extensions`, `create_token_account(mint, owner)`, `mint_to`, `token_balance`, `token_owner`.

- [ ] **Step 1: Write the failing harness test**

`programs/hodl_loans/tests/harness.rs`:

```rust
mod common;

use common::*;
use solana_signer::Signer;
use spl_token_2022_interface::extension::ExtensionType;

#[test]
fn program_loads_with_admin_as_upgrade_authority() {
    let env = Env::new();
    assert_eq!(env.upgrade_authority(), Some(env.admin.pubkey()));
}

#[test]
fn creates_mints_token_accounts_and_balances() {
    let mut env = Env::new();
    let cngn = env.create_mint(MintKind::CngnLike, 6);
    assert_eq!(env.mint_program(&cngn), TOKEN_2022);
    assert_eq!(
        env.mint_extensions(&cngn),
        vec![ExtensionType::PermanentDelegate, ExtensionType::MetadataPointer]
    );
    let usdc = env.create_mint(MintKind::SplToken, 6);
    assert_eq!(env.mint_program(&usdc), SPL_TOKEN);

    let owner = solana_keypair::Keypair::new();
    let account = env.create_token_account(&cngn, &owner.pubkey());
    env.mint_to(&cngn, &account, 1_000);
    assert_eq!(env.token_balance(&account), 1_000);
    assert_eq!(env.token_owner(&account), owner.pubkey());
}

#[test]
fn warp_moves_the_clock() {
    let mut env = Env::new();
    let before = env.now();
    env.warp_seconds(3_600);
    assert_eq!(env.now(), before + 3_600);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `./scripts/test.sh --test harness`
Expected: the program builds, then compilation fails with `file not found for module common`.

- [ ] **Step 3: Write the harness**

`programs/hodl_loans/tests/common/mod.rs`:

```rust
#![allow(dead_code, unused_imports)]

use anchor_lang::{
    prelude::{Clock, ProgramData, Pubkey},
    solana_program::{
        bpf_loader_upgradeable::get_program_data_address, instruction::Instruction, system_instruction,
        system_program,
    },
    AccountDeserialize, AccountSerialize, InstructionData, ToAccountMetas,
};
use hodl_loans::{
    constants::{ACCESS_SEED, CONFIG_SEED, LENDER_SEED, MARKET_SEED, MARKET_VAULT_SEED},
    HodlError,
};
use litesvm::LiteSVM;
use solana_keypair::Keypair;
use solana_message::{Message, VersionedMessage};
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;
use spl_token_2022_interface::{
    extension::{metadata_pointer, transfer_fee, BaseStateWithExtensions, ExtensionType, StateWithExtensions},
    state::{Account as TokenAccountState, Mint as MintState},
};

pub const TOKEN_2022: Pubkey = spl_token_2022_interface::ID;
pub const SPL_TOKEN: Pubkey = spl_token_interface::ID;
pub const ONE_CNGN: u64 = 1_000_000;
pub const YEAR_SECONDS: i64 = 31_536_000;

pub type TxResult = Result<(), String>;

/// Sends `ixs`; the first signer pays fees. Returns the error and logs as text on failure.
pub fn send(svm: &mut LiteSVM, ixs: &[Instruction], signers: &[&Keypair]) -> TxResult {
    svm.expire_blockhash();
    let msg = Message::new_with_blockhash(ixs, Some(&signers[0].pubkey()), &svm.latest_blockhash());
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), signers).unwrap();
    svm.send_transaction(tx)
        .map(|_| ())
        .map_err(|e| format!("{:?} logs: {:#?}", e.err, e.meta.logs))
}

pub fn assert_custom_error(result: TxResult, code: u32) {
    let err = result.expect_err("transaction should have failed");
    assert!(err.contains(&format!("Custom({code})")), "expected Custom({code}), got {err}");
}

pub fn assert_hodl_error(result: TxResult, expected: HodlError) {
    assert_custom_error(result, u32::from(expected));
}

pub fn assert_anchor_error(result: TxResult, expected: anchor_lang::error::ErrorCode) {
    assert_custom_error(result, u32::from(expected));
}

pub fn pda(seeds: &[&[u8]]) -> Pubkey {
    Pubkey::find_program_address(seeds, &hodl_loans::ID).0
}
pub fn config_pda() -> Pubkey {
    pda(&[CONFIG_SEED])
}
pub fn access_pda(wallet: &Pubkey) -> Pubkey {
    pda(&[ACCESS_SEED, wallet.as_ref()])
}
pub fn market_pda(mint: &Pubkey) -> Pubkey {
    pda(&[MARKET_SEED, mint.as_ref()])
}
pub fn market_vault_pda(mint: &Pubkey) -> Pubkey {
    pda(&[MARKET_VAULT_SEED, mint.as_ref()])
}
pub fn lender_pda(market: &Pubkey, owner: &Pubkey) -> Pubkey {
    pda(&[LENDER_SEED, market.as_ref(), owner.as_ref()])
}

pub fn ix<D: InstructionData, A: ToAccountMetas>(data: D, accounts: A) -> Instruction {
    Instruction::new_with_bytes(hodl_loans::ID, &data.data(), accounts.to_account_metas(None))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MintKind {
    /// Classic SPL Token mint (USDC, USDT, wrapped SOL).
    SplToken,
    /// Token-2022 mint with the Solana cNGN extensions: permanent delegate + metadata pointer.
    CngnLike,
    /// Token-2022 mint with a transfer fee (must be rejected).
    TransferFee,
}

pub struct Env {
    pub svm: LiteSVM,
    pub admin: Keypair,
    pub guardian: Keypair,
    pub whitelister: Keypair,
    pub promo_signer: Keypair,
    pub treasury: Keypair,
}

impl Env {
    /// Program loaded with `admin` as its upgrade authority. `Config` is not initialized.
    pub fn new() -> Self {
        let mut svm = LiteSVM::new();
        let bytes = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/deploy/hodl_loans.so"));
        svm.add_program(hodl_loans::ID, bytes).unwrap();
        let env = Self {
            svm,
            admin: Keypair::new(),
            guardian: Keypair::new(),
            whitelister: Keypair::new(),
            promo_signer: Keypair::new(),
            treasury: Keypair::new(),
        };
        let mut env = env;
        for key in [env.admin.pubkey(), env.guardian.pubkey(), env.whitelister.pubkey(), env.treasury.pubkey()] {
            env.svm.airdrop(&key, 100_000_000_000).unwrap();
        }
        let authority = env.admin.pubkey();
        env.set_upgrade_authority(&authority);
        env
    }

    /// Rewrites the ProgramData header (4-byte tag, 8-byte slot, 1-byte option, 32-byte key).
    pub fn set_upgrade_authority(&mut self, authority: &Pubkey) {
        let key = get_program_data_address(&hodl_loans::ID);
        let mut account = self.svm.get_account(&key).expect("program data account");
        account.data[12] = 1;
        account.data[13..45].copy_from_slice(authority.as_ref());
        self.svm.set_account(key, account).unwrap();
    }

    pub fn upgrade_authority(&self) -> Option<Pubkey> {
        let account = self.svm.get_account(&get_program_data_address(&hodl_loans::ID)).unwrap();
        ProgramData::try_deserialize(&mut account.data.as_slice()).unwrap().upgrade_authority_address
    }

    pub fn funded_keypair(&mut self) -> Keypair {
        let key = Keypair::new();
        self.svm.airdrop(&key.pubkey(), 10_000_000_000).unwrap();
        key
    }

    pub fn now(&self) -> i64 {
        self.svm.get_sysvar::<Clock>().unix_timestamp
    }

    pub fn warp_seconds(&mut self, seconds: i64) {
        let mut clock: Clock = self.svm.get_sysvar();
        clock.unix_timestamp += seconds;
        self.svm.set_sysvar(&clock);
    }

    pub fn fetch<T: AccountDeserialize>(&self, key: &Pubkey) -> T {
        let account = self.svm.get_account(key).expect("account exists");
        T::try_deserialize(&mut account.data.as_slice()).expect("account deserializes")
    }

    /// Overwrites an Anchor account's data in place (tests use this to simulate loans).
    pub fn write<T: AccountSerialize>(&mut self, key: &Pubkey, value: &T) {
        let mut account = self.svm.get_account(key).expect("account exists");
        let mut bytes = Vec::new();
        value.try_serialize(&mut bytes).unwrap();
        account.data[..bytes.len()].copy_from_slice(&bytes);
        self.svm.set_account(*key, account).unwrap();
    }

    pub fn create_mint(&mut self, kind: MintKind, decimals: u8) -> Pubkey {
        let mint = Keypair::new();
        let authority = self.admin.pubkey();
        let (program, extensions) = match kind {
            MintKind::SplToken => (SPL_TOKEN, vec![]),
            MintKind::CngnLike => (TOKEN_2022, vec![ExtensionType::PermanentDelegate, ExtensionType::MetadataPointer]),
            MintKind::TransferFee => (TOKEN_2022, vec![ExtensionType::TransferFeeConfig]),
        };
        let space = ExtensionType::try_calculate_account_len::<MintState>(&extensions).unwrap();
        let lamports = self.svm.minimum_balance_for_rent_exemption(space);
        let mut ixs = vec![system_instruction::create_account(&authority, &mint.pubkey(), lamports, space as u64, &program)];
        match kind {
            MintKind::SplToken => {
                ixs.push(spl_token_interface::instruction::initialize_mint2(&SPL_TOKEN, &mint.pubkey(), &authority, None, decimals).unwrap());
            }
            MintKind::CngnLike => {
                ixs.push(spl_token_2022_interface::instruction::initialize_permanent_delegate(&TOKEN_2022, &mint.pubkey(), &authority).unwrap());
                ixs.push(metadata_pointer::instruction::initialize(&TOKEN_2022, &mint.pubkey(), Some(authority), Some(mint.pubkey())).unwrap());
                ixs.push(spl_token_2022_interface::instruction::initialize_mint2(&TOKEN_2022, &mint.pubkey(), &authority, None, decimals).unwrap());
            }
            MintKind::TransferFee => {
                ixs.push(transfer_fee::instruction::initialize_transfer_fee_config(&TOKEN_2022, &mint.pubkey(), Some(&authority), Some(&authority), 10, 1_000).unwrap());
                ixs.push(spl_token_2022_interface::instruction::initialize_mint2(&TOKEN_2022, &mint.pubkey(), &authority, None, decimals).unwrap());
            }
        }
        send(&mut self.svm, &ixs, &[&self.admin, &mint]).expect("create mint");
        mint.pubkey()
    }

    pub fn mint_program(&self, mint: &Pubkey) -> Pubkey {
        self.svm.get_account(mint).unwrap().owner
    }

    pub fn mint_decimals(&self, mint: &Pubkey) -> u8 {
        let account = self.svm.get_account(mint).unwrap();
        StateWithExtensions::<MintState>::unpack(&account.data).unwrap().base.decimals
    }

    pub fn mint_extensions(&self, mint: &Pubkey) -> Vec<ExtensionType> {
        let account = self.svm.get_account(mint).unwrap();
        StateWithExtensions::<MintState>::unpack(&account.data).unwrap().get_extension_types().unwrap()
    }

    /// A token account for `mint` owned by `owner` (any pubkey, including a PDA).
    pub fn create_token_account(&mut self, mint: &Pubkey, owner: &Pubkey) -> Pubkey {
        let account = Keypair::new();
        let program = self.mint_program(mint);
        let space = ExtensionType::try_calculate_account_len::<TokenAccountState>(&[]).unwrap();
        let lamports = self.svm.minimum_balance_for_rent_exemption(space);
        let ixs = vec![
            system_instruction::create_account(&self.admin.pubkey(), &account.pubkey(), lamports, space as u64, &program),
            spl_token_2022_interface::instruction::initialize_account3(&program, &account.pubkey(), mint, owner).unwrap(),
        ];
        send(&mut self.svm, &ixs, &[&self.admin, &account]).expect("create token account");
        account.pubkey()
    }

    pub fn mint_to(&mut self, mint: &Pubkey, destination: &Pubkey, amount: u64) {
        let program = self.mint_program(mint);
        let decimals = self.mint_decimals(mint);
        let ix = spl_token_2022_interface::instruction::mint_to_checked(
            &program, mint, destination, &self.admin.pubkey(), &[], amount, decimals,
        )
        .unwrap();
        send(&mut self.svm, &[ix], &[&self.admin]).expect("mint to");
    }

    pub fn token_balance(&self, account: &Pubkey) -> u64 {
        let account = self.svm.get_account(account).unwrap();
        StateWithExtensions::<TokenAccountState>::unpack(&account.data).unwrap().base.amount
    }

    pub fn token_owner(&self, account: &Pubkey) -> Pubkey {
        let account = self.svm.get_account(account).unwrap();
        StateWithExtensions::<TokenAccountState>::unpack(&account.data).unwrap().base.owner
    }
}
```

- [ ] **Step 4: Run it to verify it passes**

Run: `./scripts/test.sh --test harness`
Expected: `test result: ok. 3 passed`.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "test: LiteSVM harness with mints, token accounts and clock control"
```

---

### Task 4: Config, initialize, admin transfer and role setters

**Files:**
- Create: `src/state/mod.rs`, `src/state/config.rs`, `src/events.rs`, `src/instructions/mod.rs`, `src/instructions/admin/mod.rs`, `src/instructions/admin/initialize.rs`, `src/instructions/admin/admin_transfer.rs`, `src/instructions/admin/roles.rs` (all under `programs/hodl_loans/`)
- Modify: `programs/hodl_loans/src/lib.rs`, `programs/hodl_loans/tests/common/mod.rs` (append section)
- Test: `programs/hodl_loans/tests/admin.rs`

**Interfaces:**
- Consumes: constants and `HodlError` (Task 1), harness (Task 3).
- Produces:
  - `state::Config { version, bump, admin, pending_admin: Option<Pubkey>, guardian, whitelister, promo_signer, treasury, promo_cap_bps: u16, collateral_count: u16, reserved: [u8; 128] }`
  - `events::Role::{Guardian, Whitelister, PromoSigner, Treasury}`; events `ConfigInitialized`, `AdminProposed`, `AdminAccepted`, `RoleUpdated`
  - `InitializeArgs { guardian, whitelister, promo_signer, treasury }`
  - Accounts struct `AdminConfig { admin: Signer, config: Account<Config> }`, reused by later admin-only config instructions
  - Program instructions: `initialize(args)`, `propose_admin(proposed)`, `accept_admin()`, `set_guardian(guardian)`, `set_whitelister(whitelister)`, `set_promo_signer(promo_signer)`, `set_treasury(treasury)`
  - Harness: `Env::initialized()`, `Env::init_args`, `Env::initialize_ix(&authority)`, `Env::initialize`, `Env::config`, `admin_config_accounts(&admin)`

- [ ] **Step 1: Write the failing tests**

Append to `programs/hodl_loans/tests/common/mod.rs`:

```rust
// ---- Config and roles (Task 4) ----

impl Env {
    pub fn init_args(&self) -> hodl_loans::InitializeArgs {
        hodl_loans::InitializeArgs {
            guardian: self.guardian.pubkey(),
            whitelister: self.whitelister.pubkey(),
            promo_signer: self.promo_signer.pubkey(),
            treasury: self.treasury.pubkey(),
        }
    }

    pub fn initialize_ix(&self, authority: &Pubkey) -> Instruction {
        ix(
            hodl_loans::instruction::Initialize { args: self.init_args() },
            hodl_loans::accounts::Initialize {
                authority: *authority,
                config: config_pda(),
                program: hodl_loans::ID,
                program_data: get_program_data_address(&hodl_loans::ID),
                system_program: system_program::ID,
            },
        )
    }

    /// `new()` followed by a successful `initialize`.
    pub fn initialized() -> Self {
        let mut env = Self::new();
        env.initialize().unwrap();
        env
    }

    pub fn initialize(&mut self) -> TxResult {
        let instruction = self.initialize_ix(&self.admin.pubkey());
        send(&mut self.svm, &[instruction], &[&self.admin])
    }

    pub fn config(&self) -> hodl_loans::Config {
        self.fetch(&config_pda())
    }
}

pub fn admin_config_accounts(admin: &Pubkey) -> hodl_loans::accounts::AdminConfig {
    hodl_loans::accounts::AdminConfig { admin: *admin, config: config_pda() }
}
```

`programs/hodl_loans/tests/admin.rs`:

```rust
mod common;

use anchor_lang::prelude::Pubkey;
use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

#[test]
fn upgrade_authority_initializes_config() {
    let mut env = Env::new();
    env.initialize().unwrap();
    let config = env.config();
    assert_eq!(config.version, 1);
    assert_eq!(config.admin, env.admin.pubkey());
    assert_eq!(config.pending_admin, None);
    assert_eq!(config.guardian, env.guardian.pubkey());
    assert_eq!(config.whitelister, env.whitelister.pubkey());
    assert_eq!(config.promo_signer, env.promo_signer.pubkey());
    assert_eq!(config.treasury, env.treasury.pubkey());
    assert_eq!(config.promo_cap_bps, 2_000);
    assert_eq!(config.collateral_count, 0);
}

#[test]
fn only_upgrade_authority_can_initialize() {
    let mut env = Env::new();
    let stranger = env.funded_keypair();
    let instruction = env.initialize_ix(&stranger.pubkey());
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&stranger]), HodlError::Unauthorized);
}

#[test]
fn initialize_cannot_run_twice() {
    let mut env = Env::initialized();
    assert!(env.initialize().is_err());
}

#[test]
fn admin_transfer_is_two_step() {
    let mut env = Env::initialized();
    let new_admin = env.funded_keypair();

    let propose = ix(hodl_loans::instruction::ProposeAdmin { proposed: new_admin.pubkey() }, admin_config_accounts(&env.admin.pubkey()));
    send(&mut env.svm, &[propose], &[&env.admin]).unwrap();
    assert_eq!(env.config().pending_admin, Some(new_admin.pubkey()));
    assert_eq!(env.config().admin, env.admin.pubkey());

    let accept = ix(
        hodl_loans::instruction::AcceptAdmin {},
        hodl_loans::accounts::AcceptAdmin { new_admin: new_admin.pubkey(), config: config_pda() },
    );
    send(&mut env.svm, &[accept], &[&new_admin]).unwrap();
    assert_eq!(env.config().admin, new_admin.pubkey());
    assert_eq!(env.config().pending_admin, None);

    let old_admin_propose = ix(hodl_loans::instruction::ProposeAdmin { proposed: Pubkey::new_unique() }, admin_config_accounts(&env.admin.pubkey()));
    assert_hodl_error(send(&mut env.svm, &[old_admin_propose], &[&env.admin]), HodlError::Unauthorized);
}

#[test]
fn only_proposed_admin_can_accept() {
    let mut env = Env::initialized();
    let proposed = Pubkey::new_unique();
    let propose = ix(hodl_loans::instruction::ProposeAdmin { proposed }, admin_config_accounts(&env.admin.pubkey()));
    send(&mut env.svm, &[propose], &[&env.admin]).unwrap();

    let imposter = env.funded_keypair();
    let accept = ix(
        hodl_loans::instruction::AcceptAdmin {},
        hodl_loans::accounts::AcceptAdmin { new_admin: imposter.pubkey(), config: config_pda() },
    );
    assert_hodl_error(send(&mut env.svm, &[accept], &[&imposter]), HodlError::Unauthorized);
}

#[test]
fn admin_sets_roles_and_others_cannot() {
    let mut env = Env::initialized();
    let (g, w, p, t) = (Pubkey::new_unique(), Pubkey::new_unique(), Pubkey::new_unique(), Pubkey::new_unique());
    let admin = env.admin.pubkey();
    let ixs = vec![
        ix(hodl_loans::instruction::SetGuardian { guardian: g }, admin_config_accounts(&admin)),
        ix(hodl_loans::instruction::SetWhitelister { whitelister: w }, admin_config_accounts(&admin)),
        ix(hodl_loans::instruction::SetPromoSigner { promo_signer: p }, admin_config_accounts(&admin)),
        ix(hodl_loans::instruction::SetTreasury { treasury: t }, admin_config_accounts(&admin)),
    ];
    send(&mut env.svm, &ixs, &[&env.admin]).unwrap();
    let config = env.config();
    assert_eq!((config.guardian, config.whitelister, config.promo_signer, config.treasury), (g, w, p, t));

    let stranger = env.funded_keypair();
    let attempt = ix(hodl_loans::instruction::SetGuardian { guardian: stranger.pubkey() }, admin_config_accounts(&stranger.pubkey()));
    assert_hodl_error(send(&mut env.svm, &[attempt], &[&stranger]), HodlError::Unauthorized);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test admin`
Expected: compile errors: `cannot find type InitializeArgs in crate hodl_loans` and `cannot find struct Initialize in module hodl_loans::instruction`.

- [ ] **Step 3: Implement state, events and instructions**

`programs/hodl_loans/src/state/mod.rs`:

```rust
pub mod config;

pub use config::*;
```

`programs/hodl_loans/src/state/config.rs`:

```rust
use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct Config {
    pub version: u8,
    pub bump: u8,
    pub admin: Pubkey,
    pub pending_admin: Option<Pubkey>,
    pub guardian: Pubkey,
    pub whitelister: Pubkey,
    pub promo_signer: Pubkey,
    /// Owner of the token accounts that receive harvested reserve, swept donations
    /// and withdrawn promo funds.
    pub treasury: Pubkey,
    pub promo_cap_bps: u16,
    pub collateral_count: u16,
    pub reserved: [u8; 128],
}
```

`programs/hodl_loans/src/events.rs`:

```rust
use anchor_lang::prelude::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Guardian,
    Whitelister,
    PromoSigner,
    Treasury,
}

#[event]
pub struct ConfigInitialized {
    pub admin: Pubkey,
    pub guardian: Pubkey,
    pub whitelister: Pubkey,
    pub promo_signer: Pubkey,
    pub treasury: Pubkey,
}

#[event]
pub struct AdminProposed {
    pub admin: Pubkey,
    pub proposed: Pubkey,
}

#[event]
pub struct AdminAccepted {
    pub old_admin: Pubkey,
    pub new_admin: Pubkey,
}

#[event]
pub struct RoleUpdated {
    pub role: Role,
    pub old: Pubkey,
    pub new: Pubkey,
}
```

`programs/hodl_loans/src/instructions/mod.rs`:

```rust
pub mod admin;

pub use admin::*;
```

`programs/hodl_loans/src/instructions/admin/mod.rs`:

```rust
pub mod admin_transfer;
pub mod initialize;
pub mod roles;

pub use admin_transfer::*;
pub use initialize::*;
pub use roles::*;
```

`programs/hodl_loans/src/instructions/admin/initialize.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{ACCOUNT_VERSION, CONFIG_SEED, DEFAULT_PROMO_CAP_BPS};
use crate::errors::HodlError;
use crate::events::ConfigInitialized;
use crate::state::Config;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug)]
pub struct InitializeArgs {
    pub guardian: Pubkey,
    pub whitelister: Pubkey,
    pub promo_signer: Pubkey,
    pub treasury: Pubkey,
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    /// Must be the program's upgrade authority; becomes the admin.
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(init, payer = authority, space = 8 + Config::INIT_SPACE, seeds = [CONFIG_SEED], bump)]
    pub config: Account<'info, Config>,
    #[account(constraint = program.programdata_address()? == Some(program_data.key()) @ HodlError::Unauthorized)]
    pub program: Program<'info, crate::program::HodlLoans>,
    #[account(constraint = program_data.upgrade_authority_address == Some(authority.key()) @ HodlError::Unauthorized)]
    pub program_data: Account<'info, ProgramData>,
    pub system_program: Program<'info, System>,
}

pub fn handle_initialize(ctx: Context<Initialize>, args: InitializeArgs) -> Result<()> {
    let admin = ctx.accounts.authority.key();
    ctx.accounts.config.set_inner(Config {
        version: ACCOUNT_VERSION,
        bump: ctx.bumps.config,
        admin,
        pending_admin: None,
        guardian: args.guardian,
        whitelister: args.whitelister,
        promo_signer: args.promo_signer,
        treasury: args.treasury,
        promo_cap_bps: DEFAULT_PROMO_CAP_BPS,
        collateral_count: 0,
        reserved: [0; 128],
    });
    emit!(ConfigInitialized {
        admin,
        guardian: args.guardian,
        whitelister: args.whitelister,
        promo_signer: args.promo_signer,
        treasury: args.treasury,
    });
    Ok(())
}
```

`programs/hodl_loans/src/instructions/admin/admin_transfer.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::CONFIG_SEED;
use crate::errors::HodlError;
use crate::events::{AdminAccepted, AdminProposed};
use crate::state::Config;

/// Accounts for admin-only changes to `Config`.
#[derive(Accounts)]
pub struct AdminConfig<'info> {
    pub admin: Signer<'info>,
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
}

#[derive(Accounts)]
pub struct AcceptAdmin<'info> {
    pub new_admin: Signer<'info>,
    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump,
        constraint = config.pending_admin == Some(new_admin.key()) @ HodlError::Unauthorized
    )]
    pub config: Account<'info, Config>,
}

pub fn handle_propose_admin(ctx: Context<AdminConfig>, proposed: Pubkey) -> Result<()> {
    let config = &mut ctx.accounts.config;
    config.pending_admin = Some(proposed);
    emit!(AdminProposed { admin: config.admin, proposed });
    Ok(())
}

pub fn handle_accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
    let config = &mut ctx.accounts.config;
    let old_admin = config.admin;
    config.admin = ctx.accounts.new_admin.key();
    config.pending_admin = None;
    emit!(AdminAccepted { old_admin, new_admin: config.admin });
    Ok(())
}
```

`programs/hodl_loans/src/instructions/admin/roles.rs`:

```rust
use anchor_lang::prelude::*;

use crate::events::{Role, RoleUpdated};
use crate::instructions::admin::admin_transfer::AdminConfig;

/// Shared body of `set_guardian`, `set_whitelister`, `set_promo_signer` and `set_treasury`.
pub fn handle_set_role(ctx: Context<AdminConfig>, role: Role, new: Pubkey) -> Result<()> {
    let config = &mut ctx.accounts.config;
    let slot = match role {
        Role::Guardian => &mut config.guardian,
        Role::Whitelister => &mut config.whitelister,
        Role::PromoSigner => &mut config.promo_signer,
        Role::Treasury => &mut config.treasury,
    };
    let old = *slot;
    *slot = new;
    emit!(RoleUpdated { role, old, new });
    Ok(())
}
```

- [ ] **Step 4: Wire `lib.rs`**

Replace `programs/hodl_loans/src/lib.rs` with the following. Keep your real program ID.

```rust
use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod math;
pub mod state;

pub use constants::*;
pub use errors::*;
pub use events::*;
pub use instructions::*;
pub use state::*;

declare_id!("REPLACE_WITH_PROGRAM_ID");

#[program]
pub mod hodl_loans {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>, args: InitializeArgs) -> Result<()> {
        instructions::handle_initialize(ctx, args)
    }

    pub fn propose_admin(ctx: Context<AdminConfig>, proposed: Pubkey) -> Result<()> {
        instructions::handle_propose_admin(ctx, proposed)
    }

    pub fn accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
        instructions::handle_accept_admin(ctx)
    }

    pub fn set_guardian(ctx: Context<AdminConfig>, guardian: Pubkey) -> Result<()> {
        instructions::handle_set_role(ctx, Role::Guardian, guardian)
    }

    pub fn set_whitelister(ctx: Context<AdminConfig>, whitelister: Pubkey) -> Result<()> {
        instructions::handle_set_role(ctx, Role::Whitelister, whitelister)
    }

    pub fn set_promo_signer(ctx: Context<AdminConfig>, promo_signer: Pubkey) -> Result<()> {
        instructions::handle_set_role(ctx, Role::PromoSigner, promo_signer)
    }

    pub fn set_treasury(ctx: Context<AdminConfig>, treasury: Pubkey) -> Result<()> {
        instructions::handle_set_role(ctx, Role::Treasury, treasury)
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `./scripts/test.sh --test admin`
Expected: `test result: ok. 6 passed`.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: config initialization, two-step admin transfer and role setters"
```

---

### Task 5: Whitelist and blacklist

**Files:**
- Create: `programs/hodl_loans/src/state/access.rs`, `programs/hodl_loans/src/instructions/admin/access_control.rs`
- Modify: `src/state/mod.rs`, `src/events.rs`, `src/instructions/admin/mod.rs`, `src/lib.rs`, `tests/common/mod.rs`
- Test: `programs/hodl_loans/tests/access.rs`

**Interfaces:**
- Consumes: `Config`, `CONFIG_SEED`, `ACCESS_SEED`, `ACCOUNT_VERSION` (Tasks 1, 4).
- Produces:
  - `state::Access { version, bump, wallet, whitelisted, blacklisted, reserved: [u8; 32] }` with `init_if_new(wallet, bump)` and `require_active() -> Result<()>` (checks `Blacklisted` before `NotWhitelisted`)
  - Event `AccessUpdated`
  - Instructions: `whitelist(wallet)` (whitelister or admin; creates `Access`), `blacklist(wallet)` (admin; creates `Access`), `unblacklist(wallet)` (admin)
  - Harness: `whitelist_ix`, `blacklist_ix`, `unblacklist_ix`, `Env::whitelist`, `Env::blacklist`, `Env::access`

- [ ] **Step 1: Write the failing tests**

Append to `programs/hodl_loans/tests/common/mod.rs`:

```rust
// ---- Access (Task 5) ----

pub fn whitelist_ix(signer: &Pubkey, wallet: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::Whitelist { wallet: *wallet },
        hodl_loans::accounts::Whitelist {
            signer: *signer,
            config: config_pda(),
            access: access_pda(wallet),
            system_program: system_program::ID,
        },
    )
}

pub fn blacklist_ix(admin: &Pubkey, wallet: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::Blacklist { wallet: *wallet },
        hodl_loans::accounts::Blacklist {
            admin: *admin,
            config: config_pda(),
            access: access_pda(wallet),
            system_program: system_program::ID,
        },
    )
}

pub fn unblacklist_ix(admin: &Pubkey, wallet: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::Unblacklist { wallet: *wallet },
        hodl_loans::accounts::Unblacklist { admin: *admin, config: config_pda(), access: access_pda(wallet) },
    )
}

impl Env {
    pub fn whitelist(&mut self, wallet: &Pubkey) {
        let instruction = whitelist_ix(&self.whitelister.pubkey(), wallet);
        send(&mut self.svm, &[instruction], &[&self.whitelister]).expect("whitelist");
    }

    pub fn blacklist(&mut self, wallet: &Pubkey) {
        let instruction = blacklist_ix(&self.admin.pubkey(), wallet);
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("blacklist");
    }

    pub fn access(&self, wallet: &Pubkey) -> hodl_loans::Access {
        self.fetch(&access_pda(wallet))
    }
}
```

`programs/hodl_loans/tests/access.rs`:

```rust
mod common;

use anchor_lang::prelude::Pubkey;
use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

#[test]
fn whitelister_and_admin_can_whitelist() {
    let mut env = Env::initialized();
    let (a, b) = (Pubkey::new_unique(), Pubkey::new_unique());
    env.whitelist(&a);
    let by_admin = whitelist_ix(&env.admin.pubkey(), &b);
    send(&mut env.svm, &[by_admin], &[&env.admin]).unwrap();

    for wallet in [a, b] {
        let access = env.access(&wallet);
        assert_eq!(access.version, 1);
        assert_eq!(access.wallet, wallet);
        assert!(access.whitelisted);
        assert!(!access.blacklisted);
    }
}

#[test]
fn others_cannot_whitelist() {
    let mut env = Env::initialized();
    let stranger = env.funded_keypair();
    let attempt = whitelist_ix(&stranger.pubkey(), &stranger.pubkey());
    assert_hodl_error(send(&mut env.svm, &[attempt], &[&stranger]), HodlError::Unauthorized);
}

#[test]
fn blacklist_clears_whitelist_and_blocks_readmission() {
    let mut env = Env::initialized();
    let wallet = Pubkey::new_unique();
    env.whitelist(&wallet);
    env.blacklist(&wallet);
    let access = env.access(&wallet);
    assert!(!access.whitelisted);
    assert!(access.blacklisted);

    let by_whitelister = whitelist_ix(&env.whitelister.pubkey(), &wallet);
    assert_hodl_error(send(&mut env.svm, &[by_whitelister], &[&env.whitelister]), HodlError::Blacklisted);
    let by_admin = whitelist_ix(&env.admin.pubkey(), &wallet);
    assert_hodl_error(send(&mut env.svm, &[by_admin], &[&env.admin]), HodlError::Blacklisted);
}

#[test]
fn unblacklist_does_not_rewhitelist() {
    let mut env = Env::initialized();
    let wallet = Pubkey::new_unique();
    env.whitelist(&wallet);
    env.blacklist(&wallet);
    let clear = unblacklist_ix(&env.admin.pubkey(), &wallet);
    send(&mut env.svm, &[clear], &[&env.admin]).unwrap();
    let access = env.access(&wallet);
    assert!(!access.blacklisted);
    assert!(!access.whitelisted);

    env.whitelist(&wallet);
    assert!(env.access(&wallet).whitelisted);
}

#[test]
fn only_admin_can_blacklist_or_unblacklist() {
    let mut env = Env::initialized();
    let wallet = Pubkey::new_unique();
    env.whitelist(&wallet);
    let by_whitelister = blacklist_ix(&env.whitelister.pubkey(), &wallet);
    assert_hodl_error(send(&mut env.svm, &[by_whitelister], &[&env.whitelister]), HodlError::Unauthorized);

    env.blacklist(&wallet);
    let clear = unblacklist_ix(&env.whitelister.pubkey(), &wallet);
    assert_hodl_error(send(&mut env.svm, &[clear], &[&env.whitelister]), HodlError::Unauthorized);
}

#[test]
fn blacklisting_an_unknown_wallet_creates_its_access_account() {
    let mut env = Env::initialized();
    let wallet = Pubkey::new_unique();
    env.blacklist(&wallet);
    let access = env.access(&wallet);
    assert!(access.blacklisted);
    assert!(!access.whitelisted);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test access`
Expected: compile errors: `cannot find struct Whitelist in module hodl_loans::instruction` and `cannot find type Access in crate hodl_loans`.

- [ ] **Step 3: Implement**

`programs/hodl_loans/src/state/access.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::ACCOUNT_VERSION;
use crate::errors::HodlError;

#[account]
#[derive(InitSpace)]
pub struct Access {
    pub version: u8,
    pub bump: u8,
    pub wallet: Pubkey,
    pub whitelisted: bool,
    pub blacklisted: bool,
    pub reserved: [u8; 32],
}

impl Access {
    /// Fills in identity fields the first time an `init_if_needed` account is used.
    pub fn init_if_new(&mut self, wallet: Pubkey, bump: u8) {
        if self.version == 0 {
            self.version = ACCOUNT_VERSION;
            self.bump = bump;
            self.wallet = wallet;
        }
    }

    /// Blacklisted wins over whitelisted.
    pub fn require_active(&self) -> Result<()> {
        require!(!self.blacklisted, HodlError::Blacklisted);
        require!(self.whitelisted, HodlError::NotWhitelisted);
        Ok(())
    }
}
```

Replace `programs/hodl_loans/src/state/mod.rs`:

```rust
pub mod access;
pub mod config;

pub use access::*;
pub use config::*;
```

Append to `programs/hodl_loans/src/events.rs`:

```rust
#[event]
pub struct AccessUpdated {
    pub wallet: Pubkey,
    pub whitelisted: bool,
    pub blacklisted: bool,
}
```

`programs/hodl_loans/src/instructions/admin/access_control.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{ACCESS_SEED, CONFIG_SEED};
use crate::errors::HodlError;
use crate::events::AccessUpdated;
use crate::state::{Access, Config};

#[derive(Accounts)]
#[instruction(wallet: Pubkey)]
pub struct Whitelist<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,
    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump,
        constraint = signer.key() == config.whitelister || signer.key() == config.admin @ HodlError::Unauthorized
    )]
    pub config: Account<'info, Config>,
    #[account(init_if_needed, payer = signer, space = 8 + Access::INIT_SPACE, seeds = [ACCESS_SEED, wallet.as_ref()], bump)]
    pub access: Account<'info, Access>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(wallet: Pubkey)]
pub struct Blacklist<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(init_if_needed, payer = admin, space = 8 + Access::INIT_SPACE, seeds = [ACCESS_SEED, wallet.as_ref()], bump)]
    pub access: Account<'info, Access>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(wallet: Pubkey)]
pub struct Unblacklist<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(mut, seeds = [ACCESS_SEED, wallet.as_ref()], bump = access.bump)]
    pub access: Account<'info, Access>,
}

pub fn handle_whitelist(ctx: Context<Whitelist>, wallet: Pubkey) -> Result<()> {
    let access = &mut ctx.accounts.access;
    access.init_if_new(wallet, ctx.bumps.access);
    require!(!access.blacklisted, HodlError::Blacklisted);
    access.whitelisted = true;
    emit!(AccessUpdated { wallet, whitelisted: true, blacklisted: false });
    Ok(())
}

pub fn handle_blacklist(ctx: Context<Blacklist>, wallet: Pubkey) -> Result<()> {
    let access = &mut ctx.accounts.access;
    access.init_if_new(wallet, ctx.bumps.access);
    access.whitelisted = false;
    access.blacklisted = true;
    emit!(AccessUpdated { wallet, whitelisted: false, blacklisted: true });
    Ok(())
}

/// Clears the blacklist flag. Does not re-whitelist.
pub fn handle_unblacklist(ctx: Context<Unblacklist>, wallet: Pubkey) -> Result<()> {
    let access = &mut ctx.accounts.access;
    access.blacklisted = false;
    emit!(AccessUpdated { wallet, whitelisted: access.whitelisted, blacklisted: false });
    Ok(())
}
```

Replace `programs/hodl_loans/src/instructions/admin/mod.rs`:

```rust
pub mod access_control;
pub mod admin_transfer;
pub mod initialize;
pub mod roles;

pub use access_control::*;
pub use admin_transfer::*;
pub use initialize::*;
pub use roles::*;
```

In `lib.rs`, add inside `pub mod hodl_loans { … }` after `set_treasury`:

```rust
    pub fn whitelist(ctx: Context<Whitelist>, wallet: Pubkey) -> Result<()> {
        instructions::handle_whitelist(ctx, wallet)
    }

    pub fn blacklist(ctx: Context<Blacklist>, wallet: Pubkey) -> Result<()> {
        instructions::handle_blacklist(ctx, wallet)
    }

    pub fn unblacklist(ctx: Context<Unblacklist>, wallet: Pubkey) -> Result<()> {
        instructions::handle_unblacklist(ctx, wallet)
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh --test access`
Expected: `test result: ok. 6 passed`.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: whitelist, council-only blacklist and unblacklist"
```

---

### Task 6: cNGN market creation, parameters and pause

**Files:**
- Create: `src/state/market.rs`, `src/token/mod.rs`, `src/token/extensions.rs`, `src/instructions/admin/market_admin.rs` (under `programs/hodl_loans/`)
- Modify: `src/state/mod.rs`, `src/events.rs`, `src/instructions/admin/mod.rs`, `src/lib.rs`, `tests/common/mod.rs`
- Test: `programs/hodl_loans/tests/market.rs`

**Interfaces:**
- Consumes: `Config`, `AdminConfig` pattern, `lp_interest`, `add`, `sub` (Tasks 2, 4).
- Produces:
  - `state::Market` (every spec §7 field plus `vault_bump`, `reserved: [u8; 256]`) with `params() -> MarketParams`, `apply_params(&MarketParams)`, `accrue(now: i64) -> Result<()>`, `total_assets() -> Result<u128>`, `available_cash() -> u64`
  - `state::MarketParams { interest_rate_bps, penalty_rate_bps, reserve_factor_bps, max_utilization_bps, min_loan_amount, max_tenure_seconds, bad_debt_dust_usd, ngn_feed, ngn_max_stale_slots, ngn_min_samples, ngn_max_spread_bps, promo_inactivity_seconds, max_promo_per_position }` with `validate()`
  - `token::extensions::{MARKET_MINT_EXTENSIONS, mint_extension_types(&AccountInfo) -> Result<Vec<ExtensionType>>, require_allowed_extensions(&AccountInfo, &[ExtensionType]) -> Result<()>}`
  - Events `MarketCreated`, `MarketParamsUpdated`, `MarketPauseSet`
  - Instructions: `create_market(params)`, `update_market_params(params)`, `set_market_paused(paused)`
  - Harness: `default_market_params()`, `create_market_ix`, `update_market_params_ix`, `set_market_paused_ix`, `Env::with_cngn_market() -> (Env, mint)`, `Env::market(&mint)`

- [ ] **Step 1: Write the failing tests**

Append to `programs/hodl_loans/tests/common/mod.rs`:

```rust
// ---- Market (Task 6) ----

pub fn default_market_params() -> hodl_loans::MarketParams {
    hodl_loans::MarketParams {
        interest_rate_bps: 1_500,
        penalty_rate_bps: 500,
        reserve_factor_bps: 1_000,
        max_utilization_bps: 9_000,
        min_loan_amount: 1_000 * ONE_CNGN,
        max_tenure_seconds: 365 * 86_400,
        bad_debt_dust_usd: 1_000_000_000_000_000_000,
        ngn_feed: Pubkey::new_from_array([7; 32]),
        ngn_max_stale_slots: 150,
        ngn_min_samples: 3,
        ngn_max_spread_bps: 200,
        promo_inactivity_seconds: 90 * 86_400,
        max_promo_per_position: 50_000 * ONE_CNGN,
    }
}

pub fn create_market_ix(admin: &Pubkey, mint: &Pubkey, token_program: &Pubkey, params: hodl_loans::MarketParams) -> Instruction {
    ix(
        hodl_loans::instruction::CreateMarket { params },
        hodl_loans::accounts::CreateMarket {
            admin: *admin,
            config: config_pda(),
            mint: *mint,
            market: market_pda(mint),
            vault: market_vault_pda(mint),
            token_program: *token_program,
            system_program: system_program::ID,
        },
    )
}

pub fn update_market_params_ix(admin: &Pubkey, mint: &Pubkey, params: hodl_loans::MarketParams) -> Instruction {
    ix(
        hodl_loans::instruction::UpdateMarketParams { params },
        hodl_loans::accounts::UpdateMarketParams { admin: *admin, config: config_pda(), market: market_pda(mint) },
    )
}

pub fn set_market_paused_ix(signer: &Pubkey, mint: &Pubkey, paused: bool) -> Instruction {
    ix(
        hodl_loans::instruction::SetMarketPaused { paused },
        hodl_loans::accounts::SetMarketPaused { signer: *signer, config: config_pda(), market: market_pda(mint) },
    )
}

impl Env {
    /// Initialized config plus a cNGN-like market (6 decimals). Returns the mint.
    pub fn with_cngn_market() -> (Self, Pubkey) {
        let mut env = Self::initialized();
        let mint = env.create_mint(MintKind::CngnLike, 6);
        let instruction = create_market_ix(&env.admin.pubkey(), &mint, &TOKEN_2022, default_market_params());
        send(&mut env.svm, &[instruction], &[&env.admin]).expect("create market");
        (env, mint)
    }

    pub fn market(&self, mint: &Pubkey) -> hodl_loans::Market {
        self.fetch(&market_pda(mint))
    }
}
```

`programs/hodl_loans/tests/market.rs`:

```rust
mod common;

use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

#[test]
fn admin_creates_cngn_market() {
    let (env, mint) = Env::with_cngn_market();
    let market = env.market(&mint);
    let params = default_market_params();
    assert_eq!(market.version, 1);
    assert_eq!(market.mint, mint);
    assert_eq!(market.vault, market_vault_pda(&mint));
    assert_eq!(market.token_program, TOKEN_2022);
    assert_eq!(market.decimals, 6);
    assert_eq!(market.params(), params);
    assert_eq!(market.last_accrual_ts, env.now());
    assert_eq!((market.cash, market.total_borrows, market.total_shares), (0, 0, 0));
    assert!(!market.paused);
    assert_eq!(env.token_owner(&market.vault), market_pda(&mint));
}

#[test]
fn classic_spl_mint_is_accepted() {
    let mut env = Env::initialized();
    let mint = env.create_mint(MintKind::SplToken, 6);
    let instruction = create_market_ix(&env.admin.pubkey(), &mint, &SPL_TOKEN, default_market_params());
    send(&mut env.svm, &[instruction], &[&env.admin]).unwrap();
    assert_eq!(env.market(&mint).token_program, SPL_TOKEN);
}

#[test]
fn transfer_fee_mint_is_rejected() {
    let mut env = Env::initialized();
    let mint = env.create_mint(MintKind::TransferFee, 6);
    let instruction = create_market_ix(&env.admin.pubkey(), &mint, &TOKEN_2022, default_market_params());
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::UnsupportedMintExtension);
}

#[test]
fn invalid_params_are_rejected() {
    let mut env = Env::initialized();
    let mint = env.create_mint(MintKind::CngnLike, 6);
    let admin = env.admin.pubkey();

    let mut params = default_market_params();
    params.reserve_factor_bps = 10_001;
    let instruction = create_market_ix(&admin, &mint, &TOKEN_2022, params);
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::InvalidParameters);

    let mut params = default_market_params();
    params.max_utilization_bps = 10_001;
    let instruction = create_market_ix(&admin, &mint, &TOKEN_2022, params);
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::InvalidParameters);

    let mut params = default_market_params();
    params.max_tenure_seconds = 86_399;
    let instruction = create_market_ix(&admin, &mint, &TOKEN_2022, params);
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::InvalidParameters);
}

#[test]
fn only_admin_creates_markets() {
    let mut env = Env::initialized();
    let mint = env.create_mint(MintKind::CngnLike, 6);
    let stranger = env.funded_keypair();
    let instruction = create_market_ix(&stranger.pubkey(), &mint, &TOKEN_2022, default_market_params());
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&stranger]), HodlError::Unauthorized);
}

#[test]
fn admin_updates_params() {
    let (mut env, mint) = Env::with_cngn_market();
    let mut params = default_market_params();
    params.interest_rate_bps = 2_000;
    params.min_loan_amount = 5_000 * ONE_CNGN;
    let instruction = update_market_params_ix(&env.admin.pubkey(), &mint, params);
    send(&mut env.svm, &[instruction], &[&env.admin]).unwrap();
    assert_eq!(env.market(&mint).params(), params);

    let stranger = env.funded_keypair();
    let attempt = update_market_params_ix(&stranger.pubkey(), &mint, params);
    assert_hodl_error(send(&mut env.svm, &[attempt], &[&stranger]), HodlError::Unauthorized);

    params.reserve_factor_bps = 10_001;
    let invalid = update_market_params_ix(&env.admin.pubkey(), &mint, params);
    assert_hodl_error(send(&mut env.svm, &[invalid], &[&env.admin]), HodlError::InvalidParameters);
}

#[test]
fn guardian_pauses_only_admin_unpauses() {
    let (mut env, mint) = Env::with_cngn_market();

    let pause = set_market_paused_ix(&env.guardian.pubkey(), &mint, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();
    assert!(env.market(&mint).paused);

    let guardian_unpause = set_market_paused_ix(&env.guardian.pubkey(), &mint, false);
    assert_hodl_error(send(&mut env.svm, &[guardian_unpause], &[&env.guardian]), HodlError::Unauthorized);

    let admin_unpause = set_market_paused_ix(&env.admin.pubkey(), &mint, false);
    send(&mut env.svm, &[admin_unpause], &[&env.admin]).unwrap();
    assert!(!env.market(&mint).paused);

    let stranger = env.funded_keypair();
    let stranger_pause = set_market_paused_ix(&stranger.pubkey(), &mint, true);
    assert_hodl_error(send(&mut env.svm, &[stranger_pause], &[&stranger]), HodlError::Unauthorized);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test market`
Expected: compile errors: `cannot find type MarketParams in crate hodl_loans` and `cannot find struct CreateMarket in module hodl_loans::instruction`.

- [ ] **Step 3: Implement state and token extension checks**

`programs/hodl_loans/src/state/market.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{MAX_BPS, MIN_TENURE};
use crate::errors::HodlError;
use crate::math::checked::{add, sub};
use crate::math::interest::lp_interest;

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
    pub reserved: [u8; 256],
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
        require!(self.reserve_factor_bps <= MAX_BPS, HodlError::InvalidParameters);
        require!(self.max_utilization_bps <= MAX_BPS, HodlError::InvalidParameters);
        require!(self.max_tenure_seconds >= MIN_TENURE, HodlError::InvalidParameters);
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
        let interest = lp_interest(self.lp_rate_product, elapsed)?;
        self.accrued_interest = add(self.accrued_interest, interest)?;
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

Replace `programs/hodl_loans/src/state/mod.rs`:

```rust
pub mod access;
pub mod config;
pub mod market;

pub use access::*;
pub use config::*;
pub use market::*;
```

`programs/hodl_loans/src/token/mod.rs`:

```rust
pub mod extensions;
```

`programs/hodl_loans/src/token/extensions.rs`:

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

In `lib.rs`, add `pub mod token;` below `pub mod state;`. Do not add a `pub use` for it.

- [ ] **Step 4: Implement events and instructions**

In `programs/hodl_loans/src/events.rs`, add `use crate::state::MarketParams;` below `use anchor_lang::prelude::*;`, then append:

```rust
#[event]
pub struct MarketCreated {
    pub market: Pubkey,
    pub mint: Pubkey,
    pub vault: Pubkey,
}

#[event]
pub struct MarketParamsUpdated {
    pub market: Pubkey,
    pub old: MarketParams,
    pub new: MarketParams,
}

#[event]
pub struct MarketPauseSet {
    pub market: Pubkey,
    pub paused: bool,
    pub by: Pubkey,
}
```

`programs/hodl_loans/src/instructions/admin/market_admin.rs`:

```rust
use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{ACCOUNT_VERSION, CONFIG_SEED, MARKET_SEED, MARKET_VAULT_SEED};
use crate::errors::HodlError;
use crate::events::{MarketCreated, MarketParamsUpdated, MarketPauseSet};
use crate::state::{Config, Market, MarketParams};
use crate::token::extensions::{require_allowed_extensions, MARKET_MINT_EXTENSIONS};

#[derive(Accounts)]
pub struct CreateMarket<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(mint::token_program = token_program)]
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(init, payer = admin, space = 8 + Market::INIT_SPACE, seeds = [MARKET_SEED, mint.key().as_ref()], bump)]
    pub market: Account<'info, Market>,
    #[account(
        init,
        payer = admin,
        token::mint = mint,
        token::authority = market,
        token::token_program = token_program,
        seeds = [MARKET_VAULT_SEED, mint.key().as_ref()],
        bump
    )]
    pub vault: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateMarketParams<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(mut, seeds = [MARKET_SEED, market.mint.as_ref()], bump = market.bump)]
    pub market: Account<'info, Market>,
}

#[derive(Accounts)]
pub struct SetMarketPaused<'info> {
    pub signer: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(mut, seeds = [MARKET_SEED, market.mint.as_ref()], bump = market.bump)]
    pub market: Account<'info, Market>,
}

pub fn handle_create_market(ctx: Context<CreateMarket>, params: MarketParams) -> Result<()> {
    params.validate()?;
    require_allowed_extensions(&ctx.accounts.mint.to_account_info(), MARKET_MINT_EXTENSIONS)?;

    let mut market = Market {
        version: ACCOUNT_VERSION,
        bump: ctx.bumps.market,
        vault_bump: ctx.bumps.vault,
        mint: ctx.accounts.mint.key(),
        token_program: ctx.accounts.token_program.key(),
        vault: ctx.accounts.vault.key(),
        decimals: ctx.accounts.mint.decimals,
        cash: 0,
        total_borrows: 0,
        lp_rate_product: 0,
        accrued_interest: 0,
        protocol_reserve: 0,
        total_bad_debt: 0,
        total_shares: 0,
        last_accrual_ts: Clock::get()?.unix_timestamp,
        interest_rate_bps: 0,
        penalty_rate_bps: 0,
        reserve_factor_bps: 0,
        max_utilization_bps: 0,
        min_loan_amount: 0,
        max_tenure_seconds: 0,
        bad_debt_dust_usd: 0,
        ngn_feed: Pubkey::default(),
        ngn_max_stale_slots: 0,
        ngn_min_samples: 0,
        ngn_max_spread_bps: 0,
        promo_inactivity_seconds: 0,
        max_promo_per_position: 0,
        paused: false,
        reserved: [0; 256],
    };
    market.apply_params(&params);
    ctx.accounts.market.set_inner(market);

    emit!(MarketCreated {
        market: ctx.accounts.market.key(),
        mint: ctx.accounts.mint.key(),
        vault: ctx.accounts.vault.key(),
    });
    Ok(())
}

/// Rate and reserve-factor changes apply to new loans only; existing loans keep their terms.
pub fn handle_update_market_params(ctx: Context<UpdateMarketParams>, params: MarketParams) -> Result<()> {
    params.validate()?;
    let market_key = ctx.accounts.market.key();
    let market = &mut ctx.accounts.market;
    market.accrue(Clock::get()?.unix_timestamp)?;
    let old = market.params();
    market.apply_params(&params);
    emit!(MarketParamsUpdated { market: market_key, old, new: params });
    Ok(())
}

/// Guardian or admin may pause; only admin may unpause.
pub fn handle_set_market_paused(ctx: Context<SetMarketPaused>, paused: bool) -> Result<()> {
    let signer = ctx.accounts.signer.key();
    let config = &ctx.accounts.config;
    if paused {
        require!(signer == config.admin || signer == config.guardian, HodlError::Unauthorized);
    } else {
        require!(signer == config.admin, HodlError::Unauthorized);
    }
    let market_key = ctx.accounts.market.key();
    ctx.accounts.market.paused = paused;
    emit!(MarketPauseSet { market: market_key, paused, by: signer });
    Ok(())
}
```

Replace `programs/hodl_loans/src/instructions/admin/mod.rs`:

```rust
pub mod access_control;
pub mod admin_transfer;
pub mod initialize;
pub mod market_admin;
pub mod roles;

pub use access_control::*;
pub use admin_transfer::*;
pub use initialize::*;
pub use market_admin::*;
pub use roles::*;
```

In `lib.rs`, add inside `pub mod hodl_loans { … }` after `unblacklist`:

```rust
    pub fn create_market(ctx: Context<CreateMarket>, params: MarketParams) -> Result<()> {
        instructions::handle_create_market(ctx, params)
    }

    pub fn update_market_params(ctx: Context<UpdateMarketParams>, params: MarketParams) -> Result<()> {
        instructions::handle_update_market_params(ctx, params)
    }

    pub fn set_market_paused(ctx: Context<SetMarketPaused>, paused: bool) -> Result<()> {
        instructions::handle_set_market_paused(ctx, paused)
    }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `./scripts/test.sh --test market`
Expected: `test result: ok. 7 passed`.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: cNGN market creation with mint extension allowlist, params and pause"
```

---

### Task 7: Lender deposits

**Files:**
- Create: `src/state/lender.rs`, `src/token/transfer.rs`, `src/instructions/liquidity/mod.rs`, `src/instructions/liquidity/deposit_liquidity.rs` (under `programs/hodl_loans/`)
- Modify: `src/state/mod.rs`, `src/token/mod.rs`, `src/events.rs`, `src/instructions/mod.rs`, `src/lib.rs`, `tests/common/mod.rs`
- Test: `programs/hodl_loans/tests/liquidity.rs`

**Interfaces:**
- Consumes: `Access::require_active`, `Market::{accrue, total_assets}`, `shares_for_deposit`, `add` (Tasks 2, 5, 6).
- Produces:
  - `state::LenderPosition { version, bump, market, owner, shares: u128, reserved: [u8; 32] }` with `init_if_new(market, owner, bump)`
  - `token::transfer::{transfer_from_user(token_program: Pubkey, mint: AccountInfo, decimals: u8, from: AccountInfo, to: AccountInfo, authority: AccountInfo, amount: u64) -> Result<()>, transfer_from_vault(.., signer_seeds: &[&[&[u8]]]) -> Result<()>}`
  - Event `LiquidityDeposited`
  - Instruction `deposit_liquidity(amount)` with accounts `payer, owner, access, market, mint, vault, owner_token, lender, token_program, system_program`
  - Harness: `deposit_liquidity_ix`, `Lender { key, token }`, `Env::new_lender(&mint, balance)`, `Env::deposit`, `Env::lender_shares`

- [ ] **Step 1: Write the failing tests**

Append to `programs/hodl_loans/tests/common/mod.rs`:

```rust
// ---- Lender deposits (Task 7) ----

pub fn deposit_liquidity_ix(payer: &Pubkey, owner: &Pubkey, mint: &Pubkey, owner_token: &Pubkey, amount: u64) -> Instruction {
    let market = market_pda(mint);
    ix(
        hodl_loans::instruction::DepositLiquidity { amount },
        hodl_loans::accounts::DepositLiquidity {
            payer: *payer,
            owner: *owner,
            access: access_pda(owner),
            market,
            mint: *mint,
            vault: market_vault_pda(mint),
            owner_token: *owner_token,
            lender: lender_pda(&market, owner),
            token_program: TOKEN_2022,
            system_program: system_program::ID,
        },
    )
}

pub struct Lender {
    pub key: Keypair,
    pub token: Pubkey,
}

impl Env {
    /// A whitelisted wallet holding `balance` cNGN and no SOL (the admin pays its fees and rent).
    pub fn new_lender(&mut self, mint: &Pubkey, balance: u64) -> Lender {
        let key = Keypair::new();
        self.whitelist(&key.pubkey());
        let token = self.create_token_account(mint, &key.pubkey());
        self.mint_to(mint, &token, balance);
        Lender { key, token }
    }

    pub fn deposit(&mut self, lender: &Lender, mint: &Pubkey, amount: u64) -> TxResult {
        let instruction = deposit_liquidity_ix(&self.admin.pubkey(), &lender.key.pubkey(), mint, &lender.token, amount);
        send(&mut self.svm, &[instruction], &[&self.admin, &lender.key])
    }

    pub fn lender_shares(&self, mint: &Pubkey, owner: &Pubkey) -> u128 {
        let lender: hodl_loans::LenderPosition = self.fetch(&lender_pda(&market_pda(mint), owner));
        lender.shares
    }
}
```

`programs/hodl_loans/tests/liquidity.rs`:

```rust
mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

// ---- deposit_liquidity (Task 7) ----

#[test]
fn first_deposit_mints_shares_and_moves_tokens() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, 5 * ONE_CNGN);
    env.deposit(&lender, &mint, ONE_CNGN).unwrap();

    let market = env.market(&mint);
    assert_eq!(market.cash, ONE_CNGN);
    assert_eq!(market.total_shares, 1_000_000_000);
    assert_eq!(env.lender_shares(&mint, &lender.key.pubkey()), 1_000_000_000);
    assert_eq!(env.token_balance(&market_vault_pda(&mint)), ONE_CNGN);
    assert_eq!(env.token_balance(&lender.token), 4 * ONE_CNGN);
}

#[test]
fn later_deposits_get_proportional_shares() {
    let (mut env, mint) = Env::with_cngn_market();
    let first = env.new_lender(&mint, ONE_CNGN);
    let second = env.new_lender(&mint, ONE_CNGN);
    env.deposit(&first, &mint, ONE_CNGN).unwrap();
    env.deposit(&second, &mint, ONE_CNGN / 2).unwrap();
    assert_eq!(env.lender_shares(&mint, &second.key.pubkey()), 500_000_000);
    assert_eq!(env.market(&mint).total_shares, 1_500_000_000);
}

#[test]
fn donations_do_not_change_share_price() {
    let (mut env, mint) = Env::with_cngn_market();
    let first = env.new_lender(&mint, ONE_CNGN);
    let second = env.new_lender(&mint, ONE_CNGN);
    env.deposit(&first, &mint, ONE_CNGN).unwrap();
    env.mint_to(&mint, &market_vault_pda(&mint), 10 * ONE_CNGN);

    env.deposit(&second, &mint, ONE_CNGN / 2).unwrap();
    assert_eq!(env.lender_shares(&mint, &second.key.pubkey()), 500_000_000);
    assert_eq!(env.market(&mint).cash, ONE_CNGN + ONE_CNGN / 2);
}

#[test]
fn accrued_interest_raises_share_price() {
    let (mut env, mint) = Env::with_cngn_market();
    let first = env.new_lender(&mint, 1_000 * ONE_CNGN);
    let second = env.new_lender(&mint, 1_000 * ONE_CNGN);
    env.deposit(&first, &mint, 1_000 * ONE_CNGN).unwrap();

    // Simulate a 1,000 cNGN loan at 10% with a 10% reserve factor, then let a year pass.
    let key = market_pda(&mint);
    let mut market = env.market(&mint);
    market.lp_rate_product = (1_000 * ONE_CNGN as u128) * 1_000 * 9_000;
    env.write(&key, &market);
    env.warp_seconds(YEAR_SECONDS);

    env.deposit(&second, &mint, 1_000 * ONE_CNGN).unwrap();
    assert_eq!(env.market(&mint).accrued_interest, 90 * ONE_CNGN as u128);
    assert!(env.lender_shares(&mint, &second.key.pubkey()) < env.lender_shares(&mint, &first.key.pubkey()));
}

#[test]
fn deposit_requires_an_active_whitelisted_wallet() {
    let (mut env, mint) = Env::with_cngn_market();
    let stranger = solana_keypair::Keypair::new();
    let token = env.create_token_account(&mint, &stranger.pubkey());
    env.mint_to(&mint, &token, ONE_CNGN);
    let never_whitelisted = deposit_liquidity_ix(&env.admin.pubkey(), &stranger.pubkey(), &mint, &token, ONE_CNGN);
    assert_anchor_error(send(&mut env.svm, &[never_whitelisted], &[&env.admin, &stranger]), AnchorError::AccountNotInitialized);

    let lender = env.new_lender(&mint, ONE_CNGN);
    env.blacklist(&lender.key.pubkey());
    assert_hodl_error(env.deposit(&lender, &mint, ONE_CNGN), HodlError::Blacklisted);
}

#[test]
fn deposit_is_blocked_while_paused_and_for_zero() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, ONE_CNGN);
    assert_hodl_error(env.deposit(&lender, &mint, 0), HodlError::AmountTooSmall);

    let pause = set_market_paused_ix(&env.guardian.pubkey(), &mint, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();
    assert_hodl_error(env.deposit(&lender, &mint, ONE_CNGN), HodlError::MarketPaused);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test liquidity`
Expected: compile errors: `cannot find struct DepositLiquidity in module hodl_loans::instruction` and `cannot find type LenderPosition in crate hodl_loans`.

- [ ] **Step 3: Implement**

`programs/hodl_loans/src/state/lender.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::ACCOUNT_VERSION;

#[account]
#[derive(InitSpace)]
pub struct LenderPosition {
    pub version: u8,
    pub bump: u8,
    pub market: Pubkey,
    pub owner: Pubkey,
    pub shares: u128,
    pub reserved: [u8; 32],
}

impl LenderPosition {
    pub fn init_if_new(&mut self, market: Pubkey, owner: Pubkey, bump: u8) {
        if self.version == 0 {
            self.version = ACCOUNT_VERSION;
            self.bump = bump;
            self.market = market;
            self.owner = owner;
        }
    }
}
```

Replace `programs/hodl_loans/src/state/mod.rs`:

```rust
pub mod access;
pub mod config;
pub mod lender;
pub mod market;

pub use access::*;
pub use config::*;
pub use lender::*;
pub use market::*;
```

`programs/hodl_loans/src/token/transfer.rs`:

```rust
use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, TransferChecked};

/// Transfer signed by the wallet that owns `from`.
pub fn transfer_from_user<'info>(
    token_program: Pubkey,
    mint: AccountInfo<'info>,
    decimals: u8,
    from: AccountInfo<'info>,
    to: AccountInfo<'info>,
    authority: AccountInfo<'info>,
    amount: u64,
) -> Result<()> {
    token_interface::transfer_checked(
        CpiContext::new(token_program, TransferChecked { from, mint, to, authority }),
        amount,
        decimals,
    )
}

/// Transfer out of a program vault, signed by the vault's PDA owner.
#[allow(clippy::too_many_arguments)]
pub fn transfer_from_vault<'info>(
    token_program: Pubkey,
    mint: AccountInfo<'info>,
    decimals: u8,
    from: AccountInfo<'info>,
    to: AccountInfo<'info>,
    authority: AccountInfo<'info>,
    amount: u64,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    token_interface::transfer_checked(
        CpiContext::new_with_signer(token_program, TransferChecked { from, mint, to, authority }, signer_seeds),
        amount,
        decimals,
    )
}
```

Replace `programs/hodl_loans/src/token/mod.rs`:

```rust
pub mod extensions;
pub mod transfer;
```

Append to `programs/hodl_loans/src/events.rs`:

```rust
#[event]
pub struct LiquidityDeposited {
    pub market: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    pub shares: u128,
}
```

`programs/hodl_loans/src/instructions/liquidity/mod.rs`:

```rust
pub mod deposit_liquidity;

pub use deposit_liquidity::*;
```

`programs/hodl_loans/src/instructions/liquidity/deposit_liquidity.rs`:

```rust
use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{ACCESS_SEED, LENDER_SEED, MARKET_SEED};
use crate::errors::HodlError;
use crate::events::LiquidityDeposited;
use crate::math::checked::add;
use crate::math::shares::shares_for_deposit;
use crate::state::{Access, LenderPosition, Market};
use crate::token::transfer::transfer_from_user;

#[derive(Accounts)]
pub struct DepositLiquidity<'info> {
    /// Pays rent for a new lender account (e.g. the gas-relay sponsor).
    #[account(mut)]
    pub payer: Signer<'info>,
    pub owner: Signer<'info>,
    #[account(seeds = [ACCESS_SEED, owner.key().as_ref()], bump = access.bump)]
    pub access: Account<'info, Access>,
    #[account(mut, seeds = [MARKET_SEED, mint.key().as_ref()], bump = market.bump, has_one = mint, has_one = vault)]
    pub market: Box<Account<'info, Market>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, token::mint = mint, token::authority = owner, token::token_program = token_program)]
    pub owner_token: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        init_if_needed,
        payer = payer,
        space = 8 + LenderPosition::INIT_SPACE,
        seeds = [LENDER_SEED, market.key().as_ref(), owner.key().as_ref()],
        bump
    )]
    pub lender: Box<Account<'info, LenderPosition>>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn handle_deposit_liquidity(ctx: Context<DepositLiquidity>, amount: u64) -> Result<()> {
    ctx.accounts.access.require_active()?;
    require!(amount > 0, HodlError::AmountTooSmall);
    require!(!ctx.accounts.market.paused, HodlError::MarketPaused);

    let market_key = ctx.accounts.market.key();
    let owner_key = ctx.accounts.owner.key();
    let market = &mut ctx.accounts.market;
    market.accrue(Clock::get()?.unix_timestamp)?;
    let shares = shares_for_deposit(amount, market.total_shares, market.total_assets()?)?;
    require!(shares > 0, HodlError::ZeroShares);
    market.cash = market.cash.checked_add(amount).ok_or(HodlError::MathOverflow)?;
    market.total_shares = add(market.total_shares, shares)?;

    let lender = &mut ctx.accounts.lender;
    lender.init_if_new(market_key, owner_key, ctx.bumps.lender);
    lender.shares = add(lender.shares, shares)?;

    transfer_from_user(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.owner_token.to_account_info(),
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.owner.to_account_info(),
        amount,
    )?;

    emit!(LiquidityDeposited { market: market_key, owner: owner_key, amount, shares });
    Ok(())
}
```

Replace `programs/hodl_loans/src/instructions/mod.rs`:

```rust
pub mod admin;
pub mod liquidity;

pub use admin::*;
pub use liquidity::*;
```

In `lib.rs`, add inside `pub mod hodl_loans { … }` after `set_market_paused`:

```rust
    pub fn deposit_liquidity(ctx: Context<DepositLiquidity>, amount: u64) -> Result<()> {
        instructions::handle_deposit_liquidity(ctx, amount)
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh --test liquidity`
Expected: `test result: ok. 6 passed`.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: lender deposits with virtual-offset shares and sponsor-paid rent"
```

---

### Task 8: Lender withdrawals

**Files:**
- Create: `programs/hodl_loans/src/instructions/liquidity/withdraw_liquidity.rs`
- Modify: `src/events.rs`, `src/instructions/liquidity/mod.rs`, `src/lib.rs`, `tests/common/mod.rs`, `tests/liquidity.rs`

**Interfaces:**
- Consumes: `transfer_from_vault`, `shares_to_burn`, `redeemable_amount`, `sub`, `to_u64`, `Market::available_cash` (Tasks 2, 6, 7).
- Produces:
  - Event `LiquidityWithdrawn`
  - Instruction `withdraw_liquidity(amount)` (`u64::MAX` = everything redeemable, capped at `cash − protocol_reserve`; allowed while paused) with accounts `owner, access, market, mint, vault, owner_token, lender, token_program`
  - Harness: `withdraw_liquidity_ix`, `Env::withdraw`

- [ ] **Step 1: Write the failing tests**

Append to `programs/hodl_loans/tests/common/mod.rs`:

```rust
// ---- Lender withdrawals (Task 8) ----

pub fn withdraw_liquidity_ix(owner: &Pubkey, mint: &Pubkey, owner_token: &Pubkey, amount: u64) -> Instruction {
    let market = market_pda(mint);
    ix(
        hodl_loans::instruction::WithdrawLiquidity { amount },
        hodl_loans::accounts::WithdrawLiquidity {
            owner: *owner,
            access: access_pda(owner),
            market,
            mint: *mint,
            vault: market_vault_pda(mint),
            owner_token: *owner_token,
            lender: lender_pda(&market, owner),
            token_program: TOKEN_2022,
        },
    )
}

impl Env {
    pub fn withdraw(&mut self, lender: &Lender, mint: &Pubkey, amount: u64) -> TxResult {
        let instruction = withdraw_liquidity_ix(&lender.key.pubkey(), mint, &lender.token, amount);
        send(&mut self.svm, &[instruction], &[&self.admin, &lender.key])
    }
}
```

Append to `programs/hodl_loans/tests/liquidity.rs`:

```rust
// ---- withdraw_liquidity (Task 8) ----

#[test]
fn partial_withdraw_burns_shares_and_returns_tokens() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, ONE_CNGN);
    env.deposit(&lender, &mint, ONE_CNGN).unwrap();

    env.withdraw(&lender, &mint, 400_000).unwrap();
    assert_eq!(env.lender_shares(&mint, &lender.key.pubkey()), 600_000_000);
    assert_eq!(env.market(&mint).cash, 600_000);
    assert_eq!(env.token_balance(&lender.token), 400_000);
}

#[test]
fn withdraw_max_takes_everything_available() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, ONE_CNGN);
    env.deposit(&lender, &mint, ONE_CNGN).unwrap();

    env.withdraw(&lender, &mint, u64::MAX).unwrap();
    assert_eq!(env.lender_shares(&mint, &lender.key.pubkey()), 0);
    assert_eq!(env.market(&mint).cash, 0);
    assert_eq!(env.token_balance(&lender.token), ONE_CNGN);
}

#[test]
fn withdrawals_are_limited_to_cash_minus_reserve() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, ONE_CNGN);
    env.deposit(&lender, &mint, ONE_CNGN).unwrap();

    // Simulate 700,000 lent out and a 100,000 protocol reserve.
    let key = market_pda(&mint);
    let mut market = env.market(&mint);
    market.cash = 300_000;
    market.total_borrows = 700_000;
    market.protocol_reserve = 100_000;
    env.write(&key, &market);

    assert_hodl_error(env.withdraw(&lender, &mint, 200_001), HodlError::InsufficientCash);
    env.withdraw(&lender, &mint, u64::MAX).unwrap();
    assert_eq!(env.token_balance(&lender.token), 200_000);
    assert_eq!(env.market(&mint).cash, 100_000);
}

#[test]
fn cannot_withdraw_more_than_own_shares() {
    let (mut env, mint) = Env::with_cngn_market();
    let big = env.new_lender(&mint, ONE_CNGN);
    let small = env.new_lender(&mint, 1_000);
    env.deposit(&big, &mint, ONE_CNGN).unwrap();
    env.deposit(&small, &mint, 1_000).unwrap();
    assert_hodl_error(env.withdraw(&small, &mint, 2_000), HodlError::InsufficientShares);
}

#[test]
fn withdraw_works_while_paused_but_not_when_blacklisted() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, ONE_CNGN);
    env.deposit(&lender, &mint, ONE_CNGN).unwrap();

    let pause = set_market_paused_ix(&env.guardian.pubkey(), &mint, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();
    env.withdraw(&lender, &mint, 100_000).unwrap();

    env.blacklist(&lender.key.pubkey());
    assert_hodl_error(env.withdraw(&lender, &mint, 100_000), HodlError::Blacklisted);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test liquidity`
Expected: compile error: `cannot find struct WithdrawLiquidity in module hodl_loans::instruction`.

- [ ] **Step 3: Implement**

Append to `programs/hodl_loans/src/events.rs`:

```rust
#[event]
pub struct LiquidityWithdrawn {
    pub market: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    pub shares: u128,
}
```

`programs/hodl_loans/src/instructions/liquidity/withdraw_liquidity.rs`:

```rust
use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{ACCESS_SEED, LENDER_SEED, MARKET_SEED};
use crate::errors::HodlError;
use crate::events::LiquidityWithdrawn;
use crate::math::checked::{sub, to_u64};
use crate::math::shares::{redeemable_amount, shares_to_burn};
use crate::state::{Access, LenderPosition, Market};
use crate::token::transfer::transfer_from_vault;

#[derive(Accounts)]
pub struct WithdrawLiquidity<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [ACCESS_SEED, owner.key().as_ref()], bump = access.bump)]
    pub access: Account<'info, Access>,
    #[account(mut, seeds = [MARKET_SEED, mint.key().as_ref()], bump = market.bump, has_one = mint, has_one = vault)]
    pub market: Box<Account<'info, Market>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, token::mint = mint, token::authority = owner, token::token_program = token_program)]
    pub owner_token: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        seeds = [LENDER_SEED, market.key().as_ref(), owner.key().as_ref()],
        bump = lender.bump,
        has_one = owner
    )]
    pub lender: Box<Account<'info, LenderPosition>>,
    pub token_program: Interface<'info, TokenInterface>,
}

/// `amount == u64::MAX` withdraws the lender's full redeemable value, capped at available cash.
/// Works while the market is paused.
pub fn handle_withdraw_liquidity(ctx: Context<WithdrawLiquidity>, amount: u64) -> Result<()> {
    ctx.accounts.access.require_active()?;
    require!(amount > 0, HodlError::AmountTooSmall);

    let market_key = ctx.accounts.market.key();
    let owner_key = ctx.accounts.owner.key();
    let lender_shares = ctx.accounts.lender.shares;
    let market = &mut ctx.accounts.market;
    market.accrue(Clock::get()?.unix_timestamp)?;
    let total_assets = market.total_assets()?;
    let available = market.available_cash();

    let amount = if amount == u64::MAX {
        let redeemable = redeemable_amount(lender_shares, market.total_shares, total_assets)?;
        to_u64(redeemable.min(available as u128))?
    } else {
        amount
    };
    require!(amount > 0, HodlError::AmountTooSmall);
    require!(amount <= available, HodlError::InsufficientCash);

    let burned = shares_to_burn(amount, market.total_shares, total_assets)?;
    require!(burned <= lender_shares, HodlError::InsufficientShares);
    market.total_shares = sub(market.total_shares, burned)?;
    market.cash -= amount;
    ctx.accounts.lender.shares = sub(lender_shares, burned)?;

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

    emit!(LiquidityWithdrawn { market: market_key, owner: owner_key, amount, shares: burned });
    Ok(())
}
```

Replace `programs/hodl_loans/src/instructions/liquidity/mod.rs`:

```rust
pub mod deposit_liquidity;
pub mod withdraw_liquidity;

pub use deposit_liquidity::*;
pub use withdraw_liquidity::*;
```

In `lib.rs`, add inside `pub mod hodl_loans { … }` after `deposit_liquidity`:

```rust
    pub fn withdraw_liquidity(ctx: Context<WithdrawLiquidity>, amount: u64) -> Result<()> {
        instructions::handle_withdraw_liquidity(ctx, amount)
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh --test liquidity`
Expected: `test result: ok. 11 passed`.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: lender withdrawals limited to cash minus reserve"
```

---

### Task 9: Donation sweep and full-suite check

**Files:**
- Create: `programs/hodl_loans/src/instructions/admin/sweep.rs`
- Modify: `src/events.rs`, `src/instructions/admin/mod.rs`, `src/lib.rs`, `tests/common/mod.rs`
- Test: `programs/hodl_loans/tests/sweep.rs`

**Interfaces:**
- Consumes: `transfer_from_vault` (Task 7), `Config.treasury` (Task 4).
- Produces:
  - Event `ExcessSwept`
  - Instruction `sweep_market_excess()` (admin; destination must be a token account owned by `config.treasury`)
  - Harness: `sweep_market_excess_ix`

- [ ] **Step 1: Write the failing tests**

Append to `programs/hodl_loans/tests/common/mod.rs`:

```rust
// ---- Sweep (Task 9) ----

pub fn sweep_market_excess_ix(admin: &Pubkey, mint: &Pubkey, destination: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::SweepMarketExcess {},
        hodl_loans::accounts::SweepMarketExcess {
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

`programs/hodl_loans/tests/sweep.rs`:

```rust
mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

#[test]
fn sweep_sends_only_donations_to_treasury() {
    let (mut env, mint) = Env::with_cngn_market();
    let lender = env.new_lender(&mint, ONE_CNGN);
    env.deposit(&lender, &mint, ONE_CNGN).unwrap();
    let vault = market_vault_pda(&mint);
    env.mint_to(&mint, &vault, 250_000);

    let treasury_owner = env.treasury.pubkey();
    let destination = env.create_token_account(&mint, &treasury_owner);
    let sweep = sweep_market_excess_ix(&env.admin.pubkey(), &mint, &destination);
    send(&mut env.svm, &[sweep], &[&env.admin]).unwrap();

    assert_eq!(env.token_balance(&destination), 250_000);
    assert_eq!(env.token_balance(&vault), ONE_CNGN);
    assert_eq!(env.market(&mint).cash, ONE_CNGN);
}

#[test]
fn sweep_fails_without_excess() {
    let (mut env, mint) = Env::with_cngn_market();
    let treasury_owner = env.treasury.pubkey();
    let destination = env.create_token_account(&mint, &treasury_owner);
    let sweep = sweep_market_excess_ix(&env.admin.pubkey(), &mint, &destination);
    assert_hodl_error(send(&mut env.svm, &[sweep], &[&env.admin]), HodlError::AmountTooSmall);
}

#[test]
fn sweep_only_to_treasury_and_only_by_admin() {
    let (mut env, mint) = Env::with_cngn_market();
    env.mint_to(&mint, &market_vault_pda(&mint), 250_000);

    let stranger = env.funded_keypair();
    let wrong_destination = env.create_token_account(&mint, &stranger.pubkey());
    let to_stranger = sweep_market_excess_ix(&env.admin.pubkey(), &mint, &wrong_destination);
    assert_anchor_error(send(&mut env.svm, &[to_stranger], &[&env.admin]), AnchorError::ConstraintTokenOwner);

    let treasury_owner = env.treasury.pubkey();
    let destination = env.create_token_account(&mint, &treasury_owner);
    let by_stranger = sweep_market_excess_ix(&stranger.pubkey(), &mint, &destination);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test sweep`
Expected: compile error: `cannot find struct SweepMarketExcess in module hodl_loans::instruction`.

- [ ] **Step 3: Implement**

Append to `programs/hodl_loans/src/events.rs`:

```rust
#[event]
pub struct ExcessSwept {
    pub vault: Pubkey,
    pub destination: Pubkey,
    pub amount: u64,
}
```

`programs/hodl_loans/src/instructions/admin/sweep.rs`:

```rust
use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{CONFIG_SEED, MARKET_SEED};
use crate::errors::HodlError;
use crate::events::ExcessSwept;
use crate::state::{Config, Market};
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
```

Replace `programs/hodl_loans/src/instructions/admin/mod.rs`:

```rust
pub mod access_control;
pub mod admin_transfer;
pub mod initialize;
pub mod market_admin;
pub mod roles;
pub mod sweep;

pub use access_control::*;
pub use admin_transfer::*;
pub use initialize::*;
pub use market_admin::*;
pub use roles::*;
pub use sweep::*;
```

In `lib.rs`, add inside `pub mod hodl_loans { … }` after `set_market_paused`:

```rust
    pub fn sweep_market_excess(ctx: Context<SweepMarketExcess>) -> Result<()> {
        instructions::handle_sweep_market_excess(ctx)
    }
```

- [ ] **Step 4: Run the sweep tests, then the full suite and lints**

Run: `./scripts/test.sh --test sweep`
Expected: `test result: ok. 3 passed`.

Run: `./scripts/test.sh`
Expected: every test binary reports `ok`. Totals: 10 unit, 3 harness, 6 admin, 6 access, 7 market, 11 liquidity, 3 sweep.

Run: `cargo clippy -p hodl_loans --all-targets -- -D warnings`
Expected: finishes without errors. If clippy flags an issue, fix it in the file it names and re-run both commands.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: sweep donated market tokens to the treasury"
```
