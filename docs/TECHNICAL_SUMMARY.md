# Technical summary — Switchboard collateral, PreStocks, credit records

What was built on 2026-09-24/25, why each decision went the way it did, and what is still open.
Written to be read by someone who was not here.

Companions: [`BACKEND_INTEGRATION.md`](BACKEND_INTEGRATION.md) is the client contract,
[`.devnet/prestocks/FINDINGS.md`](../.devnet/prestocks/FINDINGS.md) the on-chain facts about the real
PreStocks mints, [`.devnet/prestocks/README.md`](../.devnet/prestocks/README.md) the devnet replay.

---

## 1. Summary

Three pieces of work, in dependency order:

1. **A second collateral price source.** `CollateralAsset` can now be priced by a Switchboard
   On-Demand pull feed instead of Pyth. This was not optional — Pyth cannot price the assets we
   wanted to accept.
2. **Eight PreStocks tokens listed as collateral on devnet**, each a Token-2022 mock of the real
   mainnet mint, each with its own Switchboard feed over the PreStocks API.
3. **A `CreditRecord` PDA** — per-borrower on-chain credit history, counters only.

Plus the infrastructure the first two depend on: **Crossbar moved from a laptop container to
Railway**.

State: **311 tests** (282 before this work), clippy clean under both feature builds, devnet deployed
and byte-verified against the committed source. `./scripts/test.sh` is the only correct way to run
them — it rebuilds the `.so` first, and a bare `cargo test` will happily run the default build against
a devnet binary and fail with `DeclaredProgramIdMismatch` (4100), which is a confusing way to learn
that.

The two files this work added:

```
credit_record            8 tests
switchboard_collateral   7 tests
```

plus a Switchboard budget case and unit tests inside `hodl_loans` (74) for the `PriceSource`
validation rules, the `CreditRecord` counters and the lateness arithmetic.

---

## 2. Why Pyth was not enough

The work started from a question — can PreStocks tokens be collateral — and the answer turned on
three facts, all measured rather than assumed:

**Pyth has no on-chain feed for them.** Its catalogue carries index feeds for OPENAI, ANTHROPIC and
SPCX only; nothing for ANDURIL, NEURALINK, KALSHI, POLYMARKET or FIGUREAI. And even those three have
no sponsored push account — the `pythWSnswVUd12oZpeFP8e9CVaEqJg25g1Vtc2biRsT` PDAs were derived for
shards 0–3 and checked on both clusters, all absent. The derivation is sound: the same method
reproduces the working SOL/USD account `7UVimffxr9ow…`. Hermes returns **HTTP 401** without a key.

**The 60-second ceiling rules out sponsored equity feeds generally**, including the Backed xStocks
this program was originally built for. `Equity.US.AAPL/USD` was **41 days stale on mainnet and 84 on
devnet** when checked. Even with a Hermes key the problem is structural: equities stop publishing at
the close, and `MAX_PRICE_AGE_SECONDS` is a hard 60.

**PreStocks marks are continuous** — SPV valuations, not an order book — so a Switchboard job over
the issuer's own HTTP endpoint fits the ceiling that Pyth equity feeds structurally cannot.

---

## 3. Program change: `PriceSource`

`CollateralAsset` gained four fields, taken **out of** `reserved` so `INIT_SPACE` stays 294 and every
already-listed asset still deserializes:

| field | bytes | meaning |
|---|---|---|
| `price_source` | 1 | `Pyth` or `SwitchboardOnDemand` |
| `sb_feed_hash` | 32 | the job the feed must be running |
| `sb_max_stale_slots` | 8 | freshness in slots |
| `sb_min_samples` | 4 | oracle quorum |

**`Pyth` is discriminant 0.** Assets listed before the field existed read as `Pyth` out of zeroed
padding, so nothing changed meaning on upgrade. Verified against live data after deploying: wSOL,
listed by the previous binary, still reads `price_source Pyth` with its 60-second bound intact, all
`sb_*` zero, account still 302 bytes. Reordering those variants would silently repoint every live
asset, which is why a test pins the discriminant.

**A Switchboard asset must be pinned AND hash-bound.** Unpinned is coherent for Pyth — a
`PriceUpdateV2` carries a verified feed id, so an unpinned asset only lets the caller choose among
verified updates inside its age window. A `PullFeedAccountData` carries no such binding: any account
owned by the Switchboard program with the right discriminator would satisfy an unpinned read. And the
pin alone is not enough either, because a pull feed's `authority` can rewrite the account's
`feed_hash` and repoint a pinned address at a different job — address, owner and discriminator all
still checking out. `sb_feed_hash` is what makes that fail instead of silently repricing collateral.

`valuation.rs` branches at its one collateral-price call site. The stride is unchanged — one price
account either way — so the owner check is what separates the two layouts. A test passes a valid,
fresh Pyth update where a Switchboard feed belongs and asserts `PriceAccountMismatch`.

### The measurement that nearly passed for the wrong reason

The headline test bisects the borrow ceiling under each source and asserts they agree to the raw unit:
**1,215,049,478,079** raw cNGN for one share at $1,023.01 and a 1.4861347 multiplier.

Its first version shared one market across probes. It reported both sources agreeing on
**1,214,355,468,750** — which looks exactly like the property under test, and was really the 90%
utilization cap in both cases, 0.057% below the LTV limit. The guard on the bisection's failure path
caught it: only `Unhealthy` counts as "too big", anything else panics. That is why the helper takes an
environment builder rather than an environment.

The same trap recurred on devnet hours later: a bracket test reported `InsufficientCash` because
994,000 cNGN of liquidity capped borrowing at ~894,600 while the LTV limit was ~2.02M.

---

## 4. Program change: `CreditRecord`

One account per borrower, `["credit", owner]`, 120 bytes on chain. Two counters:

```
loans_completed   principal reached zero through repayment
loans_defaulted   principal reached zero as a default
```

Derivable from a wallet address alone, so the history cannot be shed by abandoning a position and
opening another — the address is the identity. Nothing decrements or resets. Lateness lives in the
events (`days_late`, `penalty_paid`), never in the counter, because encoding degrees of lateness in a
counter would be scoring, and a score is a weighting that belongs in a versioned off-chain function.

Two events carry what an indexer needs, including the counter value **after** the increment so the
account never has to be read back:

```
CreditRepaymentRecorded  borrower, credit_record, loan_id, principal, term_seconds,
                         days_late, penalty_paid, loans_completed
CreditDefaultRecorded    borrower, credit_record, loan_id, principal, term_seconds,
                         days_late, collateral_seized, loans_defaulted
```

### Three places the design had to diverge from its brief

**The specified `LoanRepaid` event already existed**, with richer fields, and was test-pinned. Adding
it would not have compiled and would have lost information. Hence the two `Credit*` events instead.

**`liquidate` could not take the account as required.** Adding it took an 8-collateral-slot
liquidation from **1,185 to 1,251 bytes**, past the 1,232-byte legacy limit — and the unconditional
`system_program` accounted for 32 of that, enough on its own to push 7-standard-plus-1-xStock from
1,220 to 1,252. Requiring it would mean *a liquidation that cannot be packed is an underwater position
that cannot be closed*, which is a solvency problem and strictly worse than a counter that did not
move. Both accounts are `Optional` there, so omitting them leaves the transaction exactly its former
size; Anchor encodes an absent optional account as the program id and the message compiler dedupes it.
`LoanLiquidated` is emitted either way, so an indexer reconstructing defaults from events loses
nothing.

**Wiring only `liquidate` would have left `loans_defaulted` almost never firing.** A liquidation is
bounded by the collateral it can seize, so it usually leaves principal behind — the first test
asserted a full close and could not get one in eight calls. **`write_off_loan` is the terminal
default**, so it carries the counter too, and required rather than optional: admin-only, few accounts,
no size pressure to trade against.

### A consequence worth knowing

`repay_loan`'s payer funds the record's rent — about **0.0017 SOL**, once per borrower ever — on that
borrower's first repayment. A zero-lamport payer fails with a bare system-program error 1
(`Transfer: insufficient lamports 0, need 1726080`), which looks nothing like a program error. A
transaction fee payer does not help: `init_if_needed` CPIs the system program with the *instruction's*
`payer` as the funding source, so that account itself needs the lamports.

---

## 5. Devnet state

```
program        q33KxkuB2ntHSBwPnuFGpkiCxxEmmxsAgYAM6Gjx2SN    deployed == HEAD build
mainnet id     5t7smXPGCvTMXYUAJ2uU4Zk4uPggkN7KdanoywHrveMd   never deployed
crossbar       https://crossbar-staging.up.railway.app
```

**Ten collateral assets** (`Config.collateral_count = 10`): wSOL and a mock USDC on Pyth, and all
eight PreStocks mocks on Switchboard at LTV 50% / LT 75% / bonus 10%, deposit cap 100 raw tokens.

| symbol | devnet mock mint | multiplier | mark |
|---|---|---|---|
| OPENAI | `ECbSTTymP6eUkNy4Q8VF4DNJNULXxKz63iUk7vnen2xR` | 1.4861347 | $1,023.7 |
| SPACEX | `7b5C13nRLTuiecGGdHb9KX63n2gUqU9LZnkzHV9W6K3z` | **5** | $148.2 |
| ANTHROPIC | `5ddG2fZ99xUtMWisqKWfXS1LZRfa6wF4R6jXae2cUvZc` | 1 | $1,047.8 |
| ANDURIL | `HLH1vb29GYVjgYpGgRau5firRE7aUMqNb3anLnyaPPmv` | 1 | $155.0 |
| NEURALINK | `CSKqv3CtDwDz3nHaqQEoEPBqFSgsx22q2ekNmksMgY7X` | 1 | $336.5 |
| KALSHI | `EeQ9MBtXdRFWCyJhAsxnKXjfzo66ntg39GHxhps6vmHX` | 1 | $882.8 |
| POLYMARKET | `7Z56jHJHEf82MCapqsdJS6VyosX33Hqrw44tHasE9jH3` | 1 | $144.5 |
| FIGUREAI | `E8EPGxCjX3habe7mre698SPPKcuv28DusgDXJWfVYvRk` | 1 | $180.3 |

Deposit caps are in **raw** units, so their dollar value varies with the multiplier: 100 raw SPACEX is
500 display shares, 100 raw OPENAI is 148.6.

**Proven end to end:** 2,001,000 cNGN borrowed against 2 OPENAI shares at a Switchboard-read price of
$1,023.62, and the price shown to *bind* — 2,500,000 cNGN refused `Unhealthy`, 2,000,000 accepted,
against a hand-computed limit of ~2,018,575.

**Credit record:** `56vX6uddYWcaeHyN4EeyhRt58AnoRJhCMmpazrKtAGhc`, `loans_completed: 2` after two real
repayments, with the event decoded off the live transaction.

---

## 6. A mistake worth recording

`symbols.js` originally hardcoded every multiplier, and had **SPACEX at 1 when it had been 5 since
2026-06-10**. Three of the eight mints were read in detail and the rest were defaulted. A mock at 1
against a real asset at 5 is undervalued fivefold and nothing in the pipeline would have complained.

The fix is not "check more carefully" — it is to stop storing the value. `create-mint.js` now reads
each multiplier off the mainnet mint at mint time and cross-checks it against the API's
`ui_supply / raw_supply`, throwing when the two disagree. An issuer-controlled value cannot stay
correct in a hand-copied table.

---

## 7. Measurements

Compute, worst case at 8 collateral slots with 9 existing loans, ranges not samples:

| path | CU |
|---|---|
| `take_loan`, 8 `Standard` | ~88,500 |
| `take_loan`, 8 **Pyth** xStock | 96,407–96,455 |
| `take_loan`, 8 **Switchboard** xStock | 94,101–94,118 |
| `repay_loan`, 10 loan slots | 25,724–28,724 (was 19,297–19,313) |
| `liquidate` + forfeit, 8 slots | 120,358–124,858 (was ~114,000) |
| `liquidate` + forfeit, 8 xStock | 132,943 |

Switchboard comes in **~2,300 CU below Pyth** at eight slots, which is the opposite of what the extra
32-byte hash compare suggests. The reader is why: Switchboard copies 128 bytes of `CurrentResult` out
by offset, while Pyth runs a full `PriceUpdateV2` deserialization, and eight slots of that outweighs
eight hash compares. Do not generalise past eight slots.

A single devnet `take_loan` measured 38,061 CU for the Switchboard xStock path against wSOL's 36,072 —
but those differ by a mint unpack (`XStock` vs `Standard`), not by price source, so that pair says
nothing about the reader. It was briefed the wrong way round before being measured properly.

---

## 8. Known limits

**A borrower can use one stock at a time.** Cranking four feeds in separate transactions took **196
slots** against a `sb_max_stale_slots` bound of **150** — so the first feed is stale before the last
lands:

```
NGN         age 198   STALE   <- cranked first
OPENAI      age 158   STALE
SPACEX      age 118   fresh
ANTHROPIC   age  82   fresh   <- cranked last
```

At ~49 slots a crank that puts the ceiling at two feeds — one stock plus NGN, which is exactly the
single-asset borrow that works. Three or more cannot be made simultaneously fresh by separate
transactions; no amount of sequencing fixes a window shorter than the work. The fix is the shape both
docs already call correct: put the pull instructions in `take_loan`'s own transaction. **Not built.**
The account layout is already proven — a three-asset attempt passed every oracle check, nine remaining
accounts in slot order accepted, and failed only on the utilization cap.

Underneath that: **a crank returning `success: true` with three agreeing oracles does not mean a fresh
result.** `result.slot` is the slot the oracles *signed* at, not the slot the transaction landed in;
one observed crank landed a result already **277 slots** old. `check-freshness.js` reads it off the
account at the offsets the program uses.

**The real PreStocks mints cannot be listed, and the refusal is correct.** Every one carries
`TransferFeeConfig` at a live **100 bps**, uncapped and issuer-adjustable, plus
`ConfidentialTransferFeeConfig` — neither in `XSTOCK_COLLATERAL_EXTENSIONS`. `deposit_collateral`
credits the amount it asks to transfer, so a fee credits a position more than the shared per-mint
vault received, and the shortfall is socialised across positions. Supporting them needs fee-aware
accounting on all four collateral-moving paths. **Not built** — which is why the devnet mocks omit the
fee, and why devnet cannot be used to argue the real mints are safe to list.

**Issuer trust.** One key — `WV9PJN7XTmTLVwbutCLFxp8TyePee6Xq5mRq6Fti5Wc` — holds permanent delegate,
freeze, pause, transfer-fee, scaled-UI multiplier and transfer-hook authority across all eight mints.
The permanent delegate can move collateral out of our vault; the pause blocks every transfer including
liquidation of an already-underwater position. Spec §14 records the delegate and pause as accepted
issuer risks for Backed's xStocks; the difference here is that a single key holds all of them plus an
uncapped fee dial.

**Mainnet is undeployed**, and the program has never been externally audited.

---

## 9. Infrastructure: Crossbar on Railway

Self-hosting is not a preference. The hosted `crossbar.switchboard.xyz` returns the hash of an
**empty payload** from `/store`, so a feed created against it can never be resolved and every crank
dies with `ORACLE_UNAVAILABLE`. Four different request shapes returned the same empty-payload hash.
Its `/simulate/jobs` is broken too — the known-good NGN job fails there identically.

Deployed to `gregarious-compassion` / `staging` as service `crossbar`, because `localised-backend`
lives in that project and Railway private networking is per-project-per-environment: the backend can
call `crossbar.railway.internal` without leaving Railway. `prod-active` is untouched.

One change from the upstream image: its entrypoint hard-codes `PORT=8080`, which makes Railway's
injected `$PORT` a no-op and would have left the HTTP proxy pointing at a port nothing listens on. The
replacement is the same script with the same restart supervisor, binding `${PORT:-8080}`. The base
image is pinned **by digest**, not `:latest`, because feeds resolve through this and a silent retag
would surface as a failing borrow.

Verified in the order that establishes something: it bound the port and the proxy found it;
`/simulate` served the live OPENAI job; `/simulate/solana/devnet` resolved an on-chain feed by pubkey;
a **novel** job stored through Railway was retrievable from the *public*
`crossbar.switchboard.xyz/fetch` within ~3s (the only check that proves oracles can resolve feeds
created there); a real crank landed on chain; and then the **local container was stopped** and the NGN
feed still cranked — the only check that proves nothing quietly depends on a laptop.

Two settings left open: the endpoint is **public and unauthenticated** (anyone can `POST /store` and
spend the configured RPC quota via `/updates` — Switchboard's own hosted instance is public too, so
this is the normal posture, but the bill is ours), and it runs on the **public Solana RPCs**, which
rate-limit. We hit `api.mainnet-beta.solana.com` from a single machine during this work, and its 429
body is not JSON — fed to `JSON.parse` it surfaces as `Unexpected token 'T'`. A shared Railway egress
IP makes that likelier. Both are variable changes, no redeploy.

---

## 10. Backend integration status

`localised-backend/src/loans/` is current for the PreStocks path: the IDL carries `CreditRecord` and
`PriceSource`, `creditRecordPda()` is wired into `repayLoan` with an `accountExists` check for the
first-touch rent, all eight assets are in `loans.config.ts` env-overridably, and `priceAccounts()`
builds `(CollateralAsset, feed, mint)` in slot order.

**One gap.** `priceAccounts()` pushes `asset.switchboardFeed` unconditionally and every configured
asset is hardcoded `kind: 'XStock'` — so the backend assumes every collateral asset is a
Switchboard-priced xStock. That holds for the eight it knows, but the program lists **ten**, and wSOL
and mock USDC are Pyth-priced `Standard`. A position holding wSOL would get a Switchboard feed where a
`PriceUpdateV2` belongs and fail `PriceAccountMismatch` (6007) on every priced instruction. The fix is
to branch on `CollateralAsset.price_source`, which the IDL now exposes, rather than rely on a config
field that only ever holds one kind.

Nothing in the backend references `CROSSBAR_URL`, so if it does its own cranking it needs pointing at
Railway — `http://crossbar.railway.internal:8080` from inside the same project and environment.

---

## 11. Commits
- `7d01d06` feat: Switchboard On-Demand as a collateral price source
- `4d5694f` feat: devnet tooling for PreStocks collateral, and the listing CLI
- `3e5245e` docs: spec §7/§8/§20 and backend guide for the second collateral price source
- `088f648` feat: end-to-end PreStocks borrow CLI
- `234653a` feat: PreStocks collateral working end to end on devnet
- `eec9d8f` docs: regenerate both IDLs, record the real Switchboard compute cost
- `620322d` docs: point the backend at both reference clients, not just the Pyth one
- `228a635` docs: what the backend learned wiring up Switchboard-priced borrowing
- `c76542f` feat: list all eight PreStocks mocks; find the feed-freshness ceiling
- `50f2904` feat: CreditRecord PDA — on-chain credit history per borrower
- `b288c9d` docs: regenerate both IDLs and hand the backend the CreditRecord contract
- `4a22f09` docs: point the backend at the two credit-path CLIs as well
- `5984cd5` docs: a sponsored fee does not pay the CreditRecord rent
- `4b372d0` feat: Crossbar deployable to Railway
- `edaf17b` feat: Crossbar live on Railway; scripts no longer need a laptop

