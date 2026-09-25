//! Repay a devnet loan in full, which is what moves `CreditRecord.loans_completed`.
//!
//!   BORROWER_KEYPAIR=<path> LOAN_ID=<n> cargo run --bin repay
//!
//! Needs no price accounts — `repay_loan` reads no oracle, so it works during an oracle outage or a
//! pause. With no LOAN_ID it lists the position's active loans and stops.

use anchor_lang::solana_program::instruction::AccountMeta;
use anchor_lang::{system_program, AccountDeserialize, InstructionData, ToAccountMetas};
use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::read_keypair_file, signer::Signer,
    transaction::Transaction,
};
use std::str::FromStr;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var("HOME")?;
    let admin = read_keypair_file(home.clone() + "/.config/solana/id.json").map_err(|e| e.to_string())?;
    let borrower_path = std::env::var("BORROWER_KEYPAIR")
        .unwrap_or_else(|_| home.clone() + "/.config/solana/id.json");
    let borrower = read_keypair_file(&borrower_path).map_err(|e| e.to_string())?;
    let rpc = RpcClient::new_with_commitment(
        "https://api.devnet.solana.com".to_string(),
        CommitmentConfig::confirmed(),
    );
    let cngn = Pubkey::from_str(&std::env::var("CNGN_MINT")?)?;
    let t22 = anchor_spl::token_2022::ID;
    let pda = |s: &[&[u8]]| Pubkey::find_program_address(s, &hodl_loans::ID).0;
    let owner = borrower.pubkey();
    let position = pda(&[hodl_loans::POSITION_SEED, owner.as_ref()]);
    let credit_record = pda(&[hodl_loans::constants::CREDIT_SEED, owner.as_ref()]);

    let p = hodl_loans::Position::try_deserialize(&mut &rpc.get_account_data(&position)?[..])?;
    println!("  borrower {owner}");
    println!("  active loans:");
    for loan in p.loans.iter().filter(|l| l.is_active()) {
        println!("    id {:<4} principal {:>16} original {:>16} tenure {}d",
            loan.id, loan.principal, loan.original_principal, loan.tenure_seconds / 86_400);
    }
    let Ok(loan_id) = std::env::var("LOAN_ID") else {
        println!("\n  set LOAN_ID=<id> to repay one in full");
        return Ok(());
    };
    let loan_id: u64 = loan_id.parse()?;

    let cngn_ata = anchor_spl::associated_token::get_associated_token_address_with_program_id(
        &owner, &cngn, &t22,
    );
    let before = rpc.get_token_account_balance(&cngn_ata).map(|b| b.amount.parse::<u64>().unwrap_or(0)).unwrap_or(0);
    println!("\n  repaying loan {loan_id} in full; cNGN balance {before}");

    // u64::MAX means "everything owed" — the program clamps to the balance due.
    let ix = Instruction {
        program_id: hodl_loans::ID,
        accounts: hodl_loans::accounts::RepayLoan {
            payer: owner,
            access: pda(&[hodl_loans::ACCESS_SEED, owner.as_ref()]),
            position,
            market: pda(&[hodl_loans::MARKET_SEED, cngn.as_ref()]),
            mint: cngn,
            vault: pda(&[hodl_loans::MARKET_VAULT_SEED, cngn.as_ref()]),
            payer_token: cngn_ata,
            credit_record,
            token_program: t22,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
        data: hodl_loans::instruction::RepayLoan { loan_id, amount: u64::MAX }.data(),
    };
    let _: Vec<AccountMeta> = vec![];

    let bh = rpc.get_latest_blockhash()?;
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&admin.pubkey()), &[&admin, &borrower], bh);
    match rpc.send_and_confirm_transaction(&tx) {
        Ok(sig) => {
            let after = rpc.get_token_account_balance(&cngn_ata).map(|b| b.amount.parse::<u64>().unwrap_or(0)).unwrap_or(0);
            println!("  paid {} cNGN units", before.saturating_sub(after));
            let r = hodl_loans::CreditRecord::try_deserialize(&mut &rpc.get_account_data(&credit_record)?[..])?;
            println!("\n  CREDIT RECORD  {credit_record}");
            println!("    owner            {}", r.owner);
            println!("    loans_completed  {}", r.loans_completed);
            println!("    loans_defaulted  {}", r.loans_defaulted);
            println!("\n  REPAY OK  {sig}");
        }
        Err(e) => {
            println!("\n  REPAY FAILED: {e}");
            return Err(e.into());
        }
    }
    Ok(())
}
