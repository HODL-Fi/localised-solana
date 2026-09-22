# Plan 7 follow-ups

What Plan 7 (trust boundaries and admin authority) deliberately left standing: the decisions
that will read as defects to someone meeting them cold, and the items Plan 8 inherits.

## Settled on this branch — do not re-open

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
