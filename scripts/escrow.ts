// DLP ephemeral balance ("escrow") ["balance", authority, 255]: pays the asset rent of the
// post-undelegate claim_action scheduled by settle_and_claim.
import { AccountRole, address, getAddressEncoder, getProgramDerivedAddress, type Address, type Instruction, type TransactionSigner } from "@solana/kit";

const DLP = address("DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh");
const SYSTEM = address("11111111111111111111111111111111");
const ae = getAddressEncoder();

export async function escrowPda(authority: Address): Promise<Address> {
  const [pda] = await getProgramDerivedAddress({ programAddress: DLP, seeds: [Buffer.from("balance"), ae.encode(authority), new Uint8Array([255])] });
  return pda;
}
/** DLP TopUpEphemeralBalance is discriminator 9 (u64 LE), args { amount: u64, index: u8 }. */
export async function topUpEscrowIx(payer: TransactionSigner, authority: Address, lamports: bigint): Promise<Instruction> {
  const d = Buffer.alloc(17);
  d.writeBigUInt64LE(9n, 0);
  d.writeBigUInt64LE(lamports, 8);
  d[16] = 255;
  return {
    programAddress: DLP,
    accounts: [
      { address: payer.address, role: AccountRole.WRITABLE_SIGNER, signer: payer } as any,
      { address: authority, role: AccountRole.READONLY },
      { address: await escrowPda(authority), role: AccountRole.WRITABLE },
      { address: SYSTEM, role: AccountRole.READONLY },
    ],
    data: new Uint8Array(d),
  };
}
