# 1. Initialize anchor project structure if building from scratch
mkdir -p zns-solana && cd zns-solana
anchor init . --javascript false

# 2. Insert the program code into programs/zns-solana/src/lib.rs
# 3. Insert the TypeScript test suite into tests/zns-solana.ts

# 4. Build the smart contract binary
anchor build