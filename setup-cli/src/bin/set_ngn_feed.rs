//! Point the cNGN market's `ngn_feed` at the live Switchboard NGN/USD feed.
//!
//! `create_market` stored a placeholder because no NGN feed existed on devnet; one now does
//! (see `.devnet/sb/FINDINGS.md`). `update_market_params` takes the whole MarketParams struct,
//! so every other field is re-sent unchanged.
use anchor_lang::{InstructionData, ToAccountMetas};
use solana_commitment_config::CommitmentConfig;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::read_keypair_file, signer::Signer,
    transaction::Transaction,
};
use std::str::FromStr;

const ONE: u64 = 1_000_000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let admin = read_keypair_file(std::env::var("HOME")? + "/.config/solana/id.json")
        .map_err(|e| e.to_string())?;
    let rpc = RpcClient::new_with_commitment(
        "https://api.devnet.solana.com".to_string(),
        CommitmentConfig::confirmed(),
    );
    let cngn = Pubkey::from_str(&std::env::var("CNGN_MINT")?)?;
    let ngn_feed = Pubkey::from_str(&std::env::var("NGN_FEED")?)?;
    let pda = |s: &[&[u8]]| Pubkey::find_program_address(s, &hodl_loans::ID).0;
    let config = pda(&[hodl_loans::CONFIG_SEED]);
    let market = pda(&[hodl_loans::MARKET_SEED, cngn.as_ref()]);

    let params = hodl_loans::MarketParams {
        interest_rate_bps: 1_500,
        penalty_rate_bps: 500,
        reserve_factor_bps: 1_000,
        max_utilization_bps: 9_000,
        min_loan_amount: 1_000 * ONE,
        max_tenure_seconds: 365 * 86_400,
        bad_debt_dust_usd: 5_000_000_000_000,
        ngn_feed,
        ngn_max_stale_slots: 150,
        ngn_min_samples: 3,
        ngn_max_spread_bps: 200,
        promo_inactivity_seconds: 90 * 86_400,
        max_promo_per_position: 50_000 * ONE,
    };
    let ix = Instruction::new_with_bytes(
        hodl_loans::ID,
        &hodl_loans::instruction::UpdateMarketParams { params }.data(),
        hodl_loans::accounts::UpdateMarketParams {
            admin: admin.pubkey(),
            config,
            market,
        }
        .to_account_metas(None),
    );
    let bh = rpc.get_latest_blockhash()?;
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&admin.pubkey()), &[&admin], bh);
    let sig = rpc.send_and_confirm_transaction(&tx)?;
    println!("  ngn_feed set to {ngn_feed}\n  sig {sig}");
    Ok(())
}
