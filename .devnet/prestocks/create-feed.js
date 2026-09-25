// Create a Switchboard On-Demand pull feed on devnet that prices one PreStocks symbol.
//
//   node create-feed.js OPENAI
//
// Three steps, in this order because the third depends on the first two:
//
//   1. Store the job on Crossbar. Oracles resolve a feed_hash to its job definition through
//      Crossbar, so a hash it has never seen makes every crank die with
//      "Bad Crossbar fetch status: 404". The SDK's storeOracleFeed posts to /v2/store; a
//      self-hosted container serves /store with a {queue, jobs} body, so this posts directly.
//      The hosted crossbar.switchboard.xyz is not an option: its /v2/store returns the hash of
//      an empty payload (see ../sb/FINDINGS.md).
//   2. Simulate the stored hash. Free, off chain, and it is the only cheap way to find out that
//      a JSONPath does not resolve before spending an account on it.
//   3. Create the on-chain feed account.
//
// The program then needs `sb_feed_hash` on the collateral asset to equal this feed's hash, which
// is what stops the feed authority repointing a pinned account at a different job.

const { Connection, Keypair, PublicKey } = require("@solana/web3.js");
const { OracleJob } = require("@switchboard-xyz/common");
const sb = require("@switchboard-xyz/on-demand");
const anchor = require("@coral-xyz/anchor");
const fs = require("fs");
const os = require("os");
const symbols = require("./symbols.js");

const RPC = process.env.RPC || "https://api.devnet.solana.com";
const CROSSBAR = process.env.CROSSBAR_URL || "https://crossbar-staging.up.railway.app";
const SYMBOL = (process.argv[2] || "").toUpperCase();

(async () => {
  if (!symbols[SYMBOL]) {
    console.error(`usage: node create-feed.js <SYMBOL>\nknown: ${Object.keys(symbols).filter((k) => k === k.toUpperCase() && !["DECIMALS", "API"].includes(k)).join(" ")}`);
    process.exit(2);
  }
  const connection = new Connection(RPC, "confirmed");
  const payer = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(fs.readFileSync(os.homedir() + "/.config/solana/id.json", "utf8")))
  );
  const provider = new anchor.AnchorProvider(connection, new anchor.Wallet(payer), { commitment: "confirmed" });
  const program = await sb.AnchorUtils.loadProgramFromProvider(provider);
  const queue = new PublicKey(process.env.SWITCHBOARD_QUEUE || sb.ON_DEMAND_DEVNET_QUEUE);
  console.log(`symbol   ${SYMBOL}`);
  console.log(`payer    ${payer.publicKey.toBase58()}`);
  console.log(`queue    ${queue.toBase58()}`);

  // 1. store
  // PRESTOCKS_PATH overrides the JSONPath, for probing what the oracles' task runner supports.
  const spec = process.env.PRESTOCKS_PATH || process.env.PRESTOCKS_URL
    ? {
        tasks: [
          { httpTask: { url: process.env.PRESTOCKS_URL || symbols.API } },
          { jsonParseTask: { path: process.env.PRESTOCKS_PATH || symbols.jobFor(SYMBOL).tasks[1].jsonParseTask.path } },
        ],
      }
    : symbols.jobFor(SYMBOL);
  if (process.env.PRESTOCKS_URL) console.log(`url      ${process.env.PRESTOCKS_URL}`);
  const job = OracleJob.toObject(OracleJob.fromObject(spec));
  console.log(`path     ${spec.tasks[1].jsonParseTask.path}`);
  const stored = await fetch(`${CROSSBAR}/store`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ queue: queue.toBase58(), jobs: [job] }),
  });
  if (!stored.ok) throw new Error(`crossbar /store HTTP ${stored.status}: ${(await stored.text()).slice(0, 200)}`);
  const { feedHash: hashHex, cid } = await stored.json();
  const hash = String(hashHex).replace(/^0x/, "");
  if (!/^[0-9a-f]{64}$/.test(hash)) throw new Error(`crossbar returned no usable feed hash: ${hashHex}`);
  console.log(`hash     ${hash}  (cid ${cid})`);
  console.log(`minResp  ${Number(process.env.MIN_RESPONSES || 1)}  (jobs: 1)`);

  // 2. simulate — proves the job resolves before an account is spent on it.
  //
  // Retried, because a single simulate call is not evidence either way: running seven of these
  // back to back, one returned `[]` for a job that resolves perfectly on the next attempt. The
  // check is here to catch a job that can NEVER resolve (a bad JSONPath, a dead URL), so it must
  // not fail on one slow upstream fetch — and equally must not be dropped, since without it a
  // permanently broken job costs an account and is only discovered at crank time.
  let results = [];
  for (let attempt = 0; attempt < 4; attempt++) {
    if (attempt) await new Promise((r) => setTimeout(r, 1500 * attempt));
    try {
      const sim = await fetch(`${CROSSBAR}/simulate/${hash}`);
      results = (await sim.json())?.[0]?.results ?? [];
    } catch { results = []; }
    if (results.length && Number.isFinite(Number(results[0]))) break;
    if (attempt) console.log(`  simulate returned ${JSON.stringify(results)}, retrying`);
  }
  if (!results.length || !Number.isFinite(Number(results[0]))) {
    throw new Error(`job stored but does not resolve after 4 attempts: ${JSON.stringify(results)}`);
  }
  console.log(`simulated $${Number(results[0]).toFixed(4)} per display token`);

  // 3. create the on-chain feed. Not PullFeed.initTx: it calls asV0TxWithComputeIxs with
  // neither payer nor signers, so it always throws "Payer not provided".
  const [pullFeed, feedKeypair] = sb.PullFeed.generate(program);
  const initIx = await pullFeed.initIx({
    name: `${SYMBOL}/USD`,
    queue,
    feedHash: Buffer.from(hash, "hex"),
    maxVariance: 1.0,
    minResponses: Number(process.env.MIN_RESPONSES || 1),
    numSignatures: 3,
    payer: payer.publicKey,
  });
  const tx = await sb.InstructionUtils.asV0TxWithComputeIxs({
    connection,
    ixs: [initIx],
    payer: payer.publicKey,
    signers: [payer, feedKeypair],
    // The helper sets the limit to exactly what its simulation consumed, which under-measures
    // PullFeedInit — it creates an ATA and a lookup table.
    computeUnitLimitMultiple: 1.6,
  });
  const sig = await connection.sendTransaction(tx, { skipPreflight: false });
  const conf = await connection.confirmTransaction(sig, "confirmed");
  if (conf.value?.err) {
    // confirmTransaction resolving does NOT mean the transaction succeeded.
    const failed = await connection.getTransaction(sig, { maxSupportedTransactionVersion: 0 });
    console.error("on-chain FAILURE:", JSON.stringify(conf.value.err));
    (failed?.meta?.logMessages ?? []).slice(-12).forEach((l) => console.error("  " + l));
    process.exit(1);
  }

  console.log(`\n${SYMBOL}_FEED=${pullFeed.pubkey.toBase58()}`);
  console.log(`${SYMBOL}_FEED_HASH=${hash}`);
  console.log(`SIG=${sig}`);
})().catch((e) => {
  console.error("FAILED:", e.message || e);
  if (e.logs) console.error(e.logs.slice(-14).join("\n"));
  process.exit(1);
});
