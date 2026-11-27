# Galeón d'Elote — "Barriles sorpresa"

A tequila gacha on Solana built with MagicBlock. You buy a barrel, a pack of 1, 3 or 5. MagicBlock VRF opens it
inside an Ephemeral Rollup. Each prize is a transferable Metaplex Core NFT: Blanco, Añejo, Mezcal or a silver coin.
Burning the NFT redeems it for the physical prize.

Built for **MagicBlock Solana Blitz v10 (Entertainment Finance)** by [soft.law](https://soft.law) / [Elote](https://www.elote.baby).
**Devnet only: a demo with no real bottles shipped.**

## What makes it different

- **The odds are the live stock, frozen at payment.** Each tier's weight is the number of bottles still in the
  barrel, and `buy_pull` copies those counts into the Pull. You get exactly the odds you saw when you paid. Nobody can
  reorder reveals to steer a tier, and the last silver coin really is 1 in N.
- **Only the spin runs in the rollup.** Money, the Machine and every Core asset stay on Solana. Only the `Pull`
  account is delegated, so a stalled rollup can never hold funds or NFTs.
- **Signing a pack of 3 takes one wallet signature.** A MagicBlock session key signs the spin and settle. A Magic
  Action mints the NFTs on the base layer after undelegation, and the validator pays that transaction's fee.
- **Zero-fee spins.** Rollup transactions cost 0 lamports. The VRF callback lands 36 to 77 ms after the spin.

## MagicBlock products used

| Product | Where |
|---|---|
| **Ephemeral Rollups** | `delegate_pull` (base) → `spin` in the ER → `settle_pull` / `settle_and_claim` = commit + undelegate with `MagicIntentBundleBuilder` (SDK 0.18.0) |
| **VRF** | Ephemeral queue inside the ER (free) or `DEFAULT_QUEUE` on base as a fallback. The callback identity is computed in a `const`, which costs 5k CU against ~768k CU for the macro's default |
| **Session Keys** | `SessionTokenV2` (`session-keys` 3.1.1), checked on-chain inside the ER. A session key can spin, respin and settle; only the wallet can buy, delegate or redeem |
| **Magic Actions** | `settle_and_claim` schedules one `claim_action` per pack item as a post-undelegate action, paid from the signer's DLP escrow (`TopUpEphemeralBalance` in the buy transaction) |

## Flow

```
 wallet signs once                ── Ephemeral Rollup (0 fee) ──          ── base, sent by the validator ──
 [create_session_v2 + escrow top-up  spin (session key)                   commit + undelegate Pull
  + buy_pull(SOL|USDC, count)    →   VRF callback (~40-80 ms)        →    claim_action ×count → Core NFTs
  + delegate_pull]  (base)           settle_and_claim (session key)       to the player
```

The base-layer fallback is `buy_pull + spin` on base (`DEFAULT_QUEUE`), then the callback, then `claim`. `claim`
needs no permission: anyone, including a crank, can mint a fulfilled Pull, and the NFT always goes to `pull.player`.
`redeem(index)` burns the NFT and writes a `Redemption` record; shipping is handled off-chain.

## Devnet

| | |
|---|---|
| Program | [`2LVXXXxVUMQPnewQGKpNDb3dRRfhVdbSmhYwNppfuAis`](https://explorer.solana.com/address/2LVXXXxVUMQPnewQGKpNDb3dRRfhVdbSmhYwNppfuAis?cluster=devnet). The deployed binary matches `target/deploy/tequila_gacha.so` byte for byte (434 560 B, sha256 `59d76aff…4ea6841c`) |
| Machine #2 "Galeón d'Elote" | [`4fcwnpAciPin3R2KkKL3jJAaKmCNpaxb98LBYsVXexhK`](https://explorer.solana.com/address/4fcwnpAciPin3R2KkKL3jJAaKmCNpaxb98LBYsVXexhK?cluster=devnet): Blanco 40, Añejo 25, Mezcal 10, Moneda de plata 3; 0.01 SOL or 1 USDC per pull |
| Core collection | [`8MLqk9dEudjcdSauwYP1pT4oEWw5hXkn9P43ZvDhPFrK`](https://explorer.solana.com/address/8MLqk9dEudjcdSauwYP1pT4oEWw5hXkn9P43ZvDhPFrK?cluster=devnet); its update authority is a program PDA |
| ER validator | `MAS1Dt9qreoRMQ14YQuhg8UTZMMzDdKhmkZMECCzk57` (devnet-as), stored in `Machine.er_validator` |
| Metadata | Pinata IPFS, e.g. [collection.json](https://gateway.pinata.cloud/ipfs/bafkreiekjuf7ogymo743voix6piaffiuxo3nhyc4zkyb7cos32tmzuacni) |

### Proof: a pack of 3 paid in USDC, with one wallet signature (24 Nov)

| Step | Layer | Tx |
|---|---|---|
| session + escrow top-up + `buy_pull(USDC, 3)` + `delegate_pull` (wallet 17 → 14 USDC) | base | [HGsndtCm…](https://explorer.solana.com/tx/HGsndtCm9CGoUQRF3EA6rA7U7jejgtohv1JMJCYdky4EkYmcmiRB7PDqFfMrJNQWsCsripoJa9m5QLvnnAxLkUS?cluster=devnet) |
| `spin`, signed by the session key, fee 0 | ER | [2d95nVve…](https://explorer.solana.com/tx/2d95nVveCAwNEwEqA29NKtSdLahFNZzgrktVgM2KFTByhsKmyQ2sGNrtoX8m8tnyWbdB9RKmBWprAJtNwyyeQ1rE?cluster=custom&customUrl=https%3A%2F%2Fdevnet-as.magicblock.app%2F) |
| VRF callback, 65 ms after the spin confirmed | ER | [2QW8Tpsa…](https://explorer.solana.com/tx/2QW8Tpsaicm4E95XX9pGz3oFiYZ4c5RCR9n4Djz61zoG9ykMc1UfJhEZfMMneiKWn9hYErkixdo18muqFrK4ySce?cluster=custom&customUrl=https%3A%2F%2Fdevnet-as.magicblock.app%2F) |
| `settle_and_claim`, signed by the session key, fee 0 | ER | [354omMWo…](https://explorer.solana.com/tx/354omMWok1SMakoapqmTWimo3ri1LVsPPxdUPcSTUuntW1WWDr6YR61gmjd8JSsAhQhJ5vRJga7MQwsYbumVyBGz?cluster=custom&customUrl=https%3A%2F%2Fdevnet-as.magicblock.app%2F) |
| commit + undelegate + 3 × `claim_action` → Añejo #1, Blanco #1, Blanco #2; back on base 2.4 s later | base | [3LccoTha…](https://explorer.solana.com/tx/3LccoThahZB8onrrKNcpb1civzApJJLCHd1JgykBBb72rDJk3iypezdbhrdC3FJpEa3KGxy8Kw5fwYCAjfGcrQh3?cluster=devnet) |

### Proof: a pull from the live screen (27 Nov)

On [www.elote.baby/app/barriles](https://www.elote.baby/app/barriles), a wallet bought one barrel in SOL
([5cTcfScx…](https://explorer.solana.com/tx/5cTcfScxnaYUGrvUsWfsggzse56SCo5FjcGzgKa86SZoAfiFQ2iGYvyyFbAMszv1YStL5oKVW3kPiizt5LFw4TPJ?cluster=devnet)),
spun in the ER and claimed Añejo #2
([2xgDeCB4…](https://explorer.solana.com/tx/2xgDeCB4js14vAVP3bAV4YnraLJt45eHZScP3cgpMgZmtr49QEqTkY5NsL5czGjZiVBLC32vhs9SBaefR5LFnSjr?cluster=devnet)),
about 30 seconds end to end. Demo video: [watch it](https://gateway.pinata.cloud/ipfs/bafybeihcmbtnhixrf77qcarpdls5gqxpghutca4axfgbtok6h6mhjqg6be) (also in `docs/demo.mp4`).

The scripts append every devnet run (step, signature, CU, timing) to `runs/devnet.jsonl`, which git ignores.

## Repository layout

| Path | What it is |
|---|---|
| `programs/tequila_gacha/` | **The on-chain program** (Rust, Anchor 1.0.2). `lib.rs` holds the instructions and accounts, `logic.rs` the draw (odds snapshot, sha256 picks, reroll), and `core_cpi.rs` the hand-built Metaplex Core calls. |
| `tests-magicsvm-rs/` | **The test suite** (71 tests): MagicSVM loads the real `.so` plus Metaplex Core and runs the ER lifecycle, VRF callbacks, session keys, packs and Magic Actions. |
| `idl/` | The program's interface (Anchor IDL), generated from the program. |
| `clients/js/` | The TypeScript client generated from the IDL by Codama (`clients/js/src/generated`). Never edited by hand. The web app copies it. |
| `scripts/` | **What you run against devnet** (bun + @solana/kit): `admin.ts` creates and opens a machine, `play.ts` plays a pull (ER or base, SOL or USDC, packs, session key, Magic Action), `crank-claim.ts` claims fulfilled pulls, `codama.ts` regenerates the client. `lib.ts`, `session.ts` and `escrow.ts` are their shared helpers. |
| `assets/` | The NFT metadata: `tiers.ts` holds the four approved tiers and their Pinata IPFS URIs, and `gen.ts` writes the JSON and a plain SVG card per tier into `assets/tiers/`. |
| `fixtures/mpl_core.so` | Metaplex Core's devnet binary, used only by the tests. |
| `docs/demo.mp4` | The demo video. |
| `Anchor.toml`, `Cargo.toml`, `Cargo.lock` | The Rust workspace and toolchain pins. |
| `package.json`, `bun.lock` | The TypeScript dependencies for the scripts and the client. |

## Build and test

Toolchain: solana-cli 3.1.10 (platform-tools v1.52), anchor-lang 1.0.2, ephemeral-rollups-sdk 0.18.0 (`anchor`, `vrf`),
session-keys 3.1.1, magicsvm 0.3.0, bun and @solana/kit 8.6 (Codama client).

```bash
cargo build-sbf                                # target/deploy/tequila_gacha.so
cargo +1.89.0 test -p tequila_gacha --lib      # draw logic unit tests (10)
cd tests-magicsvm-rs && cargo +stable test     # MagicSVM suite (71): ER lifecycle, VRF callbacks, sessions, packs, actions
bun install && bun scripts/codama.ts           # regenerate clients/js/src/generated from idl/
```

## Play on devnet

Keypairs are never committed. Put them in `keys/` (git-ignored) or point `KEYS=` at a folder of your own.

```bash
MACHINE_ID=2 PLAYER=<keyfile-name> bun scripts/play.ts er                     # 1 pull through the ER
MACHINE_ID=2 PLAYER=<keyfile-name> SESSION=1 COUNT=3 ACTION=1 bun scripts/play.ts er   # 1 wallet signature, pack of 3
MACHINE_ID=2 PLAYER=<keyfile-name> CURRENCY=usdc bun scripts/play.ts er       # pay in devnet USDC
MACHINE_ID=2 PLAYER=<keyfile-name> bun scripts/play.ts base                   # base-layer fallback
MACHINE_ID=2 bun scripts/crank-claim.ts                                       # claim every fulfilled Pull
```

## Design notes

- **Accounts.** `Machine ["machine", authority, id]`, `PlayerState ["player", machine, player]`,
  `Pull ["pull", machine, player, nonce]` (the only account delegated), asset `["asset", pull, index]`, which the program
  signs for, and `Redemption ["redemption", asset]`.
- **Draw.** For each item i: `r = u64(sha256(randomness ‖ "pick" ‖ i))`, weighted by the stock snapshot, without
  replacement. Paid pulls reserve stock (`pending`). If a tier runs out before the claim, that item is redrawn with
  `sha256(randomness ‖ "reroll" ‖ i)` over the current stock and flagged `rerolled`. Anyone can recompute the draw off-chain.
- **Safety rails.** `max_price_total` guards against a price change. A stale VRF callback is rejected through `request_seq`.
  `respin` is allowed after 130 s, and `settle_pull` cannot pull a Requested Pull out of the ER while its callback is
  in flight. Redeem checks the real owner in the Core asset's bytes, so a burn delegate cannot redeem.
- **Core CPIs are hand-built** (CreateCollectionV2, CreateV2, BurnV1): the same bytes as mpl-core 0.12.1 in a binary 43 KB smaller.

## Known limits

- A Pull that is paid and never spun keeps its stock reserved. A permissionless timeout is planned.
- A respin after a stale callback is covered only in MagicSVM, because the devnet callback arrives in under 100 ms.
- The program has not been audited. An internal review found no critical, high or medium issues.

## License

GPL-3.0-or-later (see [LICENSE](LICENSE)); the program links MagicBlock's GPL `session-keys` crate.
`fixtures/mpl_core.so` is Metaplex Core's devnet binary, used only by the tests and not covered by this license.
