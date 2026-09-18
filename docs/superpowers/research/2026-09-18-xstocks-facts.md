# xStocks (Backed Finance) on Solana — Fact-Finding for Lending-Protocol Collateral Design

Research date: 2026-09-18. RPC endpoint used unless noted: `https://api.mainnet-beta.solana.com`.
All on-chain calls below were re-run during this session and reflect live mainnet state at the
slots shown; extension *values* (especially the multiplier) will drift over time — treat the
raw JSON as a point-in-time snapshot, not a permanent fact.

---

## 1. Real mint addresses (VERIFIED on-chain)

Found via web search (CoinGecko, Solflare, Solscan explorer pages, Solana Compass) and then
**confirmed live** by querying `getAccountInfo` for each address and checking `program:
"spl-token-2022"` plus the `tokenMetadata` extension's `symbol`/`name` fields, which round-trip
back to the expected ticker.

| Ticker | Mint address | Source(s) |
|---|---|---|
| AAPLX (Apple xStock) | `XsbEhLAtcf6HdfpFZ5xEMdqW8nfAvcsP5bdudRLJzJp` | [Solana Compass token page](https://solanacompass.com/tokens/XsbEhLAtcf6HdfpFZ5xEMdqW8nfAvcsP5bdudRLJzJp), [Solflare](https://www.solflare.com/stocks/apple-xstock/XsbEhLAtcf6HdfpFZ5xEMdqW8nfAvcsP5bdudRLJzJp/), [CoinGecko](https://www.coingecko.com/en/coins/apple-xstock) |
| TSLAX (Tesla xStock) | `XsDoVfqeBukxuZHWhdvWHBhgEHjGNst4MLodqsJHzoB` | [Orbmarkets token page](https://orbmarkets.io/token/XsDoVfqeBukxuZHWhdvWHBhgEHjGNst4MLodqsJHzoB), corroborated by search results referencing Backed/xStocks TSLAx |
| NVDAX (NVIDIA xStock) | `Xsc9qvGR1efVDFGLrVsmkzv3qi45LTBjeUKSPmx9qEh` | [Solflare](https://www.solflare.com/prices/nvidia-xstock/Xsc9qvGR1efVDFGLrVsmkzv3qi45LTBjeUKSPmx9qEh/), [Solana Explorer](https://explorer.solana.com/address/Xsc9qvGR1efVDFGLrVsmkzv3qi45LTBjeUKSPmx9qEh), [createmycoin.app](https://createmycoin.app/tokens/solana/Xsc9qvGR1efVDFGLrVsmkzv3qi45LTBjeUKSPmx9qEh) |

All three mints are owned by the Token-2022 program `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb`
(confirmed in `value.owner` of every `getAccountInfo` response), which is on-chain proof they are
Token-2022, not legacy SPL Token, mints.

I did not independently re-derive these from a canonical Backed-published address list (I could
not find a machine-readable "official" mint-address manifest from backed.fi/xstocks.fi during
this session — see Open Questions). Confidence these three are correct: **High** — each address
decodes as an active Token-2022 mint whose on-chain `tokenMetadata.symbol`/`.name` and
`metadataPointer.metadataAddress` (self-referential, i.e. `metadataAddress == mint`) match the
expected ticker exactly, and each is corroborated by 2+ independent web sources.

---

## 2. Extension list — fetched live via `getAccountInfo` (`encoding: jsonParsed`)

### Summary table (all three mints)

| Extension | AAPLX | TSLAX | NVDAX |
|---|---|---|---|
| `metadataPointer` | present, self-pointing | present, self-pointing | present, self-pointing |
| `permanentDelegate` | delegate = `5aMNNLQJwAEeoemTEMkv5NVjqKwvvefRYCQ5Z67HFvEq` | same delegate | same delegate |
| `defaultAccountState` | **present** — `accountState: "initialized"` | **present** — `"initialized"` | **present** — `"initialized"` |
| `scaledUiAmountConfig` | authority `S7vYFFWH6BjJyEsdrPQpqpYTqLTrPRK6KW3VwsJuRaS`; multiplier `1.0026642075893797`; newMultiplier `1.0032690125398187`; effective ts `1786149000` (2026-08-08T00:30:00Z) | authority same; multiplier `"1"`; newMultiplier `"1"`; effective ts `0` | authority same; multiplier `1.0009180758490996`; newMultiplier `1.001701196801074`; effective ts `1789000200` (2026-09-10T00:30:00Z) |
| `pausableConfig` | authority `JDq14BWvqCRFNu1krb12bcRpbGtJZ1FLEakMw6FdxJNs`; `paused: false` | same authority; `paused: false` | same authority; `paused: false` |
| `confidentialTransferMint` | authority `5aMNN...FvEq`; `autoApproveNewAccounts: false`; `auditorElgamalPubkey: null` | same | same |
| `transferHook` | authority `5aMNN...FvEq`; **`programId: null`** | same, `programId: null` | same, `programId: null` |
| `tokenMetadata` | name "Apple xStock", symbol "AAPLx" | name "Tesla xStock", symbol "TSLAx" | name "NVIDIA xStock", symbol "NVDAx" |
| `decimals` | 8 | 8 | 8 |
| `freezeAuthority` | `JDq14BWvqCRFNu1krb12bcRpbGtJZ1FLEakMw6FdxJNs` | same | same |
| `mintAuthority` | `7pt9tkctJPK7PPNQJ77GKg8ZffSF6QxoMiCFYHxrtaCj` | same | same |
| Anything else present? | No. | No. | No. |
| `TransferFeeConfig`, `NonTransferable`, `InterestBearingConfig`? | Absent (not in the extension list) — confirmed by their absence from the full `extensions` array above. | Absent. | Absent. |

**Flag vs. the assumed extension set in the task brief:** the brief assumed *"PermanentDelegate,
Pausable, ScaledUiAmount, MetadataPointer, TokenMetadata, ConfidentialTransferMint, and
TransferHook (no hook program) — and nothing else (no ... `DefaultAccountState` ...)"*. On-chain
truth for all three mints checked: **`DefaultAccountState` IS present**, contradicting that
assumption. It is currently configured `accountState: "initialized"` (new token accounts are NOT
frozen by default today), so it has no immediate operational effect — but its presence means the
freeze authority holds a live, callable power to flip new-account default state in the future.
xStocks' own developer docs (see §5) confirm this is intentional: the extension is present
specifically to allow Backed to switch on blocklist-style compliance tooling (sRFC-37) later
without a new deployment.

### Raw RPC JSON — AAPLX (`XsbEhLAtcf6HdfpFZ5xEMdqW8nfAvcsP5bdudRLJzJp`), slot 448030997

```json
{
  "jsonrpc": "2.0",
  "result": {
    "context": { "apiVersion": "4.3.0-rc.0", "slot": 448030997 },
    "value": {
      "data": {
        "parsed": {
          "info": {
            "decimals": 8,
            "extensions": [
              {
                "extension": "metadataPointer",
                "state": {
                  "authority": "5aMNNLQJwAEeoemTEMkv5NVjqKwvvefRYCQ5Z67HFvEq",
                  "metadataAddress": "XsbEhLAtcf6HdfpFZ5xEMdqW8nfAvcsP5bdudRLJzJp"
                }
              },
              {
                "extension": "permanentDelegate",
                "state": { "delegate": "5aMNNLQJwAEeoemTEMkv5NVjqKwvvefRYCQ5Z67HFvEq" }
              },
              {
                "extension": "defaultAccountState",
                "state": { "accountState": "initialized" }
              },
              {
                "extension": "scaledUiAmountConfig",
                "state": {
                  "authority": "S7vYFFWH6BjJyEsdrPQpqpYTqLTrPRK6KW3VwsJuRaS",
                  "multiplier": "1.0026642075893797",
                  "newMultiplier": "1.0032690125398187",
                  "newMultiplierEffectiveTimestamp": 1786149000
                }
              },
              {
                "extension": "pausableConfig",
                "state": {
                  "authority": "JDq14BWvqCRFNu1krb12bcRpbGtJZ1FLEakMw6FdxJNs",
                  "paused": false
                }
              },
              {
                "extension": "confidentialTransferMint",
                "state": {
                  "auditorElgamalPubkey": null,
                  "authority": "5aMNNLQJwAEeoemTEMkv5NVjqKwvvefRYCQ5Z67HFvEq",
                  "autoApproveNewAccounts": false
                }
              },
              {
                "extension": "transferHook",
                "state": {
                  "authority": "5aMNNLQJwAEeoemTEMkv5NVjqKwvvefRYCQ5Z67HFvEq",
                  "programId": null
                }
              },
              {
                "extension": "tokenMetadata",
                "state": {
                  "additionalMetadata": [],
                  "mint": "XsbEhLAtcf6HdfpFZ5xEMdqW8nfAvcsP5bdudRLJzJp",
                  "name": "Apple xStock",
                  "symbol": "AAPLx",
                  "updateAuthority": "5aMNNLQJwAEeoemTEMkv5NVjqKwvvefRYCQ5Z67HFvEq",
                  "uri": "https://xstocks-metadata.backed.fi/tokens/Solana/AAPLx/metadata.json"
                }
              }
            ],
            "freezeAuthority": "JDq14BWvqCRFNu1krb12bcRpbGtJZ1FLEakMw6FdxJNs",
            "isInitialized": true,
            "mintAuthority": "7pt9tkctJPK7PPNQJ77GKg8ZffSF6QxoMiCFYHxrtaCj",
            "supply": "15376339093106"
          },
          "type": "mint"
        },
        "program": "spl-token-2022",
        "space": 678
      },
      "executable": false,
      "lamports": 647086478,
      "owner": "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
      "rentEpoch": 18446744073709551615,
      "space": 678
    }
  },
  "id": 1
}
```

Command used:
```
curl -s https://api.mainnet-beta.solana.com -X POST -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getAccountInfo","params":["XsbEhLAtcf6HdfpFZ5xEMdqW8nfAvcsP5bdudRLJzJp",{"encoding":"jsonParsed"}]}'
```

TSLAX and NVDAX were queried identically (slots 448031078 and 448031082 respectively); their
full JSON is summarized in the table above and omitted here for length — every field not called
out as different in the table is byte-identical in shape to the AAPLX response.

---

## 3. Pyth coverage and quoting convention

### Feed IDs found (via Pyth Hermes `/v2/price_feeds?query=...`, confirmed live)

| Symbol | Pyth feed id (hex) | asset_type | Notes |
|---|---|---|---|
| `Crypto.AAPLX/USD` | `978e6cc68a119ce066aa830017318563a9ed04ec3a0a6439010fc11296a58675` | Crypto | 24/7, `min_channel: fixed_rate@200ms` |
| `Crypto.AAPLX/AAPL.RR` | `25babb83691a056fd65f879bfd7197eabd840aae741f69c87ccb31e204a979b2` | Crypto Redemption Rate | AAPLX priced *in AAPL* — tracks token/underlying divergence |
| `Equity.US.AAPL/USD` | `49f6b65cb1de6b10eaf75e7c03ca029c306d0357e91b5311b175084a5ad55688` | Equity | NYSE/Nasdaq hours only (closed on weekends) |
| `Equity.Index.AAPL/USD` | `aaba35e6f33fb973bb2201d48a79ae24795affa6ba8bd50a93dcaf7da0030f36` | Equity | "PYTH PRICE IN USD FOR AAPL 24/7" — synthetic 24/7 index version of the equity feed |
| `Crypto.TSLAX/USD` | `47a156470288850a440df3a6ce85a55917b813a19bb5b31128a33a986566a362` | Crypto | |
| `Crypto.TSLAX/TSLA.RR` | `997362625415627e9e3177f6c0d32f200d4a221ccadb3dddab80d6079d03ea24` | Crypto Redemption Rate | |
| `Equity.US.TSLA/USD` | `16dad506d7db8da01c87581c87ca897a012a153557d4d578c3b9c9e1bc0632f1` | Equity | |
| `Crypto.NVDAX/USD` | `4244d07890e4610f46bbde67de8f43a4bf8b569eebe904f136b469f148503b7f` | Crypto | |
| `Crypto.NVDAX/NVDA.RR` | `b675c4e9f46d94afa9174a7df09966b77a2950970bb50a77ec8ad4fcfd8266f4` | Crypto Redemption Rate | |
| `Equity.US.NVDA/USD` | `b1073854ed24cbc755dc527418f52b7d271f6cc967bbf8d8129112b18860a593` | Equity | |

Pyth also lists a separate "Ondo tokenized stock" competitor series (`Crypto.AAPLON/USD` etc.) —
not relevant here but worth knowing these ticker collisions exist if a design ever greps for
"AAPL" broadly.

**So yes — Pyth publishes dedicated feeds for the xStock tokens themselves**, distinct from the
underlying-equity feed, and additionally publishes a **redemption-rate feed** whose entire
purpose is to measure how far the xStock's market price has drifted from the share it represents.
The existence of that redemption-rate feed is itself evidence that Backed/Pyth treat "price of
the xStock" and "price of the underlying share" as two different numbers that can diverge (premium/
discount), not the same number republished twice.

### Sponsored (continuously-updated, `write_authority`-is-Pyth's) on-chain accounts: **not established for the xStocks-specific feeds**

I confirmed the general mechanism: Pyth's push/pull architecture on Solana mainnet is served by
the receiver program `rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ`, and Pyth's docs state price
feeds are "currently sponsored in Solana mainnet and devnet" for *some* subset of feeds (with
55s/30s/3min heartbeat + 0.5%/0.05% deviation tiers), pointed to by a page that renders a table
client-side (I could not extract that table from static HTML — it's Next.js client-rendered and
not present in the raw page source I fetched).

I independently confirmed the **general concept** is real by finding and verifying on-chain a
classic sponsored/legacy price account for `Crypto.SOL/USD`:
`H6ARHf6YXhGYeQfUzQNGk6rDNnLBQKrenN712K4AQJEG`, owned by the legacy Pyth Oracle program
`FsJ3A3u2vn5cTVofAjvy6y5kwABJAqYWpe4975bi2epH` — this account exists on-chain (`getAccountInfo`
returns real data, 3312-byte account) and is a textbook example of a Pyth-write-authority,
continuously-updated account.

**I attempted but could not verify an equivalent address for any of the AAPLX/TSLAX/NVDAX Crypto
feeds or the AAPL/TSLA/NVDA Equity feeds.** I tried computing the canonical "shard 0"
program-derived-address under the newer receiver program, using the seed scheme
`["price_feed", shard_id_u16_LE, feed_id_32_bytes]` against `rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ`
(implemented with `libsodium`'s ed25519 on-curve check via `pynacl`, matching Solana's own PDA
algorithm). As a control I ran the exact same derivation for the well-known, certainly-sponsored
`Crypto.SOL/USD` feed id — and my derived address **did not exist on-chain either** (`getAccountInfo`
returned `null`). That means either my seed-scheme guess is wrong, or sponsored push-feed accounts
under the new receiver architecture are not simple shard-0 PDAs of `["price_feed", shard, feed_id]`.
Since I could not validate the method even on a known-good feed, **I am not reporting any derived
address for the xStocks feeds as fact** — this is a genuine open question (see §Open questions).

**What I can say with confidence:** these feed IDs exist, are queryable, and are actively updating
(Hermes serves live prices for them — see below). Whether a caller can read them for free from an
already-populated Solana account (sponsored/push) or must pay to post a pull update themselves
before consuming them on-chain is **not established** from this session's evidence.

### Live price snapshot (via `app.pyth.com`, a Pyth-operated price explorer — Hermes' direct REST
API required an API key during this session and returned `401 unauthorized` on `/v2/updates/price/latest`, so I used the public explorer UI instead)

| Feed | Price shown | Time |
|---|---|---|
| `Crypto.AAPLX/USD` | **337.42111404** | live |
| `Equity.US.AAPL/USD` | **336.28500** (+0.65% 24h) | "08:47:00 UTC+0" |

### Per-display-token vs. per-raw-base-unit: my answer and confidence

**My answer: the Pyth `Crypto.{TICKER}X/USD` price quotes per display/UI (post-multiplier) token,
not per raw base unit — i.e. it is designed to track the value of one underlying share, which is
what `raw_amount × multiplier` represents.**

Evidence, ranked by strength:

1. **Price magnitude match (on-chain-adjacent, strongest evidence I have).** Live `Crypto.AAPLX/USD`
   ($337.42) sits within ~0.34% of live `Equity.US.AAPL/USD` ($336.29) — i.e. one xStock token
   trades for approximately the price of one AAPL share. If the feed instead priced the *raw*
   base unit (pre-multiplier), the two numbers would only coincidentally be close (multiplier is
   ≈1.003 here, so it wouldn't actually create visible divergence at this specific moment — see
   caveat below). What *does* support "post-multiplier" as the design intent is combining this
   with point 2 and 3.
2. **xStocks developer docs are explicit that the raw amount is NOT the economically meaningful
   number**: "The raw amount is the actual number of tokens stored on-chain that never changes
   due to corporate events and should be used when building transactions. The scaled amount is
   the raw amount × multiplier, which reflects the true equity value and should be used when
   displaying values to users" (`docs.xstocks.fi`, multipliers/developers pages). A price feed
   meant to value a holding for collateral purposes is a "display value" use case by this
   docs' own taxonomy, not a "build a transaction" use case.
3. **The redemption-rate feed's entire reason to exist** (`Crypto.AAPLX/AAPL.RR` etc.) is to let a
   consumer measure the spread between "price of one xStock" and "price of one underlying share" —
   this only makes sense if both are already expressed on the same per-share (i.e. post-multiplier)
   basis; if `Crypto.AAPLX/USD` were quoting raw base units, the "redemption rate" comparison
   against AAPL/share would be off by the multiplier and non-comparable without a hidden
   normalization step nobody documents.
4. **Real-world corroboration (secondary, unverified provenance)**: I found a GitHub PR
   (`PixStock/pixstock#13`, "price the xStocks off their own feeds, not off the shares behind
   them") from an unrelated third-party project whose stated reasoning is that xStocks trade at a
   premium/discount to the underlying and should be priced off their own `Crypto.TSLAX/USD`-style
   feed rather than the equity feed for their share — consistent with (but not proof of) the
   per-display-token framing. This is a random public repo, not an authoritative source; I'm
   citing it only as directional corroboration.

**What I could NOT find:** one canonical sentence from Pyth or Backed stating literally
"Crypto.{X}/USD is quoted per display/UI token, not per raw base unit." I inferred it from the
above. **Confidence: Medium-High.** The practical design conclusion for a lending protocol is:
value collateral as `raw_token_balance × current_effective_multiplier × pyth_price`, not
`raw_token_balance × pyth_price`.

**Important secondary finding — the multiplier itself is not "one number, always up to date."**
Per Solana's own extension docs (`solana.com/docs/tokens/extensions/scaled-ui-amount`), the
on-chain `multiplier` field is *not* automatically rewritten when `new_multiplier_effective_timestamp`
passes; instead, **conversion logic must compare current time to that timestamp and pick
`multiplier` (before) or `new_multiplier` (at/after)** — the two fields co-exist on-chain
indefinitely until the issuer pushes the next update. Concretely, as of this report (2026-09-18):
- AAPLX's `newMultiplierEffectiveTimestamp` (1786149000 = 2026-08-08T00:30:00Z) has **already
  passed**, so the multiplier a correct consumer should be using **right now** is
  `newMultiplier = 1.0032690125398187`, not the stale `multiplier = 1.0026642075893797` field.
- NVDAX's effective timestamp (1789000200 = 2026-09-10T00:30:00Z) has likewise already passed;
  effective multiplier is `1.001701196801074`, not `1.0009180758490996`.
- TSLAX is simply `multiplier = newMultiplier = "1"`, `effective ts = 0` — no drift, ever, so far.

**Any consumer/oracle design must implement this "pick by comparing block time to the effective
timestamp" logic itself** — reading the raw `multiplier` field naively will be silently stale
whenever a corporate action has occurred and the issuer hasn't pushed a subsequent update yet.

---

## 4. Multiplier behaviour in practice — REAL non-1 multiplier observed on-chain

Two of the three mints checked (AAPLX, NVDAX) currently have (and have already updated past) a
non-1 multiplier; TSLAX does not (still exactly 1). This directly answers the "has any xStock
actually used a non-1 multiplier" question: **yes, confirmed on-chain for AAPLX and NVDAX.**

What kind of corporate action produced it: **dividend reinvestment, not a share-count stock
split**, per xStocks' own documentation (`docs.xstocks.fi/docs/dividends-and-stock-splits`):
> "When a company pays a dividend, the custodian receives the cash on behalf of all shares held
> in custody. Rather than distributing cash, the dividend is reinvested into additional shares of
> the same stock. This reinvestment is reflected through a multiplier increase."
> "Example: Apple pays a dividend. The multiplier moves from 1.0 to 1.008."

That worked example (1.0 → 1.008 for one Apple dividend) is the same order of magnitude as the
observed on-chain drift (AAPLX at ~1.0027 → ~1.0033; note AAPL launched on xStocks ~June 2025 and
has paid multiple quarterly dividends since, so cumulative compounding to ~1.003 by two cycles in
is plausible, not surprising). **I found no evidence, on-chain or in documentation, of an actual
realized stock-split multiplier jump (e.g. a 2x, 3x, or 4:1 change) for any of the three tickers
checked.** The docs give a 4-for-1 split as a purely illustrative example ("the multiplier moves
from 1.008 to 4.032"), not a real historical event. **"No confirmed real stock split observed" is
my answer for splits specifically — dividend-driven drift is confirmed real for 2 of 3 tickers.**

Operationally important detail also found in the multipliers doc: **the activation time is always
00:30 UTC the day after the ex-dividend date**, and xStocks explicitly recommends that "trading
venues and protocols pause all interactions with the token for a brief window (e.g., 15 minutes)
before and after each activation timestamp" — see §5.

---

## 5. Issuer powers and things that would break a lender

### Confirmed on-chain (not just documented) — the issuer currently holds:
- **Freeze authority** (`JDq14BWvqCRFNu1krb12bcRpbGtJZ1FLEakMw6FdxJNs`, same address across all
  three mints checked) — can freeze any individual token account. Currently no mint is paused
  (`pausableConfig.paused: false` for all three, meaning the *global pause switch* is off), but
  the freeze authority is a **separate, standard Token-2022 power** independent of the Pausable
  extension and can freeze individual accounts at will regardless of the pause flag.
- **Permanent delegate** (`5aMNNLQJwAEeoemTEMkv5NVjqKwvvefRYCQ5Z67HFvEq`, same across all three) —
  this address can transfer or burn tokens out of *any* holder's account without their signature,
  by design of the Token-2022 `PermanentDelegate` extension. This is the forced-transfer power the
  task asked about; it exists and is live on all three mints today.
- **Global pause switch** (`pausableConfig`, authority `JDq14BW...` — same as freeze authority) —
  can halt all transfers/mints/burns mint-wide. Off right now on all three checked.
- **`DefaultAccountState` present but currently benign** (`"initialized"`, i.e. new accounts are
  NOT frozen-by-default today) — see §2 flag. xStocks' own developer docs (from search result
  content, not independently re-fetched to a stable URL this session) state this extension is
  present specifically to allow Backed to **enable sRFC-37 blocklist-style compliance later**,
  i.e. it's a reserved, not-yet-activated power, not dead code.
- **`TransferHook` present with `programId: null`** — matches the brief's assumption (no hook
  logic wired up today), but the authority to *set* a hook program in the future exists
  (`authority: 5aMNN...FvEq`, same address as permanent delegate/confidential-transfer/metadata
  authority) — i.e. Backed could deploy transfer-time logic later without a new mint.

**Design implication:** a lending protocol holding these as collateral is trusting a single
cluster of Backed-controlled keys (two authorities observed: `JDq14BW...` for freeze+pause,
`5aMNN...FvEq` for permanent-delegate+confidential-transfer+metadata+transfer-hook-authority, plus
a third, `S7vYFFWH6BjJyEsdrPQpqpYTqLTrPRK6KW3VwsJuRaS`, solely for multiplier updates) with the
power to freeze any single account, forcibly move/burn any balance, pause the whole mint, and
(reserved, not active) default-freeze new accounts or attach transfer-hook logic. None of these
are Solana/Token-2022 bugs — they are the intended design of a regulated, custodian-backed RWA
token — but a protocol must decide how it treats liquidation/repayment flows if Backed ever
freezes or pauses a position mid-loan.

### From documentation (xStocks docs, `docs.xstocks.fi`, and secondary reporting):

- **Issuer / legal structure**: xStocks are issued by *Backed Assets (JE) Limited*, a
  Jersey-based bankruptcy-remote SPV, registered with the Jersey Financial Services Commission,
  operating under a base prospectus approved by the Liechtenstein FMA for EU/EEA distribution.
  Full 1:1 collateralization per asset, segregated custody accounts under a three-party Account
  Control Agreement, and an independent Security Agent with visibility over collateral.
- **Issuer default**: "In the event of issuer default, the Security Agent may take control of the
  collateral accounts, liquidate the underlying assets, and distribute proceeds to token holders
  in accordance with the prospectus terms." — a formal wind-down path exists, but it is off-chain,
  legal-process-dependent, and not instantaneous.
- **Redemption**: direct issuer redemption is restricted to **business days when the US market is
  open** ("normally 24/5"), subject to KYC, with a **$5,000 minimum** redemption size. Secondary
  market trading (Kraken, Bybit, DEXs) is 24/7 and does not carry this restriction — but the
  arbitrage mechanism that keeps the secondary-market price near NAV (the redemption path) is
  itself only available 24/5 with a real-money minimum, which is exactly the kind of gap that
  widens the AAPLX/AAPL premium-discount the redemption-rate feed exists to measure.
- **Multiplier activation windows**: xStocks explicitly recommends "trading venues and protocols
  pause all interactions with the token for a brief window (e.g., 15 minutes) before and after
  each activation timestamp" (00:30 UTC the day after ex-date) to avoid transaction complications
  during a multiplier flip. **A lending protocol that does not pause borrows/liquidations/oracle
  reads around scheduled multiplier flips is operating outside the issuer's own documented safe
  envelope** — this should probably become a design requirement (poll
  `https://api.xstocks.fi/api/v2/public/assets/{SYMBOL}/multiplier?network={NETWORK}` for the
  pending activation timestamp and gate around it, per the developer docs).
- **I could not find** an explicit doc passage describing trading-hours restrictions on the
  *token* itself (as distinct from direct-redemption hours) — secondary trading is stated to be
  continuous 24/7, and I found nothing describing the token becoming non-transferable outside
  market hours. I also could not reach the full legal prospectus/Final Terms documents
  (referenced by the docs as living at `assets.backed.fi/legal-documentation`) in this session —
  see Open Questions.
- A real lending protocol already exists in this space for context: search results surfaced
  **NestUSD**, described as "the first DeFi protocol letting holders of tokenized US equities
  borrow nUSD" against xStocks collateral — I did not investigate its implementation in this
  session, but it may be a useful reference for how someone else solved the multiplier/oracle
  problem in production. Flagging for follow-up, not verified here.

---

## Open questions (things I could not establish this session)

1. **No canonical, Backed-published machine-readable mint-address list found.** The three
   addresses above are corroborated by 2+ independent sources each and verified live on-chain as
   real Token-2022 mints with matching metadata, but I did not find/fetch an authoritative
   "official list" endpoint from backed.fi or xstocks.fi to cross-check against.
2. **Sponsored on-chain price accounts for the xStocks-specific and equity Pyth feeds: not
   established.** I confirmed the general "sponsored feed" mechanism is real (verified a live
   SOL/USD legacy sponsored account on-chain), but could not derive or find a verified address for
   any of the AAPLX/TSLAX/NVDAX/AAPL/TSLA/NVDA feed IDs — my attempted PDA derivation failed even
   as a control against the known-good SOL/USD feed under the newer receiver-program architecture,
   so I explicitly did not report a guessed address as fact. Hermes' REST endpoints
   (`/v2/updates/price/latest`, `/api/latest_price_feeds`) returned `401 unauthorized` for this
   session (API key required), which blocked one direct path to checking `write_authority`/update
   freshness on any candidate account. A follow-up with an authenticated Hermes key, or the
   `@pythnetwork/pyth-solana-receiver` JS SDK's `getPriceFeedAccountAddress` helper run directly,
   would resolve this cleanly.
3. **No confirmed real stock-split event** (as opposed to dividend-driven multiplier drift) for
   any xStock. Absence of evidence, not evidence of absence — I did not do an exhaustive sweep of
   all ~60+ xStocks tickers, only the three requested.
4. **Full legal prospectus / Final Terms documents** (at `assets.backed.fi/legal-documentation`,
   per the docs site's own pointer) were not fetched — trading-hours-on-the-token-itself,
   force-majeure/halted-stock handling, and the complete list of issuer powers may be more fully
   specified there than in the FAQ/overview pages I could reach.
5. **NestUSD** (an existing xStocks-collateral lending protocol surfaced in search) was not
   investigated — could be a useful precedent to review separately.
6. I did not verify whether `S7vYFFWH6BjJyEsdrPQpqpYTqLTrPRK6KW3VwsJuRaS` (multiplier authority)
   and `7pt9tkctJPK7PPNQJ77GKg8ZffSF6QxoMiCFYHxrtaCj` (mint authority) are simple keypairs, Squads
   multisigs, or program-owned PDAs — i.e. I did not assess the operational-security posture behind
   these authorities, only that they exist and what power they hold.

---

## Sources

- [Solana Compass — AAPLX token page](https://solanacompass.com/tokens/XsbEhLAtcf6HdfpFZ5xEMdqW8nfAvcsP5bdudRLJzJp)
- [Solflare — Apple xStock](https://www.solflare.com/stocks/apple-xstock/XsbEhLAtcf6HdfpFZ5xEMdqW8nfAvcsP5bdudRLJzJp/)
- [CoinGecko — Apple xStock](https://www.coingecko.com/en/coins/apple-xstock)
- [Orbmarkets — TSLAX](https://orbmarkets.io/token/XsDoVfqeBukxuZHWhdvWHBhgEHjGNst4MLodqsJHzoB)
- [Solflare — NVIDIA xStock](https://www.solflare.com/prices/nvidia-xstock/Xsc9qvGR1efVDFGLrVsmkzv3qi45LTBjeUKSPmx9qEh/)
- [Solana Explorer — NVDAX](https://explorer.solana.com/address/Xsc9qvGR1efVDFGLrVsmkzv3qi45LTBjeUKSPmx9qEh)
- [docs.xstocks.fi/developers/multipliers — How xStocks Handles Dividends & Stock Splits](https://docs.xstocks.fi/developers/multipliers)
- [docs.xstocks.fi/docs/dividends-and-stock-splits](https://docs.xstocks.fi/docs/dividends-and-stock-splits)
- [docs.xstocks.fi/docs/frequently-asked-questions](https://docs.xstocks.fi/docs/frequently-asked-questions)
- [docs.xstocks.fi/docs/product-legal-overview](https://docs.xstocks.fi/docs/product-legal-overview)
- [docs.xstocks.fi/developers](https://docs.xstocks.fi/developers)
- [solana.com/docs/tokens/extensions/scaled-ui-amount](https://solana.com/docs/tokens/extensions/scaled-ui-amount)
- [docs.pyth.network/price-feeds/core/push-feeds/solana](https://docs.pyth.network/price-feeds/core/push-feeds/solana)
- [docs.pyth.network/price-feeds/core/contract-addresses/solana](https://docs.pyth.network/price-feeds/core/contract-addresses/solana)
- [Pyth Hermes price_feeds search API](https://hermes.pyth.network/v2/price_feeds) (`?query=AAPL`, `?query=TSLA`, `?query=NVDA`, `?query=SOL/USD`)
- [app.pyth.com — Crypto.AAPLX/USD explorer](https://app.pyth.com/explore/Crypto.AAPLX%2FUSD)
- [app.pyth.com — Equity.US.AAPL/USD explorer](https://app.pyth.com/explore/Equity.US.AAPL%2FUSD)
- [PixStock/pixstock PR #13 — "price the xStocks off their own feeds"](https://github.com/PixStock/pixstock/pull/13) (secondary/unofficial source, directional only)
- Backed Finance — [xstocks are going live](https://backed.fi/news-updates/xstocks-are-going-live-tokenized-stocks-for-the-defi-era)
- Solana mainnet RPC `https://api.mainnet-beta.solana.com` (`getAccountInfo`, `jsonParsed`, live queries this session)
