# PreStocks as collateral on devnet

What exists, what is blocked, and the exact commands left. Background and the mainnet facts are
in [FINDINGS.md](FINDINGS.md).

## What the program now does

`CollateralAsset` carries a `price_source`. `Pyth` is unchanged and is the zero discriminant, so
every asset listed before the change reads as `Pyth` out of zeroed padding. `SwitchboardOnDemand`
reads a `PullFeedAccountData` — the same account shape the market's NGN feed uses — and must be
both **pinned** (`price_account`) and **hash-bound** (`sb_feed_hash`).

Proven in LiteSVM by `programs/hodl_loans/tests/switchboard_collateral.rs` (7 tests). The headline
one bisects the borrow ceiling under each source and asserts they agree to the raw unit:
**1,215,049,478,079** raw cNGN for one share at $1,023.01 and a 1.4861347 multiplier.

## Devnet state

Ten collateral assets listed (`Config.collateral_count = 10`): wSOL and a mock USDC on Pyth, and
all eight PreStocks mocks on Switchboard. Every stock feed cranks with 3 oracle samples.

| symbol | mock mint | multiplier | mark |
|---|---|---|---|
| OPENAI | `ECbSTTym…` | 1.4861347 | $1,023.7 |
| SPACEX | `7b5C13nR…` | **5** | $148.2 |
| ANTHROPIC | `5ddG2fZ9…` | 1 | $1,047.8 |
| ANDURIL | `HLH1vb29…` | 1 | $155.0 |
| NEURALINK | `CSKqv3Ct…` | 1 | $336.5 |
| KALSHI | `EeQ9MBtX…` | 1 | $882.8 |
| POLYMARKET | `7Z56jHJH…` | 1 | $144.5 |
| FIGUREAI | `E8EPGxCj…` | 1 | $180.3 |

All at LTV 50% / LT 75% / bonus 10%, deposit cap 100 **raw** tokens — so the cap's dollar value
varies with the multiplier: 100 raw SPACEX is 500 display shares, 100 raw OPENAI is 148.6.

Borrowed so far: 2,002,000 cNGN against 2 OPENAI shares, and the price shown to bind (below).

**SPACEX's multiplier is 5, and this file first recorded it as 1.** It had been 5 since
2026-06-10; only three of the eight mints were actually read and the rest were defaulted. A mock
at 1 against a real asset at 5 is undervalued fivefold and nothing would have complained.
`symbols.js` no longer stores multipliers at all — `create-mint.js` reads each from the mainnet
mint at mint time and cross-checks it against the API's `ui_supply / raw_supply`, throwing if the
two disagree.

The mint carries `metadataPointer, scaledUiAmountConfig, permanentDelegate, transferHook,
defaultAccountState, pausableConfig, tokenMetadata` — every one inside
`XSTOCK_COLLATERAL_EXTENSIONS` — with the multiplier at the live mainnet **1.4861347**,
deliberately not 1, so the scaled-UI path is actually exercised.

It does **not** carry the mainnet mints' 100 bps `TransferFeeConfig`. That is the one way these
mocks do not mirror the real asset, and it means devnet cannot be used to argue the mainnet mints
are safe to list. `deposit_collateral` credits the amount it asks to transfer, so a fee would
credit a position more than the shared per-mint vault received.

### The price binds, not just the plumbing

`take_loan` succeeding proves the accounts line up. It does not prove the *price* is doing
anything. Bracketing does:

```
2 raw shares × 1.4861347 = 2.9722694 display × $1023.6165 = $3,042.46
at 50% LTV                                                = $1,521.23
at $0.000753652729/NGN                                    ≈ 2,018,479 cNGN

2,500,000 cNGN  ->  Unhealthy (6011)
2,000,000 cNGN  ->  OK
```

The pool had to be topped up first (`fund_market`). At 994,000 cNGN the utilization cap refused
anything past ~894,600, so the bracket measured the **cap** and reported `InsufficientCash` — the
same masking trap the LiteSVM bisection hit, in the same shape, twice in one day.

### Migration: assets listed by the old binary are untouched

Decoded off chain after the upgrade with `check-collateral.js`:

```
wSOL  (listed by the OLD binary)   price_source Pyth   max_price_age_seconds 60   sb_* all zero
OPENAI (listed by the new one)     price_source SwitchboardOnDemand   sb_max_stale_slots 150
```

Both accounts are still 302 bytes (8 discriminator + `INIT_SPACE` 294), and wSOL's 0.5 wSOL
deposit is intact. That is the claim `PriceSource::Pyth = 0` was designed to make, checked against
live data rather than only in LiteSVM.

## What actually went wrong with the first three feeds

Worth recording because the wrong diagnosis survived three feeds and a commit.

`ORACLE_UNAVAILABLE: No oracle responses received` from every gateway, for a feed Crossbar could
resolve and simulate perfectly. **The cause was `minResponses: 2` on a feed with one job.** A
single job can never produce two responses, so the aggregation never reaches quorum and the
oracles return nothing. `min_responses: 1` on the identical job cranked first time.

| feed | jobs | `min_responses` | answers? |
|---|---|---|---|
| `J9SkmK6UVDie…` | 1 (prestocks, filter path) | 2 | no |
| `4TMd3L3WfGDL…` | 1 (prestocks, positional path) | 2 | no |
| `B5tPYx7Jmpkn…` | 1 (open.er-api.com — a URL known reachable) | 2 | no |
| `GDgs76wotM4m…` (NGN) | **2** | 2 | yes |
| `DTXX9vQdGojn…` | 1 (prestocks, filter path) | **1** | **yes** |

Two things I got right and one I got wrong. Right: the JSONPath was not the problem, and
prestocks.com was not blocking the oracles — the positional feed and the er-api control ruled those
out. Wrong: I read the control's failure as evidence the oracles had not *indexed* a new feed,
because the only property I noticed the three failures sharing was being new. They also all had one
job against `min_responses: 2`, and my control changed the URL while holding that constant — so it
could never have separated the two. A control has to vary only the variable under test.

**So: set `min_responses ≤ number of jobs`.** A feed created otherwise is permanently uncrankable
and has to be replaced; `list_prestocks` re-points an existing listing for exactly that reason.

## The hard limit: you cannot crank more than about two feeds per borrow

Measured, not estimated. Cranking four feeds sequentially (NGN + three stocks) takes **196 slots**
of wall time, against a `sb_max_stale_slots` bound of **150**. So the first feed is already stale
before the last one lands:

```
start slot 503785107, end 503785303  -> 196 slots for four cranks

  NGN         age 198   STALE   <- cranked first
  OPENAI      age 158   STALE
  SPACEX      age 118   fresh
  ANTHROPIC   age  82   fresh   <- cranked last
```

At roughly 49 slots a crank that puts the ceiling at **two feeds** — one stock plus NGN, which is
exactly the single-asset borrow that works. Three or more feeds cannot be made simultaneously fresh
by separate transactions, and no amount of sequencing fixes it; the window is shorter than the work.

There is one way out and it is the one the docs already call correct: **put the pull instructions in
the same transaction as `take_loan`**. Then every result lands in the borrow's own slot and freshness
is structural rather than a race. It is not built here — `prestocks_loan` cranks separately — so
**a borrower can currently use one stock at a time on devnet**, and a multi-stock position is
listed-but-unborrowable until that exists.

Two things that made this hard to see, both worth keeping:

- **A successful crank does not mean a fresh result.** `result.slot` is the slot the *oracles
  signed* at, not the slot the transaction landed in. One ANTHROPIC crank returned
  `success: true` with three agreeing oracles and landed a result already **277 slots** old.
- Use `check-freshness.js` rather than reading the crank's own output. It reads
  `result.slot`/`num_samples`/`value` straight off each account at the offsets the program uses
  (2368 / 2360 / 2264, verified by reading `value` back against a known crank) and exits non-zero
  if any feed would fail.

```bash
node check-freshness.js $OPENAI_FEED $SPACEX_FEED $NGN_FEED
```

## Redoing it from scratch

```bash
docker start crossbar || docker run -d --name crossbar -p 8099:8080 \
  -e SOLANA_DEVNET_RPC=https://api.devnet.solana.com \
  -e SOLANA_MAINNET_RPC=https://api.mainnet-beta.solana.com \
  switchboardlabs/crossbar:latest

cd .devnet/prestocks
node create-mint.js OPENAI 1000          # -> OPENAI_MINT
node create-feed.js OPENAI               # -> OPENAI_FEED, OPENAI_FEED_HASH

cd ../../setup-cli
PRESTOCKS_MINT=… PRESTOCKS_FEED=… PRESTOCKS_FEED_HASH=… cargo run --bin list_prestocks
AMOUNT_CNGN=4000000 cargo run --bin fund_market       # only if the pool is thin

# Crank BOTH feeds and borrow in one go — 150 slots is about 60 seconds.
cd ../.devnet/sb        && CROSSBAR_URL=http://localhost:8099 node crank-ngn-feed.js
cd ../prestocks         && node crank-feed.js $OPENAI_FEED
cd ../../setup-cli      && AMOUNT_CNGN=2000000 cargo run --bin prestocks_loan
```

If the program needs rebuilding: `cargo build-sbf --tools-version v1.52 --features devnet`.
**Without `--features devnet`** you get the mainnet program id and the mainnet Switchboard program
id, and every health check fails while deposits keep working. The devnet program account was
extended to 1,200,000 bytes, so it now has room for a larger binary; it originally had none.

`prestocks_loan` uses a **dedicated borrower** (`.devnet/prestocks/borrower.json`, git-ignored)
rather than the admin wallet, because the admin's position already holds wSOL — a position holding
two assets needs every one of their price accounts fresh in the same transaction, which here would
mean Pyth SOL/USD *and* the PreStocks feed *and* NGN. Its remaining accounts are **three**, because
the asset is an `XStock`: the `CollateralAsset` PDA, the **Switchboard feed** (not a Pyth account —
`price_source` decides), then the **mint** for its multiplier.

In production the pull instructions go **ahead of `take_loan` in the same transaction**, which makes
freshness structural rather than a race against `sb_max_stale_slots`.

## Scripts

| | |
|---|---|
| `symbols.js` | the eight mints, their live multipliers, and the job builder |
| `create-mint.js SYMBOL [SUPPLY]` | devnet Token-2022 mock, multiplier preset, supply minted |
| `create-feed.js SYMBOL` | store the job on Crossbar → simulate it → create the feed |
| `crank-feed.js FEED` | land oracle updates, then print what the program will read |
| `check-collateral.js LABEL=ADDR …` | decode a `CollateralAsset` off chain, to check `price_source` |

`create-feed.js` takes `PRESTOCKS_PATH`, `PRESTOCKS_URL` and `MIN_RESPONSES` overrides. The first
two exist only for the diagnosis above; `MIN_RESPONSES` defaults to 1 and must not exceed the
number of jobs.

On the setup-cli side: `list_prestocks` (list or re-point), `fund_market` (mint cNGN and deposit it
as liquidity — devnet only, it relies on the admin holding the mock mint's authority), and
`prestocks_loan` (borrow end to end).

Crossbar must be running locally — the hosted one's `/store` returns the hash of an empty payload
and its `/simulate/jobs` is broken too (the known-good NGN job fails there identically):

```bash
docker run -d --name crossbar -p 8099:8080 \
  -e SOLANA_DEVNET_RPC=https://api.devnet.solana.com \
  -e SOLANA_MAINNET_RPC=https://api.mainnet-beta.solana.com \
  switchboardlabs/crossbar:latest
```

Two route gotchas beyond what `../sb/FINDINGS.md` records: the container serves
`GET /simulate/{feedHash}` and `GET /simulate/solana/{network}/{feed}`, but **not**
`POST /simulate/jobs` — so a job must be stored before it can be simulated, which is free and off
chain. `/store` wants `{queue, jobs}`, not the SDK's `/v2/store` shape.
