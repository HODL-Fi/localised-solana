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

| | |
|---|---|
| mock OPENAI mint | `ECbSTTymP6eUkNy4Q8VF4DNJNULXxKz63iUk7vnen2xR` ✅ created |
| Switchboard feed | `J9SkmK6UVDie4K6PgFF6kQJjt7j6NjyNLQs6B2JMX8EM` ✅ created, ⛔ not yet answered by oracles |
| feed job | resolves on Crossbar at **$1,023.62** per display token ✅ |
| program redeploy | ⛔ blocked on devnet SOL |
| listing / deposit / take_loan | ⛔ waits on the redeploy |

The mint carries `metadataPointer, scaledUiAmountConfig, permanentDelegate, transferHook,
defaultAccountState, pausableConfig, tokenMetadata` — every one inside
`XSTOCK_COLLATERAL_EXTENSIONS` — with the multiplier set to the live mainnet value of
**1.4861347**, deliberately not 1, so the scaled-UI path is actually exercised.

It does **not** carry the mainnet mints' 100 bps `TransferFeeConfig`. That is the one way these
mocks do not mirror the real asset, and it means devnet cannot be used to argue the mainnet mints
are safe to list. `deposit_collateral` credits the amount it asks to transfer, so a fee would
credit a position 1% more than the shared vault received.

## Blocker 1 — devnet SOL for the redeploy

The devnet program was deployed with `--max-len` at the exact binary size, so it has **no upgrade
headroom**, and the new binary is 5,920 bytes larger (1,154,296 vs 1,148,376).

```
balance now   3.51 SOL   (7aGwNxgBd25Meyvj8Q3FYiGG5rp37bc3r7U8Ut3Tq7D6)
extend        0.26 SOL   permanent, for 51,624 extra bytes
deploy buffer 5.86 SOL   held during the upgrade, refunded when it completes
needed        ~6.2 SOL at peak  →  about 2.7 SOL short
```

`solana airdrop` is rate-limited. https://faucet.solana.com gives 5 SOL a day against a GitHub
login, which clears it.

Then:

```bash
cd ~/work/hodl/lendbit-solana
cargo build-sbf --tools-version v1.52 --features devnet     # 1,154,296 bytes

# Extend first — the upgrade fails outright if the account cannot hold the new binary.
solana program extend q33KxkuB2ntHSBwPnuFGpkiCxxEmmxsAgYAM6Gjx2SN 51624 --url devnet

solana program deploy target/deploy/hodl_loans.so \
  --program-id ~/.config/solana/hodl_loans-devnet.json --url devnet
solana program dump q33KxkuB2ntHSBwPnuFGpkiCxxEmmxsAgYAM6Gjx2SN /tmp/onchain.so --url devnet
shasum -a 256 /tmp/onchain.so target/deploy/hodl_loans.so    # must match
```

**Build with `--features devnet`.** Without it you get the mainnet program id and the mainnet
Switchboard program id, and every health check fails while deposits keep working.

## Blocker 2 — the oracle network has not picked up the new feed

Crossbar resolves the feed and simulates it correctly:

```bash
curl -s http://localhost:8099/simulate/solana/devnet/J9SkmK6UVDie4K6PgFF6kQJjt7j6NjyNLQs6B2JMX8EM
# [{"results":["1023.6193398956366"], ...}]
```

But asking for signed updates returns nothing, and through the hosted Crossbar the gateways say
`ORACLE_UNAVAILABLE: No oracle responses received`.

This is **not** the JSONPath and **not** prestocks.com. Established by three feeds:

| feed | job | oracles answer? |
|---|---|---|
| `J9SkmK6UVDie…` | prestocks + `$[?(@.symbol=='OPENAI')].markPrice` | no |
| `4TMd3L3WfGDL…` | prestocks + `$[5].markPrice` (simplest possible path) | no |
| `B5tPYx7Jmpkn…` | **open.er-api.com** + `$.rates.NGN` — a URL the working NGN feed already proves reachable | **no** |
| `GDgs76wotM4m…` | the NGN feed, created a day earlier | **yes**, right now |

The control failing is what settles it: the only property the three failing feeds share and the
working one does not is *being new*. The oracle network answers a day-old feed and not a
minutes-old one, so this reads as an indexing delay on their side. Retry with:

```bash
cd .devnet/prestocks && node crank-feed.js J9SkmK6UVDie4K6PgFF6kQJjt7j6NjyNLQs6B2JMX8EM
```

If it is still silent after a few hours, create a fresh feed and try again before assuming
anything about the job — a feed account is cheap and the diagnosis above cost three of them.

## Then, in order

```bash
set -a; . ../addresses.env; set +a

# 1. list the mock as Switchboard-priced XStock collateral (LTV 50 / LT 75 / bonus 10,
#    deposit cap 100 display tokens)
cd ../../setup-cli
PRESTOCKS_MINT=$OPENAI_MINT PRESTOCKS_FEED=$OPENAI_FEED \
PRESTOCKS_FEED_HASH=$OPENAI_FEED_HASH cargo run --quiet --bin list_prestocks

# 2. crank the feed, then borrow INSIDE the freshness window
cd ../.devnet/prestocks && node crank-feed.js $OPENAI_FEED
```

For `take_loan`, the remaining accounts for this asset are **three**, because it is an `XStock`:
`CollateralAsset` PDA, the **Switchboard feed** (not a Pyth account), then the **mint** for its
multiplier. `setup-cli/src/bin/take_loan.rs` currently appends a Pyth pair for wSOL and needs the
third account plus the feed substituted for a PreStocks position.

In production the pull instructions go **ahead of `take_loan` in the same transaction**, which
makes freshness structural rather than a race against `sb_max_stale_slots` (150 slots, ~60s).

## Scripts

| | |
|---|---|
| `symbols.js` | the eight mints, their live multipliers, and the job builder |
| `create-mint.js SYMBOL [SUPPLY]` | devnet Token-2022 mock, multiplier preset, supply minted |
| `create-feed.js SYMBOL` | store the job on Crossbar → simulate it → create the feed |
| `crank-feed.js FEED` | land oracle updates, then print what the program will read |

`create-feed.js` takes `PRESTOCKS_PATH` and `PRESTOCKS_URL` overrides; both exist only for the
diagnosis above.

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
