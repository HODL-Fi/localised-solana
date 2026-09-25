# Backend integration guide

For a backend that builds, signs and sends transactions to the HODL fixed-loans program.

- **IDL:** `idl/hodl_loans-devnet.json` and `idl/hodl_loans-mainnet.json` — 43 instructions,
  10 account types, 59 types, 43 errors. Take the one for the cluster you are on; do not edit the
  other. The two files are byte-identical apart from the program id, and the program id appears in
  **two** places — the top-level `address`, and `initialize`'s `program` account, which is pinned to
  it so the instruction can check the upgrade authority. An earlier version of this guide said to
  swap `address`, which would have left the second one wrong.

  Regenerate both after any change to an instruction, account or error (see `Anchor.toml`):

  ```bash
  anchor idl build -- --features devnet > idl/hodl_loans-devnet.json
  anchor idl build                      > idl/hodl_loans-mainnet.json
  ```

  This matters more than it sounds. The IDL is how your client learns `CollateralAsset` has a
  `price_source` at all. A client deserializing with a pre-`price_source` IDL still succeeds —
  the account size did not change, the four new fields came out of `reserved` — and then treats
  every asset as Pyth, sending a `PriceUpdateV2` where a Switchboard feed belongs and getting
  `PriceAccountMismatch` (6007) on every priced instruction for that asset.
- **Reference client:** `setup-cli/src/bin/` — Rust, but the account layouts are the same
  whatever language you build in. Read `take_loan.rs` for the Pyth/`Standard` shape and
  `prestocks_loan.rs` for the Switchboard/`XStock` one; the difference between them is exactly
  the difference your client has to handle.
- **Live devnet addresses:** `.devnet/addresses.env`.

---

## 1. The thing that will bite you first: oracle freshness

Every priced instruction reads **two** oracles, and both must be fresh **in the same
transaction that uses them**.

| | source | freshness bound | where |
|---|---|---|---|
| collateral price | Pyth pull (`PriceUpdateV2`) | `max_price_age_seconds`, currently **60s** | per collateral asset, `price_source: Pyth` |
| collateral price | Switchboard On-Demand (`PullFeedAccountData`) | `sb_max_stale_slots`, **150 slots ≈ 60s** | per collateral asset, `price_source: SwitchboardOnDemand` |
| NGN/USD | Switchboard On-Demand (`PullFeedAccountData`) | `ngn_max_stale_slots`, currently **150 slots ≈ 60s** | per market |

`MAX_PRICE_AGE_SECONDS = 60` is a **hard protocol ceiling**, not a parameter you can raise —
`list_collateral` rejects anything above it with `InvalidParameters` (6010). The Switchboard
ceiling, `MAX_COLLATERAL_STALE_SLOTS = 150`, is the same 60 seconds at the 400 ms slot target.

**Read each asset's `price_source` — do not assume Pyth.** Both sources take exactly one price
account, so the account *count* never changes, but the account itself is a different program's and
supplying the wrong one fails with `PriceAccountMismatch` (6007). A collateral asset priced by
Switchboard exists because Pyth publishes no on-chain feed for it: the private-company PreStocks
marks are the case that forced it (`.devnet/prestocks/FINDINGS.md`), and exchange-hours equities
are the case where a 60-second ceiling cannot be met at all.

Note the different units. A Pyth asset ages in **seconds**, a Switchboard asset in **slots**. Under
congestion slots run 400-650 ms, so a slot bound drifts further behind wall clock exactly when
prices move fastest.

**Build one transaction shaped like this:**

```
[ Pyth: post price update        ]  ← from Hermes
[ Switchboard: pull feed update  ]  ← ONE feed only — see §9
[ take_loan / withdraw_collateral / liquidate / write_off_loan ]
```

Do **not** refresh in a preceding transaction and borrow in the next. For a single collateral asset
it works most of the time and fails intermittently with `StalePrice` (6005) under load — the worst
kind of bug to debug in production. **Past two feeds it cannot work at all.**

That is measured, not cautionary. Cranking four feeds (NGN plus three stocks) in separate
transactions took **196 slots** against a 150-slot bound, so the first feed was stale before the
last one landed. At roughly 49 slots a crank, two feeds is the ceiling — one priced collateral asset
plus NGN. A position holding three stocks is unborrowable unless the pull instructions ride in the
borrow's own transaction.

One trap worth inheriting: a crank returning `success: true` with agreeing oracles does **not** mean
the landed result is fresh. `result.slot` is the slot the oracles *signed* at, not the slot the
transaction landed in; one observed crank landed a result already 277 slots old. Check the account,
not the crank's return value — `.devnet/prestocks/check-freshness.js` does it at the right offsets.

**Exception — two Switchboard feeds.** Only one Switchboard update fits in a transaction (§9). With
the NGN feed *and* a Switchboard-priced collateral, the NGN feed has to be refreshed in a
transaction of its own immediately before; bundle the collateral feed, and rebuild both on
`StalePrice`. That is the shape the HODL backend ships.

**Priced instructions** (need both oracles): `take_loan`, `liquidate`, `write_off_loan`,
`withdraw_collateral` **only while the position has an active loan** (with none, pass `market` and
`ngn_feed` as `None` and no remaining accounts), and `revoke_promo` when the position has a live
loan.

**Unpriced** (no oracle accounts needed): `deposit_liquidity`, `withdraw_liquidity`,
`open_position`, `close_position`, `deposit_collateral`, `repay_loan`, plus the whole admin
surface. If your first milestone is plumbing, start here — none of it can fail on oracles.

---

## 2. Remaining accounts: the layout that is not in the IDL

Priced instructions take **variable trailing accounts** that Anchor's IDL does not describe.
Getting these wrong is the second most common failure.

For each collateral slot on the position **holding a non-zero amount**, in slot order, append:

```
1. CollateralAsset PDA   readonly   ["collateral", mint]
2. the price account     readonly   depends on the asset's price_source:
                                      Pyth                -> PriceUpdateV2
                                      SwitchboardOnDemand -> PullFeedAccountData
3. the mint itself       readonly   ONLY when the asset is kind == XStock
```

Account 2 is one account either way, so the stride is 2 for a `Standard` asset and 3 for an
`XStock` regardless of source. Which account it is comes from `CollateralAsset.price_source`, and
for Switchboard it is always `CollateralAsset.price_account` — that field is mandatory on that
path, where on the Pyth path `Pubkey::default()` means unpinned.

The third account exists because an xStock's scaled-UI multiplier lives on the mint. A
`Standard` asset must **not** include it — the program counts accounts positionally.

Slots with a zero amount are skipped entirely. So the account list changes as a user deposits
and withdraws; derive it from the position each time rather than caching.

Reference: `setup-cli/src/bin/take_loan.rs`, and `price_accounts()` in
`programs/hodl_loans/tests/common/mod.rs`.

---

## 3. PDAs

All derived from the program id. Seeds are byte strings.

| account | seeds |
|---|---|
| `Config` | `["config"]` |
| `Access` (per wallet) | `["access", wallet]` |
| `Market` | `["market", borrowable_mint]` |
| market vault | `["market_vault", borrowable_mint]` |
| `Lender` | `["lender", market_pda, owner]` |
| `CollateralAsset` | `["collateral", collateral_mint]` |
| collateral vault | `["collateral_vault", collateral_mint]` |
| `Position` (per wallet) | `["position", owner]` |
| `CreditRecord` (per wallet) | `["credit", owner]` |
| `PromoVault` | `["promo_vault", **market_pda**]` |
| promo vault token | `["promo_vault_token", **market_pda**]` |
| `Campaign` | `["campaign", …]` |
| `VoucherReceipt` | `["voucher", …]` |

**Note the two promo seeds hang off the market PDA, not the mint.** Deriving them from the
mint compiles fine and fails on-chain. It cost time here; it will cost you time too.

---

## 4. Error codes worth handling explicitly

Anchor custom errors are `6000 + variant position`. Full list in the IDL's `errors` array.

| code | name | what your backend should do |
|---|---|---|
| 6000 | `NotWhitelisted` | wallet needs `whitelist` first — surface as an onboarding step, not an error |
| 6001 | `Blacklisted` | wallet is blocked; do not retry, route to support |
| 6002 | `Unauthorized` | signer is not the role the instruction requires |
| 6003 | `MarketPaused` | guardian paused it; retry later, do not loop |
| 6005 | `StalePrice` | **refresh the oracles and rebuild the transaction**, do not just retry |
| 6007 | `PriceAccountMismatch` | wrong price account for the asset, or the wrong-cluster binary. On a Switchboard-priced asset it also fires when the feed's on-chain `feed_hash` no longer matches the job the asset was listed against — i.e. the feed authority repointed it. That is not retryable: an operator has to re-point the asset or the feed. |
| 6010 | `InvalidParameters` | admin params out of bounds (e.g. `max_price_age_seconds > 60`) |
| 6011 | `Unhealthy` | the borrow would breach LTV — show the user their limit |
| 6013 | `UtilizationCapExceeded` | market is out of lendable cash; surface, do not retry |
| 6014 | `InsufficientCash` | vault holds less cash than the loan; same handling as 6013. When bracketing a borrow limit, a thin pool reports this (or 6013) **before** `Unhealthy`, so a bisection measures the pool, not the collateral |
| 6019 | `AmountTooSmall` | below `min_loan_amount` (currently 1,000 cNGN) |
| 6021 | `RepaymentBelowInterest` | a repayment must cover interest + penalty due; tell the user the minimum |
| 6035 | `MathOverflow` | should not happen; log with the full instruction for investigation |
| 6037 | `InsufficientCollateral` | withdrawing more than the slot holds |

Not ours but you will see it: **6056 `ChecksumMismatch`** from the Switchboard program
(`Aio4ga…`) means a second feed update was put in the same transaction — see §9.

`6007` on **every** priced instruction while deposits still succeed is the signature of a
binary built for the wrong cluster. Check the build flags before debugging anything else.

---

## 5. Decimals and scaling

- cNGN is **6 decimals**. `min_loan_amount` of 1,000 cNGN is `1_000_000_000` raw units.
- The NGN feed reports **USD per NGN** (~0.00075), not NGN per USD. If you compute expected
  values off-chain, invert accordingly or your numbers will be off by ~1.3 million.
- `MAX_COLLATERAL_DECIMALS = 12`. Listing above that fails loudly at `list_collateral`.

---

## 6. Transaction size, not compute, is the binding limit on `liquidate`

At 8 standard collateral slots, `liquidate` measures **1,185 bytes** against the 1,232-byte
legacy limit. An all-xStock position (which adds a mint account per slot) will not fit —
use a **v0 transaction with an address lookup table** for liquidations.

Measured compute, for budgeting priority fees (`programs/hodl_loans/tests/budget.rs`):

| instruction | CU |
|---|---|
| `take_loan`, 8 `Standard` slots / 9 existing loans | ~88,500 |
| `take_loan`, 8 **Pyth** xStock slots / 9 existing loans | 96,407-96,455 |
| `take_loan`, 8 **Switchboard** xStock slots / 9 existing loans | 94,101-94,118 |
| `withdraw_collateral`, 8 slots / 10 loans | ~88,000 |
| `liquidate` + forfeit, 8 slots | ~114,000 |
| `repay_loan`, 10 loan slots | ~19,300 |
| `set_promo_cap` | **2,634 per listed asset** |

The price source barely moves compute, and what movement there is runs the *other* way from what
you might guess: Switchboard comes in ~2,300 CU below Pyth at eight slots, because its reader
copies 128 bytes of `CurrentResult` out by offset while Pyth runs a full `PriceUpdateV2`
deserialization. Eight slots of that outweighs eight 32-byte `feed_hash` compares. Do not
generalise it past eight slots — budget from the row you are actually in.

`set_promo_cap` is the one to watch: past **74 listed assets** it exceeds the 200,000 default
budget and needs an explicit `ComputeBudgetInstruction::set_compute_unit_limit`.
`MAX_LISTED_COLLATERAL` is 96, so this is reachable.

---

## 7. Credit history: the `CreditRecord` PDA

One account per borrower, `["credit", owner]`, 120 bytes. Two counters and nothing else:

```
loans_completed   a loan whose principal reached zero through repayment
loans_defaulted   a loan whose principal reached zero as a default
```

Derivable from a wallet address alone, so you need no registry to find one. It does not exist until
something closes a loan — reading it for a wallet that has never borrowed returns "account not
found", which is not an error condition.

**There is no score here, and there should not be.** The counters are facts; a score is a weighting
of those facts and belongs in a versioned off-chain function you can revise without a program
upgrade. What you index for that is the two events, which carry more than the counters do:

```
CreditRepaymentRecorded  borrower, credit_record, loan_id, principal, term_seconds,
                         days_late, penalty_paid, loans_completed
CreditDefaultRecorded    borrower, credit_record, loan_id, principal, term_seconds,
                         days_late, collateral_seized, loans_defaulted
```

The trailing counter is the value **after** the increment, so an indexer never has to read the
account back to know where it landed. Lateness lives in the event, never in the counter: a loan
repaid 400 days late still counts as completed, because encoding degrees of lateness in a counter
would be scoring.

### What changed in the instructions you already call

| instruction | change |
|---|---|
| `repay_loan` | **two new accounts** — `credit_record`, `system_program` — and **`payer` is now `mut`** |
| `liquidate` | `credit_record` and `system_program` added, both **optional** |
| `write_off_loan` | `credit_record` and `system_program` added, both **required**; `admin` is now `mut` |

**`repay_loan`'s payer needs SOL.** On a borrower's *first* repayment the payer funds the record's
rent, about 0.0017 SOL, once per borrower ever. A zero-lamport payer fails with a bare system-program
error 1 — `Transfer: insufficient lamports 0, need 1726080` in the logs — which looks nothing like a
program error and is unpleasant to diagnose. Your admin wallet pays for every loan transaction, so
this is already covered; it matters if a borrower's own wallet ever signs a repayment.

**The record is always the borrower's, never the payer's.** Any whitelisted wallet may repay any
position's loan, and the history follows whoever borrowed. Seed it from the position's `owner`, not
from the signer. If you get this wrong the transaction fails on the seeds constraint rather than
crediting the wrong wallet, so it is not a silent error — but it is a confusing one.

### Why `liquidate`'s are optional and the others' are not

Adding the account took `liquidate` from 1,185 to 1,251 bytes at 8 collateral slots, past the
1,232-byte legacy limit, and the `system_program` alone accounts for 32 of that. Requiring it would
mean a liquidation that cannot be packed is an underwater position that cannot be closed — a
solvency problem, and strictly worse than a counter that did not move.

So: **pass both when your transaction has room, omit both when it does not.** Omitting them leaves
the transaction exactly the size it was before credit records existed, because Anchor encodes an
absent optional account as the program id and the message compiler dedupes it. `LoanLiquidated` is
emitted either way, so an indexer reconstructing defaults from events loses nothing.

**Most defaults land at `write_off_loan`, not at `liquidate`.** A liquidation is bounded by the
collateral it can seize, so it usually leaves principal behind; the write-off is what finally clears
a loan whose remaining collateral is dust. That is why the write-off carries the counter as a
required account — it is the path that actually fires. If you only wired `liquidate`, your default
count would read near zero on a book with real defaults in it.

---

## 8. Setup order

Mirrors `Env::initialized()` → `loan_ready()` in the test harness, which is the tested path:

1. `initialize` — **must be signed by the program's upgrade authority**; that wallet becomes admin
2. `create_market` per borrowable mint
3. `create_promo_vault` — **required**: `take_loan`, `liquidate` and `write_off_loan` all name
   it, so a market without one cannot be borrowed against
4. `list_collateral` per asset. For a Switchboard-priced asset the feed must exist first, because
   `price_account` is mandatory on that path and the asset records the feed's `sb_feed_hash`.
   Listing does not require the feed to be *working* — borrowing does. One trap worth inheriting:
   a pull feed created with `min_responses` greater than its number of jobs can never reach quorum
   and is permanently uncrankable, with no error at creation time. Set `min_responses ≤ jobs`; see
   `.devnet/prestocks/README.md`.
5. `whitelist` per wallet
6. `deposit_liquidity` — lenders fund before anyone can borrow

---

## 9. Events

39 event types, all in the IDL with their 8-byte discriminators. Anchor emits them as
`Program data: <base64>` log lines: 8-byte discriminator followed by the Borsh body.

For an indexer, match on the discriminator rather than parsing log text.
`decode_events` in `programs/hodl_loans/tests/common/mod.rs` is a 15-line reference
implementation.

One correctness note the test suite pins: `LoanRepaid` carries **both** `owner` and `payer`,
and they differ when a third party repays. Do not attribute repayments to `owner`.

---

## 10. Bundling Switchboard updates — measured on devnet 2026-09-25

§1 says to put every oracle update in the same transaction as the priced instruction. With two
Switchboard feeds (NGN + a Switchboard-priced collateral) that is **not possible** as the SDK ships:

1. **One secp-verified feed update per transaction.** `PullFeedSubmitResponseConsensus` checks its
   signatures against the *first* secp256k1 instruction in the transaction. Two updates in one
   transaction: the first submit passes, the second always fails `ChecksumMismatch` (6056),
   whichever feed is second.
2. **Switchboard hardcodes the secp instruction index to 0.** Crossbar's `pullIxns[0]` carries
   `signature_instruction_index = eth_address_instruction_index = message_instruction_index = 0`
   for every signature. Anything ahead of it (a compute-budget instruction) makes the precompile
   fail with custom error 2. Rewrite the three index bytes of each 11-byte offset record
   (`data[1 + 11*i + {2, 5, 10}]`) to the secp instruction's real position.
3. **`ngn_min_samples` is 3 on the devnet market**, so the NGN crank needs `numSignatures >= 3`;
   two signatures lands a 2-sample result and `take_loan` fails `StalePrice` (6005) on the sample
   check, not the slot check.
4. **Size.** Collateral pull (3 signatures) + `take_loan` is 850 bytes with an address lookup
   table holding the non-signer, non-program accounts — it does not fit without one.

The shape that works (landed `3hBBs2cz…` on devnet):

```
tx 1  [compute budget][secp (NGN, 3 sigs)][submit NGN]                   sponsor signs
tx 2  [compute budget][secp (collateral)][submit collateral][take_loan]  sponsor + owner sign, v0 + ALT
```

The collateral price is structurally fresh. NGN is one transaction older — seconds, against a
150-slot window — and on `StalePrice` the backend rebuilds both.

---

## 11. Everything else the HODL backend learned wiring this up (devnet, 2026-09-25)

Reference implementation: `localised-backend` `src/loans/` (`loans-chain.service.ts` for the
transaction plumbing, `loans.lib.ts` for the pure maths and the secp patch).

**Signing and fees.** Every instruction a user signs has a separate fee/rent payer except where the
user *is* the payer: `open_position` takes `payer` distinct from `owner`, so a sponsor can open a
position for a wallet holding no SOL. Put the sponsor first as fee payer, have the owner sign, and
have the sponsor sign last. `repay_loan`'s `payer` is the cNGN source and must be the signer whose
token account pays.

**`withdraw_collateral` prices the position *after* the withdrawal.** The handler decrements the
slot before `load_health`, so the remaining accounts must describe the post-withdrawal slots — a
slot drained to zero is skipped. Withdrawing everything while a loan is open therefore leaves
nothing to price and can never pass; refuse it off-chain with "repay first".

**`repay_loan` takes `min(amount, total owed)`** and requires `amount ≥ interest + penalty`. Interest
accrues every second between reading the position and landing, so for a full repayment send a
little more than the balance you read (two minutes of interest is plenty) — the program charges
the exact amount.

**Reading a Switchboard price off-chain the way the program does.** `PullFeedAccountData` holds the
aggregated `CurrentResult` at byte **2264** (located against a live devnet feed): `value` i128 at
+0, `num_samples` u8 at +96, `slot` u64 at +104. `value` is scaled by 1e18. Use this for display and
borrow-limit estimates so they match what the program enforces.

**Crossbar.**
- Use `fetchSolanaUpdates(network, [feed], payer, numSignatures)` — one feed per call. `fetchUpdateIx`
  also calls `/gateways`, which a self-hosted Crossbar does not serve.
- A fetch takes ~3–10 s. Fetch the bundled collateral update *while* the NGN crank lands, not after;
  that cut a borrow from ~37 s to ~15 s end to end.
- The instructions come back built against Crossbar's own copy of `@solana/web3.js`; re-wrap them in
  your own `TransactionInstruction`s if your versions differ.

**The lookup table drifts.** Which oracles answer — and so which oracle accounts a pull names —
changes between fetches. Keep the table, and before each borrow extend it with any non-signer,
non-program key it lacks. An extended table is usable only from the next slot; wait for it.

**ScaledUiAmount.** The program moves raw units; people think in display units
(`raw × multiplier / 10^decimals`). With a multiplier like 1.4861347 no raw amount maps to a round
display number, and flooring in both directions turns "deposit 2" into `1.999999999`. Round to the
nearest raw unit on the way in, clamp to the balance actually held, and round display output. RPC
`jsonParsed` token balances (`uiAmountString`) already apply the multiplier.

**cNGN on devnet is Token-2022** (`GqKF9H…`), so `take_loan`/`repay_loan` pass the Token-2022
program and cNGN associated token accounts are Token-2022 ATAs.

**RPC.** A single borrow makes ~20 reads; the public devnet endpoint starts returning 429 partway
through. Use a keyed provider.
