# Mainnet deployment runbook

Written for a same-day deployment. The mechanical part takes about 40 minutes. The rest of
the time goes on decisions only you can make, listed first because two of them change what
you deploy.

**The code is ready**: 282 tests green under both feature builds, clippy clean under both, a
whole-branch review whose only High finding was a test-script bug (fixed), and the full
priced path proven end-to-end on devnet. What follows is about configuration and keys, not
code quality.

---

## Before anything: four decisions

### D1. Program id — **DONE**

The previously declared `J9sKAhm2EhdJQ3bHeP2KUCxqZ4cYdBc65C3RDr4JjGEd` had no keypair
anywhere and nothing was ever deployed at it, so it was unusable. Replaced:

```
mainnet program id  5t7smXPGCvTMXYUAJ2uU4Zk4uPggkN7KdanoywHrveMd
keypair             ~/.config/solana/hodl_loans-mainnet.json   (outside the repo)
```

> **Back that keypair up off this machine before deploying.** It is the only key that can
> ever upgrade the program, and `target/`-adjacent files are not backed up by anything.

### D2. Pin the collateral price account — **security-relevant, do not skip**

Devnet lists wSOL **unpinned** (`price_account = Pubkey::default()`) because Pyth's sponsored
devnet feed is barely maintained. `state/collateral.rs:26` documents the cost of unpinned
plainly: *the caller may pick the most favourable update inside the age window.*

On mainnet that is a real value leak — a borrower shops the best SOL price in a 60-second
window and borrows more than they should.

**Mainnet Pyth is maintained.** Sampled just now, `7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE`
cycled 14s → 42s → 55s old at $114.81. So **pin it**.

**DONE** — `setup-cli` now pins automatically off devnet. It has its own `devnet` feature
forwarding to `hodl_loans/devnet`, so:

```bash
cargo run --bin setup-cli                       # devnet: devnet ids, UNPINNED
cargo run --no-default-features --bin setup-cli # mainnet: mainnet ids, PINNED
```

Check which you built before running anything against real money:

```bash
cargo run --quiet --no-default-features --bin whoami
#   program id     5t7smXPGCvTMXYUAJ2uU4Zk4uPggkN7KdanoywHrveMd
#   switchboard    SBondMDrcV3K4kxZR1HNVT7osZxAHVHgYXL5Ze1oMUv
#   devnet feature false
```

One caveat that follows: 55s against a 60s bound is thin. Under congestion you will see
occasional `StalePrice` (6005). Either post your own Hermes update in the transaction (needs
a Hermes API key — see D4) or have the backend rebuild and retry on 6005.

### D3. Admin key separation — **the largest residual risk**

Devnet has one wallet as admin, guardian, whitelister, promo signer and treasury. On mainnet
that single key can pause markets, change collateral parameters, revoke promos and move
treasury funds.

`initialize` takes them as four separate arguments, so separating them costs nothing but
coordination:

- **admin** — parameter changes. Strongest key. A multisig if you have one.
- **guardian** — pause only. Should be reachable fast, so a hot key is defensible.
- **whitelister** — onboarding. Likely your backend's key.
- **treasury** — receives fees.

Whoever runs `solana program deploy` becomes the **upgrade authority**, and `initialize`
requires that same wallet. Plan the deploy from the wallet you want holding upgrade rights,
or transfer it immediately after with `solana program set-upgrade-authority`.

### D4. Inputs nobody here can supply

- **The real cNGN mint address on mainnet.** Do not guess it. Everything keys off this, and
  a wrong mint means a market nobody can use.
- **Which collateral assets to list at launch**, with their Pyth feed ids and
  `price_account` addresses from Pyth's mainnet feed list.
- **A Pyth Hermes API key**, if you want the post-your-own-update flow. The public endpoint
  now returns 401.
- **Starting market parameters.** Devnet used 15% interest, 5% penalty, 10% reserve factor,
  90% utilization cap, 1,000 cNGN minimum loan, 365-day max tenure. Confirm these are the
  intended production economics rather than inherited test values.

---

## The unresolved NGN oracle problem

**This is the one that can actually stop you**, and it needs a decision early because the
mitigation takes time.

The market's `ngn_feed` must be a Switchboard On-Demand feed. There was none on devnet and
almost certainly none on mainnet — we created the devnet one. Creating a mainnet one requires
Crossbar, and **the hosted `crossbar.switchboard.xyz/v2/store` is broken**: it returns a hash
of an empty payload, so oracles can never resolve the job (four different request shapes, same
empty result — see `.devnet/sb/FINDINGS.md`).

The devnet workaround was to self-host Crossbar:

```bash
docker run -d --name crossbar -p 8099:8080 \
  -e SOLANA_MAINNET_RPC=<your mainnet RPC> \
  -e SOLANA_DEVNET_RPC=https://api.devnet.solana.com \
  switchboardlabs/crossbar:latest
```

Then `create-ngn-feed.js` / `crank-ngn-feed.js` in `.devnet/sb/`, with the mainnet queue
instead of the devnet one. Two traps recorded there: the SDK posts to `/v2/store` while the
container serves `/store` with a `{queue, jobs}` body, and `PullFeed.fetchUpdateIx` cannot
work against a self-hosted instance because it calls `/gateways`, which the container does
not serve — use `fetchSolanaUpdates`.

**Also settle who cranks it in production.** It is a pull oracle: nothing updates it on a
schedule. Either every borrowing transaction carries the pull instructions (correct, and what
the backend should do), or you run a cranker and accept that a lapse makes borrowing fail.

**If NGN pricing is not ready, you can still deploy and initialize.** Unpriced paths work
without it: liquidity in and out, positions, collateral deposits, the admin surface. Borrowing
and liquidation do not. Launching lending-disabled is a legitimate staged option.

---

## Deployment

### 1. Build and verify

```bash
cd ~/work/hodl/lendbit-solana
./scripts/test.sh                       # 282 expected
cargo clippy -p hodl_loans --all-targets -- -D warnings
cargo build-sbf --tools-version v1.52   # NO --features devnet
shasum -a 256 target/deploy/hodl_loans.so
```

**Do not pass `--features devnet`.** That flag selects the devnet Switchboard program id and
the devnet program address; a devnet binary on mainnet fails every health check while
deposits keep working, so the program looks alive and is unusable.

Sanity-check the flag really is off: build both ways and confirm the hashes differ.

### 2. Fund and deploy

A 1,148,376-byte program costs **~11.7 SOL** with the loader's default 2× upgrade headroom,
or **~5.8 SOL** with `--max-len` set to the exact size. `--max-len` means a future upgrade
larger than the current binary needs a redeploy at a new address — **for mainnet, take the
headroom.** The saving is not worth re-pointing every integration later.

```bash
solana program deploy target/deploy/hodl_loans.so \
  --program-id ~/.config/solana/hodl_loans-mainnet.json \
  --url mainnet-beta
solana program show <program-id> --url mainnet-beta
```

Verify what landed is what you built:

```bash
solana program dump <program-id> /tmp/onchain.so --url mainnet-beta
shasum -a 256 /tmp/onchain.so target/deploy/hodl_loans.so   # must match
```

### 3. Initialize

Order is in `docs/BACKEND_INTEGRATION.md` §7. `initialize` must be signed by the upgrade
authority. Use the real role keys from D3, not one wallet four times.

### 4. Smoke test, in this order

Stop at the first failure.

1. Read `Config` back; confirm all four authorities are the intended keys.
2. `create_market` + `create_promo_vault`; read the `Market`.
3. `list_collateral` for one asset — **pinned** per D2.
4. `deposit_liquidity` with a **small** amount. Proves token CPI and share math.
5. `deposit_collateral`, then **`take_loan` with a minimal amount.** This is the step that
   proves both oracles. A `PriceAccountMismatch` (6007) here while deposits succeed means a
   devnet binary — go back to step 1.
6. `repay_loan` in full, and confirm the position clears.
7. `withdraw_liquidity` for the amount from step 4.

Only after 7 passes should real liquidity go in.

---

## What I would not do in six hours

Stated once, as information rather than obstruction — it is your call and the code is ready
in the sense that it passes everything we can test.

- **This program has not been externally audited.** It holds user collateral and lender
  funds. 282 tests, every `HodlError` variant asserted, and a clean whole-branch review are
  real evidence, but they are not an audit.
- **The overdue-penalty sandwich is a known, accepted risk** (spec §11). Task 7's probe bounds
  it to a pro-rata share of the captured step; it does not prevent it. Fine if that trade is
  understood, not fine if it is news on launch day.
- **Consider launching with conservative caps** — a low `deposit_cap` per collateral asset and
  a small initial `deposit_liquidity` — so the first day's exposure is bounded by
  configuration rather than by trust. Both are adjustable later without redeploying.

A staged launch (deploy + initialize + tiny liquidity today, open up after the NGN oracle and
key separation are settled) gets the deadline met without the six-hour window deciding how
much money is at risk.

---

## Rollback

There is no undo for a mainnet deploy, but there are three levers:

- **`set_market_paused`** — guardian stops borrowing immediately. Fastest lever; make sure the
  guardian key is reachable by a human on launch day.
- **`set_collateral_paused` / `set_collateral_borrow_paused`** — narrower, per asset.
- **Upgrade** — `solana program deploy` again with the same keypair. Requires the upgrade
  authority, which is why D1's backup matters.

Funds already deposited stay withdrawable while paused: `withdraw_liquidity` and
`repay_loan` do not read prices, so they keep working even with a broken oracle.
