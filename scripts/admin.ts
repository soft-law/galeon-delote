// Admin on devnet with the generated client.
// Usage: bun scripts/admin.ts <setup|init|tiers|open|close|prices|validator|show> [args]
//   setup                      init + tiers + open in that order (definitive tiers and their IPFS URIs)
//   init                       machine MACHINE_ID (default 1) with SELLER (default: authority),
//                              price 0.01 SOL and 1 USDC (PRICE_LAMPORTS / PRICE_USDC override)
//   tiers                      writes assets/tiers.ts tiers and URIs (Draft only)
//   prices <lamports> <usdc>   set_prices
//   validator <pubkey>         set_er_validator
// Env: AUTHORITY (key name in KEYS, default gacha-deployer), MACHINE_ID, SELLER, COLLECTION_NAME,
//   COLLECTION_URI (default: the first tier's JSON), PRICE_LAMPORTS, PRICE_USDC,
//   METADATA_BASE (optional: overrides the per-tier URIs with METADATA_BASE + tier-i.json)
// Definitive machine (only with the final program), e.g.:
//   MACHINE_ID=2 bun scripts/admin.ts setup
//   -> tiers Blanco 40 / Añejo 25 / Mezcal 10 / Moneda de plata 3 with their Pinata URIs (assets/tiers.ts),
//      0.01 SOL / 1 USDC
import { address } from "@solana/kit";
import {
  fetchMachine,
  findMachinePda,
  getCloseInstruction,
  getInitMachineInstructionAsync,
  getOpenInstruction,
  getSetErValidatorInstruction,
  getSetPricesInstruction,
  getSetTiersInstruction,
} from "../clients/js/src/generated/index.ts";
import { TIERS } from "../assets/tiers.ts";
import { base, DEVNET_AS_VALIDATOR, DEVNET_USDC, explorer, loadSigner, log, send } from "./lib.ts";

const authority = await loadSigner(process.env.AUTHORITY ?? "gacha-deployer");
const machineId = Number(process.env.MACHINE_ID ?? 1);
const [machine] = await findMachinePda({ authority: authority.address, machineId });
const META = process.env.METADATA_BASE;
const tierUri = (i: number) => (META ? `${META}tier-${i}.json` : TIERS[i].uri);
const COLLECTION_URI = process.env.COLLECTION_URI ?? (META ? `${META}collection.json` : TIERS[0].uri);
const [cmd, a1, a2] = process.argv.slice(2);

const dec = (b: Uint8Array) => new TextDecoder().decode(b.slice(0, b.indexOf(0) < 0 ? b.length : b.indexOf(0)));

async function show() {
  const m = (await fetchMachine(base, machine)).data;
  console.log(
    JSON.stringify(
      {
        machine,
        authority: m.authority,
        seller: m.seller,
        collection: m.collection,
        status: m.status,
        priceLamports: m.priceLamports,
        priceUsdc: m.priceUsdc,
        usdcMint: m.usdcMint,
        erValidator: m.erValidator,
        pending: m.pending,
        tiers: m.tiers.slice(0, m.tierCount).map((t) => ({ name: dec(t.name), cap: t.cap, assigned: t.assigned, uri: dec(t.uri) })),
      },
      (_, v) => (typeof v === "bigint" ? v.toString() : v),
      1,
    ),
  );
}

async function init() {
  const ix = await getInitMachineInstructionAsync({
    authority,
    machineId,
    seller: address(process.env.SELLER ?? authority.address),
    usdcMint: DEVNET_USDC,
    priceLamports: BigInt(process.env.PRICE_LAMPORTS ?? 10_000_000),
    priceUsdc: BigInt(process.env.PRICE_USDC ?? 1_000_000),
    erValidator: DEVNET_AS_VALIDATOR,
    collectionName: process.env.COLLECTION_NAME ?? "Galeón d'Elote · Barriles sorpresa (devnet)",
    collectionUri: COLLECTION_URI,
  });
  const sig = await send(base, [ix], authority, { label: "init_machine" });
  log({ step: "init_machine", machine, machineId, link: explorer(sig) });
}

async function tiers() {
  const t = TIERS.map((t, i) => ({ cap: t.cap, name: t.name, uri: tierUri(i) }));
  for (const x of t) if (new TextEncoder().encode(x.uri).length > 128 || new TextEncoder().encode(x.name).length > 32) throw new Error(`tier too long: ${x.name}`);
  const sig = await send(base, [getSetTiersInstruction({ authority, machine, start: 0, tiers: t })], authority, { label: "set_tiers" });
  log({ step: "set_tiers", tiers: t, link: explorer(sig) });
}

switch (cmd) {
  case "setup": {
    await init();
    await tiers();
    const sig = await send(base, [getOpenInstruction({ authority, machine })], authority, { label: "open" });
    log({ step: "open", link: explorer(sig) });
    break;
  }
  case "init":
    await init();
    break;
  case "tiers":
    await tiers();
    break;
  case "open":
  case "close": {
    const ix = cmd === "open" ? getOpenInstruction({ authority, machine }) : getCloseInstruction({ authority, machine });
    const sig = await send(base, [ix], authority, { label: cmd });
    log({ step: cmd, link: explorer(sig) });
    break;
  }
  case "prices": {
    const sig = await send(base, [getSetPricesInstruction({ authority, machine, priceLamports: BigInt(a1), priceUsdc: BigInt(a2) })], authority, { label: "set_prices" });
    log({ step: "set_prices", link: explorer(sig) });
    break;
  }
  case "validator": {
    const sig = await send(base, [getSetErValidatorInstruction({ authority, machine, erValidator: address(a1) })], authority, { label: "set_er_validator" });
    log({ step: "set_er_validator", link: explorer(sig) });
    break;
  }
  case "show":
    break;
  default:
    console.error("usage: bun scripts/admin.ts <setup|init|tiers|open|close|prices|validator|show>");
    process.exit(1);
}
await show();
