// Tiers of the definitive machine ("Barriles sorpresa", Galeón d'Elote): name (<= 32 bytes), cap
// (units) and the approved metadata URI (Pinata IPFS, one CID per tier, <= 128 bytes; CIDs in
// (private notes)). Used by scripts/admin.ts and assets/gen.ts.
// Machine #1 (test) keeps its own on-chain tiers.
const IPFS = "https://gateway.pinata.cloud/ipfs/";
export const TIERS = [
  { name: "Blanco", cap: 40, color: "#5b7083", uri: `${IPFS}bafkreidamzmief43d654flxq35usc4gdzszagokhjrkoqrxeyl5yzfn3d4` },
  { name: "Añejo", cap: 25, color: "#6b3e26", uri: `${IPFS}bafkreih2dvt6poprmbwdgbtejpo3qwwg5d6i5stv2dhsehqjyduhlpvpdi` },
  { name: "Mezcal", cap: 10, color: "#4f5d2f", uri: `${IPFS}bafkreiekn2oeqwk22pwxbfbf7zcpn5ubtsypbbizl26rjw56bjb3skwwuy` },
  { name: "Moneda de plata", cap: 3, color: "#8c8c8c", uri: `${IPFS}bafkreidrdp54jxwfzqogtwxsur3nhhxfhfiyeqylige77e7dcwex7fb46i` },
];
