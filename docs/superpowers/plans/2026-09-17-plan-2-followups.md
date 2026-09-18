# Plan 2 follow-ups

What the Plan 2 reviews raised and deliberately left for later. The whole-branch review found no Critical or Important defect in Plan 2's code (`Ready to merge: Yes`); everything below is a decision or a hardening item.

## Decide before Plan 3 (liquidation and bad debt)

> **Resolved 2026-09-17 by Plan 3** (`docs/superpowers/plans/2026-09-17-plan-3-liquidation-and-bad-debt.md`): item 1 pins an admin-set price account per collateral asset and caps the fallback age at 60 s; item 2 keeps every new Plan 3 arithmetic site checked and leaves the existing guarded operators to the Plan 6 sweep; item 3 is implemented (liquidation and write-off release `R` per loan); item 4 is documented in spec §11 as accepted. Items 5 onward (Plan 4/5/6 sections below) still stand.

1. **Pyth price-update selection.** `read_pyth_price` accepts any receiver-owned, fully verified `PriceUpdateV2` for the feed within `max_price_age_seconds`, and `CollateralParams::validate` puts no upper bound on that age. A caller can therefore choose the most favourable update in the window (~150 updates in 60 s). In Plan 2 the gain is bounded by the gap between 70% LTV and the 90% threshold; in Plan 3 it becomes "liquidate at the worst price in the window". Options: pin a price account per `CollateralAsset` (e.g. Pyth's sponsored push feed, which also removes one account per slot), or cap `max_price_age_seconds` in `CollateralParams::validate` and record the residual risk in spec §15.
2. **Checked-arithmetic convention.** Several guarded raw operators remain: `take_loan` (the utilization `*`, `cash -= amount`), `repay_loan` (`paid - due`, `principal -=`), `withdraw_collateral` (`slot.amount -=`), `harvest_reserve` (`protocol_reserve -=`), and `CollateralParams::validate` (u32 `+`, u128 `*`). Each is guarded by a `require!`/`min` on the previous lines or bounded by operand type, and `overflow-checks = true` turns any missed case into an abort rather than a wrap. Liquidation will add more of the same shape, so either convert them all to `checked_*` (mechanical sweep, Plan 6) or add a "type- or require-guarded raw operator" exception to the Global Constraints, since the plan's own code contradicts "all math uses checked operations" as written.
3. **Write-off and liquidation must release `R` per loan** exactly as `repay_loan` does, or `accrued_interest` drifts.
4. **Overdue penalty reaches lenders as a step at repayment** (the market accrues only `rate × (BPS − rf)`). A lender can deposit just before a large overdue repayment and withdraw just after, taking a pro-rata share; the amount is bounded. Accept and document, or reconsider alongside the Plan 3 write-off semantics (the mirror case).

## Plan 4 (xStocks)

> **Resolved 2026-09-18 by Plan 4** (Task 1): `tests/withdraw.rs::a_token_2022_standard_asset_moves_through_the_same_paths` deposits, borrows against and withdraws a metadata-only Token-2022 asset (`MintKind::Token2022Plain`).

- No Token-2022 standard-collateral withdrawal test yet; add one with the Token-2022 collateral path.

## Plan 5 (promo)

> **Resolved 2026-09-18 by Plan 5** (`docs/superpowers/plans/2026-09-17-plan-5-promo-balance.md`): all three items. Expiry runs inside `take_loan` (Task 6), promo is counted in the health check (Task 5), `close_position` releases it (Task 6), the three vault sweeps now share `sweep_to_treasury` (Task 1), and `the_promo_clock_restarts_only_when_the_last_loan_closes` covers the timestamp case.

- Promo expiry inside `take_loan` (spec §10 step 3), promo counted in health, promo release in `close_position`.
- `SweepMarketExcess` and `SweepCollateralExcess` are parallel near-duplicates; the promo vault sweep would be a third copy, so extract a shared helper then.
- Test: promo timestamp unchanged while another loan is still active.

## Plan 6 (hardening, fuzzing, devnet)

- **Switchboard:** add an owner check against the On-Demand program ID, and either assert `permit_write_by_authority == 0` or record the feed authority as an accepted trust assumption (spec §20 item 4).
- **Supply chain:** `switchboard-on-demand` pulls `switchboard-protos` (vendored `protoc` binaries at build time), `prost`, `libsecp256k1`, `rust_decimal` and borsh 0.9 just to read a 128-byte struct. Consider vendoring `CurrentResult`, its offset and the discriminator, keeping the crate as a dev-dependency with a test pinning size/offset/discriminator.
- **Parameter bounds:** collateral `decimals` (above 38 every health check fails with `MathOverflow`), `max_price_age_seconds`, `ngn_max_stale_slots`.
- **Checked-math sweep** per decision 2 above, including `math/price.rs::rescale`'s unchecked `exponent + USD_DECIMALS`.
- **`math/interest.rs::lp_interest`** is now used only as the test oracle for `accrue_lp_interest`; mark it `#[cfg(test)]` or note it.
- **Rounding:** `scale_switchboard_value` rounds the spread down, so the debt price can land ~3e-9 relative below exact, against "debt rounds up"; rounding the spread up would be strictly conservative.
- **Test gaps:** `LoanOpened`/`LoanRepaid` event contents (no test decodes events), never-whitelisted borrower, no-collateral borrow, vault/`owner_token` substitution, non-zero `protocol_reserve` in the utilization-cap test, third-party repayment with interest due, partial repayment after maturity (integration), withdrawal after full repayment with no market accounts, correct market with a wrong `ngn_feed`.
- **Trident invariants** must tolerate the per-loan `R` rounding dust (measured: 26 atoms over 53 repayments).
- **`valuation.rs`** matches a `CollateralAsset` by owner + discriminator + stored mint rather than re-deriving `["collateral", mint]`. **Raised in priority by Plan 4:** that same account's `kind` now decides how many accounts the health walk consumes, so the soundness argument is doing more work than it was. See the Plan 4 follow-ups. Equivalent today because listing is the only creation path; either document that invariant or re-derive with the stored bump (~1.5k CU per slot, affordable).
- **`harvest_reserve` and the sweeps don't accrue** first, unlike every other market-touching instruction. No economic effect (`cash` and `protocol_reserve` fall together). Either call `accrue` for uniformity or narrow the spec §9 wording.
- **`take_loan` check order:** `MarketMismatch` and the free-slot check run after accrual and the cap, so a wrong-market borrow at full utilization reports `UtilizationCapExceeded`. Only the reported error differs; a five-line reorder matches spec §10.
- Decide whether to adopt `cargo fmt` (the tree is not rustfmt-clean, inherited from Plan 1).

## Measurements worth keeping

From the whole-branch review, with 8 collateral slots and 9 existing overdue loans: `take_loan` 65.9k CU, `withdraw_collateral` 69.5k CU, `repay_loan` 19.2k CU — roughly a third of the 200k default, each extra active loan about +1,100 CU. `tests/budget.rs` now pins these; `tests/invariants.rs` pins the accounting invariants across multiple loans and partial repayments.

A sponsored (two-signer) legacy `take_loan` fits about 6 used collateral slots; 7 or more need v0 transactions with address lookup tables, before any price-update instructions are added. Worth stating in spec §15 or the client docs.
