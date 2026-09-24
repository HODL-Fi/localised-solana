//! Mint cNGN to the admin and deposit it as market liquidity.
//!
//! Exists because the devnet pool was too small for a borrow limit to ever bind: the LTV limit on
//! two mock OPENAI shares is ~2.0M cNGN while a 994,000 cNGN pool refuses anything past ~894,600
//! on the utilization cap. A bracket test against that pool measures the cap, not the valuation —
//! the same trap the LiteSVM bisection hit.
//!
//!   AMOUNT_CNGN=4000000 cargo run --bin fund_market
//!
//! Devnet only in spirit: it mints, which only works because the devnet cNGN is our own mock and
//! the admin holds its mint authority.

use anchor_lang::solana_program::instruction::AccountMeta;
use anchor_lang::{system_program, InstructionData, ToAccountMetas};
use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::read_keypair_file, signer::Signer,
    transaction::Transaction,
};
use std::str::FromStr;

const ONE_CNGN: u64 = 1_000_000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let admin = read_keypair_file(std::env::var("HOME")? + "/.config/solana/id.json")
        .map_err(|e| e.to_string())?;
    let rpc = RpcClient::new_with_commitment(
        "https://api.devnet.solana.com".to_string(),
        CommitmentConfig::confirmed(),
    );
    let cngn = Pubkey::from_str(&std::env::var("CNGN_MINT")?)?;
    let amount: u64 =
        std::env::var("AMOUNT_CNGN").unwrap_or_else(|_| "4000000".into()).parse::<u64>()? * ONE_CNGN;

    let t22 = anchor_spl::token_2022::ID;
    let pda = |s: &[&[u8]]| Pubkey::find_program_address(s, &hodl_loans::ID).0;
    let market = pda(&[hodl_loans::MARKET_SEED, cngn.as_ref()]);
    let vault = pda(&[hodl_loans::MARKET_VAULT_SEED, cngn.as_ref()]);
    let admin_cngn = anchor_spl::associated_token::get_associated_token_address_with_program_id(
        &admin.pubkey(), &cngn, &t22,
    );

    let before = rpc
        .get_token_account_balance(&vault)
        .map(|b| b.amount.parse::<u64>().unwrap_or(0))
        .unwrap_or(0);
    println!("  vault before {} cNGN", before / ONE_CNGN);

    // MintTo is instruction 7 followed by a little-endian u64. Hand-built because the spl crates
    // here resolve to a different solana-program than anchor-lang 1.2 and the types do not unify.
    let mut mint_data = vec![7u8];
    mint_data.extend_from_slice(&amount.to_le_bytes());
    let ixs = vec![
        Instruction {
            program_id: t22,
            accounts: vec![
                AccountMeta::new(cngn, false),
                AccountMeta::new(admin_cngn, false),
                AccountMeta::new_readonly(admin.pubkey(), true),
            ],
            data: mint_data,
        },
        Instruction::new_with_bytes(
            hodl_loans::ID,
            &hodl_loans::instruction::DepositLiquidity { amount }.data(),
            hodl_loans::accounts::DepositLiquidity {
                payer: admin.pubkey(),
                owner: admin.pubkey(),
                access: pda(&[hodl_loans::ACCESS_SEED, admin.pubkey().as_ref()]),
                market,
                mint: cngn,
                vault,
                owner_token: admin_cngn,
                lender: pda(&[hodl_loans::LENDER_SEED, market.as_ref(), admin.pubkey().as_ref()]),
                token_program: t22,
                system_program: system_program::ID,
            }
            .to_account_metas(None),
        ),
    ];

    let bh = rpc.get_latest_blockhash()?;
    let tx = Transaction::new_signed_with_payer(&ixs, Some(&admin.pubkey()), &[&admin], bh);
    let sig = rpc.send_and_confirm_transaction(&tx)?;
    let after = rpc.get_token_account_balance(&vault)?.amount.parse::<u64>()?;
    println!("  vault after  {} cNGN  (+{})", after / ONE_CNGN, (after - before) / ONE_CNGN);
    println!("\n  FUND_MARKET OK  {sig}");
    Ok(())
}
