//! Ephemeral Rollup path: buy + delegate (base), spin (ER), callback (ER), settle (ER), claim (base).
mod common;
use common::*;
use magicsvm::TransactionTarget::{Base, Ephemeral};
use solana_keypair::Keypair;
use solana_signer::Signer;

fn buy_and_delegate(env: &mut Env, p: &Keypair) -> solana_address::Address {
    let nonce = env.next_nonce(&p.pubkey());
    let m = env.machine;
    let pull = pull_pda(&m, &p.pubkey(), nonce);
    let buy = buy_pull_ix(&p.pubkey(), &m, &env.seller.pubkey(), nonce, SOL, None);
    let del = delegate_pull_ix(&p.pubkey(), &m, &pull);
    let cu = env.send(&[buy, del], p, &[]).expect("buy + delegate in one base tx");
    println!("buy_pull + delegate_pull CU: {cu}");
    assert_eq!(env.svm.get_account(&pull).unwrap().owner, DLP, "delegated on base");
    pull
}

#[test]
fn full_er_flow_then_claim_on_base() {
    let mut env = open_env(&[tier(3, "blanco"), tier(2, "reposado")]);
    let p = env.player();
    let pull = buy_and_delegate(&mut env, &p);
    assert_eq!(env.pull_er(&pull).status, PULL_PAID);

    // Claim while delegated: refused (Pull is owned by the delegation program on base).
    assert!(env.claim(&p, &pull).is_err());

    let cu = env
        .send_to(Ephemeral, &[spin_ix(&p.pubkey(), &pull, EPHEMERAL_QUEUE)], &p, &[])
        .expect("spin in ER");
    println!("spin (ER) CU: {cu}");
    assert_eq!(env.pull_er(&pull).status, PULL_REQUESTED);
    env.svm.set_vrf_randomness(Some(rand_picking(&[3, 2], &[1])));
    assert_eq!(env.fulfill(), vec![Ok(())]);
    let er = env.pull_er(&pull);
    assert_eq!((er.status, er.tier), (PULL_FULFILLED, 1));

    let cu = env.settle(&p, &pull).expect("settle");
    println!("settle_pull (ER) CU: {cu}");
    let acc = env.svm.get_account(&pull).unwrap();
    assert_eq!(acc.owner, PROGRAM_ID, "undelegated on base");
    let base = env.pull(&pull);
    assert_eq!((base.status, base.tier), (PULL_FULFILLED, 1));

    env.claim(&p, &pull).unwrap();
    assert_eq!(env.pull(&pull).status, PULL_CLAIMED);
    assert_eq!(env.machine().tiers[1].assigned, 1);
}

#[test]
fn spin_in_er_rejects_base_queue_and_base_rejects_ephemeral_queue() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let pull = buy_and_delegate(&mut env, &p);
    // Base queue is not delegated: it cannot be written in the ER.
    assert!(env.send_to(Ephemeral, &[spin_ix(&p.pubkey(), &pull, DEFAULT_QUEUE)], &p, &[]).is_err());
    let q = env.player();
    let pq = env.buy_sol(&q).unwrap();
    assert!(env.send_to(Base, &[spin_ix(&q.pubkey(), &pq, EPHEMERAL_QUEUE)], &q, &[]).is_err());
}

#[test]
fn settle_from_paid_or_late_requested_and_base_respin_recovers() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    // Settle right after delegation (Paid): back on base, spin there.
    let pull = buy_and_delegate(&mut env, &p);
    env.settle(&p, &pull).expect("settle Paid");
    assert_eq!(env.pull(&pull).status, PULL_PAID);
    assert_eq!(env.svm.get_account(&pull).unwrap().owner, PROGRAM_ID);

    // Paid pull back on base can still spin on base.
    env.spin_base(&p, &pull).unwrap();
    assert_eq!(env.fulfill(), vec![Ok(())]);
    env.claim(&p, &pull).unwrap();

    // A second pull: request in the ER; the callback does not land, settle after 130 s.
    let pull = buy_and_delegate(&mut env, &p);
    env.send_to(Ephemeral, &[spin_ix(&p.pubkey(), &pull, EPHEMERAL_QUEUE)], &p, &[]).unwrap();
    env.advance_secs(130);
    env.settle(&p, &pull).expect("settle Requested after 130 s");
    assert_eq!(env.pull(&pull).status, PULL_REQUESTED);
    // The ER fulfillment now fails: Pull is no longer delegated.
    let r = env.fulfill();
    assert_eq!(r.len(), 1);
    assert!(r[0].is_err());
    assert_eq!(env.pull(&pull).status, PULL_REQUESTED);
    // 130 s after the request the player re-spins on base and gets a result.
    env.send(&[respin_ix(&p.pubkey(), &pull, DEFAULT_QUEUE)], &p, &[]).unwrap();
    assert_eq!(env.fulfill(), vec![Ok(())]);
    assert_eq!(env.pull(&pull).status, PULL_FULFILLED);
    env.claim(&p, &pull).unwrap();
}

#[test]
fn respin_inside_the_er_uses_the_er_clock() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let pull = buy_and_delegate(&mut env, &p);
    env.send_to(Ephemeral, &[spin_ix(&p.pubkey(), &pull, EPHEMERAL_QUEUE)], &p, &[]).unwrap();
    assert_eq!(
        env.send_to(Ephemeral, &[respin_ix(&p.pubkey(), &pull, EPHEMERAL_QUEUE)], &p, &[]),
        Err(custom(0, E_RESPIN_TOO_EARLY))
    );
    env.advance_secs(130);
    env.send_to(Ephemeral, &[respin_ix(&p.pubkey(), &pull, EPHEMERAL_QUEUE)], &p, &[]).unwrap();
    assert_eq!(env.pull_er(&pull).request_seq, 1);
    assert_eq!(env.fulfill(), vec![Err(custom(0, E_STALE_CALLBACK)), Ok(())]);
    assert_eq!(env.pull_er(&pull).status, PULL_FULFILLED);
}

#[test]
fn only_the_player_settles_or_delegates() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let q = env.player();
    let pull = buy_and_delegate(&mut env, &p);
    assert_eq!(env.send_to(Ephemeral, &[settle_pull_ix(&q.pubkey(), &pull)], &q, &[]), Err(custom(0, E_NOT_PLAYER_OR_SESSION)));
    // q has no live pull, so q cannot delegate p's pull.
    let r = env.buy_sol(&p);
    assert!(r.is_err());
    let m = env.machine;
    assert!(env.send(&[delegate_pull_ix(&q.pubkey(), &m, &pull)], &q, &[]).is_err());
}

#[test]
fn settle_from_requested_waits_130_seconds_fulfilled_any_time() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let pull = buy_and_delegate(&mut env, &p);
    env.send_to(Ephemeral, &[spin_ix(&p.pubkey(), &pull, EPHEMERAL_QUEUE)], &p, &[]).unwrap();
    // Settling a fresh request would pull it out from under the pending callback.
    assert_eq!(env.settle(&p, &pull), Err(custom(0, E_SETTLE_TOO_EARLY)));
    env.advance_secs(129);
    assert_eq!(env.settle(&p, &pull), Err(custom(0, E_SETTLE_TOO_EARLY)));
    assert_eq!(env.svm.get_account(&pull).unwrap().owner, DLP, "still delegated");
    // Fulfilled: settle right away.
    assert_eq!(env.fulfill(), vec![Ok(())]);
    env.settle(&p, &pull).expect("settle Fulfilled");
    assert_eq!(env.pull(&pull).status, PULL_FULFILLED);
}

#[test]
fn delegate_only_from_paid() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let m = env.machine;
    // Requested on base (base queue): delegating would strand the base callback.
    let pull = env.buy_sol(&p).unwrap();
    env.spin_base(&p, &pull).unwrap();
    assert_eq!(env.send(&[delegate_pull_ix(&p.pubkey(), &m, &pull)], &p, &[]), Err(custom(0, E_WRONG_PULL_STATE)));
    // Fulfilled on base: same.
    assert_eq!(env.fulfill(), vec![Ok(())]);
    assert_eq!(env.send(&[delegate_pull_ix(&p.pubkey(), &m, &pull)], &p, &[]), Err(custom(0, E_WRONG_PULL_STATE)));
    assert_eq!(env.svm.get_account(&pull).unwrap().owner, PROGRAM_ID);
    env.claim(&p, &pull).unwrap();
    // Paid: delegation works.
    buy_and_delegate(&mut env, &p);
}
