// Generates one neutral card (SVG) and one metadata JSON per tier.
// Usage: bun assets/gen.ts [baseUri]   (baseUri defaults to a placeholder; see README)
import { writeFileSync } from "node:fs";
import { TIERS } from "./tiers";

const base = process.argv[2] ?? "https://example.invalid/tequila-gacha/";
const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;");

for (const [i, t] of TIERS.entries()) {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="600" height="840" viewBox="0 0 600 840">
  <rect width="600" height="840" rx="32" fill="${t.color}"/>
  <rect x="24" y="24" width="552" height="792" rx="20" fill="none" stroke="#ffffff" stroke-width="4"/>
  <text x="300" y="380" font-family="Helvetica, Arial, sans-serif" font-size="64" font-weight="700" fill="#ffffff" text-anchor="middle">${esc(t.name)}</text>
  <text x="300" y="450" font-family="Helvetica, Arial, sans-serif" font-size="28" fill="#ffffff" text-anchor="middle">Nivel ${i + 1}</text>
  <text x="300" y="760" font-family="Helvetica, Arial, sans-serif" font-size="22" fill="#ffffff" text-anchor="middle">devnet · demostración · sin botella física</text>
</svg>
`;
  writeFileSync(`assets/tiers/tier-${i}.svg`, svg);
  const meta = {
    name: t.name,
    symbol: "TQG",
    description: "devnet · demostración · sin botella física. Premio de prueba del gacha; no representa ningún producto.",
    image: `${base}tier-${i}.svg`,
    attributes: [
      { trait_type: "Nivel", value: t.name },
      { trait_type: "Red", value: "devnet" },
    ],
    properties: { files: [{ uri: `${base}tier-${i}.svg`, type: "image/svg+xml" }], category: "image" },
  };
  writeFileSync(`assets/tiers/tier-${i}.json`, JSON.stringify(meta, null, 2) + "\n");
}
console.log(`wrote ${TIERS.length} tiers with base ${base}`);
