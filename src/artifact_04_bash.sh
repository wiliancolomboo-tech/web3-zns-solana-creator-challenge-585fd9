# Set Solana CLI to localnet
solana config set --url localhost

# Generate Local Deployer Keypair (if not present)
solana-keygen new --no-bip39-passphrase --outfile ~/.config/solana/id.json -f