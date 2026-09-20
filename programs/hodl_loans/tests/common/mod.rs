#![allow(dead_code, unused_imports)]

use anchor_lang::{
    prelude::{Clock, ProgramData, Pubkey},
    solana_program::{
        bpf_loader_upgradeable::get_program_data_address, instruction::{AccountMeta, Instruction}, system_instruction,
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
    extension::{
        confidential_transfer, default_account_state, metadata_pointer, pausable, scaled_ui_amount,
        transfer_fee, transfer_hook, BaseStateWithExtensions, ExtensionType, StateWithExtensions,
    },
    state::{Account as TokenAccountState, AccountState, Mint as MintState},
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
    /// Token-2022 mint carrying metadata only: the shape a `Standard` collateral asset
    /// may have (spec §14).
    Token2022Plain,
    /// Token-2022 mint with a transfer fee (must be rejected).
    TransferFee,
    /// Token-2022 mint shaped like a live Backed xStock (AAPLX, TSLAX, NVDAX, checked
    /// 2026-09-18): metadata pointer, permanent delegate, scaled-UI amount, pausable,
    /// transfer hook with no program, default account state, confidential transfer.
    XStock,
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
        // `with_precompiles` loads the native Ed25519 program, which promo vouchers need.
        let mut svm = LiteSVM::new().with_precompiles();
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
            MintKind::Token2022Plain => (TOKEN_2022, vec![ExtensionType::MetadataPointer]),
            MintKind::TransferFee => (TOKEN_2022, vec![ExtensionType::TransferFeeConfig]),
            MintKind::XStock => (
                TOKEN_2022,
                vec![
                    ExtensionType::MetadataPointer,
                    ExtensionType::PermanentDelegate,
                    ExtensionType::ScaledUiAmount,
                    ExtensionType::Pausable,
                    ExtensionType::TransferHook,
                    ExtensionType::DefaultAccountState,
                    ExtensionType::ConfidentialTransferMint,
                ],
            ),
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
            MintKind::Token2022Plain => {
                ixs.push(metadata_pointer::instruction::initialize(&TOKEN_2022, &mint.pubkey(), Some(authority), Some(mint.pubkey())).unwrap());
                ixs.push(spl_token_2022_interface::instruction::initialize_mint2(&TOKEN_2022, &mint.pubkey(), &authority, None, decimals).unwrap());
            }
            MintKind::TransferFee => {
                ixs.push(transfer_fee::instruction::initialize_transfer_fee_config(&TOKEN_2022, &mint.pubkey(), Some(&authority), Some(&authority), 10, 1_000).unwrap());
                ixs.push(spl_token_2022_interface::instruction::initialize_mint2(&TOKEN_2022, &mint.pubkey(), &authority, None, decimals).unwrap());
            }
            MintKind::XStock => {
                let m = mint.pubkey();
                ixs.push(metadata_pointer::instruction::initialize(&TOKEN_2022, &m, Some(authority), Some(m)).unwrap());
                ixs.push(spl_token_2022_interface::instruction::initialize_permanent_delegate(&TOKEN_2022, &m, &authority).unwrap());
                ixs.push(scaled_ui_amount::instruction::initialize(&TOKEN_2022, &m, Some(authority), 1.0).unwrap());
                ixs.push(pausable::instruction::initialize(&TOKEN_2022, &m, &authority).unwrap());
                ixs.push(transfer_hook::instruction::initialize(&TOKEN_2022, &m, Some(authority), None).unwrap());
                ixs.push(default_account_state::instruction::initialize_default_account_state(&TOKEN_2022, &m, &AccountState::Initialized).unwrap());
                ixs.push(confidential_transfer::instruction::initialize_mint(&TOKEN_2022, &m, Some(authority), true, None).unwrap());
                ixs.push(spl_token_2022_interface::instruction::initialize_mint2(&TOKEN_2022, &m, &authority, Some(&authority), decimals).unwrap());
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
        // A Token-2022 mint extension can oblige every account to carry a matching one.
        let required = if program == TOKEN_2022 {
            ExtensionType::get_required_init_account_extensions(&self.mint_extensions(mint))
        } else {
            vec![]
        };
        let space = ExtensionType::try_calculate_account_len::<TokenAccountState>(&required).unwrap();
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
        // $5 at USD_SCALE: below that, liquidating costs more than it recovers.
        bad_debt_dust_usd: 5_000_000_000_000,
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
        let admin = env.admin.pubkey();
        let instruction = create_market_ix(&admin, &mint, &TOKEN_2022, default_market_params());
        send(&mut env.svm, &[instruction], &[&env.admin]).expect("create market");
        // Every market gets a promo vault, as a deployed one would: `take_loan` may expire promo
        // (spec §10 step 3), so the account is part of its shape whether or not promo is funded.
        send(&mut env.svm, &[create_promo_vault_ix(&admin, &mint)], &[&env.admin])
            .expect("create promo vault");
        (env, mint)
    }

    /// A second market, with the promo vault every market needs: `take_loan`, `liquidate` and
    /// `write_off_loan` all name it, so a market without one cannot be borrowed against.
    pub fn create_market_with_promo(&mut self, mint: &Pubkey) {
        let admin = self.admin.pubkey();
        let create = create_market_ix(&admin, mint, &TOKEN_2022, default_market_params());
        send(&mut self.svm, &[create], &[&self.admin]).expect("create market");
        send(&mut self.svm, &[create_promo_vault_ix(&admin, mint)], &[&self.admin])
            .expect("create promo vault");
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

/// Spec §8 launch values for a stablecoin: LTV 70%, threshold 90%, bonus 5%,
/// pinned to the mint's test price account.
pub fn default_collateral_params(mint: &Pubkey) -> hodl_loans::CollateralParams {
    hodl_loans::CollateralParams {
        pyth_feed_id: feed_id(mint),
        price_account: pyth_account(mint),
        max_price_age_seconds: 60,
        max_conf_bps: 200,
        ltv_bps: 7_000,
        liquidation_threshold_bps: 9_000,
        liquidation_bonus_bps: 500,
        deposit_cap: u64::MAX,
    }
}

pub fn list_collateral_ix(
    admin: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
    params: hodl_loans::CollateralParams,
    kind: hodl_loans::CollateralKind,
) -> Instruction {
    ix(
        hodl_loans::instruction::ListCollateral { params, kind },
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
        let instruction = list_collateral_ix(
            &self.admin.pubkey(),
            &mint,
            &SPL_TOKEN,
            default_collateral_params(&mint),
            hodl_loans::CollateralKind::Standard,
        );
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("list collateral");
        mint
    }

    pub fn collateral(&self, mint: &Pubkey) -> hodl_loans::CollateralAsset {
        self.fetch(&collateral_pda(mint))
    }
}

// ---- Positions and collateral deposits (Task 5) ----

pub fn position_pda(owner: &Pubkey) -> Pubkey {
    pda(&[hodl_loans::constants::POSITION_SEED, owner.as_ref()])
}

pub fn open_position_ix(payer: &Pubkey, owner: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::OpenPosition {},
        hodl_loans::accounts::OpenPosition {
            payer: *payer,
            owner: *owner,
            access: access_pda(owner),
            position: position_pda(owner),
            system_program: system_program::ID,
        },
    )
}

pub fn close_position_ix(owner: &Pubkey, rent_payer: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::ClosePosition {},
        hodl_loans::accounts::ClosePosition {
            owner: *owner,
            access: access_pda(owner),
            position: position_pda(owner),
            rent_payer: *rent_payer,
            market: None,
            promo_vault: None,
        },
    )
}

// ---- Promo expiry and revocation (Task 6) ----

pub fn expire_promo_ix(mint: &Pubkey, owner: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::ExpirePromo {},
        hodl_loans::accounts::ExpirePromo {
            market: market_pda(mint),
            promo_vault: promo_vault_pda(mint),
            position: position_pda(owner),
        },
    )
}

pub fn revoke_promo_ix(admin: &Pubkey, mint: &Pubkey, owner: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::RevokePromo {},
        hodl_loans::accounts::RevokePromo {
            admin: *admin,
            config: config_pda(),
            market: market_pda(mint),
            promo_vault: promo_vault_pda(mint),
            position: position_pda(owner),
        },
    )
}

/// `close_position`, naming the promo vault so a position still holding promo can hand it back.
pub fn close_position_with_promo_ix(owner: &Pubkey, rent_payer: &Pubkey, mint: &Pubkey) -> Instruction {
    close_position_with_split_promo_ix(owner, rent_payer, mint, mint)
}

/// `close_position`, letting the `market` and `promo_vault` accounts come from two different
/// markets. `promo_vault` derives from its own self-referential seeds (no `has_one = market`
/// constraint links it back to `market`), so this shape can present a foreign vault behind an
/// otherwise-legitimate `market` account — the only caller of `release_promo` able to do so, and
/// the reason `release_promo` carries its own chokepoint check rather than trusting callers.
pub fn close_position_with_split_promo_ix(
    owner: &Pubkey,
    rent_payer: &Pubkey,
    market_mint: &Pubkey,
    vault_mint: &Pubkey,
) -> Instruction {
    ix(
        hodl_loans::instruction::ClosePosition {},
        hodl_loans::accounts::ClosePosition {
            market: Some(market_pda(market_mint)),
            promo_vault: Some(promo_vault_pda(vault_mint)),
            owner: *owner,
            access: access_pda(owner),
            position: position_pda(owner),
            rent_payer: *rent_payer,
        },
    )
}

pub fn deposit_collateral_ix(owner: &Pubkey, mint: &Pubkey, token_program: &Pubkey, owner_token: &Pubkey, amount: u64) -> Instruction {
    ix(
        hodl_loans::instruction::DepositCollateral { amount },
        hodl_loans::accounts::DepositCollateral {
            owner: *owner,
            access: access_pda(owner),
            position: position_pda(owner),
            collateral: collateral_pda(mint),
            mint: *mint,
            vault: collateral_vault_pda(mint),
            owner_token: *owner_token,
            token_program: *token_program,
        },
    )
}

/// A whitelisted wallet with no SOL; the admin pays its fees and rent.
pub struct Borrower {
    pub key: Keypair,
}

impl Borrower {
    pub fn pubkey(&self) -> Pubkey {
        self.key.pubkey()
    }
}

impl Env {
    /// Sends one instruction with the admin paying fees and `signer` co-signing.
    pub fn sponsored(&mut self, instruction: Instruction, signer: &Keypair) -> TxResult {
        send(&mut self.svm, &[instruction], &[&self.admin, signer])
    }

    /// A whitelisted wallet with an open position.
    pub fn new_borrower(&mut self) -> Borrower {
        let key = Keypair::new();
        self.whitelist(&key.pubkey());
        let instruction = open_position_ix(&self.admin.pubkey(), &key.pubkey());
        send(&mut self.svm, &[instruction], &[&self.admin, &key]).expect("open position");
        Borrower { key }
    }

    /// Reads a zero-copy `Position` (unaligned, since test buffers are not 8-byte aligned).
    pub fn position(&self, owner: &Pubkey) -> hodl_loans::Position {
        let account = self.svm.get_account(&position_pda(owner)).expect("position exists");
        let size = std::mem::size_of::<hodl_loans::Position>();
        bytemuck::pod_read_unaligned(&account.data[8..8 + size])
    }

    /// Mints `amount` of `mint` into a new token account owned by the borrower, then deposits it.
    /// Returns the token account.
    pub fn deposit_collateral(&mut self, borrower: &Borrower, mint: &Pubkey, amount: u64) -> Pubkey {
        let token = self.create_token_account(mint, &borrower.pubkey());
        self.mint_to(mint, &token, amount);
        let program = self.mint_program(mint);
        let instruction = deposit_collateral_ix(&borrower.pubkey(), mint, &program, &token, amount);
        send(&mut self.svm, &[instruction], &[&self.admin, &borrower.key]).expect("deposit collateral");
        token
    }
}

// ---- Prices and loans (Task 6) ----

/// $0.000625 per NGN (NGN/USD 1,600) at Switchboard's 18 decimals.
pub const NGN_USD: i128 = 625_000_000_000_000;
/// A 0.1% spread on `NGN_USD`.
pub const NGN_SPREAD: i128 = 625_000_000_000;
/// $1.00 at Pyth exponent -8.
pub const ONE_DOLLAR: i64 = 100_000_000;

/// Where tests store a mint's Pyth `PriceUpdateV2` account.
pub fn pyth_account(mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"pyth", mint.as_ref()], &pyth_solana_receiver_sdk::ID).0
}

pub fn ngn_feed() -> Pubkey {
    default_market_params().ngn_feed
}

/// Raw Switchboard `PullFeedAccountData` with only the aggregated result set.
pub fn pull_feed_data(value: i128, std_dev: i128, slot: u64, num_samples: u8) -> Vec<u8> {
    use switchboard_on_demand::{Discriminator, PullFeedAccountData};
    let mut feed: PullFeedAccountData = bytemuck::Zeroable::zeroed();
    feed.result.value = value;
    feed.result.std_dev = std_dev;
    feed.result.slot = slot;
    feed.result.num_samples = num_samples;
    let mut data = PullFeedAccountData::DISCRIMINATOR.to_vec();
    data.extend_from_slice(bytemuck::bytes_of(&feed));
    data
}

pub fn price_update_data(
    mint: &Pubkey,
    price: i64,
    conf: u64,
    publish_time: i64,
    level: pyth_solana_receiver_sdk::price_update::VerificationLevel,
) -> Vec<u8> {
    use pyth_solana_receiver_sdk::price_update::{PriceFeedMessage, PriceUpdateV2};
    let update = PriceUpdateV2 {
        write_authority: Pubkey::new_unique(),
        verification_level: level,
        price_message: PriceFeedMessage {
            feed_id: feed_id(mint),
            price,
            conf,
            exponent: -8,
            publish_time,
            prev_publish_time: publish_time - 1,
            ema_price: price,
            ema_conf: conf,
        },
        posted_slot: 1,
    };
    let mut data = Vec::new();
    update.try_serialize(&mut data).unwrap();
    data
}

pub fn take_loan_ix(owner: &Pubkey, mint: &Pubkey, owner_token: &Pubkey, amount: u64, tenure_seconds: i64, prices: Vec<AccountMeta>) -> Instruction {
    let mut instruction = ix(
        hodl_loans::instruction::TakeLoan { amount, tenure_seconds },
        hodl_loans::accounts::TakeLoan {
            owner: *owner,
            access: access_pda(owner),
            config: config_pda(),
            promo_vault: Some(promo_vault_pda(mint)),
            position: position_pda(owner),
            market: market_pda(mint),
            mint: *mint,
            vault: market_vault_pda(mint),
            owner_token: *owner_token,
            ngn_feed: ngn_feed(),
            token_program: TOKEN_2022,
        },
    );
    instruction.accounts.extend(prices);
    instruction
}

/// Same as `take_loan_ix`, but without the promo vault account — for exercising the
/// `PromoAccountsRequired` guard when a position's promo is due for release.
pub fn take_loan_ix_no_promo_vault(
    owner: &Pubkey,
    mint: &Pubkey,
    owner_token: &Pubkey,
    amount: u64,
    tenure_seconds: i64,
    prices: Vec<AccountMeta>,
) -> Instruction {
    let mut instruction = ix(
        hodl_loans::instruction::TakeLoan { amount, tenure_seconds },
        hodl_loans::accounts::TakeLoan {
            owner: *owner,
            access: access_pda(owner),
            config: config_pda(),
            promo_vault: None,
            position: position_pda(owner),
            market: market_pda(mint),
            mint: *mint,
            vault: market_vault_pda(mint),
            owner_token: *owner_token,
            ngn_feed: ngn_feed(),
            token_program: TOKEN_2022,
        },
    );
    instruction.accounts.extend(prices);
    instruction
}

impl Env {
    pub fn set_account_data(&mut self, key: &Pubkey, owner: &Pubkey, data: Vec<u8>) {
        let account = solana_account::Account {
            lamports: self.svm.minimum_balance_for_rent_exemption(data.len()),
            data,
            owner: *owner,
            executable: false,
            rent_epoch: 0,
        };
        self.svm.set_account(*key, account).unwrap();
    }

    /// Writes a fully verified Pyth price (exponent -8) for `mint`, published now.
    pub fn set_pyth_price(&mut self, mint: &Pubkey, price: i64, conf: u64) {
        let now = self.now();
        let data = price_update_data(mint, price, conf, now, pyth_solana_receiver_sdk::price_update::VerificationLevel::Full);
        self.set_account_data(&pyth_account(mint), &pyth_solana_receiver_sdk::ID, data);
    }

    /// Writes the Switchboard NGN/USD result at the current slot with 5 samples.
    pub fn set_ngn_price(&mut self, value: i128, std_dev: i128) {
        let slot = self.svm.get_sysvar::<Clock>().slot;
        self.set_account_data(&ngn_feed(), &switchboard_on_demand::ON_DEMAND_MAINNET_PID, pull_feed_data(value, std_dev, slot, 5));
    }

    /// The health accounts for every used collateral slot, in slot order: a pair per
    /// `Standard` asset, and the mint as well for an `XStock`.
    pub fn price_accounts(&self, owner: &Pubkey) -> Vec<AccountMeta> {
        let position = self.position(owner);
        position
            .collateral
            .iter()
            .filter(|slot| slot.amount > 0)
            .flat_map(|slot| self.collateral_accounts(&slot.mint))
            .collect()
    }

    /// One listed asset's health accounts: `(CollateralAsset, PriceUpdateV2)`, plus the mint
    /// when the asset is an `XStock` (its multiplier lives there).
    pub fn collateral_accounts(&self, mint: &Pubkey) -> Vec<AccountMeta> {
        let mut metas = price_pairs(&[*mint]);
        if self.collateral(mint).kind == hodl_loans::CollateralKind::XStock {
            metas.push(AccountMeta::new_readonly(*mint, false));
        }
        metas
    }

    pub fn take_loan(&mut self, borrower: &Borrower, setup: &LoanSetup, amount: u64, tenure_seconds: i64) -> TxResult {
        let prices = self.price_accounts(&borrower.pubkey());
        let instruction = take_loan_ix(&borrower.pubkey(), &setup.cngn, &setup.borrower_cngn, amount, tenure_seconds, prices);
        send(&mut self.svm, &[instruction], &[&self.admin, &borrower.key])
    }
}

/// A market ready to lend: see `Env::loan_ready`.
pub struct LoanSetup {
    pub cngn: Pubkey,
    pub usdc: Pubkey,
    pub lender: Lender,
    pub borrower: Borrower,
    /// The borrower's cNGN token account (loans are paid here).
    pub borrower_cngn: Pubkey,
}

/// Lender liquidity in `Env::loan_ready`: 10,000,000 cNGN.
pub const POOL_CNGN: u64 = 10_000_000 * ONE_CNGN;

impl Env {
    /// cNGN market holding `POOL_CNGN` of lender liquidity; NGN at `NGN_USD` with a 0.1% spread;
    /// USDC (6 decimals, classic SPL Token) listed at exactly $1; and a borrower with 1,000 USDC
    /// deposited, so the borrow limit is $700.
    pub fn loan_ready() -> (Self, LoanSetup) {
        let (mut env, cngn) = Self::with_cngn_market();
        // The program reads a Switchboard result from slot 0 as never updated.
        env.svm.warp_to_slot(1_000);
        let lender = env.new_lender(&cngn, POOL_CNGN);
        env.deposit(&lender, &cngn, POOL_CNGN).unwrap();
        env.set_ngn_price(NGN_USD, NGN_SPREAD);

        let usdc = env.list_spl_collateral(6);
        env.set_pyth_price(&usdc, ONE_DOLLAR, 0);

        let borrower = env.new_borrower();
        env.deposit_collateral(&borrower, &usdc, 1_000 * ONE_USDC);
        let borrower_cngn = env.create_token_account(&cngn, &borrower.pubkey());
        (env, LoanSetup { cngn, usdc, lender, borrower, borrower_cngn })
    }

    /// Same shape as `loan_ready`, but the market never got a `create_promo_vault` call —
    /// `vault.rs` states a market can run without one. Liquidation and write-off must still
    /// work here (Task 7 fix round, Fix 1): the promo accounts are `Option` and only required
    /// when a position actually holds promo, which is never possible on a promo-less market.
    pub fn loan_ready_no_promo_vault() -> (Self, LoanSetup) {
        let mut env = Self::initialized();
        let cngn = env.create_mint(MintKind::CngnLike, 6);
        let admin = env.admin.pubkey();
        let create = create_market_ix(&admin, &cngn, &TOKEN_2022, default_market_params());
        send(&mut env.svm, &[create], &[&env.admin]).expect("create market");
        // The program reads a Switchboard result from slot 0 as never updated.
        env.svm.warp_to_slot(1_000);
        let lender = env.new_lender(&cngn, POOL_CNGN);
        env.deposit(&lender, &cngn, POOL_CNGN).unwrap();
        env.set_ngn_price(NGN_USD, NGN_SPREAD);

        let usdc = env.list_spl_collateral(6);
        env.set_pyth_price(&usdc, ONE_DOLLAR, 0);

        let borrower = env.new_borrower();
        env.deposit_collateral(&borrower, &usdc, 1_000 * ONE_USDC);
        let borrower_cngn = env.create_token_account(&cngn, &borrower.pubkey());
        (env, LoanSetup { cngn, usdc, lender, borrower, borrower_cngn })
    }
}

// ---- Repayment (Task 7) ----

pub fn repay_loan_ix(payer: &Pubkey, position_owner: &Pubkey, mint: &Pubkey, payer_token: &Pubkey, loan_id: u64, amount: u64) -> Instruction {
    ix(
        hodl_loans::instruction::RepayLoan { loan_id, amount },
        hodl_loans::accounts::RepayLoan {
            payer: *payer,
            access: access_pda(payer),
            position: position_pda(position_owner),
            market: market_pda(mint),
            mint: *mint,
            vault: market_vault_pda(mint),
            payer_token: *payer_token,
            token_program: TOKEN_2022,
        },
    )
}

impl Env {
    /// The borrower repays from its own cNGN account.
    pub fn repay(&mut self, setup: &LoanSetup, loan_id: u64, amount: u64) -> TxResult {
        let owner = setup.borrower.pubkey();
        let instruction = repay_loan_ix(&owner, &owner, &setup.cngn, &setup.borrower_cngn, loan_id, amount);
        send(&mut self.svm, &[instruction], &[&self.admin, &setup.borrower.key])
    }
}

// ---- Collateral withdrawals (Task 8) ----

/// `market_mint` is the borrowed market's mint; pass `None` when the position has no active loans.
pub fn withdraw_collateral_ix(
    owner: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
    owner_token: &Pubkey,
    market_mint: Option<&Pubkey>,
    amount: u64,
    prices: Vec<AccountMeta>,
) -> Instruction {
    let mut instruction = ix(
        hodl_loans::instruction::WithdrawCollateral { amount },
        hodl_loans::accounts::WithdrawCollateral {
            owner: *owner,
            access: access_pda(owner),
            config: config_pda(),
            position: position_pda(owner),
            collateral: collateral_pda(mint),
            mint: *mint,
            vault: collateral_vault_pda(mint),
            owner_token: *owner_token,
            market: market_mint.map(market_pda),
            ngn_feed: market_mint.map(|_| ngn_feed()),
            token_program: *token_program,
        },
    );
    instruction.accounts.extend(prices);
    instruction
}

/// One `(CollateralAsset, PriceUpdateV2)` pair per listed mint, in the order given.
pub fn price_pairs(mints: &[Pubkey]) -> Vec<AccountMeta> {
    mints
        .iter()
        .flat_map(|mint| {
            [
                AccountMeta::new_readonly(collateral_pda(mint), false),
                AccountMeta::new_readonly(pyth_account(mint), false),
            ]
        })
        .collect()
}

// ---- Reserve harvest (Task 9) ----

pub fn harvest_reserve_ix(admin: &Pubkey, mint: &Pubkey, destination: &Pubkey, amount: u64) -> Instruction {
    ix(
        hodl_loans::instruction::HarvestReserve { amount },
        hodl_loans::accounts::HarvestReserve {
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

// ---- Liquidation (Task 3) ----

/// Liquidation is open to anyone, so a liquidator needs no `Access` account.
pub struct Liquidator {
    pub key: Keypair,
    /// Its cNGN account, which funds repayments.
    pub cngn: Pubkey,
}

impl Liquidator {
    pub fn pubkey(&self) -> Pubkey {
        self.key.pubkey()
    }
}

#[allow(clippy::too_many_arguments)]
pub fn liquidate_ix(
    liquidator: &Pubkey,
    position_owner: &Pubkey,
    mint: &Pubkey,
    liquidator_token: &Pubkey,
    collateral_mint: &Pubkey,
    collateral_token_program: &Pubkey,
    liquidator_collateral: &Pubkey,
    loan_id: u64,
    amount: u64,
    prices: Vec<AccountMeta>,
) -> Instruction {
    let mut instruction = ix(
        hodl_loans::instruction::Liquidate { loan_id, amount },
        hodl_loans::accounts::Liquidate {
            liquidator: *liquidator,
            config: config_pda(),
            promo_vault: Some(promo_vault_pda(mint)),
            promo_vault_token: Some(promo_vault_token_pda(mint)),
            position: position_pda(position_owner),
            market: market_pda(mint),
            mint: *mint,
            vault: market_vault_pda(mint),
            liquidator_token: *liquidator_token,
            collateral: collateral_pda(collateral_mint),
            collateral_mint: *collateral_mint,
            collateral_vault: collateral_vault_pda(collateral_mint),
            liquidator_collateral: *liquidator_collateral,
            ngn_feed: ngn_feed(),
            token_program: TOKEN_2022,
            collateral_token_program: *collateral_token_program,
        },
    );
    instruction.accounts.extend(prices);
    instruction
}

/// Same as `liquidate_ix`, but with no promo accounts — for a market that never got a
/// `create_promo_vault` call, or for exercising the `PromoAccountsRequired` guard when a
/// promo-holding position's accounts are omitted.
#[allow(clippy::too_many_arguments)]
pub fn liquidate_ix_no_promo(
    liquidator: &Pubkey,
    position_owner: &Pubkey,
    mint: &Pubkey,
    liquidator_token: &Pubkey,
    collateral_mint: &Pubkey,
    collateral_token_program: &Pubkey,
    liquidator_collateral: &Pubkey,
    loan_id: u64,
    amount: u64,
    prices: Vec<AccountMeta>,
) -> Instruction {
    let mut instruction = ix(
        hodl_loans::instruction::Liquidate { loan_id, amount },
        hodl_loans::accounts::Liquidate {
            liquidator: *liquidator,
            config: config_pda(),
            promo_vault: None,
            promo_vault_token: None,
            position: position_pda(position_owner),
            market: market_pda(mint),
            mint: *mint,
            vault: market_vault_pda(mint),
            liquidator_token: *liquidator_token,
            collateral: collateral_pda(collateral_mint),
            collateral_mint: *collateral_mint,
            collateral_vault: collateral_vault_pda(collateral_mint),
            liquidator_collateral: *liquidator_collateral,
            ngn_feed: ngn_feed(),
            token_program: TOKEN_2022,
            collateral_token_program: *collateral_token_program,
        },
    );
    instruction.accounts.extend(prices);
    instruction
}

impl Env {
    /// A funded wallet holding `balance` cNGN, with no whitelist.
    pub fn new_liquidator(&mut self, cngn: &Pubkey, balance: u64) -> Liquidator {
        let key = self.funded_keypair();
        let account = self.create_token_account(cngn, &key.pubkey());
        self.mint_to(cngn, &account, balance);
        Liquidator { key, cngn: account }
    }

    /// Repays `amount` of `loan_id` against the position's current collateral prices.
    pub fn liquidate(
        &mut self,
        liquidator: &Liquidator,
        setup: &LoanSetup,
        collateral_mint: &Pubkey,
        liquidator_collateral: &Pubkey,
        loan_id: u64,
        amount: u64,
    ) -> TxResult {
        let owner = setup.borrower.pubkey();
        let program = self.mint_program(collateral_mint);
        let prices = self.price_accounts(&owner);
        let instruction = liquidate_ix(
            &liquidator.pubkey(),
            &owner,
            &setup.cngn,
            &liquidator.cngn,
            collateral_mint,
            &program,
            liquidator_collateral,
            loan_id,
            amount,
            prices,
        );
        send(&mut self.svm, &[instruction], &[&liquidator.key])
    }
}

// ---- Write-off (Task 4) ----

pub fn write_off_loan_ix(admin: &Pubkey, position_owner: &Pubkey, mint: &Pubkey, loan_id: u64, prices: Vec<AccountMeta>) -> Instruction {
    let mut instruction = ix(
        hodl_loans::instruction::WriteOffLoan { loan_id },
        hodl_loans::accounts::WriteOffLoan {
            admin: *admin,
            config: config_pda(),
            position: position_pda(position_owner),
            market: market_pda(mint),
            ngn_feed: ngn_feed(),
            mint: *mint,
            vault: market_vault_pda(mint),
            promo_vault: Some(promo_vault_pda(mint)),
            promo_vault_token: Some(promo_vault_token_pda(mint)),
            token_program: TOKEN_2022,
        },
    );
    instruction.accounts.extend(prices);
    instruction
}

/// Same as `write_off_loan_ix`, but with no promo accounts — for a market that never got a
/// `create_promo_vault` call, or for exercising the `PromoAccountsRequired` guard when a
/// promo-holding position's accounts are omitted.
pub fn write_off_loan_ix_no_promo(admin: &Pubkey, position_owner: &Pubkey, mint: &Pubkey, loan_id: u64, prices: Vec<AccountMeta>) -> Instruction {
    let mut instruction = ix(
        hodl_loans::instruction::WriteOffLoan { loan_id },
        hodl_loans::accounts::WriteOffLoan {
            admin: *admin,
            config: config_pda(),
            position: position_pda(position_owner),
            market: market_pda(mint),
            ngn_feed: ngn_feed(),
            mint: *mint,
            vault: market_vault_pda(mint),
            promo_vault: None,
            promo_vault_token: None,
            token_program: TOKEN_2022,
        },
    );
    instruction.accounts.extend(prices);
    instruction
}

impl Env {
    pub fn write_off(&mut self, setup: &LoanSetup, loan_id: u64) -> TxResult {
        let owner = setup.borrower.pubkey();
        let prices = self.price_accounts(&owner);
        let admin = self.admin.pubkey();
        let instruction = write_off_loan_ix(&admin, &owner, &setup.cngn, loan_id, prices);
        send(&mut self.svm, &[instruction], &[&self.admin])
    }
}

// ---- Compute budget measurement (test-only) ----

/// Like `send`, but returns the compute units the transaction consumed instead of `()`. Only
/// the compute-budget stress test needs the CU number; every other test uses `send`.
pub fn send_cu(svm: &mut LiteSVM, ixs: &[Instruction], signers: &[&Keypair]) -> Result<u64, String> {
    svm.expire_blockhash();
    let msg = Message::new_with_blockhash(ixs, Some(&signers[0].pubkey()), &svm.latest_blockhash());
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), signers).unwrap();
    svm.send_transaction(tx)
        .map(|m| m.compute_units_consumed)
        .map_err(|e| format!("{:?} cu={} logs: {:#?}", e.err, e.meta.compute_units_consumed, e.meta.logs))
}

// ---- Collateral kinds and xStock mints (Task 1) ----

/// A live Backed xStock carries 8 decimals; the scaled-UI multiplier is what makes a raw
/// balance a number of shares.
pub const XSTOCK_DECIMALS: u8 = 8;
pub const ONE_XSTOCK: u64 = 100_000_000;

/// Spec §8 launch values for an xStock: LTV 50%, threshold 75%, bonus 10%, pinned price.
pub fn xstock_collateral_params(mint: &Pubkey) -> hodl_loans::CollateralParams {
    hodl_loans::CollateralParams {
        ltv_bps: 5_000,
        liquidation_threshold_bps: 7_500,
        liquidation_bonus_bps: 1_000,
        ..default_collateral_params(mint)
    }
}

impl Env {
    /// A metadata-only Token-2022 mint listed as `Standard` collateral, priced at $1.
    pub fn list_t22_collateral(&mut self, decimals: u8) -> Pubkey {
        let mint = self.create_mint(MintKind::Token2022Plain, decimals);
        let instruction = list_collateral_ix(
            &self.admin.pubkey(),
            &mint,
            &TOKEN_2022,
            default_collateral_params(&mint),
            hodl_loans::CollateralKind::Standard,
        );
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("list collateral");
        self.set_pyth_price(&mint, ONE_DOLLAR, 0);
        mint
    }

    /// A live-shaped xStock mint, listed as `XStock` collateral and priced at `dollars` a
    /// share (Pyth exponent -8), with the multiplier at 1.
    pub fn list_xstock_collateral(&mut self, dollars: i64) -> Pubkey {
        let mint = self.create_mint(MintKind::XStock, XSTOCK_DECIMALS);
        let instruction = list_collateral_ix(
            &self.admin.pubkey(),
            &mint,
            &TOKEN_2022,
            xstock_collateral_params(&mint),
            hodl_loans::CollateralKind::XStock,
        );
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("list xstock");
        self.set_pyth_price(&mint, dollars * ONE_DOLLAR, 0);
        mint
    }

    pub fn set_mint_paused(&mut self, mint: &Pubkey, paused: bool) {
        let authority = self.admin.pubkey();
        let instruction = if paused {
            pausable::instruction::pause(&TOKEN_2022, mint, &authority, &[]).unwrap()
        } else {
            pausable::instruction::resume(&TOKEN_2022, mint, &authority, &[]).unwrap()
        };
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("set mint pause");
    }

    /// Points the mint's transfer hook at a program (or clears it).
    pub fn set_transfer_hook(&mut self, mint: &Pubkey, program: Option<Pubkey>) {
        let authority = self.admin.pubkey();
        let instruction =
            transfer_hook::instruction::update(&TOKEN_2022, mint, &authority, &[], program).unwrap();
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("set transfer hook");
    }

    /// Flips the mint to freezing new token accounts by default (the issuer's blocklist power).
    pub fn set_default_account_state(&mut self, mint: &Pubkey, state: AccountState) {
        let authority = self.admin.pubkey();
        let instruction = default_account_state::instruction::update_default_account_state(
            &TOKEN_2022, mint, &authority, &[], &state,
        )
        .unwrap();
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("set default account state");
    }

    /// The issuer moving tokens out of an account it does not own, through the permanent
    /// delegate — the risk the spec accepts for xStocks.
    pub fn delegate_burn(&mut self, mint: &Pubkey, from: &Pubkey, amount: u64) {
        let authority = self.admin.pubkey();
        let decimals = self.mint_decimals(mint);
        let instruction = spl_token_2022_interface::instruction::burn_checked(
            &TOKEN_2022, from, mint, &authority, &[], amount, decimals,
        )
        .unwrap();
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("delegate burn");
    }
}

// ---- The xStock multiplier (Task 3) ----

/// Serialized size of a legacy transaction carrying `instruction`, signed `signers` times.
/// Above `PACKET_DATA_SIZE` the client needs a v0 transaction with an address lookup table.
pub fn legacy_tx_size(instruction: &Instruction, payer: &Pubkey, signers: usize) -> usize {
    1 + 64 * signers + Message::new(std::slice::from_ref(instruction), Some(payer)).serialize().len()
}

/// The 1,232-byte packet limit, from the SDK rather than restated here: `solana-packet` derives
/// it as `1280 − 40 − 8` and was already in the dependency tree.
pub use solana_packet::PACKET_DATA_SIZE;

impl Env {
    /// Schedules the issuer's next multiplier. `effective_at` in the past takes effect at once.
    pub fn set_multiplier(&mut self, mint: &Pubkey, multiplier: f64, effective_at: i64) {
        let authority = self.admin.pubkey();
        let instruction = scaled_ui_amount::instruction::update_multiplier(
            &TOKEN_2022, mint, &authority, &[], multiplier, effective_at,
        )
        .unwrap();
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("update multiplier");
    }
}

// ---- The promo vault (Task 1) ----

pub fn promo_vault_pda(market_mint: &Pubkey) -> Pubkey {
    pda(&[b"promo_vault", market_pda(market_mint).as_ref()])
}

pub fn promo_vault_token_pda(market_mint: &Pubkey) -> Pubkey {
    pda(&[b"promo_vault_token", market_pda(market_mint).as_ref()])
}

pub fn create_promo_vault_ix(admin: &Pubkey, mint: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::CreatePromoVault {},
        hodl_loans::accounts::CreatePromoVault {
            admin: *admin,
            config: config_pda(),
            market: market_pda(mint),
            mint: *mint,
            promo_vault: promo_vault_pda(mint),
            vault: promo_vault_token_pda(mint),
            token_program: TOKEN_2022,
            system_program: system_program::ID,
        },
    )
}

pub fn fund_promo_vault_ix(admin: &Pubkey, mint: &Pubkey, source: &Pubkey, amount: u64) -> Instruction {
    ix(
        hodl_loans::instruction::FundPromoVault { amount },
        hodl_loans::accounts::FundPromoVault {
            admin: *admin,
            config: config_pda(),
            market: market_pda(mint),
            mint: *mint,
            promo_vault: promo_vault_pda(mint),
            vault: promo_vault_token_pda(mint),
            source: *source,
            token_program: TOKEN_2022,
        },
    )
}

pub fn withdraw_promo_vault_ix(admin: &Pubkey, mint: &Pubkey, destination: &Pubkey, amount: u64) -> Instruction {
    ix(
        hodl_loans::instruction::WithdrawPromoVault { amount },
        hodl_loans::accounts::WithdrawPromoVault {
            admin: *admin,
            config: config_pda(),
            market: market_pda(mint),
            mint: *mint,
            promo_vault: promo_vault_pda(mint),
            vault: promo_vault_token_pda(mint),
            destination: *destination,
            token_program: TOKEN_2022,
        },
    )
}

pub fn sweep_promo_excess_ix(admin: &Pubkey, mint: &Pubkey, destination: &Pubkey) -> Instruction {
    ix(
        hodl_loans::instruction::SweepPromoExcess {},
        hodl_loans::accounts::SweepPromoExcess {
            admin: *admin,
            config: config_pda(),
            market: market_pda(mint),
            mint: *mint,
            promo_vault: promo_vault_pda(mint),
            vault: promo_vault_token_pda(mint),
            destination: *destination,
            token_program: TOKEN_2022,
        },
    )
}

impl Env {
    /// A cNGN market whose promo vault holds `funded` cNGN.
    pub fn with_promo_vault(funded: u64) -> (Self, Pubkey) {
        let (mut env, cngn) = Self::with_cngn_market();
        let admin = env.admin.pubkey();
        if funded > 0 {
            let source = env.create_token_account(&cngn, &admin);
            env.mint_to(&cngn, &source, funded);
            let instruction = fund_promo_vault_ix(&admin, &cngn, &source, funded);
            send(&mut env.svm, &[instruction], &[&env.admin]).expect("fund promo vault");
        }
        (env, cngn)
    }

    pub fn promo_vault(&self, market_mint: &Pubkey) -> hodl_loans::PromoVault {
        self.fetch(&promo_vault_pda(market_mint))
    }

    /// A treasury-owned cNGN account, the only destination the sweeps and withdrawals accept.
    pub fn treasury_token(&mut self, mint: &Pubkey) -> Pubkey {
        let treasury = self.treasury.pubkey();
        self.create_token_account(mint, &treasury)
    }
}

// ---- Campaigns (Task 2) ----

pub fn campaign_pda(market_mint: &Pubkey, campaign_id: u64) -> Pubkey {
    pda(&[b"campaign", market_pda(market_mint).as_ref(), &campaign_id.to_le_bytes()])
}

pub fn create_campaign_ix(
    admin: &Pubkey,
    mint: &Pubkey,
    campaign_id: u64,
    budget: u64,
    redeem_until: i64,
) -> Instruction {
    ix(
        hodl_loans::instruction::CreateCampaign { campaign_id, budget, redeem_until },
        hodl_loans::accounts::CreateCampaign {
            admin: *admin,
            config: config_pda(),
            market: market_pda(mint),
            promo_vault: promo_vault_pda(mint),
            campaign: campaign_pda(mint, campaign_id),
            system_program: system_program::ID,
        },
    )
}

pub fn close_campaign_ix(admin: &Pubkey, mint: &Pubkey, campaign_id: u64) -> Instruction {
    ix(
        hodl_loans::instruction::CloseCampaign {},
        hodl_loans::accounts::CloseCampaign {
            admin: *admin,
            config: config_pda(),
            market: market_pda(mint),
            promo_vault: promo_vault_pda(mint),
            campaign: campaign_pda(mint, campaign_id),
        },
    )
}

impl Env {
    pub fn campaign(&self, market_mint: &Pubkey, campaign_id: u64) -> hodl_loans::Campaign {
        self.fetch(&campaign_pda(market_mint, campaign_id))
    }

    /// Creates campaign `id` with `budget`, redeemable for a year.
    pub fn create_campaign(&mut self, mint: &Pubkey, campaign_id: u64, budget: u64) {
        let admin = self.admin.pubkey();
        let until = self.now() + 365 * 86_400;
        let instruction = create_campaign_ix(&admin, mint, campaign_id, budget, until);
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("create campaign");
    }
}

// ---- Vouchers (Task 4) ----

pub const ED25519_PROGRAM: Pubkey = hodl_loans::voucher::ED25519_PROGRAM_ID;
pub fn instructions_sysvar() -> Pubkey {
    solana_instructions_sysvar::ID
}

pub fn voucher_pda(campaign: &Pubkey, nonce: u64) -> Pubkey {
    pda(&[b"voucher", campaign.as_ref(), &nonce.to_le_bytes()])
}

/// An Ed25519 program instruction proving `signer` signed `message`, in the layout the native
/// program reads: two header bytes, one 14-byte descriptor, then the signature, key and message.
/// Built by hand because no dependency here ships the SDK's builder.
pub fn ed25519_verify_ix(signer: &Keypair, message: &[u8]) -> Instruction {
    let signature_offset: u16 = 16;
    let public_key_offset = signature_offset + 64;
    let message_offset = public_key_offset + 32;
    let mut data = vec![1u8, 0u8];
    for value in [
        signature_offset,
        u16::MAX,
        public_key_offset,
        u16::MAX,
        message_offset,
        message.len() as u16,
        u16::MAX,
    ] {
        data.extend_from_slice(&value.to_le_bytes());
    }
    data.extend_from_slice(signer.sign_message(message).as_ref());
    data.extend_from_slice(signer.pubkey().as_ref());
    data.extend_from_slice(message);
    Instruction::new_with_bytes(ED25519_PROGRAM, &data, vec![])
}

pub fn redeem_promo_ix(
    payer: &Pubkey,
    owner: &Pubkey,
    mint: &Pubkey,
    campaign_id: u64,
    amount: u64,
    nonce: u64,
    voucher_expiry: i64,
) -> Instruction {
    let campaign = campaign_pda(mint, campaign_id);
    ix(
        hodl_loans::instruction::RedeemPromo { amount, nonce, voucher_expiry },
        hodl_loans::accounts::RedeemPromo {
            payer: *payer,
            owner: *owner,
            access: access_pda(owner),
            config: config_pda(),
            market: market_pda(mint),
            position: position_pda(owner),
            promo_vault: promo_vault_pda(mint),
            campaign,
            voucher_receipt: voucher_pda(&campaign, nonce),
            instructions_sysvar: instructions_sysvar(),
            system_program: system_program::ID,
        },
    )
}

pub fn close_voucher_receipt_ix(rent_payer: &Pubkey, campaign: &Pubkey, nonce: u64) -> Instruction {
    ix(
        hodl_loans::instruction::CloseVoucherReceipt {},
        hodl_loans::accounts::CloseVoucherReceipt {
            rent_payer: *rent_payer,
            voucher_receipt: voucher_pda(campaign, nonce),
        },
    )
}

impl Env {
    /// The message the promo signer must sign for this voucher.
    pub fn voucher_message(
        &self,
        mint: &Pubkey,
        campaign_id: u64,
        wallet: &Pubkey,
        amount: u64,
        nonce: u64,
        voucher_expiry: i64,
    ) -> Vec<u8> {
        hodl_loans::voucher::PromoVoucher::new(
            market_pda(mint),
            campaign_id,
            *wallet,
            amount,
            nonce,
            voucher_expiry,
        )
        .message()
        .unwrap()
    }

    /// Redeems a voucher: the Ed25519 proof first, then the redemption, in one transaction.
    pub fn redeem_promo(
        &mut self,
        borrower: &Borrower,
        mint: &Pubkey,
        campaign_id: u64,
        amount: u64,
        nonce: u64,
    ) -> TxResult {
        let expiry = self.now() + 86_400;
        self.redeem_voucher_signed_by(&self.promo_signer.insecure_clone(), borrower, mint, campaign_id, amount, nonce, expiry)
    }

    /// The same, with the signing key and expiry spelled out — for the cases where one of them
    /// is meant to be wrong.
    #[allow(clippy::too_many_arguments)]
    pub fn redeem_voucher_signed_by(
        &mut self,
        signer: &Keypair,
        borrower: &Borrower,
        mint: &Pubkey,
        campaign_id: u64,
        amount: u64,
        nonce: u64,
        voucher_expiry: i64,
    ) -> TxResult {
        let owner = borrower.pubkey();
        let message = self.voucher_message(mint, campaign_id, &owner, amount, nonce, voucher_expiry);
        let admin = self.admin.pubkey();
        let instructions = vec![
            ed25519_verify_ix(signer, &message),
            redeem_promo_ix(&admin, &owner, mint, campaign_id, amount, nonce, voucher_expiry),
        ];
        send(&mut self.svm, &instructions, &[&self.admin, &borrower.key])
    }

    pub fn voucher_receipt(&self, campaign: &Pubkey, nonce: u64) -> hodl_loans::VoucherReceipt {
        self.fetch(&voucher_pda(campaign, nonce))
    }
}

// ---- Promo in the health check (Task 5) ----

impl Env {
    /// `Env::loan_ready` plus a funded promo vault and one open campaign, so a borrower can
    /// hold promo while borrowing against real collateral.
    pub fn promo_ready() -> (Self, LoanSetup) {
        let (mut env, setup) = Self::loan_ready();
        let admin = env.admin.pubkey();
        let source = env.create_token_account(&setup.cngn, &admin);
        env.mint_to(&setup.cngn, &source, 10_000_000 * ONE_CNGN);
        let fund = fund_promo_vault_ix(&admin, &setup.cngn, &source, 10_000_000 * ONE_CNGN);
        send(&mut env.svm, &[fund], &[&env.admin]).expect("fund promo vault");
        env.create_campaign(&setup.cngn, 1, 5_000_000 * ONE_CNGN);
        (env, setup)
    }

    /// Raises the per-position promo ceiling, for the cases that need more than the default.
    pub fn set_max_promo_per_position(&mut self, mint: &Pubkey, max: u64) {
        let params = hodl_loans::MarketParams { max_promo_per_position: max, ..default_market_params() };
        let instruction = update_market_params_ix(&self.admin.pubkey(), mint, params);
        send(&mut self.svm, &[instruction], &[&self.admin]).expect("update market params");
    }
}

// ---- The promo cap (Task 8) ----

/// `set_promo_cap` re-checks every listed asset, so the caller passes them all, in ascending
/// key order.
pub fn set_promo_cap_ix(admin: &Pubkey, promo_cap_bps: u16, assets: &[Pubkey]) -> Instruction {
    let mut instruction = ix(
        hodl_loans::instruction::SetPromoCap { promo_cap_bps },
        hodl_loans::accounts::SetPromoCap { admin: *admin, config: config_pda() },
    );
    let mut sorted: Vec<Pubkey> = assets.iter().map(collateral_pda).collect();
    sorted.sort();
    instruction.accounts.extend(sorted.into_iter().map(|k| AccountMeta::new_readonly(k, false)));
    instruction
}

/// Serialises a `CollateralAsset` (discriminator + fields), for planting one at an arbitrary
/// key with `Env::set_account_data` — used to forge a look-alike asset in tests.
pub fn collateral_asset_bytes(asset: &hodl_loans::CollateralAsset) -> Vec<u8> {
    let mut data = Vec::new();
    asset.try_serialize(&mut data).unwrap();
    data
}
