use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

declare_id!("ZNScre8rP1111111111111111111111111111111111");

pub const MAX_NAME_LEN: usize = 32;
pub const MAX_URI_LEN: usize = 96;
pub const PROTOCOL_CONFIG_SEED: &[u8] = b"config";
pub const DOMAIN_SEED: &[u8] = b"domain";
pub const SUBDOMAIN_SEED: &[u8] = b"subdomain";
pub const REVERSE_SEED: &[u8] = b"reverse";
pub const TREASURY_SEED: &[u8] = b"treasury";

#[program]
pub mod zns_creator_core {
    use super::*;

    /// Initializes global protocol state, fee structures, and root treasury PDA.
    pub fn initialize_protocol(
        ctx: Context<InitializeProtocol>,
        registration_fee: u64,
        protocol_fee_bps: u16,
    ) -> Result<()> {
        require!(protocol_fee_bps <= 10_000, ZnsError::InvalidBps);
        let config = &mut ctx.accounts.config;
        config.admin = ctx.accounts.admin.key();
        config.treasury_vault = ctx.accounts.treasury_vault.key();
        config.registration_fee = registration_fee;
        config.protocol_fee_bps = protocol_fee_bps;
        config.total_domains = 0;
        config.bump = ctx.bumps.config;
        config.treasury_bump = ctx.bumps.treasury_vault;

        emit!(ProtocolInitializedEvent {
            admin: config.admin,
            registration_fee,
            protocol_fee_bps,
        });

        Ok(())
    }

    /// Registers a unique top-level creator domain (e.g., 'solcreator.zns').
    pub fn register_creator_domain(
        ctx: Context<RegisterCreatorDomain>,
        domain_name: String,
        metadata_uri: String,
    ) -> Result<()> {
        require!(
            !domain_name.is_empty() && domain_name.len() <= MAX_NAME_LEN,
            ZnsError::InvalidNameLength
        );
        require!(
            metadata_uri.len() <= MAX_URI_LEN,
            ZnsError::MetadataTooLong
        );
        require!(
            domain_name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            ZnsError::InvalidCharacters
        );

        let config = &mut ctx.accounts.config;

        // Collect registration fee via System Program CPI
        if config.registration_fee > 0 {
            let cpi_context = CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.payer.to_account_info(),
                    to: ctx.accounts.treasury_vault.to_account_info(),
                },
            );
            transfer(cpi_context, config.registration_fee)?;
        }

        let domain = &mut ctx.accounts.domain_record;
        domain.owner = ctx.accounts.creator.key();
        domain.resolver = ctx.accounts.creator.key();
        
        // Zero-fill fixed string storage
        let mut name_bytes = [0u8; MAX_NAME_LEN];
        name_bytes[..domain_name.len()].copy_from_slice(domain_name.as_bytes());
        domain.name = name_bytes;
        domain.name_len = domain_name.len() as u8;

        let mut uri_bytes = [0u8; MAX_URI_LEN];
        uri_bytes[..metadata_uri.len()].copy_from_slice(metadata_uri.as_bytes());
        domain.metadata_uri = uri_bytes;
        domain.uri_len = metadata_uri.len() as u8;

        domain.created_at = Clock::get()?.unix_timestamp;
        domain.subdomain_count = 0;
        domain.bump = ctx.bumps.domain_record;

        config.total_domains = config.total_domains.checked_add(1).ok_or(ZnsError::MathOverflow)?;

        emit!(DomainRegisteredEvent {
            domain_name,
            owner: domain.owner,
            resolver: domain.resolver,
            timestamp: domain.created_at,
        });

        Ok(())
    }

    /// Mutates creator metadata and target resolver pubkey in-place.
    pub fn update_metadata_record(
        ctx: Context<UpdateMetadataRecord>,
        new_resolver: Option<Pubkey>,
        new_uri: Option<String>,
    ) -> Result<()> {
        let domain = &mut ctx.accounts.domain_record;

        if let Some(resolver) = new_resolver {
            domain.resolver = resolver;
        }

        if let Some(uri) = new_uri {
            require!(uri.len() <= MAX_URI_LEN, ZnsError::MetadataTooLong);
            let mut uri_bytes = [0u8; MAX_URI_LEN];
            uri_bytes[..uri.len()].copy_from_slice(uri.as_bytes());
            domain.metadata_uri = uri_bytes;
            domain.uri_len = uri.len() as u8;
        }

        emit!(DomainUpdatedEvent {
            domain_pubkey: domain.key(),
            new_resolver: domain.resolver,
        });

        Ok(())
    }

    /// Issues an on-chain sub-domain pass (e.g., 'vip.solcreator.zns') with split payments.
    pub fn issue_subdomain_pass(
        ctx: Context<IssueSubdomainPass>,
        subdomain_name: String,
        duration_seconds: i64,
        mint_price_lamports: u64,
    ) -> Result<()> {
        require!(
            !subdomain_name.is_empty() && subdomain_name.len() <= MAX_NAME_LEN,
            ZnsError::InvalidNameLength
        );
        require!(duration_seconds > 0, ZnsError::InvalidDuration);

        let config = &ctx.accounts.config;
        let clock = Clock::get()?;

        // Split economics: Primary Creator Revenue vs Protocol Fee
        if mint_price_lamports > 0 {
            let protocol_cut = (mint_price_lamports as u128)
                .checked_mul(config.protocol_fee_bps as u128)
                .ok_or(ZnsError::MathOverflow)?
                .checked_div(10_000)
                .ok_or(ZnsError::MathOverflow)? as u64;

            let creator_cut = mint_price_lamports
                .checked_sub(protocol_cut)
                .ok_or(ZnsError::MathOverflow)?;

            if protocol_cut > 0 {
                let cpi_to_treasury = CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.payer.to_account_info(),
                        to: ctx.accounts.treasury_vault.to_account_info(),
                    },
                );
                transfer(cpi_to_treasury, protocol_cut)?;
            }

            if creator_cut > 0 {
                let cpi_to_creator = CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.payer.to_account_info(),
                        to: ctx.accounts.creator.to_account_info(),
                    },
                );
                transfer(cpi_to_creator, creator_cut)?;
            }
        }

        let domain = &mut ctx.accounts.parent_domain;
        domain.subdomain_count = domain.subdomain_count.checked_add(1).ok_or(ZnsError::MathOverflow)?;

        let subdomain = &mut ctx.accounts.subdomain_record;
        subdomain.parent_domain = domain.key();
        subdomain.holder = ctx.accounts.recipient.key();
        
        let mut sub_bytes = [0u8; MAX_NAME_LEN];
        sub_bytes[..subdomain_name.len()].copy_from_slice(subdomain_name.as_bytes());
        subdomain.subdomain_name = sub_bytes;
        subdomain.name_len = subdomain_name.len() as u8;

        subdomain.issued_at = clock.unix_timestamp;
        subdomain.expires_at = clock.unix_timestamp
            .checked_add(duration_seconds)
            .ok_or(ZnsError::MathOverflow)?;
        subdomain.bump = ctx.bumps.subdomain_record;

        emit!(SubdomainIssuedEvent {
            parent: domain.key(),
            subdomain: subdomain_name,
            holder: subdomain.holder,
            expires_at: subdomain.expires_at,
        });

        Ok(())
    }

    /// Sets deterministic 1-to-1 reverse address resolution pointer.
    pub fn set_reverse_lookup(ctx: Context<SetReverseLookup>) -> Result<()> {
        let reverse = &mut ctx.accounts.reverse_record;
        let domain = &ctx.accounts.domain_record;

        require_keys_eq!(domain.owner, ctx.accounts.authority.key(), ZnsError::Unauthorized);

        reverse.wallet = ctx.accounts.authority.key();
        reverse.domain = domain.key();
        reverse.bump = ctx.bumps.reverse_record;

        emit!(ReverseLookupConfiguredEvent {
            wallet: reverse.wallet,
            domain: reverse.domain,
        });

        Ok(())
    }

    /// Permissionless keeper crank to prune expired subdomain pass accounts and reclaim rent.
    pub fn reclaim_expired_subdomain(ctx: Context<ReclaimExpiredSubdomain>) -> Result<()> {
        let clock = Clock::get()?;
        let subdomain = &ctx.accounts.subdomain_record;

        require!(
            clock.unix_timestamp > subdomain.expires_at,
            ZnsError::SubdomainNotExpired
        );

        let domain = &mut ctx.accounts.parent_domain;
        domain.subdomain_count = domain.subdomain_count.saturating_sub(1);

        emit!(SubdomainExpiredReclaimedEvent {
            parent: domain.key(),
            subdomain_pda: subdomain.key(),
            reclaimed_by: ctx.accounts.crank_payer.key(),
        });

        // Account closure is completed automatically via `close = creator` in Context
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// ACCOUNT VALIDATION CONTEXTS
// -----------------------------------------------------------------------------

#[derive(Accounts)]
pub struct InitializeProtocol<'info> {
    #[account(
        init,
        payer = admin,
        space = 8 + ProtocolConfig::LEN,
        seeds = [PROTOCOL_CONFIG_SEED],
        bump
    )]
    pub config: Account<'info, ProtocolConfig>,

    /// CHECK: Protocol Treasury PDA owned by the program to collect platform protocol cuts
    #[account(
        seeds = [TREASURY_SEED],
        bump
    )]
    pub treasury_vault: AccountInfo<'info>,

    #[account(mut)]
    pub admin: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(domain_name: String, metadata_uri: String)]
pub struct RegisterCreatorDomain<'info> {
    #[account(
        mut,
        seeds = [PROTOCOL_CONFIG_SEED],
        bump = config.bump
    )]
    pub config: Account<'info, ProtocolConfig>,

    #[account(
        init,
        payer = payer,
        space = 8 + DomainRecord::LEN,
        seeds = [DOMAIN_SEED, anchor_lang::solana_program::hash::hash(domain_name.as_bytes()).to_bytes().as_ref()],
        bump
    )]
    pub domain_record: Account<'info, DomainRecord>,

    /// CHECK: Validated as recipient of protocol fees via config.treasury_vault
    #[account(
        mut,
        address = config.treasury_vault @ ZnsError::InvalidTreasury
    )]
    pub treasury_vault: AccountInfo<'info>,

    pub creator: Signer<'info>,

    #[account(mut)]
    pub payer: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateMetadataRecord<'info> {
    #[account(
        mut,
        has_one = owner @ ZnsError::Unauthorized,
        seeds = [DOMAIN_SEED, anchor_lang::solana_program::hash::hash(&domain_record.name[..domain_record.name_len as usize]).to_bytes().as_ref()],
        bump = domain_record.bump
    )]
    pub domain_record: Account<'info, DomainRecord>,

    pub owner: Signer<'info>,
}

#[derive(Accounts)]
#[instruction(subdomain_name: String)]
pub struct IssueSubdomainPass<'info> {
    #[account(
        seeds = [PROTOCOL_CONFIG_SEED],
        bump = config.bump
    )]
    pub config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [DOMAIN_SEED, anchor_lang::solana_program::hash::hash(&parent_domain.name[..parent_domain.name_len as usize]).to_bytes().as_ref()],
        bump = parent_domain.bump,
        has_one = owner @ ZnsError::Unauthorized
    )]
    pub parent_domain: Account<'info, DomainRecord>,

    #[account(
        init,
        payer = payer,
        space = 8 + SubDomainRecord::LEN,
        seeds = [
            SUBDOMAIN_SEED,
            parent_domain.key().as_ref(),
            anchor_lang::solana_program::hash::hash(subdomain_name.as_bytes()).to_bytes().as_ref()
        ],
        bump
    )]
    pub subdomain_record: Account<'info, SubDomainRecord>,

    /// CHECK: Parent domain creator/owner who receives creator royalty share
    #[account(mut, address = parent_domain.owner)]
    pub creator: AccountInfo<'info>,

    /// CHECK: Target recipient/holder of the pass
    pub recipient: AccountInfo<'info>,

    /// CHECK: Validated matching config.treasury_vault
    #[account(mut, address = config.treasury_vault @ ZnsError::InvalidTreasury)]
    pub treasury_vault: AccountInfo<'info>,

    pub owner: Signer<'info>,

    #[account(mut)]
    pub payer: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SetReverseLookup<'info> {
    #[account(
        init_if_needed,
        payer = authority,
        space = 8 + ReverseRecord::LEN,
        seeds = [REVERSE_SEED, authority.key().as_ref()],
        bump
    )]
    pub reverse_record: Account<'info, ReverseRecord>,

    pub domain_record: Account<'info, DomainRecord>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ReclaimExpiredSubdomain<'info> {
    #[account(
        mut,
        seeds = [DOMAIN_SEED, anchor_lang::solana_program::hash::hash(&parent_domain.name[..parent_domain.name_len as usize]).to_bytes().as_ref()],
        bump = parent_domain.bump,
        has_one = owner @ ZnsError::Unauthorized
    )]
    pub parent_domain: Account<'info, DomainRecord>,

    #[account(
        mut,
        close = owner,
        seeds = [
            SUBDOMAIN_SEED,
            parent_domain.key().as_ref(),
            anchor_lang::solana_program::hash::hash(&subdomain_record.subdomain_name[..subdomain_record.name_len as usize]).to_bytes().as_ref()
        ],
        bump = subdomain_record.bump,
        has_one = parent_domain @ ZnsError::ParentMismatch
    )]
    pub subdomain_record: Account<'info, SubDomainRecord>,

    /// CHECK: Creator who original initialized the parent domain and gets rent lamports back
    #[account(mut, address = parent_domain.owner)]
    pub owner: AccountInfo<'info>,

    #[account(mut)]
    pub crank_payer: Signer<'info>,
}

// -----------------------------------------------------------------------------
// STATE SCHEMAS
// -----------------------------------------------------------------------------

#[account]
pub struct ProtocolConfig {
    pub admin: Pubkey,            // 32
    pub treasury_vault: Pubkey,   // 32
    pub registration_fee: u64,    // 8
    pub protocol_fee_bps: u16,    // 2
    pub total_domains: u64,       // 8
    pub bump: u8,                 // 1
    pub treasury_bump: u8,        // 1
}

impl ProtocolConfig {
    pub const LEN: usize = 32 + 32 + 8 + 2 + 8 + 1 + 1;
}

#[account]
pub struct DomainRecord {
    pub owner: Pubkey,                    // 32
    pub resolver: Pubkey,                 // 32
    pub name: [u8; MAX_NAME_LEN],         // 32
    pub name_len: u8,                     // 1
    pub metadata_uri: [u8; MAX_URI_LEN],  // 96
    pub uri_len: u8,                      // 1
    pub created_at: i64,                  // 8
    pub subdomain_count: u64,             // 8
    pub bump: u8,                         // 1
}

impl DomainRecord {
    pub const LEN: usize = 32 + 32 + MAX_NAME_LEN + 1 + MAX_URI_LEN + 1 + 8 + 8 + 1;
}

#[account]
pub struct SubDomainRecord {
    pub parent_domain: Pubkey,                 // 32
    pub holder: Pubkey,                        // 32
    pub subdomain_name: [u8; MAX_NAME_LEN],    // 32
    pub name_len: u8,                          // 1
    pub issued_at: i64,                        // 8
    pub expires_at: i64,                       // 8
    pub bump: u8,                              // 1
}

impl SubDomainRecord {
    pub const LEN: usize = 32 + 32 + MAX_NAME_LEN + 1 + 8 + 8 + 1;
}

#[account]
pub struct ReverseRecord {
    pub wallet: Pubkey,  // 32
    pub domain: Pubkey,  // 32
    pub bump: u8,        // 1
}

impl ReverseRecord {
    pub const LEN: usize = 32 + 32 + 1;
}

// -----------------------------------------------------------------------------
// PROTOCOL EVENTS
// -----------------------------------------------------------------------------

#[event]
pub struct ProtocolInitializedEvent {
    pub admin: Pubkey,
    pub registration_fee: u64,
    pub protocol_fee_bps: u16,
}

#[event]
pub struct DomainRegisteredEvent {
    pub domain_name: String,
    pub owner: Pubkey,
    pub resolver: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct DomainUpdatedEvent {
    pub domain_pubkey: Pubkey,
    pub new_resolver: Pubkey,
}

#[event]
pub struct SubdomainIssuedEvent {
    pub parent: Pubkey,
    pub subdomain: String,
    pub holder: Pubkey,
    pub expires_at: i64,
}

#[event]
pub struct ReverseLookupConfiguredEvent {
    pub wallet: Pubkey,
    pub domain: Pubkey,
}

#[event]
pub struct SubdomainExpiredReclaimedEvent {
    pub parent: Pubkey,
    pub subdomain_pda: Pubkey,
    pub reclaimed_by: Pubkey,
}

// -----------------------------------------------------------------------------
// ERROR CODES
// -----------------------------------------------------------------------------

#[error_code]
pub enum ZnsError {
    #[msg("Input name exceeds max allowable length of 32 bytes.")]
    InvalidNameLength,
    #[msg("Metadata URI exceeds 96 byte compact allocation limit.")]
    MetadataTooLong,
    #[msg("Name string contains forbidden characters. Must be alphanumeric, dash, or underscore.")]
    InvalidCharacters,
    #[msg("Arithmetic overflow detected in fee calculation or state counter.")]
    MathOverflow,
    #[msg("Signer is not authorized to alter this domain entity.")]
    Unauthorized,
    #[msg("Treasury address does not match protocol configuration.")]
    InvalidTreasury,
    #[msg("Basis points parameter must be between 0 and 10000.")]
    InvalidBps,
    #[msg("Subdomain pass duration must be greater than zero seconds.")]
    InvalidDuration,
    #[msg("Subdomain pass has not yet expired; cannot be reclaimed by keeper.")]
    SubdomainNotExpired,
    #[msg("Subdomain does not belong to specified parent domain.")]
    ParentMismatch,
}