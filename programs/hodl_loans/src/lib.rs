use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod math;
pub mod oracle;
pub mod state;
pub mod token;

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

    pub fn deposit_liquidity(ctx: Context<DepositLiquidity>, amount: u64) -> Result<()> {
        instructions::handle_deposit_liquidity(ctx, amount)
    }

    pub fn withdraw_liquidity(ctx: Context<WithdrawLiquidity>, amount: u64) -> Result<()> {
        instructions::handle_withdraw_liquidity(ctx, amount)
    }
}
