# 1. Set Solana CLI network target
solana config set --url https://api.devnet.solana.com

# 2. Fund your deployment wallet
solana airdrop 2

# 3. Build & Deploy Anchor Program
anchor build
anchor deploy --provider.cluster devnet

# 4. Initialize the Protocol Registry via Anchor CLI / Client Script
npx ts-node scripts/init-registry.ts --network devnet --fee-bps 500