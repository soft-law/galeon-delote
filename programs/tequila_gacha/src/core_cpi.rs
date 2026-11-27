//! Metaplex Core CPIs built by hand (same bytes as mpl-core 0.12.1's builders: CreateCollectionV2,
//! CreateV2, BurnV1, no plugins). Dropping the mpl-core crate keeps the program small enough
//! to deploy within the devnet budget.
use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    instruction::{AccountMeta, Instruction},
    program::{invoke, invoke_signed},
};

pub const MPL_CORE_ID: Pubkey = pubkey!("CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d");

const CREATE_V2: u8 = 20;
const CREATE_COLLECTION_V2: u8 = 21;
const BURN_V1: u8 = 12;
/// AssetV1 / CollectionV1 account keys (first byte of the account).
pub const KEY_ASSET_V1: u8 = 1;

fn push_str(d: &mut Vec<u8>, s: &str) {
    d.extend_from_slice(&(s.len() as u32).to_le_bytes());
    d.extend_from_slice(s.as_bytes());
}

pub fn create_collection<'a>(
    collection: &AccountInfo<'a>,
    update_authority: &AccountInfo<'a>,
    payer: &AccountInfo<'a>,
    system_program: &AccountInfo<'a>,
    core_program: &AccountInfo<'a>,
    name: &str,
    uri: &str,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    let mut data = vec![CREATE_COLLECTION_V2];
    push_str(&mut data, name);
    push_str(&mut data, uri);
    data.extend_from_slice(&[0, 0]); // plugins: None, external_plugin_adapters: None
    let ix = Instruction {
        program_id: MPL_CORE_ID,
        accounts: vec![
            AccountMeta::new(collection.key(), true),
            AccountMeta::new_readonly(update_authority.key(), false),
            AccountMeta::new(payer.key(), true),
            AccountMeta::new_readonly(system_program.key(), false),
        ],
        data,
    };
    invoke_signed(
        &ix,
        &[collection.clone(), update_authority.clone(), payer.clone(), system_program.clone(), core_program.clone()],
        signer_seeds,
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn create_asset<'a>(
    asset: &AccountInfo<'a>,
    collection: &AccountInfo<'a>,
    authority: &AccountInfo<'a>,
    payer: &AccountInfo<'a>,
    owner: &AccountInfo<'a>,
    system_program: &AccountInfo<'a>,
    core_program: &AccountInfo<'a>,
    name: &str,
    uri: &str,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    let mut data = vec![CREATE_V2, 0]; // data_state: AccountState
    push_str(&mut data, name);
    push_str(&mut data, uri);
    data.extend_from_slice(&[0, 0]);
    let ix = Instruction {
        program_id: MPL_CORE_ID,
        accounts: vec![
            AccountMeta::new(asset.key(), true),
            AccountMeta::new(collection.key(), false),
            AccountMeta::new_readonly(authority.key(), true),
            AccountMeta::new(payer.key(), true),
            AccountMeta::new_readonly(owner.key(), false),
            AccountMeta::new_readonly(MPL_CORE_ID, false), // update_authority: None (collection's)
            AccountMeta::new_readonly(system_program.key(), false),
            AccountMeta::new_readonly(MPL_CORE_ID, false), // log_wrapper: None
        ],
        data,
    };
    invoke_signed(
        &ix,
        &[
            asset.clone(),
            collection.clone(),
            authority.clone(),
            payer.clone(),
            owner.clone(),
            system_program.clone(),
            core_program.clone(),
        ],
        signer_seeds,
    )?;
    Ok(())
}

/// Burn signed by `owner` (also the payer); Core checks ownership.
pub fn burn_asset<'a>(
    asset: &AccountInfo<'a>,
    collection: &AccountInfo<'a>,
    owner: &AccountInfo<'a>,
    system_program: &AccountInfo<'a>,
    core_program: &AccountInfo<'a>,
) -> Result<()> {
    let ix = Instruction {
        program_id: MPL_CORE_ID,
        accounts: vec![
            AccountMeta::new(asset.key(), false),
            AccountMeta::new(collection.key(), false),
            AccountMeta::new(owner.key(), true),
            AccountMeta::new_readonly(owner.key(), true),
            AccountMeta::new_readonly(system_program.key(), false),
            AccountMeta::new_readonly(MPL_CORE_ID, false),
        ],
        data: vec![BURN_V1, 0], // compression_proof: None
    };
    invoke(
        &ix,
        &[asset.clone(), collection.clone(), owner.clone(), system_program.clone(), core_program.clone()],
    )?;
    Ok(())
}
