import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { ZnsCreatorCore } from "../target/types/zns_creator_core";
import { PublicKey, Keypair, SystemProgram, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { expect } from "chai";
import crypto from "crypto";

describe("zns-creator-core: Integration QA Suite", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.ZnsCreatorCore as Program<ZnsCreatorCore>;

  const adminKeypair = Keypair.generate();
  const creatorKeypair = Keypair.generate();
  const subscriberKeypair = Keypair.generate();
  const unauthorizedKeypair = Keypair.generate();

  const domainName = "cryptoking";
  const metadataUri = "arweave.net/V8qj1234abcd5678efgh9012ijklmnop";
  const subdomainPass = "alpha";

  let configPda: PublicKey;
  let treasuryVaultPda: PublicKey;
  let domainRecordPda: PublicKey;
  let subdomainRecordPda: PublicKey;
  let reverseRecordPda: PublicKey;

  const registrationFee = new anchor.BN(0.05 * LAMPORTS_PER_SOL);
  const protocolFeeBps = 500; // 5.00%
  const mintPrice = new anchor.BN(0.2 * LAMPORTS_PER_SOL);

  before(async () => {
    // Fund test actors
    const airdropSigners = [adminKeypair, creatorKeypair, subscriberKeypair, unauthorizedKeypair];
    for (const signer of airdropSigners) {
      const sig = await provider.connection.requestAirdrop(signer.publicKey, 5 * LAMPORTS_PER_SOL);
      const latestBlock = await provider.connection.getLatestBlockhash();
      await provider.connection.confirmTransaction({
        signature: sig,
        blockhash: latestBlock.blockhash,
        lastValidBlockHeight: latestBlock.lastValidBlockHeight,
      });
    }

    [configPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("config")],
      program.programId
    );

    [treasuryVaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("treasury")],
      program.programId
    );

    const domainHash = crypto.createHash("sha256").update(domainName).digest();
    [domainRecordPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("domain"), domainHash],
      program.programId
    );

    const subHash = crypto.createHash("sha256").update(subdomainPass).digest();
    [subdomainRecordPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("subdomain"), domainRecordPda.toBuffer(), subHash],
      program.programId
    );

    [reverseRecordPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("reverse"), creatorKeypair.publicKey.toBuffer()],
      program.programId
    );
  });

  it("1. Initializes protocol global configuration and treasury PDA", async () => {
    await program.methods
      .initializeProtocol(registrationFee, protocolFeeBps)
      .accounts({
        config: configPda,
        treasuryVault: treasuryVaultPda,
        admin: adminKeypair.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([adminKeypair])
      .rpc();

    const configAccount = await program.account.protocolConfig.fetch(configPda);
    expect(configAccount.admin.toBase58()).to.equal(adminKeypair.publicKey.toBase58());
    expect(configAccount.registrationFee.eq(registrationFee)).to.be.true;
    expect(configAccount.protocolFeeBps).to.equal(500);
    expect(configAccount.totalDomains.toNumber()).to.equal(0);
  });

  it("2. Registers a creator top-level domain with automated protocol fee transfer", async () => {
    const treasuryBalanceBefore = await provider.connection.getBalance(treasuryVaultPda);

    await program.methods
      .registerCreatorDomain(domainName, metadataUri)
      .accounts({
        config: configPda,
        domainRecord: domainRecordPda,
        treasuryVault: treasuryVaultPda,
        creator: creatorKeypair.publicKey,
        payer: creatorKeypair.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([creatorKeypair])
      .rpc();

    const treasuryBalanceAfter = await provider.connection.getBalance(treasuryVaultPda);
    expect(treasuryBalanceAfter - treasuryBalanceBefore).to.equal(registrationFee.toNumber());

    const domainAccount = await program.account.domainRecord.fetch(domainRecordPda);
    expect(domainAccount.owner.toBase58()).to.equal(creatorKeypair.publicKey.toBase58());
    expect(domainAccount.resolver.toBase58()).to.equal(creatorKeypair.publicKey.toBase58());
    
    // Convert fixed buffer back to string
    const decodedName = Buffer.from(domainAccount.name)
      .subarray(0, domainAccount.nameLen)
      .toString("utf-8");
    expect(decodedName).to.equal(domainName);
  });

  it("3. Updates creator metadata URI in-place", async () => {
    const newUri = "arweave.net/UPDATED_NEW_HASH_1234567890";

    await program.methods
      .updateMetadataRecord(null, newUri)
      .accounts({
        domainRecord: domainRecordPda,
        owner: creatorKeypair.publicKey,
      })
      .signers([creatorKeypair])
      .rpc();

    const domainAccount = await program.account.domainRecord.fetch(domainRecordPda);
    const decodedUri = Buffer.from(domainAccount.metadataUri)
      .subarray(0, domainAccount.uriLen)
      .toString("utf-8");
    expect(decodedUri).to.equal(newUri);
  });

  it("4. Configures reverse address lookup pointer (Creator Address -> Domain PDA)", async () => {
    await program.methods
      .setReverseLookup()
      .accounts({
        reverseRecord: reverseRecordPda,
        domainRecord: domainRecordPda,
        authority: creatorKeypair.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([creatorKeypair])
      .rpc();

    const reverseAccount = await program.account.reverseRecord.fetch(reverseRecordPda);
    expect(reverseAccount.wallet.toBase58()).to.equal(creatorKeypair.publicKey.toBase58());
    expect(reverseAccount.domain.toBase58()).to.equal(domainRecordPda.toBase58());
  });

  it("5. Issues a time-decaying subdomain pass with 95/5 creator fee split", async () => {
    const durationSeconds = new anchor.BN(30 * 86400); // 30 Days

    const creatorBalanceBefore = await provider.connection.getBalance(creatorKeypair.publicKey);
    const treasuryBalanceBefore = await provider.connection.getBalance(treasuryVaultPda);

    await program.methods
      .issueSubdomainPass(subdomainPass, durationSeconds, mintPrice)
      .accounts({
        config: configPda,
        parentDomain: domainRecordPda,
        subdomainRecord: subdomainRecordPda,
        creator: creatorKeypair.publicKey,
        recipient: subscriberKeypair.publicKey,
        treasuryVault: treasuryVaultPda,
        owner: creatorKeypair.publicKey,
        payer: subscriberKeypair.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([creatorKeypair, subscriberKeypair])
      .rpc();

    const creatorBalanceAfter = await provider.connection.getBalance(creatorKeypair.publicKey);
    const treasuryBalanceAfter = await provider.connection.getBalance(treasuryVaultPda);

    // Expected cuts
    const expectedProtocolCut = (mintPrice.toNumber() * protocolFeeBps) / 10000; // 5%
    const expectedCreatorCut = mintPrice.toNumber() - expectedProtocolCut; // 95%

    expect(treasuryBalanceAfter - treasuryBalanceBefore).to.equal(expectedProtocolCut);
    expect(creatorBalanceAfter - creatorBalanceBefore).to.equal(expectedCreatorCut);

    const subRecord = await program.account.subDomainRecord.fetch(subdomainRecordPda);
    expect(subRecord.holder.toBase58()).to.equal(subscriberKeypair.publicKey.toBase58());
    expect(subRecord.expiresAt.toNumber()).to.be.greaterThan(subRecord.issuedAt.toNumber());
  });

  it("6. Prevents unauthorized actors from updating domain metadata (Security Invariant)", async () => {
    const maliciousUri = "ipfs://malicious-exploit-payload";

    try {
      await program.methods
        .updateMetadataRecord(null, maliciousUri)
        .accounts({
          domainRecord: domainRecordPda,
          owner: unauthorizedKeypair.publicKey,
        })
        .signers([unauthorizedKeypair])
        .rpc();
      expect.fail("Exploit succeeded: unauthorized user modified metadata");
    } catch (err: any) {
      expect(err.toString()).to.include("Unauthorized");
    }
  });
});