//! Prepare a position to borrow against: open it and deposit wSOL collateral.
//!
//! Uses the admin wallet as the borrower. It is already whitelisted and funded, which keeps
//! this example to the parts that matter; a real borrower is any whitelisted wallet.
//!
//! Idempotent — each step is skipped if its account already exists.

use anchor_lang::{InstructionData, ToAccountMetas};
use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::read_keypair_file, signer::Signer,
    transaction::Transaction,
};
use std::str::FromStr;

const WSOL: &str = "So11111111111111111111111111111111111111112";
/// 0.5 wSOL of collateral. At ~$115/SOL and 70% LTV that is ~$40 of borrowing power, far
/// more than the 1,000 cNGN (~$0.75) minimum loan needs.
const COLLATERAL_LAMPORTS: u64 = 500_000_000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let me = read_keypair_file(std::env::var("HOME")? + "/.config/solana/id.json")
        .map_err(|e| e.to_string())?;
    let rpc = RpcClient::new_with_commitment(
        "https://api.devnet.solana.com".to_string(),
        CommitmentConfig::confirmed(),
    );
    let wsol = Pubkey::from_str(WSOL)?;
    let pda = |s: &[&[u8]]| Pubkey::find_program_address(s, &hodl_loans::ID).0;
    let position = pda(&[hodl_loans::POSITION_SEED, me.pubkey().as_ref()]);
    let ata = anchor_spl::associated_token::get_associated_token_address_with_program_id(
        &me.pubkey(),
        &wsol,
        &anchor_spl::token::ID,
    );

    println!("borrower {}\nposition {}\nwSOL ata {}\n", me.pubkey(), position, ata);

    let send = |label: &str, ixs: Vec<Instruction>| -> Result<(), Box<dyn std::error::Error>> {
        let bh = rpc.get_latest_blockhash()?;
        let tx = Transaction::new_signed_with_payer(&ixs, Some(&me.pubkey()), &[&me], bh);
        match rpc.send_and_confirm_transaction(&tx) {
            Ok(s) => println!("  {label:<22} OK   {s}"),
            Err(e) => {
                println!("  {label:<22} FAIL {e}");
                return Err(e.into());
            }
        }
        Ok(())
    };

    // 1. A wSOL token account holding real wrapped SOL: create the ATA, transfer lamports
    //    into it, then sync_native so the token balance reflects them.
    let have = rpc
        .get_token_account_balance(&ata)
        .map(|b| b.amount.parse::<u64>().unwrap_or(0))
        .unwrap_or(0);
    if have < COLLATERAL_LAMPORTS {
        let mut ixs = vec![];
        // Hand-built rather than via the spl/system instruction crates: those resolve to
        // different solana-program versions than anchor-lang 1.2 here, and the resulting
        // Pubkey/Instruction types do not unify. These two are trivial enough to spell out.
        use anchor_lang::solana_program::instruction::AccountMeta;
        if rpc.get_account(&ata).is_err() {
            ixs.push(Instruction {
                program_id: anchor_spl::associated_token::ID,
                accounts: vec![
                    AccountMeta::new(me.pubkey(), true),                       // payer
                    AccountMeta::new(ata, false),                              // ata
                    AccountMeta::new_readonly(me.pubkey(), false),             // owner
                    AccountMeta::new_readonly(wsol, false),                    // mint
                    AccountMeta::new_readonly(anchor_lang::system_program::ID, false),
                    AccountMeta::new_readonly(anchor_spl::token::ID, false),
                ],
                data: vec![0], // Create
            });
        }
        ixs.push(anchor_lang::solana_program::system_instruction::transfer(
            &me.pubkey(), &ata, COLLATERAL_LAMPORTS - have,
        ));
        ixs.push(Instruction {
            program_id: anchor_spl::token::ID,
            accounts: vec![AccountMeta::new(ata, false)],
            data: vec![17], // SyncNative
        });
        send("wrap 0.5 SOL", ixs)?;
    } else {
        println!("  {:<22} already holds {} lamports — skipped", "wrap 0.5 SOL", have);
    }

    // 2. open_position
    if rpc.get_account(&position).is_err() {
        send("open_position", vec![Instruction::new_with_bytes(
            hodl_loans::ID,
            &hodl_loans::instruction::OpenPosition {}.data(),
            hodl_loans::accounts::OpenPosition {
                payer: me.pubkey(),
                owner: me.pubkey(),
                access: pda(&[hodl_loans::ACCESS_SEED, me.pubkey().as_ref()]),
                position,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
        )])?;
    } else {
        println!("  {:<22} already exists — skipped", "open_position");
    }

    // 3. deposit_collateral. Needs no price feed, so it works regardless of oracle state.
    send("deposit_collateral", vec![Instruction::new_with_bytes(
        hodl_loans::ID,
        &hodl_loans::instruction::DepositCollateral { amount: COLLATERAL_LAMPORTS }.data(),
        hodl_loans::accounts::DepositCollateral {
            owner: me.pubkey(),
            access: pda(&[hodl_loans::ACCESS_SEED, me.pubkey().as_ref()]),
            position,
            collateral: pda(&[hodl_loans::COLLATERAL_SEED, wsol.as_ref()]),
            mint: wsol,
            vault: pda(&[hodl_loans::COLLATERAL_VAULT_SEED, wsol.as_ref()]),
            owner_token: ata,
            token_program: anchor_spl::token::ID,
        }
        .to_account_metas(None),
    )])?;

    println!("\nready to borrow against {position}");
    Ok(())
}
