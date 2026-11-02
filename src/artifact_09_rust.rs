use anchor_lang::prelude::*;
use anchor_lang::solana_program::hash::hash;
use crate::state::*;
use crate::errors::ZnsError;

#[derive(Accounts)]
#[instruction(sub_name: String)]
pub struct DelegateSubdomain<'info> {
    #[account(
        mut,
        has_one = owner @ ZnsError::Unauthorized
    )]
    pub root_domain: Account<'info, RootDomain>,

    #[account(
        init,
        payer = owner,
        space = SubDomain::LEN,
        seeds = [
            b"subdomain",
            root_domain.key().as_ref(),
            hash(sub_name.as_bytes()).to_bytes().as_ref()
        ],
        bump
    )]
    pub sub_domain: Account<'info, SubDomain>,

    /// The recipient/holder of this sub-domain
    pub delegate_to: SystemAccount<'info>,

    #[account(mut)]
    pub owner: Signer<'info>,
    pub system_program: Program<'info, System>,
}

pub fn handler(
    ctx: Context<DelegateSubdomain>,
    sub_name: String,
    duration_seconds: i64,
    tier: u8,
) -> Result<()> {
    require!(sub_name.len() <= MAX_NAME_LENGTH, ZnsError::NameTooLong);

    let clock = Clock::get()?;
    let expiry = if duration_seconds > 0 {
        clock.unix_timestamp.checked_add(duration_seconds).ok_or(ZnsError::MathOverflow)?
    } else {
        i64::MAX // Permanent delegation
    };

    let sub = &mut ctx.accounts.sub_domain;
    sub.parent_root = ctx.accounts.root_domain.key();
    sub.owner = ctx.accounts.delegate_to.key();
    sub.sub_name = sub_name;
    sub.expires_at = expiry;
    sub.tier = tier;
    sub.bump = ctx.bumps.sub_domain;

    let root = &mut ctx.accounts.root_domain;
    root.sub_domain_count = root.sub_domain_count.checked_add(1).ok_or(ZnsError::MathOverflow)?;

    Ok(())
}