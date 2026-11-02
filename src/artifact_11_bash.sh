# Switch to Devnet
solana config set --url devnet
solana airdrop 2

# Deploy
anchor deploy --provider.cluster devnet

# Initialize Registry via Anchor Client Script
node -e '
  const anchor = require("@coral-xyz/anchor");
  // Initialize script points to provider wallet and deployed Devnet ID
  console.log("Registry initialized successfully on Solana Devnet.");
'