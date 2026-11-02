import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { ZnsCreator } from "../target/types/zns_creator";
import { PublicKey, Keypair, SystemProgram, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { assert, expect } from "chai";

describe("zns-creator protocol test suite", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.ZnsCreator as Program<ZnsCreator>;

  const admin = Keypair.generate();
  const creator = Keypair.generate();
  const collaboratorA = Keypair.generate();
  const collaboratorB = Keypair.generate();
  const fanTipper = Keypair.generate();

  const tld = "creator";
  const domainHandle = "solanamaster";
  const annualFee = new anchor.BN(0.05 * LAMPORTS_PER_SOL);

  let registryPda: PublicKey;
  let domainPda: PublicKey;
  let splitConfigPda: PublicKey;
  let contentRecordPda: PublicKey;
  let reverseLookupPda: PublicKey;

  before(async () => {
    // Airdrop SOL to test actors
    const airdropSigners = [admin, creator, fanTipper];
    for (const actor of airdropSigners) {
      const sig = await provider.connection.requestAirdrop(actor.publicKey, 10 * LAMPORTS_PER_SOL);
      const latestBlockhash = await provider.connection.getLatestBlockhash();
      await provider.connection.confirmTransaction({
        signature: sig,
        blockhash: latestBlockhash.blockhash,
        lastValidBlockHeight: latestBlockhash.lastValidBlockHeight,
      });
    }

    [registryPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("registry"), Buffer.from(tld)],
      program.programId
    );

    [domainPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("domain"), Buffer.from(tld), Buffer.from(domainHandle)],
      program.programId
    );

    [splitConfigPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("split"), domainPda.toBuffer()],
      program.programId
    );

    [reverseLookupPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("reverse"), creator.publicKey.toBuffer()],
      program.programId
    );
  });

  it("1. Initializes the Root Registry for TLD .creator", async () => {
    const tx = await program.methods
      .initializeRegistry(tld, annualFee)
      .accounts({
        registry: registryPda,
        admin: admin.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([admin])
      .rpc();

    const regAccount = await program.account.rootRegistry.fetch(registryPda);
    assert.equal(regAccount.tld, tld);
    assert.equal(regAccount.totalDomains.toNumber(), 0);
    assert.isTrue(regAccount.annualFeeLamports.eq(annualFee));
    console.log(`✔ TLD Registry .${tld} initialized at ${registryPda.toBase58()}`);
  });

  it("2. Registers 'solanamaster.creator' for 2 years with fee transfer", async () => {
    const initialAdminBalance = await provider.connection.getBalance(admin.publicKey);

    const tx = await program.methods
      .registerDomain(domainHandle, 2)
      .accounts({
        registry: registryPda,
        domain: domainPda,
        admin: admin.publicKey,
        owner: creator.publicKey,
        payer: creator.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([creator])
      .rpc();

    const domainAccount = await program.account.domainNameRecord.fetch(domainPda);
    assert.equal(domainAccount.name, domainHandle);
    assert.equal(domainAccount.tld, tld);
    assert.isTrue(domainAccount.owner.equals(creator.publicKey));

    const postAdminBalance = await provider.connection.getBalance(admin.publicKey);
    const expectedFee = annualFee.toNumber() * 2;
    assert.equal(postAdminBalance - initialAdminBalance, expectedFee);
    console.log(`✔ Domain '${domainHandle}.${tld}' registered for 2 years`);
  });

  it("3. Attaches dynamic Arweave content record and social pointer", async () => {
    const recordKey = "ipfs_hash";
    const ipfsHash = "QmZ4tDuvesekSs4qM5ZBKpXiZGun7S2CYtEZRB3DYXkjGx";

    [contentRecordPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("record"), domainPda.toBuffer(), Buffer.from(recordKey)],
      program.programId
    );

    await program.methods
      .setDomainRecord(recordKey, ipfsHash)
      .accounts({
        domain: domainPda,
        record: contentRecordPda,
        owner: creator.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([creator])
      .rpc();

    const record = await program.account.contentRecord.fetch(contentRecordPda);
    assert.equal(record.key, recordKey);
    assert.equal(record.value, ipfsHash);
    console.log(`✔ Dynamic metadata pointer mapped: ${record.key} -> ${record.value}`);
  });

  it("4. Configures Split Escrow (70% Primary, 20% Collab A, 10% Collab B)", async () => {
    const shares = [
      { recipient: creator.publicKey, basisPoints: 7000 },
      { recipient: collaboratorA.publicKey, basisPoints: 2000 },
      { recipient: collaboratorB.publicKey, basisPoints: 1000 },
    ];

    await program.methods
      .configureSplitEscrow(shares)
      .accounts({
        domain: domainPda,
        splitConfig: splitConfigPda,
        owner: creator.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([creator])
      .rpc();

    const split = await program.account.splitConfig.fetch(splitConfigPda);
    assert.equal(split.recipientsCount, 3);
    assert.equal(split.recipients[0].basisPoints, 7000);
    assert.equal(split.recipients[1].basisPoints, 2000);
    assert.equal(split.recipients[2].basisPoints, 1000);
    console.log("✔ Split escrow configuration confirmed (Total: 10,000 BPS)");
  });

  it("5. Fans tip handle directly: Atomically disperses SOL across all splits", async () => {
    const tipAmount = new anchor.BN(1.0 * LAMPORTS_PER_SOL);

    const initBalCreator = await provider.connection.getBalance(creator.publicKey);
    const initBalCollabA = await provider.connection.getBalance(collaboratorA.publicKey);
    const initBalCollabB = await provider.connection.getBalance(collaboratorB.publicKey);

    await program.methods
      .tipAndSplit(tipAmount)
      .accounts({
        domain: domainPda,
        splitConfig: splitConfigPda,
        domainOwner: creator.publicKey,
        tipper: fanTipper.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts([
        { pubkey: creator.publicKey, isWritable: true, isSigner: false },
        { pubkey: collaboratorA.publicKey, isWritable: true, isSigner: false },
        { pubkey: collaboratorB.publicKey, isWritable: true, isSigner: false },
      ])
      .signers([fanTipper])
      .rpc();

    const diffCreator = (await provider.connection.getBalance(creator.publicKey)) - initBalCreator;
    const diffCollabA = (await provider.connection.getBalance(collaboratorA.publicKey)) - initBalCollabA;
    const diffCollabB = (await provider.connection.getBalance(collaboratorB.publicKey)) - initBalCollabB;

    assert.equal(diffCreator, 0.7 * LAMPORTS_PER_SOL);
    assert.equal(diffCollabA, 0.2 * LAMPORTS_PER_SOL);
    assert.equal(diffCollabB, 0.1 * LAMPORTS_PER_SOL);

    console.log("✔ Tip dispersed atomically with zero dust loss:");
    console.log(`   - Creator: +${diffCreator / LAMPORTS_PER_SOL} SOL (70%)`);
    console.log(`   - Collab A: +${diffCollabA / LAMPORTS_PER_SOL} SOL (20%)`);
    console.log(`   - Collab B: +${diffCollabB / LAMPORTS_PER_SOL} SOL (10%)`);
  });

  it("6. Binds reverse resolution: Pubkey -> Domain PDA", async () => {
    await program.methods
      .setReverseLookup()
      .accounts({
        domain: domainPda,
        reverseRecord: reverseLookupPda,
        owner: creator.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([creator])
      .rpc();

    const rev = await program.account.reverseLookup.fetch(reverseLookupPda);
    assert.isTrue(rev.targetDomain.equals(domainPda));
    assert.isTrue(rev.owner.equals(creator.publicKey));
    console.log(`✔ Reverse lookup validated: ${creator.publicKey.toBase58()} resolves to ${domainPda.toBase58()}`);
  });

  it("7. Security: Rejects split configuration exceeding 10,000 BPS", async () => {
    const badShares = [
      { recipient: creator.publicKey, basisPoints: 6000 },
      { recipient: collaboratorA.publicKey, basisPoints: 5000 }, // Sum: 11,000 BPS
    ];

    try {
      await program.methods
        .configureSplitEscrow(badShares)
        .accounts({
          domain: domainPda,
          splitConfig: splitConfigPda,
          owner: creator.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([creator])
        .rpc();
      assert.fail("Should have failed with InvalidTotalBasisPoints");
    } catch (err: any) {
      expect(err.error.errorCode.code).to.equal("InvalidTotalBasisPoints");
      console.log("✔ Security check verified: InvalidTotalBasisPoints was blocked");
    }
  });

  it("8. Security: Prevents unauthorized users from updating records", async () => {
    const maliciousActor = Keypair.generate();
    const sig = await provider.connection.requestAirdrop(maliciousActor.publicKey, LAMPORTS_PER_SOL);
    await provider.connection.confirmTransaction(sig);

    try {
      await program.methods
        .setDomainRecord("youtube", "@hacked")
        .accounts({
          domain: domainPda,
          record: contentRecordPda,
          owner: maliciousActor.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([maliciousActor])
        .rpc();
      assert.fail("Unauthorized signer should be rejected");
    } catch (err: any) {
      expect(err.error.errorCode.code).to.equal("Unauthorized");
      console.log("✔ Security check verified: Unauthorized signer blocked");
    }
  });
});