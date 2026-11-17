#!/usr/bin/env python3.12
"""Build the patched, static Android reader/search SDK on macOS.

Run: python3.12 scripts/build-android-sdk.py [--target aarch64-linux-android]
     [--jobs 6] [--ndk /path/to/r28c]
Use --target armv7-linux-androideabi for an isolated armeabi-v7a SDK.
The ARMv7 SDK additionally applies the required LP32 bounded raw-blob correction;
64-bit SDKs include the shared retained-input and decoder cache/lifetime corrections.
Requires Python 3.12+, Xcode command-line tools, CMake, Ninja, Meson and pkg-config.
Downloads, host tools, sources, logs and resumable build steps stay in .native/.
Completed steps are reused; a failed invocation resumes at the failed step. To
rebuild with different compiler options, use a fresh --work-dir and --prefix.
--downloads-dir may reuse checksum-verified source archives, never target outputs.
The SDK embeds ICU data and a static NDK C++ runtime; it needs no data asset or
libc++_shared.so. Consumers MUST apply the manifest's linkerFlags to their final
shared library to retain Android 16 KiB-page compatibility.
The reader-only ICU package omits the currency, timezone-display, measurement-unit
and rule-based-number-format service trees, unused by libzim's fixed search
normalizer. All other package members, including every locale, collation,
transliterator, break iterator/dictionary and converter, remain byte-identical.
This is not a general-purpose ICU formatting SDK. The exact package policy and
before/after member hashes are installed under share/provenance/.
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
import tempfile

REVISION = "26ec526f74e8342a40da5ab14e364546988e0e1a"
ICU_READER_DATA_POLICY = {
    "id": "reader-services-v1",
    "excludedTrees": ["curr", "zone", "unit", "rbnf"],
}
TARGETS = {
    "aarch64-linux-android": {
        "abi": "arm64-v8a", "compiler": "aarch64-linux-android",
        "host": "aarch64-linux-android", "runtime": "aarch64-linux-android",
        "cpuFamily": "aarch64", "cpu": "aarch64", "unwind": "aarch64",
        "elfClass": "ELF64", "elfMachine": "AArch64", "flags": [],
        "workDir": "android-sdk-build",
    },
    "armv7-linux-androideabi": {
        "abi": "armeabi-v7a", "compiler": "armv7a-linux-androideabi",
        "host": "arm-linux-androideabi", "runtime": "arm-linux-androideabi",
        "cpuFamily": "arm", "cpu": "armv7", "unwind": "arm",
        "elfClass": "ELF32", "elfMachine": "ARM",
        "flags": ["-march=armv7-a", "-mthumb", "-mfpu=neon", "-mfloat-abi=softfp"],
        "workDir": "android-armv7-sdk-build",
    },
}
MIN_API = 24  # Includes the 32-bit _FILE_OFFSET_BITS=64 stdio entry points.
SOURCES = {
    "zlib": ("zlib-1.3.1.tar.gz", "https://zlib.net/fossils/zlib-1.3.1.tar.gz", "9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23"),
    "xz": ("xz-5.2.6.tar.gz", "https://dev.kiwix.org/kiwix-build/xz-5.2.6.tar.gz", "a2105abee17bcd2ebd15ced31b4f5eda6e17efd6b10f921a01cda4a44c91b3a0"),
    "zstd": ("zstd-1.5.7.tar.gz", "https://dev.kiwix.org/kiwix-build/zstd-1.5.7.tar.gz", "eb33e51f49a15e023950cd7825ca74a4a2b43db8354825ac24fc1b7ee09e6fa3"),
    "xapian": ("xapian-core-1.4.32.tar.xz", "https://oligarchy.co.uk/xapian/1.4.32/xapian-core-1.4.32.tar.xz", "c4fd64e81127311756adf5579268d14a79f285ce8ac4ead0930c96195897aece"),
    "icu": ("icu4c-73_2-src.tgz", "https://github.com/unicode-org/icu/releases/download/release-73-2/icu4c-73_2-src.tgz", "818a80712ed3caacd9b652305e01afc7fa167e6f2e94996da44b90c2ab604ce1"),
    "zim": (f"libzim-{REVISION}.tar.gz", f"https://codeload.github.com/openzim/libzim/tar.gz/{REVISION}", "5e1030752a7e7ab98319493d3cbc1b693aea322f9822ba05dfb31819aa880809"),
}


def sha256(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def main():
    repo = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ndk", type=Path, default=Path(os.environ.get("ANDROID_NDK_HOME", str(Path.home() / "Library/Android/sdk/ndk/28.2.13676358"))))
    parser.add_argument("--jobs", type=int, default=6)
    parser.add_argument("--target", choices=TARGETS, default="aarch64-linux-android")
    parser.add_argument("--work-dir", type=Path)
    parser.add_argument("--prefix", type=Path)
    parser.add_argument("--downloads-dir", type=Path, help="Reuse checksum-verified source archives only")
    args = parser.parse_args()
    if sys.version_info < (3, 12):
        parser.error("Use Python 3.12 or newer (for example python3.12)")
    if platform.system() != "Darwin" or args.jobs < 1:
        parser.error("This route requires macOS and a positive --jobs count")
    target = TARGETS[args.target]
    ndk = args.ndk.resolve()
    work = (args.work_dir or repo / ".native" / target["workDir"]).resolve()
    prefix = (args.prefix or repo / ".native/mobile" / args.target).resolve()
    downloads = (args.downloads_dir or work / "downloads").resolve()
    properties = (ndk / "source.properties").read_text()
    if "28.2.13676358" not in properties:
        parser.error("Use the pinned NDK r28c (28.2.13676358)")
    tools = {name: shutil.which(name) for name in ("cmake", "ninja", "meson", "pkg-config", "make", "patch", "curl")}
    if not all(tools.values()):
        parser.error(f"Missing tools: {[name for name, path in tools.items() if not path]}")
    toolchain = ndk / "toolchains/llvm/prebuilt/darwin-x86_64"
    bin_dir = toolchain / "bin"
    cc = bin_dir / f"{target['compiler']}{MIN_API}-clang"
    cxx = bin_dir / f"{target['compiler']}{MIN_API}-clang++"
    for compiler in (cc, cxx):
        if not compiler.is_file():
            parser.error(f"Missing compiler: {compiler}")
    compiler_flags = ["-O2", "-fPIC", "-D_FILE_OFFSET_BITS=64", "-D_LARGEFILE_SOURCE=1", *target["flags"]]
    for directory in (downloads, work / "source", work / "build", work / "logs", prefix / "lib/pkgconfig", prefix / "share/licenses", prefix / "share/provenance"):
        directory.mkdir(parents=True, exist_ok=True)
    patches = [
        ("kiwix-split-ranges-v1", "zim", repo / "src-tauri/native/libzim-split-ranges.patch"),
        ("kiwix-retained-fd-v1", "zim", repo / "src-tauri/native/libzim-retained-fd.patch"),
        ("kiwix-xapian-fd-offset-v1", "xapian", repo / "src-tauri/native/xapian-fd-offset.patch"),
        ("kiwix-zstd-cache-accounting-v1", "zim", repo / "src-tauri/native/libzim-zstd-cache-accounting.patch"),
        ("kiwix-decoded-cluster-retirement-v1", "zim", repo / "src-tauri/native/libzim-decoded-cluster-retirement.patch"),
    ]
    if args.target == "armv7-linux-androideabi":
        patches.append(("kiwix-lp32-raw-blob-v1", "zim", repo / "src-tauri/native/libzim-lp32-raw-blob.patch"))
    patch_hashes = {marker: sha256(path) for marker, _, path in patches}
    recipe_sha = sha256(Path(__file__))
    ndk_properties_sha = hashlib.sha256(properties.encode()).hexdigest()
    clean_env = os.environ.copy()
    for key in ("CC", "CXX", "CPP", "AR", "RANLIB", "LD", "NM", "STRIP", "CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS", "LIBS", "CPATH", "C_INCLUDE_PATH", "CPLUS_INCLUDE_PATH", "LIBRARY_PATH", "SDKROOT", "MACOSX_DEPLOYMENT_TARGET", "PKG_CONFIG_PATH", "PKG_CONFIG_LIBDIR", "PKG_CONFIG_SYSROOT_DIR", "CMAKE_PREFIX_PATH"):
        clean_env.pop(key, None)
    env = clean_env | {
        "PATH": f"{bin_dir}:{clean_env['PATH']}",
        "CC": str(cc), "CXX": str(cxx), "AR": str(bin_dir / "llvm-ar"),
        "RANLIB": str(bin_dir / "llvm-ranlib"), "NM": str(bin_dir / "llvm-nm"),
        "STRIP": str(bin_dir / "llvm-strip"),
        "CFLAGS": shlex.join(compiler_flags),
        "CXXFLAGS": shlex.join(compiler_flags),
        "CPPFLAGS": f"-I{prefix / 'include'}",
        "LDFLAGS": f"-L{prefix / 'lib'} -static-libstdc++ -Wl,-z,max-page-size=16384",
        "PKG_CONFIG": tools["pkg-config"], "PKG_CONFIG_PATH": "",
        "PKG_CONFIG_LIBDIR": str(prefix / "lib/pkgconfig"),
    }
    identity = json.loads(json.dumps({
        "target": args.target, "abi": target["abi"], "api": MIN_API,
        "compiler": {"c": str(cc), "cxx": str(cxx), "flags": compiler_flags, "autotoolsHost": target["host"]},
        "ndk": str(ndk), "ndkProperties": ndk_properties_sha,
        "prefix": str(prefix), "sources": SOURCES, "patches": patch_hashes, "recipe": recipe_sha,
        "icuDataPolicy": ICU_READER_DATA_POLICY,
    }))
    work_identity = work / ".kiwix-build-inputs.json"
    if work_identity.exists() and json.loads(work_identity.read_text()) != identity:
        raise RuntimeError("SDK work directory inputs changed; select a fresh --work-dir and --prefix")
    work_identity.write_text(json.dumps(identity, indent=2) + "\n")
    manifest_file = prefix / "kiwix-sdk.json"
    if manifest_file.exists():
        previous = json.loads(manifest_file.read_text())
        if previous.get("schema") != 2 or previous.get("buildInputs") != identity:
            raise RuntimeError("SDK build inputs changed; select a fresh --work-dir and --prefix")
        # Never expose a prior success manifest while rebuilding its libraries.
        manifest_file.unlink()
    source_dirs = {}
    log = None

    def run(command, cwd, environment=env):
        command = [str(part) for part in command]
        log.write(("$ " + shlex.join(command) + "\n").encode())
        log.flush()
        subprocess.run(command, cwd=cwd, env=environment, stdout=log, stderr=subprocess.STDOUT, check=True)

    for name, (filename, url, expected) in SOURCES.items():
        archive = downloads / filename
        if not archive.exists():
            temporary = archive.with_suffix(archive.suffix + ".part")
            subprocess.run([tools["curl"], "--fail", "--location", "--retry", "3", "--output", str(temporary), url], check=True)
            temporary.replace(archive)
        if sha256(archive) != expected:
            raise RuntimeError(f"Source checksum mismatch: {archive}")
        directory = work / "source" / filename.split(".tar")[0].replace(".tgz", "")
        source_dirs[name] = directory
        extracted = directory / ".kiwix-extracted"
        if not extracted.exists():
            directory.mkdir(parents=True, exist_ok=True)
            with tarfile.open(archive) as packed:
                for member in packed.getmembers():
                    components = member.name.split("/", 1)
                    if len(components) > 1 and components[1]:
                        member.name = components[1]
                        packed.extract(member, directory, filter="data")
            extracted.write_text(expected + "\n")

    def stage(name, build, output_root=prefix):
        nonlocal log
        stamp = work / "build" / f"{name}.complete.json"
        if stamp.exists():
            previous = json.loads(stamp.read_text())
            if previous.get("inputs") != identity or previous.get("outputRoot") != str(output_root):
                raise RuntimeError(f"Build settings changed; select a fresh --work-dir and --prefix ({stamp})")
            outputs = previous.get("outputs")
            if isinstance(outputs, dict) and outputs and all(
                (output_root / relative).is_file() and sha256(output_root / relative) == digest
                for relative, digest in outputs.items()
            ):
                print(f"Reusing verified completed {name}", flush=True)
                return
            print(f"Restoring missing or changed installed outputs for {name}", flush=True)
            stamp.unlink()
        path = work / "logs" / f"{name}.log"
        print(f"Building {name}; log: {path}", flush=True)
        with path.open("ab") as log:
            try:
                build()
            except subprocess.CalledProcessError as error:
                raise RuntimeError(f"{name} failed ({error.returncode}); see {path}. Rerun to resume this step.") from error
        outputs = {str(path.relative_to(output_root)): sha256(path)
                   for path in sorted(output_root.rglob("*")) if path.is_file() and path.name != "kiwix-sdk.json"}
        if not outputs:
            raise RuntimeError(f"Stage {name} produced no installed outputs")
        stamp.write_text(json.dumps({"inputs": identity, "outputRoot": str(output_root), "outputs": outputs}, indent=2) + "\n")

    def cmake(name, source, options):
        build = work / "build" / name
        run([tools["cmake"], "-S", source, "-B", build, "-G", "Ninja",
             f"-DCMAKE_TOOLCHAIN_FILE={ndk / 'build/cmake/android.toolchain.cmake'}",
             f"-DANDROID_ABI={target['abi']}", f"-DANDROID_PLATFORM=android-{MIN_API}", "-DANDROID_STL=c++_static",
             "-DANDROID_SUPPORT_FLEXIBLE_PAGE_SIZES=ON", "-DCMAKE_BUILD_TYPE=Release",
             "-DCMAKE_POSITION_INDEPENDENT_CODE=ON", f"-DCMAKE_INSTALL_PREFIX={prefix}",
             "-DCMAKE_INSTALL_LIBDIR=lib", *options], work)
        run([tools["cmake"], "--build", build, "--parallel", args.jobs], work)
        run([tools["cmake"], "--install", build], work)

    def autotools(name, source, options, environment=env, make_options=()):
        build = work / "build" / name
        build.mkdir(parents=True, exist_ok=True)
        run([source / "configure", f"--prefix={prefix}", f"--host={target['host']}", "--disable-shared", "--enable-static", "--with-pic", *options], build, environment)
        run([tools["make"], f"-j{args.jobs}", *make_options], build, environment)
        run([tools["make"], "install", *make_options], build, environment)
    def apply_patches(component):
        source = source_dirs[component]
        for marker, target, patch in patches:
            if target != component:
                continue
            applied = source / f".{marker}"
            if applied.exists():
                if applied.read_text().strip() != patch_hashes[marker]:
                    raise RuntimeError("SDK patch changed; use a fresh --work-dir and --prefix")
                continue
            run([tools["patch"], "--batch", "--fuzz=0", "-p1", "--input", patch], source)
            applied.write_text(patch_hashes[marker] + "\n")

    def xapian():
        apply_patches("xapian")
        autotools("xapian", source_dirs["xapian"], ["--disable-sse", "--disable-backend-chert", "--disable-backend-remote", "--disable-documentation"])


    def zlib():
        cmake("zlib", source_dirs["zlib"], ["-DZLIB_BUILD_EXAMPLES=OFF"])
        # zlib 1.3.1 builds both forms; only its static archive belongs in this SDK.
        for shared in (prefix / "lib").glob("libz.so*"):
            shared.unlink()

    stage("zlib", zlib)
    stage("xz", lambda: autotools("xz", source_dirs["xz"], ["--disable-xz", "--disable-xzdec", "--disable-lzmadec", "--disable-lzmainfo", "--disable-lzma-links", "--disable-scripts", "--disable-doc", "--disable-nls"]))
    stage("zstd", lambda: cmake("zstd", source_dirs["zstd"] / "build/cmake", ["-DZSTD_BUILD_PROGRAMS=OFF", "-DZSTD_BUILD_TESTS=OFF", "-DZSTD_BUILD_SHARED=OFF", "-DZSTD_BUILD_STATIC=ON"]))
    stage("xapian", xapian)

    icu = source_dirs["icu"] / "source"
    host_build = work / "build/icu-host"
    icu_options = ["--disable-shared", "--enable-static", "--disable-tests", "--disable-samples", "--disable-extras", "--disable-icuio", "--disable-layoutex", "--disable-dyload", "--with-data-packaging=static"]

    def icu_host():
        host_build.mkdir(parents=True, exist_ok=True)
        host_env = clean_env | {"CC": "/usr/bin/clang", "CXX": "/usr/bin/clang++", "CFLAGS": "-O2", "CXXFLAGS": "-O2"}
        run([icu / "configure", f"--prefix={work / 'icu-host'}", *icu_options], host_build, host_env)
        run([tools["make"], f"-j{args.jobs}"], host_build, host_env)

    stage("icu-host", icu_host, host_build)
    icu_data = work / "build/icu-reader-data"
    filtered_package = icu_data / "icudt73l.dat"
    icu_data_receipt = icu_data / "icu-reader-data.json"

    def icu_reader_data():
        # The pinned release tar contains prebuilt data, not locale text sources:
        # ICU_DATA_FILTER_FILE would be ignored. Filter whole service trees only.
        # Keep top-level locale/pool/metadata resources (including timezoneTypes,
        # required by collation), and every other member, byte for byte.
        icu_data.mkdir(parents=True, exist_ok=True)
        original = icu / "data/in/icudt73l.dat"
        icupkg = host_build / "bin/icupkg"
        original_list = icu_data / "original-members.txt"
        retained_list = icu_data / "retained-members.txt"
        removal_list = icu_data / "removed-members.txt"
        run([icupkg, "--list", "-o", original_list, original], work, clean_env)
        original_names = sorted(original_list.read_text().splitlines())
        excluded_prefixes = tuple(f"{tree}/" for tree in ICU_READER_DATA_POLICY["excludedTrees"])
        removed = [name for name in original_names if name.startswith(excluded_prefixes)]
        retained = [name for name in original_names if not name.startswith(excluded_prefixes)]
        if not removed or not retained:
            raise RuntimeError("Pinned ICU package does not match the reader data policy")
        removal_list.write_text("".join(f"{name}\n" for name in removed))
        # Deliberately never --ignore-deps: missing parents/pools/aliases fail.
        run([icupkg, "--remove", removal_list, original, filtered_package], work, clean_env)
        run([icupkg, "--list", "-o", retained_list, filtered_package], work, clean_env)
        if sorted(retained_list.read_text().splitlines()) != retained:
            raise RuntimeError("Filtered ICU package changed members outside the reader data policy")
        with tempfile.TemporaryDirectory(prefix="icu-data-members-", dir=work / "build") as temporary:
            original_members = Path(temporary) / "original"
            filtered_members = Path(temporary) / "filtered"
            original_members.mkdir()
            filtered_members.mkdir()
            run([icupkg, "-x", "*", "-d", original_members, original], work, clean_env)
            run([icupkg, "-x", "*", "-d", filtered_members, filtered_package], work, clean_env)
            inventory = {}
            for name in original_names:
                path = original_members / name
                digest = sha256(path)
                inventory[name] = {"bytes": path.stat().st_size, "sha256": digest}
                if not name.startswith(excluded_prefixes) and sha256(filtered_members / name) != digest:
                    raise RuntimeError(f"Retained ICU data changed: {name}")
        icu_data_receipt.write_text(json.dumps({
            "schema": 1, "policy": ICU_READER_DATA_POLICY,
            "sourcePackage": {"sha256": sha256(original), "bytes": original.stat().st_size},
            "filteredPackage": {"sha256": sha256(filtered_package), "bytes": filtered_package.stat().st_size},
            "retained": {name: inventory[name] for name in retained},
            "removed": {name: inventory[name] for name in removed},
        }, indent=2, sort_keys=True) + "\n")

    stage("icu-reader-data", icu_reader_data, icu_data)
    stage("icu-target", lambda: autotools(
        "icu-target", icu, [f"--with-cross-build={host_build}", "--disable-tools", *icu_options],
        make_options=[f"ICUDATA_SOURCE_ARCHIVE={filtered_package}"]))
    cross = work / f"android-{target['abi']}.ini"
    cross.write_text("\n".join([
        "[binaries]", f"c = '{cc}'", f"cpp = '{cxx}'", f"ar = '{bin_dir / 'llvm-ar'}'", f"strip = '{bin_dir / 'llvm-strip'}'", f"pkg-config = '{tools['pkg-config']}'",
        "[host_machine]", "system = 'android'", f"cpu_family = '{target['cpuFamily']}'", f"cpu = '{target['cpu']}'", "endian = 'little'",
        "[properties]", "needs_exe_wrapper = true", f"pkg_config_libdir = '{prefix / 'lib/pkgconfig'}'",
        "[built-in options]", f"c_args = {compiler_flags!r}", f"cpp_args = {compiler_flags!r}", "c_link_args = ['-Wl,-z,max-page-size=16384']", "cpp_link_args = ['-static-libstdc++', '-Wl,-z,max-page-size=16384']", "",
    ]))

    def zim():
        source = source_dirs["zim"]
        apply_patches("zim")
        build = work / "build/zim"
        command = [tools["meson"], "setup", build, source, "--cross-file", cross, f"--prefix={prefix}", "--libdir=lib", "--buildtype=release", "--default-library=static", "--wrap-mode=nofallback", "-Dtests=false", "-Dexamples=false", "-Ddoc=false", "-Dwith_xapian=true", "-Dwithout_writer=true", "-Dstatic-linkage=true", "-DUSE_BUFFER_HEADER=false", "-Dwerror=false"]
        if (build / "meson-private/coredata.dat").exists():
            command.append("--reconfigure")
        run(command, work)
        run([tools["meson"], "compile", "-C", build, "-j", args.jobs], work)
        run([tools["meson"], "install", "-C", build], work)

    stage("zim", zim)
    runtime_dir = toolchain / "sysroot/usr/lib" / target["runtime"]
    for name in ("c++_static", "c++abi"):
        shutil.copy2(runtime_dir / f"lib{name}.a", prefix / "lib" / f"lib{name}.a")
    shutil.copy2(toolchain / "lib/clang/19/lib/linux" / target["unwind"] / "libunwind.a", prefix / "lib/libunwind.a")
    licenses = []
    for name, source in source_dirs.items():
        for license_file in sorted(source.iterdir()):
            if license_file.is_file() and (license_file.name.startswith("COPYING") or license_file.name.upper().startswith("LICENSE")):
                destination = prefix / "share/licenses" / name / license_file.name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(license_file, destination)
                licenses.append(str(destination.relative_to(prefix)))
    shutil.copy2(ndk / "NOTICE", prefix / "share/licenses/android-ndk-NOTICE")
    licenses.append("share/licenses/android-ndk-NOTICE")
    shutil.copy2(ndk / "NOTICE.toolchain", prefix / "share/licenses/android-ndk-toolchain-NOTICE")
    licenses.append("share/licenses/android-ndk-toolchain-NOTICE")
    for _, _, patch in patches:
        shutil.copy2(patch, prefix / "share/provenance" / patch.name)
    shutil.copy2(Path(__file__), prefix / "share/provenance/build-android-sdk.py")
    shutil.copy2(icu_data_receipt, prefix / "share/provenance/icu-reader-data.json")
    static_libraries = ["zim", "xapian", "lzma", "zstd", "z", "icui18n", "icuuc", "icudata", "c++_static", "c++abi", "unwind"]
    archive_objects = {}
    for name in static_libraries:
        archive = prefix / "lib" / f"lib{name}.a"
        if not archive.is_file():
            raise RuntimeError(f"SDK is incomplete: lib{name}.a is missing")
        headers = subprocess.run([str(bin_dir / "llvm-readelf"), "--file-header", str(archive)],
                                 check=True, capture_output=True, text=True).stdout
        classes = [line.split(":", 1)[1].strip() for line in headers.splitlines() if line.strip().startswith("Class:")]
        machines = [line.split(":", 1)[1].strip() for line in headers.splitlines() if line.strip().startswith("Machine:")]
        if not classes or len(classes) != len(machines) or any(value != target["elfClass"] for value in classes) or any(value != target["elfMachine"] for value in machines):
            raise RuntimeError(f"SDK archive contains missing or wrong-target ELF objects: {archive}")
        archive_objects[f"lib/lib{name}.a"] = {"class": target["elfClass"], "machine": target["elfMachine"], "members": len(classes)}
    manifest = {
        "schema": 2, "version": "9.8.2", "target": args.target, "api": MIN_API,
        "androidAbi": target["abi"], "compiler": identity["compiler"],
        "archiveObjects": archive_objects,
        "source": f"https://github.com/openzim/libzim/tree/{REVISION}",
        "libzim": {"version": "9.8.2", "commit": REVISION, "reader": True, "search": True, "writer": False},
        "patches": [marker for marker, _, _ in patches],
        "sourcePatches": [
            {"marker": marker, "component": component, "sha256": patch_hashes[marker],
             "path": f"share/provenance/{patch.name}"}
            for marker, component, patch in patches
        ],
        "buildInputs": identity,
        "sources": [{"name": name, "archive": filename, "url": url, "sha256": checksum} for name, (filename, url, checksum) in SOURCES.items()],
        "ndk": {"revision": "28.2.13676358", "sourcePropertiesSha256": sha256(ndk / "source.properties")},
        "icuData": "embedded-static", "cxxRuntime": "ndk-static",
        "icuDataPolicy": {**ICU_READER_DATA_POLICY, "receipt": "share/provenance/icu-reader-data.json",
                          "receiptSha256": sha256(icu_data_receipt)},
        "staticLibraries": static_libraries, "systemLibraries": ["m", "dl", "log"], "frameworks": [],
        "linkerFlags": ["-Wl,-z,max-page-size=16384"], "includeDirs": ["include"], "libraryDirs": ["lib"],
        "licenses": licenses,
        "files": {str(path.relative_to(prefix)): sha256(path) for path in sorted(prefix.rglob("*")) if path.is_file() and path.name != "kiwix-sdk.json"},
    }
    manifest_file.write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"Android {target['abi']} API {MIN_API} SDK: {prefix}\nManifest: {prefix / 'kiwix-sdk.json'}", flush=True)


if __name__ == "__main__":
    main()
