# Configure Solana CLI for Devnet
solana config set --url https://api.devnet.solana.com

# Ensure your wallet has Devnet SOL
solana airdrop 2

# Deploy the compiled program
anchor deploy --provider.cluster devnet

# Verify the program on-chain
solana program show $PROGRAM_ID