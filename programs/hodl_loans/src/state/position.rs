use anchor_lang::prelude::*;

use crate::constants::{MAX_COLLATERAL_SLOTS, MAX_LOAN_SLOTS};
use crate::math::loan::LoanTerms;

/// One collateral holding. `amount == 0` means the slot is free.
#[zero_copy]
#[derive(Debug, PartialEq, Eq)]
pub struct CollateralSlot {
    pub mint: Pubkey,
    pub amount: u64,
}

/// One fixed-term loan. `active == 0` means the slot is free.
#[zero_copy]
#[derive(Debug, PartialEq, Eq)]
pub struct LoanSlot {
    pub id: u64,
    pub principal: u64,
    pub original_principal: u64,
    pub repaid: u64,
    pub originated_at: i64,
    pub interest_anchor: i64,
    pub tenure_seconds: i64,
    pub rate_bps: u16,
    pub penalty_rate_bps: u16,
    pub reserve_factor_bps: u16,
    pub active: u8,
    pub _padding: u8,
}

impl LoanSlot {
    pub fn is_active(&self) -> bool {
        self.active != 0
    }

    pub fn terms(&self) -> LoanTerms {
        LoanTerms {
            principal: self.principal,
            originated_at: self.originated_at,
            interest_anchor: self.interest_anchor,
            tenure_seconds: self.tenure_seconds,
            rate_bps: self.rate_bps,
            penalty_rate_bps: self.penalty_rate_bps,
        }
    }
}

/// A borrower's collateral and loans. Zero-copy with fixed offsets so bots can scan it.
/// No `Option` fields: `market == Pubkey::default()` means no loan has been taken yet.
#[account(zero_copy)]
#[derive(Debug)]
pub struct Position {
    pub version: u8,
    /// Canonical, written once by `open_position` (`init`, so `ctx.bumps.position` is always
    /// `find_program_address`'s result) and never touched again. The seven other position-PDA
    /// sites — `take_loan`, `close_position`, `deposit_collateral`, `withdraw_collateral`,
    /// `redeem_promo`, `expire_promo`, `revoke_promo` — read it back via
    /// `bump = position.load()?.bump` instead of re-deriving it, which depends on this field
    /// never holding anything but the canonical bump for as long as `open_position` stays the
    /// only writer.
    pub bump: u8,
    pub _padding: [u8; 6],
    pub owner: Pubkey,
    pub rent_payer: Pubkey,
    /// The market this position's loans come from.
    pub market: Pubkey,
    pub next_loan_id: u64,
    pub promo_balance: u64,
    pub promo_last_activity_at: i64,
    pub collateral: [CollateralSlot; MAX_COLLATERAL_SLOTS],
    pub loans: [LoanSlot; MAX_LOAN_SLOTS],
    pub reserved: [u8; 64],
}

impl Position {
    pub fn collateral_index(&self, mint: &Pubkey) -> Option<usize> {
        self.collateral.iter().position(|s| s.amount > 0 && s.mint == *mint)
    }

    pub fn free_collateral_index(&self) -> Option<usize> {
        self.collateral.iter().position(|s| s.amount == 0)
    }

    pub fn has_collateral(&self) -> bool {
        self.collateral.iter().any(|s| s.amount > 0)
    }

    pub fn loan_index(&self, id: u64) -> Option<usize> {
        self.loans.iter().position(|l| l.is_active() && l.id == id)
    }

    pub fn free_loan_index(&self) -> Option<usize> {
        self.loans.iter().position(|l| !l.is_active())
    }

    pub fn has_active_loans(&self) -> bool {
        self.loans.iter().any(|l| l.is_active())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_fixed_size_without_padding_surprises() {
        assert_eq!(std::mem::size_of::<CollateralSlot>(), 40);
        assert_eq!(std::mem::size_of::<LoanSlot>(), 64);
        assert_eq!(std::mem::size_of::<Position>(), 8 + 32 * 3 + 8 * 3 + 40 * 8 + 64 * 10 + 64);
    }

    #[test]
    fn slot_helpers_find_used_and_free_slots() {
        let mut p: Position = bytemuck::Zeroable::zeroed();
        let mint = Pubkey::new_unique();
        assert_eq!(p.free_collateral_index(), Some(0));
        assert_eq!(p.collateral_index(&mint), None);
        p.collateral[0] = CollateralSlot { mint, amount: 5 };
        assert_eq!(p.collateral_index(&mint), Some(0));
        assert_eq!(p.free_collateral_index(), Some(1));
        assert!(p.has_collateral());

        assert_eq!(p.free_loan_index(), Some(0));
        p.loans[0].active = 1;
        p.loans[0].id = 7;
        assert_eq!(p.loan_index(7), Some(0));
        assert_eq!(p.loan_index(8), None);
        assert!(p.has_active_loans());
    }
}
