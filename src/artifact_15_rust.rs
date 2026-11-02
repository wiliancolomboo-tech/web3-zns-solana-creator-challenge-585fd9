use anchor_lang::prelude::*;

pub mod errors;
pub mod instructions;
pub mod state;

use instructions::*;

declare_id!("ZNS1111111111111111111111111111111111111111");

#[program]
pub mod zns_solana {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>, base_fee_per_year: u64) -> Result<()> {
        instructions::initialize::handler(ctx, base_fee_per_year)
    }

    pub fn register_domain(ctx: Context<RegisterDomain>, name: String, years: u8) -> Result<()> {
        instructions::register::handler(ctx, name, years)
    }

    pub fn create_subdomain(ctx: Context<CreateSubdomain>, sub_name: String) -> Result<()> {
        instructions::subdomain::handler(ctx, sub_name)
    }

    pub fn set_record(ctx: Context<SetRecord>, key: String, value: String) -> Result<()> {
        instructions::records::handler(ctx, key, value)
    }

    pub fn set_primary_domain(ctx: Context<SetPrimaryDomain>) -> Result<()> {
        instructions::reverse::handler(ctx)
    }

    pub fn renew_domain(ctx: Context<RenewDomain>, years: u8) -> Result<()> {
        instructions::renew::handler(ctx, years)
    }

    pub fn transfer_domain(ctx: Context<TransferDomain>) -> Result<()> {
        instructions::transfer::handler(ctx)
    }
}