// Create a Switchboard On-Demand NGN/USD pull feed on devnet.
//
// Why this exists: the program's market carries an `ngn_feed` pubkey that
// oracle/switchboard.rs requires be owned by SWITCHBOARD_ON_DEMAND_PID, carry the
// PullFeedAccountData discriminator, be fresher than ngn_max_stale_slots (150, ~60s)
// and have at least ngn_min_samples (3) submissions. A scan of all 5,004 PullFeed
// accounts on devnet found no NGN feed, so one has to be created.
//
// The feed reports USD per NGN (~0.00075), which is what scale_switchboard_value
// expects — the price of one NGN in dollars, not NGN per dollar. The job therefore
// inverts the quoted rate: valueTask(1) / (the fetched NGN-per-USD rate).

const { Connection, Keypair, PublicKey } = require("@solana/web3.js");
const { OracleJob } = require("@switchboard-xyz/common");
const sb = require("@switchboard-xyz/on-demand");
const fs = require("fs");
const os = require("os");

const RPC = "https://api.devnet.solana.com";

// Two independent sources so min_responses > 1 is meaningful and one API blip does
// not stall the feed. Both are free and need no key.
const jobs = [
  OracleJob.fromObject({
    tasks: [
      { valueTask: { value: 1 } },
      {
        divideTask: {
          job: {
            tasks: [
              { httpTask: { url: "https://open.er-api.com/v6/latest/USD" } },
              { jsonParseTask: { path: "$.rates.NGN" } },
            ],
          },
        },
      },
    ],
  }),
  OracleJob.fromObject({
    tasks: [
      { valueTask: { value: 1 } },
      {
        divideTask: {
          job: {
            tasks: [
              { httpTask: { url: "https://cdn.jsdelivr.net/npm/@fawazahmed0/currency-api@latest/v1/currencies/usd.json" } },
              { jsonParseTask: { path: "$.usd.ngn" } },
            ],
          },
        },
      },
    ],
  }),
];

(async () => {
  const connection = new Connection(RPC, "confirmed");
  const payer = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(fs.readFileSync(os.homedir() + "/.config/solana/id.json", "utf8")))
  );
  console.log("payer   ", payer.publicKey.toString());

  // A provider with a real wallet, because initIx resolves the payer from
  // program.provider.publicKey rather than from the params.
  const anchor = require("@coral-xyz/anchor");
  const provider = new anchor.AnchorProvider(connection, new anchor.Wallet(payer), {
    commitment: "confirmed",
  });
  const program = await sb.AnchorUtils.loadProgramFromProvider(provider);
  console.log("sb prog ", program.programId.toString());

  const queue = new PublicKey(sb.ON_DEMAND_DEVNET_QUEUE);
  console.log("queue   ", queue.toString());

  // The feed hash MUST come from Crossbar's /v2/store, not from a local computation.
  // Oracles resolve a hash to its job definition through Crossbar; a locally-derived hash
  // it has never seen makes every crank fail with "Bad Crossbar fetch status: 404".
  // That is exactly how the first attempt died.
  const { OracleJob: OJ, CrossbarClient } = require("@switchboard-xyz/common");
  const crossbar = CrossbarClient.default();
  const jobsB64 = jobs.map((j) => Buffer.from(OJ.encodeDelimited(j).finish()).toString("base64"));
  const stored = await crossbar.storeOracleFeed(
    CrossbarClient.createFeedRequestV1(jobsB64, 1.0, 2)
  );
  const feedHash = Buffer.from(stored.feedId.replace(/^0x/, ""), "hex");
  console.log("feedHash", feedHash.toString("hex"), "(stored on Crossbar, cid", stored.cid + ")");

  // Not PullFeed.initTx: it calls asV0TxWithComputeIxs with neither `payer` nor
  // `signers`, so that helper always throws "Payer not provided". Build the
  // instruction and the transaction separately and supply both.
  const [pullFeed, feedKeypair] = sb.PullFeed.generate(program);
  const initIx = await pullFeed.initIx({
    name: "NGN/USD",
    queue,
    feedHash,
    maxVariance: 1.0,      // percent
    minResponses: 2,
    numSignatures: 3,      // >= market.ngn_min_samples (3)
    payer: payer.publicKey,
  });
  const tx = await sb.InstructionUtils.asV0TxWithComputeIxs({
    connection,
    ixs: [initIx],
    payer: payer.publicKey,
    signers: [payer, feedKeypair],
    // The helper sets the limit to exactly what its simulation consumed. PullFeedInit
    // creates an ATA and a lookup table, and the first attempt died on
    // "Computational budget exceeded" at the LUT step — the simulation under-measures.
    computeUnitLimitMultiple: 1.6,
  });

  const sig = await connection.sendTransaction(tx, { skipPreflight: false });
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
  console.log("\nFEED", pullFeed.pubkey.toString());
  console.log("SIG ", sig);
})().catch((e) => {
  console.error("FAILED:", e.message || e);
  if (e.logs) console.error(e.logs.slice(-12).join("\n"));
  process.exit(1);
});
