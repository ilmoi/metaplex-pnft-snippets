// --------------------------------------- boilerplace accs you'll have to pass in every time

//(!) these are not ALL accounts, just the most common ones that you'll have to include everywhere, you need to fill in the rest
#[derive(Accounts)]
pub struct ProgNftShared<'info> {
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,

    /// CHECK: address below
    #[account(address = mpl_token_metadata::id())]
    pub token_metadata_program: UncheckedAccount<'info>,

    /// CHECK: address below
    #[account(address = anchor_lang::solana_program::sysvar::instructions::ID)]
    pub instructions: UncheckedAccount<'info>,

    /// CHECK: address below
    #[account(address = mpl_token_auth_rules::id())]
    pub authorization_rules_program: UncheckedAccount<'info>,
}

// --------------------------------------- code for sending abstracted into a fn

pub fn sent_pnft<'info>(
    //these 3 can be the same, but not necessarily
    authority: &AccountInfo<'info>,
    owner: &AccountInfo<'info>,
    //(!) payer can't carry data, has to be a normal KP:
    // https://github.com/solana-labs/solana/blob/bda0c606a19ce1cc44b5ab638ff0b993f612e76c/runtime/src/system_instruction_processor.rs#L197
    payer: &AccountInfo<'info>,
    source_ata: &Account<'info, TokenAccount>,
    dest_ata: &Account<'info, TokenAccount>,
    dest_owner: &AccountInfo<'info>,
    nft_mint: &Account<'info, Mint>,
    nft_metadata: &UncheckedAccount<'info>,
    nft_edition: &UncheckedAccount<'info>,
    system_program: &Program<'info, System>,
    token_program: &Program<'info, Token>,
    ata_program: &Program<'info, AssociatedToken>,
    metadata_program: &UncheckedAccount<'info>,
    instructions: &UncheckedAccount<'info>,
    owner_token_record: &UncheckedAccount<'info>,
    dest_token_record: &UncheckedAccount<'info>,
    authorization_rules_program: &UncheckedAccount<'info>,
    rules_acc: Option<&AccountInfo<'info>>,
    authorization_data: Option<AuthorizationDataLocal>,
    //if passed, use signed_invoke() instead of invoke()
    program_signer: Option<&Account<'info, TSwap>>,
) -> Result<()> {
    let mut builder = TransferBuilder::new();
    builder
        .authority(*authority.key)
        .token_owner(*owner.key)
        .token(source_ata.key())
        .destination_owner(*dest_owner.key)
        .destination(dest_ata.key())
        .mint(nft_mint.key())
        .metadata(nft_metadata.key())
        .edition(nft_edition.key())
        .payer(*payer.key);

    let mut account_infos = vec![
        //   0. `[writable]` Token account
        source_ata.to_account_info(),
        //   1. `[]` Token account owner
        owner.to_account_info(),
        //   2. `[writable]` Destination token account
        dest_ata.to_account_info(),
        //   3. `[]` Destination token account owner
        dest_owner.to_account_info(),
        //   4. `[]` Mint of token asset
        nft_mint.to_account_info(),
        //   5. `[writable]` Metadata account
        nft_metadata.to_account_info(),
        //   6. `[optional]` Edition of token asset
        nft_edition.to_account_info(),
        //   7. `[signer] Transfer authority (token or delegate owner)
        authority.to_account_info(),
        //   8. `[optional, writable]` Owner record PDA
        //passed in below, if needed
        //   9. `[optional, writable]` Destination record PDA
        //passed in below, if needed
        //   10. `[signer, writable]` Payer
        payer.to_account_info(),
        //   11. `[]` System Program
        system_program.to_account_info(),
        //   12. `[]` Instructions sysvar account
        instructions.to_account_info(),
        //   13. `[]` SPL Token Program
        token_program.to_account_info(),
        //   14. `[]` SPL Associated Token Account program
        ata_program.to_account_info(),
        //   15. `[optional]` Token Authorization Rules Program
        //passed in below, if needed
        //   16. `[optional]` Token Authorization Rules account
        //passed in below, if needed
    ];

    //grab assert_decode_metadata from mplex repo
    let metadata = assert_decode_metadata(nft_mint, nft_metadata)?;
    if let Some(standard) = metadata.token_standard {
        msg!("standard triggered");
        if standard == TokenStandard::ProgrammableNonFungible {
            //1. add to builder
            builder
                .owner_token_record(owner_token_record.key())
                .destination_token_record(dest_token_record.key());

            //2. add to accounts (if try to pass these for non-pNFT, will get owner errors, since they don't exist)
            account_infos.push(owner_token_record.to_account_info());
            account_infos.push(dest_token_record.to_account_info());
        }
    }

    //if auth rules passed in, validate & include it in CPI call
    if let Some(config) = metadata.programmable_config {
        match config {
            V1 { rule_set } => {
                if let Some(rule_set) = rule_set {
                    msg!("ruleset triggered");
                    //safe to unwrap here, it's expected
                    let rules_acc = rules_acc.unwrap();

                    //1. validate
                    if rule_set != *rules_acc.key {
                        throw_err!(BadRuleSet);
                    }

                    //2. add to builder
                    builder.authorization_rules(*rules_acc.key);

                    //3. add to accounts
                    account_infos.push(authorization_rules_program.to_account_info());
                    account_infos.push(rules_acc.to_account_info());
                }
            }
        }
    }

    let transfer_ix = builder
        .build(TransferArgs::V1 {
            amount: 1, //currently 1 only
            authorization_data: if let Some(authorization_data) = authorization_data {
                Some(AuthorizationData::try_from(authorization_data).unwrap())
            } else {
                None
            },
        })
        .unwrap()
        .instruction();

    if let Some(program_signer) = program_signer {
        invoke_signed(&transfer_ix, &account_infos, &[&program_signer.seeds()])?;
    } else {
        invoke(&transfer_ix, &account_infos)?;
    }

    Ok(())
}

// --------------------------------------- replicating mplex type for anchor IDL export
//have to do this because anchor won't include foreign structs in the IDL

#[derive(AnchorSerialize, AnchorDeserialize, Debug, Clone)]
pub struct AuthorizationDataLocal {
    pub payload: Vec<TaggedPayload>,
}
impl Into<AuthorizationData> for AuthorizationDataLocal {
    fn into(self) -> AuthorizationData {
        let mut p = Payload::new();
        self.payload.into_iter().for_each(|tp| {
            p.insert(tp.name, PayloadType::try_from(tp.payload).unwrap());
        });
        AuthorizationData {
            payload: Payload::try_from(p).unwrap(),
        }
    }
}

//Unfortunately anchor doesn't like HashMaps, nor Tuples, so you can't pass in:
// HashMap<String, PayloadType>, nor
// Vec<(String, PayloadTypeLocal)>
// so have to create this stupid temp struct for IDL to serialize correctly
#[derive(AnchorSerialize, AnchorDeserialize, Debug, Clone)]
pub struct TaggedPayload {
    name: String,
    payload: PayloadTypeLocal,
}

#[derive(AnchorSerialize, AnchorDeserialize, Debug, Clone)]
pub enum PayloadTypeLocal {
    /// A plain `Pubkey`.
    Pubkey(Pubkey),
    /// PDA derivation seeds.
    Seeds(SeedsVecLocal),
    /// A merkle proof.
    MerkleProof(ProofInfoLocal),
    /// A plain `u64` used for `Amount`.
    Number(u64),
}
impl Into<PayloadType> for PayloadTypeLocal {
    fn into(self) -> PayloadType {
        match self {
            Self::Pubkey(pubkey) => PayloadType::Pubkey(pubkey),
            Self::Seeds(seeds) => PayloadType::Seeds(SeedsVec::try_from(seeds).unwrap()),
            Self::MerkleProof(proof) => {
                PayloadType::MerkleProof(ProofInfo::try_from(proof).unwrap())
            }
            Self::Number(number) => PayloadType::Number(number),
        }
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Debug, Clone)]
pub struct SeedsVecLocal {
    /// The vector of derivation seeds.
    pub seeds: Vec<Vec<u8>>,
}
impl Into<SeedsVec> for SeedsVecLocal {
    fn into(self) -> SeedsVec {
        SeedsVec { seeds: self.seeds }
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Debug, Clone)]
pub struct ProofInfoLocal {
    /// The merkle proof.
    pub proof: Vec<[u8; 32]>,
}
impl Into<ProofInfo> for ProofInfoLocal {
    fn into(self) -> ProofInfo {
        ProofInfo { proof: self.proof }
    }
}

// --------------------------------------- bringing everything together - your handler

pub fn handler<'info>(
    ctx: Context<'_, '_, '_, 'info, DepositNft<'info>>,
    authorization_data: Option<AuthorizationDataLocal>,
) -> Result<()> {
    let rem_acc = &mut ctx.remaining_accounts.iter().peekable();
    let auth_rules = if let Some(rules_acc) = rem_acc.peek() {
        Some(*rules_acc)
    } else {
        None
    };
    sent_pnft(
        &ctx.accounts.owner.to_account_info(),
        &ctx.accounts.owner.to_account_info(),
        &ctx.accounts.owner.to_account_info(),
        &ctx.accounts.nft_source,
        &ctx.accounts.nft_escrow,
        &ctx.accounts.tswap.to_account_info(),
        &ctx.accounts.nft_mint,
        &ctx.accounts.nft_metadata,
        &ctx.accounts.nft_edition,
        &ctx.accounts.system_program,
        &ctx.accounts.token_program,
        &ctx.accounts.associated_token_program,
        &ctx.accounts.pnft_shared.token_metadata_program,
        &ctx.accounts.pnft_shared.instructions,
        &ctx.accounts.owner_token_record,
        &ctx.accounts.dest_token_record,
        &ctx.accounts.pnft_shared.authorization_rules_program,
        auth_rules,
        authorization_data,
        None,
    )?;

    Ok(())
}
