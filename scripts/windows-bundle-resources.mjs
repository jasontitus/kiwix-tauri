// Build the two Windows runtime dependencies and emit a Tauri resource overlay.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { lstat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const target = "x86_64-pc-windows-msvc";
// sherpa-onnx v2.0.1 win-x64-shared-MT-Release-lib, pinned per shipped DLL.
const speech = {
  "onnxruntime.dll": "7f66f939a881baf4f46a2216496798edf4a1429878b646d12674aa62f27d8a25",
  "onnxruntime_providers_shared.dll": "551d0e1fe4c227d8542314ba718d52f4379e0c7bfe729a37c59833a884e27b4d",
  "sherpa-onnx-c-api.dll": "0965a6e4c0254da2beff489cf7a8aaffaf7dc94f90b43b3e66448b8cab4177ea",
  "sherpa-onnx-cxx-api.dll": "ada772889f966426ad9caba5b5125bffb836163573791fdda99541280d45dafa",
};

export async function windowsBundleResources(release) {
  const resources = {};
  const destinations = new Set();
  async function add(file, digest) {
    const name = path.basename(file);
    if (!(await lstat(file)).isFile()) throw new Error(`Expected a regular runtime file: ${file}`);
    if (destinations.has(name.toLowerCase())) throw new Error(`Duplicate Windows runtime name: ${name}`);
    if (digest !== undefined) {
      const hash = createHash("sha256");
      for await (const chunk of createReadStream(file)) hash.update(chunk);
      if (hash.digest("hex") !== digest) throw new Error(`Windows runtime checksum mismatch: ${name}`);
    }
    destinations.add(name.toLowerCase());
    resources[file] = name;
  }
  for (const [name, digest] of Object.entries(speech)) {
    await add(path.join(release, name), digest);
  }
  await add(path.join(release, "kiwix-mcp.exe"));
  return { bundle: { resources } };
}

async function main() {
  const metadata = JSON.parse(execFileSync("cargo", [
    "metadata", "--locked", "--format-version", "1", "--no-deps", "--manifest-path", "src-tauri/Cargo.toml",
  ], { cwd: root, encoding: "utf8" }));
  const args = ["build", "--locked", "--release", "--target", target, "--target-dir", metadata.target_directory];
  for (const crate of [["src-tauri/Cargo.toml", "-p", "sherpa-onnx-sys"], ["kiwix-mcp/Cargo.toml"]]) {
    execFileSync("cargo", [...args, "--manifest-path", ...crate], { cwd: root, stdio: ["ignore", 2, 2] });
  }
  const release = path.join(metadata.target_directory, target, "release");
  console.log(JSON.stringify(await windowsBundleResources(release), null, 2));
}

if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
