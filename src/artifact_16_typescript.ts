import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { ZnsSolana } from "../target/types/zns_solana";
import { PublicKey, Keypair, SystemProgram, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { createHash } from "crypto";
import { expect } from "chai";

describe("🟢 ZNS Solana Creator Protocol E2E Test Matrix", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.ZnsSolana as Program<ZnsSolana>;

  const admin = Keypair.generate();
  const creator = Keypair.generate();
  const fan = Keypair.generate();
  const attacker = Keypair.generate();
  const treasury = Keypair.generate();

  const BASE_FEE = new anchor.BN(0.05 * LAMPORTS_PER_SOL); // 0.05 SOL/year

  let configPda: PublicKey;
  let vaultPda: PublicKey;

  function sha256(val: string): Buffer {
    return createHash("sha256").update(val).digest();
  }

  function getDomainPda(domainName: string): PublicKey {
    const hash = sha256(domainName.toLowerCase());
    const [pda] = PublicKey.findProgramAddressSync(
      [Buffer.from("domain"), hash],
      program.programId
    );
    return pda;
  }

  function getSubdomainPda(domainKey: PublicKey, subName: string): PublicKey {
    const hash = sha256(subName.toLowerCase());
    const [pda] = PublicKey.findProgramAddressSync(
      [Buffer.from("subdomain"), domainKey.toBuffer(), hash],
      program.programId
    );
    return pda;
  }

  function getRecordPda(domainKey: PublicKey, recordKey: string): PublicKey {
    const hash = sha256(recordKey.toLowerCase());
    const [pda] = PublicKey.findProgramAddressSync(
      [Buffer.from("record"), domainKey.toBuffer(), hash],
      program.programId
    );
    return pda;
  }

  function getReversePda(owner: PublicKey): PublicKey {
    const [pda] = PublicKey.findProgramAddressSync(
      [Buffer.from("reverse"), owner.toBuffer()],
      program.programId
    );
    return pda;
  }

  before(async () => {
    // Fund test wallets
    const airdropSigners = [admin, creator, fan, attacker];
    for (const s of airdropSigners) {
      const sig = await provider.connection.requestAirdrop(s.publicKey, 10 * LAMPORTS_PER_SOL);
      const latestBlockhash = await provider.connection.getLatestBlockhash();
      await provider.connection.confirmTransaction({
        signature: sig,
        blockhash: latestBlockhash.blockhash,
        lastValidBlockHeight: latestBlockhash.lastValidBlockHeight,
      });
    }

    [configPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("zns_config")],
      program.programId
    );

    [vaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), configPda.toBuffer()],
      program.programId
    );
  });

  it("1. Initializes global protocol configuration with vault PDA", async () => {
    await program.methods
      .initialize(BASE_FEE)
      .accountsStrict({
        admin: admin.publicKey,
        config: configPda,
        vault: vaultPda,
        treasury: treasury.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([admin])
      .rpc();

    const configAccount = await program.account.config.fetch(configPda);
    expect(configAccount.admin.toBase58()).to.equal(admin.publicKey.toBase58());
    expect(configAccount.treasury.toBase58()).to.equal(treasury.publicKey.toBase58());
    expect(configAccount.baseFeePerYear.toString()).to.equal(BASE_FEE.toString());
    expect(configAccount.isPaused).to.be.false;
  });

  it("2. Registers root domain for creator with multi-year duration", async () => {
    const domainName = "monke-creator";
    const domainPda = getDomainPda(domainName);
    const years = 2;

    const vaultBalanceBefore = await provider.connection.getBalance(vaultPda);

    await program.methods
      .registerDomain(domainName, years)
      .accountsStrict({
        payer: creator.publicKey,
        owner: creator.publicKey,
        config: configPda,
        domainRecord: domainPda,
        vault: vaultPda,
        systemProgram: SystemProgram.programId,
      })
      .signers([creator])
      .rpc();

    const domainAccount = await program.account.domainRecord.fetch(domainPda);
    expect(domainAccount.owner.toBase58()).to.equal(creator.publicKey.toBase58());
    expect(domainAccount.name).to.equal(domainName);
    expect(domainAccount.subdomainsCount).to.equal(0);

    const vaultBalanceAfter = await provider.connection.getBalance(vaultPda);
    expect(vaultBalanceAfter - vaultBalanceBefore).to.equal(BASE_FEE.toNumber() * years);
  });

  it("3. Attaches dynamic social, tip jar, and IPFS records to domain", async () => {
    const domainName = "monke-creator";
    const domainPda = getDomainPda(domainName);

    const records = [
      { key: "tip_jar", value: "So11111111111111111111111111111111111111112" },
      { key: "nostr", value: "npub1qny3tkh0vd..." },
      { key: "avatar_ipfs", value: "ipfs://bafybeicv5t3e..." },
      { key: "twitter", value: "@monke_sol" },
    ];

    for (const rec of records) {
      const recordPda = getRecordPda(domainPda, rec.key);
      await program.methods
        .setRecord(rec.key, rec.value)
        .accountsStrict({
          authority: creator.publicKey,
          domainRecord: domainPda,
          dynamicRecord: recordPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([creator])
        .rpc();

      const fetched = await program.account.dynamicRecord.fetch(recordPda);
      expect(fetched.key).to.equal(rec.key);
      expect(fetched.value).to.equal(rec.value);
    }
  });

  it("4. Configures primary reverse resolution for creator", async () => {
    const domainName = "monke-creator";
    const domainPda = getDomainPda(domainName);
    const reversePda = getReversePda(creator.publicKey);

    await program.methods
      .setPrimaryDomain()
      .accountsStrict({
        authority: creator.publicKey,
        domainRecord: domainPda,
        reverseRecord: reversePda,
        systemProgram: SystemProgram.programId,
      })
      .signers([creator])
      .rpc();

    const reverseAccount = await program.account.reverseRecord.fetch(reversePda);
    expect(reverseAccount.owner.toBase58()).to.equal(creator.publicKey.toBase58());
    expect(reverseAccount.domain.toBase58()).to.equal(domainPda.toBase58());
    expect(reverseAccount.domainName).to.equal(domainName);
  });

  it("5. Mints hierarchical subdomain for fan/community member", async () => {
    const parentName = "monke-creator";
    const parentPda = getDomainPda(parentName);
    const subName = "fan01";
    const subPda = getSubdomainPda(parentPda, subName);

    await program.methods
      .createSubdomain(subName)
      .accountsStrict({
        authority: creator.publicKey,
        targetOwner: fan.publicKey,
        domainRecord: parentPda,
        subdomainRecord: subPda,
        systemProgram: SystemProgram.programId,
      })
      .signers([creator])
      .rpc();

    const subAccount = await program.account.subdomainRecord.fetch(subPda);
    expect(subAccount.parentDomain.toBase58()).to.equal(parentPda.toBase58());
    expect(subAccount.owner.toBase58()).to.equal(fan.publicKey.toBase58());
    expect(subAccount.subName).to.equal(subName);

    const parentAccount = await program.account.domainRecord.fetch(parentPda);
    expect(parentAccount.subdomainsCount).to.equal(1);
  });

  it("6. Prevents unauthorized record updates (Attacker simulation)", async () => {
    const domainName = "monke-creator";
    const domainPda = getDomainPda(domainName);
    const recordPda = getRecordPda(domainPda, "tip_jar");

    try {
      await program.methods
        .setRecord("tip_jar", attacker.publicKey.toBase58())
        .accountsStrict({
          authority: attacker.publicKey,
          domainRecord: domainPda,
          dynamicRecord: recordPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([attacker])
        .rpc();
      expect.fail("Attacker should not be able to mutate creator records");
    } catch (err: any) {
      expect(err.error.errorCode.code).to.equal("Unauthorized");
    }
  });

  it("7. Renews domain validity and increments expiration timestamp", async () => {
    const domainName = "monke-creator";
    const domainPda = getDomainPda(domainName);

    const before = await program.account.domainRecord.fetch(domainPda);
    const renewYears = 1;

    await program.methods
      .renewDomain(renewYears)
      .accountsStrict({
        payer: fan.publicKey, // Anyone can pay for renewal
        config: configPda,
        domainRecord: domainPda,
        vault: vaultPda,
        systemProgram: SystemProgram.programId,
      })
      .signers([fan])
      .rpc();

    const after = await program.account.domainRecord.fetch(domainPda);
    const expectedExpiry = before.expiresAt.add(new anchor.BN(31536000));
    expect(after.expiresAt.toString()).to.equal(expectedExpiry.toString());
  });

  it("8. Transfers domain custody cleanly to new owner", async () => {
    const domainName = "monke-creator";
    const domainPda = getDomainPda(domainName);

    await program.methods
      .transferDomain()
      .accountsStrict({
        authority: creator.publicKey,
        newOwner: fan.publicKey,
        domainRecord: domainPda,
      })
      .signers([creator])
      .rpc();

    const transferred = await program.account.domainRecord.fetch(domainPda);
    expect(transferred.owner.toBase58()).to.equal(fan.publicKey.toBase58());
  });
});