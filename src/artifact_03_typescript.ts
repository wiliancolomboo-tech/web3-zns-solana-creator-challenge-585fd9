import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { ZnsCreator } from "../target/types/zns_creator";
import { PublicKey, Keypair, SystemProgram, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { expect } from "chai";
import { createHash } from "crypto";

describe("zns-creator protocol tests", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.ZnsCreator as Program<ZnsCreator>;

  const admin = Keypair.generate();
  const creator = Keypair.generate();
  const collaborator = Keypair.generate();
  const fanTipper = Keypair.generate();

  const domainName = "solcreator";
  const domainHash = createHash("sha256").update(domainName).digest();

  let registrarPda: PublicKey;
  let nameRecordPda: PublicKey;
  let reverseRecordPda: PublicKey;
  let profilePda: PublicKey;

  before(async () => {
    // Fund test actors
    for (const kp of [admin, creator, collaborator, fanTipper]) {
      const sig = await provider.connection.requestAirdrop(kp.publicKey, 5 * LAMPORTS_PER_SOL);
      await provider.connection.confirmTransaction(sig);
    }

    [registrarPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("registrar")],
      program.programId
    );

    [nameRecordPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("zns"), domainHash],
      program.programId
    );

    [reverseRecordPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("reverse"), creator.publicKey.toBuffer()],
      program.programId
    );

    [profilePda] = PublicKey.findProgramAddressSync(
      [Buffer.from("profile"), nameRecordPda.toBuffer()],
      program.programId
    );
  });

  it("1. Initializes Registrar configuration with tiered pricing", async () => {
    const feeShort = new anchor.BN(0.5 * LAMPORTS_PER_SOL);
    const feeLong = new anchor.BN(0.1 * LAMPORTS_PER_SOL);

    await program.methods
      .initializeRegistrar(feeShort, feeLong)
      .accounts({
        registrar: registrarPda,
        admin: admin.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([admin])
      .rpc();

    const regAccount = await program.account.registrar.fetch(registrarPda);
    expect(regAccount.admin.toBase58()).to.equal(admin.publicKey.toBase58());
    expect(regAccount.baseFeeLong.toNumber()).to.equal(feeLong.toNumber());
  });

  it("2. Registers handle and generates forward/reverse pointers", async () => {
    const years = 2;
    await program.methods
      .registerDomain(domainName, years)
      .accounts({
        registrar: registrarPda,
        nameRecord: nameRecordPda,
        reverseRecord: reverseRecordPda,
        owner: creator.publicKey,
        admin: admin.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([creator])
      .rpc();

    const nameAccount = await program.account.nameRecord.fetch(nameRecordPda);
    expect(nameAccount.owner.toBase58()).to.equal(creator.publicKey.toBase58());

    const revAccount = await program.account.reverseRecord.fetch(reverseRecordPda);
    const resolvedName = Buffer.from(revAccount.name).subarray(0, revAccount.nameLen).toString("utf-8");
    expect(resolvedName).to.equal(domainName);
  });

  it("3. Configures Creator Profile & 70/30 Split Rule", async () => {
    const arweaveUri = "ar://V5n6x6F294uXf6dE1xM_ZnsCreatorMetadataAnchorHash";
    const splits = [
      {
        recipient: creator.publicKey,
        basisPoints: 7000, // 70%
      },
      {
        recipient: collaborator.publicKey,
        basisPoints: 3000, // 30%
      },
    ];

    await program.methods
      .updateCreatorProfile(arweaveUri, splits)
      .accounts({
        nameRecord: nameRecordPda,
        creatorProfile: profilePda,
        owner: creator.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([creator])
      .rpc();

    const profile = await program.account.creatorProfile.fetch(profilePda);
    expect(profile.splitsCount).to.equal(2);
    expect(profile.splits[0].basisPoints).to.equal(7000);
    expect(profile.splits[1].basisPoints).to.equal(3000);
  });

  it("4. Executes Direct Atomic Tip with Automated Split Distribution", async () => {
    const tipAmount = new anchor.BN(1 * LAMPORTS_PER_SOL);
    const creatorBalBefore = await provider.connection.getBalance(creator.publicKey);
    const collabBalBefore = await provider.connection.getBalance(collaborator.publicKey);

    await program.methods
      .resolveAndTip(tipAmount)
      .accounts({
        nameRecord: nameRecordPda,
        creatorProfile: profilePda,
        owner: creator.publicKey,
        tipper: fanTipper.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts([
        { pubkey: creator.publicKey, isWritable: true, isSigner: false },
        { pubkey: collaborator.publicKey, isWritable: true, isSigner: false },
      ])
      .signers([fanTipper])
      .rpc();

    const creatorBalAfter = await provider.connection.getBalance(creator.publicKey);
    const collabBalAfter = await provider.connection.getBalance(collaborator.publicKey);

    expect(creatorBalAfter - creatorBalBefore).to.equal(0.7 * LAMPORTS_PER_SOL);
    expect(collabBalAfter - collabBalBefore).to.equal(0.3 * LAMPORTS_PER_SOL);
  });

  it("5. Rejects Malicious/Mismatched Split Remainder Exploits", async () => {
    const maliciousAttacker = Keypair.generate();
    const tipAmount = new anchor.BN(0.5 * LAMPORTS_PER_SOL);

    try {
      await program.methods
        .resolveAndTip(tipAmount)
        .accounts({
          nameRecord: nameRecordPda,
          creatorProfile: profilePda,
          owner: creator.publicKey,
          tipper: fanTipper.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .remainingAccounts([
          { pubkey: maliciousAttacker.publicKey, isWritable: true, isSigner: false }, // Violates recipient key
          { pubkey: collaborator.publicKey, isWritable: true, isSigner: false },
        ])
        .signers([fanTipper])
        .rpc();
      expect.fail("Transaction should have failed with SplitRecipientMismatch");
    } catch (err: any) {
      expect(err.error.errorCode.code).to.equal("SplitRecipientMismatch");
    }
  });
});