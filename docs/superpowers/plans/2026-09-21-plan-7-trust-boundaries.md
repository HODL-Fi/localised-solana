# Plan 7: Trust Boundaries and Admin Authority Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the trust-boundary and admin-authority items the Plan 2–5 reviews deferred — local account admission in the health walk, an owner check on the NGN feed, two per-asset brakes on borrowing, bounds on the three values that could lock an instruction out, and the promo lifecycle gaps around pauses, revocation and issuer clawbacks.

**Architecture:** Most of this adds a check where the program previously relied on a whole-program argument, or bounds a value that had no ceiling. Three tasks change behaviour: an asset can be barred from lending borrowing power without being sealed against liquidation; a market pause now stops the promo inactivity clock; and promo revocation under a live loan is allowed when the position survives without it. One task rejects vouchers that were previously valid and carries a migration note.

**Tech Stack:** Anchor 1.2.0, anchor-spl `token_interface`, LiteSVM 0.10.0 with the `precompiles` feature, `cargo build-sbf --tools-version v1.52`.

**Spec:** `docs/superpowers/specs/2026-09-17-solana-fixed-loans-design.md`

**Sources:** `docs/superpowers/plans/2026-09-17-plan-{2,3,4,5}-followups.md` and `2026-09-20-plan-6-followups.md`. This plan takes the "trust boundaries and admin authority" group; coverage, fuzzing and devnet are Plan 8. The overdue-penalty continuous accrual is explicitly **not** here — it changes how interest reaches lenders and needs its own plan.

## Global Constraints

- Anchor 1.2.0. Build and test with `./scripts/test.sh`, which rebuilds the SBF program first. **Plain `cargo test` reuses a stale `.so`** and will pass against code you have just changed. This bit the plan's own author twice: a test appeared to fail against the new code when it was really running the old binary. Any load-bearing check must rebuild first.
- A green run means **no compile errors and no failures**. `grep FAILED` alone does not catch a test binary that failed to compile — check for `error[` too.
- All arithmetic is checked: no raw `+ - *` on values that could overflow, no `unwrap()` on arithmetic, no bare `as` narrowing casts. `math::checked` has `add`, `sub`, `mul_div_floor`, `mul_div_ceil`, `to_u64` and `pow10`.
- SBF stack frames are 4 KB. Box accounts in large instruction contexts, `Config` included.
- `cargo clippy -p hodl_loans --all-targets -- -D warnings` clean. Do not silence a lint with `#[allow]`; restructure instead.
- New `HodlError` variants are **appended**, never inserted — codes are `6000 + position`. Same for `CollateralParams` fields: Borsh order is append-only.
- A new account field comes out of the account's `reserved` padding so the account size does not change. The `INIT_SPACE` guards in `state/collateral.rs` (294) and `state/market.rs` (555) will fail if you get this wrong — that is what they are for.
- Compute figures in `tests/budget.rs` are **not deterministic**: the harness keys mints randomly, so slot sort order shifts the cost in ~1,500 CU steps. Record ranges over many runs, never a single sample. Adding code to the crate also shifts inlining and moves figures by tens of CU on paths you did not touch.
- Every commit message ends with:
  `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`
- The suite is **241 tests** (57 unit, 184 LiteSVM) at the start of this plan and **264** (59 unit, 205 LiteSVM) at the end.

## File Structure

New code lands in files that already exist; nothing here creates a module.

| File | What this plan changes |
|---|---|
| `src/valuation.rs` | The collateral walk: PDA re-derivation, and the multiplier-ceiling helper that feeds `lends_borrowing_power` |
| `src/math/health.rs` | `CollateralValue::lends_borrowing_power` and the two places in `compute_health` it gates |
| `src/oracle/switchboard.rs` | Owner check on the NGN feed |
| `src/state/collateral.rs` | `borrow_paused`, `max_multiplier`, their `CollateralParams` entry and validation |
| `src/state/market.rs` | `promo_clock_resumed_at` |
| `src/constants.rs` | `MAX_LISTED_COLLATERAL`, `MAX_CAMPAIGN_LIFETIME`, and the `MAX_MULTIPLIER` note |
| `src/errors.rs` | Two appended variants |
| `src/events.rs` | Two appended events |
| `src/instructions/admin/collateral_admin.rs` | `set_collateral_borrow_paused` |
| `src/instructions/admin/market_admin.rs` | The clock restart on unpause |
| `src/instructions/admin/{reserve,sweep}.rs` | Why they do not accrue |
| `src/instructions/loans/take_loan.rs` | Check order |
| `src/instructions/promos/{campaign,redeem,lifecycle,vault}.rs` | Bounds, the pause gate, revoke-under-loan, `reconcile_promo_vault` |
| `src/lib.rs` | Three new instruction entry points |

---


### Task 1: Local admission in the health walk

The walk admits a `CollateralAsset` from `remaining_accounts` by program owner, Anchor discriminator and stored `mint`. Those narrow it to "a `CollateralAsset` this program created for this mint" — but only because `list_collateral` is the sole creation path, which is an argument about the whole program rather than about this account. Since Plan 4 the same account's `kind` also decides how many accounts the walk consumes, so more rests on it than when Plan 2 first wrote it down. One hash makes the argument local.

It is not free: this is the single most expensive change in the plan, and the compute figures move with it. Re-recording them is part of the task, not a follow-up.

**Files:**
- Modify: `programs/hodl_loans/src/valuation.rs`
- Test: `programs/hodl_loans/tests/loans.rs`, `programs/hodl_loans/tests/budget.rs`

**Interfaces:**
- Consumes: `constants::COLLATERAL_SEED`, already imported elsewhere in `valuation.rs`.
- Produces: nothing new. Later tasks add to the same walk but do not depend on this.

- [ ] **Step 1: Write the failing test**

Append to `programs/hodl_loans/tests/loans.rs`. `collateral_asset_bytes` and `set_account_data` already exist in the harness.

```rust
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
```

- [ ] **Step 2: Run it and watch it fail**

Run: `./scripts/test.sh 2>&1 | grep -E "forged_address|error\["`

Expected: **FAIL**. Without the re-derivation the forged account is admitted — owner, discriminator and `mint` all check out — so the borrow succeeds at the forged 90% LTV and `assert_hodl_error` is what fails.

If it *passes* here, you are testing a stale `.so`. Run `cargo build-sbf --tools-version v1.52` and try again. A test that passes before its fix exists is the single most expensive mistake available in this repo, because everything downstream then looks green.

- [ ] **Step 3: Re-derive the PDA in the walk**

In `programs/hodl_loans/src/valuation.rs`, immediately after the `require_keys_eq!(asset.mint, slot.mint, ...)` line:

```rust
        // Re-derive the PDA the account claims to be. Owner + discriminator + `mint` already
        // narrow it to "a CollateralAsset this program created for this mint", and listing is
        // the only path that creates one — but that is a whole-program argument, and since
        // Plan 4 this same account's `kind` also decides how many accounts the walk consumes,
        // so more rests on it than when Plan 2 first wrote it down. One hash makes the argument
        // local: this is the canonical `["collateral", mint]` address or the walk stops.
        let expected = Pubkey::create_program_address(
            &[COLLATERAL_SEED, asset.mint.as_ref(), &[asset.bump]],
            program_id,
        )
        .map_err(|_| HodlError::PriceAccountMismatch)?;
        require_keys_eq!(asset_info.key(), expected, HodlError::PriceAccountMismatch);
```

Add `COLLATERAL_SEED` to the `crate::constants` import if it is not already there.

- [ ] **Step 4: Run the test**

Run: `./scripts/test.sh 2>&1 | grep -E "forged_address|error\["`
Expected: PASS, and now for the stated reason. Confirm by reverting the block, rebuilding with `cargo build-sbf --tools-version v1.52`, and watching it fail — then restore.

- [ ] **Step 5: Re-measure every compute figure**

The walk now costs one `create_program_address` syscall per collateral slot. Do not estimate this; measure it. Temporarily turn each `assert!(cu < N, ...)` in `tests/budget.rs` into a `println!`, run the budget test 20+ times, and take min–max per figure. Restore the assertions afterwards.

Three thresholds no longer hold and move to `115_000`:

```rust
    assert!(cu < 115_000, "take_loan at 8 collateral slots / 9 existing overdue loans used {cu} CU");
    assert!(cu < 115_000, "withdraw_collateral at 8 collateral slots / 10 loans used {cu} CU");
    assert!(cu < 115_000, "take_loan at 8 xStock slots / 9 existing loans used {cu} CU");
```

- [ ] **Step 6: Record what you measured and why it moved**

Replace the note above the first figure in `full_position_stays_under_the_default_compute_budget`:

```rust
    // These figures are NOT deterministic: the harness keys its mints randomly, so where a
    // target sorts into the collateral slot array shifts the scan, moving the cost in steps of
    // ~1,500 CU. Each range below is min-max observed over 34 runs, and the tail is not fully
    // characterised — separate batches produce different maxima. Compare against the max,
    // never a single sample.
    //
    // **Every walking figure below rose ~12,500-13,200 CU in Plan 7**, when the walk began
    // re-deriving each `CollateralAsset`'s PDA. That is one `create_program_address` syscall
    // per collateral slot — ~1,600 CU each, ~12,700 at eight slots — and it accounts for
    // nearly all of the increase. It is not quite all of it: `repay_loan`, which walks no
    // collateral, still moved ~55 CU, and the liquidate paths ~364 beyond the walk's share.
    // Adding code to the crate shifts inlining decisions across it, so figures drift by tens
    // of CU on changes that do not touch the measured path at all. Treat a drift of that
    // order as noise and anything near 1,500 as the slot-order variance above.
    //
    // The cost buys a local admission argument in place of a whole-program one, and these
    // markers exist to make a change of this size visible rather than to veto it; nothing
    // here is near the 200,000 default budget. Spec §15 and
    // `docs/superpowers/plans/2026-09-21-plan-7-followups.md` carry the reasoning.
```

The measured ranges in the final tree are below. **Yours will differ** — record what you measured, not these. They are here so you can tell a plausible result from an implausible one.

```rust
    // the 9 existing overdue loans. Measured 88,192-97,192 CU over 34 runs.
    // and now 10 loans. Measured 87,656-96,656 CU over 34 runs; see the note above.
    // token CPI on top of the no-promo case's two. Measured 113,756-113,787 CU over 34 runs.
```

- [ ] **Step 7: Commit**

```bash
git add programs/hodl_loans/src/valuation.rs programs/hodl_loans/tests/loans.rs programs/hodl_loans/tests/budget.rs
git commit -m "feat(plan7): re-derive the collateral PDA in the health walk

Owner, discriminator and stored mint narrowed an account to one this program
created for that mint, but only via a whole-program argument. Since Plan 4 the
same account's kind also decides how many accounts the walk consumes. One hash
makes the argument local.

Costs one create_program_address per collateral slot on every health-checking
path; every walking figure in tests/budget.rs re-measured and three thresholds
raised to 115,000. Nothing approaches the 200,000 default budget.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: The NGN feed must be owned by Switchboard

`Market::ngn_feed` is a bare `Pubkey` the admin sets, with no account constraint behind it. The address check proves only that the caller passed the account the admin named — not that the account is a Switchboard feed. A discriminator is eight bytes anyone can write.

This does **not** remove the residual trust in whichever authority controls the real feed's writes. That is a genuine assumption, recorded in spec §20 item 4, and an owner check cannot address it. Say so rather than implying the check is worth more than it is.

**Files:**
- Modify: `programs/hodl_loans/src/oracle/switchboard.rs`
- Test: `programs/hodl_loans/tests/common/mod.rs`, `programs/hodl_loans/tests/loans.rs`

**Interfaces:**
- Produces: `Env::set_ngn_price_owned_by(&mut self, owner: &Pubkey, value: i128, std_dev: i128)` — the harness helper. No later task uses it.

- [ ] **Step 1: Write the failing tests**

The unit test first. In `switchboard.rs`'s test module, split the existing `read` helper so a test can choose the owner:

```rust
    fn read(key: Pubkey, data: &mut [u8], m: &Market, slot: u64) -> Result<UsdPrice> {
        read_owned_by(key, data, m, slot, switchboard_on_demand::ON_DEMAND_MAINNET_PID)
    }

    fn read_owned_by(
        key: Pubkey,
        data: &mut [u8],
        m: &Market,
        slot: u64,
        owner: Pubkey,
    ) -> Result<UsdPrice> {
        let mut lamports = 1_000_000u64;
        let info = AccountInfo::new(&key, false, false, &mut lamports, data, &owner, false);
        let clock = Clock { slot, ..Clock::default() };
        read_ngn_price(&info, m, &clock)
    }
```

Then add this case to `rejections_map_to_program_errors`, after the wrong-address assertion:

```rust
        // Right address, right discriminator, wrong owner. `ngn_feed` is a bare `Pubkey` the
        // admin sets, so without this the program would read any account it named as a price.
        assert_eq!(
            err(read_owned_by(key, &mut pull_feed_data(VALUE, 0, 1_000, 5), &m, 1_000, Pubkey::new_unique())),
            HodlError::PriceAccountMismatch.into()
        );
        let mut garbage = vec![0u8; 3_208];
```

(The last line above is the existing next statement — it marks where the new block ends, do not duplicate it.)

- [ ] **Step 2: Add the harness helper and the integration test**

In `programs/hodl_loans/tests/common/mod.rs`, inside the `impl Env` block that holds `set_ngn_price`:

```rust
    /// The same NGN feed bytes `set_ngn_price` writes, but owned by an account of the
    /// caller's choosing. Only a test that wants the owner check to fire has any use for this.
    pub fn set_ngn_price_owned_by(&mut self, owner: &Pubkey, value: i128, std_dev: i128) {
        let slot = self.svm.get_sysvar::<Clock>().slot;
        self.set_account_data(&ngn_feed(), owner, pull_feed_data(value, std_dev, slot, 5));
    }
```

Append to `programs/hodl_loans/tests/loans.rs`:

```rust
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
```

- [ ] **Step 3: Run both and watch them fail**

Run: `./scripts/test.sh 2>&1 | grep -E "owned_by|rejections_map|error\["`
Expected: both FAIL. The unit test's new assertion gets `Ok(..)` where it wants an error; the integration test's borrow succeeds.

- [ ] **Step 4: Add the owner check**

Replace `read_ngn_price` in `programs/hodl_loans/src/oracle/switchboard.rs`:

```rust
/// Read the market's Switchboard On-Demand NGN/USD pull feed.
///
/// The account address is pinned to `market.ngn_feed`, which the admin sets, the account must
/// be owned by the Switchboard On-Demand program, and the data must carry the
/// `PullFeedAccountData` discriminator and full length.
///
/// The owner check matters because `ngn_feed` is a bare `Pubkey` on `Market` with no
/// constraint behind it: without it, an admin who set the field to any account at all would
/// have the program read 3.2 KB of arbitrary bytes as a price feed, and a discriminator is
/// eight bytes an attacker can simply write. It does **not** remove the residual trust in
/// whichever authority controls the real feed's writes — that is a genuine assumption, recorded
/// in spec §20 item 4, and an owner check cannot address it.
///
/// Pinned to the MAINNET program id. A devnet deployment reads a feed owned by
/// `ON_DEMAND_DEVNET_PID` and would be refused here; that is a deliberate trade, since this
/// program's own id is mainnet too, and it is recorded for the devnet plan.
///
/// Only the 128-byte aggregated `result` is copied out (by offset, unaligned), keeping the
/// 3.2 KB feed off the stack. `value` is the price and `std_dev` the spread.
pub fn read_ngn_price(account: &AccountInfo, market: &Market, clock: &Clock) -> Result<UsdPrice> {
    require_keys_eq!(account.key(), market.ngn_feed, HodlError::PriceAccountMismatch);
    require_keys_eq!(
        *account.owner,
        switchboard_on_demand::ON_DEMAND_MAINNET_PID,
        HodlError::PriceAccountMismatch
    );
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
```

- [ ] **Step 5: Run the tests**

Run: `./scripts/test.sh`
Expected: all pass. The unit suite is 57 at this point in the plan; the new assertion lives inside an existing test, so the count does not move.

- [ ] **Step 6: Commit**

```bash
git add programs/hodl_loans/src/oracle/switchboard.rs programs/hodl_loans/tests/
git commit -m "feat(plan7): require the NGN feed to be owned by Switchboard On-Demand

market.ngn_feed is a bare Pubkey with no constraint behind it, so the address
check proved only that the caller passed the account the admin named. A
discriminator is eight forgeable bytes.

Pinned to the MAINNET program id, so a devnet deployment would be refused —
deliberate, since this program's own id is mainnet too, and recorded for the
devnet plan. The residual trust in the feed authority is unaffected and stays
recorded as spec §20 item 4.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Withholding borrowing power, for two reasons

Two gaps the Plan 4 and 5 reviews left, which turn out to be one mechanism.

`collateral.paused` gates deposits only; nothing stops **new borrowing** against an asset whose price feed has gone unreliable or whose issuer has switched a transfer hook on. And spec §14 names the scaled-UI authority as the most powerful of cNGN's four keys — the permanent delegate can only take back its own token, while the multiplier mints borrowing power bounded only by a `deposit_cap` denominated in raw units, which stops capping USD exposure the moment the multiplier moves. §14 defers a per-asset ceiling as the answer.

Both want the same thing: an asset that stops supporting new exposure without being sealed against liquidation. Tightening the global `MAX_MULTIPLIER` was rejected for exactly that reason — an over-cap read returns `InvalidPrice`, which fails *every* health check touching the asset, including the exit paths.

**The subtlety that makes this one task rather than two, and the one that a first draft got wrong:** it is not enough to zero the holding's `ltv_bps`. `compute_health` accumulates two independent things from each holding's value — the LTV term, and `promo_cap_total`, which is a fraction of the holding's **value** and never consulted `ltv_bps`. `promo_counted` then goes straight back into `borrow_limit`. Gate only the first and a fully borrow-paused asset still unlocks up to `min(promo, promo_cap_bps × value)` of borrowing power. So the flag lives on `CollateralValue` and gates both.

**Files:**
- Modify: `programs/hodl_loans/src/math/health.rs`, `src/valuation.rs`, `src/state/collateral.rs`, `src/constants.rs`, `src/events.rs`, `src/lib.rs`, `src/instructions/admin/collateral_admin.rs`
- Modify: `docs/superpowers/specs/2026-09-17-solana-fixed-loans-design.md` (§14)
- Test: `tests/collateral.rs`, `tests/loans.rs`, `tests/liquidation.rs`, `tests/withdraw.rs`, `tests/xstocks.rs`, `tests/promo_health.rs`, `tests/common/mod.rs`

**Interfaces:**
- Produces: `CollateralValue::lends_borrowing_power: bool`; `CollateralAsset::{borrow_paused, max_multiplier}`; `CollateralParams::max_multiplier` (appended); `set_collateral_borrow_paused(paused: bool)`; harness `set_collateral_borrow_paused_ix(signer, mint, paused)`.
- No later task depends on these.

- [ ] **Step 1: Write the unit test for the mechanism**

This is the one that catches the promo-cap leak, and it is cheaper than any SVM test. Add to the `promo_counted` section of `programs/hodl_loans/src/math/health.rs`'s test module:

```rust
    #[test]
    fn a_holding_that_lends_no_borrowing_power_unlocks_no_promo_either() {
        // The whole point of the flag. An asset the admin has borrow-paused, or whose mint has
        // scaled past its ceiling, must not raise `borrow_limit` by *either* route. Gating
        // only the LTV term leaves the promo cap — a fraction of the holding's value, which
        // never looked at `ltv_bps` — still unlocking borrowing power against an asset the
        // brake is supposedly on.
        let holding = |lends| {
            [CollateralValue {
                amount: 1_000_000_000,
                decimals: 6,
                multiplier: MULTIPLIER_SCALE,
                price: UsdPrice { price: USD, conf: 0 },
                ltv_bps: 7_000,
                liquidation_threshold_bps: 9_000,
                lends_borrowing_power: lends,
            }]
        };
        // 1,000 USDC at $1 with a 20% promo cap and promo worth more than the cap allows.
        let promo = 400_000_000_000u64;
        let lending = compute_health(&holding(true), 0, 6, ngn(), promo, 2_000).unwrap();
        assert_eq!(lending.own_value, 1_000 * USD);
        assert_eq!(lending.promo_counted, 200 * USD);
        assert_eq!(lending.borrow_limit, 900 * USD);

        let withheld = compute_health(&holding(false), 0, 6, ngn(), promo, 2_000).unwrap();
        assert_eq!(withheld.borrow_limit, 0, "a withheld holding must unlock no promo");
        assert_eq!(withheld.promo_counted, 0);
        // The collateral is still really there: neither the dust check nor the liquidation
        // line may pretend otherwise.
        assert_eq!(withheld.own_value, 1_000 * USD);
        assert_eq!(withheld.liquidation_line, 900 * USD);
    }
```

- [ ] **Step 2: Run it and watch it fail to compile**

Run: `cargo test --lib 2>&1 | grep -E "error\[|lends_borrowing_power"`
Expected: `error[E0560]: struct `CollateralValue` has no field named `lends_borrowing_power``.

- [ ] **Step 3: Add the flag and gate both routes**

In `programs/hodl_loans/src/math/health.rs`, add the field to `CollateralValue` after `liquidation_threshold_bps`:

```rust
    /// False when the asset is barred from supporting new exposure — the admin's borrow pause,
    /// or a mint that has scaled its multiplier past the asset's ceiling. It suppresses *both*
    /// ways this holding can raise `borrow_limit`: its own LTV term, and the promo cap it
    /// would otherwise unlock. Zeroing `ltv_bps` alone is not enough, because `promo_cap_total`
    /// is a fraction of the holding's *value* and never looked at `ltv_bps`.
    ///
    /// `own_value` and `liquidation_line` still count the holding in full: the collateral is
    /// really there, a paused asset must not make a live loan liquidatable, and `own_value`
    /// decides whether `write_off_loan` may treat a position as dust.
    pub lends_borrowing_power: bool,
```

Then gate the two accumulations in `compute_health`. Both gates, in full — the second is the half a first draft missed:

```rust
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
        if c.lends_borrowing_power {
            health.borrow_limit =
                add(health.borrow_limit, mul_div_floor(value, c.ltv_bps as u128, BPS)?)?;
        }
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
        //
        // Gated on the same flag as the LTV term above, and for the same reason: promo is a
        // topping on collateral the borrower can borrow against, so an asset we refuse to lend
        // against must not unlock promo either. Without this gate a borrow-paused asset still
        // raised `borrow_limit` by up to `promo_cap_bps` of its value — the brake would be on
        // and the position could still borrow.
        if c.lends_borrowing_power {
            promo_cap_total =
                add(promo_cap_total, mul_div_floor(value, promo_cap_bps as u128, BPS)?)?;
        }
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
```

Every existing `CollateralValue` literal in the unit tests takes `lends_borrowing_power: true` — they all describe ordinary assets.

- [ ] **Step 4: Run the unit suite**

Run: `cargo test --lib`
Expected: PASS, 58 tests. Confirm the new one is load-bearing by ungating `promo_cap_total` and watching it fail.

- [ ] **Step 5: Add both fields to the asset**

In `programs/hodl_loans/src/state/collateral.rs`, out of `reserved` — the size must not change:

```rust
    /// Blocks new *borrowing* backed by this asset. While set, the holding's
    /// `lends_borrowing_power` is false, which suppresses both ways it could raise
    /// `borrow_limit`: its own LTV term and the promo cap its value would otherwise unlock.
    /// It still counts in full at `own_value` and the liquidation line, and it can still be
    /// deposited, withdrawn and seized — pausing an asset must not strand the collateral
    /// already behind it, and must not make a position that was liquidatable a moment ago
    /// suddenly safe.
    /// Taken from the reserved padding, so the account size is unchanged.
    pub borrow_paused: bool,
    /// Per-asset ceiling on the mint's scaled-UI multiplier, in `MULTIPLIER_SCALE` fixed
    /// point. `0` means no per-asset ceiling — only the global `MAX_MULTIPLIER` applies.
    /// Exceeding it does not fail the price read: the asset simply stops lending borrowing
    /// power, exactly as `borrow_paused` does. See the walk in `valuation.rs`.
    /// Taken from the reserved padding, so the account size is unchanged.
    pub max_multiplier: u128,
    pub reserved: [u8; 79],
```

Append `max_multiplier` to `CollateralParams`, to `apply_params`, and to `params()`:

```rust
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
    /// See `CollateralAsset::max_multiplier`. Appended, so existing field order is unchanged.
    pub max_multiplier: u128,
}
```

`apply_params` and `params()` each gain the matching `max_multiplier` line, so the struct round-trips.

- [ ] **Step 6: Validate `max_multiplier`**

Without this an admin who means "cap at 1×" and writes `1` gets 10⁻¹², which puts **every** asset over its ceiling — a `Standard` asset included, whose multiplier is the constant `MULTIPLIER_ONE`. That silently zeroes its borrowing power and freezes every live borrower out of withdrawing collateral, with no event and no error where the mistake was made. Add to `CollateralParams::validate`, last:

```rust
        // `max_multiplier` is `MULTIPLIER_SCALE` fixed point, so a ceiling below `1.0` is
        // almost certainly an admin who meant "cap at 1x" and wrote `1`. Left unchecked that
        // reads as 10^-12 and puts *every* asset over its ceiling — a `Standard` asset
        // included, whose multiplier is the constant `MULTIPLIER_ONE`. The asset would then
        // silently lend no borrowing power, which also freezes every existing borrower out of
        // withdrawing collateral, with no event and no error at the moment it was set.
        // Above `MAX_MULTIPLIER` the ceiling can never bind, since the price read rejects
        // those multipliers first; accepting it would advertise a bound that does nothing.
        require!(
            self.max_multiplier == 0
                || (self.max_multiplier >= MULTIPLIER_ONE && self.max_multiplier <= MAX_MULTIPLIER),
            HodlError::InvalidParameters
        );
```

- [ ] **Step 7: Set the flag in the walk**

In `programs/hodl_loans/src/valuation.rs`, above `load_collateral_values`'s doc comment — **not between that doc comment and its signature**, which would silently reassign the walk's documentation to the helper:

```rust
/// Whether the mint's live multiplier has run past the ceiling the admin set for this asset.
/// `0` disables the ceiling, which is what every asset listed before the field existed carries
/// and what a `Standard` asset — whose multiplier is always `MULTIPLIER_ONE` — wants anyway.
fn over_multiplier_ceiling(asset: &CollateralAsset, multiplier: u128) -> bool {
    asset.max_multiplier != 0 && multiplier > asset.max_multiplier
}
```

Then replace the `ltv_bps` line in the `CollateralValue` the walk pushes:

```rust
            ltv_bps: asset.ltv_bps,
            liquidation_threshold_bps: asset.liquidation_threshold_bps,
            // Two reasons an asset stops supporting new exposure: the admin's borrow pause,
            // and a mint that has scaled its multiplier past the ceiling set for it. Both are
            // the same answer to different questions, so they share one flag.
            //
            // The ceiling is the reason the flag exists rather than a hard rejection:
            // refusing the price would seal the position against liquidation and write-off
            // too, trading a remote economic risk for a likely liveness failure — which is
            // why `MAX_MULTIPLIER` is deliberately loose. Withholding only borrowing power
            // bounds what the scaled-UI authority can conjure while every exit path keeps
            // working at the true multiplier.
            //
            // `compute_health` reads the flag at both places a holding can raise
            // `borrow_limit` — its LTV term and the promo cap it unlocks — and nowhere else,
            // so `own_value` and `liquidation_line` still count the holding in full. That is
            // what keeps a pause from making a live loan liquidatable.
            lends_borrowing_power: !asset.borrow_paused
                && !over_multiplier_ceiling(&asset, multiplier),
```

- [ ] **Step 8: Add the admin instruction**

`programs/hodl_loans/src/events.rs`, appended:

```rust
#[event]
pub struct CollateralBorrowPauseSet {
    pub collateral: Pubkey,
    pub old_paused: bool,
    pub paused: bool,
    pub by: Pubkey,
}
```

`programs/hodl_loans/src/instructions/admin/collateral_admin.rs`, after `handle_set_collateral_paused`:

```rust
/// Stops new borrowing backed by one asset without touching deposits, withdrawals or
/// liquidation. Separate from `set_collateral_paused` because the two answer different
/// questions: that one says "take no more of this", this one says "lend nothing new against
/// what is already here". An asset whose price feed has become unreliable wants the second
/// while borrowers keep the first.
///
/// "Lend nothing new" means both routes from a holding to `borrow_limit` — its LTV term and
/// the promo cap its value unlocks. Debt already drawn is untouched: this cannot make a live
/// loan liquidatable, because `own_value` and the liquidation line still count the asset.
#[derive(Accounts)]
pub struct SetCollateralBorrowPaused<'info> {
    pub signer: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(mut, seeds = [COLLATERAL_SEED, collateral.mint.as_ref()], bump = collateral.bump)]
    pub collateral: Box<Account<'info, CollateralAsset>>,
}

pub fn handle_set_collateral_borrow_paused(
    ctx: Context<SetCollateralBorrowPaused>,
    paused: bool,
) -> Result<()> {
    let signer = ctx.accounts.signer.key();
    let config = &ctx.accounts.config;
    // The same asymmetry the deposit pause uses: the guardian can stop the bleeding, only the
    // admin can start it again.
    if paused {
        require!(signer == config.admin || signer == config.guardian, HodlError::Unauthorized);
    } else {
        require!(signer == config.admin, HodlError::Unauthorized);
    }
    let collateral_key = ctx.accounts.collateral.key();
    let old_paused = ctx.accounts.collateral.borrow_paused;
    ctx.accounts.collateral.borrow_paused = paused;
    emit!(CollateralBorrowPauseSet { collateral: collateral_key, old_paused, paused, by: signer });
    Ok(())
}
```

`handle_list_collateral`'s initializer takes `borrow_paused: false`, `max_multiplier: 0` and `reserved: [0; 79]`. `programs/hodl_loans/src/lib.rs` gets the entry point:

```rust
    pub fn set_collateral_borrow_paused(
        ctx: Context<SetCollateralBorrowPaused>,
        paused: bool,
    ) -> Result<()> {
        instructions::handle_set_collateral_borrow_paused(ctx, paused)
    }
```

- [ ] **Step 9: Update the `MAX_MULTIPLIER` note and spec §14**

`constants.rs`'s note currently defers the per-asset ceiling. It no longer should:

```rust
/// for a more likely liveness failure. The shape that bounds the authority without that cost
/// is a per-asset, admin-settable ceiling which withholds *borrowing power* instead of
/// rejecting the price: `CollateralAsset::max_multiplier`, added in Plan 7. This constant
/// stays loose on purpose — it is the arithmetic backstop, not the policy knob.
```

In spec §14, replace the `**Deferred:**` sentence on the scaled-UI authority with the `**Resolved in Plan 7:**` paragraph describing `CollateralAsset::max_multiplier` — that it withholds borrowing power rather than rejecting the price, that this suppresses **both** routes to `borrow_limit`, and that `own_value` and `liquidation_line` keep counting the holding at its true multiplier.

- [ ] **Step 10: Write the behavioural tests**

Harness helper, in `tests/common/mod.rs`:

```rust
pub fn set_collateral_borrow_paused_ix(signer: &Pubkey, mint: &Pubkey, paused: bool) -> Instruction {
    ix(
        hodl_loans::instruction::SetCollateralBorrowPaused { paused },
        hodl_loans::accounts::SetCollateralBorrowPaused {
            signer: *signer,
            config: config_pda(),
            collateral: collateral_pda(mint),
        },
    )
}
```

Authority, in `tests/collateral.rs`:

```rust
#[test]
fn the_guardian_pauses_borrowing_against_one_asset_and_only_the_admin_lifts_it() {
    let mut env = Env::initialized();
    let mint = env.list_spl_collateral(6);
    let guardian = env.guardian.pubkey();
    let admin = env.admin.pubkey();

    let pause = set_collateral_borrow_paused_ix(&guardian, &mint, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();
    assert!(env.collateral(&mint).borrow_paused);
    // The two pauses are independent switches: this one leaves deposits open.
    assert!(!env.collateral(&mint).paused);

    let guardian_unpause = set_collateral_borrow_paused_ix(&guardian, &mint, false);
    assert_hodl_error(send(&mut env.svm, &[guardian_unpause], &[&env.guardian]), HodlError::Unauthorized);

    let stranger = env.funded_keypair();
    let stranger_pause = set_collateral_borrow_paused_ix(&stranger.pubkey(), &mint, true);
    assert_hodl_error(send(&mut env.svm, &[stranger_pause], &[&stranger]), HodlError::Unauthorized);

    let unpause = set_collateral_borrow_paused_ix(&admin, &mint, false);
    send(&mut env.svm, &[unpause], &[&env.admin]).unwrap();
    assert!(!env.collateral(&mint).borrow_paused);
}
```

The pause removes borrowing power, in `tests/loans.rs`:

```rust
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
```

**And on a position that actually holds promo** — the fixture above holds none, so it cannot tell a gated promo cap from an ungated one. In `tests/promo_health.rs`:

```rust
#[test]
fn a_borrow_paused_asset_unlocks_no_promo_either() {
    // The brake has to stop *both* routes from a holding to the borrow limit. Its own LTV
    // term is the obvious one; the promo cap is the other, and it is a fraction of the
    // holding's value that never consulted `ltv_bps`. A test on a promo-free position cannot
    // tell the two apart — this one can.
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    env.redeem_promo(borrower, &setup.cngn, 1, 50_000 * ONE_CNGN, 7).unwrap();
    env.take_loan(borrower, &setup, WITH_PROMO_CEILING, 30 * DAY).unwrap();

    // Repay in full so the position is idle, then pause borrowing against the collateral.
    env.mint_to(&setup.cngn, &setup.borrower_cngn, WITH_PROMO_CEILING);
    env.repay(&setup, 0, u64::MAX).unwrap();
    let pause = set_collateral_borrow_paused_ix(&env.guardian.pubkey(), &setup.usdc, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();
    assert_eq!(env.position(&setup.borrower.pubkey()).promo_balance, 50_000 * ONE_CNGN);

    // Promo is still held and the collateral is still there, but neither may be borrowed
    // against: the market's smallest permitted loan is refused. Before the promo cap was
    // gated on the same flag, this call succeeded for up to 20% of the collateral's value.
    assert_hodl_error(
        env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 30 * DAY),
        HodlError::Unhealthy,
    );

    // Lifting it restores exactly the promo-assisted ceiling, so nothing else moved.
    let unpause = set_collateral_borrow_paused_ix(&env.admin.pubkey(), &setup.usdc, false);
    send(&mut env.svm, &[unpause], &[&env.admin]).unwrap();
    env.take_loan(&setup.borrower, &setup, WITH_PROMO_CEILING, 30 * DAY).unwrap();
}
```

The pause does not reach liquidation, in `tests/liquidation.rs`:

```rust
#[test]
fn pausing_borrowing_leaves_the_liquidation_line_where_it_was() {
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    let pause = set_collateral_borrow_paused_ix(&env.guardian.pubkey(), &setup.usdc, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();

    let liquidator = env.new_liquidator(&setup.cngn, LOAN);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    // The position is still healthy. The pause removed borrowing power, not the collateral
    // standing behind debt already taken — otherwise every live loan against the asset would
    // become liquidatable the instant an admin paused it. This holds structurally rather than
    // by convention: the pause zeroes `ltv_bps`, `ltv_bps` reaches only `borrow_limit`, and
    // `is_liquidatable` reads `liquidation_line`. There is no call-site flag to get wrong.
    let result = env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, ONE_CNGN);
    assert_hodl_error(result, HodlError::NotLiquidatable);

    // And once the price does fall, the pause is no obstacle to seizing it.
    env.set_pyth_price(&setup.usdc, USDC_CRASHED, 0);
    env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, ONE_CNGN).unwrap();
}
```

Withdrawal is exposure-increasing too, in `tests/withdraw.rs`:

```rust
#[test]
fn a_borrow_paused_asset_backs_no_withdrawal_while_a_loan_is_live() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    let (usdc, cngn) = (setup.usdc, setup.cngn);
    env.take_loan(&setup.borrower, &setup, 500_000 * ONE_CNGN, 365 * DAY).unwrap();
    let token = env.create_token_account(&usdc, &owner);
    let withdraw =
        |amount| withdraw_collateral_ix(&owner, &usdc, &SPL_TOKEN, &token, Some(&cngn), amount, price_pairs(&[usdc]));
    let key = &setup.borrower.key;

    let pause = set_collateral_borrow_paused_ix(&env.guardian.pubkey(), &usdc, true);
    send(&mut env.svm, &[pause], &[&env.guardian]).unwrap();
    // Withdrawing is exposure-increasing in the same way borrowing is, and the slack that
    // would have allowed it is the paused asset's own borrowing power. One atom is refused.
    assert_hodl_error(env.sponsored(withdraw(1), key), HodlError::Unhealthy);

    // Lifting the pause restores the same 553 USDC of slack the unpaused case has.
    let unpause = set_collateral_borrow_paused_ix(&env.admin.pubkey(), &usdc, false);
    send(&mut env.svm, &[unpause], &[&env.admin]).unwrap();
    env.sponsored(withdraw(553 * ONE_USDC), key).unwrap();
    assert_hodl_error(env.sponsored(withdraw(ONE_USDC), key), HodlError::Unhealthy);
}
```

The ceiling, in `tests/xstocks.rs` — note it checks both that borrowing is refused **and** that every exit path still works, which is the whole design:

```rust
#[test]
fn a_multiplier_past_the_assets_ceiling_withdraws_borrowing_power_without_sealing_the_position() {
    let (mut env, setup) = Env::loan_ready();
    let stock = env.list_xstock_collateral(200);
    let borrower = env.new_borrower();
    env.deposit_collateral(&borrower, &stock, 10 * ONE_XSTOCK);
    let borrower_cngn = env.create_token_account(&setup.cngn, &borrower.pubkey());
    let setup = LoanSetup { borrower, borrower_cngn, ..setup };
    let admin = env.admin.pubkey();
    let owner = setup.borrower.pubkey();

    // The admin caps this asset at 1.5×, which is where it sits today, so nothing changes yet.
    let mut params = xstock_collateral_params(&stock);
    params.max_multiplier = 3 * hodl_loans::MULTIPLIER_SCALE / 2;
    send(&mut env.svm, &[update_collateral_params_ix(&admin, &stock, params)], &[&env.admin]).unwrap();
    let now = env.now();
    env.set_multiplier(&stock, 1.5, now);
    env.take_loan(&setup.borrower, &setup, CEILING_AT_1_5, 30 * DAY).unwrap();

    // The issuer then scales far past the ceiling. The collateral's *value* really did rise,
    // but the protocol will not lend against a number this authority can set at will.
    env.set_multiplier(&stock, 1_000.0, env.now());
    assert_hodl_error(env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 30 * DAY), HodlError::Unhealthy);

    // The whole point of withholding borrowing power rather than rejecting the price: every
    // exit path still works. The position is priced, not sealed — so it can be liquidated if
    // it ever needs to be, and the borrower can still get their collateral back.
    let liquidator = env.new_liquidator(&setup.cngn, CEILING_AT_1_5);
    let seized_to = env.create_token_account(&stock, &liquidator.pubkey());
    let result = env.liquidate(&liquidator, &setup, &stock, &seized_to, 0, ONE_CNGN);
    assert_hodl_error(result, HodlError::NotLiquidatable);

    // Repaying and withdrawing both go through — the asset is still valued at its true
    // multiplier everywhere except the borrow limit.
    env.mint_to(&setup.cngn, &setup.borrower_cngn, CEILING_AT_1_5);
    env.repay(&setup, 0, u64::MAX).unwrap();
    let token = env.create_token_account(&stock, &owner);
    let withdraw = withdraw_collateral_ix(&owner, &stock, &TOKEN_2022, &token, None, ONE_XSTOCK, vec![]);
    env.sponsored(withdraw, &setup.borrower.key).unwrap();
    assert_eq!(env.token_balance(&token), ONE_XSTOCK);

    // Lifting the ceiling restores the borrowing power the collateral genuinely carries.
    let mut params = xstock_collateral_params(&stock);
    params.max_multiplier = 0;
    send(&mut env.svm, &[update_collateral_params_ix(&admin, &stock, params)], &[&env.admin]).unwrap();
    env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 30 * DAY).unwrap();
}

#[test]
fn a_multiplier_ceiling_below_one_is_refused_rather_than_silently_freezing_the_asset() {
    // `max_multiplier` is MULTIPLIER_SCALE fixed point, so an admin who means "cap at 1x" and
    // writes `1` is asking for 10^-12. Unvalidated that puts *every* asset over its ceiling —
    // a Standard asset included, whose multiplier is the constant MULTIPLIER_ONE — which
    // silently zeroes its borrowing power and freezes every existing borrower out of
    // withdrawing collateral, with no event and no error where the mistake was made.
    let mut env = Env::initialized();
    let admin = env.admin.pubkey();
    let usdc = env.list_spl_collateral(6);

    let mut params = default_collateral_params(&usdc);
    params.max_multiplier = 1;
    let bad = update_collateral_params_ix(&admin, &usdc, params);
    assert_hodl_error(send(&mut env.svm, &[bad], &[&env.admin]), HodlError::InvalidParameters);

    // Above MAX_MULTIPLIER the ceiling could never bind — the price read rejects those
    // multipliers first — so advertising it is refused too.
    let mut params = default_collateral_params(&usdc);
    params.max_multiplier = hodl_loans::MAX_MULTIPLIER + 1;
    let too_high = update_collateral_params_ix(&admin, &usdc, params);
    assert_hodl_error(send(&mut env.svm, &[too_high], &[&env.admin]), HodlError::InvalidParameters);

    // The two values that mean something both go through: uncapped, and exactly 1x.
    for value in [0, hodl_loans::MULTIPLIER_ONE] {
        let mut params = default_collateral_params(&usdc);
        params.max_multiplier = value;
        let ok = update_collateral_params_ix(&admin, &usdc, params);
        send(&mut env.svm, &[ok], &[&env.admin]).unwrap();
        assert_eq!(env.collateral(&usdc).max_multiplier, value);
    }
}
```

- [ ] **Step 11: Run everything**

Run: `./scripts/test.sh`
Expected: all pass. `CollateralAsset::INIT_SPACE` is still 294 — the unit guard fails if either field did not come out of the padding.

Verify the mechanism is load-bearing in three separate ways, rebuilding with `cargo build-sbf --tools-version v1.52` before each: neuter the flag (`lends_borrowing_power: true` always) and the borrow/withdraw tests fail; make the pause also zero `liquidation_threshold_bps` and the liquidation test fails; replace the ceiling's withholding with a hard `require!` on the price and the xStock test fails. The second and third are the ones that prove the tests pin the *design* and not just the presence of a check.

- [ ] **Step 12: Commit**

```bash
git add -A
git commit -m "feat(plan7): two ways to withhold an asset's borrowing power

collateral.paused gated deposits only; nothing stopped new borrowing against
an asset. And spec §14 deferred a per-asset scaled-UI multiplier ceiling,
because tightening the global MAX_MULTIPLIER seals the position against
liquidation too.

Both are the same answer: CollateralValue::lends_borrowing_power, read at both
places a holding can raise borrow_limit — its LTV term and the promo cap its
value unlocks — and nowhere else. Gating only the LTV term leaves the promo cap
unlocking borrowing power against an asset the brake is on, because
promo_cap_total is a fraction of the holding's value and never consulted
ltv_bps. own_value and liquidation_line still count the holding in full, so a
pause cannot make a live loan liquidatable and write_off_loan's dust check is
unaffected.

Both fields come out of CollateralAsset's reserved padding; INIT_SPACE stays
294. max_multiplier is validated, because at MULTIPLIER_SCALE a value of 1
means 10^-12 and would silently freeze every asset.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---
### Task 4: What runs first, and what accrual actually touches

Two corrections to recorded order and recorded claims. Neither changes what the program computes; both change what it says.

`take_loan` checked `MarketMismatch` and the free-slot after accruing and after the utilization cap, so a wrong-market borrow at full utilization reported `UtilizationCapExceeded` — or, against an empty market, `InsufficientCash`. True of that market, but not what was wrong with the call. Spec §10 puts both in steps 1–2, ahead of step 3's accrual.

And spec §9 claimed accrual is "run first by every instruction that touches the market". Three do not: `harvest_reserve` and the two sweeps. `Market::accrue` writes only `accrued_interest`, `accrual_remainder` and `last_accrual_ts`, while those three read and write only `protocol_reserve` and `cash` — which accrual never touches. Adding the calls would cost compute and change nothing, so the spec is what is wrong here, not the code.

**Files:**
- Modify: `programs/hodl_loans/src/instructions/loans/take_loan.rs`, `src/instructions/admin/reserve.rs`, `src/instructions/admin/sweep.rs`
- Modify: `docs/superpowers/specs/2026-09-17-solana-fixed-loans-design.md` (§9)
- Test: `programs/hodl_loans/tests/loans.rs`

**Interfaces:** none. Nothing else depends on this task.

- [ ] **Step 1: Write the failing test**

An existing test already covers a wrong-market borrow, but against a funded market — where the cap passes anyway, so it cannot see the order. This one gives market B no liquidity, so its own checks would reject the call on their own merits. Append to `programs/hodl_loans/tests/loans.rs`:

```rust
#[test]
fn a_wrong_market_borrow_reports_the_mismatch_not_that_markets_own_state() {
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    // Bind the position to market A.
    env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 30 * DAY).unwrap();

    // Market B is real and promo-equipped but holds no lender liquidity, so its own
    // utilization and cash checks would reject any borrow on their own merits. Spec §10 puts
    // `MarketMismatch` in step 1, ahead of the step-3 accrual and the step-4 cap, so that is
    // what the caller is told: market B's emptiness is true but is not what is wrong with
    // this call. Before the checks were ordered to match the spec this reported
    // `InsufficientCash`.
    let empty = env.create_mint(MintKind::CngnLike, 6);
    env.create_market_with_promo(&empty);
    assert_eq!(env.market(&empty).cash, 0);

    let account = env.create_token_account(&empty, &owner);
    let prices = env.price_accounts(&owner);
    let borrow = take_loan_ix(&owner, &empty, &account, 1_000 * ONE_CNGN, 30 * DAY, prices);
    assert_hodl_error(
        send(&mut env.svm, &[borrow], &[&env.admin, &setup.borrower.key]),
        HodlError::MarketMismatch,
    );
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `./scripts/test.sh 2>&1 | grep -E "wrong_market_borrow|error\["`
Expected: FAIL with `InsufficientCash` where `MarketMismatch` was wanted.

- [ ] **Step 3: Move both checks ahead of accrual**

In `programs/hodl_loans/src/instructions/loans/take_loan.rs`, after the tenure check and before `market.accrue(now)?`:

```rust
    // Spec §10 puts both position preconditions in steps 1-2, before step 3 accrues. Taken
    // later, a borrow against the wrong market at full utilization reported
    // `UtilizationCapExceeded` — true of the market, but not what was wrong with the call.
    // Only the reported error changes: both are read-only, and neither can succeed here and
    // fail below, because nothing between the two points writes to the position.
    let free_index = {
        let position = ctx.accounts.position.load()?;
        require!(
            position.market == Pubkey::default() || position.market == market_key,
            HodlError::MarketMismatch
        );
        position.free_loan_index().ok_or(HodlError::NoFreeLoanSlot)?
    };
```

Then the later block reads its index from that binding instead of recomputing it:

```rust
        let mut position = ctx.accounts.position.load_mut()?;
        let index = free_index;
```

The immutable borrow must be scoped and dropped before `load_mut()?`, or the `AccountLoader`'s `RefCell` fails at runtime. The `{ ... }` block above does that.

- [ ] **Step 4: Run the test**

Run: `./scripts/test.sh`
Expected: all pass. Nothing between the two points writes to the position — the intervening code touches only `market` — so this changes the reported error and nothing else.

- [ ] **Step 5: Say why three instructions do not accrue**

In `programs/hodl_loans/src/instructions/admin/reserve.rs`, above `handle_harvest_reserve`:

```rust
/// Deliberately does **not** call `Market::accrue` first, unlike every instruction that
/// settles a loan or reads `total_assets`. Accrual moves only `accrued_interest`,
/// `accrual_remainder` and `last_accrual_ts`; `protocol_reserve` grows solely in `repay_loan`
/// and `cash` solely on real token movement, so accruing here could not change what this
/// instruction reads or writes. Harvesting drops `cash` and `protocol_reserve` by the same
/// amount, leaving `available_cash` and `total_assets` untouched, so lenders are unaffected
/// either way. Spec §9 names this exception; adding the call would cost compute and buy
/// nothing.
```

In `programs/hodl_loans/src/instructions/admin/sweep.rs`, above `handle_sweep_market_excess`:

```rust
/// Like `harvest_reserve`, this does not accrue first: it compares the vault's token balance
/// against `market.cash`, and accrual never touches `cash`. See spec §9.
```

- [ ] **Step 6: Narrow the spec claim**

Replace spec §9's heading and opening so it names which instructions accrue and why three do not — that accrual writes only `accrued_interest`, `accrual_remainder` and `last_accrual_ts`; that the three read and write only `protocol_reserve` and `cash`; and that skipping an interval costs no precision, because accrual is linear in elapsed time and carries its own remainder.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "fix(plan7): take_loan check order; narrow spec §9's accrual claim

take_loan reported UtilizationCapExceeded, or InsufficientCash against an
empty market, for a borrow against the wrong market. True of that market, not
what was wrong with the call. Spec §10 puts both position preconditions in
steps 1-2, ahead of step 3's accrual. Only the reported error changes.

Spec §9 claimed accrual runs first in every instruction that touches the
market. harvest_reserve and the two sweeps do not, and provably need not.
Narrowed the spec and recorded the reasoning at both call sites so it does not
read as an omission.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Three values that had no ceiling

Each of these locks something up at the top end, and none of them could be undone afterwards.

`Config::collateral_count` had no bound, but `set_promo_cap` must name **every** listed asset in one transaction. The binding limit is `MAX_TX_ACCOUNT_LOCKS = 128` — the accounts a transaction may lock. Address lookup tables relieve the *message size* limit, not the lock limit, so the `u8` account index's 256-key ceiling is never reached. Get this wrong and a protocol can list its way into `set_promo_cap` being permanently unsendable, with no way back: `collateral_count` falls only on delisting, which requires the asset to be unused.

`campaign.redeem_until` was bounded below but not above, and `voucher_expiry` not at all. Expiry is not in the voucher receipt's PDA seeds, so `close_voucher_receipt` waits for `now > voucher_expiry` and a far-future expiry locks the rent indefinitely.

**Files:**
- Modify: `programs/hodl_loans/src/constants.rs`, `src/errors.rs`, `src/instructions/admin/collateral_admin.rs`, `src/instructions/promos/campaign.rs`, `src/instructions/promos/redeem.rs`
- Modify: `docs/superpowers/specs/2026-09-17-solana-fixed-loans-design.md` (§8, §12)
- Test: `programs/hodl_loans/tests/collateral.rs`, `tests/promo_redeem.rs`

**Interfaces:**
- Produces: `MAX_LISTED_COLLATERAL: u16`, `MAX_CAMPAIGN_LIFETIME: i64`, and the appended errors `CollateralLimitReached` and `VoucherOutlivesCampaign`.

- [ ] **Step 1: Add the two constants**

Append to `programs/hodl_loans/src/constants.rs`, after `MAX_NGN_STALE_SLOTS`:

```rust
/// Upper bound on `Config::collateral_count`, derived from `set_promo_cap`, the one
/// instruction that must name **every** listed asset at once: its `remaining_accounts` count
/// has to equal `collateral_count` exactly.
///
/// The binding limit is **`MAX_TX_ACCOUNT_LOCKS = 128`** — the total accounts a transaction
/// may lock, read-only included. Address lookup tables relieve the *message size* limit (32
/// bytes per key), not the lock limit: an ALT-loaded address still takes a lock. The `u8`
/// account index caps a message at 256 keys, but that ceiling is never reached because the
/// lock limit bites at half of it. `SetPromoCap` spends three locks on the admin, the config
/// and the program id, leaving **125**.
///
/// 96 leaves 29 spare, for accounts a future `SetPromoCap` might need and for anything a
/// client's own lookup-table usage costs. The bound matters because there is no way back:
/// `collateral_count` only falls when an asset is delisted, and delisting requires the asset
/// to be unused, so a protocol that listed its way past the ceiling would have `set_promo_cap`
/// frozen until positions unwound. `the_asset_list_bound_keeps_set_promo_cap_inside_the_lock_limit`
/// pins the arithmetic so the constant cannot drift past it.
pub const MAX_LISTED_COLLATERAL: u16 = 96;
/// Upper bound on how far ahead of creation a campaign's `redeem_until` may sit, and so —
/// via the voucher-outlives-campaign rule in `redeem_promo` — on how long a voucher receipt
/// can hold its rent before `close_voucher_receipt` will take it.
///
/// Unlike `MAX_COLLATERAL_DECIMALS` this is a policy bound, not a derived one: no arithmetic
/// fails past it. A promo campaign still running a year after it was created is a decision
/// worth re-making by opening a new campaign, rather than one that should quietly keep
/// rent locked in receipts nobody can close.
pub const MAX_CAMPAIGN_LIFETIME: i64 = 365 * 86_400;
```

- [ ] **Step 2: Pin the asset-list arithmetic in a unit test**

This is the test that matters, because the bound is a number and the failure mode is that the number is wrong. A first draft set it to 128, derived from the 256-key `u8` index — which is not the binding limit, and would have permitted an asset list that makes `set_promo_cap` unsendable. Add to `constants.rs`'s test module:

```rust
    #[test]
    fn the_asset_list_bound_keeps_set_promo_cap_inside_the_lock_limit() {
        // `set_promo_cap` must name every listed asset in one transaction, so the asset list
        // is bounded by how many accounts a transaction may lock — not by how many a message
        // can index. Solana's `MAX_TX_ACCOUNT_LOCKS` is 128 (solana-transaction 3.1.0,
        // `sanitized.rs`); it is not importable from an on-chain crate, so it is restated
        // here and this test is what keeps the two in step.
        //
        // The first draft of `MAX_LISTED_COLLATERAL` was 128, derived from the 256-key `u8`
        // index limit, which is not the binding one — it would have permitted an asset list
        // that makes `set_promo_cap` permanently unsendable, the exact state the bound exists
        // to prevent. This assertion is why that cannot recur silently.
        const MAX_TX_ACCOUNT_LOCKS: usize = 128;
        // admin, config, program id.
        const SET_PROMO_CAP_FIXED_ACCOUNTS: usize = 3;
        assert!(
            MAX_LISTED_COLLATERAL as usize + SET_PROMO_CAP_FIXED_ACCOUNTS <= MAX_TX_ACCOUNT_LOCKS,
            "MAX_LISTED_COLLATERAL ({MAX_LISTED_COLLATERAL}) + {SET_PROMO_CAP_FIXED_ACCOUNTS} \
             exceeds MAX_TX_ACCOUNT_LOCKS ({MAX_TX_ACCOUNT_LOCKS}): set_promo_cap would be \
             unsendable at a full asset list"
        );
    }
```

- [ ] **Step 3: Append the error variants**

Codes are `6000 + position`, so these go at the **end** of `HodlError`:

```rust
    #[msg("The protocol already lists the maximum number of collateral assets")]
    CollateralLimitReached,
    #[msg("Voucher expiry is later than its campaign's redeem_until")]
    VoucherOutlivesCampaign,
```

- [ ] **Step 4: Enforce the asset-list bound**

In `handle_list_collateral`, before the count is incremented — `list_collateral` is the only thing that raises it:

```rust
    // `set_promo_cap` must name every listed asset in one transaction, so the list has a
    // ceiling; see `MAX_LISTED_COLLATERAL`. Checked here rather than in `set_promo_cap`
    // because by then it is too late — listing is the only thing that grows the count.
    require!(config.collateral_count < MAX_LISTED_COLLATERAL, HodlError::CollateralLimitReached);
```

- [ ] **Step 5: Bound the campaign and the voucher**

In `programs/hodl_loans/src/instructions/promos/campaign.rs`, replacing the existing lower-bound-only check:

```rust
    let now = Clock::get()?.unix_timestamp;
    require!(redeem_until > now, HodlError::InvalidParameters);
    // Bounded above as well, because a campaign's end is what bounds its vouchers' expiry,
    // and a voucher's expiry is what holds its receipt's rent. See `MAX_CAMPAIGN_LIFETIME`.
    // `saturating_add` cannot wrap the ceiling backwards, so an absurd clock fails closed.
    require!(redeem_until <= now.saturating_add(MAX_CAMPAIGN_LIFETIME), HodlError::InvalidParameters);
```

In `programs/hodl_loans/src/instructions/promos/redeem.rs`, **after** the two campaign-window checks — placed last so a closed campaign still reports as closed rather than as a voucher problem:

```rust
    // A voucher may not outlive the campaign it draws on. Without this the promo signer alone
    // decides how long the receipt this redemption creates holds its rent: expiry is not part
    // of the receipt's seeds, so `close_voucher_receipt` waits for `now > voucher_expiry` and
    // a far-future expiry locks the rent indefinitely. Tying it to `redeem_until` — itself
    // bounded by `MAX_CAMPAIGN_LIFETIME` — puts that bound back under admin control, where
    // the campaign's budget and lifetime already sit.
    //
    // This is a real behavioural change, not a free one. `voucher_expiry` and `now` are
    // different quantities: a voucher expiring after `redeem_until` is perfectly redeemable
    // at any `now <= redeem_until`, and both prior checks pass for it. A backend issuing
    // rolling 30-day vouchers signs such a voucher every day of a campaign's last 30, and
    // every one of them stops working here. Those vouchers must be re-signed with expiries
    // clamped to the campaign — the migration note for this change.
    require!(voucher_expiry <= ctx.accounts.campaign.redeem_until, HodlError::VoucherOutlivesCampaign);
```

Note the migration in that comment and carry it into the spec: this **rejects vouchers that were previously valid**. `voucher_expiry` and `now` are different quantities, so a voucher expiring after `redeem_until` redeems fine at any `now <= redeem_until`. A backend issuing rolling 30-day vouchers signs one every day of a campaign's last 30, and all of them stop working.

- [ ] **Step 6: Write the tests**

In `programs/hodl_loans/tests/collateral.rs`. Note what it does *not* claim: it pins that listing refuses at the bound, and never calls `set_promo_cap`. The arithmetic half is Step 2's unit test.

```rust
#[test]
fn listing_refuses_once_the_asset_list_is_at_its_bound() {
    let mut env = Env::initialized();
    let program = hodl_loans::ID;
    // This pins the guard only. That the bound is the *right* number — small enough that
    // `set_promo_cap` still fits inside `MAX_TX_ACCOUNT_LOCKS` — is arithmetic, and
    // `the_asset_list_bound_keeps_set_promo_cap_inside_the_lock_limit` in `constants.rs`
    // pins that instead; listing a full asset list here would take minutes and still not
    // exercise the transaction limit.
    //
    // Listing 96 assets for real would take minutes, and the guard is what is under test,
    // not the counter: `list_collateral` is the only writer that raises `collateral_count`,
    // and every increment runs this same check. Re-serialize the config with the count at the
    // ceiling — writing the struct rather than poking an offset, so the test does not silently
    // stop testing anything if a field is ever added above it.
    let set_count = |env: &mut Env, count: u16| {
        let mut config = env.config();
        config.collateral_count = count;
        let mut encoded = Vec::new();
        anchor_lang::AccountSerialize::try_serialize(&config, &mut encoded).unwrap();
        let mut data = env.svm.get_account(&config_pda()).unwrap().data;
        data[..encoded.len()].copy_from_slice(&encoded);
        env.set_account_data(&config_pda(), &program, data);
    };
    let listing = |env: &Env, mint: &Pubkey| {
        list_collateral_ix(
            &env.admin.pubkey(),
            mint,
            &SPL_TOKEN,
            default_collateral_params(mint),
            hodl_loans::CollateralKind::Standard,
        )
    };

    set_count(&mut env, hodl_loans::MAX_LISTED_COLLATERAL);
    assert_eq!(env.config().collateral_count, hodl_loans::MAX_LISTED_COLLATERAL);
    let mint = env.create_mint(MintKind::SplToken, 6);
    let list = listing(&env, &mint);
    assert_hodl_error(send(&mut env.svm, &[list], &[&env.admin]), HodlError::CollateralLimitReached);

    // One below the ceiling the same listing goes through, so the guard is the bound itself
    // and not a blanket refusal — and it lands exactly on the ceiling.
    set_count(&mut env, hodl_loans::MAX_LISTED_COLLATERAL - 1);
    let list = listing(&env, &mint);
    send(&mut env.svm, &[list], &[&env.admin]).unwrap();
    assert_eq!(env.config().collateral_count, hodl_loans::MAX_LISTED_COLLATERAL);
}
```

In `programs/hodl_loans/tests/promo_redeem.rs`:

```rust
#[test]
fn a_campaign_cannot_outlast_the_maximum_lifetime() {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();
    let now = env.now();

    let too_long = create_campaign_ix(&admin, &cngn, 1, BUDGET, now + MAX_CAMPAIGN_LIFETIME + 1);
    assert_hodl_error(send(&mut env.svm, &[too_long], &[&env.admin]), HodlError::InvalidParameters);

    // Exactly at the bound is allowed: the check is `<=`.
    let at_bound = create_campaign_ix(&admin, &cngn, 1, BUDGET, now + MAX_CAMPAIGN_LIFETIME);
    send(&mut env.svm, &[at_bound], &[&env.admin]).unwrap();
}

#[test]
fn a_voucher_may_not_outlive_its_campaign() {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();
    let until = env.now() + 86_400;
    send(&mut env.svm, &[create_campaign_ix(&admin, &cngn, 1, BUDGET, until)], &[&env.admin]).unwrap();
    let borrower = env.new_borrower();
    let signer = env.promo_signer.insecure_clone();

    // A validly signed voucher, inside its own expiry and inside the campaign window, but
    // whose expiry reaches past the campaign's end. Its receipt would hold rent no one could
    // reclaim until that date, so it is refused.
    let result =
        env.redeem_voucher_signed_by(&signer, &borrower, &cngn, 1, GRANT, 7, until + 1);
    assert_hodl_error(result, HodlError::VoucherOutlivesCampaign);

    // Expiry exactly at the campaign's end is fine — the check is `<=`, and the receipt
    // becomes closable the second after the campaign closes.
    env.redeem_voucher_signed_by(&signer, &borrower, &cngn, 1, GRANT, 7, until).unwrap();
    assert_eq!(env.position(&borrower.pubkey()).promo_balance, GRANT);
}
```

- [ ] **Step 7: Run everything**

Run: `./scripts/test.sh`
Expected: all pass. If `a_campaign_past_its_window_issues_nothing` fails with `VoucherOutlivesCampaign`, the new check is placed before the campaign-window checks instead of after them.

- [ ] **Step 8: Update the spec and commit**

Spec §8's `set_promo_cap` paragraph gains the `MAX_LISTED_COLLATERAL` ceiling and the lock-limit reason; §12's `create_campaign` gains its lifetime bound and `redeem_promo` the voucher condition with its migration note.

```bash
git add -A
git commit -m "feat(plan7): bound the asset list, campaign lifetime and voucher expiry

Three unbounded values, each of which locks something up at the top end with
no way back.

collateral_count is bounded at MAX_LISTED_COLLATERAL, derived from
MAX_TX_ACCOUNT_LOCKS rather than the u8 account index — lookup tables relieve
message size, not locks — with a unit test pinning the arithmetic so the
constant cannot drift past the limit.

A voucher may no longer outlive its campaign, and a campaign may not outlast
MAX_CAMPAIGN_LIFETIME, so receipt rent is always reclaimable. This rejects
vouchers that were previously valid; the migration note is in the code and
spec §12. The voucher check runs after the two campaign-window checks so a
closed campaign still reports as closed.

Both error variants are appended; existing codes are unchanged.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: A market pause stops the promo inactivity clock

Every write that refreshes `promo_last_activity_at` is either pause-gated or needs a live loan, so a pause longer than `promo_inactivity_seconds` let anyone expire every idle promo balance the moment the window lapsed — charging borrowers for downtime the protocol imposed.

**Two halves, and the first alone is not enough.** Barring `expire_promo` during the pause only defers the harvest: a long pause still leaves everything expirable the instant it lifts. So unpausing also restarts the clock.

`revoke_promo` stays open during a pause. The asymmetry looks backwards against the rule that pauses stop exposure-*increasing* work, and it is an exception to a different rule: expiry's precondition is a *measurement* of borrower inactivity, and a paused market is one the borrower cannot act on — the measurement is invalid, not the operation unsafe. Revocation measures nothing.

**Files:**
- Modify: `programs/hodl_loans/src/state/market.rs`, `src/instructions/admin/market_admin.rs`, `src/instructions/promos/lifecycle.rs`
- Modify: `docs/superpowers/specs/2026-09-17-solana-fixed-loans-design.md` (§12)
- Test: `programs/hodl_loans/tests/promo_lifecycle.rs`

**Interfaces:**
- Produces: `Market::promo_clock_resumed_at: i64`. No later task uses it.

- [ ] **Step 1: Write the failing tests**

The first covers both halves — the gate, and that the gate alone is insufficient. Append to `programs/hodl_loans/tests/promo_lifecycle.rs`:

```rust
#[test]
fn a_pause_stops_the_inactivity_clock_rather_than_merely_deferring_the_harvest() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();
    let admin = env.admin.pubkey();
    let guardian = env.guardian.pubkey();

    // The guardian pauses the market, then leaves it paused for longer than the whole
    // inactivity window. The borrower cannot act on a paused market, so none of this is
    // inactivity the protocol may charge them for.
    send(&mut env.svm, &[set_market_paused_ix(&guardian, &setup.cngn, true)], &[&env.guardian]).unwrap();
    env.warp_seconds(INACTIVITY + DAY);

    // Expiry is barred outright while paused.
    let stranger = env.funded_keypair();
    let during = expire_promo_ix(&setup.cngn, &owner);
    assert_hodl_error(send(&mut env.svm, &[during], &[&stranger]), HodlError::MarketPaused);

    // And barring it during the pause is not on its own enough: the clock also restarts, so
    // lifting the pause does not leave the promo instantly expirable. Without the restart
    // this call would succeed, charging the borrower for the protocol's own downtime one
    // moment later than before.
    send(&mut env.svm, &[set_market_paused_ix(&admin, &setup.cngn, false)], &[&env.admin]).unwrap();
    let right_after = expire_promo_ix(&setup.cngn, &owner);
    assert_hodl_error(send(&mut env.svm, &[right_after], &[&stranger]), HodlError::PromoNotExpired);
    assert_eq!(env.position(&owner).promo_balance, GRANT);

    // The borrower gets a full fresh window, and no more than one.
    env.warp_seconds(INACTIVITY - 1);
    let one_short = expire_promo_ix(&setup.cngn, &owner);
    assert_hodl_error(send(&mut env.svm, &[one_short], &[&stranger]), HodlError::PromoNotExpired);
    env.warp_seconds(1);
    send(&mut env.svm, &[expire_promo_ix(&setup.cngn, &owner)], &[&stranger]).unwrap();
    assert_eq!(env.position(&owner).promo_balance, 0);
}

#[test]
fn revoke_stays_open_during_a_pause_because_it_measures_nothing() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();
    let admin = env.admin.pubkey();
    let guardian = env.guardian.pubkey();

    send(&mut env.svm, &[set_market_paused_ix(&guardian, &setup.cngn, true)], &[&env.guardian]).unwrap();
    // Unlike expiry, revocation asserts nothing about how long the borrower has been idle —
    // it is the admin retracting a grant — so a pause does not invalidate it.
    send(&mut env.svm, &[revoke_promo_ix(&admin, &setup.cngn, &owner)], &[&env.admin]).unwrap();
    assert_eq!(env.position(&owner).promo_balance, 0);
    assert_eq!(env.promo_vault(&setup.cngn).outstanding, 0);
}
```

And this one pins a behaviour that is **left as is deliberately** — the clock is market-global, so every unpause resets it for every position:

```rust
#[test]
fn two_pause_cycles_each_restart_the_clock_for_every_position() {
    // The clock is market-global, so every unpause resets it for every position. Two
    // unrelated incidents inside one inactivity window therefore mean nothing expires at all.
    // Pinned rather than fixed: it keeps the protocol's own promo budget committed — the
    // admin's downtime, the admin's cost — and the alternative is per-position accounting of
    // paused time. This test is what makes that a decision instead of a surprise.
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();
    let admin = env.admin.pubkey();
    let guardian = env.guardian.pubkey();
    let stranger = env.funded_keypair();

    for _ in 0..2 {
        send(&mut env.svm, &[set_market_paused_ix(&guardian, &setup.cngn, true)], &[&env.guardian]).unwrap();
        env.warp_seconds(INACTIVITY - DAY);
        send(&mut env.svm, &[set_market_paused_ix(&admin, &setup.cngn, false)], &[&env.admin]).unwrap();
        env.warp_seconds(DAY);
    }

    // Well over two inactivity windows have passed in wall-clock terms, and the promo is
    // still not expirable: each unpause moved the deadline out by a full window.
    let after = expire_promo_ix(&setup.cngn, &owner);
    assert_hodl_error(send(&mut env.svm, &[after], &[&stranger]), HodlError::PromoNotExpired);
    assert_eq!(env.position(&owner).promo_balance, GRANT);

    // Left alone for one uninterrupted window, it expires as normal.
    env.warp_seconds(INACTIVITY);
    send(&mut env.svm, &[expire_promo_ix(&setup.cngn, &owner)], &[&stranger]).unwrap();
    assert_eq!(env.position(&owner).promo_balance, 0);
}
```

- [ ] **Step 2: Run them and watch the first fail**

Run: `./scripts/test.sh 2>&1 | grep -E "pause_stops|revoke_stays|two_pause|error\["`
Expected: the first FAILs on `MarketPaused` not being returned. The other two pass already — they describe behaviour this task preserves, and they are here so a later change cannot alter it silently.

- [ ] **Step 3: Add the field, out of the padding**

In `programs/hodl_loans/src/state/market.rs`, before `reserved`:

```rust
    /// When the promo inactivity clock last restarted, because the market was unpaused.
    /// The clock is meant to measure a *borrower's* inactivity, and a paused market is one
    /// the borrower cannot act on, so time spent paused must not count towards expiry.
    /// Expiry therefore runs from `max(promo_last_activity_at, promo_clock_resumed_at)`.
    /// Taken from the reserved padding, so the account size is unchanged.
    pub promo_clock_resumed_at: i64,
    pub reserved: [u8; 232],
```

`handle_create_market`'s initializer takes `promo_clock_resumed_at: 0` and `reserved: [0; 232]`.

- [ ] **Step 4: Restart the clock on unpause**

In `programs/hodl_loans/src/instructions/admin/market_admin.rs`, in `handle_set_market_paused` before the event:

```rust
    // Unpausing restarts the promo inactivity clock. Gating `expire_promo` on the pause alone
    // would only defer the harvest: a pause outlasting `promo_inactivity_seconds` would leave
    // every idle promo expirable the instant it lifted, which is the same charge for the
    // protocol's own downtime, collected a moment later. Restarting the clock gives every
    // borrower a full window to act once they can act again. Only on the true→false edge —
    // though note that guards little, since setting `paused = true` on an already-paused
    // market was never going to reach this line anyway.
    //
    // What it does not bound: the clock is market-global and every genuine pause→unpause
    // cycle resets it for every position. Two unrelated incidents inside one
    // `promo_inactivity_seconds` window mean nothing on the market ever expires, so
    // `outstanding` stays high and `free()` stays low until the admin intervenes. That is
    // the protocol's own promo budget staying committed — self-harm, not a user-facing
    // loss — and the alternative, accumulating paused time per position, costs state on
    // every position to protect against the admin's own downtime. Left as is deliberately;
    // `two_pause_cycles_each_restart_the_clock_for_every_position` pins the behaviour so it
    // stays a decision rather than a surprise.
    if old_paused && !paused {
        ctx.accounts.market.promo_clock_resumed_at = Clock::get()?.unix_timestamp;
```

Read the comment's second half carefully: the true→false edge guard is weak on its own, and what the clock genuinely does not bound is a repeated pause cycle. That is the admin's own promo budget staying committed, and the alternative costs per-position state, so it is left as is — which is what Step 1's third test pins.

- [ ] **Step 5: Gate expiry and floor its clock**

In `programs/hodl_loans/src/instructions/promos/lifecycle.rs`:

```rust
/// Spec §12. Anyone may reclaim promo a borrower has left idle, which is what stops granted
/// promo sitting on the books forever. The clock runs from the last redemption, the last loan
/// taken, or the moment the last loan closed.
pub fn handle_expire_promo(ctx: Context<ExpirePromo>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let market_key = ctx.accounts.market.key();
    let inactivity = ctx.accounts.market.promo_inactivity_seconds;
    // Expiry is the one promo operation barred by a pause, which looks backwards next to the
    // rule that pauses stop exposure-*increasing* work and leave the rest open. It is not an
    // exception to that rule but to a different one: this instruction's precondition is a
    // measurement of how long the borrower has been inactive, and a paused market is one the
    // borrower cannot act on. The measurement is invalid during a pause, not the operation
    // unsafe. `revoke_promo` stays open — it makes no such measurement.
    require!(!ctx.accounts.market.paused, HodlError::MarketPaused);
    let resumed_at = ctx.accounts.market.promo_clock_resumed_at;
    let mut position = ctx.accounts.position.load_mut()?;
    // The clock runs from the borrower's last activity or the market's last unpause,
    // whichever is later: see `Market::promo_clock_resumed_at`.
    // `saturating_add` can only push the deadline later (never wrap it earlier), so this fails
    // safe on overflow — it depends on `MarketParams::validate` requiring `inactivity > 0`, so
    // the two must not drift apart.
    let since = position.promo_last_activity_at.max(resumed_at);
    require!(now >= since.saturating_add(inactivity), HodlError::PromoNotExpired);
    let amount = release_from_idle_position(&mut position, &mut ctx.accounts.promo_vault)?;
    emit!(PromoExpired {
        market: market_key,
        position: ctx.accounts.position.key(),
        owner: position.owner,
        amount,
    });
    Ok(())
}
```

- [ ] **Step 6: Run everything**

Run: `./scripts/test.sh`
Expected: all pass, `Market::INIT_SPACE` still 555. Confirm both halves separately: remove the pause gate and the first test fails; keep the gate but remove the clock restart and it fails again, on the "right after unpausing" assertion. The second mutation is the one that proves the gate alone would not have been enough.

- [ ] **Step 7: Update the spec and commit**

Spec §12's `expire_promo` bullet gains the pause requirement, the `max(promo_last_activity_at, promo_clock_resumed_at)` deadline, why this is the one promo operation a pause blocks, and why barring it during the pause is not sufficient alone.

```bash
git add -A
git commit -m "feat(plan7): a market pause stops the promo inactivity clock

A pause longer than promo_inactivity_seconds let anyone expire every idle
promo balance the moment it lifted — charging borrowers for downtime the
protocol imposed.

Two halves. expire_promo is barred while paused; that alone only defers the
harvest, so unpausing also restarts the clock via
Market::promo_clock_resumed_at, taken from the reserved padding so INIT_SPACE
stays 555. Expiry runs from whichever of that and the borrower's own last
activity is later. A mutation test pins both halves separately.

revoke_promo stays open: expiry's precondition is a measurement of borrower
inactivity that a pause invalidates, and revocation measures nothing.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: `reconcile_promo_vault`

cNGN carries a `PermanentDelegate`, so its issuer can move tokens out of the promo vault without this program's involvement. `forfeit_promo` already clamps its transfer to what remains, so liquidation survives — but nothing afterwards reconciled the ledger. `cash` keeps the clawed-back amount, `free() = cash − outstanding − unissued` reads high, and `withdraw_promo_vault` passes our own bound only to fail inside the token program.

Solvency was never at risk: `outstanding` is what backs positions and it stays exact. This is a liveness repair on an admin path.

**Files:**
- Modify: `programs/hodl_loans/src/instructions/promos/vault.rs`, `src/events.rs`, `src/lib.rs`
- Modify: `docs/superpowers/specs/2026-09-17-solana-fixed-loans-design.md` (§12)
- Test: `programs/hodl_loans/tests/promo_vault.rs`, `tests/common/mod.rs`

**Interfaces:**
- Produces: `reconcile_promo_vault()`, event `PromoVaultReconciled`, harness `reconcile_promo_vault_ix(admin, mint)`.

- [ ] **Step 1: Write the failing tests**

Harness helper first, in `tests/common/mod.rs`:

```rust
pub fn reconcile_promo_vault_ix(admin: &Pubkey, mint: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::ReconcilePromoVault {},
        hodl_loans::accounts::ReconcilePromoVault {
            admin: *admin,
            config: config_pda(),
            market: market_pda(mint),
            mint: *mint,
            promo_vault: promo_vault_pda(mint),
            vault: promo_vault_token_pda(mint),
            token_program: TOKEN_2022,
        },
    )
}
```

Then three tests in `programs/hodl_loans/tests/promo_vault.rs`. The first is the repair:

```rust
#[test]
fn reconciling_writes_cash_down_to_the_balance_an_issuer_clawback_left() {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();
    let token = promo_vault_token_pda(&cngn);
    let destination = env.treasury_token(&cngn);

    // The issuer pulls a quarter of the vault out directly. Nothing in the program observed
    // it, so `cash` still claims the full funding and `free()` reads high with it.
    let clawed = FUNDING / 4;
    env.delegate_burn(&cngn, &token, clawed);
    assert_eq!(env.token_balance(&token), FUNDING - clawed);
    assert_eq!(env.promo_vault(&cngn).cash, FUNDING);
    assert_eq!(env.promo_vault(&cngn).free().unwrap(), FUNDING);

    // Withdrawing what `free()` promises passes our own bound and then fails inside the token
    // program — the wrong place to learn the vault is short.
    let overdraw = withdraw_promo_vault_ix(&admin, &cngn, &destination, FUNDING);
    assert!(send(&mut env.svm, &[overdraw], &[&env.admin]).is_err());

    send(&mut env.svm, &[reconcile_promo_vault_ix(&admin, &cngn)], &[&env.admin]).unwrap();
    let vault = env.promo_vault(&cngn);
    assert_eq!(vault.cash, FUNDING - clawed);
    assert_eq!(vault.free().unwrap(), FUNDING - clawed);

    // And now the books and the tokens agree, so withdrawing everything works.
    let withdraw = withdraw_promo_vault_ix(&admin, &cngn, &destination, FUNDING - clawed);
    send(&mut env.svm, &[withdraw], &[&env.admin]).unwrap();
    assert_eq!(env.token_balance(&destination), FUNDING - clawed);
    assert_eq!(env.promo_vault(&cngn).cash, 0);
}
```

The second pins the direction — upward moves belong to `fund_promo_vault`, which does the accounting:

```rust
#[test]
fn reconciling_only_ever_writes_cash_down() {
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();

    // Books and balance already agree: there is nothing to reconcile, and the call says so
    // rather than silently doing nothing.
    let noop = reconcile_promo_vault_ix(&admin, &cngn);
    assert_hodl_error(send(&mut env.svm, &[noop], &[&env.admin]), HodlError::AmountTooSmall);

    // A donation straight into the vault's token account leaves the balance *above* `cash`.
    // Reconciling must not book it as protocol funds — that is what `fund_promo_vault` is
    // for, and it is the path that does the accounting.
    env.mint_to(&cngn, &promo_vault_token_pda(&cngn), 1_000 * ONE_CNGN);
    assert_eq!(env.token_balance(&promo_vault_token_pda(&cngn)), FUNDING + 1_000 * ONE_CNGN);
    let up = reconcile_promo_vault_ix(&admin, &cngn);
    assert_hodl_error(send(&mut env.svm, &[up], &[&env.admin]), HodlError::AmountTooSmall);
    assert_eq!(env.promo_vault(&cngn).cash, FUNDING);

    // A stranger cannot call it.
    let stranger = env.funded_keypair();
    let by_stranger = reconcile_promo_vault_ix(&stranger.pubkey(), &cngn);
    assert_hodl_error(send(&mut env.svm, &[by_stranger], &[&stranger]), HodlError::Unauthorized);
}
```

The third is the one a first draft left out, and a mutation showed the whole suite passed without the invariant check:

```rust
#[test]
fn reconciling_refuses_a_clawback_that_has_already_eaten_into_outstanding_promo() {
    let (mut env, setup) = Env::promo_ready();
    let admin = env.admin.pubkey();
    let cngn = setup.cngn;
    let grant = 50_000 * ONE_CNGN;
    env.redeem_promo(&setup.borrower, &cngn, 1, grant, 7).unwrap();
    assert_eq!(env.promo_vault(&cngn).outstanding, grant);

    // The issuer takes the vault down below what positions are already holding.
    let token = promo_vault_token_pda(&cngn);
    let leave = grant / 2;
    env.delegate_burn(&cngn, &token, env.token_balance(&token) - leave);
    assert_eq!(env.token_balance(&token), leave);

    // Writing `cash` down to the real balance would leave `outstanding > cash`, and no single
    // field rewrite can honestly repair that: `outstanding` may only fall through expiry,
    // revocation or liquidation, each of which settles a named position. So the call fails
    // rather than recording a vault that claims to back more than it holds.
    let reconcile = reconcile_promo_vault_ix(&admin, &cngn);
    assert_hodl_error(send(&mut env.svm, &[reconcile], &[&env.admin]), HodlError::PromoVaultInsufficient);
    assert_eq!(env.promo_vault(&cngn).cash, 10_000_000 * ONE_CNGN);

    // Once nothing is committed against the vault any more — the position's promo revoked and
    // the campaign's unissued budget returned — the same call goes through, so the refusal
    // above is the invariant talking and not a blanket block.
    send(&mut env.svm, &[revoke_promo_ix(&admin, &cngn, &setup.borrower.pubkey())], &[&env.admin]).unwrap();
    send(&mut env.svm, &[close_campaign_ix(&admin, &cngn, 1)], &[&env.admin]).unwrap();
    let vault = env.promo_vault(&cngn);
    assert_eq!((vault.outstanding, vault.unissued), (0, 0));
    send(&mut env.svm, &[reconcile_promo_vault_ix(&admin, &cngn)], &[&env.admin]).unwrap();
    assert_eq!(env.promo_vault(&cngn).cash, leave);
}
```

And this one pins a consequence that is easy to miss — reconciling is not undone by the issuer giving the tokens back:

```rust
#[test]
fn a_clawback_then_return_moves_promo_budget_to_the_treasury_once_reconciled() {
    // Reconciling is not undone by the issuer giving the tokens back. `cash` has already been
    // written down, so the returned tokens read as unaccounted balance — which is exactly
    // what `sweep_promo_excess` sends to the treasury. Documented on `reconcile_promo_vault`
    // and pinned here, because an admin reconciling a clawback they expect to be reversed
    // should wait instead.
    let (mut env, cngn) = Env::with_promo_vault(FUNDING);
    let admin = env.admin.pubkey();
    let token = promo_vault_token_pda(&cngn);
    let treasury = env.treasury_token(&cngn);
    let clawed = FUNDING / 4;

    env.delegate_burn(&cngn, &token, clawed);
    send(&mut env.svm, &[reconcile_promo_vault_ix(&admin, &cngn)], &[&env.admin]).unwrap();
    assert_eq!(env.promo_vault(&cngn).cash, FUNDING - clawed);

    // The issuer returns what it took.
    env.mint_to(&cngn, &token, clawed);
    assert_eq!(env.token_balance(&token), FUNDING);
    // `cash` does not follow it back up — reconcile only ever writes down.
    assert_eq!(env.promo_vault(&cngn).cash, FUNDING - clawed);

    // So the sweep treats the returned tokens as a donation.
    let sweep = sweep_promo_excess_ix(&admin, &cngn, &treasury);
    send(&mut env.svm, &[sweep], &[&env.admin]).unwrap();
    assert_eq!(env.token_balance(&treasury), clawed);
    assert_eq!(env.promo_vault(&cngn).cash, FUNDING - clawed);
    // Only `fund_promo_vault` puts it back into the promo budget.
}
```

- [ ] **Step 2: Run them and watch them fail to compile**

Run: `./scripts/test.sh 2>&1 | grep -E "error\[|ReconcilePromoVault"`
Expected: `error[E0433]` — `ReconcilePromoVault` does not exist yet.

- [ ] **Step 3: Add the event**

Appended to `programs/hodl_loans/src/events.rs`:

```rust
#[event]
pub struct PromoVaultReconciled {
    pub market: Pubkey,
    pub old_cash: u64,
    pub cash: u64,
    pub by: Pubkey,
}
```

- [ ] **Step 4: Add the instruction**

In `programs/hodl_loans/src/instructions/promos/vault.rs`. Read the doc block before writing the body — it carries the direction rule and the sweep interaction:

```rust
/// Writes `promo_vault.cash` down to the token account's real balance.
///
/// cNGN carries a `PermanentDelegate`, so its issuer can move tokens out of the promo vault
/// without this program's involvement. Liquidation already survives that — `forfeit_promo`
/// clamps its transfer to what the vault actually holds — but nothing afterwards reconciles
/// the ledger, so `cash` keeps the clawed-back amount and `free() = cash − outstanding −
/// unissued` reads high. `withdraw_promo_vault` then passes our own bound and fails inside the
/// token program instead, which is a confusing place to learn the vault is short.
///
/// Solvency never depended on `cash`: `outstanding` is what backs positions and it stays
/// exact. This is a liveness repair on an admin path.
///
/// Only ever downward. Raising `cash` to match a larger balance is what `fund_promo_vault`
/// is for, and allowing it here would let a donation be booked as protocol funds without
/// passing through the funding path's accounting.
///
/// One consequence worth stating, because it is not obvious: reconciling is not reversible
/// by the issuer returning the tokens. Once `cash` has been written down, a later return
/// leaves the token balance above `cash`, which is precisely what `sweep_promo_excess` sends
/// to the treasury. A clawback-then-return therefore moves that amount from the promo budget
/// to the treasury permanently, recoverable only through `fund_promo_vault`. That follows
/// from treating unaccounted balance as a donation, which is the sweep's whole premise — but
/// an admin reconciling a clawback they expect to be reversed should wait instead.
#[derive(Accounts)]
pub struct ReconcilePromoVault<'info> {
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
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
}

pub fn handle_reconcile_promo_vault(ctx: Context<ReconcilePromoVault>) -> Result<()> {
    let balance = ctx.accounts.vault.amount;
    let old_cash = ctx.accounts.promo_vault.cash;
    require!(balance < old_cash, HodlError::AmountTooSmall);
    ctx.accounts.promo_vault.cash = balance;
    // The invariant can fail here, and that is the honest outcome: a clawback deep enough to
    // take the vault below what positions are already holding cannot be papered over by
    // rewriting one field. `outstanding` would have to fall too, and only expiry, revocation
    // or liquidation may move it — each of which settles a specific position.
    ctx.accounts.promo_vault.require_invariant()?;
    emit!(PromoVaultReconciled {
        market: ctx.accounts.market.key(),
        old_cash,
        cash: balance,
        by: ctx.accounts.admin.key(),
    });
    Ok(())
}
```

And the entry point in `programs/hodl_loans/src/lib.rs`:

```rust
    pub fn reconcile_promo_vault(ctx: Context<ReconcilePromoVault>) -> Result<()> {
        instructions::handle_reconcile_promo_vault(ctx)
    }
```

- [ ] **Step 5: Run everything**

Run: `./scripts/test.sh`
Expected: all pass. Check the invariant assertion is load-bearing — delete `require_invariant()?` and the third test must fail. If the whole suite still passes without it, the third test is not doing its job.

- [ ] **Step 6: Update the spec and commit**

Spec §12 gains a `reconcile_promo_vault` bullet: downward only, why `fund_promo_vault` owns the upward direction, and that the §12 invariant is still asserted afterwards.

```bash
git add -A
git commit -m "feat(plan7): reconcile_promo_vault

After an issuer clawback, promo_vault.cash kept the phantom amount, so free()
read high and withdraw_promo_vault failed inside the token program rather than
at our own bound. Solvency was never at risk — outstanding backs positions and
stays exact.

Downward only: raising cash to meet a larger balance is fund_promo_vault's job,
and allowing it here would book a donation as protocol funds without the
funding path's accounting. The §12 invariant is still asserted afterwards, so a
clawback deep enough to take the vault below outstanding is refused rather than
recorded — no single field rewrite can honestly repair that.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: Revoke promo under a live loan when health still passes

`revoke_promo` refused any position with an active loan. The purpose was sound — revocation must not push a borrower into liquidation — but it handed the borrower the lever: one minimum-size loan, held open, made promo permanently unrevokable, defeating the instruction in exactly the case it exists for.

The position is now priced **after** the promo is taken away and must still be healthy; the whole transaction reverts otherwise.

**Be honest in the comment about what changed class.** The old rule was unconditional — no parameter could bypass it. `is_healthy()` is not: the same admin signing this also sets `ltv_bps` and `liquidation_threshold_bps` through `update_collateral_params` with no in-program timelock, so three instructions in one transaction can raise the LTV, revoke, and restore it. Spec §18's timelocked multisig is what stands there, and it is an operational control rather than an on-chain one.

**Files:**
- Modify: `programs/hodl_loans/src/instructions/promos/lifecycle.rs`, `src/lib.rs`
- Modify: `docs/superpowers/specs/2026-09-17-solana-fixed-loans-design.md` (§12)
- Test: `programs/hodl_loans/tests/promo_lifecycle.rs`, `tests/common/mod.rs`

**Interfaces:**
- Consumes: `valuation::{load_health, ValuationRequest}`.
- Produces: harness `revoke_promo_priced_ix(admin, mint, owner, with_feed, prices)`.

- [ ] **Step 1: Replace the harness helper**

`revoke_promo_ix` keeps its signature for the idle case and delegates. In `tests/common/mod.rs`:

```rust
/// Revocation of an idle position: no prices needed, so `ngn_feed` is omitted.
pub fn revoke_promo_ix(admin: &Pubkey, mint: &Pubkey, owner: &Pubkey) -> Instruction {
    revoke_promo_priced_ix(admin, mint, owner, false, vec![])
}

/// Revocation with the accounts a live loan's health check needs. `with_feed` is separate from
/// `prices` so a test can supply one and withhold the other.
pub fn revoke_promo_priced_ix(
    admin: &Pubkey,
    mint: &Pubkey,
    owner: &Pubkey,
    with_feed: bool,
    prices: Vec<AccountMeta>,
) -> Instruction {
    let mut instruction = ix(
        hodl_loans::instruction::RevokePromo {},
        hodl_loans::accounts::RevokePromo {
            admin: *admin,
            config: config_pda(),
            market: market_pda(mint),
            promo_vault: promo_vault_pda(mint),
            position: position_pda(owner),
            ngn_feed: with_feed.then(ngn_feed),
        },
    );
    instruction.accounts.extend(prices);
    instruction
}
```

- [ ] **Step 2: Rewrite the existing test's last leg, and add the refusal case**

`an_admin_can_revoke_promo_without_waiting` currently asserts `PositionNotEmpty` for a live loan — the rule this task removes. Replace that leg so it asserts the new behaviour: bare and feed-less calls get `PriceAccountMismatch`, and a priced call against a position that does not need the promo succeeds.

Then add the case that matters most:

```rust
#[test]
fn revoking_under_a_live_loan_is_refused_when_the_promo_is_holding_the_position_up() {
    let (mut env, setup) = Env::promo_ready();
    let borrower = &setup.borrower;
    let owner = borrower.pubkey();
    let admin = env.admin.pubkey();
    env.redeem_promo(borrower, &setup.cngn, 1, GRANT, 7).unwrap();

    // Borrow past what the collateral alone supports: `OWN_CEILING` is the limit without the
    // promo, and the promo lifts it to `WITH_PROMO_CEILING`. Anything above the first is debt
    // the promo is carrying.
    env.take_loan(borrower, &setup, OWN_CEILING + ONE_CNGN, 365 * DAY).unwrap();

    // Taking the promo away would put the position under its own borrow limit, so revocation
    // is refused — the guarantee the old `!has_active_loans()` rule was reaching for, now
    // stated as the thing it actually protects rather than as a blanket ban.
    let priced = revoke_promo_priced_ix(&admin, &setup.cngn, &owner, true, price_pairs(&[setup.usdc]));
    assert_hodl_error(send(&mut env.svm, &[priced], &[&env.admin]), HodlError::Unhealthy);
    assert_eq!(env.position(&owner).promo_balance, GRANT);
    assert_eq!(env.promo_vault(&setup.cngn).outstanding, GRANT);

    // Repaying back under the unaided ceiling makes the same call succeed: the borrower can
    // no longer be hurt by it.
    env.repay(&setup, 0, 2 * ONE_CNGN).unwrap();
    let priced = revoke_promo_priced_ix(&admin, &setup.cngn, &owner, true, price_pairs(&[setup.usdc]));
    send(&mut env.svm, &[priced], &[&env.admin]).unwrap();
    assert_eq!(env.position(&owner).promo_balance, 0);
}
```

- [ ] **Step 3: Run and watch them fail**

Run: `./scripts/test.sh 2>&1 | grep -E "revoke|error\["`
Expected: compile failure first — `RevokePromo` has no `ngn_feed` field yet.

- [ ] **Step 4: Add the optional feed and the health check**

In `programs/hodl_loans/src/instructions/promos/lifecycle.rs`, `RevokePromo` gains:

```rust
    /// CHECK: required only when the position has active loans, to price the health check
    /// below; `read_ngn_price` pins it to `market.ngn_feed`.
    pub ngn_feed: Option<UncheckedAccount<'info>>,
```

and the handler becomes:

```rust
/// Spec §12. The same release without waiting out the clock — for promo granted in error or to
/// an account the backend has since judged ineligible.
///
/// A live loan no longer blocks it outright. The old rule — revoke only an idle position —
/// was sound in its purpose (revocation must not push anyone into liquidation) but handed the
/// borrower the wrong lever: one minimum-size loan, held open, made promo permanently
/// unrevokable, which defeats the instruction in exactly the case it exists for. Instead the
/// position is checked for health **after** the promo is taken away, and the whole
/// transaction reverts if it would not survive.
///
/// Note what changed class. The old rule was unconditional: no parameter could bypass it.
/// `is_healthy()` is not — the same admin signing this also sets `ltv_bps`,
/// `liquidation_threshold_bps` and `max_conf_bps` through `update_collateral_params`, with no
/// in-program timelock, so three instructions in one transaction can raise the LTV, revoke,
/// and restore it, leaving the borrower promo-less and liquidatable. What stands between that
/// and a borrower is spec §18's timelocked multisig, which is an operational control rather
/// than an on-chain one. The trade is deliberate: the unconditional rule let any borrower make
/// promo permanently unrevokable by holding one minimum-size loan open, which defeats the
/// instruction in exactly the case it exists for.
///
/// With active loans, `ngn_feed` is required and `remaining_accounts` must hold, per used
/// collateral slot in slot order, a `(CollateralAsset, PriceUpdateV2)` pair — an `XStock` slot
/// needs its mint too, as a third account.
pub fn handle_revoke_promo<'info>(ctx: Context<'info, RevokePromo<'info>>) -> Result<()> {
    let market_key = ctx.accounts.market.key();
    let mut position = ctx.accounts.position.load_mut()?;
    require!(position.promo_balance > 0, HodlError::AmountTooSmall);
    let has_loans = position.has_active_loans();
    let amount = release_promo(&mut position, &mut ctx.accounts.promo_vault)?;
    if has_loans {
        // Health is measured on the position as it stands *after* the release, so what is
        // being asked is exactly "can this borrower stand without the promo?". A failure
        // reverts the release along with everything else in the transaction.
        let Some(ngn_feed) = &ctx.accounts.ngn_feed else {
            return err!(HodlError::PriceAccountMismatch);
        };
        let ngn_feed = ngn_feed.to_account_info();
        let clock = Clock::get()?;
        let health = load_health(
            &position,
            &ValuationRequest {
                program_id: ctx.program_id,
                market: &ctx.accounts.market,
                ngn_feed: &ngn_feed,
                remaining: ctx.remaining_accounts,
                extra_debt: 0,
                promo_cap_bps: ctx.accounts.config.promo_cap_bps,
                clock: &clock,
            },
        )?;
        require!(health.is_healthy(), HodlError::Unhealthy);
    }
    emit!(PromoRevoked {
        market: market_key,
        position: ctx.accounts.position.key(),
        owner: position.owner,
        amount,
    });
    Ok(())
}
```

`release_from_idle_position` is now `expire_promo`'s alone — update its doc comment to say so. `lifecycle.rs` gains `use crate::valuation::{load_health, ValuationRequest};`.

- [ ] **Step 5: Fix the lifetime on the entry point**

`handle_revoke_promo` now reads `ctx.remaining_accounts`, so `lib.rs` needs the explicit lifetime the borrow checker asks for:

```rust
    pub fn revoke_promo<'info>(ctx: Context<'info, RevokePromo<'info>>) -> Result<()> {
        instructions::handle_revoke_promo(ctx)
    }
```

- [ ] **Step 6: Run everything**

Run: `./scripts/test.sh`
Expected: all pass, 264 tests. Confirm the health check is load-bearing: delete `require!(health.is_healthy(), ...)`, rebuild, and the refusal test must fail.

Worth checking by reading rather than testing: a wrong-market position is still blocked, transitively — `promo_vault` is seed-derived from `market` with `has_one = market`, and `release_promo` asserts `position.market == promo_vault.market`.

- [ ] **Step 7: Update the spec and commit**

Spec §12's `revoke_promo` bullet drops "requires no active loans" and gains the post-release health check, the account requirements, and a paragraph on why the old rule was replaced.

```bash
git add -A
git commit -m "feat(plan7): revoke promo under a live loan when health still passes

The old rule refused any position with an active loan. Its purpose was right —
revocation must not push a borrower into liquidation — but one minimum-size
loan held open made promo permanently unrevokable, defeating the instruction in
exactly the case it exists for.

The position is priced after the promo is taken away and must still be healthy.
Note what changed class: the old rule was unconditional, while is_healthy()
depends on parameters the same admin sets through update_collateral_params with
no in-program timelock. Spec §18's timelocked multisig is what stands there,
and it is operational rather than on-chain. The trade is deliberate.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## After the last task

Run `./scripts/test.sh` and `cargo clippy -p hodl_loans --all-targets -- -D warnings` on the whole branch. Expect **264 tests** (59 unit, 205 LiteSVM), clippy clean.

Then write `docs/superpowers/plans/2026-09-21-plan-7-followups.md`, recording what this plan deliberately left standing — the items Plan 8 inherits, and the decisions here that will look like defects to someone meeting them cold:

- **`expire_promo` is the one promo operation a pause blocks.** It reads backwards against the exposure-increasing rule and is not an exception to it.
- **The promo clock is market-global**, so repeated pause cycles keep resetting it. Left as is; `two_pause_cycles_each_restart_the_clock_for_every_position` pins it.
- **`revoke_promo`'s guarantee is now conditional on admin-set parameters**, backed by spec §18's timelock rather than by the program.
- **The voucher-expiry bound rejects previously valid vouchers** and needs a backend migration.
- **Reconciling a clawback is not reversed by the issuer returning the tokens** — the return reads as a donation and sweeps to the treasury.
- **The per-slot PDA re-derivation costs ~1,600 CU**, and the budget markers were raised to accommodate it.
