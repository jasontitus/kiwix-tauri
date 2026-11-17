#!/usr/bin/env node
// SPDX-License-Identifier: GPL-3.0-or-later
// Build the static Tauri v2 updater manifest from signed desktop artifacts.
//
// Every platform entry carries the artifact URL and the contents of its
// `.sig` file. The app verifies the signature against the public key in
// `src-tauri/tauri.conf.json`; the manifest itself is not trusted.

import { spawnSync } from "node:child_process";
import { createHash, createPublicKey, verify } from "node:crypto";
import {
  link,
  mkdir,
  readFile,
  rename,
  mkdtemp,
  stat,
  rm,
  writeFile,
} from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repository = fileURLToPath(new URL("..", import.meta.url));
const VERSION_PATTERN = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;
const MAX_VERSION_COMPONENT = 18446744073709551615n;
const TARGET_PATTERN =
  /^(?:darwin|windows|linux)-(?:aarch64|x86_64|i686|armv7)(?:-(?:app|appimage|deb|rpm|nsis|msi))?$/;

export const SUPPORTED_TARGETS = [
  "darwin-aarch64-app",
  "darwin-x86_64-app",
  "darwin-aarch64",
  "darwin-x86_64",
  "windows-x86_64-nsis",
  "windows-x86_64-msi",
  "windows-x86_64",
  "linux-x86_64-appimage",
  "linux-x86_64-deb",
  "linux-x86_64-rpm",
  "linux-x86_64",
];

const usage = `Usage: node scripts/make-update-manifest.mjs [options]

Required:
  --version VERSION              Release version, for example 0.1.1
  --base-url URL                 Public HTTPS directory that will hold the artifacts
  --artifact TARGET=PATH         Repeatable. PATH may be an artifact or a macOS .app
  --out FILE                     Manifest path, for example dist/update.json

Optional:
  --stage-dir DIR                Copy/package artifacts here (default: next to --out)
  --notes FILE                   Release notes file
  --notes-text TEXT              Release notes inline
  --pub-date ISO                 Release date (default: now)
  --private-key-path FILE        Tauri signing private key file
  --private-key KEY              Tauri signing private key literal
  --public-key KEY               Base64 Tauri public key (required with --no-sign)
  --private-key-password VALUE   Signing key password
  --allow-loopback               Permit an http:// loopback base URL for local proof
  --no-sign                      Verify source-file signatures; cannot package .app

The private key may also come from TAURI_SIGNING_PRIVATE_KEY_PATH,
TAURI_SIGNING_PRIVATE_KEY or TAURI_SIGNING_PRIVATE_KEY_PASSWORD. A key value
that names an existing file is read as a path.`;

class ManifestError extends Error {}

function fail(message) {
  throw new ManifestError(message);
}

export function validateVersion(version) {
  const match = typeof version === "string" ? VERSION_PATTERN.exec(version) : null;
  // Match the updater's Rust semver parser: core numbers are u64, numeric
  // prerelease identifiers have no leading zero, and build identifiers may.
  // Preserve the exact accepted spelling for the signed version and filenames.
  if (!match || match[0] !== version ||
      match.slice(1, 4).some((part) => part.length > 20 || BigInt(part) > MAX_VERSION_COMPONENT) ||
      match[4]?.split(".").some((part) => /^0\d+$/.test(part))) {
    fail(`Invalid version "${version}"; use a semantic version such as 0.1.1`);
  }
  return version;
}

export function validateTarget(target) {
  if (!TARGET_PATTERN.test(target)) {
    fail(
      `Invalid target "${target}". Use a Tauri target such as darwin-aarch64-app or linux-x86_64-appimage`,
    );
  }
  return target;
}

export function parseArtifact(spec) {
  const separator = spec.indexOf("=");
  if (separator <= 0) fail(`Artifact must be TARGET=PATH, got "${spec}"`);
  const target = validateTarget(spec.slice(0, separator));
  const file = spec.slice(separator + 1);
  if (!file) fail(`Artifact ${target} has no path`);
  return { target, file };
}

export function isLoopbackHost(hostname) {
  return (
    hostname === "127.0.0.1" ||
    hostname === "localhost" ||
    hostname === "::1" ||
    hostname === "[::1]"
  );
}

export function validateBaseUrl(value, { allowLoopback = false } = {}) {
  let url;
  try {
    url = new URL(value);
  } catch {
    fail(`Invalid base URL "${value}"`);
  }
  if (url.protocol !== "https:" && !(allowLoopback && url.protocol === "http:" && isLoopbackHost(url.hostname))) {
    fail(
      "The base URL must use HTTPS. HTTP is allowed only for a loopback acceptance run with --allow-loopback",
    );
  }
  if (!url.pathname.endsWith("/")) {
    url.pathname = `${url.pathname}/`;
  }
  url.search = "";
  url.hash = "";
  return url;
}

export function buildManifest({
  version,
  notes = null,
  pubDate = new Date().toISOString(),
  baseUrl,
  artifacts,
}) {
  validateVersion(version);
  if (!Array.isArray(artifacts) || artifacts.length === 0) {
    fail("At least one --artifact is required");
  }
  const platforms = {};
  for (const artifact of artifacts) {
    const target = validateTarget(artifact.target);
    if (platforms[target]) fail(`Duplicate target ${target}`);
    if (!artifact.signature || !artifact.signature.trim()) {
      fail(`Missing signature for ${target}`);
    }
    if (!artifact.fileName) fail(`Missing artifact file name for ${target}`);
    platforms[target] = {
      signature: artifact.signature.trim(),
      url: new URL(encodeURIComponent(artifact.fileName), baseUrl).href,
    };
  }
  const date = new Date(pubDate);
  if (Number.isNaN(date.getTime())) fail(`Invalid publication date "${pubDate}"`);
  return {
    version,
    notes,
    pub_date: date.toISOString(),
    platforms,
  };
}

function run(command, args, { allowFailure = false } = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0 && !allowFailure) {
    throw new Error(
      `${command} failed (${result.status}):\n${result.stderr || result.stdout}`,
    );
  }
  return result;
}

async function exists(file) {
  try {
    await stat(file);
    return true;
  } catch {
    return false;
  }
}

function parseArgs(argv) {
  const options = {
    artifacts: [],
    notesFile: null,
    notesText: null,
    pubDate: null,
    baseUrl: null,
    version: null,
    out: null,
    stageDir: null,
    privateKeyPath: process.env.TAURI_SIGNING_PRIVATE_KEY_PATH ?? null,
    privateKey: process.env.TAURI_SIGNING_PRIVATE_KEY ?? null,
    privateKeyPassword: process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ?? null,
    publicKey: process.env.KIWIX_UPDATER_PUBLIC_KEY ?? null,
    allowLoopback: false,
    sign: true,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    switch (arg) {
      case "--version":
        options.version = argv[++index];
        break;
      case "--base-url":
        options.baseUrl = argv[++index];
        break;
      case "--artifact":
        options.artifacts.push(parseArtifact(argv[++index]));
        break;
      case "--out":
        options.out = argv[++index];
        break;
      case "--stage-dir":
        options.stageDir = argv[++index];
        break;
      case "--notes":
        options.notesFile = argv[++index];
        break;
      case "--notes-text":
        options.notesText = argv[++index];
        break;
      case "--pub-date":
        options.pubDate = argv[++index];
        break;
      case "--private-key-path":
        options.privateKeyPath = argv[++index];
        break;
      case "--private-key":
        options.privateKey = argv[++index];
        break;
      case "--private-key-password":
        options.privateKeyPassword = argv[++index];
        break;
      case "--public-key":
        options.publicKey = argv[++index];
        break;
      case "--allow-loopback":
        options.allowLoopback = true;
        break;
      case "--no-sign":
        options.sign = false;
        break;
      case "--help":
      case "-h":
        console.log(usage);
        process.exit(0);
        break;
      default:
        fail(`Unknown option ${arg}\n\n${usage}`);
    }
  }
  if (!options.version) fail("--version is required");
  if (!options.baseUrl) fail("--base-url is required");
  if (!options.out) fail("--out is required");
  if (options.artifacts.length === 0) fail("At least one --artifact is required");
  return options;
}

export async function tauriCli() {
  const require = createRequire(import.meta.url);
  const packageFile = require.resolve("@tauri-apps/cli/package.json");
  const metadata = JSON.parse(await readFile(packageFile, "utf8"));
  const entry = typeof metadata.bin === "string" ? metadata.bin : metadata.bin?.tauri;
  if (typeof entry !== "string" || !entry) {
    fail("@tauri-apps/cli does not declare a tauri executable");
  }
  return path.resolve(path.dirname(packageFile), entry);
}

async function signArtifact(file, version, options) {
  const args = ["signer", "sign", "--app-version", version];
  if (options.privateKeyPath) {
    args.push("--private-key-path", options.privateKeyPath);
  } else if (options.privateKey) {
    if (await exists(options.privateKey)) {
      args.push("--private-key-path", options.privateKey);
    } else {
      args.push("--private-key", options.privateKey);
    }
  } else {
    fail(
      "No signing key supplied. Set TAURI_SIGNING_PRIVATE_KEY_PATH (or --private-key-path), or place a .sig file next to every artifact.",
    );
  }
  if (options.privateKeyPassword !== null) {
    args.push("--password", options.privateKeyPassword);
  }
  args.push(file);
  run(process.execPath, [await tauriCli(), ...args]);
}

// Tauri's minisign envelope authenticates both the exact payload and the
// trusted comment containing its release version. Never trust a staged .sig.
export function verifyArtifactSignature(bytes, signature, publicKey, version) {
  try {
    const keyLines = Buffer.from(publicKey, "base64").toString("utf8").trim().split(/\r?\n/);
    const lines = Buffer.from(signature.trim(), "base64").toString("utf8").trim().split(/\r?\n/);
    const key = Buffer.from(keyLines[1], "base64");
    const record = Buffer.from(lines[1], "base64");
    if (key.length !== 42 || record.length !== 74 ||
        !key.subarray(2, 10).equals(record.subarray(2, 10)) ||
        !lines[2].startsWith("trusted comment: ")) throw new Error("Invalid minisign envelope");
    const algorithm = record.subarray(0, 2).toString("ascii");
    const payload = algorithm === "ED" ? createHash("blake2b512").update(bytes).digest()
      : algorithm === "Ed" ? bytes : null;
    if (!payload) throw new Error("Unsupported minisign algorithm");
    const edKey = createPublicKey({
      key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), key.subarray(10)]),
      format: "der",
      type: "spki",
    });
    const comment = lines[2].slice("trusted comment: ".length);
    const payloadSignature = record.subarray(10);
    if (!verify(null, payload, edKey, payloadSignature) ||
        !verify(null, Buffer.concat([payloadSignature, Buffer.from(comment)]), edKey, Buffer.from(lines[3], "base64"))) {
      throw new Error("Signature does not authenticate artifact bytes");
    }
    const signedVersion = comment.split("\t").find((field) => field.startsWith("version:"))?.slice(8);
    if (signedVersion !== version) throw new Error("Signature does not authenticate release version");
  } catch (error) {
    fail(`Artifact signature verification failed: ${error.message}`);
  }
}

async function planArtifacts(options, stageDir, out) {
  const plans = [];
  const targets = new Set();
  // Case-fold even on case-sensitive hosts: published release files must also
  // remain distinct when downloaded/staged on Windows and default macOS.
  const outputs = new Set([out.toLowerCase(), `${out}.tmp`.toLowerCase()]);
  for (const artifact of options.artifacts) {
    const target = validateTarget(artifact.target);
    if (targets.has(target)) fail(`Duplicate target ${target}`);
    targets.add(target);
    const source = path.resolve(artifact.file);
    const info = await stat(source);
    const packaged = info.isDirectory();
    if (packaged && (!source.endsWith(".app") || !target.startsWith("darwin-"))) {
      fail(`Only a macOS .app directory can be packaged, got ${source}`);
    }
    if (!packaged && !info.isFile()) fail(`Artifact is not a regular file: ${source}`);
    if (packaged && !options.sign) fail("--no-sign cannot package a .app: freshly packaged bytes must be signed");
    const fileName = `${target}-${options.version}-${path.basename(source)}${packaged ? ".tar.gz" : ""}`;
    const staged = path.join(stageDir, fileName);
    for (const output of [staged, `${staged}.sig`]) {
      const folded = output.toLowerCase();
      if (outputs.has(folded)) fail(`Output collision: ${output}`);
      outputs.add(folded);
      if (await exists(output)) fail(`Output already exists: ${output}; use a fresh release staging directory`);
    }
    const bytes = packaged ? null : await readFile(source);
    const signature = !options.sign ? await readFile(`${source}.sig`, "utf8") : null;
    if (!options.sign) {
      if (!options.publicKey) fail("--no-sign requires --public-key to authenticate source bytes and version");
      verifyArtifactSignature(bytes, signature, options.publicKey, options.version);
    }
    plans.push({ target, source, packaged, fileName, staged, bytes, signature });
  }
  for (const plan of plans) {
    for (const source of [plan.source, `${plan.source}.sig`]) {
      if (outputs.has(source.toLowerCase())) fail(`Output collides with an input: ${source}`);
    }
  }
  return plans;
}

async function prepareArtifact(plan, options, scratch) {
  const temporary = path.join(scratch, plan.fileName);
  if (plan.packaged) {
    run("tar", ["-czf", temporary, "-C", path.dirname(plan.source), path.basename(plan.source)]);
  } else {
    // Use the same snapshot that was verified, not a second read of the source.
    await writeFile(temporary, plan.bytes);
  }
  let signature = plan.signature;
  if (options.sign) {
    // Always sign these newly staged bytes; never reuse any .sig at the final
    // output path, even when a previous release generation was interrupted.
    await signArtifact(temporary, options.version, options);
    signature = await readFile(`${temporary}.sig`, "utf8");
    if (options.publicKey) {
      verifyArtifactSignature(await readFile(temporary), signature, options.publicKey, options.version);
    }
  } else {
    await writeFile(`${temporary}.sig`, signature);
  }
  return { target: plan.target, fileName: plan.fileName, signature };
}

export async function generate(options) {
  validateVersion(options.version);
  const baseUrl = validateBaseUrl(options.baseUrl, {
    allowLoopback: options.allowLoopback,
  });
  const out = path.resolve(options.out);
  const stageDir = path.resolve(options.stageDir ?? path.join(path.dirname(out), "artifacts"));
  const notes = options.notesText
    ?? (options.notesFile ? await readFile(path.resolve(options.notesFile), "utf8") : null);
  if (!Array.isArray(options.artifacts) || options.artifacts.length === 0) fail("At least one --artifact is required");
  const plans = await planArtifacts(options, stageDir, out);
  // Complete admission, including all signatures and output collisions, before
  // writing anything into the release directory.
  await mkdir(stageDir, { recursive: true });
  const scratch = await mkdtemp(path.join(stageDir, ".updater-"));
  try {
    const artifacts = [];
    for (const plan of plans) {
      artifacts.push(await prepareArtifact(plan, options, scratch));
    }
    const manifest = buildManifest({
      version: options.version,
      notes,
      pubDate: options.pubDate ?? new Date().toISOString(),
      baseUrl,
      artifacts,
    });
    for (const plan of plans) {
      // Publish immutable artifact names without overwriting a concurrent release.
      await link(path.join(scratch, plan.fileName), plan.staged);
      await link(path.join(scratch, `${plan.fileName}.sig`), `${plan.staged}.sig`);
    }
    await mkdir(path.dirname(out), { recursive: true });
    const temporary = `${out}.tmp`;
    await writeFile(temporary, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
    await rename(temporary, out);
    return { manifest, out, stageDir };
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const result = await generate(options);
  console.log(JSON.stringify({
    manifest: result.out,
    stageDir: result.stageDir,
    version: result.manifest.version,
    platforms: Object.keys(result.manifest.platforms),
  }, null, 2));
}

if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  main().catch((error) => {
    console.error(error instanceof ManifestError ? error.message : error.stack ?? String(error));
    process.exit(1);
  });
}
