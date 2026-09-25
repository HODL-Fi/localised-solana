// Create a devnet Token-2022 mint shaped like a PreStocks token.
//
//   node create-mint.js OPENAI [SUPPLY]
//
// Mirrors the mainnet mint as closely as XSTOCK_COLLATERAL_EXTENSIONS permits: 9 decimals, and
// MetadataPointer + TokenMetadata + ScaledUiAmount + PermanentDelegate + Pausable + TransferHook
// (no program) + DefaultAccountState(Initialized).
//
// It deliberately does NOT carry TransferFeeConfig or ConfidentialTransferFeeConfig. Those are the
// two extensions the real mints hold that the program's allowlist refuses, and the live fee is
// 100 bps — `deposit_collateral` credits the amount it asks to transfer, so a fee would credit a
// position 1% more than the shared vault actually received. That is the one way this mock does not
// mirror the real asset, it is deliberate, and it is why these mocks cannot be used to argue the
// mainnet mints are safe to list. See ./FINDINGS.md.
//
// The multiplier is set at initialization to the symbol's LIVE mainnet value, because a mock at
// multiplier 1 would leave the whole scaled-UI path untested while looking green.

const {
  Connection, Keypair, PublicKey, SystemProgram, Transaction, sendAndConfirmTransaction,
} = require("@solana/web3.js");
const {
  TOKEN_2022_PROGRAM_ID, ExtensionType, getMintLen, AccountState,
  createInitializeMetadataPointerInstruction,
  createInitializeScaledUiAmountConfigInstruction,
  createInitializePermanentDelegateInstruction,
  createInitializeTransferHookInstruction,
  createInitializeDefaultAccountStateInstruction,
  createInitializePausableConfigInstruction,
  createInitializeMint2Instruction,
  createAssociatedTokenAccountInstruction,
  createMintToInstruction,
  getAssociatedTokenAddressSync,
} = require("@solana/spl-token");
const { createInitializeInstruction, pack } = require("@solana/spl-token-metadata");
const fs = require("fs");
const os = require("os");
const symbols = require("./symbols.js");

const RPC = process.env.RPC || "https://api.devnet.solana.com";
const SYMBOL = (process.argv[2] || "").toUpperCase();
const SUPPLY = Number(process.argv[3] || 1_000);

const EXTENSIONS = [
  ExtensionType.MetadataPointer,
  ExtensionType.ScaledUiAmountConfig,
  ExtensionType.PermanentDelegate,
  ExtensionType.TransferHook,
  ExtensionType.DefaultAccountState,
  ExtensionType.PausableConfig,
];

(async () => {
  const spec = symbols[SYMBOL];
  if (!spec) {
    console.error(`usage: node create-mint.js <SYMBOL> [SUPPLY]\nknown: ${Object.keys(symbols).filter((k) => k === k.toUpperCase() && !["DECIMALS", "API"].includes(k)).join(" ")}`);
    process.exit(2);
  }
  const connection = new Connection(RPC, "confirmed");
  const payer = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(fs.readFileSync(os.homedir() + "/.config/solana/id.json", "utf8")))
  );
  const mint = Keypair.generate();
  const authority = payer.publicKey;

  const metadata = {
    mint: mint.publicKey,
    name: `${SYMBOL} PreStocks (devnet mock)`,
    symbol: SYMBOL,
    uri: `https://prestocks.com/metadata/${SYMBOL.toLowerCase()}.json`,
    additionalMetadata: [],
  };
  // The account is ALLOCATED at exactly the fixed-extension length and FUNDED for the larger
  // final size. InitializeMint2 validates the account length against the extensions written so
  // far, so pre-allocating the metadata bytes makes it fail with InvalidAccountData; the metadata
  // initializer reallocs, and it can only do that if the rent is already there.
  const space = getMintLen(EXTENSIONS);
  const funded = space + 4 + pack(metadata).length;

  // Read from mainnet, not from a table. See symbols.js on why the table was removed.
  const live = await symbols.liveMultiplier(SYMBOL);
  if (live.decimals !== symbols.DECIMALS) {
    throw new Error(`${SYMBOL} is ${live.decimals} decimals on mainnet, not ${symbols.DECIMALS}`);
  }
  console.log(`symbol     ${SYMBOL}`);
  console.log(`mainnet    ${spec.mainnet}`);
  console.log(`multiplier ${live.multiplier}   (read from the mainnet mint just now)`);
  console.log(`markPrice  $${live.markPrice.toFixed(2)}   (display token)`);
  console.log(`decimals   ${symbols.DECIMALS}`);
  console.log(`mint       ${mint.publicKey.toBase58()}`);

  const ata = getAssociatedTokenAddressSync(mint.publicKey, authority, false, TOKEN_2022_PROGRAM_ID);
  const raw = BigInt(SUPPLY) * 10n ** BigInt(symbols.DECIMALS);

  // Extension initializers all run before InitializeMint2; TokenMetadata is the one that runs
  // after, because it writes into a mint that already exists.
  const tx = new Transaction().add(
    SystemProgram.createAccount({
      fromPubkey: payer.publicKey,
      newAccountPubkey: mint.publicKey,
      space,
      lamports: await connection.getMinimumBalanceForRentExemption(funded),
      programId: TOKEN_2022_PROGRAM_ID,
    }),
    createInitializeMetadataPointerInstruction(mint.publicKey, authority, mint.publicKey, TOKEN_2022_PROGRAM_ID),
    createInitializeScaledUiAmountConfigInstruction(mint.publicKey, authority, live.multiplier, TOKEN_2022_PROGRAM_ID),
    createInitializePermanentDelegateInstruction(mint.publicKey, authority, TOKEN_2022_PROGRAM_ID),
    // The all-zero program id is Token-2022's "no hook", which is what the mainnet mints carry.
    // The program refuses a mint whose hook names a real program on every path collateral leaves
    // by, so a mock with one set would be unwithdrawable.
    createInitializeTransferHookInstruction(mint.publicKey, authority, PublicKey.default, TOKEN_2022_PROGRAM_ID),
    createInitializeDefaultAccountStateInstruction(mint.publicKey, AccountState.Initialized, TOKEN_2022_PROGRAM_ID),
    createInitializePausableConfigInstruction(mint.publicKey, authority, TOKEN_2022_PROGRAM_ID),
    createInitializeMint2Instruction(mint.publicKey, symbols.DECIMALS, authority, authority, TOKEN_2022_PROGRAM_ID),
    createInitializeInstruction({
      programId: TOKEN_2022_PROGRAM_ID,
      mint: mint.publicKey,
      metadata: mint.publicKey,
      name: metadata.name,
      symbol: metadata.symbol,
      uri: metadata.uri,
      mintAuthority: authority,
      updateAuthority: authority,
    }),
    createAssociatedTokenAccountInstruction(payer.publicKey, ata, authority, mint.publicKey, TOKEN_2022_PROGRAM_ID),
    createMintToInstruction(mint.publicKey, ata, authority, raw, [], TOKEN_2022_PROGRAM_ID),
  );

  const sig = await sendAndConfirmTransaction(connection, tx, [payer, mint], { commitment: "confirmed" });

  // Read the mint back rather than trusting the transaction: the extension set is what
  // `list_collateral` will judge, so it is worth seeing.
  const info = await connection.getParsedAccountInfo(mint.publicKey);
  const parsed = info.value.data.parsed.info;
  const present = parsed.extensions.map((e) => e.extension);
  const scaled = parsed.extensions.find((e) => e.extension === "scaledUiAmountConfig").state;
  console.log(`\nextensions ${present.join(", ")}`);
  console.log(`scaled-ui  multiplier=${scaled.multiplier} new=${scaled.newMultiplier} effective_at=${scaled.newMultiplierEffectiveTimestamp}`);
  console.log(`supply     ${parsed.supply} raw = ${SUPPLY} display-adjusted by the multiplier`);
  console.log(`\n${SYMBOL}_MINT=${mint.publicKey.toBase58()}`);
  console.log(`${SYMBOL}_ATA=${ata.toBase58()}`);
  console.log(`SIG=${sig}`);
})().catch((e) => {
  console.error("FAILED:", e.message || e);
  if (e.logs) console.error(e.logs.slice(-14).join("\n"));
  process.exit(1);
});
