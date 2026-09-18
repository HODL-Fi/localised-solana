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

/// The mint policy where the protocol takes on **new** exposure: `list_collateral`, which
/// creates the collateral vault, and `deposit_collateral`, which adds to a position. Deposit
/// re-checks rather than trusting the listing — listing is a moment, an issuer's powers are
/// permanent — and refusing either only declines new business, so the check is the full one.
pub fn require_collateral_mint_on_entry(mint: &AccountInfo, kind: CollateralKind) -> Result<()> {
    match kind {
        CollateralKind::Standard => require_allowed_extensions(mint, STANDARD_COLLATERAL_EXTENSIONS),
        CollateralKind::XStock => require_xstock_mint_on_entry(mint),
    }
}

/// The mint policy where collateral **leaves** the protocol: `withdraw_collateral`,
/// `liquidate` and `sweep_collateral_excess`. Deliberately narrower than the entry policy, and
/// not keyed on the kind, because the single clause that survives applies to every mint.
///
/// **What is kept — the transfer hook must name no program.** A hook program is arbitrary
/// issuer CPI running inside our `transfer_checked`, which is a reentrancy surface. Keeping it
/// costs nothing in availability: a hooked transfer would fail in the token program anyway,
/// for want of the hook's extra account metas. This check only turns that into a clean
/// `UnsupportedMintExtension`.
///
/// **What is dropped, and why.** On the way out a failing check can only ever trap collateral,
/// because the token program is already the authority on whether a transfer is legal.
///
/// - **`DefaultAccountState`** is the state *new* accounts are initialized in — flipping it to
///   `Frozen` does not freeze accounts that already exist. None of the exit paths creates a
///   token account; only `list_collateral` does, for the vault. So enforcing it here would
///   turn an issuer action spec §14 calls expected (Backed holds the extension so it can
///   switch on blocklist-style compliance later) into a permanent seal on a live position in
///   both directions, with no recovery path short of the issuer reverting the flag:
///   `write_off_loan` needs collateral worth less than `bad_debt_dust_usd`, and
///   `delist_collateral` needs an empty vault.
/// - **`Pausable.paused`** is not checked by the entry policy either, deliberately and for the
///   same reason (spec §11): a paused xStock simply fails its transfer in the token program,
///   and the liquidator picks another collateral. `DefaultAccountState` gets that treatment.
/// - **`ScaledUiAmount` presence** is an entry clause: it makes the asset priceable. A
///   withdrawal with no active loans reads no price at all, so requiring it out here would
///   block an exit to protect a valuation nobody is doing.
/// - **The allowed-extension set** is settled before the mint exists for every entry but
///   three: those extensions are written by `initialize_*` instructions that run on an
///   *uninitialized* mint. The exceptions are `TokenMetadata`, `TokenGroup` and
///   `TokenGroupMember`, each of which is written into a **live** mint, reallocating it.
///   Length is not what separates them: `ExtensionType::sized()` is `false` for
///   `TokenMetadata` alone, while the two group entries are fixed-length `Pod` types added
///   through `alloc_and_serialize`, whose documented job is to pack a fixed-length extension
///   and realloc the account to fit.
///
///   Those same three are exactly the case where re-checking on exit would trap without
///   protecting, and none of them is dangerous: `TokenMetadata` is on both allowlists, so it
///   changes nothing, and the two group entries are on neither, so an issuer holding the mint
///   authority could seal every vault holding the asset by attaching a group label. Only the
///   *values* above are worth re-reading on the way out.
pub fn require_collateral_mint_on_exit(mint: &AccountInfo) -> Result<()> {
    if *mint.owner != anchor_spl::token_2022::ID {
        return Ok(());
    }
    let data = mint.try_borrow_data()?;
    let state = StateWithExtensions::<MintState>::unpack(&data)
        .map_err(|_| HodlError::UnsupportedMintExtension)?;
    require_no_hook_program(&state)
}

/// An `XStock` mint's allowed extensions, plus the three settings whose *values* matter:
/// a ScaledUiAmount extension must be present (to price the collateral at valuation time),
/// a transfer hook must name no program, and accounts must not be frozen by default.
fn require_xstock_mint_on_entry(mint: &AccountInfo) -> Result<()> {
    require_allowed_extensions(mint, XSTOCK_COLLATERAL_EXTENSIONS)?;
    let data = mint.try_borrow_data()?;
    let state = StateWithExtensions::<MintState>::unpack(&data)
        .map_err(|_| HodlError::UnsupportedMintExtension)?;
    // An XStock with no multiplier would be listable but unpriceable: reading it at
    // valuation time would fail. Listing is the one moment we can reject it cheaply.
    let _config = state
        .get_extension::<ScaledUiAmountConfig>()
        .map_err(|_| HodlError::UnsupportedMintExtension)?;
    require_no_hook_program(&state)?;
    if let Ok(default_state) = state.get_extension::<DefaultAccountState>() {
        // Frozen by default would freeze the collateral vault `list_collateral` is about to
        // create, and is the issuer's signal that it wants new exposure to stop.
        require!(
            default_state.state == u8::from(AccountState::Initialized),
            HodlError::UnsupportedMintExtension
        );
    }
    Ok(())
}

/// The one clause both policies share: a hook program would run issuer code inside every
/// transfer of the collateral.
fn require_no_hook_program(state: &StateWithExtensions<MintState>) -> Result<()> {
    if let Ok(hook) = state.get_extension::<TransferHook>() {
        require!(
            Option::<Pubkey>::from(hook.program_id).is_none(),
            HodlError::UnsupportedMintExtension
        );
    }
    Ok(())
}
