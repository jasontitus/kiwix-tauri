// SPDX-License-Identifier: GPL-3.0-or-later
// Build-glue and documentation drift checks: no orphaned native Rust glue, no
// documentation claiming the app links libzim, and the unreleased location
// providers stay type-checked in CI without entering default builds.
// checks.yml re-includes this file and docs/FORKS.md in its path filter; an
// edit to any other Markdown file alone is checked by the next run that is not
// documentation-only.
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const read = (base, relative) => readFileSync(path.join(base, relative), "utf8");

/** `src-tauri/native/*.rs` files that neither build.rs nor src/lib.rs names. */
export function unreferencedNativeSources(base) {
  const native = path.join(base, "src-tauri/native");
  const consumers = read(base, "src-tauri/build.rs") + "\n" + read(base, "src-tauri/src/lib.rs");
  return readdirSync(native)
    .filter((name) => name.endsWith(".rs"))
    .filter((name) => !consumers.includes(`native/${name}`))
    .sort();
}

// Historical records keep what was true when they were written.
const historical = (relative) =>
  relative.startsWith("docs/reviews/") || /^CHANGELOG/i.test(path.basename(relative));
const skippedDirectories = new Set([".git", "node_modules", "target", "vendor", "dist"]);
const libzimLinkClaim = /\bthe app links (?:a pinned )?libzim\b/i;

function markdownFiles(base, directory = base) {
  const found = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const file = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      if (!skippedDirectories.has(entry.name)) found.push(...markdownFiles(base, file));
    } else if (entry.isFile() && entry.name.endsWith(".md")) {
      found.push(path.relative(base, file).split(path.sep).join("/"));
    }
  }
  return found;
}

/** Current (non-historical) Markdown files that say the app links libzim. */
export function staleLibzimClaims(base) {
  return markdownFiles(base)
    .filter((relative) => !historical(relative))
    .filter((relative) => libzimLinkClaim.test(read(base, relative)))
    .sort();
}

/** The workflow job body that starts at `  <name>:` and ends before the next job. */
function job(workflow, name) {
  const lines = workflow.split("\n");
  const start = lines.indexOf(`  ${name}:`);
  assert.notEqual(start, -1, `missing job ${name}`);
  const end = lines.findIndex((line, index) => index > start && /^ {2}[A-Za-z0-9_-]+:\s*$/.test(line));
  return lines.slice(start, end === -1 ? undefined : end).join("\n");
}

test("every native Rust source is consumed by the build script or the crate", () => {
  assert.deepEqual(unreferencedNativeSources(root), []);
});

test("current documentation does not claim the app links libzim", () => {
  assert.deepEqual(staleLibzimClaims(root), []);
});

test("the checks detect an orphaned native source and a stale libzim claim", () => {
  const base = mkdtempSync(path.join(tmpdir(), "kiwix-build-hygiene-"));
  try {
    mkdirSync(path.join(base, "src-tauri/native"), { recursive: true });
    mkdirSync(path.join(base, "src-tauri/src"), { recursive: true });
    mkdirSync(path.join(base, "docs/reviews/old"), { recursive: true });
    writeFileSync(path.join(base, "src-tauri/build.rs"), 'cc::Build::new().file("native/used.rs");\n');
    writeFileSync(path.join(base, "src-tauri/src/lib.rs"), '#[path = "../native/also_used.rs"]\nmod also_used;\n');
    for (const name of ["used.rs", "also_used.rs", "orphan.rs"]) {
      writeFileSync(path.join(base, "src-tauri/native", name), "\n");
    }
    const claim = "The app links a pinned libzim 9.8.2 built with five patches.\n";
    writeFileSync(path.join(base, "docs/CURRENT.md"), claim);
    writeFileSync(path.join(base, "docs/reviews/old/FINDING.md"), claim);
    writeFileSync(path.join(base, "CHANGELOG.md"), claim);
    writeFileSync(path.join(base, "docs/OK.md"), "The app does not link libzim; it no longer links libzim.\n");
    assert.deepEqual(unreferencedNativeSources(base), ["orphan.rs"]);
    assert.deepEqual(staleLibzimClaims(base), ["docs/CURRENT.md"]);
  } finally {
    rmSync(base, { recursive: true, force: true });
  }
});

test("zbus stays optional and out of default Linux builds", () => {
  const manifest = read(root, "src-tauri/Cargo.toml");
  const zbus = manifest.split("\n").filter((line) => /^zbus\s*=/.test(line));
  assert.equal(zbus.length, 1, "one zbus declaration");
  assert.match(zbus[0], /optional\s*=\s*true/);
  assert.match(manifest, /^experimental-location\s*=\s*\["dep:zbus"\]/m);
  const build = read(root, "src-tauri/build.rs");
  assert.match(build, /cargo:rerun-if-env-changed=KIWIX_EXPERIMENTAL_LOCATION/);
  assert.match(build, /CARGO_FEATURE_EXPERIMENTAL_LOCATION/);
});

test("CI type-checks the unreleased Linux and Android location providers", () => {
  const workflow = read(root, ".github/workflows/checks.yml");
  for (const name of ["linux-typecheck", "android-typecheck"]) {
    const body = job(workflow, name);
    assert.match(body, /KIWIX_EXPERIMENTAL_LOCATION: '1'/, name);
    assert.match(body, /--features [^\n]*experimental-location/, name);
    // The shipping cfg (no provider) must keep its own clippy run.
    assert.ok((body.match(/cargo clippy/g) ?? []).length >= 2, `${name}: needs a default and an opt-in clippy run`);
    const features = body.split("\n").filter((line) => /--features /.test(line));
    assert.ok(features.some((line) => !line.includes("experimental-location")),
      `${name}: one clippy run must use the shipping feature set`);
  }
});
