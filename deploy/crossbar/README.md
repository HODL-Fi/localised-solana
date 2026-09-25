# Crossbar on Railway

Self-hosted Switchboard Crossbar, so feed creation and cranking stop depending on a container
running on someone's laptop.

## Why self-host at all

The hosted `crossbar.switchboard.xyz` returns the hash of an **empty payload** from `/store`. A feed
created against that hash can never be resolved by an oracle, and every crank dies with
`ORACLE_UNAVAILABLE: No oracle responses received`. Four different request shapes returned the same
empty-payload hash — see `.devnet/sb/FINDINGS.md`. Its `/simulate/jobs` is broken too: the
known-good NGN job fails there identically.

So this is not a preference. Without a working `/store` there is no way to create a Switchboard feed.

## What this image changes, and what it does not

One line. Upstream's entrypoint runs `PORT=8080 bun run`, hard-coded, which makes Railway's injected
`$PORT` a no-op and leaves its HTTP proxy pointing at a port nothing is listening on.
`entrypoint.sh` here is the same script — same restart supervisor — binding `${PORT:-8080}`, so it
still behaves identically under a plain `docker run`.

The base image is pinned **by digest**, not `:latest`. This is infrastructure that feeds resolve
through; a silent upstream retag would change how prices reach the program, and a failing borrow is
a poor way to find that out.

## Verified before deploying

Built and run locally with `PORT=9111` injected:

- bound 9111, not 8080 — `crossbar listening on 9111` in the logs
- `GET /simulate/{hash}` returned the live OPENAI job at $1,023.68
- `POST /store` returned the **exact hash already on chain** for KALSHI
  (`475b93d2…`), so it agrees with the container that created the live feeds
- a **novel** job — one nobody had stored — became retrievable from the *public*
  `crossbar.switchboard.xyz/fetch/{hash}` within ~3 seconds, which is the decisive test: it means
  oracles can resolve feeds created through this instance

That last one is the only check that actually proves a hosted Crossbar is usable. Storing
successfully is not enough; the job has to reach the oracles.

## Live

```
URL          https://crossbar-staging.up.railway.app
workspace    HODL
project      gregarious-compassion      environment  staging      service  crossbar
image        built from this directory, base pinned by digest 4aad27ab…
```

In `gregarious-compassion` on purpose: `localised-backend` lives there, so Railway's private
networking applies and the backend can call `crossbar.railway.internal` without leaving Railway. That
is **per-environment** — it resolves for the staging instance only. `prod-active` is untouched; when
you want it there it is `railway up --environment prod-active` against the same service, not a new
project.

Verified after deploying, in this order:

- `crossbar listening on 8080` in the logs, and Railway's proxy found it
- `/simulate/{hash}` returned the live OPENAI job at $1,023.70
- `/simulate/solana/devnet/{feed}` resolved an **on-chain** feed by pubkey
- a novel job stored through the Railway instance was retrievable from the public
  `crossbar.switchboard.xyz/fetch` in ~3s
- a real crank landed on chain — 3 oracles agreeing at $1,023.68, `result.slot` 98 slots old
- then the **local container was stopped** and the NGN feed still cranked, which is the only proof
  that nothing quietly depends on a laptop any more

`.devnet/addresses.env` and all four scripts now default to the Railway URL. A local container still
works as a fallback with `CROSSBAR_URL=http://localhost:8099`.

## Deploy

```bash
railway login                 # interactive, opens a browser
railway init                  # or `railway link` into an existing project
railway up                    # from this directory
```

Then set the variables and the target port:

```bash
railway variables --set SOLANA_DEVNET_RPC=https://api.devnet.solana.com \
                  --set SOLANA_MAINNET_RPC=https://api.mainnet-beta.solana.com
railway domain                # generates the public URL
```

The service must be told the container listens on **8080** unless `PORT` is injected, which Railway
does by default — the entrypoint here honours it, so no target-port override should be needed. Check
the deploy logs for `crossbar listening on <port>` and confirm it matches what the proxy targets.

Finally, point the scripts at it. Every one reads `CROSSBAR_URL`, so this is the whole migration:

```bash
# .devnet/addresses.env
CROSSBAR_URL=https://<service>.up.railway.app
```

Verify with a job whose answer is already known:

```bash
curl -s "$CROSSBAR_URL/simulate/b1226ec9186ade5aa70a064db0b5d136584b4d8e449eecb8d4ddb937e870859f"
# expect a markPrice near the live OPENAI figure
```

## Three things to decide before leaving it running

**The endpoint is public and unauthenticated.** Anyone who finds the URL can `POST /store` — which
pins to IPFS — and hit `/updates`, which spends the RPC quota configured below. Switchboard's own
hosted instance is public too, so this is the normal posture for the software; the difference is that
here it is your bill. Railway's private networking is an option if only your own backend needs it,
at the cost of no longer being able to crank from a laptop.

**The RPC URLs above are the public Solana endpoints, and they rate-limit.** We hit
`api.mainnet-beta.solana.com`'s limit during this work from a single machine, and the error arrives
as a non-JSON body that naive clients feed to `JSON.parse` — it surfaced as `Unexpected token 'T'`.
A shared Railway egress IP makes this likelier, not less. A dedicated RPC (Helius, QuickNode, Triton)
on a free tier is the fix, and it is a variable change, not a code change.

**It wants about 650 MiB of RAM at idle** — measured on the local container. That is comfortably
inside Railway's limits but it is not free at 24/7, so it is worth knowing before it becomes a line
item nobody remembers approving.
