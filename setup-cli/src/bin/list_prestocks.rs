//! List a devnet PreStocks mock mint as `XStock` collateral priced by a Switchboard On-Demand
//! pull feed.
//!
//! Pyth cannot price these: it publishes no on-chain feed for a private-company SPV mark. See
//! `.devnet/prestocks/FINDINGS.md`. The Switchboard source needs three things the Pyth path does
//! not:
//!
//!   - `price_source: SwitchboardOnDemand`
//!   - `price_account` **pinned** to the feed. A `PriceUpdateV2` proves which feed it carries, so
//!     Pyth tolerates unpinned; a `PullFeedAccountData` proves nothing, so `validate` refuses to
//!     list without the pin.
//!   - `sb_feed_hash` equal to the feed's job hash, because the pin alone is not enough — the
//!     feed authority can rewrite `feed_hash` and repoint a pinned address at another job.
//!
//! LTV 50% / threshold 75% / bonus 10%: the xStock launch values from spec §8, not the
//! stablecoin ones. These are illiquid private-company marks, so if anything that is generous.
//!
//! Usage, after `create-mint.js` and `create-feed.js`:
//!
//!   PRESTOCKS_MINT=… PRESTOCKS_FEED=… PRESTOCKS_FEED_HASH=… cargo run --bin list_prestocks

use anchor_lang::{system_program, InstructionData, ToAccountMetas};
use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::read_keypair_file, signer::Signer,
    transaction::Transaction,
};
use std::str::FromStr;

fn hex32(s: &str) -> Result<[u8; 32], Box<dyn std::error::Error>> {
    let s = s.trim().trim_start_matches("0x");
    if s.len() != 64 {
        return Err(format!("feed hash must be 64 hex chars, got {}", s.len()).into());
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)?;
    }
    Ok(out)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let admin = read_keypair_file(std::env::var("HOME")? + "/.config/solana/id.json")
        .map_err(|e| e.to_string())?;
    let rpc = RpcClient::new_with_commitment(
        "https://api.devnet.solana.com".to_string(),
        CommitmentConfig::confirmed(),
    );
    let mint = Pubkey::from_str(&std::env::var("PRESTOCKS_MINT")?)?;
    let feed = Pubkey::from_str(&std::env::var("PRESTOCKS_FEED")?)?;
    let feed_hash = hex32(&std::env::var("PRESTOCKS_FEED_HASH")?)?;

    let pda = |s: &[&[u8]]| Pubkey::find_program_address(s, &hodl_loans::ID).0;
    let collateral = pda(&[hodl_loans::COLLATERAL_SEED, mint.as_ref()]);
    let vault = pda(&[hodl_loans::COLLATERAL_VAULT_SEED, mint.as_ref()]);

    let params = hodl_loans::CollateralParams {
        price_source: hodl_loans::PriceSource::SwitchboardOnDemand,
        // Both Pyth fields must be zero on this path: the program rejects a Switchboard asset
        // carrying a freshness bound nothing on its path would ever read.
        pyth_feed_id: [0u8; 32],
        max_price_age_seconds: 0,
        price_account: feed,
        sb_feed_hash: feed_hash,
        // 150 slots is ~60s at the 400 ms target — the same budget MAX_PRICE_AGE_SECONDS gives
        // the Pyth path, denominated in the only unit a Switchboard result carries.
        sb_max_stale_slots: 150,
        sb_min_samples: 2,
        max_conf_bps: 200,
        ltv_bps: 5_000,
        liquidation_threshold_bps: 7_500,
        liquidation_bonus_bps: 1_000,
        // A launch cap rather than u64::MAX: 100 display tokens, which at OPENAI's mark is about
        // $100k of exposure. Raisable with update_collateral_params without a redeploy.
        deposit_cap: 100 * 1_000_000_000,
        // No per-asset multiplier ceiling. The issuer's scaled-UI authority can raise the
        // multiplier, and the global MAX_MULTIPLIER still applies; a ceiling here is worth setting
        // once there is a view on how far a legitimate corporate action would move it.
        max_multiplier: 0,
    };

    // Listing creates the asset; re-running re-points it. Both take the same validated
    // `CollateralParams`, so a feed that had to be recreated (a wrong `minResponses` makes a feed
    // permanently uncrankable) is a re-run rather than a delisting.
    let already_listed = rpc.get_account(&collateral).is_ok();
    let ix = if already_listed {
        Instruction::new_with_bytes(
            hodl_loans::ID,
            &hodl_loans::instruction::UpdateCollateralParams { params }.data(),
            hodl_loans::accounts::UpdateCollateralParams {
                admin: admin.pubkey(),
                config: pda(&[hodl_loans::CONFIG_SEED]),
                collateral,
            }
            .to_account_metas(None),
        )
    } else {
        Instruction::new_with_bytes(
            hodl_loans::ID,
            &hodl_loans::instruction::ListCollateral {
                params,
                kind: hodl_loans::CollateralKind::XStock,
            }
            .data(),
            hodl_loans::accounts::ListCollateral {
                admin: admin.pubkey(),
                config: pda(&[hodl_loans::CONFIG_SEED]),
                mint,
                collateral,
                vault,
                token_program: anchor_spl::token_2022::ID,
                system_program: system_program::ID,
            }
            .to_account_metas(None),
        )
    };

    println!("  action     {}", if already_listed { "update_collateral_params" } else { "list_collateral" });
    println!("  mint       {mint}");
    println!("  feed       {feed}");
    println!("  feed_hash  {}", std::env::var("PRESTOCKS_FEED_HASH")?.trim());
    println!("  collateral {collateral}");
    println!("  vault      {vault}");

    let bh = rpc.get_latest_blockhash()?;
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&admin.pubkey()), &[&admin], bh);
    match rpc.send_and_confirm_transaction(&tx) {
        Ok(sig) => println!("\n  OK  {sig}"),
        Err(e) => {
            println!("\n  FAILED: {e}");
            return Err(e.into());
        }
    }
    Ok(())
}
