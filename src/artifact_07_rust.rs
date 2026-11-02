pub mod errors;
pub mod events;
pub mod state;

use anchor_lang::prelude::*;
use anchor_lang::system_program;
use errors::ZnsError;
use events::*;
use sha2::{Digest, Sha256};
use state::*;

declare_id!("ZNScre8or1111111111111111111111111111111111");

#[program]
pub mod zns_creator_core {
    use super::*;

    /// Initialize the Root Configuration singleton
    pub fn initialize_protocol(
        ctx: Context<InitializeProtocol>,
        platform_fee_bps: u16,
    ) -> Result<()> {
        require!(platform_fee_bps <= 1000, ZnsError::InvalidFeeConfiguration); // max 10%

        let root_config = &mut ctx.accounts.root_config;
        root_config.admin = ctx.accounts.admin.key();
        root_config.treasury = ctx.accounts.treasury.key();
        root_config.platform_fee_bps = platform_fee_bps;
        root_config.total_domains = 0;
        root_config.bump = ctx.bumps.root_config;

        Ok(())
    }

    /// Register a Top-Level Domain (TLD) e.g., "creator", "sol"
    pub fn register_tld(
        ctx: Context<RegisterTld>,
        tld_name: String,
        min_registration_fee: u64,
    ) -> Result<()> {
        validate_name_string(&tld_name)?;

        let tld = &mut ctx.accounts.tld_record;
        tld.tld_name = tld_name;
        tld.admin = ctx.accounts.admin.key();
        tld.min_registration_fee = min_registration_fee;
        tld.is_active = true;
        tld.bump = ctx.bumps.tld_record;

        Ok(())
    }

    /// Mint a Creator Root Domain (e.g. alice.creator)
    pub fn mint_domain(
        ctx: Context<MintDomain>,
        name: String,
        duration_years: u8,
        allow_subdomains: bool,
        subdomain_fee_lamports: u64,
    ) -> Result<()> {
        validate_name_string(&name)?;
        require!(duration_years >= 1 && duration_years <= 10, ZnsError::InvalidFeeConfiguration);

        let tld = &ctx.accounts.tld_record;
        require!(tld.is_active, ZnsError::TldInactive);

        let total_fee = tld
            .min_registration_fee
            .checked_mul(duration_years as u64)
            .ok_or(ZnsError::MathOverflow)?;

        // Execute payment to protocol treasury
        if total_fee > 0 {
            system_program::transfer(
                CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    system_program::Transfer {
                        from: ctx.accounts.payer.to_account_info(),
                        to: ctx.accounts.treasury.to_account_info(),
                    },
                ),
                total_fee,
            )?;
        }

        let clock = Clock::get()?;
        let duration_seconds = (duration_years as i64)
            .checked_mul(31_536_000)
            .ok_or(ZnsError::MathOverflow)?;
        let expires_at = clock.unix_timestamp.checked_add(duration_seconds).ok_or(ZnsError::MathOverflow)?;

        let domain = &mut ctx.accounts.domain_record;
        domain.parent_tld = tld.key();
        domain.owner = ctx.accounts.owner.key();
        domain.name_hash = compute_sha256(&name);
        domain.name = name.clone();
        domain.created_at = clock.unix_timestamp;
        domain.expires_at = expires_at;
        domain.subdomain_count = 0;
        domain.allow_subdomains = allow_subdomains;
        domain.subdomain_fee_lamports = subdomain_fee_lamports;
        domain.resolver = ctx.accounts.resolver.key();
        domain.bump = ctx.bumps.domain_record;

        // Initialize empty resolver record
        let resolver = &mut ctx.accounts.resolver;
        resolver.domain = domain.key();
        resolver.tipping_address = ctx.accounts.owner.key();
        resolver.content_hash = [0u8; 32];
        resolver.arweave_cid = [0u8; 43];
        resolver.social_handle_hash = [0u8; 32];
        resolver.updated_at = clock.unix_timestamp;
        resolver.bump = ctx.bumps.resolver;

        // Update protocol metrics
        let root = &mut ctx.accounts.root_config;
        root.total_domains = root.total_domains.checked_add(1).ok_or(ZnsError::MathOverflow)?;

        emit!(DomainRegistered {
            domain: domain.key(),
            owner: ctx.accounts.owner.key(),
            name,
            expires_at,
        });

        Ok(())
    }

    /// Mint a programmable Subdomain (e.g. vip.alice.creator) with protocol & creator fee splits
    pub fn mint_subdomain(
        ctx: Context<MintSubdomain>,
        subdomain_name: String,
        tier_level: u8,
    ) -> Result<()> {
        validate_name_string(&subdomain_name)?;

        let clock = Clock::get()?;
        let domain = &mut ctx.accounts.parent_domain;

        require!(domain.expires_at > clock.unix_timestamp, ZnsError::DomainExpired);
        require!(domain.allow_subdomains, ZnsError::SubdomainsDisabled);

        let fee = domain.subdomain_fee_lamports;
        if fee > 0 {
            let root = &ctx.accounts.root_config;
            let platform_fee = fee
                .checked_mul(root.platform_fee_bps as u64)
                .ok_or(ZnsError::MathOverflow)?
                .checked_div(10000)
                .ok_or(ZnsError::MathOverflow)?;

            let creator_share = fee.checked_sub(platform_fee).ok_or(ZnsError::MathOverflow)?;

            // Transfer platform cut
            if platform_fee > 0 {
                system_program::transfer(
                    CpiContext::new(
                        ctx.accounts.system_program.to_account_info(),
                        system_program::Transfer {
                            from: ctx.accounts.payer.to_account_info(),
                            to: ctx.accounts.treasury.to_account_info(),
                        },
                    ),
                    platform_fee,
                )?;
            }

            // Transfer creator cut directly to domain owner
            if creator_share > 0 {
                system_program::transfer(
                    CpiContext::new(
                        ctx.accounts.system_program.to_account_info(),
                        system_program::Transfer {
                            from: ctx.accounts.payer.to_account_info(),
                            to: ctx.accounts.domain_owner.to_account_info(),
                        },
                    ),
                    creator_share,
                )?;
            }
        }

        let subdomain = &mut ctx.accounts.subdomain_record;
        subdomain.parent_domain = domain.key();
        subdomain.owner = ctx.accounts.recipient.key();
        subdomain.name_hash = compute_sha256(&subdomain_name);
        subdomain.name = subdomain_name.clone();
        subdomain.created_at = clock.unix_timestamp;
        subdomain.expires_at = domain.expires_at; // Capped to parent validity
        subdomain.tier_level = tier_level;
        subdomain.bump = ctx.bumps.subdomain_record;

        domain.subdomain_count = domain.subdomain_count.checked_add(1).ok_or(ZnsError::MathOverflow)?;

        emit!(SubdomainMinted {
            subdomain: subdomain.key(),
            parent_domain: domain.key(),
            owner: ctx.accounts.recipient.key(),
            name: subdomain_name,
            paid_fee: fee,
        });

        Ok(())
    }

    /// Update dynamic resolver profile: multihash, arweave CID, tipping destination
    pub fn update_resolver(
        ctx: Context<UpdateResolver>,
        tipping_address: Pubkey,
        content_hash: [u8; 32],
        arweave_cid: [u8; 43],
        social_handle_hash: [u8; 32],
    ) -> Result<()> {
        let clock = Clock::get()?;
        let resolver = &mut ctx.accounts.resolver;

        resolver.tipping_address = tipping_address;
        resolver.content_hash = content_hash;
        resolver.arweave_cid = arweave_cid;
        resolver.social_handle_hash = social_handle_hash;
        resolver.updated_at = clock.unix_timestamp;

        emit!(ResolverUpdated {
            domain: ctx.accounts.domain_record.key(),
            tipping_address,
            content_hash,
            arweave_cid,
        });

        Ok(())
    }

    /// Direct on-chain tip routed via creator domain resolver
    pub fn tip_creator(ctx: Context<TipCreator>, amount_lamports: u64) -> Result<()> {
        require!(amount_lamports > 0, ZnsError::PaymentBelowThreshold);

        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.tipper.to_account_info(),
                    to: ctx.accounts.tipping_recipient.to_account_info(),
                },
            ),
            amount_lamports,
        )?;

        emit!(CreatorTipped {
            domain: ctx.accounts.domain_record.key(),
            from: ctx.accounts.tipper.key(),
            recipient: ctx.accounts.tipping_recipient.key(),
            amount: amount_lamports,
        });

        Ok(())
    }
}

// -----------------------------------------------------------------------------
// ACCOUNT VALIDATION STRUCTS
// -----------------------------------------------------------------------------

#[derive(Accounts)]
pub struct InitializeProtocol<'info> {
    #[account(
        init,
        payer = admin,
        space = RootConfig::LEN,
        seeds = [b"root_config"],
        bump
    )]
    pub root_config: Account<'info, RootConfig>,
    #[account(mut)]
    pub admin: Signer<'info>,
    /// CHECK: Protocol Treasury recipient address
    pub treasury: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(tld_name: String)]
pub struct RegisterTld<'info> {
    #[account(
        has_one = admin @ ZnsError::Unauthorized
    )]
    pub root_config: Account<'info, RootConfig>,
    #[account(
        init,
        payer = admin,
        space = TldRecord::LEN,
        seeds = [b"tld", tld_name.as_bytes()],
        bump
    )]
    pub tld_record: Account<'info, TldRecord>,
    #[account(mut)]
    pub admin: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(name: String)]
pub struct MintDomain<'info> {
    #[account(mut)]
    pub root_config: Account<'info, RootConfig>,
    #[account(
        seeds = [b"tld", tld_record.tld_name.as_bytes()],
        bump = tld_record.bump
    )]
    pub tld_record: Account<'info, TldRecord>,
    #[account(
        init,
        payer = payer,
        space = DomainRecord::LEN,
        seeds = [b"domain", tld_record.key().as_ref(), &compute_sha256(&name)],
        bump
    )]
    pub domain_record: Account<'info, DomainRecord>,
    #[account(
        init,
        payer = payer,
        space = CreatorProfileResolver::LEN,
        seeds = [b"resolver", domain_record.key().as_ref()],
        bump
    )]
    pub resolver: Account<'info, CreatorProfileResolver>,
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK: Beneficiary domain owner
    pub owner: AccountInfo<'info>,
    /// CHECK: Validated against root_config treasury
    #[account(
        mut,
        constraint = treasury.key() == root_config.treasury @ ZnsError::Unauthorized
    )]
    pub treasury: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(subdomain_name: String)]
pub struct MintSubdomain<'info> {
    pub root_config: Account<'info, RootConfig>,
    #[account(
        mut,
        seeds = [b"domain", parent_domain.parent_tld.as_ref(), parent_domain.name_hash.as_ref()],
        bump = parent_domain.bump
    )]
    pub parent_domain: Account<'info, DomainRecord>,
    #[account(
        init,
        payer = payer,
        space = SubdomainRecord::LEN,
        seeds = [b"subdomain", parent_domain.key().as_ref(), &compute_sha256(&subdomain_name)],
        bump
    )]
    pub subdomain_record: Account<'info, SubdomainRecord>,
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK: Subdomain holder
    pub recipient: AccountInfo<'info>,
    /// CHECK: Enforced to match parent domain owner
    #[account(
        mut,
        constraint = domain_owner.key() == parent_domain.owner @ ZnsError::Unauthorized
    )]
    pub domain_owner: AccountInfo<'info>,
    /// CHECK: Validated against root_config treasury
    #[account(
        mut,
        constraint = treasury.key() == root_config.treasury @ ZnsError::Unauthorized
    )]
    pub treasury: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateResolver<'info> {
    #[account(
        has_one = owner @ ZnsError::Unauthorized
    )]
    pub domain_record: Account<'info, DomainRecord>,
    #[account(
        mut,
        seeds = [b"resolver", domain_record.key().as_ref()],
        bump = resolver.bump
    )]
    pub resolver: Account<'info, CreatorProfileResolver>,
    pub owner: Signer<'info>,
}

#[derive(Accounts)]
pub struct TipCreator<'info> {
    pub domain_record: Account<'info, DomainRecord>,
    #[account(
        seeds = [b"resolver", domain_record.key().as_ref()],
        bump = resolver.bump
    )]
    pub resolver: Account<'info, CreatorProfileResolver>,
    #[account(mut)]
    pub tipper: Signer<'info>,
    /// CHECK: Verified against resolver.tipping_address
    #[account(
        mut,
        constraint = tipping_recipient.key() == resolver.tipping_address @ ZnsError::Unauthorized
    )]
    pub tipping_recipient: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
}

// -----------------------------------------------------------------------------
// HELPER FUNCTIONS
// -----------------------------------------------------------------------------

pub fn compute_sha256(input: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

pub fn validate_name_string(name: &str) -> Result<()> {
    require!(!name.is_empty() && name.len() <= MAX_NAME_LEN, ZnsError::NameTooLong);
    for b in name.bytes() {
        let is_valid = matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-');
        require!(is_valid, ZnsError::InvalidNameCharacters);
    }
    Ok(())
}