use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    hash::hash,
    program::invoke,
    system_instruction,
};

declare_id!("ZNS1111111111111111111111111111111111111111");

pub const MAX_NAME_LEN: usize = 32;
pub const MAX_CONTENT_HASH_LEN: usize = 64;
pub const BASIS_POINTS_DIVISOR: u16 = 10_000;
pub const PROTOCOL_FEE_BPS: u16 = 200; // 2% protocol fee on tipping/registration

#[program]
pub mod zns_solana {
    use super::*;

    /// Initializes the root protocol state with treasury authority and fees.
    pub fn initialize(
        ctx: Context<Initialize>,
        fee_basis_points: u16,
        base_registration_fee: u64,
    ) -> Result<()> {
        require!(fee_basis_points <= 1000, ZnsError::FeeTooHigh); // Max 10%
        let config = &mut ctx.accounts.config;
        config.authority = ctx.accounts.authority.key();
        config.treasury = ctx.accounts.treasury.key();
        config.fee_basis_points = fee_basis_points;
        config.base_registration_fee = base_registration_fee;
        config.total_registered = 0;
        config.bump = ctx.bumps.config;
        Ok(())
    }

    /// Registers a unique, canonical domain under the ZNS creator space.
    pub fn register_domain(
        ctx: Context<RegisterDomain>,
        name: String,
        duration_seconds: i64,
        content_hash: Option<String>,
        collaborator: Option<Pubkey>,
        split_basis_points: u16,
    ) -> Result<()> {
        validate_domain_name(&name)?;
        require!(duration_seconds >= 86_400, ZnsError::InvalidDuration); // Min 1 day
        require!(split_basis_points <= BASIS_POINTS_DIVISOR, ZnsError::InvalidSplit);

        let clock = Clock::get()?;
        let config = &mut ctx.accounts.config;

        // Calculate dynamic registration fee based on length tiers
        let dynamic_fee = calculate_registration_fee(name.len(), config.base_registration_fee)?;

        // Execute payment transfer from applicant to treasury
        invoke(
            &system_instruction::transfer(
                &ctx.accounts.payer.key(),
                &ctx.accounts.treasury.key(),
                dynamic_fee,
            ),
            &[
                ctx.accounts.payer.to_account_info(),
                ctx.accounts.treasury.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        let domain_record = &mut ctx.accounts.domain_record;
        domain_record.owner = ctx.accounts.owner.key();
        domain_record.name = name.clone();
        domain_record.name_hash = hash(name.as_bytes()).to_bytes();
        domain_record.created_at = clock.unix_timestamp;
        domain_record.expires_at = clock.unix_timestamp.checked_add(duration_seconds).ok_or(ZnsError::MathOverflow)?;
        domain_record.bump = ctx.bumps.domain_record;
        domain_record.collaborator = collaborator.unwrap_or(ctx.accounts.owner.key());
        domain_record.collaborator_split_bps = split_basis_points;

        if let Some(hash_str) = content_hash {
            require!(hash_str.len() <= MAX_CONTENT_HASH_LEN, ZnsError::ContentHashTooLong);
            domain_record.content_hash = hash_str;
        } else {
            domain_record.content_hash = String::new();
        }

        config.total_registered = config.total_registered.checked_add(1).ok_or(ZnsError::MathOverflow)?;

        emit!(DomainRegisteredEvent {
            domain_name: name,
            owner: domain_record.owner,
            expires_at: domain_record.expires_at,
            fee_paid: dynamic_fee,
        });

        Ok(())
    }

    /// Sets or updates the 1-to-1 reverse resolution pointer for the sender.
    pub fn set_reverse_record(ctx: Context<SetReverseRecord>, name: String) -> Result<()> {
        let name_hash = hash(name.as_bytes()).to_bytes();
        require!(
            ctx.accounts.domain_record.name_hash == name_hash,
            ZnsError::DomainMismatch
        );
        require!(
            ctx.accounts.domain_record.owner == ctx.accounts.owner.key(),
            ZnsError::Unauthorized
        );

        let reverse_record = &mut ctx.accounts.reverse_record;
        reverse_record.owner = ctx.accounts.owner.key();
        reverse_record.domain_name = name;
        reverse_record.bump = ctx.bumps.reverse_record;

        emit!(ReverseRecordUpdatedEvent {
            owner: reverse_record.owner,
            domain_name: reverse_record.domain_name.clone(),
        });

        Ok(())
    }

    /// Issues a delegated sub-domain for fans or platform projects.
    pub fn create_subdomain(
        ctx: Context<CreateSubdomain>,
        subdomain_prefix: String,
        beneficiary: Pubkey,
    ) -> Result<()> {
        validate_domain_name(&subdomain_prefix)?;
        let parent = &ctx.accounts.parent_domain;

        let clock = Clock::get()?;
        require!(parent.expires_at > clock.unix_timestamp, ZnsError::DomainExpired);

        let sub = &mut ctx.accounts.subdomain_record;
        sub.parent_domain = parent.key();
        sub.owner = beneficiary;
        sub.subdomain_prefix = subdomain_prefix.clone();
        sub.created_at = clock.unix_timestamp;
        sub.bump = ctx.bumps.subdomain_record;

        emit!(SubdomainCreatedEvent {
            parent: parent.key(),
            subdomain: subdomain_prefix,
            beneficiary,
        });

        Ok(())
    }

    /// Direct atomic creator tipping engine with co-creator splits and platform fees.
    pub fn tip_creator(ctx: Context<TipCreator>, tip_amount: u64) -> Result<()> {
        require!(tip_amount > 0, ZnsError::ZeroTipAmount);

        let domain = &ctx.accounts.domain_record;
        let clock = Clock::get()?;
        require!(domain.expires_at > clock.unix_timestamp, ZnsError::DomainExpired);

        // Calculate platform protocol fee
        let config = &ctx.accounts.config;
        let platform_fee = (tip_amount as u128)
            .checked_mul(config.fee_basis_points as u128)
            .ok_or(ZnsError::MathOverflow)?
            .checked_div(BASIS_POINTS_DIVISOR as u128)
            .ok_or(ZnsError::MathOverflow)? as u64;

        let post_fee_amount = tip_amount.checked_sub(platform_fee).ok_or(ZnsError::MathOverflow)?;

        // Calculate co-creator split
        let collaborator_cut = (post_fee_amount as u128)
            .checked_mul(domain.collaborator_split_bps as u128)
            .ok_or(ZnsError::MathOverflow)?
            .checked_div(BASIS_POINTS_DIVISOR as u128)
            .ok_or(ZnsError::MathOverflow)? as u64;

        let primary_owner_cut = post_fee_amount
            .checked_sub(collaborator_cut)
            .ok_or(ZnsError::MathOverflow)?;

        // 1. Transfer platform fee to Treasury
        if platform_fee > 0 {
            invoke(
                &system_instruction::transfer(
                    &ctx.accounts.tipper.key(),
                    &ctx.accounts.treasury.key(),
                    platform_fee,
                ),
                &[
                    ctx.accounts.tipper.to_account_info(),
                    ctx.accounts.treasury.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;
        }

        // 2. Transfer primary cut to domain owner
        invoke(
            &system_instruction::transfer(
                &ctx.accounts.tipper.key(),
                &ctx.accounts.domain_owner.key(),
                primary_owner_cut,
            ),
            &[
                ctx.accounts.tipper.to_account_info(),
                ctx.accounts.domain_owner.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // 3. Transfer collaborator cut
        if collaborator_cut > 0 {
            invoke(
                &system_instruction::transfer(
                    &ctx.accounts.tipper.key(),
                    &ctx.accounts.collaborator.key(),
                    collaborator_cut,
                ),
                &[
                    ctx.accounts.tipper.to_account_info(),
                    ctx.accounts.collaborator.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;
        }

        emit!(CreatorTippedEvent {
            domain: domain.key(),
            tipper: ctx.accounts.tipper.key(),
            total_amount: tip_amount,
            primary_amount: primary_owner_cut,
            split_amount: collaborator_cut,
            platform_fee,
        });

        Ok(())
    }

    /// Modifies routing content hash (IPFS/Arweave/Social Record).
    pub fn update_content_hash(
        ctx: Context<UpdateContentHash>,
        new_content_hash: String,
    ) -> Result<()> {
        require!(new_content_hash.len() <= MAX_CONTENT_HASH_LEN, ZnsError::ContentHashTooLong);
        let domain = &mut ctx.accounts.domain_record;
        domain.content_hash = new_content_hash.clone();

        emit!(ContentHashUpdatedEvent {
            domain: domain.key(),
            content_hash: new_content_hash,
        });

        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Validation & Utility Helpers
// -----------------------------------------------------------------------------

pub fn validate_domain_name(name: &str) -> Result<()> {
    require!(!name.is_empty(), ZnsError::NameEmpty);
    require!(name.len() <= MAX_NAME_LEN, ZnsError::NameTooLong);

    for c in name.chars() {
        let is_valid = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_';
        require!(is_valid, ZnsError::InvalidCharacter);
    }
    Ok(())
}

pub fn calculate_registration_fee(name_len: usize, base_fee: u64) -> Result<u64> {
    // Shorter names command higher valuation tier
    let multiplier: u64 = match name_len {
        1 => 20,
        2 => 10,
        3 => 5,
        4 => 2,
        _ => 1,
    };
    base_fee.checked_mul(multiplier).ok_or(ZnsError::MathOverflow.into())
}

// -----------------------------------------------------------------------------
// Account Context Structs
// -----------------------------------------------------------------------------

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + Config::LEN,
        seeds = [b"config"],
        bump
    )]
    pub config: Account<'info, Config>,
    /// CHECK: Protocol treasury address checked by authority signature
    pub treasury: UncheckedAccount<'info>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(name: String)]
pub mod register_domain_context {
    use super::*;

    #[derive(Accounts)]
    #[instruction(name: String)]
    pub struct RegisterDomain<'info> {
        #[account(
            mut,
            seeds = [b"config"],
            bump = config.bump
        )]
        pub config: Account<'info, Config>,

        #[account(
            init,
            payer = payer,
            space = 8 + DomainRecord::LEN,
            seeds = [b"zns_domain", hash(name.as_bytes()).to_bytes().as_ref()],
            bump
        )]
        pub domain_record: Account<'info, DomainRecord>,

        #[account(mut)]
        pub payer: Signer<'info>,

        /// CHECK: Verified as owner recipient of domain
        pub owner: UncheckedAccount<'info>,

        #[account(mut, address = config.treasury @ ZnsError::InvalidTreasury)]
        /// CHECK: Treasury account verified against config state
        pub treasury: UncheckedAccount<'info>,

        pub system_program: Program<'info, System>,
    }
}
pub use register_domain_context::RegisterDomain;

#[derive(Accounts)]
pub struct SetReverseRecord<'info> {
    #[account(
        init_if_needed,
        payer = owner,
        space = 8 + ReverseRecord::LEN,
        seeds = [b"zns_reverse", owner.key().as_ref()],
        bump
    )]
    pub reverse_record: Account<'info, ReverseRecord>,

    #[account(
        has_one = owner @ ZnsError::Unauthorized
    )]
    pub domain_record: Account<'info, DomainRecord>,

    #[account(mut)]
    pub owner: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(subdomain_prefix: String)]
pub struct CreateSubdomain<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + SubdomainRecord::LEN,
        seeds = [
            b"zns_subdomain",
            parent_domain.key().as_ref(),
            hash(subdomain_prefix.as_bytes()).to_bytes().as_ref()
        ],
        bump
    )]
    pub subdomain_record: Account<'info, SubdomainRecord>,

    #[account(
        has_one = owner @ ZnsError::Unauthorized
    )]
    pub parent_domain: Account<'info, DomainRecord>,

    pub owner: Signer<'info>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TipCreator<'info> {
    #[account(
        seeds = [b"config"],
        bump = config.bump
    )]
    pub config: Account<'info, Config>,

    #[account(
        seeds = [b"zns_domain", domain_record.name_hash.as_ref()],
        bump = domain_record.bump
    )]
    pub domain_record: Account<'info, DomainRecord>,

    #[account(mut)]
    pub tipper: Signer<'info>,

    #[account(mut, address = domain_record.owner @ ZnsError::InvalidRecipient)]
    /// CHECK: Primary creator key matches domain record
    pub domain_owner: UncheckedAccount<'info>,

    #[account(mut, address = domain_record.collaborator @ ZnsError::InvalidCollaborator)]
    /// CHECK: Collaborator key matches domain record
    pub collaborator: UncheckedAccount<'info>,

    #[account(mut, address = config.treasury @ ZnsError::InvalidTreasury)]
    /// CHECK: Verified treasury vault
    pub treasury: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateContentHash<'info> {
    #[account(
        mut,
        has_one = owner @ ZnsError::Unauthorized,
        seeds = [b"zns_domain", domain_record.name_hash.as_ref()],
        bump = domain_record.bump
    )]
    pub domain_record: Account<'info, DomainRecord>,
    pub owner: Signer<'info>,
}

// -----------------------------------------------------------------------------
// State Layouts
// -----------------------------------------------------------------------------

#[account]
pub struct Config {
    pub authority: Pubkey,
    pub treasury: Pubkey,
    pub fee_basis_points: u16,
    pub base_registration_fee: u64,
    pub total_registered: u64,
    pub bump: u8,
}

impl Config {
    pub const LEN: usize = 32 + 32 + 2 + 8 + 8 + 1;
}

#[account]
pub struct DomainRecord {
    pub owner: Pubkey,
    pub collaborator: Pubkey,
    pub collaborator_split_bps: u16,
    pub name_hash: [u8; 32],
    pub created_at: i64,
    pub expires_at: i64,
    pub bump: u8,
    pub name: String,
    pub content_hash: String,
}

impl DomainRecord {
    // 32 + 32 + 2 + 32 + 8 + 8 + 1 + (4 + 32) + (4 + 64)
    pub const LEN: usize = 32 + 32 + 2 + 32 + 8 + 8 + 1 + 36 + 68;
}

#[account]
pub struct ReverseRecord {
    pub owner: Pubkey,
    pub domain_name: String,
    pub bump: u8,
}

impl ReverseRecord {
    pub const LEN: usize = 32 + (4 + MAX_NAME_LEN) + 1;
}

#[account]
pub struct SubdomainRecord {
    pub parent_domain: Pubkey,
    pub owner: Pubkey,
    pub subdomain_prefix: String,
    pub created_at: i64,
    pub bump: u8,
}

impl SubdomainRecord {
    pub const LEN: usize = 32 + 32 + (4 + MAX_NAME_LEN) + 8 + 1;
}

// -----------------------------------------------------------------------------
// Events & Errors
// -----------------------------------------------------------------------------

#[event]
pub struct DomainRegisteredEvent {
    pub domain_name: String,
    pub owner: Pubkey,
    pub expires_at: i64,
    pub fee_paid: u64,
}

#[event]
pub struct ReverseRecordUpdatedEvent {
    pub owner: Pubkey,
    pub domain_name: String,
}

#[event]
pub struct SubdomainCreatedEvent {
    pub parent: Pubkey,
    pub subdomain: String,
    pub beneficiary: Pubkey,
}

#[event]
pub struct CreatorTippedEvent {
    pub domain: Pubkey,
    pub tipper: Pubkey,
    pub total_amount: u64,
    pub primary_amount: u64,
    pub split_amount: u64,
    pub platform_fee: u64,
}

#[event]
pub struct ContentHashUpdatedEvent {
    pub domain: Pubkey,
    pub content_hash: String,
}

#[error_code]
pub enum ZnsError {
    #[msg("Protocol fee basis points exceeds allowed threshold")]
    FeeTooHigh,
    #[msg("Domain name is empty")]
    NameEmpty,
    #[msg("Domain name exceeds 32 byte limit")]
    NameTooLong,
    #[msg("Domain contains illegal characters (only lowercase a-z, 0-9, -, _)")]
    InvalidCharacter,
    #[msg("Domain registration duration must be at least 1 day")]
    InvalidDuration,
    #[msg("Collaborator split basis points must be <= 10000")]
    InvalidSplit,
    #[msg("Arithmetic overflow detected in fee calculation")]
    MathOverflow,
    #[msg("Content hash string exceeds 64 byte limit")]
    ContentHashTooLong,
    #[msg("Provided name hash does not match target domain account")]
    DomainMismatch,
    #[msg("Signer is not authorized to manage this domain")]
    Unauthorized,
    #[msg("Target domain record has expired")]
    DomainExpired,
    #[msg("Tip amount must be greater than zero")]
    ZeroTipAmount,
    #[msg("Treasury account provided does not match protocol configuration")]
    InvalidTreasury,
    #[msg("Recipient address does not match domain owner")]
    InvalidRecipient,
    #[msg("Collaborator address does not match domain record")]
    InvalidCollaborator,
}