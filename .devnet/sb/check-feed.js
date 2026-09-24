const { Connection, Keypair, PublicKey } = require("@solana/web3.js");
const sb = require("@switchboard-xyz/on-demand");
const anchor = require("@coral-xyz/anchor");
const fs = require("fs"), os = require("os");
const FEED = process.env.NGN_FEED;
(async () => {
  const connection = new Connection("https://api.devnet.solana.com", "confirmed");
  const payer = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(
    fs.readFileSync(os.homedir() + "/.config/solana/id.json", "utf8"))));
  const provider = new anchor.AnchorProvider(connection, new anchor.Wallet(payer), { commitment: "confirmed" });
  const program = await sb.AnchorUtils.loadProgramFromProvider(provider);
  const feed = new sb.PullFeed(program, new PublicKey(FEED));
  const d = await feed.loadData();
  const slot = await connection.getSlot();
  const r = d.result;
  console.log("  current slot  ", slot);
  console.log("  result.slot   ", r.slot.toString(), " age", slot - Number(r.slot.toString()), "slots");
  console.log("  result.value  ", r.value.toString(), "->", Number(r.value.toString()) / 1e18, "USD per NGN");
  console.log("  result.std_dev", r.stdDev.toString(), "->", Number(r.stdDev.toString()) / 1e18);
  console.log("  num_samples   ", r.numSamples ?? r.num_samples);
  console.log("  min_responses ", d.minResponses);
  console.log("  --- program requires: age <= 150 slots, num_samples >= 3 ---");
})().catch(e => console.error("ERR", e.message));
