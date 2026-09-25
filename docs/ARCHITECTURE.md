# Architecture

`hodl_loans` is a Solana program for **fixed-term stablecoin loans against tokenised-equity
collateral**. A borrower pledges tokenised stocks, draws cNGN for a fixed tenure at a fixed rate, and
accrues an on-chain credit history as they repay.

This document covers the on-chain design. For addresses and how to run it, see the
[README](../README.md).

---

## 1. Model

Three parties, no intermediary holding funds:

**Lenders** deposit cNGN into a per-market pool and receive shares. Shares use a virtual offset, so
the first deposit cannot be manipulated to distort later depositors' entitlements. Withdrawals are
bounded by cash on hand minus the protocol reserve — a lender cannot withdraw principal that is
currently lent out.

**Borrowers** open a position, deposit collateral, and take loans. Principal, interest rate, tenure
and penalty rate are **fixed at origination** and the loan keeps those terms for its whole life, even
if the market's parameters change afterwards. A position holds up to **8 collateral assets** and **10
concurrent loans**.

**Liquidators** are permissionless. Any wallet may liquidate an unhealthy position, repaying part of
its debt in exchange for collateral plus a bonus. No whitelist, so a liquidation bot needs no
onboarding.

Being overdue is not on its own grounds for liquidation — only an *unhealthy* position is
liquidatable. That separation matters: a borrower who is late but well collateralised pays a penalty
rate, not a seizure.

## 2. Valuation and health

Every priced instruction values the **whole position**, not just the asset being touched, because
health decides whether the action is permitted at all.

```
collateral value  = Σ  amount × multiplier × (price − confidence)
borrow limit      = Σ  value × ltv_bps            (+ capped promo credit)
liquidation line  = Σ  value × liquidation_threshold_bps
```

Prices are read at `price − confidence` for collateral and `price + confidence` for debt, so
uncertainty always counts against the protocol's exposure rather than for it. The confidence term
rounds **up** while the price rounds **down** — the only direction that is conservative on both sides
of the comparison.

All arithmetic is checked. There are no bare casts on value paths: a conversion that would truncate
returns `MathOverflow` instead.

## 3. Two oracle sources, and why

Collateral is priced by **either** Pyth or Switchboard On-Demand, named per asset in
`CollateralAsset.price_source`. cNGN is always priced by Switchboard.

| | mechanism | freshness bound |
|---|---|---|
| Pyth | pull `PriceUpdateV2`, posted in the same transaction | 60 seconds |
| Switchboard On-Demand | `PullFeedAccountData`, oracle-signed aggregate | 150 slots (~60 s) |

A single source would have been simpler. It was not sufficient:

- **Pyth publishes no on-chain feed for a private-company valuation.** For the tokenised
  private-company equities this protocol accepts, its catalogue covers a minority of the names, and
  those have no sponsored push account on either cluster.
- **Sponsored equity feeds do not meet a 60-second bound.** A sampled US-equity feed was over a month
  stale. The cause is structural rather than operational: equities stop trading at the close, so no
  amount of publishing effort makes an exchange-hours instrument continuously fresh.
- **Continuously-marked assets fit the ceiling that exchange-hours instruments cannot.** An asset with
  a continuous mark, read through a Switchboard job over its issuer's endpoint, is fresh whenever it
  is queried.

Both readers converge on the same internal `UsdPrice`, and the valuation path is identical afterwards
— a test bisects the maximum borrow under each source and asserts they agree **to the raw unit**, so
the choice of oracle cannot change what collateral is worth.

### Binding a Switchboard feed to an asset

A Pyth update carries a verified feed id, so the price proves which feed it came from. A Switchboard
feed account proves nothing of the kind, so two constraints replace that proof:

1. **The account address is pinned.** Listing an asset without a pin is rejected.
2. **The job is pinned.** The asset stores the feed's `feed_hash`, checked on every read.

The second exists because the first is insufficient. A pull feed's authority can rewrite its
`feed_hash`, repointing a pinned address at an entirely different job — the address, the owning
program and the account discriminator would all still check out. Binding the job is what turns that
into a failed read rather than silently repriced collateral.

## 4. Tokenised equity as collateral

Tokenised equities are Token-2022 mints carrying a **scaled-UI multiplier**: corporate actions such as
splits and dividend reinvestment change a factor on the mint rather than reissuing balances. A holder's
economic position is `raw_balance × multiplier`, and the quoted price is per *display* token, so the
two must travel together.

The program reads the multiplier from the mint on every valuation, and reads it correctly: Token-2022
does **not** migrate a scheduled multiplier into the active field when its timestamp passes, so a
consumer must compare block time and choose. A reader taking the active field alone would undervalue
a split asset by the whole factor.

Two guards bound what the issuer's multiplier authority can do:

- a global arithmetic ceiling, above which a price read fails
- an optional per-asset ceiling, above which the asset simply **stops lending borrowing power** rather
  than failing

The second is deliberately not a hard rejection. Refusing the price would seal the position against
liquidation and write-off as well, trading a remote economic risk for a likely liveness failure.
Withholding only *new* exposure bounds what a scaled-UI authority can conjure while every exit path
keeps working at the true multiplier.

### Mint-extension policy

Token-2022 extensions are allowlisted, and the policy is **asymmetric by direction**:

- **On entry** (listing, deposit) the full allowlist applies. Refusing only declines new business, so
  the check is strict. It is re-run on every deposit rather than trusted from listing time, because an
  issuer can enable a capability after an asset is listed.
- **On exit** (withdraw, liquidate, sweep) only one clause survives: the transfer hook must name no
  program. On the way out, a failing check can only trap collateral — the token program is already the
  authority on whether a transfer is legal.

That asymmetry is the difference between a policy that protects the protocol and one that locks users
out of their own assets during an issuer action.

## 5. Credit records

A `CreditRecord` PDA per borrower, derived from the **wallet address alone**:

```
loans_completed   principal reached zero through repayment
loans_defaulted   principal reached zero as a default
```

Derivation from the wallet is what makes the history durable — a borrower cannot shed it by abandoning
a position and opening another, because the address is the identity. Nothing in the program decrements
or resets either counter.

**Counters, not a score.** A score is a weighting of these facts — how much a 30-day lateness should
cost, how fast it should decay — and those are judgements that will be revised. Freezing them into a
program would require an upgrade to change a number. Two events therefore carry the richer signal for
an off-chain scoring function to weigh:

```
CreditRepaymentRecorded   principal, term_seconds, days_late, penalty_paid, loans_completed
CreditDefaultRecorded     principal, term_seconds, days_late, collateral_seized, loans_defaulted
```

Each carries the counter value *after* its increment, so an indexer never needs to read the account
back. Lateness lives only in the events: a loan repaid late still counts as completed, because
grading lateness in a counter would be scoring by another route.

The default counter is written where a default actually concludes. A liquidation is bounded by the
collateral it can seize and usually leaves principal behind, so the administrative write-off is what
clears a defaulted loan — and it is the path that carries the counter as a required account.

## 6. Constraints the design is built around

**Transaction size, not compute, binds liquidation.** At 8 collateral slots a liquidation measures
close to the legacy packet limit, and an all-tokenised-equity position exceeds it, because each such
slot contributes a third account. Liquidators must use versioned transactions with address lookup
tables for large positions. This is documented rather than worked around: the limit is real and a
client that ignores it will fail at the worst moment.

Consequently the credit-record accounts on the liquidation path are **optional**. Requiring them
pushed a full-size liquidation past the packet limit, and a liquidation that cannot be packed is an
underwater position that cannot be closed — a solvency failure, strictly worse than a counter that did
not move. The liquidation event is emitted either way, so nothing is lost for off-chain accounting.

**Oracle freshness must be structural, not raced.** Both price sources are pull-based, so the correct
client shape puts the update instructions *ahead of the borrow in the same transaction*. Refreshing in
a prior transaction works most of the time and fails intermittently under load, which is the worst
class of production bug. Past two feeds it cannot work at all — the crank sequence takes longer than
the freshness window.

**The collateral list has a hard ceiling.** One administrative instruction must name every listed asset
in a single transaction, so the number of listable assets is bounded by the accounts a transaction can
lock. The bound is enforced at listing time, where it is a cheap refusal, rather than discovered later
when the instruction becomes unusable.

## 7. Testing

311 tests: unit tests over the math, price scaling, state layouts and validation rules, plus
integration tests against an in-process Solana VM covering every instruction, every error variant, and
the interactions between features.

Three kinds of test carry more weight than coverage:

- **Layout pins.** Account sizes are asserted. New fields must come out of reserved padding rather
  than extend a struct, or every account already on chain becomes too small to deserialize — a failure
  with no error until something touches one.
- **Equivalence properties.** The oracle-agnostic valuation is asserted by measuring the maximum
  borrow under each source and comparing, rather than by checking each against a figure derived by
  hand.
- **Budget ceilings.** Compute and transaction size are measured and asserted as ranges, so a change
  that quietly doubles the cost of liquidation fails a test instead of surfacing as a liquidation that
  will not land.

## 8. Current limits

Stated because they bound what the program does today.

- **One tokenised-equity asset per borrow.** The freshness window is shorter than the time it takes to
  crank three or more feeds in separate transactions. Bundling the updates into the borrow transaction
  removes the limit; the account layout for multi-asset positions is already implemented and tested.
- **Assets carrying a transfer fee are refused, correctly.** Deposit accounting credits the amount it
  transfers, so a fee-bearing mint would credit a position more than the vault received, socialising
  the shortfall across depositors. Supporting such assets requires fee-aware accounting on every path
  that moves collateral.
- **Issuer trust is real and unmitigated.** A tokenised-equity issuer typically holds permanent
  delegate, freeze and pause authority over its mint. The delegate can move collateral out of custody;
  a pause blocks every transfer, including liquidation of a position that is already underwater. No
  on-chain design removes this; it is a counterparty choice.
- **Not externally audited.** The test suite, asserted error surface and reviewed diff are real
  evidence. They are not an audit.
