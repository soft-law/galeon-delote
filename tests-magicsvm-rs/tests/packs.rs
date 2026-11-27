//! Packs: one Pull with count 1, 3 or 5, one VRF, picks without replacement from the snapshot
//! copy, claim cursor with assets at ["asset", pull, index], re-roll per index.
mod common;
use common::*;
use magicsvm::TransactionTarget::Ephemeral;
use solana_address::Address;
use solana_keypair::Keypair;
use solana_signer::Signer;

fn fulfilled_pack(env: &mut Env, p: &Keypair, count: u8, r: [u8; 32]) -> Address {
    let pull = env.buy_pack(p, count).unwrap();
    env.spin_base(p, &pull).unwrap();
    env.svm.set_vrf_randomness(Some(r));
    assert_eq!(env.fulfill(), vec![Ok(())]);
    env.svm.set_vrf_randomness(None);
    pull
}

fn asset_owner(env: &Env, asset: &Address) -> [u8; 32] {
    let a = env.svm.get_account(asset).unwrap();
    assert_eq!((a.owner, a.data[0]), (CORE_ID, 1));
    a.data[1..33].try_into().unwrap()
}

#[test]
fn buy_pack_charges_count_times_price_and_reserves_count() {
    let mut env = open_env(&[tier(4, "blanco"), tier(3, "reposado")]);
    let p = env.player();
    let seller_before = env.svm.get_balance(&env.seller.pubkey()).unwrap();
    let pull = env.buy_pack(&p, 3).unwrap();
    assert_eq!(env.svm.get_balance(&env.seller.pubkey()).unwrap() - seller_before, 3 * PRICE_LAMPORTS);
    let pl = env.pull(&pull);
    assert_eq!((pl.count, pl.claimed, pl.price, pl.snapshot), (3, 0, 3 * PRICE_LAMPORTS, [4, 3, 0, 0, 0, 0]));
    assert_eq!(env.machine().pending, 3);
}

#[test]
fn buy_pack_in_usdc() {
    let mut env = open_env(&[tier(9, "blanco")]);
    let p = env.player();
    let u = env.usdc_accounts(&p.pubkey(), 10 * PRICE_USDC);
    let (m, seller) = (env.machine, env.seller.pubkey());
    let ix = buy_pack_ix(&p.pubkey(), &m, &seller, 0, USDC, Some(&u), 5, 5 * PRICE_USDC);
    env.send(&[ix], &p, &[]).unwrap();
    assert_eq!(token_amount(&env.svm, &u.seller_ata), 5 * PRICE_USDC);
}

#[test]
fn count_must_be_1_3_or_5() {
    let mut env = open_env(&[tier(20, "blanco")]);
    let p = env.player();
    let (m, seller) = (env.machine, env.seller.pubkey());
    for count in [0u8, 2, 4, 6] {
        let ix = buy_pack_ix(&p.pubkey(), &m, &seller, 0, SOL, None, count, u64::MAX);
        assert_eq!(env.send(&[ix], &p, &[]), Err(custom(0, E_INVALID_COUNT)), "count {count}");
    }
}

#[test]
fn max_price_total_bounds_the_pack_price() {
    let mut env = open_env(&[tier(20, "blanco")]);
    let p = env.player();
    let (m, seller) = (env.machine, env.seller.pubkey());
    let ix = buy_pack_ix(&p.pubkey(), &m, &seller, 0, SOL, None, 3, 3 * PRICE_LAMPORTS - 1);
    assert_eq!(env.send(&[ix], &p, &[]), Err(custom(0, E_PRICE_ABOVE_MAX)));
    let ix = buy_pack_ix(&p.pubkey(), &m, &seller, 0, SOL, None, 3, 3 * PRICE_LAMPORTS);
    env.send(&[ix], &p, &[]).unwrap();
}

#[test]
fn stock_below_count_is_sold_out() {
    let mut env = open_env(&[tier(2, "blanco"), tier(2, "reposado")]);
    let p = env.player();
    assert_eq!(env.buy_pack(&p, 5), Err(custom(0, E_SOLD_OUT)));
    env.buy_pack(&p, 3).unwrap(); // 4 left, 3 reserved
    let q = env.player();
    assert_eq!(env.buy_pack(&q, 3), Err(custom(0, E_SOLD_OUT)));
    env.buy_sol(&q).expect("one unreserved unit is still sellable");
}

#[test]
fn callback_draws_count_tiers_without_replacement_from_the_copy() {
    let mut env = open_env(&[tier(2, "blanco"), tier(1, "reposado"), tier(4, "anejo")]);
    let p = env.player();
    let r = [42u8; 32];
    let pull = fulfilled_pack(&mut env, &p, 5, r);
    let pl = env.pull(&pull);
    assert_eq!(pl.status, PULL_FULFILLED);
    assert_eq!(pl.tiers.to_vec(), pack_tiers(&[2, 1, 4], &r, 5));
    let mut used = [0u32; 3];
    for t in pl.tiers {
        used[t as usize] += 1;
    }
    assert!(used[0] <= 2 && used[1] <= 1 && used[2] <= 4, "never more of a tier than the snapshot had");
    // The callback writes only Pull.
    assert!(env.machine().tiers.iter().all(|t| t.assigned == 0));
}

#[test]
fn pack_of_5_claims_all_indices_in_one_tx() {
    let mut env = open_env(&[tier(6, "blanco"), tier(4, "reposado")]);
    let p = env.player();
    let pull = fulfilled_pack(&mut env, &p, 5, [7u8; 32]);
    let m = env.machine;
    let ixs: Vec<_> = (0..5).map(|i| claim_index_ix(&p.pubkey(), &p.pubkey(), &m, &pull, i)).collect();
    let cu = env.send(&ixs, &p, &[]).expect("5 claims in one tx");
    println!("5 claims in one tx CU: {cu}");
    let pl = env.pull(&pull);
    assert_eq!((pl.status, pl.claimed), (PULL_CLAIMED, 5));
    for i in 0..5u8 {
        assert_eq!(asset_owner(&env, &asset_pda_i(&pull, i)), p.pubkey().to_bytes());
    }
    let mc = env.machine();
    assert_eq!(mc.pending, 0);
    assert_eq!(mc.tiers[0].assigned + mc.tiers[1].assigned, 5);
    assert_eq!(env.player_state(&p.pubkey()).live_pull, None);
    // Numbers within each tier are 1..n in claim order.
    let mut seen = [0u32; 2];
    for i in 0..5 {
        let t = pl.tiers[i] as usize;
        seen[t] += 1;
        assert_eq!(pl.allocations[i], seen[t]);
    }
    assert_eq!(env.claim(&p, &pull), Err(custom(0, E_NOT_FULFILLED)), "nothing left to claim");
}

#[test]
fn claims_of_3_in_one_tx_cu() {
    let mut env = open_env(&[tier(6, "blanco"), tier(4, "reposado")]);
    let p = env.player();
    let pull = fulfilled_pack(&mut env, &p, 3, [8u8; 32]);
    let m = env.machine;
    let ixs: Vec<_> = (0..3).map(|i| claim_index_ix(&p.pubkey(), &p.pubkey(), &m, &pull, i)).collect();
    let cu = env.send(&ixs, &p, &[]).expect("3 claims in one tx");
    println!("3 claims in one tx CU: {cu}");
    assert_eq!(env.pull(&pull).status, PULL_CLAIMED);
}

#[test]
fn a_pack_stays_live_until_every_index_is_claimed() {
    let mut env = open_env(&[tier(6, "blanco")]);
    let p = env.player();
    let pull = fulfilled_pack(&mut env, &p, 3, [1u8; 32]);
    env.claim(&p, &pull).unwrap();
    let pl = env.pull(&pull);
    assert_eq!((pl.status, pl.claimed), (PULL_FULFILLED, 1));
    assert_eq!(env.machine().pending, 2);
    assert_eq!(env.buy_sol(&p), Err(custom(0, E_LIVE_PULL_EXISTS)));
    // The cursor decides the asset: claiming index 2 out of order is refused (seeds).
    let m = env.machine;
    assert!(env.send(&[claim_index_ix(&p.pubkey(), &p.pubkey(), &m, &pull, 2)], &p, &[]).is_err());
    env.claim(&p, &pull).unwrap();
    env.claim(&p, &pull).unwrap();
    assert_eq!(env.pull(&pull).status, PULL_CLAIMED);
    env.buy_sol(&p).unwrap();
}

#[test]
fn reroll_inside_a_pack_uses_the_index_and_current_stock() {
    // Caps [1, 3]. a takes the only blanco; b's pack of 3 drew [0, 1, 1] from its copy [1, 3].
    let mut env = open_env(&[tier(1, "blanco"), tier(3, "reposado")]);
    let a = env.player();
    let b = env.player();
    let pa = env.fulfilled_pull(&a, rand_picking(&[1, 3], &[0]));
    let pb = fulfilled_pack(&mut env, &b, 3, rand_picking(&[1, 3], &[0, 1, 1]));
    env.claim(&a, &pa).unwrap();
    let m = env.machine;
    let ixs: Vec<_> = (0..3).map(|i| claim_index_ix(&b.pubkey(), &b.pubkey(), &m, &pb, i)).collect();
    env.send(&ixs, &b, &[]).unwrap();
    let pl = env.pull(&pb);
    assert_eq!(pl.tiers[..3], [1, 1, 1], "index 0 re-rolled over [0, 3]");
    assert_eq!(pl.rerolled_mask, 0b001);
    assert_eq!(pl.allocations[..3], [1, 2, 3]);
    let mc = env.machine();
    assert_eq!((mc.tiers[0].assigned, mc.tiers[1].assigned, mc.pending), (1, 3, 0));
}

#[test]
fn redeem_reads_tier_and_number_of_the_index() {
    let mut env = open_env(&[tier(6, "blanco"), tier(4, "reposado")]);
    let p = env.player();
    let pull = fulfilled_pack(&mut env, &p, 3, [5u8; 32]);
    env.claim(&p, &pull).unwrap();
    env.claim(&p, &pull).unwrap();
    let m = env.machine;
    assert_eq!(env.send(&[redeem_index_ix(&p.pubkey(), &m, &pull, 2)], &p, &[]), Err(custom(0, E_NOT_CLAIMED)));
    env.send(&[redeem_index_ix(&p.pubkey(), &m, &pull, 1)], &p, &[]).unwrap();
    let asset = asset_pda_i(&pull, 1);
    let r: Redemption = decode("Redemption", &env.svm.get_account(&redemption_pda(&asset)).unwrap().data);
    let pl = env.pull(&pull);
    assert_eq!((r.tier, r.allocation, r.asset), (pl.tiers[1], pl.allocations[1], asset.to_bytes()));
}

#[test]
fn settle_and_claim_mints_a_whole_pack_through_actions() {
    let mut env = open_env(&[tier(6, "blanco"), tier(4, "reposado")]);
    let p = env.player();
    let nonce = env.next_nonce(&p.pubkey());
    let m = env.machine;
    let pull = pull_pda(&m, &p.pubkey(), nonce);
    let top = top_up_escrow_ix(&p.pubkey(), &p.pubkey(), 20_000_000);
    let buy = buy_pack_ix(&p.pubkey(), &m, &env.seller.pubkey(), nonce, SOL, None, 3, u64::MAX);
    let del = delegate_pull_ix(&p.pubkey(), &m, &pull);
    env.send(&[top, buy, del], &p, &[]).unwrap();
    env.send_to(Ephemeral, &[spin_ix(&p.pubkey(), &pull, EPHEMERAL_QUEUE)], &p, &[]).unwrap();
    assert_eq!(env.fulfill(), vec![Ok(())]);
    // MagicSVM 0.3.0 re-delegates an account written on base while its DLP delegation PDAs exist,
    // so the 2nd action would see Pull owned by DLP. Devnet closes them on undelegation (and runs the
    // three actions in the same base tx, after ProcessUndelegation); close them here first.
    for tag in [&b"delegation"[..], &b"delegation-metadata"[..]] {
        let a = Address::find_program_address(&[tag, pull.as_ref()], &DLP).0;
        env.svm.set_account(a, solana_account::Account::default()).unwrap();
    }
    let cu = env.send_to(Ephemeral, &[settle_and_claim_ix(&p.pubkey(), &pull, &m, None)], &p, &[]).expect("settle_and_claim pack");
    println!("settle_and_claim (ER, pack of 3) CU: {cu}");
    let pl = env.pull(&pull);
    assert_eq!((pl.status, pl.claimed), (PULL_CLAIMED, 3));
    for i in 0..3u8 {
        assert_eq!(asset_owner(&env, &asset_pda_i(&pull, i)), p.pubkey().to_bytes());
    }
}
