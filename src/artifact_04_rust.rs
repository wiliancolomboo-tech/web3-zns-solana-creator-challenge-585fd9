use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

declare_id!("ZNSCRTR111111111111111111111111111111111111");

pub const MAX_TLD_LEN: usize = 16;
pub const MAX_DOMAIN_LEN: usize = 32;
pub const MAX_KEY_LEN: usize = 32;
pub const MAX_VALUE_LEN: usize = 256;
pub const MAX_SPLIT_RECIPIENTS: usize = 5;
pub const BASIS_POINTS_DIVISOR: u64 = 10_000;

#[program]
pub mod zns_creator {
    use super::*;

    /// Initializes a Top-Level Domain (TLD) registry root (e.g. "sol", "zns").
    pub fn initialize_registry(
        ctx: Context<InitializeRegistry>,
        tld: String,
        annual_fee_lamports: u64,
    ) -> Result<()> {
        require!(
            !tld.is_empty() && tld.len() <= MAX_TLD_LEN,
            ZnsError::InvalidStringLength
        );

        let registry = &mut ctx.accounts.registry;
        registry.admin = ctx.accounts.admin.key();
        registry.tld = tld;
        registry.annual_fee_lamports = annual_fee_lamports;
        registry.total_domains = 0;
        registry.bump = ctx.bumps.registry;

        msg!("TLD Registry initialized: .{}", registry.tld);
        Ok(())
    }

    /// Registers a domain handle under a given TLD for a specified duration in years.
    pub fn register_domain(
        ctx: Context<RegisterDomain>,
        domain_name: String,
        years: u8,
    ) -> Result<()> {
        require!(
            !domain_name.is_empty() && domain_name.len() <= MAX_DOMAIN_LEN,
            ZnsError::InvalidStringLength
        );
        require!(years > 0 && years <= 10, ZnsError::InvalidDuration);

        let registry = &mut ctx.accounts.registry;
        let domain = &mut ctx.accounts.domain;
        let clock = Clock::get()?;

        let total_cost = registry
            .annual_fee_lamports
            .checked_mul(years as u64)
            .ok_or(ZnsError::MathOverflow)?;

        if total_cost > 0 {
            let cpi_ctx = CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.payer.to_account_info(),
                    to: ctx.accounts.admin.to_account_info(),
                },
            );
            transfer(cpi_ctx, total_cost)?;
        }

        let duration_seconds = (years as i64)
            .checked_mul(31_536_000)
            .ok_or(ZnsError::MathOverflow)?;
        let expires_at = clock
            .unix_timestamp
            .checked_add(duration_seconds)
            .ok_or(ZnsError::MathOverflow)?;

        domain.owner = ctx.accounts.owner.key();
        domain.tld = registry.tld.clone();
        domain.name = domain_name.clone();
        domain.created_at = clock.unix_timestamp;
        domain.expires_at = expires_at;
        domain.bump = ctx.bumps.domain;

        registry.total_domains = registry
            .total_domains
            .checked_add(1)
            .ok_or(ZnsError::MathOverflow)?;

        msg!(
            "Registered domain: {}.{} until epoch timestamp: {}",
            domain.name,
            domain.tld,
            domain.expires_at
        );
        Ok(())
    }

    /// Creates or updates a dynamic key-value pointer (e.g., "arweave", "avatar", "farcaster").
    pub fn set_domain_record(
        ctx: Context<SetDomainRecord>,
        record_key: String,
        record_value: String,
    ) -> Result<()> {
        require!(
            !record_key.is_empty() && record_key.len() <= MAX_KEY_LEN,
            ZnsError::InvalidStringLength
        );
        require!(
            record_value.len() <= MAX_VALUE_LEN,
            ZnsError::InvalidStringLength
        );

        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp < ctx.accounts.domain.expires_at,
            ZnsError::DomainExpired
        );

        let record = &mut ctx.accounts.record;
        record.domain = ctx.accounts.domain.key();
        record.key = record_key;
        record.value = record_value;
        record.updated_at = clock.unix_timestamp;
        record.bump = ctx.bumps.record;

        msg!("Record updated: {} => {}", record.key, record.value);
        Ok(())
    }

    /// Configures the atomic tip/revenue split routing for a domain.
    pub fn configure_split_escrow(
        ctx: Context<ConfigureSplitEscrow>,
        shares: Vec<SplitRecipient>,
    ) -> Result<()> {
        require!(
            !shares.is_empty() && shares.len() <= MAX_SPLIT_RECIPIENTS,
            ZnsError::InvalidRecipientCount
        );

        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp < ctx.accounts.domain.expires_at,
            ZnsError::DomainExpired
        );

        let mut total_bps: u16 = 0;
        for share in &shares {
            require!(share.basis_points > 0, ZnsError::InvalidBasisPoints);
            total_bps = total_bps
                .checked_add(share.basis_points)
                .ok_or(ZnsError::MathOverflow)?;
        }
        require!(
            total_bps == BASIS_POINTS_DIVISOR as u16,
            ZnsError::InvalidTotalBasisPoints
        );

        let split_config = &mut ctx.accounts.split_config;
        split_config.domain = ctx.accounts.domain.key();
        split_config.owner = ctx.accounts.owner.key();
        split_config.recipients_count = shares.len() as u8;
        split_config.recipients = shares;
        split_config.bump = ctx.bumps.split_config;

        msg!("Split engine configured with {} recipients", split_config.recipients_count);
        Ok(())
    }

    /// Tips a creator handle and atomically disperses the native lamports according to split rules.
    pub fn tip_and_split<'info>(
        ctx: Context<'_, '_, '_, 'info, TipAndSplit<'info>>,
        tip_amount: u64,
    ) -> Result<()> {
        require!(tip_amount > 0, ZnsError::InvalidTipAmount);

        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp < ctx.accounts.domain.expires_at,
            ZnsError::DomainExpired
        );

        let split_config = &ctx.accounts.split_config;
        let remaining_accounts = ctx.remaining_accounts;

        require!(
            remaining_accounts.len() == split_config.recipients.len(),
            ZnsError::RemainingAccountsMismatch
        );

        let mut dispersed_lamports: u64 = 0;

        for (i, recipient_def) in split_config.recipients.iter().enumerate() {
            let target_account_info = &remaining_accounts[i];
            require_keys_eq!(
                target_account_info.key(),
                recipient_def.recipient,
                ZnsError::RecipientKeyMismatch
            );

            let recipient_share = (tip_amount as u128)
                .checked_mul(recipient_def.basis_points as u128)
                .ok_or(ZnsError::MathOverflow)?
                .checked_div(BASIS_POINTS_DIVISOR as u128)
                .ok_or(ZnsError::MathOverflow)? as u64;

            if recipient_share > 0 {
                dispersed_lamports = dispersed_lamports
                    .checked_add(recipient_share)
                    .ok_or(ZnsError::MathOverflow)?;

                let cpi_ctx = CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.tipper.to_account_info(),
                        to: target_account_info.clone(),
                    },
                );
                transfer(cpi_ctx, recipient_share)?;
            }
        }

        // Send residual dust to primary domain owner
        let remainder = tip_amount
            .checked_sub(dispersed_lamports)
            .ok_or(ZnsError::MathOverflow)?;

        if remainder > 0 {
            let cpi_ctx = CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.tipper.to_account_info(),
                    to: ctx.accounts.domain_owner.to_account_info(),
                },
            );
            transfer(cpi_ctx, remainder)?;
        }

        msg!(
            "Tip executed: {} lamports routed atomically across {} splits",
            tip_amount,
            split_config.recipients.len()
        );
        Ok(())
    }

    /// Sets the reverse resolution PDA for the caller's pubkey pointing back to their primary domain.
    pub fn set_reverse_lookup(ctx: Context<SetReverseLookup>) -> Result<()> {
        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp < ctx.accounts.domain.expires_at,
            ZnsError::DomainExpired
        );

        let reverse_record = &mut ctx.accounts.reverse_record;
        reverse_record.target_domain = ctx.accounts.domain.key();
        reverse_record.owner = ctx.accounts.owner.key();
        reverse_record.bump = ctx.bumps.reverse_record;

        msg!(
            "Reverse lookup mapped: Pubkey({}) => DomainPDA({})",
            reverse_record.owner,
            reverse_record.target_domain
        );
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Account Structures
// ---------------------------------------------------------------------------

#[account]
pub struct RootRegistry {
    pub admin: Pubkey,
    pub tld: String,
    pub annual_fee_lamports: u64,
    pub total_domains: u64,
    pub bump: u8,
}

impl RootRegistry {
    pub const LEN: usize = 8 + 32 + (4 + MAX_TLD_LEN) + 8 + 8 + 1;
}

#[account]
pub struct DomainNameRecord {
    pub owner: Pubkey,
    pub tld: String,
    pub name: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub bump: u8,
}

impl DomainNameRecord {
    pub const LEN: usize = 8 + 32 + (4 + MAX_TLD_LEN) + (4 + MAX_DOMAIN_LEN) + 8 + 8 + 1;
}

#[account]
pub struct ContentRecord {
    pub domain: Pubkey,
    pub key: String,
    pub value: String,
    pub updated_at: i64,
    pub bump: u8,
}

impl ContentRecord {
    pub const LEN: usize = 8 + 32 + (4 + MAX_KEY_LEN) + (4 + MAX_VALUE_LEN) + 8 + 1;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq)]
pub struct SplitRecipient {
    pub recipient: Pubkey,
    pub basis_points: u16,
}

#[account]
pub struct SplitConfig {
    pub domain: Pubkey,
    pub owner: Pubkey,
    pub recipients_count: u8,
    pub recipients: Vec<SplitRecipient>,
    pub bump: u8,
}

impl SplitConfig {
    pub const LEN: usize = 8 + 32 + 32 + 1 + (4 + (MAX_SPLIT_RECIPIENTS * (32 + 2))) + 1;
}

#[account]
pub struct ReverseLookup {
    pub target_domain: Pubkey,
    pub owner: Pubkey,
    pub bump: u8,
}

impl ReverseLookup {
    pub const LEN: usize = 8 + 32 + 32 + 1;
}

// ---------------------------------------------------------------------------
// Context Validations
// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(tld: String)]
pub struct InitializeRegistry<'info> {
    #[account(
        init,
        payer = admin,
        space = RootRegistry::LEN,
        seeds = [b"registry", tld.as_bytes()],
        bump
    )]
    pub registry: Account<'info, RootRegistry>,
    #[account(mut)]
    pub admin: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(domain_name: String)]
pub struct RegisterDomain<'info> {
    #[account(
        mut,
        seeds = [b"registry", registry.tld.as_bytes()],
        bump = registry.bump
    )]
    pub registry: Account<'info, RootRegistry>,
    #[account(
        init,
        payer = payer,
        space = DomainNameRecord::LEN,
        seeds = [b"domain", registry.tld.as_bytes(), domain_name.as_bytes()],
        bump
    )]
    pub domain: Account<'info, DomainNameRecord>,
    #[account(mut)]
    /// CHECK: Target admin treasury validating fee deposit
    pub admin: UncheckedAccount<'info>,
    /// CHECK: Domain owner wallet which will control the registered domain
    pub owner: UncheckedAccount<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(record_key: String)]
pub struct SetDomainRecord<'info> {
    #[account(
        has_one = owner @ ZnsError::Unauthorized,
        seeds = [b"domain", domain.tld.as_bytes(), domain.name.as_bytes()],
        bump = domain.bump
    )]
    pub domain: Account<'info, DomainNameRecord>,
    #[account(
        init_if_needed,
        payer = owner,
        space = ContentRecord::LEN,
        seeds = [b"record", domain.key().as_ref(), record_key.as_bytes()],
        bump
    )]
    pub record: Account<'info, ContentRecord>,
    #[account(mut)]
    pub owner: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ConfigureSplitEscrow<'info> {
    #[account(
        has_one = owner @ ZnsError::Unauthorized,
        seeds = [b"domain", domain.tld.as_bytes(), domain.name.as_bytes()],
        bump = domain.bump
    )]
    pub domain: Account<'info, DomainNameRecord>,
    #[account(
        init_if_needed,
        payer = owner,
        space = SplitConfig::LEN,
        seeds = [b"split", domain.key().as_ref()],
        bump
    )]
    pub split_config: Account<'info, SplitConfig>,
    #[account(mut)]
    pub owner: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TipAndSplit<'info> {
    #[account(
        seeds = [b"domain", domain.tld.as_bytes(), domain.name.as_bytes()],
        bump = domain.bump
    )]
    pub domain: Account<'info, DomainNameRecord>,
    #[account(
        seeds = [b"split", domain.key().as_ref()],
        bump = split_config.bump,
        has_one = domain @ ZnsError::InvalidDomainReference
    )]
    pub split_config: Account<'info, SplitConfig>,
    #[account(
        mut,
        address = domain.owner @ ZnsError::Unauthorized
    )]
    /// CHECK: Matches verified domain owner for fallback residual dust
    pub domain_owner: UncheckedAccount<'info>,
    #[account(mut)]
    pub tipper: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SetReverseLookup<'info> {
    #[account(
        has_one = owner @ ZnsError::Unauthorized,
        seeds = [b"domain", domain.tld.as_bytes(), domain.name.as_bytes()],
        bump = domain.bump
    )]
    pub domain: Account<'info, DomainNameRecord>,
    #[account(
        init_if_needed,
        payer = owner,
        space = ReverseLookup::LEN,
        seeds = [b"reverse", owner.key().as_ref()],
        bump
    )]
    pub reverse_record: Account<'info, ReverseLookup>,
    #[account(mut)]
    pub owner: Signer<'info>,
    pub system_program: Program<'info, System>,
}

// ---------------------------------------------------------------------------
// Error Codes
// ---------------------------------------------------------------------------

#[error_code]
pub enum ZnsError {
    #[msg("Provided string exceeds maximum allowed byte size.")]
    InvalidStringLength,
    #[msg("Duration must be between 1 and 10 full solar years.")]
    InvalidDuration,
    #[msg("Mathematical overflow occurred during computation.")]
    MathOverflow,
    #[msg("Domain has surpassed its expiration epoch timestamp.")]
    DomainExpired,
    #[msg("Caller lacks the required authorization signature for this domain.")]
    Unauthorized,
    #[msg("Split configurations require between 1 and 5 active recipients.")]
    InvalidRecipientCount,
    #[msg("Basis points allocation for recipient must be strictly greater than 0.")]
    InvalidBasisPoints,
    #[msg("Cumulative basis points allocation must evaluate exactly to 10,000 (100%).")]
    InvalidTotalBasisPoints,
    #[msg("Tip amount must be strictly greater than 0 lamports.")]
    InvalidTipAmount,
    #[msg("Provided remaining accounts count does not match split recipient definitions.")]
    RemainingAccountsMismatch,
    #[msg("Pubkey in remaining accounts does not match configured split recipient pubkey.")]
    RecipientKeyMismatch,
    #[msg("Domain reference on split account does not match provided domain PDA.")]
    InvalidDomainReference,
}