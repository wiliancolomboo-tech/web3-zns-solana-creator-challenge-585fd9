use anchor_lang::prelude::*;
use anchor_lang::solana_program::hash::hash;
use crate::state::*;
use crate::errors::ZnsError;

#[derive(Accounts)]
#[instruction(name: String)]
pub struct RegisterRootDomain<'info> {
    #[account(
        mut,
        seeds = [b"global_config"],
        bump = config.bump
    )]
    pub config: Account<'info, GlobalConfig>,

    #[account(
        init,
        payer = creator,
        space = RootDomain::LEN,
        seeds = [b"root_domain", hash(name.as_bytes()).to_bytes().as_ref()],
        bump
    )]
    pub root_domain: Account<'info, RootDomain>,

    #[account(
        mut,
        seeds = [b"vault"],
        bump
    )]
    /// CHECK: Vault PDA collecting base domain creation fees
    pub protocol_vault: AccountInfo<'info>,

    #[account(mut)]
    pub creator: Signer<'info>,
    pub system_program: Program<'info, System>,
}

pub fn handler(
    ctx: Context<RegisterRootDomain>,
    name: String,
    metadata_uri: String,
    custom_fee_bps: u16,
) -> Result<()> {
    require!(name.len() <= MAX_NAME_LENGTH, ZnsError::NameTooLong);
    require!(metadata_uri.len() <= MAX_URI_LENGTH, ZnsError::UriTooLong);
    require!(custom_fee_bps <= 10000, ZnsError::InvalidFeeBps);

    // Dynamic base cost: 0.05 SOL base registration fee
    let base_fee: u64 = 50_000_000;
    
    // Transfer fee to Protocol Vault via System Program CPI
    anchor_lang::system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.creator.to_account_info(),
                to: ctx.accounts.protocol_vault.to_account_info(),
            },
        ),
        base_fee,
    )?;

    let domain = &mut ctx.accounts.root_domain;
    let clock = Clock::get()?;

    domain.owner = ctx.accounts.creator.key();
    domain.name = name;
    domain.metadata_uri = metadata_uri;
    domain.custom_fee_bps = custom_fee_bps;
    domain.sub_domain_count = 0;
    domain.bump = ctx.bumps.root_domain;
    domain.created_at = clock.unix_timestamp;

    let config = &mut ctx.accounts.config;
    config.total_domains = config.total_domains.checked_add(1).ok_or(ZnsError::MathOverflow)?;

    Ok(())
}