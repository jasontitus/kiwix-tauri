#!/usr/bin/env node
// SPDX-License-Identifier: GPL-3.0-or-later
// Run AFTER `tauri build --bundles app`, before making any distribution archive.
import { spawnSync } from "node:child_process";
import { constants } from "node:fs";
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  open,
  readdir,
  realpath,
  rm,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { checkArchiveLibrary } from "./check-archive-libraries.mjs";

const repository = fileURLToPath(new URL("..", import.meta.url));

const usage = `Usage: node bundle-macos.mjs [APP.app] [options]
  --library-dir DIR          Additional dylib lookup directory (repeatable)
  --identity IDENTITY        Installed codesign identity; '-' is local ad-hoc signing
  --entitlements FILE        App entitlements (defaults to src-tauri/entitlements.plist)
  --notarize-profile NAME    Submit using an existing notarytool keychain profile

APP.app defaults to Kiwix.app under the effective cargo target directory:
CARGO_TARGET_DIR when set, otherwise src-tauri/target. A relative
CARGO_TARGET_DIR is resolved from the current directory (the repository root for
\`npm run bundle:macos\`), falling back to src-tauri.
The default identity is APPLE_SIGNING_IDENTITY, or '-' for an unsigned/ad-hoc app.
An existing Apple-signed app requires an explicit identity; it is never silently downgraded.
Notarization requires a real identity and credentials; no release is published.`;

function run(command, args, { allowFailure = false } = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0 && !allowFailure) {
    throw new Error(
      `${command} failed (${result.status}):\n${result.stderr || result.stdout}`,
    );
  }
  return result;
}

const inside = (root, file) => {
  const relative = path.relative(root, file);
  return (
    relative === "" ||
    (!path.isAbsolute(relative) &&
      relative !== ".." &&
      !relative.startsWith(`..${path.sep}`))
  );
};
const systemLibrary = (value) =>
  value.startsWith("/System/Library/") || value.startsWith("/usr/lib/");
const machoMagic = new Set([
  "feedface",
  "feedfacf",
  "cefaedfe",
  "cffaedfe",
  "cafebabe",
  "bebafeca",
  "cafebabf",
  "bfbafeca",
]);

async function isMachO(file) {
  const handle = await open(file, "r");
  try {
    const buffer = Buffer.alloc(4);
    const { bytesRead } = await handle.read(buffer, 0, 4, 0);
    return bytesRead === 4 && machoMagic.has(buffer.toString("hex"));
  } finally {
    await handle.close();
  }
}

// Read load commands, not `otool -L`: the latter also lists LC_ID_DYLIB as a dependency.
// Sets combine the load commands of every slice in a universal Mach-O.
function inspect(file) {
  const dependencies = new Set();
  const ids = new Set();
  const rpaths = new Set();
  let command = "";
  for (const line of run("/usr/bin/otool", ["-l", file]).stdout.split("\n")) {
    const match = line.match(/^\s*cmd (LC_\S+)\s*$/);
    if (match) command = match[1];
    const name = line.match(/^\s*name (.+) \(offset \d+\)\s*$/);
    if (name && command === "LC_ID_DYLIB") ids.add(name[1]);
    else if (
      name &&
      /^LC_(?:LOAD_DYLIB|LOAD_WEAK_DYLIB|REEXPORT_DYLIB|LOAD_UPWARD_DYLIB|LAZY_LOAD_DYLIB)$/.test(
        command,
      )
    )
      dependencies.add(name[1]);
    const rpath = line.match(/^\s*path (.+) \(offset \d+\)\s*$/);
    if (rpath && command === "LC_RPATH") rpaths.add(rpath[1]);
  }
  const architectures = new Set(
    run("/usr/bin/lipo", ["-archs", file]).stdout.trim().split(/\s+/),
  );
  return { dependencies, ids, rpaths, architectures };
}

// Cargo resolves a relative CARGO_TARGET_DIR against the directory it runs in;
// Tauri may run it from the repository root or src-tauri, so try both.
function appCandidates() {
  const configured = process.env.CARGO_TARGET_DIR;
  const targets = configured
    ? path.isAbsolute(configured)
      ? [configured]
      : [
          path.resolve(configured),
          path.resolve(repository, "src-tauri", configured),
        ]
    : [path.join(repository, "src-tauri", "target")];
  return [...new Set(targets)].map((target) =>
    path.join(target, "release", "bundle", "macos", "Kiwix.app"),
  );
}
async function defaultApp() {
  const candidates = appCandidates();
  for (const candidate of candidates) {
    try {
      await lstat(candidate);
      return candidate;
    } catch (error) {
      if (error.code !== "ENOENT" && error.code !== "ENOTDIR") throw error;
    }
  }
  throw new Error(
    `No app bundle found at ${candidates.join(" or ")}. Build it first with \`npm run tauri -- build --bundles app\`, point CARGO_TARGET_DIR at the directory that build used, or pass APP.app explicitly.`,
  );
}

async function main() {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    console.log(usage);
    return;
  }
  if (process.platform !== "darwin")
    throw new Error("macOS with Xcode command-line tools is required.");
  const requestedApp =
    args.length && !args[0].startsWith("-") ? args.shift() : null;
  const appArgument = requestedApp ?? (await defaultApp());
  const options = {
    libraryDirs: [],
    identity: process.env.APPLE_SIGNING_IDENTITY,
    entitlements: fileURLToPath(
      new URL("../src-tauri/entitlements.plist", import.meta.url),
    ),
  };
  while (args.length) {
    const option = args.shift();
    const value = args.shift();
    if (
      !value ||
      ![
        "--library-dir",
        "--identity",
        "--entitlements",
        "--notarize-profile",
      ].includes(option)
    )
      throw new Error(usage);
    if (option === "--library-dir")
      options.libraryDirs.push(await realpath(value));
    else if (option === "--identity") options.identity = value;
    else if (option === "--entitlements")
      options.entitlements = await realpath(value);
    else options.notarizeProfile = value;
  }

  const suppliedApp = path.resolve(appArgument);
  const stat = await lstat(suppliedApp);
  if (
    !stat.isDirectory() ||
    stat.isSymbolicLink() ||
    path.extname(suppliedApp) !== ".app"
  )
    throw new Error("APP must be an existing, non-symlink .app directory.");
  const app = await realpath(suppliedApp);
  const contents = path.join(app, "Contents");
  const frameworks = path.join(contents, "Frameworks");
  // Validate the bundle before creating or changing anything.
  const executableName = run("/usr/libexec/PlistBuddy", [
    "-c",
    "Print :CFBundleExecutable",
    path.join(contents, "Info.plist"),
  ]).stdout.trim();
  if (
    !executableName ||
    executableName !== path.basename(executableName) ||
    executableName === "." ||
    executableName === ".."
  )
    throw new Error("Invalid CFBundleExecutable.");
  const executable = await realpath(
    path.join(contents, "MacOS", executableName),
  );
  if (!inside(app, executable) || !(await isMachO(executable)))
    throw new Error("Bundle executable must be a Mach-O inside the app.");
  const signature = run("/usr/bin/codesign", ["-d", "--verbose=2", app], {
    allowFailure: true,
  });
  if (!options.identity && /^Authority=/m.test(signature.stderr))
    throw new Error(
      "App already has an Apple signature. Supply --identity to re-sign with your real certificate, or explicitly --identity - for local use.",
    );
  options.identity ||= "-";
  if (options.notarizeProfile && options.identity === "-")
    throw new Error(
      "Notarization requires --identity with a real Developer ID certificate.",
    );
  await realpath(options.entitlements);

  const images = [];
  const nestedBundles = [];
  const visitedDirectories = new Set();
  async function walk(directory) {
    const actual = await realpath(directory);
    if (!inside(app, actual))
      throw new Error(`Bundle symlink escapes the app: ${directory}`);
    if (visitedDirectories.has(actual)) return;
    visitedDirectories.add(actual);
    for (const entry of await readdir(actual, { withFileTypes: true })) {
      const file = path.join(actual, entry.name);
      checkArchiveLibrary(file);
      const target = await realpath(file);
      if (!inside(app, target))
        throw new Error(`Bundle symlink escapes the app: ${file}`);
      const info = await lstat(target);
      if (info.isDirectory()) {
        if (/\.(?:app|framework|xpc|appex)$/.test(file))
          nestedBundles.push(file);
        await walk(target);
      } else if (info.isFile() && (await isMachO(target))) images.push(target);
    }
  }
  await walk(app);
  await mkdir(frameworks, { recursive: true });
  if (!inside(app, await realpath(frameworks)))
    throw new Error("Frameworks directory escapes the app.");

  const metadata = new Map();
  const getMetadata = (file) => {
    if (!metadata.has(file)) metadata.set(file, inspect(file));
    return metadata.get(file);
  };
  function expand(value, loader) {
    if (value === "@loader_path" || value.startsWith("@loader_path/"))
      return path.resolve(
        path.dirname(loader),
        value.slice("@loader_path".length).replace(/^\//, ""),
      );
    if (value === "@executable_path" || value.startsWith("@executable_path/"))
      return path.resolve(
        path.dirname(executable),
        value.slice("@executable_path".length).replace(/^\//, ""),
      );
    return path.isAbsolute(value) ? value : null;
  }
  const executableRpaths = [...getMetadata(executable).rpaths]
    .map((value) => expand(value, executable))
    .filter(Boolean);
  async function resolveDependency(reference, source, rpaths) {
    const candidates = [];
    if (reference.startsWith("@rpath/")) {
      for (const directory of rpaths)
        candidates.push(
          path.join(directory, reference.slice("@rpath/".length)),
        );
    } else {
      const expanded = expand(reference, source);
      if (expanded) candidates.push(expanded);
      else if (!reference.startsWith("@"))
        candidates.push(path.resolve(path.dirname(source), reference));
    }
    // Explicit SDK directories are a fallback, never a global recursive search.
    if (!path.isAbsolute(reference))
      for (const directory of options.libraryDirs)
        candidates.push(path.join(directory, path.basename(reference)));
    for (const candidate of candidates) {
      // Apple's shared-cache dylibs need not exist on disk.
      if (systemLibrary(candidate)) return candidate;
      try {
        return await realpath(candidate);
      } catch (error) {
        if (error.code !== "ENOENT" && error.code !== "ENOTDIR") throw error;
      }
    }
    throw new Error(
      `Cannot resolve ${reference} required by ${source}. Tried: ${candidates.join(", ")}. Supply --library-dir for a trusted SDK if necessary.`,
    );
  }

  const destinationBySource = new Map();
  const sourceByDestination = new Map();
  const graph = new Map();
  const queue = [...new Set([executable, ...images])].map((source) => ({
    source,
    inheritedRpaths: executableRpaths,
  }));
  for (const { source } of queue) {
    destinationBySource.set(source, source);
    sourceByDestination.set(source, source);
  }
  // Resolve the entire closure before install_name_tool changes the original search paths.
  for (let index = 0; index < queue.length; index++) {
    const { source, inheritedRpaths } = queue[index];
    if (graph.has(source)) continue;
    const info = getMetadata(source);
    const rpaths = [...info.rpaths]
      .map((value) => expand(value, source))
      .filter(Boolean)
      .concat(inheritedRpaths);
    const edges = [];
    for (const reference of info.dependencies) {
      checkArchiveLibrary(reference);
      if (systemLibrary(reference)) continue;
      const dependency = await resolveDependency(reference, source, rpaths);
      checkArchiveLibrary(dependency);
      if (systemLibrary(dependency)) {
        edges.push({ reference, system: dependency });
        continue;
      }
      if (!(await isMachO(dependency)))
        throw new Error(`Dependency is not a Mach-O: ${dependency}`);
      const dependencyInfo = getMetadata(dependency);
      for (const architecture of info.architectures) {
        if (!dependencyInfo.architectures.has(architecture))
          throw new Error(
            `${dependency} lacks ${architecture}, required by ${source}. Use an SDK matching the app architecture.`,
          );
      }
      if (!destinationBySource.has(dependency)) {
        if (dependency.includes(".framework/"))
          throw new Error(
            `External framework ${dependency} needs a complete framework bundle; this script only imports external dylibs.`,
          );
        if (!dependencyInfo.ids.size)
          throw new Error(
            `Expected an external dylib, not an executable: ${dependency}`,
          );
        const destination = inside(app, dependency)
          ? dependency
          : path.join(frameworks, path.basename(dependency));
        if (
          sourceByDestination.has(destination) &&
          sourceByDestination.get(destination) !== dependency
        )
          throw new Error(
            `Dylib basename collision at ${destination}; refusing to replace another library.`,
          );
        try {
          if (destination !== dependency) {
            await lstat(destination);
            throw new Error(
              `Refusing to overwrite existing bundle file: ${destination}`,
            );
          }
        } catch (error) {
          if (error.code !== "ENOENT") throw error;
        }
        destinationBySource.set(dependency, destination);
        sourceByDestination.set(destination, dependency);
        queue.push({ source: dependency, inheritedRpaths: rpaths });
      }
      edges.push({ reference, dependency });
    }
    graph.set(source, edges);
  }

  for (const [source, destination] of destinationBySource) {
    if (source !== destination)
      await copyFile(source, destination, constants.COPYFILE_EXCL);
    const fileStat = await lstat(destination);
    await chmod(destination, fileStat.mode | 0o200);
  }
  for (const [source, edges] of graph) {
    const destination = destinationBySource.get(source);
    const info = getMetadata(source);
    const changes = [];
    if (info.ids.size) {
      const id = inside(frameworks, destination)
        ? `@rpath/${path.relative(frameworks, destination)}`
        : `@loader_path/${path.basename(destination)}`;
      changes.push("-id", id);
    }
    for (const edge of edges) {
      const target =
        edge.dependency && destinationBySource.get(edge.dependency);
      const reference =
        edge.system ||
        (destination === executable && inside(frameworks, target)
          ? `@rpath/${path.relative(frameworks, target)}`
          : `@loader_path/${path.relative(path.dirname(destination), target)}`);
      if (reference !== edge.reference)
        changes.push("-change", edge.reference, reference);
    }
    // All non-system references are explicit bundle-relative paths now. Drop SDK rpaths.
    const executableRpath = "@executable_path/../Frameworks";
    for (const rpath of info.rpaths) {
      if (destination !== executable || rpath !== executableRpath)
        changes.push("-delete_rpath", rpath);
    }
    if (destination === executable && !info.rpaths.has(executableRpath))
      changes.push("-add_rpath", executableRpath);
    if (changes.length)
      run("/usr/bin/install_name_tool", [...changes, destination]);
  }

  // Audit the rewritten load commands, including every architecture, before signing.
  for (const destination of sourceByDestination.keys()) {
    const info = inspect(destination);
    for (const value of [...info.dependencies, ...info.ids, ...info.rpaths]) {
      if (
        value.includes("/opt/homebrew/") ||
        value.includes("/usr/local/Cellar/")
      )
        throw new Error(`SDK path remains in ${destination}: ${value}`);
    }
    for (const reference of info.dependencies) {
      if (systemLibrary(reference)) continue;
      let target;
      if (reference.startsWith("@rpath/"))
        target = path.join(frameworks, reference.slice("@rpath/".length));
      else target = expand(reference, destination);
      if (!target || !inside(app, await realpath(target)))
        throw new Error(
          `Non-portable dependency in ${destination}: ${reference}`,
        );
    }
  }

  const realIdentity = options.identity !== "-";
  const sign = (target) => {
    const args = [
      "--force",
      "--sign",
      options.identity,
      "--timestamp" + (realIdentity ? "" : "=none"),
    ];
    if (realIdentity) args.push("--options", "runtime");
    if (target === app) args.push("--entitlements", options.entitlements);
    run("/usr/bin/codesign", [...args, target]);
  };
  // Signing the bundle executable validates every nested Mach-O, so a broken
  // framework image must be repaired before the inside-out pass reaches it.
  // The x86_64 SDK ships dylibs the closure walk does not rewrite, and
  // codesign rejected the executable with "code object is not signed at all;
  // In subcomponent: libicui18n…". Verify each framework image, re-sign only a
  // broken one, and name anything codesign still refuses.
  const unsigned = [];
  const verified = (target) =>
    run("/usr/bin/codesign", ["--verify", target], { allowFailure: true }).status === 0;
  for (const entry of await readdir(frameworks, { withFileTypes: true })) {
    const target = await realpath(path.join(frameworks, entry.name));
    // A resolved .framework is a directory; probing its magic would fail.
    if (!inside(app, target) || !(await lstat(target)).isFile() ||
        !(await isMachO(target)) || verified(target)) continue;
    sign(target);
    if (!verified(target)) unsigned.push(path.relative(app, target));
  }
  if (unsigned.length)
    throw new Error(`Unsigned framework code after signing: ${unsigned.join(", ")}`);
  // Sign inside-out. Do not use --deep for signing: that hides incomplete closure handling.
  const signed = new Set();
  const signOnce = (target) => {
    if (signed.has(target)) return;
    signed.add(target);
    sign(target);
  };
  for (const destination of [...sourceByDestination.keys()].sort(
    (a, b) => b.split(path.sep).length - a.split(path.sep).length,
  ))
    signOnce(destination);
  for (const bundle of [...new Set(nestedBundles)].sort(
    (a, b) => b.split(path.sep).length - a.split(path.sep).length,
  ))
    signOnce(bundle);
  sign(app);
  run("/usr/bin/codesign", [
    "--verify",
    "--deep",
    "--strict",
    "--verbose=2",
    app,
  ]);

  if (options.notarizeProfile) {
    const temporary = await mkdtemp(path.join(tmpdir(), "kiwix-notarize-"));
    try {
      const archive = path.join(temporary, "app.zip");
      run("/usr/bin/ditto", [
        "-c",
        "-k",
        "--sequesterRsrc",
        "--keepParent",
        app,
        archive,
      ]);
      const submission = JSON.parse(
        run("/usr/bin/xcrun", [
          "notarytool",
          "submit",
          archive,
          "--keychain-profile",
          options.notarizeProfile,
          "--wait",
          "--output-format",
          "json",
        ]).stdout,
      );
      if (submission.status !== "Accepted")
        throw new Error(
          `Apple notarization was not accepted: ${JSON.stringify(submission)}`,
        );
      run("/usr/bin/xcrun", ["stapler", "staple", app]);
      run("/usr/bin/xcrun", ["stapler", "validate", app]);
    } finally {
      await rm(temporary, { recursive: true, force: true });
    }
  }
  console.log(`Bundled ${destinationBySource.size} Mach-O images in ${app}.`);
  console.log(
    realIdentity
      ? `Signed with ${options.identity}.${options.notarizeProfile ? " Apple notarization accepted and stapled." : " Not notarized by this invocation."}`
      : "Ad-hoc signed for local use only; not Apple-authenticated or notarized.",
  );
}

main().catch((error) => {
  console.error(`macOS packaging failed: ${error.message}`);
  process.exitCode = 1;
});
