use anchor_lang::prelude::*;
use anchor_spl::token_2022::spl_token_2022::{
    extension::{BaseStateWithExtensions, ExtensionType, StateWithExtensions},
    state::Mint as MintState,
};

use crate::errors::HodlError;

/// Extensions a borrowable (market) mint may carry. The Solana cNGN mint uses exactly
/// these (checked 2026-09-17): its issuer can move tokens through the permanent delegate,
/// which is an accepted issuer risk.
pub const MARKET_MINT_EXTENSIONS: &[ExtensionType] = &[
    ExtensionType::MetadataPointer,
    ExtensionType::TokenMetadata,
    ExtensionType::PermanentDelegate,
];

/// Extensions a `Standard` collateral mint may carry (spec §14): metadata only.
pub const STANDARD_COLLATERAL_EXTENSIONS: &[ExtensionType] =
    &[ExtensionType::MetadataPointer, ExtensionType::TokenMetadata];

/// Extension types on a mint. Classic SPL Token mints have none.
pub fn mint_extension_types(mint: &AccountInfo) -> Result<Vec<ExtensionType>> {
    if *mint.owner != anchor_spl::token_2022::ID {
        return Ok(Vec::new());
    }
    let data = mint.try_borrow_data()?;
    let state = StateWithExtensions::<MintState>::unpack(&data)?;
    Ok(state.get_extension_types()?)
}

pub fn require_allowed_extensions(mint: &AccountInfo, allowed: &[ExtensionType]) -> Result<()> {
    for extension in mint_extension_types(mint)? {
        require!(allowed.contains(&extension), HodlError::UnsupportedMintExtension);
    }
    Ok(())
}
