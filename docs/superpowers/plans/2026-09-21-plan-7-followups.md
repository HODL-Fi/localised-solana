# Plan 7 follow-ups

What Plan 7 (trust boundaries and admin authority) deliberately left standing: the decisions
that will read as defects to someone meeting them cold, and the items Plan 8 inherits.

## Settled on this branch — do not re-open

- **Zeroing an asset's `ltv_bps` does not withhold its borrowing power, and this is the most
  important thing on the branch.** `compute_health` (`math/health.rs`) accumulates two
  independent things from each holding's value: the LTV term, and `promo_cap_total` — a
  fraction of the holding's *value* that never consulted `ltv_bps` — and `promo_counted` is
  then added straight back into `borrow_limit`. A first draft of the borrow pause and the
  multiplier ceiling gated only the LTV term, so a fully borrow-paused asset still unlocked up
  to `min(promo_value, promo_cap_bps × value)` of borrowing power: 1,000 USDC paused with $150
  of promo held still let a $150 loan through, against the asset the guardian had just paused.
  Worse for the multiplier ceiling, where `own_value` is computed at the true inflated
  multiplier by design, so the scaled-UI authority could inflate the promo cap through the very
  channel the ceiling exists to close. Lenders were never short — the extra borrowing is backed
  1:1 by promo-vault cNGN — but an emergency brake did not brake, worst exactly when it matters,
  since the admin pauses *because* the feed is unreliable.

  `CollateralValue::lends_borrowing_power` is read at both sites and nowhere else.
  **It is a property of two call sites, not of the type:** anything new that raises
  `borrow_limit` must be gated too, and no test enumerates the gate sites, so nothing will
  catch a third contributor added ungated.

  **And the first fix for it was itself wrong, which is the part most worth remembering.**
  `promo_counted` fed *both* limits from one total, so gating that total dropped
  `liquidation_line` as well — meaning a guardian pause, or the scaled-UI authority crossing
  its ceiling with no protocol action at all, made a live loan liquidatable with no price
  movement, and a liquidator took the bonus while forfeiture handed over the borrower's promo.
  The whole-branch review caught it; the branch now accumulates the promo cap **twice**, once
  over all holdings for the line and once over lending holdings for the limit. The line's lift
  is justified by forfeiture — seizure returns the full, uncapped promo balance — and that
  argument never depended on the pause. `own_value` also decides whether `write_off_loan` may
  treat a position as dust, so it stays ungated too.

  Promo-free fixtures cannot see any of this. The tests that can are
  `pausing_borrowing_must_not_drop_the_liquidation_line_of_a_promo_holding_position` and
  `a_borrow_paused_asset_unlocks_no_promo_either` (both `promo_health.rs`), plus the
  `compute_health` unit test — whose earlier version asserted the *dropped* line as correct,
  under a comment denying the drop could happen.

- **`MAX_LISTED_COLLATERAL` is derived from `MAX_TX_ACCOUNT_LOCKS` (128), not the `u8` account
  index (256).** Address lookup tables relieve message *size*, not the lock limit, so the index
  ceiling is never reached; minus the three accounts `set_promo_cap` always needs, the real
  ceiling is 125 and the bound is 96. The first draft used the index limit and landed on 128 —
  *above* the real ceiling, which would have permitted an asset list that makes `set_promo_cap`
  permanently unsendable, the exact state the bound exists to prevent, with no way back since
  `collateral_count` falls only on delisting.
  `the_asset_list_bound_keeps_set_promo_cap_inside_the_lock_limit` (`constants.rs`) pins the
  arithmetic so it cannot drift back.

- **`expire_promo` is the one promo operation a pause blocks, and it is not an exception to
  the exposure-increasing rule — it reads backwards against it.** Every other pause guard in
  this codebase exists to stop a borrower from taking on more exposure while the market is
  paused; `expire_promo` takes exposure *away* (it releases promo borrowing power) and still
  gets barred. The reason is not "unsafe operation" but "invalid measurement": the
  instruction's precondition is `now ≥ max(promo_last_activity_at,
  market.promo_clock_resumed_at) + promo_inactivity_seconds`, a claim about how long the
  borrower has been idle, and a paused market is one the borrower cannot act on — so the
  clock cannot honestly be read as idleness during the pause. `revoke_promo` makes no such
  measurement (it is the admin retracting a grant, not a claim about the borrower's
  behaviour), so it stays open through a pause; `revoke_stays_open_during_a_pause_because_it_
  measures_nothing` pins that asymmetry directly. See the comment on `handle_expire_promo`
  (`lifecycle.rs`) and spec §12 "Expiry".

- **The promo clock is market-global, so repeated pause cycles keep resetting it for every
  position.** Unpausing sets `market.promo_clock_resumed_at = now` on the true→false edge, and
  `expire_promo`'s deadline runs from whichever of that and the borrower's own last activity is
  later. Two unrelated pause incidents inside one inactivity window mean nothing expires at
  all, for any position on the market, even ones untouched by either incident. Left as is —
  the alternative is per-position accounting of paused time, which is real complexity for a
  cost that only ever favours the borrower (it keeps promo committed longer, not less).
  `two_pause_cycles_each_restart_the_clock_for_every_position` pins the behaviour so a future
  change to it is a deliberate decision, not a silent regression.

- **`revoke_promo`'s guarantee is now conditional on admin-set parameters, backed by spec
  §18's timelock rather than by the program.** Task 8 replaced the old unconditional "never
  touch a position with a live loan" rule with a post-release `is_healthy()` check — deliberately,
  since the unconditional rule let any borrower make their promo permanently unrevokable by
  holding one minimum-size loan open. `is_healthy()` reads `ltv_bps`, `liquidation_threshold_bps`
  and `max_conf_bps` off `CollateralAsset`, and the same admin key that signs `revoke_promo` also
  signs `update_collateral_params` with no in-program timelock on that instruction. Three
  instructions in one transaction — raise the LTV, revoke, restore the LTV — can revoke a promo
  that a fairly-configured health check would have refused, leaving the borrower promo-less and
  liquidatable. Nothing on-chain prevents that sequence; what stands between it and a borrower is
  operational (spec §18's timelocked multisig around the admin key itself), not a program
  invariant. The trade was made deliberately rather than left unnoticed — see the doc comment on
  `handle_revoke_promo` (`lifecycle.rs`) and spec §12's `revoke_promo` bullet, both of which say
  this in the same terms.

- **The voucher-expiry bound rejects previously valid vouchers and needs a backend
  migration.** `redeem_promo`'s `voucher_expiry ≤ redeem_until` check (spec §12, `redeem_promo`
  step 3) means a voucher signed to expire after its campaign's `redeem_until` — valid at the
  moment it was signed, if `redeem_until` was later extended downward or the voucher was signed
  against stale campaign data — now fails at redemption. This is upstream of Task 8 (it lives in
  `redeem_promo`, not `revoke_promo`), but it is exactly the kind of thing that reads as a
  regression to someone who signed vouchers under the old assumption. A backend issuing rolling
  vouchers must clamp `voucher_expiry` to the live campaign and re-sign anything already
  outstanding; no code change closes this, since the fix is operational (backend re-signing
  policy), not on-chain.

- **Reconciling a clawback is not reversed by the issuer returning the tokens — the return
  reads as a donation and sweeps to the treasury.** `reconcile_promo_vault` only ever writes
  `cash` down (never up), so once a `PermanentDelegate` clawback has been reconciled, tokens the
  issuer subsequently sends back to the promo vault's token account are indistinguishable from an
  ordinary donation: `sweep_promo_excess` moves `vault balance − cash` to the treasury, and the
  returned amount is exactly that excess. `a_clawback_then_return_moves_promo_budget_to_the_
  treasury_once_reconciled` pins this. An admin who reconciles a clawback they expect to be
  reversed should wait for the return before reconciling — reconciling first and expecting the
  budget to come back is the trap. Documented on `reconcile_promo_vault` (spec §12) and on the
  instruction's own doc comment.

- **The per-slot PDA re-derivation costs ~1,587-1,588 CU per collateral slot, and the budget
  markers were raised to accommodate it.** `load_collateral_values` (`valuation.rs`) now calls
  `Pubkey::create_program_address` once per used collateral slot to prove an admitted
  `CollateralAsset` account is the canonical `["collateral", mint]` PDA rather than trusting
  owner + discriminator + stored mint alone (needed since Plan 4, when the same account's `kind`
  started deciding how many accounts the walk consumes). `tests/budget.rs`'s thresholds were
  re-measured and raised to 115,000 CU across the affected paths (`take_loan`,
  `withdraw_collateral`, `revoke_promo` under a live loan is on the same walk); the program-wide
  default budget is 200,000 CU, so there is real headroom left, but a future plan that adds
  another per-slot syscall should re-measure rather than assume it.

## Method note

Task 8's health check is the only place in this plan where a guarantee's class visibly changed —
every other Plan 7 fix tightened an existing check (a missing modifier, a missing revalidation).
The doc comment on `handle_revoke_promo` and the matching spec §12 paragraph both name the
weakened guarantee explicitly rather than only describing the new happy path, which is why they
read almost identically: the spec was written to already describe this change (it landed in the
plan's own commit before Task 8 ran), and Task 8's implementation was checked against it rather
than the other way around.

## Needs action outside this repo

- **`CollateralParams` gained a field, so `list_collateral` and `update_collateral_params`
  instruction data grew by 16 bytes.** Append-only Borsh ordering protects *account* layout —
  which is what the `reserved` arithmetic is for — but it does not make an instruction-argument
  change non-breaking: a deployed caller sending eight-field params now fails to deserialize
  because the buffer is short. There are no in-tree consumers (no `*.ts`, no checked-in IDL
  outside `target/`), so nothing here needs updating, but deploy and admin tooling does.

## Plan 8 (coverage, fuzzing, devnet)

- **The liquidate paths' per-slot delta is ~81 CU below the other walking paths** —
  ~1,506-1,510 against ~1,587, about 650 across eight slots. The whole-branch review supplied
  the likely mechanism: every CU/slot figure in `budget.rs` is one 8-slot total divided by
  eight, and nothing varies the slot count, so nothing separates a true per-slot term from a
  fixed per-instruction one. Varying the slot count would settle it. (An earlier version of
  this note called the gap "a few hundred CU per slot", which was 4x wrong — it is a few
  hundred in total.)
- **No test in the repo asserts any event.** Not a regression — it is the existing convention,
  confirmed by grep — but Plan 7 added two (`CollateralBorrowPauseSet`, `PromoVaultReconciled`)
  and neither is covered.
- **Seed the harness *borrower* keypairs, and consider storing the position bump.** The
  ~1,500 CU stepping is not the collateral: `TakeLoan` and `WithdrawCollateral` declare the
  position PDA with a bare `bump`, so Anchor emits `find_program_address` and pays ~1,500 CU
  per candidate bump tried. A random borrower's canonical bump is 255 with p≈1/2, 254 with
  p≈1/4 — a geometric ladder with an unbounded tail, not a spread bounded by eight slots.
  `Liquidate` and `RepayLoan` constrain the position by no seeds at all, which is why their
  figures are stable to ~31 CU. Seeding fixes the measurement; storing the bump and using
  `bump = position.bump` would remove the cost itself, on the program's hottest path. Plan 7
  spent real effort on bucket-coverage arguments that either change makes unnecessary —
  including one review finding that was itself an artifact of unequal bucket coverage.
- **`cargo fmt` remains unadopted** — 2,106 `Diff in` hunks repo-wide, overwhelmingly
  pre-existing and inherited from Plan 1. Worth a commit that does nothing else, so a real
  change is never buried in reformatting noise.
- Carried from the Plan 6 follow-ups and still open: `rescale`/`rescale_ceil` are
  near-duplicates; `accrue_lp_interest` and `accrued_lp_interest` are one letter apart and both
  reachable from liquidation; `MAX_NGN_STALE_SLOTS = 150` is a parameter decision rather than a
  code one.

## Not in Plan 7 — needs its own plan

- **The overdue-penalty continuous accrual.** Spec §11's accepted-risk note records that the
  penalty reaches lenders as a step released at repayment or liquidation, and that `liquidate`
  having no access check lets a lender deposit, trigger the step through someone else's
  liquidation, and withdraw in one transaction — diluting honest lenders' share of penalty
  income without threatening solvency. Fixing it means accruing the penalty into
  `lp_rate_product` continuously, or amortising the release. That changes how interest reaches
  lenders, which is too large to attach to a hardening plan.
