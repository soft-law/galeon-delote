//! Magic Action: settle_and_claim (ER) commits + undelegates Pull and schedules claim_action as a
//! post-undelegate base action; the escrow of the signer pays the asset rent.
mod common;
use common::*;
use magicsvm::TransactionTarget::Ephemeral;
use solana_address::Address;
use solana_keypair::Keypair;
use solana_signer::Signer;

fn fulfilled_in_er(env: &mut Env, p: &Keypair, escrow_lamports: u64) -> Address {
    let nonce = env.next_nonce(&p.pubkey());
    let m = env.machine;
    let pull = pull_pda(&m, &p.pubkey(), nonce);
    let top = top_up_escrow_ix(&p.pubkey(), &p.pubkey(), escrow_lamports);
    let buy = buy_pull_ix(&p.pubkey(), &m, &env.seller.pubkey(), nonce, SOL, None);
    let del = delegate_pull_ix(&p.pubkey(), &m, &pull);
    env.send(&[top, buy, del], p, &[]).expect("top_up + buy + delegate");
    env.send_to(Ephemeral, &[spin_ix(&p.pubkey(), &pull, EPHEMERAL_QUEUE)], p, &[]).unwrap();
    env.svm.set_vrf_randomness(Some(randomness_u64s(4, 0)));
    assert_eq!(env.fulfill(), vec![Ok(())]);
    pull
}

fn clear_delegation_pdas(env: &mut Env, pull: &Address) {
    for tag in [&b"delegation"[..], &b"delegation-metadata"[..]] {
        let a = Address::find_program_address(&[tag, pull.as_ref()], &DLP).0;
        env.svm.set_account(a, solana_account::Account::default()).unwrap();
    }
}

#[test]
fn settle_and_claim_mints_through_the_post_undelegate_action() {
    let mut env = open_env(&[tier(3, "blanco"), tier(2, "reposado")]);
    let p = env.player();
    let pull = fulfilled_in_er(&mut env, &p, 10_000_000);
    let escrow = escrow_pda(&p.pubkey());
    let before = env.svm.get_balance(&escrow).unwrap();
    let m = env.machine;
    let cu = env
        .send_to(Ephemeral, &[settle_and_claim_ix(&p.pubkey(), &pull, &m, None)], &p, &[])
        .expect("settle_and_claim");
    println!("settle_and_claim (ER) CU: {cu}");
    clear_delegation_pdas(&mut env, &pull);
    let base = env.pull(&pull);
    assert_eq!(base.status, PULL_CLAIMED, "claimed by the action");
    let asset = env.svm.get_account(&asset_pda(&pull)).expect("asset minted");
    assert_eq!(asset.owner, CORE_ID);
    assert_eq!(&asset.data[1..33], p.pubkey().as_ref(), "asset to the player");
    assert!(before - env.svm.get_balance(&escrow).unwrap() >= asset.lamports, "escrow paid the asset rent");
    assert_eq!(env.machine().pending, 0);
    assert!(env.player_state(&p.pubkey()).live_pull.is_none());
}

#[test]
fn settle_and_claim_with_a_session_key_uses_the_session_key_escrow() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let sk = Keypair::new();
    env.svm.airdrop(&sk.pubkey(), 10_000_000).unwrap();
    let until = env.svm.get_sysvar::<solana_clock::Clock>().unix_timestamp + 3600;
    let token = put_session_token(&mut env.svm, &p.pubkey(), &PROGRAM_ID, &sk.pubkey(), until);
    let nonce = env.next_nonce(&p.pubkey());
    let m = env.machine;
    let pull = pull_pda(&m, &p.pubkey(), nonce);
    let top = top_up_escrow_ix(&p.pubkey(), &sk.pubkey(), 10_000_000);
    let buy = buy_pull_ix(&p.pubkey(), &m, &env.seller.pubkey(), nonce, SOL, None);
    let del = delegate_pull_ix(&p.pubkey(), &m, &pull);
    env.send(&[top, buy, del], &p, &[]).unwrap();
    env.send_to(Ephemeral, &[spin_session_ix(&sk.pubkey(), &pull, EPHEMERAL_QUEUE, &token)], &sk, &[]).unwrap();
    assert_eq!(env.fulfill(), vec![Ok(())]);
    env.send_to(Ephemeral, &[settle_and_claim_ix(&sk.pubkey(), &pull, &m, Some(&token))], &sk, &[])
        .expect("settle_and_claim by the session key");
    clear_delegation_pdas(&mut env, &pull);
    assert_eq!(env.pull(&pull).status, PULL_CLAIMED);
}

#[test]
fn settle_and_claim_needs_fulfilled() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let nonce = env.next_nonce(&p.pubkey());
    let m = env.machine;
    let pull = pull_pda(&m, &p.pubkey(), nonce);
    let buy = buy_pull_ix(&p.pubkey(), &m, &env.seller.pubkey(), nonce, SOL, None);
    let del = delegate_pull_ix(&p.pubkey(), &m, &pull);
    env.send(&[buy, del], &p, &[]).unwrap();
    assert_eq!(
        env.send_to(Ephemeral, &[settle_and_claim_ix(&p.pubkey(), &pull, &m, None)], &p, &[]),
        Err(custom(0, E_NOT_FULFILLED))
    );
}

#[test]
fn claim_action_cannot_be_called_directly() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let pull = env.fulfilled_pull(&p, randomness_u64s(0, 0));
    let m = env.machine;
    env.send(&[top_up_escrow_ix(&p.pubkey(), &p.pubkey(), 10_000_000)], &p, &[]).unwrap();
    let r = env.send(&[claim_action_direct_ix(&p.pubkey(), &p.pubkey(), &m, &pull)], &p, &[]);
    assert_eq!(r, Err(custom(0, E_NOT_ESCROW_SIGNER)));
    // The permissionless claim still works.
    env.claim(&p, &pull).unwrap();
}
