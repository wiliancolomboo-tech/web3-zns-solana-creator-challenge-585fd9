# Configure Solana CLI for Devnet
solana config set --url https://api.devnet.solana.com

# Fund deployer wallet
solana airdrop 2

# Deploy binary
anchor deploy --provider.cluster devnet