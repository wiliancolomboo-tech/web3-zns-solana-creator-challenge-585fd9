import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { ZnsSolana } from "../target/types/zns_solana";
import { PublicKey, SystemProgram, Keypair, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { expect } from "chai";

describe("🟢 ZNS Solana Creator Protocol Matrix", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.ZnsSolana as Program<ZnsSolana>;

  const authority = Keypair.generate();
  const treasury = Keypair.generate();
  const creatorAlice = Keypair.generate();
  const fanBob = Keypair.generate();

  const TLD = "creator";
  const DOMAIN_NAME = "alice";
  const SUBDOMAIN_NAME = "vip";
  const ANNUAL_FEE = new anchor.BN(0.05 * LAMPORTS_PER_SOL);

  let tldPda: PublicKey;
  let domainPda: PublicKey;
  let reversePda: PublicKey;
  let subdomainPda: PublicKey;

  before(async () => {
    // Fund all participant wallets with devnet SOL
    for (const kp of [authority, creatorAlice, fanBob]) {
      const sig = await provider.connection.requestAirdrop(kp.publicKey, 10 * LAMPORTS_PER_SOL);
      const latestBlockhash = await provider.connection.getLatestBlockhash();
      await provider.connection.confirmTransaction({
        signature: sig,
        ...latestBlockhash,
      });
    }

    [tldPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("zns_tld"), Buffer.from(TLD)],
      program.programId
    );

    [domainPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("zns_domain"), tldPda.toBuffer(), Buffer.from(DOMAIN_NAME)],
      program.programId
    );

    [reversePda] = PublicKey.findProgramAddressSync(
      [Buffer.from("zns_reverse"), creatorAlice.publicKey.toBuffer()],
      program.programId
    );

    [subdomainPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("zns_subdomain"), domainPda.toBuffer(), Buffer.from(SUBDOMAIN_NAME)],
      program.programId
    );
  });

  it("1. Initializes TLD '.creator'", async () => {
    const tx = await program.methods
      .initializeTld(TLD, ANNUAL_FEE)
      .accountsStrict({
        tldConfig: tldPda,
        authority: authority.publicKey,
        treasury: treasury.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([authority])
      .rpc();

    expect(tx).to.be.a("string");

    const tldAccount = await program.account.tldConfig.fetch(tldPda);
    expect(tldAccount.tld).to.equal(TLD);
    expect(tldAccount.registrationFee.toString()).to.equal(ANNUAL_FEE.toString());
    expect(tldAccount.authority.toBase58()).to.equal(authority.publicKey.toBase58());
  });

  it("2. Registers Creator Domain 'alice.creator' with Reverse Lookup Record", async () => {
    const durationYears = 2;
    const initialTreasuryBalance = await provider.connection.getBalance(treasury.publicKey);

    await program.methods
      .registerDomain(DOMAIN_NAME, durationYears)
      .accountsStrict({
        tldConfig: tldPda,
        domainRecord: domainPda,
        reverseRecord: reversePda,
        payer: creatorAlice.publicKey,
        treasury: treasury.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([creatorAlice])
      .rpc();

    const domainAccount = await program.account.domainRecord.fetch(domainPda);
    expect(domainAccount.name).to.equal(DOMAIN_NAME);
    expect(domainAccount.owner.toBase58()).to.equal(creatorAlice.publicKey.toBase58());

    const reverseAccount = await program.account.reverseRecord.fetch(reversePda);
    expect(reverseAccount.fullName).to.equal("alice.creator");
    expect(reverseAccount.owner.toBase58()).to.equal(creatorAlice.publicKey.toBase58());

    const finalTreasuryBalance = await provider.connection.getBalance(treasury.publicKey);
    expect(finalTreasuryBalance - initialTreasuryBalance).to.equal(
      ANNUAL_FEE.toNumber() * durationYears
    );
  });

  it("3. Configures Creator Domain Records (Social Handle & IPFS CID)", async () => {
    const dummyHash = new Array(32).fill(7);
    const twitter = "alice_sol";
    const subdomainPrice = new anchor.BN(0.1 * LAMPORTS_PER_SOL);

    await program.methods
      .setRecords(true, subdomainPrice, dummyHash, twitter)
      .accountsStrict({
        domainRecord: domainPda,
        owner: creatorAlice.publicKey,
      })
      .signers([creatorAlice])
      .rpc();

    const updated = await program.account.domainRecord.fetch(domainPda);
    expect(updated.twitterHandle).to.equal(twitter);
    expect(updated.subdomainFee.toString()).to.equal(subdomainPrice.toString());
    expect(Buffer.from(updated.contentHash).toString("hex")).to.equal(
      Buffer.from(dummyHash).toString("hex")
    );
  });

  it("4. Mints Fan Subdomain 'vip.alice.creator' with Automated 90/10 Split", async () => {
    const initialCreatorBalance = await provider.connection.getBalance(creatorAlice.publicKey);
    const initialTreasuryBalance = await provider.connection.getBalance(treasury.publicKey);

    await program.methods
      .registerSubdomain(SUBDOMAIN_NAME)
      .accountsStrict({
        tldConfig: tldPda,
        parentDomain: domainPda,
        subdomainRecord: subdomainPda,
        payer: fanBob.publicKey,
        parentOwner: creatorAlice.publicKey,
        treasury: treasury.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([fanBob])
      .rpc();

    const sub = await program.account.subdomainRecord.fetch(subdomainPda);
    expect(sub.subName).to.equal(SUBDOMAIN_NAME);
    expect(sub.owner.toBase58()).to.equal(fanBob.publicKey.toBase58());

    const finalCreatorBalance = await provider.connection.getBalance(creatorAlice.publicKey);
    const finalTreasuryBalance = await provider.connection.getBalance(treasury.publicKey);

    // 0.1 SOL subdomain fee: 90% (0.09 SOL) to Alice, 10% (0.01 SOL) to Treasury
    expect(finalCreatorBalance - initialCreatorBalance).to.equal(0.09 * LAMPORTS_PER_SOL);
    expect(finalTreasuryBalance - initialTreasuryBalance).to.equal(0.01 * LAMPORTS_PER_SOL);
  });

  it("5. Directly Tips Creator via 'alice.creator' Handle", async () => {
    const tipAmount = new anchor.BN(0.5 * LAMPORTS_PER_SOL);
    const initialCreatorBalance = await provider.connection.getBalance(creatorAlice.publicKey);

    await program.methods
      .tipCreator(tipAmount)
      .accountsStrict({
        domainRecord: domainPda,
        tipper: fanBob.publicKey,
        creatorOwner: creatorAlice.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([fanBob])
      .rpc();

    const finalCreatorBalance = await provider.connection.getBalance(creatorAlice.publicKey);
    expect(finalCreatorBalance - initialCreatorBalance).to.equal(tipAmount.toNumber());

    const domain = await program.account.domainRecord.fetch(domainPda);
    expect(domain.tipCount.toNumber()).to.equal(1);
    expect(domain.totalTipsLamports.toString()).to.equal(tipAmount.toString());
  });

  it("6. Rejects Invalid Domain Inputs (Uppercase & Illegal Characters)", async () => {
    try {
      const invalidName = "Alice_Cool!";
      const [badPda] = PublicKey.findProgramAddressSync(
        [Buffer.from("zns_domain"), tldPda.toBuffer(), Buffer.from(invalidName)],
        program.programId
      );

      await program.methods
        .registerDomain(invalidName, 1)
        .accountsStrict({
          tldConfig: tldPda,
          domainRecord: badPda,
          reverseRecord: reversePda,
          payer: creatorAlice.publicKey,
          treasury: treasury.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([creatorAlice])
        .rpc();

      expect.fail("Should have failed validation");
    } catch (err: any) {
      expect(err.error.errorCode.code).to.equal("InvalidCharacters");
    }
  });
});