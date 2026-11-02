import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { ZnsCreator } from "../target/types/zns_creator";
import { PublicKey, Keypair, SystemProgram, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { expect } from "chai";
import * as crypto from "crypto";

describe("🟢 ZNS Solana Creator Challenge Full QA Matrix", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.ZnsCreator as Program<ZnsCreator>;

  const admin = provider.wallet;
  const creator = Keypair.generate();
  const fan = Keypair.generate();
  const communityMember = Keypair.generate();

  let globalConfigPda: PublicKey;
  let vaultPda: PublicKey;

  const sha256 = (val: string): Buffer => {
    return crypto.createHash("sha256").update(val).digest();
  };

  before(async () => {
    // Fund test wallets
    const airdrop1 = await provider.connection.requestAirdrop(creator.publicKey, 10 * LAMPORTS_PER_SOL);
    const airdrop2 = await provider.connection.requestAirdrop(fan.publicKey, 10 * LAMPORTS_PER_SOL);
    await provider.connection.confirmTransaction(airdrop1);
    await provider.connection.confirmTransaction(airdrop2);

    [globalConfigPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("global_config")],
      program.programId
    );

    [vaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault")],
      program.programId
    );
  });

  it("1. Initializes the Registry Global Config with 5% (500 bps) Protocol Fee", async () => {
    const protocolFeeBps = 500; // 5%

    await program.methods
      .initializeRegistry(protocolFeeBps)
      .accounts({
        config: globalConfigPda,
        protocolVault: vaultPda,
        admin: admin.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const configAccount = await program.account.globalConfig.fetch(globalConfigPda);
    expect(configAccount.admin.toBase58()).to.equal(admin.publicKey.toBase58());
    expect(configAccount.protocolFeeBps).to.equal(protocolFeeBps);
    expect(configAccount.totalDomains.toNumber()).to.equal(0);
  });

  it("2. Registers a Root Creator Domain ('solanadevs')", async () => {
    const domainName = "solanadevs";
    const nameHash = sha256(domainName);

    const [rootDomainPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("root_domain"), nameHash],
      program.programId
    );

    const initialVaultBalance = await provider.connection.getBalance(vaultPda);

    await program.methods
      .registerRootDomain(domainName, "https://arweave.net/metadata_hash_solana", 250)
      .accounts({
        config: globalConfigPda,
        rootDomain: rootDomainPda,
        protocolVault: vaultPda,
        creator: creator.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([creator])
      .rpc();

    const rootAccount = await program.account.rootDomain.fetch(rootDomainPda);
    expect(rootAccount.owner.toBase58()).to.equal(creator.publicKey.toBase58());
    expect(rootAccount.name).to.equal(domainName);
    expect(rootAccount.subDomainCount).to.equal(0);

    // Assert base fee of 0.05 SOL reached vault
    const finalVaultBalance = await provider.connection.getBalance(vaultPda);
    expect(finalVaultBalance - initialVaultBalance).to.equal(50_000_000);
  });

  it("3. Delegates a Tier-1 Subdomain ('alpha.solanadevs')", async () => {
    const domainName = "solanadevs";
    const subName = "alpha";
    const nameHash = sha256(domainName);
    const subHash = sha256(subName);

    const [rootDomainPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("root_domain"), nameHash],
      program.programId
    );

    const [subDomainPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("subdomain"), rootDomainPda.toBuffer(), subHash],
      program.programId
    );

    const oneYearSeconds = new anchor.BN(365 * 24 * 60 * 60);

    await program.methods
      .delegateSubdomain(subName, oneYearSeconds, 1)
      .accounts({
        rootDomain: rootDomainPda,
        subDomain: subDomainPda,
        delegateTo: communityMember.publicKey,
        owner: creator.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([creator])
      .rpc();

    const subAccount = await program.account.subDomain.fetch(subDomainPda);
    expect(subAccount.owner.toBase58()).to.equal(communityMember.publicKey.toBase58());
    expect(subAccount.subName).to.equal(subName);
    expect(subAccount.tier).to.equal(1);
  });

  it("4. Sets and updates Dynamic Resolver Records (IPFS Hash)", async () => {
    const domainName = "solanadevs";
    const recordKey = "ipfs_avatar";
    const recordVal = Buffer.from("QmXoypizjW3WknFiJnKLwHCnL72vedxjQkDDP1mXWo6uco");
    const nameHash = sha256(domainName);
    const keyHash = sha256(recordKey);

    const [rootDomainPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("root_domain"), nameHash],
      program.programId
    );

    const [resolverPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("resolver"), rootDomainPda.toBuffer(), keyHash],
      program.programId
    );

    await program.methods
      .setResolverRecord(recordKey, recordVal)
      .accounts({
        rootDomain: rootDomainPda,
        resolverRecord: resolverPda,
        owner: creator.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([creator])
      .rpc();

    const recordAccount = await program.account.resolverRecord.fetch(resolverPda);
    expect(recordAccount.recordKey).to.equal(recordKey);
    expect(Buffer.from(recordAccount.recordValue).toString()).to.equal(recordVal.toString());
  });

  it("5. Routes atomic tips with a 95/5 Creator/Protocol split", async () => {
    const domainName = "solanadevs";
    const nameHash = sha256(domainName);

    const [rootDomainPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("root_domain"), nameHash],
      program.programId
    );

    const tipAmount = new anchor.BN(1 * LAMPORTS_PER_SOL);
    const creatorBalBefore = await provider.connection.getBalance(creator.publicKey);
    const vaultBalBefore = await provider.connection.getBalance(vaultPda);

    await program.methods
      .tipCreatorWithSplit(tipAmount)
      .accounts({
        config: globalConfigPda,
        rootDomain: rootDomainPda,
        protocolVault: vaultPda,
        owner: creator.publicKey,
        tipper: fan.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([fan])
      .rpc();

    const creatorBalAfter = await provider.connection.getBalance(creator.publicKey);
    const vaultBalAfter = await provider.connection.getBalance(vaultPda);

    // 5% of 1 SOL = 0.05 SOL; 95% of 1 SOL = 0.95 SOL
    expect(creatorBalAfter - creatorBalBefore).to.equal(0.95 * LAMPORTS_PER_SOL);
    expect(vaultBalAfter - vaultBalBefore).to.equal(0.05 * LAMPORTS_PER_SOL);
  });

  it("6. Rejects Subdomain delegation when called by an Impostor", async () => {
    const impostor = Keypair.generate();
    const airdrop = await provider.connection.requestAirdrop(impostor.publicKey, LAMPORTS_PER_SOL);
    await provider.connection.confirmTransaction(airdrop);

    const domainName = "solanadevs";
    const subName = "fraud";
    const nameHash = sha256(domainName);
    const subHash = sha256(subName);

    const [rootDomainPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("root_domain"), nameHash],
      program.programId
    );

    const [subDomainPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("subdomain"), rootDomainPda.toBuffer(), subHash],
      program.programId
    );

    try {
      await program.methods
        .delegateSubdomain(subName, new anchor.BN(3600), 1)
        .accounts({
          rootDomain: rootDomainPda,
          subDomain: subDomainPda,
          delegateTo: impostor.publicKey,
          owner: impostor.publicKey, // Invalid signer
          systemProgram: SystemProgram.programId,
        })
        .signers([impostor])
        .rpc();
      expect.fail("Transaction should have failed with unauthorized error");
    } catch (err: any) {
      expect(err.error.errorCode.code).to.equal("Unauthorized");
    }
  });
});