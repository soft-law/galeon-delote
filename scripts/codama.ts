// IDL (anchor-cli 1.2.1 `idl build`) -> Codama JS client for @solana/kit 8.
import { createFromRoot } from "codama";
import { rootNodeFromAnchor } from "@codama/nodes-from-anchor";
import { renderVisitor } from "@codama/renderers-js";
import { readFileSync } from "node:fs";

const idl = JSON.parse(readFileSync("idl/tequila_gacha.json", "utf8"));
const codama = createFromRoot(rootNodeFromAnchor(idl));
await codama.accept(renderVisitor("clients/js", { deleteFolderBeforeRendering: true, formatCode: true }));
console.log("client written to clients/js/src/generated");
