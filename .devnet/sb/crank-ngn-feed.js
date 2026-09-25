// Crank the NGN/USD feed: fetch signed oracle responses and land them on-chain.
//
// Switchboard On-Demand is a pull oracle, so the feed account holds no price until
// someone submits one. The program reads `result` from the account and requires
// result.slot within market.ngn_max_stale_slots (150, ~60s) and
// result.num_samples >= market.ngn_min_samples (3).
//
// In production a client bundles this update instruction ahead of its own
// instruction in the SAME transaction, so the price is fresh by construction.
// Run standalone it just proves the feed produces a value.

const { Connection, Keypair, PublicKey } = require("@solana/web3.js");
const sb = require("@switchboard-xyz/on-demand");
const anchor = require("@coral-xyz/anchor");
const fs = require("fs");
const os = require("os");

const RPC = "https://api.devnet.solana.com";
const FEED = process.env.NGN_FEED || "Hed3Py1cr8Y37jfdMpjFabR7Mewrqk2D4k9MtpYeoQJX";

(async () => {
  const connection = new Connection(RPC, "confirmed");
  const payer = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(fs.readFileSync(os.homedir() + "/.config/solana/id.json", "utf8")))
  );
  const provider = new anchor.AnchorProvider(connection, new anchor.Wallet(payer), {
    commitment: "confirmed",
  });
  const program = await sb.AnchorUtils.loadProgramFromProvider(provider);
  const feed = new sb.PullFeed(program, new PublicKey(FEED));

  console.log("feed ", FEED);
  // payer must be passed explicitly: without it Crossbar rejects the request with
  // "Invalid payer pubkey: String is the wrong size".
  // Point at a self-hosted Crossbar. The public one's /v2/store returns a hash of an
  // empty payload, so oracles can never resolve the job and every crank dies with
  // ORACLE_UNAVAILABLE. A local instance stores correctly; its /updates route is the
  // same shape the SDK expects, so only the store side differed.
  const { CrossbarClient } = require("@switchboard-xyz/common");
  const crossbarUrl = process.env.CROSSBAR_URL || "https://crossbar-staging.up.railway.app";
  const crossbarClient = new CrossbarClient(crossbarUrl);
  console.log("crossbar", crossbarUrl);

  // Use fetchSolanaUpdates (the /updates route) rather than fetchUpdateIx: the latter
  // also calls /gateways, which a self-hosted Crossbar does not serve. /updates returns
  // the same signed pull instructions, already decoded by the SDK.
  const upd = (await crossbarClient.fetchSolanaUpdates(
    "devnet", [FEED], payer.publicKey.toBase58(), 3
  ))[0];
  console.log("success:", upd.success, " pullIxns:", (upd.pullIxns || []).length);
  for (const r of (upd.responses || []).slice(0, 4)) {
    console.log(`  oracle ${String(r.oracle).slice(0, 10)}… result=${r.result} ${r.errors ? "err=" + String(r.errors).slice(0, 70) : ""}`);
  }
  if (!upd.success || !(upd.pullIxns || []).length) throw new Error("no pull instructions returned");

  const tx = await sb.InstructionUtils.asV0TxWithComputeIxs({
    connection,
    ixs: upd.pullIxns,
    payer: payer.publicKey,
    signers: [payer],
    // upd.lookupTables are raw records, not AddressLookupTableAccount objects; the tx
    // fits without them.
    lookupTables: [],
    computeUnitLimitMultiple: 1.6,
  });
  const sig = await connection.sendTransaction(tx, { skipPreflight: false, maxRetries: 3 });
  const conf = await connection.confirmTransaction(sig, "confirmed");
  if (conf.value?.err) {
    // confirmTransaction resolving does NOT mean the transaction succeeded. The first
    // run of this script reported a feed address for a transaction that had actually
    // failed, because this check was missing.
    const tx = await connection.getTransaction(sig, { maxSupportedTransactionVersion: 0 });
    console.error("on-chain FAILURE:", JSON.stringify(conf.value.err));
    (tx?.meta?.logMessages ?? []).slice(-12).forEach((l) => console.error("  " + l));
    process.exit(1);
  }
  console.log("\ncranked, sig", sig);
})().catch((e) => {
  console.error("FAILED:", e.message || e);
  if (e.logs) console.error(e.logs.slice(-14).join("\n"));
  process.exit(1);
});
