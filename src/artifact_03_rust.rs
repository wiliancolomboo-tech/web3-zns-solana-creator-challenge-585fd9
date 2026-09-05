use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

declare_id!("ZNScre8or1111111111111111111111111111111111");

pub const MAX_HANDLE_LEN: usize = 32;
pub const MAX_METADATA_URI_LEN: usize = 96;
pub const BASIS_POINTS_DIVISOR: u64 = 10_000;

#[program]
pub mod zns_creator {
    use super::*;

    /// Initializes the protocol registry configuration.
    pub fn initialize_registry(
        ctx: Context<InitializeRegistry>,
        protocol_fee_bps: u16,
    ) -> Result<()> {
        require!(
            protocol_fee_bps <= 2_500,
            ZNSError::ProtocolFeeTooHigh
        ); // Max 25% protocol take rate
        let registry = &mut ctx.accounts.global_registry;
        registry.admin = ctx.accounts.admin.key();
        registry.treasury = ctx.accounts.treasury.key();
        registry.protocol_fee_bps = protocol_fee_bps;
        registry.total_creators = 0;
        registry.bump = ctx.bumps.global_registry;
        emit!(RegistryInitialized {
            admin: registry.admin,
            treasury: registry.treasury,
            protocol_fee_bps,
        });
        Ok(())
    }

    /// Registers a unique root creator identity (e.g. "alice").
    pub fn register_creator_handle(
        ctx: Context<RegisterCreatorHandle>,
        handle: String,
        metadata_uri: String,
        subdomain_base_fee_lamports: u64,
        subdomain_royalty_bps: u16,
    ) -> Result<()> {
        let handle_bytes = handle.as_bytes();
        require!(
            !handle_bytes.is_empty() && handle_bytes.len() <= MAX_HANDLE_LEN,
            ZNSError::InvalidHandleLength
        );
        require!(
            metadata_uri.as_bytes().len() <= MAX_METADATA_URI_LEN,
            ZNSError::MetadataUriTooLong
        );
        require!(
            subdomain_royalty_bps <= 5_000,
            ZNSError::RoyaltyBpsTooHigh
        ); // Max 50%

        // Fixed anti-squatting pricing curve based on length:
        // <= 3 chars: 0.5 SOL, 4-6 chars: 0.1 SOL, 7+ chars: 0.02 SOL
        let reg_fee: u64 = match handle_bytes.len() {
            1..=3 => 500_000_000,
            4..=6 => 100_000_000,
            _ => 20_000_000,
        };

        // Transfer registration fee to Protocol Treasury
        let transfer_cpi = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.creator.to_account_info(),
                to: ctx.accounts.treasury.to_account_info(),
            },
        );
        transfer(transfer_cpi, reg_fee)?;

        // Populate Creator Profile PDA
        let creator_profile = &mut ctx.accounts.creator_profile;
        let mut handle_arr = [0u8; MAX_HANDLE_LEN];
        handle_arr[..handle_bytes.len()].copy_from_slice(handle_bytes);

        let mut uri_arr = [0u8; MAX_METADATA_URI_LEN];
        uri_arr[..metadata_uri.as_bytes().len()].copy_from_slice(metadata_uri.as_bytes());

        creator_profile.authority = ctx.accounts.creator.key();
        creator_profile.handle = handle_arr;
        creator_profile.handle_len = handle_bytes.len() as u8;
        creator_profile.metadata_uri = uri_arr;
        creator_profile.metadata_uri_len = metadata_uri.as_bytes().len() as u8;
        creator_profile.subdomain_base_fee = subdomain_base_fee_lamports;
        creator_profile.subdomain_royalty_bps = subdomain_royalty_bps;
        creator_profile.total_subdomains = 0;
        creator_profile.total_tips_received = 0;
        creator_profile.bump = ctx.bumps.creator_profile;

        let registry = &mut ctx.accounts.global_registry;
        registry.total_creators = registry.total_creators.checked_add(1).ok_or(ZNSError::NumericalOverflow)?;

        emit!(CreatorHandleRegistered {
            creator: creator_profile.authority,
            handle,
            fee_paid: reg_fee,
        });

        Ok(())
    }

    /// Issues a fan subdomain under a creator namespace (e.g. "fan.alice").
    pub fn issue_fan_subdomain(
        ctx: Context<IssueFanSubdomain>,
        subdomain_name: String,
        tier: u8,
        duration_epochs: i64,
    ) -> Result<()> {
        let sub_bytes = subdomain_name.as_bytes();
        require!(
            !sub_bytes.is_empty() && sub_bytes.len() <= MAX_HANDLE_LEN,
            ZNSError::InvalidHandleLength
        );
        require!(duration_epochs > 0, ZNSError::InvalidDuration);

        let profile = &mut ctx.accounts.creator_profile;
        let registry = &ctx.accounts.global_registry;

        let base_fee = profile.subdomain_base_fee;
        if base_fee > 0 {
            // Calculate protocol cut & creator cut
            let protocol_cut = (base_fee as u128)
                .checked_mul(registry.protocol_fee_bps as u128)
                .ok_or(ZNSError::NumericalOverflow)?
                .checked_div(BASIS_POINTS_DIVISOR as u128)
                .ok_or(ZNSError::NumericalOverflow)? as u64;

            let creator_cut = base_fee.checked_sub(protocol_cut).ok_or(ZNSError::NumericalOverflow)?;

            // Disburse Protocol Fee
            if protocol_cut > 0 {
                let treasury_cpi = CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.fan.to_account_info(),
                        to: ctx.accounts.treasury.to_account_info(),
                    },
                );
                transfer(treasury_cpi, protocol_cut)?;
            }

            // Disburse Creator Fee
            let creator_cpi = CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.fan.to_account_info(),
                    to: ctx.accounts.creator_authority.to_account_info(),
                },
            );
            transfer(creator_cpi, creator_cut)?;
        }

        // Initialize Subdomain Record
        let subdomain_record = &mut ctx.accounts.subdomain_record;
        let mut sub_arr = [0u8; MAX_HANDLE_LEN];
        sub_arr[..sub_bytes.len()].copy_from_slice(sub_bytes);

        let clock = Clock::get()?;
        let expiry_epoch = clock.epoch.checked_add(duration_epochs).ok_or(ZNSError::NumericalOverflow)?;

        subdomain_record.parent_creator = profile.key();
        subdomain_record.owner = ctx.accounts.fan.key();
        subdomain_record.subdomain_name = sub_arr;
        subdomain_record.subdomain_len = sub_bytes.len() as u8;
        subdomain_record.tier = tier;
        subdomain_record.expiry_epoch = expiry_epoch;
        subdomain_record.bump = ctx.bumps.subdomain_record;

        profile.total_subdomains = profile.total_subdomains.checked_add(1).ok_or(ZNSError::NumericalOverflow)?;

        emit!(FanSubdomainIssued {
            parent_creator: profile.key(),
            fan: subdomain_record.owner,
            subdomain: subdomain_name,
            tier,
            expiry_epoch,
        });

        Ok(())
    }

    /// Atomic, direct-to-creator tip routing with protocol fee capture.
    pub fn send_creator_tip(ctx: Context<SendCreatorTip>, amount_lamports: u64) -> Result<()> {
        require!(amount_lamports > 0, ZNSError::ZeroTipAmount);

        let registry = &ctx.accounts.global_registry;
        let profile = &mut ctx.accounts.creator_profile;

        let protocol_cut = (amount_lamports as u128)
            .checked_mul(registry.protocol_fee_bps as u128)
            .ok_or(ZNSError::NumericalOverflow)?
            .checked_div(BASIS_POINTS_DIVISOR as u128)
            .ok_or(ZNSError::NumericalOverflow)? as u64;

        let creator_net = amount_lamports.checked_sub(protocol_cut).ok_or(ZNSError::NumericalOverflow)?;

        // Transfer net tip to Creator Authority
        let tip_cpi = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.supporter.to_account_info(),
                to: ctx.accounts.creator_authority.to_account_info(),
            },
        );
        transfer(tip_cpi, creator_net)?;

        // Transfer fee to Protocol Treasury
        if protocol_cut > 0 {
            let fee_cpi = CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.supporter.to_account_info(),
                    to: ctx.accounts.treasury.to_account_info(),
                },
            );
            transfer(fee_cpi, protocol_cut)?;
        }

        profile.total_tips_received = profile.total_tips_received.checked_add(amount_lamports).ok_or(ZNSError::NumericalOverflow)?;

        emit!(CreatorTipped {
            creator: profile.key(),
            supporter: ctx.accounts.supporter.key(),
            amount: amount_lamports,
            net_creator_amount: creator_net,
        });

        Ok(())
    }

    /// Allows the creator to update pricing parameters and metadata URI.
    pub fn update_creator_metadata(
        ctx: Context<UpdateCreatorMetadata>,
        new_metadata_uri: String,
        new_subdomain_fee: u64,
        new_royalty_bps: u16,
    ) -> Result<()> {
        require!(
            new_metadata_uri.as_bytes().len() <= MAX_METADATA_URI_LEN,
            ZNSError::MetadataUriTooLong
        );
        require!(
            new_royalty_bps <= 5_000,
            ZNSError::RoyaltyBpsTooHigh
        );

        let profile = &mut ctx.accounts.creator_profile;
        let mut uri_arr = [0u8; MAX_METADATA_URI_LEN];
        uri_arr[..new_metadata_uri.as_bytes().len()].copy_from_slice(new_metadata_uri.as_bytes());

        profile.metadata_uri = uri_arr;
        profile.metadata_uri_len = new_metadata_uri.as_bytes().len() as u8;
        profile.subdomain_base_fee = new_subdomain_fee;
        profile.subdomain_royalty_bps = new_royalty_bps;

        emit!(CreatorMetadataUpdated {
            creator: profile.key(),
            new_subdomain_fee,
            new_royalty_bps,
        });

        Ok(())
    }
}

// -----------------------------------------------------------------------------
// ACCOUNT INSTRUCTION STRUCTS
// -----------------------------------------------------------------------------

#[derive(Accounts)]
pub struct InitializeRegistry<'info> {
    #[account(
        init,
        payer = admin,
        space = GlobalRegistry::ACCOUNT_SIZE,
        seeds = [b"global_registry"],
        bump
    )]
    pub global_registry: Account<'info, GlobalRegistry>,
    #[account(mut)]
    pub admin: Signer<'info>,
    /// CHECK: Protocol revenue collection treasury
    pub treasury: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(handle: String)]
pub mod register_handle_accounts {
    use super::*;
}

#[derive(Accounts)]
#[instruction(handle: String)]
pub struct RegisterCreatorHandle<'info> {
    #[account(
        mut,
        seeds = [b"global_registry"],
        bump = global_registry.bump
    )]
    pub global_registry: Account<'info, GlobalRegistry>,
    #[account(
        init,
        payer = creator,
        space = CreatorProfile::ACCOUNT_SIZE,
        seeds = [b"creator", anchor_lang::solana_program::hash::hash(handle.as_bytes()).to_bytes().as_ref()],
        bump
    )]
    pub creator_profile: Account<'info, CreatorProfile>,
    #[account(mut)]
    pub creator: Signer<'info>,
    /// CHECK: Validated against global_registry.treasury
    #[account(
        mut,
        address = global_registry.treasury @ ZNSError::InvalidTreasuryAddress
    )]
    pub treasury: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(subdomain_name: String)]
pub struct IssueFanSubdomain<'info> {
    #[account(
        seeds = [b"global_registry"],
        bump = global_registry.bump
    )]
    pub global_registry: Account<'info, GlobalRegistry>,
    #[account(
        mut,
        seeds = [b"creator", anchor_lang::solana_program::hash::hash(&creator_profile.handle[..creator_profile.handle_len as usize]).to_bytes().as_ref()],
        bump = creator_profile.bump
    )]
    pub creator_profile: Account<'info, CreatorProfile>,
    #[account(
        init,
        payer = fan,
        space = SubdomainRecord::ACCOUNT_SIZE,
        seeds = [
            b"subdomain",
            creator_profile.key().as_ref(),
            anchor_lang::solana_program::hash::hash(subdomain_name.as_bytes()).to_bytes().as_ref()
        ],
        bump
    )]
    pub subdomain_record: Account<'info, SubdomainRecord>,
    #[account(mut)]
    pub fan: Signer<'info>,
    /// CHECK: Must match creator authority
    #[account(
        mut,
        address = creator_profile.authority @ ZNSError::InvalidCreatorAuthority
    )]
    pub creator_authority: AccountInfo<'info>,
    /// CHECK: Must match protocol treasury
    #[account(
        mut,
        address = global_registry.treasury @ ZNSError::InvalidTreasuryAddress
    )]
    pub treasury: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SendCreatorTip<'info> {
    #[account(
        seeds = [b"global_registry"],
        bump = global_registry.bump
    )]
    pub global_registry: Account<'info, GlobalRegistry>,
    #[account(
        mut,
        seeds = [b"creator", anchor_lang::solana_program::hash(&creator_profile.handle[..creator_profile.handle_len as usize]).to_bytes().as_ref()],
        bump = creator_profile.bump
    )]
    pub creator_profile: Account<'info, CreatorProfile>,
    #[account(mut)]
    pub supporter: Signer<'info>,
    /// CHECK: Verified against creator_profile.authority
    #[account(
        mut,
        address = creator_profile.authority @ ZNSError::InvalidCreatorAuthority
    )]
    pub creator_authority: AccountInfo<'info>,
    /// CHECK: Verified against global_registry.treasury
    #[account(
        mut,
        address = global_registry.treasury @ ZNSError::InvalidTreasuryAddress
    )]
    pub treasury: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateCreatorMetadata<'info> {
    #[account(
        mut,
        seeds = [b"creator", anchor_lang::solana_program::hash(&creator_profile.handle[..creator_profile.handle_len as usize]).to_bytes().as_ref()],
        bump = creator_profile.bump,
        has_one = authority @ ZNSError::InvalidCreatorAuthority
    )]
    pub creator_profile: Account<'info, CreatorProfile>,
    pub authority: Signer<'info>,
}

// -----------------------------------------------------------------------------
// STATE STRUCTS
// -----------------------------------------------------------------------------

#[account]
pub struct GlobalRegistry {
    pub admin: Pubkey,          // 32
    pub treasury: Pubkey,       // 32
    pub total_creators: u64,    // 8
    pub protocol_fee_bps: u16,  // 2
    pub bump: u8,               // 1
}

impl GlobalRegistry {
    pub const ACCOUNT_SIZE: usize = 8 + 32 + 32 + 8 + 2 + 1; // 83 bytes
}

#[account]
pub struct CreatorProfile {
    pub authority: Pubkey,                  // 32
    pub handle: [u8; MAX_HANDLE_LEN],       // 32
    pub handle_len: u8,                     // 1
    pub metadata_uri: [u8; MAX_METADATA_URI_LEN], // 96
    pub metadata_uri_len: u8,               // 1
    pub subdomain_base_fee: u64,            // 8
    pub subdomain_royalty_bps: u16,         // 2
    pub total_subdomains: u32,              // 4
    pub total_tips_received: u64,           // 8
    pub bump: u8,                           // 1
}

impl CreatorProfile {
    pub const ACCOUNT_SIZE: usize = 8 + 32 + 32 + 1 + 96 + 1 + 8 + 2 + 4 + 8 + 1; // 193 bytes
}

#[account]
pub struct SubdomainRecord {
    pub parent_creator: Pubkey,             // 32
    pub owner: Pubkey,                      // 32
    pub subdomain_name: [u8; MAX_HANDLE_LEN], // 32
    pub subdomain_len: u8,                  // 1
    pub tier: u8,                           // 1 (e.g. 1=Bronze, 2=Silver, 3=Gold)
    pub expiry_epoch: i64,                  // 8
    pub bump: u8,                           // 1
}

impl SubdomainRecord {
    pub const ACCOUNT_SIZE: usize = 8 + 32 + 32 + 32 + 1 + 1 + 8 + 1; // 115 bytes
}

// -----------------------------------------------------------------------------
// EVENTS & ERRORS
// -----------------------------------------------------------------------------

#[event]
pub struct RegistryInitialized {
    pub admin: Pubkey,
    pub treasury: Pubkey,
    pub protocol_fee_bps: u16,
}

#[event]
pub struct CreatorHandleRegistered {
    pub creator: Pubkey,
    pub handle: String,
    pub fee_paid: u64,
}

#[event]
pub struct FanSubdomainIssued {
    pub parent_creator: Pubkey,
    pub fan: Pubkey,
    pub subdomain: String,
    pub tier: u8,
    pub expiry_epoch: i64,
}

#[event]
pub struct CreatorTipped {
    pub creator: Pubkey,
    pub supporter: Pubkey,
    pub amount: u64,
    pub net_creator_amount: u64,
}

#[event]
pub struct CreatorMetadataUpdated {
    pub creator: Pubkey,
    pub new_subdomain_fee: u64,
    pub new_royalty_bps: u16,
}

#[error_code]
pub enum ZNSError {
    #[msg("Protocol fee exceeds maximum allowed basis points (2500 bps).")]
    ProtocolFeeTooHigh,
    #[msg("Handle must be between 1 and 32 characters.")]
    InvalidHandleLength,
    #[msg("Metadata URI must not exceed 96 bytes.")]
    MetadataUriTooLong,
    #[msg("Royalty bps cannot exceed 5000 bps (50%).")]
    RoyaltyBpsTooHigh,
    #[msg("Arithmetic overflow occurred.")]
    NumericalOverflow,
    #[msg("Duration epoch must be greater than zero.")]
    InvalidDuration,
    #[msg("Provided treasury does not match registered protocol treasury.")]
    InvalidTreasuryAddress,
    #[msg("Signer is not authorized to act as creator authority.")]
    InvalidCreatorAuthority,
    #[msg("Tip amount must be greater than zero.")]
    ZeroTipAmount,
}