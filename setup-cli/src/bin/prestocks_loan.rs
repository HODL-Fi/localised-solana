//! Borrow cNGN against a Switchboard-priced PreStocks mock, end to end.
//!
//! Uses a **dedicated borrower** (`.devnet/prestocks/borrower.json`, created on first run) rather
//! than the admin wallet, because the admin's position already holds wSOL. A position holding two
//! assets needs every one of their price accounts fresh in the same transaction — here that would
//! be Pyth SOL/USD *and* the PreStocks feed *and* the NGN feed. Isolating the borrower keeps the
//! demo about the one thing it is demonstrating.
//!
//! Every step is idempotent, so a failed borrow can be retried without redoing the setup.
//!
//! The remaining accounts are **three**, in this order, because the asset is an `XStock`:
//!
//!   1. `CollateralAsset` PDA          ["collateral", mint]
//!   2. the Switchboard pull feed      NOT a Pyth account — this asset's price_source says so
//!   3. the mint                       its scaled-UI multiplier lives there
//!
//! Crank the feed immediately before running this. `sb_max_stale_slots` is 150 slots (~60s), so
//! the window is tight. In production the pull instructions belong AHEAD of take_loan in the SAME
//! transaction, which makes freshness structural instead of a race.

use anchor_lang::solana_program::instruction::AccountMeta;
use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::read_keypair_file, signature::Keypair,
    signer::Signer, transaction::Transaction,
};
use std::str::FromStr;

const ONE_CNGN: u64 = 1_000_000;
/// PreStocks mints are 9 decimals.
const ONE_SHARE: u64 = 1_000_000_000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var("HOME")?;
    let admin = read_keypair_file(home.clone() + "/.config/solana/id.json").map_err(|e| e.to_string())?;
    let rpc = RpcClient::new_with_commitment(
        "https://api.devnet.solana.com".to_string(),
        CommitmentConfig::confirmed(),
    );

    // One or more assets, comma-separated and positionally paired. A multi-asset position is the
    // case worth exercising: the remaining accounts must follow the POSITION's slot order, not the
    // order given here, and every feed has to be fresh in the same transaction.
    let mints: Vec<Pubkey> = std::env::var("PRESTOCKS_MINT")?
        .split(',').map(|m| Pubkey::from_str(m.trim())).collect::<Result<_, _>>()?;
    let feeds: Vec<Pubkey> = std::env::var("PRESTOCKS_FEED")?
        .split(',').map(|f| Pubkey::from_str(f.trim())).collect::<Result<_, _>>()?;
    if mints.len() != feeds.len() {
        return Err(format!("{} mints but {} feeds", mints.len(), feeds.len()).into());
    }
    let feed_of: std::collections::HashMap<Pubkey, Pubkey> =
        mints.iter().copied().zip(feeds.iter().copied()).collect();
    let cngn = Pubkey::from_str(&std::env::var("CNGN_MINT")?)?;
    let shares: u64 = std::env::var("SHARES").unwrap_or_else(|_| "2".into()).parse()?;
    let amount: u64 = std::env::var("AMOUNT_CNGN").unwrap_or_else(|_| "1000".into()).parse::<u64>()? * ONE_CNGN;
    let tenure: i64 = 30 * 86_400;

    // A dedicated borrower, persisted so repeat runs reuse the same position.
    let path = home.clone() + "/work/hodl/lendbit-solana/.devnet/prestocks/borrower.json";
    let borrower = match read_keypair_file(&path) {
        Ok(k) => k,
        Err(_) => {
            let k = Keypair::new();
            std::fs::write(&path, serde_json_bytes(&k))?;
            println!("  created borrower keypair {path}");
            k
        }
    };

    let pda = |s: &[&[u8]]| Pubkey::find_program_address(s, &hodl_loans::ID).0;
    let t22 = anchor_spl::token_2022::ID;
    let ata = |owner: &Pubkey, m: &Pubkey| {
        anchor_spl::associated_token::get_associated_token_address_with_program_id(owner, m, &t22)
    };
    let position = pda(&[hodl_loans::POSITION_SEED, borrower.pubkey().as_ref()]);
    let access = pda(&[hodl_loans::ACCESS_SEED, borrower.pubkey().as_ref()]);
    let cngn_ata = ata(&borrower.pubkey(), &cngn);

    println!("  borrower   {}", borrower.pubkey());
    for (m, f) in mints.iter().zip(feeds.iter()) {
        println!("  asset      {m}  <- {f}");
    }
    println!("  position   {position}");

    let send = |label: &str, ixs: Vec<Instruction>, signers: Vec<&Keypair>| -> Result<(), Box<dyn std::error::Error>> {
        let bh = rpc.get_latest_blockhash()?;
        let tx = Transaction::new_signed_with_payer(&ixs, Some(&admin.pubkey()), &signers, bh);
        match rpc.send_and_confirm_transaction(&tx) {
            Ok(sig) => { println!("  {label:<24} ok   {sig}"); Ok(()) }
            Err(e) => { println!("  {label:<24} FAILED {e}"); Err(e.into()) }
        }
    };

    // 1. Fund the borrower for rent and fees.
    let lamports = rpc.get_balance(&borrower.pubkey())?;
    if lamports < 20_000_000 {
        send("fund borrower", vec![anchor_lang::solana_program::system_instruction::transfer(
            &admin.pubkey(), &borrower.pubkey(), 30_000_000 - lamports,
        )], vec![&admin])?;
    } else {
        println!("  {:<24} skipped (has {lamports} lamports)", "fund borrower");
    }

    // 2. Whitelist it. Signed by the whitelister, which on devnet is the admin.
    if rpc.get_account(&access).is_err() {
        send("whitelist", vec![Instruction::new_with_bytes(
            hodl_loans::ID,
            &hodl_loans::instruction::Whitelist { wallet: borrower.pubkey() }.data(),
            hodl_loans::accounts::Whitelist {
                signer: admin.pubkey(),
                config: pda(&[hodl_loans::CONFIG_SEED]),
                access,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
        )], vec![&admin])?;
    } else {
        println!("  {:<24} skipped (already whitelisted)", "whitelist");
    }

    // 3. Token accounts, and mint the borrower some shares. Hand-built: the spl/system
    // instruction crates here resolve to a different solana-program than anchor-lang 1.2, and the
    // Instruction types do not unify. These three are trivial enough to spell out.
    let create_ata = |m: &Pubkey, a: &Pubkey| Instruction {
        program_id: anchor_spl::associated_token::ID,
        accounts: vec![
            AccountMeta::new(admin.pubkey(), true),
            AccountMeta::new(*a, false),
            AccountMeta::new_readonly(borrower.pubkey(), false),
            AccountMeta::new_readonly(*m, false),
            AccountMeta::new_readonly(anchor_lang::system_program::ID, false),
            AccountMeta::new_readonly(t22, false),
        ],
        data: vec![1], // CreateIdempotent
    };
    let mut ixs = vec![create_ata(&cngn, &cngn_ata)];
    for m in &mints {
        let a = ata(&borrower.pubkey(), m);
        ixs.push(create_ata(m, &a));
        if token_balance(&rpc, &a) < shares * ONE_SHARE {
            let mut data = vec![7u8]; // MintTo
            data.extend_from_slice(&(shares * ONE_SHARE).to_le_bytes());
            ixs.push(Instruction {
                program_id: t22,
                accounts: vec![
                    AccountMeta::new(*m, false),
                    AccountMeta::new(a, false),
                    AccountMeta::new_readonly(admin.pubkey(), true),
                ],
                data,
            });
        }
    }
    send("token accounts + mint", ixs, vec![&admin])?;

    // 4. open_position
    if rpc.get_account(&position).is_err() {
        send("open_position", vec![Instruction::new_with_bytes(
            hodl_loans::ID,
            &hodl_loans::instruction::OpenPosition {}.data(),
            hodl_loans::accounts::OpenPosition {
                payer: admin.pubkey(),
                owner: borrower.pubkey(),
                access,
                position,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
        )], vec![&admin, &borrower])?;
    } else {
        println!("  {:<24} skipped (already open)", "open_position");
    }

    // 5. deposit_collateral, one per asset. Reads no price, so it works whatever the oracles
    // are doing.
    for m in &mints {
        let held = position_amount(&rpc, &position, m)?;
        if held < shares * ONE_SHARE {
            send("deposit_collateral", vec![Instruction::new_with_bytes(
                hodl_loans::ID,
                &hodl_loans::instruction::DepositCollateral { amount: shares * ONE_SHARE - held }.data(),
                hodl_loans::accounts::DepositCollateral {
                    owner: borrower.pubkey(),
                    access,
                    position,
                    collateral: pda(&[hodl_loans::COLLATERAL_SEED, m.as_ref()]),
                    mint: *m,
                    vault: pda(&[hodl_loans::COLLATERAL_VAULT_SEED, m.as_ref()]),
                    owner_token: ata(&borrower.pubkey(), m),
                    token_program: t22,
                }
                .to_account_metas(None),
            )], vec![&admin, &borrower])?;
        } else {
            println!("  {:<24} skipped ({m} holds {held})", "deposit_collateral");
        }
    }

    // 6. take_loan. Three remaining accounts, and the middle one is the Switchboard feed.
    let market = pda(&[hodl_loans::MARKET_SEED, cngn.as_ref()]);
    let before = token_balance(&rpc, &cngn_ata);
    println!("\n  borrowing {} cNGN against {shares} share(s) of each of {} asset(s)",
        amount / ONE_CNGN, mints.len());
    println!("  cNGN before {before}");

    let ix = Instruction {
        program_id: hodl_loans::ID,
        accounts: {
            let mut a = hodl_loans::accounts::TakeLoan {
                owner: borrower.pubkey(),
                access,
                config: pda(&[hodl_loans::CONFIG_SEED]),
                promo_vault: Some(pda(&[hodl_loans::PROMO_VAULT_SEED, market.as_ref()])),
                position,
                market,
                mint: cngn,
                vault: pda(&[hodl_loans::MARKET_VAULT_SEED, cngn.as_ref()]),
                owner_token: cngn_ata,
                ngn_feed: Pubkey::from_str(&std::env::var("NGN_FEED")?)?,
                token_program: t22,
            }
            .to_account_metas(None);
            // Slot order, read back from the position — NOT the order the env var listed them
            // in. The program walks its own slots and advances a cursor; a mismatched order
            // fails with PriceAccountMismatch rather than silently mispricing.
            for slot_mint in used_slots(&rpc, &position)? {
                let feed = *feed_of.get(&slot_mint).ok_or_else(|| {
                    format!("position holds {slot_mint} but no feed was given for it")
                })?;
                a.push(AccountMeta::new_readonly(pda(&[hodl_loans::COLLATERAL_SEED, slot_mint.as_ref()]), false));
                a.push(AccountMeta::new_readonly(feed, false));
                a.push(AccountMeta::new_readonly(slot_mint, false));
            }
            a
        },
        data: hodl_loans::instruction::TakeLoan { amount, tenure_seconds: tenure }.data(),
    };
    let bh = rpc.get_latest_blockhash()?;
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&admin.pubkey()), &[&admin, &borrower], bh);
    match rpc.send_and_confirm_transaction(&tx) {
        Ok(sig) => {
            let after = token_balance(&rpc, &cngn_ata);
            println!("  cNGN after  {after}   (+{})", after.saturating_sub(before));
            println!("\n  TAKE_LOAN OK  {sig}");
        }
        Err(e) => {
            println!("\n  TAKE_LOAN FAILED: {e}");
            return Err(e.into());
        }
    }
    Ok(())
}

fn token_balance(rpc: &RpcClient, ata: &Pubkey) -> u64 {
    rpc.get_token_account_balance(ata).map(|b| b.amount.parse().unwrap_or(0)).unwrap_or(0)
}

/// The mints in the position's funded slots, in slot order — the order the program's health walk
/// expects its remaining accounts in.
fn used_slots(rpc: &RpcClient, position: &Pubkey) -> Result<Vec<Pubkey>, Box<dyn std::error::Error>> {
    let data = rpc.get_account_data(position)?;
    let p = hodl_loans::Position::try_deserialize(&mut &data[..])?;
    Ok(p.collateral.iter().filter(|s| s.amount > 0).map(|s| s.mint).collect())
}

fn position_amount(rpc: &RpcClient, position: &Pubkey, mint: &Pubkey) -> Result<u64, Box<dyn std::error::Error>> {
    let Ok(data) = rpc.get_account_data(position) else { return Ok(0) };
    let p = hodl_loans::Position::try_deserialize(&mut &data[..])?;
    Ok(p.collateral.iter().find(|s| s.mint == *mint).map(|s| s.amount).unwrap_or(0))
}

/// A keypair as the 64-byte JSON array `solana-keygen` writes, without pulling in serde_json.
fn serde_json_bytes(k: &Keypair) -> String {
    let bytes = k.to_bytes();
    let list: Vec<String> = bytes.iter().map(|b| b.to_string()).collect();
    format!("[{}]", list.join(","))
}
