// The eight PreStocks tokens.
//
// **Only the mainnet mint address is recorded here.** The scaled-UI multiplier is deliberately NOT,
// because it is issuer-controlled and this file got it wrong: SPACEX was written down as 1 when it
// had been 5 since 2026-06-10, because only three of the eight mints were actually read and the
// rest were defaulted. A mock at multiplier 1 against a real asset at 5 is undervalued fivefold,
// and nothing in the pipeline would have complained.
//
// `liveMultiplier()` below reads it from mainnet instead. A hand-copied value cannot be right for
// longer than the issuer leaves it alone.
//
// Recorded for reference: those mints carry a live 100 bps TransferFeeConfig, which
// XSTOCK_COLLATERAL_EXTENSIONS refuses — see ./FINDINGS.md. The devnet mocks omit it, which is the
// one way they do not mirror the real asset.
module.exports = {
  ANDURIL:    { mainnet: "PresTj4Yc2bAR197Er7wz4UUKSfqt6FryBEdAriBoQB" },
  ANTHROPIC:  { mainnet: "Pren1FvFX6J3E4kXhJuCiAD5aDmGEb7qJRncwA8Lkhw" },
  FIGUREAI:   { mainnet: "PreZad18qfPtbxNpMtMuAuX2zVpvkEU8DnJx56faCWd" },
  KALSHI:     { mainnet: "PreLWGkkeqG1s4HEfFZSy9moCrJ7btsHuUtfcCeoRua" },
  NEURALINK:  { mainnet: "PrekqLJvJ3qVdXmBGDiexvwUTF4rLFDa6HWS4HJbw9S" },
  OPENAI:     { mainnet: "PreweJYECqtQwBtpxHL171nL2K6umo692gTm7Q3rpgF" },
  POLYMARKET: { mainnet: "Pre8AREmFPtoJFT8mQSXQLh56cwJmM7CFDRuoGBZiUP" },
  SPACEX:     { mainnet: "PreANxuXjsy2pvisWWMNB6YaJNzr7681wJJr2rHsfTh" },
};

module.exports.MAINNET_RPC = process.env.MAINNET_RPC || "https://api.mainnet-beta.solana.com";

/// `fetch` that retries on a rate limit and refuses to guess.
///
/// The public mainnet RPC rate-limits, and the first version of this called `res.json()` straight
/// on the response — so a "Too Many Requests" body surfaced as `Unexpected token 'T'`, which reads
/// like a bug in the JSON rather than a throttle. Four mints in a row hit it.
async function fetchJson(url, init, label) {
  let lastErr;
  for (let attempt = 0; attempt < 5; attempt++) {
    if (attempt) await new Promise((r) => setTimeout(r, 500 * 2 ** attempt));
    let res;
    try { res = await fetch(url, init); } catch (e) { lastErr = e; continue; }
    const text = await res.text();
    if (res.status === 429 || /too many requests/i.test(text)) {
      lastErr = new Error(`${label}: rate limited (HTTP ${res.status})`);
      continue;
    }
    if (!res.ok) throw new Error(`${label}: HTTP ${res.status} ${text.slice(0, 120)}`);
    try { return JSON.parse(text); }
    catch { throw new Error(`${label}: response was not JSON: ${text.slice(0, 120)}`); }
  }
  throw lastErr;
}

/// The symbol's live effective scaled-UI multiplier, read off the mainnet mint.
///
/// Token-2022 does NOT move `newMultiplier` into `multiplier` when its timestamp passes — the
/// consumer compares block time and picks, exactly as `read_xstock_multiplier` does on chain. A
/// reader that took `multiplier` alone would be 48% low on OPENAI and 80% low on SPACEX.
///
/// Cross-checked against the API's own arithmetic: `ui_supply / raw_supply` must equal the
/// multiplier, because the API quotes display units. A mismatch throws rather than proceeds — two
/// independent sources disagreeing about a valuation input is not something to paper over.
module.exports.liveMultiplier = async (symbol) => {
  const spec = module.exports[symbol];
  if (!spec) throw new Error(`unknown symbol ${symbol}`);
  const body = JSON.stringify({
    jsonrpc: "2.0", id: 1, method: "getAccountInfo",
    params: [spec.mainnet, { encoding: "jsonParsed" }],
  });
  const rpc = await fetchJson(
    module.exports.MAINNET_RPC,
    { method: "POST", headers: { "Content-Type": "application/json" }, body },
    `mainnet mint for ${symbol}`
  );
  const info = rpc.result?.value?.data?.parsed?.info;
  if (!info) throw new Error(`could not read mainnet mint for ${symbol}`);
  const ext = info.extensions.find((e) => e.extension === "scaledUiAmountConfig");
  if (!ext) throw new Error(`${symbol} has no ScaledUiAmount extension`);
  const now = Math.floor(Date.now() / 1000);
  const effectiveAt = Number(ext.state.newMultiplierEffectiveTimestamp);
  const multiplier = Number(now >= effectiveAt ? ext.state.newMultiplier : ext.state.multiplier);
  if (!Number.isFinite(multiplier) || multiplier <= 0) {
    throw new Error(`${symbol} multiplier is unusable: ${multiplier}`);
  }

  const api = await fetchJson(module.exports.API, undefined, "PreStocks API");
  const row = api.find((a) => a.symbol === symbol);
  if (!row) throw new Error(`${symbol} is not in the PreStocks API`);
  const implied = row.supply / (Number(info.supply) / 10 ** info.decimals);
  if (Math.abs(implied - multiplier) / multiplier > 1e-6) {
    throw new Error(
      `${symbol}: mint says multiplier ${multiplier}, API supply ratio implies ${implied.toFixed(7)}`
    );
  }
  return { multiplier, markPrice: row.markPrice, decimals: info.decimals, effectiveAt };
};

// Every PreStocks mint is 9 decimals.
module.exports.DECIMALS = 9;
module.exports.API = "https://prestocks.com/api/prestocks";

/// The Switchboard job that prices one symbol.
///
/// The filter, not an array index: the API returns a flat array with no per-symbol endpoint
/// (`?symbol=` is ignored, `/api/prestocks/openai` 404s), and the array is ordered
/// alphabetically, so a new listing would shift every index below it. `$[5].markPrice` resolves
/// today and would silently price OPENAI as POLYMARKET the day ANTHROPIC gets a sibling.
module.exports.jobFor = (symbol) => ({
  tasks: [
    { httpTask: { url: module.exports.API } },
    { jsonParseTask: { path: `$[?(@.symbol=='${symbol}')].markPrice` } },
  ],
});
