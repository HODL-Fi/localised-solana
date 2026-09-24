// Decode a CollateralAsset straight off devnet.
//
// The point is the migration claim: an asset listed BEFORE `price_source` existed must read as
// `Pyth`, because the four new fields were taken out of the zeroed `reserved` padding and `Pyth`
// is discriminant 0. wSOL below was listed by the previous binary; if it reads as anything other
// than Pyth with a 60-second age bound, the upgrade silently repointed a live asset.
const { Connection, PublicKey } = require("@solana/web3.js");
const RPC = process.env.RPC || "https://api.devnet.solana.com";
const KIND = ["Standard", "XStock"];
const SOURCE = ["Pyth", "SwitchboardOnDemand"];

function decode(buf) {
  let o = 8;
  const u8 = () => buf.readUInt8(o++);
  const key = () => { const k = new PublicKey(buf.subarray(o, o + 32)).toBase58(); o += 32; return k; };
  const arr = () => { const h = buf.subarray(o, o + 32).toString("hex"); o += 32; return h; };
  const u16 = () => { const v = buf.readUInt16LE(o); o += 2; return v; };
  const u32 = () => { const v = buf.readUInt32LE(o); o += 4; return v; };
  const u64 = () => { const v = buf.readBigUInt64LE(o); o += 8; return v; };
  const u128 = () => { const v = buf.readBigUInt64LE(o) + (buf.readBigUInt64LE(o + 8) << 64n); o += 16; return v; };
  const a = {};
  a.version = u8(); a.bump = u8(); a.vault_bump = u8();
  a.mint = key(); a.token_program = key(); a.vault = key();
  a.decimals = u8(); a.kind = KIND[u8()];
  a.pyth_feed_id = arr(); a.price_account = key();
  a.max_price_age_seconds = u64();
  a.max_conf_bps = u16(); a.ltv_bps = u16();
  a.liquidation_threshold_bps = u16(); a.liquidation_bonus_bps = u16();
  a.deposit_cap = u64(); a.total_deposited = u64();
  a.paused = !!u8(); a.borrow_paused = !!u8();
  a.max_multiplier = u128();
  a.price_source = SOURCE[u8()];
  a.sb_feed_hash = arr(); a.sb_max_stale_slots = u64(); a.sb_min_samples = u32();
  a.reserved_nonzero = buf.subarray(o, o + 34).some((b) => b !== 0);
  a.bytes_read = o + 34;
  return a;
}

(async () => {
  const c = new Connection(RPC, "confirmed");
  for (const [label, addr] of process.argv.slice(2).map((s) => s.split("="))) {
    const info = await c.getAccountInfo(new PublicKey(addr));
    if (!info) { console.log(`${label}: NOT FOUND`); continue; }
    const a = decode(info.data);
    console.log(`=== ${label}  (${info.data.length} bytes on chain, ${a.bytes_read} decoded)`);
    for (const k of ["kind", "price_source", "decimals", "pyth_feed_id", "price_account",
                     "max_price_age_seconds", "sb_feed_hash", "sb_max_stale_slots",
                     "sb_min_samples", "ltv_bps", "liquidation_threshold_bps", "max_conf_bps",
                     "deposit_cap", "total_deposited", "max_multiplier", "reserved_nonzero"])
      console.log(`   ${k.padEnd(26)} ${a[k]}`);
  }
})().catch((e) => { console.error("FAILED:", e.message); process.exit(1); });
