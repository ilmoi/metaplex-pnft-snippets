// --------------------------------------- step 1 - create ruleset
// from here: https://github.com/metaplex-foundation/mpl-token-auth-rules/blob/main/README.md

export const findRuleSetPDA = async (payer: PublicKey, name: string) => {
  return await PublicKey.findProgramAddress(
    [Buffer.from(PREFIX), payer.toBuffer(), Buffer.from(name)],
    PROGRAM_ID
  );
};

export const createTokenAuthorizationRules = async (
  connection: Connection,
  payer: Keypair,
  name: string,
  data?: Uint8Array
) => {
  const [ruleSetAddress] = await findRuleSetPDA(payer.publicKey, name);

  // Encode the file using msgpack so the pre-encoded data can be written directly to a Solana program account
  // TODO I couldn't get this to work with the more complex rules like this https://github.com/danenbm/mpl-token-auth-rules-example/tree/royalties-dev
  //  when I copy pasta JSON from rust it fails to serialize it
  //  need metaplex team to provide clear examples in typescript
  let finalData =
    data ??
    encode([
      1,
      name,
      payer.publicKey.toBuffer().toJSON().data,
      {
        "Transfer:Owner": "Pass",
      },
    ]);

  let createIX = createCreateOrUpdateInstruction(
    {
      payer: payer.publicKey,
      ruleSetPda: ruleSetAddress,
      systemProgram: SystemProgram.programId,
    },
    {
      createOrUpdateArgs: { __kind: "V1", serializedRuleSet: finalData },
    },
    PROGRAM_ID
  );

  await buildAndSendTx({ ixs: [createIX], extraSigners: [payer] });

  return ruleSetAddress;
};

// --------------------------------------- step 2 - create NFT and attach ruleset

const _createAndMintPNft = async ({
  owner,
  mint,
  royaltyBps,
  creators,
  collection,
  collectionVerified = true,
  ruleSet = null,
}: {
  owner: Keypair;
  mint: Keypair;
  royaltyBps?: number;
  creators?: CreatorInput[];
  collection?: Keypair;
  collectionVerified?: boolean;
  ruleSet?: PublicKey | null; //<-- pass the ruleset you created above into here, it will store it into the NFT's metadata
}) => {
  // --------------------------------------- create

  // metadata account
  const [metadata] = PublicKey.findProgramAddressSync(
    [Buffer.from("metadata"), PROGRAM_ID.toBuffer(), mint.publicKey.toBuffer()],
    PROGRAM_ID
  );

  // master edition account
  const [masterEdition] = PublicKey.findProgramAddressSync(
    [
      Buffer.from("metadata"),
      PROGRAM_ID.toBuffer(),
      mint.publicKey.toBuffer(),
      Buffer.from("edition"),
    ],
    PROGRAM_ID
  );

  const accounts: CreateInstructionAccounts = {
    metadata,
    masterEdition,
    mint: mint.publicKey,
    authority: owner.publicKey,
    payer: owner.publicKey,
    splTokenProgram: TOKEN_PROGRAM_ID,
    sysvarInstructions: SYSVAR_INSTRUCTIONS_PUBKEY,
    updateAuthority: owner.publicKey,
  };

  const args: CreateInstructionArgs = {
    createArgs: {
      __kind: "V1",
      assetData: {
        updateAuthority: owner.publicKey,
        name: "Whatever",
        symbol: "TSR",
        uri: "https://www.tensor.trade",
        sellerFeeBasisPoints: royaltyBps ?? 0,
        creators:
          creators?.map((c) => {
            return {
              address: c.address,
              share: c.share,
              verified: !!c.authority,
            };
          }) ?? null,
        primarySaleHappened: true,
        isMutable: true,
        tokenStandard: TokenStandard.ProgrammableNonFungible,
        collection: collection
          ? { verified: collectionVerified, key: collection.publicKey }
          : null,
        uses: null,
        collectionDetails: null,
        ruleSet,
      },
      decimals: 0,
      maxSupply: 1,
    },
  };

  const createIx = createCreateInstruction(accounts, args);

  // this test always initializes the mint, we we need to set the
  // account to be writable and a signer
  for (let i = 0; i < createIx.keys.length; i++) {
    if (createIx.keys[i].pubkey.toBase58() === mint.publicKey.toBase58()) {
      createIx.keys[i].isSigner = true;
      createIx.keys[i].isWritable = true;
    }
  }

  // --------------------------------------- mint

  // mint instrution will initialize a ATA account
  const [tokenPda] = PublicKey.findProgramAddressSync(
    [
      owner.publicKey.toBuffer(),
      TOKEN_PROGRAM_ID.toBuffer(),
      mint.publicKey.toBuffer(),
    ],
    ASSOCIATED_TOKEN_PROGRAM_ID
  );

  const [tokenRecord] = findTokenRecordPDA(mint.publicKey, owner.publicKey);

  const mintAcccounts: MintInstructionAccounts = {
    token: tokenPda,
    tokenOwner: owner.publicKey,
    metadata,
    masterEdition,
    tokenRecord,
    mint: mint.publicKey,
    payer: owner.publicKey,
    authority: owner.publicKey,
    sysvarInstructions: SYSVAR_INSTRUCTIONS_PUBKEY,
    splAtaProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
    splTokenProgram: TOKEN_PROGRAM_ID,
    authorizationRules: ruleSet ?? undefined,
    authorizationRulesProgram: AUTH_PROGRAM_ID,
  };

  const payload: Payload = {
    map: new Map(),
  };

  const mintArgs: MintInstructionArgs = {
    mintArgs: {
      __kind: "V1",
      amount: 1,
      authorizationData: {
        payload: payload as any,
      },
    },
  };

  const mintIx = createMintInstruction(mintAcccounts, mintArgs);

  // --------------------------------------- send

  await buildAndSendTx({
    ixs: [createIx, mintIx],
    extraSigners: [owner, mint],
  });

  return {
    tokenAddress: tokenPda,
    metadataAddress: metadata,
    masterEditionAddress: masterEdition,
  };
};

// --------------------------------------- step 3 -
// call your ix normally, and if you did the onchain part correctly it should work both with and withour ruleset

// --------------------------------------- if you're using LUT, quick fn to create one for your tests
export const createCoreTswapLUT = async () => {
  const conn = TEST_PROVIDER.connection;
  //intentionally going for > confirmed, otherwise get "is not a recent slot err"
  const slot = await conn.getSlot("finalized");

  //create
  const [lookupTableInst, lookupTableAddress] =
    AddressLookupTableProgram.createLookupTable({
      authority: TEST_PROVIDER.publicKey,
      payer: TEST_PROVIDER.publicKey,
      recentSlot: slot,
    });

  //see if already created
  let lookupTableAccount = (
    await conn.getAddressLookupTable(lookupTableAddress)
  ).value;
  if (!!lookupTableAccount) {
    return lookupTableAccount;
  }

  const [tswapPda] = findTSwapPDA({});

  //add addresses
  const extendInstruction = AddressLookupTableProgram.extendLookupTable({
    payer: TEST_PROVIDER.publicKey,
    authority: TEST_PROVIDER.publicKey,
    lookupTable: lookupTableAddress,
    addresses: [
      tswapPda,
      TSWAP_FEE_ACC,
      TOKEN_PROGRAM_ID,
      SystemProgram.programId,
      SYSVAR_RENT_PUBKEY,
      ASSOCIATED_TOKEN_PROGRAM_ID,
      AUTH_PROGRAM_ID,
      TOKEN_METADATA_PROGRAM_ID,
      SYSVAR_INSTRUCTIONS_PUBKEY,
    ],
  });
  await buildAndSendTx({ ixs: [lookupTableInst, extendInstruction] });

  //fetch
  lookupTableAccount = (await conn.getAddressLookupTable(lookupTableAddress))
    .value;

  return lookupTableAccount;
};
