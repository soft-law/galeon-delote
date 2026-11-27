//! spin / callback_spin / respin on the base layer.
mod common;
use common::*;
use solana_keypair::Keypair;
use solana_message::AccountMeta;
use solana_signer::Signer;

#[test]
fn spin_then_callback_picks_tier_from_snapshot() {
    let mut env = open_env(&[tier(3, "blanco"), tier(0 + 2, "reposado"), tier(5, "anejo")]);
    let p = env.player();
    let pull = env.buy_sol(&p).unwrap();
    env.spin_base(&p, &pull).unwrap();
    let pl = env.pull(&pull);
    assert_eq!(pl.status, PULL_REQUESTED);
    assert!(pl.requested_at > 0 || pl.requested_at == env.svm.get_sysvar::<solana_clock::Clock>().unix_timestamp);
    // sha256(randomness ‖ "pick" ‖ 0) mod 10 lands in tier 1 (cumulative 3, 5, 10).
    let r = rand_picking(&[3, 2, 5], &[1]);
    env.svm.set_vrf_randomness(Some(r));
    assert_eq!(env.fulfill(), vec![Ok(())]);
    let pl = env.pull(&pull);
    assert_eq!(pl.status, PULL_FULFILLED);
    assert_eq!(pl.tier, 1);
    assert_eq!(pl.randomness, r);
    // The callback writes only Pull: Machine is unchanged.
    let m = env.machine();
    assert_eq!(m.pending, 1);
    assert!(m.tiers.iter().all(|t| t.assigned == 0));
}

#[test]
fn snapshot_odds_follow_remaining_stock() {
    // Exhaust tier 0 first, then the next buyer's snapshot gives it weight zero.
    let mut env = open_env(&[tier(1, "blanco"), tier(4, "reposado")]);
    let a = env.player();
    let pa = env.fulfilled_pull(&a, rand_picking(&[1, 4], &[0]));
    assert_eq!(env.pull(&pa).tier, 0);
    env.claim(&a, &pa).unwrap();
    let b = env.player();
    // The same randomness would give tier 0 with the old odds; with snapshot [0, 4] it is tier 1.
    let pb = env.fulfilled_pull(&b, rand_picking(&[1, 4], &[0]));
    assert_eq!(env.pull(&pb).snapshot, [0, 4, 0, 0, 0, 0]);
    assert_eq!(env.pull(&pb).tier, 1);
}

#[test]
fn distribution_matches_snapshot_weights() {
    const N: u64 = 1500;
    let mut env = open_env(&[tier(5000, "a"), tier(3000, "b"), tier(2000, "c")]);
    let mut counts = [0u64; 3];
    for _ in 0..N {
        let p = env.player();
        let pull = env.buy_sol(&p).unwrap();
        env.spin_base(&p, &pull).unwrap();
        assert_eq!(env.fulfill(), vec![Ok(())]);
        counts[env.pull(&pull).tier as usize] += 1;
    }
    for (i, w) in [0.5f64, 0.3, 0.2].iter().enumerate() {
        let obs = counts[i] as f64 / N as f64;
        let sigma = (w * (1.0 - w) / N as f64).sqrt();
        assert!((obs - w).abs() <= 4.0 * sigma, "tier {i}: {obs} vs {w}");
    }
}

#[test]
fn spin_only_from_paid_and_only_by_player() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let pull = env.buy_sol(&p).unwrap();
    let other = env.player();
    assert_eq!(
        env.send(&[spin_ix(&other.pubkey(), &pull, DEFAULT_QUEUE)], &other, &[]),
        Err(custom(0, E_NOT_PLAYER_OR_SESSION)),
        "neither the player nor a session key"
    );
    env.spin_base(&p, &pull).unwrap();
    assert_eq!(env.spin_base(&p, &pull), Err(custom(0, E_WRONG_PULL_STATE)));
}

#[test]
fn spin_rejects_foreign_queue() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let pull = env.buy_sol(&p).unwrap();
    let fake = Keypair::new().pubkey();
    assert_eq!(env.send(&[spin_ix(&p.pubkey(), &pull, fake)], &p, &[]), Err(custom(0, E_WRONG_QUEUE)));
}

#[test]
fn base_spin_charges_the_player_the_vrf_fee() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let pull = env.buy_sol(&p).unwrap();
    let before = env.svm.get_balance(&p.pubkey()).unwrap();
    env.spin_base(&p, &pull).unwrap();
    let after = env.svm.get_balance(&p.pubkey()).unwrap();
    println!("base spin cost to player: {} lamports (incl. 5000 fee)", before - after);
    assert!(before - after >= 500_000);
}

#[test]
fn duplicate_callback_is_rejected() {
    let mut env = open_env(&[tier(3, "blanco"), tier(3, "reposado")]);
    let p = env.player();
    let pull = env.buy_sol(&p).unwrap();
    env.spin_base(&p, &pull).unwrap();
    let exec = magicsvm_executor();
    // First delivery by hand with the real scoped identity: tier 0.
    env.send(&[provide_randomness_ix(pull, rand_picking(&[3, 3], &[0]), 0, 1)], &exec, &[]).unwrap();
    let before = env.pull(&pull);
    assert_eq!((before.status, before.tier), (PULL_FULFILLED, 0));
    // The queued fulfillment for the same request arrives: rejected.
    env.svm.set_vrf_randomness(Some(randomness_u64s(5, 0)));
    assert_eq!(env.fulfill(), vec![Err(custom(0, E_WRONG_PULL_STATE))]);
    // A third replay: rejected.
    assert_eq!(
        env.send(&[provide_randomness_ix(pull, randomness_u64s(4, 0), 0, 2)], &exec, &[]),
        Err(custom(0, E_WRONG_PULL_STATE))
    );
    let after = env.pull(&pull);
    assert_eq!((after.tier, after.randomness, after.fulfilled_at), (before.tier, before.randomness, before.fulfilled_at));
}

#[test]
fn callback_needs_scoped_vrf_identity() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let pull = env.buy_sol(&p).unwrap();
    env.spin_base(&p, &pull).unwrap();
    let attacker = env.player();
    let ix = callback_ix(AccountMeta::new_readonly(attacker.pubkey(), true), &pull, [0; 32], 0);
    assert_eq!(env.send(&[ix], &attacker, &[]), Err(custom(0, 2012)));
    let ix = callback_ix(AccountMeta::new_readonly(scoped_vrf_identity(), false), &pull, [0; 32], 0);
    assert_eq!(env.send(&[ix], &attacker, &[]), Err(custom(0, 3010)));
    assert_eq!(env.pull(&pull).status, PULL_REQUESTED);
}

#[test]
fn respin_waits_130_seconds() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let pull = env.buy_sol(&p).unwrap();
    // Not before a request exists.
    assert_eq!(env.send(&[respin_ix(&p.pubkey(), &pull, DEFAULT_QUEUE)], &p, &[]), Err(custom(0, E_WRONG_PULL_STATE)));
    env.spin_base(&p, &pull).unwrap();
    env.advance_secs(129);
    assert_eq!(env.send(&[respin_ix(&p.pubkey(), &pull, DEFAULT_QUEUE)], &p, &[]), Err(custom(0, E_RESPIN_TOO_EARLY)));
    env.advance_secs(1);
    let before = env.pull(&pull);
    env.send(&[respin_ix(&p.pubkey(), &pull, DEFAULT_QUEUE)], &p, &[]).unwrap();
    let after = env.pull(&pull);
    assert_eq!(after.request_seq, before.request_seq + 1);
    assert_eq!(after.requested_at, before.requested_at + 130);
    assert_eq!(after.status, PULL_REQUESTED);
    // Old request is stale, new one lands. Both were queued.
    let results = env.fulfill();
    assert_eq!(results, vec![Err(custom(0, E_STALE_CALLBACK)), Ok(())]);
    assert_eq!(env.pull(&pull).status, PULL_FULFILLED);
    // No second charge: the machine still has one reservation for this pull.
    assert_eq!(env.machine().pending, 1);
}

#[test]
fn late_callback_after_respin_and_fulfillment_is_stale() {
    let mut env = open_env(&[tier(3, "blanco"), tier(3, "reposado")]);
    let p = env.player();
    let pull = env.buy_sol(&p).unwrap();
    env.spin_base(&p, &pull).unwrap();
    env.advance_secs(130);
    env.send(&[respin_ix(&p.pubkey(), &pull, DEFAULT_QUEUE)], &p, &[]).unwrap();
    let exec = magicsvm_executor();
    // New request (seq 1) fulfilled by hand first.
    env.send(&[provide_randomness_ix(pull, randomness_u64s(0, 0), 1, 10)], &exec, &[]).unwrap();
    // Late delivery for seq 0: stale.
    assert_eq!(
        env.send(&[provide_randomness_ix(pull, randomness_u64s(5, 0), 0, 11)], &exec, &[]),
        Err(custom(0, E_STALE_CALLBACK))
    );
    assert_eq!(env.pull(&pull).tier, 0);
}

#[test]
fn respin_not_after_fulfilled() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let pull = env.fulfilled_pull(&p, [3; 32]);
    env.advance_secs(500);
    assert_eq!(env.send(&[respin_ix(&p.pubkey(), &pull, DEFAULT_QUEUE)], &p, &[]), Err(custom(0, E_WRONG_PULL_STATE)));
}
