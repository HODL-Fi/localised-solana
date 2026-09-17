mod common;

use common::*;
use solana_signer::Signer;
use spl_token_2022_interface::extension::ExtensionType;

#[test]
fn program_loads_with_admin_as_upgrade_authority() {
    let env = Env::new();
    assert_eq!(env.upgrade_authority(), Some(env.admin.pubkey()));
}

#[test]
fn creates_mints_token_accounts_and_balances() {
    let mut env = Env::new();
    let cngn = env.create_mint(MintKind::CngnLike, 6);
    assert_eq!(env.mint_program(&cngn), TOKEN_2022);
    assert_eq!(
        env.mint_extensions(&cngn),
        vec![ExtensionType::PermanentDelegate, ExtensionType::MetadataPointer]
    );
    let usdc = env.create_mint(MintKind::SplToken, 6);
    assert_eq!(env.mint_program(&usdc), SPL_TOKEN);

    let owner = solana_keypair::Keypair::new();
    let account = env.create_token_account(&cngn, &owner.pubkey());
    env.mint_to(&cngn, &account, 1_000);
    assert_eq!(env.token_balance(&account), 1_000);
    assert_eq!(env.token_owner(&account), owner.pubkey());
}

#[test]
fn warp_moves_the_clock() {
    let mut env = Env::new();
    let before = env.now();
    env.warp_seconds(3_600);
    assert_eq!(env.now(), before + 3_600);
}
