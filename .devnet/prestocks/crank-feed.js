// Land signed oracle responses on a PreStocks feed, then read back exactly what the program will
// read: value, slot age, sample count, and feed_hash.
//
//   node crank-feed.js <FEED_PUBKEY>
//
// Switchboard On-Demand is a pull oracle — the account holds no price until someone submits one.
// Run standalone this only proves the feed produces a value. In production the pull instructions
// go AHEAD of take_loan in the SAME transaction, which makes freshness structural instead of a
// race against sb_max_stale_slots (150 slots, ~60s).

const { Connection, Keypair, PublicKey } = require("@solana/web3.js");
const { CrossbarClient } = require("@switchboard-xyz/common");
const sb = require("@switchboard-xyz/on-demand");
const anchor = require("@coral-xyz/anchor");
const fs = require("fs");
const os = require("os");

const RPC = process.env.RPC || "https://api.devnet.solana.com";
const CROSSBAR = process.env.CROSSBAR_URL || "https://crossbar-staging.up.railway.app";
const FEED = process.argv[2] || process.env.PRESTOCKS_FEED;

// Byte offset of `feed_hash` in the account, past the 8-byte Anchor discriminator. Same layout the
// program walks in oracle/switchboard.rs; read by offset here for the same reason — no need to
// materialise 3.2 KB to look at one field.
//
//   8 discriminator + 32 x sizeof(OracleSubmission) + 32 authority + 32 queue
//   OracleSubmission = Pubkey(32) + slot u64(8) + landed_at u64(8) + value i128(16) = 64
const OFF_FEED_HASH = 8 + 32 * 64 + 32 + 32; // 2120

(async () => {
  if (!FEED) { console.error("usage: node crank-feed.js <FEED_PUBKEY>"); process.exit(2); }
  const connection = new Connection(RPC, "confirmed");
  const payer = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(fs.readFileSync(os.homedir() + "/.config/solana/id.json", "utf8")))
  );
  const provider = new anchor.AnchorProvider(connection, new anchor.Wallet(payer), { commitment: "confirmed" });
  const program = await sb.AnchorUtils.loadProgramFromProvider(provider);
  const feedKey = new PublicKey(FEED);
  console.log(`feed     ${FEED}`);

  // fetchSolanaUpdates (the /updates route), not fetchUpdateIx: the latter also calls /gateways,
  // which a self-hosted Crossbar does not serve. The payer must be passed explicitly or Crossbar
  // rejects the request with "Invalid payer pubkey".
  const crossbar = new CrossbarClient(CROSSBAR);
  const upd = (await crossbar.fetchSolanaUpdates("devnet", [FEED], payer.publicKey.toBase58(), 3))[0];
  console.log(`success  ${upd.success}  pullIxns ${(upd.pullIxns || []).length}`);
  for (const r of (upd.responses || []).slice(0, 4)) {
    console.log(`  oracle ${String(r.oracle).slice(0, 10)}… result=${r.result}${r.errors ? " err=" + String(r.errors).slice(0, 70) : ""}`);
  }
  if (!upd.success || !(upd.pullIxns || []).length) throw new Error("no pull instructions returned");

  const tx = await sb.InstructionUtils.asV0TxWithComputeIxs({
    connection, ixs: upd.pullIxns, payer: payer.publicKey, signers: [payer],
    lookupTables: [], computeUnitLimitMultiple: 1.6,
  });
  const sig = await connection.sendTransaction(tx, { skipPreflight: false, maxRetries: 3 });
  const conf = await connection.confirmTransaction(sig, "confirmed");
  if (conf.value?.err) {
    // confirmTransaction resolving does NOT mean the transaction succeeded.
    const failed = await connection.getTransaction(sig, { maxSupportedTransactionVersion: 0 });
    console.error("on-chain FAILURE:", JSON.stringify(conf.value.err));
    (failed?.meta?.logMessages ?? []).slice(-12).forEach((l) => console.error("  " + l));
    process.exit(1);
  }
  console.log(`cranked  ${sig}`);

  // Read back what the program reads, rather than trusting the crank's own report.
  const feed = new sb.PullFeed(program, feedKey);
  const data = await feed.loadData();
  const raw = (await connection.getAccountInfo(feedKey)).data;
  const slot = await connection.getSlot();
  const hash = Buffer.from(raw.subarray(OFF_FEED_HASH, OFF_FEED_HASH + 32)).toString("hex");
  const value = Number(data.result.value.toString()) / 1e18;

  console.log("\n--- what the program will read ---");
  console.log(`  owner        ${(await connection.getAccountInfo(feedKey)).owner.toBase58()}`);
  console.log(`  size         ${raw.length} bytes`);
  console.log(`  feed_hash    ${hash}`);
  console.log(`  value        $${value.toFixed(6)} per display token  (raw ${data.result.value.toString()})`);
  console.log(`  std_dev      ${data.result.stdDev.toString()}`);
  console.log(`  num_samples  ${data.result.numSamples}`);
  console.log(`  result.slot  ${data.result.slot.toString()}   current ${slot}   age ${slot - Number(data.result.slot.toString())} slots`);
})().catch((e) => {
  console.error("FAILED:", e.message || e);
  if (e.logs) console.error(e.logs.slice(-14).join("\n"));
  process.exit(1);
});
