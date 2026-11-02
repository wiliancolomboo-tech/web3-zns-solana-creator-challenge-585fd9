# Generate a new program keypair if needed
solana-keygen new -o target/deploy/zns_solana-keypair.json --no-bip39-passphrase --force

# Grab public address
export PROGRAM_ID=$(solana-keygen pubkey target/deploy/zns_solana-keypair.json)
echo "Program ID: $PROGRAM_ID"

# Update Anchor.toml and declare_id!() in lib.rs with the generated PROGRAM_ID
anchor keys sync