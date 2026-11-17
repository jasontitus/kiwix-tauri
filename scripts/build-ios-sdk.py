#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Build separate patched static libzim SDKs for iOS device and arm64 simulator.

Usage: python3 scripts/build-ios-sdk.py [--target TARGET] [--jobs N]
Requires full Xcode, Python 3.10+, git, curl, make, cmake and pkg-config.
All downloads, host tools, sources and resumable build state stay in .native/ios.
The two configurations run sequentially: upstream shares its native ICU tools.
Only libzim's reader/search dependencies are built; never CoreKiwix or libkiwix.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tarfile

ROOT = Path(__file__).resolve().parent.parent
WORK = ROOT / ".native/ios"
BUILDER = WORK / "kiwix-build"
BUILD_PIN = "e7343a1640de5379c212eac2aec6ba32c9fd2d31"
ZIM_PIN = "26ec526f74e8342a40da5ab14e364546988e0e1a"
ZIM_URL = f"https://codeload.github.com/openzim/libzim/tar.gz/{ZIM_PIN}"
ZIM_SHA = "5e1030752a7e7ab98319493d3cbc1b693aea322f9822ba05dfb31819aa880809"
PATCHES = [
    ("kiwix-split-ranges-v1", "zim", ROOT / "src-tauri/native/libzim-split-ranges.patch"),
    ("kiwix-retained-fd-v1", "zim", ROOT / "src-tauri/native/libzim-retained-fd.patch"),
    ("kiwix-xapian-fd-offset-v1", "xapian", ROOT / "src-tauri/native/xapian-fd-offset.patch"),
]
TARGETS = {"aarch64-apple-ios": "ios_arm64", "aarch64-apple-ios-sim": "ios_simulator_arm64"}
TOOLS = ["setuptools==80.9.0", "wheel==0.45.1", "meson==1.9.1", "ninja==1.13.0", "distro==1.9.0", "paramiko==4.0.0"]


def run(args, **kwargs):
    print("+", shlex.join(str(x) for x in args), flush=True)
    return subprocess.run([str(x) for x in args], check=True, **kwargs)


def text(args, **kwargs):
    return subprocess.check_output([str(x) for x in args], text=True, **kwargs).strip()


def sha(file):
    h = hashlib.sha256()
    with open(file, "rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def dependencies(config, jobs):
    # Local compatibility adaptations, not edits to the pinned upstream tree.
    sys.path.insert(0, str(BUILDER))
    import kiwixbuild
    from kiwixbuild.dependencies.libzim import Libzim
    from kiwixbuild.dependencies.xapian import Xapian
    from kiwixbuild.dependencies.base import MakeBuilder
    Libzim.Builder.get_dependencies = classmethod(lambda cls, cfg, all_deps: ["lzma", "zstd", "xapian-core", "icu4c"])
    # Upstream formats a plain dict as attributes; preserve its intended flags.
    Xapian.Builder.configure_env = {
        "_format_LDFLAGS": "{env[LDFLAGS]} -L{buildEnv.install_dir}/{buildEnv.libprefix}",
        "_format_CXXFLAGS": "{env[CXXFLAGS]} -O3 -I{buildEnv.install_dir}/include",
    }
    def patch_xapian(source, context):
        context.try_skip(source.source_path)
        for _, component, patch in PATCHES:
            if component == "xapian":
                run(["patch", "--batch", "--fuzz=0", "-p1", "-i", patch], cwd=source.source_path)
    Xapian.Source._post_prepare_script = patch_xapian
    MakeBuilder.make_options = [f"-j{jobs}"]
    sys.argv = ["kiwix-build", "--working-dir", str(WORK), "--config", config,
                "--dont-install-packages", "--build-deps-only", "--libprefix", "lib", "libzim"]
    kiwixbuild.main()


def sources():
    result = [{"name": "libzim", "version": "9.8.2", "commit": ZIM_PIN, "url": ZIM_URL, "sha256": ZIM_SHA}]
    # The actual archives are authoritative, not kiwix-build's versions dictionary
    # (which labels Xapian 1.4.26 while downloading the 1.4.23 archive).
    for archive in sorted((WORK / "ARCHIVE").iterdir()):
        if archive.is_file() and not archive.name.startswith("."):
            base = ("https://github.com/unicode-org/icu/releases/download/release-73-2/"
                    if archive.name.startswith("icu4c-") else "https://dev.kiwix.org/kiwix-build/")
            result.append({"name": archive.name, "url": base + archive.name, "sha256": sha(archive)})
    return result


def dependency_patches(prefix):
    dest = prefix / "share/provenance/kiwix-build-patches"
    dest.mkdir(parents=True, exist_ok=True)
    result = []
    for name in ["lzma_meson_install.patch", "zstd_qsort.patch", "icu4c_fix_static_lib_name_mingw.patch",
                 "icu4c_rpath.patch", "icu4c_wasm.patch"]:
        file = BUILDER / "kiwixbuild/patches" / name
        shutil.copy2(file, dest / name)
        result.append({"path": str((dest / name).relative_to(prefix)), "sha256": sha(file)})
    return result


def licenses(prefix, source):
    dest = prefix / "share/licenses"
    dest.mkdir(parents=True, exist_ok=True)
    roots = [BUILDER, source] + sorted((WORK / "SOURCE").iterdir())
    for tree in roots:
        for file in tree.rglob("*"):
            if not file.is_file() or ".git" in file.parts:
                continue
            name = file.name.lower()
            if name.startswith(("copying", "copyright", "license", "licence")) or name in ("authors", "notice"):
                target = dest / tree.name / file.relative_to(tree)
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(file, target)
    for _, _, patch in PATCHES:
        shutil.copy2(patch, dest / patch.name)
    return "share/licenses"


def main():
    global WORK, BUILDER
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS)
    parser.add_argument("--jobs", type=int, default=min(8, os.cpu_count() or 4))
    parser.add_argument("--work-dir", type=Path, default=WORK)
    parser.add_argument("--prefix-root", type=Path, default=ROOT / ".native/mobile")
    parser.add_argument("--internal-dependencies", choices=TARGETS.values(), help=argparse.SUPPRESS)
    args = parser.parse_args()
    WORK = args.work_dir.resolve()
    BUILDER = WORK / "kiwix-build"
    if args.internal_dependencies:
        dependencies(args.internal_dependencies, args.jobs)
        return
    if sys.platform != "darwin" or sys.version_info < (3, 10):
        parser.error("macOS with Python 3.10+ and full Xcode is required")
    if args.jobs < 1:
        parser.error("--jobs must be positive")
    WORK.mkdir(parents=True, exist_ok=True)
    # Covers source patching and host ICU as well as both target installations.
    with open(WORK / "build.lock", "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        build(args)


def build(args):
    for tool in ["git", "curl", "cmake", "pkg-config", "make", "xcrun"]:
        if not shutil.which(tool):
            raise RuntimeError(f"Required build tool missing: {tool}")
    xcode = text(["xcodebuild", "-version"])
    patch_hashes = {marker: sha(patch) for marker, _, patch in PATCHES}
    identity = {
        "recipeSha256": sha(Path(__file__)), "kiwixBuild": BUILD_PIN,
        "libzim": ZIM_PIN, "libzimArchiveSha256": ZIM_SHA, "sourcePatches": patch_hashes,
        "xcode": xcode, "clang": text(["xcrun", "clang", "--version"]),
        "python": sys.version, "tools": TOOLS, "configurations": TARGETS,
        "cmake": text(["cmake", "--version"]), "pkgConfig": text(["pkg-config", "--version"]),
        "prefixRoot": str(args.prefix_root.resolve()),
        "sdks": {sdk: {"path": text(["xcrun", "--sdk", sdk, "--show-sdk-path"]),
                      "version": text(["xcrun", "--sdk", sdk, "--show-sdk-version"])}
                 for sdk in ("iphoneos", "iphonesimulator")},
        "environment": {key: os.environ.get(key) for key in (
            "CC", "CXX", "CPP", "AR", "RANLIB", "LD", "NM", "STRIP",
            "CPATH", "C_INCLUDE_PATH", "CPLUS_INCLUDE_PATH", "LIBRARY_PATH", "DEVELOPER_DIR")},
    }
    fingerprint = WORK / "build-inputs.json"
    if fingerprint.exists():
        if json.loads(fingerprint.read_text()) != identity:
            raise RuntimeError("SDK build inputs changed; select fresh --work-dir and --prefix-root directories")
    elif any(WORK.glob("BUILD_*")) or any(WORK.glob("libzim-build-*")) or (WORK / "SOURCE").exists():
        raise RuntimeError("Legacy SDK cache has no build-input fingerprint; select fresh --work-dir and --prefix-root directories")
    else:
        fingerprint.write_text(json.dumps(identity, indent=2) + "\n")
    if not BUILDER.exists():
        run(["git", "clone", "--no-checkout", "https://github.com/kiwix/kiwix-build.git", BUILDER])
        run(["git", "-C", BUILDER, "checkout", "--detach", BUILD_PIN])
    if text(["git", "-C", BUILDER, "rev-parse", "HEAD"]) != BUILD_PIN:
        raise RuntimeError("Unexpected kiwix-build checkout; refusing to reuse it")
    if text(["git", "-C", BUILDER, "status", "--porcelain", "--untracked-files=no"]):
        raise RuntimeError("Pinned kiwix-build has local modifications")
    venv = WORK / "venv"
    python = venv / "bin/python"
    tools_stamp = venv / "kiwix-tools.json"
    if not python.exists():
        run([sys.executable, "-m", "venv", venv])
    if not tools_stamp.exists() or json.loads(tools_stamp.read_text()) != TOOLS:
        run([python, "-m", "pip", "--isolated", "install", *TOOLS])
        tools_stamp.write_text(json.dumps(TOOLS))
    env = os.environ.copy()
    # Never let a desktop SDK or Homebrew pkg-config path leak into target lookup.
    for key in ["PKG_CONFIG_PATH", "PKG_CONFIG_LIBDIR", "PKG_CONFIG_SYSROOT_DIR", "CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS", "SDKROOT", "MACOSX_DEPLOYMENT_TARGET"]:
        env.pop(key, None)
    env["PATH"] = str(venv / "bin") + os.pathsep + env["PATH"]
    archive = WORK / f"libzim-{ZIM_PIN}.tar.gz"
    if not archive.exists():
        temporary = archive.with_suffix(archive.suffix + ".part")
        run(["curl", "--fail", "--location", "--retry", "3", "--output", temporary, ZIM_URL])
        if sha(temporary) != ZIM_SHA:
            raise RuntimeError("Downloaded libzim source archive SHA256 mismatch")
        temporary.replace(archive)
    if sha(archive) != ZIM_SHA:
        raise RuntimeError("libzim source archive SHA256 mismatch")
    zim_patches = {marker: patch_hashes[marker] for marker, component, _ in PATCHES if component == "zim"}
    zim_patch_hash = hashlib.sha256(json.dumps(zim_patches, sort_keys=True).encode()).hexdigest()
    source = WORK / f"libzim-{ZIM_PIN}-{zim_patch_hash[:16]}"
    if not (source / ".kiwix-patch.json").exists():
        if source.exists():
            raise RuntimeError(f"Incomplete source extraction: remove only {source} and retry")
        source.mkdir()
        with tarfile.open(archive) as tar:
            for member in tar.getmembers():
                relative = Path(*Path(member.name).parts[1:])
                if not relative.parts:
                    continue
                if relative.is_absolute() or ".." in relative.parts or member.issym() or member.islnk():
                    raise RuntimeError("Unsafe source archive member")
                member.name = str(relative)
                tar.extract(member, source)
        for _, component, patch in PATCHES:
            if component == "zim":
                run(["patch", "--batch", "--fuzz=0", "-p1", "-i", patch], cwd=source)
        (source / ".kiwix-patch.json").write_text(json.dumps({"sourcePatches": zim_patches, "commit": ZIM_PIN}))
    for target, config in TARGETS.items():
        if args.target and args.target != target:
            continue
        prefix = args.prefix_root.resolve() / target
        manifest_file = prefix / "kiwix-sdk.json"
        # A failed rerun must never leave a stale success manifest for consumers.
        manifest_file.unlink(missing_ok=True)
        run([python, __file__, "--internal-dependencies", config, "--jobs", args.jobs, "--work-dir", WORK], env=env)
        dep_build = WORK / f"BUILD_{config}"
        dep_prefix = dep_build / "INSTALL"
        prefix.mkdir(parents=True, exist_ok=True)
        for subdir in ["include", "lib"]:
            shutil.copytree(dep_prefix / subdir, prefix / subdir, dirs_exist_ok=True)
        # Rewrite installed .pc/.la absolute dependency prefixes to the final SDK.
        for metadata in (prefix / "lib").rglob("*"):
            if metadata.suffix in [".pc", ".la"]:
                metadata.write_text(metadata.read_text().replace(str(dep_prefix), str(prefix)))
        cross = WORK / f"{config}-libzim-cross.txt"
        cross.write_text((dep_build / "meson_cross_file.txt").read_text()
                         .replace("cpu_family = 'arm64'", "cpu_family = 'aarch64'")
                         .replace("endian = ''", "endian = 'little'")
                         .replace(str(dep_prefix), str(prefix)))
        target_env = env | {"PKG_CONFIG_LIBDIR": str(prefix / "lib/pkgconfig"), "PKG_CONFIG_PATH": ""}
        zim_build = WORK / f"libzim-build-{target}-{ZIM_PIN}-{zim_patch_hash[:16]}"
        if not (zim_build / "build.ninja").exists():
            run([venv / "bin/meson", "setup", zim_build, source, "--cross-file", cross,
                 "--prefix", prefix, "--libdir=lib", "--buildtype=release", "--default-library=static",
                 "--wrap-mode=nofallback", "-Dtests=false", "-Dexamples=false", "-Ddoc=false",
                 "-Dwith_xapian=true", "-Dwithout_writer=true", "-Dwerror=false"], env=target_env)
        run([venv / "bin/meson", "compile", "-C", zim_build, "-j", args.jobs], env=target_env)
        run([venv / "bin/meson", "install", "-C", zim_build], env=target_env)
        libs = ["zim", "xapian", "icui18n", "icuuc", "icudata", "lzma", "zstd", "z"]
        for name in libs:
            if not (prefix / f"lib/lib{name}.a").is_file():
                raise RuntimeError(f"Required native static archive missing: {name}")
        if list((prefix / "lib").glob("*.dylib")):
            raise RuntimeError("Unexpected shared library in static mobile SDK")
        provenance = prefix / "share/provenance"
        provenance.mkdir(parents=True, exist_ok=True)
        shutil.copy2(Path(__file__), provenance / "build-ios-sdk.py")
        for _, _, patch in PATCHES:
            shutil.copy2(patch, provenance / patch.name)
        manifest = {"schema": 2, "target": target, "patches": [marker for marker, _, _ in PATCHES],
                    "staticLibraries": libs, "systemLibraries": ["c++", "resolv", "m"],
                    "frameworks": [], "linkerFlags": [], "includeDirs": ["include"], "libraryDirs": ["lib"],
                    "minimumIosVersion": "15.0", "xcode": xcode,
                    "libzim": {"commit": ZIM_PIN, "version": "9.8.2"},
                    "sourcePatches": [
                        {"marker": marker, "component": component, "sha256": patch_hashes[marker],
                         "path": f"share/provenance/{patch.name}"}
                        for marker, component, patch in PATCHES
                    ],
                    "kiwixBuild": {"commit": BUILD_PIN, "url": "https://github.com/kiwix/kiwix-build.git",
                                   "adaptations": ["omit writer test-data-only dependency", "fix Xapian dict formatting", "normalize Meson aarch64/little host metadata"]},
                    "sources": sources(), "xapianVersion": "1.4.23", "icuVersion": "73.2",
                    "dependencyPatches": dependency_patches(prefix),
                    "licenses": licenses(prefix, source), "tools": TOOLS,
                    "buildInputs": identity,
                    "archives": {f"lib/lib{name}.a": sha(prefix / f"lib/lib{name}.a") for name in libs}}
        manifest_file.write_text(json.dumps(manifest, indent=2) + "\n")
        print(f"Native SDK installed: {prefix}", flush=True)


if __name__ == "__main__":
    main()
