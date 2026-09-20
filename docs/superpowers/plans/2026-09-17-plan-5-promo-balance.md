# Plan 5: Promo Balance — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** HODL can hand a borrower shop-only borrowing power that is backed by real cNGN, counts only as a topping on collateral they put up themselves, goes to lenders if they default, and comes back if they never use it.

**Architecture:**
- **Builds on Plan 4** (`main` at `ba979df`): lender pool, collateral, prices, loans, liquidation, write-off, xStocks.
- **Promo is money, not a number.** Every unit of promo a position holds is backed by cNGN sitting in a `PromoVault`, and the invariant `outstanding + unissued ≤ cash` is checked on every write. Nothing is ever granted that the vault cannot cover.
- **Three states for the same cNGN.** *Free* (nothing claims it), *unissued* (a campaign has reserved it), *outstanding* (a position holds it). Creating a campaign moves free → unissued; redeeming a voucher moves unissued → outstanding; expiry and revocation move it back to free; forfeiture moves the tokens themselves out to the market vault.
- **The program builds the voucher message it expects.** It never parses one the caller supplies: the domain, the program id and the market are pinned by construction, and an Ed25519 instruction earlier in the transaction must have signed exactly those bytes.
- **Promo is a topping, never a substitute.** It counts at a fraction of the borrower's *own* collateral value, so a position holding nothing of its own counts none of it — which is what makes defaulting a loss for the borrower rather than a way to profit.

**Tech Stack:** Rust 1.89+, Anchor 1.2.0, Solana CLI 3.1.x (`cargo build-sbf`, platform tools v1.52), LiteSVM 0.10.0 with the `precompiles` feature, `solana-instructions-sysvar` 3, `spl-token-2022-interface` 2.1.0, `pyth-solana-receiver-sdk` 2.0.0, `switchboard-on-demand` 0.13.0.

**Spec:** `docs/superpowers/specs/2026-09-17-solana-fixed-loans-design.md`

## Plan series

| Plan | Delivers |
|---|---|
| 1. Foundation and lender pool (merged) | Workspace, math, roles, whitelist/blacklist, cNGN market, lender deposit/withdraw, donation sweep |
| 2. Collateral, prices and loans (merged) | `CollateralAsset`, positions, Pyth and Switchboard reads, health checks, `take_loan`, `repay_loan`, reserve harvest |
| 3. Liquidation and bad debt (merged) | Pinned price accounts, seizure math, `liquidate`, `write_off_loan` (reserve first) |
| 4. xStocks (merged) | Per-kind mint policy, scaled-UI multiplier, display-amount pricing, display-price seizure |
| **5. Promo balance** (this plan) | `PromoVault`, campaigns, Ed25519 vouchers, promo in the health check, expiry and revocation, forfeiture, `set_promo_cap` |
| 6. Hardening and devnet | Trident invariant fuzzing, ported EVM regressions, devnet run, pre-audit scan |

## Global Constraints

> **The tree is authoritative where this plan and the code disagree.** Eight task reviews, their
> fix rounds and a final whole-branch review all changed code after this plan was generated from
> it. Most of those changes are improvements the plan simply predates — a dedicated error variant,
> an invariant call moved inside a helper, an extra field on an event — and following the plan
> verbatim would produce working software without them. Two were different in kind, because
> following the plan would have reintroduced a real defect, and both have been written back into
> the text above: `compute_health` floors the promo cap **per asset**, never once on the
> aggregate, and `forfeit_promo` **clamps** its transfer to the balance the vault actually holds.
> The reasoning for both is in `2026-09-17-plan-5-followups.md` under "Settled on this branch".


Carried from Plans 1–4:

- Anchor `1.2.0` for `anchor-lang` and `anchor-spl`; LiteSVM `0.10.0`; Rust `1.89` or newer; build with `cargo build-sbf --tools-version v1.52` (through `scripts/test.sh`).
- `[profile.release] overflow-checks = true`; all math on `u128` with checked operations. Every new arithmetic site uses `checked_*` or the `math::checked` helpers.
- Rounding: borrower debt rounds up; lender accrual and minted shares round down; burned shares round up. A display amount rounds down.
- Constants: `BPS = 10_000`, `YEAR = 31_536_000` seconds, `VIRTUAL_SHARES = 1_000`, `VIRTUAL_ASSETS = 1`, `MIN_TENURE = 86_400`, `MAX_COLLATERAL_SLOTS = 8`, `MAX_LOAN_SLOTS = 10`, `USD_SCALE = 10^12`, `MAX_PRICE_AGE_SECONDS = 60`, `MAX_BAD_DEBT_DUST_USD = 1_000 × USD_SCALE`, `MULTIPLIER_SCALE = 10^12`, `MAX_MULTIPLIER = 10^6 × MULTIPLIER_SCALE`.
- Every account starts with `version: u8` and `bump: u8` and ends with reserved padding; new fields come out of that padding, so account sizes don't change. **This plan adds no fields to an existing account** — `Position.promo_balance` and `promo_last_activity_at`, `Config.promo_cap_bps` and `promo_signer`, and both `MarketParams` promo fields already exist, unused, from Plans 1 and 2.
- One `#[error_code] HodlError` enum. **Append new variants only**, since codes are `6000 + position`. **This plan adds two**, both out of review rather than the original design: `PromoAccountsRequired` (a required-but-optional promo account was not supplied — distinct from a vault that is genuinely short) and `PromoVaultMismatch` (the promo token account named does not belong to the vault). The rest were reserved in Plan 1: `InvalidVoucherSignature`, `VoucherExpired`, `CampaignInactive`, `CampaignBudgetExceeded`, `PromoCapExceeded`, `PromoNotExpired` and `PromoVaultInsufficient`.
- Collateral counts at `price − confidence` (round down); debt counts at `NGN price + spread` (round up).
- Commits made by Claude end with the attribution trailer from the session instructions.

New in this plan:

- **Box every account in a large instruction, `Config` included.** The SBF stack frame is 4 KB, and this plan pushed `liquidate` over it: an unboxed `Config` was enough to fail with `Access violation in stack frame 5` before the handler ran a single line. The rule was already "Box `Market`, `LenderPosition`, `CollateralAsset` and every `InterfaceAccount`"; `Config` now joins it wherever the instruction is big.
- **The promo vault invariant is re-checked after every write.** `PromoVault::require_invariant` asserts `outstanding + unissued ≤ cash`; call it at the end of any handler that moves one of the three.
- **Every market has a promo vault.** `take_loan`, `liquidate` and `write_off_loan` all name it, so a market without one cannot be borrowed against. The test harness creates one with every market.
- **Redeeming binds a position to a market**, exactly as its first loan does — promo is backed by one market's vault, and expiry has to know which vault to credit for a position that has never borrowed.

## Facts verified while writing this plan (2026-09-18)

- **The whole plan was built and tested before it was written:** 234 tests pass (52 unit, 182 LiteSVM), `cargo clippy -p hodl_loans --all-targets -- -D warnings` is clean, every task's end state was rebuilt from Plan 4's head and passes its own suite and clippy, and each task's failing-test step was run to capture its real errors.
- **`liquidate` overflows the SBF stack before it overflows compute.** Adding the promo accounts made it fail with `Access violation in stack frame 5 at address 0x200005ff8`, having burned only 12,719 CU — an account-construction failure, not a compute one. Boxing `Config` fixed it; grouping the forfeiture helper's arguments into a struct was not enough on its own.
- **LiteSVM does not load the native Ed25519 program by default.** Without `features = ["precompiles"]` every voucher redemption fails with `InvalidProgramForExecution`, which looks like a program bug and is not one.
- **Anchor 1.2.0's `solana_program` shim re-exports neither `ed25519_program` nor the instructions-sysvar loaders.** `solana-instructions-sysvar` 3 is already in the lockfile through `anchor-lang`, so depending on it directly costs no version churn; the Ed25519 program address is pinned as a constant and matches `solana_sdk_ids::ed25519_program::ID`.
- **Compute at full load is measured in `tests/budget.rs`, and the numbers are not repeated here.** They moved five times during execution and every copy outside that file went stale; the file records each figure beside the assertion that guards it. Two things worth knowing that the figures alone do not say: the measurements are **not deterministic** — the harness keys its mints randomly, so where a target sorts into the collateral slot array shifts the scan and the cost moves in steps of about 1,500 CU, which is why they are recorded as ranges — and compute is not the binding constraint anywhere. Transaction *size* is: `liquidate` sits within ~50 bytes of the 1,232-byte legacy limit at 8 standard slots.
- **A `min` is what caps promo, not a `require`.** `promo_counted = min(promo_value, own_value × promo_cap_bps / BPS)`, so a borrower holding more promo than their collateral supports is not rejected — the surplus simply does not count. That is what makes `promo_is_worth_nothing_to_a_position_holding_no_collateral` pass rather than error.

## Plan-level refinements to the spec

This plan's commit already writes these into the spec.

- **§12 gains `create_promo_vault`.** Spec §12 lists funding and withdrawal but never says where the vault comes from. Making it an explicit admin instruction — rather than `init_if_needed` inside `fund_promo_vault` — keeps the re-initialisation question from arising at all on an account that holds funds.
- **§12 gains `sweep_promo_excess`.** The market and collateral vaults both have a sweep; without one, a direct transfer into the promo vault's token account is stranded, recoverable only by an upgrade. It is also the third copy of the same body, which is where the shared `sweep_to_treasury` helper comes from (a Plan 2 follow-up).
- **§12: redeeming binds `position.market`.** Promo is backed by one market's vault and counted against its cap, so redemption binds the position exactly as a first loan does. Without it, `expire_promo` could not tell which vault to credit for a position that has never borrowed.
- **§13 lists the two new admin instructions**, and **§17 lists the events this plan adds**: `PromoVaultCreated`, `PromoVaultFunded`, `PromoVaultWithdrawn`, `CampaignCreated`, `CampaignClosed`, `PromoReleased` (promo returned because the position itself is closing) and `PromoForfeited`.
- **§15 records the account-set changes:** `take_loan`, `withdraw_collateral` and `liquidate` now take the `Config` account, because `promo_cap_bps` lives there and every health check needs it; `take_loan` optionally takes the promo vault, and `liquidate` and `write_off_loan` take it and its token account.

## File Structure

New and changed files under `programs/hodl_loans/`:

```text
Cargo.toml                                    + solana-instructions-sysvar; litesvm precompiles
src/constants.rs                              + promo seeds, VOUCHER_DOMAIN
src/state/promo.rs                            new: PromoVault, Campaign, VoucherReceipt
src/state/mod.rs
src/voucher.rs                                new: the voucher message and its Ed25519 check
src/instructions/promos/vault.rs              new: create, fund, withdraw, sweep
src/instructions/promos/campaign.rs           new: create_campaign, close_campaign
src/instructions/promos/redeem.rs             new: redeem_promo, close_voucher_receipt
src/instructions/promos/lifecycle.rs          new: expire, revoke, release, forfeit
src/instructions/promos/cap.rs                new: set_promo_cap
src/instructions/promos/mod.rs                new
src/instructions/mod.rs
src/instructions/admin/sweep.rs               shared sweep_to_treasury helper
src/instructions/loans/take_loan.rs           + Config, optional promo vault, step-3 expiry
src/instructions/positions/withdraw_collateral.rs   + Config
src/instructions/positions/close_position.rs  releases promo
src/instructions/liquidation/liquidate.rs     + Config, promo accounts, forfeiture
src/instructions/liquidation/write_off.rs     + token accounts, promo accounts, forfeiture
src/math/health.rs                            Health.promo_counted
src/valuation.rs                              ValuationRequest
src/events.rs                                 + 11 events
src/lib.rs                                    + 11 entry points
tests/common/mod.rs                           harness: promo vault, campaigns, vouchers, lifecycle
tests/budget.rs, tests/loans.rs, tests/liquidation.rs   updated for the new account sets
tests/promo_vault.rs, tests/campaign.rs, tests/promo_redeem.rs, tests/promo_health.rs,
tests/promo_lifecycle.rs, tests/promo_forfeit.rs, tests/promo_cap.rs      new
```

All paths below are relative to the repository root.

---

### Task 1: The promo vault, and one sweep for three vaults

**Files:**
- Create: `programs/hodl_loans/src/state/promo.rs`, `src/instructions/promos/vault.rs`, `src/instructions/promos/mod.rs`, `tests/promo_vault.rs`
- Modify: `src/constants.rs`, `src/state/mod.rs`, `src/instructions/mod.rs`, `src/instructions/admin/sweep.rs`, `src/events.rs`, `src/lib.rs`, `tests/common/mod.rs`

**Interfaces:**
- Consumes: `Config`, `Market`, `transfer_from_user`, `transfer_from_vault`, `ExcessSwept`, `math::checked::{add, sub}`.
- Produces:
  - `state::PromoVault` with `free()`, `release(amount)` and `require_invariant()`
  - Seeds `PROMO_VAULT_SEED = b"promo_vault"` and `PROMO_VAULT_TOKEN_SEED = b"promo_vault_token"`
  - `instructions::admin::sweep::sweep_to_treasury(...)`, shared by all three vault sweeps
  - `create_promo_vault`, `fund_promo_vault(amount)`, `withdraw_promo_vault(amount)`, `sweep_promo_excess`
  - Harness: `promo_vault_pda`, `promo_vault_token_pda`, the four instruction builders, `Env::with_promo_vault(funded)`, `Env::promo_vault(mint)`, `Env::treasury_token(mint)`, `Env::create_market_with_promo(mint)`; `Env::with_cngn_market` now creates a promo vault too

- [ ] **Step 1: Write the harness and the failing tests**

Add the seeds to `programs/hodl_loans/src/constants.rs`, after `POSITION_SEED`, and add the four new names to the `seeds_are_distinct_and_bps_matches` list:

```rust
#[constant]
pub const PROMO_VAULT_SEED: &[u8] = b"promo_vault";
#[constant]
pub const PROMO_VAULT_TOKEN_SEED: &[u8] = b"promo_vault_token";
#[constant]
pub const CAMPAIGN_SEED: &[u8] = b"campaign";
#[constant]
pub const VOUCHER_SEED: &[u8] = b"voucher";
```

In `programs/hodl_loans/tests/common/mod.rs`, make every market create a promo vault — `take_loan`, `liquidate` and `write_off_loan` all name it by the end of this plan, so a market without one cannot be borrowed against:

```rust
    /// Initialized config plus a cNGN-like market (6 decimals). Returns the mint.
    pub fn with_cngn_market() -> (Self, Pubkey) {
        let mut env = Self::initialized();
        let mint = env.create_mint(MintKind::CngnLike, 6);
        let admin = env.admin.pubkey();
        let instruction = create_market_ix(&admin, &mint, &TOKEN_2022, default_market_params());
        send(&mut env.svm, &[instruction], &[&env.admin]).expect("create market");
        // Every market gets a promo vault, as a deployed one would: `take_loan` may expire promo
        // (spec §10 step 3), so the account is part of its shape whether or not promo is funded.
        send(&mut env.svm, &[create_promo_vault_ix(&admin, &mint)], &[&env.admin])
            .expect("create promo vault");
        (env, mint)
    }
```

and add the helper that does the same for a second market:

```rust
    /// A second market, with the promo vault every market needs: `take_loan`, `liquidate` and
    /// `write_off_loan` all name it, so a market without one cannot be borrowed against.
    pub fn create_market_with_promo(&mut self, mint: &Pubkey) {
        let admin = self.admin.pubkey();
        let create = create_market_ix(&admin, mint, &TOKEN_2022, default_market_params());
        send(&mut self.svm, &[create], &[&self.admin]).expect("create market");
        send(&mut self.svm, &[create_promo_vault_ix(&admin, mint)], &[&self.admin])
            .expect("create promo vault");
    }
```

Append the section:

```rust
// ---- The promo vault (Task 1) ----

pub fn promo_vault_pda(market_mint: &Pubkey) -> Pubkey {
    pda(&[b"promo_vault", market_pda(market_mint).as_ref()])
}

pub fn promo_vault_token_pda(market_mint: &Pubkey) -> Pubkey {
    pda(&[b"promo_vault_token", market_pda(market_mint).as_ref()])
}

pub fn create_promo_vault_ix(admin: &Pubkey, mint: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::CreatePromoVault {},
        hodl_loans::accounts::CreatePromoVault {
            admin: *admin,
            config: config_pda(),
            market: market_pda(mint),
            mint: *mint,
            promo_vault: promo_vault_pda(mint),
            vault: promo_vault_token_pda(mint),
            token_program: TOKEN_2022,
            system_program: system_program::ID,
        },
    )
}

pub fn fund_promo_vault_ix(admin: &Pubkey, mint: &Pubkey, source: &Pubkey, amount: u64) -> Instruction {
    ix(
        hodl_loans::instruction::FundPromoVault { amount },
        hodl_loans::accounts::FundPromoVault {
            admin: *admin,
            config: config_pda(),
            market: market_pda(mint),
            mint: *mint,
            promo_vault: promo_vault_pda(mint),
            vault: promo_vault_token_pda(mint),
            source: *source,
            token_program: TOKEN_2022,
        },
    )
}

pub fn withdraw_promo_vault_ix(admin: &Pubkey, mint: &Pubkey, destination: &Pubkey, amount: u64) -> Instruction {
    ix(
        hodl_loans::instruction::WithdrawPromoVault { amount },
        hodl_loans::accounts::WithdrawPromoVault {
            admin: *admin,
            config: config_pda(),
            market: market_pda(mint),
            mint: *mint,
            promo_vault: promo_vault_pda(mint),
            vault: promo_vault_token_pda(mint),
            destination: *destination,
            token_program: TOKEN_2022,
        },
    )
}

pub fn sweep_promo_excess_ix(admin: &Pubkey, mint: &Pubkey, destination: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::SweepPromoExcess {},
        hodl_loans::accounts::SweepPromoExcess {
            admin: *admin,
            config: config_pda(),
            market: market_pda(mint),
            mint: *mint,
            promo_vault: promo_vault_pda(mint),
            vault: promo_vault_token_pda(mint),
            destination: *destination,
            token_program: TOKEN_2022,
        },
    )
}

impl Env {
    /// A cNGN market whose promo vault holds `funded` cNGN.
    pub fn with_promo_vault(funded: u64) -> (Self, Pubkey) {
        let (mut env, cngn) = Self::with_cngn_market();
        let admin = env.admin.pubkey();
        if funded > 0 {
            let source = env.create_token_account(&cngn, &admin);
            env.mint_to(&cngn, &source, funded);
            let instruction = fund_promo_vault_ix(&admin, &cngn, &source, funded);
            send(&mut env.svm, &[instruction], &[&env.admin]).expect("fund promo vault");
        }
        (env, cngn)
    }

    pub fn promo_vault(&self, market_mint: &Pubkey) -> hodl_loans::PromoVault {
        self.fetch(&promo_vault_pda(market_mint))
    }

    /// A treasury-owned cNGN account, the only destination the sweeps and withdrawals accept.
    pub fn treasury_token(&mut self, mint: &Pubkey) -> Pubkey {
        let treasury = self.treasury.pubkey();
        self.create_token_account(mint, &treasury)
    }
}
```

Create `programs/hodl_loans/tests/promo_vault.rs`:

```rust
mod common;

use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

const FUNDING: u64 = 1_000_000 * ONE_CNGN;

#[test]
fn a_promo_vault_holds_cngn_the_admin_funds_and_can_take_back() {
    let (mut env, cngn) = Env::with_promo_vault(0);
    let admin = env.admin.pubkey();
    let vault = env.promo_vault(&cngn);
    assert_eq!((vault.market, vault.vault), (market_pda(&cngn), promo_vault_token_pda(&cngn)));
    assert_eq!((vault.cash, vault.outstanding, vault.unissued), (0, 0, 0));

    let source = env.create_token_account(&cngn, &admin);
    env.mint_to(&cngn, &source, FUNDING);
    send(&mut env.svm, &[fund_promo_vault_ix(&admin, &cngn, &source, FUNDING)], &[&env.admin]).unwrap();
    assert_eq!(env.promo_vault(&cngn).cash, FUNDING);
    assert_eq!(env.token_balance(&promo_vault_token_pda(&cngn)), FUNDING);

    // Nothing is committed yet, so all of it is free to withdraw.
    let destination = env.treasury_token(&cngn);
    let withdraw = withdraw_promo_vault_ix(&admin, &cngn, &destination, FUNDING);
    send(&mut env.svm, &[withdraw], &[&env.admin]).unwrap();
    assert_eq!(env.promo_vault(&cngn).cash, 0);
    assert_eq!(env.token_balance(&destination), FUNDING);
}

#[test]
fn promo_vault_rejections() {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();
    let stranger = env.funded_keypair();
    let destination = env.treasury_token(&cngn);

    // Creating it twice fails: the PDA already exists.
    let again = create_promo_vault_ix(&admin, &cngn);
    assert!(send(&mut env.svm, &[again], &[&env.admin]).is_err());

    // Every promo vault instruction is admin-only. `create_promo_vault` needs a market that
    // doesn't already have one, so `init` doesn't fail on "already in use" before the
    // authorization check ever runs.
    let other_mint = env.create_mint(MintKind::CngnLike, 6);
    let create_market = create_market_ix(&admin, &other_mint, &TOKEN_2022, default_market_params());
    send(&mut env.svm, &[create_market], &[&env.admin]).expect("create market");
    let by_stranger = create_promo_vault_ix(&stranger.pubkey(), &other_mint);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);
    let source = env.create_token_account(&cngn, &stranger.pubkey());
    let by_stranger = fund_promo_vault_ix(&stranger.pubkey(), &cngn, &source, ONE_CNGN);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);
    let by_stranger = withdraw_promo_vault_ix(&stranger.pubkey(), &cngn, &destination, ONE_CNGN);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);
    let by_stranger = sweep_promo_excess_ix(&stranger.pubkey(), &cngn, &destination);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);

    // Zero moves nothing.
    let zero = fund_promo_vault_ix(&admin, &cngn, &source, 0);
    assert_hodl_error(send(&mut env.svm, &[zero], &[&env.admin]), HodlError::AmountTooSmall);
    let zero = withdraw_promo_vault_ix(&admin, &cngn, &destination, 0);
    assert_hodl_error(send(&mut env.svm, &[zero], &[&env.admin]), HodlError::AmountTooSmall);

    // More than the vault holds fails on our own accounting, before the token program sees it.
    let too_much = withdraw_promo_vault_ix(&admin, &cngn, &destination, FUNDING + 1);
    assert_hodl_error(send(&mut env.svm, &[too_much], &[&env.admin]), HodlError::PromoVaultInsufficient);

    // The destination must belong to the treasury.
    let wrong = env.create_token_account(&cngn, &stranger.pubkey());
    let to_stranger = withdraw_promo_vault_ix(&admin, &cngn, &wrong, ONE_CNGN);
    assert!(send(&mut env.svm, &[to_stranger], &[&env.admin]).is_err());
}

#[test]
fn committed_promo_cannot_be_withdrawn() {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();
    let destination = env.treasury_token(&cngn);

    // Simulate a campaign's reservation and a position's balance against the same cash.
    let mut vault = env.promo_vault(&cngn);
    vault.unissued = 400_000 * ONE_CNGN;
    vault.outstanding = 100_000 * ONE_CNGN;
    env.write(&promo_vault_pda(&cngn), &vault);
    assert_eq!(env.promo_vault(&cngn).free().unwrap(), 500_000 * ONE_CNGN);

    let over = withdraw_promo_vault_ix(&admin, &cngn, &destination, 500_000 * ONE_CNGN + 1);
    assert_hodl_error(send(&mut env.svm, &[over], &[&env.admin]), HodlError::PromoVaultInsufficient);
    let exact = withdraw_promo_vault_ix(&admin, &cngn, &destination, 500_000 * ONE_CNGN);
    send(&mut env.svm, &[exact], &[&env.admin]).unwrap();
    assert_eq!(env.promo_vault(&cngn).cash, 500_000 * ONE_CNGN);
}

#[test]
fn the_promo_sweep_moves_only_donations() {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();
    let destination = env.treasury_token(&cngn);

    // Nothing donated yet.
    let sweep = sweep_promo_excess_ix(&admin, &cngn, &destination);
    assert_hodl_error(send(&mut env.svm, std::slice::from_ref(&sweep), &[&env.admin]), HodlError::AmountTooSmall);

    // A direct transfer to the vault is not promo backing until it is swept.
    env.mint_to(&cngn, &promo_vault_token_pda(&cngn), 7);
    assert_eq!(env.promo_vault(&cngn).cash, FUNDING);
    send(&mut env.svm, &[sweep], &[&env.admin]).unwrap();
    assert_eq!(env.token_balance(&destination), 7);
    assert_eq!(env.promo_vault(&cngn).cash, FUNDING);
    assert_eq!(env.token_balance(&promo_vault_token_pda(&cngn)), FUNDING);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test promo_vault`
Expected: `error[E0422]: cannot find struct, variant or union type CreatePromoVault in module hodl_loans::instruction`, and the same for `hodl_loans::accounts` and for `FundPromoVault`, `WithdrawPromoVault` and `SweepPromoExcess` — eight of those — plus `error[E0412]: cannot find type PromoVault in crate hodl_loans`. Nine errors in all: none of the four instructions exists yet, and neither does the account the test reads back.

- [ ] **Step 3: Implement**

Create `programs/hodl_loans/src/state/promo.rs` with the vault only — the campaign and the receipt arrive in Tasks 2 and 4:

```rust
use anchor_lang::prelude::*;

use crate::errors::HodlError;
use crate::math::checked::{add, sub, to_u64};

/// Spec §12. One per market: the cNGN behind every promo balance, and the accounting that keeps
/// `outstanding + unissued ≤ cash` true at all times.
///
/// - `cash` is what the program has recorded as held; a direct transfer to the token account is
///   ignored until it is swept, exactly as the market and collateral vaults treat donations.
/// - `outstanding` is the sum of every position's `promo_balance` — promo already handed out.
/// - `unissued` is the sum over active campaigns of `budget − granted` — promo promised to a
///   campaign but not yet redeemed.
#[account]
#[derive(InitSpace)]
pub struct PromoVault {
    pub version: u8,
    pub bump: u8,
    pub vault_bump: u8,
    pub market: Pubkey,
    pub vault: Pubkey,
    pub cash: u64,
    pub outstanding: u64,
    pub unissued: u64,
    pub reserved: [u8; 64],
}

impl PromoVault {
    /// cNGN committed to neither a position nor a campaign. Funding a campaign and withdrawing
    /// to the treasury both draw from here, so the §12 invariant holds by construction.
    pub fn free(&self) -> Result<u64> {
        let committed = add(self.outstanding as u128, self.unissued as u128)?;
        to_u64(sub(self.cash as u128, committed)?)
    }

    /// Promo leaving a position, on expiry, revocation, forfeiture or `close_position`. The cNGN
    /// itself does not move on expiry or revocation — it becomes free HODL funds again.
    pub fn release(&mut self, amount: u64) -> Result<()> {
        self.outstanding = to_u64(sub(self.outstanding as u128, amount as u128)?)?;
        Ok(())
    }

    pub fn require_invariant(&self) -> Result<()> {
        let committed = add(self.outstanding as u128, self.unissued as u128)?;
        require!(committed <= self.cash as u128, HodlError::PromoVaultInsufficient);
        Ok(())
    }
}
```

In `programs/hodl_loans/src/state/mod.rs`, add `pub mod promo;` and `pub use promo::*;` in alphabetical order.

Extract the shared sweep body in `programs/hodl_loans/src/instructions/admin/sweep.rs`, above `SweepMarketExcess`:

```rust
/// The body every vault sweep shares: whatever the token account holds above what the program
/// has recorded is a direct donation, and goes to the treasury. Three vaults record their
/// holdings in three different fields, which is the only thing that differs between them.
pub fn sweep_to_treasury<'info>(
    token_program: &Interface<'info, TokenInterface>,
    mint: &InterfaceAccount<'info, Mint>,
    vault: &InterfaceAccount<'info, TokenAccount>,
    destination: &InterfaceAccount<'info, TokenAccount>,
    authority: AccountInfo<'info>,
    recorded: u64,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    let excess = vault.amount.saturating_sub(recorded);
    require!(excess > 0, HodlError::AmountTooSmall);
    transfer_from_vault(
        token_program.key(),
        mint.to_account_info(),
        mint.decimals,
        vault.to_account_info(),
        destination.to_account_info(),
        authority,
        excess,
        signer_seeds,
    )?;
    emit!(ExcessSwept {
        vault: vault.key(),
        destination: destination.key(),
        amount: excess,
    });
    Ok(())
}
```

and rewrite both existing handlers to call it. They differ only in which field records the vault's holdings:

```rust
/// Sends vault tokens above `market.cash` (direct donations) to the treasury.
pub fn handle_sweep_market_excess(ctx: Context<SweepMarketExcess>) -> Result<()> {
    let mint_key = ctx.accounts.mint.key();
    let seeds: &[&[u8]] = &[MARKET_SEED, mint_key.as_ref(), &[ctx.accounts.market.bump]];
    sweep_to_treasury(
        &ctx.accounts.token_program,
        &ctx.accounts.mint,
        &ctx.accounts.vault,
        &ctx.accounts.destination,
        ctx.accounts.market.to_account_info(),
        ctx.accounts.market.cash,
        &[seeds],
    )
}
```

```rust
/// Sends collateral vault tokens above `total_deposited` (direct donations) to the treasury.
pub fn handle_sweep_collateral_excess(ctx: Context<SweepCollateralExcess>) -> Result<()> {
    require_collateral_mint_on_exit(&ctx.accounts.mint.to_account_info())?;

    let mint_key = ctx.accounts.mint.key();
    let seeds: &[&[u8]] = &[COLLATERAL_SEED, mint_key.as_ref(), &[ctx.accounts.collateral.bump]];
    sweep_to_treasury(
        &ctx.accounts.token_program,
        &ctx.accounts.mint,
        &ctx.accounts.vault,
        &ctx.accounts.destination,
        ctx.accounts.collateral.to_account_info(),
        ctx.accounts.collateral.total_deposited,
        &[seeds],
    )
}
```

Create `programs/hodl_loans/src/instructions/promos/vault.rs`:

```rust
use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{ACCOUNT_VERSION, CONFIG_SEED, MARKET_SEED, PROMO_VAULT_SEED, PROMO_VAULT_TOKEN_SEED};
use crate::errors::HodlError;
use crate::events::{PromoVaultCreated, PromoVaultFunded, PromoVaultWithdrawn};
use crate::instructions::admin::sweep::sweep_to_treasury;
use crate::math::checked::{add, sub, to_u64};
use crate::state::{Config, Market, PromoVault};
use crate::token::transfer::{transfer_from_user, transfer_from_vault};

#[derive(Accounts)]
pub struct CreatePromoVault<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(seeds = [MARKET_SEED, mint.key().as_ref()], bump = market.bump, has_one = mint)]
    pub market: Box<Account<'info, Market>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        init,
        payer = admin,
        space = 8 + PromoVault::INIT_SPACE,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
    #[account(
        init,
        payer = admin,
        token::mint = mint,
        token::authority = promo_vault,
        token::token_program = token_program,
        seeds = [PROMO_VAULT_TOKEN_SEED, market.key().as_ref()],
        bump
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

/// Spec §12. One promo vault per market, holding the cNGN behind every promo balance. Separate
/// from `create_market` so a market can run without promo, and so turning promo on is its own
/// admin action rather than a decision baked in at market creation.
pub fn handle_create_promo_vault(ctx: Context<CreatePromoVault>) -> Result<()> {
    let promo_vault = &mut ctx.accounts.promo_vault;
    promo_vault.set_inner(PromoVault {
        version: ACCOUNT_VERSION,
        bump: ctx.bumps.promo_vault,
        vault_bump: ctx.bumps.vault,
        market: ctx.accounts.market.key(),
        vault: ctx.accounts.vault.key(),
        cash: 0,
        outstanding: 0,
        unissued: 0,
        reserved: [0; 64],
    });
    emit!(PromoVaultCreated {
        market: ctx.accounts.market.key(),
        promo_vault: promo_vault.key(),
        vault: ctx.accounts.vault.key(),
    });
    Ok(())
}

#[derive(Accounts)]
pub struct FundPromoVault<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(seeds = [MARKET_SEED, mint.key().as_ref()], bump = market.bump, has_one = mint)]
    pub market: Box<Account<'info, Market>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market,
        has_one = vault
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, token::mint = mint, token::token_program = token_program)]
    pub source: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
}

/// Spec §12. cNGN in; `cash += amount`. The tokens are the backing for promo the program has
/// not yet handed out, so this is the only way `free()` grows.
pub fn handle_fund_promo_vault(ctx: Context<FundPromoVault>, amount: u64) -> Result<()> {
    require!(amount > 0, HodlError::AmountTooSmall);
    transfer_from_user(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.source.to_account_info(),
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.admin.to_account_info(),
        amount,
    )?;

    let promo_vault = &mut ctx.accounts.promo_vault;
    promo_vault.cash = to_u64(add(promo_vault.cash as u128, amount as u128)?)?;
    emit!(PromoVaultFunded {
        market: ctx.accounts.market.key(),
        amount,
        cash: promo_vault.cash,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct WithdrawPromoVault<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(seeds = [MARKET_SEED, mint.key().as_ref()], bump = market.bump, has_one = mint)]
    pub market: Box<Account<'info, Market>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market,
        has_one = vault
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
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

/// Spec §12. Only free cNGN leaves: what is already promised to a position (`outstanding`) or to
/// a campaign (`unissued`) stays, so the invariant survives the withdrawal.
pub fn handle_withdraw_promo_vault(ctx: Context<WithdrawPromoVault>, amount: u64) -> Result<()> {
    require!(amount > 0, HodlError::AmountTooSmall);
    require!(amount <= ctx.accounts.promo_vault.free()?, HodlError::PromoVaultInsufficient);

    let market_key = ctx.accounts.market.key();
    let seeds: &[&[u8]] = &[PROMO_VAULT_SEED, market_key.as_ref(), &[ctx.accounts.promo_vault.bump]];
    transfer_from_vault(
        ctx.accounts.token_program.key(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.mint.decimals,
        ctx.accounts.vault.to_account_info(),
        ctx.accounts.destination.to_account_info(),
        ctx.accounts.promo_vault.to_account_info(),
        amount,
        &[seeds],
    )?;

    let promo_vault = &mut ctx.accounts.promo_vault;
    promo_vault.cash = to_u64(sub(promo_vault.cash as u128, amount as u128)?)?;
    promo_vault.require_invariant()?;
    emit!(PromoVaultWithdrawn {
        market: market_key,
        amount,
        cash: promo_vault.cash,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct SweepPromoExcess<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(seeds = [MARKET_SEED, mint.key().as_ref()], bump = market.bump, has_one = mint)]
    pub market: Box<Account<'info, Market>>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market,
        has_one = vault
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
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

/// Sends promo vault tokens above `cash` (direct donations) to the treasury. The third vault to
/// need this, and the reason the three share `sweep_to_treasury`.
pub fn handle_sweep_promo_excess(ctx: Context<SweepPromoExcess>) -> Result<()> {
    let market_key = ctx.accounts.market.key();
    let seeds: &[&[u8]] = &[PROMO_VAULT_SEED, market_key.as_ref(), &[ctx.accounts.promo_vault.bump]];
    sweep_to_treasury(
        &ctx.accounts.token_program,
        &ctx.accounts.mint,
        &ctx.accounts.vault,
        &ctx.accounts.destination,
        ctx.accounts.promo_vault.to_account_info(),
        ctx.accounts.promo_vault.cash,
        &[seeds],
    )
}
```

Create `programs/hodl_loans/src/instructions/promos/mod.rs`:

```rust
pub mod vault;

pub use vault::*;
```

In `programs/hodl_loans/src/instructions/mod.rs`, add `pub mod promos;` and `pub use promos::*;`. **The module is `promos`, plural, deliberately:** `state::promo` and `instructions::promo` are both glob re-exported from the crate root, and same-named modules collide with `error: ambiguous glob re-exports`. This is the same collision `instructions::positions` avoids.

Add the events to `programs/hodl_loans/src/events.rs`:

```rust
#[event]
pub struct PromoVaultCreated {
    pub market: Pubkey,
    pub promo_vault: Pubkey,
    pub vault: Pubkey,
}

#[event]
pub struct PromoVaultFunded {
    pub market: Pubkey,
    pub amount: u64,
    pub cash: u64,
}

#[event]
pub struct PromoVaultWithdrawn {
    pub market: Pubkey,
    pub amount: u64,
    pub cash: u64,
}
```

And the entry points to `programs/hodl_loans/src/lib.rs`, after `sweep_collateral_excess`:

```rust
    pub fn create_promo_vault(ctx: Context<CreatePromoVault>) -> Result<()> {
        instructions::handle_create_promo_vault(ctx)
    }

    pub fn fund_promo_vault(ctx: Context<FundPromoVault>, amount: u64) -> Result<()> {
        instructions::handle_fund_promo_vault(ctx, amount)
    }

    pub fn withdraw_promo_vault(ctx: Context<WithdrawPromoVault>, amount: u64) -> Result<()> {
        instructions::handle_withdraw_promo_vault(ctx, amount)
    }

    pub fn sweep_promo_excess(ctx: Context<SweepPromoExcess>) -> Result<()> {
        instructions::handle_sweep_promo_excess(ctx)
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, 172 tests in all (`promo_vault` is new with 4).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: the promo vault, and one sweep body for three vaults"
```

---

### Task 2: Campaigns

**Files:**
- Create: `programs/hodl_loans/src/instructions/promos/campaign.rs`, `tests/campaign.rs`
- Modify: `src/state/promo.rs`, `src/instructions/promos/mod.rs`, `src/events.rs`, `src/lib.rs`, `tests/common/mod.rs`

**Interfaces:**
- Consumes: `PromoVault::{free, require_invariant}` (Task 1).
- Produces:
  - `state::Campaign`
  - `create_campaign(campaign_id, budget, redeem_until)`, `close_campaign`
  - Harness: `campaign_pda`, `create_campaign_ix`, `close_campaign_ix`, `Env::campaign(mint, id)`, `Env::create_campaign(mint, id, budget)`

- [ ] **Step 1: Write the harness and the failing tests**

Append to `programs/hodl_loans/tests/common/mod.rs`:

```rust
// ---- Campaigns (Task 2) ----

pub fn campaign_pda(market_mint: &Pubkey, campaign_id: u64) -> Pubkey {
    pda(&[b"campaign", market_pda(market_mint).as_ref(), &campaign_id.to_le_bytes()])
}

pub fn create_campaign_ix(
    admin: &Pubkey,
    mint: &Pubkey,
    campaign_id: u64,
    budget: u64,
    redeem_until: i64,
) -> Instruction {
    ix(
        hodl_loans::instruction::CreateCampaign { campaign_id, budget, redeem_until },
        hodl_loans::accounts::CreateCampaign {
            admin: *admin,
            config: config_pda(),
            market: market_pda(mint),
            promo_vault: promo_vault_pda(mint),
            campaign: campaign_pda(mint, campaign_id),
            system_program: system_program::ID,
        },
    )
}

pub fn close_campaign_ix(admin: &Pubkey, mint: &Pubkey, campaign_id: u64) -> Instruction {
    ix(
        hodl_loans::instruction::CloseCampaign {},
        hodl_loans::accounts::CloseCampaign {
            admin: *admin,
            config: config_pda(),
            market: market_pda(mint),
            promo_vault: promo_vault_pda(mint),
            campaign: campaign_pda(mint, campaign_id),
        },
    )
}

impl Env {
    pub fn campaign(&self, market_mint: &Pubkey, campaign_id: u64) -> hodl_loans::Campaign {
        self.fetch(&campaign_pda(market_mint, campaign_id))
    }

    /// Creates campaign `id` with `budget`, redeemable for a year.
    pub fn create_campaign(&mut self, mint: &Pubkey, campaign_id: u64, budget: u64) {
        let admin = self.admin.pubkey();
        let until = self.now() + 365 * 86_400;
        let instruction = create_campaign_ix(&admin, mint, campaign_id, budget, until);
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("create campaign");
    }
}
```

Create `programs/hodl_loans/tests/campaign.rs`:

```rust
mod common;

use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

const FUNDING: u64 = 1_000_000 * ONE_CNGN;
const BUDGET: u64 = 300_000 * ONE_CNGN;

#[test]
fn a_campaign_reserves_budget_out_of_the_vaults_free_cngn() {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    env.create_campaign(&cngn, 1, BUDGET);

    let campaign = env.campaign(&cngn, 1);
    assert_eq!((campaign.campaign_id, campaign.budget, campaign.granted), (1, BUDGET, 0));
    assert!(campaign.active);
    assert_eq!(campaign.market, market_pda(&cngn));

    // The reservation shows up as `unissued`, and the free balance shrinks by exactly it.
    let vault = env.promo_vault(&cngn);
    assert_eq!((vault.cash, vault.unissued, vault.outstanding), (FUNDING, BUDGET, 0));
    assert_eq!(vault.free().unwrap(), FUNDING - BUDGET);

    // A second campaign reserves against what is left, not against the whole vault.
    env.create_campaign(&cngn, 2, FUNDING - BUDGET);
    assert_eq!(env.promo_vault(&cngn).free().unwrap(), 0);
}

#[test]
fn campaign_rejections() {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();
    let stranger = env.funded_keypair();
    let until = env.now() + 86_400;

    let by_stranger = create_campaign_ix(&stranger.pubkey(), &cngn, 1, BUDGET, until);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);

    let zero = create_campaign_ix(&admin, &cngn, 1, 0, until);
    assert_hodl_error(send(&mut env.svm, &[zero], &[&env.admin]), HodlError::AmountTooSmall);

    // A campaign that is already over cannot be created.
    let past = create_campaign_ix(&admin, &cngn, 1, BUDGET, env.now());
    assert_hodl_error(send(&mut env.svm, &[past], &[&env.admin]), HodlError::InvalidParameters);

    // More than the vault holds free.
    let over = create_campaign_ix(&admin, &cngn, 1, FUNDING + 1, until);
    assert_hodl_error(send(&mut env.svm, &[over], &[&env.admin]), HodlError::PromoVaultInsufficient);

    // One account per (market, campaign_id): the same id twice fails to init.
    env.create_campaign(&cngn, 1, BUDGET);
    let again = create_campaign_ix(&admin, &cngn, 1, ONE_CNGN, until);
    assert!(send(&mut env.svm, &[again], &[&env.admin]).is_err());

    // Closing is admin-only, and only once.
    let by_stranger = close_campaign_ix(&stranger.pubkey(), &cngn, 1);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);
    send(&mut env.svm, &[close_campaign_ix(&admin, &cngn, 1)], &[&env.admin]).unwrap();
    let twice = close_campaign_ix(&admin, &cngn, 1);
    assert_hodl_error(send(&mut env.svm, &[twice], &[&env.admin]), HodlError::CampaignInactive);
}

#[test]
fn closing_a_campaign_twice_is_rejected_even_with_a_second_campaign_still_open() {
    // A single-campaign vault drains `unissued` to zero on its first close, so closing it again
    // happens to fail on an underflow in `sub` — the right outcome, for the wrong reason. With a
    // second campaign still holding its own budget in `unissued`, that underflow no longer fires:
    // a second close would silently subtract campaign 1's `unspent` a second time, over-crediting
    // the vault's `free()` at campaign 2's expense. This pins the `active` guard itself.
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();

    let budget_1 = 200_000 * ONE_CNGN;
    let budget_2 = 300_000 * ONE_CNGN;
    env.create_campaign(&cngn, 1, budget_1);
    env.create_campaign(&cngn, 2, budget_2);

    send(&mut env.svm, &[close_campaign_ix(&admin, &cngn, 1)], &[&env.admin]).unwrap();
    assert!(!env.campaign(&cngn, 1).active);
    assert!(env.campaign(&cngn, 2).active);
    // Only campaign 1's reservation came back; campaign 2's is untouched.
    assert_eq!(env.promo_vault(&cngn).unissued, budget_2);

    let twice = close_campaign_ix(&admin, &cngn, 1);
    assert_hodl_error(send(&mut env.svm, &[twice], &[&env.admin]), HodlError::CampaignInactive);
}

#[test]
fn closing_a_campaign_returns_only_what_it_never_granted() {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();
    env.create_campaign(&cngn, 1, BUDGET);

    // Simulate the campaign having issued a third of its budget.
    let granted = 100_000 * ONE_CNGN;
    let mut campaign = env.campaign(&cngn, 1);
    campaign.granted = granted;
    env.write(&campaign_pda(&cngn, 1), &campaign);
    let mut vault = env.promo_vault(&cngn);
    vault.unissued -= granted;
    vault.outstanding += granted;
    env.write(&promo_vault_pda(&cngn), &vault);

    send(&mut env.svm, &[close_campaign_ix(&admin, &cngn, 1)], &[&env.admin]).unwrap();

    // The granted third stays committed as `outstanding`; only the rest returns to free.
    let vault = env.promo_vault(&cngn);
    assert_eq!((vault.unissued, vault.outstanding, vault.cash), (0, granted, FUNDING));
    assert_eq!(vault.free().unwrap(), FUNDING - granted);
    assert!(!env.campaign(&cngn, 1).active);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test campaign`
Expected: `error[E0422]: cannot find struct, variant or union type CreateCampaign in module hodl_loans::instruction` and the matching `accounts` error, the same pair for `CloseCampaign`, and `error[E0412]: cannot find type Campaign in crate hodl_loans`.

- [ ] **Step 3: Implement**

Add the account to `programs/hodl_loans/src/state/promo.rs`:

```rust
/// Spec §12. A budget the promo signer may issue vouchers against, until `redeem_until`.
#[account]
#[derive(InitSpace)]
pub struct Campaign {
    pub version: u8,
    pub bump: u8,
    pub market: Pubkey,
    pub campaign_id: u64,
    pub budget: u64,
    pub granted: u64,
    pub redeem_until: i64,
    pub active: bool,
    pub reserved: [u8; 32],
}
```

Create `programs/hodl_loans/src/instructions/promos/campaign.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{ACCOUNT_VERSION, CAMPAIGN_SEED, CONFIG_SEED, PROMO_VAULT_SEED};
use crate::errors::HodlError;
use crate::events::{CampaignClosed, CampaignCreated};
use crate::math::checked::{add, sub, to_u64};
use crate::state::{Campaign, Config, Market, PromoVault};

#[derive(Accounts)]
#[instruction(campaign_id: u64)]
pub struct CreateCampaign<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
    #[account(
        init,
        payer = admin,
        space = 8 + Campaign::INIT_SPACE,
        seeds = [CAMPAIGN_SEED, market.key().as_ref(), &campaign_id.to_le_bytes()],
        bump
    )]
    pub campaign: Box<Account<'info, Campaign>>,
    pub system_program: Program<'info, System>,
}

/// Spec §12. Reserves `budget` out of the promo vault's free cNGN, so every voucher the signer
/// issues against this campaign is already backed before it is redeemed. The reservation is what
/// `unissued` counts.
pub fn handle_create_campaign(
    ctx: Context<CreateCampaign>,
    campaign_id: u64,
    budget: u64,
    redeem_until: i64,
) -> Result<()> {
    require!(budget > 0, HodlError::AmountTooSmall);
    require!(redeem_until > Clock::get()?.unix_timestamp, HodlError::InvalidParameters);
    require!(budget <= ctx.accounts.promo_vault.free()?, HodlError::PromoVaultInsufficient);

    let promo_vault = &mut ctx.accounts.promo_vault;
    promo_vault.unissued = to_u64(add(promo_vault.unissued as u128, budget as u128)?)?;
    promo_vault.require_invariant()?;

    ctx.accounts.campaign.set_inner(Campaign {
        version: ACCOUNT_VERSION,
        bump: ctx.bumps.campaign,
        market: ctx.accounts.market.key(),
        campaign_id,
        budget,
        granted: 0,
        redeem_until,
        active: true,
        reserved: [0; 32],
    });
    emit!(CampaignCreated {
        market: ctx.accounts.market.key(),
        campaign: ctx.accounts.campaign.key(),
        campaign_id,
        budget,
        redeem_until,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct CloseCampaign<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
    #[account(
        mut,
        seeds = [CAMPAIGN_SEED, market.key().as_ref(), &campaign.campaign_id.to_le_bytes()],
        bump = campaign.bump,
        has_one = market
    )]
    pub campaign: Box<Account<'info, Campaign>>,
}

/// Spec §12. Hands the unspent part of the budget back to the vault's free cNGN. The account
/// stays so its `granted` total remains readable, and so vouchers already redeemed against it
/// keep a campaign to point at.
pub fn handle_close_campaign(ctx: Context<CloseCampaign>) -> Result<()> {
    require!(ctx.accounts.campaign.active, HodlError::CampaignInactive);
    let unspent = sub(ctx.accounts.campaign.budget as u128, ctx.accounts.campaign.granted as u128)?;

    let promo_vault = &mut ctx.accounts.promo_vault;
    promo_vault.unissued = to_u64(sub(promo_vault.unissued as u128, unspent)?)?;
    promo_vault.require_invariant()?;

    let campaign = &mut ctx.accounts.campaign;
    campaign.active = false;
    emit!(CampaignClosed {
        market: ctx.accounts.market.key(),
        campaign: campaign.key(),
        campaign_id: campaign.campaign_id,
        granted: campaign.granted,
        returned: to_u64(unspent)?,
    });
    Ok(())
}
```

Add `pub mod campaign;` and `pub use campaign::*;` to `programs/hodl_loans/src/instructions/promos/mod.rs`, the events to `src/events.rs`:

```rust
#[event]
pub struct CampaignCreated {
    pub market: Pubkey,
    pub campaign: Pubkey,
    pub campaign_id: u64,
    pub budget: u64,
    pub redeem_until: i64,
}

#[event]
pub struct CampaignClosed {
    pub market: Pubkey,
    pub campaign: Pubkey,
    pub campaign_id: u64,
    pub granted: u64,
    /// The unspent budget handed back to the vault's free cNGN.
    pub returned: u64,
}
```

and the entry points to `src/lib.rs`:

```rust
    pub fn create_campaign(
        ctx: Context<CreateCampaign>,
        campaign_id: u64,
        budget: u64,
        redeem_until: i64,
    ) -> Result<()> {
        instructions::handle_create_campaign(ctx, campaign_id, budget, redeem_until)
    }

    pub fn close_campaign(ctx: Context<CloseCampaign>) -> Result<()> {
        instructions::handle_close_campaign(ctx)
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, 175 tests in all (`campaign` is new with 3).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: promo campaigns reserve budget out of the vault"
```

---

### Task 3: The voucher message and its signature

**Files:**
- Create: `programs/hodl_loans/src/voucher.rs`
- Modify: `programs/hodl_loans/Cargo.toml`, `Cargo.lock`, `src/constants.rs`, `src/lib.rs`

**Interfaces:**
- Consumes: `HodlError::InvalidVoucherSignature`.
- Produces:
  - `constants::VOUCHER_DOMAIN`
  - `voucher::PromoVoucher` with `new(market, campaign_id, wallet, amount, nonce, voucher_expiry)` and `message() -> Result<Vec<u8>>`
  - `voucher::ED25519_PROGRAM_ID`
  - `voucher::require_ed25519_signature(instructions_sysvar, signer, message) -> Result<()>`

- [ ] **Step 1: Add the dependency and the constant, then write the failing tests**

`anchor_lang`'s `solana_program` shim re-exports neither `ed25519_program` nor the instructions-sysvar loaders, so depend on the crate directly. It is already in the lockfile through `anchor-lang`, so this resolves without a version bump. In `programs/hodl_loans/Cargo.toml`, under `[dependencies]`:

```toml
solana-instructions-sysvar = "3"
```

Add the domain separator to `programs/hodl_loans/src/constants.rs`, with the seeds:

```rust
/// Domain separator in the voucher message (spec §12). It binds a signature to this program's
/// voucher format, so a `promo_signer` key reused elsewhere cannot produce a valid voucher.
/// Published in the IDL so the off-chain promo signer reads it rather than hardcoding a copy
/// that could drift from the program's.
#[constant]
pub const VOUCHER_DOMAIN: &str = "hodl_loans:promo_voucher:v1";
```

Add `pub mod voucher;` to `programs/hodl_loans/src/lib.rs`, after `pub mod valuation;`, then create `programs/hodl_loans/src/voucher.rs` with the imports and the test module only:

```rust
use anchor_lang::prelude::*;
use solana_instructions_sysvar::{load_current_index_checked, load_instruction_at_checked};

use crate::constants::VOUCHER_DOMAIN;
use crate::errors::HodlError;

#[cfg(test)]
mod tests {
    use super::*;

    const SIGNER: Pubkey = Pubkey::new_from_array([9; 32]);

    /// The layout `solana_sdk`'s Ed25519 instruction builder produces: header, one descriptor,
    /// then the signature, the public key and the message.
    fn ed25519_data(signer: &Pubkey, message: &[u8]) -> Vec<u8> {
        let signature_offset = ED25519_HEADER_LEN + ED25519_DESCRIPTOR_LEN;
        let public_key_offset = signature_offset + 64;
        let message_offset = public_key_offset + 32;
        let mut data = vec![1u8, 0u8];
        for value in [
            signature_offset as u16,
            THIS_INSTRUCTION,
            public_key_offset as u16,
            THIS_INSTRUCTION,
            message_offset as u16,
            message.len() as u16,
            THIS_INSTRUCTION,
        ] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        data.extend_from_slice(&[0u8; 64]);
        data.extend_from_slice(signer.as_ref());
        data.extend_from_slice(message);
        data
    }

    #[test]
    fn a_well_formed_instruction_covers_its_key_and_message() {
        let data = ed25519_data(&SIGNER, b"hello");
        assert!(ed25519_instruction_covers(&data, &SIGNER, b"hello"));
        // A different key or a different message is not covered.
        assert!(!ed25519_instruction_covers(&data, &Pubkey::new_from_array([8; 32]), b"hello"));
        assert!(!ed25519_instruction_covers(&data, &SIGNER, b"hellp"));
        // Nor is a prefix: the length is compared before the bytes.
        assert!(!ed25519_instruction_covers(&data, &SIGNER, b"hell"));
    }

    #[test]
    fn descriptors_that_point_elsewhere_are_rejected() {
        // A second signature the caller could leave unchecked.
        let mut two = ed25519_data(&SIGNER, b"hello");
        two[0] = 2;
        assert!(!ed25519_instruction_covers(&two, &SIGNER, b"hello"));

        // A message that lives in another instruction: the native program would verify it
        // against bytes we never see, so the pointer must be self-referential.
        for index_field in [2usize, 6, 12] {
            let mut elsewhere = ed25519_data(&SIGNER, b"hello");
            elsewhere[ED25519_HEADER_LEN + index_field] = 0;
            elsewhere[ED25519_HEADER_LEN + index_field + 1] = 0;
            assert!(!ed25519_instruction_covers(&elsewhere, &SIGNER, b"hello"));
        }
    }

    #[test]
    fn malformed_data_is_rejected_rather_than_panicking() {
        assert!(!ed25519_instruction_covers(&[], &SIGNER, b"hello"));
        assert!(!ed25519_instruction_covers(&[1, 0], &SIGNER, b"hello"));
        // An offset past the end of the data must not slice out of bounds.
        let mut past_end = ed25519_data(&SIGNER, b"hello");
        past_end[ED25519_HEADER_LEN + 8] = 0xff;
        past_end[ED25519_HEADER_LEN + 9] = 0xff;
        assert!(!ed25519_instruction_covers(&past_end, &SIGNER, b"hello"));
        let mut key_past_end = ed25519_data(&SIGNER, b"hello");
        key_past_end[ED25519_HEADER_LEN + 4] = 0xff;
        key_past_end[ED25519_HEADER_LEN + 5] = 0xff;
        assert!(!ed25519_instruction_covers(&key_past_end, &SIGNER, b"hello"));
    }

    #[test]
    fn the_message_pins_the_domain_the_program_and_the_market() {
        let market = Pubkey::new_from_array([3; 32]);
        let wallet = Pubkey::new_from_array([4; 32]);
        let voucher = PromoVoucher::new(market, 7, wallet, 500, 11, 1_800_000_000);
        assert_eq!(voucher.domain, VOUCHER_DOMAIN);
        assert_eq!(voucher.program_id, crate::ID);

        // Borsh puts the domain first, length-prefixed, so a message for another format cannot
        // collide with one of ours.
        let bytes = voucher.message().unwrap();
        assert_eq!(&bytes[..4], &(VOUCHER_DOMAIN.len() as u32).to_le_bytes());
        assert_eq!(&bytes[4..4 + VOUCHER_DOMAIN.len()], VOUCHER_DOMAIN.as_bytes());

        // Every field is covered: changing any one changes the bytes.
        for other in [
            PromoVoucher::new(Pubkey::new_from_array([5; 32]), 7, wallet, 500, 11, 1_800_000_000),
            PromoVoucher::new(market, 8, wallet, 500, 11, 1_800_000_000),
            PromoVoucher::new(market, 7, Pubkey::new_from_array([6; 32]), 500, 11, 1_800_000_000),
            PromoVoucher::new(market, 7, wallet, 501, 11, 1_800_000_000),
            PromoVoucher::new(market, 7, wallet, 500, 12, 1_800_000_000),
            PromoVoucher::new(market, 7, wallet, 500, 11, 1_800_000_001),
        ] {
            assert_ne!(other.message().unwrap(), bytes);
        }
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `./scripts/test.sh`
Expected: 28 errors, of five distinct messages — `error[E0425]: cannot find value ED25519_HEADER_LEN in this scope`, the same for `ED25519_DESCRIPTOR_LEN` and `THIS_INSTRUCTION`, `error[E0425]: cannot find function ed25519_instruction_covers in this scope`, and `error[E0433]: failed to resolve: use of undeclared type PromoVoucher`. `Pubkey` and `VOUCHER_DOMAIN` do NOT error: the test module is a child of `voucher`, so its `use super::*` picks up the file's own private imports.

- [ ] **Step 3: Implement**

Add the implementation above the test module in `programs/hodl_loans/src/voucher.rs`:

```rust
use anchor_lang::prelude::*;
use solana_instructions_sysvar::{load_current_index_checked, load_instruction_at_checked};

use crate::constants::VOUCHER_DOMAIN;
use crate::errors::HodlError;

/// The native Ed25519 signature-verification program. Hard-coded rather than pulled from a
/// crate: it is a fixed address, and `anchor_lang`'s `solana_program` shim does not re-export
/// `ed25519_program`. Matches `solana_sdk_ids::ed25519_program::ID`.
pub const ED25519_PROGRAM_ID: Pubkey = pubkey!("Ed25519SigVerify111111111111111111111111111");

/// Spec §12's voucher message. The program builds this itself from the redeeming instruction's
/// arguments and its own context, then requires the Ed25519 instruction to have signed exactly
/// these bytes. The domain, the program id and the market are therefore pinned by construction
/// rather than parsed out of something the caller supplied: a signature the promo signer
/// produced for a different program, a different market or a different format cannot be
/// replayed here.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct PromoVoucher {
    pub domain: String,
    pub program_id: Pubkey,
    pub market: Pubkey,
    pub campaign_id: u64,
    pub wallet: Pubkey,
    pub amount: u64,
    pub nonce: u64,
    pub voucher_expiry: i64,
}

impl PromoVoucher {
    pub fn new(
        market: Pubkey,
        campaign_id: u64,
        wallet: Pubkey,
        amount: u64,
        nonce: u64,
        voucher_expiry: i64,
    ) -> Self {
        Self {
            domain: VOUCHER_DOMAIN.to_string(),
            program_id: crate::ID,
            market,
            campaign_id,
            wallet,
            amount,
            nonce,
            voucher_expiry,
        }
    }

    /// The exact bytes the promo signer must have signed.
    pub fn message(&self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.serialize(&mut bytes)?;
        Ok(bytes)
    }
}

/// Offsets inside one Ed25519 program signature descriptor, in the order the native program
/// reads them. The layout is two header bytes followed by one 14-byte descriptor per signature.
const ED25519_HEADER_LEN: usize = 2;
const ED25519_DESCRIPTOR_LEN: usize = 14;
/// `u16::MAX` in an instruction-index field means "this instruction's own data".
const THIS_INSTRUCTION: u16 = u16::MAX;

/// Spec §12 step 2: require that an Ed25519 program instruction *earlier in this transaction*
/// proves `signer` signed exactly `message`.
///
/// The native Ed25519 program does the cryptography; a bad signature fails its own instruction
/// and takes the whole transaction with it. What is left for us is making sure such an
/// instruction exists and covers the right key and the right bytes — which is where the care
/// goes, because the descriptor can point anywhere:
///
/// - exactly one signature, so a second descriptor cannot smuggle in an unchecked one;
/// - all three data pointers must be `u16::MAX`, so the key and the message must live in the
///   Ed25519 instruction's own data and cannot reference bytes from another instruction that
///   the native program verified against a different key;
/// - the key and the message bytes must match, read with bounds checks rather than slicing.
pub fn require_ed25519_signature(
    instructions_sysvar: &AccountInfo,
    signer: &Pubkey,
    message: &[u8],
) -> Result<()> {
    let current = load_current_index_checked(instructions_sysvar)?;
    for index in 0..current {
        let instruction = load_instruction_at_checked(index as usize, instructions_sysvar)?;
        if instruction.program_id != ED25519_PROGRAM_ID {
            continue;
        }
        if ed25519_instruction_covers(&instruction.data, signer, message) {
            return Ok(());
        }
    }
    Err(HodlError::InvalidVoucherSignature.into())
}

/// Whether one Ed25519 instruction's data is a single self-contained signature by `signer` over
/// `message`. Every read is bounds-checked: the descriptor's offsets are attacker-supplied.
fn ed25519_instruction_covers(data: &[u8], signer: &Pubkey, message: &[u8]) -> bool {
    if data.len() < ED25519_HEADER_LEN + ED25519_DESCRIPTOR_LEN || data[0] != 1 {
        return false;
    }
    let field = |at: usize| -> u16 {
        u16::from_le_bytes([data[ED25519_HEADER_LEN + at], data[ED25519_HEADER_LEN + at + 1]])
    };
    let public_key_offset = field(4) as usize;
    let message_offset = field(8) as usize;
    let message_size = field(10) as usize;
    // Every pointer must stay inside this instruction: indices 2, 6 and 12 are the signature,
    // public-key and message instruction indices.
    if field(2) != THIS_INSTRUCTION || field(6) != THIS_INSTRUCTION || field(12) != THIS_INSTRUCTION {
        return false;
    }
    if message_size != message.len() {
        return false;
    }
    let Some(key_bytes) = data.get(public_key_offset..public_key_offset + 32) else {
        return false;
    };
    let Some(message_bytes) = data.get(message_offset..message_offset + message_size) else {
        return false;
    };
    key_bytes == signer.as_ref() && message_bytes == message
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, 180 tests in all (the unit suite is now 47). (Higher than this plan was first generated with, because review rounds added tests: +1 from Task 3 onward for the Task 2 multi-campaign double-close test pinning `close_campaign`'s `active` guard, and +5 more from Task 4 onward for the same-transaction voucher-replay test plus the four its fix round added (expiry boundary, market rebinding, market paused, zero amount). The totals below already include all of them.)

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: the promo voucher message and its Ed25519 check"
```

---

### Task 4: Redeeming a voucher

**Files:**
- Create: `programs/hodl_loans/src/instructions/promos/redeem.rs`, `tests/promo_redeem.rs`
- Modify: `programs/hodl_loans/Cargo.toml`, `src/state/promo.rs`, `src/instructions/promos/mod.rs`, `src/events.rs`, `src/lib.rs`, `tests/common/mod.rs`

**Interfaces:**
- Consumes: `PromoVoucher`, `require_ed25519_signature` (Task 3); `Campaign` (Task 2); `PromoVault` (Task 1); `Access::require_active`.
- Produces:
  - `state::VoucherReceipt`
  - `redeem_promo(amount, nonce, voucher_expiry)`, `close_voucher_receipt`
  - Harness: `ED25519_PROGRAM`, `instructions_sysvar()`, `voucher_pda`, `ed25519_verify_ix`, `redeem_promo_ix`, `close_voucher_receipt_ix`, `Env::voucher_message`, `Env::redeem_promo`, `Env::redeem_voucher_signed_by`, `Env::voucher_receipt`

- [ ] **Step 1: Turn on the precompiles, write the harness and the failing tests**

LiteSVM does not load the native Ed25519 program unless asked, and without it every redemption fails with `InvalidProgramForExecution` — which looks like a program bug and is not one. In `programs/hodl_loans/Cargo.toml`:

```toml
[dev-dependencies]
solana-instructions-sysvar = "3"
litesvm = { version = "0.10.0", features = ["precompiles"] }
```

and in `programs/hodl_loans/tests/common/mod.rs`, in `Env::new`:

```rust
        // `with_precompiles` loads the native Ed25519 program, which promo vouchers need.
        let mut svm = LiteSVM::new().with_precompiles();
```

Append the section — `ed25519_verify_ix` builds the instruction by hand, because no dependency here ships the SDK's builder:

```rust
// ---- Vouchers (Task 4) ----

pub const ED25519_PROGRAM: Pubkey = hodl_loans::voucher::ED25519_PROGRAM_ID;
pub fn instructions_sysvar() -> Pubkey {
    solana_instructions_sysvar::ID
}

pub fn voucher_pda(campaign: &Pubkey, nonce: u64) -> Pubkey {
    pda(&[b"voucher", campaign.as_ref(), &nonce.to_le_bytes()])
}

/// An Ed25519 program instruction proving `signer` signed `message`, in the layout the native
/// program reads: two header bytes, one 14-byte descriptor, then the signature, key and message.
/// Built by hand because no dependency here ships the SDK's builder.
pub fn ed25519_verify_ix(signer: &Keypair, message: &[u8]) -> Instruction {
    let signature_offset: u16 = 16;
    let public_key_offset = signature_offset + 64;
    let message_offset = public_key_offset + 32;
    let mut data = vec![1u8, 0u8];
    for value in [
        signature_offset,
        u16::MAX,
        public_key_offset,
        u16::MAX,
        message_offset,
        message.len() as u16,
        u16::MAX,
    ] {
        data.extend_from_slice(&value.to_le_bytes());
    }
    data.extend_from_slice(signer.sign_message(message).as_ref());
    data.extend_from_slice(signer.pubkey().as_ref());
    data.extend_from_slice(message);
    Instruction::new_with_bytes(ED25519_PROGRAM, &data, vec![])
}

pub fn redeem_promo_ix(
    payer: &Pubkey,
    owner: &Pubkey,
    mint: &Pubkey,
    campaign_id: u64,
    amount: u64,
    nonce: u64,
    voucher_expiry: i64,
) -> Instruction {
    let campaign = campaign_pda(mint, campaign_id);
    ix(
        hodl_loans::instruction::RedeemPromo { amount, nonce, voucher_expiry },
        hodl_loans::accounts::RedeemPromo {
            payer: *payer,
            owner: *owner,
            access: access_pda(owner),
            config: config_pda(),
            market: market_pda(mint),
            position: position_pda(owner),
            promo_vault: promo_vault_pda(mint),
            campaign,
            voucher_receipt: voucher_pda(&campaign, nonce),
            instructions_sysvar: instructions_sysvar(),
            system_program: system_program::ID,
        },
    )
}

pub fn close_voucher_receipt_ix(rent_payer: &Pubkey, campaign: &Pubkey, nonce: u64) -> Instruction {
    ix(
        hodl_loans::instruction::CloseVoucherReceipt {},
        hodl_loans::accounts::CloseVoucherReceipt {
            rent_payer: *rent_payer,
            voucher_receipt: voucher_pda(campaign, nonce),
        },
    )
}

impl Env {
    /// The message the promo signer must sign for this voucher.
    pub fn voucher_message(
        &self,
        mint: &Pubkey,
        campaign_id: u64,
        wallet: &Pubkey,
        amount: u64,
        nonce: u64,
        voucher_expiry: i64,
    ) -> Vec<u8> {
        hodl_loans::voucher::PromoVoucher::new(
            market_pda(mint),
            campaign_id,
            *wallet,
            amount,
            nonce,
            voucher_expiry,
        )
        .message()
        .unwrap()
    }

    /// Redeems a voucher: the Ed25519 proof first, then the redemption, in one transaction.
    pub fn redeem_promo(
        &mut self,
        borrower: &Borrower,
        mint: &Pubkey,
        campaign_id: u64,
        amount: u64,
        nonce: u64,
    ) -> TxResult {
        let expiry = self.now() + 86_400;
        self.redeem_voucher_signed_by(&self.promo_signer.insecure_clone(), borrower, mint, campaign_id, amount, nonce, expiry)
    }

    /// The same, with the signing key and expiry spelled out — for the cases where one of them
    /// is meant to be wrong.
    #[allow(clippy::too_many_arguments)]
    pub fn redeem_voucher_signed_by(
        &mut self,
        signer: &Keypair,
        borrower: &Borrower,
        mint: &Pubkey,
        campaign_id: u64,
        amount: u64,
        nonce: u64,
        voucher_expiry: i64,
    ) -> TxResult {
        let owner = borrower.pubkey();
        let message = self.voucher_message(mint, campaign_id, &owner, amount, nonce, voucher_expiry);
        let admin = self.admin.pubkey();
        let instructions = vec![
            ed25519_verify_ix(signer, &message),
            redeem_promo_ix(&admin, &owner, mint, campaign_id, amount, nonce, voucher_expiry),
        ];
        send(&mut self.svm, &instructions, &[&self.admin, &borrower.key])
    }

    pub fn voucher_receipt(&self, campaign: &Pubkey, nonce: u64) -> hodl_loans::VoucherReceipt {
        self.fetch(&voucher_pda(campaign, nonce))
    }
}
```

Create `programs/hodl_loans/tests/promo_redeem.rs`:

```rust
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
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test promo_redeem`
Expected: `error[E0422]: cannot find struct, variant or union type RedeemPromo in module hodl_loans::instruction` and its `accounts` pair, the same for `CloseVoucherReceipt`, and `error[E0412]: cannot find type VoucherReceipt in crate hodl_loans`.

- [ ] **Step 3: Implement**

Add the account to `programs/hodl_loans/src/state/promo.rs`:

```rust
/// Spec §12. Proof that one voucher nonce has been redeemed. Its existence is what prevents a
/// replay: `redeem_promo` creates it, so a second redemption of the same nonce cannot init.
#[account]
#[derive(InitSpace)]
pub struct VoucherReceipt {
    pub version: u8,
    pub bump: u8,
    pub campaign: Pubkey,
    pub nonce: u64,
    pub voucher_expiry: i64,
    /// Refunded when the receipt is closed, which anyone may do once the voucher has expired.
    pub rent_payer: Pubkey,
    pub reserved: [u8; 32],
}
```

Create `programs/hodl_loans/src/instructions/promos/redeem.rs`:

```rust
use anchor_lang::prelude::*;
use solana_instructions_sysvar::ID as INSTRUCTIONS_SYSVAR_ID;

use crate::constants::{ACCESS_SEED, ACCOUNT_VERSION, CAMPAIGN_SEED, CONFIG_SEED, POSITION_SEED, PROMO_VAULT_SEED, VOUCHER_SEED};
use crate::errors::HodlError;
use crate::events::PromoRedeemed;
use crate::math::checked::{add, sub, to_u64};
use crate::state::{Access, Campaign, Config, Market, Position, PromoVault, VoucherReceipt};
use crate::voucher::{require_ed25519_signature, PromoVoucher};

#[derive(Accounts)]
#[instruction(amount: u64, nonce: u64)]
pub struct RedeemPromo<'info> {
    /// Pays the receipt's rent (e.g. the gas-relay sponsor) and is refunded when it is closed.
    #[account(mut)]
    pub payer: Signer<'info>,
    pub owner: Signer<'info>,
    #[account(seeds = [ACCESS_SEED, owner.key().as_ref()], bump = access.bump)]
    pub access: Box<Account<'info, Access>>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    pub market: Box<Account<'info, Market>>,
    #[account(mut, seeds = [POSITION_SEED, owner.key().as_ref()], bump)]
    pub position: AccountLoader<'info, Position>,
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
    #[account(
        mut,
        seeds = [CAMPAIGN_SEED, market.key().as_ref(), &campaign.campaign_id.to_le_bytes()],
        bump = campaign.bump,
        has_one = market
    )]
    pub campaign: Box<Account<'info, Campaign>>,
    /// Its creation is what stops a voucher being redeemed twice: the second attempt cannot
    /// initialize an account that already exists.
    #[account(
        init,
        payer = payer,
        space = 8 + VoucherReceipt::INIT_SPACE,
        seeds = [VOUCHER_SEED, campaign.key().as_ref(), &nonce.to_le_bytes()],
        bump
    )]
    pub voucher_receipt: Box<Account<'info, VoucherReceipt>>,
    /// CHECK: the address is the only thing that matters; the sysvar's contents are read
    /// through `load_instruction_at_checked`, which validates the layout itself.
    #[account(address = INSTRUCTIONS_SYSVAR_ID)]
    pub instructions_sysvar: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

/// Spec §12 `redeem_promo`. Turns a voucher the promo signer issued off-chain into promo
/// borrowing power on the caller's position, against a campaign's reserved budget.
///
/// Nothing moves: the cNGN backing the promo is already in the vault, and redeeming only moves
/// it from `unissued` (promised to a campaign) to `outstanding` (promised to a position).
pub fn handle_redeem_promo(
    ctx: Context<RedeemPromo>,
    amount: u64,
    nonce: u64,
    voucher_expiry: i64,
) -> Result<()> {
    ctx.accounts.access.require_active()?;
    require!(amount > 0, HodlError::AmountTooSmall);
    let now = Clock::get()?.unix_timestamp;

    // The program builds the message it expects and requires that exact signature, so the
    // wallet, amount, nonce and expiry below are the ones the signer actually authorised.
    let voucher = PromoVoucher::new(
        ctx.accounts.market.key(),
        ctx.accounts.campaign.campaign_id,
        ctx.accounts.owner.key(),
        amount,
        nonce,
        voucher_expiry,
    );
    require_ed25519_signature(
        &ctx.accounts.instructions_sysvar.to_account_info(),
        &ctx.accounts.config.promo_signer,
        &voucher.message()?,
    )?;

    require!(now <= voucher_expiry, HodlError::VoucherExpired);
    require!(ctx.accounts.campaign.active, HodlError::CampaignInactive);
    require!(now <= ctx.accounts.campaign.redeem_until, HodlError::CampaignInactive);

    let granted = add(ctx.accounts.campaign.granted as u128, amount as u128)?;
    require!(granted <= ctx.accounts.campaign.budget as u128, HodlError::CampaignBudgetExceeded);

    {
        let position = ctx.accounts.position.load()?;
        require_keys_eq!(position.owner, ctx.accounts.owner.key(), HodlError::Unauthorized);
        // A position is bound to one market from its first loan; promo is market-scoped too.
        if position.market != Pubkey::default() {
            require_keys_eq!(position.market, ctx.accounts.market.key(), HodlError::MarketMismatch);
        }
        let balance = add(position.promo_balance as u128, amount as u128)?;
        require!(
            balance <= ctx.accounts.market.max_promo_per_position as u128,
            HodlError::PromoCapExceeded
        );
    }

    ctx.accounts.campaign.granted = to_u64(granted)?;
    let promo_vault = &mut ctx.accounts.promo_vault;
    promo_vault.unissued = to_u64(sub(promo_vault.unissued as u128, amount as u128)?)?;
    promo_vault.outstanding = to_u64(add(promo_vault.outstanding as u128, amount as u128)?)?;
    promo_vault.require_invariant()?;

    ctx.accounts.voucher_receipt.set_inner(VoucherReceipt {
        version: ACCOUNT_VERSION,
        bump: ctx.bumps.voucher_receipt,
        campaign: ctx.accounts.campaign.key(),
        nonce,
        voucher_expiry,
        rent_payer: ctx.accounts.payer.key(),
        reserved: [0; 32],
    });

    let mut position = ctx.accounts.position.load_mut()?;
    // Promo is backed by one market's vault and counted against its cap, so redeeming binds the
    // position to that market exactly as a first loan would — and `expire_promo` needs to know
    // which vault to credit when the position has never borrowed.
    position.market = ctx.accounts.market.key();
    position.promo_balance = to_u64(add(position.promo_balance as u128, amount as u128)?)?;
    position.promo_last_activity_at = now;
    emit!(PromoRedeemed {
        market: ctx.accounts.market.key(),
        position: ctx.accounts.position.key(),
        owner: position.owner,
        campaign_id: ctx.accounts.campaign.campaign_id,
        nonce,
        amount,
        promo_balance: position.promo_balance,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct CloseVoucherReceipt<'info> {
    /// CHECK: only receives the rent it paid; the receipt names it.
    #[account(mut, address = voucher_receipt.rent_payer)]
    pub rent_payer: UncheckedAccount<'info>,
    #[account(mut, close = rent_payer)]
    pub voucher_receipt: Box<Account<'info, VoucherReceipt>>,
}

/// Spec §12. Once a voucher can no longer be redeemed, its receipt has nothing left to prevent,
/// so anyone may close it and return the rent to whoever paid it.
pub fn handle_close_voucher_receipt(ctx: Context<CloseVoucherReceipt>) -> Result<()> {
    require!(
        Clock::get()?.unix_timestamp > ctx.accounts.voucher_receipt.voucher_expiry,
        HodlError::PromoNotExpired
    );
    Ok(())
}
```

Add `pub mod redeem;` and `pub use redeem::*;` to the promos module, the event:

```rust
#[event]
pub struct PromoRedeemed {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub campaign_id: u64,
    pub nonce: u64,
    pub amount: u64,
    pub promo_balance: u64,
}
```

and the entry points:

```rust
    pub fn redeem_promo(
        ctx: Context<RedeemPromo>,
        amount: u64,
        nonce: u64,
        voucher_expiry: i64,
    ) -> Result<()> {
        instructions::handle_redeem_promo(ctx, amount, nonce, voucher_expiry)
    }

    pub fn close_voucher_receipt(ctx: Context<CloseVoucherReceipt>) -> Result<()> {
        instructions::handle_close_voucher_receipt(ctx)
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, 193 tests in all (`promo_redeem` is new with 13).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: redeem a promo voucher against a campaign"
```

---

### Task 5: Promo in the health check

**Files:**
- Create: `programs/hodl_loans/tests/promo_health.rs`
- Modify: `src/math/health.rs`, `src/valuation.rs`, `src/instructions/loans/take_loan.rs`, `src/instructions/positions/withdraw_collateral.rs`, `src/instructions/liquidation/liquidate.rs`, `src/instructions/liquidation/write_off.rs`, `tests/common/mod.rs`, `tests/loans.rs`, `tests/budget.rs`

**Interfaces:**
- Consumes: `Config::promo_cap_bps`, `Position::promo_balance`, `token_value`, `mul_div_floor`.
- Produces:
  - `Health.promo_counted`; `compute_health(collateral, debt_cngn, cngn_decimals, ngn, promo_balance, promo_cap_bps)`
  - `valuation::ValuationRequest`; `load_valuation(position, request)` and `load_health(position, request)`
  - `Config` on `take_loan`, `withdraw_collateral` and `liquidate`, boxed
  - Harness: `Env::promo_ready()`, `Env::set_max_promo_per_position(mint, max)`

- [ ] **Step 1: Write the harness and the failing tests**

Append to `programs/hodl_loans/tests/common/mod.rs`:

```rust
// ---- Promo in the health check (Task 5) ----

impl Env {
    /// `Env::loan_ready` plus a funded promo vault and one open campaign, so a borrower can
    /// hold promo while borrowing against real collateral.
    pub fn promo_ready() -> (Self, LoanSetup) {
        let (mut env, setup) = Self::loan_ready();
        let admin = env.admin.pubkey();
        let source = env.create_token_account(&setup.cngn, &admin);
        env.mint_to(&setup.cngn, &source, 10_000_000 * ONE_CNGN);
        let fund = fund_promo_vault_ix(&admin, &setup.cngn, &source, 10_000_000 * ONE_CNGN);
        send(&mut env.svm, &[fund], &[&env.admin]).expect("fund promo vault");
        env.create_campaign(&setup.cngn, 1, 5_000_000 * ONE_CNGN);
        (env, setup)
    }

    /// Raises the per-position promo ceiling, for the cases that need more than the default.
    pub fn set_max_promo_per_position(&mut self, mint: &Pubkey, max: u64) {
        let params = hodl_loans::MarketParams { max_promo_per_position: max, ..default_market_params() };
        let instruction = update_market_params_ix(&self.admin.pubkey(), mint, params);
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("update market params");
    }
}
```

and add `config: config_pda(),` to the `TakeLoan`, `WithdrawCollateral` and `Liquidate` account literals, each immediately after `access`/`liquidator`.

One existing test reaches into the account list by index, which this task's new account shifts. In `programs/hodl_loans/tests/loans.rs`, in `price_accounts_must_match_the_positions_slots`:

```rust
    let mut instruction = take(good.clone());
    // Found by key rather than by index: the account list grows between plans.
    let slot = instruction.accounts.iter().position(|a| a.pubkey == ngn_feed()).unwrap();
    instruction.accounts[slot] = AccountMeta::new_readonly(pyth_account(&setup.usdc), false);
```

Create `programs/hodl_loans/tests/promo_health.rs`:

```rust
mod common;

use common::*;
use hodl_loans::HodlError;

const DAY: i64 = 86_400;
/// 1,000 USDC at 70% LTV backs $700, which is 1,118,881 cNGN at the NGN ask.
const OWN_CEILING: u64 = 1_118_881 * ONE_CNGN;
/// The same position holding 50,000 cNGN of promo: $700 + $31.21875 of counted promo.
const WITH_PROMO_CEILING: u64 = 1_168_781 * ONE_CNGN;
/// Promo worth $312.19 but capped at 20% of $1,000 of collateral, so $900 in all.
const CAPPED_CEILING: u64 = 1_438_561 * ONE_CNGN;

#[test]
fn promo_lifts_the_borrow_limit_by_what_it_is_worth() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;

    // Before any promo, the collateral alone sets the ceiling.
    assert_hodl_error(env.take_loan(borrower, &setup, OWN_CEILING + ONE_CNGN, 30 * DAY), HodlError::Unhealthy);

    env.redeem_promo(borrower, &setup.cngn, 1, 50_000 * ONE_CNGN, 7).unwrap();

    // The same loan the collateral could not carry now fits, and the new ceiling is exact.
    assert_hodl_error(env.take_loan(borrower, &setup, WITH_PROMO_CEILING + ONE_CNGN, 30 * DAY), HodlError::Unhealthy);
    env.take_loan(borrower, &setup, WITH_PROMO_CEILING, 30 * DAY).unwrap();
    assert_eq!(env.token_balance(&setup.borrower_cngn), WITH_PROMO_CEILING);
}

#[test]
fn promo_counts_only_up_to_a_fifth_of_the_borrowers_own_collateral() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    env.set_max_promo_per_position(&setup.cngn, 1_000_000 * ONE_CNGN);

    // 500,000 cNGN of promo is worth $312.19, but only $200 of it can count against $1,000
    // of collateral at the 20% cap — so the ceiling stops at $900, not $1,012.19.
    env.redeem_promo(borrower, &setup.cngn, 1, 500_000 * ONE_CNGN, 7).unwrap();
    assert_eq!(env.position(&borrower.pubkey()).promo_balance, 500_000 * ONE_CNGN);

    assert_hodl_error(env.take_loan(borrower, &setup, CAPPED_CEILING + ONE_CNGN, 30 * DAY), HodlError::Unhealthy);
    env.take_loan(borrower, &setup, CAPPED_CEILING, 30 * DAY).unwrap();
}

#[test]
fn promo_is_worth_nothing_to_a_position_holding_no_collateral() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = env.new_borrower();
    let borrower_cngn = env.create_token_account(&setup.cngn, &borrower.pubkey());
    let setup = LoanSetup { borrower, borrower_cngn, ..setup };

    env.redeem_promo(&setup.borrower, &setup.cngn, 1, 50_000 * ONE_CNGN, 7).unwrap();
    assert_eq!(env.position(&setup.borrower.pubkey()).promo_balance, 50_000 * ONE_CNGN);

    // The cap is a fraction of the borrower's own collateral, and a fraction of nothing is
    // nothing — this is what stops promo being borrowed against on its own.
    let result = env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 30 * DAY);
    assert_hodl_error(result, HodlError::Unhealthy);
}

#[test]
fn promo_lifts_the_liquidation_line_with_the_borrow_limit() {
    // Two identical positions and one price crash: the promo decides which is liquidatable.
    let (mut env, setup) = Env::promo_ready();
    let plain = &setup.borrower;
    env.take_loan(plain, &setup, 700_000 * ONE_CNGN, 365 * DAY).unwrap();

    let promoed = env.new_borrower();
    let owner = promoed.pubkey();
    env.deposit_collateral(&promoed, &setup.usdc, 1_000 * ONE_USDC);
    let promoed_cngn = env.create_token_account(&setup.cngn, &owner);
    env.redeem_promo(&promoed, &setup.cngn, 1, 50_000 * ONE_CNGN, 7).unwrap();
    let prices = env.price_accounts(&owner);
    let borrow = take_loan_ix(&owner, &setup.cngn, &promoed_cngn, 700_000 * ONE_CNGN, 365 * DAY, prices);
    send(&mut env.svm, &[borrow], &[&env.admin, &promoed.key]).unwrap();

    // $0.4525 leaves the plain position under its 90% line ($407.25 against $437.94 of debt)
    // and the promoed one just over it ($438.47), on $31.21875 of counted promo.
    env.set_pyth_price(&setup.usdc, 45_250_000, 0);
    let liquidator = env.new_liquidator(&setup.cngn, 1_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    let prices = env.price_accounts(&owner);
    let seize = liquidate_ix(
        &liquidator.pubkey(),
        &owner,
        &setup.cngn,
        &liquidator.cngn,
        &setup.usdc,
        &SPL_TOKEN,
        &seized_to,
        0,
        1_000 * ONE_CNGN,
        prices,
    );
    let result = send(&mut env.svm, &[seize], &[&liquidator.key]);
    assert_hodl_error(result, HodlError::NotLiquidatable);

    // The only difference between the two positions is the promo.
    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 1_000 * ONE_CNGN).unwrap();
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test promo_health`
Expected: `error[E0560]: struct hodl_loans::accounts::TakeLoan has no field named config`, and the same for `WithdrawCollateral` and `Liquidate`.

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
    /// Promo actually counted: the position's promo valued at the NGN bid, capped at
    /// `promo_cap_bps` of the collateral the borrower put up themselves. It lifts both limits
    /// below, and is zero for a position holding no collateral of its own.
    pub promo_counted: u128,
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
    promo_balance: u64,
    promo_cap_bps: u16,
) -> Result<Health> {
    let mut health = Health::default();
    let mut promo_cap_total: u128 = 0;
    for c in collateral {
        let value = token_value(c.display_amount()?, c.decimals, c.price.lower())?;
        health.own_value = add(health.own_value, value)?;
        health.borrow_limit = add(health.borrow_limit, mul_div_floor(value, c.ltv_bps as u128, BPS)?)?;
        health.liquidation_line = add(
            health.liquidation_line,
            mul_div_floor(value, c.liquidation_threshold_bps as u128, BPS)?,
        )?;
        // Floor the promo cap PER ASSET, inside this same loop, instead of once on the
        // aggregate `own_value` below. `floor(sum(v_i) * bps / BPS)` can exceed
        // `sum(floor(v_i * bps / BPS))` by up to n-1 base units; at the shipped defaults
        // (ltv 7000 + cap 2000 == lt 9000, zero slack) that gap alone was enough to push a
        // maxed-out multi-asset position's `borrow_limit` a hair above `liquidation_line` the
        // moment the cap changed — instant liquidation with no price move. Sum-of-floors ≤
        // floor-of-sum always, so accumulating here is conservative: promo counts for slightly
        // less, never more.
        promo_cap_total = add(promo_cap_total, mul_div_floor(value, promo_cap_bps as u128, BPS)?)?;
    }
    // Spec §12: promo is a topping on collateral the borrower owns, never a substitute for it.
    // The cap is `promo_cap_total`, summed per asset in the loop above rather than taken as a
    // single fraction of the aggregate `own_value` — flooring the aggregate can exceed the sum
    // of the per-asset floors by up to n-1 base units, which at the zero-slack shipped defaults
    // (ltv 7000 + cap 2000 == lt 9000) can make a position borrowed to its limit liquidatable the
    // moment the cap is lowered. A position with nothing of its own still counts none of it
    // either way — which is what makes defaulting a loss for the borrower rather than a way to
    // profit.
    let promo_value = token_value(promo_balance as u128, cngn_decimals, ngn.lower())?;
    health.promo_counted = promo_value.min(promo_cap_total);
    health.borrow_limit = add(health.borrow_limit, health.promo_counted)?;
    health.liquidation_line = add(health.liquidation_line, health.promo_counted)?;

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
        let h = compute_health(&collateral, 1_600_000_000_000, 6, ngn(), 0, 0).unwrap();
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
        let at_limit = compute_health(&collateral, 1_120_000_000_000, 6, ngn(), 0, 0).unwrap();
        assert_eq!(at_limit.debt, 700 * USD);
        assert!(at_limit.is_healthy());
        // A 1% spread pushes the same debt over the limit.
        let wide = UsdPrice { price: 625_000_000, conf: 6_250_000 };
        let over = compute_health(&collateral, 1_120_000_000_000, 6, wide, 0, 0).unwrap();
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
        let h = compute_health(&split, 0, 6, ngn(), 0, 0).unwrap();
        assert_eq!(h.own_value, 30_000 * USD);
        assert_eq!(h.borrow_limit, 15_000 * USD);
        assert_eq!(h.liquidation_line, 22_500 * USD);
        // The same holding at multiplier 1 is worth the raw balance.
        let plain = [CollateralValue { multiplier: MULTIPLIER_SCALE, ..split[0] }];
        assert_eq!(compute_health(&plain, 0, 6, ngn(), 0, 0).unwrap().own_value, 20_000 * USD);
    }

    #[test]
    fn display_amount_rounds_down_not_up() {
        // A live AAPLX-shaped multiplier (token/scaled_ui.rs documents this exact value)
        // applied to a raw balance of 3: floor gives 3 display units, ceil would give 4. An
        // xStock balance must never be valued above what it represents.
        let c = CollateralValue {
            amount: 3,
            decimals: 8,
            multiplier: 1_000_899_999_999,
            price: UsdPrice { price: 200 * USD, conf: 0 },
            ltv_bps: 5_000,
            liquidation_threshold_bps: 7_500,
        };
        assert_eq!(c.display_amount().unwrap(), 3);
    }

    #[test]
    fn no_collateral_means_any_debt_is_unhealthy() {
        let h = compute_health(&[], 1, 6, ngn(), 0, 0).unwrap();
        assert_eq!(h.borrow_limit, 0);
        assert!(!h.is_healthy());
        assert!(h.is_liquidatable());
        assert!(compute_health(&[], 0, 6, ngn(), 0, 0).unwrap().is_healthy());
    }
}
```

Replace `programs/hodl_loans/src/valuation.rs`. The positional argument list had reached eight, with `extra_debt` and `promo_cap_bps` both small integers that would transpose silently, so it becomes a request struct:

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
use crate::token::scaled_ui::read_xstock_multiplier;

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
        // The match is exhaustive, so a third `CollateralKind` has to state its own stride and
        // multiplier here rather than silently inherit the xStock's.
        let (stride, multiplier) = match asset.kind {
            CollateralKind::Standard => (ACCOUNTS_PER_COLLATERAL, MULTIPLIER_ONE),
            CollateralKind::XStock => {
                let mint_info = remaining.get(cursor + 2).ok_or(HodlError::PriceAccountMismatch)?;
                require_keys_eq!(mint_info.key(), slot.mint, HodlError::PriceAccountMismatch);
                (ACCOUNTS_PER_XSTOCK, read_xstock_multiplier(mint_info, clock.unix_timestamp)?)
            }
        };
        cursor += stride;
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
/// Everything a health check needs besides the position itself. Grouped rather than passed
/// positionally: the list grew past what a reader can keep straight, and `extra_debt` and
/// `promo_cap_bps` are both small integers that would transpose silently.
pub struct ValuationRequest<'a, 'info> {
    pub program_id: &'a Pubkey,
    pub market: &'a Market,
    pub ngn_feed: &'a AccountInfo<'info>,
    pub remaining: &'a [AccountInfo<'info>],
    /// Debt to count on top of the position's own — the loan `take_loan` is about to write.
    pub extra_debt: u64,
    pub promo_cap_bps: u16,
    pub clock: &'a Clock,
}

pub fn load_valuation(position: &Position, request: &ValuationRequest) -> Result<Valuation> {
    let ValuationRequest { program_id, market, ngn_feed, remaining, extra_debt, promo_cap_bps, clock } = *request;
    let collateral = load_collateral_values(program_id, position, remaining, clock)?;
    let ngn = read_ngn_price(ngn_feed, market, clock)?;
    let debt = add(total_debt(position, clock.unix_timestamp)?, extra_debt as u128)?;
    let health = compute_health(
        &collateral,
        debt,
        market.decimals,
        ngn,
        position.promo_balance,
        promo_cap_bps,
    )?;
    Ok(Valuation { health, collateral, ngn })
}

/// Spec §8 health for a position, optionally including `extra_debt` about to be borrowed.
pub fn load_health(position: &Position, request: &ValuationRequest) -> Result<Health> {
    Ok(load_valuation(position, request)?.health)
}
```

Each of the four health-checking instructions now takes `Config` — **boxed**, because `liquidate` is close enough to the 4 KB SBF stack frame that an unboxed one makes it fail before the handler runs. Add to `take_loan`, `withdraw_collateral` and `liquidate` (and box the one `write_off_loan` already has):

```rust
    /// Carries `promo_cap_bps`, which bounds how much of a position's promo counts (spec §12).
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
```

and convert each call site to the request form, for example in `take_loan`:

```rust
        let ngn_feed = ctx.accounts.ngn_feed.to_account_info();
        let health = load_health(
            &position,
            &ValuationRequest {
                program_id: ctx.program_id,
                market,
                ngn_feed: &ngn_feed,
                remaining: ctx.remaining_accounts,
                extra_debt: amount,
                promo_cap_bps: ctx.accounts.config.promo_cap_bps,
                clock: &clock,
            },
        )?;
```

Finally, promo costs roughly 4,000 CU on every health check, which puts `withdraw_collateral` over its old ceiling. In `programs/hodl_loans/tests/budget.rs`, record the new measurements and raise the two tightest limits to `85_000`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, 202 tests in all (`promo_health` is new with 4).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: count promo in the health check, capped by own collateral"
```

---

### Task 6: Expiry, revocation and release

**Files:**
- Create: `programs/hodl_loans/src/instructions/promos/lifecycle.rs`, `tests/promo_lifecycle.rs`
- Modify: `src/instructions/promos/redeem.rs`, `src/instructions/promos/mod.rs`, `src/instructions/loans/take_loan.rs`, `src/instructions/positions/close_position.rs`, `src/events.rs`, `src/lib.rs`, `tests/common/mod.rs`, `tests/loans.rs`

**Interfaces:**
- Consumes: `PromoVault::release`, `Position::has_active_loans`, `Market::promo_inactivity_seconds`.
- Produces:
  - `instructions::promos::release_promo(position, promo_vault) -> Result<u64>` — the one way promo leaves a position
  - `expire_promo` (anyone), `revoke_promo` (admin)
  - `take_loan` gains an optional promo vault and performs spec §10 step 3; `close_position` gains optional `market` and `promo_vault`; `redeem_promo` binds `position.market`
  - Harness: `expire_promo_ix`, `revoke_promo_ix`, `close_position_with_promo_ix`

- [ ] **Step 1: Write the harness and the failing tests**

Append to `programs/hodl_loans/tests/common/mod.rs`:

```rust
// ---- Promo expiry and revocation (Task 6) ----

pub fn expire_promo_ix(mint: &Pubkey, owner: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::ExpirePromo {},
        hodl_loans::accounts::ExpirePromo {
            market: market_pda(mint),
            promo_vault: promo_vault_pda(mint),
            position: position_pda(owner),
        },
    )
}

pub fn revoke_promo_ix(admin: &Pubkey, mint: &Pubkey, owner: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::RevokePromo {},
        hodl_loans::accounts::RevokePromo {
            admin: *admin,
            config: config_pda(),
            market: market_pda(mint),
            promo_vault: promo_vault_pda(mint),
            position: position_pda(owner),
        },
    )
}

/// `close_position`, naming the promo vault so a position still holding promo can hand it back.
pub fn close_position_with_promo_ix(owner: &Pubkey, rent_payer: &Pubkey, mint: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::ClosePosition {},
        hodl_loans::accounts::ClosePosition {
            market: Some(market_pda(mint)),
            promo_vault: Some(promo_vault_pda(mint)),
            owner: *owner,
            access: access_pda(owner),
            position: position_pda(owner),
            rent_payer: *rent_payer,
        },
    )
}
```

In the same file, `take_loan_ix` now names the promo vault, and `close_position_ix` passes `None` for both new optional accounts:

```rust
            config: config_pda(),
            promo_vault: Some(promo_vault_pda(mint)),
```

```rust
            market: None,
            promo_vault: None,
            owner: *owner,
```

`tests/loans.rs` builds a second market directly; it needs a promo vault now that `take_loan` names one. Replace the two lines that create it with `env.create_market_with_promo(&other);`.

Create `programs/hodl_loans/tests/promo_lifecycle.rs`:

```rust
mod common;

use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

const DAY: i64 = 86_400;
/// `default_market_params` leaves promo idle for 90 days before anyone can reclaim it.
const INACTIVITY: i64 = 90 * DAY;
const GRANT: u64 = 50_000 * ONE_CNGN;
const OWN_CEILING: u64 = 1_118_881 * ONE_CNGN;
const WITH_PROMO_CEILING: u64 = 1_168_781 * ONE_CNGN;

#[test]
fn idle_promo_expires_back_into_the_vault() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();
    let committed = env.promo_vault(&setup.cngn).free().unwrap();

    env.warp_seconds(INACTIVITY);
    // Anyone may do this — it is the protocol's own housekeeping, not the borrower's.
    let stranger = env.funded_keypair();
    send(&mut env.svm, &[expire_promo_ix(&setup.cngn, &owner)], &[&stranger]).unwrap();

    assert_eq!(env.position(&owner).promo_balance, 0);
    let vault = env.promo_vault(&setup.cngn);
    assert_eq!(vault.outstanding, 0);
    // The cNGN never moved; it is simply free again.
    assert_eq!(vault.free().unwrap(), committed + GRANT);
    assert_eq!(env.token_balance(&promo_vault_token_pda(&setup.cngn)), vault.cash);
}

#[test]
fn promo_will_not_expire_early_or_under_a_live_loan() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();

    // One second short of the window.
    env.warp_seconds(INACTIVITY - 1);
    let early = expire_promo_ix(&setup.cngn, &owner);
    assert_hodl_error(send(&mut env.svm, &[early], &[&env.admin]), HodlError::PromoNotExpired);

    // Borrowing restarts the clock, and promo cannot be pulled from under a live loan.
    env.set_pyth_price(&setup.usdc, ONE_DOLLAR, 0);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    env.take_loan(borrower, &setup, 1_000 * ONE_CNGN, 365 * DAY).unwrap();
    env.warp_seconds(INACTIVITY + 1);
    let live = expire_promo_ix(&setup.cngn, &owner);
    assert_hodl_error(send(&mut env.svm, &[live], &[&env.admin]), HodlError::PositionNotEmpty);

    // And a position with no promo has nothing to expire.
    let empty = env.new_borrower();
    let none = expire_promo_ix(&setup.cngn, &empty.pubkey());
    assert_hodl_error(send(&mut env.svm, &[none], &[&env.admin]), HodlError::AmountTooSmall);
}

#[test]
fn taking_a_loan_expires_stale_promo_before_pricing_the_position() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();
    env.warp_seconds(INACTIVITY);
    env.set_pyth_price(&setup.usdc, ONE_DOLLAR, 0);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);

    // Spec §10 step 3: the promo is gone before the health check runs, so the loan the promo
    // would have supported no longer fits.
    let result = env.take_loan(borrower, &setup, WITH_PROMO_CEILING, 30 * DAY);
    assert_hodl_error(result, HodlError::Unhealthy);

    env.take_loan(borrower, &setup, OWN_CEILING, 30 * DAY).unwrap();
    assert_eq!(env.position(&borrower.pubkey()).promo_balance, 0);
    assert_eq!(env.promo_vault(&setup.cngn).outstanding, 0);
}

#[test]
fn an_admin_can_revoke_promo_without_waiting() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    let admin = env.admin.pubkey();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();

    let stranger = env.funded_keypair();
    let by_stranger = revoke_promo_ix(&stranger.pubkey(), &setup.cngn, &owner);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);

    send(&mut env.svm, &[revoke_promo_ix(&admin, &setup.cngn, &owner)], &[&env.admin]).unwrap();
    assert_eq!(env.position(&owner).promo_balance, 0);
    assert_eq!(env.promo_vault(&setup.cngn).outstanding, 0);

    // Revocation is still barred while a loan is live, so it cannot force a liquidation.
    let other = env.new_borrower();
    env.deposit_collateral(&other, &setup.usdc, 1_000 * ONE_USDC);
    let other_cngn = env.create_token_account(&setup.cngn, &other.pubkey());
    env.redeem_promo(&other, &setup.cngn, 1, GRANT, 8).unwrap();
    let prices = env.price_accounts(&other.pubkey());
    let borrow = take_loan_ix(&other.pubkey(), &setup.cngn, &other_cngn, 1_000 * ONE_CNGN, 365 * DAY, prices);
    send(&mut env.svm, &[borrow], &[&env.admin, &other.key]).unwrap();
    let live = revoke_promo_ix(&admin, &setup.cngn, &other.pubkey());
    assert_hodl_error(send(&mut env.svm, &[live], &[&env.admin]), HodlError::PositionNotEmpty);
}

#[test]
fn closing_a_position_hands_its_promo_back() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = env.new_borrower();
    let owner = borrower.pubkey();
    // Redemption only moves GRANT from `unissued` (promised to the campaign) to `outstanding`
    // (promised to the position) — `free()` does not move yet.
    let free_before = env.promo_vault(&setup.cngn).free().unwrap();
    env.redeem_promo(&borrower, &setup.cngn, 1, GRANT, 7).unwrap();
    assert_eq!(env.promo_vault(&setup.cngn).outstanding, GRANT);
    assert_eq!(env.promo_vault(&setup.cngn).free().unwrap(), free_before);

    // Closing without naming the vault would strand the promo, so it is refused.
    let bare = close_position_ix(&owner, &env.admin.pubkey());
    assert_hodl_error(env.sponsored(bare, &borrower.key), HodlError::MarketMismatch);
    // The position, and the vault's committed promo, are both still there: the refusal did not
    // silently drop the promo along with the close.
    assert_eq!(env.promo_vault(&setup.cngn).outstanding, GRANT);

    let close = close_position_with_promo_ix(&owner, &env.admin.pubkey(), &setup.cngn);
    env.sponsored(close, &borrower.key).unwrap();
    // The position is gone, so `position.promo_balance == 0` alone would prove nothing — the
    // vault side is what confirms the promo actually came back rather than being stranded.
    // Release does not credit the campaign back (its `granted` stays permanently spent), so the
    // GRANT becomes genuinely free vault cash — `free()` ends up `free_before + GRANT`, not
    // merely back at `free_before`.
    let vault = env.promo_vault(&setup.cngn);
    assert_eq!(vault.outstanding, 0);
    assert_eq!(vault.free().unwrap(), free_before + GRANT);
    assert!(env.svm.get_account(&position_pda(&owner)).is_none_or(|a| a.lamports == 0));
}

#[test]
fn the_promo_clock_restarts_only_when_the_last_loan_closes() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();
    env.take_loan(borrower, &setup, 1_000 * ONE_CNGN, 365 * DAY).unwrap();
    env.take_loan(borrower, &setup, 1_000 * ONE_CNGN, 365 * DAY).unwrap();
    let after_second = env.position(&owner).promo_last_activity_at;

    // Repaying one of two loans leaves the clock alone: the position is still active.
    env.warp_seconds(DAY);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    env.mint_to(&setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN);
    env.repay(&setup, 0, 2_000 * ONE_CNGN).unwrap();
    assert_eq!(env.position(&owner).promo_last_activity_at, after_second);

    // Repaying the last one restarts it, which is when the idle window begins.
    env.warp_seconds(DAY);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    env.mint_to(&setup.cngn, &setup.borrower_cngn, 1_000 * ONE_CNGN);
    env.repay(&setup, 1, 2_000 * ONE_CNGN).unwrap();
    assert_eq!(env.position(&owner).promo_last_activity_at, env.now());
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test promo_lifecycle`
Expected: `error[E0422]: cannot find struct, variant or union type ExpirePromo in module hodl_loans::instruction` and its `accounts` pair, the same for `RevokePromo`, `error[E0560]: struct hodl_loans::accounts::ClosePosition has no field named market` (and `promo_vault`), and `error[E0560]: struct hodl_loans::accounts::TakeLoan has no field named promo_vault` — seven distinct errors, which cargo reports as nine because two of them occur at more than one call site.

- [ ] **Step 3: Implement**

Create `programs/hodl_loans/src/instructions/promos/lifecycle.rs` with the release helpers and the two instructions — forfeiture arrives in Task 7:

```rust
use anchor_lang::prelude::*;

use crate::constants::{CONFIG_SEED, POSITION_SEED, PROMO_VAULT_SEED};
use crate::errors::HodlError;
use crate::events::{PromoExpired, PromoRevoked};
use crate::state::{Config, Market, Position, PromoVault};

/// The one way promo leaves a position: expiry, revocation, forfeiture on liquidation or
/// write-off, and `close_position` all end here. The cNGN itself does not move — it stops being
/// promised to this position and becomes free vault funds again.
pub fn release_promo(position: &mut Position, promo_vault: &mut PromoVault) -> Result<u64> {
    let amount = position.promo_balance;
    promo_vault.release(amount)?;
    position.promo_balance = 0;
    Ok(amount)
}

/// Shared by `expire_promo` and `revoke_promo`: promo only leaves a quiet position, so neither
/// can be used to strip borrowing power out from under a live loan.
fn release_from_idle_position(
    position: &mut Position,
    promo_vault: &mut PromoVault,
    market_key: Pubkey,
) -> Result<u64> {
    require!(position.promo_balance > 0, HodlError::AmountTooSmall);
    require!(!position.has_active_loans(), HodlError::PositionNotEmpty);
    require_keys_eq!(position.market, market_key, HodlError::MarketMismatch);
    release_promo(position, promo_vault)
}

#[derive(Accounts)]
pub struct ExpirePromo<'info> {
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
    #[account(mut, seeds = [POSITION_SEED, position.load()?.owner.as_ref()], bump)]
    pub position: AccountLoader<'info, Position>,
}

/// Spec §12. Anyone may reclaim promo a borrower has left idle, which is what stops granted
/// promo sitting on the books forever. The clock runs from the last redemption, the last loan
/// taken, or the moment the last loan closed.
pub fn handle_expire_promo(ctx: Context<ExpirePromo>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let market_key = ctx.accounts.market.key();
    let inactivity = ctx.accounts.market.promo_inactivity_seconds;
    let mut position = ctx.accounts.position.load_mut()?;
    require!(
        now >= position.promo_last_activity_at.saturating_add(inactivity),
        HodlError::PromoNotExpired
    );
    let amount = release_from_idle_position(&mut position, &mut ctx.accounts.promo_vault, market_key)?;
    emit!(PromoExpired {
        market: market_key,
        position: ctx.accounts.position.key(),
        owner: position.owner,
        amount,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct RevokePromo<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
    #[account(mut, seeds = [POSITION_SEED, position.load()?.owner.as_ref()], bump)]
    pub position: AccountLoader<'info, Position>,
}

/// Spec §12. The same release without waiting out the clock — for promo granted in error or to
/// an account the backend has since judged ineligible. It still cannot touch a position with a
/// live loan, so it cannot be used to push someone into liquidation.
pub fn handle_revoke_promo(ctx: Context<RevokePromo>) -> Result<()> {
    let market_key = ctx.accounts.market.key();
    let mut position = ctx.accounts.position.load_mut()?;
    let amount = release_from_idle_position(&mut position, &mut ctx.accounts.promo_vault, market_key)?;
    emit!(PromoRevoked {
        market: market_key,
        position: ctx.accounts.position.key(),
        owner: position.owner,
        amount,
    });
    Ok(())
}
```

The market binding in `redeem.rs` that this step used to add — `position.market = ctx.accounts.market.key();` and its comment — is already there: Task 4's code block carries it, so do not add it a second time.

In `programs/hodl_loans/src/instructions/loans/take_loan.rs`, add the optional account after `config`:

```rust
    /// Required when the position holds promo: step 3 may expire it, which credits the vault.
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market
    )]
    pub promo_vault: Option<Box<Account<'info, PromoVault>>>,
```

and perform spec §10 step 3 immediately before the health check:

```rust
        // Spec §10 step 3: a quiet position's promo expires before it can support a new loan.
        if position.promo_balance > 0
            && !position.has_active_loans()
            && now >= position.promo_last_activity_at.saturating_add(market.promo_inactivity_seconds)
        {
            let promo_vault = ctx
                .accounts
                .promo_vault
                .as_mut()
                .ok_or(HodlError::PromoVaultInsufficient)?;
            let amount = release_promo(&mut position, promo_vault)?;
            emit!(PromoExpired {
                market: market_key,
                position: ctx.accounts.position.key(),
                owner: position.owner,
                amount,
            });
        }
```

Replace `programs/hodl_loans/src/instructions/positions/close_position.rs`:

```rust
use anchor_lang::prelude::*;

use crate::constants::{ACCESS_SEED, POSITION_SEED, PROMO_VAULT_SEED};
use crate::errors::HodlError;
use crate::events::{PositionClosed, PromoReleased};
use crate::instructions::promos::release_promo;
use crate::state::{Access, Market, Position, PromoVault};

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
    /// Both required when the position still holds promo: closing it hands the promo back.
    pub market: Option<Box<Account<'info, Market>>>,
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, promo_vault.market.as_ref()],
        bump = promo_vault.bump
    )]
    pub promo_vault: Option<Box<Account<'info, PromoVault>>>,
}

/// Requires no collateral and no active loans. Any promo goes back to the vault, and the rent
/// goes back to whoever paid it.
pub fn handle_close_position(ctx: Context<ClosePosition>) -> Result<()> {
    ctx.accounts.access.require_active()?;
    let mut position = ctx.accounts.position.load_mut()?;
    require!(!position.has_collateral() && !position.has_active_loans(), HodlError::PositionNotEmpty);

    if position.promo_balance > 0 {
        let market = ctx.accounts.market.as_ref().ok_or(HodlError::MarketMismatch)?;
        let promo_vault = ctx.accounts.promo_vault.as_mut().ok_or(HodlError::MarketMismatch)?;
        require_keys_eq!(position.market, market.key(), HodlError::MarketMismatch);
        require_keys_eq!(promo_vault.market, market.key(), HodlError::MarketMismatch);
        let amount = release_promo(&mut position, promo_vault)?;
        emit!(PromoReleased {
            market: market.key(),
            position: ctx.accounts.position.key(),
            owner: position.owner,
            amount,
        });
    }

    emit!(PositionClosed {
        position: ctx.accounts.position.key(),
        owner: position.owner,
        rent_payer: position.rent_payer,
    });
    Ok(())
}
```

Add `pub mod lifecycle;` and `pub use lifecycle::*;` to the promos module, the events:

```rust
#[event]
pub struct PromoExpired {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
}

#[event]
pub struct PromoRevoked {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
}

#[event]
pub struct PromoReleased {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
}
```

and the entry points:

```rust
    pub fn expire_promo(ctx: Context<ExpirePromo>) -> Result<()> {
        instructions::handle_expire_promo(ctx)
    }

    pub fn revoke_promo(ctx: Context<RevokePromo>) -> Result<()> {
        instructions::handle_revoke_promo(ctx)
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, 212 tests in all (`promo_lifecycle` is new with 10).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: promo expiry, revocation and release on close"
```

---

### Task 7: Forfeiture to lenders

**Files:**
- Create: `programs/hodl_loans/tests/promo_forfeit.rs`
- Modify: `src/instructions/promos/lifecycle.rs`, `src/instructions/liquidation/liquidate.rs`, `src/instructions/liquidation/write_off.rs`, `src/events.rs`, `tests/common/mod.rs`, `tests/liquidation.rs`, `tests/budget.rs`

**Interfaces:**
- Consumes: `PromoVault::{release, require_invariant}`, `transfer_from_vault`.
- Produces:
  - `instructions::promos::{ForfeitAccounts, forfeit_promo}` — returns the amount, which the caller adds to `market.cash`
  - `liquidate` and `write_off_loan` gain the promo vault and its token account; `write_off_loan` also gains the market mint, the market vault and the token program, because until now it moved no tokens at all

- [ ] **Step 1: Write the harness and the failing test**

In `programs/hodl_loans/tests/common/mod.rs`, add the promo accounts to the `Liquidate` literal (after `config`) and to `WriteOffLoan`:

```rust
            promo_vault: promo_vault_pda(mint),
            promo_vault_token: promo_vault_token_pda(mint),
```

```rust
            mint: *mint,
            vault: market_vault_pda(mint),
            promo_vault: promo_vault_pda(mint),
            promo_vault_token: promo_vault_token_pda(mint),
            token_program: TOKEN_2022,
```

`tests/liquidation.rs` builds a second market directly; replace those two lines with `env.create_market_with_promo(&other);` as `tests/loans.rs` already does.

Create `programs/hodl_loans/tests/promo_forfeit.rs`:

```rust
mod common;

use common::*;
use anchor_lang::error::ErrorCode as AnchorError;
use anchor_lang::prelude::AccountMeta;
use solana_signer::Signer;

const DAY: i64 = 86_400;
const GRANT: u64 = 50_000 * ONE_CNGN;
const LOAN: u64 = 700_000 * ONE_CNGN;

/// A position holding 1,000 USDC and `GRANT` of promo, owing `LOAN`, with USDC crashed to
/// $0.45 — under its 90% line even with the promo counted.
fn underwater_with_promo() -> (Env, LoanSetup) {
    let (mut env, setup) = Env::promo_ready();
    env.redeem_promo(&setup.borrower, &setup.cngn, 1, GRANT, 7).unwrap();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    env.set_pyth_price(&setup.usdc, 45_000_000, 0);
    (env, setup)
}

#[test]
fn liquidating_a_position_hands_its_promo_to_lenders() {
    let (mut env, setup) = underwater_with_promo();
    let owner = setup.borrower.pubkey();
    let liquidator = env.new_liquidator(&setup.cngn, 1_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    let vault_before = env.promo_vault(&setup.cngn);
    let cash_before = env.market(&setup.cngn).cash;
    let market_tokens_before = env.token_balance(&market_vault_pda(&setup.cngn));
    let repaid_before = env.position(&owner).loans[0].repaid;

    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 100_000 * ONE_CNGN).unwrap();

    // The promo is gone from the position and from the vault's books, and this time the cNGN
    // really moved: `cash` falls with `outstanding`, unlike expiry.
    assert_eq!(env.position(&owner).promo_balance, 0);
    let vault = env.promo_vault(&setup.cngn);
    assert_eq!(vault.outstanding, vault_before.outstanding - GRANT);
    assert_eq!(vault.cash, vault_before.cash - GRANT);
    assert_eq!(env.token_balance(&promo_vault_token_pda(&setup.cngn)), vault.cash);

    // Lenders receive it on top of the liquidator's repayment.
    let paid = env.position(&owner).loans[0].repaid - repaid_before;
    assert_eq!(env.market(&setup.cngn).cash, cash_before + paid + GRANT);
    assert_eq!(env.token_balance(&market_vault_pda(&setup.cngn)), market_tokens_before + paid + GRANT);
}

#[test]
fn the_forfeit_does_not_reduce_what_the_borrower_owes() {
    // Two identical debts, one backed by promo. Liquidating both by the same amount must leave
    // the two loans in exactly the same state: the promo goes to lenders, not to the borrower's
    // balance. Anything else would mean the protocol had paid down its own borrower's debt.
    let (mut env, setup) = underwater_with_promo();
    let promoed = setup.borrower.pubkey();

    let plain = env.new_borrower();
    env.deposit_collateral(&plain, &setup.usdc, 1_000 * ONE_USDC);
    let plain_cngn = env.create_token_account(&setup.cngn, &plain.pubkey());
    env.set_pyth_price(&setup.usdc, ONE_DOLLAR, 0);
    let prices = env.price_accounts(&plain.pubkey());
    let borrow = take_loan_ix(&plain.pubkey(), &setup.cngn, &plain_cngn, LOAN, 365 * DAY, prices);
    send(&mut env.svm, &[borrow], &[&env.admin, &plain.key]).unwrap();
    env.set_pyth_price(&setup.usdc, 45_000_000, 0);

    let liquidator = env.new_liquidator(&setup.cngn, 2_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 100_000 * ONE_CNGN).unwrap();

    let prices = env.price_accounts(&plain.pubkey());
    let seize = liquidate_ix(
        &liquidator.pubkey(), &plain.pubkey(), &setup.cngn, &liquidator.cngn, &setup.usdc,
        &SPL_TOKEN, &seized_to, 0, 100_000 * ONE_CNGN, prices,
    );
    send(&mut env.svm, &[seize], &[&liquidator.key]).unwrap();

    let with_promo = env.position(&promoed).loans[0];
    let without = env.position(&plain.pubkey()).loans[0];
    assert_eq!(with_promo.principal, without.principal);
    assert_eq!(with_promo.repaid, without.repaid);
    assert!(with_promo.principal > 0);

    // The only difference is where the promo went.
    assert_eq!(env.position(&promoed).promo_balance, 0);
    assert_eq!(env.promo_vault(&setup.cngn).outstanding, 0);
}

#[test]
fn promo_is_forfeited_once_and_a_later_liquidation_finds_none() {
    let (mut env, setup) = underwater_with_promo();
    let owner = setup.borrower.pubkey();
    let liquidator = env.new_liquidator(&setup.cngn, 1_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 50_000 * ONE_CNGN).unwrap();
    let after_first = env.promo_vault(&setup.cngn);
    assert_eq!(env.position(&owner).promo_balance, 0);

    // The position is still under water, so it can be liquidated again — and the vault is not
    // charged a second time.
    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 50_000 * ONE_CNGN).unwrap();
    let after_second = env.promo_vault(&setup.cngn);
    assert_eq!((after_second.cash, after_second.outstanding), (after_first.cash, after_first.outstanding));
}

#[test]
fn writing_off_a_loan_forfeits_the_promo_as_well() {
    let (mut env, setup) = underwater_with_promo();
    let owner = setup.borrower.pubkey();
    let admin = env.admin.pubkey();

    // Collateral worth $1 is below the $5 dust threshold, so the loan can be written off.
    env.set_pyth_price(&setup.usdc, 100_000, 0);
    let vault_before = env.promo_vault(&setup.cngn);
    let cash_before = env.market(&setup.cngn).cash;

    let prices = env.price_accounts(&owner);
    let write_off = write_off_loan_ix(&admin, &owner, &setup.cngn, 0, prices);
    send(&mut env.svm, &[write_off], &[&env.admin]).unwrap();

    assert_eq!(env.position(&owner).promo_balance, 0);
    let vault = env.promo_vault(&setup.cngn);
    assert_eq!(vault.cash, vault_before.cash - GRANT);
    assert_eq!(vault.outstanding, vault_before.outstanding - GRANT);
    // The promo offsets part of the loss the lenders would otherwise carry alone.
    assert_eq!(env.market(&setup.cngn).cash, cash_before + GRANT);
    assert!(env.market(&setup.cngn).total_bad_debt > 0);
}

#[test]
fn a_liquidation_must_name_the_positions_own_promo_vault() {
    let (mut env, setup) = underwater_with_promo();
    let owner = setup.borrower.pubkey();
    let liquidator = env.new_liquidator(&setup.cngn, 1_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    // Another market's promo vault would let a liquidator skip the forfeit by charging a vault
    // the position never drew on. The seeds are derived from this market, so it cannot be used.
    let other = env.create_mint(MintKind::CngnLike, 6);
    env.create_market_with_promo(&other);
    let prices = env.price_accounts(&owner);
    let mut instruction = liquidate_ix(
        &liquidator.pubkey(), &owner, &setup.cngn, &liquidator.cngn, &setup.usdc, &SPL_TOKEN,
        &seized_to, 0, 100_000 * ONE_CNGN, prices,
    );
    let slot = instruction.accounts.iter().position(|a| a.pubkey == promo_vault_pda(&setup.cngn)).unwrap();
    instruction.accounts[slot] = AccountMeta::new(promo_vault_pda(&other), false);
    // The seeds are derived from this market, so another market's vault cannot be substituted.
    let result = send(&mut env.svm, &[instruction], &[&liquidator.key]);
    assert_anchor_error(result, AnchorError::ConstraintSeeds);

    // Named correctly, the same liquidation goes through.
    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 100_000 * ONE_CNGN).unwrap();
    assert_eq!(env.position(&owner).promo_balance, 0);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `./scripts/test.sh --test promo_forfeit`
Expected: `error[E0560]: struct hodl_loans::accounts::Liquidate has no field named promo_vault`, the same for `promo_vault_token`, and five more for `WriteOffLoan`'s `mint`, `vault`, `promo_vault`, `promo_vault_token` and `token_program` — seven in all.

- [ ] **Step 3: Implement**

Append to `programs/hodl_loans/src/instructions/promos/lifecycle.rs`, and widen its imports to `use anchor_spl::token_interface::{Mint, TokenAccount};`, `PromoForfeited`, `math::checked::sub` and `token::transfer::transfer_from_vault`:

```rust
/// The accounts forfeiture moves cNGN between. Grouped and passed by reference because the
/// callers are already close to the SBF 4 KB stack frame: nine positional arguments, three of
/// them `AccountInfo`, was enough to overflow `liquidate`.
pub struct ForfeitAccounts<'a, 'info> {
    pub promo_vault: &'a mut Account<'info, PromoVault>,
    pub promo_token: &'a InterfaceAccount<'info, TokenAccount>,
    pub market_vault: &'a InterfaceAccount<'info, TokenAccount>,
    pub mint: &'a InterfaceAccount<'info, Mint>,
    pub token_program: Pubkey,
}

/// Spec §11 step 3. Unlike expiry, forfeiture *moves* the cNGN: the promo backing a defaulting
/// position leaves the promo vault for the market vault, where it becomes lender cash. It does
/// not reduce the borrower's debt — it is the protocol taking back what it lent them for free.
///
/// Returns the amount forfeited, which the caller adds to `market.cash`; the market account is
/// already borrowed mutably at every call site.
pub fn forfeit_promo(
    position: &mut Position,
    position_key: Pubkey,
    market_key: Pubkey,
    accounts: &mut ForfeitAccounts,
) -> Result<u64> {
    if position.promo_balance == 0 {
        return Ok(0);
    }
    let amount = release_promo(position, accounts.promo_vault)?;

    // cNGN carries a PermanentDelegate, so its issuer can move tokens out of the promo vault's
    // token account without the program's involvement. After such a clawback,
    // `promo_vault.cash` (this program's ledger) can overstate the vault's real token balance.
    // Transferring the full nominal `amount` unconditionally would then revert — bricking every
    // liquidation and write-off of a promo-holding position on this market, a liveness failure
    // on the protocol's solvency backstop. Clamp the transfer, and the cash debit, to what the
    // vault actually holds: lenders receive whatever remains instead of the call reverting
    // outright.
    let available = accounts.promo_token.amount;
    let moved = amount.min(available);

    if moved > 0 {
        let seeds: &[&[u8]] = &[PROMO_VAULT_SEED, market_key.as_ref(), &[accounts.promo_vault.bump]];
        transfer_from_vault(
            accounts.token_program,
            accounts.mint.to_account_info(),
            accounts.mint.decimals,
            accounts.promo_token.to_account_info(),
            accounts.market_vault.to_account_info(),
            accounts.promo_vault.to_account_info(),
            moved,
            &[seeds],
        )?;
    }

    accounts.promo_vault.cash = to_u64(sub(accounts.promo_vault.cash as u128, moved as u128)?)?;
    accounts.promo_vault.require_invariant()?;

    emit!(PromoForfeited {
        market: market_key,
        position: position_key,
        owner: position.owner,
        amount,
        moved,
    });
    Ok(moved)
}
```

Add the two accounts to `Liquidate`, after `market`:

```rust
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
    /// The promo vault's cNGN, which forfeiture moves into the market vault (spec §11 step 3).
    #[account(mut, address = promo_vault.vault @ HodlError::PromoVaultInsufficient)]
    pub promo_vault_token: Box<InterfaceAccount<'info, TokenAccount>>,
```

and call it immediately after the liquidatable check:

```rust
        // Spec §11 step 3: the promo behind a defaulting position goes to lenders, before any
        // of the repayment is priced. It does not reduce what the borrower owes.
        let forfeited = forfeit_promo(
            &mut position,
            position_key,
            market_key,
            &mut ForfeitAccounts {
                promo_vault: &mut ctx.accounts.promo_vault,
                promo_token: &ctx.accounts.promo_vault_token,
                market_vault: &ctx.accounts.vault,
                mint: &ctx.accounts.mint,
                token_program: ctx.accounts.token_program.key(),
            },
        )?;
        market.cash = to_u64(add(market.cash as u128, forfeited as u128)?)?;
```

`position_key` already exists in `handle_liquidate`, declared just before the events at the end — hoist that declaration to the top of the handler rather than adding a second one.

`write_off_loan` needs the same, and five accounts it never had, since it moved no tokens before this task:

```rust
    #[account(address = market.mint, mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, address = market.vault @ HodlError::MarketMismatch)]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    /// Forfeiture moves the position's promo backing into the market vault (spec §11 step 3),
    /// so a written-off loan still returns what the protocol lent the borrower for free.
    #[account(
        mut,
        seeds = [PROMO_VAULT_SEED, market.key().as_ref()],
        bump = promo_vault.bump,
        has_one = market
    )]
    pub promo_vault: Box<Account<'info, PromoVault>>,
    #[account(mut, address = promo_vault.vault @ HodlError::PromoVaultInsufficient)]
    pub promo_vault_token: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
```

with the same call placed after the dust check and before the loss is booked, and netting the forfeited amount out of the loss before it is booked:

```rust
    let loss = add(loan.principal as u128, released)?;
    // `forfeited` already landed in `market.cash` above, so it already repaid part of this
    // shortfall — booking the gross `loss` as bad debt on top would double-count it. Net it out
    // before it reaches `covered` or `total_bad_debt`; it can exceed `loss` (forfeiture releases
    // the position's whole promo balance, not just enough to cover this one loan), so floor at
    // zero rather than using checked subtraction.
    let net_loss = loss.saturating_sub(forfeited as u128);
```

Without that netting, `covered = min(loss, reserve)` draws against `protocol_reserve` for cNGN
the forfeit already supplied — real state, not just a reporting figure.

Add the event:

```rust
#[event]
pub struct PromoForfeited {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    /// Promo removed from the position. Always the position's whole balance.
    pub amount: u64,
    /// cNGN that actually reached the market vault. Equal to `amount` except after an issuer
    /// clawback from the promo vault, where the transfer is clamped to the balance on hand —
    /// see `forfeit_promo`. An indexer summing what lenders received must use this, not
    /// `amount`.
    pub moved: u64,
}
```

Finally, liquidation now moves a third token transfer's worth of work. In `programs/hodl_loans/tests/budget.rs`, record 93,917 CU and raise its ceiling to `120_000` — the old `100_000` would leave 6% headroom.

- [ ] **Step 4: Run the tests, then the full suite and lints**

Run: `./scripts/test.sh --test promo_forfeit`
Expected: 5 tests, all `ok`.

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, 225 tests in all (`promo_forfeit` is new with 9).

Run: `cargo clippy -p hodl_loans --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: forfeit a defaulting position's promo to lenders"
```

---

### Task 8: The promo cap

**Files:**
- Create: `programs/hodl_loans/src/instructions/promos/cap.rs`, `tests/promo_cap.rs`
- Modify: `src/instructions/promos/mod.rs`, `src/events.rs`, `src/lib.rs`, `tests/common/mod.rs`

**Interfaces:**
- Consumes: `Config::{promo_cap_bps, collateral_count}`, `CollateralAsset::{ltv_bps, liquidation_threshold_bps, mint, bump}`, `MAX_BPS`.
- Produces:
  - `set_promo_cap(promo_cap_bps)` — admin, with every listed `CollateralAsset` in `remaining_accounts`
  - Harness: `set_promo_cap_ix(admin, promo_cap_bps, assets)`, which derives and sorts the asset accounts for the caller

- [ ] **Step 1: Write the harness and the failing tests**

Append to `programs/hodl_loans/tests/common/mod.rs`:

```rust
// ---- The promo cap (Task 8) ----

/// `set_promo_cap` re-checks every listed asset, so the caller passes them all, in ascending
/// key order.
pub fn set_promo_cap_ix(admin: &Pubkey, promo_cap_bps: u16, assets: &[Pubkey]) -> Instruction {
    let mut instruction = ix(
        hodl_loans::instruction::SetPromoCap { promo_cap_bps },
        hodl_loans::accounts::SetPromoCap { admin: *admin, config: config_pda() },
    );
    let mut sorted: Vec<Pubkey> = assets.iter().map(collateral_pda).collect();
    sorted.sort();
    instruction.accounts.extend(sorted.into_iter().map(|k| AccountMeta::new_readonly(k, false)));
    instruction
}
```

Create `programs/hodl_loans/tests/promo_cap.rs`:

```rust
mod common;

use common::*;
use hodl_loans::HodlError;
use solana_signer::Signer;

#[test]
fn the_cap_is_re_checked_against_every_listed_asset() {
    let (mut env, _cngn) = Env::with_cngn_market();
    let admin = env.admin.pubkey();
    // Default collateral is 70% LTV against a 90% threshold, so 20 points of room.
    let usdc = env.list_spl_collateral(6);
    let sol = env.list_spl_collateral(9);
    assert_eq!(env.config().collateral_count, 2);
    assert_eq!(env.config().promo_cap_bps, 2_000);

    // Exactly the room every asset has is allowed; one point more is not.
    let raise = set_promo_cap_ix(&admin, 2_001, &[usdc, sol]);
    assert_hodl_error(send(&mut env.svm, &[raise], &[&env.admin]), HodlError::InvalidParameters);
    let exact = set_promo_cap_ix(&admin, 2_000, &[usdc, sol]);
    send(&mut env.svm, &[exact], &[&env.admin]).unwrap();

    // Lowering it is always safe.
    let lower = set_promo_cap_ix(&admin, 500, &[usdc, sol]);
    send(&mut env.svm, &[lower], &[&env.admin]).unwrap();
    assert_eq!(env.config().promo_cap_bps, 500);

    // The tightest asset is the one that binds: 5% of room leaves room for a 5% cap.
    let tight = hodl_loans::CollateralParams {
        ltv_bps: 7_000,
        liquidation_threshold_bps: 7_500,
        ..default_collateral_params(&sol)
    };
    let update = update_collateral_params_ix(&admin, &sol, tight);
    send(&mut env.svm, &[update], &[&env.admin]).unwrap();
    let over = set_promo_cap_ix(&admin, 501, &[usdc, sol]);
    assert_hodl_error(send(&mut env.svm, &[over], &[&env.admin]), HodlError::InvalidParameters);
    let ok = set_promo_cap_ix(&admin, 500, &[usdc, sol]);
    send(&mut env.svm, &[ok], &[&env.admin]).unwrap();
}

#[test]
fn no_listed_asset_can_be_skipped_or_stood_in_for() {
    let (mut env, _cngn) = Env::with_cngn_market();
    let admin = env.admin.pubkey();
    let permissive = env.list_spl_collateral(6);
    let strict_params_mint = env.list_spl_collateral(9);
    // The cap has to come down before an asset can be tightened past it: the same rule binds
    // both directions, and it is currently 20%.
    send(&mut env.svm, &[set_promo_cap_ix(&admin, 100, &[permissive, strict_params_mint])], &[&env.admin]).unwrap();
    let strict = hodl_loans::CollateralParams {
        ltv_bps: 7_000,
        liquidation_threshold_bps: 7_100,
        ..default_collateral_params(&strict_params_mint)
    };
    send(&mut env.svm, &[update_collateral_params_ix(&admin, &strict_params_mint, strict)], &[&env.admin]).unwrap();

    // Leaving the strict asset out fails on the count.
    let short = set_promo_cap_ix(&admin, 1_000, &[permissive]);
    assert_hodl_error(send(&mut env.svm, &[short], &[&env.admin]), HodlError::InvalidParameters);

    // Passing the permissive one twice satisfies a bare count check, which is why the keys must
    // strictly increase: a duplicate is not a second asset.
    let duplicated = set_promo_cap_ix(&admin, 1_000, &[permissive, permissive]);
    assert_hodl_error(send(&mut env.svm, &[duplicated], &[&env.admin]), HodlError::InvalidParameters);

    // With both passed honestly, the strict asset's 1% of room is what binds.
    let over = set_promo_cap_ix(&admin, 101, &[permissive, strict_params_mint]);
    assert_hodl_error(send(&mut env.svm, &[over], &[&env.admin]), HodlError::InvalidParameters);
    let ok = set_promo_cap_ix(&admin, 100, &[permissive, strict_params_mint]);
    send(&mut env.svm, &[ok], &[&env.admin]).unwrap();
}

#[test]
fn setting_the_cap_is_admin_only_and_bounded() {
    let (mut env, _cngn) = Env::with_cngn_market();
    let admin = env.admin.pubkey();
    let stranger = env.funded_keypair();

    let by_stranger = set_promo_cap_ix(&stranger.pubkey(), 100, &[]);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);

    // Above 100% is meaningless, and with no assets listed there is nothing to contradict it.
    let absurd = set_promo_cap_ix(&admin, 10_001, &[]);
    assert_hodl_error(send(&mut env.svm, &[absurd], &[&env.admin]), HodlError::InvalidParameters);
    send(&mut env.svm, &[set_promo_cap_ix(&admin, 10_000, &[])], &[&env.admin]).unwrap();
    assert_eq!(env.config().promo_cap_bps, 10_000);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `./scripts/test.sh --test promo_cap`
Expected: `error[E0422]: cannot find struct, variant or union type SetPromoCap in module hodl_loans::instruction`, and the same for `hodl_loans::accounts`.

- [ ] **Step 3: Implement**

Create `programs/hodl_loans/src/instructions/promos/cap.rs`. Spec §8 says the account count must equal `collateral_count` "so none can be skipped" — which a bare count does not achieve, since the same permissive asset can be passed several times. Requiring the keys to strictly increase closes that, and re-deriving each PDA stops a look-alike account standing in for a stricter asset:

```rust
use anchor_lang::prelude::*;

use crate::constants::{COLLATERAL_SEED, CONFIG_SEED, MAX_BPS};
use crate::errors::HodlError;
use crate::events::PromoCapSet;
use crate::state::{CollateralAsset, Config};

#[derive(Accounts)]
pub struct SetPromoCap<'info> {
    pub admin: Signer<'info>,
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ HodlError::Unauthorized)]
    pub config: Account<'info, Config>,
}

/// Spec §8 and §12. The cap bounds how much promo counts against a borrower's own collateral,
/// so raising it eats into the gap between every asset's LTV and its liquidation threshold —
/// the gap that keeps a fully drawn position solvent. Every listed asset is re-checked against
/// the new value before it takes effect.
///
/// `remaining_accounts` carries every `CollateralAsset`, **in ascending key order**. The count
/// must equal `config.collateral_count` and the keys must strictly increase, which together
/// rule out the mistake a bare count check allows: passing one permissive asset several times
/// and leaving the rest unexamined.
pub fn handle_set_promo_cap(ctx: Context<SetPromoCap>, promo_cap_bps: u16) -> Result<()> {
    require!(promo_cap_bps <= MAX_BPS, HodlError::InvalidParameters);
    require!(
        ctx.remaining_accounts.len() == ctx.accounts.config.collateral_count as usize,
        HodlError::InvalidParameters
    );

    let mut previous = Pubkey::default();
    for info in ctx.remaining_accounts {
        require_keys_eq!(*info.owner, *ctx.program_id, HodlError::InvalidParameters);
        require!(info.key() > previous, HodlError::InvalidParameters);
        previous = info.key();

        let data = info.try_borrow_data()?;
        let asset = CollateralAsset::try_deserialize(&mut &data[..])
            .map_err(|_| HodlError::InvalidParameters)?;
        // The asset account must be the one the program derives for its own mint, so a
        // look-alike cannot stand in for a stricter asset.
        let expected = Pubkey::create_program_address(
            &[COLLATERAL_SEED, asset.mint.as_ref(), &[asset.bump]],
            ctx.program_id,
        )
        .map_err(|_| HodlError::InvalidParameters)?;
        require_keys_eq!(info.key(), expected, HodlError::InvalidParameters);

        require!(
            asset.ltv_bps as u32 + promo_cap_bps as u32 <= asset.liquidation_threshold_bps as u32,
            HodlError::InvalidParameters
        );
    }

    let config = &mut ctx.accounts.config;
    let old = config.promo_cap_bps;
    config.promo_cap_bps = promo_cap_bps;
    emit!(PromoCapSet { old, new: promo_cap_bps });
    Ok(())
}
```

Add `pub mod cap;` and `pub use cap::*;` to the promos module, the event:

```rust
#[event]
pub struct PromoCapSet {
    pub old: u16,
    pub new: u16,
}
```

and the entry point:

```rust
    pub fn set_promo_cap(ctx: Context<SetPromoCap>, promo_cap_bps: u16) -> Result<()> {
        instructions::handle_set_promo_cap(ctx, promo_cap_bps)
    }
```

- [ ] **Step 4: Run the tests, then the full suite and lints**

Run: `./scripts/test.sh --test promo_cap`
Expected: 3 tests, all `ok`.

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, 234 tests in all — 52 unit and 182 LiteSVM.

Run: `cargo clippy -p hodl_loans --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: set_promo_cap, re-checked against every listed asset"
```

---

## Done when

- `./scripts/test.sh` reports 234 passing tests and `cargo clippy -p hodl_loans --all-targets -- -D warnings` is clean.
- `outstanding + unissued ≤ cash` holds after every instruction that touches the promo vault, and the free balance is what bounds both campaign creation and withdrawal.
- A voucher only works for the wallet, amount, nonce and expiry the promo signer actually signed, only once, and only against an open campaign within its window.
- Promo lifts both the borrow limit and the liquidation line by the same capped amount, and is worth nothing to a position holding no collateral of its own.
- Idle promo can be reclaimed by anyone, revoked by the admin, and neither can touch a position with a live loan.
- A defaulting position's promo reaches lenders as cNGN in the market vault, without reducing what the borrower owes.
- The cap cannot be raised past the room any listed asset has between its LTV and its liquidation threshold, and no asset can be skipped or stood in for while it is checked.

## Deliberately not in this plan

- **Per-referral rules** (one voucher per invited friend, and so on). Spec §12 puts these in the backend, before it signs.
- **A per-asset multiplier ceiling** (the Plan 4 follow-up). It belongs with promo's own caps but is a `CollateralAsset` change, not a promo one.
- **Trident invariants over the promo vault.** The `outstanding + unissued ≤ cash` invariant is exactly the shape a fuzzer should attack; that arrives with the rest of the fuzzing in Plan 6.
- **Migrating positions when the cap changes.** `set_promo_cap` (Task 8) re-checks every listed asset, but it does nothing to positions already holding promo: lowering the cap simply counts less of their promo from that block on. That is the intended behaviour — promo was never theirs to spend — and no migration is needed.
