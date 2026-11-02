use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};
use crate::errors::ZnsError;
use crate::state::{Config, DomainRecord, CONFIG_SEED, SECONDS_PER_YEAR, VAULT_SEED};

#[derive(Accounts)]
#[instruction(years: u8)]
pub struct RenewDomain<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump,
        constraint = !config.is_paused @ ZnsError::ProgramPaused
    )]
    pub config: Account<'info, Config>,

    #[account(mut)]
    pub domain_record: Account<'info, DomainRecord>,

    /// CHECK: Vault destination PDA
    #[account(
        mut,
        seeds = [VAULT_SEED, config.key().as_ref()],
        bump = config.vault_bump
    )]
    pub vault: SystemAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<RenewDomain>, years: u8) -> Result<()> {
    require!(years >= 1 && years <= 10, ZnsError::InvalidDuration);

    let domain = &mut ctx.accounts.domain_record;
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;

    // Renew extends from the current expiration if unexpired, or from current time if expired
    let base_time = if domain.expires_at > now {
        domain.expires_at
    } else {
        now
    };

    let duration_seconds = (years as i64)
        .checked_mul(SECONDS_PER_YEAR)
        .ok_or(ZnsError::MathOverflow)?;
    let new_expiration = base_time
        .checked_add(duration_seconds)
        .ok_or(ZnsError::MathOverflow)?;

    let total_cost = (ctx.accounts.config.base_fee_per_year as u128)
        .checked_mul(years as u128)
        .ok_or(ZnsError::MathOverflow)? as u64;

    if total_cost > 0 {
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.payer.to_account_info(),
                to: ctx.accounts.vault.to_account_info(),
            },
        );
        transfer(cpi_ctx, total_cost)?;
    }

    domain.expires_at = new_expiration;

    emit!(DomainRenewedEvent {
        domain: domain.key(),
        new_expires_at: new_expiration,
        cost_lamports: total_cost
    });

    Ok(())
}

#[event]
pub struct DomainRenewedEvent {
    pub domain: Pubkey,
    pub new_expires_at: i64,
    pub cost_lamports: u64,
}