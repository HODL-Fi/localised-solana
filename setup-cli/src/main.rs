//! One-shot devnet setup driver: initialize the program, create the cNGN market and its
//! promo vault, list a collateral asset, whitelist the admin, and seed lender liquidity.
//!
//! Instruction construction mirrors `programs/hodl_loans/tests/common/mod.rs`, which is the
//! shape 282 tests exercise. Types come from the program crate itself rather than through the
//! IDL, so a mismatch is a compile error instead of a runtime one.
//!
//! Idempotent: every step checks whether its account already exists and skips if so, because
//! a devnet run that fails halfway should be safe to re-run.

use anchor_lang::{system_program, InstructionData, ToAccountMetas};
use anchor_spl::token_2022::ID as TOKEN_2022;
use solana_client::rpc_client::RpcClient;
use anchor_lang::solana_program::bpf_loader_upgradeable;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::read_keypair_file, signer::Signer,
    transaction::Transaction,
};
use std::str::FromStr;

const RPC: &str = "https://api.devnet.solana.com";
const ONE: u64 = 1_000_000; // both mints are 6-decimal

fn pda(seeds: &[&[u8]]) -> Pubkey {
    Pubkey::find_program_address(seeds, &hodl_loans::ID).0
}
fn ix<D: InstructionData, A: ToAccountMetas>(data: D, accounts: A) -> Instruction {
    Instruction::new_with_bytes(hodl_loans::ID, &data.data(), accounts.to_account_metas(None))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let kp_path = std::env::var("HOME")? + "/.config/solana/id.json";
    let admin = read_keypair_file(&kp_path).map_err(|e| format!("{kp_path}: {e}"))?;
    let rpc = RpcClient::new_with_commitment(RPC.to_string(), CommitmentConfig::confirmed());

    let cngn = Pubkey::from_str(&std::env::var("CNGN_MINT")?)?;
    // Listed in a later pass, once this one is confirmed on-chain.
    let usdc = Pubkey::from_str(&std::env::var("USDC_MINT")?)?;

    println!("program {}\nadmin   {}\ncNGN    {}\ncoll    {}\n", hodl_loans::ID, admin.pubkey(), cngn, usdc);

    let config = pda(&[hodl_loans::CONFIG_SEED]);
    let market = pda(&[hodl_loans::MARKET_SEED, cngn.as_ref()]);

    let send = |name: &str, key: Pubkey, ixs: Vec<Instruction>| -> Result<(), Box<dyn std::error::Error>> {
        if rpc.get_account(&key).is_ok() {
            println!("  {name:<22} already exists ({key}) — skipped");
            return Ok(());
        }
        let bh = rpc.get_latest_blockhash()?;
        let tx = Transaction::new_signed_with_payer(&ixs, Some(&admin.pubkey()), &[&admin], bh);
        match rpc.send_and_confirm_transaction(&tx) {
            Ok(sig) => println!("  {name:<22} OK  {key}\n  {:<22} sig {sig}", ""),
            Err(e) => {
                println!("  {name:<22} FAILED: {e}");
                return Err(e.into());
            }
        }
        Ok(())
    };

    // 1. initialize — the caller must be the program's upgrade authority.
    let (program_data, _) = Pubkey::find_program_address(&[hodl_loans::ID.as_ref()], &bpf_loader_upgradeable::ID);
    send("initialize", config, vec![ix(
        hodl_loans::instruction::Initialize {
            args: hodl_loans::InitializeArgs {
                guardian: admin.pubkey(),
                whitelister: admin.pubkey(),
                promo_signer: admin.pubkey(),
                treasury: admin.pubkey(),
            },
        },
        hodl_loans::accounts::Initialize {
            authority: admin.pubkey(),
            config,
            program: hodl_loans::ID,
            program_data,
            system_program: system_program::ID,
        },
    )])?;

    // 2. create_market. `ngn_feed` is a placeholder: it is stored here and only validated at
    //    use time, and devnet has no Switchboard NGN feed yet. Point it at a real feed with
    //    update_market_params before anything priced is attempted.
    let params = hodl_loans::MarketParams {
        interest_rate_bps: 1_500,
        penalty_rate_bps: 500,
        reserve_factor_bps: 1_000,
        max_utilization_bps: 9_000,
        min_loan_amount: 1_000 * ONE,
        max_tenure_seconds: 365 * 86_400,
        bad_debt_dust_usd: 5_000_000_000_000,
        ngn_feed: Pubkey::new_from_array([7; 32]),
        ngn_max_stale_slots: 150,
        ngn_min_samples: 3,
        ngn_max_spread_bps: 200,
        promo_inactivity_seconds: 90 * 86_400,
        max_promo_per_position: 50_000 * ONE,
    };
    send("create_market", market, vec![ix(
        hodl_loans::instruction::CreateMarket { params },
        hodl_loans::accounts::CreateMarket {
            admin: admin.pubkey(),
            config,
            mint: cngn,
            market,
            vault: pda(&[hodl_loans::MARKET_VAULT_SEED, cngn.as_ref()]),
            token_program: TOKEN_2022,
            system_program: system_program::ID,
        },
    )])?;

    // 3. create_promo_vault — required: take_loan, liquidate and write_off_loan all name it,
    //    so a market without one cannot be borrowed against.
    //    Note the seed: the promo vault PDAs hang off the MARKET pda, not the mint.
    let promo_vault = pda(&[hodl_loans::PROMO_VAULT_SEED, market.as_ref()]);
    send("create_promo_vault", promo_vault, vec![ix(
        hodl_loans::instruction::CreatePromoVault {},
        hodl_loans::accounts::CreatePromoVault {
            admin: admin.pubkey(),
            config,
            market,
            mint: cngn,
            promo_vault,
            vault: pda(&[hodl_loans::PROMO_VAULT_TOKEN_SEED, market.as_ref()]),
            token_program: TOKEN_2022,
            system_program: system_program::ID,
        },
    )])?;

    // 4. list_collateral. `pyth_feed_id` / `price_account` are placeholders for the same
    //    reason ngn_feed is — devnet has no matching Pyth publisher for this test mint, and
    //    both are validated only when a priced path reads them. Point them at real accounts
    //    with update_collateral_params before borrowing.
    //
    //    max_multiplier: 0 means "not a scaled-UI asset", which is right for a plain mint.
    let collateral = pda(&[hodl_loans::COLLATERAL_SEED, usdc.as_ref()]);
    send("list_collateral", collateral, vec![ix(
        hodl_loans::instruction::ListCollateral {
            params: hodl_loans::CollateralParams {
                pyth_feed_id: [9u8; 32],
                price_account: Pubkey::new_from_array([9; 32]),
                max_price_age_seconds: 60,
                max_conf_bps: 200,
                ltv_bps: 7_000,
                liquidation_threshold_bps: 9_000,
                liquidation_bonus_bps: 500,
                deposit_cap: u64::MAX,
                max_multiplier: 0,
            },
            kind: hodl_loans::CollateralKind::Standard,
        },
        hodl_loans::accounts::ListCollateral {
            admin: admin.pubkey(),
            config,
            mint: usdc,
            collateral,
            vault: pda(&[hodl_loans::COLLATERAL_VAULT_SEED, usdc.as_ref()]),
            token_program: anchor_spl::token::ID, // the collateral mint is classic SPL Token
            system_program: system_program::ID,
        },
    )])?;

    // 5. whitelist the admin, so it can act as a lender below.
    let access = pda(&[hodl_loans::ACCESS_SEED, admin.pubkey().as_ref()]);
    send("whitelist(admin)", access, vec![ix(
        hodl_loans::instruction::Whitelist { wallet: admin.pubkey() },
        hodl_loans::accounts::Whitelist {
            signer: admin.pubkey(),
            config,
            access,
            system_program: system_program::ID,
        },
    )])?;

    // 6. deposit_liquidity — lenders must fund the market before anything can borrow.
    //    Needs no price feed, so it works today.
    let admin_cngn = anchor_spl::associated_token::get_associated_token_address_with_program_id(
        &admin.pubkey(), &cngn, &TOKEN_2022,
    );
    let lender = pda(&[hodl_loans::LENDER_SEED, market.as_ref(), admin.pubkey().as_ref()]);
    send("deposit_liquidity", lender, vec![ix(
        hodl_loans::instruction::DepositLiquidity { amount: 1_000_000 * ONE },
        hodl_loans::accounts::DepositLiquidity {
            payer: admin.pubkey(),
            owner: admin.pubkey(),
            access,
            market,
            mint: cngn,
            vault: pda(&[hodl_loans::MARKET_VAULT_SEED, cngn.as_ref()]),
            owner_token: admin_cngn,
            lender,
            token_program: TOKEN_2022,
            system_program: system_program::ID,
        },
    )])?;

    println!("\ndone.");
    println!("  config     {config}");
    println!("  market     {market}");
    println!("  collateral {collateral}");
    println!("  lender     {lender}");
    Ok(())
}
