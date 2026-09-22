use anchor_lang::prelude::*;

/// Program errors. Append new variants at the end only: codes are 6000 + position.
#[error_code]
pub enum HodlError {
    #[msg("Wallet is not whitelisted")]
    NotWhitelisted,
    #[msg("Wallet is blacklisted")]
    Blacklisted,
    #[msg("Signer is not authorized for this action")]
    Unauthorized,
    #[msg("Market is paused")]
    MarketPaused,
    #[msg("Collateral asset is paused")]
    CollateralPaused,
    #[msg("Price is stale")]
    StalePrice,
    #[msg("Price confidence interval is too wide")]
    PriceConfidenceTooWide,
    #[msg("Price account does not match the expected asset")]
    PriceAccountMismatch,
    #[msg("Price is invalid")]
    InvalidPrice,
    #[msg("Mint has an unsupported extension")]
    UnsupportedMintExtension,
    #[msg("Parameters are invalid")]
    InvalidParameters,
    #[msg("Position would be unhealthy")]
    Unhealthy,
    #[msg("Position is not liquidatable")]
    NotLiquidatable,
    #[msg("Utilization cap exceeded")]
    UtilizationCapExceeded,
    #[msg("Not enough cash in the market")]
    InsufficientCash,
    #[msg("No free loan slot")]
    NoFreeLoanSlot,
    #[msg("No free collateral slot")]
    NoFreeCollateralSlot,
    #[msg("Loan not found")]
    LoanNotFound,
    #[msg("Deposit cap exceeded")]
    DepositCapExceeded,
    #[msg("Amount is too small")]
    AmountTooSmall,
    #[msg("Tenure is out of range")]
    TenureOutOfRange,
    #[msg("Repayment does not cover interest and penalty")]
    RepaymentBelowInterest,
    #[msg("Liquidation would repay zero principal")]
    ZeroPrincipalRepaid,
    #[msg("Deposit would mint zero shares")]
    ZeroShares,
    #[msg("Not enough shares")]
    InsufficientShares,
    #[msg("Collateral is still in use")]
    CollateralStillInUse,
    #[msg("Position is not empty")]
    PositionNotEmpty,
    #[msg("Write-off is not allowed")]
    WriteOffNotAllowed,
    #[msg("Voucher signature is invalid")]
    InvalidVoucherSignature,
    #[msg("Voucher has expired")]
    VoucherExpired,
    #[msg("Campaign is inactive")]
    CampaignInactive,
    #[msg("Campaign budget exceeded")]
    CampaignBudgetExceeded,
    #[msg("Promo cap exceeded")]
    PromoCapExceeded,
    #[msg("Promo has not expired")]
    PromoNotExpired,
    #[msg("Promo vault has insufficient free funds")]
    PromoVaultInsufficient,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Position's loans belong to a different market")]
    MarketMismatch,
    #[msg("Not enough collateral in the position")]
    InsufficientCollateral,
    #[msg("Position holds promo: the market and promo vault accounts are required")]
    PromoAccountsRequired,
    #[msg("Promo vault token account does not match the promo vault")]
    PromoVaultMismatch,
    #[msg("Collateral vault token account does not match the collateral asset")]
    CollateralVaultMismatch,
    #[msg("The protocol already lists the maximum number of collateral assets")]
    CollateralLimitReached,
    #[msg("Voucher expiry is later than its campaign's redeem_until")]
    VoucherOutlivesCampaign,
}
