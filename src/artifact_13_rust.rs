use anchor_lang::prelude::*;
use crate::errors::ZnsError;
use crate::state::DomainRecord;

#[derive(Accounts)]
pub struct TransferDomain<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: New target owner
    pub new_owner: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = domain_record.owner == authority.key() @ ZnsError::Unauthorized,
        constraint = domain_record.expires_at > Clock::get()?.unix_timestamp @ ZnsError::DomainExpired
    )]
    pub domain_record: Account<'info, DomainRecord>,
}

pub fn handler(ctx: Context<TransferDomain>) -> Result<()> {
    let domain = &mut ctx.accounts.domain_record;
    let previous_owner = domain.owner;
    domain.owner = ctx.accounts.new_owner.key();

    emit!(DomainTransferredEvent {
        domain: domain.key(),
        old_owner: previous_owner,
        new_owner: domain.owner,
    });

    Ok(())
}

#[event]
pub struct DomainTransferredEvent {
    pub domain: Pubkey,
    pub old_owner: Pubkey,
    pub new_owner: Pubkey,
}