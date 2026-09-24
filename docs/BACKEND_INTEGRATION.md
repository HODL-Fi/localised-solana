# Backend integration guide

For a backend that builds, signs and sends transactions to the HODL fixed-loans program.

- **IDL:** `idl/hodl_loans-devnet.json` and `idl/hodl_loans-mainnet.json` — 43 instructions,
  9 account types, 56 types, 43 errors. Take the one for the cluster you are on; do not edit the
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
  whatever language you build in. `take_loan.rs` is the one worth reading.
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
[ Switchboard: pull feed update  ]  ← 1–2 instructions
[ take_loan / withdraw_collateral / liquidate / write_off_loan ]
```

Do **not** refresh in a preceding transaction and borrow in the next. It works most of the
time and fails intermittently with `StalePrice` (6005) under load or congestion — the worst
kind of bug to debug in production.

**Priced instructions** (need both oracles): `take_loan`, `withdraw_collateral`, `liquidate`,
`write_off_loan`, and `revoke_promo` when the position has a live loan.

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
| 6002 | `Unauthorized` | signer is not the role the instruction requires |
| 6003 | `MarketPaused` | guardian paused it; retry later, do not loop |
| 6005 | `StalePrice` | **refresh the oracles and rebuild the transaction**, do not just retry |
| 6007 | `PriceAccountMismatch` | wrong price account for the asset, or the wrong-cluster binary. On a Switchboard-priced asset it also fires when the feed's on-chain `feed_hash` no longer matches the job the asset was listed against — i.e. the feed authority repointed it. That is not retryable: an operator has to re-point the asset or the feed. |
| 6010 | `InvalidParameters` | admin params out of bounds (e.g. `max_price_age_seconds > 60`) |
| 6011 | `Unhealthy` | the borrow would breach LTV — show the user their limit |
| 6013 | `UtilizationCapExceeded` | market is out of lendable cash; surface, do not retry |
| 6019 | `AmountTooSmall` | below `min_loan_amount` (currently 1,000 cNGN) |
| 6035 | `MathOverflow` | should not happen; log with the full instruction for investigation |

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

## 7. Setup order

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

## 8. Events

39 event types, all in the IDL with their 8-byte discriminators. Anchor emits them as
`Program data: <base64>` log lines: 8-byte discriminator followed by the Borsh body.

For an indexer, match on the discriminator rather than parsing log text.
`decode_events` in `programs/hodl_loans/tests/common/mod.rs` is a 15-line reference
implementation.

One correctness note the test suite pins: `LoanRepaid` carries **both** `owner` and `payer`,
and they differ when a third party repays. Do not attribute repayments to `owner`.

---

## 9. Bundling Switchboard updates — measured on devnet 2026-09-25

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
