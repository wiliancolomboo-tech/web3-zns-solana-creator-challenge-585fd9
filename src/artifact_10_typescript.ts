import { PublicKey } from "@solana/web3.js";
import { Program, AnchorProvider } from "@coral-xyz/anchor";
import { ZnsCreator, IDL } from "./target/types/zns_creator";

const PROGRAM_ID = new PublicKey("ZNSCrEAtor111111111111111111111111111111111");

export async function resolveCreatorSubdomain(
  provider: AnchorProvider,
  namespaceName: string,
  subdomainName: string
) {
  const program = new Program<ZnsCreator>(IDL, PROGRAM_ID, provider);

  const [namespacePda] = PublicKey.findProgramAddressSync(
    [Buffer.from("namespace"), Buffer.from(namespaceName)],
    PROGRAM_ID
  );

  const [subdomainPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("subdomain"), namespacePda.toBuffer(), Buffer.from(subdomainName)],
    PROGRAM_ID
  );

  const record = await program.account.subdomainRecord.fetch(subdomainPda);
  return {
    owner: record.owner.toBase58(),
    tipJar: record.tipJar.toBase58(),
    contentHash: record.contentHash,
    expiresAt: new Date(record.expiresAt.toNumber() * 1000),
  };
}