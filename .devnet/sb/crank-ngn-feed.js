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
  const [ix, responses, success] = await feed.fetchUpdateIx({
    numSignatures: 3,
    payer: payer.publicKey,
  });
  console.log("oracle responses:", responses?.length ?? 0, "success:", success);
  if (responses?.length) {
    for (const r of responses.slice(0, 5)) {
      const v = r.value ? r.value.toString() : "(none)";
      console.log(`  oracle ${String(r.oracle ?? "?").slice(0, 8)}… value=${v} ${r.errors?.length ? "errors=" + r.errors.join(",") : ""}`);
    }
  }
  if (!ix) throw new Error("no update instruction produced — the job may be failing on the oracles");

  const tx = await sb.InstructionUtils.asV0TxWithComputeIxs({
    connection,
    ixs: [ix],
    payer: payer.publicKey,
    signers: [payer],
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
