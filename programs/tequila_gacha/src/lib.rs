//! tequila_gacha: a gacha machine with VRF draws (base layer or MagicBlock Ephemeral Rollup)
//! and Metaplex Core prizes.
//!
//! Money, `Machine` and Core live on the base layer. Only `Pull` is delegated: the ER runs
//! `spin`, the VRF callback and `settle_pull`. Odds are the remaining stock per tier, frozen
//! into `Pull.snapshot` at payment.
use anchor_lang::prelude::*;
use anchor_lang::solana_program::{instruction::Instruction, program::invoke};
use anchor_lang::system_program;
use anchor_lang::Discriminator;
use ephemeral_rollups_sdk::anchor::{action, commit, delegate, ephemeral, vrf, vrf_callback};
use ephemeral_rollups_sdk::cpi::DelegateConfig;
use ephemeral_rollups_sdk::ephem::{CallHandler, MagicIntentBundleBuilder};
use ephemeral_rollups_sdk::{ActionArgs, ShortAccountMeta};
use ephemeral_rollups_sdk::vrf::{
    self,
    consts::{program_identity_pubkey, scoped_vrf_identity_const},
    instructions::{create_request_randomness_ix_const, RequestRandomnessParams},
    types::SerializableAccountMeta,
};
use session_keys::SessionTokenV2;
use solana_sha256_hasher::hashv;

pub mod core_cpi;
pub mod logic;
use core_cpi::MPL_CORE_ID;
use logic::{draw, draw_pack, indexed_roll, sellable, MAX_PACK, MAX_TIERS, REROLL};

declare_id!("2LVXXXxVUMQPnewQGKpNDb3dRRfhVdbSmhYwNppfuAis");

pub const MACHINE_SEED: &[u8] = b"machine";
pub const PLAYER_SEED: &[u8] = b"player";
pub const PULL_SEED: &[u8] = b"pull";
pub const ASSET_SEED: &[u8] = b"asset";
pub const REDEMPTION_SEED: &[u8] = b"redemption";
pub const COLLECTION_SEED: &[u8] = b"collection";
pub const AUTHORITY_SEED: &[u8] = b"authority";

pub const RESPIN_AFTER_SECS: i64 = 130;
/// Escrow index of the post-undelegate claim action (`ActionArgs::new` default).
pub const ACTION_ESCROW_INDEX: u8 = 255;
/// Base-layer compute budget requested for claim_action.
pub const CLAIM_ACTION_CU: u32 = 120_000;
/// Pack sizes `buy_pull` accepts.
pub const PACK_COUNTS: [u8; 3] = [1, 3, 5];
pub const NAME_LEN: usize = 32;
pub const URI_LEN: usize = 128;
/// SPL Token (classic). Devnet/mainnet USDC uses it.
pub const TOKEN_PROGRAM_ID: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

const PROGRAM_IDENTITY: Pubkey = program_identity_pubkey(&crate::ID);
/// The scoped VRF identity `#[vrf_callback]` would inject, computed at compile time
/// (the macro's runtime expression costs ~768k CU under anchor-lang 1.0.2).
const SCOPED_VRF_IDENTITY: Pubkey = Pubkey::new_from_array(scoped_vrf_identity_const(&crate::ID).0);

fn remaining(m: &Machine) -> [u32; MAX_TIERS] {
    let mut r = [0u32; MAX_TIERS];
    for i in 0..m.tier_count as usize {
        r[i] = m.tiers[i].cap - m.tiers[i].assigned;
    }
    r
}

fn fixed<const N: usize>(s: &str) -> Result<[u8; N]> {
    require!(s.len() <= N, GachaError::InvalidTiers);
    let mut out = [0u8; N];
    out[..s.len()].copy_from_slice(s.as_bytes());
    Ok(out)
}

fn trimmed(b: &[u8]) -> String {
    let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// spin, respin and settle_pull: the pull's player, or a MagicBlock session key whose
/// SessionTokenV2 (owned by the session program, checked by `Account`) names this program,
/// that signer and the pull's player, and has not expired.
fn authorize(signer: &Pubkey, player: &Pubkey, token: &Option<Box<Account<SessionTokenV2>>>) -> Result<()> {
    if signer == player {
        return Ok(());
    }
    let Some(t) = token else {
        return err!(GachaError::NotPlayerOrSession);
    };
    require!(
        t.authority == *player && t.session_signer == *signer && t.target_program == crate::ID,
        GachaError::NotPlayerOrSession
    );
    require!(Clock::get()?.unix_timestamp < t.valid_until, GachaError::SessionExpired);
    Ok(())
}

/// Requests randomness for the current `request_seq`. The seed is derived here and the
/// callback only receives Pull plus the request sequence (never odds).
fn request_randomness(ctx: &Spin, seq: u8, requested_at: i64) -> Result<()> {
    let pull_key = ctx.pull.key();
    let caller_seed = hashv(&[
        b"tequila-gacha",
        pull_key.as_ref(),
        &[seq],
        &requested_at.to_le_bytes(),
    ])
    .to_bytes();
    let ix = create_request_randomness_ix_const(
        RequestRandomnessParams {
            payer: ctx.signer.key(),
            oracle_queue: ctx.oracle_queue.key(),
            callback_program_id: crate::ID,
            callback_discriminator: instruction::CallbackSpin::DISCRIMINATOR.to_vec(),
            caller_seed,
            accounts_metas: Some(vec![SerializableAccountMeta {
                pubkey: pull_key,
                is_signer: false,
                is_writable: true,
            }]),
            callback_args: Some(vec![seq]),
        },
        PROGRAM_IDENTITY,
    );
    ctx.invoke_signed_vrf(&ctx.signer.to_account_info(), &ix)?;
    Ok(())
}

/// The claim, shared by `claim` (any signer pays) and `claim_action` (the action escrow pays).
#[allow(clippy::too_many_arguments)]
fn claim_core<'info>(
    payer: &AccountInfo<'info>,
    player: &AccountInfo<'info>,
    machine: &mut Machine,
    player_state: &mut PlayerState,
    pull: &mut Account<'info, Pull>,
    asset: &AccountInfo<'info>,
    collection: &AccountInfo<'info>,
    core_authority: &AccountInfo<'info>,
    core_program: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    asset_bump: u8,
    authority_bump: u8,
) -> Result<()> {
    let pull_key = pull.key();
    let m = machine;
    let p = pull;
    require!(p.status == PullStatus::Fulfilled, GachaError::NotFulfilled);
    let i = p.claimed;
    let idx = i as usize;

    // Re-roll this index over current stock only if its tier ran out.
    let mut t = p.tiers[idx] as usize;
    if t >= m.tier_count as usize || m.tiers[t].cap == m.tiers[t].assigned {
        t = draw(&remaining(m), indexed_roll(&p.randomness, REROLL, i)).ok_or(GachaError::SoldOut)? as usize;
        p.rerolled |= 1 << i;
        p.tiers[idx] = t as u8;
    }
    let tier = &mut m.tiers[t];
    tier.assigned += 1;
    let allocation = tier.assigned;
    let name = format!("{} #{}", trimmed(&tier.name), allocation);
    let uri = trimmed(&tier.uri);
    m.pending = m.pending.checked_sub(1).ok_or(GachaError::MathOverflow)?;
    p.allocations[idx] = allocation;
    p.claimed = i + 1;
    if p.claimed == p.count {
        p.status = PullStatus::Claimed;
        player_state.live_pull = None;
    }

    // Anyone can send lamports to the predictable asset PDA, which would make Core's
    // create_account fail forever. Sweep them to the player first.
    let asset_seeds: &[&[u8]] = &[ASSET_SEED, pull_key.as_ref(), &[i], &[asset_bump]];
    let prefunded = asset.lamports();
    if prefunded > 0 {
        system_program::transfer(
            CpiContext::new_with_signer(
                system_program.key(),
                system_program::Transfer { from: asset.clone(), to: player.clone() },
                &[asset_seeds],
            ),
            prefunded,
        )?;
    }

    core_cpi::create_asset(
        asset,
        collection,
        core_authority,
        payer,
        player,
        system_program,
        core_program,
        &name,
        &uri,
        &[&[AUTHORITY_SEED, &[authority_bump]], asset_seeds],
    )?;
    Ok(())
}

/// Reads (mint, owner) of a classic SPL token account.
fn token_account_info(a: &AccountInfo) -> Result<(Pubkey, Pubkey)> {
    require_keys_eq!(*a.owner, TOKEN_PROGRAM_ID, GachaError::InvalidTokenAccount);
    let d = a.try_borrow_data()?;
    require!(d.len() == 165, GachaError::InvalidTokenAccount);
    let mint = Pubkey::new_from_array(d[0..32].try_into().unwrap());
    let owner = Pubkey::new_from_array(d[32..64].try_into().unwrap());
    Ok((mint, owner))
}

#[ephemeral]
#[program]
pub mod tequila_gacha {
    use super::*;

    // ---------- admin ----------

    #[allow(clippy::too_many_arguments)]
    pub fn init_machine(
        ctx: Context<InitMachine>,
        machine_id: u32,
        seller: Pubkey,
        usdc_mint: Pubkey,
        price_lamports: u64,
        price_usdc: u64,
        er_validator: Pubkey,
        collection_name: String,
        collection_uri: String,
    ) -> Result<()> {
        require_keys_neq!(er_validator, Pubkey::default(), GachaError::InvalidValidator);
        let machine_key = ctx.accounts.machine.key();
        let m = &mut ctx.accounts.machine;
        m.authority = ctx.accounts.authority.key();
        m.seller = seller;
        m.collection = ctx.accounts.collection.key();
        m.price_lamports = price_lamports;
        m.usdc_mint = usdc_mint;
        m.price_usdc = price_usdc;
        m.status = MachineStatus::Draft;
        m.er_validator = er_validator;
        m.pending = 0;
        m.tier_count = 0;
        m.tiers = vec![Tier::default(); MAX_TIERS];
        m.machine_id = machine_id;
        m.bump = ctx.bumps.machine;

        core_cpi::create_collection(
            &ctx.accounts.collection,
            &ctx.accounts.core_authority,
            &ctx.accounts.authority,
            &ctx.accounts.system_program,
            &ctx.accounts.mpl_core_program,
            &collection_name,
            &collection_uri,
            &[&[COLLECTION_SEED, machine_key.as_ref(), &[ctx.bumps.collection]]],
        )?;
        Ok(())
    }

    /// Writes tiers `start..start + tiers.len()` and sets `tier_count` to that end. Draft only.
    pub fn set_tiers(ctx: Context<Admin>, start: u8, tiers: Vec<TierInput>) -> Result<()> {
        let m = &mut ctx.accounts.machine;
        require!(m.status == MachineStatus::Draft, GachaError::TiersLocked);
        let start = start as usize;
        let end = start + tiers.len();
        require!(!tiers.is_empty() && end <= MAX_TIERS, GachaError::InvalidTiers);
        for (i, t) in tiers.iter().enumerate() {
            require!(t.cap > 0, GachaError::InvalidTiers);
            m.tiers[start + i] = Tier {
                cap: t.cap,
                assigned: 0,
                name: fixed::<NAME_LEN>(&t.name)?,
                uri: fixed::<URI_LEN>(&t.uri)?,
            };
        }
        for i in end..MAX_TIERS {
            m.tiers[i] = Tier::default();
        }
        m.tier_count = end as u8;
        Ok(())
    }

    pub fn open(ctx: Context<Admin>) -> Result<()> {
        let m = &mut ctx.accounts.machine;
        require!(m.status != MachineStatus::Open, GachaError::WrongMachineState);
        require!(m.tier_count > 0, GachaError::NoTiers);
        m.status = MachineStatus::Open;
        Ok(())
    }

    pub fn close(ctx: Context<Admin>) -> Result<()> {
        let m = &mut ctx.accounts.machine;
        require!(m.status == MachineStatus::Open, GachaError::WrongMachineState);
        m.status = MachineStatus::Closed;
        Ok(())
    }

    pub fn set_prices(ctx: Context<Admin>, price_lamports: u64, price_usdc: u64) -> Result<()> {
        let m = &mut ctx.accounts.machine;
        m.price_lamports = price_lamports;
        m.price_usdc = price_usdc;
        Ok(())
    }

    pub fn set_er_validator(ctx: Context<Admin>, er_validator: Pubkey) -> Result<()> {
        require_keys_neq!(er_validator, Pubkey::default(), GachaError::InvalidValidator);
        ctx.accounts.machine.er_validator = er_validator;
        Ok(())
    }

    // ---------- player ----------

    /// A pack of `count` (1, 3 or 5) for count × price, paid to the seller directly. Reserves
    /// `count` units and freezes the odds into the Pull. Refuses if the total exceeds
    /// `max_price_total` (the price the player agreed to).
    pub fn buy_pull(ctx: Context<BuyPull>, currency: Currency, count: u8, max_price_total: u64) -> Result<()> {
        let a = &ctx.accounts;
        let m = &a.machine;
        require!(PACK_COUNTS.contains(&count), GachaError::InvalidCount);
        require!(m.status == MachineStatus::Open, GachaError::MachineNotOpen);
        require!(a.player_state.live_pull.is_none(), GachaError::LivePullExists);
        let snapshot = remaining(m);
        require!(sellable(&snapshot, m.pending, count), GachaError::SoldOut);

        let unit = match currency {
            Currency::Sol => m.price_lamports,
            Currency::Usdc => m.price_usdc,
        };
        require!(unit > 0, GachaError::CurrencyNotAccepted);
        let price = unit.checked_mul(count as u64).ok_or(GachaError::MathOverflow)?;
        require!(price <= max_price_total, GachaError::PriceAboveMax);

        match currency {
            Currency::Sol => {
                system_program::transfer(
                    CpiContext::new(
                        a.system_program.key(),
                        system_program::Transfer {
                            from: a.player.to_account_info(),
                            to: a.seller.to_account_info(),
                        },
                    ),
                    price,
                )?;
            }
            Currency::Usdc => {
                let (Some(src), Some(dst), Some(mint), Some(tp)) =
                    (&a.player_usdc, &a.seller_usdc, &a.usdc_mint, &a.token_program)
                else {
                    return err!(GachaError::MissingTokenAccounts);
                };
                require_keys_eq!(mint.key(), m.usdc_mint, GachaError::InvalidTokenAccount);
                require_keys_eq!(tp.key(), TOKEN_PROGRAM_ID, GachaError::InvalidTokenAccount);
                require_keys_eq!(*mint.owner, TOKEN_PROGRAM_ID, GachaError::InvalidTokenAccount);
                let (dst_mint, dst_owner) = token_account_info(dst)?;
                require_keys_eq!(dst_mint, m.usdc_mint, GachaError::InvalidTokenAccount);
                require_keys_eq!(dst_owner, m.seller, GachaError::InvalidTokenAccount);
                let decimals = {
                    let d = mint.try_borrow_data()?;
                    require!(d.len() == 82, GachaError::InvalidTokenAccount);
                    d[44]
                };
                // TransferChecked: the token program checks the source mint and the authority.
                let mut data = vec![12u8];
                data.extend_from_slice(&price.to_le_bytes());
                data.push(decimals);
                invoke(
                    &Instruction {
                        program_id: TOKEN_PROGRAM_ID,
                        accounts: vec![
                            AccountMeta::new(src.key(), false),
                            AccountMeta::new_readonly(mint.key(), false),
                            AccountMeta::new(dst.key(), false),
                            AccountMeta::new_readonly(a.player.key(), true),
                        ],
                        data,
                    },
                    &[
                        src.to_account_info(),
                        mint.to_account_info(),
                        dst.to_account_info(),
                        a.player.to_account_info(),
                    ],
                )?;
            }
        }

        let now = Clock::get()?.unix_timestamp;
        let pull_key = ctx.accounts.pull.key();
        let machine_key = ctx.accounts.machine.key();
        let player_key = ctx.accounts.player.key();

        let ps = &mut ctx.accounts.player_state;
        let nonce = ps.next_nonce;
        ps.next_nonce = nonce.checked_add(1).ok_or(GachaError::MathOverflow)?;
        ps.live_pull = Some(pull_key);
        ps.bump = ctx.bumps.player_state;

        ctx.accounts.machine.pending =
            ctx.accounts.machine.pending.checked_add(count as u32).ok_or(GachaError::MathOverflow)?;

        let p = &mut ctx.accounts.pull;
        p.machine = machine_key;
        p.player = player_key;
        p.nonce = nonce;
        p.status = PullStatus::Paid;
        p.snapshot = snapshot;
        p.request_seq = 0;
        p.paid_with = currency;
        p.price = price;
        p.paid_at = now;
        p.requested_at = 0;
        p.fulfilled_at = 0;
        p.randomness = [0u8; 32];
        p.count = count;
        p.claimed = 0;
        p.tiers = [logic::NO_PICK; MAX_PACK];
        p.rerolled = 0;
        p.allocations = [0; MAX_PACK];
        p.bump = ctx.bumps.pull;
        Ok(())
    }

    /// Base layer. Delegates the live Pull, only while Paid, to the ER validator stored in Machine.
    pub fn delegate_pull(ctx: Context<DelegatePull>) -> Result<()> {
        let status = Pull::try_deserialize(&mut &ctx.accounts.pull.try_borrow_data()?[..])?.status;
        require!(status == PullStatus::Paid, GachaError::WrongPullState);
        let machine = ctx.accounts.machine.key();
        let player = ctx.accounts.player.key();
        let nonce = ctx.accounts.player_state.next_nonce - 1;
        let v = ctx.accounts.machine.er_validator;
        ctx.accounts.delegate_pull(
            &ctx.accounts.player,
            &[PULL_SEED, machine.as_ref(), player.as_ref(), &nonce.to_le_bytes()],
            DelegateConfig {
                validator: if v == Pubkey::default() { None } else { Some(v) },
                ..Default::default()
            },
        )?;
        Ok(())
    }

    /// Base (DEFAULT_QUEUE) or ER (DEFAULT_EPHEMERAL_QUEUE). Paid -> Requested.
    pub fn spin(ctx: Context<Spin>) -> Result<()> {
        let a = &ctx.accounts;
        authorize(&a.signer.key(), &a.pull.player, &a.session_token)?;
        require!(ctx.accounts.pull.status == PullStatus::Paid, GachaError::WrongPullState);
        let now = Clock::get()?.unix_timestamp;
        let p = &mut ctx.accounts.pull;
        p.status = PullStatus::Requested;
        p.requested_at = now;
        let seq = p.request_seq;
        request_randomness(&ctx.accounts, seq, now)
    }

    /// New request for a Requested pull after 130 s on the clock of the layer where Pull
    /// lives. Bumps `request_seq`, so the old request's callback is rejected as stale.
    pub fn respin(ctx: Context<Spin>) -> Result<()> {
        let a = &ctx.accounts;
        authorize(&a.signer.key(), &a.pull.player, &a.session_token)?;
        require!(ctx.accounts.pull.status == PullStatus::Requested, GachaError::WrongPullState);
        let now = Clock::get()?.unix_timestamp;
        let p = &mut ctx.accounts.pull;
        require!(now - p.requested_at >= RESPIN_AFTER_SECS, GachaError::RespinTooEarly);
        p.request_seq = p.request_seq.wrapping_add(1);
        p.requested_at = now;
        let seq = p.request_seq;
        request_randomness(&ctx.accounts, seq, now)
    }

    /// VRF oracle. Writes only Pull: `count` tiers drawn without replacement from a copy of the
    /// snapshot, pick i with sha256(randomness ‖ "pick" ‖ i).
    pub fn callback_spin(ctx: Context<CallbackSpin>, randomness: [u8; 32], request_seq: u8) -> Result<()> {
        let p = &mut ctx.accounts.pull;
        require!(request_seq == p.request_seq, GachaError::StaleCallback);
        require!(p.status == PullStatus::Requested, GachaError::WrongPullState);
        p.tiers = draw_pack(&p.snapshot, &randomness, p.count).ok_or(GachaError::SoldOut)?;
        p.randomness = randomness;
        p.fulfilled_at = Clock::get()?.unix_timestamp;
        p.status = PullStatus::Fulfilled;
        Ok(())
    }

    /// ER. Commit + undelegate Pull, signed by the player: Paid or Fulfilled any time,
    /// Requested only 130 s after the request (so a pending callback is not cut off).
    pub fn settle_pull(ctx: Context<SettlePull>) -> Result<()> {
        let a = &ctx.accounts;
        authorize(&a.signer.key(), &a.pull.player, &a.session_token)?;
        let p = &ctx.accounts.pull;
        if p.status == PullStatus::Requested {
            let now = Clock::get()?.unix_timestamp;
            require!(now - p.requested_at >= RESPIN_AFTER_SECS, GachaError::SettleTooEarly);
        }
        ctx.accounts.pull.exit(&crate::ID)?;
        MagicIntentBundleBuilder::new(
            ctx.accounts.signer.to_account_info(),
            ctx.accounts.magic_context.to_account_info(),
            ctx.accounts.magic_program.to_account_info(),
        )
        .commit_and_undelegate(&[ctx.accounts.pull.to_account_info()])
        .build_and_invoke()?;
        Ok(())
    }

    /// Base, permissionless: any signer pays rent and fee. Re-rolls over current stock with
    /// randomness[8..16] only if the drawn tier is exhausted, assigns the unit and mints a
    /// transferable Core asset to `pull.player`.
    pub fn claim(ctx: Context<Claim>) -> Result<()> {
        let a = ctx.accounts;
        claim_core(
            &a.payer.to_account_info(),
            &a.player.to_account_info(),
            &mut a.machine,
            &mut a.player_state,
            &mut a.pull,
            &a.asset.to_account_info(),
            &a.collection.to_account_info(),
            &a.core_authority.to_account_info(),
            &a.mpl_core_program.to_account_info(),
            &a.system_program.to_account_info(),
            ctx.bumps.asset,
            ctx.bumps.core_authority,
        )
    }

    /// ER. Fulfilled only: commit + undelegate Pull and schedule `claim_action` as a
    /// post-undelegate base action (Pull is the program's again when it runs). The signer's
    /// DLP escrow ["balance", signer, 255] pays the asset rent; if the action fails, Pull is
    /// back on base Fulfilled and `claim` still works.
    pub fn settle_and_claim(ctx: Context<SettleAndClaim>) -> Result<()> {
        let a = &ctx.accounts;
        authorize(&a.signer.key(), &a.pull.player, &a.session_token)?;
        require!(a.pull.status == PullStatus::Fulfilled, GachaError::NotFulfilled);
        let pull_key = a.pull.key();
        let player = a.pull.player;
        let machine = a.machine.key();
        let pda = |seeds: &[&[u8]]| Pubkey::find_program_address(seeds, &crate::ID).0;
        let meta = |k: Pubkey, w: bool| ShortAccountMeta { pubkey: k.to_bytes().into(), is_writable: w };
        let player_state = pda(&[PLAYER_SEED, machine.as_ref(), player.as_ref()]);
        let core_authority = pda(&[AUTHORITY_SEED]);
        // One claim_action per unclaimed index, in cursor order.
        let actions: Vec<CallHandler> = (a.pull.claimed..a.pull.count)
            .map(|i| CallHandler {
                args: ActionArgs::new(anchor_lang::InstructionData::data(&crate::instruction::ClaimAction {})),
                compute_units: CLAIM_ACTION_CU,
                escrow_authority: a.signer.to_account_info(),
                destination_program: crate::ID.to_bytes().into(),
                accounts: vec![
                    meta(player, true),
                    meta(machine, true),
                    meta(player_state, true),
                    meta(pull_key, true),
                    meta(pda(&[ASSET_SEED, pull_key.as_ref(), &[i]]), true),
                    meta(a.machine.collection, true),
                    meta(core_authority, false),
                    meta(MPL_CORE_ID, false),
                    meta(system_program::ID, false),
                ],
            })
            .collect();
        ctx.accounts.pull.exit(&crate::ID)?;
        MagicIntentBundleBuilder::new(
            ctx.accounts.signer.to_account_info(),
            ctx.accounts.magic_context.to_account_info(),
            ctx.accounts.magic_program.to_account_info(),
        )
        .commit_and_undelegate(&[ctx.accounts.pull.to_account_info()])
        .add_post_undelegate_actions(actions)
        .build_and_invoke()?;
        Ok(())
    }

    /// Base, only through the delegation program (the escrow PDA must sign): the claim of
    /// `settle_and_claim`, paid by the escrow.
    pub fn claim_action(ctx: Context<ClaimAction>) -> Result<()> {
        let a = ctx.accounts;
        claim_core(
            &a.escrow.to_account_info(),
            &a.player.to_account_info(),
            &mut a.machine,
            &mut a.player_state,
            &mut a.pull,
            &a.asset.to_account_info(),
            &a.collection.to_account_info(),
            &a.core_authority.to_account_info(),
            &a.mpl_core_program.to_account_info(),
            &a.system_program.to_account_info(),
            ctx.bumps.asset,
            ctx.bumps.core_authority,
        )
    }

    /// Base. The current owner of the pack's asset `index` burns it and gets a Redemption record
    /// with that index's tier and number.
    pub fn redeem(ctx: Context<Redeem>, index: u8) -> Result<()> {
        let pull = &ctx.accounts.pull;
        require!(index < pull.claimed, GachaError::NotClaimed);
        {
            let d = ctx.accounts.asset.try_borrow_data()?;
            // AssetV1: key (1) | owner (32). Rejects burn delegates and previous owners.
            require!(
                *ctx.accounts.asset.owner == MPL_CORE_ID
                    && d.len() >= 33
                    && d[0] == core_cpi::KEY_ASSET_V1
                    && d[1..33] == ctx.accounts.owner.key().to_bytes(),
                GachaError::NotAssetOwner
            );
        }
        core_cpi::burn_asset(
            &ctx.accounts.asset,
            &ctx.accounts.collection,
            &ctx.accounts.owner,
            &ctx.accounts.system_program,
            &ctx.accounts.mpl_core_program,
        )?;

        let r = &mut ctx.accounts.redemption;
        r.asset = ctx.accounts.asset.key();
        r.redeemer = ctx.accounts.owner.key();
        r.machine = ctx.accounts.machine.key();
        r.tier = pull.tiers[index as usize];
        r.allocation = pull.allocations[index as usize];
        r.redeemed_at = Clock::get()?.unix_timestamp;
        r.bump = ctx.bumps.redemption;
        Ok(())
    }
}

// ---------- contexts ----------

#[derive(Accounts)]
#[instruction(machine_id: u32)]
pub struct InitMachine<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(init, payer = authority, space = 8 + Machine::INIT_SPACE,
        seeds = [MACHINE_SEED, authority.key().as_ref(), &machine_id.to_le_bytes()], bump)]
    pub machine: Box<Account<'info, Machine>>,
    /// CHECK: created by Core, signed with these seeds.
    #[account(mut, seeds = [COLLECTION_SEED, machine.key().as_ref()], bump)]
    pub collection: UncheckedAccount<'info>,
    /// CHECK: program PDA, update authority of every collection.
    #[account(seeds = [AUTHORITY_SEED], bump)]
    pub core_authority: UncheckedAccount<'info>,
    /// CHECK: fixed address.
    #[account(address = MPL_CORE_ID)]
    pub mpl_core_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Admin<'info> {
    pub authority: Signer<'info>,
    #[account(mut, has_one = authority)]
    pub machine: Box<Account<'info, Machine>>,
}

#[derive(Accounts)]
pub struct BuyPull<'info> {
    #[account(mut)]
    pub player: Signer<'info>,
    #[account(mut)]
    pub machine: Box<Account<'info, Machine>>,
    #[account(init_if_needed, payer = player, space = 8 + PlayerState::INIT_SPACE,
        seeds = [PLAYER_SEED, machine.key().as_ref(), player.key().as_ref()], bump)]
    pub player_state: Box<Account<'info, PlayerState>>,
    #[account(init, payer = player, space = 8 + Pull::INIT_SPACE,
        seeds = [PULL_SEED, machine.key().as_ref(), player.key().as_ref(),
            &player_state.next_nonce.to_le_bytes()], bump)]
    pub pull: Box<Account<'info, Pull>>,
    /// CHECK: receives SOL directly; must be the machine's seller.
    #[account(mut, address = machine.seller)]
    pub seller: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    /// CHECK: player's USDC token account (USDC only); the token program checks it.
    #[account(mut)]
    pub player_usdc: Option<UncheckedAccount<'info>>,
    /// CHECK: seller's USDC token account (USDC only); checked in the handler.
    #[account(mut)]
    pub seller_usdc: Option<UncheckedAccount<'info>>,
    /// CHECK: must be machine.usdc_mint (checked in the handler).
    pub usdc_mint: Option<UncheckedAccount<'info>>,
    /// CHECK: must be SPL Token (checked in the handler).
    pub token_program: Option<UncheckedAccount<'info>>,
}

#[delegate]
#[derive(Accounts)]
pub struct DelegatePull<'info> {
    #[account(mut)]
    pub player: Signer<'info>,
    pub machine: Box<Account<'info, Machine>>,
    #[account(seeds = [PLAYER_SEED, machine.key().as_ref(), player.key().as_ref()],
        bump = player_state.bump,
        constraint = player_state.live_pull == Some(pull.key()) @ GachaError::WrongPullState)]
    pub player_state: Box<Account<'info, PlayerState>>,
    /// CHECK: the player's live Pull; seeds re-derived in the handler for the CPI.
    #[account(mut, del)]
    pub pull: UncheckedAccount<'info>,
}

/// `signer`: the pull's player, or a session key with `session_token` (see `authorize`).
#[vrf]
#[derive(Accounts)]
pub struct Spin<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,
    #[account(mut)]
    pub pull: Account<'info, Pull>,
    /// CHECK: base queue on the base layer, delegated queue inside the ER.
    #[account(mut, constraint = oracle_queue.key() == vrf::consts::DEFAULT_QUEUE
        || oracle_queue.key() == vrf::consts::DEFAULT_EPHEMERAL_QUEUE @ GachaError::WrongQueue)]
    pub oracle_queue: UncheckedAccount<'info>,
    pub session_token: Option<Box<Account<'info, SessionTokenV2>>>,
}

#[vrf_callback]
#[derive(Accounts)]
pub struct CallbackSpin<'info> {
    #[account(address = SCOPED_VRF_IDENTITY)]
    pub vrf_program_identity: Signer<'info>,
    #[account(mut)]
    pub pull: Account<'info, Pull>,
}

#[commit]
#[derive(Accounts)]
pub struct SettlePull<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,
    #[account(mut)]
    pub pull: Account<'info, Pull>,
    pub session_token: Option<Box<Account<'info, SessionTokenV2>>>,
}

#[derive(Accounts)]
pub struct Claim<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK: the pull's player; receives the asset (and any lamports swept from its PDA).
    #[account(mut)]
    pub player: UncheckedAccount<'info>,
    #[account(mut)]
    pub machine: Box<Account<'info, Machine>>,
    #[account(mut, seeds = [PLAYER_SEED, machine.key().as_ref(), player.key().as_ref()],
        bump = player_state.bump)]
    pub player_state: Box<Account<'info, PlayerState>>,
    #[account(mut, has_one = player, has_one = machine)]
    pub pull: Box<Account<'info, Pull>>,
    /// CHECK: PDA [asset, pull, claimed]: the pack's next asset; created by mpl-core.
    #[account(mut, seeds = [ASSET_SEED, pull.key().as_ref(), &[pull.claimed]], bump)]
    pub asset: UncheckedAccount<'info>,
    /// CHECK: the machine's collection; mpl-core validates it.
    #[account(mut, address = machine.collection)]
    pub collection: UncheckedAccount<'info>,
    /// CHECK: program PDA, collection update authority.
    #[account(seeds = [AUTHORITY_SEED], bump)]
    pub core_authority: UncheckedAccount<'info>,
    /// CHECK: fixed address.
    #[account(address = MPL_CORE_ID)]
    pub mpl_core_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[commit]
#[derive(Accounts)]
pub struct SettleAndClaim<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,
    #[account(mut)]
    pub pull: Account<'info, Pull>,
    pub session_token: Option<Box<Account<'info, SessionTokenV2>>>,
    #[account(address = pull.machine)]
    pub machine: Box<Account<'info, Machine>>,
    /// CHECK: destination program of the post-undelegate action.
    #[account(address = crate::ID)]
    pub program: UncheckedAccount<'info>,
}

#[action]
#[derive(Accounts)]
pub struct ClaimAction<'info> {
    /// CHECK: the pull's player; receives the asset.
    #[account(mut)]
    pub player: UncheckedAccount<'info>,
    #[account(mut)]
    pub machine: Box<Account<'info, Machine>>,
    #[account(mut, seeds = [PLAYER_SEED, machine.key().as_ref(), player.key().as_ref()],
        bump = player_state.bump)]
    pub player_state: Box<Account<'info, PlayerState>>,
    #[account(mut, has_one = player, has_one = machine)]
    pub pull: Box<Account<'info, Pull>>,
    /// CHECK: PDA [asset, pull, claimed]: the pack's next asset; created by mpl-core.
    #[account(mut, seeds = [ASSET_SEED, pull.key().as_ref(), &[pull.claimed]], bump)]
    pub asset: UncheckedAccount<'info>,
    /// CHECK: the machine's collection; mpl-core validates it.
    #[account(mut, address = machine.collection)]
    pub collection: UncheckedAccount<'info>,
    /// CHECK: program PDA, collection update authority.
    #[account(seeds = [AUTHORITY_SEED], bump)]
    pub core_authority: UncheckedAccount<'info>,
    /// CHECK: fixed address.
    #[account(address = MPL_CORE_ID)]
    pub mpl_core_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    /// CHECK: program that scheduled the action.
    #[account(address = crate::ID @ GachaError::NotEscrowSigner)]
    pub source_program: UncheckedAccount<'info>,
    /// CHECK: payer identity the action was scheduled with (the settle signer).
    pub escrow_auth: UncheckedAccount<'info>,
    /// CHECK: only the delegation program can sign for this PDA, so `signer` proves the call
    /// came through the post-undelegate action; it pays the asset rent.
    #[account(mut, signer @ GachaError::NotEscrowSigner,
        address = ephemeral_rollups_sdk::pda::ephemeral_balance_pda_from_payer(
            &escrow_auth.key().to_bytes().into(), ACTION_ESCROW_INDEX).to_bytes().into()
            @ GachaError::NotEscrowSigner)]
    pub escrow: UncheckedAccount<'info>,
}

#[derive(Accounts)]
#[instruction(index: u8)]
pub struct Redeem<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    pub machine: Box<Account<'info, Machine>>,
    #[account(has_one = machine)]
    pub pull: Box<Account<'info, Pull>>,
    /// CHECK: the pull's asset `index`; ownership checked in the handler, burn by mpl-core.
    #[account(mut, seeds = [ASSET_SEED, pull.key().as_ref(), &[index]], bump)]
    pub asset: UncheckedAccount<'info>,
    /// CHECK: the machine's collection.
    #[account(mut, address = machine.collection)]
    pub collection: UncheckedAccount<'info>,
    #[account(init, payer = owner, space = 8 + Redemption::INIT_SPACE,
        seeds = [REDEMPTION_SEED, asset.key().as_ref()], bump)]
    pub redemption: Box<Account<'info, Redemption>>,
    /// CHECK: fixed address.
    #[account(address = MPL_CORE_ID)]
    pub mpl_core_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

// ---------- state ----------

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Debug)]
pub enum MachineStatus {
    Draft,
    Open,
    Closed,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Debug)]
pub enum PullStatus {
    Paid,
    Requested,
    Fulfilled,
    Claimed,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Debug)]
pub enum Currency {
    Sol,
    Usdc,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug)]
pub struct Tier {
    pub cap: u32,
    pub assigned: u32,
    pub name: [u8; NAME_LEN],
    pub uri: [u8; URI_LEN],
}

impl Default for Tier {
    fn default() -> Self {
        Tier { cap: 0, assigned: 0, name: [0; NAME_LEN], uri: [0; URI_LEN] }
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct TierInput {
    pub cap: u32,
    pub name: String,
    pub uri: String,
}

#[account]
#[derive(InitSpace)]
pub struct Machine {
    pub authority: Pubkey,
    pub seller: Pubkey,
    pub collection: Pubkey,
    pub price_lamports: u64,
    pub usdc_mint: Pubkey,
    pub price_usdc: u64,
    pub status: MachineStatus,
    pub er_validator: Pubkey,
    pub pending: u32,
    pub tier_count: u8,
    #[max_len(6)]
    pub tiers: Vec<Tier>,
    pub machine_id: u32,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct PlayerState {
    pub next_nonce: u64,
    pub live_pull: Option<Pubkey>,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct Pull {
    pub machine: Pubkey,
    pub player: Pubkey,
    pub nonce: u64,
    pub status: PullStatus,
    pub snapshot: [u32; MAX_TIERS],
    pub request_seq: u8,
    pub paid_with: Currency,
    pub price: u64,
    pub paid_at: i64,
    pub requested_at: i64,
    pub fulfilled_at: i64,
    pub randomness: [u8; 32],
    /// Pack size: 1, 3 or 5.
    pub count: u8,
    /// Claim cursor: the next index to mint (assets ["asset", pull, index]).
    pub claimed: u8,
    /// Tier per index (from the callback; changed in claim only if re-rolled).
    pub tiers: [u8; MAX_PACK],
    /// Bit i set if index i was re-rolled at claim.
    pub rerolled: u8,
    /// Number within its tier per claimed index.
    pub allocations: [u32; MAX_PACK],
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct Redemption {
    pub asset: Pubkey,
    pub redeemer: Pubkey,
    pub machine: Pubkey,
    pub tier: u8,
    pub allocation: u32,
    pub redeemed_at: i64,
    pub bump: u8,
}

#[error_code]
pub enum GachaError {
    #[msg("Sold out: no unreserved units left")]
    SoldOut,
    #[msg("Machine is not open")]
    MachineNotOpen,
    #[msg("Player already has a live pull")]
    LivePullExists,
    #[msg("Pull is not in the required state")]
    WrongPullState,
    #[msg("Respin allowed 130 s after the request")]
    RespinTooEarly,
    #[msg("Callback for a superseded request")]
    StaleCallback,
    #[msg("Currency not accepted (price is 0)")]
    CurrencyNotAccepted,
    #[msg("Tiers can only be set in Draft")]
    TiersLocked,
    #[msg("Pull is not Fulfilled")]
    NotFulfilled,
    #[msg("Oracle queue not allowed")]
    WrongQueue,
    #[msg("Invalid tiers")]
    InvalidTiers,
    #[msg("Machine has no tiers")]
    NoTiers,
    #[msg("Machine is not in the required state")]
    WrongMachineState,
    #[msg("USDC payment needs token accounts, mint and token program")]
    MissingTokenAccounts,
    #[msg("Invalid token account or mint")]
    InvalidTokenAccount,
    #[msg("Signer is not the asset's owner")]
    NotAssetOwner,
    #[msg("Pull is not Claimed")]
    NotClaimed,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("Price is above the buyer's max_price")]
    PriceAboveMax,
    #[msg("ER validator cannot be the default pubkey")]
    InvalidValidator,
    #[msg("Settle of a Requested pull allowed 130 s after the request")]
    SettleTooEarly,
    #[msg("Signer is neither the pull's player nor a session key of the player for this program")]
    NotPlayerOrSession,
    #[msg("Session token has expired")]
    SessionExpired,
    #[msg("claim_action runs only as the post-undelegate action (escrow must sign)")]
    NotEscrowSigner,
    #[msg("Pack size must be 1, 3 or 5")]
    InvalidCount,
}
