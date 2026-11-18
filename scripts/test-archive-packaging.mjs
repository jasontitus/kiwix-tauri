// SPDX-License-Identifier: GPL-3.0-or-later
import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { checkArchiveLibrary, checkArchiveLibraryFiles } from "./check-archive-libraries.mjs";
import { windowsBundleResources } from "./windows-bundle-resources.mjs";

test("rejects native SDK libraries and permits Rust reader notices and unrelated libraries", () => {
  for (const name of ["libzim.9.dylib", "libzim.so.9.8.2", "Xapian-1.4.DLL", "libxapian.a", "zim.lib", "zim.dll"]) {
    assert.throws(() => checkArchiveLibrary(name), /Unexpected native archive library/);
  }
  for (const name of ["licenses/zimru-xapian/LICENSE", "licenses/zimru/LICENSE", "libzstd.so.1", "onnxruntime.dll"]) {
    checkArchiveLibrary(name);
  }
});

test("inspects nested package names including symlinks", async (t) => {
  const root = await mkdtemp(path.join(tmpdir(), "kiwix-bundle-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const lib = path.join(root, "lib");
  await mkdir(lib);
  await checkArchiveLibraryFiles(root);
  // Creating Windows symlinks may need elevated privileges.
  if (process.platform === "win32") await writeFile(path.join(lib, "libzim.so"), "");
  else await symlink("libzim.so.9", path.join(lib, "libzim.so"));
  await assert.rejects(checkArchiveLibraryFiles(root), /libzim\.so/);
});

test("Windows resource selection requires the pinned speech runtime with no archive SDK", async (t) => {
  const root = await mkdtemp(path.join(tmpdir(), "kiwix-win-resources-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(path.join(root, "zim.dll"), "obsolete");
  await assert.rejects(windowsBundleResources(root), { code: "ENOENT", path: path.join(root, "onnxruntime.dll") });
  await writeFile(path.join(root, "onnxruntime.dll"), "wrong bytes");
  await assert.rejects(windowsBundleResources(root), /checksum mismatch: onnxruntime\.dll/);
});

test("ELF import inspection rejects an SDK link with no SDK file in the package", { skip: process.platform !== "linux" }, async (t) => {
  const { execFileSync } = await import("node:child_process");
  const root = await mkdtemp(path.join(tmpdir(), "kiwix-elf-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(path.join(root, "package"));
  const library = path.join(root, "outside.so");
  const binary = path.join(root, "package", "reader");
  execFileSync("cc", ["-shared", "-fPIC", "-Wl,-soname,libzim.so.9", "-x", "c", "-", "-o", library], {
    input: "int archived(void) { return 0; }",
  });
  execFileSync("cc", ["-x", "c", "-", "-x", "none", library, "-o", binary], {
    input: "extern int archived(void); int main(void) { return archived(); }",
  });
  await assert.rejects(checkArchiveLibraryFiles(path.dirname(binary), true), /libzim\.so\.9/);
});

test("macOS finalizer rejects a linked SDK outside the bundle before signing", { skip: process.platform !== "darwin" }, async (t) => {
  const { execFileSync, spawnSync } = await import("node:child_process");
  const { fileURLToPath } = await import("node:url");
  const root = await mkdtemp(path.join(tmpdir(), "kiwix-macho-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const app = path.join(root, "Reader.app");
  const contents = path.join(app, "Contents");
  const executable = path.join(contents, "MacOS", "reader");
  await mkdir(path.dirname(executable), { recursive: true });
  await writeFile(path.join(contents, "Info.plist"), '<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>reader</string></dict></plist>');
  const library = path.join(root, "external.dylib");
  execFileSync("cc", ["-dynamiclib", "-Wl,-install_name,@rpath/libzim.9.dylib", "-x", "c", "-", "-o", library], {
    input: "int archived(void) { return 0; }",
  });
  execFileSync("cc", ["-x", "c", "-", "-x", "none", library, "-o", executable], {
    input: "extern int archived(void); int main(void) { return archived(); }",
  });
  const tool = fileURLToPath(new URL("./bundle-macos.mjs", import.meta.url));
  const result = spawnSync(process.execPath, [tool, app, "--identity", "-"], { encoding: "utf8" });
  assert.equal(result.status, 1, result.stderr);
  assert.match(result.stderr, /Unexpected native archive library.*libzim\.9\.dylib/);
});
