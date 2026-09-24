# PreStocks as collateral — what the chain actually says

Investigated 2026-09-24 against `https://prestocks.com/api/prestocks` (sample saved
alongside as `api-sample-2026-09-24.json`) and mainnet RPC.

## The assets

Eight Token-2022 mints, 9 decimals, one issuer authority for everything:
`WV9PJN7XTmTLVwbutCLFxp8TyePee6Xq5mRq6Fti5Wc`.

| symbol | mint | mark price | UI supply |
|---|---|---|---|
| ANDURIL | `PresTj4Yc2bAR197Er7wz4UUKSfqt6FryBEdAriBoQB` | $152.98 | 11,805.8 |
| ANTHROPIC | `Pren1FvFX6J3E4kXhJuCiAD5aDmGEb7qJRncwA8Lkhw` | $1,035.24 | 7,381.8 |
| FIGUREAI | `PreZad18qfPtbxNpMtMuAuX2zVpvkEU8DnJx56faCWd` | $180.89 | 3,012.9 |
| KALSHI | `PreLWGkkeqG1s4HEfFZSy9moCrJ7btsHuUtfcCeoRua` | $881.45 | 904.9 |
| NEURALINK | `PrekqLJvJ3qVdXmBGDiexvwUTF4rLFDa6HWS4HJbw9S` | $336.49 | 2,595.3 |
| OPENAI | `PreweJYECqtQwBtpxHL171nL2K6umo692gTm7Q3rpgF` | $1,023.01 | 2,826.3 |
| POLYMARKET | `Pre8AREmFPtoJFT8mQSXQLh56cwJmM7CFDRuoGBZiUP` | $144.38 | 4,816.9 |
| SPACEX | `PreANxuXjsy2pvisWWMNB6YaJNzr7681wJJr2rHsfTh` | $148.67 | 43,712.5 |

## They are structurally the `XStock` kind we already built

Every mint carries `ScaledUiAmount`, and the API reports **display** units, not raw ones —
which is exactly what `token/scaled_ui.rs` assumes and what `valuation.rs` passes the mint
for. Verified arithmetically on OPENAI:

```
raw supply 1,901.808687  ×  multiplier 1.4861347  =  2,826.3439  =  API `supply`
```

OPENAI's multiplier moved to 1.4861347 at `new_multiplier_effective_timestamp`
1784305800 = 2026-07-17T16:30Z, i.e. it is live. `read_xstock_multiplier` already reads
`new_multiplier` once that timestamp passes, so it gets this right; a reader that took the
`multiplier` field alone would be 48% low on every OPENAI position.

## Three things block listing them, in order

### 1. A live 1% transfer fee — and the extension allowlist correctly refuses it

Every mint carries `TransferFeeConfig` **and** `ConfidentialTransferFeeConfig`. Neither is
in `XSTOCK_COLLATERAL_EXTENSIONS` (`token/extensions.rs:31`), so `list_collateral` rejects
all eight today with `UnsupportedMintExtension`. That refusal is correct, not incidental:

```
newerTransferFee: { epoch: 1039, transferFeeBasisPoints: 100, maximumFee: u64::MAX }
olderTransferFee: { epoch: 1032, transferFeeBasisPoints:  50, maximumFee: u64::MAX }
```

Mainnet epoch is 1041, so the **100 bps fee is active**, uncapped, and the issuer can change
it at will. `deposit_collateral` credits the stated `amount` to the position slot and then
`transfer_checked`s that same `amount` — so a 1,000-token deposit credits 1,000 while the
shared vault receives 990. The vault is per-mint and shared across positions, so the
shortfall is socialised: early withdrawers are made whole out of later depositors' balances.

Supporting these for real means fee-aware accounting on every path that moves collateral —
credit the delta actually received on the way in, and gross up on the way out
(`deposit_collateral`, `withdraw_collateral`, `liquidate`, `sweep_collateral_excess`).

### 2. There is no usable on-chain price for six of the eight

Collateral pricing is Pyth-only: `valuation.rs:66` is the single call site, and
`CollateralParams::validate` requires `pyth_feed_id != [0; 32]`.

Pyth's catalogue has index feeds for three names only:

| symbol | Pyth feed |
|---|---|
| OPENAI | `Equity.Index.OPENAI/USD` `96d4bb23…c0519c483` |
| ANTHROPIC | `Equity.Index.ANTHROPIC/USD` `5da511a7…a64b6689d` |
| SPACEX | `Equity.Index.SPCX/USD` `2dbfb179…acdf17b9` |

Nothing for ANDURIL, NEURALINK, KALSHI, POLYMARKET or FIGUREAI. And even those three are
not reachable for free:

- `hermes.pyth.network/v2/updates/price/latest` returns **HTTP 401** without an API key.
- No sponsored on-chain push account exists. Derived the `pythWSnswVUd12oZpeFP8e9CVaEqJg25g1Vtc2biRsT`
  PDAs for shards 0–3 and checked devnet and mainnet: **absent**. The same derivation
  reproduces the known-good SOL/USD account `7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE`,
  so the method is right and the accounts genuinely are not published.

The one price source that covers all eight is the PreStocks API itself — plain HTTP JSON,
which is a Switchboard On-Demand job and nothing more. We already run self-hosted Crossbar
and already read `PullFeedAccountData` in `oracle/switchboard.rs` for NGN.

### 3. The mints do not exist on devnet

Devnet work needs mock mints built to the same extension shape.

## The shape of the fix

Collateral gets a price source, exactly as the market already has two oracles:

- `CollateralAsset` grows `price_source: { Pyth, SwitchboardOnDemand }` out of the 79 reserved
  bytes, so the account size does not change.
- `pyth_feed_id` becomes `feed_id` in meaning: for Switchboard it holds the feed's 32-byte
  `feed_hash`, checked against `PullFeedAccountData.feed_hash`. That is **stronger** than what
  the NGN feed does today, which pins only the account address — the feed authority can
  repoint a pinned account's job, and a `feed_hash` check catches exactly that.
- `valuation.rs:66` branches on `price_source`. One call site.

## Issuer trust, stated plainly

`WV9PJN7XTmTLVwbutCLFxp8TyePee6Xq5mRq6Fti5Wc` holds, on one key: permanent delegate,
freeze authority, pause authority, transfer-fee authority, scaled-UI multiplier authority,
transfer-hook authority (no program set today), and metadata update authority.

The permanent delegate can move collateral out of our vault at any time. The pause blocks
every transfer, including liquidation of a position that is already underwater. Spec §14
already records the delegate and the pause as accepted issuer risks for Backed's xStocks;
the difference here is that a single key holds all of them, plus an uncapped fee dial.

## The 60-second ceiling rules out sponsored equity feeds generally

Worth recording because it also applies to the Backed xStocks the program was built for.
`MAX_PRICE_AGE_SECONDS = 60` is a hard protocol ceiling. Pyth's sponsored equity accounts do
not keep up with it — checked 2026-09-24:

| feed | account | devnet | mainnet |
|---|---|---|---|
| `Equity.US.AAPL/USD` | `DJ2FyTgUAkEtXW3U5P9PF19meFTRtW4ZWKKFgACfVbUy` | $301.81, **84 days** old | $305.92, **41 days** old |
| `Equity.Index.AAPL/USD` | `7PqjmsTV5aH1YGRfqMVyvMg7N2kMASYokC7rgByy9BvM` | absent | absent |

So the sponsored route is dead for equities on both clusters, and posting our own Hermes
updates still leaves the structural problem: equities stop trading at the close, and a
60-second bound means no borrowing against them overnight or at weekends.

PreStocks marks are continuous — they are SPV valuations, not an order book — so a
Switchboard job over the PreStocks API actually fits the 60-second ceiling *better* than any
Pyth equity feed does.
