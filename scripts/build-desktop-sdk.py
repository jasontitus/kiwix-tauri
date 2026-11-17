#!/usr/bin/env python3.12
"""Build the matched, patched dynamic Xapian/libzim SDK for native Linux/macOS.

Run with Python 3.12+ and native C/C++ compilers, make, Meson, Ninja,
pkg-config, patch and curl. System ICU, lzma, zstd and zlib development files
are required. Only Xapian and libzim are built here. Work/prefix identities
are immutable: changed sources, patches, recipe, tools or system dependencies
require fresh --work-dir and --prefix. Failed stages can resume unchanged.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shlex
import shutil
import subprocess
import sys
import tarfile

REVISION = "26ec526f74e8342a40da5ab14e364546988e0e1a"
SOURCES = {
    "xapian": ("xapian-core-1.4.32.tar.xz", "https://oligarchy.co.uk/xapian/1.4.32/xapian-core-1.4.32.tar.xz", "c4fd64e81127311756adf5579268d14a79f285ce8ac4ead0930c96195897aece"),
    "zim": (f"libzim-{REVISION}.tar.gz", f"https://codeload.github.com/openzim/libzim/tar.gz/{REVISION}", "5e1030752a7e7ab98319493d3cbc1b693aea322f9822ba05dfb31819aa880809"),
}


def sha256(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def inventory(root):
    return {str(path.relative_to(root)): sha256(path) for path in sorted(root.rglob("*"))
            if path.is_file() and path.name not in ("kiwix-sdk.json", "kiwix-sdk.json.part", ".kiwix-build-inputs.json")}


def main():
    repo = Path(__file__).resolve().parent.parent
    system, machine = platform.system(), platform.machine()
    targets = {
        ("Darwin", "arm64"): "aarch64-apple-darwin",
        ("Darwin", "x86_64"): "x86_64-apple-darwin",
        ("Linux", "aarch64"): "aarch64-unknown-linux-gnu",
        ("Linux", "x86_64"): "x86_64-unknown-linux-gnu",
    }
    target = targets.get((system, machine))
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jobs", type=int, default=6)
    parser.add_argument("--work-dir", type=Path)
    parser.add_argument("--prefix", type=Path)
    args = parser.parse_args()
    if sys.version_info < (3, 12):
        parser.error("Use Python 3.12 or newer")
    if not target or args.jobs < 1:
        parser.error("Native glibc Linux/macOS arm64 or x86_64 and positive --jobs are required")
    if system == "Linux" and platform.libc_ver()[0] != "glibc":
        parser.error("This recipe targets glibc Linux, not musl")
    work = (args.work_dir or repo / ".native/desktop-retained-build" / target).resolve()
    prefix = (args.prefix or repo / ".native/desktop-retained" / target).resolve()
    if work == prefix or work in prefix.parents or prefix in work.parents:
        parser.error("Work and prefix must be separate, non-nested directories")
    for path in (work, prefix):
        if any(part in ("mobile", "mobile-reviewed", "ios-reviewed", "android-sdk-build-reviewed") for part in path.parts):
            parser.error("Never reuse an existing mobile/reviewed SDK directory")
    tools = {name: shutil.which(name) for name in ("cc", "c++", "make", "meson", "ninja", "pkg-config", "patch", "curl")}
    if not all(tools.values()):
        parser.error(f"Missing tools: {[name for name, path in tools.items() if not path]}")
    patches = [
        ("kiwix-split-ranges-v1", "zim", repo / "src-tauri/native/libzim-split-ranges.patch"),
        ("kiwix-retained-fd-v1", "zim", repo / "src-tauri/native/libzim-retained-fd.patch"),
        ("kiwix-xapian-fd-offset-v1", "xapian", repo / "src-tauri/native/xapian-fd-offset.patch"),
    ]
    patch_hashes = {marker: sha256(path) for marker, _, path in patches}
    environment = os.environ.copy()
    for key in ("CC", "CXX", "CPP", "AR", "RANLIB", "LD", "NM", "STRIP", "CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS", "LIBS", "CPATH", "C_INCLUDE_PATH", "CPLUS_INCLUDE_PATH", "LIBRARY_PATH", "SDKROOT", "MACOSX_DEPLOYMENT_TARGET", "PKG_CONFIG_LIBDIR", "PKG_CONFIG_SYSROOT_DIR", "CMAKE_PREFIX_PATH", "CONFIG_SITE", "XAPIAN_CONFIG"):
        environment.pop(key, None)
    environment["LC_ALL"] = "C"
    environment["CONFIG_SITE"] = "/dev/null"

    def capture(command):
        return subprocess.check_output([str(part) for part in command], env=environment, text=True).strip()

    tool_identity = {
        name: {"path": path, "sha256": sha256(Path(path).resolve()),
               "version": capture([path, "--version"])}
        for name, path in tools.items()
    }
    if system == "Darwin":
        tool_identity["appleSdk"] = {
            "path": capture(["xcrun", "--show-sdk-path"]),
            "version": capture(["xcrun", "--sdk", "macosx", "--show-sdk-version"]),
            "build": capture(["xcrun", "--show-sdk-build-version"]),
        }
    compiler_target = capture([tools["c++"], "-dumpmachine"])
    architecture = compiler_target.split("-", 1)[0]
    expected_architectures = ("arm64", "aarch64") if machine in ("arm64", "aarch64") else ("x86_64",)
    if architecture not in expected_architectures or ("apple" in compiler_target) != (system == "Darwin"):
        parser.error(f"Compiler {compiler_target} does not target native {target}")
    tool_identity["compilerTarget"] = compiler_target
    # Resolve native system dependencies before the private SDK can affect lookup.
    dependencies = []
    for package, libraries in (("icu-i18n", ("icui18n", "icuuc", "icudata")),
                               ("liblzma", ("lzma",)), ("libzstd", ("zstd",)), ("zlib", ("z",))):
        pcdir = Path(capture([tools["pkg-config"], "--variable=pcfiledir", package]))
        libdir = Path(capture([tools["pkg-config"], "--variable=libdir", package]))
        runtime = {}
        for library in libraries:
            pattern = f"lib{library}*.dylib" if system == "Darwin" else f"lib{library}.so*"
            matches = sorted(libdir.glob(pattern))
            if not matches and system == "Linux":
                # Some Debian .pc files report /usr/lib while the native linker
                # resolves the actual library from its multiarch search path.
                linked = Path(capture([tools["c++"], f"-print-file-name=lib{library}.so"]))
                if linked.is_file():
                    matches = [linked.resolve()]
            if not matches:
                raise RuntimeError(f"Missing native dynamic dependency {library} in {libdir}")
            runtime.update({str(path): sha256(path) for path in matches if path.is_file()})
        dependencies.append({
            "name": package, "version": capture([tools["pkg-config"], "--modversion", package]),
            "cflags": capture([tools["pkg-config"], "--cflags", package]),
            "libs": capture([tools["pkg-config"], "--libs", package]),
            "libdir": str(libdir),
            "pcfile": str(pcdir / f"{package}.pc"), "pcfileSha256": sha256(pcdir / f"{package}.pc"),
            "runtimeFiles": runtime,
        })
    system_pcpath = environment.get("PKG_CONFIG_PATH", "")
    environment.update({
        "CC": tools["cc"], "CXX": tools["c++"],
        "CFLAGS": "-O2 -fPIC -D_FILE_OFFSET_BITS=64 -D_LARGEFILE_SOURCE=1",
        "CXXFLAGS": "-O2 -fPIC -D_FILE_OFFSET_BITS=64 -D_LARGEFILE_SOURCE=1",
        "CPPFLAGS": " ".join([f"-I{shlex.quote(str(prefix / 'include'))}", *[dep["cflags"] for dep in dependencies]]),
        "LDFLAGS": " ".join([f"-L{shlex.quote(str(prefix / 'lib'))}", f"-Wl,-rpath,{shlex.quote(str(prefix / 'lib'))}",
                             *[f"-L{shlex.quote(dep['libdir'])}" for dep in dependencies]]),
        "PKG_CONFIG": tools["pkg-config"],
        "PKG_CONFIG_PATH": os.pathsep.join(filter(None, [str(prefix / "lib/pkgconfig"), system_pcpath])),
    })
    if system == "Darwin":
        environment["SDKROOT"] = tool_identity["appleSdk"]["path"]
    # Serialize once so tuples and paths cannot differ when comparing a resumed run.
    identity = json.loads(json.dumps({
        "target": target, "prefix": str(prefix), "work": str(work), "sources": SOURCES,
        "patches": patch_hashes, "recipe": sha256(Path(__file__)), "tools": tool_identity,
        "systemDependencies": dependencies,
        "environment": {key: environment.get(key) for key in ("PATH", "CC", "CXX", "CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS", "PKG_CONFIG_PATH", "SDKROOT")},
    }))
    for directory in (work, prefix):
        marker = directory / ".kiwix-build-inputs.json"
        if marker.exists():
            if json.loads(marker.read_text()) != identity:
                raise RuntimeError(f"SDK build inputs changed at {directory}; use fresh --work-dir and --prefix")
        else:
            if directory.exists() and any(directory.iterdir()):
                raise RuntimeError(f"Refusing unowned nonempty directory: {directory}")
    for directory in (work, prefix):
        directory.mkdir(parents=True, exist_ok=True)
        (directory / ".kiwix-build-inputs.json").write_text(json.dumps(identity, indent=2) + "\n")
    for directory in (work / "downloads", work / "source", work / "build", work / "logs", prefix / "share/licenses", prefix / "share/provenance"):
        directory.mkdir(parents=True, exist_ok=True)
    manifest_file = prefix / "kiwix-sdk.json"
    # Never leave an old success declaration visible during a resumed install.
    manifest_file.unlink(missing_ok=True)
    log = None

    def run(command, cwd):
        command = [str(part) for part in command]
        log.write(("$ " + shlex.join(command) + "\n").encode())
        log.flush()
        subprocess.run(command, cwd=cwd, env=environment, stdout=log, stderr=subprocess.STDOUT, check=True)

    source_dirs = {}
    for name, (filename, url, expected) in SOURCES.items():
        archive = work / "downloads" / filename
        if not archive.exists():
            temporary = archive.with_suffix(archive.suffix + ".part")
            subprocess.run([tools["curl"], "--fail", "--location", "--retry", "3", "--output", str(temporary), url], env=environment, check=True)
            if sha256(temporary) != expected:
                raise RuntimeError(f"Downloaded source checksum mismatch: {temporary}")
            temporary.replace(archive)
        if sha256(archive) != expected:
            raise RuntimeError(f"Source checksum mismatch: {archive}")
        source = work / "source" / name
        source_dirs[name] = source
        extracted = source / ".kiwix-extracted"
        if extracted.exists():
            if extracted.read_text().strip() != expected:
                raise RuntimeError(f"Source identity changed: {source}; use fresh directories")
        else:
            if source.exists():
                # A failed extraction is not a completed or patched source tree.
                shutil.rmtree(source)
            source.mkdir()
            with tarfile.open(archive) as packed:
                for member in packed.getmembers():
                    components = member.name.split("/", 1)
                    if len(components) > 1 and components[1]:
                        member.name = components[1]
                        packed.extract(member, source, filter="data")
            extracted.write_text(expected + "\n")

    def stage(name, build):
        nonlocal log
        stamp = work / "build" / f"{name}.complete.json"
        if stamp.exists():
            previous = json.loads(stamp.read_text())
            if previous["buildInputs"] != identity:
                raise RuntimeError(f"Stage fingerprint mismatch: {stamp}")
            for relative, expected in previous["files"].items():
                path = prefix / relative
                if not path.is_file() or sha256(path) != expected:
                    raise RuntimeError(f"Completed stage output changed: {path}; use fresh directories")
            print(f"Reusing completed {name}", flush=True)
            return
        path = work / "logs" / f"{name}.log"
        print(f"Building {name}; log: {path}", flush=True)
        with path.open("ab") as log:
            try:
                build()
            except subprocess.CalledProcessError as error:
                raise RuntimeError(f"{name} failed ({error.returncode}); see {path}. Rerun unchanged to resume.") from error
        stamp.write_text(json.dumps({"buildInputs": identity, "files": inventory(prefix)}, indent=2) + "\n")

    def apply_patches(component):
        source = source_dirs[component]
        for marker, owner, patch in patches:
            if owner != component:
                continue
            applied = source / f".{marker}"
            if applied.exists():
                if applied.read_text().strip() != patch_hashes[marker]:
                    raise RuntimeError("SDK patch changed; use fresh directories")
                continue
            run([tools["patch"], "--batch", "--fuzz=0", "-p1", "--input", patch], source)
            applied.write_text(patch_hashes[marker] + "\n")
        fingerprint = work / "source" / f"{component}.patched-files.json"
        files = inventory(source)
        if fingerprint.exists():
            if json.loads(fingerprint.read_text()) != files:
                raise RuntimeError(f"Patched source tree changed: {source}; use fresh directories")
        else:
            fingerprint.write_text(json.dumps(files, indent=2) + "\n")

    def xapian():
        apply_patches("xapian")
        build = work / "build/xapian"
        build.mkdir(exist_ok=True)
        run([source_dirs["xapian"] / "configure", f"--prefix={prefix}", f"--libdir={prefix / 'lib'}",
             "--enable-shared", "--disable-static", "--with-pic", "--disable-sse",
             "--disable-backend-chert", "--disable-backend-remote", "--disable-documentation"], build)
        run([tools["make"], f"-j{args.jobs}"], build)
        run([tools["make"], "install"], build)

    stage("xapian", xapian)
    # Do not permit Meson to select a stock system Xapian through any discovery route.
    environment["XAPIAN_CONFIG"] = str(prefix / "bin/xapian-config")
    selected = Path(capture([tools["pkg-config"], "--variable=prefix", "xapian-core"])).resolve()
    if selected != prefix or capture([tools["pkg-config"], "--modversion", "xapian-core"]) != "1.4.32":
        raise RuntimeError("pkg-config did not select the matched patched Xapian 1.4.32")

    def zim():
        apply_patches("zim")
        build = work / "build/zim"
        command = [tools["meson"], "setup", build, source_dirs["zim"], f"--prefix={prefix}", "--libdir=lib",
                   "--buildtype=release", "--default-library=shared", "--wrap-mode=nofallback",
                   "-Dtests=false", "-Dexamples=false", "-Ddoc=false", "-Dwith_xapian=true", "-Dwerror=false"]
        if (build / "meson-private/coredata.dat").exists():
            command.append("--reconfigure")
        run(command, work)
        run([tools["meson"], "compile", "-C", build, "-j", args.jobs], work)
        run([tools["meson"], "install", "-C", build], work)

    stage("zim", zim)
    licenses = []
    for name, source in source_dirs.items():
        for license_file in sorted(source.iterdir()):
            if license_file.is_file() and (license_file.name.startswith("COPYING") or license_file.name.upper().startswith("LICENSE")):
                destination = prefix / "share/licenses" / name / license_file.name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(license_file, destination)
                licenses.append(str(destination.relative_to(prefix)))
        if not any(path.startswith(f"share/licenses/{name}/") for path in licenses):
            raise RuntimeError(f"Missing license for {name}")
    for _, _, patch in patches:
        shutil.copy2(patch, prefix / "share/provenance" / patch.name)
    shutil.copy2(Path(__file__), prefix / "share/provenance/build-desktop-sdk.py")
    extension = "dylib" if system == "Darwin" else "so"
    for library in ("zim", "xapian"):
        if not (prefix / "lib" / f"lib{library}.{extension}").is_file():
            raise RuntimeError(f"Missing dynamic {library} library")
    manifest = {
        "schema": 2, "version": "9.8.2", "target": target,
        "source": f"https://github.com/openzim/libzim/tree/{REVISION}",
        "libzim": {"version": "9.8.2", "commit": REVISION, "reader": True, "search": True, "writer": True},
        "xapian": {"version": "1.4.32", "linkage": "shared"},
        "patches": [marker for marker, _, _ in patches],
        "sourcePatches": [{"marker": marker, "component": component, "sha256": patch_hashes[marker],
                           "path": f"share/provenance/{patch.name}"} for marker, component, patch in patches],
        "buildInputs": identity,
        "sources": [{"name": name, "archive": filename, "url": url, "sha256": checksum}
                    for name, (filename, url, checksum) in SOURCES.items()],
        "systemDependencies": dependencies,
        "dynamicLibraries": ["zim", "xapian"], "includeDirs": ["include"], "libraryDirs": ["lib"],
        "licenses": licenses, "files": inventory(prefix),
    }
    temporary = manifest_file.with_suffix(".json.part")
    temporary.write_text(json.dumps(manifest, indent=2) + "\n")
    temporary.replace(manifest_file)
    print(f"Desktop SDK: {prefix}\nTarget: {target}\nManifest: {manifest_file}", flush=True)
    print(f"export KIWIX_DESKTOP_SDK_ROOT={shlex.quote(str(prefix))}", flush=True)
    print(f"export {'DYLD_LIBRARY_PATH' if system == 'Darwin' else 'LD_LIBRARY_PATH'}={shlex.quote(str(prefix / 'lib'))}", flush=True)


if __name__ == "__main__":
    main()
