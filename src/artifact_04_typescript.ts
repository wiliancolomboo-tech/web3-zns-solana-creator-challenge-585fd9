import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { ZnsCreator } from "../target/types/zns_creator";
import { PublicKey, SystemProgram, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { expect } from "chai";
import { createHash } from "crypto";

describe("zns-creator protocol tests", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.ZnsCreator as Program<ZnsCreator>;

  const admin = anchor.web3.Keypair.generate();
  const treasury = anchor.web3.Keypair.generate();
  const creator = anchor.web3.Keypair.generate();
  const fanUser = anchor.web3.Keypair.generate();
  const intruder = anchor.web3.Keypair.generate();

  const sha256 = (str: string): Buffer => {
    return createHash("sha256").update(str).digest();
  };

  const [globalRegistryPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("global_registry")],
    program.programId
  );

  const handle = "zenith";
  const handleHash = sha256(handle);
  const [creatorProfilePda] = PublicKey.findProgramAddressSync(
    [Buffer.from("creator"), handleHash],
    program.programId
  );

  const subdomain = "alphafan";
  const subdomainHash = sha256(subdomain);
  const [subdomainRecordPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("subdomain"), creatorProfilePda.toBuffer(), subdomainHash],
    program.programId
  );

  before(async () => {
    // Fund test wallets
    const airdropSigners = [admin, creator, fanUser, intruder];
    for (const kp of airdropSigners) {
      const sig = await provider.connection.requestAirdrop(
        kp.publicKey,
        5 * LAMPORTS_PER_SOL
      );
      await provider.connection.confirmTransaction(sig);
    }
  });

  it("1. Initializes Global Registry with valid protocol fee", async () => {
    const feeBps = 500; // 5% fee
    await program.methods
      .initializeRegistry(feeBps)
      .accounts({
        globalRegistry: globalRegistryPda,
        admin: admin.publicKey,
        treasury: treasury.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([admin])
      .rpc();

    const registry = await program.account.globalRegistry.fetch(globalRegistryPda);
    expect(registry.admin.toBase58()).to.equal(admin.publicKey.toBase58());
    expect(registry.treasury.toBase58()).to.equal(treasury.publicKey.toBase58());
    expect(registry.protocolFeeBps).to.equal(500);
    expect(registry.totalCreators.toNumber()).to.equal(0);
  });

  it("2. Registers root creator handle with dynamic fee assessment", async () => {
    const initialTreasuryBalance = await provider.connection.getBalance(treasury.publicKey);
    const subdomainFee = new anchor.BN(0.05 * LAMPORTS_PER_SOL);
    const royaltyBps = 1000; // 10%
    const uri = "https://zns.link/creator/zenith.json";

    await program.methods
      .registerCreatorHandle(handle, uri, subdomainFee, royaltyBps)
      .accounts({
        globalRegistry: globalRegistryPda,
        creatorProfile: creatorProfilePda,
        creator: creator.publicKey,
        treasury: treasury.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([creator])
      .rpc();

    const profile = await program.account.creatorProfile.fetch(creatorProfilePda);
    expect(profile.authority.toBase58()).to.equal(creator.publicKey.toBase58());
    expect(profile.subdomainBaseFee.toString()).to.equal(subdomainFee.toString());
    expect(profile.totalSubdomains).to.equal(0);

    const postTreasuryBalance = await provider.connection.getBalance(treasury.publicKey);
    // "zenith" is 6 letters -> 100_000_000 lamports (0.1 SOL)
    expect(postTreasuryBalance - initialTreasuryBalance).to.equal(100_000_000);
  });

  it("3. Issues monetized fan subdomain with automated revenue split", async () => {
    const creatorBalBefore = await provider.connection.getBalance(creator.publicKey);
    const treasuryBalBefore = await provider.connection.getBalance(treasury.publicKey);

    const tier = 2; // Silver tier
    const duration = new anchor.BN(100); // 100 epochs

    await program.methods
      .issueFanSubdomain(subdomain, tier, duration)
      .accounts({
        globalRegistry: globalRegistryPda,
        creatorProfile: creatorProfilePda,
        subdomainRecord: subdomainRecordPda,
        fan: fanUser.publicKey,
        creatorAuthority: creator.publicKey,
        treasury: treasury.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([fanUser])
      .rpc();

    const subRecord = await program.account.subdomainRecord.fetch(subdomainRecordPda);
    expect(subRecord.owner.toBase58()).to.equal(fanUser.publicKey.toBase58());
    expect(subRecord.tier).to.equal(tier);

    // Subdomain fee is 0.05 SOL = 50,000,000 lamports
    // Protocol fee = 5% (2,500,000)
    // Creator net cut = 95% (47,500,000)
    const creatorBalAfter = await provider.connection.getBalance(creator.publicKey);
    const treasuryBalAfter = await provider.connection.getBalance(treasury.publicKey);

    expect(creatorBalAfter - creatorBalBefore).to.equal(47_500_000);
    expect(treasuryBalAfter - treasuryBalBefore).to.equal(2_500_000);
  });

  it("4. Dispatches zero-hop atomic tip to creator", async () => {
    const tipAmount = new anchor.BN(1 * LAMPORTS_PER_SOL);
    const creatorBalBefore = await provider.connection.getBalance(creator.publicKey);
    const treasuryBalBefore = await provider.connection.getBalance(treasury.publicKey);

    await program.methods
      .sendCreatorTip(tipAmount)
      .accounts({
        globalRegistry: globalRegistryPda,
        creatorProfile: creatorProfilePda,
        supporter: fanUser.publicKey,
        creatorAuthority: creator.publicKey,
        treasury: treasury.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([fanUser])
      .rpc();

    const creatorBalAfter = await provider.connection.getBalance(creator.publicKey);
    const treasuryBalAfter = await provider.connection.getBalance(treasury.publicKey);

    // 5% protocol cut = 0.05 SOL, 95% creator cut = 0.95 SOL
    expect(creatorBalAfter - creatorBalBefore).to.equal(0.95 * LAMPORTS_PER_SOL);
    expect(treasuryBalAfter - treasuryBalBefore).to.equal(0.05 * LAMPORTS_PER_SOL);
  });

  it("5. Fails when unauthorized user attempts to update creator profile", async () => {
    let failed = false;
    try {
      await program.methods
        .updateCreatorMetadata("https://malicious.org", new anchor.BN(0), 0)
        .accounts({
          creatorProfile: creatorProfilePda,
          authority: intruder.publicKey,
        })
        .signers([intruder])
        .rpc();
    } catch (err: any) {
      failed = true;
      expect(err.toString()).to.include("InvalidCreatorAuthority");
    }
    expect(failed).to.be.true;
  });
});