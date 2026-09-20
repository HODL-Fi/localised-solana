use anchor_lang::prelude::*;
use solana_instructions_sysvar::{load_current_index_checked, load_instruction_at_checked};

use crate::constants::VOUCHER_DOMAIN;
use crate::errors::HodlError;

/// The native Ed25519 signature-verification program. Hard-coded rather than pulled from a
/// crate: it is a fixed address, and `anchor_lang`'s `solana_program` shim does not re-export
/// `ed25519_program`. Matches `solana_sdk_ids::ed25519_program::ID`.
pub const ED25519_PROGRAM_ID: Pubkey = pubkey!("Ed25519SigVerify111111111111111111111111111");

/// Spec §12's voucher message. The program builds this itself from the redeeming instruction's
/// arguments and its own context, then requires the Ed25519 instruction to have signed exactly
/// these bytes. The domain, the program id and the market are therefore pinned by construction
/// rather than parsed out of something the caller supplied: a signature the promo signer
/// produced for a different program, a different market or a different format cannot be
/// replayed here.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct PromoVoucher {
    pub domain: String,
    pub program_id: Pubkey,
    pub market: Pubkey,
    pub campaign_id: u64,
    pub wallet: Pubkey,
    pub amount: u64,
    pub nonce: u64,
    pub voucher_expiry: i64,
}

impl PromoVoucher {
    pub fn new(
        market: Pubkey,
        campaign_id: u64,
        wallet: Pubkey,
        amount: u64,
        nonce: u64,
        voucher_expiry: i64,
    ) -> Self {
        Self {
            domain: VOUCHER_DOMAIN.to_string(),
            program_id: crate::ID,
            market,
            campaign_id,
            wallet,
            amount,
            nonce,
            voucher_expiry,
        }
    }

    /// The exact bytes the promo signer must have signed.
    pub fn message(&self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.serialize(&mut bytes)?;
        Ok(bytes)
    }
}

/// Offsets inside one Ed25519 program signature descriptor, in the order the native program
/// reads them. The layout is two header bytes followed by one 14-byte descriptor per signature.
const ED25519_HEADER_LEN: usize = 2;
const ED25519_DESCRIPTOR_LEN: usize = 14;
/// `u16::MAX` in an instruction-index field means "this instruction's own data".
const THIS_INSTRUCTION: u16 = u16::MAX;

/// Spec §12 step 2: require that an Ed25519 program instruction *earlier in this transaction*
/// proves `signer` signed exactly `message`.
///
/// The native Ed25519 program does the cryptography; a bad signature fails its own instruction
/// and takes the whole transaction with it. What is left for us is making sure such an
/// instruction exists and covers the right key and the right bytes — which is where the care
/// goes, because the descriptor can point anywhere:
///
/// - exactly one signature, so a second descriptor cannot smuggle in an unchecked one;
/// - all three data pointers must be `u16::MAX`, so the key and the message must live in the
///   Ed25519 instruction's own data and cannot reference bytes from another instruction that
///   the native program verified against a different key;
/// - the key and the message bytes must match, read with bounds checks rather than slicing.
pub fn require_ed25519_signature(
    instructions_sysvar: &AccountInfo,
    signer: &Pubkey,
    message: &[u8],
) -> Result<()> {
    let current = load_current_index_checked(instructions_sysvar)?;
    for index in 0..current {
        let instruction = load_instruction_at_checked(index as usize, instructions_sysvar)?;
        if instruction.program_id != ED25519_PROGRAM_ID {
            continue;
        }
        if ed25519_instruction_covers(&instruction.data, signer, message) {
            return Ok(());
        }
    }
    Err(HodlError::InvalidVoucherSignature.into())
}

/// Whether one Ed25519 instruction's data is a single self-contained signature by `signer` over
/// `message`. Every read is bounds-checked: the descriptor's offsets are attacker-supplied.
fn ed25519_instruction_covers(data: &[u8], signer: &Pubkey, message: &[u8]) -> bool {
    if data.len() < ED25519_HEADER_LEN + ED25519_DESCRIPTOR_LEN || data[0] != 1 {
        return false;
    }
    let field = |at: usize| -> u16 {
        u16::from_le_bytes([data[ED25519_HEADER_LEN + at], data[ED25519_HEADER_LEN + at + 1]])
    };
    let public_key_offset = field(4) as usize;
    let message_offset = field(8) as usize;
    let message_size = field(10) as usize;
    // Every pointer must stay inside this instruction: indices 2, 6 and 12 are the signature,
    // public-key and message instruction indices.
    if field(2) != THIS_INSTRUCTION || field(6) != THIS_INSTRUCTION || field(12) != THIS_INSTRUCTION {
        return false;
    }
    if message_size != message.len() {
        return false;
    }
    let Some(key_bytes) = data.get(public_key_offset..public_key_offset + 32) else {
        return false;
    };
    let Some(message_bytes) = data.get(message_offset..message_offset + message_size) else {
        return false;
    };
    key_bytes == signer.as_ref() && message_bytes == message
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIGNER: Pubkey = Pubkey::new_from_array([9; 32]);

    /// The layout `solana_sdk`'s Ed25519 instruction builder produces: header, one descriptor,
    /// then the signature, the public key and the message.
    fn ed25519_data(signer: &Pubkey, message: &[u8]) -> Vec<u8> {
        let signature_offset = ED25519_HEADER_LEN + ED25519_DESCRIPTOR_LEN;
        let public_key_offset = signature_offset + 64;
        let message_offset = public_key_offset + 32;
        let mut data = vec![1u8, 0u8];
        for value in [
            signature_offset as u16,
            THIS_INSTRUCTION,
            public_key_offset as u16,
            THIS_INSTRUCTION,
            message_offset as u16,
            message.len() as u16,
            THIS_INSTRUCTION,
        ] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        data.extend_from_slice(&[0u8; 64]);
        data.extend_from_slice(signer.as_ref());
        data.extend_from_slice(message);
        data
    }

    #[test]
    fn a_well_formed_instruction_covers_its_key_and_message() {
        let data = ed25519_data(&SIGNER, b"hello");
        assert!(ed25519_instruction_covers(&data, &SIGNER, b"hello"));
        // A different key or a different message is not covered.
        assert!(!ed25519_instruction_covers(&data, &Pubkey::new_from_array([8; 32]), b"hello"));
        assert!(!ed25519_instruction_covers(&data, &SIGNER, b"hellp"));
        // Nor is a prefix: the length is compared before the bytes.
        assert!(!ed25519_instruction_covers(&data, &SIGNER, b"hell"));
    }

    #[test]
    fn descriptors_that_point_elsewhere_are_rejected() {
        // A second signature the caller could leave unchecked.
        let mut two = ed25519_data(&SIGNER, b"hello");
        two[0] = 2;
        assert!(!ed25519_instruction_covers(&two, &SIGNER, b"hello"));

        // A message that lives in another instruction: the native program would verify it
        // against bytes we never see, so the pointer must be self-referential.
        for index_field in [2usize, 6, 12] {
            let mut elsewhere = ed25519_data(&SIGNER, b"hello");
            elsewhere[ED25519_HEADER_LEN + index_field] = 0;
            elsewhere[ED25519_HEADER_LEN + index_field + 1] = 0;
            assert!(!ed25519_instruction_covers(&elsewhere, &SIGNER, b"hello"));
        }
    }

    #[test]
    fn malformed_data_is_rejected_rather_than_panicking() {
        assert!(!ed25519_instruction_covers(&[], &SIGNER, b"hello"));
        assert!(!ed25519_instruction_covers(&[1, 0], &SIGNER, b"hello"));
        // An offset past the end of the data must not slice out of bounds.
        let mut past_end = ed25519_data(&SIGNER, b"hello");
        past_end[ED25519_HEADER_LEN + 8] = 0xff;
        past_end[ED25519_HEADER_LEN + 9] = 0xff;
        assert!(!ed25519_instruction_covers(&past_end, &SIGNER, b"hello"));
        let mut key_past_end = ed25519_data(&SIGNER, b"hello");
        key_past_end[ED25519_HEADER_LEN + 4] = 0xff;
        key_past_end[ED25519_HEADER_LEN + 5] = 0xff;
        assert!(!ed25519_instruction_covers(&key_past_end, &SIGNER, b"hello"));
    }

    #[test]
    fn the_message_pins_the_domain_the_program_and_the_market() {
        let market = Pubkey::new_from_array([3; 32]);
        let wallet = Pubkey::new_from_array([4; 32]);
        let voucher = PromoVoucher::new(market, 7, wallet, 500, 11, 1_800_000_000);
        assert_eq!(voucher.domain, VOUCHER_DOMAIN);
        assert_eq!(voucher.program_id, crate::ID);

        // Borsh puts the domain first, length-prefixed, so a message for another format cannot
        // collide with one of ours.
        let bytes = voucher.message().unwrap();
        assert_eq!(&bytes[..4], &(VOUCHER_DOMAIN.len() as u32).to_le_bytes());
        assert_eq!(&bytes[4..4 + VOUCHER_DOMAIN.len()], VOUCHER_DOMAIN.as_bytes());

        // Every field is covered: changing any one changes the bytes.
        for other in [
            PromoVoucher::new(Pubkey::new_from_array([5; 32]), 7, wallet, 500, 11, 1_800_000_000),
            PromoVoucher::new(market, 8, wallet, 500, 11, 1_800_000_000),
            PromoVoucher::new(market, 7, Pubkey::new_from_array([6; 32]), 500, 11, 1_800_000_000),
            PromoVoucher::new(market, 7, wallet, 501, 11, 1_800_000_000),
            PromoVoucher::new(market, 7, wallet, 500, 12, 1_800_000_000),
            PromoVoucher::new(market, 7, wallet, 500, 11, 1_800_000_001),
        ] {
            assert_ne!(other.message().unwrap(), bytes);
        }
    }
}
