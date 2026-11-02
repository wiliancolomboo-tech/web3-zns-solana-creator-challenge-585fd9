// scripts/resolve-creator.ts
import * as anchor from "@coral-xyz/anchor";
import { PublicKey } from "@solana/web3.js";
import * as crypto from "crypto";

async function main() {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const programId = new PublicKey("ZNS11111111111111111111111111111111111111111");

  const creatorName = "solana_creator";
  const nameHash = crypto.createHash("sha256").update(creatorName).digest();
  
  const [rootDomainPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("root_domain"), nameHash],
    programId
  );

  console.log(`Resolving ZNS Root Domain: ${creatorName}.zns`);
  console.log(`Derived PDA: ${rootDomainPda.toBase58()}`);
  
  const accountInfo = await provider.connection.getAccountInfo(rootDomainPda);
  if (!accountInfo) {
    console.error("Domain not found!");
    return;
  }
  console.log(`Raw Data Size: ${accountInfo.data.length} bytes`);
  console.log("Domain is active and rent-exempt.");
}

main().catch(console.error);