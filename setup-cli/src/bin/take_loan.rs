//! Borrow cNGN against the wSOL collateral deposited by `borrower_setup`.
//!
//! This is the instruction that exercises BOTH oracles, and it is the one that fails if
//! either is stale or mis-pinned. The remaining accounts are `(CollateralAsset, PriceUpdateV2)`
//! pairs — one per collateral slot holding a non-zero amount.
//!
//! Reads the Pyth price account from `PYTH_ACCOUNT` because it is not a fixed address in the
//! general case: the collateral is listed UNPINNED, so any receiver-owned account carrying the
//! SOL/USD feed id and younger than max_price_age_seconds (60) is accepted. In production that
//! is an ephemeral account your own transaction posts; here it can also be Pyth's sponsored
//! devnet account during one of its refresh windows.
//!
//! Prints the position's loan state before and after, so the effect is visible rather than
//! inferred from a signature.

use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use anchor_lang::solana_program::instruction::AccountMeta;
use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::read_keypair_file, signer::Signer,
    transaction::Transaction,
};
use std::str::FromStr;

const WSOL: &str = "So11111111111111111111111111111111111111112";
const ONE_CNGN: u64 = 1_000_000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let me = read_keypair_file(std::env::var("HOME")? + "/.config/solana/id.json")
        .map_err(|e| e.to_string())?;
    let rpc = RpcClient::new_with_commitment(
        "https://api.devnet.solana.com".to_string(),
        CommitmentConfig::confirmed(),
    );
    let cngn = Pubkey::from_str(&std::env::var("CNGN_MINT")?)?;
    let ngn_feed = Pubkey::from_str(&std::env::var("NGN_FEED")?)?;
    let pyth = Pubkey::from_str(&std::env::var("PYTH_ACCOUNT")?)?;
    let amount: u64 = std::env::var("AMOUNT_CNGN").unwrap_or_else(|_| "5000".into()).parse::<u64>()? * ONE_CNGN;
    let tenure: i64 = 30 * 86_400;

    let wsol = Pubkey::from_str(WSOL)?;
    let pda = |s: &[&[u8]]| Pubkey::find_program_address(s, &hodl_loans::ID).0;
    let position_key = pda(&[hodl_loans::POSITION_SEED, me.pubkey().as_ref()]);
    let market = pda(&[hodl_loans::MARKET_SEED, cngn.as_ref()]);
    let cngn_ata = anchor_spl::associated_token::get_associated_token_address_with_program_id(
        &me.pubkey(), &cngn, &anchor_spl::token_2022::ID,
    );

    let loans_before = active_loans(&rpc, &position_key)?;
    let cngn_before = token_balance(&rpc, &cngn_ata);
    println!("  borrowing {} cNGN against 0.5 wSOL", amount / ONE_CNGN);
    println!("  before: active loans = {loans_before}, cNGN balance = {cngn_before}");

    let ix = Instruction {
        program_id: hodl_loans::ID,
        accounts: {
            let mut a = hodl_loans::accounts::TakeLoan {
                owner: me.pubkey(),
                access: pda(&[hodl_loans::ACCESS_SEED, me.pubkey().as_ref()]),
                config: pda(&[hodl_loans::CONFIG_SEED]),
                promo_vault: Some(pda(&[hodl_loans::PROMO_VAULT_SEED, market.as_ref()])),
                position: position_key,
                market,
                mint: cngn,
                vault: pda(&[hodl_loans::MARKET_VAULT_SEED, cngn.as_ref()]),
                owner_token: cngn_ata,
                ngn_feed,
                token_program: anchor_spl::token_2022::ID,
            }
            .to_account_metas(None);
            // One (CollateralAsset, PriceUpdateV2) pair per funded slot. wSOL is `Standard`,
            // so no mint account is appended — an XStock would need one for its multiplier.
            a.push(AccountMeta::new_readonly(pda(&[hodl_loans::COLLATERAL_SEED, wsol.as_ref()]), false));
            a.push(AccountMeta::new_readonly(pyth, false));
            a
        },
        data: hodl_loans::instruction::TakeLoan { amount, tenure_seconds: tenure }.data(),
    };

    let bh = rpc.get_latest_blockhash()?;
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&me.pubkey()), &[&me], bh);
    match rpc.send_and_confirm_transaction(&tx) {
        Ok(sig) => {
            let loans_after = active_loans(&rpc, &position_key)?;
            let cngn_after = token_balance(&rpc, &cngn_ata);
            println!("  after : active loans = {loans_after}, cNGN balance = {cngn_after}");
            println!("  delta : +{} cNGN units, +{} loan(s)",
                cngn_after.saturating_sub(cngn_before), loans_after - loans_before);
            println!("\n  TAKE_LOAN OK  {sig}");
        }
        Err(e) => {
            println!("\n  TAKE_LOAN FAILED: {e}");
            return Err(e.into());
        }
    }
    Ok(())
}

fn active_loans(rpc: &RpcClient, position: &Pubkey) -> Result<usize, Box<dyn std::error::Error>> {
    let data = rpc.get_account_data(position)?;
    let p = hodl_loans::Position::try_deserialize(&mut &data[..])?;
    Ok(p.loans.iter().filter(|l| l.principal > 0).count())
}

fn token_balance(rpc: &RpcClient, ata: &Pubkey) -> u64 {
    rpc.get_token_account_balance(ata)
        .map(|b| b.amount.parse().unwrap_or(0))
        .unwrap_or(0)
}
