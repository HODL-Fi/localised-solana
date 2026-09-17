use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod math;
pub mod oracle;
pub mod state;
pub mod token;
pub mod valuation;

pub use constants::*;
pub use errors::*;
pub use events::*;
pub use instructions::*;
pub use state::*;

declare_id!("J9sKAhm2EhdJQ3bHeP2KUCxqZ4cYdBc65C3RDr4JjGEd");

#[program]
pub mod hodl_loans {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>, args: InitializeArgs) -> Result<()> {
        instructions::handle_initialize(ctx, args)
    }

    pub fn propose_admin(ctx: Context<AdminConfig>, proposed: Pubkey) -> Result<()> {
        instructions::handle_propose_admin(ctx, proposed)
    }

    pub fn accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
        instructions::handle_accept_admin(ctx)
    }

    pub fn set_guardian(ctx: Context<AdminConfig>, guardian: Pubkey) -> Result<()> {
        instructions::handle_set_role(ctx, Role::Guardian, guardian)
    }

    pub fn set_whitelister(ctx: Context<AdminConfig>, whitelister: Pubkey) -> Result<()> {
        instructions::handle_set_role(ctx, Role::Whitelister, whitelister)
    }

    pub fn set_promo_signer(ctx: Context<AdminConfig>, promo_signer: Pubkey) -> Result<()> {
        instructions::handle_set_role(ctx, Role::PromoSigner, promo_signer)
    }

    pub fn set_treasury(ctx: Context<AdminConfig>, treasury: Pubkey) -> Result<()> {
        instructions::handle_set_role(ctx, Role::Treasury, treasury)
    }

    pub fn whitelist(ctx: Context<Whitelist>, wallet: Pubkey) -> Result<()> {
        instructions::handle_whitelist(ctx, wallet)
    }

    pub fn blacklist(ctx: Context<Blacklist>, wallet: Pubkey) -> Result<()> {
        instructions::handle_blacklist(ctx, wallet)
    }

    pub fn unblacklist(ctx: Context<Unblacklist>, wallet: Pubkey) -> Result<()> {
        instructions::handle_unblacklist(ctx, wallet)
    }

    pub fn create_market(ctx: Context<CreateMarket>, params: MarketParams) -> Result<()> {
        instructions::handle_create_market(ctx, params)
    }

    pub fn update_market_params(ctx: Context<UpdateMarketParams>, params: MarketParams) -> Result<()> {
        instructions::handle_update_market_params(ctx, params)
    }

    pub fn set_market_paused(ctx: Context<SetMarketPaused>, paused: bool) -> Result<()> {
        instructions::handle_set_market_paused(ctx, paused)
    }

    pub fn sweep_market_excess(ctx: Context<SweepMarketExcess>) -> Result<()> {
        instructions::handle_sweep_market_excess(ctx)
    }

    pub fn list_collateral(ctx: Context<ListCollateral>, params: CollateralParams) -> Result<()> {
        instructions::handle_list_collateral(ctx, params)
    }

    pub fn update_collateral_params(ctx: Context<UpdateCollateralParams>, params: CollateralParams) -> Result<()> {
        instructions::handle_update_collateral_params(ctx, params)
    }

    pub fn set_collateral_paused(ctx: Context<SetCollateralPaused>, paused: bool) -> Result<()> {
        instructions::handle_set_collateral_paused(ctx, paused)
    }

    pub fn delist_collateral(ctx: Context<DelistCollateral>) -> Result<()> {
        instructions::handle_delist_collateral(ctx)
    }

    pub fn sweep_collateral_excess(ctx: Context<SweepCollateralExcess>) -> Result<()> {
        instructions::handle_sweep_collateral_excess(ctx)
    }

    pub fn harvest_reserve(ctx: Context<HarvestReserve>, amount: u64) -> Result<()> {
        instructions::handle_harvest_reserve(ctx, amount)
    }

    pub fn open_position(ctx: Context<OpenPosition>) -> Result<()> {
        instructions::handle_open_position(ctx)
    }

    pub fn close_position(ctx: Context<ClosePosition>) -> Result<()> {
        instructions::handle_close_position(ctx)
    }

    pub fn deposit_collateral(ctx: Context<DepositCollateral>, amount: u64) -> Result<()> {
        instructions::handle_deposit_collateral(ctx, amount)
    }

    pub fn withdraw_collateral<'info>(ctx: Context<'info, WithdrawCollateral<'info>>, amount: u64) -> Result<()> {
        instructions::handle_withdraw_collateral(ctx, amount)
    }

    pub fn take_loan<'info>(ctx: Context<'info, TakeLoan<'info>>, amount: u64, tenure_seconds: i64) -> Result<()> {
        instructions::handle_take_loan(ctx, amount, tenure_seconds)
    }

    pub fn repay_loan(ctx: Context<RepayLoan>, loan_id: u64, amount: u64) -> Result<()> {
        instructions::handle_repay_loan(ctx, loan_id, amount)
    }

    pub fn deposit_liquidity(ctx: Context<DepositLiquidity>, amount: u64) -> Result<()> {
        instructions::handle_deposit_liquidity(ctx, amount)
    }

    pub fn withdraw_liquidity(ctx: Context<WithdrawLiquidity>, amount: u64) -> Result<()> {
        instructions::handle_withdraw_liquidity(ctx, amount)
    }
}
