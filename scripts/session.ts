// MagicBlock Session Keys (gpl_session, SessionTokenV2): hand-built create/revoke instructions.
// The token PDA is ["session_token_v2", target_program, session_signer, authority] under KeyspM2ss….
import {
  AccountRole,
  address,
  getAddressEncoder,
  getProgramDerivedAddress,
  type Address,
  type Instruction,
  type TransactionSigner,
} from "@solana/kit";
import { createHash } from "node:crypto";

export const SESSION_PROGRAM = address("KeyspM2ssCJbqUhQ4k7sveSiY4WjnYsrXkC8oDbwde5");
const SYSTEM = address("11111111111111111111111111111111");
const enc = getAddressEncoder();

const disc = (name: string) => createHash("sha256").update(`global:${name}`).digest().subarray(0, 8);

export async function sessionTokenPda(target: Address, sessionSigner: Address, authority: Address): Promise<Address> {
  const [pda] = await getProgramDerivedAddress({
    programAddress: SESSION_PROGRAM,
    seeds: [Buffer.from("session_token_v2"), enc.encode(target), enc.encode(sessionSigner), enc.encode(authority)],
  });
  return pda;
}

/** create_session_v2(top_up, valid_until, lamports). The wallet signs as authority and fee payer
 *  (pays the token rent and the top-up); the session key co-signs. */
export async function createSessionV2Ix(o: {
  wallet: TransactionSigner;
  sessionSigner: TransactionSigner;
  target: Address;
  validUntil: bigint;
  topUpLamports?: bigint;
}): Promise<Instruction> {
  const token = await sessionTokenPda(o.target, o.sessionSigner.address, o.wallet.address);
  const parts: Buffer[] = [disc("create_session_v2")];
  parts.push(o.topUpLamports ? Buffer.from([1, 1]) : Buffer.from([1, 0])); // Some(top_up)
  const vu = Buffer.alloc(9);
  vu[0] = 1;
  vu.writeBigInt64LE(o.validUntil, 1);
  parts.push(vu);
  if (o.topUpLamports) {
    const l = Buffer.alloc(9);
    l[0] = 1;
    l.writeBigUInt64LE(o.topUpLamports, 1);
    parts.push(l);
  } else parts.push(Buffer.from([0]));
  return {
    programAddress: SESSION_PROGRAM,
    accounts: [
      { address: token, role: AccountRole.WRITABLE },
      { address: o.sessionSigner.address, role: AccountRole.WRITABLE_SIGNER, signer: o.sessionSigner } as any,
      { address: o.wallet.address, role: AccountRole.WRITABLE_SIGNER, signer: o.wallet } as any,
      { address: o.wallet.address, role: AccountRole.READONLY_SIGNER, signer: o.wallet } as any,
      { address: o.target, role: AccountRole.READONLY },
      { address: SYSTEM, role: AccountRole.READONLY },
    ],
    data: new Uint8Array(Buffer.concat(parts)),
  };
}

/** revoke_session_v2: closes the token, rent back to the fee payer. While the token is still valid
 *  the authority must sign; once expired anyone can send it. */
export function revokeSessionV2Ix(o: { token: Address; authority: Address; authoritySigner?: TransactionSigner; feePayer: Address }): Instruction {
  return {
    programAddress: SESSION_PROGRAM,
    accounts: [
      { address: o.token, role: AccountRole.WRITABLE },
      { address: o.feePayer, role: AccountRole.WRITABLE },
      o.authoritySigner
        ? ({ address: o.authority, role: AccountRole.READONLY_SIGNER, signer: o.authoritySigner } as any)
        : { address: o.authority, role: AccountRole.READONLY },
      { address: SYSTEM, role: AccountRole.READONLY },
    ],
    data: new Uint8Array(disc("revoke_session_v2")),
  };
}
