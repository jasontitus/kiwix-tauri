// SPDX-License-Identifier: GPL-3.0-or-later
// Fail when first-party Rust adds undocumented unsafe code or exceeds the
// reviewed per-file census. The baseline is a per-file ceiling: documented
// blocks that fit existing headroom pass; a new file or higher count needs a
// reviewed baseline update. A small Rust-aware scanner blanks comments and
// string/char literals before counting tokens.
import { readFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";

let baselinePath = "scripts/unsafe-census-baseline.json";
let printOnly = false;
const roots = [];
for (let index = 2; index < process.argv.length; index += 1) {
  const argument = process.argv[index];
  if (argument === "--baseline") baselinePath = process.argv[++index];
  else if (argument === "--print") printOnly = true;
  else roots.push(argument);
}
if (roots.length === 0) roots.push("src-tauri/src", "kiwix-mcp/src");

const repoRoot = process.cwd();
const relative = (file) => path.relative(repoRoot, file).split(path.sep).join("/");

function rustFiles(directory) {
  const found = [];
  for (const name of readdirSync(directory)) {
    const file = path.join(directory, name);
    if (statSync(file).isDirectory()) found.push(...rustFiles(file));
    else if (name.endsWith(".rs")) found.push(file);
  }
  return found;
}

function rawStringEnd(source, start) {
  let cursor = start;
  if (source[cursor] === "b" && source[cursor + 1] === "r") cursor += 2;
  else if (source[cursor] === "r" && source[cursor + 1] === "b") cursor += 2;
  else if (source[cursor] === "r") cursor += 1;
  else return null;
  let hashes = 0;
  while (source[cursor] === "#") { hashes += 1; cursor += 1; }
  if (source[cursor] !== '"') return null;
  cursor += 1;
  while (cursor < source.length) {
    if (source[cursor] === '"') {
      let end = cursor + 1;
      let matched = 0;
      while (matched < hashes && source[end] === "#") { matched += 1; end += 1; }
      if (matched === hashes) return end;
    }
    cursor += 1;
  }
  return source.length;
}

function charEnd(source, start) {
  let cursor = start + 1;
  if (source[cursor] === "\\") {
    if (source[cursor + 1] === "u" && source[cursor + 2] === "{") {
      cursor += 3;
      while (cursor < source.length && source[cursor] !== "}") cursor += 1;
      cursor += 1;
    } else {
      cursor += 2;
    }
  } else {
    cursor += 1;
  }
  return source[cursor] === "'" ? cursor + 1 : null;
}

// Blank comments and literal contents, preserving newlines and positions.
function scan(source) {
  const out = source.split("");
  const comments = [];
  const blank = (start, end) => {
    for (let index = start; index < end; index += 1) {
      if (out[index] !== "\n") out[index] = " ";
    }
  };
  let index = 0;
  while (index < source.length) {
    const char = source[index];
    const next = source[index + 1];
    if (char === "/" && next === "/") {
      const start = index;
      while (index < source.length && source[index] !== "\n") index += 1;
      comments.push({ start, end: index, text: source.slice(start, index) });
      blank(start, index);
      continue;
    }
    if (char === "/" && next === "*") {
      const start = index;
      let depth = 0;
      while (index < source.length) {
        if (source[index] === "/" && source[index + 1] === "*") { depth += 1; index += 2; continue; }
        if (source[index] === "*" && source[index + 1] === "/") {
          depth -= 1;
          index += 2;
          if (depth === 0) break;
          continue;
        }
        index += 1;
      }
      comments.push({ start, end: index, text: source.slice(start, index) });
      blank(start, index);
      continue;
    }
    const rawEnd = rawStringEnd(source, index);
    if (rawEnd !== null) { blank(index, rawEnd); index = rawEnd; continue; }
    if (char === '"' || (char === "b" && next === '"')) {
      const start = index;
      index += char === "b" ? 2 : 1;
      while (index < source.length) {
        if (source[index] === "\\") { index += 2; continue; }
        if (source[index] === '"') { index += 1; break; }
        index += 1;
      }
      blank(start, index);
      continue;
    }
    if (char === "'") {
      const end = charEnd(source, index);
      if (end !== null) { blank(index, end); index = end; continue; }
    }
    index += 1;
  }
  return { code: out.join(""), comments };
}

function lineAt(source, index) {
  return source.slice(0, index).split(/\r?\n/).length;
}

function attachedComment(comments, source, code, index, kind) {
  let cursor = comments.length - 1;
  while (cursor >= 0 && comments[cursor].end > index) cursor -= 1;
  if (cursor < 0) return "";
  // Use masked code for punctuation: braces/semicolons in strings are spaces.
  const gap = code.slice(comments[cursor].end, index);
  const sourceGap = source.slice(comments[cursor].end, index);
  if (gap.split(/\r?\n/).length > 8) return "";
  if (/\n[ \t]*\r?\n/.test(sourceGap)) return "";
  if (kind === "impl" && gap.trim() !== "") return "";
  // Semicolons inside a declaration's brackets do not end the statement.
  // A brace group without a semicolon is a destructuring pattern; one with a
  // semicolon is a completed block statement and is not stripped.
  let stripped = gap;
  for (;;) {
    const next = stripped.replace(/\[[^\[\]]*\]|\([^()]*\)|\{[^{};]*\}/g, "");
    if (next === stripped) break;
    stripped = next;
  }
  // A prior unsafe block or impl in the gap owns its own comment; a prior
  // `unsafe fn` type does not.
  if (/\bunsafe\s*\{|\bunsafe\s+impl\b/.test(gap)) return "";
  // A completed statement between the comment and the item means the comment
  // documents something else.
  if (stripped.includes(";") || stripped.includes("}")) return "";
  let text = comments[cursor].text;
  let start = comments[cursor].start;
  for (let previous = cursor - 1; previous >= 0; previous -= 1) {
    const between = source.slice(comments[previous].end, start);
    if (between.trim() !== "" || between.split(/\r?\n/).length > 2) break;
    text = `${comments[previous].text}\n${text}`;
    start = comments[previous].start;
  }
  return text;
}

const blockPattern = /\bunsafe\s*\{/g;
const implPattern = /\bunsafe\s+impl\b/g;
const marker = /SAFETY\s*:/;
const perFile = new Map();
const implsByFile = new Map();
const undocumented = [];
let matches = 0;
let blocks = 0;
let impls = 0;

for (const file of roots.flatMap(rustFiles).sort()) {
  const source = readFileSync(file, "utf8");
  const { code, comments } = scan(source);
  let fileBlocks = 0;
  for (const match of code.matchAll(blockPattern)) {
    matches += 1;
    blocks += 1;
    fileBlocks += 1;
    if (!marker.test(attachedComment(comments, source, code, match.index, "block"))) {
      undocumented.push(`${relative(file)}:${lineAt(source, match.index)}`);
    }
  }
  let fileImpls = 0;
  for (const match of code.matchAll(implPattern)) {
    impls += 1;
    fileImpls += 1;
    if (!marker.test(attachedComment(comments, source, code, match.index, "impl"))) {
      undocumented.push(`${relative(file)}:${lineAt(source, match.index)} (unsafe impl)`);
    }
  }
  if (fileBlocks > 0) perFile.set(relative(file), fileBlocks);
  if (fileImpls > 0) implsByFile.set(relative(file), fileImpls);
}

if (printOnly) {
  console.log(JSON.stringify({ matches, blocks, impls, files: Object.fromEntries(perFile), implsByFile: Object.fromEntries(implsByFile) }, null, 2));
  process.exit(0);
}

const baseline = JSON.parse(readFileSync(baselinePath, "utf8"));
const forbidBaseline = JSON.parse(readFileSync("scripts/unsafe-forbid-baseline.json", "utf8"));
const problems = [];
const missingForbid = forbidBaseline.filter((file) => !readFileSync(file, "utf8").includes("forbid(unsafe_code)"));
if (missingForbid.length > 0) {
  problems.push(`modules lost #![forbid(unsafe_code)]: ${missingForbid.join(", ")}`);
}
if (undocumented.length > 0) {
  problems.push(`undocumented unsafe items:\n  ${undocumented.join("\n  ")}`);
}
if (matches > baseline.matches || blocks > baseline.blocks) {
  problems.push(`unsafe census grew: matches ${baseline.matches} -> ${matches}, blocks ${baseline.blocks} -> ${blocks}`);
}
if (impls > baseline.impls) {
  problems.push(`unsafe impl items grew: ${baseline.impls} -> ${impls}`);
}
const grown = [...perFile].filter(([file, count]) => count > (baseline.files[file] ?? 0));
if (grown.length > 0) {
  problems.push(`unsafe blocks grew in: ${grown.map(([file, count]) => `${file} (${baseline.files[file] ?? 0} -> ${count})`).join(", ")}`);
}
const grownImpls = [...implsByFile].filter(([file, count]) => count > (baseline.implsByFile?.[file] ?? 0));
if (grownImpls.length > 0) {
  problems.push(`unsafe impls grew in: ${grownImpls.map(([file, count]) => `${file} (${baseline.implsByFile?.[file] ?? 0} -> ${count})`).join(", ")}`);
}
const changed = [...perFile].filter(([file, count]) => baseline.files[file] !== count);
if (changed.length > 0) {
  console.log("Unsafe census per-file changes:");
  for (const [file, count] of changed) {
    console.log(`  ${file}: ${baseline.files[file] ?? 0} -> ${count}`);
  }
}
if (problems.length > 0) {
  console.error(problems.join("\n"));
  console.error(`Census: ${matches} matches / ${blocks} blocks / ${impls} impls in ${perFile.size} files.`);
  console.error("Review the new unsafe code, add a SAFETY comment, and update scripts/unsafe-census-baseline.json.");
  process.exit(1);
}
console.log(`Unsafe census: ${matches} matches / ${blocks} blocks / ${impls} impls in ${perFile.size} files; every item documented.`);
