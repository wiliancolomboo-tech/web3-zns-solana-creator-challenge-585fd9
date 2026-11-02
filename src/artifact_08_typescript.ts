// Initialize Root TLD '.creator'
await program.methods
  .initializeTld("creator", new anchor.BN(50000000), 500)
  .accounts({ tld: tldPda, treasury: treasuryPubkey, authority: adminPubkey })
  .rpc();

// Mint user domain 'alex.creator'
await program.methods
  .registerDomain("alex", 1)
  .accounts({ tld: tldPda, domain: domainPda, treasury: treasuryPubkey, owner: alexPubkey, payer: alexPubkey })
  .rpc();

// Tip creator directly via resolver
await program.methods
  .tipCreator(new anchor.BN(100000000))
  .accounts({ creatorProfile: profilePda, creatorOwner: alexPubkey, partnerAccount: partnerPubkey, tipper: tipperPubkey })
  .rpc();