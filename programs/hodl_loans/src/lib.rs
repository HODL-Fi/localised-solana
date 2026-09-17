use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod math;
pub mod state;

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
}
