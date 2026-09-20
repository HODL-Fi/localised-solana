# Plan 5 follow-ups

What the Plan 5 reviews raised and deliberately left for later. Each item was ruled on
during execution, not merely noticed; the reasoning is here so nobody re-derives it.

## Settled on this branch — do not re-open

- **`release_promo` asserts its own market binding, and the duplicates were removed.** Two of
  its callers used to check `position.market == promo_vault.market` themselves and two did not.
  The assertion now lives in `release_promo`, which is documented as the single exit. This is
  unreachable from `take_loan` — seeds plus `has_one` plus its own pin make a foreign vault
  structurally impossible there — but it is live defence on the other callers: with it removed,
  a `close_position` naming market A's position and market B's vault succeeds and drains B's
  `outstanding`. `close_position` is the one caller whose vault account has self-derived seeds
  and no `has_one`, which is why it is the one that needs it.
- **The two promo-clock resets in `liquidate` and `write_off` were deleted, not commented.**
  They were provably inert once forfeiture zeroes `promo_balance` before them. Keeping dead code
  alive behind an explanatory comment is worse than deleting it, because the comment's truth
  depends on `forfeit_promo` staying unconditional and nothing enforces that coupling.
- **The promo cap is floored per asset, not on the aggregate.** Flooring the sum exceeds the sum
  of floors by up to n−1, and the shipped defaults have zero slack (`ltv 7000 + cap 2000 ==
  lt 9000`), so lowering the cap could make a position borrowed to exactly its limit instantly
  liquidatable — trigger 1e-12 USD, consequence a full liquidation with bonus. Do not "simplify"
  this back to a single `mul_div_floor` on `own_value`.
- **`liquidate` and `write_off_loan` take their promo accounts as `Option`.** Making them
  mandatory disables both instructions on a market with no promo vault, which
  `create_promo_vault`'s own doc comment says is a supported configuration — lenders would
  absorb every loss with no recovery path. They are required only when `promo_balance > 0`,
  which cannot be satisfied without a vault existing.
- **`redeem_promo` checks `market.paused`.** The spec omitted it. This program pauses
  exposure-increasing operations and deliberately leaves exposure-reducing ones open so users can
  always exit; redemption draws down the promo vault and raises borrowing power, so it belongs
  with `take_loan`. It is an entry check, so unlike an exit check it cannot trap anyone's funds.
- **`forfeit_promo` clamps its transfer to the balance the vault actually holds.** cNGN carries
  `PermanentDelegate`, so its issuer can move tokens out of the promo vault without the program's
  involvement. Before the clamp, `promo_vault.cash` would then exceed the real balance and the
  forfeit transfer would revert — bricking every liquidation and write-off of a promo-holding
  position on that market, a liveness failure on the solvency backstop. The clamp turns that into
  a partial recovery: lenders get what remains, `cash` falls by what moved, `outstanding` falls by
  the full amount, and the invariant survives because `amount >= moved`. An earlier draft of this
  document claimed there was no clean defence against a permanent delegate. That was wrong; the
  whole-branch review supplied one in three lines.
- **`close_voucher_receipt`'s `>` must stay strict.** `redeem` uses `now <= voucher_expiry` and
  close uses `now > voucher_expiry`; they are exact complements, and that complementarity is the
  only thing preventing cross-transaction replay. A `>=` here makes the voucher replayable in a
  loop at the boundary second, and block timestamps hold across slots, so that window is minutes.

## Plan 6 (hardening, fuzzing, devnet)

- **Seed the test harness's keypairs so compute measurements are reproducible.** `tests/budget.rs`
  figures move in ~1,500 CU steps between runs because the harness keys its mints randomly and
  where a target sorts into the collateral slot array shifts the scan. Observed spreads: 4,500 CU
  on `take_loan` and `withdraw_collateral`, 6,000 on the xStock case, and two batches produced
  xStock maxima 4,500 apart — so even 17 runs does not characterise the tail. The figures are
  currently recorded as ranges, which documents the problem rather than fixing it. Seeding
  removes it.
- **Address lookup tables for `liquidate`.** It measures 1,185 bytes against the 1,232-byte
  legacy transaction limit. At ~33 bytes per xStock collateral slot that leaves room for exactly
  one, so a liquidation bot sending legacy transactions cannot submit against a two-xStock
  position — a liveness failure on the solvency backstop. Spec §15 currently warns only about
  all-xStock positions.
- **The inactivity clock runs during a market pause.** Every write that refreshes
  `promo_last_activity_at` is either pause-gated or needs a live loan, so a pause longer than
  `promo_inactivity_seconds` lets anyone expire every idle promo balance the moment the window
  lapses — charging users for downtime the protocol imposed. Pause-gating `expire_promo` while
  leaving `revoke_promo` open is the likely answer; it superficially contradicts the
  exposure-reducing rule, but the clock measures borrower inactivity and during a pause the
  borrower cannot act, so the measurement is invalid rather than the operation unsafe. Worth its
  own decision rather than a drive-by.
- **A borrower can keep promo unrevokable by holding one minimum-size loan open.**
  `!has_active_loans()` gates both expire and revoke, and revocation exists precisely for promo
  granted in error or to an account since judged ineligible. The guard's rationale is sound —
  revocation must not force a liquidation — but a middle path exists: allow revoke under a live
  loan when the health check still passes with the promo removed.
- **`collateral_count` is unbounded, so `set_promo_cap` can become uncallable.** It must carry
  every listed asset, so past roughly thirty it no longer fits a legacy transaction and past ~250
  not even with a lookup table, freezing the cap until assets are delisted. `MAX_COLLATERAL_SLOTS`
  (8) bounds a position's slots, not the protocol's asset list, so it is the wrong bound to
  reuse — this needs its own number.
- **`Health.promo_counted` is write-only** — confirmed by the whole-branch review: nothing
  outside `compute_health` and its unit tests reads it. If it is meant for observability it
  should be emitted in an event; if not, it should be a local.
- **`voucher_expiry` is unbounded above.** A voucher signed with a far-future expiry produces a
  receipt that can never be closed and whose rent is locked indefinitely. The promo signer's
  choice, so not a vulnerability — but a `max_voucher_lifetime` bound would make the rent
  reclaimable.
- **The receipt guarantees one redemption per `(campaign, nonce)` *per expiry epoch*, not
  outright.** Expiry is stored in the receipt but is not part of its seeds, so if the backend
  reissues a nonce with a later expiry the holder can redeem the first, wait past its expiry,
  close the receipt and redeem the second. Backend nonce discipline is what prevents this; the
  comments now say so rather than claiming the stronger property.
- **A per-asset, admin-settable multiplier ceiling**, alongside `deposit_cap` — carried over from
  Plan 4 and re-deferred again. It bounds the scaled-UI authority without the liveness cost of a
  tighter global `MAX_MULTIPLIER`. `CollateralAsset` has reserved padding for the field.
- **`market` is seed-pinned in six promo account structs and left bare in six others.** All
  twelve are safe — the bare ones are pinned transitively through the promo vault's seeds plus
  `has_one = market` — but Plans 1-4 pin uniformly. Adding six PDA derivations costs compute on
  paths the branch just found tighter than advertised, for no safety gain, so it is a
  consistency item rather than a correctness one.
- **Bad-debt reporting depends on when the forfeit happened.** `write_off_loan` nets a
  same-call forfeit out of `total_bad_debt`, because that cNGN repaid this very shortfall. A
  forfeit that happened earlier, at liquidation, is not netted — it was a cash injection against
  a position that may have carried several loans, so attributing it to one loan's bad debt would
  be arbitrary. Defensible, but it means the figure's meaning varies with timing; worth settling
  deliberately if `total_bad_debt` ever gets a consumer beyond the event.

## Measurements worth keeping

All at the branch head, all non-deterministic where noted, min–max over 17 runs:

| path | CU | ceiling |
|---|---|---|
| `take_loan`, 8 standard slots, 9 loans | 74,763 – 79,263 | 95,000 |
| `withdraw_collateral`, 8 slots, 10 loans | 74,182 – 78,682 | 95,000 |
| `take_loan`, 8 xStock slots, 9 loans | 82,670 – 88,670 | 100,000 |
| `liquidate`, no forfeit | 96,162 (stable) | 120,000 |
| `liquidate`, with forfeit | 100,884 – 100,915 | 120,000 |
| `repay_loan`, 10 slots | 19,248 (stable) | 25,000 |

`liquidate` at 8 collateral slots serialises to **1,185 bytes** against the 1,232-byte legacy
limit. `tests/budget.rs` pins both the compute and the size, and is the only place either is
written down — the spec points at the test rather than quoting it, because a spec cannot be kept
honest by a test run and a test can.
