use {
    crate::{
        input_validators::*,
        keypair::{CliSignerInfo, SignerIndex},
        offline::BLOCKHASH_ARG,
        ArgConstant,
    },
    clap::{App, Arg},
    solana_sdk::pubkey::Pubkey,
};

pub const NONCE_ARG: ArgConstant<'static> = ArgConstant {
    name: "nonce",
    long: "nonce",
    help: "Provide the nonce account to use when creating a nonced \n\
           transaction. Nonced transactions are useful when a transaction \n\
           requires a lengthy signing process. Learn more about nonced \n\
           transactions at https://docs.solana.com/offline-signing/durable-nonce",
};

pub const NONCE_AUTHORITY_ARG: ArgConstant<'static> = ArgConstant {
    name: "nonce_authority",
    long: "nonce-authority",
    help: "Provide the nonce authority keypair to use when signing a nonced transaction",
};

fn nonce_arg<'a, 'b>() -> Arg<'a, 'b> {
    Arg::with_name(NONCE_ARG.name)
        .long(NONCE_ARG.long)
        .takes_value(true)
        .value_name("PUBKEY")
        .requires(BLOCKHASH_ARG.name)
        .validator(is_valid_pubkey)
        .help(NONCE_ARG.help)
}

pub fn nonce_authority_arg<'a, 'b>() -> Arg<'a, 'b> {
    Arg::with_name(NONCE_AUTHORITY_ARG.name)
        .long(NONCE_AUTHORITY_ARG.long)
        .takes_value(true)
        .value_name("KEYPAIR")
        .validator(is_valid_signer)
        .help(NONCE_AUTHORITY_ARG.help)
}

pub trait NonceArgs {
    fn nonce_args(self, global: bool) -> Self;
}

impl NonceArgs for App<'_, '_> {
    fn nonce_args(self, global: bool) -> Self {
        self.arg(nonce_arg().global(global)).arg(
            nonce_authority_arg()
                .requires(NONCE_ARG.name)
                .global(global),
        )
    }
}

#[derive(Debug, PartialEq)]
pub struct NonceSignerInfo {
    nonce_account: Pubkey,
    signer_index: SignerIndex,
}

impl NonceSignerInfo {
    pub fn new(
        nonce_account: Option<Pubkey>,
        nonce_authority: Option<Pubkey>,
        signer_info: &CliSignerInfo,
    ) -> Option<Self> {
        nonce_account
            .zip(nonce_authority)
            .and_then(|(nonce_account, nonce_authority)| {
                signer_info
                    .index_of(Some(nonce_authority))
                    .map(|signer_index| Self {
                        nonce_account,
                        signer_index,
                    })
            })
    }
}
