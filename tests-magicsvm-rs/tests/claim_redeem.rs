//! claim (mint, reroll, order independence) and redeem (burn by current owner).
mod common;
use common::*;
use solana_signer::Signer;

fn asset_owner(env: &Env, asset: &solana_address::Address) -> [u8; 32] {
    let a = env.svm.get_account(asset).unwrap();
    assert_eq!(a.owner, CORE_ID);
    assert_eq!(a.data[0], 1, "AssetV1");
    a.data[1..33].try_into().unwrap()
}

#[test]
fn claim_mints_transferable_core_asset_at_pull_pda() {
    let mut env = open_env(&[tier(3, "blanco"), tier(2, "reposado")]);
    let p = env.player();
    let pull = env.fulfilled_pull(&p, rand_picking(&[3, 2], &[1]));
    let cu = env.claim(&p, &pull).unwrap();
    println!("claim CU: {cu}");
    let pl = env.pull(&pull);
    let asset = asset_pda(&pull);
    assert_eq!(pl.status, PULL_CLAIMED);
    assert_eq!((pl.tier, pl.allocation, pl.rerolled), (1, 1, false));
    let m = env.machine();
    assert_eq!((m.pending, m.tiers[1].assigned, m.tiers[0].assigned), (0, 1, 0));
    assert_eq!(asset_owner(&env, &asset), p.pubkey().to_bytes());
    let a = env.svm.get_account(&asset).unwrap();
    // update authority = Collection(collection)
    assert_eq!(a.data[33], 2);
    assert_eq!(&a.data[34..66], collection_pda(&env.machine).as_ref());
    // name "reposado #1" and the tier uri follow (borsh strings).
    let name_len = u32::from_le_bytes(a.data[66..70].try_into().unwrap()) as usize;
    assert_eq!(std::str::from_utf8(&a.data[70..70 + name_len]).unwrap(), "reposado #1");
    let u = 70 + name_len;
    let uri_len = u32::from_le_bytes(a.data[u..u + 4].try_into().unwrap()) as usize;
    assert_eq!(std::str::from_utf8(&a.data[u + 4..u + 4 + uri_len]).unwrap(), "https://example.com/reposado.json");
    // seq: None, then nothing: no plugin header, no plugins.
    assert_eq!(a.data[u + 4 + uri_len], 0);
    assert_eq!(a.data.len(), u + 4 + uri_len + 1, "AssetV1 without plugins");
    assert_eq!(env.player_state(&p.pubkey()).live_pull, None);
    println!(
        "rent: Pull {} PlayerState {} asset {}",
        env.svm.get_balance(&pull).unwrap(),
        env.svm.get_balance(&player_state_pda(&env.machine, &p.pubkey())).unwrap(),
        a.lamports
    );
}

#[test]
fn claim_only_once_only_fulfilled_only_by_player() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let pull = env.buy_sol(&p).unwrap();
    assert_eq!(env.claim(&p, &pull), Err(custom(0, E_NOT_FULFILLED)));
    env.spin_base(&p, &pull).unwrap();
    assert_eq!(env.claim(&p, &pull), Err(custom(0, E_NOT_FULFILLED)));
    assert_eq!(env.fulfill(), vec![Ok(())]);
    // The asset can only go to the pull's player: naming anyone else as player is refused.
    let q = env.player();
    env.buy_sol(&q).unwrap(); // q has a PlayerState, so only has_one = player stands in the way
    let m = env.machine;
    assert_eq!(env.send(&[claim_ix(&q.pubkey(), &q.pubkey(), &m, &pull)], &q, &[]), Err(custom(0, 2001)));
    env.claim(&p, &pull).unwrap();
    assert_eq!(env.claim(&p, &pull), Err(custom(0, E_NOT_FULFILLED)));
}

#[test]
fn a_third_party_claims_and_the_asset_goes_to_the_player() {
    let mut env = open_env(&[tier(3, "blanco"), tier(2, "reposado")]);
    let p = env.player();
    let pull = env.fulfilled_pull(&p, rand_picking(&[3, 2], &[1]));
    let crank = env.player();
    let (p_before, c_before) = (env.svm.get_balance(&p.pubkey()).unwrap(), env.svm.get_balance(&crank.pubkey()).unwrap());
    let m = env.machine;
    env.send(&[claim_ix(&crank.pubkey(), &p.pubkey(), &m, &pull)], &crank, &[]).expect("third-party claim");
    let asset = asset_pda(&pull);
    assert_eq!(asset_owner(&env, &asset), p.pubkey().to_bytes());
    assert_eq!(env.pull(&pull).status, PULL_CLAIMED);
    assert_eq!(env.player_state(&p.pubkey()).live_pull, None);
    assert_eq!(env.machine().tiers[1].assigned, 1);
    // The crank pays rent and fee; the player pays nothing.
    assert_eq!(env.svm.get_balance(&p.pubkey()).unwrap(), p_before);
    let rent = env.svm.get_balance(&asset).unwrap();
    assert!(c_before - env.svm.get_balance(&crank.pubkey()).unwrap() >= rent);
    // The player can buy again right away.
    env.buy_sol(&p).unwrap();
}

#[test]
fn two_fulfilled_pulls_keep_their_tiers_whatever_the_claim_order() {
    for order in [[0usize, 1], [1, 0]] {
        let mut env = open_env(&[tier(2, "blanco"), tier(2, "reposado")]);
        let a = env.player();
        let b = env.player();
        let pa = env.fulfilled_pull(&a, rand_picking(&[2, 2], &[0]));
        let pb = env.fulfilled_pull(&b, rand_picking(&[2, 2], &[1]));
        let players = [(&a, pa), (&b, pb)];
        for i in order {
            let (k, pl) = players[i];
            env.claim(k, &pl).unwrap();
        }
        assert_eq!((env.pull(&pa).tier, env.pull(&pa).rerolled), (0, false));
        assert_eq!((env.pull(&pb).tier, env.pull(&pb).rerolled), (1, false));
    }
}

#[test]
fn claim_rerolls_over_current_stock_when_tier_is_exhausted() {
    // One unit of tier 0, three of tier 1. Both buyers see snapshot [1, 3]; both draw tier 0.
    let mut env = open_env(&[tier(1, "blanco"), tier(3, "reposado")]);
    let a = env.player();
    let b = env.player();
    let pa = env.fulfilled_pull(&a, rand_picking(&[1, 3], &[0]));
    let pb = env.fulfilled_pull(&b, rand_picking(&[1, 3], &[0]));
    assert_eq!((env.pull(&pa).tier, env.pull(&pb).tier), (0, 0));
    env.claim(&a, &pa).unwrap();
    env.claim(&b, &pb).unwrap();
    let la = env.pull(&pa);
    let lb = env.pull(&pb);
    assert_eq!((la.tier, la.rerolled, la.allocation), (0, false, 1));
    // Re-roll with sha256(randomness ‖ "reroll" ‖ 0) over current [0, 3]: only tier 1 is left.
    assert_eq!((lb.tier, lb.rerolled, lb.allocation), (1, true, 1));
    let m = env.machine();
    assert_eq!((m.tiers[0].assigned, m.tiers[1].assigned, m.pending), (1, 1, 0));
}

#[test]
fn claim_order_matters_only_through_the_reroll() {
    // Same setup, opposite order: now a is the one re-rolled.
    let mut env = open_env(&[tier(1, "blanco"), tier(3, "reposado")]);
    let a = env.player();
    let b = env.player();
    let pa = env.fulfilled_pull(&a, rand_picking(&[1, 3], &[0]));
    let pb = env.fulfilled_pull(&b, rand_picking(&[1, 3], &[0]));
    env.claim(&b, &pb).unwrap();
    env.claim(&a, &pa).unwrap();
    assert_eq!((env.pull(&pb).tier, env.pull(&pb).rerolled), (0, false));
    assert_eq!((env.pull(&pa).tier, env.pull(&pa).rerolled), (1, true));
}

#[test]
fn claim_still_works_after_close() {
    let mut env = open_env(&[tier(2, "blanco")]);
    let p = env.player();
    let pull = env.fulfilled_pull(&p, [9; 32]);
    let (ad, m) = (env.admin.pubkey(), env.machine);
    env.admin_send(close_ix(&ad, &m)).unwrap();
    env.claim(&p, &pull).unwrap();
}

#[test]
fn redeem_by_current_owner_after_transfer_and_refusal_for_previous_owner() {
    let mut env = open_env(&[tier(2, "blanco")]);
    let p = env.player();
    let pull = env.fulfilled_pull(&p, [9; 32]);
    env.claim(&p, &pull).unwrap();
    let asset = asset_pda(&pull);
    let col = collection_pda(&env.machine);
    let q = env.player();
    env.send(&[core_transfer_ix(asset, col, p.pubkey(), q.pubkey())], &p, &[]).unwrap();
    assert_eq!(asset_owner(&env, &asset), q.pubkey().to_bytes());

    let m = env.machine;
    assert_eq!(env.send(&[redeem_ix(&p.pubkey(), &m, &pull)], &p, &[]), Err(custom(0, E_NOT_ASSET_OWNER)));

    let cu = env.send(&[redeem_ix(&q.pubkey(), &m, &pull)], &q, &[]).unwrap();
    println!("redeem CU: {cu}");
    let r: Redemption = decode("Redemption", &env.svm.get_account(&redemption_pda(&asset)).unwrap().data);
    assert_eq!(r.asset, asset.to_bytes());
    assert_eq!(r.redeemer, q.pubkey().to_bytes());
    assert_eq!(r.machine, m.to_bytes());
    assert_eq!((r.tier, r.allocation), (env.pull(&pull).tier, 1));
    println!("rent: Redemption {}", env.svm.get_balance(&redemption_pda(&asset)).unwrap());
    // The asset is burned.
    let left = env.svm.get_account(&asset);
    assert!(left.map(|a| a.data.is_empty() || a.data[0] != 1).unwrap_or(true));
    // Twice: refused.
    assert!(env.send(&[redeem_ix(&q.pubkey(), &m, &pull)], &q, &[]).is_err());
}

#[test]
fn redeem_needs_a_claimed_pull() {
    let mut env = open_env(&[tier(2, "blanco")]);
    let p = env.player();
    let pull = env.fulfilled_pull(&p, [9; 32]);
    let m = env.machine;
    assert_eq!(env.send(&[redeem_ix(&p.pubkey(), &m, &pull)], &p, &[]), Err(custom(0, E_NOT_CLAIMED)));
}

#[test]
fn claim_works_when_someone_prefunds_the_asset_pda() {
    let mut env = open_env(&[tier(2, "blanco")]);
    let p = env.player();
    let pull = env.fulfilled_pull(&p, [9; 32]);
    let asset = asset_pda(&pull);
    let griefer = env.player();
    let ix = solana_message::Instruction {
        program_id: SYSTEM,
        accounts: vec![
            solana_message::AccountMeta::new(griefer.pubkey(), true),
            solana_message::AccountMeta::new(asset, false),
        ],
        data: {
            let mut d = 2u32.to_le_bytes().to_vec();
            d.extend(1_000_000u64.to_le_bytes());
            d
        },
    };
    env.send(&[ix], &griefer, &[]).unwrap();
    assert_eq!(env.svm.get_balance(&asset), Some(1_000_000));
    env.claim(&p, &pull).expect("claim must not be blocked by a pre-funded asset PDA");
    assert_eq!(env.pull(&pull).status, PULL_CLAIMED);
}
