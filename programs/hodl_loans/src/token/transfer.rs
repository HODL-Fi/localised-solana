use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, TransferChecked};

/// Transfer signed by the wallet that owns `from`.
pub fn transfer_from_user<'info>(
    token_program: Pubkey,
    mint: AccountInfo<'info>,
    decimals: u8,
    from: AccountInfo<'info>,
    to: AccountInfo<'info>,
    authority: AccountInfo<'info>,
    amount: u64,
) -> Result<()> {
    token_interface::transfer_checked(
        CpiContext::new(token_program, TransferChecked { from, mint, to, authority }),
        amount,
        decimals,
    )
}

/// Transfer out of a program vault, signed by the vault's PDA owner.
#[allow(clippy::too_many_arguments)]
pub fn transfer_from_vault<'info>(
    token_program: Pubkey,
    mint: AccountInfo<'info>,
    decimals: u8,
    from: AccountInfo<'info>,
    to: AccountInfo<'info>,
    authority: AccountInfo<'info>,
    amount: u64,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    token_interface::transfer_checked(
        CpiContext::new_with_signer(token_program, TransferChecked { from, mint, to, authority }, signer_seeds),
        amount,
        decimals,
    )
}
