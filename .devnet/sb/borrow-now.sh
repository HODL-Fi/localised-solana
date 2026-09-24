#!/usr/bin/env bash
# End-to-end take_loan against both live oracles.
#
# Pyth's sponsored devnet SOL/USD account refreshes in bursts every few minutes, and the
# program accepts it only while it is younger than max_price_age_seconds (60). So: wait for
# a refresh, then crank the NGN feed and borrow immediately, inside that window.
#
# This sequencing is a devnet workaround, not the production shape. In production you pull a
# signed update from Hermes and post it in the SAME transaction as take_loan, which makes
# freshness structural. Hermes now requires an API key, which is why this script uses the
# sponsored account instead.
set -euo pipefail
cd "$(dirname "$0")"
set -a; . ../addresses.env; set +a

PYTH_ACCOUNT=7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE
export PYTH_ACCOUNT

echo "waiting for a Pyth refresh window (age < 45s)…"
node -e '
const A = process.env.PYTH_ACCOUNT;
(async () => {
  for (let i = 0; i < 60; i++) {
    const r = await fetch("https://api.devnet.solana.com", { method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "getAccountInfo",
        params: [A, { encoding: "base64" }] }) });
    const d = Buffer.from((await r.json()).result.value.data[0], "base64");
    const age = Math.floor(Date.now() / 1000) - Number(d.readBigInt64LE(41 + 52));
    const px  = Number(d.readBigInt64LE(41 + 32)) / 1e8;
    process.stdout.write(`  age=${age}s price=$${px.toFixed(2)}\n`);
    if (age < 45) { console.log("  window open"); process.exit(0); }
    await new Promise((s) => setTimeout(s, 10000));
  }
  console.error("  no window in 10 minutes"); process.exit(1);
})();'

echo "cranking the NGN feed…"
CROSSBAR_URL="${CROSSBAR_URL:-http://localhost:8099}" node crank-ngn-feed.js | tail -3

echo "borrowing…"
cd ../../setup-cli && cargo run --quiet --bin take_loan
