#!/usr/bin/env node
/** Extract the strings that can reach a person and audit them for internal jargon.
 *
 * Usage:
 *   node scripts/error-inventory.mjs           human summary of the inventory
 *   node scripts/error-inventory.mjs --json    machine-readable entries
 *
 * The extraction is deliberately conservative: only literals used where an
 * error, warning, notice or dialog message is built count as user-facing.
 * Machine identifiers, JSON keys, SQL, cookies and log lines never enter the
 * inventory. `docs/ERRORS.md` documents the extracted source inventory;
 * behavioral tests separately verify error severity and visibility.
 */
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/** Words that describe the implementation, never the person's next step. */
export const BANNED_TERMS = [
  "database",
  "record",
  "ipc",
  "snapshot",
  "generation",
  "authority",
  "retired",
  "installation id",
  "uuid",
  "handoff",
  "quarantine",
  "managed storage",
  "token",
  "shared archive",
  "schema",
];

export function bannedTermsIn(text) {
  const lower = text.toLowerCase();
  return BANNED_TERMS.filter((term) => lower.includes(term));
}

/** Machine codes, JSON keys and identifiers are not sentences for a person. */
export function isHumanCopy(text) {
  // Native copy interpolates API values; check only the written sentence.
  const trimmed = text.split("${")[0].trim();
  if (!trimmed || !trimmed.includes(" ")) return false;
  if (/^[A-Z][A-Z0-9_.\-]*$/.test(trimmed)) return false;
  if (/^[a-z][A-Za-z0-9_.\-]*$/.test(trimmed)) return false;
  if (trimmed.includes("__")) return false;
  // A native interpolation names API values, not output copy.
  if (/^\$\{/.test(trimmed) || trimmed.includes("${")) {
    const withoutExpressions = trimmed.replace(/\$\{[^}]*\}/g, "");
    if (!/[a-z]{3,}\s+[a-z]{3,}/i.test(withoutExpressions)) return false;
  }
  return /[a-z]{2}/i.test(trimmed);
}

export function humanCopyBannedTerms(text) {
  if (!isHumanCopy(text)) return [];
  return bannedTermsIn(text.split("${")[0]);
}

const RUST_ERROR_MARKERS = [
  "Err(",
  ".ok_or(",
  ".ok_or_else(",
  "bail!(",
  ".context(",
  ".with_context(",
  "warnings.push(",
  "errors.push(",
  "note_startup_warning(",
  "failure.record(",
  "notice(",
];

/** One pass over Rust source: comments removed, literals recorded. */
export function scanRust(source) {
  const literals = [];
  let code = "";
  let index = 0;
  const lineAt = (offset) => source.slice(0, offset).split("\n").length;
  while (index < source.length) {
    const char = source[index];
    if (char === "/" && source[index + 1] === "/") {
      while (index < source.length && source[index] !== "\n") index += 1;
      continue;
    }
    if (char === "/" && source[index + 1] === "*") {
      index += 2;
      while (index < source.length && !(source[index] === "*" && source[index + 1] === "/")) index += 1;
      index += 2;
      continue;
    }
    // Raw string: r"...", r#"..."#, r##"..."##.
    if (char === "r" && /^r#*"/.test(source.slice(index))) {
      const hashes = /^r(#*)"/.exec(source.slice(index))[1];
      const close = '"' + hashes;
      const start = index + 2 + hashes.length;
      const end = source.indexOf(close, start);
      if (end >= 0) {
        literals.push({ line: lineAt(index), text: source.slice(start, end) });
        code += source.slice(index, end + close.length);
        index = end + close.length;
        continue;
      }
    }
    if (char === '"') {
      let end = index + 1;
      let text = "";
      while (end < source.length) {
        const next = source[end];
        if (next === "\\") {
          text += source.slice(end, end + 2);
          end += 2;
        } else if (next === '"') {
          break;
        } else {
          text += next;
          end += 1;
        }
      }
      literals.push({ line: lineAt(index), text });
      code += source.slice(index, end + 1);
      index = end + 1;
      continue;
    }
    // Char literal: 'x', '\n', '\''. A lifetime ('a) has no closing quote.
    if (char === "'") {
      if (source[index + 1] === "\\") {
        const close = source.indexOf("'", index + 2);
        if (close > 0 && close - index <= 12) {
          code += source.slice(index, close + 1);
          index = close + 1;
          continue;
        }
      } else if (source[index + 2] === "'") {
        code += source.slice(index, index + 3);
        index += 3;
        continue;
      }
    }
    code += char;
    index += 1;
  }
  return { code, literals };
}

/** Drop inline test modules so fixtures never dictate user-facing copy. */
function stripTestModules(source) {
  const marker = "#[cfg(test)]";
  let result = source;
  let at = result.indexOf(marker);
  while (at >= 0) {
    const brace = result.indexOf("{", at);
    if (brace < 0) break;
    let depth = 0;
    let end = brace;
    for (; end < result.length; end += 1) {
      if (result[end] === "{") depth += 1;
      else if (result[end] === "}") {
        depth -= 1;
        if (depth === 0) {
          end += 1;
          break;
        }
      }
    }
    result = result.slice(0, at) + result.slice(end);
    at = result.indexOf(marker);
  }
  return result;
}

async function walk(dir) {
  const entries = await fs.readdir(dir, { withFileTypes: true });
  return (
    await Promise.all(
      entries.map((entry) =>
        entry.isDirectory()
          ? walk(path.join(dir, entry.name))
          : [path.join(dir, entry.name)],
      ),
    )
  ).flat();
}

/** Rust error, warning, notice and dialog literals. */
export async function rustInventory() {
  const files = (await walk(path.join(root, "src-tauri/src"))).filter(
    (file) =>
      file.endsWith(".rs") &&
      !/(^|\/)(.*_tests|tests)\.rs$/.test(file),
  );
  const entries = [];
  for (const file of files) {
    const relative = path.relative(root, file);
    const source = stripTestModules(await fs.readFile(file, "utf8"));
    const { code, literals } = scanRust(source);
    const lines = code.split("\n");
    for (const literal of literals) {
      const start = Math.max(0, literal.line - 4);
      const context = lines.slice(start, literal.line).join("\n");
      const own = lines[literal.line - 1] ?? "";
      if (/^\s*eprintln!|^\s*println!|^\s*log::|^\s*debug!|^\s*assert/.test(own)) continue;
      const dialog = /error\s*\(\s*StatusCode|mobile_labels|Localizer::/.test(context);
      if (!RUST_ERROR_MARKERS.some((marker) => context.includes(marker)) && !dialog) continue;
      const contextKind = /warnings\.push\(|note_startup_warning\(|notice\(|diagnostics::record\(/.test(context)
        ? "warning"
        : "error";
      entries.push({ file: relative, line: literal.line, text: literal.text, contextKind });
    }
  }
  return entries;
}

/** English strings compiled into the frontend catalog (source of translations). */
export async function localeSource() {
  const directory = path.join(root, "src/locales");
  const values = [];
  for (const name of await fs.readdir(directory)) {
    if (!/^extra-en-.*\.json$/.test(name)) continue;
    const data = JSON.parse(await fs.readFile(path.join(directory, name), "utf8"));
    for (const [key, value] of Object.entries(data)) {
      values.push({ file: "src/locales/" + name, text: key });
      if (typeof value === "string" && value !== key)
        values.push({ file: "src/locales/" + name, text: value });
    }
  }
  return values;
}

/** Copy passed through t(), tPlural(), thrown errors and JSX text. */
export async function frontendInventory() {
  const files = (await walk(path.join(root, "src"))).filter(
    (file) => /\.(ts|tsx|js|jsx)$/.test(file) && !file.endsWith("i18n.tsx"),
  );
  const entries = [];
  for (const file of files) {
    const relative = path.relative(root, file);
    const source = await fs.readFile(file, "utf8");
    const lines = source.split("\n");
    for (const [index, line] of lines.entries()) {
      if (/^\s*(\/\/|\*)/.test(line)) continue;
      for (const match of line.matchAll(
        /\b(?:t|tPlural)\(\s*("(?:[^"\\]|\\.)*")/g,
      ))
        entries.push({ file: relative, line: index + 1, text: JSON.parse(match[1]) });
      for (const match of line.matchAll(
        /(?:new Error|report|setError|setInitialError|setFailure)\(\s*("(?:[^"\\]|\\.)*")/g,
      ))
        entries.push({ file: relative, line: index + 1, text: JSON.parse(match[1]) });
      for (const match of line.matchAll(/>([^<>{}\n]*\p{L}[^<>{}\n]*)</gu))
        entries.push({ file: relative, line: index + 1, text: match[1].trim() });
    }
  }
  return entries;
}

/** Native copy the shell shows verbatim (Android has no catalog bridge). */
export async function nativeInventory() {
  const targets = [
    "src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt",
    "src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt",
  ];
  const entries = [];
  for (const relative of targets) {
    const source = await fs.readFile(path.join(root, relative), "utf8");
    for (const [index, line] of source.split("\n").entries()) {
      if (/^\s*\/\//.test(line)) continue;
      for (const match of line.matchAll(/"((?:[^"\\]|\\.){12,})"/g)) {
        const text = match[1];
        if (/^[A-Z0-9_]+$/.test(text)) continue;
        if (!/\s/.test(text)) continue;
        entries.push({ file: relative, line: index + 1, text });
      }
    }
  }
  return entries;
}

/** Frontend sites that surface a failure, with their declared severity. */
export async function frontendErrorSites() {
  const files = (await walk(path.join(root, "src"))).filter(
    (file) => /\.(ts|tsx)$/.test(file) && !file.endsWith("i18n.tsx"),
  );
  const entries = [];
  for (const file of files) {
    const relative = path.relative(root, file);
    const source = await fs.readFile(file, "utf8");
    for (const [index, line] of source.split("\n").entries()) {
      if (!/(new Error|report\(|onError\(|setError\(|setInitialError\(|setFailure\(|confirmAction\()/.test(line))
        continue;
      for (const match of line.matchAll(
        /\b(?:t|tPlural)\(\s*("(?:[^"\\]|\\.)*")/g,
      ))
        entries.push({ file: relative, line: index + 1, text: JSON.parse(match[1]) });
      for (const match of line.matchAll(/"(?:[^"\\]|\\.){12,}"/g)) {
        const text = JSON.parse(match[0]);
        if (!/\s/.test(text) || /^[a-z][A-Za-z0-9_.]*$/.test(text)) continue;
        entries.push({ file: relative, line: index + 1, text });
      }
    }
  }
  return entries;
}

export async function inventory() {
  return {
    rust: await rustInventory(),
    locale: await localeSource(),
    frontend: await frontendInventory(),
    native: await nativeInventory(),
  };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const data = await inventory();
  const all = Object.entries(data).flatMap(([kind, entries]) =>
    entries.map((entry) => ({ kind, ...entry })),
  );
  if (process.argv.includes("--json")) {
    console.log(JSON.stringify(all, null, 2));
  } else {
    const offenders = all.filter((entry) => humanCopyBannedTerms(entry.text).length);
    console.log(
      JSON.stringify(
        {
          counts: Object.fromEntries(
            Object.entries(data).map(([kind, entries]) => [kind, entries.length]),
          ),
          unique: new Set(all.map((entry) => entry.text)).size,
          offenders: offenders.length,
        },
        null,
        2,
      ),
    );
    for (const entry of offenders)
      console.log(
        `${entry.file}:${entry.line ?? ""} [${humanCopyBannedTerms(entry.text).join(",")}] ${entry.text.replace(/\n/g, " ").slice(0, 180)}`,
      );
  }
}
