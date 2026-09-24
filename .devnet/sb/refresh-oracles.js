// Refresh BOTH oracles the program reads, then print the accounts take_loan needs.
//
//   - Pyth SOL/USD: pull a signed update from Hermes and post it through the receiver
//     program. That creates an ephemeral PriceUpdateV2 account. The collateral is listed
//     UNPINNED, so the program accepts any receiver-owned account carrying the right feed
//     id inside max_price_age_seconds (60).
//   - Switchboard NGN/USD: fetch signed oracle responses and land them on the feed account.
//     The program wants result.slot within ngn_max_stale_slots (150, ~60s).
//
// Run standalone this proves both paths. In production, put these instructions AHEAD of
// take_loan in the SAME transaction so both prices are fresh by construction rather than
// by racing a second transaction.

const { Connection, Keypair, PublicKey } = require("@solana/web3.js");
const { PythSolanaReceiver } = require("@pythnetwork/pyth-solana-receiver");
const { HermesClient } = require("@pythnetwork/hermes-client");
const { CrossbarClient } = require("@switchboard-xyz/common");
const sb = require("@switchboard-xyz/on-demand");
const anchor = require("@coral-xyz/anchor");
const fs = require("fs"), os = require("os");

const RPC = "https://api.devnet.solana.com";
const SOL_USD = "0xef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d";
const NGN_FEED = process.env.NGN_FEED || "GDgs76wotM4mxXSYPmizeKtdXoHqWUxqHAASNSLnrBQ1";
const CROSSBAR = process.env.CROSSBAR_URL || "http://localhost:8099";

(async () => {
  const connection = new Connection(RPC, "confirmed");
  const payer = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(
    fs.readFileSync(os.homedir() + "/.config/solana/id.json", "utf8"))));
  const wallet = new anchor.Wallet(payer);

  // ---- Pyth ----
  const hermes = new HermesClient("https://hermes.pyth.network", {});
  const updates = await hermes.getLatestPriceUpdates([SOL_USD], { encoding: "base64" });
  const receiver = new PythSolanaReceiver({ connection, wallet });
  const builder = receiver.newTransactionBuilder({ closeUpdateAccounts: false });
  await builder.addPostPriceUpdates(updates.binary.data);

  let pythAccount = null;
  await builder.addPriceConsumerInstructions(async (getPriceUpdateAccount) => {
    pythAccount = getPriceUpdateAccount(SOL_USD).toBase58();
    return [];
  });
  const txs = await builder.buildVersionedTransactions({ computeUnitPriceMicroLamports: 1000 });
  for (const { tx, signers } of txs) {
    tx.sign([payer, ...(signers ?? [])]);
    const sig = await connection.sendTransaction(tx, { skipPreflight: false });
    const conf = await connection.confirmTransaction(sig, "confirmed");
    if (conf.value?.err) throw new Error("pyth post failed: " + JSON.stringify(conf.value.err));
  }
  console.log("PYTH_ACCOUNT=" + pythAccount);

  // ---- Switchboard ----
  const provider = new anchor.AnchorProvider(connection, wallet, { commitment: "confirmed" });
  const program = await sb.AnchorUtils.loadProgramFromProvider(provider);
  const crossbar = new CrossbarClient(CROSSBAR);
  const upd = (await crossbar.fetchSolanaUpdates("devnet", [NGN_FEED], payer.publicKey.toBase58(), 3))[0];
  if (!upd.success) throw new Error("switchboard update failed");
  const sbTx = await sb.InstructionUtils.asV0TxWithComputeIxs({
    connection, ixs: upd.pullIxns, payer: payer.publicKey, signers: [payer],
    lookupTables: [], computeUnitLimitMultiple: 1.6,
  });
  const sbSig = await connection.sendTransaction(sbTx, { skipPreflight: false });
  const sbConf = await connection.confirmTransaction(sbSig, "confirmed");
  if (sbConf.value?.err) throw new Error("switchboard crank failed: " + JSON.stringify(sbConf.value.err));
  console.log("NGN_PRICE=" + upd.responses[0].result);
  console.log("NGN_FEED=" + NGN_FEED);
})().catch((e) => { console.error("FAILED:", e.message || e); process.exit(1); });
