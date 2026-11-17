// SPDX-License-Identifier: GPL-3.0-or-later
#include "archive.h"

#include <zim/archive.h>
#include <zim/blob.h>
#include <zim/error.h>
#include <zim/item.h>
#include <zim/search.h>
#include <zim/suggestion.h>

#include <algorithm>
#include <climits>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <stdexcept>
#include <string>
#include <unordered_set>
#include <vector>
#ifdef _WIN32
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

struct WindowsPathLease {
    explicit WindowsPathLease(HANDLE value) : value(value) {}
    WindowsPathLease(const WindowsPathLease &) = delete;
    WindowsPathLease &operator=(const WindowsPathLease &) = delete;
    WindowsPathLease(WindowsPathLease &&other) noexcept : value(other.value) {
        other.value = INVALID_HANDLE_VALUE;
    }
    ~WindowsPathLease() { if (value != INVALID_HANDLE_VALUE) CloseHandle(value); }
    HANDLE value;
};
#endif

struct kz_archive {
    // Keep full-text initialization on first search, not every archive open.
    explicit kz_archive(const std::string &path)
        : archive(path, zim::OpenConfig().preloadXapianDb(false)) {}
#ifndef _WIN32
    explicit kz_archive(const std::vector<zim::FdInput> &files)
        : archive(files, zim::OpenConfig().preloadXapianDb(false)) {}
#else
    // libzim opens exactly these leased parts, in order; it never discovers
    // `.zimXX` siblings by name (case-sensitive, and greedy for extra files).
    kz_archive(std::vector<WindowsPathLease> guards, const std::vector<std::string> &parts)
        : path_leases(std::move(guards)), archive(parts, zim::OpenConfig().preloadXapianDb(false)) {}
    // Destroy the archive before releasing the names it is allowed to open.
    std::vector<WindowsPathLease> path_leases;
#endif
    zim::Archive archive;
};

namespace {
constexpr uint64_t MAX_READ = 16 * 1024 * 1024;
constexpr size_t MAX_STRING = 1024 * 1024;
constexpr size_t MAX_QUERY = 64 * 1024;
constexpr uint32_t MAX_HITS = 1000;
constexpr size_t MAX_JSON = 32 * 1024 * 1024;
constexpr size_t MAX_REDIRECTS = 256;

struct BridgeError : std::runtime_error {
    BridgeError(int code, const char *message) : std::runtime_error(message), code(code) {}
    int code;
};

// Error construction is allocation-free apart from malloc, whose failure is
// represented by an empty payload. In particular, this is safe in a bad_alloc catch.
kz_result failure(int code, const char *message) noexcept {
    const size_t len = std::strlen(message);
    auto *data = static_cast<uint8_t *>(std::malloc(len));
    if (data && len) std::memcpy(data, message, len);
    return {data, data ? len : 0, code};
}

kz_result result(const std::string &value) {
    if (value.size() > MAX_JSON) throw BridgeError(KZ_INVALID_ARGUMENT, "Archive response exceeds 32 MiB limit");
    auto *data = static_cast<uint8_t *>(std::malloc(value.size()));
    if (!data && !value.empty()) throw std::bad_alloc();
    if (!value.empty()) std::memcpy(data, value.data(), value.size());
    return {data, value.size(), KZ_OK};
}

template<class F> kz_result guarded(F &&fn) noexcept {
    try {
        return fn();
    } catch (const BridgeError &e) {
        return failure(e.code, e.what());
    } catch (const zim::EntryNotFound &e) {
        return failure(KZ_NOT_FOUND, e.what());
    } catch (const zim::ZimFileFormatError &e) {
        return failure(KZ_MALFORMED, e.what());
    } catch (const std::out_of_range &e) {
        return failure(KZ_MALFORMED, e.what());
    } catch (const std::bad_alloc &) {
        return failure(KZ_INTERNAL, "Insufficient memory for archive operation");
    } catch (const std::exception &e) {
        return failure(KZ_INTERNAL, e.what());
    } catch (...) {
        return failure(KZ_INTERNAL, "Unknown native archive exception");
    }
}

std::string input_string(const uint8_t *data, size_t len, size_t maximum = MAX_STRING) {
    if ((!data && len) || len > maximum)
        throw BridgeError(KZ_INVALID_ARGUMENT, "Invalid or oversized archive string");
    return len ? std::string(reinterpret_cast<const char *>(data), len) : std::string();
}

void quoted(std::string &out, const std::string &value) {
    if (value.size() > MAX_STRING)
        throw BridgeError(KZ_MALFORMED, "Archive string exceeds 1 MiB limit");
    static const char hex[] = "0123456789abcdef";
    out += '"';
    for (unsigned char ch : value) {
        if (ch == '"' || ch == '\\') {
            out += '\\';
            out += static_cast<char>(ch);
        } else if (ch < 0x20) {
            out += "\\u00";
            out += hex[ch >> 4];
            out += hex[ch & 15];
        } else {
            out += static_cast<char>(ch);
        }
    }
    out += '"';
    if (out.size() > MAX_JSON)
        throw BridgeError(KZ_INVALID_ARGUMENT, "Archive response exceeds 32 MiB limit");
}

void field(std::string &out, const char *key, const std::string &value) {
    if (out.back() != '{') out += ',';
    quoted(out, key);
    out += ':';
    quoted(out, value);
}
void number(std::string &out, const char *key, uint64_t value) {
    if (out.back() != '{') out += ',';
    quoted(out, key);
    out += ':';
    out += std::to_string(value);
}
void boolean(std::string &out, const char *key, bool value) {
    if (out.back() != '{') out += ',';
    quoted(out, key);
    out += value ? ":true" : ":false";
}

zim::Archive &archive_of(kz_archive *handle) {
    if (!handle) throw BridgeError(KZ_INVALID_ARGUMENT, "Null archive handle");
    return handle->archive;
}

zim::Entry resolved(zim::Entry entry) {
    std::unordered_set<zim::entry_index_type> visited;
    while (entry.isRedirect()) {
        if (visited.size() == MAX_REDIRECTS || !visited.insert(entry.getIndex()).second)
            throw BridgeError(KZ_MALFORMED, "Cyclic or excessive archive redirect chain (limit 256)");
        try {
            entry = entry.getRedirectEntry();
        } catch (const zim::EntryNotFound &) {
            throw BridgeError(KZ_MALFORMED, "Archive redirect points to an absent entry");
        }
    }
    return entry;
}

std::string item_json(const zim::Item &item) {
    std::string out = "{";
    field(out, "path", item.getPath());
    field(out, "title", item.getTitle());
    field(out, "mime", item.getMimetype());
    number(out, "size", item.getSize());
    out += '}';
    return out;
}

std::string metadata_value(zim::Archive &archive, const char *key) {
    try {
        const auto item = archive.getMetadataItem(key);
        const auto size = item.getSize();
        if (size > MAX_STRING)
            throw BridgeError(KZ_MALFORMED, "Archive metadata exceeds 1 MiB limit");
        if (!size) return {};
        const auto blob = item.getData(0, size);
        if (blob.size() != size)
            throw BridgeError(KZ_MALFORMED, "Truncated archive metadata");
        return std::string(blob.data(), static_cast<size_t>(blob.size()));
    } catch (const zim::EntryNotFound &) {
        return {};
    }
}

std::string metadata(zim::Archive &archive) {
    std::string out = "{";
    field(out, "id", static_cast<std::string>(archive.getUuid()));
    const char *keys[][2] = {
        {"title", "Title"}, {"description", "Description"}, {"language", "Language"},
        {"creator", "Creator"}, {"publisher", "Publisher"}, {"date", "Date"}, {"name", "Name"}
    };
    for (const auto &key : keys) field(out, key[0], metadata_value(archive, key[1]));
    const auto tags = metadata_value(archive, "Tags");
    out += ",\"tags\":[";
    size_t start = 0;
    bool first = true;
    while (start < tags.size()) {
        auto end = tags.find(';', start);
        if (end == std::string::npos) end = tags.size();
        if (end != start) {
            if (!first) out += ',';
            quoted(out, tags.substr(start, end - start));
            first = false;
        }
        start = end + 1;
    }
    out += ']';
    number(out, "size", archive.getFilesize());
    // libzim's count APIs internally read Counter metadata in full. Preflight
    // its size so a malicious metadata item cannot turn this into a media read.
    try {
        if (archive.getMetadataItem("Counter").getSize() > MAX_STRING)
            throw BridgeError(KZ_MALFORMED, "Archive Counter metadata exceeds 1 MiB limit");
    } catch (const zim::EntryNotFound &) {
        // A missing Counter is supported by libzim's namespace-count fallback.
    }
    number(out, "articleCount", archive.getArticleCount());
    number(out, "mediaCount", archive.getMediaCount());
    boolean(out, "hasFulltextIndex", archive.hasFulltextIndex());
    boolean(out, "hasTitleIndex", archive.hasTitleIndex());
    std::string main_path;
    if (archive.hasMainEntry()) main_path = resolved(archive.getMainEntry()).getPath();
    field(out, "mainPath", main_path);
    out += '}';
    return out;
}

void hit(std::string &out, const std::string &path, const std::string &title,
         const std::string &snippet, uint64_t score, bool &first) {
    if (!first) out += ',';
    first = false;
    out += '{';
    field(out, "path", path);
    field(out, "title", title);
    field(out, "snippet", snippet);
    number(out, "score", score);
    out += '}';
}

std::string search(zim::Archive &archive, const std::string &query, bool fulltext,
                   uint32_t offset, uint32_t limit, bool snippets) {
    if (limit > MAX_HITS || offset > static_cast<uint32_t>(INT_MAX) ||
        limit > static_cast<uint32_t>(INT_MAX) - offset)
        throw BridgeError(KZ_INVALID_ARGUMENT, "Search limit must be at most 1000; pagination must fit signed 32 bits");
    std::string out = "{\"hits\":[";
    bool first = true;
    int total = 0;
    if (fulltext) {
        if (!archive.hasFulltextIndex())
            throw BridgeError(KZ_UNSUPPORTED, "Archive has no full-text search index");
#if defined(LIBZIM_WITH_XAPIAN)
        zim::Searcher searcher(archive);
        searcher.setVerbose(false);
        const auto search = searcher.search(zim::Query(query));
        total = search.getEstimatedMatches();
        if (limit) {
            const auto results = search.getResults(static_cast<int>(offset), static_cast<int>(limit));
            // libzim constructs heap-owned iterator state in end(); reuse it.
            const auto end = results.end();
            for (auto it = results.begin(); it != end; ++it) {
                const auto path = resolved(*it).getPath();
                hit(out, path, it.getTitle(), snippets ? it.getSnippet() : std::string(), std::max(0, it.getScore()), first);
            }
        }
#else
        throw BridgeError(KZ_UNSUPPORTED, "libzim was built without full-text search support");
#endif
    } else {
        // libzim uses its suggestion index when available, otherwise its
        // logarithmic title-prefix range. No full-archive scan or silent FT fallback.
        zim::SuggestionSearcher searcher(archive);
        searcher.setVerbose(false);
        const auto search = searcher.suggest(query);
        total = search.getEstimatedMatches();
        if (limit) {
            const auto results = search.getResults(static_cast<int>(offset), static_cast<int>(limit));
            const auto end = results.end();
            for (auto it = results.begin(); it != end; ++it) {
                if (snippets) {
                    const auto path = resolved(it.getEntry()).getPath();
                    hit(out, path, it->getTitle(), it->getSnippet(), 0, first);
                } else {
                    // SuggestionIterator::operator-> materializes a snippet even
                    // for getTitle(). Read the entry directly in title-only mode.
                    auto entry = it.getEntry();
                    const auto title = entry.getTitle();
                    const auto path = resolved(std::move(entry)).getPath();
                    hit(out, path, title, "", 0, first);
                }
            }
        }
    }
    out += "],\"total\":";
    out += std::to_string(std::max(0, total));
    out += '}';
    return out;
}

// Public libzim title listing: direct bounded offset, never a private index
// reader or a repeated walk from the beginning. Titles only; no article bodies.
std::string title_batch(zim::Archive &archive, uint32_t offset, uint32_t limit) {
    if (!limit || limit > 256 || offset > static_cast<uint32_t>(INT_MAX) ||
        limit > static_cast<uint32_t>(INT_MAX) - offset)
        throw BridgeError(KZ_INVALID_ARGUMENT, "Title batch limit must be 1..256 and offset must fit signed 32 bits");
    const auto range = archive.iterByTitle();
    const auto total = range.size();
    if (total < 0 || offset > static_cast<uint32_t>(total))
        throw BridgeError(KZ_INVALID_ARGUMENT, "Title batch offset exceeds title count");
    std::string out = "{\"titles\":[";
    bool first = true;
    for (const auto &entry : range.offset(static_cast<int>(offset), static_cast<int>(limit))) {
        const auto title = entry.getTitle();
        if (title.size() > 16 * 1024)
            throw BridgeError(KZ_MALFORMED, "Spelling title exceeds 16 KiB limit");
        if (!first) out += ',';
        first = false;
        quoted(out, title);
    }
    out += "],\"total\":";
    out += std::to_string(total);
    out += '}';
    return out;
}

zim::Item illustration_item(zim::Archive &archive) {
    // Preserve the historical favicon fallback in libzim's default lookup.
    try {
        return archive.getIllustrationItem();
    } catch (const zim::EntryNotFound &) {
        const auto illustrations = archive.getIllustrationInfos();
        if (illustrations.empty()) throw;
        return archive.getIllustrationItem(illustrations.front());
    }
}
} // namespace

extern "C" kz_result kz_open(const uint8_t *path, size_t path_len, kz_archive **out) noexcept {
    if (!out) return failure(KZ_INVALID_ARGUMENT, "Null archive output pointer");
    *out = nullptr;
    return guarded([&]() -> kz_result {
        const auto filename = input_string(path, path_len);
        if (filename.empty() || filename.find('\0') != std::string::npos)
            throw BridgeError(KZ_INVALID_ARGUMENT, "Archive filesystem path is empty or contains NUL");
        auto handle = std::make_unique<kz_archive>(filename);
        *out = handle.release();
        return {nullptr, 0, KZ_OK};
    });
}

extern "C" kz_result kz_open_files(const kz_file_input *files, size_t count, kz_archive **out) noexcept {
    if (!out) return failure(KZ_INVALID_ARGUMENT, "Null archive output pointer");
    *out = nullptr;
    return guarded([&]() -> kz_result {
        if (!files || count == 0 || count > 676)
            throw BridgeError(KZ_INVALID_ARGUMENT, "Invalid retained archive part count");
        uint64_t expected_size = 0;
        for (size_t index = 0; index < count; ++index) {
            if (files[index].size == 0 || files[index].size > UINT64_MAX - expected_size)
                throw BridgeError(KZ_INVALID_ARGUMENT, "Invalid retained archive part size");
            expected_size += files[index].size;
        }
#ifndef _WIN32
        std::vector<zim::FdInput> inputs;
        inputs.reserve(count);
        for (size_t index = 0; index < count; ++index) {
            if (files[index].handle < 0 || files[index].handle > INT_MAX || files[index].size == 0)
                throw BridgeError(KZ_INVALID_ARGUMENT, "Invalid retained archive descriptor");
            inputs.push_back(zim::FdInput{static_cast<int>(files[index].handle), 0, files[index].size});
        }
        auto handle = std::make_unique<kz_archive>(inputs);
#else
        std::vector<WindowsPathLease> leases;
        const auto first_name = input_string(files[0].path, files[0].path_len);
        if (first_name.empty() || first_name.find('\0') != std::string::npos)
            throw BridgeError(KZ_INVALID_ARGUMENT, "Invalid retained archive filename");
        auto wide_path = [](const std::string &name) {
            if (name.find('/') != std::string::npos)
                throw BridgeError(KZ_INVALID_ARGUMENT, "Retained archive paths must use native separators");
            const int length = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, name.data(),
                static_cast<int>(name.size()), nullptr, 0);
            if (!length) throw BridgeError(KZ_INVALID_ARGUMENT, "Invalid UTF-8 archive filename");
            std::wstring path(static_cast<size_t>(length), L'\0');
            if (MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, name.data(),
                static_cast<int>(name.size()), &path[0], length) != length)
                throw BridgeError(KZ_INVALID_ARGUMENT, "Invalid UTF-8 archive filename");
            return path;
        };
        const auto first = wide_path(first_name);
        const auto separator = first.find_last_of(L'\\');
        if (separator == std::wstring::npos)
            throw BridgeError(KZ_INVALID_ARGUMENT, "Retained archive path must be absolute");
        auto parent = first.substr(0, separator + 1);
        auto lease = [&](const wchar_t *path, bool directory) -> HANDLE {
            const HANDLE value = CreateFileW(path, directory ? FILE_READ_ATTRIBUTES : GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr, OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | (directory ? FILE_FLAG_BACKUP_SEMANTICS : 0), nullptr);
            if (value == INVALID_HANDLE_VALUE)
                throw BridgeError(KZ_INTERNAL, "Cannot retain nonreplaceable archive path lease");
            WindowsPathLease guard(value);
            BY_HANDLE_FILE_INFORMATION info{};
            if (!GetFileInformationByHandle(value, &info) ||
                (info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) ||
                !!(info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) != directory)
                throw BridgeError(KZ_INVALID_ARGUMENT, "Archive path lease is not the expected regular entry");
            leases.push_back(std::move(guard));
            return value;
        };
        auto drive = [](wchar_t letter) { return (letter >= L'A' && letter <= L'Z') || (letter >= L'a' && letter <= L'z'); };
        auto unc_root = [&](size_t start) {
            const auto server_end = parent.find(L'\\', start);
            const auto share_end = server_end == std::wstring::npos ? std::wstring::npos : parent.find(L'\\', server_end + 1);
            if (server_end == start || server_end == std::wstring::npos
                || share_end == server_end + 1 || share_end == std::wstring::npos)
                throw BridgeError(KZ_INVALID_ARGUMENT, "Invalid retained UNC archive root");
            return share_end + 1;
        };
        size_t root_end = 0;
        if (parent.size() >= 7 && parent.compare(0, 4, L"\\\\?\\") == 0
            && drive(parent[4]) && parent[5] == L':' && parent[6] == L'\\') {
            root_end = 7;
        } else if (parent.compare(0, 8, L"\\\\?\\UNC\\") == 0) {
            root_end = unc_root(8);
        } else if (parent.compare(0, 11, L"\\\\?\\Volume{") == 0) {
            const auto end = parent.find(L"}\\", 11);
            if (end == std::wstring::npos)
                throw BridgeError(KZ_INVALID_ARGUMENT, "Invalid retained volume archive root");
            root_end = end + 2;
        } else if (parent.size() >= 3 && drive(parent[0]) && parent[1] == L':' && parent[2] == L'\\') {
            root_end = 3;
        } else if (parent.size() > 2 && parent.compare(0, 2, L"\\\\") == 0
            && parent[2] != L'?' && parent[2] != L'.') {
            root_end = unc_root(2);
        } else {
            throw BridgeError(KZ_INVALID_ARGUMENT, "Unsupported retained archive filesystem root");
        }
        auto lease_prefix = [&](size_t length) {
            if (length == parent.size()) { lease(parent.c_str(), true); return; }
            const auto saved = parent[length];
            parent[length] = L'\0';
            try { lease(parent.c_str(), true); }
            catch (...) { parent[length] = saved; throw; }
            parent[length] = saved;
        };
        lease_prefix(root_end);
        for (size_t start = root_end; start < parent.size();) {
            const auto end = parent.find(L'\\', start);
            const auto length = end - start;
            if (end == std::wstring::npos || length == 0 ||
                (length == 1 && parent[start] == L'.') ||
                (length == 2 && parent[start] == L'.' && parent[start + 1] == L'.') ||
                parent.find(L':', start) < end)
                throw BridgeError(KZ_INVALID_ARGUMENT, "Retained archive path is not normalized");
            lease_prefix(end);
            start = end + 1;
        }
        std::vector<std::string> part_names;
        part_names.reserve(count);
        for (size_t index = 0; index < count; ++index) {
            const auto name = input_string(files[index].path, files[index].path_len);
            if (name.empty() || name.find('\0') != std::string::npos || files[index].size == 0)
                throw BridgeError(KZ_INVALID_ARGUMENT, "Invalid retained archive filename");
            part_names.push_back(name);
            const auto path = wide_path(name);
            if (path.size() <= parent.size() || path.compare(0, parent.size(), parent) != 0
                || path.find_last_of(L'\\') + 1 != parent.size())
                throw BridgeError(KZ_INVALID_ARGUMENT, "Retained archive parts must share one directory");
            const auto opened = lease(path.c_str(), false);
            BY_HANDLE_FILE_INFORMATION expected{}, actual{};
            if (!GetFileInformationByHandle(reinterpret_cast<HANDLE>(files[index].handle), &expected) ||
                !GetFileInformationByHandle(opened, &actual) || actual.nNumberOfLinks != 1 ||
                expected.dwVolumeSerialNumber != actual.dwVolumeSerialNumber ||
                expected.nFileIndexHigh != actual.nFileIndexHigh ||
                expected.nFileIndexLow != actual.nFileIndexLow ||
                ((static_cast<uint64_t>(actual.nFileSizeHigh) << 32) | actual.nFileSizeLow) != files[index].size)
                throw BridgeError(KZ_INVALID_ARGUMENT, "Retained archive part identity changed");
        }
        auto handle = std::make_unique<kz_archive>(std::move(leases), part_names);
#endif
        if (handle->archive.getFilesize() != expected_size)
            throw BridgeError(KZ_INVALID_ARGUMENT, "Native archive opened a different part set");
        *out = handle.release();
        return {nullptr, 0, KZ_OK};
    });
}

extern "C" void kz_close(kz_archive *archive) noexcept {
    try { delete archive; } catch (...) { /* Destruction cannot propagate across the C ABI. */ }
}
extern "C" void kz_free(uint8_t *data) noexcept { std::free(data); }

extern "C" kz_result kz_query(kz_archive *handle, int operation,
                                const uint8_t *input, size_t input_len, int fulltext,
                                uint32_t offset, uint32_t limit, int snippets) noexcept {
    return guarded([&]() -> kz_result {
        auto &archive = archive_of(handle);
        switch (operation) {
        case KZ_METADATA: return result(metadata(archive));
        case KZ_ENTRY:
            return result(item_json(resolved(archive.getEntryByPath(input_string(input, input_len))).getItem()));
        case KZ_MAIN: return result(item_json(resolved(archive.getMainEntry()).getItem()));
        case KZ_RANDOM: return result(item_json(resolved(archive.getRandomEntry()).getItem()));
        case KZ_SEARCH:
            return result(search(archive, input_string(input, input_len, MAX_QUERY), fulltext != 0, offset, limit, snippets != 0));
        case KZ_INTEGRITY:
        case KZ_STRUCTURAL_INTEGRITY:
            // Retained inputs use Rust's descriptor-bound checksum, because
            // libzim's checksum implementation reopens filenames. Keep every
            // stock structural check, and still reject missing checksums.
            if (!archive.hasChecksum()) return result("false");
            for (int check = 0; check < static_cast<int>(zim::IntegrityCheck::COUNT); ++check) {
                if (operation == KZ_STRUCTURAL_INTEGRITY && check == static_cast<int>(zim::IntegrityCheck::CHECKSUM)) continue;
                if (!archive.checkIntegrity(static_cast<zim::IntegrityCheck>(check))) return result("false");
            }
            return result("true");
        case KZ_ILLUSTRATION:
            try { return result(item_json(illustration_item(archive))); }
            catch (const zim::EntryNotFound &) { return result("null"); }
        case KZ_TITLE_BATCH: return result(title_batch(archive, offset, limit));
        default: throw BridgeError(KZ_INVALID_ARGUMENT, "Unknown archive operation");
        }
    });
}

extern "C" kz_result kz_read(kz_archive *handle, const uint8_t *path, size_t path_len,
                               uint64_t offset, uint64_t length, uint8_t *destination,
                               size_t capacity, int illustration) noexcept {
    return guarded([&]() -> kz_result {
        auto &archive = archive_of(handle);
        if (length > MAX_READ || length > capacity || (length && !destination))
            throw BridgeError(KZ_INVALID_ARGUMENT, "Archive read exceeds destination or 16 MiB range limit");
        const auto item = illustration ? illustration_item(archive)
            : resolved(archive.getEntryByPath(input_string(path, path_len))).getItem();
        const uint64_t size = item.getSize();
        if (offset > size) throw BridgeError(KZ_INVALID_ARGUMENT, "Archive read offset exceeds item size");
        const auto count = std::min(length, size - offset);
        if (!count) return {nullptr, 0, KZ_OK};
        // Crucially, never call the unbounded getData() overload for media.
        const auto blob = item.getData(offset, count);
        if (blob.size() != count)
            throw BridgeError(KZ_MALFORMED, "Truncated archive item range");
        std::memcpy(destination, blob.data(), static_cast<size_t>(count));
        return {nullptr, static_cast<size_t>(count), KZ_OK};
    });
}
