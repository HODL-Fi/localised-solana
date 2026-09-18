use anchor_lang::prelude::*;
use anchor_spl::token_2022::spl_token_2022::{
    extension::{
        default_account_state::DefaultAccountState, scaled_ui_amount::ScaledUiAmountConfig,
        transfer_hook::TransferHook, BaseStateWithExtensions, ExtensionType, StateWithExtensions,
    },
    state::{AccountState, Mint as MintState},
};

use crate::errors::HodlError;
use crate::state::CollateralKind;

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

/// Extensions an `XStock` collateral mint may carry (spec §14). The live Backed xStock mints
/// (AAPLX, TSLAX, NVDAX, checked 2026-09-18) carry exactly this set, `TransferHook` with no
/// program and `DefaultAccountState` at `Initialized`. Accepted issuer risks: the permanent
/// delegate can move or burn tokens from the custody vault, a pause blocks every transfer of
/// the asset, and the issuer may later freeze newly created accounts by default.
pub const XSTOCK_COLLATERAL_EXTENSIONS: &[ExtensionType] = &[
    ExtensionType::MetadataPointer,
    ExtensionType::TokenMetadata,
    ExtensionType::PermanentDelegate,
    ExtensionType::Pausable,
    ExtensionType::ScaledUiAmount,
    ExtensionType::ConfidentialTransferMint,
    ExtensionType::TransferHook,
    ExtensionType::DefaultAccountState,
];

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

/// The mint policy for a collateral kind, checked at listing and again before every transfer
/// of the asset, so an issuer that turns something on after listing causes a clean failure
/// instead of an unnoticed one.
pub fn require_collateral_mint(mint: &AccountInfo, kind: CollateralKind) -> Result<()> {
    match kind {
        CollateralKind::Standard => require_allowed_extensions(mint, STANDARD_COLLATERAL_EXTENSIONS),
        CollateralKind::XStock => require_xstock_mint(mint),
    }
}

/// An `XStock` mint's allowed extensions, plus the three settings whose *values* matter:
/// a ScaledUiAmount extension must be present (to price the collateral at valuation time),
/// a transfer hook must name no program, and accounts must not be frozen by default.
fn require_xstock_mint(mint: &AccountInfo) -> Result<()> {
    require_allowed_extensions(mint, XSTOCK_COLLATERAL_EXTENSIONS)?;
    let data = mint.try_borrow_data()?;
    let state = StateWithExtensions::<MintState>::unpack(&data)
        .map_err(|_| HodlError::UnsupportedMintExtension)?;
    // An XStock with no multiplier would be listable but unpriceable: reading it at
    // valuation time would fail. Listing is the one moment we can reject it cheaply.
    let _config = state
        .get_extension::<ScaledUiAmountConfig>()
        .map_err(|_| HodlError::UnsupportedMintExtension)?;
    if let Ok(hook) = state.get_extension::<TransferHook>() {
        // A hook program would run issuer code inside every transfer of the collateral.
        require!(
            Option::<Pubkey>::from(hook.program_id).is_none(),
            HodlError::UnsupportedMintExtension
        );
    }
    if let Ok(default_state) = state.get_extension::<DefaultAccountState>() {
        // Frozen by default would freeze any token account created after the flip.
        require!(
            default_state.state == u8::from(AccountState::Initialized),
            HodlError::UnsupportedMintExtension
        );
    }
    Ok(())
}
