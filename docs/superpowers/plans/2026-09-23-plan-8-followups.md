# Plan 8 Follow-ups

What Plan 8 leaves standing. This is the last plan in the series, so unlike Plans 2–7 there
is no "next plan" to inherit these — anything here needs its own decision.

**Branch state at close:** 281 tests (60 unit, 221 LiteSVM) passing under both the default
and `--features devnet` builds; `cargo clippy -p hodl_loans --all-targets -- -D warnings`
clean under both. This is now reproducible with the repo's own tooling —
`scripts/test.sh` forwards its arguments to `cargo build-sbf` as well as `cargo test`, so
`./scripts/test.sh --features devnet` builds and tests against a devnet-featured `.so`
instead of silently testing a devnet harness against a mainnet binary (fix-round H-1). The
prior "278 passing under both builds" claim here was never actually run that way; three
more tests (fix-round M-2) now pin that the seven stored-bump position-PDA constraints
still reject a foreign `Position`, bringing the total to 281.

---

## Blockers on a devnet deployment

### `declare_id!` does not match the deploy keypair

```
declare_id!    J9sKAhm2EhdJQ3bHeP2KUCxqZ4cYdBc65C3RDr4JjGEd   (src/lib.rs)
deploy keypair CmDBvi4ZiBDND1XzEwKFuH3kontC5Lokd2faXBxEgmci   (target/deploy/hodl_loans-keypair.json)
```

Deploying with that keypair puts the program at an address it does not believe it lives at.
Anchor's generated entrypoint compares the two before dispatching anything
(`anchor-syn-1.2.0/src/codegen/program/entry.rs:61`), so **every instruction fails
immediately with error 4100, "The declared program id does not match the actual program id"**
— no PDA derivation, no partial state, no funds at risk. The cost is a wasted deploy (≈8 SOL
of rent on an inert program) and a redeploy, not a debugging maze. An earlier draft of this
note claimed it surfaces as confusing seed errors; that was wrong. This is a key-custody
decision, not a code one — the runbook
(`docs/superpowers/runbooks/2026-09-23-devnet-deployment.md`) lays out the options as Gate A.
Nobody should deploy before it is settled.

`target/` is gitignored, so no program keypair is in version control. Whichever key is used
must be backed up outside the repo: losing it means losing the ability to upgrade.

### Devnet has none of the accounts the program needs

The larger part of a devnet deployment, and none of it is the deploy itself:

| Dependency | Status on devnet |
|---|---|
| Switchboard On-Demand NGN/USD feed | must exist under `ON_DEMAND_DEVNET_PID`, or be created |
| Pyth price accounts per collateral | devnet publishes a different set; the feed ids in `default_collateral_params` are test fixtures |
| cNGN mint | does not exist — create a Token-2022 mint with the same extension shape (`MintKind::CngnLike` is the reference) |
| Collateral mints | devnet USDC exists; Backed xStocks do not |

---

## Deferred work

### `cargo fmt` is still unadopted, for the fourth plan running

Investigated during Plan 8 and deferred again. It is **not** the mechanical cleanup the
earlier notes assumed: rustfmt ignores `max_width` inside macro bodies, so all 42 one-line
`emit!` calls explode across multiple lines, and even a tuned `rustfmt.toml` leaves ~1,196
hunks. Measurements are in the Plan 8 scratch notes.

This wants a decision rather than a fifth deferral. The realistic options are: adopt it and
absorb one large mechanical commit; adopt it with `format_macro_bodies = false` and accept
the remaining churn; or write it off explicitly and stop carrying it forward.

### The overdue-penalty sandwich needs continuous accrual

Task 7's `a_lender_cannot_sandwich_a_liquidation_for_the_penalty_step` **does not prevent the
sandwich** — it bounds it. The probe establishes that a sandwicher cannot extract more than a
pro-rata share of the penalty step they captured, and that every market invariant survives the
sequence. The actual fix is continuous penalty accrual instead of a step applied at
liquidation, which changes the interest model and needs its own plan.

Spec §11 records this as an accepted risk. The probe means a regression that *widened* the
window would now be caught.

### Trident proper was never run

Plans 2 and 3 both deferred "Trident invariants" to Plan 8. Trident could not be installed —
this environment has no outbound network access (crates.io returns 403). Task 7 delivered the
two specific properties those plans asked for as ordinary sequence tests against the existing
`assert_invariants` helper, which is what was verifiable here.

That is not a substitute for coverage-guided fuzzing. If Trident becomes installable, the
highest-value targets are the liquidation math and the share-accounting round-trip.

### `ExpirePromo` / `RevokePromo` seeds constraints are self-referential

Found during the Plan 8 fix round, while writing the regression tests for the stored-bump
change. Both sites declare:

```rust
seeds = [POSITION_SEED, position.load()?.owner.as_ref()], bump = position.load()?.bump
```

The expected address is derived from the account's **own stored fields**, so any valid
`Position` satisfies it — substituting another user's position passes the seeds check. The
actual guard is `release_promo`'s `require_keys_eq`, raising `MarketMismatch`, and the two
new tests in `tests/promo_lifecycle.rs` pin that.

**This predates Plan 8** — the pre-branch form also read `position.load()?.owner`, so the
stored-bump change neither introduced nor worsened it. It is recorded because the isolation
at those two sites comes from somewhere other than where a reader would look for it, and
because a future instruction added to that family would not inherit the chokepoint
automatically.

### Delete the tautological seeds constraints rather than keep them

The re-review's recommendation, and it is right: `ExpirePromo` and `RevokePromo`
(`lifecycle.rs:114`, `:166`) should drop their `seeds`/`bump` clauses outright instead of
carrying a constraint that implies a binding it does not have. `write_off.rs` already takes
that approach. Not done during Plan 8 — removing an access-control-adjacent constraint after
the review pass is the kind of small safe change that deserves its own cycle, and the two new
tests now pin the guard that actually holds.

**The security question is settled, and the answer is that the tautology confers no
privilege.** Both instructions are designed to operate on arbitrary positions — `expire_promo`
is permissionless, `revoke_promo` is admin-gated — so substituting position B is exactly
equivalent to calling the instruction for B directly. The seeds constraint at those two sites
was never an authorization boundary, only a misleading one. `MarketMismatch` binds position to
market, and `promo_vault`'s own `seeds = [PROMO_VAULT_SEED, market.key()]` plus `has_one =
market` compose with it transitively, so a doctored market cannot soften the expiry clock or
the pause check either.

### Dead tail assertions in the three new foreign-position tests

Same class as the review's L-4. The transaction reverted, so assertions after it cannot fail.
Harmless but misleading about what the test proves.

### Naming and duplication, carried from Plan 6

- `rescale` / `rescale_ceil` share most of their body; worth deduplicating.
- `accrue_lp_interest` / `accrued_lp_interest` differ by one character and mean different
  things. This is a trap for anyone reading quickly.

---

## Things Plan 8 measured that are worth keeping in view

- **`set_promo_cap` runs out of compute before it runs out of account locks.** 2,634 CU per
  listed asset, deterministic. 74 assets still fits under the 200,000 default budget; 75 does
  not. `MAX_LISTED_COLLATERAL` is 96, so a full list needs an explicit
  `ComputeBudgetInstruction::set_compute_unit_limit`. The 96 bound was derived from
  `MAX_TX_ACCOUNT_LOCKS` alone, which says nothing about compute.
- **Transaction size, not compute, is the binding limit on `liquidate`** — 1,185 bytes against
  the 1,232-byte legacy limit at 8 standard slots. An all-xStock position needs a v0
  transaction with an address lookup table.
- **The compute figures are comparable now, but not single-valued.** The stored-bump change
  removed a geometric `find_program_address` ladder worth up to 10,170 CU, collapsing spreads
  from ~10,500 CU to between 0 and 48. A max is worth comparing against; it never was before.

---

## Process notes worth carrying to any future plan

Plan 8 was executed subagent-driven, and **five of seven implementers contradicted the plan's
own predictions. All five were right.** Every correction is in the plan document, not just the
code, so the plan no longer teaches what it got wrong.

Four distinct ways a measurement went wrong, all found during execution:

1. **A stale `.so`.** Plain `cargo test` reuses the previously built program, so a deleted
   guard is still in the binary under test and the mutation appears to have no effect.
   Rebuild with `cargo build-sbf` before every mutation check.
2. **Early stop.** Plain `cargo test` stops launching further test binaries once one fails,
   hiding a mutation's cross-file blast radius. Use `--no-fail-fast`.
3. **Too few samples.** A 5-run sample showed zero spread where a 20-run sample shows 0–48
   CU, which turned into a false claim that the figures were deterministic.
4. **`mv`-restored backups.** `mv backup file` preserves the backup's older mtime, so Cargo's
   fingerprint check sees nothing newer than the mutated build and silently reuses the stale
   binary — a reverted test keeps failing. Restore with an editor, `cp`, or `touch` after.

Note the directions: (1), (2) and (4) make an effect look **smaller** than it is; (3) made one
look **bigger**. There is no directional bias to correct for. The common cause is insufficient
sampling and unverified builds.

The two constraints that actually caught things: requiring a mismatched test count to be
**reported rather than reconciled**, and telling implementers explicitly that contradicting
the plan is a valuable finding rather than a failure.
