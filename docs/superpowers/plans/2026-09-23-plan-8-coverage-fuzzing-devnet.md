# Plan 8: Coverage, Fuzzing and Devnet Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the coverage gaps the Plan 2–5 reviews recorded, add the two sequence probes those reviews asked a fuzzer for, and make the program deployable to devnet — which it currently is not.

**Architecture:** Mostly tests, but three tasks change the program. One is a real performance fix (seven instructions searched for a PDA bump the account already stored). One makes the Switchboard program id cluster-selectable, without which every health check fails on devnet. One extends the test harness so events can be decoded at all — a capability the suite has never had.

**Tech Stack:** Anchor 1.2.0, anchor-spl `token_interface`, LiteSVM 0.10.0 with the `precompiles` feature, `cargo build-sbf --tools-version v1.52`.

**Spec:** `docs/superpowers/specs/2026-09-17-solana-fixed-loans-design.md`

**Sources:** `docs/superpowers/plans/2026-09-17-plan-{2,3,4,5}-followups.md`, `2026-09-20-plan-6-followups.md` and `2026-09-21-plan-7-followups.md`. This is the last plan in the series.

> **The recorded gap list was stale, and checking it first is part of the work.**
> Fourteen coverage gaps were named across those files. **Six were already closed**
> by Plans 5–7 without the notes being updated — `ZeroPrincipalRepaid` was recorded
> as open and *is*, but "never-whitelisted borrower", "no-collateral borrow",
> "vault substitution", "third-party repayment", "partial repay after maturity"
> and "withdrawal after full repayment" all had tests by the time this plan
> started. Two gaps nobody had written down turned up instead: `MathOverflow` was
> the most-raised error in the program with nothing asserting it, and
> `PromoVaultMismatch` was raised by code no test reached.
>
> Task 1 re-derives that list rather than trusting it. Do not skip it.

## Global Constraints

- Anchor 1.2.0. Build and test with `./scripts/test.sh`, which rebuilds the SBF program first. **Plain `cargo test` reuses a stale `.so`** and will pass against code you have just changed. `cargo test --lib` is safe for unit tests alone; anything touching LiteSVM needs the rebuild.
- **"Green" means no failures AND no compile errors.** Grepping for `FAILED` alone misses a test binary that did not compile — check for `error[` too.
- **Never restore a mutated file with `mv backup file`.** `mv` preserves the backup's original mtime, which is *older* than the mutated build Cargo fingerprinted — so Cargo silently reuses the stale binary and the "reverted" run still reports the mutation's failure. Restore with an editor, `cp`, or `touch` the file afterwards. Task 8 hit this and only noticed because a reverted test kept failing.
- **Run mutation checks with `cargo test -p hodl_loans --no-fail-fast`.** Plain `cargo test` stops launching further test binaries once one reports a failure, so a mutation whose blast radius crosses files looks smaller than it is. Task 3 found a second failing test this way that an earlier measurement had missed. Combine with the rebuild rule above: `cargo build-sbf --tools-version v1.52 && cargo test -p hodl_loans --no-fail-fast`.
- All arithmetic is checked: no raw `+ - *` on values that could overflow, no `unwrap()` on arithmetic, no bare `as` narrowing casts.
- `cargo clippy -p hodl_loans --all-targets -- -D warnings` clean, and also clean under `--features devnet` once Task 8 lands. Do not silence a lint with `#[allow]`.
- New `HodlError` variants are **appended**, never inserted — codes are `6000 + position`.
- Every commit message ends with:
  `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`
- Write commit messages with `git commit -F -` and a heredoc when they contain backticks; a double-quoted `-m` gets them command-substituted.
- Stage only the files your task touches. No `git add -A`, no `git commit -a`.
- The suite is **266 tests** (59 unit, 207 LiteSVM) at the start of this plan and **278** (60 unit, 218 LiteSVM) at the end.

| After task | Total | Unit | Why |
|---|---|---|---|
| 1 | 266 | 59 | reading only, no change |
| 2 | 269 | 59 | +3 |
| 3 | 272 | 59 | +3 |
| 4 | **273** | 59 | +1 only — Step 1 *replaces* a unit test, so the unit count cannot move |
| 5 | 274 | 59 | +1 |
| 6 | 274 | 59 | +0 — re-measures, adds no test |
| 7 | 277 | 59 | +3 |
| 8 | 278 | **60** | +1, and it is a unit test (`constants.rs`) — this is where 60 arrives |
| 9 | 278 | 60 | docs only |

  **If your count does not match, report it — do not delete a test to reach the number.** An
  earlier draft of this table was read off the reference tree's *commit* sequence rather than
  this plan's *task* decomposition, and one of those commits bundled Task 4's work with Task
  5's — so every figure from Task 4 on was shifted by one, and "60 unit" was attributed to
  Task 4 instead of Task 8. Task 4's implementer caught it by reporting rather than forcing
  the number.

## A note on the compute figures

`tests/budget.rs` is the only place compute is written down, and several tasks re-measure it. Two things that have caught people, both now fixed but worth knowing:

- Until Task 6 the figures carried a geometric ladder: seven instructions declared the position PDA with a bare `bump`, so Anchor emitted `find_program_address` and paid ~1,500 CU per candidate tried, producing spreads of 9,000–10,500 CU. Task 6 removes the ladder. It does **not** make the figures single-valued — a residual 0–48 CU of ordinary jitter remains, measured over 20 runs. The useful change is that a max is now worth comparing against; it never was before. (An earlier draft of this plan claimed the figures become deterministic. That came from a 5-run sample, which is too few to see jitter this small.)
- Measure, do not estimate. Figures in this plan are the author's; record what *you* measure. Two drafts of Task 5's assertion and one of Task 7's shipped numbers nobody had measured, and each was wrong by more than the tolerance.

## File Structure

| File | What this plan changes |
|---|---|
| `tests/common/mod.rs` | Event decoding (`send_logs`, `decode_events`, `one_event`); a real `TokenMetadata` on the xStock fixture |
| `tests/{write_off,promo_forfeit,loans,liquidation,invariants,budget}.rs` | The coverage gaps and the two probes |
| `src/math/checked.rs` | `MathOverflow` pinned by variant |
| `src/constants.rs` | `SWITCHBOARD_ON_DEMAND_PID`, feature-selected |
| `src/oracle/switchboard.rs` | Routed through that constant |
| `src/instructions/**` | Seven position-PDA declarations read the stored bump |
| `Cargo.toml` | `devnet` feature; `base64` and `spl-token-metadata-interface` as dev-dependencies |
| `docs/superpowers/runbooks/` | The devnet deployment runbook |

---


### Task 1: Re-derive the gap list before closing anything

The recorded gaps are stale — six of fourteen were already closed. Working from the notes would mean writing tests for behaviour that already has them, and missing the two gaps nobody wrote down. This task produces the list the rest of the plan works from.

It is deliberately first and deliberately cheap: reading, not writing.

**Files:** none modified. Output is a written list.

**Interfaces:**
- Produces: the adjudicated gap list Tasks 2–5 consume. If your list disagrees with the plan's, **say so and follow yours** — the plan's was correct at the moment it was written and the tree has moved since.

- [ ] **Step 1: Adjudicate each recorded gap**

For each of the fourteen below, find the test that covers it and read it, or establish there is none. **Grepping an identifier across the repo gives false positives** — an error variant's definition in `src/errors.rs` is not a test of it, and that is exactly the mistake that made this list stale.

`ZeroPrincipalRepaid`; `write_off_loan` rejecting a `MarketMismatch`; `write_off_loan` with a surviving active sibling loan; a never-whitelisted borrower (distinct from blacklisted); borrowing with no collateral; foreign vault/`owner_token` substitution; the utilization cap with a non-zero `protocol_reserve`; third-party repayment when interest is due; partial repayment after maturity; withdrawal after full repayment with no market accounts; a correct market with a wrong `ngn_feed`; event contents decoded and asserted; `set_promo_cap` compute near `MAX_LISTED_COLLATERAL`; `revoke_promo` compute at 8 collateral slots.

- [ ] **Step 2: Find the gaps nobody recorded**

Two open-ended checks, and they are worth more than the list above:

- **Every `HodlError` variant with no test asserting it.** Enumerate `src/errors.rs` and cross-reference `tests/` *and* the `#[cfg(test)]` blocks in `src/**`. Note that `assert!(x.is_err())` does not count — it passes for any error.
- **Every instruction in `src/lib.rs` with no test calling it.** Note `tests/common/mod.rs` carries `#![allow(dead_code)]`, so an `_ix` builder existing there does not prove anything calls it.

- [ ] **Step 3: Write the list down**

Expected result at the time of writing: eight of the fourteen open (`ZeroPrincipalRepaid`, both `write_off_loan` cases, the utilization cap's reserve, third-party repayment with interest, event contents, and both compute measurements), and two unrecorded gaps — `MathOverflow` asserted only via `is_err()`, and `PromoVaultMismatch` reached by no test. All 43 instructions had coverage.

**No commit — this task produces the input to the next four, not a change.** That is
deliberate: a task whose output is a decision should not manufacture a diff to look
productive. Record your list where the next tasks can read it (the ledger, or a scratch
file), and carry any disagreement with the plan's expected result into Task 2's brief.

---

### Task 2: Three gaps, one of them the last unreached error

`write_off_loan` is the only market-touching instruction with no `MarketMismatch` test, and it is the one that writes `total_bad_debt` — a wrong market charges the loss to lenders who never funded the loan. No write-off test has ever run with a *surviving* sibling loan, so a neighbouring slot being zeroed alongside the written-off one would be silent. And `PromoVaultMismatch` guards the account forfeiture drains.

**Files:**
- Test: `programs/hodl_loans/tests/write_off.rs`, `tests/promo_forfeit.rs`

**Interfaces:** none produced.

- [ ] **Step 1: The two `write_off_loan` cases**

Append to `programs/hodl_loans/tests/write_off.rs`:

```rust
#[test]
fn a_write_off_against_the_wrong_market_is_rejected() {
    // Every other market-touching instruction has a `MarketMismatch` test — `repay_loan`,
    // `withdraw_collateral`, `take_loan`, `liquidate`, three promo paths and `close_position`.
    // `write_off_loan` was the one that did not, which matters more here than elsewhere: it is
    // the instruction that writes `total_bad_debt`, so pointing it at the wrong market would
    // charge the loss to lenders who never funded the loan.
    //
    // `require_keys_eq!(position.market, market_key)` in the handler is the ONLY guard that
    // catches this. The `address = market.vault` constraint looks like a second one and is not:
    // the instruction builder derives `market` and `vault` from the same mint, and `market.vault`
    // IS `market_vault_pda(mint)` by construction, so that constraint is trivially satisfied no
    // matter whose position is passed. It guards a different attack — a mismatched vault supplied
    // alongside a *correct* market. Delete the `require_keys_eq!` and this test fails (it reverts
    // on unrelated `MathOverflow` arithmetic instead), so the test is load-bearing for that one
    // line. Established by mutation, after an earlier draft of this comment claimed the
    // opposite.
    let (mut env, setup) = dust_collateral();
    let admin = env.admin.pubkey();
    let owner = setup.borrower.pubkey();

    let other = env.create_mint(MintKind::CngnLike, 6);
    env.create_market_with_promo(&other);

    let prices = env.price_accounts(&owner);
    let wrong = write_off_loan_ix(&admin, &owner, &other, 0, prices);
    assert_hodl_error(send(&mut env.svm, &[wrong], &[&env.admin]), HodlError::MarketMismatch);

    // Nothing moved: the loan is still there and the other market never saw the loss.
    assert!(env.position(&owner).loans[0].is_active());
    assert_eq!(env.market(&other).total_bad_debt, 0);
}

#[test]
fn writing_off_one_loan_leaves_an_active_sibling_untouched() {
    // Every multi-loan write-off test repays loan 0 in full before writing off loan 1, so no
    // sibling has ever been live at the moment of a write-off. The slot is
    // `bytemuck::Zeroable::zeroed()`-ed by the write-off, and a neighbouring slot getting the
    // same treatment would be silent — the position would simply have less debt than it owes.
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    env.take_loan(&setup.borrower, &setup, LOAN, 365 * DAY).unwrap();
    env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 365 * DAY).unwrap();
    let sibling_before = env.position(&owner).loans[1];
    assert!(sibling_before.is_active());

    env.set_pyth_price(&setup.usdc, USDC_DUST, 0);
    env.write_off(&setup, 0).unwrap();

    let position = env.position(&owner);
    assert_eq!(position.loans[0], bytemuck::Zeroable::zeroed(), "the written-off slot is cleared");
    assert_eq!(position.loans[1], sibling_before, "the sibling is byte-for-byte untouched");
    assert!(position.has_active_loans());

    // And the market still owes the sibling's principal — only the written-off loan left
    // `total_borrows`.
    assert_eq!(env.market(&setup.cngn).total_borrows, 1_000 * ONE_CNGN);
}
```

- [ ] **Step 2: `PromoVaultMismatch`, on both paths that carry it**

`promo_vault` is `Option`, so its `vault` field cannot be named in an `address` constraint — the binding is a `constraint` expression instead, which is easier to lose. Append to `programs/hodl_loans/tests/promo_forfeit.rs`, which needs `use anchor_lang::solana_program::instruction::Instruction;`:

```rust
#[test]
fn a_foreign_promo_vault_token_is_rejected_on_both_forfeit_paths() {
    // `promo_vault` is `Option`, so its `vault` field cannot be named in an `address`
    // constraint — the binding is a `constraint` expression instead, and `PromoVaultMismatch`
    // was the only error in the program raised by code no test reached. It guards the account
    // forfeiture *drains*: without it, a liquidation could name the real promo vault for its
    // bookkeeping and a different token account for the transfer.
    let (mut env, setup) = underwater_with_promo();
    let owner = setup.borrower.pubkey();
    let admin = env.admin.pubkey();

    // A second market's promo vault token: a real, program-owned cNGN account of exactly the
    // right shape, and not this market's.
    let other = env.create_mint(MintKind::CngnLike, 6);
    env.create_market_with_promo(&other);
    let foreign_token = promo_vault_token_pda(&other);
    assert_ne!(foreign_token, promo_vault_token_pda(&setup.cngn));

    let swap_promo_token = |ix: &mut Instruction| {
        let real = promo_vault_token_pda(&setup.cngn);
        let mut swapped = 0;
        for meta in ix.accounts.iter_mut() {
            if meta.pubkey == real {
                meta.pubkey = foreign_token;
                swapped += 1;
            }
        }
        assert_eq!(swapped, 1, "the promo vault token must appear exactly once");
    };

    // liquidate
    let liquidator = env.new_liquidator(&setup.cngn, LOAN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    let mut seize = liquidate_ix(
        &liquidator.pubkey(), &owner, &setup.cngn, &liquidator.cngn, &setup.usdc, &SPL_TOKEN,
        &seized_to, 0, 1_000 * ONE_CNGN, env.price_accounts(&owner),
    );
    swap_promo_token(&mut seize);
    assert_hodl_error(send(&mut env.svm, &[seize], &[&liquidator.key]), HodlError::PromoVaultMismatch);

    // write_off_loan, which carries the same pair of optional accounts and the same constraint
    env.set_pyth_price(&setup.usdc, 100_000, 0);
    let mut write_off = write_off_loan_ix(&admin, &owner, &setup.cngn, 0, env.price_accounts(&owner));
    swap_promo_token(&mut write_off);
    assert_hodl_error(send(&mut env.svm, &[write_off], &[&env.admin]), HodlError::PromoVaultMismatch);

    // The position and both vaults are untouched by either attempt.
    assert_eq!(env.position(&owner).promo_balance, GRANT);
    assert_eq!(env.promo_vault(&other).cash, 0);
}
```

- [ ] **Step 3: Run, and check what each test actually pins**

Run: `./scripts/test.sh` — expect 269.

Then verify each is load-bearing, rebuilding with `cargo build-sbf --tools-version v1.52` before each check:

- Delete the `PromoVaultMismatch` constraint from `liquidate.rs` and `write_off.rs` — the forfeit test must fail.
- Delete `write_off.rs`'s `require_keys_eq!(position.market, market_key, ...)` — the wrong-market test must fail, and it fails in an informative way: the transaction still reverts, but on `MathOverflow` rather than `MarketMismatch`. `require_keys_eq!` is the **only** guard for this attack. The `address = market.vault` constraint reads like a second one and is not — the ix builder derives `market` and `vault` from the same mint, and `market.vault` IS `market_vault_pda(mint)` by construction, so it is trivially satisfied whichever position is passed. Removing it *as well* changes nothing observable. (An earlier draft of this plan asserted the opposite — that removing one guard still passed. That was measured against a stale `.so`; rebuild with `cargo build-sbf` before every mutation check.)

- [ ] **Step 4: Commit**

```bash
git add programs/hodl_loans/tests/write_off.rs programs/hodl_loans/tests/promo_forfeit.rs
git commit -F - <<'MSG'
test(plan8): close three coverage gaps, one of them the last unreached error

write_off_loan had no MarketMismatch test, alone among the market-touching
instructions — and it is the one that writes total_bad_debt, so a wrong market
charges the loss to lenders who never funded the loan. require_keys_eq! is the
only guard that catches it: the address = market.vault constraint looks like a
second one but the ix builder derives market and vault from the same mint, so it
is trivially satisfied. Deleting require_keys_eq! fails this test.

No write-off test had ever run with a surviving sibling loan — every multi-loan
case repaid loan 0 in full first.

PromoVaultMismatch was raised by code no test reached. It guards the account
forfeiture drains, and because promo_vault is Option the binding is a
constraint expression rather than an address, which is easier to lose.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
```

---

### Task 3: Teach the harness to read events, and the cap to see the reserve

Two gaps, one of which needs harness support the repo has never had.

`send()` throws away a successful transaction's metadata with `.map(|_| ())`, so **no test can read an emitted event** — which is why none does, across 269 of them. Every event is emitted on a path some test already exercises, so a wrong *field* is invisible: the transaction succeeds and the state assertions still hold.

And `available_cash()` is `cash − protocol_reserve`, but every utilization-cap test runs on a market whose reserve is 0, so that subtraction has never been anything but a no-op.

**Files:**
- Modify: `programs/hodl_loans/Cargo.toml`, `tests/common/mod.rs`
- Test: `programs/hodl_loans/tests/loans.rs`

**Interfaces:**
- Produces: `send_logs`, `decode_events::<E>`, `one_event::<E>` in the harness. Nothing later in this plan uses them, but they are the tool for any future event assertion.

- [ ] **Step 1: Add the decoding support**

`base64` as a **dev-dependency only** — nothing on-chain needs it. Then in `tests/common/mod.rs`, above `assert_custom_error`:

```rust
/// Send, and keep the logs a successful transaction produced. `send` throws them away
/// (`.map(|_| ())`), which is why nothing in this suite could check an event until now.
pub fn send_logs(svm: &mut LiteSVM, ixs: &[Instruction], signers: &[&Keypair]) -> Result<Vec<String>, String> {
    svm.expire_blockhash();
    let msg = Message::new_with_blockhash(ixs, Some(&signers[0].pubkey()), &svm.latest_blockhash());
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), signers).unwrap();
    svm.send_transaction(tx)
        .map(|meta| meta.logs)
        .map_err(|e| format!("{:?} logs: {:#?}", e.err, e.meta.logs))
}

/// Decode every `emit!`ed event of type `E` from a transaction's logs, in order.
///
/// Anchor writes events as `Program data: <base64>`, where the payload is the event's
/// 8-byte discriminator followed by its Borsh body. Filtering on the discriminator is what
/// makes this type-safe: a log line for a different event deserializes to nothing here
/// rather than to a wrong-but-plausible `E`.
pub fn decode_events<E: anchor_lang::Event + anchor_lang::AnchorDeserialize>(logs: &[String]) -> Vec<E> {
    use base64::Engine;
    logs.iter()
        .filter_map(|l| l.strip_prefix("Program data: "))
        .filter_map(|b64| base64::engine::general_purpose::STANDARD.decode(b64).ok())
        .filter(|bytes| bytes.len() >= 8 && bytes[..8] == *E::DISCRIMINATOR)
        .filter_map(|bytes| E::try_from_slice(&bytes[8..]).ok())
        .collect()
}

/// The single event of type `E` a transaction emitted. Panics if there is not exactly one,
/// because a test that says "the event" and gets two is not testing what it thinks.
pub fn one_event<E: anchor_lang::Event + anchor_lang::AnchorDeserialize>(logs: &[String]) -> E {
    let mut found = decode_events::<E>(logs);
    assert_eq!(found.len(), 1, "expected exactly one event of this type, found {}", found.len());
    found.pop().unwrap()
}
```

Filtering on the event's own discriminator is what makes this type-safe: a log line for a different event yields nothing rather than a wrong-but-plausible decode. Note `bytes[..8] == *E::DISCRIMINATOR` — the deref matters, `[u8]` and `&[u8]` do not compare.

- [ ] **Step 2: The first two users**

Append to `programs/hodl_loans/tests/loans.rs`:

```rust
#[test]
fn the_loan_events_carry_what_an_indexer_would_read() {
    // No test in this repo had ever decoded an event. Every one is emitted on a path some
    // test exercises, so a wrong *field* — a swapped `owner`/`payer`, a principal that is
    // really the balance — is invisible: the transaction still succeeds and the state
    // assertions still hold. An off-chain indexer reads only this.
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    let opened_at = env.now();

    let prices = env.price_accounts(&owner);
    let borrow = take_loan_ix(&owner, &setup.cngn, &setup.borrower_cngn, 100_000 * ONE_CNGN, 365 * DAY, prices);
    let logs = send_logs(&mut env.svm, &[borrow], &[&env.admin, &setup.borrower.key]).unwrap();
    let opened: hodl_loans::LoanOpened = one_event(&logs);
    assert_eq!(opened.market, market_pda(&setup.cngn));
    assert_eq!(opened.position, position_pda(&owner));
    assert_eq!(opened.owner, owner);
    assert_eq!((opened.loan_id, opened.principal), (0, 100_000 * ONE_CNGN));
    assert_eq!(opened.tenure_seconds, 365 * DAY);
    // Copied from the market at origination, not read live — `a_loan_pays_out_cngn_and_records
    // _fixed_terms` pins that the slot works this way; this pins the event agrees with it.
    assert_eq!((opened.rate_bps, opened.penalty_rate_bps, opened.reserve_factor_bps), (1_500, 500, 1_000));
    assert_eq!(opened.originated_at, opened_at);

    // Repay in full a year later, so interest is non-zero and the split is checkable.
    env.warp_seconds(365 * DAY);
    env.mint_to(&setup.cngn, &setup.borrower_cngn, 100_000 * ONE_CNGN);
    let repay = repay_loan_ix(&owner, &owner, &setup.cngn, &setup.borrower_cngn, 0, u64::MAX);
    let logs = send_logs(&mut env.svm, &[repay], &[&env.admin, &setup.borrower.key]).unwrap();
    let repaid: hodl_loans::LoanRepaid = one_event(&logs);
    assert_eq!(repaid.owner, owner);
    assert_eq!(repaid.payer, owner, "owner and payer are distinct fields and must not be swapped");
    assert_eq!((repaid.loan_id, repaid.remaining_principal), (0, 0));
    // 15% of 100,000 cNGN over a year.
    assert_eq!(repaid.interest_paid, 15_000 * ONE_CNGN);
    assert_eq!(repaid.principal_repaid, 100_000 * ONE_CNGN);
    assert_eq!(repaid.amount, repaid.principal_repaid + repaid.interest_paid);
}

#[test]
fn a_third_party_repayment_names_the_payer_and_still_settles_the_interest() {
    // The one third-party repay test repays immediately after borrowing, so interest is ~0 —
    // the interest-first split it should exercise never runs. And `payer` is the field most
    // likely to be wrong, since it is the only one that differs from `owner` here.
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    env.take_loan(&setup.borrower, &setup, 100_000 * ONE_CNGN, 365 * DAY).unwrap();
    env.warp_seconds(365 * DAY);

    let good_samaritan = env.new_borrower();
    let payer = good_samaritan.pubkey();
    let payer_cngn = env.create_token_account(&setup.cngn, &payer);
    env.mint_to(&setup.cngn, &payer_cngn, 200_000 * ONE_CNGN);

    let repay = repay_loan_ix(&payer, &owner, &setup.cngn, &payer_cngn, 0, u64::MAX);
    let logs = send_logs(&mut env.svm, &[repay], &[&env.admin, &good_samaritan.key]).unwrap();
    let repaid: hodl_loans::LoanRepaid = one_event(&logs);
    assert_eq!(repaid.owner, owner);
    assert_eq!(repaid.payer, payer);
    assert_eq!(repaid.interest_paid, 15_000 * ONE_CNGN, "a year of interest, paid by a stranger");
    assert_eq!(repaid.remaining_principal, 0);

    // The debt is settled and the payer is out of pocket — nobody else's balance moved.
    assert!(!env.position(&owner).has_active_loans());
    assert_eq!(env.token_balance(&payer_cngn), 200_000 * ONE_CNGN - repaid.amount);
    assert_eq!(env.token_balance(&setup.borrower_cngn), 100_000 * ONE_CNGN);
}
```

The second closes a second gap: the one existing third-party repay test repays immediately after borrowing, so the interest-first split it should exercise never runs.

- [ ] **Step 3: The utilization cap, against a real reserve**

```rust
#[test]
fn the_utilization_cap_counts_only_cash_the_reserve_does_not_own() {
    // `available_cash()` is `cash - protocol_reserve`, and the cap is computed from it — but
    // the existing cap test runs on a market whose reserve is 0, so that subtraction has never
    // been anything but a no-op. A cap that quietly counted the reserve as lendable would let
    // the pool lend out money earmarked for the protocol, and every current test would pass.
    let (mut env, setup) = Env::loan_ready();
    env.deposit_collateral(&setup.borrower, &setup.usdc, 100_000 * ONE_USDC);
    let b = &setup.borrower;

    // Build a real reserve: borrow, let a year of interest accrue, repay in full. The reserve
    // factor is 10% of the interest.
    env.take_loan(b, &setup, 1_000_000 * ONE_CNGN, 365 * DAY).unwrap();
    env.warp_seconds(365 * DAY);
    env.mint_to(&setup.cngn, &setup.borrower_cngn, 1_000_000 * ONE_CNGN);
    env.repay(&setup, 0, u64::MAX).unwrap();
    // A year of warping staled both feeds; repayment needs no prices but borrowing does.
    env.set_pyth_price(&setup.usdc, ONE_DOLLAR, 0);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);

    let market = env.market(&setup.cngn);
    let reserve = market.protocol_reserve;
    assert!(reserve > 0, "the reserve must be non-zero for this test to mean anything");
    assert_eq!(market.total_borrows, 0);
    let cash = market.cash;
    let available = cash - reserve;

    // The cap is `borrows_after * BPS <= (available + total_borrows) * max_utilization_bps`,
    // with `total_borrows` now 0 — so the ceiling is 90% of `available`, not of `cash`. One
    // atom past it is refused, and the difference between the two readings is 90% of the
    // reserve, which is far more than the 1,000 cNGN minimum loan.
    let ceiling = available / 10 * 9;
    assert!(cash / 10 * 9 > ceiling + 1_000 * ONE_CNGN, "the two readings must differ by a usable margin");
    assert_hodl_error(env.take_loan(b, &setup, ceiling + 1, 30 * DAY), HodlError::UtilizationCapExceeded);
    env.take_loan(b, &setup, ceiling, 30 * DAY).unwrap();

    // And the reserve is still there afterwards — lending against the cap never spends it.
    assert_eq!(env.market(&setup.cngn).protocol_reserve, reserve);
}
```

Note the price refresh after the warp — a year of warping stales both feeds, and repayment needs no prices but borrowing does.

- [ ] **Step 4: Run and verify**

Run: `./scripts/test.sh` — expect 272.

Two checks, each with a rebuild first:

- Change `LoanRepaid`'s `payer` field in `repay_loan.rs` to emit `owner` instead. Exactly one test must fail — and **before this task, none would have.**
- Make `available_cash()` return `self.cash` outright. All sixteen pre-existing tests in `loans.rs` still pass and the new cap test fails — but run this one with `--no-fail-fast`, because the blast radius is **two tests, not one**: `available_cash()` has exactly two consumers, `take_loan.rs:89` (the utilization cap, what the new test targets) and `withdraw_liquidity.rs:47`, whose pre-existing `liquidity.rs::withdrawals_are_limited_to_cash_minus_reserve` also fails. An earlier draft of this plan said "only the new cap test fails", which is true within `loans.rs` and misleading suite-wide.

- [ ] **Step 5: Commit**

```bash
git add programs/hodl_loans/Cargo.toml programs/hodl_loans/tests/
git commit -F - <<'MSG'
test(plan8): the suite can decode events now, and the cap sees the reserve

send() discards a successful transaction's metadata with .map(|_| ()), so no
test could read an emitted event. Added send_logs plus decode_events/one_event,
which filter Anchor's `Program data:` lines by the event's own discriminator.
base64 is a dev-dependency only.

Mutating LoanRepaid's payer to report the owner now fails exactly one test;
before this it failed none. The third-party repayment case also closes a second
gap — the existing third-party test repays immediately, so the interest-first
split never ran.

And the utilization cap now has a test with a non-zero protocol_reserve.
Making available_cash return cash outright still passes all sixteen
pre-existing tests in that file and fails only the new one.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
```

---
### Task 4: Two error gaps nobody had written down

`MathOverflow` is the most-raised error in the program — 37 sites — and nothing asserts it. The overflow tests in `math/checked.rs` use `.is_err()`, which passes for *any* error, so a helper that started returning `InvalidParameters` would be caught by nothing.

`ZeroPrincipalRepaid` stops a liquidation whose repayment is entirely interest: the liquidator pays, takes collateral at the bonus, and the principal does not move — repeatable until the slot is empty. Its only coverage was a unit test on `principal_share`'s flooring, which pins the value, not that the instruction refuses it.

**Files:**
- Modify: `programs/hodl_loans/src/math/checked.rs`
- Test: `programs/hodl_loans/tests/liquidation.rs`

**Interfaces:** none produced.

- [ ] **Step 1: Pin `MathOverflow` by variant**

Replace `zero_denominator_and_overflow_fail` in `programs/hodl_loans/src/math/checked.rs`:

```rust
    #[test]
    fn zero_denominator_and_overflow_fail() {
        // `is_err()` would pass for *any* error, and `MathOverflow` is the most-raised variant
        // in the program — 37 sites — with nothing anywhere pinning that it is what these
        // return. A helper that started returning `InvalidParameters` would be caught by
        // nothing. Every branch of every helper, by the variant.
        // Compare the error *code*, not the Debug string: Anchor stamps a source file and
        // line into the latter, so two `MathOverflow`s from different helpers never match.
        // The code is also what a client actually sees.
        let code = |e: anchor_lang::error::Error| match e {
            anchor_lang::error::Error::AnchorError(a) => a.error_code_number,
            other => panic!("expected an AnchorError, got {other:?}"),
        };
        let want = u32::from(HodlError::MathOverflow);
        let is_overflow = |r: Result<u128>| assert_eq!(code(r.unwrap_err()), want);
        is_overflow(mul_div_floor(1, 1, 0));
        is_overflow(mul_div_floor(u128::MAX, 2, 1));
        is_overflow(mul_div_ceil(1, 1, 0));
        is_overflow(mul_div_ceil(u128::MAX, 2, 1));
        is_overflow(add(u128::MAX, 1));
        is_overflow(sub(1, 2));
        is_overflow(pow10(39));
        assert_eq!(code(to_u64(u64::MAX as u128 + 1).unwrap_err()), want);

        // The boundaries on each side still succeed, so the guards are not simply always-on.
        assert_eq!(mul_div_floor(u128::MAX, 1, u128::MAX).unwrap(), 1);
        assert_eq!(add(u128::MAX - 1, 1).unwrap(), u128::MAX);
        assert_eq!(sub(1, 1).unwrap(), 0);
        assert_eq!(pow10(38).unwrap(), 10u128.pow(38));
        assert_eq!(to_u64(u64::MAX as u128).unwrap(), u64::MAX);
    }
```

Comparing the error **code** and not the `Debug` string is deliberate and was learned the hard way: Anchor stamps a source file and line into the latter, so two `MathOverflow`s from different helpers never compare equal.

- [ ] **Step 2: Reach `ZeroPrincipalRepaid`**

The arithmetic has to be derived, not guessed. `principal_share` is `floor(paid × principal / balance)`, so it needs `paid` of a *single atom* against an overdue balance. The slot cap is what forces that: a 12-decimal asset (the `MAX_COLLATERAL_DECIMALS` ceiling) at $1 makes one atom worth 1e-12 dollars, and **657 atoms** scales a full-LOAN request down to exactly `paid = 1`.

Append to `programs/hodl_loans/tests/liquidation.rs`:

```rust
#[test]
fn a_repayment_too_small_to_touch_principal_is_refused() {
    // `ZeroPrincipalRepaid` was the one error in the program raised by code no test reached.
    // It fires when `principal_share` — `floor(paid x principal / balance)` — rounds to zero:
    // the liquidator pays something, all of it lands on interest, and the loan's principal
    // does not move. Letting that through would let someone seize collateral at the
    // liquidation bonus while the debt it is supposed to be retiring stays exactly where it
    // was, repeatable until the slot is empty.
    //
    // Reaching it needs the slot cap to force `paid` down to a single atom. A 12-decimal
    // asset (the `MAX_COLLATERAL_DECIMALS` ceiling) priced at $1 makes one atom worth
    // 1e-12 dollars, so a slot holding 657 atoms covers exactly one atom of cNGN —
    // and against an overdue balance, one atom of repayment is all interest.
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    let dust = env.list_spl_collateral(12);
    env.set_pyth_price(&dust, ONE_DOLLAR, 0);

    // Borrow against the USDC, then strand a few hundred atoms of the dust asset in the
    // position and let the loan go a year overdue so interest dominates the balance.
    env.take_loan(&setup.borrower, &setup, LOAN, 30 * DAY).unwrap();
    env.deposit_collateral(&setup.borrower, &dust, 657);
    env.warp_seconds(400 * DAY);
    env.set_pyth_price(&setup.usdc, USDC_CRASHED, 0);
    env.set_pyth_price(&dust, ONE_DOLLAR, 0);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);

    let liquidator = env.new_liquidator(&setup.cngn, LOAN);
    let seized_to = env.create_token_account(&dust, &liquidator.pubkey());
    let slot = env.position(&owner).collateral.iter().position(|s| s.mint == dust).unwrap();

    // Ask to repay far more than the dust slot can cover: the cap scales the repayment down
    // to the slot, and what survives is too small to move the principal.
    // 657 atoms is the exact width of the window: the slot cap scales the requested LOAN down
    // to `paid = 1` atom, and the balance is 1.2176x the principal after the overdue period,
    // so `floor(1 x principal / balance)` is 0. At 656 the cap rounds `paid` to 0 and
    // `AmountTooSmall` fires one line earlier instead; above ~5,000 the repayment is large
    // enough to move the principal and the liquidation simply succeeds.
    let result = env.liquidate(&liquidator, &setup, &dust, &seized_to, 0, LOAN);
    assert_hodl_error(result, HodlError::ZeroPrincipalRepaid);

    // Nothing moved — the position keeps its dust and the loan keeps its principal.
    let position = env.position(&owner);
    assert_eq!(position.collateral[slot].amount, 657);
    assert_eq!(position.loans[0].principal, LOAN);
    assert_eq!(env.token_balance(&seized_to), 0);

    // The same liquidator against the *real* collateral succeeds, so the position is
    // genuinely liquidatable and it is the dust slot that is refused.
    let usdc_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    env.liquidate(&liquidator, &setup, &setup.usdc, &usdc_to, 0, 1_000 * ONE_CNGN).unwrap();
}
```

The window is one atom wide on the low side: at 656 the cap rounds `paid` to 0 and `AmountTooSmall` fires a line earlier. Both boundaries are in the test, because a change to the bonus or the price scale moves them.

- [ ] **Step 3: Run and verify**

Run: `cargo test --lib` (**59** — Step 1 replaces a test, it does not add one) and `./scripts/test.sh` (**273**).

Two checks, rebuilding first:

- Make `pow10` return `InvalidParameters` instead of `MathOverflow` — the unit test must fail. Under the old `is_err()` assertions it would not have.
- Delete `require!(principal_repaid > 0, ...)` from `liquidate.rs` — the new test must fail.

Every `HodlError` variant now has a test asserting it.

- [ ] **Step 4: Commit**

```bash
git add programs/hodl_loans/src/math/checked.rs programs/hodl_loans/tests/liquidation.rs
git commit -F - <<'MSG'
test(plan8): pin MathOverflow by variant, and reach ZeroPrincipalRepaid

MathOverflow is raised at 37 sites and nothing asserted it — the overflow tests
used is_err(), which passes for any error. Now every branch of every checked
helper is pinned by error code, with the succeeding boundary on each side so
the guards are not simply always-on. Codes rather than Debug strings: Anchor
stamps file and line into the latter.

ZeroPrincipalRepaid needed paid of a single atom against an overdue balance.
657 atoms of a 12-decimal $1 asset scales a full-LOAN request down to exactly
that; at 656 the cap rounds to 0 and a different guard fires first. Both
boundaries are recorded.

Every HodlError variant now has a test asserting it.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
```

---

### Task 5: Measure `set_promo_cap` at scale

`MAX_LISTED_COLLATERAL = 96` was derived entirely from `MAX_TX_ACCOUNT_LOCKS`, which says nothing about compute — and the compute runs out first. This is the one instruction whose cost scales with an admin-controlled list, in a repo with a file dedicated to CU ceilings, and it had no measurement.

**Files:**
- Test: `programs/hodl_loans/tests/budget.rs`

**Interfaces:** none produced.

- [ ] **Step 1: Measure it**

Append to `programs/hodl_loans/tests/budget.rs`, which needs `use anchor_lang::prelude::Pubkey;`:

```rust
/// `set_promo_cap` is the one instruction whose cost scales with an admin-controlled list:
/// `MAX_LISTED_COLLATERAL` assets, each deserialized, PDA-re-derived and re-validated. The
/// bound on that list was derived entirely from `MAX_TX_ACCOUNT_LOCKS` — nothing has ever
/// measured the compute, and 96 assets is well past what the 200,000 default budget covers.
/// This measures the real per-asset cost and states what a full list implies.
#[test]
fn set_promo_cap_costs_scale_with_the_asset_list() {
    let mut env = Env::initialized();
    let admin = env.admin.pubkey();
    let mut assets: Vec<Pubkey> = Vec::new();

    // Ten is enough to fit a legacy transaction and to fix the slope; the interesting figure
    // is per-asset, not the total at ten.
    for _ in 0..10 {
        assets.push(env.list_spl_collateral(6));
    }
    let at_ten = send_cu(&mut env.svm, &[set_promo_cap_ix(&admin, 2_000, &assets)], &[&env.admin]).unwrap();

    // The account count must equal `collateral_count` exactly, so a shorter list cannot be
    // measured on the same env — take the two-asset reading from a fresh one and subtract.
    let mut small = Env::initialized();
    let mut two_assets = Vec::new();
    for _ in 0..2 {
        two_assets.push(small.list_spl_collateral(6));
    }
    let small_admin = small.admin.pubkey();
    let at_two =
        send_cu(&mut small.svm, &[set_promo_cap_ix(&small_admin, 2_000, &two_assets)], &[&small.admin]).unwrap();

    let per_asset = (at_ten - at_two) / 8;
    // **Measured, and unlike the walking figures these are deterministic** — no position PDA,
    // so no `find_program_address` bump search: 10,023 CU at two assets, 31,095 at ten,
    // **2,634 CU per asset**. That is a 294-byte Borsh deserialize plus the
    // `create_program_address` this instruction re-derives per asset (~1,587, see above) plus
    // `validate`.
    //
    // At the `MAX_LISTED_COLLATERAL` bound of 96 that extrapolates to **~257,600 CU — past the
    // 200,000 default budget.** An admin at a full asset list must send an explicit
    // `ComputeBudgetInstruction::set_compute_unit_limit`; without one, `set_promo_cap` starts
    // failing somewhere around 74 assets. It fits well inside the 1.4M maximum and the extra
    // program key still leaves ~100 of the 128 account locks, so the 96 bound is sound — but
    // the default budget stops covering it first, which is not something the bound's own
    // derivation (account locks) would ever tell you.
    assert!(
        (2_400..2_900).contains(&per_asset),
        "per-asset cost moved to {per_asset} CU from the measured 2,634 — re-derive what the \
         96-asset bound implies for compute before accepting this"
    );
    let at_bound = at_two + per_asset * (hodl_loans::MAX_LISTED_COLLATERAL as u64 - 2);
    assert!(
        at_bound > 200_000,
        "a full asset list now fits the default budget ({at_bound} CU) — the warning above is \
         stale and should be removed"
    );
    // And the default budget runs out well before the bound does.
    let affordable = 2 + (200_000 - at_two) / per_asset;
    assert!(
        affordable < hodl_loans::MAX_LISTED_COLLATERAL as u64,
        "{affordable} assets now fit the default budget, at or past the {} bound",
        hodl_loans::MAX_LISTED_COLLATERAL
    );
}
```

Two details that cost time: the account count must equal `collateral_count` **exactly**, so a shorter list cannot be measured on the same env — hence the second `Env`. And that second env has its own admin keypair; signing with the first env's admin gives `NotEnoughSigners`.

- [ ] **Step 2: Record what you measure**

The author measured 10,023 CU at two assets, 31,095 at ten, **2,634 per asset** — deterministic, since this instruction touches no position PDA. At the 96-asset bound that extrapolates to **~257,600 CU, past the 200,000 default budget**, so a full list needs an explicit `ComputeBudgetInstruction::set_compute_unit_limit` and fails around 74 assets without one.

A first draft of this test asserted a 3,000–3,400 band. The measurement said 2,634. **Record yours.**

- [ ] **Step 3: Run and commit**

Run: `./scripts/test.sh` — expect 274.

```bash
git add programs/hodl_loans/tests/budget.rs
git commit -F - <<'MSG'
test(plan8): measure set_promo_cap at scale

The MAX_LISTED_COLLATERAL bound was derived entirely from MAX_TX_ACCOUNT_LOCKS,
which says nothing about compute — and the compute runs out first. Measured
2,634 CU per asset, deterministic. At the 96-asset bound that is ~257,600 CU,
past the 200,000 default, so a full list needs an explicit ComputeBudget
instruction and fails around 74 assets without one.

The test asserts both the per-asset cost and that the default budget still runs
out before the bound, so if either changes someone re-derives it rather than
discovering it as a lockout.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
```

---

### Task 6: Read the stored position bump instead of searching for it

`Position` has stored its own bump since Plan 1, and seven of the eight instructions that pin the position PDA ignore it — declaring a bare `bump`, so Anchor emits `find_program_address` and tries candidates from 255 downward at ~1,500 CU each. A randomly-keyed borrower's canonical bump is 255 with p≈½, 254 with p≈¼, and so on.

**That geometric ladder is what every "measured X–Y over N runs" range in `tests/budget.rs` has been measuring.** This task removes it, and the figures become deterministic.

**Files:**
- Modify: `src/instructions/loans/take_loan.rs`, `src/instructions/positions/{deposit_collateral,withdraw_collateral,close_position}.rs`, `src/instructions/promos/{redeem,lifecycle}.rs`
- Modify: `programs/hodl_loans/tests/budget.rs`

**Interfaces:** none produced, but every later measurement depends on this landing first.

- [ ] **Step 1: Use the stored bump**

In each declaration, `bump)]` becomes `bump = position.load()?.bump)]`. Seven sites across six files — `lifecycle.rs` has two.

**`open_position` keeps its bare bump.** It is `init`: there is no stored bump to read yet, and the search is what establishes it.

- [ ] **Step 2: Run, then re-measure everything**

Run: `./scripts/test.sh` — expect 274, unchanged.

Then re-measure every figure in `tests/budget.rs`: turn each `assert!(cu < N, ...)` into a `println!`, run the budget test 16+ times, take min–max, restore the assertions.

**Only re-measure figures that exist in `budget.rs` at this point.** At Task 6 the file carries `take_loan`, `withdraw_collateral`, the three `liquidate` figures, `repay_loan`, the xStock pair, and Task 5's `set_promo_cap`. It does **not** yet carry `revoke_promo` — Task 7 adds that, and measures it post-bump. Do not go looking for it.

What was measured over 20 runs, max-to-max, on the figures you will have: `take_loan` **−10,052**, `withdraw_collateral` **−10,170**, xStock `take_loan` **−8,535**. (An earlier draft quoted −8,680 / −8,680 / −7,163 from a 5-run sample; the arithmetic against the ranges this file carried — 98,595 − 88,543 = 10,052 — gives the figures above.) The three `liquidate` figures went the *other* way by ~260 CU — they never constrained the position by seeds, so they paid no search and see only the extra account read. `set_promo_cap` should not move at all (it touches no position PDA); if it does, report it. The comment block you install in Step 3 also cites a `revoke_promo` −**13,432**: that is the end-state figure this plan reaches at Task 7, deliberately left in the comment so the finished file reads coherently. It is not something for you to reproduce.

The more useful result is the collapse in spread: `take_loan` 10,500 → 31, `withdraw_collateral` 10,500 → 0, xStock `take_loan` 9,000 → 48. Not zero everywhere — `liquidate` (no-forfeit) and xStock `take_loan` sit at up to 48 CU — but two orders of magnitude down, which is what makes a max comparable.

- [ ] **Step 3: Replace the file's causal note**

The note explaining the nondeterminism is now history rather than current behaviour:

```rust
    // **These figures are deterministic as of Plan 8, and were not before.** Every instruction
    // that pins the position PDA now passes `bump = position.load()?.bump` rather than a bare
    // `bump`, so Anchor reads the stored bump instead of emitting `find_program_address`,
    // which tried candidates from 255 downward at ~1,500 CU each. A randomly-keyed borrower's
    // canonical bump is 255 with p≈1/2, 254 with p≈1/4, and so on — a geometric ladder with an
    // unbounded tail, and that is what every "measured X-Y over N runs" range in this file's
    // history was measuring. `Liquidate` and `RepayLoan` never constrained the position by
    // seeds, which is why their spreads were ~31-62 CU while the others ran to thousands.
    //
    // What the change bought, max-to-max: take_loan -8,680, withdraw_collateral -8,680,
    // xStock take_loan -7,163, revoke_promo -13,432. The liquidate figures went the other way
    // by ~260 CU — no search to save, only an extra account read. `open_position` still uses a
    // bare bump because it is `init`: there is no stored bump to read yet.
    //
    // Ranges below are min-max over 16 runs. The residual ~30-50 CU on the liquidate figures
    // is ordinary jitter, not a step.
```

- [ ] **Step 4: Commit**

```bash
git add programs/hodl_loans/src programs/hodl_loans/tests/budget.rs
git commit -F - <<'MSG'
perf(plan8): read the stored position bump instead of searching for it

Position has stored its bump since Plan 1, and seven of the eight instructions
that pin the position PDA ignored it, so Anchor emitted find_program_address
and paid ~1,500 CU per candidate bump tried. open_position keeps its bare bump:
it is init, so there is no stored bump to read yet.

Measured, max-to-max: take_loan -8,680, withdraw_collateral -8,680, xStock
take_loan -7,163, revoke_promo -13,432. The three liquidate figures went the
other way by ~260 CU — they never constrained the position by seeds.

The more useful result is that the figures are now deterministic. take_loan's
spread went 10,500 -> 0, revoke_promo's 13,500 -> 0. That removes the reason
this file needed bucket-coverage arguments at all, and retires the "seed the
harness keypairs" follow-up — the nondeterminism is gone at the source.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
```

---

### Task 7: The two probes the follow-ups asked a fuzzer for

Plan 2 and Plan 3 both deferred "Trident invariants" here: a probe for two consecutive partial liquidations, and one for the deposit → liquidate → withdraw sandwich in spec §11's accepted-risk note.

**Neither needs a fuzzer.** Both are properties of a specific *sequence*, and `tests/invariants.rs` already has an `assert_invariants` helper that recomputes the market's accounting from position and vault state. (Trident also could not be installed in the authoring environment — the network was blocked — so this is what was verifiable.)

**Files:**
- Test: `programs/hodl_loans/tests/invariants.rs`
- Modify: `programs/hodl_loans/tests/budget.rs` (the `revoke_promo` measurement)

**Interfaces:** none produced.

- [ ] **Step 1: Convergence**

```rust
#[test]
fn two_consecutive_partial_liquidations_converge() {
    // The first of the two probes the Plan 3 follow-ups asked a fuzzer for. It is a property
    // about a *sequence*, which is why a single-step test cannot see it: each partial
    // liquidation seizes collateral and repays debt, and the position's health must move
    // monotonically toward solvency. If the seizure and the repayment disagreed — the bonus
    // taking more value than the repayment retires — a position could be liquidated
    // repeatedly and end further under water each time, which is the shape of a drain.
    let (mut env, setup) = Env::loan_ready();
    let owner = setup.borrower.pubkey();
    env.take_loan(&setup.borrower, &setup, 700_000 * ONE_CNGN, 365 * DAY).unwrap();
    env.set_pyth_price(&setup.usdc, 45_000_000, 0);
    assert_invariants(&env, &setup, "before any liquidation");

    let liquidator = env.new_liquidator(&setup.cngn, 1_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());

    let debt_and_collateral = |env: &Env| {
        let p = env.position(&owner);
        (p.loans[0].principal, p.collateral.iter().find(|s| s.mint == setup.usdc).unwrap().amount)
    };
    let (debt0, coll0) = debt_and_collateral(&env);

    // Two partial liquidations, each repaying a tenth of the principal.
    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 70_000 * ONE_CNGN).unwrap();
    assert_invariants(&env, &setup, "after the first partial liquidation");
    let (debt1, coll1) = debt_and_collateral(&env);

    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 70_000 * ONE_CNGN).unwrap();
    assert_invariants(&env, &setup, "after the second partial liquidation");
    let (debt2, coll2) = debt_and_collateral(&env);

    // Both quantities fall, every time — no oscillation, no growth.
    assert!(debt1 < debt0 && debt2 < debt1, "principal must fall with each liquidation");
    assert!(coll1 < coll0 && coll2 < coll1, "collateral must fall with each liquidation");

    // The property worth pinning is not that the two steps match each other — at a fixed
    // price they are symmetric by construction, so asserting that catches almost nothing.
    // It is that **each step leaves the position better collateralised than it found it**:
    // the value seized must not exceed the value of the debt retired by more than the bonus
    // the asset is configured to pay. A liquidation that took more than that would let a
    // liquidator walk a healthy-ish position down to nothing one call at a time.
    //
    // Both sides converted to micro-dollars, which is the trap here: the crashed Pyth price
    // is at exponent -8 (45_000_000 == $0.45) while `NGN_USD` is at Switchboard's 18 decimals
    // (625_000_000_000_000 == $0.000625). Mixing the two silently inflates one side by ten
    // orders of magnitude and makes any ceiling vacuous — which is exactly what a first
    // version of this assertion did.
    const MICRO: u128 = 1_000_000;
    let seized_usd = |atoms: u64| atoms as u128 * 45_000_000 * MICRO / 100_000_000 / ONE_USDC as u128;
    let retired_usd =
        |atoms: u64| atoms as u128 * NGN_USD as u128 * MICRO / 1_000_000_000_000_000_000 / ONE_CNGN as u128;
    for (label, seized, retired) in
        [("first", coll0 - coll1, debt0 - debt1), ("second", coll1 - coll2, debt1 - debt2)]
    {
        let taken = seized_usd(seized);
        let given = retired_usd(retired);
        // 5% is `default_collateral_params`' `liquidation_bonus_bps`. One atom of slack for
        // the per-step flooring in `principal_share` and `seize_for_repayment`.
        let ceiling = given * 10_500 / 10_000 + 1;
        assert!(
            taken <= ceiling,
            "{label} step seized {taken} USD against {given} retired — past the 5% bonus"
        );
        assert!(taken > 0 && given > 0, "{label} step moved nothing");
    }
}
```

**Read the scale comment before writing the assertion.** Two earlier versions of it were worthless and survived a *tripled* liquidation bonus. The first compared the two steps to each other — vacuous, since at a fixed price they are symmetric by construction. The second mixed scales: the Pyth price is at exponent −8 while `NGN_USD` is at Switchboard's 18 decimals, and combining them inflated one side by ten orders of magnitude, so the ceiling could never bind. The version here catches a *doubled* bonus.

- [ ] **Step 2: The sandwich**

```rust
#[test]
fn a_lender_cannot_sandwich_a_liquidation_for_the_penalty_step() {
    // The second probe. Spec §11's accepted-risk note records that overdue penalty interest
    // reaches lenders as a *step* at repayment or liquidation rather than continuously, and
    // that `liquidate` has no access check — so a lender could deposit immediately before
    // someone else's liquidation, collect a share of the step, and withdraw, diluting the
    // lenders who actually carried the loan.
    //
    // This pins the accounting around that sequence. It does not prevent the sandwich — the
    // fix is continuous penalty accrual, which is its own plan — but it establishes that the
    // sandwich cannot extract *more* than the step it is capturing, and that every invariant
    // survives the sequence. A regression that let the sandwicher withdraw more than they put
    // in plus their share would fail here.
    let (mut env, setup) = Env::loan_ready();
    env.take_loan(&setup.borrower, &setup, 700_000 * ONE_CNGN, 30 * DAY).unwrap();

    // Go well past maturity so the penalty term is substantial, then crash the price.
    env.warp_seconds(400 * DAY);
    env.set_pyth_price(&setup.usdc, 45_000_000, 0);
    env.set_ngn_price(NGN_USD, NGN_SPREAD);
    assert_invariants(&env, &setup, "overdue, before the sandwich");

    // The sandwicher deposits just before the liquidation.
    let sandwicher = env.new_lender(&setup.cngn, 1_000_000 * ONE_CNGN);
    let deposited = 1_000_000 * ONE_CNGN;
    env.deposit(&sandwicher, &setup.cngn, deposited).unwrap();
    assert_invariants(&env, &setup, "after the sandwicher deposits");

    let liquidator = env.new_liquidator(&setup.cngn, 1_000_000 * ONE_CNGN);
    let seized_to = env.create_token_account(&setup.usdc, &liquidator.pubkey());
    env.liquidate(&liquidator, &setup, &setup.usdc, &seized_to, 0, 100_000 * ONE_CNGN).unwrap();
    assert_invariants(&env, &setup, "after the liquidation releases the penalty step");

    // Withdraw everything the sandwicher can.
    let before = env.token_balance(&sandwicher.token);
    env.withdraw(&sandwicher, &setup.cngn, u64::MAX).unwrap();
    let gained = env.token_balance(&sandwicher.token) - before;
    assert_invariants(&env, &setup, "after the sandwicher withdraws");

    // They get back what they put in, plus at most their pro-rata share of what the
    // liquidation released — never more. The point of the assertion is the upper bound: a
    // change that let a same-slot deposit claim more than its share of the step would break
    // it, and that is the failure mode the accepted-risk note is about.
    assert!(gained >= deposited, "a lender must never lose principal to someone else's liquidation");
    let share = gained - deposited;
    assert!(
        share < deposited / 100,
        "a same-slot sandwich took {share} atoms on {deposited} deposited — more than a \
         pro-rata share of one liquidation's penalty step"
    );
}
```

This does **not** prevent the sandwich — the fix is continuous penalty accrual, which needs its own plan. It establishes that the sandwicher cannot extract more than a pro-rata share of the step they captured, and that every invariant survives the sequence.

- [ ] **Step 3: Measure `revoke_promo`**

Plan 7's follow-ups recorded `revoke_promo` as covered by the raised `take_loan` ceilings because it is "strictly cheaper on the same walk". That is an argument, not a measurement, and this file should not contain arguments.

```rust
/// `revoke_promo` joined the health-walking instructions in Plan 7 — it prices the position
/// after releasing the promo — but was never measured. The follow-ups claimed it was covered
/// by the raised `take_loan` ceilings on the grounds that it is strictly cheaper on the same
/// walk; that is an argument, not a measurement, and this is the measurement.
#[test]
fn revoke_promo_stays_under_the_default_compute_budget_at_eight_slots() {
    let (mut env, setup) = Env::promo_ready();
    let owner = setup.borrower.pubkey();
    let admin = env.admin.pubkey();
    env.redeem_promo(&setup.borrower, &setup.cngn, 1, 50_000 * ONE_CNGN, 7).unwrap();

    // Same shape as the other eight-slot fixtures: usdc is slot 0, seven more fill the rest.
    let mut mints = vec![setup.usdc];
    for i in 0..7 {
        let decimals = if i % 2 == 0 { 9 } else { 6 };
        let mint = env.list_spl_collateral(decimals);
        env.set_pyth_price(&mint, 150 * ONE_DOLLAR, 10_000_000);
        env.deposit_collateral(&setup.borrower, &mint, 10u64.pow(decimals as u32) * 10);
        mints.push(mint);
    }
    assert_eq!(env.price_accounts(&owner).len(), 16);

    // A live loan is what makes revocation take the priced path at all. Small enough that the
    // position stands without the promo, so the health check passes and the walk completes.
    env.take_loan(&setup.borrower, &setup, 1_000 * ONE_CNGN, 30 * DAY).unwrap();

    let prices = env.price_accounts(&owner);
    let ixn = revoke_promo_priced_ix(&admin, &setup.cngn, &owner, true, prices);
    let cu = send_cu(&mut env.svm, &[ixn], &[&env.admin]).unwrap();

    // **Measured 63,410 CU, deterministic.** Before the stored bump this was 63,342-76,842
    // over 20 runs — a 13,500 spread, the widest in the file, because `RevokePromo` paid the
    // bump search with no large fixed cost for it to hide behind. That made it the single
    // biggest beneficiary of storing the bump: -13,432 CU at the max.
    //
    // Comfortably the cheapest of the walking instructions even at its max: it prices eight
    // slots but settles no loan, moves no tokens and touches no collateral vault. The
    // follow-ups asserted it was "strictly cheaper than `take_loan`" and therefore covered by
    // that ceiling; the assertion was right and is now measured rather than argued.
    assert!(cu < 95_000, "revoke_promo at 8 collateral slots used {cu} CU");
    assert_eq!(env.position(&owner).promo_balance, 0);
}
```

The claim holds. A first draft of this test asserted a range nobody had measured; the real figures were 3,000 CU higher at the min.

- [ ] **Step 4: Verify both probes bite**

Run: `./scripts/test.sh` — expect 277. Then, rebuilding each time:

- Double the liquidation bonus in `seize_for_repayment` — the convergence probe must fail. **If it passes, your assertion has the scale bug.**
- Make `shares_for_deposit` mint against `total_assets / 2` — the sandwich probe must fail.

- [ ] **Step 5: Commit**

```bash
git add programs/hodl_loans/tests/invariants.rs programs/hodl_loans/tests/budget.rs
git commit -F - <<'MSG'
test(plan8): the two probes the follow-ups asked a fuzzer for, and revoke_promo's compute

Both probes are properties of a sequence, not fuzzing targets, and are written
against the existing assert_invariants helper.

The convergence probe pins value conservation per step: what a liquidation
seizes may exceed what it retires only by the asset's configured bonus. Two
earlier versions were worthless and were caught only by mutating the bonus and
watching them survive — one vacuous, one with a scale bug between the Pyth
price at exponent -8 and NGN_USD at 18 decimals. The version here catches a
doubled bonus.

The sandwich probe does not prevent the sandwich; it establishes the
sandwicher cannot take more than a pro-rata share of the step.

And revoke_promo is measured rather than argued about: 63,410 CU at eight
slots, deterministic. The follow-ups' claim that it is cheaper than take_loan
was right.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
```

---

### Task 8: Make the program deployable to devnet

Plan 7 pinned the NGN feed's owner check to `ON_DEMAND_MAINNET_PID` and recorded the consequence for this plan: **a devnet deployment fails every health check at the NGN feed.** Deposits and repayment still work, which makes it worse — the program looks alive and is unusable.

**Files:**
- Modify: `programs/hodl_loans/src/constants.rs`, `src/oracle/switchboard.rs`, `Cargo.toml`, `tests/loans.rs`, `tests/common/mod.rs`

**Interfaces:**
- Produces: `constants::SWITCHBOARD_ON_DEMAND_PID` and the `devnet` feature.

- [ ] **Step 1: The feature-selected constant**

In `programs/hodl_loans/src/constants.rs`:

```rust
/// The Switchboard On-Demand program that must own the NGN feed (`oracle/switchboard.rs`).
///
/// Chosen at **compile time**, not stored in `Config`, and that is the whole point. The owner
/// check exists because `Market::ngn_feed` is an admin-settable bare `Pubkey` with nothing
/// behind it — a mis-set feed would have the program read 3.2 KB of arbitrary bytes as a
/// price. Putting the expected *owner* in an account would reintroduce exactly that shape one
/// level up: another admin-settable value that, set wrong, turns the check off. A deployed
/// binary cannot be misconfigured after the fact.
///
/// The cost is two binaries to keep straight, and a devnet build that is silently wrong if
/// someone forgets `--features devnet`. `the_switchboard_pid_matches_the_build` below turns
/// that into a test failure rather than a production one.
///
/// The `switchboard_on_demand` crate has its own selector, but it is client-only: it reads
/// `std::env::var("SB_ENV")`, which does not exist on SBF. The two PID constants themselves
/// are plain and usable on-chain, so we choose between them ourselves.
#[cfg(not(feature = "devnet"))]
pub const SWITCHBOARD_ON_DEMAND_PID: Pubkey = switchboard_on_demand::ON_DEMAND_MAINNET_PID;
#[cfg(feature = "devnet")]
pub const SWITCHBOARD_ON_DEMAND_PID: Pubkey = switchboard_on_demand::ON_DEMAND_DEVNET_PID;
```

Add `devnet = []` to `[features]`, and route all four pin sites through the constant — two in `oracle/switchboard.rs`, two in tests.

**Compile-time, not a `Config` field, and that is the point.** The owner check exists *because* `Market::ngn_feed` is an admin-settable bare `Pubkey` with nothing behind it. Storing the expected owner in an account would reintroduce that exact shape one level up. The crate's own selector reads `std::env::var`, so it is client-only and unusable on SBF.

- [ ] **Step 2: Make a forgotten flag a test failure**

```rust
    #[test]
    fn the_switchboard_pid_matches_the_build() {
        // The one failure mode of choosing this at compile time: a binary built for the wrong
        // cluster is indistinguishable until the first health check fails on-chain. Asserting
        // the constant against the feature turns that into a test failure — `cargo test` and
        // `cargo test --features devnet` each pin their own half, so a mainnet build that
        // somehow selected the devnet id (or the reverse) cannot ship green.
        if cfg!(feature = "devnet") {
            assert_eq!(SWITCHBOARD_ON_DEMAND_PID, switchboard_on_demand::ON_DEMAND_DEVNET_PID);
        } else {
            assert_eq!(SWITCHBOARD_ON_DEMAND_PID, switchboard_on_demand::ON_DEMAND_MAINNET_PID);
        }
        // And the two are genuinely different, so the assertion above is not vacuous on a
        // future crate version that collapsed them.
        assert_ne!(
            switchboard_on_demand::ON_DEMAND_MAINNET_PID,
            switchboard_on_demand::ON_DEMAND_DEVNET_PID
        );
    }
```

- [ ] **Step 3: Verify both builds**

```
cargo test --lib                      # 60
cargo test -p hodl_loans --lib --features devnet   # 60
./scripts/test.sh                     # 278
cargo clippy -p hodl_loans --all-targets -- -D warnings
cargo clippy -p hodl_loans --all-targets --features devnet -- -D warnings
```

Confirm the feature actually changes the constant — build both and compare the `.so` hashes; they must differ. Then flip the devnet arm to select the mainnet id: the devnet build must fail and the default must not.

- [ ] **Step 4: Give the xStock fixture real metadata**

Recorded as a devnet prerequisite since Plan 4. Two constraints box you in: `try_calculate_account_len` cannot size a variable-length extension, and Token-2022 **rejects `InitializeMint2` on an account longer than the calculated length** — so padding up front fails with `InvalidAccountData`.

The shape that works: create the account at exactly the calculated size, **fund it for the larger final size**, and let `spl_token_metadata_interface::instruction::initialize` grow the account itself while packing the TLV entry. **Do not reach for Token-2022's `Reallocate`** — that instruction is token-account-only and rejects a Mint with `InvalidAccountData` (its own doc comment says "Check to see if a *token account* is large enough"). An earlier draft of this plan said to use it; the code it was drafted from never did.

Add `spl-token-metadata-interface` as a dev-dependency, an `XSTOCK_METADATA_SPACE` constant, and the metadata `initialize` call after `initialize_mint2`.

Measured cost: **+0 CU** on the all-xStock `take_loan` and **+54** on the forfeit case. Unpacking a mint walks TLV headers and the program never reads the metadata's contents, so a longer entry is nearly free — which retires the "these figures are a floor" caveat rather than passing it on.

- [ ] **Step 5: Commit**

Two commits, since they are independent:

```bash
git add programs/hodl_loans/src programs/hodl_loans/Cargo.toml programs/hodl_loans/tests
git commit -F - <<'MSG'
feat(plan8): make the Switchboard program id cluster-selectable

A devnet deployment reads a feed owned by ON_DEMAND_DEVNET_PID, so every health
check failed at the NGN feed while deposits and repayment kept working — the
program looked alive and was unusable.

Compile-time rather than a Config field, deliberately: the owner check exists
because Market::ngn_feed is an admin-settable bare Pubkey, and storing the
expected owner in an account would reintroduce that shape one level up.

The cost is a devnet build that is silently wrong if someone forgets the flag,
so the_switchboard_pid_matches_the_build asserts the constant against the
feature in both directions.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
```

then the fixture, with its own message.

---

### Task 9: The devnet runbook

A deployment is outward-facing and spends real SOL on a public cluster. **This task writes a runbook; it does not deploy.** The deployment is the user's to run, with their explicit go-ahead.

**Files:**
- Create: `docs/superpowers/runbooks/YYYY-MM-DD-devnet-deployment.md`

- [ ] **Step 1: Verify what you can, mark what you cannot**

Check against the tree, and record the result either way:

- The two builds produce different binaries (hash them).
- **`declare_id!` against the deploy keypair.** At the time of writing these *disagree* — `J9sKAhm2EhdJQ3bHeP2KUCxqZ4cYdBc65C3RDr4JjGEd` declared, `CmDBvi4ZiBDND1XzEwKFuH3kontC5Lokd2faXBxEgmci` in `target/deploy/hodl_loans-keypair.json`. Deploying with that keypair puts the program at an address it does not believe it lives at, so every PDA is computed wrong and the failure looks like unrelated seed errors. This is a hard gate.
- `anchor-cli` is 0.31.1 against `anchor-lang` 1.2.0 and there is no `Anchor.toml`, so the runbook uses `solana program deploy`, not `anchor deploy`.

Mark every step you could not run — anything needing the network — as unverified rather than implying it was tested.

- [ ] **Step 2: Write down what devnet does not have**

The larger part of the work, and none of it is the deploy: a Switchboard On-Demand NGN feed under the devnet program id, devnet Pyth accounts for each collateral, a cNGN mint, and collateral mints. Backed xStocks do not exist on devnet.

- [ ] **Step 3: Initialization order and smoke test**

Mirror `Env::initialized()` → `loan_ready()`, which is the tested ordering. The smoke test's key step is `take_loan`: **a `take_loan` that fails with `PriceAccountMismatch` while deposits succeed is the signature of a mainnet binary on devnet.**

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/runbooks/
git commit -F - <<'MSG'
docs: devnet deployment runbook

Written to be run by a person, not executed by a plan task — a deployment is
outward-facing and spends real SOL on a public cluster.

Two blockers, both of which fail closed. The build must carry --features
devnet, or the NGN feed's owner check refuses every health check while deposits
and repayment keep working. And declare_id! does not match the local deploy
keypair, so deploying with it would put the program at an address it does not
believe it lives at — every PDA wrong, failing as unrelated seed errors.

Records what devnet does not have: a Switchboard NGN feed under the devnet
program id, devnet Pyth accounts, and cNGN and collateral mints. That is the
larger part of the work.

Steps needing network access are marked unverified rather than implied tested.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
```

---

## After the last task

Run `./scripts/test.sh` and clippy under both feature combinations. Expect **278 tests** (60 unit, 218 LiteSVM).

Then write `docs/superpowers/plans/YYYY-MM-DD-plan-8-followups.md`, recording what this plan leaves standing:

- **`cargo fmt` is still unadopted**, and it is not the mechanical cleanup the notes assumed — rustfmt ignores `max_width` inside macro bodies, so all 42 one-line `emit!` calls explode, and even a tuned config leaves ~1,196 hunks. Investigated in Plan 8 and deferred with the measurements recorded; decide it rather than carrying it a fifth time.
- **The overdue-penalty continuous accrual** still needs its own plan. Task 7's sandwich probe pins the accounting around it but does not fix it.
- **`declare_id!` does not match the local deploy keypair** — settle before any deployment.
- **Devnet's missing dependencies** are the real work of deploying.
