use derivation_path::DerivationPath;
use ed25519_dalek_bip32::{ChildIndex, ExtendedSecretKey};
use solana_remote_wallet::remote_wallet::{DerivationPathComponent, RemoteWalletError};
use solana_sdk::signature::Keypair;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Bip32Error {
    #[error("ed25519-dalek-bip32 error {0}")]
    Ed25519DalekBip32Error(String),

    #[error("{0}")]
    RemoteWalletError(#[from] RemoteWalletError),
}

fn component_to_child_index(component: DerivationPathComponent) -> ChildIndex {
    ChildIndex::Hardened(component.as_u32() & !DerivationPathComponent::HARDENED_BIT)
}

pub fn parse_derivation_input(key_path: &str) -> Result<DerivationPath, Bip32Error> {
    let mut child_indexes = vec![];
    let _ = key_path.strip_suffix("/");
    let mut parts = key_path.split('/');
    if let Some(m) = parts.next() {
        println!("m was there");
    }
    if let Some(purpose) = parts.next() {
        let component = DerivationPathComponent::from_str(purpose)?;
        child_indexes.push(component_to_child_index(component));
    }
    if let Some(coin_type) = parts.next() {
        let component = DerivationPathComponent::from_str(coin_type)?;
        child_indexes.push(component_to_child_index(component));
    }
    if let Some(account) = parts.next() {
        let component = DerivationPathComponent::from_str(account)?;
        child_indexes.push(component_to_child_index(component));
    }
    if let Some(change) = parts.next() {
        let component = DerivationPathComponent::from_str(change)?;
        child_indexes.push(component_to_child_index(component));
    }
    if parts.next().is_some() {
        return Err(RemoteWalletError::InvalidDerivationPath(format!(
            "key path `{}` too deep, only <account>/<change> supported",
            key_path
        ))
        .into());
    }
    Ok(DerivationPath::new(child_indexes))
}

pub fn derive_keypair(
    seed: &[u8],
    derivation_path: &DerivationPath,
) -> Result<Keypair, Bip32Error> {
    // let seed = generate_seed_from_seed_phrase_and_passphrase(seed_phrase, passphrase);
    let extended = ExtendedSecretKey::from_seed(seed)
        .and_then(|seed| seed.derive(derivation_path))
        .map_err(|err| Bip32Error::Ed25519DalekBip32Error(err.to_string()))?;
    let extended_public_key = extended.public_key();
    Ok(Keypair::from_parts(
        extended.secret_key,
        extended_public_key,
    ))
}
