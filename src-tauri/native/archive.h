// SPDX-License-Identifier: GPL-3.0-or-later
#ifndef KIWIX_NATIVE_ARCHIVE_H
#define KIWIX_NATIVE_ARCHIVE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
#define KZ_NOEXCEPT noexcept
extern "C" {
#else
#define KZ_NOEXCEPT
#endif

/* Private, versioned-by-source ABI. No C++ object or allocator crosses it.
 * Callers serialize ALL operations and destruction on each handle. Strings are
 * length-delimited bytes, never URL-decoded or namespace-rewritten here.
 * Results own data (including error text); release exactly once with kz_free.
 * Successful reads have data == NULL and len == bytes written into destination.
 * No function propagates a C++ exception, including allocation failures.
 */
typedef struct kz_archive kz_archive;
typedef struct kz_result {
    uint8_t *data;
    size_t len;
    int code;
} kz_result;
typedef struct kz_file_input {
    intptr_t handle;
    uint64_t size;
    const uint8_t *path;
    size_t path_len;
} kz_file_input;

enum kz_status {
    KZ_OK = 0,
    KZ_NOT_FOUND = 1,
    KZ_MALFORMED = 2,
    KZ_UNSUPPORTED = 3,
    KZ_INVALID_ARGUMENT = 4,
    KZ_INTERNAL = 5
};
enum kz_operation {
    KZ_METADATA = 0,
    KZ_ENTRY = 1,
    KZ_MAIN = 2,
    KZ_RANDOM = 3,
    KZ_ILLUSTRATION = 6,
    KZ_TITLE_BATCH = 7,
    /* Structural checks only; Rust verifies the checksum over retained handles. */
    KZ_STRUCTURAL_INTEGRITY = 8
};

/* Retained regular file handles in exact split order. POSIX requires the patched
 * descriptor-preserving libzim FdInput; Windows also leases names against rename. */
kz_result kz_open_files(const kz_file_input *files, size_t count, kz_archive **out) KZ_NOEXCEPT;
void kz_close(kz_archive *archive) KZ_NOEXCEPT;
void kz_free(uint8_t *data) KZ_NOEXCEPT;
kz_result kz_query(kz_archive *archive, int operation,
                   const uint8_t *input, size_t input_len,
                   uint32_t offset, uint32_t limit) KZ_NOEXCEPT;
kz_result kz_read(kz_archive *archive, const uint8_t *path, size_t path_len,
                  uint64_t offset, uint64_t length, uint8_t *destination,
                  size_t capacity, int illustration) KZ_NOEXCEPT;

#ifdef __cplusplus
}
#endif
#undef KZ_NOEXCEPT
#endif
