//! Shared helpers: hand-built instructions, borsh mirrors of the program accounts, setup.
#![allow(dead_code)]

use borsh::{BorshDeserialize, BorshSerialize};
use magicsvm::{MagicSVM, TransactionTarget};
use solana_account::Account as StockAccount;
use solana_address::Address;
use solana_clock::Clock;
use solana_instruction::error::InstructionError;
use solana_keypair::Keypair;
use solana_message::{AccountMeta, Instruction, Message};
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction_error::TransactionError;

pub const PROGRAM_ID: Address = Address::from_str_const("2LVXXXxVUMQPnewQGKpNDb3dRRfhVdbSmhYwNppfuAis");
pub const SO_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../target/deploy/tequila_gacha.so");
pub const CORE_SO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mpl_core.so");

pub const CORE_ID: Address = Address::from_str_const("CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d");
pub const SYSTEM: Address = Address::from_str_const("11111111111111111111111111111111");
pub const TOKEN_PROGRAM: Address = Address::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub const VRF_PROGRAM: Address = Address::from_str_const("Vrf1RNUjXmQGjmQrQLvJHs9SNkvDJEsRVFPkfSQUwGz");
pub const DEFAULT_QUEUE: Address = Address::from_str_const("Cuj97ggrhhidhbu39TijNVqE74xvKJ69gDervRUXAxGh");
pub const EPHEMERAL_QUEUE: Address = Address::from_str_const("5hBR571xnXppuCPveTrctfTU7tJLSN94nq7kv7FRK5Tc");
pub const SLOT_HASHES: Address = Address::from_str_const("SysvarS1otHashes111111111111111111111111111");
pub const DLP: Address = Address::from_str_const("DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh");
pub const MAGIC_PROGRAM: Address = Address::from_str_const("Magic11111111111111111111111111111111111111");
pub const MAGIC_CONTEXT: Address = Address::from_str_const("MagicContext1111111111111111111111111111111");

// Program error codes (GachaError, Anchor offset 6000).
pub const E_SOLD_OUT: u32 = 6000;
pub const E_MACHINE_NOT_OPEN: u32 = 6001;
pub const E_LIVE_PULL_EXISTS: u32 = 6002;
pub const E_WRONG_PULL_STATE: u32 = 6003;
pub const E_RESPIN_TOO_EARLY: u32 = 6004;
pub const E_STALE_CALLBACK: u32 = 6005;
pub const E_CURRENCY_NOT_ACCEPTED: u32 = 6006;
pub const E_TIERS_LOCKED: u32 = 6007;
pub const E_NOT_FULFILLED: u32 = 6008;
pub const E_WRONG_QUEUE: u32 = 6009;
pub const E_INVALID_TIERS: u32 = 6010;
pub const E_NO_TIERS: u32 = 6011;
pub const E_WRONG_MACHINE_STATE: u32 = 6012;
pub const E_MISSING_TOKEN_ACCOUNTS: u32 = 6013;
pub const E_INVALID_TOKEN_ACCOUNT: u32 = 6014;
pub const E_NOT_ASSET_OWNER: u32 = 6015;
pub const E_NOT_CLAIMED: u32 = 6016;
pub const E_PRICE_ABOVE_MAX: u32 = 6018;
pub const E_INVALID_VALIDATOR: u32 = 6019;
pub const E_SETTLE_TOO_EARLY: u32 = 6020;
pub const E_NOT_PLAYER_OR_SESSION: u32 = 6021;
pub const E_SESSION_EXPIRED: u32 = 6022;
pub const E_NOT_ESCROW_SIGNER: u32 = 6023;
pub const E_INVALID_COUNT: u32 = 6024;
pub const ANCHOR_OWNED_BY_WRONG_PROGRAM: u32 = 3007;
pub const ANCHOR_ACCOUNT_NOT_INITIALIZED: u32 = 3012;
pub const SESSION_PROGRAM: Address = Address::from_str_const("KeyspM2ssCJbqUhQ4k7sveSiY4WjnYsrXkC8oDbwde5");

pub const PULL_PAID: u8 = 0;
pub const PULL_REQUESTED: u8 = 1;
pub const PULL_FULFILLED: u8 = 2;
pub const PULL_CLAIMED: u8 = 3;

// ---------- account mirrors ----------

#[derive(BorshDeserialize, Debug, Clone, Copy)]
pub struct Tier {
    pub cap: u32,
    pub assigned: u32,
    pub name: [u8; 32],
    pub uri: [u8; 128],
}

#[derive(BorshDeserialize, Debug, Clone)]
pub struct Machine {
    pub authority: [u8; 32],
    pub seller: [u8; 32],
    pub collection: [u8; 32],
    pub price_lamports: u64,
    pub usdc_mint: [u8; 32],
    pub price_usdc: u64,
    pub status: u8,
    pub er_validator: [u8; 32],
    pub pending: u32,
    pub tier_count: u8,
    pub tiers: Vec<Tier>,
    pub machine_id: u32,
    pub bump: u8,
}

/// On-chain Pull (v3, packs).
#[derive(BorshDeserialize, Debug, Clone)]
pub struct PullRaw {
    pub machine: [u8; 32],
    pub player: [u8; 32],
    pub nonce: u64,
    pub status: u8,
    pub snapshot: [u32; 6],
    pub request_seq: u8,
    pub paid_with: u8,
    pub price: u64,
    pub paid_at: i64,
    pub requested_at: i64,
    pub fulfilled_at: i64,
    pub randomness: [u8; 32],
    pub count: u8,
    pub claimed: u8,
    pub tiers: [u8; 5],
    pub rerolled: u8,
    pub allocations: [u32; 5],
    pub bump: u8,
}

/// PullRaw plus the index-0 view the single-pull tests read (tier, rerolled, allocation).
#[derive(Debug, Clone)]
pub struct Pull {
    pub machine: [u8; 32],
    pub player: [u8; 32],
    pub nonce: u64,
    pub status: u8,
    pub snapshot: [u32; 6],
    pub request_seq: u8,
    pub paid_with: u8,
    pub price: u64,
    pub requested_at: i64,
    pub fulfilled_at: i64,
    pub randomness: [u8; 32],
    pub count: u8,
    pub claimed: u8,
    pub tiers: [u8; 5],
    pub rerolled_mask: u8,
    pub allocations: [u32; 5],
    pub tier: u8,
    pub rerolled: bool,
    pub allocation: u32,
}

impl From<PullRaw> for Pull {
    fn from(r: PullRaw) -> Self {
        Pull {
            machine: r.machine,
            player: r.player,
            nonce: r.nonce,
            status: r.status,
            snapshot: r.snapshot,
            request_seq: r.request_seq,
            paid_with: r.paid_with,
            price: r.price,
            requested_at: r.requested_at,
            fulfilled_at: r.fulfilled_at,
            randomness: r.randomness,
            count: r.count,
            claimed: r.claimed,
            tiers: r.tiers,
            rerolled_mask: r.rerolled,
            allocations: r.allocations,
            tier: r.tiers[0],
            rerolled: r.rerolled & 1 == 1,
            allocation: r.allocations[0],
        }
    }
}

// ---------- the draw, replicated (sha256(randomness ‖ tag ‖ i) → u64 LE) ----------

pub fn indexed_roll(r: &[u8; 32], tag: &[u8], i: u8) -> u64 {
    let h = solana_sha256_hasher::hashv(&[r, tag, &[i]]).to_bytes();
    u64::from_le_bytes(h[..8].try_into().unwrap())
}

pub fn draw(weights: &[u32; 6], r: u64) -> Option<u8> {
    let total: u64 = weights.iter().map(|w| *w as u64).sum();
    if total == 0 {
        return None;
    }
    let x = r % total;
    let mut acc = 0;
    for (i, w) in weights.iter().enumerate() {
        acc += *w as u64;
        if x < acc {
            return Some(i as u8);
        }
    }
    None
}

fn pad(w: &[u32]) -> [u32; 6] {
    let mut a = [0u32; 6];
    a[..w.len()].copy_from_slice(w);
    a
}

/// The tiers a pack of `count` gets from `snapshot` (picks without replacement from the copy).
pub fn pack_tiers(snapshot: &[u32], r: &[u8; 32], count: u8) -> Vec<u8> {
    let mut copy = pad(snapshot);
    (0..count)
        .map(|i| {
            let t = draw(&copy, indexed_roll(r, b"pick", i)).unwrap();
            copy[t as usize] -= 1;
            t
        })
        .collect()
}

/// A randomness whose picks from `snapshot` are exactly `want` (searched).
pub fn rand_picking(snapshot: &[u32], want: &[u8]) -> [u8; 32] {
    rand_where(|r| pack_tiers(snapshot, r, want.len() as u8) == want)
}

pub fn rand_where(ok: impl Fn(&[u8; 32]) -> bool) -> [u8; 32] {
    for i in 0u64.. {
        let r = solana_sha256_hasher::hash(&i.to_le_bytes()).to_bytes();
        if ok(&r) {
            return r;
        }
    }
    unreachable!()
}

#[derive(BorshDeserialize, Debug, Clone)]
pub struct PlayerState {
    pub next_nonce: u64,
    pub live_pull: Option<[u8; 32]>,
    pub bump: u8,
}

#[derive(BorshDeserialize, Debug, Clone)]
pub struct Redemption {
    pub asset: [u8; 32],
    pub redeemer: [u8; 32],
    pub machine: [u8; 32],
    pub tier: u8,
    pub allocation: u32,
    pub redeemed_at: i64,
    pub bump: u8,
}

pub fn discriminator(ns: &str, name: &str) -> [u8; 8] {
    let h = solana_sha256_hasher::hash(format!("{ns}:{name}").as_bytes()).to_bytes();
    h[..8].try_into().unwrap()
}

pub fn decode<T: BorshDeserialize>(name: &str, data: &[u8]) -> T {
    assert_eq!(&data[..8], &discriminator("account", name), "discriminator of {name}");
    let mut s = &data[8..];
    T::deserialize(&mut s).unwrap()
}

// ---------- PDAs ----------

pub fn pda(seeds: &[&[u8]]) -> Address {
    Address::find_program_address(seeds, &PROGRAM_ID).0
}
pub fn machine_pda(authority: &Address, id: u32) -> Address {
    pda(&[b"machine", authority.as_ref(), &id.to_le_bytes()])
}
pub fn collection_pda(machine: &Address) -> Address {
    pda(&[b"collection", machine.as_ref()])
}
pub fn core_authority_pda() -> Address {
    pda(&[b"authority"])
}
pub fn player_state_pda(machine: &Address, player: &Address) -> Address {
    pda(&[b"player", machine.as_ref(), player.as_ref()])
}
pub fn pull_pda(machine: &Address, player: &Address, nonce: u64) -> Address {
    pda(&[b"pull", machine.as_ref(), player.as_ref(), &nonce.to_le_bytes()])
}
/// Asset of index 0 (single pulls).
pub fn asset_pda(pull: &Address) -> Address {
    asset_pda_i(pull, 0)
}
pub fn asset_pda_i(pull: &Address, index: u8) -> Address {
    pda(&[b"asset", pull.as_ref(), &[index]])
}
pub fn redemption_pda(asset: &Address) -> Address {
    pda(&[b"redemption", asset.as_ref()])
}
pub fn program_identity() -> Address {
    pda(&[b"identity"])
}
pub fn scoped_vrf_identity() -> Address {
    Address::find_program_address(&[b"identity", PROGRAM_ID.as_ref()], &VRF_PROGRAM).0
}

// ---------- instruction builders ----------

fn ix(name: &str, accounts: Vec<AccountMeta>, args: Vec<u8>) -> Instruction {
    let mut data = discriminator("global", name).to_vec();
    data.extend(args);
    Instruction { program_id: PROGRAM_ID, accounts, data }
}

fn borsh<T: BorshSerialize>(v: &T) -> Vec<u8> {
    borsh::to_vec(v).unwrap()
}

pub struct MachineCfg {
    pub id: u32,
    pub seller: Address,
    pub usdc_mint: Address,
    pub price_lamports: u64,
    pub price_usdc: u64,
    pub er_validator: Address,
}

pub fn init_machine_ix(authority: &Address, c: &MachineCfg) -> Instruction {
    let machine = machine_pda(authority, c.id);
    let mut args = Vec::new();
    args.extend(c.id.to_le_bytes());
    args.extend(c.seller.as_ref());
    args.extend(c.usdc_mint.as_ref());
    args.extend(c.price_lamports.to_le_bytes());
    args.extend(c.price_usdc.to_le_bytes());
    args.extend(c.er_validator.as_ref());
    args.extend(borsh(&"Tequila Gacha".to_string()));
    args.extend(borsh(&"https://example.com/collection.json".to_string()));
    ix(
        "init_machine",
        vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new(machine, false),
            AccountMeta::new(collection_pda(&machine), false),
            AccountMeta::new_readonly(core_authority_pda(), false),
            AccountMeta::new_readonly(CORE_ID, false),
            AccountMeta::new_readonly(SYSTEM, false),
        ],
        args,
    )
}

#[derive(BorshSerialize, Clone)]
pub struct TierInput {
    pub cap: u32,
    pub name: String,
    pub uri: String,
}

pub fn tier(cap: u32, name: &str) -> TierInput {
    TierInput { cap, name: name.into(), uri: format!("https://example.com/{name}.json") }
}

fn admin_metas(authority: &Address, machine: &Address) -> Vec<AccountMeta> {
    vec![AccountMeta::new(*authority, true), AccountMeta::new(*machine, false)]
}

pub fn set_tiers_ix(authority: &Address, machine: &Address, start: u8, tiers: &[TierInput]) -> Instruction {
    let mut args = vec![start];
    args.extend(borsh(&tiers.to_vec()));
    ix("set_tiers", admin_metas(authority, machine), args)
}
pub fn open_ix(authority: &Address, machine: &Address) -> Instruction {
    ix("open", admin_metas(authority, machine), vec![])
}
pub fn close_ix(authority: &Address, machine: &Address) -> Instruction {
    ix("close", admin_metas(authority, machine), vec![])
}
pub fn set_prices_ix(authority: &Address, machine: &Address, lamports: u64, usdc: u64) -> Instruction {
    let mut args = lamports.to_le_bytes().to_vec();
    args.extend(usdc.to_le_bytes());
    ix("set_prices", admin_metas(authority, machine), args)
}
pub fn set_er_validator_ix(authority: &Address, machine: &Address, v: &Address) -> Instruction {
    ix("set_er_validator", admin_metas(authority, machine), v.as_ref().to_vec())
}

pub const SOL: u8 = 0;
pub const USDC: u8 = 1;

pub struct UsdcAccounts {
    pub player_ata: Address,
    pub seller_ata: Address,
    pub mint: Address,
}

pub fn buy_pull_ix(
    player: &Address,
    machine: &Address,
    seller: &Address,
    nonce: u64,
    currency: u8,
    usdc: Option<&UsdcAccounts>,
) -> Instruction {
    buy_pull_max_ix(player, machine, seller, nonce, currency, usdc, u64::MAX)
}

pub fn buy_pull_max_ix(
    player: &Address,
    machine: &Address,
    seller: &Address,
    nonce: u64,
    currency: u8,
    usdc: Option<&UsdcAccounts>,
    max_price: u64,
) -> Instruction {
    buy_pack_ix(player, machine, seller, nonce, currency, usdc, 1, max_price)
}

#[allow(clippy::too_many_arguments)]
pub fn buy_pack_ix(
    player: &Address,
    machine: &Address,
    seller: &Address,
    nonce: u64,
    currency: u8,
    usdc: Option<&UsdcAccounts>,
    count: u8,
    max_price_total: u64,
) -> Instruction {
    let mut args = vec![currency, count];
    args.extend(max_price_total.to_le_bytes());
    let opt = |a: Option<Address>, w: bool| match a {
        Some(a) if w => AccountMeta::new(a, false),
        Some(a) => AccountMeta::new_readonly(a, false),
        None => AccountMeta::new_readonly(PROGRAM_ID, false),
    };
    ix(
        "buy_pull",
        vec![
            AccountMeta::new(*player, true),
            AccountMeta::new(*machine, false),
            AccountMeta::new(player_state_pda(machine, player), false),
            AccountMeta::new(pull_pda(machine, player, nonce), false),
            AccountMeta::new(*seller, false),
            AccountMeta::new_readonly(SYSTEM, false),
            opt(usdc.map(|u| u.player_ata), true),
            opt(usdc.map(|u| u.seller_ata), true),
            opt(usdc.map(|u| u.mint), false),
            opt(usdc.map(|_| TOKEN_PROGRAM), false),
        ],
        args,
    )
}

fn spin_like(name: &str, signer: &Address, pull: &Address, queue: Address, token: Option<Address>) -> Instruction {
    ix(
        name,
        vec![
            AccountMeta::new(*signer, true),
            AccountMeta::new(*pull, false),
            AccountMeta::new(queue, false),
            AccountMeta::new_readonly(token.unwrap_or(PROGRAM_ID), false),
            AccountMeta::new_readonly(program_identity(), false),
            AccountMeta::new_readonly(VRF_PROGRAM, false),
            AccountMeta::new_readonly(SLOT_HASHES, false),
            AccountMeta::new_readonly(SYSTEM, false),
        ],
        vec![],
    )
}
pub fn spin_ix(player: &Address, pull: &Address, queue: Address) -> Instruction {
    spin_like("spin", player, pull, queue, None)
}
pub fn respin_ix(player: &Address, pull: &Address, queue: Address) -> Instruction {
    spin_like("respin", player, pull, queue, None)
}
/// spin signed by a session key, with its SessionTokenV2.
pub fn spin_session_ix(signer: &Address, pull: &Address, queue: Address, token: &Address) -> Instruction {
    spin_like("spin", signer, pull, queue, Some(*token))
}
pub fn respin_session_ix(signer: &Address, pull: &Address, queue: Address, token: &Address) -> Instruction {
    spin_like("respin", signer, pull, queue, Some(*token))
}

pub fn callback_ix(identity: AccountMeta, pull: &Address, randomness: [u8; 32], seq: u8) -> Instruction {
    let mut args = randomness.to_vec();
    args.push(seq);
    ix("callback_spin", vec![identity, AccountMeta::new(*pull, false)], args)
}

pub fn delegate_pull_ix(player: &Address, machine: &Address, pull: &Address) -> Instruction {
    let buffer = pda(&[b"buffer", pull.as_ref()]);
    let record = Address::find_program_address(&[b"delegation", pull.as_ref()], &DLP).0;
    let meta = Address::find_program_address(&[b"delegation-metadata", pull.as_ref()], &DLP).0;
    ix(
        "delegate_pull",
        vec![
            AccountMeta::new(*player, true),
            AccountMeta::new_readonly(*machine, false),
            AccountMeta::new_readonly(player_state_pda(machine, player), false),
            AccountMeta::new(buffer, false),
            AccountMeta::new(record, false),
            AccountMeta::new(meta, false),
            AccountMeta::new(*pull, false),
            AccountMeta::new_readonly(PROGRAM_ID, false),
            AccountMeta::new_readonly(DLP, false),
            AccountMeta::new_readonly(SYSTEM, false),
        ],
        vec![],
    )
}

pub fn settle_pull_ix(player: &Address, pull: &Address) -> Instruction {
    settle_session_ix(player, pull, None)
}

pub fn settle_session_ix(signer: &Address, pull: &Address, token: Option<&Address>) -> Instruction {
    ix(
        "settle_pull",
        vec![
            AccountMeta::new(*signer, true),
            AccountMeta::new(*pull, false),
            AccountMeta::new_readonly(token.copied().unwrap_or(PROGRAM_ID), false),
            AccountMeta::new_readonly(MAGIC_PROGRAM, false),
            AccountMeta::new(MAGIC_CONTEXT, false),
        ],
        vec![],
    )
}

// ---------- MagicBlock Session Keys (SessionTokenV2, hand-packed) ----------

pub fn session_token_pda(target: &Address, signer: &Address, authority: &Address) -> Address {
    Address::find_program_address(
        &[b"session_token_v2", target.as_ref(), signer.as_ref(), authority.as_ref()],
        &SESSION_PROGRAM,
    )
    .0
}

/// Writes a SessionTokenV2 at its PDA (owner KeyspM…) as create_session_v2 would. Returns the PDA.
pub fn put_session_token(
    svm: &mut MagicSVM,
    authority: &Address,
    target: &Address,
    signer: &Address,
    valid_until: i64,
) -> Address {
    let at = session_token_pda(target, signer, authority);
    put_session_token_at(svm, at, SESSION_PROGRAM, authority, target, signer, valid_until);
    at
}

pub fn put_session_token_at(
    svm: &mut MagicSVM,
    at: Address,
    owner: Address,
    authority: &Address,
    target: &Address,
    signer: &Address,
    valid_until: i64,
) {
    let mut d = discriminator("account", "SessionTokenV2").to_vec();
    d.extend(authority.as_ref());
    d.extend(target.as_ref());
    d.extend(signer.as_ref());
    d.extend(authority.as_ref()); // fee_payer
    d.extend(valid_until.to_le_bytes());
    svm.set_account(at, StockAccount { lamports: 1_900_000, data: d, owner, executable: false, rent_epoch: 0 })
        .unwrap();
}

/// `payer` signs and pays; the asset goes to `player` (the pull's player).
pub fn claim_ix(payer: &Address, player: &Address, machine: &Address, pull: &Address) -> Instruction {
    claim_index_ix(payer, player, machine, pull, 0)
}

/// claim of the pack's next index (`index` = the pull's `claimed` when it runs).
pub fn claim_index_ix(payer: &Address, player: &Address, machine: &Address, pull: &Address, index: u8) -> Instruction {
    ix(
        "claim",
        vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(*player, false),
            AccountMeta::new(*machine, false),
            AccountMeta::new(player_state_pda(machine, player), false),
            AccountMeta::new(*pull, false),
            AccountMeta::new(asset_pda_i(pull, index), false),
            AccountMeta::new(collection_pda(machine), false),
            AccountMeta::new_readonly(core_authority_pda(), false),
            AccountMeta::new_readonly(CORE_ID, false),
            AccountMeta::new_readonly(SYSTEM, false),
        ],
        vec![],
    )
}

pub fn redeem_ix(owner: &Address, machine: &Address, pull: &Address) -> Instruction {
    redeem_index_ix(owner, machine, pull, 0)
}

pub fn redeem_index_ix(owner: &Address, machine: &Address, pull: &Address, index: u8) -> Instruction {
    let asset = asset_pda_i(pull, index);
    ix(
        "redeem",
        vec![
            AccountMeta::new(*owner, true),
            AccountMeta::new_readonly(*machine, false),
            AccountMeta::new_readonly(*pull, false),
            AccountMeta::new(asset, false),
            AccountMeta::new(collection_pda(machine), false),
            AccountMeta::new(redemption_pda(&asset), false),
            AccountMeta::new_readonly(CORE_ID, false),
            AccountMeta::new_readonly(SYSTEM, false),
        ],
        vec![index],
    )
}

/// Core TransferV1 signed by the current owner.
pub fn core_transfer_ix(asset: Address, collection: Address, owner: Address, new_owner: Address) -> Instruction {
    Instruction {
        program_id: CORE_ID,
        accounts: vec![
            AccountMeta::new(asset, false),
            AccountMeta::new_readonly(collection, false),
            AccountMeta::new(owner, true),
            AccountMeta::new_readonly(owner, true),
            AccountMeta::new_readonly(new_owner, false),
            AccountMeta::new_readonly(CORE_ID, false),
            AccountMeta::new_readonly(CORE_ID, false),
        ],
        data: vec![14, 0],
    }
}

// ---------- VRF stand-in replica (MagicSVM 0.3.0 internals) ----------

#[derive(BorshSerialize)]
struct ProvideRandomnessArgs {
    nonce: u64,
    scoped: bool,
    randomness: [u8; 32],
    callback_discriminator: Vec<u8>,
    callback_args: Vec<u8>,
    callback_signers: Vec<bool>,
}

/// MagicSVM's oracle executor key.
pub fn magicsvm_executor() -> Keypair {
    Keypair::new_from_array([7; 32])
}

/// A fulfillment delivered through the VRF program with the real scoped identity.
pub fn provide_randomness_ix(pull: Address, randomness: [u8; 32], seq: u8, vrf_nonce: u64) -> Instruction {
    let mut data = vec![4u8, 0, 0, 0, 0, 0, 0, 0];
    data.extend(borsh(&ProvideRandomnessArgs {
        nonce: vrf_nonce,
        scoped: true,
        randomness,
        callback_discriminator: discriminator("global", "callback_spin").to_vec(),
        callback_args: vec![seq],
        callback_signers: vec![false],
    }));
    Instruction {
        program_id: VRF_PROGRAM,
        accounts: vec![
            AccountMeta::new(magicsvm_executor().pubkey(), true),
            AccountMeta::new_readonly(scoped_vrf_identity(), false),
            AccountMeta::new_readonly(PROGRAM_ID, false),
            AccountMeta::new(pull, false),
        ],
        data,
    }
}

// ---------- SPL token packing ----------

pub fn mint_account(decimals: u8) -> StockAccount {
    let mut d = vec![0u8; 82];
    d[44] = decimals;
    d[45] = 1;
    StockAccount { lamports: 1_461_600, data: d, owner: TOKEN_PROGRAM, executable: false, rent_epoch: 0 }
}

pub fn token_account(mint: &Address, owner: &Address, amount: u64) -> StockAccount {
    let mut d = vec![0u8; 165];
    d[0..32].copy_from_slice(mint.as_ref());
    d[32..64].copy_from_slice(owner.as_ref());
    d[64..72].copy_from_slice(&amount.to_le_bytes());
    d[108] = 1;
    StockAccount { lamports: 2_039_280, data: d, owner: TOKEN_PROGRAM, executable: false, rent_epoch: 0 }
}

pub fn token_amount(svm: &MagicSVM, a: &Address) -> u64 {
    let d = svm.get_account(a).unwrap().data;
    u64::from_le_bytes(d[64..72].try_into().unwrap())
}

// ---------- harness ----------

pub fn custom(ix_index: u8, code: u32) -> TransactionError {
    TransactionError::InstructionError(ix_index, InstructionError::Custom(code))
}

pub fn randomness_u64s(first: u64, second: u64) -> [u8; 32] {
    let mut r = [0u8; 32];
    r[0..8].copy_from_slice(&first.to_le_bytes());
    r[8..16].copy_from_slice(&second.to_le_bytes());
    r
}

pub struct Env {
    pub svm: MagicSVM,
    pub admin: Keypair,
    pub seller: Keypair,
    pub machine: Address,
    pub usdc_mint: Address,
}

pub const PRICE_LAMPORTS: u64 = 10_000_000;
pub const PRICE_USDC: u64 = 2_500_000;

impl Env {
    pub fn send(&mut self, ixs: &[Instruction], payer: &Keypair, extra: &[&Keypair]) -> Result<u64, TransactionError> {
        self.send_to(TransactionTarget::Base, ixs, payer, extra)
    }

    pub fn send_to(
        &mut self,
        target: TransactionTarget,
        ixs: &[Instruction],
        payer: &Keypair,
        extra: &[&Keypair],
    ) -> Result<u64, TransactionError> {
        let mut signers: Vec<&Keypair> = vec![payer];
        signers.extend_from_slice(extra);
        let msg = Message::new(ixs, Some(&payer.pubkey()));
        let tx = Transaction::new(&signers, msg, self.svm.latest_blockhash_for(target));
        let r = self
            .svm
            .send_transaction_to(target, tx)
            .map(|m| m.compute_units_consumed)
            .map_err(|f| {
                eprintln!("tx failed: {:?}\n  logs: {:#?}", f.err, f.meta.logs);
                f.err
            });
        self.svm.expire_blockhash_for(target);
        r
    }

    pub fn admin_send(&mut self, ix: Instruction) -> Result<u64, TransactionError> {
        let admin = self.admin.insecure_clone();
        self.send(&[ix], &admin, &[])
    }

    pub fn player(&mut self) -> Keypair {
        let p = Keypair::new();
        self.svm.airdrop(&p.pubkey(), 10_000_000_000).unwrap();
        p
    }

    pub fn machine(&self) -> Machine {
        decode("Machine", &self.svm.get_account(&self.machine).unwrap().data)
    }
    pub fn pull(&self, pull: &Address) -> Pull {
        decode::<PullRaw>("Pull", &self.svm.get_account(pull).unwrap().data).into()
    }
    pub fn pull_er(&self, pull: &Address) -> Pull {
        decode::<PullRaw>("Pull", &self.svm.get_account_for(TransactionTarget::Ephemeral, pull).unwrap().data).into()
    }
    pub fn player_state(&self, player: &Address) -> PlayerState {
        decode("PlayerState", &self.svm.get_account(&player_state_pda(&self.machine, player)).unwrap().data)
    }

    pub fn next_nonce(&self, player: &Address) -> u64 {
        match self.svm.get_account(&player_state_pda(&self.machine, player)) {
            Some(a) if !a.data.is_empty() => decode::<PlayerState>("PlayerState", &a.data).next_nonce,
            _ => 0,
        }
    }

    /// buy_pull in SOL; returns the Pull address.
    pub fn buy_sol(&mut self, player: &Keypair) -> Result<Address, TransactionError> {
        let nonce = self.next_nonce(&player.pubkey());
        let ix = buy_pull_ix(&player.pubkey(), &self.machine, &self.seller.pubkey(), nonce, SOL, None);
        self.send(&[ix], player, &[])?;
        Ok(pull_pda(&self.machine, &player.pubkey(), nonce))
    }

    pub fn usdc_accounts(&mut self, player: &Address, amount: u64) -> UsdcAccounts {
        let player_ata = Address::new_unique();
        let seller_ata = Address::new_unique();
        self.svm.set_account(player_ata, token_account(&self.usdc_mint, player, amount)).unwrap();
        self.svm.set_account(seller_ata, token_account(&self.usdc_mint, &self.seller.pubkey(), 0)).unwrap();
        UsdcAccounts { player_ata, seller_ata, mint: self.usdc_mint }
    }

    pub fn spin_base(&mut self, player: &Keypair, pull: &Address) -> Result<u64, TransactionError> {
        self.send(&[spin_ix(&player.pubkey(), pull, DEFAULT_QUEUE)], player, &[])
    }

    pub fn slot(&self) -> u64 {
        self.svm.get_sysvar::<Clock>().slot
    }

    /// Runs every due VRF fulfillment (each request is due one slot after it).
    pub fn fulfill(&mut self) -> Vec<Result<(), TransactionError>> {
        let slot = self.slot();
        self.svm
            .warp_to_slot(slot + 1)
            .into_iter()
            .map(|r| {
                r.map(|_| ()).map_err(|f| {
                    eprintln!("fulfillment failed: {:?}\n  logs: {:#?}", f.err, f.meta.logs);
                    f.err
                })
            })
            .collect()
    }

    /// Moves unix time forward on both ledgers without advancing the slot (so no
    /// pending fulfillment fires).
    pub fn advance_secs(&mut self, secs: i64) {
        let mut c = self.svm.get_sysvar::<Clock>();
        c.unix_timestamp += secs;
        self.svm.set_sysvar(&c);
    }

    /// buy (SOL) + spin on base + fulfill with pinned randomness.
    pub fn fulfilled_pull(&mut self, player: &Keypair, randomness: [u8; 32]) -> Address {
        let pull = self.buy_sol(player).unwrap();
        self.spin_base(player, &pull).unwrap();
        self.svm.set_vrf_randomness(Some(randomness));
        let r = self.fulfill();
        assert_eq!(r, vec![Ok(())]);
        self.svm.set_vrf_randomness(None);
        pull
    }

    /// settle_pull on the ER. MagicSVM 0.3.0 does not close the DLP delegation record and
    /// metadata on undelegation (devnet's DLP does), and its harness re-delegates any account
    /// written on base while that metadata exists. Close them here as devnet would.
    pub fn settle(&mut self, player: &Keypair, pull: &Address) -> Result<u64, TransactionError> {
        let cu = self.send_to(
            magicsvm::TransactionTarget::Ephemeral,
            &[settle_pull_ix(&player.pubkey(), pull)],
            player,
            &[],
        )?;
        for tag in [&b"delegation"[..], &b"delegation-metadata"[..]] {
            let a = Address::find_program_address(&[tag, pull.as_ref()], &DLP).0;
            self.svm.set_account(a, StockAccount::default()).unwrap();
        }
        Ok(cu)
    }

    /// Claims the pull's next index.
    pub fn claim(&mut self, player: &Keypair, pull: &Address) -> Result<u64, TransactionError> {
        let m = self.machine;
        let i = self.svm.get_account(pull).map(|a| decode::<PullRaw>("Pull", &a.data).claimed).unwrap_or(0);
        self.send(&[claim_index_ix(&player.pubkey(), &player.pubkey(), &m, pull, i)], player, &[])
    }

    /// buy a pack of `count` in SOL; returns the Pull address.
    pub fn buy_pack(&mut self, player: &Keypair, count: u8) -> Result<Address, TransactionError> {
        let nonce = self.next_nonce(&player.pubkey());
        let ix = buy_pack_ix(&player.pubkey(), &self.machine, &self.seller.pubkey(), nonce, SOL, None, count, u64::MAX);
        self.send(&[ix], player, &[])?;
        Ok(pull_pda(&self.machine, &player.pubkey(), nonce))
    }
}

/// A machine in Draft with the given tiers set, SOL and USDC prices, not opened.
pub fn draft_env(tiers: &[TierInput]) -> Env {
    let mut svm = MagicSVM::new();
    svm.add_program_from_file(PROGRAM_ID, SO_PATH).expect("run `cargo build-sbf` at the repo root first");
    svm.add_program_from_file(CORE_ID, CORE_SO).unwrap();
    let admin = Keypair::new();
    let seller = Keypair::new();
    svm.airdrop(&admin.pubkey(), 50_000_000_000).unwrap();
    svm.airdrop(&seller.pubkey(), 1_000_000_000).unwrap();
    let usdc_mint = Address::new_unique();
    svm.set_account(usdc_mint, mint_account(6)).unwrap();
    let validator = svm.validator_identity();
    let cfg = MachineCfg {
        id: 1,
        seller: seller.pubkey(),
        usdc_mint,
        price_lamports: PRICE_LAMPORTS,
        price_usdc: PRICE_USDC,
        er_validator: validator,
    };
    let machine = machine_pda(&admin.pubkey(), 1);
    let mut env = Env { svm, admin, seller, machine, usdc_mint };
    let a = env.admin.pubkey();
    env.admin_send(init_machine_ix(&a, &cfg)).expect("init_machine");
    if !tiers.is_empty() {
        env.admin_send(set_tiers_ix(&a, &machine, 0, tiers)).expect("set_tiers");
    }
    env
}

/// An open machine with the given tiers.
pub fn open_env(tiers: &[TierInput]) -> Env {
    let mut env = draft_env(tiers);
    let (a, m) = (env.admin.pubkey(), env.machine);
    env.admin_send(open_ix(&a, &m)).expect("open");
    env
}

// ---------- Magic Action: settle_and_claim (post-undelegate claim_action) ----------

pub const ACTION_ESCROW_INDEX: u8 = 255;

/// DLP ephemeral balance PDA ["balance", authority, index]: pays the asset rent in claim_action.
pub fn escrow_pda(authority: &Address) -> Address {
    Address::find_program_address(&[b"balance", authority.as_ref(), &[ACTION_ESCROW_INDEX]], &DLP).0
}

/// DLP TopUpEphemeralBalance (discriminator 9): payer funds the escrow of `authority`.
pub fn top_up_escrow_ix(payer: &Address, authority: &Address, lamports: u64) -> Instruction {
    let mut data = 9u64.to_le_bytes().to_vec();
    data.extend(lamports.to_le_bytes());
    data.push(ACTION_ESCROW_INDEX);
    Instruction {
        program_id: DLP,
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new_readonly(*authority, false),
            AccountMeta::new(escrow_pda(authority), false),
            AccountMeta::new_readonly(SYSTEM, false),
        ],
        data,
    }
}

pub fn settle_and_claim_ix(signer: &Address, pull: &Address, machine: &Address, token: Option<&Address>) -> Instruction {
    ix(
        "settle_and_claim",
        vec![
            AccountMeta::new(*signer, true),
            AccountMeta::new(*pull, false),
            AccountMeta::new_readonly(token.copied().unwrap_or(PROGRAM_ID), false),
            AccountMeta::new_readonly(*machine, false),
            AccountMeta::new_readonly(PROGRAM_ID, false),
            AccountMeta::new_readonly(MAGIC_PROGRAM, false),
            AccountMeta::new(MAGIC_CONTEXT, false),
        ],
        vec![],
    )
}

/// claim_action called directly (not through DLP): escrow cannot sign.
pub fn claim_action_direct_ix(caller: &Address, player: &Address, machine: &Address, pull: &Address) -> Instruction {
    ix(
        "claim_action",
        vec![
            AccountMeta::new(*player, false),
            AccountMeta::new(*machine, false),
            AccountMeta::new(player_state_pda(machine, player), false),
            AccountMeta::new(*pull, false),
            AccountMeta::new(asset_pda(pull), false),
            AccountMeta::new(collection_pda(machine), false),
            AccountMeta::new_readonly(core_authority_pda(), false),
            AccountMeta::new_readonly(CORE_ID, false),
            AccountMeta::new_readonly(SYSTEM, false),
            AccountMeta::new_readonly(PROGRAM_ID, false),
            AccountMeta::new(*caller, true),
            AccountMeta::new(escrow_pda(caller), false),
        ],
        vec![],
    )
}
