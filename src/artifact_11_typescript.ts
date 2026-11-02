// Initialize Root Namespace via Web3 script
const ns = "creators";
const hash = crypto.createHash("sha256").update(ns).digest();
const [pda] = PublicKey.findProgramAddressSync([Buffer.from("root_namespace"), hash], program.programId);
const [vault] = PublicKey.findProgramAddressSync([Buffer.from("creator_vault"), Buffer.from(ns)], program.programId);

await program.methods.initializeRootNamespace(
  ns,
  new anchor.BN(50000000), // 0.05 SOL base
  new anchor.BN(20000000)  // 0.02 SOL annual
).accounts({
  rootNamespace: pda,
  vault: vault,
  authority: wallet.publicKey,
  systemProgram: SystemProgram.programId,
}).rpc();