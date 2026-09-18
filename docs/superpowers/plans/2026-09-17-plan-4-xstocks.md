# Plan 4: xStocks — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Tokenised stocks can be listed as collateral and priced correctly — the issuer's scaled-UI multiplier turns a raw balance into the shares Pyth quotes, and every transfer re-checks the mint, so an issuer that changes the rules after listing causes a clean failure instead of a silent mispricing.

**Architecture:**
- **Builds on Plan 3** (`main` at `08ff1ed`): collateral, prices, positions, loans, liquidation, write-off.
- **The listing decides the policy.** `list_collateral` gains a `kind`, and each kind has its own allowed extension set. `Standard` stays metadata-only; `XStock` allows the seven extensions the live Backed mints carry, and checks the two whose *values* matter — a transfer hook must name no program, and accounts must not be frozen by default.
- **The multiplier is read, never stored.** `ScaledUiAmountConfig` holds both the current factor and a scheduled one; the program picks by comparing block time to the effective timestamp, every time it prices the asset. Nothing is cached, so a corporate action needs no protocol action.
- **Pricing and seizure convert in opposite directions.** Health multiplies a raw balance up to display units, because Pyth quotes the display token; seizure divides a display amount back down to the raw units the vault actually moves.
- **Every transfer re-checks the mint.** Listing is a moment; an issuer's powers are permanent.

**Tech Stack:** Rust 1.89+, Anchor 1.2.0, Solana CLI 3.1.x (`cargo build-sbf`, platform tools v1.52), LiteSVM 0.10.0, `spl-token-2022-interface` 2.1.0, `pyth-solana-receiver-sdk` 2.0.0, `switchboard-on-demand` 0.13.0.

**Spec:** `docs/superpowers/specs/2026-09-17-solana-fixed-loans-design.md`

## Plan series

| Plan | Delivers |
|---|---|
| 1. Foundation and lender pool (merged) | Workspace, math, roles, whitelist/blacklist, cNGN market, lender deposit/withdraw, donation sweep |
| 2. Collateral, prices and loans (merged) | `CollateralAsset`, positions, Pyth and Switchboard reads, health checks, `take_loan`, `repay_loan`, reserve harvest |
| 3. Liquidation and bad debt (merged) | Pinned price accounts, seizure math, `liquidate`, `write_off_loan` (reserve first) |
| **4. xStocks** (this plan) | Per-kind mint policy, scaled-UI multiplier, display-amount pricing, display-price seizure, transfer-time re-checks |
| 5. Promo balance | Promo vault, campaigns, Ed25519 vouchers, forfeiture, expiry, `set_promo_cap` |
| 6. Hardening and devnet | Trident invariant fuzzing, ported EVM regressions, devnet run, pre-audit scan |

## Global Constraints

Carried from Plans 1–3:

- Anchor `1.2.0` for `anchor-lang` and `anchor-spl`; LiteSVM `0.10.0`; Rust `1.89` or newer; build with `cargo build-sbf --tools-version v1.52` (through `scripts/test.sh`).
- `[profile.release] overflow-checks = true`; all math on `u128` with checked operations.
- **Checked arithmetic, no exceptions.** Every new arithmetic site uses `checked_*` or the `math::checked` helpers, even where a `require!` on the previous line already proves it safe. (The guarded raw operators Plan 2 left behind stay for the Plan 6 sweep; do not add more.)
- Rounding: borrower debt rounds up; lender accrual and minted shares round down; burned shares round up.
- Constants: `BPS = 10_000`, `YEAR = 31_536_000` seconds, `VIRTUAL_SHARES = 1_000`, `VIRTUAL_ASSETS = 1`, `MIN_TENURE = 86_400`, `MAX_COLLATERAL_SLOTS = 8`, `MAX_LOAN_SLOTS = 10`, `USD_SCALE = 10^12`, `MAX_PRICE_AGE_SECONDS = 60`, `MAX_BAD_DEBT_DUST_USD = 1_000 × USD_SCALE`.
- Every account starts with `version: u8` and `bump: u8` and ends with reserved padding; new fields come out of that padding, so account sizes don't change. **This plan adds no account fields** — `CollateralAsset.kind` already exists, unused, from Plan 2.
- In user-facing instructions, wrap `Market`, `LenderPosition`, `CollateralAsset` and every `InterfaceAccount` in `Box<…>`. The SBF stack frame is 4 KB.
- One `#[error_code] HodlError` enum. **Append new variants only**, since codes are `6000 + position`. This plan adds none: `UnsupportedMintExtension`, `InvalidPrice` and `PriceAccountMismatch` already exist and carry every new failure.
- Collateral counts at `price − confidence` (round down); debt counts at `NGN price + spread` (round up). Healthy is `debt ≤ Σ value × ltv_bps / BPS`; liquidatable is `debt > Σ value × liquidation_threshold_bps / BPS`.
- Seizure uses plain prices — no confidence subtraction, no NGN spread.
- Commits made by Claude end with the attribution trailer from the session instructions.

New in this plan:

- **A raw balance is never priced directly.** Every valuation path goes through `CollateralValue::display_amount()`, which is `amount × multiplier / MULTIPLIER_SCALE` rounded down. A `Standard` asset's multiplier is `MULTIPLIER_ONE`, so the conversion is exact and the existing behaviour is unchanged.
- **The multiplier is never stored in a protocol account.** It is read from the mint on every use.
- Health accounts stay `(CollateralAsset, PriceUpdateV2)` for a `Standard` asset, and become `(CollateralAsset, PriceUpdateV2, mint)` for an `XStock`. The loop walks a cursor over the supplied accounts, and the cursor must land exactly on the end.
- `require_collateral_mint` runs on **every** instruction that moves collateral: deposit, withdraw, liquidate, sweep — not only at listing.

## Facts verified while writing this plan (2026-09-18)

- **The live xStock mints carry `DefaultAccountState`, which spec §14 rejected.** AAPLX (`XsbEhLAtcf6HdfpFZ5xEMdqW8nfAvcsP5bdudRLJzJp`), TSLAX (`XsDoVfqeBukxuZHWhdvWHBhgEHjGNst4MLodqsJHzoB`) and NVDAX (`Xsc9qvGR1efVDFGLrVsmkzv3qi45LTBjeUKSPmx9qEh`) each hold exactly `MetadataPointer`, `TokenMetadata`, `PermanentDelegate`, `Pausable`, `ScaledUiAmount`, `ConfidentialTransferMint`, `TransferHook` (program `null`) and `DefaultAccountState` (`initialized`), with 8 decimals and a freeze authority. Rejecting `DefaultAccountState` outright would have rejected every real xStock, so the extension is allowed and its *value* is checked instead. Full RPC output: `docs/superpowers/research/2026-09-18-xstocks-facts.md`.
- **Pyth quotes the display token.** `Crypto.AAPLX/USD` ($337.42) tracked `Equity.US.AAPL/USD` ($336.29) within 0.34% while AAPLX's effective multiplier was ≈1.0033, and Backed's own developer docs call the scaled amount — raw × multiplier — the one that "reflects the true equity value". Confidence medium-high; §8's assumption stands.
- **The stored `multiplier` field goes stale on its own.** Token-2022 never rewrites `multiplier` when `new_multiplier_effective_timestamp` passes; a consumer must compare block time and choose. Both AAPLX and NVDAX were already past their effective timestamps when checked, so reading `multiplier` naively would have undervalued both.
- **No sponsored Pyth push account was found for the xStock feeds**, so an xStock is listed with `price_account` pinned to whatever account HODL itself maintains, and the 60-second age cap does the rest.
- **Float conversion truncates in the protocol's favour.** `1.0009 × 10^12` is `1_000_899_999_999` in binary floating point, one unit low — it undervalues collateral by 10^-12 of a token, never the borrower's debt.
- **Compute at full load, measured in LiteSVM:** a position holding 8 xStock slots with 9 existing loans spends 79,912 CU on `take_loan`, against the 200,000 default. Compute is not the binding limit — the 24 price-related accounts push the legacy transaction past the 1,232-byte packet limit, so such a position needs a v0 transaction with an address lookup table. `tests/budget.rs` pins both.
- **Verification.** The full Plan 4 code was compiled and tested before this plan was written: 157 tests pass (41 unit, 116 LiteSVM), `cargo clippy -D warnings` is clean, every task's end state was rebuilt from Plan 3's head and passes its own suite and clippy, and each task's failing-test step was run to capture its expected errors.

## Plan-level refinements to the spec

This plan's commit already writes these into the spec.

- **§14 allows `DefaultAccountState` for `XStock` mints, checked at listing and at every transfer** (`state == Initialized`). The issuer's power to flip it later is a third accepted risk, alongside the permanent delegate and the pause.
- **§8 price accounts:** an `XStock` slot passes three accounts — `(CollateralAsset, PriceUpdateV2, mint)` — and the program walks a cursor rather than fixed-size chunks, requiring the supplied accounts to be consumed exactly.
- **§8 multiplier:** the `f64` conversion rounds down, and a multiplier that is not finite, not positive, above `MAX_MULTIPLIER = 10^6` or that rounds to zero is rejected with `InvalidPrice`.
- **§15:** 8 xStock slots is 24 price-related accounts; the measured legacy transaction no longer fits a packet.
- **§20 items 2 and 3 are resolved** with the findings above.
- **Deferred:** promo forfeiture on liquidation (spec §11 step 3) still arrives with Plan 5. Nothing in this plan touches the promo path.

## File Structure

New and changed files under `programs/hodl_loans/`:

```text
src/constants.rs                              + MULTIPLIER_SCALE, MULTIPLIER_ONE, MAX_MULTIPLIER
src/token/extensions.rs                       + XSTOCK_COLLATERAL_EXTENSIONS, require_collateral_mint, require_xstock_mint
src/token/scaled_ui.rs                        new: read_multiplier
src/token/mod.rs                              + scaled_ui
src/instructions/admin/collateral_admin.rs    list_collateral takes a kind
src/instructions/admin/sweep.rs               re-check before the sweep transfer
src/instructions/positions/deposit_collateral.rs    re-check before the transfer
src/instructions/positions/withdraw_collateral.rs   re-check before the transfer
src/instructions/liquidation/liquidate.rs     re-check before the seizure; pass the multiplier
src/math/health.rs                            CollateralValue.multiplier, display_amount
src/math/liquidation.rs                       seize_for_repayment takes the multiplier
src/valuation.rs                              three accounts per xStock, cursor walk, read_multiplier
src/events.rs                                 CollateralListed gains kind
src/lib.rs                                    list_collateral gains kind
tests/common/mod.rs                           harness: xStock mints, issuer actions, per-kind health accounts
tests/collateral.rs                           list_collateral_ix gains a kind
tests/withdraw.rs                             + a Token-2022 Standard asset round trip
tests/budget.rs                               + an all-xStock position
tests/xstocks.rs                              new
```

All paths below are relative to the repository root.

---

### Task 1: Collateral kinds and the xStock mint policy

**Files:**
- Create: `programs/hodl_loans/tests/xstocks.rs`
- Modify (under `programs/hodl_loans/`): `src/token/extensions.rs`, `src/instructions/admin/collateral_admin.rs`, `src/instructions/admin/sweep.rs`, `src/instructions/positions/deposit_collateral.rs`, `src/instructions/positions/withdraw_collateral.rs`, `src/instructions/liquidation/liquidate.rs`, `src/events.rs`, `src/lib.rs`, `tests/common/mod.rs`, `tests/collateral.rs`, `tests/withdraw.rs`

**Interfaces:**
- Consumes: `CollateralKind` (Plan 2, declared but until now always `Standard`), `require_allowed_extensions`, `STANDARD_COLLATERAL_EXTENSIONS`, `CollateralAsset.kind`.
- Produces:
  - `token::extensions::XSTOCK_COLLATERAL_EXTENSIONS: &[ExtensionType]`
  - `token::extensions::require_collateral_mint(mint: &AccountInfo, kind: CollateralKind) -> Result<()>`
  - `list_collateral(ctx, params: CollateralParams, kind: CollateralKind)` — the instruction data gains a second argument; `CollateralListed` gains `kind`
  - Harness: `MintKind::{Token2022Plain, XStock}`; `list_collateral_ix(admin, mint, token_program, params, kind)`; `XSTOCK_DECIMALS`, `ONE_XSTOCK`, `xstock_collateral_params`; `Env::{list_t22_collateral, list_xstock_collateral, set_mint_paused, set_transfer_hook, set_default_account_state, delegate_burn}`

- [ ] **Step 1: Update the harness and write the failing tests**

In `programs/hodl_loans/tests/common/mod.rs`, widen the Token-2022 imports:

```rust
use spl_token_2022_interface::{
    extension::{
        confidential_transfer, default_account_state, metadata_pointer, pausable, scaled_ui_amount,
        transfer_fee, transfer_hook, BaseStateWithExtensions, ExtensionType, StateWithExtensions,
    },
    state::{Account as TokenAccountState, AccountState, Mint as MintState},
};
```

Add two variants to `MintKind`:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MintKind {
    /// Classic SPL Token mint (USDC, USDT, wrapped SOL).
    SplToken,
    /// Token-2022 mint with the Solana cNGN extensions: permanent delegate + metadata pointer.
    CngnLike,
    /// Token-2022 mint carrying metadata only: the shape a `Standard` collateral asset
    /// may have (spec §14).
    Token2022Plain,
    /// Token-2022 mint with a transfer fee (must be rejected).
    TransferFee,
    /// Token-2022 mint shaped like a live Backed xStock (AAPLX, TSLAX, NVDAX, checked
    /// 2026-09-18): metadata pointer, permanent delegate, scaled-UI amount, pausable,
    /// transfer hook with no program, default account state, confidential transfer.
    XStock,
}
```

Replace `Env::create_mint` (the two new arms build those mints; the xStock mint takes a freeze authority, since that is what `DefaultAccountState` updates are signed by):

```rust
    pub fn create_mint(&mut self, kind: MintKind, decimals: u8) -> Pubkey {
        let mint = Keypair::new();
        let authority = self.admin.pubkey();
        let (program, extensions) = match kind {
            MintKind::SplToken => (SPL_TOKEN, vec![]),
            MintKind::CngnLike => (TOKEN_2022, vec![ExtensionType::PermanentDelegate, ExtensionType::MetadataPointer]),
            MintKind::Token2022Plain => (TOKEN_2022, vec![ExtensionType::MetadataPointer]),
            MintKind::TransferFee => (TOKEN_2022, vec![ExtensionType::TransferFeeConfig]),
            MintKind::XStock => (
                TOKEN_2022,
                vec![
                    ExtensionType::MetadataPointer,
                    ExtensionType::PermanentDelegate,
                    ExtensionType::ScaledUiAmount,
                    ExtensionType::Pausable,
                    ExtensionType::TransferHook,
                    ExtensionType::DefaultAccountState,
                    ExtensionType::ConfidentialTransferMint,
                ],
            ),
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
            MintKind::Token2022Plain => {
                ixs.push(metadata_pointer::instruction::initialize(&TOKEN_2022, &mint.pubkey(), Some(authority), Some(mint.pubkey())).unwrap());
                ixs.push(spl_token_2022_interface::instruction::initialize_mint2(&TOKEN_2022, &mint.pubkey(), &authority, None, decimals).unwrap());
            }
            MintKind::TransferFee => {
                ixs.push(transfer_fee::instruction::initialize_transfer_fee_config(&TOKEN_2022, &mint.pubkey(), Some(&authority), Some(&authority), 10, 1_000).unwrap());
                ixs.push(spl_token_2022_interface::instruction::initialize_mint2(&TOKEN_2022, &mint.pubkey(), &authority, None, decimals).unwrap());
            }
            MintKind::XStock => {
                let m = mint.pubkey();
                ixs.push(metadata_pointer::instruction::initialize(&TOKEN_2022, &m, Some(authority), Some(m)).unwrap());
                ixs.push(spl_token_2022_interface::instruction::initialize_permanent_delegate(&TOKEN_2022, &m, &authority).unwrap());
                ixs.push(scaled_ui_amount::instruction::initialize(&TOKEN_2022, &m, Some(authority), 1.0).unwrap());
                ixs.push(pausable::instruction::initialize(&TOKEN_2022, &m, &authority).unwrap());
                ixs.push(transfer_hook::instruction::initialize(&TOKEN_2022, &m, Some(authority), None).unwrap());
                ixs.push(default_account_state::instruction::initialize_default_account_state(&TOKEN_2022, &m, &AccountState::Initialized).unwrap());
                ixs.push(confidential_transfer::instruction::initialize_mint(&TOKEN_2022, &m, Some(authority), true, None).unwrap());
                ixs.push(spl_token_2022_interface::instruction::initialize_mint2(&TOKEN_2022, &m, &authority, Some(&authority), decimals).unwrap());
            }
        }
        send(&mut self.svm, &ixs, &[&self.admin, &mint]).expect("create mint");
        mint.pubkey()
    }
```

Replace `Env::create_token_account` — a Token-2022 mint extension can oblige every token account to carry a matching one, and an xStock's `Pausable` and `TransferHook` both do:

```rust
    /// A token account for `mint` owned by `owner` (any pubkey, including a PDA).
    pub fn create_token_account(&mut self, mint: &Pubkey, owner: &Pubkey) -> Pubkey {
        let account = Keypair::new();
        let program = self.mint_program(mint);
        // A Token-2022 mint extension can oblige every account to carry a matching one.
        let required = if program == TOKEN_2022 {
            ExtensionType::get_required_init_account_extensions(&self.mint_extensions(mint))
        } else {
            vec![]
        };
        let space = ExtensionType::try_calculate_account_len::<TokenAccountState>(&required).unwrap();
        let lamports = self.svm.minimum_balance_for_rent_exemption(space);
        let ixs = vec![
            system_instruction::create_account(&self.admin.pubkey(), &account.pubkey(), lamports, space as u64, &program),
            spl_token_2022_interface::instruction::initialize_account3(&program, &account.pubkey(), mint, owner).unwrap(),
        ];
        send(&mut self.svm, &ixs, &[&self.admin, &account]).expect("create token account");
        account.pubkey()
    }
```

Replace `list_collateral_ix`:

```rust
pub fn list_collateral_ix(
    admin: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
    params: hodl_loans::CollateralParams,
    kind: hodl_loans::CollateralKind,
) -> Instruction {
    ix(
        hodl_loans::instruction::ListCollateral { params, kind },
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
```

Replace `Env::list_spl_collateral`:

```rust
    /// Creates a classic SPL Token mint and lists it with default parameters. Returns the mint.
    pub fn list_spl_collateral(&mut self, decimals: u8) -> Pubkey {
        let mint = self.create_mint(MintKind::SplToken, decimals);
        let instruction = list_collateral_ix(
            &self.admin.pubkey(),
            &mint,
            &SPL_TOKEN,
            default_collateral_params(&mint),
            hodl_loans::CollateralKind::Standard,
        );
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("list collateral");
        mint
    }
```

Append a new section at the end of the file:

```rust
// ---- Collateral kinds and xStock mints (Task 1) ----

/// A live Backed xStock carries 8 decimals; the scaled-UI multiplier is what makes a raw
/// balance a number of shares.
pub const XSTOCK_DECIMALS: u8 = 8;
pub const ONE_XSTOCK: u64 = 100_000_000;

/// Spec §8 launch values for an xStock: LTV 50%, threshold 75%, bonus 10%, pinned price.
pub fn xstock_collateral_params(mint: &Pubkey) -> hodl_loans::CollateralParams {
    hodl_loans::CollateralParams {
        ltv_bps: 5_000,
        liquidation_threshold_bps: 7_500,
        liquidation_bonus_bps: 1_000,
        ..default_collateral_params(mint)
    }
}

impl Env {
    /// A metadata-only Token-2022 mint listed as `Standard` collateral, priced at $1.
    pub fn list_t22_collateral(&mut self, decimals: u8) -> Pubkey {
        let mint = self.create_mint(MintKind::Token2022Plain, decimals);
        let instruction = list_collateral_ix(
            &self.admin.pubkey(),
            &mint,
            &TOKEN_2022,
            default_collateral_params(&mint),
            hodl_loans::CollateralKind::Standard,
        );
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("list collateral");
        self.set_pyth_price(&mint, ONE_DOLLAR, 0);
        mint
    }

    /// A live-shaped xStock mint, listed as `XStock` collateral and priced at `dollars` a
    /// share (Pyth exponent -8), with the multiplier at 1.
    pub fn list_xstock_collateral(&mut self, dollars: i64) -> Pubkey {
        let mint = self.create_mint(MintKind::XStock, XSTOCK_DECIMALS);
        let instruction = list_collateral_ix(
            &self.admin.pubkey(),
            &mint,
            &TOKEN_2022,
            xstock_collateral_params(&mint),
            hodl_loans::CollateralKind::XStock,
        );
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("list xstock");
        self.set_pyth_price(&mint, dollars * ONE_DOLLAR, 0);
        mint
    }

    pub fn set_mint_paused(&mut self, mint: &Pubkey, paused: bool) {
        let authority = self.admin.pubkey();
        let instruction = if paused {
            pausable::instruction::pause(&TOKEN_2022, mint, &authority, &[]).unwrap()
        } else {
            pausable::instruction::resume(&TOKEN_2022, mint, &authority, &[]).unwrap()
        };
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("set mint pause");
    }

    /// Points the mint's transfer hook at a program (or clears it).
    pub fn set_transfer_hook(&mut self, mint: &Pubkey, program: Option<Pubkey>) {
        let authority = self.admin.pubkey();
        let instruction =
            transfer_hook::instruction::update(&TOKEN_2022, mint, &authority, &[], program).unwrap();
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("set transfer hook");
    }

    /// Flips the mint to freezing new token accounts by default (the issuer's blocklist power).
    pub fn set_default_account_state(&mut self, mint: &Pubkey, state: AccountState) {
        let authority = self.admin.pubkey();
        let instruction = default_account_state::instruction::update_default_account_state(
            &TOKEN_2022, mint, &authority, &[], &state,
        )
        .unwrap();
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("set default account state");
    }

    /// The issuer moving tokens out of an account it does not own, through the permanent
    /// delegate — the risk the spec accepts for xStocks.
    pub fn delegate_burn(&mut self, mint: &Pubkey, from: &Pubkey, amount: u64) {
        let authority = self.admin.pubkey();
        let decimals = self.mint_decimals(mint);
        let instruction = spl_token_2022_interface::instruction::burn_checked(
            &TOKEN_2022, from, mint, &authority, &[], amount, decimals,
        )
        .unwrap();
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("delegate burn");
    }
}
```

Replace `programs/hodl_loans/tests/collateral.rs` (five call sites gain `hodl_loans::CollateralKind::Standard`):

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
    let by_stranger = list_collateral_ix(&stranger.pubkey(), &mint, &SPL_TOKEN, default_collateral_params(&mint), hodl_loans::CollateralKind::Standard);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);

    let listing = list_collateral_ix(&env.admin.pubkey(), &mint, &SPL_TOKEN, default_collateral_params(&mint), hodl_loans::CollateralKind::Standard);
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
        let instruction = list_collateral_ix(&admin, &mint, &TOKEN_2022, default_collateral_params(&mint), hodl_loans::CollateralKind::Standard);
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
    let instruction = list_collateral_ix(&admin, &other, &SPL_TOKEN, bad_listing, hodl_loans::CollateralKind::Standard);
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
    let relist = list_collateral_ix(&admin, &mint, &SPL_TOKEN, default_collateral_params(&mint), hodl_loans::CollateralKind::Standard);
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

In `programs/hodl_loans/tests/withdraw.rs`, add this test before `pairs_cover_the_slots_left_after_withdrawal` — it closes the Plan 2 follow-up asking for a Token-2022 `Standard` collateral path:

```rust
#[test]
fn a_token_2022_standard_asset_moves_through_the_same_paths() {
    // 6 decimals, metadata only: the Token-2022 collateral shape spec §14 allows as `Standard`.
    const ONE_T22: u64 = 1_000_000;
    let (mut env, setup) = Env::loan_ready();
    let t22 = env.list_t22_collateral(6);
    let borrower = env.new_borrower();
    let owner = borrower.pubkey();
    let token = env.deposit_collateral(&borrower, &t22, 1_000 * ONE_T22);
    let borrower_cngn = env.create_token_account(&setup.cngn, &owner);
    let setup = LoanSetup { borrower, borrower_cngn, ..setup };
    assert_eq!(env.token_balance(&collateral_vault_pda(&t22)), 1_000 * ONE_T22);
    assert_eq!(env.position(&owner).collateral[0].amount, 1_000 * ONE_T22);

    // $1,000 of collateral backs 500,000 cNGN ($312.8125 at the NGN ask).
    env.take_loan(&setup.borrower, &setup, 500_000 * ONE_CNGN, 365 * DAY).unwrap();
    let cngn = setup.cngn;
    let withdraw = |amount| withdraw_collateral_ix(&owner, &t22, &TOKEN_2022, &token, Some(&cngn), amount, price_pairs(&[t22]));
    let key = &setup.borrower.key;

    // 447 units × 70% = $312.90 still covers the debt; 446 ($312.20) does not.
    env.sponsored(withdraw(553 * ONE_T22), key).unwrap();
    assert_eq!(env.token_balance(&token), 553 * ONE_T22);
    assert_hodl_error(env.sponsored(withdraw(ONE_T22), key), HodlError::Unhealthy);
}
```

Create `programs/hodl_loans/tests/xstocks.rs`:

```rust
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
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test collateral`
Expected: `error[E0560]: struct hodl_loans::instruction::ListCollateral has no field named kind` — the instruction does not take a kind yet.

- [ ] **Step 3: Implement**

Replace `programs/hodl_loans/src/token/extensions.rs`:

```rust
use anchor_lang::prelude::*;
use anchor_spl::token_2022::spl_token_2022::{
    extension::{
        default_account_state::DefaultAccountState, transfer_hook::TransferHook,
        BaseStateWithExtensions, ExtensionType, StateWithExtensions,
    },
    state::{AccountState, Mint as MintState},
};

use crate::errors::HodlError;
use crate::state::CollateralKind;

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

/// Extensions an `XStock` collateral mint may carry (spec §14). The live Backed xStock mints
/// (AAPLX, TSLAX, NVDAX, checked 2026-09-18) carry exactly this set, `TransferHook` with no
/// program and `DefaultAccountState` at `Initialized`. Accepted issuer risks: the permanent
/// delegate can move or burn tokens from the custody vault, a pause blocks every transfer of
/// the asset, and the issuer may later freeze newly created accounts by default.
pub const XSTOCK_COLLATERAL_EXTENSIONS: &[ExtensionType] = &[
    ExtensionType::MetadataPointer,
    ExtensionType::TokenMetadata,
    ExtensionType::PermanentDelegate,
    ExtensionType::Pausable,
    ExtensionType::ScaledUiAmount,
    ExtensionType::ConfidentialTransferMint,
    ExtensionType::TransferHook,
    ExtensionType::DefaultAccountState,
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

/// The mint policy for a collateral kind, checked at listing and again before every transfer
/// of the asset, so an issuer that turns something on after listing causes a clean failure
/// instead of an unnoticed one.
pub fn require_collateral_mint(mint: &AccountInfo, kind: CollateralKind) -> Result<()> {
    match kind {
        CollateralKind::Standard => require_allowed_extensions(mint, STANDARD_COLLATERAL_EXTENSIONS),
        CollateralKind::XStock => require_xstock_mint(mint),
    }
}

/// An `XStock` mint's allowed extensions, plus the two settings whose *values* matter:
/// a transfer hook must name no program, and accounts must not be frozen by default.
fn require_xstock_mint(mint: &AccountInfo) -> Result<()> {
    require_allowed_extensions(mint, XSTOCK_COLLATERAL_EXTENSIONS)?;
    let data = mint.try_borrow_data()?;
    let state = StateWithExtensions::<MintState>::unpack(&data)
        .map_err(|_| HodlError::UnsupportedMintExtension)?;
    if let Ok(hook) = state.get_extension::<TransferHook>() {
        // A hook program would run issuer code inside every transfer of the collateral.
        require!(
            Option::<Pubkey>::from(hook.program_id).is_none(),
            HodlError::UnsupportedMintExtension
        );
    }
    if let Ok(default_state) = state.get_extension::<DefaultAccountState>() {
        // Frozen by default would freeze any token account created after the flip.
        require!(
            default_state.state == u8::from(AccountState::Initialized),
            HodlError::UnsupportedMintExtension
        );
    }
    Ok(())
}
```

In `programs/hodl_loans/src/instructions/admin/collateral_admin.rs`, swap the import:

```rust
use crate::token::extensions::require_collateral_mint;
```

and replace `handle_list_collateral`'s signature, its check, the `kind` field of the `CollateralAsset` literal, and the event:

```rust
pub fn handle_list_collateral(
    ctx: Context<ListCollateral>,
    params: CollateralParams,
    kind: CollateralKind,
) -> Result<()> {
    params.validate(ctx.accounts.config.promo_cap_bps)?;
    require_collateral_mint(&ctx.accounts.mint.to_account_info(), kind)?;
```

```rust
        kind,
```

```rust
    emit!(CollateralListed {
        collateral: ctx.accounts.collateral.key(),
        mint: ctx.accounts.mint.key(),
        vault: ctx.accounts.vault.key(),
        kind,
        params,
    });
```

In `programs/hodl_loans/src/events.rs`, import `CollateralKind` and add the field to `CollateralListed`:

```rust
use crate::state::{CollateralKind, CollateralParams, MarketParams};
```

```rust
#[event]
pub struct CollateralListed {
    pub collateral: Pubkey,
    pub mint: Pubkey,
    pub vault: Pubkey,
    pub kind: CollateralKind,
    pub params: CollateralParams,
}
```

In `programs/hodl_loans/src/lib.rs`, replace the `list_collateral` entry point:

```rust
    pub fn list_collateral(
        ctx: Context<ListCollateral>,
        params: CollateralParams,
        kind: CollateralKind,
    ) -> Result<()> {
        instructions::handle_list_collateral(ctx, params, kind)
    }
```

Add the transfer-time re-check to the four instructions that move collateral. In `programs/hodl_loans/src/instructions/positions/deposit_collateral.rs`, add the import and the check immediately before `transfer_from_user`:

```rust
use crate::token::extensions::require_collateral_mint;
```

```rust
    // An issuer can turn something on after listing, so every transfer re-checks the mint.
    require_collateral_mint(&ctx.accounts.mint.to_account_info(), ctx.accounts.collateral.kind)?;
```

In `programs/hodl_loans/src/instructions/positions/withdraw_collateral.rs`, the same import, and the check immediately before the `let seeds` line that precedes `transfer_from_vault`:

```rust
    require_collateral_mint(&ctx.accounts.mint.to_account_info(), ctx.accounts.collateral.kind)?;
```

In `programs/hodl_loans/src/instructions/admin/sweep.rs`, the same import, and the check in `handle_sweep_collateral_excess` immediately after the `require!(excess > 0, …)` line:

```rust
    require_collateral_mint(&ctx.accounts.mint.to_account_info(), ctx.accounts.collateral.kind)?;
```

In `programs/hodl_loans/src/instructions/liquidation/liquidate.rs`, the same import, and the check after the liquidator's cNGN transfer and immediately before the `let seeds` line that precedes the seizure:

```rust
    require_collateral_mint(&ctx.accounts.collateral_mint.to_account_info(), ctx.accounts.collateral.kind)?;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, 150 tests in all (`xstocks` is new with 7, `withdraw` is now 6).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: per-kind collateral mint policy with transfer-time re-checks"
```

---

### Task 2: Reading an xStock's multiplier

**Files:**
- Create: `programs/hodl_loans/src/token/scaled_ui.rs`
- Modify: `programs/hodl_loans/src/constants.rs`, `programs/hodl_loans/src/token/mod.rs`

**Interfaces:**
- Consumes: `CollateralKind`, `HodlError::{InvalidPrice, UnsupportedMintExtension}`, `anchor_spl::token_2022::spl_token_2022::extension::scaled_ui_amount::ScaledUiAmountConfig`.
- Produces:
  - `constants::MULTIPLIER_SCALE: u128 = 1_000_000_000_000`, `constants::MULTIPLIER_ONE: u128`, `constants::MAX_MULTIPLIER: u128 = 1_000_000 × MULTIPLIER_SCALE`
  - `token::scaled_ui::read_multiplier(mint: &AccountInfo, kind: CollateralKind, now: i64) -> Result<u128>` — `MULTIPLIER_ONE` for a `Standard` asset, the effective scaled-UI factor for an `XStock`

- [ ] **Step 1: Add the constants, wire the module, and write the failing tests**

Add the three constants to `programs/hodl_loans/src/constants.rs`, after `MAX_PRICE_AGE_SECONDS`:

```rust
/// Fixed-point scale for an xStock's scaled-UI multiplier (10^12 per whole multiple).
pub const MULTIPLIER_SCALE: u128 = 1_000_000_000_000;
/// The multiplier a `Standard` asset always carries.
pub const MULTIPLIER_ONE: u128 = MULTIPLIER_SCALE;
/// Largest multiplier an xStock mint may declare. A corporate action moves it by small
/// factors; anything beyond this is a misconfigured or hostile mint, and large values would
/// push `amount × multiplier` towards overflow.
pub const MAX_MULTIPLIER: u128 = 1_000_000 * MULTIPLIER_SCALE;
```

In `programs/hodl_loans/src/token/mod.rs`:

```rust
pub mod extensions;
pub mod scaled_ui;
pub mod transfer;
```

Create `programs/hodl_loans/src/token/scaled_ui.rs` with the header and the tests only, so the module compiles against nothing yet:

```rust
use anchor_lang::prelude::*;
use anchor_spl::token_2022::spl_token_2022::{
    extension::{scaled_ui_amount::ScaledUiAmountConfig, BaseStateWithExtensions, StateWithExtensions},
    state::Mint as MintState,
};

use crate::constants::{MAX_MULTIPLIER, MULTIPLIER_ONE, MULTIPLIER_SCALE};
use crate::errors::HodlError;
use crate::state::CollateralKind;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multipliers_scale_to_twelve_decimals() {
        assert_eq!(scale_multiplier(1.0).unwrap(), MULTIPLIER_SCALE);
        assert_eq!(scale_multiplier(1.000123).unwrap(), 1_000_123_000_000);
        // A dividend-reinvestment factor just above 1, as the live AAPLX mint carries. The
        // conversion truncates, so a factor the binary float cannot hold exactly lands one
        // unit low — which undervalues collateral by 10^-12 of a token, in the protocol's
        // favour, and never the borrower's.
        assert_eq!(scale_multiplier(1.0009).unwrap(), 1_000_899_999_999);
        assert_eq!(scale_multiplier(0.5).unwrap(), 500_000_000_000);
    }

    #[test]
    fn unusable_multipliers_are_rejected() {
        assert!(scale_multiplier(0.0).is_err());
        assert!(scale_multiplier(-1.0).is_err());
        assert!(scale_multiplier(f64::NAN).is_err());
        assert!(scale_multiplier(f64::INFINITY).is_err());
        // Above MAX_MULTIPLIER (1,000,000).
        assert!(scale_multiplier(1_000_001.0).is_err());
        // Positive but rounds to zero at 10^12.
        assert!(scale_multiplier(1e-13).is_err());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `./scripts/test.sh`
Expected: `error[E0425]: cannot find function scale_multiplier in this scope` — ten times, once per assertion.

- [ ] **Step 3: Implement**

Add the implementation above the test module in `programs/hodl_loans/src/token/scaled_ui.rs`:

```rust
use anchor_lang::prelude::*;
use anchor_spl::token_2022::spl_token_2022::{
    extension::{scaled_ui_amount::ScaledUiAmountConfig, BaseStateWithExtensions, StateWithExtensions},
    state::Mint as MintState,
};

use crate::constants::{MAX_MULTIPLIER, MULTIPLIER_ONE, MULTIPLIER_SCALE};
use crate::errors::HodlError;
use crate::state::CollateralKind;

/// The multiplier to apply to raw token amounts, at `MULTIPLIER_SCALE`.
///
/// `Standard` assets are always 1. For an `XStock`, the issuer's `ScaledUiAmount` extension
/// carries the factor its corporate actions (dividend reinvestment, splits) apply to balances:
/// `new_multiplier` once `now` reaches `new_multiplier_effective_timestamp`, else `multiplier`.
/// The stored value never moves into `multiplier` on its own, so reading that field alone goes
/// stale the moment a scheduled change takes effect.
pub fn read_multiplier(mint: &AccountInfo, kind: CollateralKind, now: i64) -> Result<u128> {
    if kind == CollateralKind::Standard {
        return Ok(MULTIPLIER_ONE);
    }
    let data = mint.try_borrow_data()?;
    let state = StateWithExtensions::<MintState>::unpack(&data)
        .map_err(|_| HodlError::UnsupportedMintExtension)?;
    let config = state
        .get_extension::<ScaledUiAmountConfig>()
        .map_err(|_| HodlError::UnsupportedMintExtension)?;
    let raw: f64 = if now >= i64::from(config.new_multiplier_effective_timestamp) {
        config.new_multiplier.into()
    } else {
        config.multiplier.into()
    };
    scale_multiplier(raw)
}

/// Convert the extension's `f64` to `MULTIPLIER_SCALE` fixed point, rejecting anything that
/// cannot price collateral: not finite, not positive, above `MAX_MULTIPLIER`, or so small it
/// rounds to nothing.
fn scale_multiplier(raw: f64) -> Result<u128> {
    require!(raw.is_finite() && raw > 0.0, HodlError::InvalidPrice);
    let scaled = raw * MULTIPLIER_SCALE as f64;
    require!(scaled <= MAX_MULTIPLIER as f64, HodlError::InvalidPrice);
    let scaled = scaled as u128;
    require!(scaled > 0, HodlError::InvalidPrice);
    Ok(scaled)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, 152 tests in all (the unit suite is now 40).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: read an xStock mint's effective scaled-UI multiplier"
```

---

### Task 3: Pricing collateral at its display amount

**Files:**
- Modify (under `programs/hodl_loans/`): `src/math/health.rs`, `src/valuation.rs`, `tests/common/mod.rs`, `tests/xstocks.rs`, `tests/budget.rs`

**Interfaces:**
- Consumes: `read_multiplier` (Task 2), `MULTIPLIER_ONE`, `MULTIPLIER_SCALE`, `CollateralKind`, `math::checked::mul_div_floor`.
- Produces:
  - `CollateralValue.multiplier: u128` and `CollateralValue::display_amount() -> Result<u128>`; `compute_health` prices the display amount
  - `valuation::ACCOUNTS_PER_XSTOCK: usize = 3`; `load_collateral_values` walks a cursor and requires it to land exactly on `remaining.len()`
  - Harness: `Env::collateral_accounts(mint)` returns one asset's health accounts by kind, and `Env::price_accounts` is built from it; `Env::set_multiplier(mint, multiplier, effective_at)`; `legacy_tx_size`, `PACKET_DATA_SIZE`

- [ ] **Step 1: Update the harness and write the failing tests**

In `programs/hodl_loans/tests/common/mod.rs`, replace `Env::price_accounts` with it and its new helper:

```rust
    /// The health accounts for every used collateral slot, in slot order: a pair per
    /// `Standard` asset, and the mint as well for an `XStock`.
    pub fn price_accounts(&self, owner: &Pubkey) -> Vec<AccountMeta> {
        let position = self.position(owner);
        position
            .collateral
            .iter()
            .filter(|slot| slot.amount > 0)
            .flat_map(|slot| self.collateral_accounts(&slot.mint))
            .collect()
    }

    /// One listed asset's health accounts: `(CollateralAsset, PriceUpdateV2)`, plus the mint
    /// when the asset is an `XStock` (its multiplier lives there).
    pub fn collateral_accounts(&self, mint: &Pubkey) -> Vec<AccountMeta> {
        let mut metas = price_pairs(&[*mint]);
        if self.collateral(mint).kind == hodl_loans::CollateralKind::XStock {
            metas.push(AccountMeta::new_readonly(*mint, false));
        }
        metas
    }
```

Append a second new section at the end of the file:

```rust
// ---- The xStock multiplier (Task 3) ----

/// Serialized size of a legacy transaction carrying `instruction`, signed `signers` times.
/// Solana's packet limit is 1,232 bytes; above it the client needs a v0 transaction with an
/// address lookup table.
pub fn legacy_tx_size(instruction: &Instruction, payer: &Pubkey, signers: usize) -> usize {
    1 + 64 * signers + Message::new(std::slice::from_ref(instruction), Some(payer)).serialize().len()
}

pub const PACKET_DATA_SIZE: usize = 1_232;

impl Env {
    /// Schedules the issuer's next multiplier. `effective_at` in the past takes effect at once.
    pub fn set_multiplier(&mut self, mint: &Pubkey, multiplier: f64, effective_at: i64) {
        let authority = self.admin.pubkey();
        let instruction = scaled_ui_amount::instruction::update_multiplier(
            &TOKEN_2022, mint, &authority, &[], multiplier, effective_at,
        )
        .unwrap();
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("update multiplier");
    }
}
```

Append two tests to `programs/hodl_loans/tests/xstocks.rs`, with the two borrow ceilings they need added under the existing `const DAY`:

```rust
/// 10 shares of a $200 stock at 50% LTV back $1,000, which is 1,598,401 cNGN at the ask.
const CEILING: u64 = 1_598_401 * ONE_CNGN;
/// The same position once a 1.5 multiplier makes the balance 15 shares.
const CEILING_AT_1_5: u64 = 2_397_602 * ONE_CNGN;
```

```rust
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
```

In `programs/hodl_loans/tests/budget.rs`, add `use solana_signer::Signer;` under the existing `use common::*;` and append:

```rust
#[test]
fn an_all_xstock_position_stays_under_the_default_compute_budget() {
    let (mut env, setup) = Env::loan_ready();
    let borrower = env.new_borrower();
    let owner = borrower.pubkey();
    let borrower_cngn = env.create_token_account(&setup.cngn, &owner);
    let setup = LoanSetup { borrower, borrower_cngn, ..setup };

    // All 8 collateral slots hold an xStock priced at $200 a share, so 8 shares is $1,600.
    for _ in 0..8 {
        let mint = env.list_xstock_collateral(200);
        env.deposit_collateral(&setup.borrower, &mint, 8 * ONE_XSTOCK);
    }
    assert_eq!(env.price_accounts(&owner).len(), 24);

    for i in 0..9 {
        let prices = env.price_accounts(&owner);
        let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, prices);
        send_cu(&mut env.svm, &[ixn], &[&env.admin, &setup.borrower.key]).unwrap_or_else(|e| panic!("take_loan #{i} failed: {e}"));
    }

    let prices = env.price_accounts(&owner);
    let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, prices);
    let cu = send_cu(&mut env.svm, &[ixn], &[&env.admin, &setup.borrower.key]).unwrap();
    assert!(cu < 100_000, "take_loan at 8 xStock slots / 9 existing loans used {cu} CU");

    // Compute is not the binding limit here: at three accounts a slot the transaction no longer
    // fits in a packet, so such a position needs a v0 transaction with an address lookup table.
    let prices = env.price_accounts(&owner);
    let ixn = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN, 30 * DAY, prices);
    let size = legacy_tx_size(&ixn, &env.admin.pubkey(), 2);
    assert!(size > PACKET_DATA_SIZE, "8 xStock slots now fit a legacy transaction ({size} bytes)");
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test xstocks`
Expected: three tests fail — `the_multiplier_scales_borrowing_power` and `a_scheduled_multiplier_takes_effect_on_its_timestamp` with `expected Custom(6011), got InstructionError(0, Custom(6007))`, and Task 1's `a_hook_switched_on_after_listing_blocks_the_liquidation_seizure` with the same `Custom(6007)` in place of the error it expects. All three are `PriceAccountMismatch` from `valuation.rs`: the harness now supplies three accounts for an xStock slot and the program still reads pairs. Task 1's test goes green again with the rest of this task.

- [ ] **Step 3: Implement**

Replace `programs/hodl_loans/src/math/health.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{BPS, MULTIPLIER_SCALE};
use crate::math::checked::{add, mul_div_floor};
use crate::math::price::{token_value, token_value_ceil, UsdPrice};

/// One collateral holding with its price and risk settings.
#[derive(Clone, Copy, Debug)]
pub struct CollateralValue {
    pub amount: u64,
    pub decimals: u8,
    /// The mint's scaled-UI multiplier at `MULTIPLIER_SCALE` (always `MULTIPLIER_ONE` for a
    /// `Standard` asset). Pyth quotes an xStock per display token, so the raw balance is
    /// scaled by it before pricing.
    pub multiplier: u128,
    pub price: UsdPrice,
    pub ltv_bps: u16,
    pub liquidation_threshold_bps: u16,
}

impl CollateralValue {
    /// The balance Pyth's price applies to: raw amount × multiplier, rounded down.
    pub fn display_amount(&self) -> Result<u128> {
        mul_div_floor(self.amount as u128, self.multiplier, MULTIPLIER_SCALE)
    }
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
        let value = token_value(c.display_amount()?, c.decimals, c.price.lower())?;
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
                multiplier: MULTIPLIER_SCALE,
                price: UsdPrice { price: 150 * USD, conf: USD },
                ltv_bps: 7_000,
                liquidation_threshold_bps: 9_000,
            },
            // 500 USDC (6 decimals) at $1.
            CollateralValue {
                amount: 500_000_000,
                decimals: 6,
                multiplier: MULTIPLIER_SCALE,
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
            multiplier: MULTIPLIER_SCALE,
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
    fn an_xstock_multiplier_scales_the_balance_before_pricing() {
        // 100 raw AAPLX (8 decimals) at a 1.5 multiplier is 150 display tokens at $200 = $30,000.
        let split = [CollateralValue {
            amount: 10_000_000_000,
            decimals: 8,
            multiplier: 1_500_000_000_000,
            price: UsdPrice { price: 200 * USD, conf: 0 },
            ltv_bps: 5_000,
            liquidation_threshold_bps: 7_500,
        }];
        let h = compute_health(&split, 0, 6, ngn()).unwrap();
        assert_eq!(h.own_value, 30_000 * USD);
        assert_eq!(h.borrow_limit, 15_000 * USD);
        assert_eq!(h.liquidation_line, 22_500 * USD);
        // The same holding at multiplier 1 is worth the raw balance.
        let plain = [CollateralValue { multiplier: MULTIPLIER_SCALE, ..split[0] }];
        assert_eq!(compute_health(&plain, 0, 6, ngn()).unwrap().own_value, 20_000 * USD);
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
use crate::constants::MULTIPLIER_ONE;
use crate::state::{CollateralAsset, CollateralKind, Market, Position};
use crate::token::scaled_ui::read_multiplier;

/// Accounts per used collateral slot in `remaining_accounts`: `(CollateralAsset, PriceUpdateV2)`
/// for a `Standard` asset, and `(CollateralAsset, PriceUpdateV2, mint)` for an `XStock`, whose
/// scaled-UI multiplier lives on the mint.
pub const ACCOUNTS_PER_COLLATERAL: usize = 2;
pub const ACCOUNTS_PER_XSTOCK: usize = 3;

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
    let mut values = Vec::with_capacity(used.len());
    let mut cursor = 0usize;
    for slot in used.iter() {
        let asset_info = remaining.get(cursor).ok_or(HodlError::PriceAccountMismatch)?;
        let price_info = remaining.get(cursor + 1).ok_or(HodlError::PriceAccountMismatch)?;
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
        // An xStock passes its mint too: the multiplier its issuer applies to balances lives
        // there, and Pyth prices the display token, not the raw unit.
        let multiplier = if asset.kind == CollateralKind::Standard {
            cursor += ACCOUNTS_PER_COLLATERAL;
            MULTIPLIER_ONE
        } else {
            let mint_info = remaining.get(cursor + 2).ok_or(HodlError::PriceAccountMismatch)?;
            require_keys_eq!(mint_info.key(), slot.mint, HodlError::PriceAccountMismatch);
            cursor += ACCOUNTS_PER_XSTOCK;
            read_multiplier(mint_info, asset.kind, clock.unix_timestamp)?
        };
        values.push(CollateralValue {
            amount: slot.amount,
            decimals: asset.decimals,
            multiplier,
            price,
            ltv_bps: asset.ltv_bps,
            liquidation_threshold_bps: asset.liquidation_threshold_bps,
        });
    }
    // Every account supplied must have been consumed: an extra one is a mismatch.
    require!(cursor == remaining.len(), HodlError::PriceAccountMismatch);
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

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, 156 tests in all (`xstocks` is now 9, `budget` 2, the unit suite 41).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: price collateral at its display amount"
```

---

### Task 4: Seizing at the display price

**Files:**
- Modify (under `programs/hodl_loans/`): `src/math/liquidation.rs`, `src/instructions/liquidation/liquidate.rs`, `tests/xstocks.rs`

**Interfaces:**
- Consumes: `CollateralValue.multiplier` (Task 3), `MULTIPLIER_SCALE`, `MULTIPLIER_ONE`.
- Produces: `seize_for_repayment(repay_amount, ngn_price, cngn_decimals, collateral_price, collateral_decimals, multiplier, bonus_bps, slot_amount) -> Result<Seizure>` — the `multiplier` argument is new, sixth, and converts the display amount back to raw units.

- [ ] **Step 1: Write the failing test**

Append to `programs/hodl_loans/tests/xstocks.rs`:

```rust
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
```

- [ ] **Step 2: Run it to verify it fails**

Run: `./scripts/test.sh --test xstocks`
Expected: `liquidating_an_xstock_seizes_at_the_display_price` fails with `left: 275000000, right: 412500000` — the first seizure is right (the multiplier is 1), and the second takes 137,500,000 raw units instead of 275,000,000, because the seizure still treats raw units as display units.

- [ ] **Step 3: Implement**

Replace `programs/hodl_loans/src/math/liquidation.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{BPS, MULTIPLIER_SCALE};
use crate::errors::HodlError;
use crate::math::checked::{add, mul_div_floor, to_u64};

/// How much cNGN a liquidator pays and how much collateral it takes for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seizure {
    pub repay_amount: u64,
    pub seize_amount: u64,
}

/// Spec §11 seizure, at plain prices (no confidence or spread adjustment):
///
/// ```text
/// seize = repay × ngn_price × (BPS + bonus) × 10^collateral_decimals × MULTIPLIER_SCALE
///         / (10^cngn_decimals × collateral_price × BPS × multiplier)
/// ```
///
/// `multiplier` is the asset's scaled-UI factor (`MULTIPLIER_ONE` for a `Standard` asset):
/// Pyth quotes an xStock per display token, so the seizure converts back to raw units.
///
/// The division is interleaved so a large repayment cannot overflow `u128`, and every step
/// rounds down, so the liquidator never receives more collateral than the formula allows.
/// When the slot holds less than that, the seizure takes the whole slot and the repayment
/// shrinks in proportion (spec §11 step 6).
#[allow(clippy::too_many_arguments)]
pub fn seize_for_repayment(
    repay_amount: u64,
    ngn_price: u128,
    cngn_decimals: u8,
    collateral_price: u128,
    collateral_decimals: u8,
    multiplier: u128,
    bonus_bps: u16,
    slot_amount: u64,
) -> Result<Seizure> {
    require!(collateral_price > 0 && ngn_price > 0, HodlError::InvalidPrice);
    require!(multiplier > 0, HodlError::InvalidPrice);
    let repaid_usd = mul_div_floor(repay_amount as u128, ngn_price, pow10(cngn_decimals)?)?;
    let with_bonus = mul_div_floor(repaid_usd, add(BPS, bonus_bps as u128)?, BPS)?;
    // The price is per display token, so the display amount converts back to raw units.
    let display = mul_div_floor(with_bonus, pow10(collateral_decimals)?, collateral_price)?;
    let seize = mul_div_floor(display, MULTIPLIER_SCALE, multiplier)?;

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
    use crate::constants::MULTIPLIER_ONE;

    const USD: u128 = 1_000_000_000_000;
    /// 1 NGN = $0.000625.
    const NGN: u128 = 625_000_000;
    /// 1,600,000 cNGN, worth $1,000.
    const REPAY: u64 = 1_600_000_000_000;

    #[test]
    fn seizure_pays_the_bonus_on_top_of_the_repaid_value() {
        // 1,050 USDC (6 decimals at $1) for $1,000 of cNGN at a 5% bonus.
        let s = seize_for_repayment(REPAY, NGN, 6, USD, 6, MULTIPLIER_ONE, 500, u64::MAX).unwrap();
        assert_eq!(s, Seizure { repay_amount: REPAY, seize_amount: 1_050_000_000 });

        // The same $1,050 is 7 SOL (9 decimals at $150).
        let s = seize_for_repayment(REPAY, NGN, 6, 150 * USD, 9, MULTIPLIER_ONE, 500, u64::MAX).unwrap();
        assert_eq!(s.seize_amount, 7_000_000_000);

        // No bonus seizes exactly the repaid value.
        let s = seize_for_repayment(REPAY, NGN, 6, USD, 6, MULTIPLIER_ONE, 0, u64::MAX).unwrap();
        assert_eq!(s.seize_amount, 1_000_000_000);
    }

    #[test]
    fn a_full_bonus_still_scales_the_seizure() {
        // A 100% bonus doubles the collateral seized for the same repayment.
        let s = seize_for_repayment(REPAY, NGN, 6, USD, 6, MULTIPLIER_ONE, 10_000, u64::MAX).unwrap();
        assert_eq!(s.seize_amount, 2_000_000_000);
    }

    #[test]
    fn a_short_slot_caps_both_the_seizure_and_the_repayment() {
        // The slot holds 500 USDC of the 1,050 the full repayment would take.
        let s = seize_for_repayment(REPAY, NGN, 6, USD, 6, MULTIPLIER_ONE, 500, 500_000_000).unwrap();
        assert_eq!(s.seize_amount, 500_000_000);
        // 1,600,000 × 500 / 1,050 cNGN, rounded down.
        assert_eq!(s.repay_amount, 761_904_761_904);
        // Re-pricing the capped repayment seizes no more than the slot.
        let again = seize_for_repayment(s.repay_amount, NGN, 6, USD, 6, MULTIPLIER_ONE, 500, 500_000_000).unwrap();
        assert!(again.seize_amount <= 500_000_000);
    }

    #[test]
    fn seizure_rejects_missing_prices() {
        assert!(seize_for_repayment(REPAY, NGN, 6, 0, 6, MULTIPLIER_ONE, 500, u64::MAX).is_err());
        assert!(seize_for_repayment(REPAY, 0, 6, USD, 6, MULTIPLIER_ONE, 500, u64::MAX).is_err());
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

In `programs/hodl_loans/src/instructions/liquidation/liquidate.rs`, read the slot's multiplier from the valuation, next to its price:

```rust
        let collateral_price = valuation.collateral[priced].price.price;
        let multiplier = valuation.collateral[priced].multiplier;
```

and pass it to `seize_for_repayment`, between the collateral decimals and the bonus:

```rust
        let seizure = seize_for_repayment(
            requested,
            valuation.ngn.price,
            market.decimals,
            collateral_price,
            ctx.accounts.collateral.decimals,
            multiplier,
            ctx.accounts.collateral.liquidation_bonus_bps,
            position.collateral[slot_index].amount,
        )?;
```

- [ ] **Step 4: Run the tests to verify they pass, then the full suite and lints**

Run: `./scripts/test.sh --test xstocks`
Expected: 10 tests, all `ok`.

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, 157 tests in all — 41 unit and 116 LiteSVM.

Run: `cargo clippy -p hodl_loans --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: seize xStock collateral at the display price"
```

---

## Done when

- `./scripts/test.sh` reports 157 passing tests and `cargo clippy -p hodl_loans --all-targets -- -D warnings` is clean.
- A live-shaped xStock mint lists only as `XStock`; a hook program or a frozen default is rejected at listing and again at every transfer.
- A position's borrowing power follows the mint's effective multiplier, including one scheduled for a future timestamp.
- A liquidator seizing an xStock receives raw units worth the display value it paid for, at any multiplier.
- The issuer's powers — pause, permanent delegate, transfer hook, default account state — each have a test showing exactly what they do to the protocol.

## Deliberately not in this plan

- **Promo balance in health and promo forfeiture on liquidation** (spec §11 step 3, §12): Plan 5.
- **A deposit cap per xStock** already exists as `CollateralParams.deposit_cap` from Plan 2; choosing the launch numbers is a deployment decision, not code.
- **Reacting to an issuer action.** The program fails cleanly when a mint changes under it; deciding whether to delist, unpin or pause is an operational call for the admin multisig.
- **The redemption-rate feed** (`Crypto.AAPLX/AAPL.RR`), which measures an xStock's premium or discount against the share behind it. Pricing off it, or bounding it, is a Plan 6 question.
- **Trident invariants over the multiplier** (a corporate action mid-loan): Plan 6.
