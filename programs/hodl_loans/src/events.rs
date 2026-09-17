use anchor_lang::prelude::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Guardian,
    Whitelister,
    PromoSigner,
    Treasury,
}

#[event]
pub struct ConfigInitialized {
    pub admin: Pubkey,
    pub guardian: Pubkey,
    pub whitelister: Pubkey,
    pub promo_signer: Pubkey,
    pub treasury: Pubkey,
}

#[event]
pub struct AdminProposed {
    pub admin: Pubkey,
    pub proposed: Pubkey,
}

#[event]
pub struct AdminAccepted {
    pub old_admin: Pubkey,
    pub new_admin: Pubkey,
}

#[event]
pub struct RoleUpdated {
    pub role: Role,
    pub old: Pubkey,
    pub new: Pubkey,
}
