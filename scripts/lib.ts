// Shared devnet helpers: signers, RPCs, router, sending v0/v1 transactions, logging.
import {
  address,
  appendTransactionMessageInstructions,
  createKeyPairSignerFromBytes,
  createSolanaRpc,
  createTransactionMessage,
  estimateAndSetResourceLimitsFactory,
  estimateResourceLimitsFactory,
  getBase64EncodedWireTransaction,
  getSignatureFromTransaction,
  pipe,
  setTransactionMessageComputeUnitLimit,
  setTransactionMessageComputeUnitPrice,
  setTransactionMessageConfig,
  setTransactionMessageFeePayerSigner,
  setTransactionMessageLifetimeUsingBlockhash,
  signTransactionMessageWithSigners,
  type Address,
  type Instruction,
  type KeyPairSigner,
  type Signature,
} from "@solana/kit";
import { appendFileSync, mkdirSync, readFileSync } from "node:fs";

export const PROGRAM_ID = address("2LVXXXxVUMQPnewQGKpNDb3dRRfhVdbSmhYwNppfuAis");
export const BASE_RPC = process.env.RPC ?? "https://api.devnet.solana.com";
export const ROUTER = process.env.ROUTER ?? "https://devnet-router.magicblock.app/";
export const KEYS = process.env.KEYS ?? "keys";
export const DEFAULT_QUEUE = address("Cuj97ggrhhidhbu39TijNVqE74xvKJ69gDervRUXAxGh");
export const EPHEMERAL_QUEUE = address("5hBR571xnXppuCPveTrctfTU7tJLSN94nq7kv7FRK5Tc");
/// devnet-as (Asia), the validator the ER spike pinned.
export const DEVNET_AS_VALIDATOR = address("MAS1Dt9qreoRMQ14YQuhg8UTZMMzDdKhmkZMECCzk57");
export const DEVNET_USDC = address("4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU");
export const MAGIC_PROGRAM = address("Magic11111111111111111111111111111111111111");
export const MAGIC_CONTEXT = address("MagicContext1111111111111111111111111111111");

export const base = createSolanaRpc(BASE_RPC);
export type AnyRpc = ReturnType<typeof createSolanaRpc>;

export async function loadSigner(name: string): Promise<KeyPairSigner> {
  const path = name.includes("/") ? name : `${KEYS}/${name}.json`;
  return createKeyPairSignerFromBytes(new Uint8Array(JSON.parse(readFileSync(path, "utf8"))));
}

const LOG_DIR = new URL("../runs/", import.meta.url).pathname;
export function log(o: Record<string, unknown>) {
  mkdirSync(LOG_DIR, { recursive: true });
  const line = JSON.stringify({ t: new Date().toISOString(), ...o }, (_, v) => (typeof v === "bigint" ? v.toString() : v));
  console.log(line);
  appendFileSync(`${LOG_DIR}devnet.jsonl`, line + "\n");
}

export async function routerStatus(account: Address): Promise<{ isDelegated: boolean; fqdn?: string; delegationRecord?: unknown }> {
  const res = await fetch(ROUTER, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "getDelegationStatus", params: [account] }),
  });
  const j = (await res.json()) as { result?: { isDelegated: boolean; fqdn?: string } };
  return j.result ?? { isDelegated: false };
}

export type SendOpts = {
  version?: 0 | 1;
  /** v0 only: add SetComputeUnitLimit + SetComputeUnitPrice instructions. */
  computeBudget?: { limit: number; microLamports: bigint };
  /** v1 only: fixed limits instead of simulation (used on the ER). */
  fixedLimits?: { computeUnitLimit: number; loadedAccountsDataSizeLimit: number };
  label: string;
};

export async function send(rpc: AnyRpc, ixs: Instruction[], payer: KeyPairSigner, opts: SendOpts): Promise<Signature> {
  const version = opts.version ?? 1;
  const { value: bh } = await rpc.getLatestBlockhash({ commitment: "confirmed" }).send();
  let msg: any = pipe(
    createTransactionMessage({ version }),
    (m) => setTransactionMessageFeePayerSigner(payer, m),
    (m) => setTransactionMessageLifetimeUsingBlockhash(bh, m),
    (m) => appendTransactionMessageInstructions(ixs, m),
  );
  if (version === 0 && opts.computeBudget) {
    msg = setTransactionMessageComputeUnitLimit(opts.computeBudget.limit, msg);
    msg = setTransactionMessageComputeUnitPrice(opts.computeBudget.microLamports, msg);
  }
  if (version === 1) {
    if (opts.fixedLimits) msg = setTransactionMessageConfig(opts.fixedLimits, msg);
    else msg = await estimateAndSetResourceLimitsFactory(estimateResourceLimitsFactory({ rpc }))(msg);
  }
  const tx = await signTransactionMessageWithSigners(msg);
  const sig = getSignatureFromTransaction(tx);
  const wire = getBase64EncodedWireTransaction(tx);
  const t0 = Date.now();
  try {
    await rpc.sendTransaction(wire, { encoding: "base64", preflightCommitment: "confirmed" }).send();
  } catch (e: any) {
    log({ step: opts.label, version, error: String(e?.message ?? e), logs: e?.context?.logs ?? e?.cause?.context?.logs });
    throw e;
  }
  for (let i = 0; i < 120; i++) {
    const { value } = await rpc.getSignatureStatuses([sig]).send();
    const st = value[0];
    if (st && (st.confirmationStatus === "confirmed" || st.confirmationStatus === "finalized")) {
      if (st.err) {
        const t = await getTx(rpc, sig);
        log({ step: opts.label, version, sig, err: st.err, logs: t?.meta?.logMessages });
        throw new Error(`${opts.label} failed: ${JSON.stringify(st.err, (_, v) => (typeof v === "bigint" ? v.toString() : v))}`);
      }
      const t = await getTx(rpc, sig);
      log({ step: opts.label, version, sig, ms: Date.now() - t0, cu: t?.meta?.computeUnitsConsumed, fee: t?.meta?.fee });
      return sig;
    }
    await sleep(500);
  }
  throw new Error(`${opts.label}: not confirmed in 60 s (${sig})`);
}

/** simulateTransaction of a signed v0 transaction; returns the error and logs (nothing is sent). */
export async function simulate(rpc: AnyRpc, ixs: Instruction[], payer: KeyPairSigner): Promise<{ err: unknown; logs: readonly string[] | null }> {
  const { value: bh } = await rpc.getLatestBlockhash({ commitment: "confirmed" }).send();
  const msg = pipe(
    createTransactionMessage({ version: 0 }),
    (m) => setTransactionMessageFeePayerSigner(payer, m),
    (m) => setTransactionMessageLifetimeUsingBlockhash(bh, m),
    (m) => appendTransactionMessageInstructions(ixs, m),
  );
  const tx = await signTransactionMessageWithSigners(msg);
  const { value } = await rpc
    .simulateTransaction(getBase64EncodedWireTransaction(tx), { encoding: "base64", sigVerify: false, commitment: "confirmed" })
    .send();
  return { err: value.err, logs: value.logs };
}

export async function getTx(rpc: AnyRpc, sig: Signature): Promise<any> {
  try {
    return await rpc
      .getTransaction(sig, { maxSupportedTransactionVersion: 1, commitment: "confirmed", encoding: "json" })
      .send();
  } catch {
    return null;
  }
}

export const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export const explorer = (sig: string, er?: string) =>
  er
    ? `https://explorer.solana.com/tx/${sig}?cluster=custom&customUrl=${encodeURIComponent(er)}`
    : `https://explorer.solana.com/tx/${sig}?cluster=devnet`;
