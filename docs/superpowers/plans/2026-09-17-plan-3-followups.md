# Plan 3 follow-ups

What the Plan 3 whole-branch review raised and deliberately left standing or pushed further out. The review found no Critical defect (`Ready to merge: Yes, with fixes`); the fixes it did call for landed in this same fix wave — see `.superpowers/sdd/2026-09-17-plan-3-liquidation-and-bad-debt/final-fix-report.md`. Everything below is a decision already made, or a hardening item deferred to a later plan.

## Plan 4 (xStocks)

- `seize_for_repayment` (`math/liquidation.rs`) needs the xStock scaled-UI multiplier. Spec §11 step 5's formula already carries `multiplier` in the seizure divisor, but the function takes no multiplier parameter yet — it's the one site outside `valuation.rs` that prices collateral, and it will need the same `ScaledUiAmountConfig` read Plan 4 adds there.

## Plan 6 (hardening, fuzzing, devnet)

- **Collateral `decimals` bound, derived from the liquidation path.** The health path's `token_value`/`token_value_ceil` tolerate collateral decimals up to roughly 38 before overflowing `u128`. The liquidation path is tighter: `seize_for_repayment`'s `with_bonus × 10^collateral_decimals` term overflows around 24 decimals. `CollateralParams::validate` should enforce the bound the liquidation path actually needs, not the more permissive one the health path tolerates.
- **`INIT_SPACE` guards for `CollateralAsset` and `Market`.** Both now carry fields taken from their reserved padding (`price_account`, `accrual_remainder`), and neither has a test pinning `Market::INIT_SPACE` / `CollateralAsset::INIT_SPACE` (or the account's actual byte size) the way `position.rs`'s `layout_is_fixed_size_without_padding_surprises` pins `Position`. Add the same guard for both.
- **The permissionless overdue-penalty step.** Spec §11's accepted-risk note (added this fix wave) documents that the penalty only reaches lenders as a step, released at repayment or liquidation, and that `liquidate` having no access check lets a lender deposit, trigger the step via someone else's liquidation, and withdraw in one transaction — diluting honest lenders' share of penalty income without threatening solvency. Fix it by accruing the penalty into `lp_rate_product` continuously instead of releasing it as a step, or by amortising the release.
- **Shared `pow10` helper.** `math/liquidation.rs` and `math/price.rs` each define their own `pow10` (different signatures — `u8 → u128` vs `u32 → u128`). Extract one into `math/checked.rs`.
- **A vault-mismatch error variant.** `Liquidate`'s `collateral.vault == collateral_vault.key()` constraint reports `PriceAccountMismatch` on failure — a vault substitution, not a price problem. Give it its own error variant.
- **A `ZeroPrincipalRepaid` test.** No test exercises it, but it is reachable: a low-priced, high-decimal collateral slot can produce `seize ≥ 1` (so `AmountTooSmall` never fires) while `principal_share` still floors the requested repayment to 0.
- **`write_off_loan` test gaps.** No test covers `MarketMismatch`, and no test writes off one loan on a position with a surviving, still-active sibling loan and asserts the sibling is left untouched.
- **Trident invariants:** add a probe for two consecutive partial liquidations on the same position (the convergence property this fix wave's `liquidation.rs` test now pins by hand), and a probe for the deposit-liquidity → liquidate → withdraw-liquidity sandwich the new §11 accepted-risk note describes.

## Recorded decisions

- **A blacklisted wallet may liquidate.** Spec §13 lists `liquidate` under "Anyone" — `liquidate` has no access check (§11), so a blacklisted wallet can still call it as the liquidator, even though it cannot call any whitelisted-user instruction. This is the existing, intentional design: §13 already notes that "anyone can liquidate" a blacklisted borrower; this entry records that the liquidator side of that same sentence was reviewed and is accepted too.
- **The slot-cap rounding sliver is deliberate.** When `seize_for_repayment`'s slot-cap branch binds, `seize_amount` is set to the exact remaining slot while `capped_repayment` is floored — so the liquidator pays fractionally less than the exact proportional share for that last, whole-slot seizure. This hands the liquidator a sub-base-unit rounding sliver once per slot-capped liquidation call. It is the same round-down discipline the function applies everywhere else, just landing on the repayment instead of the seizure in this one branch, and is accepted as dust.

## Cosmetic, picked up whenever the file is next touched

- `programs/hodl_loans/tests/liquidation.rs`, in `two_consecutive_partial_liquidations_converge_to_healthy`: a comment labels the closing rate "$0.035 per cNGN" where the value is $0.000035. The arithmetic that follows it uses the right magnitude, so only the label is wrong.
