# Plan 6 follow-ups

What the Plan 6 whole-branch review raised and deliberately left standing. The verdict was
`Ready to merge: Yes` with no Critical finding; the three Important items it did raise were all
corrections to the *recorded reasoning* rather than the code, and all landed before merge.

## Settled on this branch — do not re-open

- **`MAX_COLLATERAL_DECIMALS = 12`, and the reason is not "unliquidatable".** The first
  overflow in `seize_for_repayment` is at 15 decimals, measured against `u64::MAX` cNGN, a 100%
  bonus, a $0.01 collateral and an NGN price of $0.000625 — all four premises matter, and the
  fourth was missing from the first draft. Past the bound liquidation does **not** stop: the
  overflow scales linearly in `repay_amount` and a liquidator picks `amount` freely, so it
  degrades into chunked repayments from roughly 13 to 20 decimals and only becomes genuinely
  impossible past about 22. The bound exists to make the failure loud at listing rather than
  surprising a liquidator mid-liquidation, and to keep the program's other `u128` headroom
  arguments valid. `the_max_collateral_decimals_bound_is_derived_not_asserted` pins the
  arithmetic, so widening the constant fails a test rather than passing silently.
- **Oracle uncertainty rounds up; the price still rounds down.** The reason is structural, not
  just directional: one stored `price` field feeds both `lower()` (collateral) and `upper()`
  (debt), so no rounding of `price` can be conservative for both. `conf` is the only field where
  a both-sides-conservative adjustment can live at all. Worth knowing the live effect is small —
  the Pyth ceiling is a no-op for every exponent ≥ −12, so it never fires on a real USD feed, and
  the Switchboard spread moves a $1M position's debt by about $0.0016. This closed a reasoning
  hole; it did not fix a leak, and should not be described as one.
- **`lp_interest` is `#[cfg(test)]`.** `Market::accrue` → `accrue_lp_interest` is the only
  production interest path. Note the third, similarly-named `accrued_lp_interest` in
  `math/loan.rs` is a different, loan-level function and is still production.

## Plan 7 (trust boundaries and admin authority)

- **`valuation.rs`'s PDA re-derivation is now marginally more load-bearing.** `SeizureInputs`
  made the split visible at the call site: the seizure takes `collateral_decimals` and
  `bonus_bps` from the PDA-verified `ctx.accounts.collateral`, while `collateral_price` and
  `multiplier` come from the account admitted by owner + discriminator out of
  `remaining_accounts`. Sound today by the same whole-program argument as before — nothing new
  is broken — but the asymmetry now reads off the code, which is a better argument for closing
  it than the one the Plan 2 and 4 lists carry.

## Plan 8 (coverage, fuzzing, devnet)

- **`rescale` and `rescale_ceil` are near-duplicates.** Seven lines each, differing in one
  operator, and both carrying a copy of the `checked_add` exponent guard. This is the shape
  Task 1 of this very plan set out to remove — its shared-`pow10` note says two copies is "how
  the same bound came to be reasoned about twice", and the next task introduced two copies of a
  different guard. A `rescale_with(value, exponent, round_up)` with two thin wrappers keeps the
  guard in one place.
- **`accrue_lp_interest` and `accrued_lp_interest` are one letter apart**, both `pub`, both
  reachable from liquidation code. Renaming one (`accrue_market_lp_interest`,
  `loan_lp_interest_since_anchor`) is cheap now and gets harder once a third arrives.
- **`MAX_NGN_STALE_SLOTS = 150` is 60 s at the 400 ms target and up to ~100 s under sustained
  congestion**, so the NGN feed can drift somewhat further behind than the 60 s
  `MAX_PRICE_AGE_SECONDS` the collateral feeds get. The doc comment now says so. Tightening the
  number is a parameter decision, not a code one.
- **`cargo fmt` remains unadopted.** The tree has never been rustfmt-clean, inherited from
  Plan 1, and `cargo fmt --check` flags files no recent plan touched. Adopting it is worth doing
  in a commit that does nothing else, so a real change is never buried in reformatting noise.

## Method note worth keeping

This plan was executed against a **verified reference tree** — all the code was built and tested
in a scratch clone before the plan was written, and each task's output was byte-compared against
it. That caught things a review would not: a blank-line-only difference distinguished from real
drift, a declaration reorder proven semantically identical, and — once — a defect in the
*reference* rather than the implementation, when the branch's import ordering turned out to match
the codebase convention and the reference's did not.

Two cautions if it is used again. Proving a difference is cosmetic needs a method that matches
the claim: normalising whitespace while preserving order reports a deliberate reorder as a
difference, and splitting on newlines reports a line wrap as one. Both produced false alarms here.
And the reference is not automatically the better artefact — it is just the one that was tested.
