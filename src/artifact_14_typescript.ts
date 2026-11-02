import { PublicKey } from "@solana/web3.js";

// Deterministic Reverse Lookup Calculation
const [reversePda] = PublicKey.findProgramAddressSync(
  [Buffer.from("reverse"), creatorPublicKey.toBuffer()],
  new PublicKey("ZNScre8orHub111111111111111111111111111111")
);

console.log("Derived Reverse Lookup Address:", reversePda.toBase58());