#!/usr/bin/env node
/** Generate a candidate-literal inventory, never inferred rewrite history or severity. */
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  BANNED_TERMS,
  frontendErrorSites,
  humanCopyBannedTerms,
  inventory,
  isHumanCopy,
} from "./error-inventory.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const data = await inventory();
const frontendErrors = await frontendErrorSites();
const cell = (text) => String(text).replace(/\n/g, " ").replace(/\|/g, "\\|").trim();
function table(entries) {
  const seen = new Set();
  const rows = entries.filter((entry) => {
    if (seen.has(entry.text)) return false;
    seen.add(entry.text);
    return true;
  }).map((entry) => [
    `${entry.file}:${entry.line}`,
    entry.text,
    isHumanCopy(entry.text) ? humanCopyBannedTerms(entry.text).join(", ") : "",
  ]);
  return [
    "| Source | Extracted literal | Vocabulary review candidates |",
    "| --- | --- | --- |",
    ...rows.map((row) => `| ${row.map(cell).join(" | ")} |`),
  ].join("\n");
}

const document = `# Error copy inventory

Generated from current sources by \`node scripts/generate-error-docs.mjs\`.
This is a heuristic candidate-literal inventory, not proof that a string reaches
a user, is translated, or has a particular runtime severity. Extraction can
include code, tests and interpolation fragments; review candidates in context.
No replacement history is inferred by pairing unrelated added/deleted strings.

## Runtime error policy

- **Blocking:** the requested action could not complete. Display its explanation
  even when **Show technical error details** is off.
- **Recoverable:** reading or the main action can continue. Record the failure;
  display it only when technical details are enabled.
- Lifecycle suspension is a typed gate, not an error banner.
- The severity argument at the actual call site is authoritative. Behavioral
  regressions exercise visibility, suspension and retry; this inventory does not.
- Technical diagnostics may retain developer detail. Normal copy should explain
  what failed and the available recovery, rather than internal mechanisms.

Vocabulary review terms: ${BANNED_TERMS.join(", ")}.
An extraction hit is not automatically a user-visible defect or certification
failure. Command dispatch also applies \`user_error::plain\`.

## Rust candidate literals

${table(data.rust)}

## Frontend error and notice candidates

${table(frontendErrors)}

## Native prompt candidates

${table(data.native)}
`;

await fs.writeFile(path.join(root, "docs/ERRORS.md"), document);
console.log(JSON.stringify({
  rustCandidates: data.rust.length,
  frontendCandidates: frontendErrors.length,
  nativeCandidates: data.native.length,
  bytes: document.length,
}, null, 2));
