use anchor_lang::prelude::*;
use anchor_lang::solana_program::hash::hash;
use crate::state::*;
use crate::errors::ZnsError;

#[derive(Accounts)]
#[instruction(record_key: String)]
pub struct SetResolverRecord<'info> {
    #[account(
        has_one = owner @ ZnsError::Unauthorized
    )]
    pub root_domain: Account<'info, RootDomain>,

    #[account(
        init_if_needed,
        payer = owner,
        space = ResolverRecord::LEN,
        seeds = [
            b"resolver",
            root_domain.key().as_ref(),
            hash(record_key.as_bytes()).to_bytes().as_ref()
        ],
        bump
    )]
    pub resolver_record: Account<'info, ResolverRecord>,

    #[account(mut)]
    pub owner: Signer<'info>,
    pub system_program: Program<'info, System>,
}

pub fn handler(
    ctx: Context<SetResolverRecord>,
    record_key: String,
    record_value: Vec<u8>,
) -> Result<()> {
    require!(record_key.len() <= MAX_RECORD_KEY_LEN, ZnsError::KeyTooLong);
    require!(record_value.len() <= MAX_RECORD_VAL_LEN, ZnsError::ValueTooLong);

    let clock = Clock::get()?;
    let record = &mut ctx.accounts.resolver_record;

    record.parent_entity = ctx.accounts.root_domain.key();
    record.record_key = record_key;
    record.record_value = record_value;
    record.updated_at = clock.unix_timestamp;
    record.bump = ctx.bumps.resolver_record;

    Ok(())
}