//! Read a borrower's `CreditRecord` off devnet.
//!
//!   BORROWER=<pubkey> cargo run --bin credit_record
//!
//! Prints the PDA so it can be opened in an explorer, which is the demo shot: the account exists
//! only once something has closed a loan, and `loans_completed` is what a repayment moves.

use anchor_lang::AccountDeserialize;
use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{pubkey::Pubkey, signature::read_keypair_file, signer::Signer};
use std::str::FromStr;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rpc = RpcClient::new_with_commitment(
        "https://api.devnet.solana.com".to_string(),
        CommitmentConfig::confirmed(),
    );
    let borrower = match std::env::var("BORROWER") {
        Ok(b) => Pubkey::from_str(&b)?,
        Err(_) => read_keypair_file(std::env::var("HOME")? + "/.config/solana/id.json")
            .map_err(|e| e.to_string())?
            .pubkey(),
    };
    let (pda, _) =
        Pubkey::find_program_address(&[hodl_loans::constants::CREDIT_SEED, borrower.as_ref()], &hodl_loans::ID);

    println!("  borrower       {borrower}");
    println!("  credit record  {pda}");
    println!("  explorer       https://explorer.solana.com/address/{pda}?cluster=devnet");
    match rpc.get_account_data(&pda) {
        Err(_) => println!("\n  no record yet — nothing has closed a loan for this wallet"),
        Ok(data) => {
            let r = hodl_loans::CreditRecord::try_deserialize(&mut &data[..])?;
            println!("\n  version          {}", r.version);
            println!("  owner            {}", r.owner);
            println!("  loans_completed  {}", r.loans_completed);
            println!("  loans_defaulted  {}", r.loans_defaulted);
            println!("  account size     {} bytes", data.len());
            assert_eq!(r.owner, borrower, "record owner must be the borrower");
        }
    }
    Ok(())
}
