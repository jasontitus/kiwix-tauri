#!/usr/bin/env node
/** Generate a candidate-literal inventory, never inferred rewrite history or severity.
 *
 * Usage:
 *   node scripts/generate-error-docs.mjs           rewrite docs/ERRORS.md
 *   node scripts/generate-error-docs.mjs --check   compare docs/ERRORS.md with the
 *                                                  current sources; never writes
 *   node scripts/generate-error-docs.mjs --help    print this usage; never writes
 *
 * Any other argument is an error and nothing is written.
 */
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
const DOCUMENT = "docs/ERRORS.md";
const USAGE = `Usage: node scripts/generate-error-docs.mjs [--check]

  (no argument)  rewrite ${DOCUMENT} from the current sources
  --check        compare ${DOCUMENT} with the current sources and exit 1 if it
                 is out of date; never writes
  --help         print this text; never writes`;

const cell = (text) => String(text).replace(/\n/g, " ").replace(/\|/g, "\\|").trim();
const TABLE_HEADER = [
  "| Source | Extracted literal | Vocabulary review candidates |",
  "| --- | --- | --- |",
];

/**
 * Rows name the source file only and are ordered by file, then literal. A line
 * number or source-order row would change on every unrelated edit above it and
 * make `--check` fail for changes that touch no error copy; directory listing
 * order differs between hosts.
 */
function table(entries) {
  const byUnit = (left, right) => (left < right ? -1 : left > right ? 1 : 0);
  const seen = new Set();
  const rows = entries
    .map((entry) => ({ file: entry.file.split(path.sep).join("/"), text: entry.text }))
    .sort((left, right) => byUnit(left.file, right.file) || byUnit(left.text, right.text))
    .filter((entry) => {
      if (seen.has(entry.text)) return false;
      seen.add(entry.text);
      return true;
    })
    .map((entry) => [
      entry.file,
      entry.text,
      isHumanCopy(entry.text) ? humanCopyBannedTerms(entry.text).join(", ") : "",
    ]);
  return [...TABLE_HEADER, ...rows.map((row) => `| ${row.map(cell).join(" | ")} |`)].join("\n");
}

/** The whole document for the given inventories; pure, so it can be tested. */
export function renderDocument({ rust, frontendErrors, native }) {
  return `# Error copy inventory

Generated from current sources by \`node scripts/generate-error-docs.mjs\`;
\`node scripts/generate-error-docs.mjs --check\` reports whether it is current.
This is a heuristic candidate-literal inventory, not proof that a string reaches
a user, is translated, or has a particular runtime severity. Extraction can
include code, tests and interpolation fragments; review candidates in context.
No replacement history is inferred by pairing unrelated added/deleted strings.
The Source column names the file that holds the literal, without a line number;
a literal found in several files is listed once, under the first of them.

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

${table(rust)}

## Frontend error and notice candidates

${table(frontendErrors)}

## Native prompt candidates

${table(native)}
`;
}

/** Read the current sources and render the document without writing anything. */
export async function buildDocument() {
  const data = await inventory();
  const frontendErrors = await frontendErrorSites();
  return {
    document: renderDocument({ rust: data.rust, frontendErrors, native: data.native }),
    counts: {
      rustCandidates: data.rust.length,
      frontendCandidates: frontendErrors.length,
      nativeCandidates: data.native.length,
    },
  };
}

const dataRows = (document) =>
  document.split("\n").filter((line) => line.startsWith("| ") && !TABLE_HEADER.includes(line));

/**
 * Compare the committed document with the generated one. Returns `null` when
 * they are identical, otherwise a short description of the difference.
 */
export function describeDifference(existing, generated, limit = 10) {
  if (existing === generated) return null;
  const committed = new Set(dataRows(existing));
  const current = new Set(dataRows(generated));
  const missing = [...current].filter((row) => !committed.has(row));
  const stale = [...committed].filter((row) => !current.has(row));
  const lines = [`${DOCUMENT} is out of date with the current sources.`];
  const sample = (title, rows) => {
    if (!rows.length) return;
    lines.push(`${title}: ${rows.length}`);
    for (const row of rows.slice(0, limit)) lines.push(`  ${row.length > 200 ? `${row.slice(0, 200)}…` : row}`);
    if (rows.length > limit) lines.push(`  … and ${rows.length - limit} more`);
  };
  sample("Rows the current sources produce that the file lacks", missing);
  sample("Rows in the file that the current sources no longer produce", stale);
  if (!missing.length && !stale.length)
    lines.push("The rows match; the introduction, row order or duplicate rows differ.");
  lines.push("Regenerate it with `npm run docs:errors` and commit the result.");
  return lines.join("\n");
}

/** `write`, `check` or `help`; anything unrecognized throws with the usage text. */
export function parseArguments(args) {
  if (args.length === 0) return "write";
  if (args.length === 1 && args[0] === "--check") return "check";
  if (args.length === 1 && (args[0] === "--help" || args[0] === "-h")) return "help";
  throw new Error(`Unrecognized arguments: ${args.join(" ")}\n${USAGE}`);
}

async function main(args) {
  let mode;
  try {
    mode = parseArguments(args);
  } catch (error) {
    console.error(error.message);
    return 2;
  }
  if (mode === "help") {
    console.log(USAGE);
    return 0;
  }
  const target = path.join(root, DOCUMENT);
  const { document, counts } = await buildDocument();
  if (mode === "check") {
    let existing;
    try {
      existing = await fs.readFile(target, "utf8");
    } catch (error) {
      console.error(`Cannot read ${DOCUMENT}: ${error.message}`);
      return 1;
    }
    const difference = describeDifference(existing, document);
    if (difference) {
      console.error(difference);
      return 1;
    }
    console.log(`${DOCUMENT} matches the current sources.`);
    return 0;
  }
  await fs.writeFile(target, document);
  console.log(JSON.stringify({ ...counts, bytes: document.length }, null, 2));
  return 0;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  process.exitCode = await main(process.argv.slice(2));
}
