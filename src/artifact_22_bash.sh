# Switch config to Devnet
solana config set --url https://api.devnet.solana.com

# Ensure deployer wallet has Devnet SOL
solana airdrop 2

# Deploy the program to Devnet
anchor deploy --provider.cluster devnet

# Verify program deployment on Solana Explorer
# https://explorer.solana.com/address/<PROGRAM_ID>?cluster=devnet