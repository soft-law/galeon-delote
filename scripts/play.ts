// Player flows on devnet with the generated client.
// Usage:
//   bun scripts/play.ts er [v0cb|v0|v1] (default v0cb) buy+delegate (base) -> spin (ER) -> callback (ER) -> settle (ER) -> claim (base)
//   bun scripts/play.ts base [noclaim]     buy+spin (base, DEFAULT_QUEUE) -> callback (base) -> claim (base);
//                                          noclaim leaves it Fulfilled for scripts/crank-claim.ts
//   bun scripts/play.ts transfer <pull> <toKeyName> [index]   Core TransferV1 by the current owner (PLAYER)
//   bun scripts/play.ts redeem <pull> [index]                 redeem by the current owner (PLAYER)
//   bun scripts/play.ts status
// Env: PLAYER (key name, default tequila-gacha-player-1), AUTHORITY, MACHINE_ID,
//   COUNT=1|3|5 (pack size, default 1), CLAIM_TX=one (all of a pack's claims in one tx; default one per index),
//   ACTION=1 (er only): settle_and_claim (claims as post-undelegate actions paid by the signer's DLP escrow,
//     funded in the buy tx) instead of settle_pull + claim,
//   SESSION=1 (er only): the buy tx also runs create_session_v2 (MagicBlock Session Keys, SessionTokenV2,
//     one wallet signature for create_session + buy + delegate; the in-memory session key co-signs);
//     spin, settle and claim are then signed and paid ONLY by the session key. Default: the wallet signs.
//   CLAIMER (key name that signs and pays claim; default PLAYER. The asset always goes to the pull's player)
// The ER variant chooses the transaction format for the ER-side transactions (spin, settle):
//   v1   = v1, limits in message.config (no ComputeBudget instructions)
//   v0cb = v0 with SetComputeUnitLimit + SetComputeUnitPrice instructions
//   v0   = v0 without ComputeBudget
import { AccountRole, address, createSolanaRpc, generateKeyPairSigner, type KeyPairSigner, getAddressDecoder, getAddressEncoder, getProgramDerivedAddress, getU64Encoder, type Address, type Instruction } from "@solana/kit";
import {
  fetchMachine,
  fetchMaybePlayerState,
  fetchPull,
  findMachinePda,
  findPlayerStatePda,
  getBuyPullInstructionAsync,
  getClaimInstructionAsync,
  getDelegatePullInstructionAsync,
  getRedeemInstructionAsync,
  getSettleAndClaimInstruction,
  getSettlePullInstruction,
  getSpinInstructionAsync,
  Currency,
  PullStatus,
} from "../clients/js/src/generated/index.ts";
import {
  base,
  DEFAULT_QUEUE,
  EPHEMERAL_QUEUE,
  explorer,
  getTx,
  loadSigner,
  simulate,
  log,
  PROGRAM_ID,
  routerStatus,
  send,
  sleep,
  type AnyRpc,
  type SendOpts,
} from "./lib.ts";
import { createSessionV2Ix, sessionTokenPda } from "./session.ts";
import { escrowPda, topUpEscrowIx } from "./escrow.ts";

const COUNT = Number(process.env.COUNT ?? 1);
const ACTION = process.env.ACTION === "1";
const ASSET_RENT = 2_900_000n; // Core AssetV1 rent (~2.83 M) plus margin
async function assetPda(pull: Address, index: number): Promise<Address> {
  const [a] = await getProgramDerivedAddress({ programAddress: PROGRAM_ID, seeds: [new TextEncoder().encode("asset"), getAddressEncoder().encode(pull), new Uint8Array([index])] });
  return a;
}

const CORE = address("CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d");
const player = await loadSigner(process.env.PLAYER ?? "tequila-gacha-player-1");
const authority = await loadSigner(process.env.AUTHORITY ?? "gacha-deployer");
const [machine] = await findMachinePda({ authority: authority.address, machineId: Number(process.env.MACHINE_ID ?? 1) });
let claimer = process.env.CLAIMER ? await loadSigner(process.env.CLAIMER) : player;
/** Who signs spin / respin / settle_pull: the wallet, or the session key with its token. */
type Spinner = { signer: KeyPairSigner; sessionToken?: Address };
let spinner: Spinner = { signer: player };
const SESSION_TOP_UP = BigInt(process.env.SESSION_TOP_UP ?? 15_000_000); // fees + claim rent (~0.0029 SOL per asset)
const [cmd, a1, a2] = process.argv.slice(2);

async function nextPull(): Promise<{ pull: Address; nonce: bigint }> {
  const [ps] = await findPlayerStatePda({ machine, player: player.address });
  const st = await fetchMaybePlayerState(base, ps);
  const nonce = st.exists ? st.data.nextNonce : 0n;
  // Pull seeds include PlayerState.next_nonce (account data), so Codama does not emit a PDA helper.
  const ae = getAddressEncoder();
  const [pull] = await getProgramDerivedAddress({
    programAddress: PROGRAM_ID,
    seeds: [new TextEncoder().encode("pull"), ae.encode(machine), ae.encode(player.address), getU64Encoder().encode(nonce)],
  });
  return { pull, nonce };
}

async function buyIx(pull: Address): Promise<Instruction> {
  const m = (await fetchMachine(base, machine)).data;
  // max_price = the price read now: a price raised before the buy lands is refused (PriceAboveMax).
  if (process.env.CURRENCY === "usdc") {
    // CURRENCY=usdc: pays price_usdc from the player's USDC ATA to the seller's (classic SPL Token, both ATAs must exist).
    const playerUsdc = await ata(player.address, m.usdcMint);
    const sellerUsdc = await ata(m.seller, m.usdcMint);
    return getBuyPullInstructionAsync({
      player, machine, pull, seller: m.seller, currency: Currency.Usdc, count: COUNT, maxPriceTotal: m.priceUsdc * BigInt(COUNT),
      playerUsdc, sellerUsdc, usdcMint: m.usdcMint, tokenProgram: TOKEN_PROGRAM,
    });
  }
  return getBuyPullInstructionAsync({ player, machine, pull, seller: m.seller, currency: Currency.Sol, count: COUNT, maxPriceTotal: m.priceLamports * BigInt(COUNT) });
}

const TOKEN_PROGRAM = address("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
const ATA_PROGRAM = address("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
async function ata(owner: Address, mint: Address): Promise<Address> {
  const ae = getAddressEncoder();
  const [a] = await getProgramDerivedAddress({ programAddress: ATA_PROGRAM, seeds: [ae.encode(owner), ae.encode(TOKEN_PROGRAM), ae.encode(mint)] });
  return a;
}

async function waitStatus(rpc: AnyRpc, pull: Address, want: PullStatus, label: string, timeoutMs = 150_000) {
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) {
    try {
      const p = (await fetchPull(rpc, pull)).data;
      if (p.status === want) return { p, ms: Date.now() - t0 };
    } catch {}
    await sleep(300);
  }
  throw new Error(`${label}: timeout waiting for status ${PullStatus[want]}`);
}

async function findCallback(rpc: AnyRpc, pull: Address, exclude: string[]): Promise<string | undefined> {
  const sigs = await rpc.getSignaturesForAddress(pull, { limit: 10 }).send();
  for (const s of sigs) {
    if (exclude.includes(s.signature)) continue;
    const t = await getTx(rpc, s.signature);
    if (t?.meta?.logMessages?.some((l: string) => l.includes("Instruction: CallbackSpin"))) return s.signature;
  }
}

function erOpts(variant: string, label: string): SendOpts {
  if (variant === "v0cb") return { version: 0, computeBudget: { limit: 400_000, microLamports: 1n }, label };
  if (variant === "v0") return { version: 0, label };
  return { version: 1, fixedLimits: { computeUnitLimit: 400_000, loadedAccountsDataSizeLimit: 4 * 1024 * 1024 }, label };
}

/** Claims every unclaimed index of the pull (one tx per index, or all in one with CLAIM_TX=one). */
async function claim(pull: Address) {
  const collection = (await fetchMachine(base, machine)).data.collection;
  let p = (await fetchPull(base, pull)).data;
  const ixFor = async (i: number) =>
    getClaimInstructionAsync({ payer: claimer, player: player.address, machine, pull, collection, asset: await assetPda(pull, i) });
  const left = Array.from({ length: p.count - p.claimed }, (_, k) => p.claimed + k);
  const opts = (label: string) => (process.env.BASE_TX ? erOpts(process.env.BASE_TX, label + "(" + process.env.BASE_TX + ")") : { label });
  const sigs: string[] = [];
  if (process.env.CLAIM_TX === "one") sigs.push(await send(base, await Promise.all(left.map(ixFor)), claimer, opts(`claim x${left.length}`)));
  else for (const i of left) sigs.push(await send(base, [await ixFor(i)], claimer, opts(`claim[${i}]`)));
  await reportClaimed(pull, sigs);
}

async function reportClaimed(pull: Address, sigs: string[]) {
  const p = (await fetchPull(base, pull)).data;
  const assets = [];
  for (let i = 0; i < p.claimed; i++) {
    const asset = await assetPda(pull, i);
    assets.push({ index: i, asset, tier: p.tiers[i], allocation: p.allocations[i], rerolled: ((p.rerolled >> i) & 1) === 1, owner: await assetOwner(asset) });
  }
  log({ step: "claimed", pull, status: PullStatus[p.status], count: p.count, claimed: p.claimed, claimer: claimer.address, player: player.address, assets, links: sigs.map((s) => explorer(s)) });
  if (assets.some((a) => a.owner !== player.address)) throw new Error("an asset is not owned by the player");
}

/// AssetV1: key (1) | owner (32).
async function assetOwner(asset: Address): Promise<string> {
  const a = await base.getAccountInfo(asset, { encoding: "base64" }).send();
  const d = Buffer.from(a.value!.data[0], "base64");
  return getAddressDecoder().decode(d.subarray(1, 33));
}

async function erFlow(variant: string) {
  const { pull, nonce } = await nextPull();
  log({ step: "er_start", variant, player: player.address, pull, nonce });
  const buy = await buyIx(pull);
  const del = await getDelegatePullInstructionAsync({ player, machine, pull });
  const pre: Instruction[] = [];
  let escrowSigner: Address = player.address;
  if (process.env.SESSION === "1") {
    const sk = await generateKeyPairSigner();
    const validUntil = BigInt(Math.floor(Date.now() / 1000) + 3600);
    pre.push(await createSessionV2Ix({ wallet: player, sessionSigner: sk, target: PROGRAM_ID, validUntil, topUpLamports: SESSION_TOP_UP }));
    spinner = { signer: sk, sessionToken: await sessionTokenPda(PROGRAM_ID, sk.address, player.address) };
    claimer = sk;
    escrowSigner = sk.address;
    log({ step: "session", sessionKey: sk.address, token: spinner.sessionToken, validUntil, topUp: SESSION_TOP_UP });
  }
  if (ACTION) {
    // The escrow pays each asset's rent in claim_action; top it up to COUNT rents.
    const escrow = await escrowPda(escrowSigner);
    const have = (await base.getBalance(escrow).send()).value;
    const need = ASSET_RENT * BigInt(COUNT) + 1_000_000n;
    if (have < need) pre.push(await topUpEscrowIx(player, escrowSigner, need - have));
    log({ step: "escrow", escrow, have, topUp: have < need ? need - have : 0n });
  }
  const s1 = await send(base, [...pre, buy, del], player, { label: `${pre.length ? "pre(" + pre.length + ")+" : ""}buy_pull(x${COUNT})+delegate_pull` });
  log({ step: "buy+delegate", link: explorer(s1) });

  let st = await routerStatus(pull);
  for (let i = 0; !st.isDelegated && i < 60; i++) {
    await sleep(500);
    st = await routerStatus(pull);
  }
  if (!st.isDelegated || !st.fqdn) throw new Error("router does not report Pull as delegated");
  log({ step: "router", fqdn: st.fqdn, record: st.delegationRecord });
  const er = createSolanaRpc(st.fqdn);
  // Standard getLatestBlockhash against the ER fqdn.
  const bh = await er.getLatestBlockhash().send();
  log({ step: "er_getLatestBlockhash", ok: true, blockhash: bh.value.blockhash });
  await waitStatus(er, pull, PullStatus.Paid, "clone into ER", 30_000);

  const spinIx = await getSpinInstructionAsync({ ...spinner, pull, oracleQueue: EPHEMERAL_QUEUE });
  const settleIx = ACTION
    ? getSettleAndClaimInstruction({ ...spinner, pull, machine, program: PROGRAM_ID })
    : getSettlePullInstruction({ ...spinner, pull });
  // Probe (simulation only): the ER runs the upgraded binary if [spin, settle] is refused with
  // SettleTooEarly (6020); an older binary would settle a fresh request.
  const probe = await simulate(er, [spinIx, settleIx], spinner.signer);
  log({ step: "er_probe_spin+settle", err: probe.err, last_logs: probe.logs?.slice(-3) });
  const s2 = await send(er, [spinIx], spinner.signer, erOpts(variant, `spin(ER,${variant})`));
  const { p, ms } = await waitStatus(er, pull, PullStatus.Fulfilled, "ER callback");
  const cb = await findCallback(er, pull, [s2]);
  log({ step: "er_fulfilled", ms_after_spin_confirm: ms, tiers: p.tiers.slice(0, p.count), spin: explorer(s2, st.fqdn), callback: cb && explorer(cb, st.fqdn) });

  const s3 = await send(er, [settleIx], spinner.signer, erOpts(variant, `${ACTION ? "settle_and_claim" : "settle_pull"}(ER,${variant})`));
  const t0 = Date.now();
  for (;;) {
    const acc = await base.getAccountInfo(pull, { encoding: "base64" }).send();
    if (acc.value?.owner === PROGRAM_ID) break;
    if (Date.now() - t0 > 90_000) throw new Error("Pull did not return to base");
    await sleep(500);
  }
  const baseSigs = await base.getSignaturesForAddress(pull, { limit: 3 }).send();
  log({ step: "settled", ms_to_base: Date.now() - t0, settle: explorer(s3, st.fqdn), base_commit: explorer(baseSigs[0].signature) });
  if (ACTION) {
    let b = (await fetchPull(base, pull)).data;
    for (let i = 0; i < 20 && b.status !== PullStatus.Claimed; i++) { await sleep(1000); b = (await fetchPull(base, pull)).data; }
    if (b.status === PullStatus.Claimed) return reportClaimed(pull, [baseSigs[0].signature]);
    log({ step: "action_did_not_claim", status: PullStatus[b.status], claimed: b.claimed });
  }
  await claim(pull);
}

async function baseFlow(noClaim = false) {
  const { pull, nonce } = await nextPull();
  log({ step: "base_start", player: player.address, pull, nonce });
  const buy = await buyIx(pull);
  const spin = await getSpinInstructionAsync({ signer: player, pull, oracleQueue: DEFAULT_QUEUE });
  const s1 = await send(base, [buy, spin], player, process.env.BASE_TX ? erOpts(process.env.BASE_TX, "buy_pull+spin(base," + process.env.BASE_TX + ")") : { label: "buy_pull+spin(base)" });
  const { p, ms } = await waitStatus(base, pull, PullStatus.Fulfilled, "base callback");
  const cb = await findCallback(base, pull, [s1]);
  log({ step: "base_fulfilled", ms_after_confirm: ms, tiers: p.tiers.slice(0, p.count), buy_spin: explorer(s1), callback: cb && explorer(cb) });
  if (!noClaim) await claim(pull);
}

async function transfer(pull: Address, toName: string, index = 0) {
  const to = (await loadSigner(toName)).address;
  const asset = await assetPda(pull, index);
  const collection = (await fetchMachine(base, machine)).data.collection;
  const ix: Instruction = {
    programAddress: CORE,
    accounts: [
      { address: asset, role: AccountRole.WRITABLE },
      { address: collection, role: AccountRole.READONLY },
      { address: player.address, role: AccountRole.WRITABLE_SIGNER, signer: player } as any,
      { address: CORE, role: AccountRole.READONLY },
      { address: to, role: AccountRole.READONLY },
      { address: CORE, role: AccountRole.READONLY },
      { address: CORE, role: AccountRole.READONLY },
    ],
    data: new Uint8Array([14, 0]), // TransferV1 { compression_proof: None }; authority = payer
  };
  const sig = await send(base, [ix], player, { label: "core_transfer" });
  log({ step: "transferred", asset, to, link: explorer(sig) });
}

async function redeem(pull: Address, index = 0) {
  const ix = await getRedeemInstructionAsync({ owner: player, machine, pull, index, asset: await assetPda(pull, index), collection: (await fetchMachine(base, machine)).data.collection });
  const sig = await send(base, [ix], player, { label: "redeem" });
  log({ step: "redeemed", pull, owner: player.address, link: explorer(sig) });
}

switch (cmd) {
  case "er":
    await erFlow(a1 ?? "v0cb");
    break;
  case "settle": {
    // Recovery: settle a delegated Pull (any state) and claim it if Fulfilled.
    const pull = address(a1);
    const st = await routerStatus(pull);
    if (!st.isDelegated || !st.fqdn) throw new Error("not delegated");
    const er = createSolanaRpc(st.fqdn);
    const s3 = await send(er, [getSettlePullInstruction({ signer: player, pull })], player, erOpts(a2 ?? "v0cb", `settle_pull(ER,${a2 ?? "v0cb"})`));
    log({ step: "settle_sent", settle: explorer(s3, st.fqdn) });
    for (let i = 0; i < 180; i++) {
      const acc = await base.getAccountInfo(pull, { encoding: "base64" }).send();
      if (acc.value?.owner === PROGRAM_ID) break;
      await sleep(500);
    }
    const p = (await fetchPull(base, pull)).data;
    if (p.status === PullStatus.Fulfilled) await claim(pull);
    break;
  }
  case "base":
    await baseFlow(a1 === "noclaim");
    break;
  case "transfer":
    await transfer(address(a1), a2, Number(process.argv[5] ?? 0));
    break;
  case "redeem":
    await redeem(address(a1), Number(a2 ?? 0));
    break;
  case "status": {
    const { pull, nonce } = await nextPull();
    console.log({ player: player.address, nextPull: pull, nonce, balance: (await base.getBalance(player.address).send()).value });
    break;
  }
  default:
    console.error("usage: bun scripts/play.ts <er [v1|v0cb|v0]|base|transfer|redeem|status>");
    process.exit(1);
}
