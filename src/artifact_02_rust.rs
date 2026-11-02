use anchor_lang::prelude::*;
use anchor_lang::solana_program::hash::hash;

declare_id!("ZNScre8or1111111111111111111111111111111111");

pub const MAX_NAME_LEN: usize = 32;
pub const MAX_URI_LEN: usize = 96;
pub const MAX_SPLIT_RECIPIENTS: usize = 3;
pub const BASE_RENT_SECONDS: i64 = 31_536_000; // 1 standard year

#[program]
pub mod zns_creator {
    use super::*;

    /// Initialize Global Registrar Parameters
    pub fn initialize_registrar(
        ctx: Context<InitializeRegistrar>,
        base_fee_short: u64, // <= 4 chars
        base_fee_long: u64,  // > 4 chars
    ) -> Result<()> {
        let registrar = &mut ctx.accounts.registrar;
        registrar.admin = ctx.accounts.admin.key();
        registrar.base_fee_short = base_fee_short;
        registrar.base_fee_long = base_fee_long;
        registrar.total_domains = 0;
        registrar.bump = ctx.bumps.registrar;
        Ok(())
    }

    /// Register a new creator handle (e.g. "solana") with reverse lookup linkage
    pub fn register_domain(
        ctx: Context<RegisterDomain>,
        name: String,
        years: u8,
    ) -> Result<()> {
        require!(years > 0 && years <= 10, ZNSError::InvalidLeaseDuration);
        require!(
            !name.is_empty() && name.len() <= MAX_NAME_LEN,
            ZNSError::NameLengthExceeded
        );
        require!(
            name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            ZNSError::InvalidCharacters
        );

        let registrar = &mut ctx.accounts.registrar;
        let clock = Clock::get()?;
        let name_bytes = name.as_bytes();
        let name_hash = hash(name_bytes).to_bytes();

        // Calculate linear-tiered pricing
        let fee_per_year = if name.len() <= 4 {
            registrar.base_fee_short
        } else {
            registrar.base_fee_long
        };
        let total_fee = fee_per_year.checked_mul(years as u64).ok_or(ZNSError::MathOverflow)?;

        // Charge registration fee to the treasury
        if total_fee > 0 {
            anchor_lang::solana_program::program::invoke(
                &anchor_lang::solana_program::system_instruction::transfer(
                    &ctx.accounts.owner.key(),
                    &ctx.accounts.admin.key(),
                    total_fee,
                ),
                &[
                    ctx.accounts.owner.to_account_info(),
                    ctx.accounts.admin.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;
        }

        // Initialize Name Record
        let name_record = &mut ctx.accounts.name_record;
        name_record.owner = ctx.accounts.owner.key();
        name_record.name_hash = name_hash;
        name_record.expires_at = clock.unix_timestamp.checked_add(
            BASE_RENT_SECONDS.checked_mul(years as i64).ok_or(ZNSError::MathOverflow)?
        ).ok_or(ZNSError::MathOverflow)?;
        name_record.bump = ctx.bumps.name_record;

        // Initialize or overwrite Reverse Lookup Record
        let reverse_record = &mut ctx.accounts.reverse_record;
        reverse_record.owner = ctx.accounts.owner.key();
        let mut name_buf = [0u8; MAX_NAME_LEN];
        name_buf[..name_bytes.len()].copy_from_slice(name_bytes);
        reverse_record.name = name_buf;
        reverse_record.name_len = name_bytes.len() as u8;
        reverse_record.bump = ctx.bumps.reverse_record;

        registrar.total_domains = registrar.total_domains.checked_add(1).ok_or(ZNSError::MathOverflow)?;

        emit!(DomainRegisteredEvent {
            owner: name_record.owner,
            name_hash,
            expires_at: name_record.expires_at,
        });

        Ok(())
    }

    /// Set Creator Hub Profile, Metadata URI, and Payment Splits
    pub fn update_creator_profile(
        ctx: Context<UpdateCreatorProfile>,
        metadata_uri: String,
        splits: Vec<SplitShare>,
    ) -> Result<()> {
        require!(metadata_uri.len() <= MAX_URI_LEN, ZNSError::UriLengthExceeded);
        require!(splits.len() <= MAX_SPLIT_RECIPIENTS, ZNSError::TooManySplits);

        // Validate splits total 10,000 basis points (100.00%)
        let total_basis_points: u16 = splits.iter().map(|s| s.basis_points).sum();
        require!(total_basis_points == 10_000, ZNSError::InvalidSplitBasisPoints);

        let profile = &mut ctx.accounts.creator_profile;
        profile.name_record = ctx.accounts.name_record.key();
        
        let mut uri_buf = [0u8; MAX_URI_LEN];
        let uri_bytes = metadata_uri.as_bytes();
        uri_buf[..uri_bytes.len()].copy_from_slice(uri_bytes);
        profile.metadata_uri = uri_buf;
        profile.metadata_len = uri_bytes.len() as u8;

        profile.splits_count = splits.len() as u8;
        for (i, split) in splits.iter().enumerate() {
            profile.splits[i] = *split;
        }
        profile.bump = ctx.bumps.creator_profile;

        emit!(ProfileUpdatedEvent {
            name_record: profile.name_record,
            splits_count: profile.splits_count,
        });

        Ok(())
    }

    /// Direct Tip to Creator Handle with Program-Enforced Royalty Split
    pub fn resolve_and_tip<'info>(
        ctx: Context<'_, '_, '_, 'info, ResolveAndTip<'info>>,
        amount_lamports: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let name_record = &ctx.accounts.name_record;
        require!(name_record.expires_at > clock.unix_timestamp, ZNSError::DomainExpired);
        require!(amount_lamports > 0, ZNSError::ZeroTipAmount);

        let profile = &ctx.accounts.creator_profile;
        let count = profile.splits_count as usize;
        
        // Remaining accounts must provide exact destinations matching the profile splits
        require!(ctx.remaining_accounts.len() >= count, ZNSError::MissingSplitAccounts);

        let mut distributed_total: u64 = 0;
        for i in 0..count {
            let split_def = &profile.splits[i];
            let target_account = &ctx.remaining_accounts[i];
            require_keys_eq!(target_account.key(), split_def.recipient, ZNSError::SplitRecipientMismatch);

            // Compute split cut
            let share_lamports = (amount_lamports as u128)
                .checked_mul(split_def.basis_points as u128)
                .ok_or(ZNSError::MathOverflow)?
                .checked_div(10_000)
                .ok_or(ZNSError::MathOverflow)? as u64;

            if share_lamports > 0 {
                anchor_lang::solana_program::program::invoke(
                    &anchor_lang::solana_program::system_instruction::transfer(
                        &ctx.accounts.tipper.key(),
                        &target_account.key(),
                        share_lamports,
                    ),
                    &[
                        ctx.accounts.tipper.to_account_info(),
                        target_account.to_account_info(),
                        ctx.accounts.system_program.to_account_info(),
                    ],
                )?;
                distributed_total = distributed_total.checked_add(share_lamports).ok_or(ZNSError::MathOverflow)?;
            }
        }

        // Remainder routing (dust handling) sent to primary owner
        let remainder = amount_lamports.checked_sub(distributed_total).ok_or(ZNSError::MathOverflow)?;
        if remainder > 0 {
            anchor_lang::solana_program::program::invoke(
                &anchor_lang::solana_program::system_instruction::transfer(
                    &ctx.accounts.tipper.key(),
                    &ctx.accounts.owner.key(),
                    remainder,
                ),
                &[
                    ctx.accounts.tipper.to_account_info(),
                    ctx.accounts.owner.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;
        }

        emit!(TipRoutedEvent {
            name_record: ctx.accounts.name_record.key(),
            sender: ctx.accounts.tipper.key(),
            total_lamports: amount_lamports,
        });

        Ok(())
    }
}

// -----------------------------------------------------------------------------
// ACCOUNT CONTEXTS
// -----------------------------------------------------------------------------

#[derive(Accounts)]
pub struct InitializeRegistrar<'info> {
    #[account(
        init,
        payer = admin,
        space = 8 + Registrar::INIT_SPACE,
        seeds = [b"registrar"],
        bump
    )]
    pub registrar: Account<'info, Registrar>,
    #[account(mut)]
    pub admin: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(name: String, years: u8)]
pub struct RegisterDomain<'info> {
    #[account(
        mut,
        seeds = [b"registrar"],
        bump = registrar.bump
    )]
    pub registrar: Account<'info, Registrar>,

    #[account(
        init,
        payer = owner,
        space = 8 + NameRecord::INIT_SPACE,
        seeds = [b"zns", hash(name.as_bytes()).to_bytes().as_ref()],
        bump
    )]
    pub name_record: Account<'info, NameRecord>,

    #[account(
        init_if_needed,
        payer = owner,
        space = 8 + ReverseRecord::INIT_SPACE,
        seeds = [b"reverse", owner.key().as_ref()],
        bump
    )]
    pub reverse_record: Account<'info, ReverseRecord>,

    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: Validated against registrar.admin
    #[account(mut, address = registrar.admin)]
    pub admin: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateCreatorProfile<'info> {
    #[account(
        has_one = owner @ ZNSError::UnauthorizedOwner
    )]
    pub name_record: Account<'info, NameRecord>,

    #[account(
        init_if_needed,
        payer = owner,
        space = 8 + CreatorProfile::INIT_SPACE,
        seeds = [b"profile", name_record.key().as_ref()],
        bump
    )]
    pub creator_profile: Account<'info, CreatorProfile>,

    #[account(mut)]
    pub owner: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ResolveAndTip<'info> {
    #[account(
        has_one = owner
    )]
    pub name_record: Account<'info, NameRecord>,

    #[account(
        seeds = [b"profile", name_record.key().as_ref()],
        bump = creator_profile.bump
    )]
    pub creator_profile: Account<'info, CreatorProfile>,

    /// CHECK: Primary owner receives remainder dust and identity verification
    #[account(mut)]
    pub owner: AccountInfo<'info>,

    #[account(mut)]
    pub tipper: Signer<'info>,
    pub system_program: Program<'info, System>,
}

// -----------------------------------------------------------------------------
// DATA STRUCTURES
// -----------------------------------------------------------------------------

#[account]
#[derive(InitSpace)]
pub struct Registrar {
    pub admin: Pubkey,
    pub base_fee_short: u64,
    pub base_fee_long: u64,
    pub total_domains: u64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct NameRecord {
    pub owner: Pubkey,
    pub name_hash: [u8; 32],
    pub expires_at: i64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct ReverseRecord {
    pub owner: Pubkey,
    pub name: [u8; MAX_NAME_LEN],
    pub name_len: u8,
    pub bump: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Default)]
pub struct SplitShare {
    pub recipient: Pubkey,
    pub basis_points: u16, // e.g., 5000 = 50.00%
}

#[account]
#[derive(InitSpace)]
pub struct CreatorProfile {
    pub name_record: Pubkey,
    pub metadata_uri: [u8; MAX_URI_LEN],
    pub metadata_len: u8,
    pub splits_count: u8,
    pub splits: [SplitShare; MAX_SPLIT_RECIPIENTS],
    pub bump: u8,
}

// -----------------------------------------------------------------------------
// EVENTS & ERRORS
// -----------------------------------------------------------------------------

#[event]
pub struct DomainRegisteredEvent {
    pub owner: Pubkey,
    pub name_hash: [u8; 32],
    pub expires_at: i64,
}

#[event]
pub struct ProfileUpdatedEvent {
    pub name_record: Pubkey,
    pub splits_count: u8,
}

#[event]
pub struct TipRoutedEvent {
    pub name_record: Pubkey,
    pub sender: Pubkey,
    pub total_lamports: u64,
}

#[error_code]
pub enum ZNSError {
    #[msg("Lease duration must be between 1 and 10 years")]
    InvalidLeaseDuration,
    #[msg("Domain name length exceeds 32 bytes")]
    NameLengthExceeded,
    #[msg("Contains invalid ASCII characters")]
    InvalidCharacters,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("Caller is not the registered owner of this domain")]
    UnauthorizedOwner,
    #[msg("Metadata URI exceeds 96 bytes")]
    UriLengthExceeded,
    #[msg("Exceeded maximum of 3 split recipients")]
    TooManySplits,
    #[msg("Split allocation basis points must total exactly 10,000")]
    InvalidSplitBasisPoints,
    #[msg("Domain lease has expired")]
    DomainExpired,
    #[msg("Tip amount must be greater than zero lamports")]
    ZeroTipAmount,
    #[msg("Insufficient remaining accounts supplied for royalty split")]
    MissingSplitAccounts,
    #[msg("Target account key does not match configured split recipient")]
    SplitRecipientMismatch,
}