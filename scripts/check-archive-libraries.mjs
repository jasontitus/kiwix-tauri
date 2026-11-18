// SPDX-License-Identifier: GPL-3.0-or-later
import { execFileSync } from "node:child_process";
import { open, readdir } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

// Match the native SDK's OS library names, including versioned links and targets.
// License directories and the Rust crate names zimru / zimfile-xapian are allowed.
export function checkArchiveLibrary(file) {
  const name = path.basename(file);
  if (/^(?:lib)?(?:zim|xapian)(?:[.-]\d+(?:\.\d+)*)?\.(?:so(?:\.\d+)*|dylib|dll|a|lib|framework)$/i.test(name)) {
    throw new Error(`Unexpected native archive library in application package: ${file}`);
  }
}

async function checkElfDependencies(file) {
  const handle = await open(file, "r");
  const magic = Buffer.alloc(4);
  try {
    if ((await handle.read(magic)).bytesRead !== 4 || !magic.equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]))) return;
  } finally {
    await handle.close();
  }
  const dynamic = execFileSync("readelf", ["--dynamic", "--wide", file], {
    encoding: "utf8", env: { ...process.env, LC_ALL: "C" },
  });
  for (const match of dynamic.matchAll(/\(NEEDED\).*?\[([^\]]+)\]/g)) {
    checkArchiveLibrary(match[1]);
  }
}

export async function checkArchiveLibraryFiles(directory, inspectElf = false) {
  for (const file of await readdir(directory, { withFileTypes: true })) {
    const candidate = path.join(directory, file.name);
    checkArchiveLibrary(candidate);
    if (file.isDirectory()) await checkArchiveLibraryFiles(candidate, inspectElf);
    else if (inspectElf && file.isFile()) await checkElfDependencies(candidate);
  }
}

if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  const [directory, flag] = process.argv.slice(2);
  if (!directory || (flag && flag !== "--elf") || process.argv.length > 4) throw new Error("Usage: check-archive-libraries.mjs DIRECTORY [--elf]");
  await checkArchiveLibraryFiles(directory, flag === "--elf");
}
