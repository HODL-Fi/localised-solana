# Plan 4 follow-ups

What the Plan 4 reviews raised and deliberately left for later. The whole-branch review's verdict was `Ready to merge: With fixes`; every Critical and Important finding was fixed on the branch, and the scoped re-review of that fix wave returned `Ready to merge: Yes`. Everything below is a decision or a hardening item.

## Settled on this branch — do not re-open

- **The mint policy is asymmetric, and deliberately so.** `require_collateral_mint_on_entry` runs at `list_collateral` and `deposit_collateral`; `require_collateral_mint_on_exit` runs at `withdraw_collateral`, `liquidate` and `sweep_collateral_excess` and checks only that a transfer hook names no program. The first draft re-checked the full policy everywhere, which meant that the moment Backed flipped `DefaultAccountState` to `Frozen` — an action spec §20 records as expected — every xStock position sealed in both directions with no recovery path. `DefaultAccountState` governs the state of *newly created* accounts and none of the exit instructions creates one, so the check protected nothing while trapping collateral. A check that blocks an exit can only ever trap collateral; the token program is already the authority on whether a transfer is legal.
- **`MAX_MULTIPLIER` stays at 10^6 × `MULTIPLIER_SCALE`.** Tightening it looks like cheap protection against the scaled-UI authority and is not: an effective multiplier above the cap makes `read_xstock_multiplier` return `InvalidPrice`, which fails every health check touching the asset and seals the position the same way an over-eager exit check would. That trades a remote economic risk for a likelier liveness failure. The reasoning is recorded next to the constant.
- **The exit policy dropped the `Standard` allowlist too**, not only the xStock clauses. A `Standard` mint cannot gain a `TransferHook` after initialisation, so the allowlist was a no-op on exit — except against the three extensions an issuer can realloc into a live mint (`TokenMetadata`, `TokenGroup`, `TokenGroupMember`), where a cosmetic token-group label would have sealed every vault holding the asset. Removing a trap, losing nothing.

## Plan 5 (promo)

> **Re-deferred 2026-09-18:** Plan 5 did not take the multiplier ceiling. It is a `CollateralAsset` change rather than a promo one, and promo's own caps (`promo_cap_bps`, `max_promo_per_position`) turned out to share none of its machinery. It moves to Plan 6.

- **A per-asset, admin-settable multiplier ceiling**, alongside `deposit_cap`. This is the shape that bounds the scaled-UI authority without the liveness cost of a tighter global cap: a governance action would be required before an asset's valuation could move by more than the admin sanctioned. `CollateralAsset` has reserved padding for the field. Spec §14 defers it explicitly.
- Promo forfeiture on liquidation and write-off (spec §11 step 3) and promo in the health check, as already recorded in the Plan 2 and Plan 3 follow-ups.

## Plan 6 (hardening, fuzzing, devnet)

> **Partly resolved 2026-09-21 by Plan 6** (`2026-09-20-plan-6-math-bounds-and-shape.md`):
> `seize_for_repayment`'s eight positional arguments are now a `SeizureInputs` struct and the
> `#[allow(clippy::too_many_arguments)]` is gone, so `collateral_price` and `multiplier` can no
> longer be transposed silently. The `withdraw_collateral` CU headroom item was already closed by
> Plan 5, which raised that ceiling to 95,000. The `valuation.rs` PDA re-derivation and the
> missing per-asset borrow pause go to Plan 7; the xStock fixture's real `TokenMetadata` to
> Plan 8. The pathological-asset overflow and the `CollateralListed.kind` field order remain
> documented rather than fixed.

- **Re-derive the `CollateralAsset` PDA in `valuation.rs`.** It admits the account by owner + discriminator + stored mint rather than re-deriving `["collateral", mint]` with the stored bump. That is sound today only because of a *global* argument — the program creates these accounts at that PDA and nowhere else — and since Plan 4 the same account's `kind` also decides how many accounts the health walk consumes, so the assumption carries more weight than when Plan 2 first recorded it. Two lines and one hash of compute converts a whole-program argument into a local one. Raised out of the Plan 2 list by the Plan 4 whole-branch review.
- **`take_loan` does not re-check the collateral mint**, so new debt can be drawn against an asset whose transfer hook the issuer has since switched on — the token program will refuse to seize it, but borrowing continues. The obvious fix (checking inside `load_collateral_values`) is wrong: it would re-seal `liquidate` and `write_off_loan`. What is missing is an admin action that stops *new borrowing* against one asset; `set_collateral_paused` blocks deposits only, and `ltv_bps` cannot go below its 1,000 floor.
- **The `MintKind::XStock` fixture is one extension short of live:** it initialises `MetadataPointer` but never writes a real `TokenMetadata` extension, though the allowed set names it. `try_calculate_account_len` cannot size a variable-length extension, so the fix is extra space plus `token_metadata_initialize` after `initialize_mint2`. Worth doing before the devnet run, since the compute figures in `tests/budget.rs` are measured against a mint materially smaller than the real thing and a mainnet `take_loan` will read higher.
- **`seize_for_repayment`'s eight positional arguments**, with `#[allow(clippy::too_many_arguments)]`: `collateral_price` and `multiplier` are both `u128` at the same 10^12 scale, so transposing them compiles cleanly. One unit test would now catch it, but a `SeizureInputs` struct or a multiplier newtype would make it impossible — worth doing before a ninth argument arrives.
- **`withdraw_collateral` sits at 70,170 CU against a 75,000 ceiling** — about 7% headroom, the tightest of the four budget assertions, and pre-existing. Raise the ceiling on a future change rather than as a drive-by.
- **A pathological asset is unliquidatable rather than slot-capped.** `seize_for_repayment`'s unconditional `× MULTIPLIER_SCALE` returns `MathOverflow` above `display ≈ 3.4 × 10^26`, where the pre-multiplier code fell through to the slot cap. It fails closed and needs a collateral priced under ~3 × 10^-8 USD with a near-`u64::MAX` repayment, so it is documented in `math/liquidation.rs` and spec §11 rather than fixed.
- **`CollateralListed.kind` was inserted before `params`**, changing the event's field order. Harmless today — there is no off-chain consumer in the repo — but append rather than insert once indexers exist.

## Measurements worth keeping

Measured in LiteSVM at the branch head, all against the 200,000 CU default: `take_loan` at 8 xStock slots with 9 existing loans, **72,533 CU**; the same position's legacy transaction serialises to **1,346 bytes**, past the 1,232-byte packet limit, so an all-xStock position needs a v0 transaction with an address lookup table. `tests/budget.rs` pins both and is the only place either number is written down — spec §15 points at the test rather than quoting a figure, because a spec cannot be kept honest by a test run and a test can.
