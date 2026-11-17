#!/usr/bin/env node
// Build the kiwix-mcp relay for the host target and place it where Tauri's
// externalBin bundler expects it: src-tauri/binaries/kiwix-mcp-<target-triple>.
import { spawnSync } from "node:child_process";
import { chmod, copyFile, mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repository = fileURLToPath(new URL("..", import.meta.url));
const manifest = path.join(repository, "kiwix-mcp", "Cargo.toml");

function run(command, args) {
  const result = spawnSync(command, args, { encoding: "utf8", stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} exited ${result.status}`);
}

function hostTriple() {
  const result = spawnSync("rustc", ["-vV"], { encoding: "utf8" });
  if (result.error) throw result.error;
  const host = result.stdout.split("\n").find((line) => line.startsWith("host: "));
  if (!host) throw new Error("rustc -vV did not report a host triple");
  return host.slice("host: ".length).trim();
}

const host = hostTriple();
const target = process.env.TAURI_ENV_TARGET_TRIPLE || process.env.CARGO_BUILD_TARGET || host;
const windows = target.includes("windows");
const args = ["build", "--release", "--locked", "--manifest-path", manifest, "--target", target];

const targetDirectory = process.env.CARGO_TARGET_DIR
  ? path.resolve(process.cwd(), process.env.CARGO_TARGET_DIR)
  : path.join(repository, "kiwix-mcp", "target");
run("cargo", [...args, "--target-dir", targetDirectory]);
const profile = path.join(targetDirectory, target, "release");
const source = path.join(profile, windows ? "kiwix-mcp.exe" : "kiwix-mcp");
const directory = path.join(repository, "src-tauri", "binaries");
const destination = path.join(directory, `kiwix-mcp-${target}${windows ? ".exe" : ""}`);
await mkdir(directory, { recursive: true });
await copyFile(source, destination);
await chmod(destination, 0o755);
console.log(`kiwix-mcp -> ${path.relative(repository, destination)}`);
