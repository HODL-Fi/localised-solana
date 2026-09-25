// Read the age of every feed a position needs, in one shot.
//
// Exists because a crank landing successfully does NOT mean the result is fresh: `result.slot` is
// the slot the ORACLES SIGNED at, not the slot the transaction landed in. A gateway that serves a
// slightly cached signature produces a result that is already outside `sb_max_stale_slots` the
// moment it hits the chain — observed at 277 slots against a 150-slot bound.
//
//   node check-freshness.js FEED [FEED ...]
const { Connection, PublicKey } = require("@solana/web3.js");
// Account offsets into PullFeedAccountData, derived from the crate's struct and verified by
// reading `value` back against a known crank (1023.705627). CurrentResult starts at struct offset
// 2256, which is 16-byte aligned, so no padding is inserted before it.
const OFF_VALUE = 2264;
const OFF_NUM_SAMPLES = 2360;
const OFF_RESULT_SLOT = 2368;
(async () => {
  const c = new Connection(process.env.RPC || "https://api.devnet.solana.com", "confirmed");
  const keys = process.argv.slice(2).map((k) => new PublicKey(k));
  const infos = await c.getMultipleAccountsInfo
    ? await c.getMultipleAccountsInfo(keys)
    : await Promise.all(keys.map((k) => c.getAccountInfo(k)));
  const slot = await c.getSlot();
  console.log(`current slot ${slot}   bound 150 slots\n`);
  let worst = 0;
  keys.forEach((k, i) => {
    const d = infos[i].data;
    const rs = Number(d.readBigUInt64LE(OFF_RESULT_SLOT));
    const ns = d.readUInt8(OFF_NUM_SAMPLES);
    const val = Number(d.readBigUInt64LE(OFF_VALUE) + (d.readBigUInt64LE(OFF_VALUE + 8) << 64n)) / 1e18;
    const age = slot - rs;
    worst = Math.max(worst, age);
    console.log(`  ${k.toBase58().padEnd(45)} result.slot ${rs}  age ${String(age).padStart(4)}  samples ${ns}  $${val.toFixed(4).padStart(10)}  ${age <= 150 ? "fresh" : "STALE"}`);
  });
  console.log(`\nworst age ${worst} slots -> take_loan would ${worst <= 150 ? "pass" : "FAIL with StalePrice"}`);
  process.exit(worst <= 150 ? 0 : 1);
})();
