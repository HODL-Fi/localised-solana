#![allow(dead_code, unused_imports)]

use anchor_lang::{
    prelude::{Clock, ProgramData, Pubkey},
    solana_program::{
        bpf_loader_upgradeable::get_program_data_address, instruction::Instruction, system_instruction,
        system_program,
    },
    AccountDeserialize, AccountSerialize, InstructionData, ToAccountMetas,
};
use hodl_loans::{
    constants::{ACCESS_SEED, CONFIG_SEED, LENDER_SEED, MARKET_SEED, MARKET_VAULT_SEED},
    HodlError,
};
use litesvm::LiteSVM;
use solana_keypair::Keypair;
use solana_message::{Message, VersionedMessage};
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;
use spl_token_2022_interface::{
    extension::{metadata_pointer, transfer_fee, BaseStateWithExtensions, ExtensionType, StateWithExtensions},
    state::{Account as TokenAccountState, Mint as MintState},
};

pub const TOKEN_2022: Pubkey = spl_token_2022_interface::ID;
pub const SPL_TOKEN: Pubkey = spl_token_interface::ID;
pub const ONE_CNGN: u64 = 1_000_000;
pub const YEAR_SECONDS: i64 = 31_536_000;

pub type TxResult = Result<(), String>;

/// Sends `ixs`; the first signer pays fees. Returns the error and logs as text on failure.
pub fn send(svm: &mut LiteSVM, ixs: &[Instruction], signers: &[&Keypair]) -> TxResult {
    svm.expire_blockhash();
    let msg = Message::new_with_blockhash(ixs, Some(&signers[0].pubkey()), &svm.latest_blockhash());
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), signers).unwrap();
    svm.send_transaction(tx)
        .map(|_| ())
        .map_err(|e| format!("{:?} logs: {:#?}", e.err, e.meta.logs))
}

pub fn assert_custom_error(result: TxResult, code: u32) {
    let err = result.expect_err("transaction should have failed");
    assert!(err.contains(&format!("Custom({code})")), "expected Custom({code}), got {err}");
}

pub fn assert_hodl_error(result: TxResult, expected: HodlError) {
    assert_custom_error(result, u32::from(expected));
}

pub fn assert_anchor_error(result: TxResult, expected: anchor_lang::error::ErrorCode) {
    assert_custom_error(result, u32::from(expected));
}

pub fn pda(seeds: &[&[u8]]) -> Pubkey {
    Pubkey::find_program_address(seeds, &hodl_loans::ID).0
}
pub fn config_pda() -> Pubkey {
    pda(&[CONFIG_SEED])
}
pub fn access_pda(wallet: &Pubkey) -> Pubkey {
    pda(&[ACCESS_SEED, wallet.as_ref()])
}
pub fn market_pda(mint: &Pubkey) -> Pubkey {
    pda(&[MARKET_SEED, mint.as_ref()])
}
pub fn market_vault_pda(mint: &Pubkey) -> Pubkey {
    pda(&[MARKET_VAULT_SEED, mint.as_ref()])
}
pub fn lender_pda(market: &Pubkey, owner: &Pubkey) -> Pubkey {
    pda(&[LENDER_SEED, market.as_ref(), owner.as_ref()])
}

pub fn ix<D: InstructionData, A: ToAccountMetas>(data: D, accounts: A) -> Instruction {
    Instruction::new_with_bytes(hodl_loans::ID, &data.data(), accounts.to_account_metas(None))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MintKind {
    /// Classic SPL Token mint (USDC, USDT, wrapped SOL).
    SplToken,
    /// Token-2022 mint with the Solana cNGN extensions: permanent delegate + metadata pointer.
    CngnLike,
    /// Token-2022 mint with a transfer fee (must be rejected).
    TransferFee,
}

pub struct Env {
    pub svm: LiteSVM,
    pub admin: Keypair,
    pub guardian: Keypair,
    pub whitelister: Keypair,
    pub promo_signer: Keypair,
    pub treasury: Keypair,
}

impl Env {
    /// Program loaded with `admin` as its upgrade authority. `Config` is not initialized.
    pub fn new() -> Self {
        let mut svm = LiteSVM::new();
        let bytes = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/deploy/hodl_loans.so"));
        svm.add_program(hodl_loans::ID, bytes).unwrap();
        let env = Self {
            svm,
            admin: Keypair::new(),
            guardian: Keypair::new(),
            whitelister: Keypair::new(),
            promo_signer: Keypair::new(),
            treasury: Keypair::new(),
        };
        let mut env = env;
        for key in [env.admin.pubkey(), env.guardian.pubkey(), env.whitelister.pubkey(), env.treasury.pubkey()] {
            env.svm.airdrop(&key, 100_000_000_000).unwrap();
        }
        let authority = env.admin.pubkey();
        env.set_upgrade_authority(&authority);
        env
    }

    /// Rewrites the ProgramData header (4-byte tag, 8-byte slot, 1-byte option, 32-byte key).
    pub fn set_upgrade_authority(&mut self, authority: &Pubkey) {
        let key = get_program_data_address(&hodl_loans::ID);
        let mut account = self.svm.get_account(&key).expect("program data account");
        account.data[12] = 1;
        account.data[13..45].copy_from_slice(authority.as_ref());
        self.svm.set_account(key, account).unwrap();
    }

    pub fn upgrade_authority(&self) -> Option<Pubkey> {
        let account = self.svm.get_account(&get_program_data_address(&hodl_loans::ID)).unwrap();
        ProgramData::try_deserialize(&mut account.data.as_slice()).unwrap().upgrade_authority_address
    }

    pub fn funded_keypair(&mut self) -> Keypair {
        let key = Keypair::new();
        self.svm.airdrop(&key.pubkey(), 10_000_000_000).unwrap();
        key
    }

    pub fn now(&self) -> i64 {
        self.svm.get_sysvar::<Clock>().unix_timestamp
    }

    pub fn warp_seconds(&mut self, seconds: i64) {
        let mut clock: Clock = self.svm.get_sysvar();
        clock.unix_timestamp += seconds;
        self.svm.set_sysvar(&clock);
    }

    pub fn fetch<T: AccountDeserialize>(&self, key: &Pubkey) -> T {
        let account = self.svm.get_account(key).expect("account exists");
        T::try_deserialize(&mut account.data.as_slice()).expect("account deserializes")
    }

    /// Overwrites an Anchor account's data in place (tests use this to simulate loans).
    pub fn write<T: AccountSerialize>(&mut self, key: &Pubkey, value: &T) {
        let mut account = self.svm.get_account(key).expect("account exists");
        let mut bytes = Vec::new();
        value.try_serialize(&mut bytes).unwrap();
        account.data[..bytes.len()].copy_from_slice(&bytes);
        self.svm.set_account(*key, account).unwrap();
    }

    pub fn create_mint(&mut self, kind: MintKind, decimals: u8) -> Pubkey {
        let mint = Keypair::new();
        let authority = self.admin.pubkey();
        let (program, extensions) = match kind {
            MintKind::SplToken => (SPL_TOKEN, vec![]),
            MintKind::CngnLike => (TOKEN_2022, vec![ExtensionType::PermanentDelegate, ExtensionType::MetadataPointer]),
            MintKind::TransferFee => (TOKEN_2022, vec![ExtensionType::TransferFeeConfig]),
        };
        let space = ExtensionType::try_calculate_account_len::<MintState>(&extensions).unwrap();
        let lamports = self.svm.minimum_balance_for_rent_exemption(space);
        let mut ixs = vec![system_instruction::create_account(&authority, &mint.pubkey(), lamports, space as u64, &program)];
        match kind {
            MintKind::SplToken => {
                ixs.push(spl_token_interface::instruction::initialize_mint2(&SPL_TOKEN, &mint.pubkey(), &authority, None, decimals).unwrap());
            }
            MintKind::CngnLike => {
                ixs.push(spl_token_2022_interface::instruction::initialize_permanent_delegate(&TOKEN_2022, &mint.pubkey(), &authority).unwrap());
                ixs.push(metadata_pointer::instruction::initialize(&TOKEN_2022, &mint.pubkey(), Some(authority), Some(mint.pubkey())).unwrap());
                ixs.push(spl_token_2022_interface::instruction::initialize_mint2(&TOKEN_2022, &mint.pubkey(), &authority, None, decimals).unwrap());
            }
            MintKind::TransferFee => {
                ixs.push(transfer_fee::instruction::initialize_transfer_fee_config(&TOKEN_2022, &mint.pubkey(), Some(&authority), Some(&authority), 10, 1_000).unwrap());
                ixs.push(spl_token_2022_interface::instruction::initialize_mint2(&TOKEN_2022, &mint.pubkey(), &authority, None, decimals).unwrap());
            }
        }
        send(&mut self.svm, &ixs, &[&self.admin, &mint]).expect("create mint");
        mint.pubkey()
    }

    pub fn mint_program(&self, mint: &Pubkey) -> Pubkey {
        self.svm.get_account(mint).unwrap().owner
    }

    pub fn mint_decimals(&self, mint: &Pubkey) -> u8 {
        let account = self.svm.get_account(mint).unwrap();
        StateWithExtensions::<MintState>::unpack(&account.data).unwrap().base.decimals
    }

    pub fn mint_extensions(&self, mint: &Pubkey) -> Vec<ExtensionType> {
        let account = self.svm.get_account(mint).unwrap();
        StateWithExtensions::<MintState>::unpack(&account.data).unwrap().get_extension_types().unwrap()
    }

    /// A token account for `mint` owned by `owner` (any pubkey, including a PDA).
    pub fn create_token_account(&mut self, mint: &Pubkey, owner: &Pubkey) -> Pubkey {
        let account = Keypair::new();
        let program = self.mint_program(mint);
        let space = ExtensionType::try_calculate_account_len::<TokenAccountState>(&[]).unwrap();
        let lamports = self.svm.minimum_balance_for_rent_exemption(space);
        let ixs = vec![
            system_instruction::create_account(&self.admin.pubkey(), &account.pubkey(), lamports, space as u64, &program),
            spl_token_2022_interface::instruction::initialize_account3(&program, &account.pubkey(), mint, owner).unwrap(),
        ];
        send(&mut self.svm, &ixs, &[&self.admin, &account]).expect("create token account");
        account.pubkey()
    }

    pub fn mint_to(&mut self, mint: &Pubkey, destination: &Pubkey, amount: u64) {
        let program = self.mint_program(mint);
        let decimals = self.mint_decimals(mint);
        let ix = spl_token_2022_interface::instruction::mint_to_checked(
            &program, mint, destination, &self.admin.pubkey(), &[], amount, decimals,
        )
        .unwrap();
        send(&mut self.svm, &[ix], &[&self.admin]).expect("mint to");
    }

    pub fn token_balance(&self, account: &Pubkey) -> u64 {
        let account = self.svm.get_account(account).unwrap();
        StateWithExtensions::<TokenAccountState>::unpack(&account.data).unwrap().base.amount
    }

    pub fn token_owner(&self, account: &Pubkey) -> Pubkey {
        let account = self.svm.get_account(account).unwrap();
        StateWithExtensions::<TokenAccountState>::unpack(&account.data).unwrap().base.owner
    }
}

// ---- Config and roles (Task 4) ----

impl Env {
    pub fn init_args(&self) -> hodl_loans::InitializeArgs {
        hodl_loans::InitializeArgs {
            guardian: self.guardian.pubkey(),
            whitelister: self.whitelister.pubkey(),
            promo_signer: self.promo_signer.pubkey(),
            treasury: self.treasury.pubkey(),
        }
    }

    pub fn initialize_ix(&self, authority: &Pubkey) -> Instruction {
        ix(
            hodl_loans::instruction::Initialize { args: self.init_args() },
            hodl_loans::accounts::Initialize {
                authority: *authority,
                config: config_pda(),
                program: hodl_loans::ID,
                program_data: get_program_data_address(&hodl_loans::ID),
                system_program: system_program::ID,
            },
        )
    }

    /// `new()` followed by a successful `initialize`.
    pub fn initialized() -> Self {
        let mut env = Self::new();
        env.initialize().unwrap();
        env
    }

    pub fn initialize(&mut self) -> TxResult {
        let instruction = self.initialize_ix(&self.admin.pubkey());
        send(&mut self.svm, &[instruction], &[&self.admin])
    }

    pub fn config(&self) -> hodl_loans::Config {
        self.fetch(&config_pda())
    }
}

pub fn admin_config_accounts(admin: &Pubkey) -> hodl_loans::accounts::AdminConfig {
    hodl_loans::accounts::AdminConfig { admin: *admin, config: config_pda() }
}

// ---- Access (Task 5) ----

pub fn whitelist_ix(signer: &Pubkey, wallet: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::Whitelist { wallet: *wallet },
        hodl_loans::accounts::Whitelist {
            signer: *signer,
            config: config_pda(),
            access: access_pda(wallet),
            system_program: system_program::ID,
        },
    )
}

pub fn blacklist_ix(admin: &Pubkey, wallet: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::Blacklist { wallet: *wallet },
        hodl_loans::accounts::Blacklist {
            admin: *admin,
            config: config_pda(),
            access: access_pda(wallet),
            system_program: system_program::ID,
        },
    )
}

pub fn unblacklist_ix(admin: &Pubkey, wallet: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::Unblacklist { wallet: *wallet },
        hodl_loans::accounts::Unblacklist { admin: *admin, config: config_pda(), access: access_pda(wallet) },
    )
}

impl Env {
    pub fn whitelist(&mut self, wallet: &Pubkey) {
        let instruction = whitelist_ix(&self.whitelister.pubkey(), wallet);
        send(&mut self.svm, &[instruction], &[&self.whitelister]).expect("whitelist");
    }

    pub fn blacklist(&mut self, wallet: &Pubkey) {
        let instruction = blacklist_ix(&self.admin.pubkey(), wallet);
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("blacklist");
    }

    pub fn access(&self, wallet: &Pubkey) -> hodl_loans::Access {
        self.fetch(&access_pda(wallet))
    }
}

// ---- Market (Task 6) ----

pub fn default_market_params() -> hodl_loans::MarketParams {
    hodl_loans::MarketParams {
        interest_rate_bps: 1_500,
        penalty_rate_bps: 500,
        reserve_factor_bps: 1_000,
        max_utilization_bps: 9_000,
        min_loan_amount: 1_000 * ONE_CNGN,
        max_tenure_seconds: 365 * 86_400,
        bad_debt_dust_usd: 1_000_000_000_000_000_000,
        ngn_feed: Pubkey::new_from_array([7; 32]),
        ngn_max_stale_slots: 150,
        ngn_min_samples: 3,
        ngn_max_spread_bps: 200,
        promo_inactivity_seconds: 90 * 86_400,
        max_promo_per_position: 50_000 * ONE_CNGN,
    }
}

pub fn create_market_ix(admin: &Pubkey, mint: &Pubkey, token_program: &Pubkey, params: hodl_loans::MarketParams) -> Instruction {
    ix(
        hodl_loans::instruction::CreateMarket { params },
        hodl_loans::accounts::CreateMarket {
            admin: *admin,
            config: config_pda(),
            mint: *mint,
            market: market_pda(mint),
            vault: market_vault_pda(mint),
            token_program: *token_program,
            system_program: system_program::ID,
        },
    )
}

pub fn update_market_params_ix(admin: &Pubkey, mint: &Pubkey, params: hodl_loans::MarketParams) -> Instruction {
    ix(
        hodl_loans::instruction::UpdateMarketParams { params },
        hodl_loans::accounts::UpdateMarketParams { admin: *admin, config: config_pda(), market: market_pda(mint) },
    )
}

pub fn set_market_paused_ix(signer: &Pubkey, mint: &Pubkey, paused: bool) -> Instruction {
    ix(
        hodl_loans::instruction::SetMarketPaused { paused },
        hodl_loans::accounts::SetMarketPaused { signer: *signer, config: config_pda(), market: market_pda(mint) },
    )
}

impl Env {
    /// Initialized config plus a cNGN-like market (6 decimals). Returns the mint.
    pub fn with_cngn_market() -> (Self, Pubkey) {
        let mut env = Self::initialized();
        let mint = env.create_mint(MintKind::CngnLike, 6);
        let instruction = create_market_ix(&env.admin.pubkey(), &mint, &TOKEN_2022, default_market_params());
        send(&mut env.svm, &[instruction], &[&env.admin]).expect("create market");
        (env, mint)
    }

    pub fn market(&self, mint: &Pubkey) -> hodl_loans::Market {
        self.fetch(&market_pda(mint))
    }
}

// ---- Lender deposits (Task 7) ----

pub fn deposit_liquidity_ix(payer: &Pubkey, owner: &Pubkey, mint: &Pubkey, owner_token: &Pubkey, amount: u64) -> Instruction {
    let market = market_pda(mint);
    ix(
        hodl_loans::instruction::DepositLiquidity { amount },
        hodl_loans::accounts::DepositLiquidity {
            payer: *payer,
            owner: *owner,
            access: access_pda(owner),
            market,
            mint: *mint,
            vault: market_vault_pda(mint),
            owner_token: *owner_token,
            lender: lender_pda(&market, owner),
            token_program: TOKEN_2022,
            system_program: system_program::ID,
        },
    )
}

pub struct Lender {
    pub key: Keypair,
    pub token: Pubkey,
}

impl Env {
    /// A whitelisted wallet holding `balance` cNGN and no SOL (the admin pays its fees and rent).
    pub fn new_lender(&mut self, mint: &Pubkey, balance: u64) -> Lender {
        let key = Keypair::new();
        self.whitelist(&key.pubkey());
        let token = self.create_token_account(mint, &key.pubkey());
        self.mint_to(mint, &token, balance);
        Lender { key, token }
    }

    pub fn deposit(&mut self, lender: &Lender, mint: &Pubkey, amount: u64) -> TxResult {
        let instruction = deposit_liquidity_ix(&self.admin.pubkey(), &lender.key.pubkey(), mint, &lender.token, amount);
        send(&mut self.svm, &[instruction], &[&self.admin, &lender.key])
    }

    pub fn lender_shares(&self, mint: &Pubkey, owner: &Pubkey) -> u128 {
        let lender: hodl_loans::LenderPosition = self.fetch(&lender_pda(&market_pda(mint), owner));
        lender.shares
    }
}

// ---- Lender withdrawals (Task 8) ----

pub fn withdraw_liquidity_ix(owner: &Pubkey, mint: &Pubkey, owner_token: &Pubkey, amount: u64) -> Instruction {
    let market = market_pda(mint);
    ix(
        hodl_loans::instruction::WithdrawLiquidity { amount },
        hodl_loans::accounts::WithdrawLiquidity {
            owner: *owner,
            access: access_pda(owner),
            market,
            mint: *mint,
            vault: market_vault_pda(mint),
            owner_token: *owner_token,
            lender: lender_pda(&market, owner),
            token_program: TOKEN_2022,
        },
    )
}

impl Env {
    pub fn withdraw(&mut self, lender: &Lender, mint: &Pubkey, amount: u64) -> TxResult {
        let instruction = withdraw_liquidity_ix(&lender.key.pubkey(), mint, &lender.token, amount);
        send(&mut self.svm, &[instruction], &[&self.admin, &lender.key])
    }
}

// ---- Sweep (Task 9) ----

pub fn sweep_market_excess_ix(admin: &Pubkey, mint: &Pubkey, destination: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::SweepMarketExcess {},
        hodl_loans::accounts::SweepMarketExcess {
            admin: *admin,
            config: config_pda(),
            market: market_pda(mint),
            mint: *mint,
            vault: market_vault_pda(mint),
            destination: *destination,
            token_program: TOKEN_2022,
        },
    )
}

// ---- Collateral listing (Task 4) ----

pub const ONE_USDC: u64 = 1_000_000;

pub fn collateral_pda(mint: &Pubkey) -> Pubkey {
    pda(&[hodl_loans::constants::COLLATERAL_SEED, mint.as_ref()])
}
pub fn collateral_vault_pda(mint: &Pubkey) -> Pubkey {
    pda(&[hodl_loans::constants::COLLATERAL_VAULT_SEED, mint.as_ref()])
}

/// Tests use a mint's own bytes as its Pyth feed ID.
pub fn feed_id(mint: &Pubkey) -> [u8; 32] {
    mint.to_bytes()
}

/// Spec §8 launch values for a stablecoin: LTV 70%, threshold 90%, bonus 5%.
pub fn default_collateral_params(mint: &Pubkey) -> hodl_loans::CollateralParams {
    hodl_loans::CollateralParams {
        pyth_feed_id: feed_id(mint),
        max_price_age_seconds: 60,
        max_conf_bps: 200,
        ltv_bps: 7_000,
        liquidation_threshold_bps: 9_000,
        liquidation_bonus_bps: 500,
        deposit_cap: u64::MAX,
    }
}

pub fn list_collateral_ix(admin: &Pubkey, mint: &Pubkey, token_program: &Pubkey, params: hodl_loans::CollateralParams) -> Instruction {
    ix(
        hodl_loans::instruction::ListCollateral { params },
        hodl_loans::accounts::ListCollateral {
            admin: *admin,
            config: config_pda(),
            mint: *mint,
            collateral: collateral_pda(mint),
            vault: collateral_vault_pda(mint),
            token_program: *token_program,
            system_program: system_program::ID,
        },
    )
}

pub fn update_collateral_params_ix(admin: &Pubkey, mint: &Pubkey, params: hodl_loans::CollateralParams) -> Instruction {
    ix(
        hodl_loans::instruction::UpdateCollateralParams { params },
        hodl_loans::accounts::UpdateCollateralParams { admin: *admin, config: config_pda(), collateral: collateral_pda(mint) },
    )
}

pub fn set_collateral_paused_ix(signer: &Pubkey, mint: &Pubkey, paused: bool) -> Instruction {
    ix(
        hodl_loans::instruction::SetCollateralPaused { paused },
        hodl_loans::accounts::SetCollateralPaused { signer: *signer, config: config_pda(), collateral: collateral_pda(mint) },
    )
}

pub fn delist_collateral_ix(admin: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::DelistCollateral {},
        hodl_loans::accounts::DelistCollateral {
            admin: *admin,
            config: config_pda(),
            collateral: collateral_pda(mint),
            mint: *mint,
            vault: collateral_vault_pda(mint),
            token_program: *token_program,
        },
    )
}

pub fn sweep_collateral_excess_ix(admin: &Pubkey, mint: &Pubkey, token_program: &Pubkey, destination: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::SweepCollateralExcess {},
        hodl_loans::accounts::SweepCollateralExcess {
            admin: *admin,
            config: config_pda(),
            collateral: collateral_pda(mint),
            mint: *mint,
            vault: collateral_vault_pda(mint),
            destination: *destination,
            token_program: *token_program,
        },
    )
}

impl Env {
    /// Creates a classic SPL Token mint and lists it with default parameters. Returns the mint.
    pub fn list_spl_collateral(&mut self, decimals: u8) -> Pubkey {
        let mint = self.create_mint(MintKind::SplToken, decimals);
        let instruction = list_collateral_ix(&self.admin.pubkey(), &mint, &SPL_TOKEN, default_collateral_params(&mint));
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("list collateral");
        mint
    }

    pub fn collateral(&self, mint: &Pubkey) -> hodl_loans::CollateralAsset {
        self.fetch(&collateral_pda(mint))
    }
}
