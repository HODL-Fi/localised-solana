# HODL Fixed Loans on Solana — Design

**Status:** Approved in design review, pending spec review
**Date:** 2026-09-17
**Ports:** the fixed-tenure loan product of `lendbit-localised` (EVM, branch `track1-remediation`)

## 1. Summary

`hodl_loans` is a native Solana program (Anchor) for fixed-term cNGN loans.

- **Lenders** deposit cNGN into a pool and earn interest.
- **Borrowers** post SOL, USDC, USDT or listed xStocks as collateral and take fixed-term cNGN loans.
- **Liquidators** (anyone) repay unhealthy loans and receive collateral plus a bonus.
- **Promo balances** give users extra borrowing power, backed by real cNGN that HODL locks in a promo vault.

Collateral is priced by Pyth. cNGN is priced by a Switchboard On-Demand NGN/USD feed.

## 2. Decisions

| Topic | Decision |
|---|---|
| Deployment | Native Solana program; no cross-chain flow |
| Borrowable asset | cNGN only; `Market` is keyed by mint so more can be added later |
| Collateral | SOL (as wrapped SOL), USDC, USDT, admin-listed xStocks only |
| Liquidity | Outside lenders through a pool |
| Access | Borrowers and lenders must be whitelisted; liquidation is open to anyone |
| cNGN price | Switchboard On-Demand feed taking the median of several NGN/USD sources |
| Layout | One `Position` account per user holding collateral slots and loan slots |
| Overdue loans | Not liquidatable for being late; only when unhealthy (as on EVM) |
| Bad-debt loss | Protocol reserve absorbs it first, then lenders |
| Promo on honest repay | Stays on the position and keeps boosting borrowing power |
| Promo expiry | After 90 days with no open loan and no new loan |
| Promo forfeit | On any liquidation of the position |
| Promo grants | Backend-signed vouchers that users redeem |
| Promo cap vs LTV | `LTV + promo_cap ≤ liquidation threshold` for every collateral; standard LTV is 70% |
| Repo | Separate repo, `~/work/hodl/lendbit-solana` |

## 3. Scope

**In scope:**

- cNGN lending pool: lender deposit and withdraw.
- Collateral deposit and withdraw.
- Fixed-term loans: open, repay (by the borrower or any whitelisted payer).
- Permissionless liquidation and admin bad-debt write-off.
- Promo balances: promo vault, campaigns, vouchers, forfeiture, expiry.
- Admin controls: listing, risk parameters, pausing, whitelist and blacklist, reserve harvest, donation sweep.

**Out of scope:**

- Open-ended (floating) borrows.
- Cross-chain signed borrows.
- A collateral yield strategy.
- An on-chain equivalent of Chainlink Functions.
- Position ownership transfer.
- Backend, app and liquidation-bot integration (separate projects that consume this program's accounts and events).

## 4. Roles

| Role | Key | Permissions |
|---|---|---|
| Admin | Squads multisig; also the program upgrade authority | All admin instructions. Admin transfer is two-step (`propose_admin`, `accept_admin`) |
| Guardian | One key, set by admin | Pause a market or a collateral asset. Cannot unpause |
| Whitelister | One key, set by admin | Whitelist wallets. Cannot blacklist or whitelist a blacklisted wallet |
| Promo signer | One backend key, set by admin | Signs promo vouchers off-chain. Sends no transactions |
| User | Whitelisted, not blacklisted wallet | Lend, manage a position, borrow, repay, redeem promo |
| Liquidator | Anyone | `liquidate` |
| Cranker | Anyone | `expire_promo`, `close_voucher_receipt` |

## 5. Repository Layout

```text
lendbit-solana/
  Anchor.toml
  Cargo.toml
  programs/hodl_loans/src/
    lib.rs
    errors.rs
    events.rs
    constants.rs
    state/        config.rs, market.rs, collateral.rs, position.rs, lender.rs, access.rs, promo.rs
    math/         fixed_point.rs, interest.rs, shares.rs, health.rs, liquidation.rs
    oracle/       pyth.rs, switchboard.rs
    token/        extensions.rs, transfer.rs
    instructions/ admin/, lender/, borrower/, liquidation/, promo/
  tests/          LiteSVM program tests
  trident-tests/  invariant fuzzing
  docs/superpowers/specs/
```

`math/` holds pure functions with no account access, so it can be unit-tested directly.

**Dependencies:** `anchor-lang` and `anchor-spl` (`token_interface`), `spl-token-2022` (extension parsing), `pyth-solana-receiver-sdk`, `switchboard-on-demand`, `litesvm`, `trident`.

## 6. Constants

| Name | Value |
|---|---|
| `BPS` | 10_000 |
| `YEAR` | 31_536_000 seconds |
| `MAX_COLLATERAL_SLOTS` | 8 |
| `MAX_LOAN_SLOTS` | 10 |
| `MIN_TENURE` | 86_400 seconds |
| `VIRTUAL_SHARES` | 1_000 |
| `VIRTUAL_ASSETS` | 1 |
| `USD_SCALE` | 10^12 (fixed-point USD used in all health math; 10^18 would let `amount × price` overflow `u128` for large balances) |
| `MAX_PRICE_AGE_SECONDS` | 60 (the largest `max_price_age_seconds` an admin may set) |
| `MAX_NGN_STALE_SLOTS` | 150 (the largest `ngn_max_stale_slots` an admin may set — the same 60 seconds at a 400 ms slot, so the NGN feed cannot drift further behind than the collateral feeds) |
| `MAX_COLLATERAL_DECIMALS` | 12 (the largest `decimals` a collateral mint may have; set by the liquidation path, see §8) |

## 7. Accounts

Every account starts with `version: u8` and `bump: u8`, and ends with reserved padding so fields can be added without resizing.

### `Config` — seeds `["config"]`

`admin`, `pending_admin: Option<Pubkey>`, `guardian`, `whitelister`, `promo_signer`, `treasury` (owner of the token accounts that receive harvested reserve, swept donations and withdrawn promo funds), `promo_cap_bps: u16` (default 2_000; applies to every market because collateral assets are shared), `collateral_count: u16` (number of listed collateral assets).

### `Market` — seeds `["market", mint]`

| Field | Type | Meaning |
|---|---|---|
| `mint`, `token_program`, `vault` | Pubkey | cNGN mint, its token program, vault token account owned by this PDA |
| `decimals` | u8 | cNGN decimals |
| `cash` | u64 | cNGN the program has recorded as held; changed only by program instructions |
| `total_borrows` | u64 | Outstanding loan principal |
| `lp_rate_product` | u128 | Σ over active loans of `principal × rate_bps × (BPS − reserve_factor_bps)` |
| `accrued_interest` | u128 | Lender interest accrued but not yet paid, already net of reserve |
| `protocol_reserve` | u64 | Protocol's share of interest received |
| `total_bad_debt` | u128 | Running total of written-off value |
| `total_shares` | u128 | Lender shares outstanding |
| `last_accrual_ts` | i64 | Last interest accrual |
| `accrual_remainder` | u128 | Division remainder carried from one accrual to the next (§9) |
| `interest_rate_bps`, `penalty_rate_bps`, `reserve_factor_bps` | u16 | Terms for **new** loans |
| `max_utilization_bps` | u16 | Default 9_000 |
| `min_loan_amount` | u64 | Smallest loan |
| `max_tenure_seconds` | i64 | Default 365 days |
| `bad_debt_dust_usd` | u128 | Collateral value under which a write-off is allowed |
| `ngn_feed` | Pubkey | Switchboard On-Demand pull feed for NGN/USD |
| `ngn_max_stale_slots`, `ngn_min_samples`, `ngn_max_spread_bps` | u64, u32, u16 | Switchboard read limits |
| `promo_inactivity_seconds` | i64 | Default 90 days |
| `max_promo_per_position` | u64 | Largest promo balance one position can hold |
| `paused` | bool | Blocks lender deposits and new loans |

### `CollateralAsset` — seeds `["collateral", mint]`

| Field | Meaning |
|---|---|
| `mint`, `token_program`, `vault`, `decimals` | Collateral mint, program, custody vault (seeds `["collateral_vault", mint]`, owned by this PDA), decimals |
| `kind` | `Standard` or `XStock` (enables Token-2022 extension checks and the scaled-UI multiplier) |
| `pyth_feed_id: [u8; 32]` | Pyth feed |
| `price_account` | The one Pyth price account accepted for this asset (a sponsored push feed). `Pubkey::default()` leaves it unpinned, accepting any verified update for the feed inside the age window |
| `max_price_age_seconds`, `max_conf_bps` | Pyth read limits; the age may not exceed `MAX_PRICE_AGE_SECONDS` |
| `ltv_bps`, `liquidation_threshold_bps`, `liquidation_bonus_bps` | Risk parameters |
| `deposit_cap`, `total_deposited` | Raw token amounts |
| `paused` | Blocks new deposits of this asset |

### `Position` — seeds `["position", owner]`

| Field | Meaning |
|---|---|
| `owner`, `rent_payer` | Borrower; who paid the account rent |
| `market` | The market all of this position's loans come from; `Pubkey::default()` until the first loan. Loans from another market fail with `MarketMismatch` |
| `collateral: [CollateralSlot; 8]` | `{ mint: Pubkey, amount: u64 }`; `amount == 0` means the slot is free |
| `loans: [LoanSlot; 10]` | See below |
| `next_loan_id: u64` | Loan IDs count up and are unique for the lifetime of this position account; closing and reopening a position (the PDA is `["position", owner]`) restarts them at `0`, so a backend keyed only on `(position, loan_id)` must disambiguate across that reset, e.g. by `originated_at` |
| `promo_balance: u64` | cNGN-denominated promo |
| `promo_last_activity_at: i64` | Start of the promo inactivity window |

`LoanSlot`: `id: u64`, `principal: u64`, `original_principal: u64`, `repaid: u64`, `originated_at: i64`, `interest_anchor: i64`, `tenure_seconds: i64`, `rate_bps: u16`, `penalty_rate_bps: u16`, `reserve_factor_bps: u16`, `active: u8` (`0` = free; a closed slot is zeroed).

`Position` is a zero-copy account with no `Option` fields, so bots can read it at fixed offsets.

### `LenderPosition` — seeds `["lender", market, owner]`

`market`, `owner`, `shares: u128`. Shares are not a token and can't be transferred.

### `Access` — seeds `["access", wallet]`

`wallet`, `whitelisted: bool`, `blacklisted: bool`.

### `PromoVault` — seeds `["promo_vault", market]`

`market`, `vault` (cNGN token account owned by this PDA), `cash: u64`, `outstanding: u64` (sum of all position promo balances), `unissued: u64` (sum over campaigns of `budget − granted`).

### `Campaign` — seeds `["campaign", market, campaign_id (u64 LE)]`

`market`, `campaign_id`, `budget: u64`, `granted: u64`, `redeem_until: i64`, `active: bool`.

### `VoucherReceipt` — seeds `["voucher", campaign, nonce (u64 LE)]`

`campaign`, `nonce`, `voucher_expiry: i64`, `rent_payer`.

## 8. Prices and Health

### Price reads

**Pyth (collateral):**
- The account must be owned by the Pyth receiver program (`rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ`), and, when the asset pins one, be exactly `collateral.price_account`.
- Pull updates are ephemeral accounts anyone may post, so without a pinned account the caller chooses which verified update inside `max_price_age_seconds` to present — the most favourable price in that window. Pinning removes the choice; `MAX_PRICE_AGE_SECONDS` bounds it for assets with no sponsored feed to pin.
- **Liveness risk of a pinned feed.** A pinned `PriceUpdateV2` is a sponsored push account with a fixed write authority — a liquidator cannot refresh it directly. If the sponsor's crank stalls beyond `max_price_age_seconds`, every position holding that asset becomes un-liquidatable, and because `write_off_loan` prices the position the same way, the bad-debt escape hatch closes at the same time. The remedy is operational: `update_collateral_params` unpins the asset, through the admin multisig's timelock (§18).
- Read the `PriceUpdateV2` account with `get_price_no_older_than(max_price_age_seconds, pyth_feed_id)`, and require full verification. A too-old price fails with `StalePrice`, another feed with `PriceAccountMismatch`, anything else (such as partial verification or a non-positive price) with `InvalidPrice`.
- Reject when `conf × BPS > price × max_conf_bps`.
- Convert price and confidence to `USD_SCALE` using the feed exponent. **The price rounds down and the confidence rounds up**, and the asymmetry is deliberate: collateral counts at `price − conf` and debt at `price + conf`, so a confidence rounded down would value collateral too high *and* debt too low. Rounding the uncertainty up is the only direction conservative for both. The same rule applies to the Switchboard spread below. The exponent comes from the feed account, so the shift is computed with a checked add — an absurd exponent is `InvalidPrice`, not an aborted transaction.

**Switchboard (cNGN):**
- The account address must equal `market.ngn_feed`, and its data must carry the `PullFeedAccountData` discriminator.
- Read the aggregated `result`. Fail with `StalePrice` when `result.slot` is 0, older than `ngn_max_stale_slots`, or has fewer than `ngn_min_samples` samples. Require a positive value.
- The spread is `result.std_dev`. Reject when it exceeds `ngn_max_spread_bps` of the value.
- Treat 1 cNGN as 1 NGN.

### Collateral decimals

A collateral mint's `decimals` is bounded at `MAX_COLLATERAL_DECIMALS` when it is listed, and the bound comes from the **liquidation** path rather than the health path. `token_value` copes with roughly 38 decimals before `u128` gives out, which is the number a reader reaches for; `seize_for_repayment` multiplies twice — `with_bonus × 10^decimals`, then `display × MULTIPLIER_SCALE` — and the second term carries the collateral price in its denominator, so a cheap asset overflows sooner. Measured against the worst repayment the program permits (`u64::MAX` cNGN, a 100% bonus, a $0.01 collateral, and today's NGN price of $0.000625), the first failure is at 15 decimals.

The consequence of getting this wrong is not that the position becomes unliquidatable outright: the overflow scales linearly in `repay_amount`, and a liquidator chooses `amount` freely (`requested = min(amount, balance)`), so it can route around the overflow by repaying less. Liquidation degrades into chunked repayments from roughly 13 to 20 decimals and only becomes genuinely impossible past about 22. The bound at 12 exists to make the failure loud at listing time — a cheap, one-time refusal — instead of surprising a liquidator deep in the liquidation path, and to keep every `u128` headroom argument elsewhere in the program valid. Listing is the only place the mint's own decimals enter the program, so it is the only place the configuration can be refused. The figure lives in `constants.rs` beside its derivation.

### xStock multiplier

For `XStock` assets, read `ScaledUiAmountConfig` from the mint. Use `new_multiplier` once `now ≥ new_multiplier_effective_timestamp`, otherwise `multiplier`. Token-2022 never moves the scheduled value into `multiplier` on its own, so reading that field alone goes stale the moment a corporate action takes effect — as it had on both AAPLX and NVDAX when they were checked (§20 item 2).

The stored `f64` is converted to a `MULTIPLIER_SCALE = 10^12` fixed-point value, **rounded down**: a factor binary floating point cannot hold exactly lands one unit low, which undervalues collateral by 10^-12 of a token and never the borrower's debt. A multiplier that is not finite, not positive, above `MAX_MULTIPLIER = 10^6` or that rounds to zero is rejected with `InvalidPrice`.

The multiplier is never stored in a protocol account; every valuation reads it from the mint.

A `Standard` asset has multiplier 1.

### Price accounts

For health checks, the instruction's remaining accounts are, for each non-empty collateral slot **in slot order**:
- `Standard`: `(CollateralAsset, PriceUpdateV2)` — two accounts. The mint is not passed: the asset carries the decimals, and its own `mint` field identifies it.
- `XStock`: `(CollateralAsset, PriceUpdateV2, mint)` — three, because the multiplier lives on the mint.

The program loops over the position's slots, not over the accounts supplied, advancing a cursor by two or three depending on the asset's kind. For each slot it requires:
- the `CollateralAsset` to be program-owned and its `mint` to equal `slot.mint`;
- the price account to carry that asset's feed ID, and to be the asset's `price_account` when it pins one;
- for an `XStock`, the third account's key to equal `slot.mint`.

After the loop the cursor must land exactly on the end of the supplied accounts. A missing, extra or mismatched account fails with `PriceAccountMismatch`.

### Values

```text
value_i          = amount_i × multiplier_i × (collateral_price_i − conf_i) / 10^decimals_i
own_value        = Σ value_i
promo_value      = promo_balance × (ngn_price − ngn_spread) / 10^cngn_decimals
promo_counted    = min(promo_value, own_value × promo_cap_bps / BPS)
borrow_limit     = Σ value_i × ltv_i / BPS                    + promo_counted
liquidation_line = Σ value_i × liquidation_threshold_i / BPS  + promo_counted
debt             = Σ balance_j(now) × (ngn_price + ngn_spread) / 10^cngn_decimals
```

`amount_i × multiplier_i` is the display amount, **rounded down** before it is priced: Pyth quotes an xStock per display token, not per raw unit (§20 item 2).

- **Healthy for borrowing or withdrawing:** `debt ≤ borrow_limit`.
- **Liquidatable:** `debt > liquidation_line`.
- All values are `u128` at `USD_SCALE`, using checked arithmetic.

### Parameter rules

Collateral rules, enforced by `list_collateral` and `update_collateral_params`:

- `1_000 ≤ ltv_bps`
- `ltv_bps + config.promo_cap_bps ≤ liquidation_threshold_bps`
- `liquidation_threshold_bps × (BPS + liquidation_bonus_bps) ≤ BPS²`

Market rules, enforced by `create_market` and `update_market_params`:

- `reserve_factor_bps ≤ BPS`
- `max_utilization_bps ≤ BPS`
- `MIN_TENURE ≤ max_tenure_seconds`
- `interest_rate_bps ≤ BPS` and `penalty_rate_bps ≤ BPS`
- `ngn_feed` is not the default key; `ngn_max_stale_slots > 0`; `ngn_min_samples ≥ 1`; `ngn_max_spread_bps ≤ BPS`
- `promo_inactivity_seconds ≥ 0`

`set_promo_cap` rejects a value that breaks the second collateral rule for any listed asset. Every `CollateralAsset` account is passed as a remaining account, **in ascending key order**, and the count must equal `config.collateral_count`. The count alone would not stop one permissive asset being passed several times while the rest go unexamined; requiring the keys to strictly increase does, and each account's PDA is re-derived from its own stored mint and bump so a look-alike cannot stand in for a stricter asset. `list_collateral` increments `collateral_count` and `delist_collateral` decrements it.

### Launch values

| Asset | LTV | Threshold | Bonus |
|---|---|---|---|
| SOL | 70% | 90% | 10% |
| USDC | 70% | 90% | 5% |
| USDT | 70% | 90% | 5% |
| xStocks (each) | 50% | 75% | 10% |

- Each xStock also gets a deposit cap at listing.
- Config: `promo_cap` 20%.
- Market: `max_utilization` 90%, `promo_inactivity` 90 days, `max_tenure` 365 days.
- Interest rate, penalty rate, reserve factor, `min_loan_amount`, `max_promo_per_position` and `bad_debt_dust_usd` are business settings chosen at deployment.

## 9. Market Accounting

### Accrual — run first by every instruction that touches the market

```text
elapsed           = now − last_accrual_ts
numerator         = lp_rate_product × elapsed + accrual_remainder
accrued_interest += numerator / (BPS × BPS × YEAR)     (round down)
accrual_remainder = numerator mod (BPS × BPS × YEAR)
last_accrual_ts   = now
```

### Total assets

```text
total_assets = cash + total_borrows + accrued_interest − protocol_reserve
```

This is evaluated after accrual.

### Lender shares

- **`deposit_liquidity(amount)`:** `shares = amount × (total_shares + VIRTUAL_SHARES) / (total_assets + VIRTUAL_ASSETS)`, rounded down. Must be > 0.
- **`withdraw_liquidity(amount)`:** `shares_burned = ceil(amount × (total_shares + VIRTUAL_SHARES) / (total_assets + VIRTUAL_ASSETS))`. Requires `shares_burned ≤ lender.shares` and `amount ≤ cash − protocol_reserve`.
- `amount = u64::MAX` withdraws the lender's full redeemable value, capped at available cash.

### Utilization cap (checked by `take_loan`)

```text
total_borrows + amount ≤ (cash − protocol_reserve + total_borrows) × max_utilization_bps / BPS
amount ≤ cash − protocol_reserve
```

### A loan's accrued lender interest

This is the amount the market has accrued for principal `p` of a loan since its anchor:

```text
R(p, loan) = p × loan.rate_bps × (BPS − loan.reserve_factor_bps) × (now − loan.interest_anchor) / (BPS × BPS × YEAR)   (round down)
```

Repayment, liquidation and write-off subtract `R` from `accrued_interest`, saturating at zero.

### Donations

`cash` ignores tokens sent directly to the vault. `sweep_excess` moves `vault balance − cash` (market), `vault balance − total_deposited` (collateral), or `vault balance − cash` (promo vault) to the treasury. If the balance is below the recorded amount, nothing moves.

## 10. Loans

### Balance owed (rounds up)

```text
maturity = originated_at + tenure_seconds
interest = principal × rate_bps × max(0, min(now, maturity) − interest_anchor) / (BPS × YEAR)
penalty  = (principal + interest) × (rate_bps + penalty_rate_bps) × max(0, now − max(maturity, interest_anchor)) / (BPS × YEAR)
balance  = principal + interest + penalty
due      = interest + penalty
```

### `take_loan(amount, tenure_seconds)`

1. Require the signer's `Access` (whitelisted, not blacklisted), the market not paused, `amount ≥ min_loan_amount`, and `position.market` unset or equal to this market (`MarketMismatch`).
2. Require `MIN_TENURE ≤ tenure_seconds ≤ max_tenure_seconds`, and a free loan slot.
3. Accrue. If the position has no active loans and `now ≥ promo_last_activity_at + promo_inactivity_seconds`, expire the promo (see §12).
4. Check the utilization cap.
5. Require `debt + amount's value ≤ borrow_limit`.
6. Write the slot: `id = next_loan_id++`, principal and original principal = `amount`, `originated_at = interest_anchor = now`, and the rate, penalty rate and reserve factor copied from the market.
7. `total_borrows += amount`; `lp_rate_product += amount × rate × (BPS − reserve_factor)`; `cash −= amount`; `promo_last_activity_at = now`; `position.market = market`.
8. Transfer `amount` cNGN from the market vault to a cNGN token account owned by the borrower. Emit `LoanOpened`.

### `repay_loan(position, loan_id, amount)`

1. Require the payer's `Access`. The payer doesn't need to own the position.
2. Accrue. Require `position.market` to be this market (`MarketMismatch`). Find the active slot with `id == loan_id`.
3. `amount = min(amount, balance)`. Require `amount ≥ due` (`RepaymentBelowInterest`).
4. `principal_repaid = amount − due`. Release `R(principal, loan)` from `accrued_interest`.
5. `reserve = due × reserve_factor_bps / BPS` (round down); `protocol_reserve += reserve`.
6. `total_borrows −= principal_repaid`; `lp_rate_product −= principal_repaid × rate × (BPS − reserve_factor)`; `cash += amount`.
7. Update the loan: `principal −= principal_repaid`; `repaid += amount`; `interest_anchor = now`.
8. Transfer `amount` from the payer to the market vault.
9. If `principal == 0`:
   - Clear the slot and emit `LoanRepaid`.
   - If no loans remain active, set `promo_last_activity_at = now`.
10. Otherwise emit `LoanPartiallyRepaid`.

Repayment needs no prices, so it works during an oracle outage.

## 11. Liquidation and Bad Debt

### `liquidate(position, loan_id, collateral_mint, amount)` — no access check

1. Accrue. Load the price accounts for every collateral slot and the NGN feed.
2. Require the position to be liquidatable (health includes promo). Require an active slot `loan_id` and a collateral slot for `collateral_mint`.
3. **Promo forfeit:** if `promo_balance > 0`:
   - Move that amount of cNGN from the promo vault to the market vault.
   - `promo_vault.cash −= P`; `promo_vault.outstanding −= P`; `market.cash += P`.
   - `promo_balance = 0`. Emit `PromoForfeited`.
   - This does not reduce the borrower's debt.
4. `amount = min(amount, balance)`.
5. Compute seized collateral from **plain prices** (no confidence adjustment):

   ```text
   seize_raw = amount × ngn_price × (BPS + bonus) × 10^decimals / (10^cngn_decimals × collateral_price × multiplier × BPS)
   ```

   The division is interleaved — value, then bonus, then collateral units, then the multiplier — so a large repayment cannot overflow `u128`, and every step rounds down. Dividing by the multiplier converts the display amount the price bought back into the raw units the vault moves; for a `Standard` asset the multiplier is 1 and the step leaves the value exact. It is not a no-op there, though: the `× MULTIPLIER_SCALE` runs unconditionally, so a display amount above ≈3.4 × 10^26 fails with `MathOverflow` where the pre-multiplier formula fell through to the slot cap in step 6. It fails closed, but the consequence is specific — such an asset is **unliquidatable** rather than slot-capped. Reaching it takes a collateral price near zero and a repayment near `u64::MAX`, a position that would already be a write-off candidate.

6. If `seize_raw > slot.amount`: set `amount = amount × slot.amount / seize_raw` and `seize_raw = slot.amount`.
7. `principal_repaid = amount × principal / balance` (round down). Require > 0 (`ZeroPrincipalRepaid`).
8. `interest_paid = amount − principal_repaid`. Release `R(principal_repaid, loan)`. `reserve = interest_paid × reserve_factor_bps / BPS`; `protocol_reserve += reserve`.
9. `total_borrows −= principal_repaid`; `lp_rate_product −= principal_repaid × rate × (BPS − reserve_factor)`; `cash += amount`.
10. Update the loan: `principal −= principal_repaid`; `repaid += amount`. **`interest_anchor` is not changed.**
11. `slot.amount −= seize_raw`; `collateral.total_deposited −= seize_raw`.
12. Transfer `amount` cNGN from the liquidator to the market vault, and `seize_raw` collateral from custody to the liquidator.
13. If `principal == 0`, clear the slot and emit `LoanLiquidated`; otherwise emit `LoanPartiallyLiquidated`.

There is no close factor. If the chosen collateral can't move (for example, an issuer-paused xStock), the transaction fails and the liquidator chooses another collateral.

**Accepted risk: permissionless penalty step.** The market accrues lender interest only at `rate × (BPS − reserve_factor)` (§9); an overdue loan's penalty is not part of that continuous accrual, so it reaches lenders as a single step — released through `R(principal_repaid, loan)` — only when the loan is repaid or liquidated. Repayment is gated to the borrower or a whitelisted payer, but `liquidate` has no access check, and `deposit_liquidity`, `liquidate`, `withdraw_liquidity` all fit in one transaction: a lender can deposit immediately before the step and share in penalty income it did not wait for. This dilutes honest lenders' share of penalty income; it does not threaten solvency, since the penalty is debt already owed rather than newly minted value. Accruing the penalty into `lp_rate_product` as it builds up, or amortising its release, is deferred to Plan 6.

### `write_off_loan(position, loan_id)` — admin

1. Accrue. Load prices.
2. Require the position to be liquidatable and `own_value < bad_debt_dust_usd`.
3. If `promo_balance > 0`, forfeit it into the market exactly as in liquidation step 3.
4. `loss = principal + R(principal, loan)`.
   - `total_borrows −= principal`; `lp_rate_product −= principal × rate × (BPS − reserve_factor)`.
   - Release `R` from `accrued_interest`.
5. `covered = min(protocol_reserve, loss)`; `protocol_reserve −= covered`. Lenders absorb `loss − covered` through the share price.
6. `total_bad_debt += loss`. Clear the slot and emit `LoanWrittenOff { loss, covered_by_reserve: covered }`.

The collateral stays in the position: a write-off clears the debt, it does not seize the dust.

**Accepted risk.** The admin signs through a timelocked multisig (§18), so a lender watching the queue can withdraw before the loss lands, leaving it to those who stay. A write-off requires the collateral to be worth less than `bad_debt_dust_usd`, which bounds that loss, and the reserve absorbs it first. The alternative — letting the guardian freeze lender withdrawals around a write-off — buys less than the trust it costs.

## 12. Promo Balance

**Purpose.** Promo is shop-only borrowing power. It is backed by real cNGN in `PromoVault`, counts only as a small topping on the user's own collateral, is forfeited to lenders on liquidation, and expires when unused.

### Invariant

`promo_vault.outstanding + promo_vault.unissued ≤ promo_vault.cash`, always.

### Funding and campaigns (admin)

- **`create_promo_vault`:** creates the `PromoVault` and its cNGN token account for a market. Separate from `create_market` so a market can run without promo, and explicit rather than `init_if_needed` inside funding, so the question of re-initializing an account that holds funds never arises.
- **`fund_promo_vault(amount)`:** transfer cNGN in; `cash += amount`.
- **`withdraw_promo_vault(amount)`:** requires `amount ≤ cash − outstanding − unissued`; sends to the treasury.
- **`create_campaign(campaign_id, budget, redeem_until)`:** requires `cash − outstanding − unissued ≥ budget`; `unissued += budget`.
- **`close_campaign(campaign_id)`:** `unissued −= budget − granted`; `active = false`. The account stays, so a campaign's `granted` total remains readable and redeemed vouchers keep a campaign to point at.
- **`sweep_promo_excess`:** moves `vault balance − cash` to the treasury, as §9 does for the market vault and §13 for a collateral vault. Without it a direct transfer into the promo vault's token account is stranded. It is the third copy of the same body, and the three share one helper.

### Voucher

The voucher message is the Borsh serialization of:

```text
{ domain: "hodl_loans:promo_voucher:v1", program_id, market, campaign_id, wallet, amount, nonce, voucher_expiry }
```

### `redeem_promo(voucher)`

1. Require the signer's `Access`, `voucher.wallet == signer`, and a position owned by the signer.
2. Signature check:
   - Read the Instructions sysvar and require an Ed25519 program instruction earlier in the same transaction.
   - Its public key must equal `config.promo_signer`, and its message must equal the voucher bytes.
3. Require:
   - `now ≤ voucher_expiry`;
   - the campaign is active with `now ≤ redeem_until`;
   - `granted + amount ≤ budget`;
   - `promo_balance + amount ≤ max_promo_per_position`.
4. Create `VoucherReceipt` for `(campaign, nonce)`. It already existing fails the instruction, which prevents replay.
5. Update: `granted += amount`; `unissued −= amount`; `outstanding += amount`; `promo_balance += amount`; `promo_last_activity_at = now`; `position.market = market`. Emit `PromoRedeemed`.

Redeeming binds the position to the market exactly as its first loan does: the promo is backed by that market's vault and counted against its cap, and `expire_promo` would otherwise have no way to tell which vault to credit for a position that has never borrowed.

Per-referral rules (one voucher per invited friend, and so on) are enforced by the backend before signing.

### Counting

Counted as `promo_counted` in §8: capped at `promo_cap_bps` of own collateral value, and zero when the user has no collateral. Promo can never be withdrawn.

### Forfeit

On the first liquidation of a position with promo (§11 step 3), and on write-off.

### Expiry

- `promo_last_activity_at` is set on redeem, on `take_loan`, and when the last active loan closes.
- **`expire_promo(position)`** (anyone): requires `promo_balance > 0`, no active loans, and `now ≥ promo_last_activity_at + promo_inactivity_seconds`. Then `outstanding −= promo_balance` and `promo_balance = 0`; the cNGN stays in the promo vault as free HODL funds. Emit `PromoExpired`.
- `take_loan` performs the same expiry before its health check.

### Other promo instructions

- **`revoke_promo(position)`** (admin): requires no active loans; releases the promo the same way as expiry. Emit `PromoRevoked`.
- **`close_position`:** releases any promo the same way, and emits `PromoReleased`. The market and promo vault accounts are required when the position still holds promo, so closing it cannot strand the backing.
- **`close_voucher_receipt`** (anyone): allowed once `now > voucher_expiry`; refunds rent to `rent_payer`.

### Why cheating loses

At maximum borrowing, debt ≤ own value × (LTV + promo_cap) ≤ own value × threshold < own value. A defaulting borrower has taken out less than the collateral they lose to liquidation (debt plus bonus, up to all of it), and their promo backing goes to lenders.

## 13. Instructions

**Setup and admin:**

| Instruction | Signer | Notes |
|---|---|---|
| `initialize` | Program upgrade authority | Creates `Config`; checked against the `ProgramData` upgrade authority so it can't be front-run |
| `propose_admin`, `accept_admin` | Admin; proposed admin | Two-step admin transfer |
| `set_guardian`, `set_whitelister`, `set_promo_signer`, `set_treasury` | Admin | |
| `set_promo_cap` | Admin | Rules in §8; all listed `CollateralAsset` accounts required |
| `create_market` | Admin | Checks the mint's extensions (§14) and creates the vault |
| `update_market_params` | Admin | Rules in §8; rate and reserve changes apply to new loans only |
| `set_market_paused` | Guardian (`true` only) or admin | |
| `list_collateral`, `update_collateral_params` | Admin | Rules in §8; `list_collateral` checks extensions (§14) |
| `set_collateral_paused` | Guardian (`true` only) or admin | |
| `delist_collateral` | Admin | Requires `total_deposited == 0` and an empty vault; closes the vault and the asset account |
| `sweep_collateral_excess` | Admin | Moves `vault balance − total_deposited` to the treasury (§9) |
| `whitelist` | Whitelister or admin | Creates `Access` if missing; fails if blacklisted |
| `blacklist`, `unblacklist` | Admin | `blacklist` also clears `whitelisted`; `unblacklist` does not re-whitelist |
| `harvest_reserve(amount)` | Admin | `amount ≤ protocol_reserve` (else `InsufficientCash`); `cash` and `protocol_reserve` both decrease |
| `sweep_excess` | Admin | §9 |
| `write_off_loan` | Admin | §11 |
| `create_promo_vault`, `fund_promo_vault`, `withdraw_promo_vault`, `sweep_promo_excess`, `create_campaign`, `close_campaign`, `revoke_promo` | Admin | §12 |

**Users:**

| Instruction | Signer | Notes |
|---|---|---|
| `deposit_liquidity`, `withdraw_liquidity` | Whitelisted lender | §9; withdraw works while paused |
| `open_position` | Whitelisted user; any fee payer | Records `rent_payer` |
| `close_position` | Whitelisted owner | Requires no collateral and no active loans; releases promo; refunds rent to `rent_payer` |
| `deposit_collateral(amount)` | Whitelisted owner | Mint comes from the accounts. Asset not paused, under deposit cap, free or matching slot; no prices needed |
| `withdraw_collateral(amount)` | Whitelisted owner | Mint comes from the accounts. More than the slot holds fails with `InsufficientCollateral`. If loans are active, the `market` (must equal `position.market`) and `ngn_feed` accounts are required, price pairs cover the slots still used after the withdrawal, and `debt ≤ borrow_limit` must hold afterwards. No accrual: it doesn't change a position's debt |
| `take_loan`, `repay_loan` | Whitelisted | §10 |
| `redeem_promo` | Whitelisted | §12 |

**Anyone:** `liquidate` (§11), `expire_promo` and `close_voucher_receipt` (§12).

**Blacklisted wallets** can't call any user instruction. Another whitelisted wallet can still repay their loans, and anyone can liquidate them.

**Instructions that need prices:** `take_loan`, `withdraw_collateral` (when loans are active), `liquidate`, `write_off_loan`. All others work without prices.

## 14. Token Handling

- All transfers use `transfer_checked` through `token_interface`. Vault token accounts are PDAs owned by their parent account.
- **SOL** is accepted only as wrapped SOL. Wrapping and unwrapping happen in the client.
- **`Standard` mints** (SOL, USDC, USDT, cNGN) are rejected if they carry any Token-2022 extension other than metadata (`MetadataPointer`, `TokenMetadata`).
- **`XStock` mints** may carry only:
  - `PermanentDelegate`, `Pausable`, `ScaledUiAmount`, `MetadataPointer`, `TokenMetadata`;
  - `ConfidentialTransferMint` (vaults never configure confidential balances);
  - `TransferHook` with **no hook program set**;
  - `DefaultAccountState` at **`Initialized`**.

  `ScaledUiAmount` must be **present**: it is what makes the asset priceable, and the rest of the set is permissive about presence, so a mint carrying only allowed extensions — metadata and a permanent delegate, say — would otherwise list as an `XStock` and then fail every health check that touched it, breaking borrowing, withdrawal and liquidation for any position holding it. Listing is the one moment that is cheap to catch.

  Anything else, including `TransferFeeConfig`, `NonTransferable` and `InterestBearingConfig`, is rejected with `UnsupportedMintExtension`. The last two entries are the only ones whose *value* is checked as well as their presence: a hook program would run issuer code inside every transfer, and a frozen default would freeze any token account created after the flip — including a collateral vault.
- **Accepted issuer risks for xStocks**, limited by lower LTV and deposit caps. The live mints spread these powers across three separate keys (§20 item 3), so they carry three different compromise probabilities rather than one:
  - The permanent delegate (`5aMNN…FvEq`) can move or burn tokens from the custody vault. The protocol's books keep the borrower's balance; the vault is short, and withdrawals fail in the token program once it empties. The loss is bounded by what borrowers deposited: the delegate can only take back its own token.
  - A paused mint (`JDq14…xJNs`) blocks deposits, withdrawals and liquidation seizures of that asset.
  - The freeze authority (`JDq14…xJNs`, the same key as the pause) can set `DefaultAccountState` to `Frozen`. Every live xStock carries the extension at `Initialized` today, and xStocks' own docs say it is there so Backed can switch on blocklist-style compliance tooling later (§20 item 3). Because only the entry policy checks it, this stops new listings and new deposits without sealing positions already open.
  - The **scaled-UI authority** (`S7vYFFWH6BjJyEsdrPQpqpYTqLTrPRK6KW3VwsJuRaS`, a *different* key from the other two) sets the multiplier, and is the most powerful of the four. The permanent delegate can only take back its own token, bounded by what borrowers deposited; the multiplier mints **cNGN borrowing power**, bounded only by the pool's cash and `deposit_cap` — and `deposit_cap` is denominated in raw token amounts, so it does not cap USD exposure once the multiplier moves. `MAX_MULTIPLIER = 10^6` is not the answer: an effective multiplier above the cap makes the read fail with `InvalidPrice`, which fails every health check touching the asset and seals the position exactly as an over-eager exit check would, so tightening it trades a remote economic risk for a more likely liveness failure. **Deferred:** a per-asset, admin-settable multiplier ceiling — checked at listing and again at valuation — is the shape that bounds this without the liveness cost.

### Where the mint policy runs — entry and exit

The `Standard` and `XStock` mint policies above are the **entry** policy, and they run at the two moments the protocol takes on new exposure: `list_collateral`, which creates the collateral vault, and `deposit_collateral`. Deposit re-checks rather than trusting the listing — listing is a moment, an issuer's powers are permanent — and refusing either only declines new business.

The instructions that move collateral **out** — `withdraw_collateral`, `liquidate`, `sweep_collateral_excess` — run a deliberately narrower **exit** policy: the transfer hook must name no program, and nothing else. It is not keyed on the kind, because that one clause reads the same for every mint.

The asymmetry is the point. **A check that blocks an exit can only ever trap collateral**, because the token program is already the authority on whether a transfer is legal — anything genuinely forbidden fails there without our help. A clause earns its place on the exit path only if it buys something the token program does not already enforce.

- **The hook clause does.** A hook program is arbitrary issuer CPI running inside our `transfer_checked`, which is a reentrancy surface. It also costs nothing in availability: a hooked transfer would fail in the token program anyway, for want of the hook's extra account metas, so the check only turns that into a clean `UnsupportedMintExtension`.
- **`DefaultAccountState` does not.** Token-2022 documents it as the state in which *new* accounts are initialized; flipping it to `Frozen` does not freeze accounts that already exist. No exit path creates a token account — only `list_collateral` does, for the vault — so out there the clause protects nothing, while turning an issuer action this section calls expected into a permanent seal on a live position in both directions, with no recovery short of the issuer reverting the flag: `write_off_loan` requires collateral worth less than `bad_debt_dust_usd`, and `delist_collateral` requires an empty vault.
- **`Pausable.paused` is not checked at all**, entry or exit, for the same reason (§11): a paused xStock simply fails its transfer in the token program and the liquidator picks another collateral. `DefaultAccountState` gets that treatment on the way out.
- **`ScaledUiAmount` presence stays an entry clause**: it is what makes the asset priceable. A withdrawal with no active loans reads no price at all, so requiring it on exit would block a withdrawal to protect a valuation nobody is performing.
- **The allowed-extension set is settled before the mint exists for every entry but three**: those extensions are written by `initialize_*` instructions that run on an *uninitialized* mint. The exceptions are `TokenMetadata`, `TokenGroup` and `TokenGroupMember`, each written into a **live** mint, reallocating it. Length is not what separates them — `ExtensionType::sized()` is `false` for `TokenMetadata` alone, while the two group entries are fixed-length `Pod` types added through `alloc_and_serialize`, whose documented job is to pack a fixed-length extension and realloc the account to fit. Those same three are exactly the case where re-checking on exit would trap without protecting, and none of them is dangerous: `TokenMetadata` is on both allowlists, so it changes nothing, while the two group entries are on neither, so an issuer holding the mint authority could seal every vault holding the asset by attaching a group label. Only the *values* above are worth re-reading on the way out.

## 15. Transactions

- Every instruction that runs a health check also takes the `Config` account, because `promo_cap_bps` lives there (§12). `take_loan` additionally takes the market's promo vault, optionally — it is required only when the position holds promo, since step 3 may expire it — and `liquidate` and `write_off_loan` take the promo vault and its token account on the same terms, because forfeiture moves cNGN out of it. Optional on those two as well, and for a sharper reason than convenience: `create_promo_vault` is a separate admin action (§12), so a market may legitimately have no promo vault at all, and making the accounts mandatory would make liquidation and write-off impossible on such a market — disabling the protocol's entire solvency mechanism by configuration. Required whenever the position holds promo, which cannot be true unless the vault exists, so a liquidator still cannot skip the forfeit by omitting them. `write_off_loan` therefore also takes the market mint, the market vault and the token program, which it never needed before.
- **Box every account in a large instruction, `Config` included.** The SBF stack frame is 4 KB. An unboxed `Config` was enough to make `liquidate` fail during account construction, before the handler ran.
- A health check with 8 collateral slots passes 16 price-related accounts (24 if every slot is an `XStock`, which takes three each) plus fixed accounts. Clients and liquidators must use versioned transactions with address lookup tables: measured in LiteSVM, a sponsored legacy `take_loan` fits about 6 `Standard` slots, and an all-xStock position at 8 slots exceeds the 1,232-byte packet limit. `liquidate` is tighter than `take_loan` and worth stating separately: forfeiture added two accounts, and at 8 `Standard` slots it already serialises to within ~50 bytes of the limit, so it accommodates **one** xStock slot and not two. A liquidation bot sending legacy transactions therefore needs a lookup table far sooner than a borrower does, which matters because liquidation is the backstop. Compute is not the binding constraint — that same transaction spends well under the 200,000 default. **The figure lives in `tests/budget.rs`, not here:** a spec cannot be kept honest by a test run, but a test can, and quoting a number here only produces a third copy to go stale. That file records the measurement beside the assertion that guards it, including that the fixture mint is smaller than a live xStock, so a mainnet `take_loan` reads somewhat higher.
- Pyth price updates and Switchboard feed updates must be posted in the same transaction or recently enough to meet the age limits. This is the caller's job.
- The program calls no other program except the token programs. Pyth and Switchboard data is read from accounts, so there is no re-entrancy path.

## 16. Errors

`NotWhitelisted`, `Blacklisted`, `Unauthorized`, `MarketPaused`, `CollateralPaused`, `StalePrice`, `PriceConfidenceTooWide`, `PriceAccountMismatch`, `InvalidPrice`, `UnsupportedMintExtension`, `InvalidParameters`, `Unhealthy`, `NotLiquidatable`, `UtilizationCapExceeded`, `InsufficientCash`, `NoFreeLoanSlot`, `NoFreeCollateralSlot`, `LoanNotFound`, `DepositCapExceeded`, `AmountTooSmall`, `TenureOutOfRange`, `RepaymentBelowInterest`, `ZeroPrincipalRepaid`, `ZeroShares`, `InsufficientShares`, `CollateralStillInUse`, `PositionNotEmpty`, `WriteOffNotAllowed`, `InvalidVoucherSignature`, `VoucherExpired`, `CampaignInactive`, `CampaignBudgetExceeded`, `PromoCapExceeded`, `PromoNotExpired`, `PromoVaultInsufficient`, `MathOverflow`, `MarketMismatch`, `InsufficientCollateral`.

## 17. Events

- **Lenders:** `LiquidityDeposited`, `LiquidityWithdrawn`.
- **Positions:** `PositionOpened`, `PositionClosed`, `CollateralDeposited`, `CollateralWithdrawn`.
- **Loans:** `LoanOpened`, `LoanRepaid`, `LoanPartiallyRepaid`, `LoanLiquidated`, `LoanPartiallyLiquidated`, `LoanWrittenOff`.
- **Promo:** `PromoRedeemed`, `PromoForfeited`, `PromoExpired`, `PromoRevoked`, `PromoReleased` (promo returned because the position itself is closing), `CampaignCreated`, `CampaignClosed`, `PromoVaultCreated`, `PromoVaultFunded`, `PromoVaultWithdrawn`.
- **Admin:** one event per admin instruction, carrying the old and new value.

Loan events carry position, owner, loan ID, amounts and the resulting principal, so the backend can rebuild loan history after slots are cleared.

## 18. Upgrades and Operations

- The program is upgradeable. The upgrade authority is the Squads admin multisig with a Squads time lock.
- Accounts grow only into their reserved padding; a `version` bump marks layout changes.
- `Cargo.toml` release profile sets `overflow-checks = true`. All math uses checked operations.
- If a pinned collateral feed's sponsor crank stalls (§8), `update_collateral_params` unpins it, through the timelock above, so liquidators can go back to presenting any fresh verified update. Leaving every asset unpinned from the start is not a practical fallback at scale: a liquidator would have to post one update per collateral slot, which exhausts the compute budget well before the 8-slot maximum (§15).

## 19. Testing

### 1. Math unit tests (`math/`)

- Loan balance before maturity, at maturity, after maturity, and after anchor resets.
- Penalty start after partial repayment.
- Share mint and burn rounding.
- Accrual and `R` release consistency.
- Health values across 6, 8 and 9 decimals, negative Pyth exponents, and scaled-UI multipliers.
- Seizure conversion and the cap.
- Promo counting at zero, partial and full cap.

### 2. LiteSVM program tests

- Success and every error path for every instruction.
- Pyth and Switchboard accounts written directly into state.
- Token-2022 mints with xStock extensions covering:
  - issuer pause during liquidation;
  - issuer burn from the custody vault;
  - a multiplier change;
  - a transfer hook enabled after listing;
  - an unsupported extension at listing.

### 3. Ported EVM regressions

From `lendbit-localised/test/audit/`:
- penalty clock and double-charge;
- fixed rate immutability;
- LTV versus threshold;
- dust liquidation revert;
- lifecycle solvency;
- blacklist freeze;
- guardian can't unpause;
- whitelister can't re-admit;
- donations don't move the share price.

### 4. Trident invariant fuzzing

**Actions:** every user and admin instruction, price moves, and time warps.

**Invariants checked after each action:**
- Market vault balance ≥ `cash`. Collateral vault balance ≥ `total_deposited`, except in scenarios with an issuer burn. Promo vault balance ≥ `promo_vault.cash`.
- Σ active principals = `total_borrows`, and Σ per-loan contributions = `lp_rate_product`.
- Σ position collateral per mint = `total_deposited`.
- `cash ≥ protocol_reserve`.
- `outstanding + unissued ≤ promo_vault.cash`, and Σ position promo = `outstanding`.
- A position with `debt ≤ borrow_limit` is not liquidatable.
- Loan terms never change after opening.
- A lender can withdraw up to `cash − protocol_reserve`.
- Deposit followed by an immediate withdrawal never returns more than deposited.
- The share price falls only on a write-off whose loss exceeds the reserve.
- A fuzzed attacker running many positions, redeeming promo, borrowing the maximum and defaulting never ends with more value than they put in.

### 5. Devnet

End-to-end with live Pyth devnet feeds, a real Switchboard NGN/USD devnet feed, and a Token-2022 test mint with xStock extensions: lend, redeem promo, borrow, move price, liquidate, write off, expire promo.

### 6. Before mainnet

Run the `solana-vulnerability-scanner` skill, then an external audit.

## 20. Verify Before Implementation

These facts determine exact code paths and must be confirmed first:

1. **cNGN mint on Solana** (`3jiqwBQVRC5zRwHyqvnkQurebJ5RNxg3F5fXMwaxgkv8`, from `localised-backend/docs/circle/cngn_addesses.md`): token program, decimals, extensions. If it carries a permanent delegate or pausable extension, record them as issuer risks and add them to the market's accepted list.
   **Resolved 2026-09-17 (mainnet RPC):** Token-2022, 6 decimals, extensions `permanentDelegate`, `metadataPointer`, `tokenMetadata`, freeze authority set. The market mint allowlist is those three extensions. Accepted issuer risks: the permanent delegate can move funds out of the market vault, and the freeze authority can freeze it.
2. **Pyth xStock feeds** (e.g. `Crypto.AAPLX/USD`): whether the price is per display token (after the scaled-UI multiplier) or per raw unit. §8 assumes per display token. Also whether sponsored on-chain feed accounts exist for them.
   **Resolved 2026-09-18:** per display token. Pyth publishes dedicated `Crypto.{TICKER}X/USD` feeds for the tokens themselves, plus redemption-rate feeds (`Crypto.AAPLX/AAPL.RR`) measuring their drift from the underlying share — which only makes sense if both are per share. Live, `Crypto.AAPLX/USD` ($337.42) tracked `Equity.US.AAPL/USD` ($336.29) within 0.34% while AAPLX's effective multiplier was ≈1.0033, and Backed's developer docs call the scaled amount (raw × multiplier) the one that "reflects the true equity value". Confidence medium-high: no single authoritative sentence states it. **No sponsored push account was found for these feeds**, so HODL maintains and pins the price account itself, bounded by `MAX_PRICE_AGE_SECONDS`. Also confirmed: Token-2022 does not move `new_multiplier` into `multiplier` when its timestamp passes — the consumer must compare block time and choose (§8).
3. **Mainnet xStock extensions:** the actual extension list matches §14.
   **Resolved 2026-09-18 (mainnet RPC):** AAPLX `XsbEhLAtcf6HdfpFZ5xEMdqW8nfAvcsP5bdudRLJzJp`, TSLAX `XsDoVfqeBukxuZHWhdvWHBhgEHjGNst4MLodqsJHzoB`, NVDAX `Xsc9qvGR1efVDFGLrVsmkzv3qi45LTBjeUKSPmx9qEh` — each Token-2022, 8 decimals, freeze authority set, carrying `MetadataPointer`, `TokenMetadata`, `PermanentDelegate`, `Pausable`, `ScaledUiAmount`, `ConfidentialTransferMint`, `TransferHook` (program `null`) and `DefaultAccountState` (`initialized`), and nothing else. `DefaultAccountState` was **not** in §14's allowed set; rejecting it would have rejected every real xStock, so §14 now allows it and checks its value. Full RPC output: `docs/superpowers/research/2026-09-18-xstocks-facts.md`.
4. **Switchboard On-Demand NGN/USD feed:** the source list, the result field that gives spread or standard deviation, the update cost, and who cranks it.
5. **Pyth feed IDs** for SOL/USD, USDC/USD and USDT/USD.
