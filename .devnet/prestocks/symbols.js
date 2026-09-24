// The eight PreStocks tokens and the mainnet facts a devnet mock has to reproduce.
//
// `multiplier` is the mint's LIVE effective scaled-UI multiplier on mainnet, read 2026-09-24 —
// `newMultiplier` where `newMultiplierEffectiveTimestamp` has passed, else `multiplier`. The
// PreStocks API quotes `markPrice` per **display** token, so the multiplier and the price have to
// travel together or a mock is worth the wrong amount.
//
// `mainnet` is recorded for reference only. Those mints carry a live 100 bps TransferFeeConfig,
// which `XSTOCK_COLLATERAL_EXTENSIONS` refuses — see ../prestocks/FINDINGS.md. The devnet mocks
// below deliberately omit it, which is the one way they do not mirror the real asset.
module.exports = {
  ANDURIL:    { mainnet: "PresTj4Yc2bAR197Er7wz4UUKSfqt6FryBEdAriBoQB", multiplier: 1 },
  ANTHROPIC:  { mainnet: "Pren1FvFX6J3E4kXhJuCiAD5aDmGEb7qJRncwA8Lkhw", multiplier: 1 },
  FIGUREAI:   { mainnet: "PreZad18qfPtbxNpMtMuAuX2zVpvkEU8DnJx56faCWd", multiplier: 1 },
  KALSHI:     { mainnet: "PreLWGkkeqG1s4HEfFZSy9moCrJ7btsHuUtfcCeoRua", multiplier: 1 },
  NEURALINK:  { mainnet: "PrekqLJvJ3qVdXmBGDiexvwUTF4rLFDa6HWS4HJbw9S", multiplier: 1 },
  OPENAI:     { mainnet: "PreweJYECqtQwBtpxHL171nL2K6umo692gTm7Q3rpgF", multiplier: 1.4861347 },
  POLYMARKET: { mainnet: "Pre8AREmFPtoJFT8mQSXQLh56cwJmM7CFDRuoGBZiUP", multiplier: 1 },
  SPACEX:     { mainnet: "PreANxuXjsy2pvisWWMNB6YaJNzr7681wJJr2rHsfTh", multiplier: 1 },
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
