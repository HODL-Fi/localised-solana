# HODL fixed loans on devnet

Everything below is live and verified by reading state back, not by trusting signatures.
Addresses are in `addresses.env`; the IDL is at `../idl/hodl_loans-devnet.json`.

| | |
|---|---|
| program | `q33KxkuB2ntHSBwPnuFGpkiCxxEmmxsAgYAM6Gjx2SN` |
| config | `7ncUa7ay1zJpmQhDPdBiXz2N7X1brVkuxdUEDNRgQNkB` |
| market (cNGN) | `9nHacBpGv2ByBUt1sTc3Mi6FFBL3usZEfSgBuGgVbCiw` |
| promo vault | `EFUex4KjG9jqw7MavERwUV2j1ScKz1qQah2pqVCWK1n` |
| cNGN mint | `GqKF9H12LqBBnrb8PoKEbgiNfcJaFSsEzTWhWYeRZecg` (Token-2022, 6dp) |
| wSOL collateral | `5Xd4kRrzsBBqnQrpJRETrWFQb9HHNZsVjp6NbGQFME7M` (Pyth SOL/USD, **unpinned**) |
| NGN/USD feed | `GDgs76wotM4mxXSYPmizeKtdXoHqWUxqHAASNSLnrBQ1` (Switchboard On-Demand) |
| admin / upgrade authority | `7aGwNxgBd25Meyvj8Q3FYiGG5rp37bc3r7U8Ut3Tq7D6` |

**Back up `~/.config/solana/hodl_loans-devnet.json`.** It is the only key that can upgrade
this deployment.

## Proven working

`take_loan` completed on devnet reading both live oracles — 5,000 cNGN against 0.5 wSOL,
Pyth SOL/USD at $114.74 and Switchboard NGN/USD at 0.000753652729058755:

```
sig 5xPNrfPfRQEvzo3nYHAUiSE3vJ8sg4BcFfk5T6rbxuGohXxoHP1oCPvfTL2Fq6Dz6nEoi6zHpy7nf49xT4jHVVpL
active loans 0 -> 1      cNGN balance +5,000,000,000 units
```

## The transaction shape your client needs

`take_loan`'s fixed accounts are in `setup-cli/src/bin/take_loan.rs`. What is easy to miss:

- **Remaining accounts are `(CollateralAsset, PriceUpdateV2)` pairs**, one per collateral slot
  holding a non-zero amount. An `XStock` asset appends its mint as a third account (the
  scaled-UI multiplier lives there); a `Standard` asset does not.
- **Both oracles must be fresh in the same transaction.** Pyth: younger than
  `max_price_age_seconds` (60). Switchboard: `result.slot` within `ngn_max_stale_slots`
  (150, ~60s). Put the Pyth post and the Switchboard pull instructions **ahead of `take_loan`
  in one transaction** — that makes freshness structural instead of a race between
  transactions.
- The collateral is listed **unpinned** (`price_account = Pubkey::default()`), so the program
  accepts any receiver-owned account with the right feed id inside the age window. That is
  what lets a client post its own ephemeral update. The documented cost, in
  `state/collateral.rs:26`, is that a caller may choose the most favourable update in the
  window — pin to a maintained feed on mainnet.

## Running it

```bash
# Crossbar must be local: the hosted /v2/store returns a hash of an empty payload,
# so oracles can never resolve the job. See sb/FINDINGS.md.
docker run -d --name crossbar -p 8099:8080 \
  -e SOLANA_DEVNET_RPC=https://api.devnet.solana.com \
  -e SOLANA_MAINNET_RPC=https://api.mainnet-beta.solana.com \
  switchboardlabs/crossbar:latest

cd setup-cli
set -a; . ../.devnet/addresses.env; set +a
cargo run --bin setup-cli        # initialize, market, promo vault, collateral, liquidity
cargo run --bin borrower_setup   # wrap wSOL, open position, deposit collateral
../.devnet/sb/borrow-now.sh      # wait for a Pyth window, crank NGN, borrow
```

## Two devnet-only caveats

**Pyth Hermes now requires an API key** (`hermes.pyth.network` returns 401), so
`refresh-oracles.js` — which posts its own update, the production shape — cannot run here.
`borrow-now.sh` works around it by waiting for Pyth's sponsored devnet account to refresh.
That account updates in bursts every few minutes, leaving a ~60s usable window. Get a Hermes
key and the workaround disappears.

**The NGN feed needs cranking before each priced call.** It is a pull oracle: nothing updates
it on a schedule. `crank-ngn-feed.js` does it standalone; production bundles the same
instructions into the borrowing transaction.
