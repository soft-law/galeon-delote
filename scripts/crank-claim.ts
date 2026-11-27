// Crank: finds the Fulfilled pulls of a machine on base and claims every unclaimed index of each
// (one tx per pull). claim is permissionless: the crank signs and pays rent and fee; the assets go
// to pull.player.
// Usage: bun scripts/crank-claim.ts [--dry]
// Env: CRANK (key name that signs and pays, default gacha-deployer), AUTHORITY, MACHINE_ID
import { getAddressEncoder, getBase58Decoder, getProgramDerivedAddress, type Address } from "@solana/kit";
import {
  fetchMachine,
  findMachinePda,
  getClaimInstructionAsync,
  getPullDecoder,
  PULL_DISCRIMINATOR,
  PullStatus,
} from "../clients/js/src/generated/index.ts";
import { base, explorer, loadSigner, log, PROGRAM_ID, send } from "./lib.ts";

const crank = await loadSigner(process.env.CRANK ?? "gacha-deployer");
const authority = await loadSigner(process.env.AUTHORITY ?? "gacha-deployer");
const [machine] = await findMachinePda({ authority: authority.address, machineId: Number(process.env.MACHINE_ID ?? 1) });
const dry = process.argv.includes("--dry");

// Pull layout: discriminator (8) | machine (32) | player (32) | nonce (8) | status (1).
const STATUS_OFFSET = 8 + 32 + 32 + 8;
const b58 = getBase58Decoder();
const accounts = await base
  .getProgramAccounts(PROGRAM_ID, {
    encoding: "base64",
    filters: [
      { memcmp: { offset: 0n, bytes: b58.decode(PULL_DISCRIMINATOR) as any, encoding: "base58" } },
      { memcmp: { offset: 8n, bytes: machine as any, encoding: "base58" } },
      { memcmp: { offset: BigInt(STATUS_OFFSET), bytes: b58.decode(new Uint8Array([PullStatus.Fulfilled])) as any, encoding: "base58" } },
    ],
  })
  .send();

const collection = (await fetchMachine(base, machine)).data.collection;
log({ step: "crank_scan", machine, crank: crank.address, fulfilled: accounts.length });
const decoder = getPullDecoder();
for (const { pubkey, account } of accounts) {
  const pull = pubkey as Address;
  const p = decoder.decode(Buffer.from(account.data[0], "base64"));
  if (p.status !== PullStatus.Fulfilled) continue;
  if (dry) {
    log({ step: "crank_found", pull, player: p.player, count: p.count, claimed: p.claimed, tiers: p.tiers.slice(0, p.count) });
    continue;
  }
  try {
    const ixs = [];
    for (let i = p.claimed; i < p.count; i++) {
      const [asset] = await getProgramDerivedAddress({ programAddress: PROGRAM_ID, seeds: [new TextEncoder().encode("asset"), getAddressEncoder().encode(pull), new Uint8Array([i])] });
      ixs.push(await getClaimInstructionAsync({ payer: crank, player: p.player, machine, pull, collection, asset }));
    }
    const sig = await send(base, ixs, crank, { label: `crank_claim x${ixs.length}` });
    log({ step: "crank_claimed", pull, player: p.player, link: explorer(sig) });
  } catch (e: any) {
    log({ step: "crank_claim_failed", pull, error: String(e?.message ?? e) });
  }
}
