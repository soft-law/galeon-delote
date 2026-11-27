//! MagicBlock Session Keys: spin, respin and settle_pull signed by a session key whose
//! SessionTokenV2 (authority = the pull's player, target = this program) is valid.
mod common;
use common::*;
use magicsvm::TransactionTarget::Ephemeral;
use solana_address::Address;
use solana_clock::Clock;
use solana_keypair::Keypair;
use solana_signer::Signer;

fn now(env: &Env) -> i64 {
    env.svm.get_sysvar::<Clock>().unix_timestamp
}

/// A funded session key with a valid token for `player` (1 h).
fn session(env: &mut Env, player: &Address) -> (Keypair, Address) {
    let sk = Keypair::new();
    env.svm.airdrop(&sk.pubkey(), 50_000_000).unwrap();
    let until = now(env) + 3600;
    let token = put_session_token(&mut env.svm, player, &PROGRAM_ID, &sk.pubkey(), until);
    (sk, token)
}

#[test]
fn session_key_spins_on_base_and_pays_the_vrf_fee() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let pull = env.buy_sol(&p).unwrap();
    let (sk, token) = session(&mut env, &p.pubkey());
    let before = env.svm.get_balance(&sk.pubkey()).unwrap();
    let cu = env.send(&[spin_session_ix(&sk.pubkey(), &pull, DEFAULT_QUEUE, &token)], &sk, &[]).expect("session spin");
    println!("spin (base, session) CU: {cu}");
    assert_eq!(env.pull(&pull).status, PULL_REQUESTED);
    assert!(before - env.svm.get_balance(&sk.pubkey()).unwrap() >= 500_000, "session key paid the VRF fee");
    assert_eq!(env.fulfill(), vec![Ok(())]);
    assert_eq!(env.pull(&pull).status, PULL_FULFILLED);
}

#[test]
fn session_key_spins_and_settles_in_the_er_then_claims() {
    let mut env = open_env(&[tier(3, "blanco"), tier(2, "reposado")]);
    let p = env.player();
    let (sk, token) = session(&mut env, &p.pubkey());
    let nonce = env.next_nonce(&p.pubkey());
    let m = env.machine;
    let pull = pull_pda(&m, &p.pubkey(), nonce);
    let buy = buy_pull_ix(&p.pubkey(), &m, &env.seller.pubkey(), nonce, SOL, None);
    let del = delegate_pull_ix(&p.pubkey(), &m, &pull);
    env.send(&[buy, del], &p, &[]).expect("buy + delegate");

    let cu = env
        .send_to(Ephemeral, &[spin_session_ix(&sk.pubkey(), &pull, EPHEMERAL_QUEUE, &token)], &sk, &[])
        .expect("spin in ER by the session key");
    println!("spin (ER, session) CU: {cu}");
    env.svm.set_vrf_randomness(Some(randomness_u64s(4, 0)));
    assert_eq!(env.fulfill(), vec![Ok(())]);
    assert_eq!(env.pull_er(&pull).status, PULL_FULFILLED);

    let cu = env
        .send_to(Ephemeral, &[settle_session_ix(&sk.pubkey(), &pull, Some(&token))], &sk, &[])
        .expect("settle in ER by the session key");
    println!("settle_pull (ER, session) CU: {cu}");
    for tag in [&b"delegation"[..], &b"delegation-metadata"[..]] {
        let a = Address::find_program_address(&[tag, pull.as_ref()], &DLP).0;
        env.svm.set_account(a, solana_account::Account::default()).unwrap();
    }
    assert_eq!(env.svm.get_account(&pull).unwrap().owner, PROGRAM_ID);
    env.send(&[claim_ix(&sk.pubkey(), &p.pubkey(), &m, &pull)], &sk, &[]).expect("claim paid by the session key");
    assert_eq!(env.pull(&pull).status, PULL_CLAIMED);
}

#[test]
fn session_key_respins_after_130_s() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let pull = env.buy_sol(&p).unwrap();
    let (sk, token) = session(&mut env, &p.pubkey());
    env.send(&[spin_session_ix(&sk.pubkey(), &pull, DEFAULT_QUEUE, &token)], &sk, &[]).unwrap();
    env.advance_secs(RESPIN);
    env.send(&[respin_session_ix(&sk.pubkey(), &pull, DEFAULT_QUEUE, &token)], &sk, &[]).expect("session respin");
    assert_eq!(env.pull(&pull).request_seq, 1);
}
const RESPIN: i64 = 130;

#[test]
fn session_key_without_token_is_refused() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let pull = env.buy_sol(&p).unwrap();
    let (sk, _) = session(&mut env, &p.pubkey());
    assert_eq!(
        env.send(&[spin_ix(&sk.pubkey(), &pull, DEFAULT_QUEUE)], &sk, &[]),
        Err(custom(0, E_NOT_PLAYER_OR_SESSION))
    );
}

#[test]
fn token_for_another_wallet_is_refused() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let pull = env.buy_sol(&p).unwrap();
    let other = env.player();
    let (sk, token) = session(&mut env, &other.pubkey());
    assert_eq!(
        env.send(&[spin_session_ix(&sk.pubkey(), &pull, DEFAULT_QUEUE, &token)], &sk, &[]),
        Err(custom(0, E_NOT_PLAYER_OR_SESSION))
    );
}

#[test]
fn token_of_another_session_key_is_refused() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let pull = env.buy_sol(&p).unwrap();
    let (_sk, token) = session(&mut env, &p.pubkey());
    let thief = env.player();
    assert_eq!(
        env.send(&[spin_session_ix(&thief.pubkey(), &pull, DEFAULT_QUEUE, &token)], &thief, &[]),
        Err(custom(0, E_NOT_PLAYER_OR_SESSION))
    );
}

#[test]
fn token_for_another_program_is_refused() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let pull = env.buy_sol(&p).unwrap();
    let sk = env.player();
    let other_program = Address::new_unique();
    let until = now(&env) + 3600;
    let token = put_session_token(&mut env.svm, &p.pubkey(), &other_program, &sk.pubkey(), until);
    assert_eq!(
        env.send(&[spin_session_ix(&sk.pubkey(), &pull, DEFAULT_QUEUE, &token)], &sk, &[]),
        Err(custom(0, E_NOT_PLAYER_OR_SESSION))
    );
}

#[test]
fn expired_token_is_refused_on_chain() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let pull = env.buy_sol(&p).unwrap();
    let (sk, token) = session(&mut env, &p.pubkey());
    env.advance_secs(3600);
    assert_eq!(
        env.send(&[spin_session_ix(&sk.pubkey(), &pull, DEFAULT_QUEUE, &token)], &sk, &[]),
        Err(custom(0, E_SESSION_EXPIRED))
    );
}

#[test]
fn forged_token_not_owned_by_the_session_program_is_refused() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let pull = env.buy_sol(&p).unwrap();
    let sk = env.player();
    let at = session_token_pda(&PROGRAM_ID, &sk.pubkey(), &p.pubkey());
    let until = now(&env) + 3600;
    put_session_token_at(&mut env.svm, at, SYSTEM, &p.pubkey(), &PROGRAM_ID, &sk.pubkey(), until);
    assert_eq!(
        env.send(&[spin_session_ix(&sk.pubkey(), &pull, DEFAULT_QUEUE, &at)], &sk, &[]),
        Err(custom(0, ANCHOR_OWNED_BY_WRONG_PROGRAM))
    );
}

#[test]
fn session_key_cannot_settle_without_token_nor_delegate_nor_redeem() {
    let mut env = open_env(&[tier(3, "blanco")]);
    let p = env.player();
    let (sk, _token) = session(&mut env, &p.pubkey());
    let nonce = env.next_nonce(&p.pubkey());
    let m = env.machine;
    let pull = pull_pda(&m, &p.pubkey(), nonce);
    env.send(&[buy_pull_ix(&p.pubkey(), &m, &env.seller.pubkey(), nonce, SOL, None)], &p, &[]).unwrap();
    // delegate_pull: the session key is not the player (PlayerState seeds use the signer).
    assert!(env.send(&[delegate_pull_ix(&sk.pubkey(), &m, &pull)], &sk, &[]).is_err());
    env.send(&[delegate_pull_ix(&p.pubkey(), &m, &pull)], &p, &[]).unwrap();
    assert_eq!(
        env.send_to(Ephemeral, &[settle_session_ix(&sk.pubkey(), &pull, None)], &sk, &[]),
        Err(custom(0, E_NOT_PLAYER_OR_SESSION))
    );
    env.settle(&p, &pull).unwrap();
    env.spin_base(&p, &pull).unwrap();
    assert_eq!(env.fulfill(), vec![Ok(())]);
    env.claim(&p, &pull).unwrap();
    // redeem: only the asset's owner.
    assert_eq!(env.send(&[redeem_ix(&sk.pubkey(), &m, &pull)], &sk, &[]), Err(custom(0, E_NOT_ASSET_OWNER)));
}

/// buy + delegate by the wallet; returns the delegated Pull.
fn delegated(env: &mut Env, p: &Keypair) -> Address {
    let nonce = env.next_nonce(&p.pubkey());
    let m = env.machine;
    let pull = pull_pda(&m, &p.pubkey(), nonce);
    let buy = buy_pull_ix(&p.pubkey(), &m, &env.seller.pubkey(), nonce, SOL, None);
    let del = delegate_pull_ix(&p.pubkey(), &m, &pull);
    env.send(&[buy, del], p, &[]).unwrap();
    pull
}

fn er_spin(env: &mut Env, sk: &Keypair, pull: &Address, token: &Address) -> Result<u64, solana_transaction_error::TransactionError> {
    env.send_to(Ephemeral, &[spin_session_ix(&sk.pubkey(), pull, EPHEMERAL_QUEUE, token)], sk, &[])
}

#[test]
fn in_the_er_bad_tokens_are_refused_and_the_wallet_still_works() {
    let mut env = open_env(&[tier(9, "blanco")]);
    let p = env.player();
    let pull = delegated(&mut env, &p);

    // Token of another wallet.
    let other = env.player();
    let (sk, token) = session(&mut env, &other.pubkey());
    assert_eq!(er_spin(&mut env, &sk, &pull, &token), Err(custom(0, E_NOT_PLAYER_OR_SESSION)));

    // Token for another program.
    let sk = env.player();
    let until = now(&env) + 3600;
    let token = put_session_token(&mut env.svm, &p.pubkey(), &Address::new_unique(), &sk.pubkey(), until);
    assert_eq!(er_spin(&mut env, &sk, &pull, &token), Err(custom(0, E_NOT_PLAYER_OR_SESSION)));

    // Revoked token (closed on base): the account no longer deserializes.
    let (sk, token) = session(&mut env, &p.pubkey());
    env.svm.set_account(token, solana_account::Account::default()).unwrap();
    assert_eq!(er_spin(&mut env, &sk, &pull, &token), Err(custom(0, ANCHOR_ACCOUNT_NOT_INITIALIZED)));

    // Expired token.
    let (sk, token) = session(&mut env, &p.pubkey());
    env.advance_secs(3600);
    assert_eq!(er_spin(&mut env, &sk, &pull, &token), Err(custom(0, E_SESSION_EXPIRED)));
    assert_eq!(
        env.send_to(Ephemeral, &[settle_session_ix(&sk.pubkey(), &pull, Some(&token))], &sk, &[]),
        Err(custom(0, E_SESSION_EXPIRED))
    );

    // The wallet path is unaffected.
    env.send_to(Ephemeral, &[spin_ix(&p.pubkey(), &pull, EPHEMERAL_QUEUE)], &p, &[]).expect("wallet spin in ER");
    assert_eq!(env.fulfill(), vec![Ok(())]);
    env.settle(&p, &pull).expect("wallet settle");
    env.claim(&p, &pull).unwrap();
}
