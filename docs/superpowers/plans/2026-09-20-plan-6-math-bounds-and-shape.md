# Plan 6: Math, Bounds and Shape Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the mechanical safety items the Plan 2–5 reviews deferred — one power-of-ten helper, checked arithmetic everywhere, parameter bounds the liquidation path actually needs, layout guards on the two accounts that grew into their padding, and named inputs where positional ones could be transposed.

**Architecture:** Nothing here changes what the protocol does. Every task either removes a second copy of something, converts a guarded raw operator to a checked one, refuses a configuration that would break a later instruction, or pins a property a test could not previously catch the removal of. Two tasks do change behaviour, both strictly conservative: oracle uncertainty now rounds up instead of down, and a collateral mint wider than `MAX_COLLATERAL_DECIMALS` can no longer be listed.

**Tech Stack:** Anchor 1.2.0, anchor-spl `token_interface`, LiteSVM 0.10.0 with the `precompiles` feature, `cargo build-sbf --tools-version v1.52`.

**Spec:** `docs/superpowers/specs/2026-09-17-solana-fixed-loans-design.md`

**Sources:** `docs/superpowers/plans/2026-09-17-plan-{2,3,4,5}-followups.md`. This plan takes the "math, bounds and shape" group; trust boundaries and admin authority are Plan 7, coverage and devnet Plan 8.

## Global Constraints

- Anchor 1.2.0. Build and test with `./scripts/test.sh`, which rebuilds the SBF program first. **Plain `cargo test` reuses a stale `.so`** and will pass against code you have just changed.
- All arithmetic is checked: no raw `+ - *` on values that could overflow, no `unwrap()` on arithmetic, no bare `as` narrowing casts. `math::checked` has `add`, `sub`, `mul_div_floor`, `mul_div_ceil`, `to_u64` and, after Task 1, `pow10`.
- SBF stack frames are 4 KB. Box accounts in large instruction contexts, `Config` included.
- `cargo clippy -p hodl_loans --all-targets -- -D warnings` clean. Do not silence a lint with `#[allow]`; restructure instead.
- New `HodlError` variants are **appended**, never inserted — codes are `6000 + position`.
- Every commit message ends with:
  `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`
- The suite is **234 tests** at the start of this plan and **240** at the end.

---

### Task 1: One `pow10`, and `lp_interest` stops shipping

Two copies of the same helper, with different parameter types, is how the same overflow bound came to be reasoned about in two places at once — which is exactly what Task 4 has to reason about. And `lp_interest` is compiled into the on-chain program while having no caller outside the test module.

**Files:**
- Modify: `programs/hodl_loans/src/math/checked.rs`, `src/math/price.rs`, `src/math/liquidation.rs`, `src/math/interest.rs`

**Interfaces:**
- Produces: `math::checked::pow10(exp: u32) -> Result<u128>` — the only power-of-ten helper in the program. Tasks 4 and 6 depend on it existing.

- [ ] **Step 1: Add the shared helper**

In `programs/hodl_loans/src/math/checked.rs`, above `to_u64`:

```rust
/// `10^exp`. The only power-of-ten helper in the program: `math/price.rs` and
/// `math/liquidation.rs` each had their own, with different parameter types (`u32` and `u8`),
/// which is how the same bound came to be reasoned about twice. Callers holding a `u8` widen.
pub fn pow10(exp: u32) -> Result<u128> {
    Ok(10u128.checked_pow(exp).ok_or(HodlError::MathOverflow)?)
}
```

- [ ] **Step 2: Point `price.rs` at it and delete its copy**

In `programs/hodl_loans/src/math/price.rs`, change the import:

```rust
use crate::math::checked::{add, mul_div_ceil, mul_div_floor, pow10};
```

Then delete the local `fn pow10(exp: u32)` entirely. Its three call sites in this file already pass a `u32` and need no change.

- [ ] **Step 3: Point `liquidation.rs` at it and delete its copy**

In `programs/hodl_loans/src/math/liquidation.rs`, change the import:

```rust
use crate::math::checked::{add, mul_div_floor, pow10, to_u64};
```

Delete the local `fn pow10(exponent: u8)`. Its two call sites took a `u8`, so widen them:

```rust
    let repaid_usd = mul_div_floor(repay_amount as u128, ngn_price, pow10(cngn_decimals as u32)?)?;
```

```rust
    let display = mul_div_floor(with_bonus, pow10(collateral_decimals as u32)?, collateral_price)?;
```

- [ ] **Step 4: Stop shipping `lp_interest`**

`Market::accrue` uses `accrue_lp_interest`, which carries the division remainder between calls. `lp_interest` is the closed-form version the tests check that one against, and nothing else calls it. In `programs/hodl_loans/src/math/interest.rs`:

```rust
/// Lender interest accrued over `elapsed_seconds` for the market's `lp_rate_product`
/// (Σ principal × rate_bps × (BPS − reserve_factor_bps)). Rounds down.
///
/// Test-only. `Market::accrue` uses `accrue_lp_interest`, which carries the division remainder
/// between calls; this is the closed-form version the tests check that one against. Keeping it
/// compiled into the program would ship a second, subtly different interest formula that
/// nothing calls.
#[cfg(test)]
pub fn lp_interest(lp_rate_product: u128, elapsed_seconds: u64) -> Result<u128> {
    mul_div_floor(lp_rate_product, elapsed_seconds as u128, BPS * BPS * YEAR)
}
```

Its only remaining user is the test module, so `mul_div_floor` becomes a test-only import. Replace the import line with:

```rust
use crate::math::checked::add;
#[cfg(test)]
use crate::math::checked::mul_div_floor;
```

- [ ] **Step 5: Run the suite**

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, **234 tests in all** — unchanged. This task removes duplication; if the count moves, something else changed.

Run: `cargo clippy -p hodl_loans --all-targets -- -D warnings`
Expected: no warnings. In particular no `unused import`, which is how you will know the `#[cfg(test)]` split in Step 4 is right.

- [ ] **Step 6: Commit**

```bash
git add programs/hodl_loans/src/math/
git commit -m "refactor: one pow10, and lp_interest stops shipping on chain"
```

---

### Task 2: The oracle exponent, and uncertainty that rounds the right way

Two defects in the same twelve lines. `rescale` computes its shift with a raw `+` on a value read from the oracle account, so an absurd exponent aborts the transaction instead of returning an error. And it rounds the uncertainty **down**, which is anti-conservative on both sides at once — easy to miss, because the two uses pull in opposite directions.

**Files:**
- Modify: `programs/hodl_loans/src/math/price.rs`

**Interfaces:**
- Consumes: `math::checked::pow10` (Task 1).

- [ ] **Step 1: Write the failing tests**

In `programs/hodl_loans/src/math/price.rs`, add to the existing `mod tests`, above `pyth_rejects_non_positive_and_vanishing_prices`:

```rust
    #[test]
    fn uncertainty_rounds_up_so_it_can_never_vanish() {
        // The price rounds down and the uncertainty rounds up; they are not symmetric, and the
        // asymmetry is the point. Collateral counts at `price - conf` and debt at
        // `price + conf`, so a conf rounded DOWN values collateral too high and debt too low —
        // anti-conservative on both sides at once.
        //
        // A spread finer than USD_SCALE is where it shows: floored it becomes 0, which claims
        // the oracle is perfectly certain.
        let p = scale_switchboard_value(650_000_000_000_000, 1).unwrap();
        assert_eq!(p.price, 650_000_000);
        assert_eq!(p.conf, 1, "a sub-scale spread must not round away to zero uncertainty");

        // And it rounds up, not to nearest: 6_250_000_123_456 / 1e6 is 6_250_000.123456.
        let p = scale_switchboard_value(650_000_000_000_000, 6_250_000_123_456).unwrap();
        assert_eq!(p.conf, 6_250_001);

        // Same rule on the Pyth path, which takes its shift from the feed's exponent.
        let p = scale_pyth_price(15_012_345_678, 1, -15).unwrap();
        assert_eq!(p.conf, 1);
    }

    #[test]
    fn an_absurd_feed_exponent_is_an_invalid_price_not_an_abort() {
        // `exponent` is read from the oracle account. Computing the shift with `+` would
        // overflow i32 and abort the whole transaction; `checked_add` makes it a normal error
        // the caller can see.
        assert!(scale_pyth_price(1, 0, i32::MAX).is_err());
        assert!(scale_pyth_price(1, 0, i32::MIN).is_err());
    }
```

- [ ] **Step 2: Run them to watch them fail**

Run: `cargo test -p hodl_loans --lib price`
Expected: `uncertainty_rounds_up_so_it_can_never_vanish` fails on its first assertion — `assertion `left == right` failed: a sub-scale spread must not round away to zero uncertainty`, `left: 0`, `right: 1`. These are pure unit tests, so `--lib` is enough and the stale-`.so` trap does not apply.

`an_absurd_feed_exponent_is_an_invalid_price_not_an_abort` fails differently and more loudly: the raw `exponent + USD_DECIMALS` overflows `i32` and the test panics with `attempt to add with overflow` rather than returning an error. That panic is the defect.

- [ ] **Step 3: Check the exponent and add a ceiling variant**

Replace `rescale` with the two functions below:

```rust
/// Rescale `value × 10^exponent` to `USD_SCALE`. Rounds down when shrinking.
///
/// `exponent` comes from the oracle account, so the shift is computed with `checked_add`
/// rather than `+`: a feed reporting an exponent near `i32::MAX` would otherwise abort the
/// transaction on an arithmetic overflow instead of returning `InvalidPrice`.
fn rescale(value: u128, exponent: i32) -> Result<u128> {
    let shift = exponent.checked_add(USD_DECIMALS).ok_or(HodlError::InvalidPrice)?;
    if shift >= 0 {
        Ok(value.checked_mul(pow10(shift as u32)?).ok_or(HodlError::MathOverflow)?)
    } else {
        Ok(value / pow10((-shift) as u32)?)
    }
}

/// `rescale`, rounding **up** when shrinking. Used only for the uncertainty term.
///
/// Rounding a confidence or spread down is anti-conservative on both sides at once, which is
/// easy to miss because the two uses pull in opposite directions: collateral counts at
/// `price − conf`, so a smaller `conf` values it higher, and debt counts at `price + conf`, so
/// a smaller `conf` values it lower. Rounding the uncertainty up is the only direction that is
/// conservative for both.
fn rescale_ceil(value: u128, exponent: i32) -> Result<u128> {
    let shift = exponent.checked_add(USD_DECIMALS).ok_or(HodlError::InvalidPrice)?;
    if shift >= 0 {
        Ok(value.checked_mul(pow10(shift as u32)?).ok_or(HodlError::MathOverflow)?)
    } else {
        Ok(value.div_ceil(pow10((-shift) as u32)?))
    }
}
```

- [ ] **Step 4: Round the uncertainty up on both oracle paths**

The price keeps rounding down; only the uncertainty term changes. In `scale_pyth_price`:

```rust
    Ok(UsdPrice { price: scaled, conf: rescale_ceil(conf as u128, exponent)? })
```

In `scale_switchboard_value`:

```rust
    Ok(UsdPrice { price: scaled, conf: rescale_ceil(spread as u128, -18)? })
```

- [ ] **Step 5: Run the suite**

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, **236 tests in all** (the unit suite is now 54).

No integration test changes its expected values. That is worth noticing rather than assuming: the existing fixtures use spreads that divide evenly, so before these tests were written nothing in the suite could tell the two rounding directions apart.

- [ ] **Step 6: Commit**

```bash
git add programs/hodl_loans/src/math/price.rs
git commit -m "fix: uncertainty rounds up, and an absurd feed exponent is an error"
```

---

### Task 3: The checked-math sweep

Five guarded raw `-=` operators, each on a `u64` account field. Every one is preceded by a `require!` that makes it safe today, and `overflow-checks = true` turns a missed case into an abort rather than a wrap. The reason to convert them anyway is that the guard and the operator are separate lines: a future edit can move, weaken or bypass the guard without the operator changing at all.

**Files:**
- Modify: `programs/hodl_loans/src/instructions/loans/take_loan.rs`, `src/instructions/loans/repay_loan.rs`, `src/instructions/admin/reserve.rs`, `src/instructions/positions/withdraw_collateral.rs`, `src/instructions/liquidity/withdraw_liquidity.rs`

- [ ] **Step 1: Convert all five**

Leave every existing `require!` exactly where it is; these replace only the arithmetic. `take_loan.rs`:

```rust
    market.cash = market.cash.checked_sub(amount).ok_or(HodlError::MathOverflow)?;
```

`repay_loan.rs`:

```rust
    slot.principal = slot.principal.checked_sub(principal_repaid).ok_or(HodlError::MathOverflow)?;
```

`reserve.rs`:

```rust
    market.protocol_reserve =
        market.protocol_reserve.checked_sub(amount).ok_or(HodlError::MathOverflow)?;
```

`withdraw_collateral.rs`:

```rust
        slot.amount = slot.amount.checked_sub(amount).ok_or(HodlError::MathOverflow)?;
```

`withdraw_liquidity.rs`:

```rust
    market.cash = market.cash.checked_sub(amount).ok_or(HodlError::MathOverflow)?;
```

- [ ] **Step 2: Run the suite**

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, **236 tests in all** — unchanged. Each converted site was already unreachable past its guard, so no test changes behaviour. What this buys is that the guard is no longer the only thing standing between a future edit and a wrap.

Run: `cargo clippy -p hodl_loans --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 3: Commit**

```bash
git add programs/hodl_loans/src/instructions/
git commit -m "refactor: checked subtraction at the five remaining raw sites"
```

---

### Task 4: Two parameter bounds

Both are configurations an admin can set today that break an instruction later, with no error at the point the mistake is made.

**Files:**
- Modify: `programs/hodl_loans/src/constants.rs`, `src/state/market.rs`, `src/instructions/admin/collateral_admin.rs`, `tests/collateral.rs`, `tests/market.rs`

**Interfaces:**
- Consumes: `math::checked::pow10` indirectly, through the liquidation path the decimals bound is derived from.
- Produces: `constants::MAX_COLLATERAL_DECIMALS`, `constants::MAX_NGN_STALE_SLOTS`.

- [ ] **Step 1: Write the failing tests**

Append to `programs/hodl_loans/tests/collateral.rs`:

```rust
#[test]
fn a_mint_with_too_many_decimals_cannot_be_listed() {
    // The bound comes from the LIQUIDATION path, not the health path. `token_value` copes with
    // roughly 38 decimals; `seize_for_repayment` multiplies twice and, against the worst
    // repayment the program permits, first overflows at 15. Past the bound a position holding
    // the asset could be opened and then never liquidated, so listing is refused instead.
    let mut env = Env::initialized();
    let admin = env.admin.pubkey();

    let too_wide = env.create_mint(MintKind::SplToken, 13);
    let instruction = list_collateral_ix(
        &admin,
        &too_wide,
        &SPL_TOKEN,
        default_collateral_params(&too_wide),
        CollateralKind::Standard,
    );
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::InvalidParameters);
    assert_eq!(env.config().collateral_count, 0);

    // The bound itself is allowed, and so is every decimals count the protocol actually uses.
    env.list_spl_collateral(12);
    env.list_spl_collateral(9);
    env.list_spl_collateral(6);
    assert_eq!(env.config().collateral_count, 3);
}
```

In `programs/hodl_loans/tests/market.rs`, extend `invalid_params_are_rejected` — add this to the end of that function, before its closing brace:

```rust
    // The NGN feed prices the debt side of every health check, so it may not drift further
    // behind than the collateral feeds do. 150 slots is the same 60 seconds
    // `MAX_PRICE_AGE_SECONDS` allows them, at a 400 ms slot.
    let mut params = default_market_params();
    params.ngn_max_stale_slots = 151;
    let instruction = create_market_ix(&admin, &mint, &TOKEN_2022, params);
    assert_hodl_error(send(&mut env.svm, &[instruction], &[&env.admin]), HodlError::InvalidParameters);

    let mut params = default_market_params();
    params.ngn_max_stale_slots = 150;
    let instruction = create_market_ix(&admin, &mint, &TOKEN_2022, params);
    send(&mut env.svm, &[instruction], &[&env.admin]).expect("150 slots is the bound, not past it");
```

- [ ] **Step 2: Run them to watch them fail**

Run: `./scripts/test.sh --test collateral --test market`
Expected: two failures, one in each binary, and both for the same reason — the configuration is currently accepted.

`a_mint_with_too_many_decimals_cannot_be_listed` fails at its first assertion with `assert_hodl_error` reporting the transaction succeeded where `InvalidParameters` was expected. `invalid_params_are_rejected` fails the same way on the 151-slot case, panicking at `tests/common/mod.rs` inside `assert_hodl_error`.

- [ ] **Step 3: Add the two constants**

In `programs/hodl_loans/src/constants.rs`, after `MAX_PRICE_AGE_SECONDS`:

```rust
/// Upper bound on a collateral mint's `decimals`, set by the LIQUIDATION path rather than the
/// health path.
///
/// `token_value` tolerates roughly 38 decimals before `u128` gives out, which is the number a
/// reader reaches for. `seize_for_repayment` is far tighter, because it multiplies twice:
/// `with_bonus × 10^decimals` first, then `display × MULTIPLIER_SCALE`. The second term carries
/// the collateral price in its denominator, so a cheap asset overflows sooner. Measured against
/// the worst repayment the program permits — `u64::MAX` cNGN, a 100% liquidation bonus, and a
/// $0.01 collateral — the first failure is at 15 decimals.
///
/// 12 clears every asset the protocol lists (USDC 6, SOL 9, the xStocks 8) with three decimals
/// of margin against that measured cliff. The consequence of getting this wrong is not a bad
/// price but an unliquidatable position: `seize_for_repayment` returns `MathOverflow` and the
/// liquidator simply cannot act.
pub const MAX_COLLATERAL_DECIMALS: u8 = 12;
/// Upper bound on `MarketParams::ngn_max_stale_slots`. A Solana slot targets 400 ms, so 150
/// slots is the same 60 seconds `MAX_PRICE_AGE_SECONDS` allows the collateral feeds — the NGN
/// feed prices the debt side of every health check, and there is no reason to let it drift
/// further behind than the collateral side.
pub const MAX_NGN_STALE_SLOTS: u64 = 150;
```

- [ ] **Step 4: Enforce the NGN bound**

In `programs/hodl_loans/src/state/market.rs`, add `MAX_NGN_STALE_SLOTS` to the `crate::constants` import, then replace the existing `ngn_max_stale_slots` check:

```rust
            self.ngn_max_stale_slots > 0 && self.ngn_max_stale_slots <= MAX_NGN_STALE_SLOTS,
            HodlError::InvalidParameters
        );
```

- [ ] **Step 5: Enforce the decimals bound**

Listing is the only place the mint's own decimals enter the program, so it is the only place this can be refused. In `programs/hodl_loans/src/instructions/admin/collateral_admin.rs`, add `MAX_COLLATERAL_DECIMALS` to the `crate::constants` import, then insert between `params.validate(...)` and `require_collateral_mint_on_entry(...)`:

```rust
    // Set by the liquidation path, not the health path — see `MAX_COLLATERAL_DECIMALS`. Listing
    // is the only place the mint's own decimals enter the program, so it is the only place this
    // can be refused; past the bound, `seize_for_repayment` overflows and positions holding the
    // asset cannot be liquidated at all.
    require!(
        ctx.accounts.mint.decimals <= MAX_COLLATERAL_DECIMALS,
        HodlError::InvalidParameters
    );
```

- [ ] **Step 6: Run the suite**

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, **237 tests in all** (`collateral` gains one).

- [ ] **Step 7: Commit**

```bash
git add programs/hodl_loans/src/ programs/hodl_loans/tests/
git commit -m "fix: bound collateral decimals and NGN feed staleness"
```

---

### Task 5: Account layout guards

`CollateralAsset` and `Market` have both taken fields out of their reserved padding, so their sizes did not change. Nothing checks that. `Position` pins the same property with `size_of` because it is zero-copy; these two are Borsh, so `INIT_SPACE` is the number that matters.

**Files:**
- Modify: `programs/hodl_loans/src/state/collateral.rs`, `src/state/market.rs`

- [ ] **Step 1: Write the failing tests**

Append to `programs/hodl_loans/src/state/collateral.rs`:

```rust
#[cfg(test)]
mod layout {
    use super::*;

    #[test]
    fn init_space_is_pinned_so_new_fields_come_out_of_the_padding() {
        // `price_account` was taken out of the reserved padding in Plan 2, and `kind` in Plan 4, so the account's size did not change. That is the whole
        // contract: a field added on top of `reserved` rather than out of it grows
        // `INIT_SPACE`, and every account already on chain is then too small to deserialize
        // into — with no error until someone touches one.
        //
        // `Position` pins the same property with `size_of` (it is zero-copy); these two are
        // Borsh, so `INIT_SPACE` is the number that matters. If this assertion fails, take the
        // bytes out of `reserved` instead of appending them.
        assert_eq!(CollateralAsset::INIT_SPACE, 294);
    }
}
```

Append to `programs/hodl_loans/src/state/market.rs`:

```rust
#[cfg(test)]
mod layout {
    use super::*;

    #[test]
    fn init_space_is_pinned_so_new_fields_come_out_of_the_padding() {
        // `accrual_remainder` was taken out of the reserved padding in Plan 3, so the account's size did not change. That is the whole
        // contract: a field added on top of `reserved` rather than out of it grows
        // `INIT_SPACE`, and every account already on chain is then too small to deserialize
        // into — with no error until someone touches one.
        //
        // `Position` pins the same property with `size_of` (it is zero-copy); these two are
        // Borsh, so `INIT_SPACE` is the number that matters. If this assertion fails, take the
        // bytes out of `reserved` instead of appending them.
        assert_eq!(Market::INIT_SPACE, 555);
    }
}
```

- [ ] **Step 2: Run them**

Run: `cargo test -p hodl_loans --lib layout`
Expected: PASS, both of them, immediately — 2 passed.

This task is the exception to red-then-green in this plan, and the reason is worth stating. There is no defect to fix: the two numbers are already correct, and a failing step would mean writing a wrong constant and then correcting it, which pins nothing. What the guards protect against is a *future* change — a field appended on top of `reserved` rather than taken out of it grows `INIT_SPACE`, and every account already on chain is then too small to deserialize into, with no error until someone touches one.

To see them work, temporarily add `pub extra: u64` to `CollateralAsset` above `reserved`, run `cargo test -p hodl_loans --lib layout`, watch it fail with `left: 302, right: 294`, and remove it again.

- [ ] **Step 3: Run the suite**

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, **239 tests in all** (the unit suite is now 56).

- [ ] **Step 4: Commit**

```bash
git add programs/hodl_loans/src/state/
git commit -m "test: pin CollateralAsset and Market INIT_SPACE"
```

---

### Task 6: `SeizureInputs`

`seize_for_repayment` took eight positional arguments behind an `#[allow(clippy::too_many_arguments)]`. Two of them, `collateral_price` and `multiplier`, are both `u128` at the same `10^12` scale — so transposing them compiled cleanly and produced a seizure wrong by the multiplier.

**Files:**
- Modify: `programs/hodl_loans/src/math/liquidation.rs`, `src/instructions/liquidation/liquidate.rs`

**Interfaces:**
- Produces: `math::liquidation::SeizureInputs`; `seize_for_repayment(inputs: &SeizureInputs) -> Result<Seizure>`.

- [ ] **Step 1: Add the struct and change the signature**

In `programs/hodl_loans/src/math/liquidation.rs`, put the struct **above** the `/// Spec §11 seizure` doc comment — that comment documents the function, and a struct inserted under it would silently take it over:

```rust
/// Everything `seize_for_repayment` needs, named.
///
/// It used to take these as eight positional arguments behind an
/// `#[allow(clippy::too_many_arguments)]`. Two of them — `collateral_price` and `multiplier` —
/// are both `u128` at the same `10^12` scale, so transposing them compiled cleanly and produced
/// a seizure wrong by the multiplier. Named fields make that particular mistake unrepresentable.
pub struct SeizureInputs {
    pub repay_amount: u64,
    pub ngn_price: u128,
    pub cngn_decimals: u8,
    pub collateral_price: u128,
    pub collateral_decimals: u8,
    pub multiplier: u128,
    pub bonus_bps: u16,
    pub slot_amount: u64,
}
```

Then replace the `#[allow(clippy::too_many_arguments)]` attribute and the eight-parameter signature with a destructuring one. The body below is unchanged:

```rust
pub fn seize_for_repayment(inputs: &SeizureInputs) -> Result<Seizure> {
    let &SeizureInputs {
        repay_amount,
        ngn_price,
        cngn_decimals,
        collateral_price,
        collateral_decimals,
        multiplier,
        bonus_bps,
        slot_amount,
    } = inputs;
```

- [ ] **Step 2: Run the build to watch it fail**

Run: `cargo build -p hodl_loans --tests`
Expected: `error[E0061]: this function takes 1 argument but 8 arguments were supplied`, at eleven sites. The build reports them in two phases, which is worth knowing so the first number does not look wrong: the library alone fails at the single call in `liquidate.rs` (`could not compile hodl_loans (lib) due to 1 previous error`), and the test build repeats that one and adds the ten positional calls in this file's own test module (`due to 11 previous errors`).

- [ ] **Step 3: Update the production call site**

In `programs/hodl_loans/src/instructions/liquidation/liquidate.rs`, add `SeizureInputs` to the `crate::math::liquidation` import, then:

```rust
        let seizure = seize_for_repayment(&SeizureInputs {
            repay_amount: requested,
            ngn_price: valuation.ngn.price,
            cngn_decimals: market.decimals,
            collateral_price,
            collateral_decimals: ctx.accounts.collateral.decimals,
            multiplier,
            bonus_bps: ctx.accounts.collateral.liquidation_bonus_bps,
            slot_amount: position.collateral[slot_index].amount,
        })?;
```

- [ ] **Step 4: Give the tests a base to vary from**

Ten positional calls become ten struct literals, which is unreadable unless each states only what it varies. In `liquidation.rs`'s `mod tests`, after the `use` lines:

```rust
    /// The common case, so each test states only what it is varying.
    fn base() -> SeizureInputs {
        SeizureInputs {
            repay_amount: REPAY,
            ngn_price: NGN,
            cngn_decimals: 6,
            collateral_price: USD,
            collateral_decimals: 6,
            multiplier: MULTIPLIER_ONE,
            bonus_bps: 500,
            slot_amount: u64::MAX,
        }
    }
```

Then rewrite each call in that module as `seize_for_repayment(&base())` or `seize_for_repayment(&SeizureInputs { field: value, ..base() })`, naming only the fields that test varies. For example the slot-cap test becomes `seize_for_repayment(&SeizureInputs { slot_amount: 500_000_000, ..base() })`, and the zero-multiplier rejection becomes `seize_for_repayment(&SeizureInputs { multiplier: 0, ..base() })`.

- [ ] **Step 5: Run the suite**

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, **239 tests in all** — unchanged. The arithmetic is identical; only the call shape moved.

Run: `cargo clippy -p hodl_loans --all-targets -- -D warnings`
Expected: no warnings, and no `#[allow(clippy::too_many_arguments)]` left in the file.

- [ ] **Step 6: Commit**

```bash
git add programs/hodl_loans/src/math/liquidation.rs programs/hodl_loans/src/instructions/liquidation/liquidate.rs
git commit -m "refactor: seize_for_repayment takes named inputs"
```

---

### Task 7: A vault substitution is not a price problem

`Liquidate`'s constraint pinning `collateral_vault` reports `PriceAccountMismatch`. A liquidator bot operator reading that will go looking at their oracle accounts. The Plan 3 follow-up also records that no test exercises the substitution at all, so the constraint and its error are both unpinned.

**Files:**
- Modify: `programs/hodl_loans/src/errors.rs`, `src/instructions/liquidation/liquidate.rs`, `tests/liquidation.rs`

**Interfaces:**
- Produces: `HodlError::CollateralVaultMismatch`.

- [ ] **Step 1: Write the failing test**

Append to `programs/hodl_loans/tests/liquidation.rs`:

```rust
#[test]
fn a_liquidation_must_name_the_collateral_assets_own_vault() {
    // The constraint that pins `collateral_vault` used to report `PriceAccountMismatch`, which
    // reads as an oracle problem to whoever is debugging a bot. Substituting another listed
    // asset's vault is a vault substitution and now says so.
    let (mut env, setup) = underwater();
    let liquidator = env.new_liquidator(&setup.cngn, LOAN);
    let collateral_account = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    let owner = setup.borrower.pubkey();
    let program = env.mint_program(&setup.usdc);

    // A second listed asset, so the substituted account is a real, initialized collateral vault
    // rather than something Anchor would reject before the constraint runs.
    let other = env.list_spl_collateral(6);
    let real_vault = collateral_vault_pda(&setup.usdc);
    let other_vault = collateral_vault_pda(&other);

    let prices = env.price_accounts(&owner);
    let mut swapped = liquidate_ix(
        &liquidator.pubkey(), &owner, &setup.cngn, &liquidator.cngn, &setup.usdc, &program,
        &collateral_account, 0, ONE_CNGN, prices,
    );
    let mut replaced = 0;
    for meta in swapped.accounts.iter_mut() {
        if meta.pubkey == real_vault {
            meta.pubkey = other_vault;
            replaced += 1;
        }
    }
    assert_eq!(replaced, 1, "the collateral vault must appear exactly once to be substituted");

    assert_hodl_error(
        send(&mut env.svm, &[swapped], &[&liquidator.key]),
        HodlError::CollateralVaultMismatch,
    );

    // The same call against the position's own vault succeeds, so the rejection is about the
    // substitution and not about the rest of the setup.
    env.liquidate(&liquidator, &setup, &setup.usdc, &collateral_account, 0, ONE_CNGN)
        .expect("the real vault liquidates");
}
```

- [ ] **Step 2: Run it to watch it fail**

Run: `./scripts/test.sh --test liquidation`
Expected: `error[E0599]: no variant or associated item named `CollateralVaultMismatch` found for enum `HodlError`` — the variant does not exist yet, so the test does not compile.

- [ ] **Step 3: Append the variant**

Codes are `6000 + position`, so this goes at the end of the enum and nowhere else. In `programs/hodl_loans/src/errors.rs`:

```rust
    #[msg("Collateral vault token account does not match the collateral asset")]
    CollateralVaultMismatch,
```

- [ ] **Step 4: Use it on the constraint**

In `programs/hodl_loans/src/instructions/liquidation/liquidate.rs`:

```rust
        constraint = collateral.vault == collateral_vault.key() @ HodlError::CollateralVaultMismatch
```

- [ ] **Step 5: Run the suite**

Run: `./scripts/test.sh`
Expected: every binary reports `ok`, **240 tests in all** (`liquidation` gains one).

- [ ] **Step 6: Commit**

```bash
git add programs/hodl_loans/src/ programs/hodl_loans/tests/liquidation.rs
git commit -m "fix: a substituted collateral vault reports CollateralVaultMismatch"
```

---

## Done when

- `./scripts/test.sh` reports 240 passing tests and `cargo clippy -p hodl_loans --all-targets -- -D warnings` is clean.
- `grep -rn "fn pow10" programs/hodl_loans/src/` returns exactly one hit, in `math/checked.rs`.
- `grep -rn "too_many_arguments" programs/hodl_loans/src/` returns no **attribute** in `math/liquidation.rs`. Two hits remain and both are correct: the `#[allow]` in `token/transfer.rs` is pre-existing, unrelated to `Liquidate`, and out of this plan's scope; the other is the phrase inside `SeizureInputs`'s own doc comment explaining why the attribute went away.
- No raw `-=` remains on an account field in `programs/hodl_loans/src/instructions/`.
- The Plan 2, 3 and 4 follow-ups carry a resolution note on their Plan 6 sections naming the entries this plan closed, in the same form Plan 2 already uses for the items Plans 3, 4 and 5 resolved. **This is the controller's to do, not a task's** — it spans three documents no task owns, and listing it here without a task to produce it was a defect in this plan that Task 7's implementer caught.
