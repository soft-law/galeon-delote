//! Admin instructions and buy_pull: payments, reservation, snapshot, guards.
mod common;
use common::*;
use solana_keypair::Keypair;
use solana_signer::Signer;

fn name_str(b: &[u8]) -> String {
    String::from_utf8(b.iter().copied().take_while(|c| *c != 0).collect()).unwrap()
}

#[test]
fn init_machine_creates_machine_and_collection_owned_by_program_pda() {
    let env = draft_env(&[]);
    let m = env.machine();
    assert_eq!(m.authority, env.admin.pubkey().to_bytes());
    assert_eq!(m.seller, env.seller.pubkey().to_bytes());
    assert_eq!(m.status, 0);
    assert_eq!(m.tier_count, 0);
    assert_eq!(m.price_lamports, PRICE_LAMPORTS);
    assert_eq!(m.price_usdc, PRICE_USDC);
    assert_eq!(m.er_validator, env.svm.validator_identity().to_bytes());
    let col = collection_pda(&env.machine);
    assert_eq!(m.collection, col.to_bytes());
    let acc = env.svm.get_account(&col).unwrap();
    assert_eq!(acc.owner, CORE_ID);
    assert_eq!(acc.data[0], 5, "CollectionV1");
    assert_eq!(&acc.data[1..33], core_authority_pda().as_ref(), "update authority = program PDA");
}

#[test]
fn set_tiers_only_in_draft_and_validates() {
    let mut env = draft_env(&[tier(3, "blanco"), tier(2, "reposado")]);
    let (a, m) = (env.admin.pubkey(), env.machine);
    let mach = env.machine();
    assert_eq!(mach.tier_count, 2);
    assert_eq!(mach.tiers[1].cap, 2);
    assert_eq!(name_str(&mach.tiers[1].name), "reposado");
    assert_eq!(name_str(&mach.tiers[0].uri), "https://example.com/blanco.json");

    // Chunked write: start = 2 extends to 3 tiers.
    env.admin_send(set_tiers_ix(&a, &m, 2, &[tier(1, "anejo")])).unwrap();
    assert_eq!(env.machine().tier_count, 3);

    // Bad input.
    assert_eq!(env.admin_send(set_tiers_ix(&a, &m, 0, &[tier(0, "x")])), Err(custom(0, E_INVALID_TIERS)));
    assert_eq!(env.admin_send(set_tiers_ix(&a, &m, 6, &[tier(1, "x")])), Err(custom(0, E_INVALID_TIERS)));
    let long = TierInput { cap: 1, name: "n".repeat(33), uri: "u".into() };
    assert_eq!(env.admin_send(set_tiers_ix(&a, &m, 0, &[long])), Err(custom(0, E_INVALID_TIERS)));

    // Not the authority.
    let intruder = env.player();
    let err = env.send(&[set_tiers_ix(&intruder.pubkey(), &m, 0, &[tier(1, "x")])], &intruder, &[]).unwrap_err();
    assert_eq!(err, custom(0, 2001), "ConstraintHasOne");

    env.admin_send(open_ix(&a, &m)).unwrap();
    assert_eq!(env.admin_send(set_tiers_ix(&a, &m, 0, &[tier(5, "x")])), Err(custom(0, E_TIERS_LOCKED)));
}

#[test]
fn open_needs_tiers_close_and_reopen() {
    let mut env = draft_env(&[]);
    let (a, m) = (env.admin.pubkey(), env.machine);
    assert_eq!(env.admin_send(open_ix(&a, &m)), Err(custom(0, E_NO_TIERS)));
    assert_eq!(env.admin_send(close_ix(&a, &m)), Err(custom(0, E_WRONG_MACHINE_STATE)));
    env.admin_send(set_tiers_ix(&a, &m, 0, &[tier(1, "x")])).unwrap();
    env.admin_send(open_ix(&a, &m)).unwrap();
    assert_eq!(env.machine().status, 1);
    env.admin_send(close_ix(&a, &m)).unwrap();
    assert_eq!(env.machine().status, 2);
    env.admin_send(open_ix(&a, &m)).unwrap();
    assert_eq!(env.machine().status, 1);
}

#[test]
fn set_prices_and_er_validator() {
    let mut env = draft_env(&[tier(1, "x")]);
    let (a, m) = (env.admin.pubkey(), env.machine);
    env.admin_send(set_prices_ix(&a, &m, 7, 0)).unwrap();
    let v = Keypair::new().pubkey();
    env.admin_send(set_er_validator_ix(&a, &m, &v)).unwrap();
    let mach = env.machine();
    assert_eq!((mach.price_lamports, mach.price_usdc), (7, 0));
    assert_eq!(mach.er_validator, v.to_bytes());
}

#[test]
fn buy_sol_pays_seller_directly_reserves_and_snapshots() {
    let mut env = open_env(&[tier(3, "blanco"), tier(2, "reposado"), tier(1, "anejo")]);
    let p = env.player();
    let seller_before = env.svm.get_balance(&env.seller.pubkey()).unwrap();
    let pull = env.buy_sol(&p).unwrap();
    assert_eq!(env.svm.get_balance(&env.seller.pubkey()).unwrap() - seller_before, PRICE_LAMPORTS);
    // No program treasury: the machine only holds its rent.
    let pl = env.pull(&pull);
    assert_eq!(pl.status, PULL_PAID);
    assert_eq!(pl.snapshot, [3, 2, 1, 0, 0, 0]);
    assert_eq!(pl.player, p.pubkey().to_bytes());
    assert_eq!(pl.machine, env.machine.to_bytes());
    assert_eq!((pl.price, pl.nonce, pl.request_seq, pl.count, pl.claimed), (PRICE_LAMPORTS, 0, 0, 1, 0));
    assert!(!pl.rerolled);
    assert_eq!(env.machine().pending, 1);
    let ps = env.player_state(&p.pubkey());
    assert_eq!(ps.next_nonce, 1);
    assert_eq!(ps.live_pull, Some(pull.to_bytes()));
}

#[test]
fn buy_usdc_pays_seller_token_account() {
    let mut env = open_env(&[tier(5, "blanco")]);
    let p = env.player();
    let u = env.usdc_accounts(&p.pubkey(), 10_000_000);
    let ix = buy_pull_ix(&p.pubkey(), &env.machine, &env.seller.pubkey(), 0, USDC, Some(&u));
    env.send(&[ix], &p, &[]).unwrap();
    assert_eq!(token_amount(&env.svm, &u.seller_ata), PRICE_USDC);
    assert_eq!(token_amount(&env.svm, &u.player_ata), 10_000_000 - PRICE_USDC);
    let pl = env.pull(&pull_pda(&env.machine, &p.pubkey(), 0));
    assert_eq!((pl.paid_with, pl.price), (USDC, PRICE_USDC));
}

#[test]
fn buy_usdc_rejects_wrong_destination_and_missing_accounts() {
    let mut env = open_env(&[tier(5, "blanco")]);
    let p = env.player();
    // Destination owned by someone other than the seller.
    let mut u = env.usdc_accounts(&p.pubkey(), 10_000_000);
    let other = Keypair::new().pubkey();
    let bad = solana_address::Address::new_unique();
    env.svm.set_account(bad, token_account(&env.usdc_mint, &other, 0)).unwrap();
    u.seller_ata = bad;
    let ix = buy_pull_ix(&p.pubkey(), &env.machine, &env.seller.pubkey(), 0, USDC, Some(&u));
    assert_eq!(env.send(&[ix], &p, &[]), Err(custom(0, E_INVALID_TOKEN_ACCOUNT)));
    // Destination of another mint.
    let mint2 = solana_address::Address::new_unique();
    env.svm.set_account(mint2, mint_account(6)).unwrap();
    let bad2 = solana_address::Address::new_unique();
    env.svm.set_account(bad2, token_account(&mint2, &env.seller.pubkey(), 0)).unwrap();
    u.seller_ata = bad2;
    let ix = buy_pull_ix(&p.pubkey(), &env.machine, &env.seller.pubkey(), 0, USDC, Some(&u));
    assert_eq!(env.send(&[ix], &p, &[]), Err(custom(0, E_INVALID_TOKEN_ACCOUNT)));
    // USDC without token accounts.
    let ix = buy_pull_ix(&p.pubkey(), &env.machine, &env.seller.pubkey(), 0, USDC, None);
    assert_eq!(env.send(&[ix], &p, &[]), Err(custom(0, E_MISSING_TOKEN_ACCOUNTS)));
}

#[test]
fn currency_with_zero_price_is_not_accepted() {
    let mut env = open_env(&[tier(5, "blanco")]);
    let (a, m) = (env.admin.pubkey(), env.machine);
    env.admin_send(set_prices_ix(&a, &m, 0, PRICE_USDC)).unwrap();
    let p = env.player();
    assert_eq!(env.buy_sol(&p).unwrap_err(), custom(0, E_CURRENCY_NOT_ACCEPTED));
    env.admin_send(set_prices_ix(&a, &m, PRICE_LAMPORTS, 0)).unwrap();
    let u = env.usdc_accounts(&p.pubkey(), 10_000_000);
    let ix = buy_pull_ix(&p.pubkey(), &m, &env.seller.pubkey(), 0, USDC, Some(&u));
    assert_eq!(env.send(&[ix], &p, &[]), Err(custom(0, E_CURRENCY_NOT_ACCEPTED)));
}

#[test]
fn seller_must_match_machine() {
    let mut env = open_env(&[tier(5, "blanco")]);
    let p = env.player();
    let fake = Keypair::new().pubkey();
    let ix = buy_pull_ix(&p.pubkey(), &env.machine, &fake, 0, SOL, None);
    assert_eq!(env.send(&[ix], &p, &[]), Err(custom(0, 2012)), "ConstraintAddress");
}

#[test]
fn buy_needs_open_machine() {
    let mut env = draft_env(&[tier(5, "blanco")]);
    let p = env.player();
    assert_eq!(env.buy_sol(&p).unwrap_err(), custom(0, E_MACHINE_NOT_OPEN));
    let (a, m) = (env.admin.pubkey(), env.machine);
    env.admin_send(open_ix(&a, &m)).unwrap();
    env.admin_send(close_ix(&a, &m)).unwrap();
    assert_eq!(env.buy_sol(&p).unwrap_err(), custom(0, E_MACHINE_NOT_OPEN));
}

#[test]
fn one_live_pull_per_player_until_claimed() {
    let mut env = open_env(&[tier(5, "blanco")]);
    let p = env.player();
    let pull = env.buy_sol(&p).unwrap();
    assert_eq!(env.buy_sol(&p).unwrap_err(), custom(0, E_LIVE_PULL_EXISTS));
    // Still live while Requested and Fulfilled.
    env.spin_base(&p, &pull).unwrap();
    assert_eq!(env.buy_sol(&p).unwrap_err(), custom(0, E_LIVE_PULL_EXISTS));
    assert_eq!(env.fulfill(), vec![Ok(())]);
    assert_eq!(env.buy_sol(&p).unwrap_err(), custom(0, E_LIVE_PULL_EXISTS));
    env.claim(&p, &pull).unwrap();
    assert_eq!(env.player_state(&p.pubkey()).live_pull, None);
    let pull2 = env.buy_sol(&p).unwrap();
    assert_eq!(env.pull(&pull2).nonce, 1);
    // Another player is independent.
    let q = env.player();
    env.buy_sol(&q).unwrap();
}

#[test]
fn sold_out_counts_reservations() {
    let mut env = open_env(&[tier(1, "blanco"), tier(1, "reposado")]);
    let a = env.player();
    let b = env.player();
    let c = env.player();
    env.buy_sol(&a).unwrap();
    env.buy_sol(&b).unwrap();
    // Two units, two reservations: the third buyer is refused before any claim.
    assert_eq!(env.buy_sol(&c).unwrap_err(), custom(0, E_SOLD_OUT));
    assert_eq!(env.machine().pending, 2);
}

#[test]
fn sold_out_after_claims_too() {
    let mut env = open_env(&[tier(1, "blanco")]);
    let a = env.player();
    let pull = env.fulfilled_pull(&a, [1; 32]);
    env.claim(&a, &pull).unwrap();
    let b = env.player();
    assert_eq!(env.buy_sol(&b).unwrap_err(), custom(0, E_SOLD_OUT));
    let m = env.machine();
    assert_eq!((m.pending, m.tiers[0].assigned), (0, 1));
}

#[test]
fn snapshot_copies_remaining_without_pending() {
    let mut env = open_env(&[tier(2, "blanco"), tier(3, "reposado")]);
    let a = env.player();
    let pa = env.fulfilled_pull(&a, rand_picking(&[2, 3], &[0]));
    env.claim(&a, &pa).unwrap();
    let b = env.player();
    env.buy_sol(&b).unwrap();
    let c = env.player();
    let pc = env.buy_sol(&c).unwrap();
    // One claimed from tier 0; b's reservation is not subtracted.
    assert_eq!(env.pull(&pc).snapshot, [1, 3, 0, 0, 0, 0]);
}

#[test]
fn buy_refuses_a_price_above_max_price() {
    let mut env = open_env(&[tier(5, "blanco")]);
    let (a, m) = (env.admin.pubkey(), env.machine);
    let p = env.player();
    let seller = env.seller.pubkey();
    // The player saw PRICE_LAMPORTS; the authority raises the price before the buy lands.
    env.admin_send(set_prices_ix(&a, &m, PRICE_LAMPORTS * 3, PRICE_USDC)).unwrap();
    let ix = buy_pull_max_ix(&p.pubkey(), &m, &seller, 0, SOL, None, PRICE_LAMPORTS);
    assert_eq!(env.send(&[ix], &p, &[]), Err(custom(0, E_PRICE_ABOVE_MAX)));
    assert_eq!(env.machine().pending, 0);
    // USDC too.
    env.admin_send(set_prices_ix(&a, &m, PRICE_LAMPORTS, PRICE_USDC + 1)).unwrap();
    let u = env.usdc_accounts(&p.pubkey(), 10 * PRICE_USDC);
    let ix = buy_pull_max_ix(&p.pubkey(), &m, &seller, 0, USDC, Some(&u), PRICE_USDC);
    assert_eq!(env.send(&[ix], &p, &[]), Err(custom(0, E_PRICE_ABOVE_MAX)));
    // max_price equal to the price passes.
    let ix = buy_pull_max_ix(&p.pubkey(), &m, &seller, 0, SOL, None, PRICE_LAMPORTS);
    env.send(&[ix], &p, &[]).unwrap();
}

#[test]
fn er_validator_cannot_be_the_default_pubkey() {
    let mut env = draft_env(&[tier(1, "x")]);
    let (a, m) = (env.admin.pubkey(), env.machine);
    let zero = solana_address::Address::default();
    assert_eq!(env.admin_send(set_er_validator_ix(&a, &m, &zero)), Err(custom(0, E_INVALID_VALIDATOR)));
    let cfg = MachineCfg {
        id: 2,
        seller: env.seller.pubkey(),
        usdc_mint: env.usdc_mint,
        price_lamports: 1,
        price_usdc: 0,
        er_validator: zero,
    };
    assert_eq!(env.admin_send(init_machine_ix(&a, &cfg)), Err(custom(0, E_INVALID_VALIDATOR)));
}
