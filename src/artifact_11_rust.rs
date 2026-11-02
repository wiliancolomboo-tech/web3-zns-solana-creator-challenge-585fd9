use anchor_lang::prelude::*;
use crate::state::*;
use crate::errors::ZnsError;

#[derive(Accounts)]
pub struct TipCreatorWithSplit<'info> {
    #[account(
        seeds = [b"global_config"],
        bump = config.bump
    )]
    pub config: Account<'info, GlobalConfig>,

    #[account(
        has_one = owner @ ZnsError::Unauthorized
    )]
    pub root_domain: Account<'info, RootDomain>,

    #[account(
        mut,
        seeds = [b"vault"],
        bump
    )]
    /// CHECK: Vault PDA for protocol cut
    pub protocol_vault: AccountInfo<'info>,

    #[account(mut)]
    /// CHECK: The domain owner receiving tip
    pub owner: AccountInfo<'info>,

    #[account(mut)]
    pub tipper: Signer<'info>,
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<TipCreatorWithSplit>, tip_lamports: u64) -> Result<()> {
    require!(tip_lamports > 0, ZnsError::InsufficientFunds);

    let config = &ctx.accounts.config;
    // Protocol share calculated via safe integer math: (tip * protocol_fee_bps) / 10000
    let protocol_fee = (tip_lamports as u128)
        .checked_mul(config.protocol_fee_bps as u128)
        .ok_or(ZnsError::MathOverflow)?
        .checked_div(10000)
        .ok_or(ZnsError::MathOverflow)? as u64;

    let creator_amount = tip_lamports
        .checked_sub(protocol_fee)
        .ok_or(ZnsError::MathOverflow)?;

    // 1. Transfer Creator Portion
    if creator_amount > 0 {
        anchor_lang::system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                anchor_lang::system_program::Transfer {
                    from: ctx.accounts.tipper.to_account_info(),
                    to: ctx.accounts.owner.to_account_info(),
                },
            ),
            creator_amount,
        )?;
    }

    // 2. Transfer Protocol Vault Portion
    if protocol_fee > 0 {
        anchor_lang::system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                anchor_lang::system_program::Transfer {
                    from: ctx.accounts.tipper.to_account_info(),
                    to: ctx.accounts.protocol_vault.to_account_info(),
                },
            ),
            protocol_fee,
        )?;
    }

    Ok(())
}