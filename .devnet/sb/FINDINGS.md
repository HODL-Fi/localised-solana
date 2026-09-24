# Switchboard NGN/USD feed on devnet — state and blocker

## Answer to "does one already exist": no

Scanned **all 5,004** `PullFeedAccountData` accounts owned by the devnet Switchboard
On-Demand program (`Aio4gaXjXzJNVLtzwtNVmSqGKpANtXhybbkhtAC94ji2`), matched by Anchor
discriminator, and read each one's `name` field. **Zero NGN feeds.** The scan is sound —
names came back real and varied (`DRIFT/USDC`, `BTC Price Feed`, meme markets).

## What was built and works

- **Feed account created on devnet:** `C6QkvT3N7EZd5a1Uk2zKWgdqpc4dhkCsgQeiVmtdGQem`
  - owner `Aio4gaXjXzJNVLtzwtNVmSqGKpANtXhybbkhtAC94ji2` — what `read_ngn_price` requires
  - 3,208 bytes = `8 + sizeof(PullFeedAccountData)` — the discriminator/length check passes
  - on-chain `feed_hash` = `6e340b9cffb37a989ca544e6bb780a2c78901d3fb33738768511a30617afa01d`
  - name `NGN/USD`, queue `EYiAmGSdsQTuCw413V5BzaruWuCCSDgTPtBGvLkXHbe7`, min_responses 2
- **The jobs are correct.** Crossbar's own `/simulate/jobs` executes them and returns
  `0.0007536527` and `0.0007539746` USD per NGN — 0.04% apart, well inside the 1% variance
  bound. Two independent free sources, no API key:
  - `https://open.er-api.com/v6/latest/USD` → `$.rates.NGN`
  - `https://cdn.jsdelivr.net/npm/@fawazahmed0/currency-api@latest/v1/currencies/usd.json` → `$.usd.ngn`
  - Each inverted via `valueTask(1) / fetched`, because the program wants USD per NGN
    (`scale_switchboard_value` reads the price of one NGN in dollars), not NGN per USD.

## The blocker: Crossbar's public `/v2/store` is not storing job definitions

Cranking fails with `ORACLE_UNAVAILABLE: No oracle responses received` from every gateway.
Root cause: oracles resolve a `feed_hash` to its job definition through Crossbar, and the
stored record is empty.

`fetchOracleFeed(hash)` returns `{"data":"AA==","feed":{},"size":1}` — a single zero byte.

Established it is service-side, not a payload problem, by varying everything:

| attempt | request shape | encoding | returned feedId | stored size |
|---|---|---|---|---|
| 1 | `createFeedRequestV1` | `encodeDelimited`, 2 jobs | `0x6e340b9cffb3…` | 1 |
| 2 | `createFeedRequestV1` | `encode`, 1 job | `0x6e340b9cffb3…` | 1 |
| 3 | `createFeedRequestV1` | `encodeDelimited`, 1 job | `0x6e340b9cffb3…` | 1 |
| 4 | `createFeedRequestV2` | `OracleFeed.encode`, 222 real proto bytes | `0x6e340b9cffb3…` | 1 |

**The same feed id comes back for four different payloads**, including a valid 222-byte
`OracleFeed` proto. It is the hash of an empty payload. The endpoint accepts the POST,
returns 200, and stores nothing.

The devnet oracle network itself is alive: `1LgYinFAUveKLWbxwMKceENKgmrVV8oEQSHZU2A1q5V`
(`DRIFT/USDC`) cranks fine and returns `0.018836`. Several pre-existing `BTC Price Feed`
accounts fail identically to ours, which is consistent with the same store problem having
affected whoever created them.

## Ways forward

1. **Self-host Crossbar.** Switchboard ships it as a container; a local instance does the
   store itself, and `CrossbarClient` takes a custom URL. Most likely to just work.
2. **Create the feed through Switchboard's own Explorer UI**, which stores server-side, then
   point `market.ngn_feed` at the resulting account. Costs nothing to try.
3. **Point `ngn_feed` at an existing working feed** (e.g. the DRIFT/USDC one) to exercise
   every priced path end-to-end. NGN prices are then wrong, so loan sizing is wrong — fine
   for testing client plumbing, useless for economics. One `update_market_params` to set,
   one to undo.
4. **Leave the placeholder** and test only unpriced paths (liquidity, positions, collateral
   deposits, admin).

Collateral pricing is NOT blocked by any of this — wSOL is listed against the real Pyth
SOL/USD feed id, unpinned, so a client that posts a Hermes update alongside its instruction
gets a real price today.

## Scripts

- `create-ngn-feed.js` — stores jobs on Crossbar, creates the on-chain feed. Works; the
  feed it produces cannot crank until the store problem is resolved.
- `crank-ngn-feed.js` — fetches oracle signatures and lands them. This is also the shape a
  client uses in production: bundle the update instruction ahead of your own in the SAME
  transaction so the price is fresh by construction.

Two gotchas worth keeping, both cost real time here:

- `connection.confirmTransaction()` resolving does **not** mean the transaction succeeded. An
  earlier run reported a feed address for a transaction that had failed with
  `Computational budget exceeded`. Always check `conf.value.err`.
- `PullFeed.initTx` calls its transaction builder with neither `payer` nor `signers`, so it
  always throws `Payer not provided`. Build the instruction and the transaction separately.
- `asV0TxWithComputeIxs` sets the compute limit to exactly what its simulation consumed,
  which under-measures `PullFeedInit` (it creates an ATA and a lookup table). Pass
  `computeUnitLimitMultiple: 1.6`.
