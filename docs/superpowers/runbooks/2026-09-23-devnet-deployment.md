# Devnet deployment runbook — `hodl_loans`

**Status: not executed.** This document is a runbook for a person to run, with their
explicit go-ahead, not a script this agent ran. A deployment is outward-facing, spends
real SOL on a public cluster, and changes state other people can observe. Producing
this document involved no `solana program deploy`, no `solana airdrop`, and no
`solana config set` (that command is global and would affect every project on this
machine, not just this one) — see [§13 Confirmation of scope](#13-confirmation-of-scope).

Everything below was checked against the repository tree, the local toolchain, and
vendored dependency sources on **2026-09-23**, at commit `7ff0157` on
`plan-8-coverage-fuzzing-devnet`. This environment has no outbound network access
(crates.io returns 403; there is no reachable RPC endpoint). Every step that needs a
live cluster is marked **[UNVERIFIED]** below and is written from documented Solana /
Anchor / Switchboard / Pyth behaviour, not from having been run. Nothing in this
document should be read as "tested end-to-end" — only the pieces explicitly marked
**[VERIFIED]** were.

---

## Contents

1. [Two hard gates](#1-two-hard-gates)
2. [Verified facts](#2-verified-facts)
3. [Unverified steps and why](#3-unverified-steps-and-why)
4. [Tooling on this machine](#4-tooling-on-this-machine)
5. [What devnet does not have](#5-what-devnet-does-not-have)
6. [Roles and keys](#6-roles-and-keys)
7. [Program size and rent estimate](#7-program-size-and-rent-estimate)
8. [Deployment procedure](#8-deployment-procedure)
9. [Initialization order](#9-initialization-order)
10. [Smoke test and its diagnostic signature](#10-smoke-test-and-its-diagnostic-signature)
11. [Known operational limits](#11-known-operational-limits)
12. [Troubleshooting / rollback](#12-troubleshooting--rollback)
13. [Confirmation of scope](#13-confirmation-of-scope)

---

## 1. Two hard gates

Both of these fail closed — the program either won't deploy correctly or will silently
misbehave. Resolve both before touching a live cluster.

### Gate A — `declare_id!` does not match the local deploy keypair

**[VERIFIED, disagree.]**

```
declare_id!() in programs/hodl_loans/src/lib.rs:20   → J9sKAhm2EhdJQ3bHeP2KUCxqZ4cYdBc65C3RDr4JjGEd
solana-keygen pubkey target/deploy/hodl_loans-keypair.json → CmDBvi4ZiBDND1XzEwKFuH3kontC5Lokd2faXBxEgmci
```

The Solana upgradeable loader derives the program's on-chain address from whichever
keypair signs the deploy transaction, not from the `declare_id!` string — that string
is compiled into the binary as the address the *program itself* believes it lives at.
Anchor's `#[account(seeds = …)]` PDA derivations, and every `find_program_address` call
in `tests/common/mod.rs` (`config_pda`, `market_pda`, `position_pda`, …), use
`declare_id!`'s value. Deploy with `hodl_loans-keypair.json` today and the live program
sits at `CmDBv...` while every PDA the program computes internally (and every PDA a
client computes against `hodl_loans::ID`) is derived from `J9sK...` — a different seed
namespace entirely. Every instruction that touches a PDA fails, and the failure surface
looks like unrelated `ConstraintSeeds` / seed-mismatch errors, not an address problem.

**This must be resolved before any deploy. The options, in the order they fit the
common case:**

| Option | What it means | When to pick it |
|---|---|---|
| **A. Mint a fresh keypair, update `declare_id!`** | `solana-keygen new -o target/deploy/hodl_loans-keypair.json --force`, then copy its pubkey into `declare_id!("...")` in `src/lib.rs`, rebuild. | Default choice for a devnet rehearsal — nobody has ever deployed at `J9sK...`, and a devnet program id has no reason to match a hypothetical future mainnet id. |
| **B. Locate the private key for `J9sKAhm2EhdJQ3bHeP2KUCxqZ4cYdBc65C3RDr4JjGEd`** | If this pubkey was deliberately reserved (e.g. a vanity address minted earlier and stored outside this working tree — a secrets vault, a teammate's machine), replace `target/deploy/hodl_loans-keypair.json` with the real keypair file. | Only if `J9sK...` is a real reservation someone can produce the private key for. A pubkey alone cannot be reverse-engineered into a keypair. |
| **C. Regrind a vanity keypair matching the existing prefix** | `solana-keygen grind --starts-with J9sK:1` (or similar) — probabilistically expensive, and will not reproduce the *exact* existing pubkey, only one sharing a prefix. | Rarely worth it; only if the exact string matters for branding and nobody has key B. |
| **D. Keep `declare_id!` as-is, deploy with the mismatched keypair anyway** | Not offered as a real option — the PDA-seed failure above is not cosmetic. | Never. |

This runbook does not pick A vs B for you — that is a decision about who holds what
keys. It only asserts that A or B must happen, and that a devnet rehearsal is the
lowest-cost place to default to **A** since nothing has been deployed at `J9sK...` yet.

### Gate B — the binary must be built with `--features devnet`

**[VERIFIED — the mechanism; not verified against a live Switchboard devnet feed.]**

`programs/hodl_loans/src/constants.rs` selects the Switchboard On-Demand program id
that `oracle/switchboard.rs::read_ngn_price` requires as the NGN feed account's
*owner*, at compile time:

```rust
#[cfg(not(feature = "devnet"))]
pub const SWITCHBOARD_ON_DEMAND_PID: Pubkey = switchboard_on_demand::ON_DEMAND_MAINNET_PID; // SBondMDrcV3K4kxZR1HNVT7osZxAHVHgYXL5Ze1oMUv
#[cfg(feature = "devnet")]
pub const SWITCHBOARD_ON_DEMAND_PID: Pubkey = switchboard_on_demand::ON_DEMAND_DEVNET_PID;  // Aio4gaXjXzJNVLtzwtNVmSqGKpANtXhybbkhtAC94ji2
```

(Pubkeys read from the vendored crate source at
`~/.cargo/registry/src/.../switchboard-on-demand-0.13.0/src/program_id.rs` —
**[VERIFIED against the dependency source in this tree's cache]**, not against a live
cluster.)

A binary built without `--features devnet` and deployed to devnet reads a real devnet
Switchboard feed — which is owned by the *devnet* On-Demand program — and rejects it,
because the binary only accepts the *mainnet* program id as owner. The failure mode is
the worst kind: `deposit`, `deposit_collateral`, and `repay_loan` never call
`read_ngn_price` at all, so they keep working. Only `take_loan`, `liquidate`,
`write_off_loan`, and a priced `revoke_promo` call it — the program looks alive and is
unusable. See [§10](#10-smoke-test-and-its-diagnostic-signature) for the exact
diagnostic.

**Build for devnet with:**

```bash
cargo build-sbf --tools-version v1.52 -- --features devnet --locked
```

Never `cargo build-sbf` (default features) for a devnet target.

---

## 2. Verified facts

Everything in this section was run in this sandbox, on this tree, at commit `7ff0157`.

| # | Fact | Method | Result |
|---|---|---|---|
| 1 | The two builds produce different binaries | `cargo build-sbf --tools-version v1.52 -- --locked` then `... -- --features devnet --locked`; `shasum -a 256` both `.so` files | **Confirmed different.** Default: `3ace34b9f08254204c493ad719fc10c82be641a5e842224caf3eaa080d32d23f`. Devnet: `93d63c04bf9965aa56f5fedbd743e14f67ef1b6dcd962a259854af98e5744156`. Both **1,148,376 bytes** (identical size — the feature flips an embedded constant, not the binary's gross length). `cmp` reports the first differing byte at offset 1909. |
| 2 | `declare_id!` vs. deploy keypair | `grep declare_id! src/lib.rs`; `solana-keygen pubkey target/deploy/hodl_loans-keypair.json` | **Disagree** — see Gate A above. |
| 3 | Tooling versions | `solana --version`, `anchor --version`, `cargo --version`, `rustc --version`, `ls Anchor.toml` | `solana-cli 2.1.0`; `anchor-cli 0.31.1` (against `anchor-lang = "1.2.0"` in `Cargo.toml` — a materially newer CLI than the lang crate the program actually builds against); `cargo`/`rustc 1.91.0` (host toolchain; `cargo build-sbf` uses its own vendored platform-tools, not the host rustc); **no `Anchor.toml` in the repo** → `anchor deploy` cannot be used here, only `solana program deploy`. |
| 4 | Program size | `ls -la target/deploy/hodl_loans.so` | **1,148,376 bytes** (≈ 1,121.5 KiB, ≈ 1.10 MiB) for both feature variants. |
| 5 | Reproducible build | `cargo build-sbf --tools-version v1.52 -- --locked` in a no-network sandbox | Succeeds fully offline — `Cargo.lock` pins resolve against the already-populated `~/.cargo/registry` cache. A clean CI runner would still need one network round-trip to populate that cache; this sandbox already had it from Tasks 1–8. |
| 6 | Test suite unaffected by this task | `./scripts/test.sh` (builds default SBF binary, then `cargo test -p hodl_loans`) | **278 passed, 0 failed** — 60 unit (`unittests src/lib.rs`) + 218 across 24 integration files (summed per-file `test result: ok.` lines: access 6, admin 6, budget 9, campaign 4, collateral 12, harness 3, invariants 5, liquidation 13, liquidity 20, loans 19, market 8, position 6, promo_cap 4, promo_forfeit 11, promo_health 6, promo_lifecycle 15, promo_redeem 15, promo_vault 8, repay 6, reserve 2, sweep 3, withdraw 8, write_off 8, xstocks 21 = 218). Matches the count stated at the top of this task. This task adds no tests, so the count is unchanged by writing this runbook. |
| 7 | Clippy clean, both feature sets | `cargo clippy -p hodl_loans --all-targets -- -D warnings`; same `--features devnet` | Both clean, zero warnings. |
| 8 | `PriceAccountMismatch` error code | `programs/hodl_loans/src/errors.rs` — 8th variant (index 7) in `#[error_code] pub enum HodlError`, and the doc comment "codes are 6000 + position" | Anchor custom error `6007` (`0x1777`). This is the number to grep client/validator logs for during the smoke test. |
| 9 | Switchboard / Pyth program ids | Read directly from the vendored crate sources this build actually links (`switchboard-on-demand-0.13.0/src/program_id.rs`, `pyth-solana-receiver-sdk-2.0.0/src/lib.rs`) | Switchboard On-Demand mainnet: `SBondMDrcV3K4kxZR1HNVT7osZxAHVHgYXL5Ze1oMUv`. Switchboard On-Demand devnet: `Aio4gaXjXzJNVLtzwtNVmSqGKpANtXhybbkhtAC94ji2`. Pyth Solana Receiver (this crate's default, non-`pro-compatible` build — matches this program's `Cargo.toml`, which requests no extra pyth features): `rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ`, used as `pyth_solana_receiver_sdk::ID` on **both** mainnet and devnet per the crate's own `cfg_if!` (there is no devnet-specific branch for it) — **not independently confirmed against a live devnet RPC in this environment.** |
| 10 | `initialize` requires the program's upgrade authority | `programs/hodl_loans/src/instructions/admin/initialize.rs` | `Initialize::authority` must equal `program_data.upgrade_authority_address`, checked via `program.programdata_address()? == Some(program_data.key())` and `program_data.upgrade_authority_address == Some(authority.key())`, both `@ HodlError::Unauthorized`. Whoever calls `solana program deploy` becomes the upgrade authority by default and is the only wallet that can call `initialize`. |

---

## 3. Unverified steps and why

Every item below needs a live devnet RPC endpoint, a devnet faucet, or a real
Switchboard/Pyth crank — none reachable from this sandbox. They are written from
documented CLI/SDK behaviour, not from having been executed here.

- **`solana program deploy` itself** — not run. No `.so` was uploaded to any cluster.
- **`solana airdrop`** — not run. No SOL was requested on any cluster.
- **`solana config set`** — not run, and would not be run by this task regardless: it
  is a global, machine-wide setting, not scoped to this repository.
- **`solana rent <bytes>`** (the CLI's own rent calculator) — attempted in this sandbox
  and failed: it dials `SysvarRent111111111111111111111111111111111` over the
  currently-configured RPC URL (`http://localhost:8899` in this environment, which has
  nothing listening), rather than computing offline. [§7](#7-program-size-and-rent-estimate)
  below uses the documented default `Rent` formula
  (`lamports_per_byte_year = 3480`, `exemption_threshold = 2.0`,
  `ACCOUNT_STORAGE_OVERHEAD = 128` bytes) by hand instead. Re-run
  `solana rent <bytes> --url https://api.devnet.solana.com` before funding, in case the
  live Rent sysvar has since diverged from these compiled-in defaults.
- **Whether a Switchboard On-Demand NGN/USD feed under the devnet program id actually
  exists or can be created without additional off-chain infrastructure (a Switchboard
  crank/oracle operator)** — Switchboard On-Demand feeds are typically created via their
  `sb-on-demand` CLI/SDK against a specific job definition and then require an active
  crank to keep publishing; none of that tooling or its accounts exist in this repo or
  were reachable to test.
  Whether `pyth_solana_receiver_sdk::ID` is actually deployed and resolvable on devnet.
  Whether the commonly-cited devnet USDC mint address is still current. All three:
  **look up / confirm at deploy time**, don't trust a hardcoded address carried in this
  document.
- **Actual transaction fees for the buffer-write phase of `solana program deploy`** —
  the CLI chunks the ~1.1 MiB binary into many small transactions; the per-chunk size
  and therefore the transaction count depends on the CLI version and current cluster
  parameters, and was not measured here.
- **`anchor idl init`/`anchor idl upgrade` against a deployed program** — not attempted;
  this repo's absence of `Anchor.toml` means the operator would run these as raw
  `anchor-cli` invocations against an externally-tracked program id, which is outside
  this runbook's scope (uploading an IDL is optional and does not affect program
  behaviour).

---

## 4. Tooling on this machine

```
solana-cli 2.1.0 (src:c1080de4; feat:3176011155, client:Agave)
anchor-cli 0.31.1
cargo 1.91.0 / rustc 1.91.0   (host toolchain — not what compiles the SBF binary)
```

`cargo build-sbf` platform-tools available locally: `v1.43`, `v1.52`, `v1.57`, `v2.3.2`.
This task and Plan 8 pin `v1.52` — use that exact tools version for parity with every
`.so` hash recorded in this document and in `scripts/test.sh`.

**No `Anchor.toml` exists in this repository.** `anchor-lang`/`anchor-spl` are pinned
to `1.2.0` in `programs/hodl_loans/Cargo.toml`, materially older than the installed
`anchor-cli` (`0.31.1`). Anchor's CLI workflow (`anchor build` / `anchor deploy` /
its own IDL pipeline) assumes an `Anchor.toml` and a matching `anchor-lang` version;
neither holds here. **Deploy with `solana program deploy`, not `anchor deploy`.**

---

## 5. What devnet does not have

This is the larger part of standing the program up on devnet — none of it is the
`solana program deploy` step itself, and all of it is **[UNVERIFIED]** (no RPC access
to confirm current addresses/state):

1. **A Switchboard On-Demand NGN/USD feed under the devnet program id
   (`Aio4gaXjXzJNVLtzwtNVmSqGKpANtXhybbkhtAC94ji2`).** Nothing today publishes an
   NGN/USD result on devnet under that program. Standing one up needs Switchboard's
   On-Demand feed-creation tooling (a job definition plus an active crank/oracle
   operator willing to keep publishing) — this is infrastructure outside this
   repository, not a config value to set.
2. **Devnet Pyth `PriceUpdateV2` accounts for each collateral asset.** The Pyth
   Solana Receiver program itself (`rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ`) is
   documented as deployed on both mainnet-beta and devnet, but the *price update*
   accounts this program reads (`oracle/pyth.rs`) are per-feed and per-publish-time —
   they must be posted fresh via Pyth's pull-oracle flow (Hermes price feed IDs +
   `pyth_solana_receiver_sdk`'s `postUpdate`/`postUpdateAtomic`) for every collateral
   asset the market lists, before any health check that reads them can succeed.
3. **A cNGN mint.** cNGN does not exist on devnet. One must be created matching the
   shape the program expects for a "cNGN-like" market mint — Token-2022 with the
   `PermanentDelegate` and `MetadataPointer` extensions, in this exact order (mirrors
   `MintKind::CngnLike` in `tests/common/mod.rs::create_mint`, the only place this
   shape is defined in the repo):
   `system_instruction::create_account` (owner = Token-2022 program) →
   `initialize_permanent_delegate` → `metadata_pointer::initialize` →
   `initialize_mint2`. Decimals: the test harness uses 6; match whatever the real
   cNGN's decimals are intended to be.
4. **Collateral mints.** Devnet USDC exists as a well-known SPL Token mint, but
   **look up its current address at deploy time rather than trusting one hardcoded
   here** — this document has no way to confirm it live. Any collateral beyond USDC
   needs its own devnet mint plus a devnet Pyth feed for it.
5. **Backed xStocks do not exist on devnet at all.** `CollateralKind::XStock` and its
   Token-2022 extension stack (`ScaledUiAmount`, `Pausable`, `TransferHook`,
   `DefaultAccountState`, `ConfidentialTransferMint` — see `MintKind::XStock` in
   `tests/common/mod.rs`) is exercised only by the LiteSVM suite (`xstocks.rs`, 21
   tests). A devnet rehearsal should skip xStock collateral entirely and list only
   `CollateralKind::Standard` assets.

---

## 6. Roles and keys

`initialize` takes an `InitializeArgs { guardian, whitelister, promo_signer, treasury }`
plus the transaction's fee-payer/signer (`authority`), which becomes `Config.admin` and
**must be the program's upgrade authority** (Gate — see §2, item 10). That is five
roles in total, mapped from source:

| Role | Set by | Used for |
|---|---|---|
| `authority` → `Config.admin` | Signs `initialize`; must match `program_data.upgrade_authority_address` | Highest privilege: market admin, collateral admin, role rotation (`admin/roles.rs`) |
| `guardian` | `InitializeArgs.guardian` | Co-authorized (alongside admin) for pausing markets/collateral (`market_admin.rs`, `collateral_admin.rs`) |
| `whitelister` | `InitializeArgs.whitelister` | Co-authorized (alongside admin) to call `whitelist` (`access_control.rs`) |
| `promo_signer` | `InitializeArgs.promo_signer` | Signs promo-redemption messages verified in `promos/redeem.rs` |
| `treasury` | `InitializeArgs.treasury` | Destination for swept excess funds (`admin/sweep.rs`) |

For a devnet rehearsal it is acceptable to use one keypair for convenience across
guardian/whitelister/promo_signer/treasury, but keep the mapping explicit in whatever
deploy script or notes you keep — do not silently reuse `admin` for all five without
recording that choice, since `set_guardian`/`set_whitelister`/`set_promo_signer`/
`set_treasury` (`admin/roles.rs`) exist specifically to let you separate them later.

The upgrade authority (the deploying wallet, by default) additionally needs enough
devnet SOL to cover deployment (see §7) — request it from a devnet faucet
**[UNVERIFIED — not attempted here]**, not from this runbook.

---

## 7. Program size and rent estimate

**[VERIFIED size; UNVERIFIED rent — computed by hand, not read from a live Rent sysvar.]**

Binary size: **1,148,376 bytes** (both feature variants — see §2, item 1).

`solana program deploy` (BPF Loader Upgradeable) creates two permanent accounts:

- A **Program** account: fixed 36 bytes (4-byte state tag + 32-byte pointer to the
  ProgramData account).
- A **ProgramData** account: 45-byte header (4-byte tag + 8-byte slot + 1-byte
  `Option` tag + 32-byte upgrade-authority pubkey) + the program's byte length.

Using the documented default `Rent` parameters
(`lamports_per_byte_year = 3480`, `exemption_threshold = 2.0`,
`ACCOUNT_STORAGE_OVERHEAD = 128`):

```
rent(data_len) = (data_len + 128) * 3480 * 2   lamports

Program account:      data_len = 36
  rent = (36 + 128) * 6960            =     1,141,440 lamports  (≈ 0.00114 SOL)

ProgramData account:  data_len = 45 + 1,148,376 = 1,148,421
  rent = (1,148,421 + 128) * 6960      = 7,993,901,040 lamports  (≈ 7.99390 SOL)

Total permanent rent-exempt minimum  ≈ 7,995,042,480 lamports  (≈ 7.99504 SOL)
```

On top of that, the deploy process temporarily funds a **buffer account** the same
size as the ProgramData account (≈ 7.99 SOL of rent) while it uploads the binary in
chunks; modern `solana program deploy` folds that buffer's lamports forward into the
final ProgramData account rather than requiring it twice, but the wallet still needs
to hold the buffer's rent plus per-chunk transaction fees (5,000 lamports per
signature; chunk count depends on CLI version, unmeasured here) at some point during
the upload. **Practical guidance: fund the deploying wallet with generous headroom —
at least ~9–10 devnet SOL — before starting, and request more from a devnet faucet if
the CLI reports insufficient funds mid-upload.** Re-run `solana rent 1148421 --url
https://api.devnet.solana.com` (ProgramData) and `solana rent 36 --url
https://api.devnet.solana.com` (Program) before funding, to catch any drift from the
compiled-in defaults used above.

---

## 8. Deployment procedure

**Not run in this environment. Written for a human to execute, after Gate A and Gate B
are resolved.** Every command below hits network or spends devnet SOL and is therefore
**[UNVERIFIED]**.

```bash
# 0. Resolve Gate A first (pick one option from §1) and rebuild if declare_id! changed.

# 1. Build the devnet binary (Gate B).
cargo build-sbf --tools-version v1.52 -- --features devnet --locked

# 2. Confirm the two builds still differ before deploying the devnet one (repeat §2 item 1
#    locally — cheap, and catches a forgotten --features devnet immediately).
shasum -a 256 target/deploy/hodl_loans.so
cargo build-sbf --tools-version v1.52 -- --locked
shasum -a 256 target/deploy/hodl_loans.so   # must differ from the devnet hash above
cargo build-sbf --tools-version v1.52 -- --features devnet --locked   # rebuild devnet last

# 3. Fund the deploying wallet on devnet (a devnet faucet, or `solana airdrop` run by a
#    person — never by an automated task). Target ~9-10 SOL of headroom (§7).

# 4. Deploy. This wallet becomes the upgrade authority and therefore the only signer
#    that can later call `initialize`.
solana program deploy \
  --program-id target/deploy/hodl_loans-keypair.json \
  --url https://api.devnet.solana.com \
  --keypair <deploying-wallet-keypair.json> \
  target/deploy/hodl_loans.so

# 5. Confirm the program landed at the address `declare_id!` expects.
solana program show <program-id> --url https://api.devnet.solana.com
```

If step 5's reported program id does not match `declare_id!` in
`programs/hodl_loans/src/lib.rs`, stop — Gate A was not actually resolved (the keypair
used in step 4 does not match what was compiled into the binary).

---

## 9. Initialization order

This mirrors the tested harness sequence in `tests/common/mod.rs`
(`Env::initialized()` → `Env::with_cngn_market()` → `Env::loan_ready()`), which is the
only ordering this program's test suite has actually exercised. Substituting a
different order is unverified even on devnet dependencies that do exist. Each numbered
step is a real on-chain instruction unless marked "off-chain / provisioning".

| # | Step | Instruction / action | Signer(s) | Mirrors |
|---|---|---|---|---|
| 1 | Deploy | `solana program deploy` | deploying wallet (becomes upgrade authority) | §8 |
| 2 | Initialize config | `initialize` (`InitializeArgs{guardian, whitelister, promo_signer, treasury}`) | `authority` = upgrade authority | `Env::initialized()` |
| 3 | Create the cNGN mint | off-chain provisioning — Token-2022, `PermanentDelegate` + `MetadataPointer` (§5 item 3) | mint authority (yours to choose) | `Env::with_cngn_market()`'s `create_mint(MintKind::CngnLike, …)` |
| 4 | Create the market | `create_market` (params: interest/penalty/reserve rates, `ngn_feed` = the real devnet Switchboard feed address once it exists, `ngn_max_stale_slots`, `ngn_min_samples`, `ngn_max_spread_bps`, etc.) | `admin` | `create_market_ix` |
| 5 | Create the promo vault | `create_promo_vault` — every market needs one; `take_loan` can expire promo even when none is funded | `admin` | `create_promo_vault_ix`, called immediately after step 4 in every harness path |
| 6 | List collateral | `list_collateral` (kind = `Standard` only — no xStock on devnet, §5 item 5) for each collateral mint, with its Pyth feed id / price account wired in `CollateralParams` | `admin` | `list_spl_collateral` |
| 7 | Provision a live NGN price | off-chain — Switchboard feed must actually be publishing (§5 item 1) | n/a | `Env::set_ngn_price` (test-only account write; devnet has no such shortcut) |
| 8 | Provision live collateral prices | off-chain — post a fresh Pyth `PriceUpdateV2` for each listed collateral (§5 item 2) | whoever runs the Hermes → `postUpdate` relay | `Env::set_pyth_price` (also test-only) |
| 9 | Whitelist the lender wallet | `whitelist` | `whitelister` or `admin` | `Env::new_lender` calls `whitelist` |
| 10 | Lender deposits liquidity | `deposit_liquidity` | lender wallet (needs a real cNGN balance — mint it to them, or transfer from wherever the cNGN mint authority holds supply) | `Env::deposit` |
| 11 | Whitelist the borrower wallet | `whitelist` | `whitelister` or `admin` | `Env::new_borrower` |
| 12 | Open the borrower's position | `open_position` | borrower (payer may be sponsored) | `Env::new_borrower` |
| 13 | Borrower deposits collateral | `deposit_collateral` | borrower (needs a real collateral-token balance) | `Env::deposit_collateral` |
| 14 | **Smoke test: take a loan** | `take_loan` | borrower (`admin` co-signs in the test harness only because it sponsors fees there; not a protocol requirement) | `Env::take_loan` — see §10 |

Steps 9 and 11 need only `whitelist`; only the **borrower** additionally needs
`open_position` (step 12) — lenders have no position account (`deposit_liquidity`'s
`Accounts` struct references no `Position`). Do not add an unnecessary
`open_position` call for the lender.

---

## 10. Smoke test and its diagnostic signature

The last step of §9 is the smoke test: call `take_loan` for the borrower set up in
steps 9–13. This is the load-bearing check because `take_loan` is the first instruction
in the initialization sequence that calls `oracle/switchboard.rs::read_ngn_price`
(via `has_one = ngn_feed @ HodlError::PriceAccountMismatch` on the `TakeLoan` accounts,
then the owner check inside `read_ngn_price` itself) — nothing before it in §9 does.

**The signature of a mainnet binary deployed to devnet:**

> `deposit_collateral` (step 13) succeeds, and `take_loan` (step 14) fails with
> Anchor custom program error `0x1777` (decimal `6007`, `HodlError::PriceAccountMismatch`).

Why this specific pattern is diagnostic: `deposit`/`deposit_collateral`/`repay_loan`
never read the NGN feed at all — they only touch collateral-side Pyth prices and token
balances — so they keep succeeding regardless of which Switchboard program id the
binary was compiled against. `take_loan` is the first call in the sequence whose
`market.ngn_feed` account is checked against
`constants::SWITCHBOARD_ON_DEMAND_PID` (mainnet id, if `--features devnet` was
forgotten) rather than the real devnet feed's actual owner (the devnet On-Demand
program). `liquidate`, `write_off_loan`, and a priced `revoke_promo` share the same
symptom for the same reason.

**Remedy:** confirm the deployed binary was built with `--features devnet`
(§1 Gate B); if not, rebuild and redeploy (`solana program deploy` again with the same
`--program-id` upgrades the existing program in place — no new address, no re-running
§9), then retry `take_loan`.

A clean `take_loan` success (loan recorded, cNGN transferred to the borrower) is the
positive signal that both gates were actually resolved and the devnet dependencies in
§5 are wired correctly.

---

## 11. Known operational limits

Measured during this plan's coverage/fuzzing work (`tests/budget.rs`), relevant to
operating on devnet (or mainnet) where the default 200,000 CU / 1,232-byte legacy
transaction budgets are real constraints, unlike LiteSVM which does not enforce the
packet-size limit at all:

- **`set_promo_cap` costs 2,634 CU per listed asset**, deterministically (it touches no
  position PDA). At 74 listed assets that's `10,023 + 2,634 × 72 = 199,671 CU` — fits
  under the 200,000 default compute budget. At 75 assets,
  `10,023 + 2,634 × 73 = 202,305 CU` — does not fit. **Any deployment expecting to list
  75+ assets must send `set_promo_cap` with an explicit
  `ComputeBudgetInstruction::set_compute_unit_limit`.**
- **`liquidate`'s binding constraint is transaction size, not compute.** Measured CU
  stays under 120,000–130,000 even at the program's structural maximum (8 collateral
  slots, 10 active/overdue loans, or an all-xStock position) — comfortably inside the
  200,000 default and nowhere near the 1.4M ceiling. The legacy transaction's 1,232-byte
  (`PACKET_DATA_SIZE`) limit is the actual wall: measured 1,185 bytes at 8 Standard
  collateral slots (fits), 1,218 bytes at 7 Standard + 1 xStock slot (fits, 14-byte
  margin), and 1,251 bytes at 6 Standard + 2 xStock slots (does **not** fit — a v0
  transaction with an address lookup table is required past that point). Since Backed
  xStocks do not exist on devnet (§5 item 5), the two-xStock overflow case cannot arise
  in a devnet rehearsal, but a devnet position filling all 8 slots with Standard
  collateral is still worth watching if the collateral list grows further, since each
  slot's `(CollateralAsset, PriceUpdateV2)` pair — or the triple for an xStock slot —
  is what consumes the budget.

---

## 12. Troubleshooting / rollback

- **Wrong-cluster binary deployed (Gate B missed), Gate A already resolved correctly:**
  rebuild with `--features devnet` and run `solana program deploy` again with the same
  `--program-id` / keypair. The upgradeable loader replaces the program in place; no
  new address, no re-running §9's initialization sequence (`Config`/`Market`/`Position`
  PDAs are untouched by a program upgrade).
- **Gate A was not actually resolved (wrong keypair used for the initial deploy):**
  there is no in-place fix. Every PDA derived so far used the wrong program id as a
  seed input; the program must be redeployed at the correct address and the
  entire §9 sequence re-run from `initialize`. This is exactly why Gate A is listed
  first and marked hard: it is cheap to fix before anything is initialized and
  expensive after.
- **`take_loan` fails with `PriceAccountMismatch` (`0x1777`) after confirming
  `--features devnet` was used:** check that `market.ngn_feed` (set in `create_market`,
  step 4 of §9) is actually the live devnet Switchboard feed's address, and that the
  feed account's owner is genuinely the devnet On-Demand program
  (`Aio4gaXjXzJNVLtzwtNVmSqGKpANtXhybbkhtAC94ji2`) — a stale or misconfigured feed
  address produces the identical error code for a different reason than Gate B.
- **Insufficient funds mid-deploy:** the buffer-account rent (§7) is the largest single
  cost; request more devnet SOL from a faucet and resume — `solana program deploy`
  supports resuming an interrupted buffer upload.

---

## 13. Confirmation of scope

This document was produced without:

- running `solana program deploy` (no `.so` was uploaded to any cluster, devnet or
  otherwise),
- running `solana airdrop` (no SOL was requested on any cluster),
- running `solana config set` or otherwise changing this machine's Solana CLI
  configuration (it is global, not scoped to this repository, and was left untouched).

Commands that were run, all local and read-only with respect to any network:
`grep`/`cat`/`sed` reads of source and test files; `solana-keygen pubkey` against the
already-present `target/deploy/hodl_loans-keypair.json`; `solana --version` /
`anchor --version` / `cargo --version` / `rustc --version`; two
`cargo build-sbf` invocations (default and `--features devnet`) plus `shasum -a 256`
and `cmp` on their outputs, followed by a third `cargo build-sbf` (default features) to
leave `target/deploy/hodl_loans.so` as this task found it (`target/` is gitignored and
none of this touches tracked files); `./scripts/test.sh`; and
`cargo clippy -p hodl_loans --all-targets [-- --features devnet]`. `solana rent` was
attempted and failed for lack of a reachable RPC endpoint, confirmed by its own error
message rather than assumed.
