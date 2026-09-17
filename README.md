# lendbit-solana

HODL fixed-term cNGN loans on Solana: an Anchor program (`hodl_loans`) with a lender pool,
collateral positions, fixed-term loans, liquidation and promo balances.

Design: `docs/superpowers/specs/2026-09-17-solana-fixed-loans-design.md`

## Prerequisites

- Rust 1.89 or newer
- Solana CLI 3.1.x (Agave) — provides `cargo build-sbf`; builds use platform tools v1.52 (downloaded on first build if missing)

## Build and test

```bash
./scripts/test.sh            # cargo build-sbf, then unit + LiteSVM integration tests
./scripts/test.sh --lib      # unit tests only (still builds the program first)
```

Integration tests load `target/deploy/hodl_loans.so`, so the program must be built before `cargo test`.

## Program ID

`target/deploy/hodl_loans-keypair.json` holds the program keypair and is not committed.
Back it up before any devnet or mainnet deploy: losing it changes the program ID.
