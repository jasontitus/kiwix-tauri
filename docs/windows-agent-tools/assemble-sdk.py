"""Assemble a self-built schema-2 Windows MSVC SDK for kiwix-tauri local testing.

Hashes are computed from the files actually installed; nothing is invented.
This is a LOCAL TEST SDK, not release provenance.
Usage: python -I assemble-sdk.py <repo> <libzim-prefix> <deps-prefix> <libzim-src> <out>

<out> is created fresh; an existing directory is replaced only if it holds a
previous kiwix-sdk.json, and never when it overlaps any input path.
"""
import hashlib
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

REVISION = "26ec526f74e8342a40da5ab14e364546988e0e1a"
SYSTEM = re.compile(r"^(kernel32|user32|advapi32|ws2_32|shell32|ole32|oleaut32|rpcrt4|bcrypt|crypt32|"
                    r"shlwapi|gdi32|winmm|ntdll|msvcp\d+.*|vcruntime\d+.*|concrt\d+|ucrtbase|api-ms-.*|ext-ms-.*)\.dll$", re.I)


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def dependents(dll):
    out = subprocess.check_output(["dumpbin", "/nologo", "/dependents", str(dll)], text=True)
    return [line.strip() for line in out.splitlines() if line.strip().lower().endswith(".dll")
            and not line.strip().lower().startswith("dump of")]


def clear_output(out, inputs):
    """Remove a previous SDK directory only when it is a marked output, disjoint from inputs."""
    target = out.resolve()
    for name, path in inputs.items():
        source = path.resolve()
        if target == source or target in source.parents or source in target.parents:
            raise SystemExit(f"Refusing output {out}: overlaps {name} input {path}")
    if out.is_symlink() or (out.exists() and not out.is_dir()):
        raise SystemExit(f"Refusing output {out}: not a plain directory")
    if out.exists():
        if not (out / "kiwix-sdk.json").is_file():
            raise SystemExit(f"Refusing to delete {out}: no kiwix-sdk.json marker; remove it manually")
        shutil.rmtree(out)


def main():
    repo, zim_prefix, deps_prefix, zim_src, out = map(Path, sys.argv[1:6])
    zim_build = Path(sys.argv[6]) if len(sys.argv) > 6 else None
    inputs = {"repo": repo, "libzim-prefix": zim_prefix, "deps-prefix": deps_prefix,
              "libzim-src": zim_src}
    if zim_build is not None:
        inputs["libzim-build"] = zim_build
    clear_output(out, inputs)
    (out / "bin").mkdir(parents=True)
    (out / "lib").mkdir()
    (out / "share/provenance").mkdir(parents=True)
    shutil.copytree(zim_prefix / "include", out / "include")
    shutil.copy2(zim_prefix / "lib/zim.lib", out / "lib/zim.lib")

    search = [zim_prefix / "bin", deps_prefix / "bin"]
    # Meson wrap fallbacks (e.g. xz/liblzma) are built in the libzim build tree, not installed.
    if zim_build:
        search += sorted({p.parent for p in (zim_build / "subprojects").rglob("*.dll")})
    zim_dlls = sorted((zim_prefix / "bin").glob("zim*.dll"))
    if len(zim_dlls) != 1:
        raise SystemExit(f"expected one zim DLL, found {zim_dlls}")
    pending, seen, unresolved = [zim_dlls[0]], {}, set()
    while pending:
        dll = pending.pop()
        if dll.name.lower() in seen:
            continue
        seen[dll.name.lower()] = dll
        for name in dependents(dll):
            if SYSTEM.match(name) or name.lower() in seen:
                continue
            hit = next((d / name for d in search if (d / name).is_file()), None)
            if hit is None:
                unresolved.add(name)
            else:
                pending.append(hit)
    if unresolved:
        raise SystemExit(f"Unresolved non-system DLLs: {sorted(unresolved)}")
    for dll in seen.values():
        shutil.copy2(dll, out / "bin" / dll.name)

    patches = [("kiwix-split-ranges-v1", "libzim-split-ranges.patch"),
               ("kiwix-retained-fd-v1", "libzim-retained-fd.patch")]
    records = []
    for marker, name in patches:
        src = repo / "src-tauri/native" / name
        shutil.copy2(src, out / "share/provenance" / name)
        records.append({"marker": marker, "component": "zim", "sha256": sha256(src),
                        "path": f"share/provenance/{name}"})
    local_patches = []
    for extra in sorted((Path(__file__).parent / "patches").glob("*.patch")):
        shutil.copy2(extra, out / "share/provenance" / extra.name)
        local_patches.append({"component": "zim", "sha256": sha256(extra),
                              "path": f"share/provenance/{extra.name}",
                              "note": "local MSVC build-compat patch, applied after kiwix-retained-fd-v1; not a kiwix-tauri patch"})
    shutil.copy2(Path(__file__), out / "share/provenance/assemble-sdk.py")
    build_script = Path(__file__).with_name("build-sdk.ps1")
    shutil.copy2(build_script, out / "share/provenance/build-sdk.ps1")

    licenses = []
    lic_dir = out / "share/licenses/libzim"
    lic_dir.mkdir(parents=True)
    for f in zim_src.iterdir():
        if f.is_file() and (f.name.startswith("COPYING") or f.name.upper().startswith("LICENSE")):
            shutil.copy2(f, lic_dir / f.name)
            licenses.append(f"share/licenses/libzim/{f.name}")

    files = {p.relative_to(out).as_posix(): sha256(p) for p in sorted(out.rglob("*")) if p.is_file()}
    xapian_pc = deps_prefix / "lib/pkgconfig/xapian-core.pc"
    xver = next((l.split(":", 1)[1].strip() for l in xapian_pc.read_text().splitlines()
                 if l.startswith("Version:")), None) if xapian_pc.exists() else None
    manifest = {
        "schema": 2, "version": "9.8.2", "target": "x86_64-pc-windows-msvc",
        "provenance": "SELF-BUILT LOCAL TEST SDK (kiwix-build deps + meson libzim, MSVC release); not release provenance",
        "source": f"https://github.com/openzim/libzim/tree/{REVISION}",
        "libzim": {"version": "9.8.2", "commit": REVISION, "reader": True, "search": True, "writer": True},
        "xapian": {"version": xver,
                   "linkage": "shared" if any("xapian" in n for n in seen) else "static (linked into zim DLL)"},
        "patches": [m for m, _ in patches],
        "sourcePatches": records,
        "localCompatPatches": local_patches,
        "dynamicLibraries": sorted(seen),
        "includeDirs": ["include"], "libraryDirs": ["lib"],
        "licenses": licenses, "files": files,
    }
    (out / "kiwix-sdk.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"SDK at {out}; DLL closure: {sorted(seen)}")


if __name__ == "__main__":
    main()
