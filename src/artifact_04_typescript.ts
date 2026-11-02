import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { ZnsSolana } from "../target/types/zns_solana";
import { PublicKey, Keypair, SystemProgram, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { expect } from "chai";
import { createHash } from "crypto";

describe("🟢 ZNS Solana Creator Registry & Monitization E2E Suite", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.ZnsSolana as Program<ZnsSolana>;

  const admin = Keypair.generate();
  const treasury = Keypair.generate();
  const creatorAlice = Keypair.generate();
  const collaboratorBob = Keypair.generate();
  const fanCharlie = Keypair.generate();

  const domainName = "alice_creator";
  const nameHash = createHash("sha256").update(domainName).digest();

  let configPda: PublicKey;
  let domainPda: PublicKey;
  let reversePda: PublicKey;

  before(async () => {
    // Fund test actors
    for (const actor of [admin, creatorAlice, collaboratorBob, fanCharlie]) {
      const sig = await provider.connection.requestAirdrop(actor.publicKey, 10 * LAMPORTS_PER_SOL);
      const latestBlockhash = await provider.connection.getLatestBlockhash();
      await provider.connection.confirmTransaction({
        signature: sig,
        ...latestBlockhash,
      });
    }

    [configPda] = PublicKey.findProgramAddressSync([Buffer.from("config")], program.programId);

    [domainPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("zns_domain"), nameHash],
      program.programId
    );

    [reversePda] = PublicKey.findProgramAddressSync(
      [Buffer.from("zns_reverse"), creatorAlice.publicKey.toBuffer()],
      program.programId
    );
  });

  it("1. Initializes the ZNS Global Configuration", async () => {
    const feeBps = 200; // 2%
    const baseFee = new anchor.BN(0.05 * LAMPORTS_PER_SOL);

    await program.methods
      .initialize(feeBps, baseFee)
      .accounts({
        config: configPda,
        treasury: treasury.publicKey,
        authority: admin.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([admin])
      .rpc();

    const configAccount = await program.account.config.fetch(configPda);
    expect(configAccount.authority.toBase58()).to.equal(admin.publicKey.toBase58());
    expect(configAccount.treasury.toBase58()).to.equal(treasury.publicKey.toBase58());
    expect(configAccount.feeBasisPoints).to.equal(feeBps);
    expect(configAccount.totalRegistered.toNumber()).to.equal(0);
  });

  it("2. Registers a domain with valid syntax and collaborator split", async () => {
    const duration = new anchor.BN(86400 * 365); // 1 year
    const splitBps = 2000; // 20% to Bob
    const contentHash = "ipfs://QmZtmD2qt8fJpq3rBsEQZRiZHSn45nHYaswqunB685kcw5";

    const initialTreasuryBalance = await provider.connection.getBalance(treasury.publicKey);

    await program.methods
      .registerDomain(
        domainName,
        duration,
        contentHash,
        collaboratorBob.publicKey,
        splitBps
      )
      .accounts({
        config: configPda,
        domainRecord: domainPda,
        payer: creatorAlice.publicKey,
        owner: creatorAlice.publicKey,
        treasury: treasury.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([creatorAlice])
      .rpc();

    const domainAcc = await program.account.domainRecord.fetch(domainPda);
    expect(domainAcc.owner.toBase58()).to.equal(creatorAlice.publicKey.toBase58());
    expect(domainAcc.collaborator.toBase58()).to.equal(collaboratorBob.publicKey.toBase58());
    expect(domainAcc.collaboratorSplitBps).to.equal(2000);
    expect(domainAcc.name).to.equal(domainName);
    expect(domainAcc.contentHash).to.equal(contentHash);

    const postTreasuryBalance = await provider.connection.getBalance(treasury.publicKey);
    expect(postTreasuryBalance).to.be.greaterThan(initialTreasuryBalance);
  });

  it("3. Rejects illegal domain names (capitalization, special chars)", async () => {
    const invalidName = "Alice.Sol!";
    const invalidHash = createHash("sha256").update(invalidName).digest();
    const [invalidPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("zns_domain"), invalidHash],
      program.programId
    );

    try {
      await program.methods
        .registerDomain(
          invalidName,
          new anchor.BN(86400 * 30),
          null,
          null,
          0
        )
        .accounts({
          config: configPda,
          domainRecord: invalidPda,
          payer: creatorAlice.publicKey,
          owner: creatorAlice.publicKey,
          treasury: treasury.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([creatorAlice])
        .rpc();
      expect.fail("Should have failed on invalid characters");
    } catch (err: any) {
      expect(err.error.errorCode.code).to.equal("InvalidCharacter");
    }
  });

  it("4. Sets and resolves Reverse Record deterministically", async () => {
    await program.methods
      .setReverseRecord(domainName)
      .accounts({
        reverseRecord: reversePda,
        domainRecord: domainPda,
        owner: creatorAlice.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([creatorAlice])
      .rpc();

    const reverseAcc = await program.account.reverseRecord.fetch(reversePda);
    expect(reverseAcc.owner.toBase58()).to.equal(creatorAlice.publicKey.toBase58());
    expect(reverseAcc.domainName).to.equal(domainName);
  });

  it("5. Delegates a Subdomain to a Community Member", async () => {
    const subPrefix = "vip";
    const subHash = createHash("sha256").update(subPrefix).digest();

    const [subPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("zns_subdomain"), domainPda.toBuffer(), subHash],
      program.programId
    );

    await program.methods
      .createSubdomain(subPrefix, fanCharlie.publicKey)
      .accounts({
        subdomainRecord: subPda,
        parentDomain: domainPda,
        owner: creatorAlice.publicKey,
        authority: creatorAlice.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([creatorAlice])
      .rpc();

    const subAcc = await program.account.subdomainRecord.fetch(subPda);
    expect(subAcc.parentDomain.toBase58()).to.equal(domainPda.toBase58());
    expect(subAcc.owner.toBase58()).to.equal(fanCharlie.publicKey.toBase58());
    expect(subAcc.subdomainPrefix).to.equal("vip");
  });

  it("6. Executes Fan Tipping with Exact Fee Split Routing", async () => {
    const tipAmount = new anchor.BN(1.0 * LAMPORTS_PER_SOL);

    const preAlice = await provider.connection.getBalance(creatorAlice.publicKey);
    const preBob = await provider.connection.getBalance(collaboratorBob.publicKey);
    const preTreasury = await provider.connection.getBalance(treasury.publicKey);

    await program.methods
      .tipCreator(tipAmount)
      .accounts({
        config: configPda,
        domainRecord: domainPda,
        tipper: fanCharlie.publicKey,
        domainOwner: creatorAlice.publicKey,
        collaborator: collaboratorBob.publicKey,
        treasury: treasury.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([fanCharlie])
      .rpc();

    const postAlice = await provider.connection.getBalance(creatorAlice.publicKey);
    const postBob = await provider.connection.getBalance(collaboratorBob.publicKey);
    const postTreasury = await provider.connection.getBalance(treasury.publicKey);

    // Tip = 1.0 SOL
    // Fee = 2% = 0.02 SOL
    // Post Fee = 0.98 SOL
    // Bob Split = 20% of 0.98 SOL = 0.196 SOL
    // Alice Primary = 0.98 - 0.196 = 0.784 SOL

    const expectedTreasuryDelta = 0.02 * LAMPORTS_PER_SOL;
    const expectedBobDelta = 0.196 * LAMPORTS_PER_SOL;
    const expectedAliceDelta = 0.784 * LAMPORTS_PER_SOL;

    expect(postTreasury - preTreasury).to.equal(expectedTreasuryDelta);
    expect(postBob - preBob).to.equal(expectedBobDelta);
    expect(postAlice - preAlice).to.equal(expectedAliceDelta);
  });
});