"""Regenerate selected file sections of libzim-retained-fd.patch (idempotent).

PRE  = pinned libzim + libzim-split-ranges.patch
CUR  = PRE + current libzim-retained-fd.patch
POST = CUR + the edits below (each applied only if not already present)
Only the sections for EDITED_FILES are rewritten; the rest of the patch is kept
byte-for-byte. Verifies PRE + new patch == POST for every file.
Usage: python -I regen_patch.py <repo> <libzim tarball> <work dir>
"""
import difflib
import filecmp
import shutil
import subprocess
import sys
import tarfile
from pathlib import Path

PATCH_EXE = r"C:\Program Files\Git\usr\bin\patch.exe"
repo, tarball, work = map(Path, sys.argv[1:4])
native = repo / "src-tauri/native"
target_patch = native / "libzim-retained-fd.patch"

H_DECLS = ["static std::shared_ptr<FileCompound> openSinglePieceOrSplitZimFile(const std::string& filename);",
           "explicit FileCompound(const std::string& filename);",
           "explicit FileCompound(const std::string& filename, MultiPartToken token);",
           "explicit FileCompound(int fd);",
           "explicit FileCompound(FdInput fd);",
           "explicit FileCompound(const std::vector<FdInput>& fds);",
           "time_t getMTime() const;",
           "void addPart(std::unique_ptr<FilePart> fpart);"]
EDITS = {
    "src/file_compound.h": [
        ("class LIBZIM_PRIVATE_API FileCompound : private std::map",
         "// Only out-of-line members are exported: exporting the class itself would\n"
         "// make MSVC instantiate every member of the std::map base, including the\n"
         "// copying insert that cannot copy std::unique_ptr (C2280).\n"
         "class FileCompound : private std::map"),
        # Keep destruction inside zim.dll, where addPart allocated the parts.
        ("    ~FileCompound() = default;\n", "    LIBZIM_PRIVATE_API ~FileCompound();\n"),
    ] + [("    " + d + "\n", "    LIBZIM_PRIVATE_API " + d + "\n") for d in H_DECLS] + [
        # After the member exports above.
        ("    LIBZIM_PRIVATE_API explicit FileCompound(const std::vector<FdInput>& fds);\n#endif\n",
         "    LIBZIM_PRIVATE_API explicit FileCompound(const std::vector<FdInput>& fds);\n"
         "#else\n"
         "    LIBZIM_PRIVATE_API explicit FileCompound(const std::vector<std::string>& partPaths);\n"
         "#endif\n"),
    ],
    "src/file_compound.cpp": [(
        "  const auto size = fpart->size();\n"
        "  if (!size || size.v > std::numeric_limits<offset_type>::max() - _fsize.v) {\n"
        "    throw std::runtime_error(\"Invalid compound file size\");\n"
        "  }\n"
        "  const Range newRange(offset_t(_fsize.v), offset_t(_fsize.v + size.v));\n"
        "  emplace(newRange, std::move(fpart));\n",
        "  const auto size = fpart->size();\n"
        "  // Parts are keyed by their start offset. A lone empty piece is kept so an\n"
        "  // empty archive still fails as ZimFileFormatError, but an empty piece can\n"
        "  // never share a compound with another part.\n"
        "  if ((!empty() && (!size || !_fsize.v))\n"
        "      || size.v > std::numeric_limits<offset_type>::max() - _fsize.v) {\n"
        "    throw std::runtime_error(\"Invalid compound file size\");\n"
        "  }\n"
        "  const Range newRange(offset_t(_fsize.v), offset_t(_fsize.v + size.v));\n"
        "  if (!emplace(newRange, std::move(fpart)).second) {\n"
        "    throw std::runtime_error(\"Invalid compound file size\");\n"
        "  }\n"), (
        "  try {\n"
        "    for (char ch0 = 'a'; ch0 <= 'z'; ++ch0)\n"
        "    {\n"
        "      const std::string fname0 = base_filename + ch0;\n"
        "      for (char ch1 = 'a'; ch1 <= 'z'; ++ch1)\n"
        "      {\n"
        "        addPart(std::unique_ptr<FilePart>(new FilePart(fname0 + ch1)));\n"
        "      }\n"
        "    }\n"
        "  } catch (std::runtime_error& e) {\n"
        "    // This catch acts as a break for the double loop.\n"
        "  }\n",
        "  for (char ch0 = 'a'; ch0 <= 'z'; ++ch0)\n"
        "  {\n"
        "    const std::string fname0 = base_filename + ch0;\n"
        "    for (char ch1 = 'a'; ch1 <= 'z'; ++ch1)\n"
        "    {\n"
        "      std::unique_ptr<FilePart> part;\n"
        "      try {\n"
        "        part.reset(new FilePart(fname0 + ch1));\n"
        "      } catch (std::runtime_error&) {\n"
        "        // The first part that cannot be opened ends the split set.\n"
        "        return;\n"
        "      }\n"
        "      // An empty discovered part (such as a stray zero-length sibling)\n"
        "      // ends the set, as before; an overflowing or colliding part is a\n"
        "      // damaged set and is reported instead of silently truncating it.\n"
        "      if (!part->size()) return;\n"
        "      addPart(std::move(part));\n"
        "    }\n"
        "  }\n"), (
        "    addPart(std::unique_ptr<FilePart>(new FilePart(fd)));\n  }\n}\n#endif\n",
        "    addPart(std::unique_ptr<FilePart>(new FilePart(fd)));\n  }\n}\n"
        "#else\n"
        "FileCompound::FileCompound(const std::vector<std::string>& partPaths):\n"
        "  _filename(partPaths.empty() ? std::string() : partPaths.front()),\n"
        "  _fsize(0)\n"
        "{\n"
        "  if (partPaths.empty()) throw std::runtime_error(\"Empty split part list\");\n"
        "  for (const auto& path: partPaths) {\n"
        "    addPart(std::unique_ptr<FilePart>(new FilePart(path)));\n"
        "  }\n"
        "}\n"
        "#endif\n"), (
        "time_t FileCompound::getMTime() const {\n",
        "FileCompound::~FileCompound() = default;\n"
        "\n"
        "time_t FileCompound::getMTime() const {\n")],
    # Windows counterpart of the FdInput constructors: open exactly the named,
    # ordered part set instead of discovering `.zimXX` siblings by name.
    "include/zim/archive.h": [(
        "      Archive(const std::vector<FdInput>& fds, OpenConfig openConfig);\n#endif\n",
        "      Archive(const std::vector<FdInput>& fds, OpenConfig openConfig);\n"
        "#else\n"
        "      /** Open exactly the given ZIM parts, in order (Windows).\n"
        "       *\n"
        "       *  Unlike opening the first part by name, no further `.zimXX` parts\n"
        "       *  are discovered: name discovery only recognizes a lower-case\n"
        "       *  `.zimaa` suffix and greedily adds every following sibling, so it\n"
        "       *  is not bound to the caller's proven part set. This is the\n"
        "       *  Windows counterpart of the FdInput constructors.\n"
        "       *\n"
        "       *  @param partPaths The UTF-8 path of every part, in archive order.\n"
        "       *  @param openConfig The open configuration to use.\n"
        "       */\n"
        "      Archive(const std::vector<std::string>& partPaths, OpenConfig openConfig);\n"
        "#endif\n")],
    "src/archive.cpp": [(
        "  Archive::Archive(const std::vector<FdInput>& fds, OpenConfig openConfig)\n"
        "    : m_impl(new FileImpl(fds, openConfig))\n"
        "    { }\n"
        "#endif\n",
        "  Archive::Archive(const std::vector<FdInput>& fds, OpenConfig openConfig)\n"
        "    : m_impl(new FileImpl(fds, openConfig))\n"
        "    { }\n"
        "#else\n"
        "  Archive::Archive(const std::vector<std::string>& partPaths, OpenConfig openConfig)\n"
        "    : m_impl(new FileImpl(partPaths, openConfig))\n"
        "    { }\n"
        "#endif\n")],
    "src/fileimpl.h": [(
        "      FileImpl(const std::vector<FdInput>& fds, OpenConfig openConfig);\n#endif\n",
        "      FileImpl(const std::vector<FdInput>& fds, OpenConfig openConfig);\n"
        "#else\n"
        "      FileImpl(const std::vector<std::string>& partPaths, OpenConfig openConfig);\n"
        "#endif\n")],
    # Windows no longer reopens archives by path (see tools.cpp), so drop the
    # path from the private native access record entirely.
    "src/file_part.h": [(
        "  zsize_t length{0};\n#ifdef _WIN32\n  std::string filename;\n#endif\n",
        "  zsize_t length{0};\n")],
    "src/fileimpl.cpp": [(
        "    info.length = length;\n#ifdef _WIN32\n    info.filename = part->filename();\n#endif\n",
        "    info.length = length;\n"), (
        "    : FileImpl(std::make_shared<FileCompound>(fds), openConfig)\n  {}\n#endif\n",
        "    : FileImpl(std::make_shared<FileCompound>(fds), openConfig)\n  {}\n"
        "#else\n"
        "  FileImpl::FileImpl(const std::vector<std::string>& partPaths, OpenConfig openConfig)\n"
        "    : FileImpl(std::make_shared<FileCompound>(partPaths), openConfig)\n"
        "  {}\n"
        "#endif\n")],
    # ENABLE_XAPIAN comes from config.h, which this test only includes later
    # (via tools.h), so the guarded include was always skipped while the
    # patch's Xapian test below still used zim::Searcher. search.h is a public
    # header installed in every build.
    "test/archive.cpp": [(
        "#if defined(ENABLE_XAPIAN)\n#include <zim/search.h>\n#endif\n",
        "#include <zim/search.h>\n")],
    "src/tools.cpp": [(
        "    // Windows has only genuinely path-opened archives; preserve its native\n"
        "    // path implementation rather than inventing a POSIX descriptor alias.\n"
        "    auto databasefd = zim::DEFAULTFS::openFile(accessInfo.filename);\n",
        "    // Reopen through the part's own handle, never its path: archives are\n"
        "    // opened with FILE_SHARE_DELETE, so the path may since name another\n"
        "    // file. ReOpenFile yields a new file object (an independent position\n"
        "    // for Xapian) bound to the same file.\n"
        "    const HANDLE reopened = ReOpenFile(accessInfo.handle->getNativeHandle(), GENERIC_READ,\n"
        "                                       FILE_SHARE_READ | FILE_SHARE_DELETE, FILE_FLAG_RANDOM_ACCESS);\n"
        "    if (reopened == INVALID_HANDLE_VALUE) return false;\n"
        "    zim::DEFAULTFS::FD databasefd(reopened);\n")],
    "src/fs_windows.h": [(
        "    bool    close();\n};",
        "    bool    close();\n    fd_t    getNativeHandle() const;\n};")],
    "src/fs_windows.cpp": [(
        "int FD::release()\n",
        "FD::fd_t FD::getNativeHandle() const\n"
        "{\n"
        "  return mp_impl ? mp_impl->m_handle : INVALID_HANDLE_VALUE;\n"
        "}\n"
        "\n"
        "int FD::release()\n"), (
        "  handle = CreateFileW(wpath.get(),\n"
        "             GENERIC_READ,\n"
        "             FILE_SHARE_READ,\n",
        "  // FILE_SHARE_DELETE matches POSIX: an archive that is open for reading\n"
        "  // can still be deleted, renamed or moved; open readers keep their bytes.\n"
        "  handle = CreateFileW(wpath.get(),\n"
        "             GENERIC_READ,\n"
        "             FILE_SHARE_READ | FILE_SHARE_DELETE,\n")],
}
# Section boundaries inside the patch: (first header line, next file's header line).
SECTIONS = [("src/file_compound.cpp", "src/file_compound.h"), ("src/file_compound.h", "src/file_part.h"),
            ("src/fs_windows.cpp", "src/fs_windows.h"), ("src/fs_windows.h", "src/suggestion.cpp"),
            ("src/tools.cpp", "src/tools.h")]


def extract(dest):
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    with tarfile.open(tarball) as packed:
        for member in packed.getmembers():
            parts = member.name.split("/", 1)
            if len(parts) > 1 and parts[1]:
                member.name = parts[1]
                packed.extract(member, dest, filter="data")


def apply(tree, patch):
    subprocess.run([PATCH_EXE, "--batch", "--fuzz=0", "-p1", "--binary", "--input", str(patch)],
                   cwd=tree, check=True, stdout=subprocess.DEVNULL)


def fresh_copy(source, dest):
    if dest.exists():
        shutil.rmtree(dest)
    shutil.copytree(source, dest)


pre, cur, post, check = (work / n for n in ("pre", "cur", "post", "check"))
extract(pre)
apply(pre, native / "libzim-split-ranges.patch")
fresh_copy(pre, cur)
# Start from the committed (HEAD) patch so every edit below is an
# original -> final transformation, independent of earlier regenerations.
head_patch = work / "head-retained-fd.patch"
head_patch.write_bytes(subprocess.run(
    ["git", "-C", str(repo), "-c", "core.autocrlf=false", "show", "HEAD:src-tauri/native/libzim-retained-fd.patch"],
    check=True, capture_output=True).stdout)
apply(cur, head_patch)
fresh_copy(cur, post)
for rel, edits in EDITS.items():
    path = post / rel
    text = path.read_text(encoding="utf-8")
    for old, new in edits:
        # Idempotent: decide "already applied" without being fooled when one
        # side contains the other.
        if old in new:            # insertion around old: applied iff new present
            applied = new in text
        elif new in old:          # deletion from old: applied iff old absent
            applied = old not in text and new in text
        else:
            applied = old not in text and new in text
        if applied:
            continue
        assert text.count(old) == 1, f"{rel}: expected exactly one occurrence of:\n{old}"
        text = text.replace(old, new)
    path.write_text(text, encoding="utf-8", newline="")


def section(rel):
    a = (pre / rel).read_text(encoding="utf-8").splitlines(keepends=True)
    b = (post / rel).read_text(encoding="utf-8").splitlines(keepends=True)
    return "".join(difflib.unified_diff(a, b, f"a/{rel}", f"b/{rel}", n=3))


patch_text = target_patch.read_bytes().decode("utf-8")
for first, following in SECTIONS:
    start = patch_text.index(f"--- a/{first}\n")
    end = patch_text.index(f"--- a/{following}\n")
    patch_text = patch_text[:start] + section(first) + patch_text[end:]
# Files edited above that have no section yet (or whose section is not in
# SECTIONS) are handled here: replace an existing section up to the next
# header, or insert a new one before the first header that sorts after it.
import re as _re
handled = {first for first, _ in SECTIONS}
for rel in EDITS:
    if rel in handled:
        continue
    headers = [(m.start(), m.group(1)) for m in _re.finditer(r"^--- a/(\S+)\n", patch_text, _re.M)]
    current = [i for i, (_, name) in enumerate(headers) if name == rel]
    if current:
        i = current[0]
        start = headers[i][0]
        end = headers[i + 1][0] if i + 1 < len(headers) else len(patch_text)
        patch_text = patch_text[:start] + section(rel) + patch_text[end:]
    else:
        later = [pos for pos, name in headers if name > rel]
        at = later[0] if later else len(patch_text)
        patch_text = patch_text[:at] + section(rel) + patch_text[at:]
target_patch.write_bytes(patch_text.encode("utf-8"))

fresh_copy(pre, check)
apply(check, target_patch)


def mismatches(d, prefix=""):
    out = [prefix + f for f in d.diff_files + d.left_only + d.right_only]
    for name, sub in d.subdirs.items():
        out += mismatches(sub, prefix + name + "/")
    return out


bad = mismatches(filecmp.dircmp(check, post))
for rel in EDITS:
    if (check / rel).read_bytes() != (post / rel).read_bytes():
        bad.append(rel)
print("mismatches:", bad or "none")
sys.exit(1 if bad else 0)
