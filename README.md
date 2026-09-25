# localised-solana — HODL fixed-term loans

A native Solana program (`hodl_loans`, Anchor 1.2) for **fixed-term cNGN loans against
tokenised-equity collateral**. Borrowers pledge tokenised stocks, borrow cNGN for a fixed tenure at a
fixed rate, and build an on-chain credit history as they repay.

Live on **devnet** with ten collateral assets, two oracle sources, and a per-borrower credit record.
Not deployed to mainnet, and not externally audited.

- **Technical breakdown:** [`docs/TECHNICAL_SUMMARY.md`](docs/TECHNICAL_SUMMARY.md) — what was built, why each decision went the way it did, what was measured, what is open
- **Client contract:** [`docs/BACKEND_INTEGRATION.md`](docs/BACKEND_INTEGRATION.md) — the one to read before writing a client
- **Design spec:** [`docs/superpowers/specs/2026-09-17-solana-fixed-loans-design.md`](docs/superpowers/specs/2026-09-17-solana-fixed-loans-design.md)

---

## How it works

**Lenders** deposit cNGN into a per-market pool and hold shares against it. **Borrowers** open a
position, deposit collateral, and take fixed-term loans — principal, rate, tenure and penalty rate are
fixed at origination and the loan keeps those terms for life. A position holds up to **8** collateral
assets and **10** concurrent loans.

Every priced instruction values the whole position, not just the asset being touched, because health
decides whether an action is allowed at all. Two prices are needed and both must be fresh **in the
same transaction that uses them**:

| | source | bound |
|---|---|---|
| collateral, per asset | Pyth pull (`PriceUpdateV2`) **or** Switchboard On-Demand (`PullFeedAccountData`) | 60 s / 150 slots |
| cNGN (NGN/USD) | Switchboard On-Demand | 150 slots |

**Why two collateral sources.** Pyth publishes no on-chain feed for a private-company SPV mark, and
its sponsored equity feeds do not keep up with a 60-second ceiling — `Equity.US.AAPL/USD` was 41 days
stale on mainnet when checked. A Switchboard job over an issuer's own HTTP endpoint fits a ceiling
that exchange-hours equities structurally cannot. Each asset names its own source in
`CollateralAsset.price_source`; a client must read it rather than assume.

**Credit history.** A `CreditRecord` PDA per borrower, derived from the wallet address alone, counts
loans repaid and loans defaulted. Counters only — no score. A score is a weighting of these facts and
belongs in a versioned off-chain function, so the events carry the richer signal (`days_late`,
`penalty_paid`, `term_seconds`) for an indexer to weigh.

### Instruction surface

43 instructions. The ones that matter to a client:

```
lender      deposit_liquidity  withdraw_liquidity
position    open_position  close_position  deposit_collateral  withdraw_collateral
loans       take_loan  repay_loan
default     liquidate (permissionless)  write_off_loan (admin)
promos      redeem_promo  revoke_promo  expire_promo
admin       initialize  create_market  list_collateral  update_collateral_params
            whitelist  set_market_paused  set_collateral_paused  harvest_reserve
```

**Priced** (need oracle accounts): `take_loan`, `withdraw_collateral`, `liquidate`,
`write_off_loan`, and `revoke_promo` against a live loan.
**Unpriced** (work during an oracle outage): everything else, including `repay_loan`.

---

## Deployed addresses — devnet

### Program and authorities

| | |
|---|---|
| program | [`q33KxkuB2ntHSBwPnuFGpkiCxxEmmxsAgYAM6Gjx2SN`](https://explorer.solana.com/address/q33KxkuB2ntHSBwPnuFGpkiCxxEmmxsAgYAM6Gjx2SN?cluster=devnet) |
| upgrade authority | `7aGwNxgBd25Meyvj8Q3FYiGG5rp37bc3r7U8Ut3Tq7D6` |
| admin / guardian / whitelister / promo signer / treasury | `7aGwNxgBd25Meyvj8Q3FYiGG5rp37bc3r7U8Ut3Tq7D6` (all five — devnet only, see *Known limits*) |
| mainnet program id | `5t7smXPGCvTMXYUAJ2uU4Zk4uPggkN7KdanoywHrveMd` — **declared, never deployed** |

### Core accounts

| account | address |
|---|---|
| `Config` | `7ncUa7ay1zJpmQhDPdBiXz2N7X1brVkuxdUEDNRgQNkB` |
| `Market` (cNGN) | `9nHacBpGv2ByBUt1sTc3Mi6FFBL3usZEfSgBuGgVbCiw` |
| market vault | `9Qr5oTymnUtwe6kaRmRDKSaf6DmbqTi8EgcYdE8eoXpK` |
| `PromoVault` | `EFUex4KjG9jqw7MavERwUV2j1ScKz1qQah2pqVCWK1n` |
| promo vault token | `GUtZ4U2TuyVkqBCn8z6cqdhkaZExYfGsvV9cxCtj8T2y` |
| cNGN mint (devnet mock, 6 dec) | `GqKF9H12LqBBnrb8PoKEbgiNfcJaFSsEzTWhWYeRZecg` |

### Collateral — Pyth priced

| asset | mint | `CollateralAsset` | vault | LTV / LT |
|---|---|---|---|---|
| wSOL | `So11111111111111111111111111111111111111112` | `5Xd4kRrzsBBqnQrpJRETrWFQb9HHNZsVjp6NbGQFME7M` | `FCo4vfhV4Rft3VbgtdhkBPwWwjimVRxFLCyee2j35MiL` | 70 / 90% |
| USDC (mock) | `BSXMLmTJnAsUywuGE7sQ9mzAxVVzNjh4W2xbLBum4925` | `BLV4JNbV1fPbhhKrPVLfWUwRydjyhxYaH9PzGuLaw2bg` | `HMWzpZM4ygLXdFNFtiZJJT1Zxk2XMwYBTxXpRX4L8kRH` | 70 / 90% |

wSOL is listed **unpinned** against Pyth's sponsored SOL/USD account
`7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE` (feed id
`0xef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d`). Unpinned means the caller may
present any verified update inside the 60-second window — acceptable on devnet, documented as a value
leak for mainnet.

### Collateral — Switchboard priced (PreStocks)

All eight are **devnet Token-2022 mocks** of real mainnet PreStocks mints, 9 decimals, `XStock` kind,
LTV 50% / LT 75% / bonus 10%, deposit cap 100 raw tokens. Each has its own Switchboard On-Demand feed
over `https://prestocks.com/api/prestocks`.

| symbol | devnet mint | `CollateralAsset` | Switchboard feed | mult. |
|---|---|---|---|---|
| OPENAI | `ECbSTTymP6eUkNy4Q8VF4DNJNULXxKz63iUk7vnen2xR` | `JBoLjsAw3X1zt7VCGYS5FZQ8A5kKmoVdfVv1mMAnao1g` | `DTXX9vQdGojn88EZCNKhjHNJ4yPn2T8TiFWsyRCwUKHc` | 1.4861347 |
| ANTHROPIC | `5ddG2fZ99xUtMWisqKWfXS1LZRfa6wF4R6jXae2cUvZc` | `HV6khokD4DBDjUff8iHZC43xur9cPe7Vhe3Scwitbm5F` | `FeTy2t3yZwmX4cqxWPZF21qSmL7aRCkMs2biFo5kPB26` | 1 |
| SPACEX | `7b5C13nRLTuiecGGdHb9KX63n2gUqU9LZnkzHV9W6K3z` | `5BjdPTGpteBX5AVzUPe4yi4yqTmQCWXYK6gXw2KjatJC` | `8G4Np6z9QShuZmod4iZ3cuAfSmieRFpe3Gnec3vuhwew` | **5** |
| ANDURIL | `HLH1vb29GYVjgYpGgRau5firRE7aUMqNb3anLnyaPPmv` | `9WDzZcE26YSf6gE6CTABp2UgGhfuMiJktCwmvgyMQVgc` | `97GsTWhXyBkeYPqT1biqZRtrKBzb2y2QBjqdMF4oRevY` | 1 |
| NEURALINK | `CSKqv3CtDwDz3nHaqQEoEPBqFSgsx22q2ekNmksMgY7X` | `CnBGPBc75VuZfhFcKoLFjdWwrnmCAjVXoadrFrktpfyo` | `95TDbrjnwbsh9yUqKHcsGkX9tFg5kqWVu9s9vKB26zrS` | 1 |
| KALSHI | `EeQ9MBtXdRFWCyJhAsxnKXjfzo66ntg39GHxhps6vmHX` | `AKwyU81E3uEckAEVuBiKWS7gcP7wd7gx4riKzAZMkGh4` | `BybBY2H4NJQrPpE48BP1zbyTct32yuh3VW4pgaYh51MH` | 1 |
| POLYMARKET | `7Z56jHJHEf82MCapqsdJS6VyosX33Hqrw44tHasE9jH3` | `35NXZtZUSWeyFnK9DKmPhKx1L3T4q76TgqZgX9k8BMkj` | `E165VXk3FXZef3BvqYUEJkPb8dXwn9LfxQfbC5EhYKpC` | 1 |
| FIGUREAI | `E8EPGxCjX3habe7mre698SPPKcuv28DusgDXJWfVYvRk` | `FMUFj5PKFEJq6WucXEtz4stKx7HXZG8GYdFkkpMNMDxU` | `6gspjnbkx3PX4Npt7StuQNnFdqqWqagunXd6XhqnaVr3` | 1 |

Collateral vaults, in the same order: `4tfak6bR…`, `2HCXKTZF…`, `9VBeqvWz…`, `7iyrSAcR…`,
`8j6ei2pJ…`, `FMZXST5s…`, `DGasgECj…`, `BVTrLV9w…` — full values in
[`.devnet/addresses.env`](.devnet/addresses.env).

The multiplier is the mint's live scaled-UI factor, read from the **mainnet** mint at mock-creation
time. The API quotes price per *display* token, so the multiplier and the price travel together: 2 raw
SPACEX is 10 display shares.

### Oracles and infrastructure

| | |
|---|---|
| NGN/USD feed (Switchboard) | `GDgs76wotM4mxXSYPmizeKtdXoHqWUxqHAASNSLnrBQ1` |
| Switchboard On-Demand program | `Aio4gaXjXzJNVLtzwtNVmSqGKpANtXhybbkhtAC94ji2` (devnet) |
| Switchboard queue | `EYiAmGSdsQTuCw413V5BzaruWuCCSDgTPtBGvLkXHbe7` |
| Pyth receiver | `rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ` |
| Crossbar (self-hosted, Railway) | `https://crossbar-staging.up.railway.app` |

Crossbar is self-hosted because the hosted `crossbar.switchboard.xyz` returns the hash of an **empty
payload** from `/store` — a feed created against it can never be resolved and every crank fails with
`ORACLE_UNAVAILABLE`. Deployment in [`deploy/crossbar/`](deploy/crossbar/).

### Proven end to end

| | |
|---|---|
| borrow against a Switchboard-priced stock | 2,001,000 cNGN against 2 OPENAI shares at $1,023.62 |
| credit record | [`56vX6uddYWcaeHyN4EeyhRt58AnoRJhCMmpazrKtAGhc`](https://explorer.solana.com/address/56vX6uddYWcaeHyN4EeyhRt58AnoRJhCMmpazrKtAGhc?cluster=devnet) — `loans_completed: 2` |

The price was shown to *bind*, not merely to be read: 2,500,000 cNGN refused `Unhealthy`, 2,000,000
accepted, against a hand-computed limit of ~2,018,575.

---

## Build and test

```bash
./scripts/test.sh              # builds the .so, then 311 unit + LiteSVM integration tests
./scripts/test.sh --lib        # unit tests only (still builds first)
cargo clippy -p hodl_loans --all-targets -- -D warnings
```

**Use `scripts/test.sh`, not bare `cargo test`.** It rebuilds the program first; without that, the
default build runs against whatever `.so` is on disk and fails with `DeclaredProgramIdMismatch`
(4100), which is an opaque way to discover a stale artifact.

The `devnet` feature selects the program id *and* the Switchboard program id at compile time:

```bash
cargo build-sbf --tools-version v1.52 --features devnet   # devnet
cargo build-sbf --tools-version v1.52                     # mainnet
```

Building without `--features devnet` and deploying to devnet produces a program whose deposits work
and whose every health check fails. The hashes differ; check them.

## Client tooling

`setup-cli/src/bin/` — Rust, but the account layouts are language-agnostic:

| bin | shows |
|---|---|
| `take_loan.rs` | the Pyth / `Standard` collateral shape |
| `prestocks_loan.rs` | the Switchboard / `XStock` shape, multi-asset, ordered by the position's own slots |
| `repay.rs` | `repay_loan` with `credit_record` and a `mut` payer |
| `credit_record.rs` | reading a borrower's record by wallet address alone |
| `list_prestocks.rs` | listing a Switchboard-priced asset |
| `fund_market.rs` | minting mock cNGN and depositing it as liquidity |

Devnet scripts in [`.devnet/prestocks/`](.devnet/prestocks/) create mock mints, create and crank
feeds, and decode accounts. See its [README](.devnet/prestocks/README.md).

---

## Known limits

Stated because they bound what the program can currently do, not as future work.

**A borrower can use one stock at a time.** Cranking four feeds in separate transactions takes ~196
slots against a 150-slot freshness bound, so the first is stale before the last lands. Two feeds — one
stock plus NGN — is the ceiling. Fixing it means putting the pull instructions in `take_loan`'s own
transaction; the account layout is already proven, the bundled sender is not built.

**The real PreStocks mints cannot be listed, and the refusal is correct.** All eight carry
`TransferFeeConfig` at a live, uncapped **100 bps**. `deposit_collateral` credits the amount it asks to
transfer, so a fee credits a position more than the shared vault received and the shortfall is
socialised. Supporting them needs fee-aware accounting on all four collateral-moving paths. The devnet
mocks omit the fee, which is the one way they do not mirror the real asset — so devnet cannot be used
to argue the real mints are safe to list.

**Issuer trust.** One key holds permanent-delegate, freeze, pause, transfer-fee and scaled-UI
authority across all eight PreStocks mints. The permanent delegate can move collateral out of a vault;
a pause blocks every transfer including liquidation of an already-underwater position.

**Devnet runs all five protocol roles on one wallet.** `initialize` takes them separately, so
separating them costs only coordination — see `docs/superpowers/runbooks/`.

**Not externally audited**, and mainnet is undeployed. 311 tests, every `HodlError` variant asserted,
and a clean whole-branch review are real evidence, but they are not an audit.
