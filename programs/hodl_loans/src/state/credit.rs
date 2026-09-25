use anchor_lang::prelude::*;

use crate::constants::ACCOUNT_VERSION;
use crate::errors::HodlError;

/// One record per borrower, holding the facts of their credit history: how many loans they have
/// repaid in full, and how many were closed by liquidation.
///
/// **Facts only — no score.** A score is a versioned function over these facts and the events
/// beside them, and it belongs off chain where it can be revised without an upgrade. Putting
/// weights, decay or tiers in here would freeze a judgement call into the program.
///
/// **Append-only, forever.** Nothing in the program decrements either counter, and nothing resets
/// or closes the account. That is the whole value of the record: a borrower cannot clear their
/// history by closing a position and opening a new one, because the address is the identity.
/// Derivable by anyone from a wallet address alone — `["credit", owner]` — so an indexer needs no
/// registry to find it.
///
/// Note what the counters do NOT distinguish: a loan repaid late still counts as completed. The
/// lateness is in the event (`days_late`, `penalty_paid`), not the counter, because a counter that
/// tried to encode degrees of lateness would be a score.
#[account]
#[derive(InitSpace)]
pub struct CreditRecord {
    pub version: u8,
    pub bump: u8,
    /// The borrower. Always equal to the `owner` of the position whose loan moved the counter, and
    /// the seed the account is derived from, so the two cannot disagree.
    pub owner: Pubkey,
    /// Loans whose principal reached zero through repayment.
    pub loans_completed: u64,
    /// Loans whose principal reached zero through liquidation.
    pub loans_defaulted: u64,
    pub reserved: [u8; 62],
}

impl CreditRecord {
    /// Set `owner` and `bump` the first time the record is touched, and leave them alone after.
    ///
    /// `init_if_needed` hands us a zeroed account on creation and the existing one afterwards, with
    /// nothing to distinguish the two — so this keys off `owner` being unset rather than trusting a
    /// flag. Re-running it on a live record must be a no-op: if it overwrote `owner` it would be a
    /// way to point one borrower's record at another.
    pub fn ensure_initialized(&mut self, owner: Pubkey, bump: u8) {
        if self.owner == Pubkey::default() {
            self.version = ACCOUNT_VERSION;
            self.bump = bump;
            self.owner = owner;
        }
    }

    pub fn record_completed(&mut self) -> Result<u64> {
        self.loans_completed = self.loans_completed.checked_add(1).ok_or(HodlError::MathOverflow)?;
        Ok(self.loans_completed)
    }

    pub fn record_defaulted(&mut self) -> Result<u64> {
        self.loans_defaulted = self.loans_defaulted.checked_add(1).ok_or(HodlError::MathOverflow)?;
        Ok(self.loans_defaulted)
    }
}

/// Whole days past due at `now`, or 0 when the loan is not late.
pub fn days_late(originated_at: i64, tenure_seconds: i64, now: i64) -> u32 {
    let due_at = originated_at.saturating_add(tenure_seconds);
    if now <= due_at {
        return 0;
    }
    (now.saturating_sub(due_at) / 86_400).clamp(0, u32::MAX as i64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    #[test]
    fn lateness_counts_whole_days_only() {
        // Due exactly now, and one second before due, are both on time.
        assert_eq!(days_late(0, 30 * DAY, 30 * DAY), 0);
        assert_eq!(days_late(0, 30 * DAY, 30 * DAY - 1), 0);
        // Part of a day late is not yet a day late.
        assert_eq!(days_late(0, 30 * DAY, 30 * DAY + DAY - 1), 0);
        assert_eq!(days_late(0, 30 * DAY, 30 * DAY + DAY), 1);
        assert_eq!(days_late(0, 30 * DAY, 30 * DAY + 400 * DAY), 400);
    }

    #[test]
    fn lateness_does_not_overflow_or_go_negative() {
        // A clock far behind the origination must read 0, not wrap.
        assert_eq!(days_late(i64::MAX, 30 * DAY, 0), 0);
        // And a clock far ahead saturates at the type's ceiling rather than wrapping.
        assert_eq!(days_late(0, 0, i64::MAX), u32::MAX);
    }

    #[test]
    fn the_first_touch_sets_identity_and_later_ones_cannot_change_it() {
        let first = Pubkey::new_unique();
        let other = Pubkey::new_unique();
        let mut record = CreditRecord {
            version: 0, bump: 0, owner: Pubkey::default(),
            loans_completed: 0, loans_defaulted: 0, reserved: [0; 62],
        };
        record.ensure_initialized(first, 7);
        assert_eq!((record.owner, record.bump, record.version), (first, 7, ACCOUNT_VERSION));

        // The guard that matters: re-running it must not repoint the record at another borrower.
        record.ensure_initialized(other, 9);
        assert_eq!((record.owner, record.bump), (first, 7));
    }

    #[test]
    fn counters_only_go_up() {
        let mut record = CreditRecord {
            version: 1, bump: 1, owner: Pubkey::new_unique(),
            loans_completed: 0, loans_defaulted: 0, reserved: [0; 62],
        };
        assert_eq!(record.record_completed().unwrap(), 1);
        assert_eq!(record.record_completed().unwrap(), 2);
        assert_eq!(record.record_defaulted().unwrap(), 1);
        assert_eq!((record.loans_completed, record.loans_defaulted), (2, 1));

        // Saturating would silently stop counting; overflow is an error instead.
        record.loans_completed = u64::MAX;
        assert!(record.record_completed().is_err());
    }
}

#[cfg(test)]
mod layout {
    use super::*;

    /// The spec fixed this account at 120 bytes on chain (8 discriminator + 112). `version` and
    /// `bump` came out of the 64 reserved bytes rather than on top of them, so the size is the one
    /// the spec published and a future field still has 62 bytes to come from.
    #[test]
    fn init_space_is_pinned_so_new_fields_come_out_of_the_padding() {
        assert_eq!(CreditRecord::INIT_SPACE, 112);
        assert_eq!(8 + CreditRecord::INIT_SPACE, 120);
    }
}
